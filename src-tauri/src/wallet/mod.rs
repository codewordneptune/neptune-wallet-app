use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::range::Range;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use anyhow::Context;
use anyhow::Result;
use itertools::Itertools;
use neptune_cash::api::export::AdditionRecord;
use neptune_cash::api::export::Network;
use neptune_cash::api::export::Timestamp;
use neptune_cash::api::export::Tip5;
use neptune_cash::api::export::Utxo;
use neptune_cash::application::config::data_directory::DataDirectory;
use neptune_cash::prelude::tasm_lib::prelude::Digest;
use neptune_cash::state::wallet::incoming_utxo::IncomingUtxo;
use neptune_cash::state::wallet::wallet_entropy::WalletEntropy;
use neptune_cash::util_types::mutator_set::commit;
use neptune_cash::util_types::mutator_set::removal_record::absolute_index_set::AbsoluteIndexSet;
use pending::TransactionUpdater;
use rayon::prelude::*;
use serde::Deserialize;
use serde::Serialize;
use sqlx::Pool;
use sqlx::Sqlite;
use sqlx::SqliteConnection;
use tracing::*;
use wallet_file::wallet_dir_by_id;
use wallet_state_table::UtxoBlockInfo;
use wallet_state_table::UtxoDbData;

use crate::config::wallet::ScanConfig;
use crate::config::wallet::WalletConfig;
use crate::config::Config;
use crate::wallet::wallet_block::WalletBlock;

// mod archive_state;
pub(crate) mod balance;
pub(crate) mod fake_archival_state;
pub(crate) mod fork;
mod input;
pub(crate) mod wallet_block;
pub(crate) use input::InputSelectionRule;
pub(crate) mod block_cache;
mod key_cache;
mod keys;
mod pending;
mod spend;
pub(crate) mod sync;
pub(crate) mod wallet_file;
mod wallet_state_table;

pub(crate) struct WalletState {
    key: WalletEntropy,
    scan_config: ScanConfig,
    pub(crate) network: Network,
    symmetric_key_index: AtomicU64,
    generation_key_index: AtomicU64,
    num_future_keys: AtomicU64,
    pub(crate) pool: Pool<Sqlite>,
    updater: TransactionUpdater,
    key_cache: key_cache::KeyCache,
    id: i64,
    spend_lock: tokio::sync::Mutex<()>,
}

impl WalletState {
    pub(crate) async fn new_from_config(config: &Config) -> Result<Self> {
        let wallet_config = config.get_current_wallet().await?;
        let database = Self::wallet_database_path(config, wallet_config.id).await?;
        Self::new(wallet_config, &database).await
    }

    pub(crate) async fn wallet_database_path(config: &Config, id: i64) -> Result<PathBuf> {
        let wallet_dir = Self::wallet_path(config, id).await?;
        DataDirectory::create_dir_if_not_exists(&wallet_dir).await?;
        Ok(wallet_dir.join("wallet_state.db"))
    }

    async fn wallet_database_internal(database: &Path) -> Result<Pool<Sqlite>> {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(database)
            .create_if_missing(true);

        sqlx::SqlitePool::connect_with(options)
            .await
            .map_err(|err| {
                anyhow::anyhow!(
                    "Could not connect to database: {err}. Path: {}",
                    database.to_string_lossy()
                )
            })
    }

    pub(crate) async fn wallet_database_connection(
        config: &Config,
        id: i64,
    ) -> Result<Pool<Sqlite>> {
        let database = Self::wallet_database_path(config, id).await?;
        Self::wallet_database_internal(&database).await
    }

    pub(crate) async fn wallet_path(config: &Config, id: i64) -> Result<PathBuf> {
        let data_dir = config.get_data_dir().await?;
        let network = config.get_network().await?;
        let wallet_dir = wallet_dir_by_id(&data_dir, network, id);
        Ok(wallet_dir)
    }

    pub(crate) async fn new(wallet_config: WalletConfig, database: &Path) -> Result<Self> {
        let pool = Self::wallet_database_internal(database).await?;

        let num_future_keys = wallet_config.scan_config.num_keys;

        let updater = TransactionUpdater::new(pool.clone()).await?;

        let state = Self {
            key: wallet_config.key,
            scan_config: wallet_config.scan_config,
            network: wallet_config.network,
            symmetric_key_index: AtomicU64::new(0),
            generation_key_index: AtomicU64::new(0),
            num_future_keys: AtomicU64::new(num_future_keys),
            pool: pool.clone(),
            updater,
            key_cache: key_cache::KeyCache::new(),
            id: wallet_config.id,
            spend_lock: tokio::sync::Mutex::new(()),
        };

        state.migrate_tables().await.context("migrate_tables")?;
        state
            .generation_key_index
            .store(state.get_generation_key_index().await?, Ordering::Relaxed);
        state
            .symmetric_key_index
            .store(state.get_symmetric_key_index().await?, Ordering::Relaxed);

        debug!("Wallet state initialized");

        Ok(state)
    }

    pub(crate) async fn start_height(&self) -> Result<u64> {
        if let Some(tip) = self.get_tip().await? {
            return Ok(tip.0 + 1);
        }
        info!(
            "new sync, using scan_config height: {}",
            self.scan_config.start_height
        );
        Ok(self.scan_config.start_height)
    }

    pub(crate) async fn update_new_tip(
        &self,
        block: &WalletBlock,
        should_update: bool,
    ) -> Result<Option<u64>> {
        let height: u64 = block.kernel.header.height.into();

        let mut tx = self.pool.begin().await?;

        let _spend_guard = self.spend_lock.lock().await;

        debug!("check fork");
        if let Some((luca_height, luca_digest)) =
            self.find_rollback_luca(block).await.context("check fork")?
        {
            info!(
                "Fork detected! Reorganizing: reorganize_to_height: {luca_height} {luca_digest:x}"
            );
            self.roll_back(&mut tx, luca_height, luca_digest)
                .await
                .context("reorganize_to_height")?;
            tx.commit().await.context("commit db")?;
            return Ok(Some(luca_height));
        }
        debug!("update mutator set");

        let addition_records = block.all_addition_records();

        debug!("get removal_records");

        debug!("scan for incoming utxo");
        let incommings = self.par_scan_for_incoming_utxo(block).await?;
        let mut recovery_datas = Vec::with_capacity(incommings.len());

        let incoming = incommings
            .into_iter()
            .map(|v| (v.addition_record(), v))
            .collect::<std::collections::HashMap<_, _>>();

        debug!("iterate addition records");
        let num_aocl_leafs_prior_to_this_block = block.num_aocl_leafs_prior();
        for (aocl_leaf_index, addition_record) in
            (num_aocl_leafs_prior_to_this_block..).zip(addition_records.iter())
        {
            if let Some(incoming_utxo) = incoming.get(addition_record) {
                let r = incoming_utxo_recovery_data_from_incomming_utxo(
                    incoming_utxo.clone(),
                    aocl_leaf_index,
                );
                assert_eq!(
                    *addition_record,
                    r.addition_record(),
                    "Addition record of wallet's UTXO must match that from block"
                );

                let timelock_info = if let Some(timelock) = incoming_utxo.utxo.release_date() {
                    if timelock > Timestamp::now() {
                        timelock.standard_format()
                    } else {
                        "released".to_owned()
                    }
                } else {
                    "none".to_owned()
                };
                info!(
                    "Received UTXO in block {height}. Value: {}. Timelock: {timelock_info}",
                    incoming_utxo.utxo.get_native_currency_amount(),
                );

                recovery_datas.push(r);
            }
        }

        debug!("append utxos");
        let mut db_datas = vec![];
        for recovery_data in recovery_datas {
            let digest = Tip5::hash(&recovery_data.utxo);
            let db_data = UtxoDbData {
                id: 0,
                hash: digest.to_hex(),
                recovery_data,
                spent_in_block: None,
                confirmed_in_block: UtxoBlockInfo {
                    block_height: height,
                    block_digest: block.hash,
                    timestamp: block.kernel.header.timestamp,
                },
                spent_height: None,
                confirm_height: height.try_into()?,
                confirmed_txid: None,
                spent_txid: None,
            };
            db_datas.push(db_data);
        }

        self.append_utxos(&mut tx, db_datas).await?;

        debug!("scan for spent utxos");
        let spents = self.scan_for_spent_utxos(&mut tx, block).await?;
        debug!("Found {} spent UTXOs", spents.len());

        let block_info = UtxoBlockInfo {
            block_height: block.kernel.header.height.into(),
            block_digest: block.hash,
            timestamp: block.kernel.header.timestamp,
        };

        let spent_updates = spents
            .iter()
            .map(|v| (v.2, block_info.clone()))
            .collect_vec();

        debug!("update spent utxos");
        self.update_spent_utxos(&mut tx, spent_updates).await?;

        debug!("scan for expected utxos");
        // update expected utxo with txid
        let expected = self
            .scan_for_expected_utxos(block)
            .await?
            .into_iter()
            .map(|(recovery, txid)| {
                let digest = Tip5::hash(&recovery.utxo);
                (digest, txid)
            })
            .collect_vec();

        debug!("update utxos with expected utxos");
        self.update_utxos_with_expected_utxos(&mut tx, expected, height.try_into()?)
            .await?;

        debug!(
            "set tip {} {:x}",
            block.kernel.header.height.value(),
            block.hash
        );
        self.set_tip(&mut tx, (block.kernel.header.height.into(), block.hash))
            .await?;

        tx.commit().await?;

        self.clean_old_expected_utxos().await?;

        if should_update {
            self.updater.update_transactions(self).await;
        }

        if height.is_multiple_of(20) {
            info!("sync finished: {}", height);
        } else {
            debug!("sync finished: {}", height);
        }

        Ok(None)
    }

    async fn par_scan_for_incoming_utxo(
        &self,
        block: &WalletBlock,
    ) -> anyhow::Result<Vec<IncomingUtxo>> {
        let transaction = block.kernel.body.transaction_kernel();

        let all_addition_records: HashSet<_> = block.all_addition_records().into_iter().collect();

        let spendingkeys = self.get_future_generation_spending_keys(Range {
            start: 0,
            end: self.generation_key_index() + self.num_future_keys(),
        });
        let spend_to_spendingkeys = spendingkeys.par_iter().flat_map(|(key_idx, key)| {
            let utxo = key.scan_for_announced_utxos(transaction);
            let actually_received = utxo
                .iter()
                .any(|utxo| all_addition_records.contains(&utxo.addition_record()));
            if !utxo.is_empty() && actually_received {
                // Only bump index if block actually contains this output.
                self.generation_key_index
                    .fetch_max(*key_idx, Ordering::SeqCst);
            }
            utxo
        });

        let symmetric_keys = self.get_future_symmetric_keys(Range {
            start: 0,
            end: self.symmetric_key_index() + self.num_future_keys(),
        });
        let spend_to_symmetrickeys = symmetric_keys.par_iter().flat_map(|(key_idx, key)| {
            let utxo = key.scan_for_announced_utxos(transaction);
            let actually_received = utxo
                .iter()
                .any(|utxo| all_addition_records.contains(&utxo.addition_record()));
            if !utxo.is_empty() && actually_received {
                // Only bump index if block actually contains this output.
                self.symmetric_key_index
                    .fetch_max(*key_idx, Ordering::SeqCst);
            }
            utxo
        });

        let (own_guesser_address, guesser_key_preimage) = {
            let own_guesser_key = self.key.guesser_fee_key();
            (
                own_guesser_key.to_address(),
                own_guesser_key.receiver_preimage(),
            )
        };
        let was_guessed_by_us = block
            .kernel
            .header
            .was_guessed_by(&own_guesser_address.into());

        let gusser_incoming_utxos = if was_guessed_by_us {
            let sender_randomness = block.hash;
            block
                .kernel
                .guesser_fee_utxos()
                .expect("Exported block must have guesser fee UTXOs")
                .into_iter()
                .map(|utxo| IncomingUtxo {
                    utxo,
                    sender_randomness,
                    receiver_preimage: guesser_key_preimage,
                    is_guesser_fee: true,
                })
                .collect_vec()
        } else {
            vec![]
        };

        let expected_utxos = self
            .scan_for_expected_utxos(block)
            .await?
            .into_iter()
            .map(|(utxo, _)| utxo)
            .collect_vec();

        let receive = spend_to_spendingkeys
            .chain(spend_to_symmetrickeys)
            .chain(gusser_incoming_utxos)
            .chain(expected_utxos)
            .collect::<Vec<_>>();

        // Bump derivation indices. Must be done *after* the iterators above
        // have been consumed.
        self.set_generation_key_index(self.generation_key_index())
            .await?;
        self.set_symmetric_key_index(self.symmetric_key_index())
            .await?;

        Ok(receive)
    }

    /// Return a list of UTXOs spent by this wallet in the transaction
    ///
    /// Returns a list of tuples (utxo, absolute-index-set, index-into-database).
    async fn scan_for_spent_utxos(
        &self,
        tx: &mut SqliteConnection,
        block: &WalletBlock,
    ) -> Result<Vec<(Utxo, AbsoluteIndexSet, i64)>> {
        let confirmed_absolute_index_sets: HashSet<_> = block
            .kernel
            .body
            .transaction_kernel()
            .inputs
            .iter()
            .map(|rr| rr.absolute_indices)
            .collect();

        let monitored_utxos = self.get_unspent_utxos(tx).await?;

        let mut spent_own_utxos = vec![];

        for monitored_utxo in monitored_utxos {
            let utxo: UtxoRecoveryData = monitored_utxo.recovery_data;

            if confirmed_absolute_index_sets.contains(&utxo.abs_i()) {
                spent_own_utxos.push((utxo.utxo.clone(), utxo.abs_i(), monitored_utxo.id));
            }
        }

        Ok(spent_own_utxos)
    }

    // returns (IncomingUtxo, txid) pairs
    pub(crate) async fn scan_for_expected_utxos(
        &self,
        block: &WalletBlock,
    ) -> Result<Vec<(IncomingUtxo, String)>> {
        let outputs = block.all_addition_records();

        let expected_utxos = self.expected_utxos().await?;
        let eu_map: HashMap<_, _> = expected_utxos
            .into_iter()
            .map(|eu| (eu.expected_utxo.addition_record, eu))
            .collect();

        let incommings = outputs
            .iter()
            .filter_map(move |a| {
                eu_map
                    .get(a)
                    .map(|eu| (IncomingUtxo::from(&eu.expected_utxo), eu.txid.to_owned()))
            })
            .collect_vec();
        Ok(incommings)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct UtxoRecoveryData {
    pub(crate) utxo: Utxo,
    pub(crate) sender_randomness: Digest,
    pub(crate) receiver_preimage: Digest,
    pub(crate) aocl_index: u64,
}

impl UtxoRecoveryData {
    pub(crate) fn abs_i(&self) -> AbsoluteIndexSet {
        let utxo_digest = Tip5::hash(&self.utxo);

        AbsoluteIndexSet::compute(
            utxo_digest,
            self.sender_randomness,
            self.receiver_preimage,
            self.aocl_index,
        )
    }

    pub(crate) fn addition_record(&self) -> AdditionRecord {
        commit(
            Tip5::hash(&self.utxo),
            self.sender_randomness,
            self.receiver_preimage.hash(),
        )
    }
}

fn incoming_utxo_recovery_data_from_incomming_utxo(
    utxo: IncomingUtxo,
    num_aocl_leafs: u64,
) -> UtxoRecoveryData {
    let aocl_index = num_aocl_leafs;

    UtxoRecoveryData {
        utxo: utxo.utxo,
        sender_randomness: utxo.sender_randomness,
        receiver_preimage: utxo.receiver_preimage,
        aocl_index,
    }
}

#[cfg(test)]
mod tests {
    use neptune_cash::api::export::NativeCurrencyAmount;
    use neptune_cash::api::export::SpendingKey;
    use neptune_cash::application::json_rpc::core::model::wallet::block::RpcWalletBlock;
    use neptune_cash::protocol::consensus::block::Block;
    use tracing_test::traced_test;

    use super::*;
    use crate::tests::test_wallet_db;
    use crate::tests::wallet_block_from_test_data;
    use crate::wallet::sync::SyncState;

    impl WalletState {
        fn get_future_spending_keys(&self) -> Vec<(u64, std::sync::Arc<SpendingKey>)> {
            let last_gen_key = self.generation_key_index() + self.scan_config.num_keys;
            let last_sym_key = self.symmetric_key_index() + self.scan_config.num_keys;

            let mut keys = self.get_future_generation_spending_keys(Range {
                start: 0,
                end: last_gen_key,
            });
            keys.extend(self.get_future_symmetric_keys(Range {
                start: 0,
                end: last_sym_key,
            }));

            keys
        }
    }

    #[traced_test]
    #[tokio::test]
    async fn print_future_addresses() {
        // Verify that generation and symmetric keys of higher indices can be
        // found.
        let network = Network::Main;
        let config = WalletConfig {
            id: 0,
            key: WalletEntropy::devnet_wallet(),
            scan_config: ScanConfig {
                num_keys: 20,
                start_height: 0,
            },
            network,
        };

        let db_path = test_wallet_db().await;
        let wallet_state = WalletState::new(config, &db_path).await.unwrap();

        println!("Known addresses:");
        for key in wallet_state.get_known_spending_keys() {
            println!("{}", key.to_address().to_display_bech32m(network).unwrap());
        }

        println!("Future addresses:");
        for (i, key) in wallet_state.get_future_spending_keys() {
            println!(
                "{i}: {}",
                key.to_address().to_display_bech32m(network).unwrap()
            );
        }
    }

    #[traced_test]
    #[tokio::test]
    async fn credits_utxo_to_address_with_derivation_index_1_and_2() {
        let network = Network::Main;
        let config = WalletConfig {
            id: 0,
            key: WalletEntropy::devnet_wallet(),
            scan_config: ScanConfig {
                num_keys: 100,
                start_height: 0,
            },
            network,
        };

        let db_path = test_wallet_db().await;
        println!("db_path: {}", db_path.to_string_lossy());
        let wallet_state = WalletState::new(config.clone(), &db_path).await.unwrap();
        assert_eq!(
            0,
            wallet_state.generation_key_index(),
            "Key index must be 0 prior to handling of block"
        );

        let mut block_height = 38260;
        let block_38260 = wallet_block_from_test_data(block_height).unwrap();

        wallet_state
            .update_new_tip(&block_38260, false)
            .await
            .unwrap();

        // Genesis block was never retrieved, so only income in block 38260
        // should be registered: 0.75 NPT.
        let three_quarters =
            NativeCurrencyAmount::coins(1).half() + NativeCurrencyAmount::coins(1).half().half();
        assert_eq!(three_quarters, wallet_state.get_balance().await.unwrap());
        assert_eq!(
            1,
            wallet_state.generation_key_index(),
            "Key index must be 1 after handling block, as key with index 1 got a UTXO in it"
        );

        // Verify that bumping of keys was persisted.
        let wallet_stated_persisted1 = WalletState::new(config.clone(), &db_path).await.unwrap();
        assert_eq!(
            1,
            wallet_stated_persisted1.generation_key_index(),
            "Persisted key index must be 1"
        );

        // In block 38'289, the wallet sends 0.75 NPT, and receives 0.50 NPT.
        // Output is received on gen key with index 2.
        block_height += 1;
        while block_height < 38290 {
            let block = wallet_block_from_test_data(block_height).unwrap();
            wallet_state.update_new_tip(&block, false).await.unwrap();
            block_height += 1;
        }

        assert_eq!(
            NativeCurrencyAmount::coins(1).half(),
            wallet_state.get_balance().await.unwrap()
        );
        assert_eq!(
            2,
            wallet_state.generation_key_index(),
            "Key index must be 2 after receiving UTXO to key with index 2"
        );

        // Verify that bumping of keys was persisted.
        let wallet_stated_persisted2 = WalletState::new(config, &db_path).await.unwrap();
        assert_eq!(
            2,
            wallet_stated_persisted2.generation_key_index(),
            "Persisted key index must be 2"
        );
    }

    #[traced_test]
    #[tokio::test]
    async fn genesis_credits_devnet_wallet() {
        let network = Network::Main;
        let config = WalletConfig {
            id: 0,
            key: WalletEntropy::devnet_wallet(),
            scan_config: ScanConfig {
                num_keys: 1,
                start_height: 0,
            },
            network,
        };

        let db_path = test_wallet_db().await;
        let wallet_state = WalletState::new(config, &db_path).await.unwrap();

        let genesis: RpcWalletBlock = (&Block::genesis(network)).into();
        let genesis: WalletBlock = genesis.into();
        let premine_keys = wallet_state.get_known_spending_keys();

        let expected_utxos = SyncState::check_premine_for_tests(network, &premine_keys);
        wallet_state
            .add_expected_utxo(expected_utxos)
            .await
            .unwrap();
        wallet_state.update_new_tip(&genesis, false).await.unwrap();

        assert_eq!(
            NativeCurrencyAmount::coins(20),
            wallet_state.get_balance().await.unwrap()
        );
    }
}
