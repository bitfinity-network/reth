//! Utils for raw transaction batching.

use std::time::Duration;

use did::H256;
use ethereum_json_rpc_client::reqwest::ReqwestClient;
use ethereum_json_rpc_client::{EthJsonRpcClient, Id, Params};
use eyre::eyre;
use futures::future::join_all;
use lightspeed_scheduler::job::Job;
use lightspeed_scheduler::scheduler::Scheduler;
use lightspeed_scheduler::JobExecutor;
use reth_node_core::version::SHORT_VERSION;
use reth_rpc_api::eth::helpers::bitfinity_tx_forwarder::SharedQueue;
use revm_primitives::{hex, U256};
use tracing::{info, trace, warn};

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
    pub async fn single_execution(&self) -> eyre::Result<()> {
        let mut to_send = self.get_transactions_to_send().await;
        let batch_size = self.batch_size.max(1);
        let mut send_futures = vec![];

        loop {
            let last_idx = batch_size.min(to_send.len());
            if last_idx == 0 {
                break;
            }

            let to_send_batch: Vec<_> = to_send.drain(..last_idx).collect();

            let send_future = async move {
                let result = match to_send_batch.len() {
                    0 => return,
                    1 => self.send_single_tx(&to_send_batch[0].1).await,
                    _ => self.send_txs_batch(&to_send_batch).await,
                };

                if let Err(e) = result {
                    warn!("Failed to send transactions to EVM: {e}");
                }
            };
            send_futures.push(send_future);
        }

        join_all(send_futures).await;

        Ok(())
    }

    async fn get_transactions_to_send(&self) -> Vec<(U256, Vec<u8>)> {
        let mut batch = Vec::with_capacity(self.txs_per_execution_threshold);
        let mut queue = self.queue.lock().await;
        let txs_to_pop = self.txs_per_execution_threshold.max(1); // if batch size is zero, take at least one tx.

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
