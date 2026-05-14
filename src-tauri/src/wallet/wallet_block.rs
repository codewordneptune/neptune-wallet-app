use neptune_cash::api::export::AdditionRecord;
use neptune_cash::api::export::Digest;
use neptune_cash::application::json_rpc::core::model::wallet::block::RpcWalletBlock;
use neptune_cash::prelude::twenty_first::prelude::Mmr;
use neptune_cash::protocol::consensus::block::block_kernel::BlockKernel;
use neptune_cash::util_types::mutator_set::mutator_set_accumulator::MutatorSetAccumulator;
use serde::Deserialize;
use serde::Serialize;

/// A block tailored for this program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WalletBlock {
    pub(crate) kernel: BlockKernel,
    pub(crate) hash: Digest,
}

impl From<RpcWalletBlock> for WalletBlock {
    fn from(value: RpcWalletBlock) -> Self {
        let hash = value.hash();
        Self {
            kernel: value.kernel.into(),
            hash,
        }
    }
}

impl WalletBlock {
    pub(crate) fn all_addition_records(&self) -> Vec<AdditionRecord> {
        self.kernel
            .all_addition_records(self.hash)
            .expect("Stored block must have valid guesser fee addition records")
    }

    pub(crate) fn mutator_set_accumulator_after(&self) -> MutatorSetAccumulator {
        let guesser_fees_outputs = self
            .kernel
            .guesser_fee_addition_records(self.hash)
            .expect("Stored block must have valid guesser fee addition records");
        self.kernel
            .body
            .mutator_set_accumulator_after(guesser_fees_outputs)
    }

    /// The number of AOCL leafs prior to the application of this block.
    pub(crate) fn num_aocl_leafs_prior(&self) -> u64 {
        // TODO: Replace this with a call to
        // `BlockBody::num_aocl_leafs_prior` when neptune-core dependency is
        // updated.
        const NUM_GUESSER_OUTPUTS: u64 = 2;
        let num_outputs: u64 = self
            .kernel
            .body
            .transaction_kernel()
            .outputs
            .len()
            .try_into()
            .expect("Can't contain more than u64::MAX outputs");

        let num_guesser_outputs = if self.kernel.header.height.is_genesis() {
            0
        } else {
            NUM_GUESSER_OUTPUTS
        };
        self.mutator_set_accumulator_after().aocl.num_leafs() - num_outputs - num_guesser_outputs
    }
}
