use std::{collections::BTreeMap, sync::Arc};

use crate::primitives::ruint::Uint;
use alloy_rlp::{Bytes, Decodable};
use reth_chainspec::ChainSpec;
use reth_primitives::{TransactionSigned, U256};
use reth_rpc::eth::RawTransactionForwarder;
use reth_rpc_eth_types::{EthApiError, EthResult};
use tokio::sync::Mutex;
use tracing::warn;

/// Command to send transactions by timer.
#[derive(Debug)]
pub struct BitfinityTransactionsForwarder {
    queue: Arc<Mutex<TransactionsPriorityQueue>>,
    chain_spec: Arc<ChainSpec>,
}

#[async_trait::async_trait]
impl RawTransactionForwarder for BitfinityTransactionsForwarder {
    async fn forward_raw_transaction(&self, mut raw: &[u8]) -> EthResult<()> {
        let typed_tx = TransactionSigned::decode(&mut raw).map_err(|e| {
            warn!("failed to decode signed transaction in the BitfinityTransactionsForwarder: {e}");
            EthApiError::FailedToDecodeSignedTransaction
        })?;

        let gas_price = typed_tx.effective_gas_price(None);

        self.queue.lock().await.push(Uint::from(gas_price), raw.to_vec().into());

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct TransactionsPriorityQueue(BTreeMap<U256, Bytes>);

impl TransactionsPriorityQueue {
    pub fn push(&mut self, gas_price: U256, tx: Bytes) {
        self.0.insert(gas_price, tx);
    }

    pub fn pop_tx_with_highest_price(&mut self) -> Option<(U256, Bytes)> {
        self.0.pop_last()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
