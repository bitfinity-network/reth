//!
//! Integration tests for the bitfinity node command.
//!

use super::utils::*;
use did::keccak;
use eth_server::{EthImpl, EthServer};
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient};
use ethereum_json_rpc_client::{Block, CertifiedResult, H256};
use jsonrpsee::{
    server::{Server, ServerHandle},
    Methods, RpcModule,
};
use reth::commands::bitfinity_send_raw_txs::{
    BitfinityTransactionSender, BitfinityTransactionsForwarder, TransactionsPriorityQueue,
};
use reth::{
    args::{DatadirArgs, RpcServerArgs},
    dirs::{DataDirPath, MaybePlatformPath},
};
use reth_consensus::Consensus;
use reth_db::{init_db, test_utils::tempdir_path};
use reth_node_builder::{NodeBuilder, NodeConfig, NodeHandle};
use reth_node_ethereum::EthereumNode;
use reth_primitives::{Transaction, TransactionSigned};
use reth_tasks::TaskManager;
use reth_transaction_pool::test_utils::MockTransaction;
use revm_primitives::{hex, Address, U256};
use std::time::Duration;
use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::sync::Mutex;

#[tokio::test]
async fn bitfinity_test_should_start_local_reth_node() {
    // Arrange
    let _log = init_logs();
    let (reth_client, _reth_node) = start_reth_node(None, None).await;

    // Act & Assert
    assert!(reth_client.get_chain_id().await.is_ok());
}

#[tokio::test]
async fn bitfinity_test_node_forward_ic_or_eth_get_last_certified_block() {
    // Arrange
    let _log = init_logs();

    let eth_server = EthImpl::new();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

    // Act
    let result = reth_client.get_last_certified_block().await;

    // Assert
    assert!(result.is_ok());

    // Try with `eth_getLastCertifiedBlock` alias
    let result: CertifiedResult<Block<H256>> = reth_client
        .single_request(
            "eth_getLastCertifiedBlock".to_owned(),
            ethereum_json_rpc_client::Params::None,
            ethereum_json_rpc_client::Id::Num(1),
        )
        .await
        .unwrap();

    assert_eq!(result.certificate, vec![1u8, 3, 11]);
}

#[tokio::test]
async fn bitfinity_test_node_forward_get_gas_price_requests() {
    // Arrange
    let _log = init_logs();

    let eth_server = EthImpl::new();
    let gas_price = eth_server.gas_price;
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

    // Act
    let gas_price_result = reth_client.gas_price().await;

    // Assert
    assert_eq!(gas_price_result.unwrap().as_u128(), gas_price);
}

#[tokio::test]
async fn bitfinity_test_node_forward_max_priority_fee_per_gas_requests() {
    // Arrange
    let _log = init_logs();

    let eth_server = EthImpl::new();
    let max_priority_fee_per_gas = eth_server.max_priority_fee_per_gas;
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

    // Act
    let result = reth_client.max_priority_fee_per_gas().await;

    // Assert
    assert_eq!(result.unwrap().as_u128(), max_priority_fee_per_gas);
}

#[tokio::test]
async fn bitfinity_test_node_forward_eth_get_genesis_balances() {
    // Arrange
    let _log = init_logs();

    let eth_server = EthImpl::new();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

    // Act
    let result: Vec<(ethereum_json_rpc_client::H160, ethereum_json_rpc_client::U256)> = reth_client
        .single_request(
            "eth_getGenesisBalances".to_owned(),
            ethereum_json_rpc_client::Params::None,
            ethereum_json_rpc_client::Id::Num(1),
        )
        .await
        .unwrap();

    // Assert
    assert_eq!(result.len(), 3);

    assert_eq!(did::H160::from(result[0].0), Address::from_slice(&[1u8; 20]).into());
    assert_eq!(did::U256::from(result[0].1), U256::from(10).into());

    assert_eq!(did::H160::from(result[1].0), Address::from_slice(&[2u8; 20]).into());
    assert_eq!(did::U256::from(result[1].1), U256::from(20).into());

    assert_eq!(did::H160::from(result[2].0), Address::from_slice(&[3u8; 20]).into());
    assert_eq!(did::U256::from(result[2].1), U256::from(30).into());
}

#[tokio::test]
async fn bitfinity_test_node_forward_ic_get_genesis_balances() {
    // Arrange
    let _log = init_logs();

    let eth_server = EthImpl::new();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

    // Act
    let result = reth_client.get_genesis_balances().await.unwrap();

    // Assert
    assert_eq!(result.len(), 3);

    assert_eq!(did::H160::from(result[0].0), Address::from_slice(&[1u8; 20]).into());
    assert_eq!(did::U256::from(result[0].1), U256::from(10).into());

    assert_eq!(did::H160::from(result[1].0), Address::from_slice(&[2u8; 20]).into());
    assert_eq!(did::U256::from(result[1].1), U256::from(20).into());

    assert_eq!(did::H160::from(result[2].0), Address::from_slice(&[3u8; 20]).into());
    assert_eq!(did::U256::from(result[2].1), U256::from(30).into());
}

#[tokio::test]
async fn bitfinity_test_node_forward_send_raw_transaction_requests() {
    // Arrange
    let _log = init_logs();

    let eth_server = EthImpl::new();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

    // Create a random transaction
    let tx = transaction_with_gas_price(100);
    // let codec = TransactionSigned::decode(&mut alloy_rlp::encode(&tx).as_ref()).unwrap();
    let encoded = alloy_rlp::encode(&tx);
    let expected_tx_hash = keccak::keccak_hash(&encoded);

    // Act
    let result = reth_client.send_raw_transaction_bytes(&encoded).await.unwrap();

    // Assert
    assert_eq!(result.to_fixed_bytes(), expected_tx_hash.0.to_fixed_bytes());

    tokio::time::sleep(Duration::from_secs(3)).await;

    assert_eq!(eth_server::TXS_ORDER.lock().await[0].0, expected_tx_hash.0.to_fixed_bytes());
}

// #[tokio::test]
// async fn bitfinity_test_node_send_raw_transaction_in_gas_price_order() {
//     // Arrange
//     let _log = init_logs();

//     let eth_server = EthImpl::new();
//     let (_server, eth_server_address) =
//         mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
//     let (reth_client, _reth_node) =
//         start_reth_node(Some(format!("http://{}", eth_server_address)), None).await;

//     // Create a random transaction
//     let mut tx = [0u8; 256];
//     rand::thread_rng().fill_bytes(&mut tx);
//     let expected_tx_hash =
//         keccak::keccak_hash(format!("0x{}", reth_primitives::hex::encode(tx)).as_bytes());

//     // Act
//     let result = reth_client.send_raw_transaction_bytes(&tx).await;

//     // Assert
//     assert_eq!(result.unwrap(), expected_tx_hash.0);
// }

fn transaction_with_gas_price(gas_price: u128) -> TransactionSigned {
    let mock = MockTransaction::legacy().with_gas_price(gas_price);
    let transaction: Transaction = mock.into();

    TransactionSigned { hash: Default::default(), signature: Default::default(), transaction }
}

/// Start a local reth node
async fn start_reth_node(
    bitfinity_evm_url: Option<String>,
    import_data: Option<ImportData>,
) -> (
    EthJsonRpcClient<ReqwestClient>,
    NodeHandle<
        reth_node_builder::NodeAdapter<
            reth_node_api::FullNodeTypesAdapter<
                EthereumNode,
                Arc<reth_db::DatabaseEnv>,
                reth_provider::providers::BlockchainProvider<Arc<reth_db::DatabaseEnv>>,
            >,
            reth_node_builder::components::Components<
                reth_node_api::FullNodeTypesAdapter<
                    EthereumNode,
                    Arc<reth_db::DatabaseEnv>,
                    reth_provider::providers::BlockchainProvider<Arc<reth_db::DatabaseEnv>>,
                >,
                reth_transaction_pool::Pool<
                    reth_transaction_pool::TransactionValidationTaskExecutor<
                        reth_transaction_pool::EthTransactionValidator<
                            reth_provider::providers::BlockchainProvider<Arc<reth_db::DatabaseEnv>>,
                            reth_transaction_pool::EthPooledTransaction,
                        >,
                    >,
                    reth_transaction_pool::CoinbaseTipOrdering<
                        reth_transaction_pool::EthPooledTransaction,
                    >,
                    reth_transaction_pool::blobstore::DiskFileBlobStore,
                >,
                reth_node_ethereum::EthEvmConfig,
                reth_node_ethereum::EthExecutorProvider,
                Arc<dyn Consensus>,
            >,
        >,
    >,
) {
    let tasks = TaskManager::current();

    // create node config
    let mut node_config =
        NodeConfig::test().dev().with_rpc(RpcServerArgs::default().with_http()).with_unused_ports();
    node_config.dev.dev = false;

    let mut chain = node_config.chain.as_ref().clone();
    chain.bitfinity_evm_url = bitfinity_evm_url.clone();
    let mut node_config = node_config.with_chain(chain);

    let database = if let Some(import_data) = import_data {
        let data_dir = MaybePlatformPath::<DataDirPath>::from_str(
            import_data.data_dir.data_dir().to_str().unwrap(),
        )
        .unwrap();
        let mut data_dir_args = node_config.datadir.clone();
        data_dir_args.datadir = data_dir;
        data_dir_args.static_files_path = Some(import_data.data_dir.static_files());
        node_config = node_config.with_datadir_args(data_dir_args);
        node_config = node_config.with_chain(import_data.chain.clone());
        import_data.database
    } else {
        let path = MaybePlatformPath::<DataDirPath>::from(tempdir_path());
        node_config = node_config
            .with_datadir_args(DatadirArgs { datadir: path.clone(), ..Default::default() });
        let data_dir =
            path.unwrap_or_chain_default(node_config.chain.chain, node_config.datadir.clone());
        Arc::new(init_db(data_dir.db(), Default::default()).unwrap())
    };

    let queue = Arc::new(Mutex::new(TransactionsPriorityQueue::new(1000)));
    let queue_clone = Arc::clone(&queue);

    let node_handle = NodeBuilder::new(node_config)
        .with_database(database)
        .with_launch_context(tasks.executor())
        .node(EthereumNode::default())
        .extend_rpc_modules(|ctx| {
            // Add custom forwarder with transactions priority queue.
            let forwarder = BitfinityTransactionsForwarder::new(queue);
            ctx.registry.set_eth_raw_transaction_forwarder(Arc::new(forwarder));

            Ok(())
        })
        .launch()
        .await
        .unwrap();

    let transaction_sending = BitfinityTransactionSender::new(
        queue_clone,
        bitfinity_evm_url.unwrap_or_default(),
        Duration::from_millis(100),
        10,
        100,
    );
    let _sending_handle = transaction_sending.schedule_execution(None).await.unwrap();

    let reth_address = node_handle.node.rpc_server_handle().http_local_addr().unwrap();
    let addr_string = format!("http://{}", reth_address);
    let client: EthJsonRpcClient<ReqwestClient> =
        EthJsonRpcClient::new(ReqwestClient::new(addr_string));

    (client, node_handle)
}

/// Start a local Eth server.
/// Reth requests will be forwarded to this server
async fn mock_eth_server_start(methods: impl Into<Methods>) -> (ServerHandle, SocketAddr) {
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let server = Server::builder().build(addr).await.unwrap();

    let mut module = RpcModule::new(());
    module.merge(methods).unwrap();

    let server_address = server.local_addr().unwrap();
    let handle = server.start(module);

    (handle, server_address)
}

pub mod eth_server {
    use alloy_rlp::{Bytes, Decodable};
    use ethereum_json_rpc_client::{Block, CertifiedResult, H256};
    use jsonrpsee::{core::RpcResult, proc_macros::rpc};
    use reth_primitives::TransactionSigned;
    use revm_primitives::{hex, Address, B256, U256};
    use tokio::sync::Mutex;

    #[rpc(server, namespace = "eth")]
    pub trait Eth {
        #[method(name = "gasPrice")]
        async fn gas_price(&self) -> RpcResult<U256>;

        #[method(name = "maxPriorityFeePerGas")]
        async fn max_priority_fee_per_gas(&self) -> RpcResult<U256>;

        #[method(name = "sendRawTransaction")]
        async fn send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256>;

        #[method(name = "getGenesisBalances", aliases = ["ic_getGenesisBalances"])]
        async fn get_genesis_balances(&self) -> RpcResult<Vec<(Address, U256)>>;

        #[method(name = "getLastCertifiedBlock", aliases = ["ic_getLastCertifiedBlock"])]
        async fn get_last_certified_block(&self) -> RpcResult<CertifiedResult<Block<H256>>>;
    }

    pub static TXS_ORDER: Mutex<Vec<B256>> = Mutex::const_new(Vec::new());

    #[derive(Debug)]
    pub struct EthImpl {
        pub gas_price: u128,
        pub max_priority_fee_per_gas: u128,
    }

    impl EthImpl {
        pub fn new() -> Self {
            Self { gas_price: rand::random(), max_priority_fee_per_gas: rand::random() }
        }
    }

    impl Default for EthImpl {
        fn default() -> Self {
            Self::new()
        }
    }

    #[async_trait::async_trait]
    impl EthServer for EthImpl {
        async fn gas_price(&self) -> RpcResult<U256> {
            Ok(U256::from(self.gas_price))
        }

        async fn max_priority_fee_per_gas(&self) -> RpcResult<U256> {
            Ok(U256::from(self.max_priority_fee_per_gas))
        }

        async fn send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256> {
            let decoded = hex::decode(&tx).unwrap();
            let tx = TransactionSigned::decode(&mut decoded.as_ref()).unwrap();
            let hash = tx.hash();
            TXS_ORDER.lock().await.push(hash);
            Ok(hash)
        }

        async fn get_genesis_balances(&self) -> RpcResult<Vec<(Address, U256)>> {
            Ok(vec![
                (Address::from_slice(&[1u8; 20]), U256::from(10)),
                (Address::from_slice(&[2u8; 20]), U256::from(20)),
                (Address::from_slice(&[3u8; 20]), U256::from(30)),
            ])
        }

        async fn get_last_certified_block(&self) -> RpcResult<CertifiedResult<Block<H256>>> {
            Ok(CertifiedResult {
                data: Default::default(),
                witness: vec![],
                certificate: vec![1u8, 3, 11],
            })
        }
    }
}
