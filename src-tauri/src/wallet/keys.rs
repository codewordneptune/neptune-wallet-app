use std::collections::HashMap;
use std::ops::Deref;
use std::range::Range;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::ensure;
use anyhow::Result;
use neptune_cash::api::export::KeyType;
use neptune_cash::api::export::Network;
use neptune_cash::api::export::ReceivingAddress;
use neptune_cash::api::export::SpendingKey;
use neptune_cash::api::export::SymmetricKey;
use neptune_cash::api::export::WalletEntropy;
use rayon::prelude::*;
use serde::Serialize;
use strum::IntoEnumIterator;

/// Display information about an address
#[derive(Debug, Serialize)]
pub(crate) struct AddressRecord {
    pub key_index: i32,
    pub address: String,
    pub address_short_form: String,
    pub label: Option<String>,
}

impl AddressRecord {
    fn new(
        receiving_address: ReceivingAddress,
        key_index: i32,
        network: Network,
        label: Option<String>,
    ) -> Self {
        let address_short_form = receiving_address.to_bech32m_abbreviated(network).unwrap();
        Self {
            key_index,
            address: receiving_address.to_bech32m(network).unwrap(),
            address_short_form,
            label,
        }
    }
}

impl super::WalletState {
    pub(crate) async fn get_address(&self, index: u64) -> Result<String> {
        let symmetric_key = self.key.nth_generation_spending_key(index);
        let spending_key = SpendingKey::from(symmetric_key);

        spending_key.to_address().to_bech32m(self.network)
    }

    /// Return all addresses of the specified type, except for symmetric
    /// addresses.
    ///
    /// Returns an error if called with the key type of symmetric addresses
    /// since these cannot be presented in a secure manner, at the time of
    /// writing.
    pub(crate) async fn known_addresses(&self, key_type: KeyType) -> Result<Vec<AddressRecord>> {
        ensure!(
            key_type != KeyType::Symmetric,
            "Symmetric keys cannot be shown in a secure manner"
        );

        // Key index represents the *next* address to be derived.
        let key_index = self.ephemeral_key_index(key_type);
        ensure!(
            key_index <= i32::MAX as u64,
            "Key index cannot exceed i32::MAX"
        );
        let range = Range {
            start: 0,
            end: key_index,
        };
        let keys = self.keys(range, key_type);
        let addresses = keys.into_iter().map(|(idx, x)| (idx, x.to_address()));

        // Above cap on key index guarantees that this unwrap can never panic.
        let addresses = addresses
            .map(|(idx, addr)| {
                AddressRecord::new(addr, idx.try_into().unwrap(), self.network, None)
            })
            .collect();

        Ok(addresses)
    }

    /// Return all known spending keys.
    ///
    /// Will always return at least one key per type.
    pub(crate) fn get_known_spending_keys(&self) -> Vec<SpendingKey> {
        let mut all_keys = vec![];
        for key_type in KeyType::iter() {
            let end = self.ephemeral_key_index(key_type);
            let range = Range { start: 0, end };
            let keys = self.keys(range, key_type);

            all_keys.extend(keys);
        }

        all_keys.iter().map(|v| v.1.deref().clone()).collect()
    }

    /// Return all tracked keys, and all future keys up the specified
    /// look-ahead index.
    pub(crate) fn known_and_future_keys(&self) -> HashMap<KeyType, HashMap<u64, Arc<SpendingKey>>> {
        let num_future_keys = self.num_future_keys();
        let mut all_keys = HashMap::new();
        for key_type in KeyType::iter() {
            let start = 0;
            let end = match key_type {
                KeyType::Generation => self.generation_key_index(),
                KeyType::Symmetric => self.symmetric_key_index(),
                KeyType::EcHybrid => self.ec_hybrid_key_index(),
                KeyType::ViewingAddress => self.viewing_address_key_index(),
                _ => todo!(),
            };
            let end = end + num_future_keys;
            let range = Range { start, end };
            let keys: HashMap<u64, Arc<SpendingKey>> =
                self.keys(range, key_type).into_iter().collect();

            all_keys.insert(key_type, keys);
        }

        all_keys
    }

    pub(crate) fn ephemeral_key_index(&self, key_type: KeyType) -> u64 {
        match key_type {
            KeyType::Generation => self.generation_key_index(),
            KeyType::Symmetric => self.symmetric_key_index(),
            KeyType::EcHybrid => self.ec_hybrid_key_index(),
            KeyType::ViewingAddress => self.viewing_address_key_index(),
            _ => todo!(),
        }
    }

    fn symmetric_key_index(&self) -> u64 {
        self.symmetric_key_index.load(Ordering::Relaxed)
    }

    fn generation_key_index(&self) -> u64 {
        self.generation_key_index.load(Ordering::Relaxed)
    }

    fn ec_hybrid_key_index(&self) -> u64 {
        self.ec_hybrid_key_index.load(Ordering::Relaxed)
    }

    fn viewing_address_key_index(&self) -> u64 {
        self.viewing_address_key_index.load(Ordering::Relaxed)
    }

    pub(crate) fn num_future_keys(&self) -> u64 {
        self.num_future_keys.load(Ordering::Relaxed)
    }

    /// Return a list of (key index, key) for the requested key type, in the
    /// specified range.
    pub(crate) fn keys(
        &self,
        range: Range<u64>,
        key_type: KeyType,
    ) -> Vec<(u64, Arc<SpendingKey>)> {
        // TODO: Replace with same function in neptune-core, once available
        // (anything after v0.11.0 should have this functionality).
        /// Return the nth spending key, of the specified type.
        fn nth_spending_key(
            wallet_entropy: &WalletEntropy,
            key_type: KeyType,
            derivation_index: u64,
        ) -> SpendingKey {
            match key_type {
                KeyType::Generation => wallet_entropy
                    .nth_generation_spending_key(derivation_index)
                    .into(),
                KeyType::Symmetric => wallet_entropy.nth_symmetric_key(derivation_index).into(),
                KeyType::EcHybrid => wallet_entropy.nth_ec_hybrid_key(derivation_index).into(),
                KeyType::ViewingAddress => wallet_entropy
                    .nth_viewing_address_key(derivation_index)
                    .into(),
                _ => todo!("Only known key types are supported"),
            }
        }

        let entropy = &self.key;

        let mut keys: Vec<_> = (range.start..range.end)
            .into_par_iter()
            .map(|i| {
                if let Some(key) = self.key_cache.get_key(key_type, i) {
                    return (i, key);
                }
                let new_key = Arc::new(nth_spending_key(entropy, key_type, i));
                self.key_cache.add_key(i, new_key.clone());
                (i, new_key)
            })
            .collect();

        if key_type == KeyType::Symmetric && self.scan_config.recover_from_sym_digest_keys {
            let malformed: Vec<_> = (range.start..range.end)
                .into_par_iter()
                .map(|i| {
                    let key = Arc::new(SpendingKey::from(entropy.nth_symmetric_key(i)));
                    let key = SymmetricKey::from_seed(key.privacy_preimage());
                    let key: SpendingKey = key.into();
                    (i, Arc::new(key))
                })
                .collect();
            keys.extend(malformed);
        };

        keys
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::test_devnet_wallet;

    use super::*;

    #[tokio::test]
    async fn looks_ahead_right_number_of_addresses() {
        let wallet_state = test_devnet_wallet().await;
        let num_known_keys = wallet_state.get_known_spending_keys().len();
        let num_known_and_future: usize = wallet_state
            .known_and_future_keys()
            .values()
            .map(|x| x.len())
            .sum();

        // Looks ahead for *each* key type. So must multiply by number of key
        // types to get total number of future keys.
        let total_num_future = KeyType::iter().count() * wallet_state.num_future_keys() as usize;

        assert_eq!(num_known_keys + total_num_future, num_known_and_future);
    }

    #[tokio::test]
    async fn knows_one_key_per_key_type_at_init() {
        let wallet_state = test_devnet_wallet().await;
        assert_eq!(
            KeyType::iter().count(),
            wallet_state.get_known_spending_keys().len()
        );
    }
}
