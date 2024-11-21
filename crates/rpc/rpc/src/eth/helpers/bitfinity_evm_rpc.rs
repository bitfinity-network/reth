//! Implements Bifitnity EVM RPC methods.
//!
use std::sync::Arc;

use alloy_primitives::Uint;
use reth_primitives::TransactionSigned;
use reth_provider::ChainSpecProvider;
use reth_rpc_eth_api::{helpers::bitfinity_evm_rpc::BitfinityEvmRpc, RawTransactionForwarder};

use crate::EthApi;

impl<Provider, Pool, Network, EvmConfig> BitfinityEvmRpc
    for EthApi<Provider, Pool, Network, EvmConfig>
where
    Provider: ChainSpecProvider,
{
    fn raw_tx_forwarder(&self) -> Option<Arc<dyn RawTransactionForwarder>> {
        self.inner.raw_tx_forwarder()
    }

    fn chain_spec(&self) -> Arc<reth_chainspec::ChainSpec> {
        self.inner.provider().chain_spec()
    }
}
