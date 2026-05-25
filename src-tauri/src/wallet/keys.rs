use std::collections::HashMap;
use std::ops::Deref;
use std::range::Range;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use neptune_cash::api::export::KeyType;
use neptune_cash::api::export::SpendingKey;
use neptune_cash::api::export::SymmetricKey;
use rayon::prelude::*;

impl super::WalletState {
    pub(crate) async fn get_address(&self, index: u64) -> Result<String> {
        let symmetric_key = self.key.nth_generation_spending_key(index);
        let spending_key = SpendingKey::from(symmetric_key);

        spending_key.to_address().to_bech32m(self.network)
    }

    pub(crate) fn get_known_spending_keys(&self) -> Vec<SpendingKey> {
        let spending_keys = self.generation_keys(Range {
            start: 0,
            end: self.generation_key_index() + 1,
        });
        let spending_keys = spending_keys.iter().map(|v| *v.1.deref());

        let symmetric_keys = self.symmetric_keys(Range {
            start: 0,
            end: self.symmetric_key_index() + 1,
        });
        let symmetric_keys = symmetric_keys.iter().map(|v| *v.1.deref());

        spending_keys.chain(symmetric_keys).collect()
    }

    /// Return all tracked keys, and all future keys up the specified
    /// look-ahead index.
    pub(crate) fn known_and_future_keys(&self) -> HashMap<KeyType, HashMap<u64, Arc<SpendingKey>>> {
        let num_future_keys = self.num_future_keys();
        let mut all_keys = HashMap::new();
        for key_type in KeyType::all_types() {
            let keys: HashMap<u64, Arc<SpendingKey>> = match key_type {
                KeyType::Generation => self
                    .generation_keys(Range {
                        start: 0,
                        end: self.generation_key_index() + num_future_keys,
                    })
                    .into_iter()
                    .map(|(index, key)| (index, key.to_owned()))
                    .collect(),
                KeyType::Symmetric => self
                    .symmetric_keys(Range {
                        start: 0,
                        end: self.symmetric_key_index() + num_future_keys,
                    })
                    .into_iter()
                    .map(|(index, key)| (index, key.to_owned()))
                    .collect(),
                _ => unreachable!(),
            };

            all_keys.insert(key_type, keys);
        }

        all_keys
    }

    pub(crate) fn symmetric_key_index(&self) -> u64 {
        self.symmetric_key_index.load(Ordering::Relaxed)
    }

    pub(crate) fn generation_key_index(&self) -> u64 {
        self.generation_key_index.load(Ordering::Relaxed)
    }

    pub(crate) fn num_future_keys(&self) -> u64 {
        self.num_future_keys.load(Ordering::Relaxed)
    }

    /// Return a list of (key index, key) of symmetric keys.
    pub(crate) fn symmetric_keys(&self, range: Range<u64>) -> Vec<(u64, Arc<SpendingKey>)> {
        let key = &self.key;

        let well_formed: Vec<_> = (range.start..range.end)
            .into_par_iter()
            .map(|i| {
                if let Some(key) = self.key_cache.get_symmetric_key(i) {
                    return (i, key);
                }
                let new_key = Arc::new(SpendingKey::from(key.nth_symmetric_key(i)));
                self.key_cache.add_symmetric_key(i, new_key.clone());
                (i, new_key)
            })
            .collect();

        let malformed: Vec<_> = if self.scan_config.recover_from_sym_digest_keys {
            (range.start..range.end)
                .into_par_iter()
                .map(|i| {
                    let key = Arc::new(SpendingKey::from(key.nth_symmetric_key(i)));
                    let key = SymmetricKey::from_seed(key.privacy_preimage());
                    let key: SpendingKey = key.into();
                    (i, Arc::new(key))
                })
                .collect()
        } else {
            vec![]
        };

        [well_formed, malformed].concat()
    }

    /// Return a list of (key index, key) of generation keys.
    pub(crate) fn generation_keys(&self, range: Range<u64>) -> Vec<(u64, Arc<SpendingKey>)> {
        let key = &self.key;
        (range.start..range.end)
            .into_par_iter()
            .map(|i| {
                if let Some(key) = self.key_cache.get_generation_spending_key(i) {
                    return (i, key);
                }
                let new_key = Arc::new(SpendingKey::from(key.nth_generation_spending_key(i)));
                self.key_cache
                    .add_generation_spending_key(i, new_key.clone());
                (i, new_key)
            })
            .collect()
    }
}
