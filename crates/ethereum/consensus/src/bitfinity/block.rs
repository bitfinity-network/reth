use std::iter::Iterator;

use did::H256;

use reth_primitives::{BlockWithSenders, Receipt};

/// It's just [`Block`] with additional functions for validation.
pub struct BitfinityBlock<'a>(&'a BlockWithSenders);

impl<'a> From<&'a BlockWithSenders> for BitfinityBlock<'a> {
    fn from(block: &'a BlockWithSenders) -> Self {
        Self(block)
    }
}

impl<'a> BitfinityBlock<'a> {
    /// Get transaction hashes
    pub fn txs_hash(&self) -> impl Iterator<Item = H256> + '_ {
        self.0.body.iter().map(|tx| H256::from_slice(tx.hash.as_ref())).into_iter()
    }

    /// Get block number
    pub fn number(&self) -> u64 {
        self.0.number
    }

    /// Calculate the block hash
    pub fn hash(&self) -> H256 {
        let hash = self.0.clone().seal_slow().hash();

        H256::from_slice(hash.as_ref())
    }

    /// Get the parent hash
    pub fn state_root(&self) -> H256 {
        let hash = self.0.state_root;

        H256::from_slice(hash.as_ref())
    }

    /// Calculate transaction root for this block
    pub fn transaction_root(&self) -> H256 {
        let calculated_root = reth_primitives::proofs::calculate_transaction_root(&self.0.body);

        H256::from_slice(calculated_root.as_ref())
    }

    /// Calculate and get the receipts root for this block
    pub fn receipts_root(&self, receipts: &[Receipt]) -> H256 {
        let receipts_with_bloom =
            receipts.iter().map(|receipt| receipt.clone().with_bloom()).collect::<Vec<_>>();
        let calculated_root = reth_primitives::proofs::calculate_receipt_root(&receipts_with_bloom);

        H256::from_slice(calculated_root.as_ref())
    }
}
