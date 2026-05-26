use anyhow::Result;
use neptune_cash::api::export::Digest;
use neptune_cash::api::export::KeyType;
use neptune_cash::state::wallet::wallet_state::IncomingUtxoRecoveryData;
use std::collections::HashMap;
use std::collections::HashSet;
use std::range::Range;

use crate::wallet::UtxoRecoveryData;

impl super::WalletState {
    /// Return the highest key index that has received an incoming UTXO for each
    /// key type.
    ///
    /// Will look ahead exactly [`Self::num_future_keys`]. If a gap of more than
    /// this number of keys exist in the incoming UTXOs, those UTXOs will not be
    /// considered. In other words: If the wallet has received UTXOs for keys
    /// with key indices more than [`Self::num_future_keys`] above the key index
    /// where the previous UTXO was received, then all UTXOs received after this
    /// gap will not be counted.
    pub(super) fn max_key_index_used(
        &self,
        incoming_utxos: &[IncomingUtxoRecoveryData],
    ) -> HashMap<KeyType, Option<u64>> {
        let mut ret = HashMap::new();
        let receiver_preimages: HashSet<_> =
            incoming_utxos.iter().map(|x| x.receiver_preimage).collect();
        for key_type in KeyType::all_types() {
            let mut max_index = None;

            loop {
                let start = max_index.map(|x| x + 1).unwrap_or(0);
                let end = start + self.num_future_keys();
                let keys = match key_type {
                    KeyType::Generation => self.generation_keys(Range { start, end }),
                    KeyType::Symmetric => self.symmetric_keys(Range { start, end }),
                    _ => todo!("Only generation and symmetric key types are currently supported"),
                };

                let max = keys
                    .iter()
                    .filter(|(_index, key)| receiver_preimages.contains(&key.privacy_preimage()))
                    .map(|(index, _key)| *index)
                    .max();

                if let Some(max) = max {
                    max_index = Some(max);
                } else {
                    break;
                }
            }

            ret.insert(key_type, max_index);
        }

        ret
    }

    /// Return the incoming UTXOs that are not yet known to this wallet.
    pub(super) async fn not_yet_claimed(
        &self,
        incoming_utxos: Vec<IncomingUtxoRecoveryData>,
    ) -> Result<Vec<UtxoRecoveryData>> {
        let already_claimed: HashSet<_> = self
            .get_utxos()
            .await?
            .into_iter()
            .map(|x| x.recovery_data.abs_i())
            .collect();

        let incoming_utxos: Vec<_> = incoming_utxos
            .into_iter()
            .map(|incoming| {
                let recovery_data: UtxoRecoveryData = incoming.into();
                recovery_data
            })
            .filter(|incoming| {
                let absi = incoming.abs_i();
                !already_claimed.contains(&absi)
            })
            .collect();

        Ok(incoming_utxos)
    }

    /// Return the incoming UTXOs that have keys that are known to this wallet.
    pub(super) fn have_matching_keys(
        &self,
        incoming_utxos: Vec<UtxoRecoveryData>,
    ) -> Vec<UtxoRecoveryData> {
        let all_keys = self.known_and_future_keys();
        let all_privacy_preimages: HashSet<Digest> = all_keys
            .values()
            .flat_map(|keys| keys.values().map(|key| key.privacy_preimage()))
            .collect();
        incoming_utxos
            .into_iter()
            .filter(|incoming| all_privacy_preimages.contains(&incoming.receiver_preimage))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use neptune_cash::api::export::Network;
    use neptune_cash::api::export::WalletEntropy;

    use crate::config::wallet::ScanConfig;
    use crate::config::wallet::WalletConfig;
    use crate::tests::load_incoming_randomness;
    use crate::tests::test_wallet_db;
    use crate::wallet::WalletState;

    use super::*;

    async fn setup() -> (Vec<IncomingUtxoRecoveryData>, WalletState) {
        let network = Network::Main;
        let config = WalletConfig {
            id: 0,
            key: WalletEntropy::devnet_wallet(),
            scan_config: ScanConfig {
                num_keys: 100,
                start_height: 0,
                recover_from_sym_digest_keys: false,
            },
            network,
        };

        let db_path = test_wallet_db().await;
        let wallet = WalletState::new(config.clone(), &db_path).await.unwrap();
        let incoming_utxos = load_incoming_randomness("devnet_incoming_randomness_block38434.dat");

        (incoming_utxos, wallet)
    }

    #[tokio::test]
    async fn max_key_index_devnet() {
        let (incoming_utxos, wallet) = setup().await;
        let result = wallet.max_key_index_used(&incoming_utxos);
        assert_eq!(result[&KeyType::Generation], Some(2));
        assert_eq!(result[&KeyType::Symmetric], Some(4));
    }

    #[tokio::test]
    async fn not_yet_claimed_devnet() {
        let (incoming_utxos, wallet) = setup().await;
        let expected: Vec<UtxoRecoveryData> = incoming_utxos
            .clone()
            .into_iter()
            .map(|x| x.into())
            .collect();
        let result = wallet.not_yet_claimed(incoming_utxos).await.unwrap();
        assert_eq!(expected, result);
    }

    #[tokio::test]
    async fn have_matching_keys_devnet() {
        let (incoming_utxos, wallet) = setup().await;
        let incoming_utxos: Vec<UtxoRecoveryData> =
            incoming_utxos.into_iter().map(|x| x.into()).collect();
        let expected = incoming_utxos.clone();
        let result = wallet.have_matching_keys(incoming_utxos);
        assert_eq!(expected, result);
    }
}
