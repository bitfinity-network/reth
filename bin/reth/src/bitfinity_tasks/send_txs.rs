//! Utils for raw transaction batching.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use crate::primitives::ruint::Uint;
use alloy_rlp::Decodable;
use did::keccak::keccak_hash;
use did::H256;
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient, Id, Params};
use eyre::eyre;
use lightspeed_scheduler::{job::Job, scheduler::Scheduler, JobExecutor};
use reth_node_core::version::SHORT_VERSION;
use reth_primitives::hex;
use reth_primitives::{TransactionSigned, U256};
use reth_rpc::eth::RawTransactionForwarder;
use reth_rpc_eth_types::{EthApiError, EthResult};
use tokio::sync::Mutex;
use tracing::{debug, info, trace, warn};

/// Alias for multithread transactions priority queue.
pub type SharedQueue = Arc<Mutex<TransactionsPriorityQueue>>;

/// Periodically sends transactions from priority queue.
#[derive(Debug, Clone)]
pub struct BitfinityTransactionSender {
    queue: SharedQueue,
    rpc_url: String,
    period: Duration,
    batch_size: usize,
    txs_per_execution_threshold: usize,
}

impl BitfinityTransactionSender {
    /// Creates new instance of the transaction sender.
    pub const fn new(
        queue: SharedQueue,
        rpc_url: String,
        period: Duration,
        batch_size: usize,
        txs_per_execution_threshold: usize,
    ) -> Self {
        Self { queue, rpc_url, period, batch_size, txs_per_execution_threshold }
    }

    /// Schedule the transaction sending job and return a handle to it.
    pub async fn schedule_execution(
        self,
        job_executor: Option<JobExecutor>,
    ) -> eyre::Result<(JobExecutor, tokio::task::JoinHandle<()>)> {
        info!(target: "reth::cli - BitfinityTransactionSender", "reth {} starting", SHORT_VERSION);

        let job_executor = job_executor.unwrap_or_else(JobExecutor::new_with_local_tz);

        // Schedule the import job
        {
            let interval =
                Scheduler::Interval { interval_duration: self.period, execute_at_startup: true };
            job_executor
                .add_job_with_scheduler(
                    interval,
                    Job::new("send transactions", "bitfinity tx sending", None, move || {
                        let tx_sender = self.clone();
                        Box::pin(async move {
                            tx_sender.single_execution().await?;
                            Ok(())
                        })
                    }),
                )
                .await;
        }

        let job_handle = job_executor.run().await?;
        Ok((job_executor, job_handle))
    }

    /// Execute the transaction sending job.
    async fn single_execution(&self) -> eyre::Result<()> {
        let mut total_sent = 0;
        loop {
            let to_send = self.get_transactions_to_send().await;
            let result = match to_send.len() {
                0 => return Ok(()),
                1 => self.send_single_tx(&to_send[0].1).await,
                _ => self.send_txs_batch(&to_send).await,
            };

            if let Err(e) = result {
                warn!("Failed to send transactions to EVM: {e}");
                continue;
            }

            total_sent += to_send.len();
            if total_sent > self.txs_per_execution_threshold {
                return Ok(());
            }
        }
    }

    async fn get_transactions_to_send(&self) -> Vec<(U256, Vec<u8>)> {
        let mut batch = Vec::with_capacity(self.batch_size);
        let mut queue = self.queue.lock().await;
        let txs_to_pop = self.batch_size.max(1); // if batch size is zero, take at least one tx.

        for _ in 0..txs_to_pop {
            let Some(entry) = queue.pop_tx_with_highest_price() else {
                break;
            };

            batch.push(entry);
        }

        batch
    }

    async fn send_single_tx(&self, to_send: &[u8]) -> Result<(), eyre::Error> {
        let client = self.get_client()?;
        let hash = client
            .send_raw_transaction_bytes(to_send)
            .await
            .map_err(|e| eyre!("failed to send single transaction: {e}"))?;

        trace!("Single transaction with hash {hash} sent.");

        Ok(())
    }

    async fn send_txs_batch(&self, to_send: &[(U256, Vec<u8>)]) -> Result<(), eyre::Error> {
        let client = self.get_client()?;

        let params =
            to_send.iter().map(|(_, raw)| (Params::Array(vec![hex::encode(raw).into()]), Id::Null));
        let max_batch_size = usize::MAX;
        let hashes = client
            .batch_request::<H256>("eth_sendRawTransaction".into(), params, max_batch_size)
            .await
            .map_err(|e| eyre!("failed to send single transaction: {e}"))?;

        trace!("Raw transactions batch sent. Hashes: {hashes:?}");

        Ok(())
    }

    fn get_client(&self) -> eyre::Result<EthJsonRpcClient<ReqwestClient>> {
        let client = EthJsonRpcClient::new(ReqwestClient::new(self.rpc_url.clone()));

        Ok(client)
    }
}

/// Forwarder to push transactions to the priority queue.
#[derive(Debug)]
pub struct BitfinityTransactionsForwarder {
    queue: SharedQueue,
}

impl BitfinityTransactionsForwarder {
    /// Creates new forwarder with the given parameters.
    pub const fn new(queue: SharedQueue) -> Self {
        Self { queue }
    }
}

#[async_trait::async_trait]
impl RawTransactionForwarder for BitfinityTransactionsForwarder {
    async fn forward_raw_transaction(&self, raw: &[u8]) -> EthResult<()> {
        let typed_tx = TransactionSigned::decode(&mut (&raw[..])).map_err(|e| {
            warn!("Failed to decode signed transaction in the BitfinityTransactionsForwarder: {e}");
            EthApiError::FailedToDecodeSignedTransaction
        })?;

        debug!("Pushing tx with hash {} to priority queue", typed_tx.hash);
        let gas_price = typed_tx.effective_gas_price(None);

        self.queue.lock().await.push(Uint::from(gas_price), raw.to_vec());

        Ok(())
    }
}

/// Priority queue to get transactions sorted by gas price.
#[derive(Debug)]
pub struct TransactionsPriorityQueue {
    queue: BTreeMap<TxKey, Vec<u8>>,
    size_limit: usize,
}

impl TransactionsPriorityQueue {
    /// Creates new instance of the queue with the given limil.
    pub fn new(size_limit: usize) -> Self {
        Self { queue: BTreeMap::default(), size_limit }
    }

    /// Adds the tx with the given gas price.
    pub fn push(&mut self, gas_price: U256, tx: Vec<u8>) {
        let key = TxKey { gas_price, hash: keccak_hash(&tx) };
        self.queue.insert(key, tx);

        if self.len() > self.size_limit {
            self.queue.pop_first();
        }
    }

    /// Returns tx with highest gas price, if present.
    pub fn pop_tx_with_highest_price(&mut self) -> Option<(U256, Vec<u8>)> {
        let entry = self.queue.pop_last();
        entry.map(|(key, tx)| (key.gas_price, tx))
    }

    /// Number of transactions in the queue.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Change size limit of the queue.
    pub fn set_size_limit(&mut self, new_limit: usize) {
        self.size_limit = new_limit;

        while self.len() > self.size_limit {
            self.queue.pop_first();
        }
    }

    /// Returns true if length == 0.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// This struct will sort transactions by gas price,
/// but if it is equal, the key will still be different due to hash difference.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TxKey {
    gas_price: U256,
    hash: H256,
}

#[cfg(test)]
mod tests {
    use super::TransactionsPriorityQueue;
    use crate::primitives::U256;
    use reth_primitives::TransactionSigned;
    use reth_transaction_pool::test_utils::MockTransaction;
    use reth_transaction_pool::PoolTransaction;

    #[test]
    fn test_pop_order() {
        let mut queue = TransactionsPriorityQueue::new(10);
        let tx1 = transaction_with_gas_price(100);
        let tx2 = transaction_with_gas_price(300);
        let tx3 = transaction_with_gas_price(200);

        let tx1_bytes = alloy_rlp::encode(&tx1);
        let tx2_bytes = alloy_rlp::encode(&tx2);
        let tx3_bytes = alloy_rlp::encode(&tx3);

        queue.push(U256::from(tx1.effective_gas_price(None)), tx1_bytes.clone());
        queue.push(U256::from(tx2.effective_gas_price(None)), tx2_bytes.clone());
        queue.push(U256::from(tx3.effective_gas_price(None)), tx3_bytes.clone());

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

        queue.push(U256::from(tx1.effective_gas_price(None)), tx1_bytes);
        queue.push(U256::from(tx2.effective_gas_price(None)), tx2_bytes.clone());
        queue.push(U256::from(tx3.effective_gas_price(None)), tx3_bytes.clone());

        let expected_order = [tx2_bytes, tx3_bytes];
        for expected_tx in expected_order {
            let popped_tx = queue.pop_tx_with_highest_price().unwrap().1;
            assert_eq!(popped_tx, expected_tx);
        }

        assert!(queue.is_empty())
    }

    fn transaction_with_gas_price(gas_price: u128) -> TransactionSigned {
        let tx = MockTransaction::legacy().with_gas_price(gas_price);

        TransactionSigned {
            hash: *tx.hash(),
            signature: Default::default(),
            transaction: tx.into(),
        }
    }
}
