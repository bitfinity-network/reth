use std::{collections::BTreeSet, sync::Arc};

use alloy_consensus::Transaction;
use alloy_primitives::Uint;
use alloy_rlp::Decodable;
use reth_primitives::TransactionSigned;
use reth_rpc_eth_types::{EthApiError, EthResult};
use revm_primitives::{HashMap, B256, U256};
use tokio::sync::Mutex;
use tracing::{debug, warn};

/// Alias for multithread transactions priority queue.
pub type SharedQueue = Arc<Mutex<TransactionsPriorityQueue>>;

/// Forwarder to push transactions to the priority queue.
#[derive(Debug, Clone)]
pub struct BitfinityTransactionsForwarder {
    queue: SharedQueue,
}

impl BitfinityTransactionsForwarder {
    /// Creates new forwarder with the given parameters.
    pub const fn new(queue: SharedQueue) -> Self {
        Self { queue }
    }

    pub async fn forward_raw_transaction(&self, raw: &[u8]) -> EthResult<()> {
        let typed_tx = TransactionSigned::decode(&mut (&raw[..])).map_err(|e| {
            warn!("Failed to decode signed transaction in the BitfinityTransactionsForwarder: {e}");
            EthApiError::FailedToDecodeSignedTransaction
        })?;

        debug!("Pushing tx with hash {:?} to priority queue", typed_tx.hash);
        let gas_price = typed_tx.effective_gas_price(None);

        self.queue.lock().await.push(typed_tx.hash(), Uint::from(gas_price), raw.to_vec());

        Ok(())
    }

    pub async fn get_transaction_by_hash(&self, hash: B256) -> Option<Vec<u8>> {
        self.queue.lock().await.get(&hash)
    }
}

/// Priority queue to get transactions sorted by gas price.
#[derive(Debug)]
pub struct TransactionsPriorityQueue {
    priority: BTreeSet<TxKey>,
    transactions: HashMap<B256, Vec<u8>>,
    size_limit: usize,
}

impl TransactionsPriorityQueue {
    /// Creates new instance of the queue with the given limil.
    pub fn new(size_limit: usize) -> Self {
        Self { priority: BTreeSet::default(), transactions: HashMap::default(), size_limit }
    }

    /// Adds the tx with the given gas price.
    pub fn push(&mut self, hash: B256, gas_price: U256, tx: Vec<u8>) {
        let key = TxKey { gas_price, hash };
        self.priority.insert(key);
        self.transactions.insert(hash, tx);

        if self.len() > self.size_limit {
            self.pop_tx_with_lowest_price();
        }
    }

    /// Returns raw transaction if it is present in the queue.
    pub fn get(&self, hash: &B256) -> Option<Vec<u8>> {
        self.transactions.get(hash).cloned()
    }

    /// Returns tx with highest gas price, if present.
    pub fn pop_tx_with_highest_price(&mut self) -> Option<(U256, Vec<u8>)> {
        let tx_key = self.priority.pop_last()?;
        let Some(tx) = self.transactions.remove(&tx_key.hash) else {
            warn!("Transaction key present in priority queue, but not found in transactions map.");
            return None;
        };

        Some((tx_key.gas_price, tx))
    }

    /// Returns tx with lowest gas price, if present.
    pub fn pop_tx_with_lowest_price(&mut self) -> Option<(U256, Vec<u8>)> {
        let tx_key = self.priority.pop_first()?;
        let Some(tx) = self.transactions.remove(&tx_key.hash) else {
            warn!("Transaction key present in priority queue, but not found in transactions map.");
            return None;
        };

        Some((tx_key.gas_price, tx))
    }

    /// Number of transactions in the queue.
    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    /// Change size limit of the queue.
    pub fn set_size_limit(&mut self, new_limit: usize) {
        self.size_limit = new_limit;

        while self.len() > self.size_limit {
            self.pop_tx_with_lowest_price();
        }
    }

    /// Returns true if length == 0.
    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }
}

/// This struct will sort transactions by gas price,
/// but if it is equal, the key will still be different due to hash difference.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TxKey {
    gas_price: U256,
    hash: B256,
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::TransactionsPriorityQueue;
    use alloy_consensus::Transaction;
    use reth_primitives::TransactionSigned;
    use reth_transaction_pool::test_utils::MockTransaction;
    use reth_transaction_pool::PoolTransaction;
    use revm_primitives::{PrimitiveSignature, U256};

    #[test]
    fn test_pop_order() {
        let mut queue = TransactionsPriorityQueue::new(10);
        let tx1 = transaction_with_gas_price(100);
        let tx2 = transaction_with_gas_price(300);
        let tx3 = transaction_with_gas_price(200);

        let tx1_bytes = alloy_rlp::encode(&tx1);
        let tx2_bytes = alloy_rlp::encode(&tx2);
        let tx3_bytes = alloy_rlp::encode(&tx3);

        queue.push(tx1.hash(), U256::from(tx1.effective_gas_price(None)), tx1_bytes.clone());
        queue.push(tx2.hash(), U256::from(tx2.effective_gas_price(None)), tx2_bytes.clone());
        queue.push(tx3.hash(), U256::from(tx3.effective_gas_price(None)), tx3_bytes.clone());

        let expected_order = [tx2_bytes, tx3_bytes, tx1_bytes];
        for expected_tx in expected_order {
            let popped_tx = queue.pop_tx_with_highest_price().unwrap().1;
            assert_eq!(popped_tx, expected_tx);
        }

        assert!(queue.is_empty())
    }

    #[test]
    fn test_size_limit_should_shrink_tx_with_lowest_price() {
        let mut queue = TransactionsPriorityQueue::new(2);
        let tx1 = transaction_with_gas_price(100);
        let tx2 = transaction_with_gas_price(300);
        let tx3 = transaction_with_gas_price(200);

        let tx1_bytes = alloy_rlp::encode(&tx1);
        let tx2_bytes = alloy_rlp::encode(&tx2);
        let tx3_bytes = alloy_rlp::encode(&tx3);

        queue.push(tx1.hash(), U256::from(tx1.effective_gas_price(None)), tx1_bytes);
        queue.push(tx2.hash(), U256::from(tx2.effective_gas_price(None)), tx2_bytes.clone());
        queue.push(tx3.hash(), U256::from(tx3.effective_gas_price(None)), tx3_bytes.clone());

        let expected_order = [tx2_bytes, tx3_bytes];
        for expected_tx in expected_order {
            let popped_tx = queue.pop_tx_with_highest_price().unwrap().1;
            assert_eq!(popped_tx, expected_tx);
        }

        assert!(queue.is_empty())
    }

    #[test]
    fn test_get_transaction_from_queue() {
        let mut queue = TransactionsPriorityQueue::new(100);
        let tx1 = transaction_with_gas_price(100);
        let tx2 = transaction_with_gas_price(300);
        let tx3 = transaction_with_gas_price(200);

        let tx1_bytes = alloy_rlp::encode(&tx1);
        let tx2_bytes = alloy_rlp::encode(&tx2);
        let tx3_bytes = alloy_rlp::encode(&tx3);

        queue.push(tx1.hash(), U256::from(tx1.effective_gas_price(None)), tx1_bytes);
        queue.push(tx2.hash(), U256::from(tx2.effective_gas_price(None)), tx2_bytes);
        queue.push(tx3.hash(), U256::from(tx3.effective_gas_price(None)), tx3_bytes);

        let hashes = [tx1.hash(), tx2.hash(), tx3.hash()];
        for hash in hashes {
            assert!(queue.get(&hash).is_some());
        }

        assert_eq!(queue.len(), 3);

        let tx4 = transaction_with_gas_price(400);
        assert!(queue.get(&tx4.hash()).is_none());
    }

    fn transaction_with_gas_price(gas_price: u128) -> TransactionSigned {
        let tx = MockTransaction::legacy().with_gas_price(gas_price).rng_hash();

        let hash = OnceLock::new();
        hash.get_or_init(|| *tx.hash());

        TransactionSigned {
            hash,
            signature: PrimitiveSignature::test_signature(),
            transaction: tx.into(),
        }
    }
}
