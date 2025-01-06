//!
//! Integration tests for the bitfinity node command.
//!

use super::utils::*;
use did::keccak;
use discv5::enr::secp256k1::{Keypair, Secp256k1};
use eth_server::{EthImpl, EthServer};
use ethereum_json_rpc_client::CertifiedResult;
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient};
use jsonrpsee::{
    server::{Server, ServerHandle},
    Methods, RpcModule,
};
use reth::bitfinity_tasks::send_txs::{
    BitfinityTransactionSender, BitfinityTransactionsForwarder, SharedQueue,
    TransactionsPriorityQueue,
};
use reth::{
    args::{DatadirArgs, RpcServerArgs},
    dirs::{DataDirPath, MaybePlatformPath},
};
use reth_consensus::FullConsensus;
use reth_db::DatabaseEnv;
use reth_db::{init_db, test_utils::tempdir_path};
<<<<<<< HEAD
use reth_node_builder::{NodeBuilder, NodeConfig, NodeHandle};
use reth_node_ethereum::EthereumNode;
use reth_primitives::{sign_message, Transaction, TransactionSigned};
||||||| 0e2237228
use reth_node_builder::{NodeBuilder, NodeConfig, NodeHandle};
use reth_node_ethereum::EthereumNode;
=======
use reth_network::NetworkHandle;
use reth_node_api::{FullNodeTypesAdapter, NodeTypesWithDBAdapter};
use reth_node_builder::components::Components;
use reth_node_builder::rpc::RpcAddOns;
use reth_node_builder::{NodeAdapter, NodeBuilder, NodeConfig, NodeHandle};
use reth_node_ethereum::node::EthereumEngineValidatorBuilder;
use reth_node_ethereum::{
    BasicBlockExecutorProvider, EthEvmConfig, EthExecutionStrategyFactory, EthereumNode,
};
use reth_provider::providers::BlockchainProvider;
use reth_rpc::EthApi;
>>>>>>> bitfinity-archive-node
use reth_tasks::TaskManager;
<<<<<<< HEAD
use reth_transaction_pool::test_utils::MockTransaction;
use revm_primitives::{Address, B256, U256};
use std::time::Duration;
||||||| 0e2237228
use revm_primitives::{Address, U256};
=======
use reth_transaction_pool::blobstore::DiskFileBlobStore;
use reth_transaction_pool::{
    CoinbaseTipOrdering, EthPooledTransaction, EthTransactionValidator, Pool,
    TransactionValidationTaskExecutor,
};
use revm_primitives::{hex, Address, U256};
>>>>>>> bitfinity-archive-node
use std::{net::SocketAddr, str::FromStr, sync::Arc};
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;

#[tokio::test]
async fn bitfinity_test_should_start_local_reth_node() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();
    let (reth_client, _reth_node) = start_reth_node(&tasks, None, None, None).await;

    // Act & Assert
    assert!(reth_client.get_chain_id().await.is_ok());
}

#[tokio::test]
async fn bitfinity_test_node_forward_ic_or_eth_get_last_certified_block() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let eth_server = EthImpl::default();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(format!("http://{}", eth_server_address)), None, None).await;

    // Act
    let result = reth_client.get_last_certified_block().await;

    // Assert
    assert!(result.is_ok());

    // Try with `eth_getLastCertifiedBlock` alias
    let result: CertifiedResult<did::Block<did::H256>> = reth_client
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
    let tasks = TaskManager::current();

    let eth_server = EthImpl::default();
    let gas_price = eth_server.gas_price;
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(format!("http://{}", eth_server_address)), None, None).await;

    // Act
    let gas_price_result = reth_client.gas_price().await;

    // Assert
    assert_eq!(gas_price_result.unwrap(), gas_price.into());
}

#[tokio::test]
async fn bitfinity_test_node_forward_max_priority_fee_per_gas_requests() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let eth_server = EthImpl::default();
    let max_priority_fee_per_gas = eth_server.max_priority_fee_per_gas;
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(format!("http://{}", eth_server_address)), None, None).await;

    // Act
    let result = reth_client.max_priority_fee_per_gas().await;

    // Assert
    assert_eq!(result.unwrap(), max_priority_fee_per_gas.into());
}

#[tokio::test]
async fn bitfinity_test_node_forward_eth_get_genesis_balances() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let eth_server = EthImpl::default();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(format!("http://{}", eth_server_address)), None, None).await;

    // Act
    let result: Vec<(did::H160, did::U256)> = reth_client
        .single_request(
            "eth_getGenesisBalances".to_owned(),
            ethereum_json_rpc_client::Params::None,
            ethereum_json_rpc_client::Id::Num(1),
        )
        .await
        .unwrap();

    // Assert
    assert_eq!(result.len(), 3);

    assert_eq!(result[0].0, Address::from_slice(&[1u8; 20]).into());
    assert_eq!(result[0].1, U256::from(10).into());

    assert_eq!(result[1].0, Address::from_slice(&[2u8; 20]).into());
    assert_eq!(result[1].1, U256::from(20).into());

    assert_eq!(result[2].0, Address::from_slice(&[3u8; 20]).into());
    assert_eq!(result[2].1, U256::from(30).into());
}

#[tokio::test]
async fn bitfinity_test_node_forward_ic_get_genesis_balances() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let eth_server = EthImpl::default();
    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(format!("http://{}", eth_server_address)), None, None).await;

    // Act
    let result = reth_client.get_genesis_balances().await.unwrap();

    // Assert
    assert_eq!(result.len(), 3);

    assert_eq!(result[0].0, Address::from_slice(&[1u8; 20]).into());
    assert_eq!(result[0].1, U256::from(10).into());

    assert_eq!(result[1].0, Address::from_slice(&[2u8; 20]).into());
    assert_eq!(result[1].1, U256::from(20).into());

    assert_eq!(result[2].0, Address::from_slice(&[3u8; 20]).into());
    assert_eq!(result[2].1, U256::from(30).into());
}

#[tokio::test]
async fn bitfinity_test_node_forward_send_raw_transaction_requests() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let (tx_sender, mut tx_receiver) = tokio::sync::mpsc::channel(10);
    let eth_server = EthImpl::new(Some(tx_sender));

    let queue = Arc::new(Mutex::new(TransactionsPriorityQueue::new(10)));

    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let bitfinity_evm_url = format!("http://{}", eth_server_address);
    let (reth_client, _reth_node) = start_reth_node(
        &tasks,
        Some(format!("http://{}", eth_server_address)),
        None,
        Some(queue.clone()),
    )
    .await;

    // Create a random transaction
<<<<<<< HEAD
    let tx = transaction_with_gas_price(100);
    let encoded = alloy_rlp::encode(&tx);
    let expected_tx_hash = keccak::keccak_hash(&encoded);
||||||| 0e2237228
    let mut tx = [0u8; 256];
    rand::thread_rng().fill_bytes(&mut tx);
    let expected_tx_hash =
        keccak::keccak_hash(format!("0x{}", reth_primitives::hex::encode(tx)).as_bytes());
=======
    let mut tx = [0u8; 256];
    rand::thread_rng().fill_bytes(&mut tx);
    let expected_tx_hash = keccak::keccak_hash(format!("0x{}", hex::encode(tx)).as_bytes());
>>>>>>> bitfinity-archive-node

    // Act
    let result = reth_client.send_raw_transaction_bytes(&encoded).await.unwrap();

    // Assert
<<<<<<< HEAD
    assert_eq!(result.to_fixed_bytes(), expected_tx_hash.0.to_fixed_bytes());

    let transaction_sending = BitfinityTransactionSender::new(
        queue,
        bitfinity_evm_url,
        Duration::from_millis(200),
        10,
        100,
    );
    transaction_sending.single_execution().await.unwrap();

    let received_txs = consume_received_txs(&mut tx_receiver, 1).await.unwrap();

    assert_eq!(received_txs[0].0, expected_tx_hash.0.to_fixed_bytes());
}

#[tokio::test]
async fn bitfinity_test_node_send_raw_transaction_in_gas_price_order() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let (tx_sender, mut tx_receiver) = tokio::sync::mpsc::channel(10);
    let eth_server = EthImpl::new(Some(tx_sender));

    let queue = Arc::new(Mutex::new(TransactionsPriorityQueue::new(10)));

    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let bitfinity_evm_url = format!("http://{}", eth_server_address);
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(bitfinity_evm_url.clone()), None, Some(queue.clone())).await;

    const TXS_NUMBER: usize = 100;

    // Create a random transactions
    let transactions = (1..=TXS_NUMBER)
        .map(|i| alloy_rlp::encode(transaction_with_gas_price(100 * i as u128)))
        .collect::<Vec<_>>();

    // Only highest price transactions should be sent.
    let expected_hashes =
        transactions.iter().rev().take(10).map(|tx| keccak::keccak_hash(tx)).collect::<Vec<_>>();

    // Act
    for tx in &transactions {
        reth_client.send_raw_transaction_bytes(tx).await.unwrap();
    }

    let transaction_sending = BitfinityTransactionSender::new(
        queue.clone(),
        bitfinity_evm_url,
        Duration::from_millis(200),
        10,
        100,
    );
    transaction_sending.single_execution().await.unwrap();

    let received_txs = consume_received_txs(&mut tx_receiver, 10).await.unwrap();

    // Check all queued transactions sent.
    assert!(queue.lock().await.is_empty());

    for expected_hash in expected_hashes.iter().rev() {
        let expected = B256::from(expected_hash.0.to_fixed_bytes());
        assert!(received_txs.contains(&expected));
    }
}

#[tokio::test]
async fn bitfinity_test_node_get_transaction_when_it_is_queued() {
    // Arrange
    let _log = init_logs();
    let tasks = TaskManager::current();

    let eth_server = EthImpl::new(None);

    let queue = Arc::new(Mutex::new(TransactionsPriorityQueue::new(10)));

    let (_server, eth_server_address) =
        mock_eth_server_start(EthServer::into_rpc(eth_server)).await;
    let bitfinity_evm_url = format!("http://{}", eth_server_address);
    let (reth_client, _reth_node) =
        start_reth_node(&tasks, Some(bitfinity_evm_url.clone()), None, Some(queue.clone())).await;

    const TXS_NUMBER: usize = 10;

    // Create a random transactions
    let transactions = (1..=TXS_NUMBER)
        .map(|i| alloy_rlp::encode(transaction_with_gas_price(100 * i as u128)))
        .collect::<Vec<_>>();

    let expected_hashes = transactions.iter().map(|tx| keccak::keccak_hash(tx)).collect::<Vec<_>>();

    // Act
    for (tx, expected_hash) in transactions.iter().zip(expected_hashes.iter()) {
        let hash = reth_client.send_raw_transaction_bytes(tx).await.unwrap();
        assert_eq!(hash.to_fixed_bytes(), expected_hash.0.to_fixed_bytes());
    }

    for hash in &expected_hashes {
        let tx = reth_client.get_transaction_by_hash(hash.0).await.unwrap().unwrap();
        // Transaction in forwarder has NO block number.
        assert!(tx.block_number.is_none());
    }

    let transaction_sending = BitfinityTransactionSender::new(
        queue,
        bitfinity_evm_url,
        Duration::from_millis(200),
        10,
        100,
    );
    transaction_sending.single_execution().await.unwrap();

    for hash in &expected_hashes {
        let tx = reth_client.get_transaction_by_hash(hash.0).await.unwrap().unwrap();
        // Transaction in mock has block number.
        assert!(tx.block_number.is_some());
    }
}

/// Waits until `n` transactions appear in `received_txs` with one second timeout.
/// Returns true if `received_txs` contains at least `n` transactions.
async fn consume_received_txs(received_txs: &mut Receiver<B256>, n: usize) -> Option<Vec<B256>> {
    let wait_future = async {
        let mut txs = Vec::with_capacity(n);
        while txs.len() < n {
            let tx = received_txs.recv().await.unwrap();
            txs.push(tx);
        }
        txs
    };

    let wait_result = tokio::time::timeout(Duration::from_secs(3), wait_future).await;
    wait_result.ok()
}

fn transaction_with_gas_price(gas_price: u128) -> TransactionSigned {
    let mock = MockTransaction::legacy().with_gas_price(gas_price);
    let transaction: Transaction = mock.into();

    sign_tx_with_random_key_pair(transaction)
}

fn sign_tx_with_random_key_pair(tx: Transaction) -> TransactionSigned {
    let secp = Secp256k1::new();
    let key_pair = Keypair::new(&secp, &mut rand::thread_rng());
    sign_tx_with_key_pair(key_pair, tx)
}

fn sign_tx_with_key_pair(key_pair: Keypair, tx: Transaction) -> TransactionSigned {
    let signature =
        sign_message(B256::from_slice(&key_pair.secret_bytes()[..]), tx.signature_hash()).unwrap();
    TransactionSigned::from_transaction_and_signature(tx, signature)
||||||| 0e2237228
    assert_eq!(result.unwrap(), expected_tx_hash.0);
=======
    assert_eq!(result.unwrap(), expected_tx_hash);
>>>>>>> bitfinity-archive-node
}

/// Start a local reth node
async fn start_reth_node(
    tasks: &TaskManager,
    bitfinity_evm_url: Option<String>,
    import_data: Option<ImportData>,
    queue: Option<SharedQueue>,
) -> (
    EthJsonRpcClient<ReqwestClient>,
    NodeHandle<
        NodeAdapter<
            FullNodeTypesAdapter<
                NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
            >,
            Components<
                FullNodeTypesAdapter<
                    NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                    BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
                >,
                Pool<
                    TransactionValidationTaskExecutor<
                        EthTransactionValidator<
                            BlockchainProvider<
                                NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                            >,
                            EthPooledTransaction,
                        >,
                    >,
                    CoinbaseTipOrdering<EthPooledTransaction>,
                    DiskFileBlobStore,
                >,
                EthEvmConfig,
                BasicBlockExecutorProvider<EthExecutionStrategyFactory>,
                Arc<dyn FullConsensus>,
            >,
        >,
        RpcAddOns<
            NodeAdapter<
                FullNodeTypesAdapter<
                    NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                    BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
                >,
                Components<
                    FullNodeTypesAdapter<
                        NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                        BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
                    >,
                    Pool<
                        TransactionValidationTaskExecutor<
                            EthTransactionValidator<
                                BlockchainProvider<
                                    NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                                >,
                                EthPooledTransaction,
                            >,
                        >,
                        CoinbaseTipOrdering<EthPooledTransaction>,
                        DiskFileBlobStore,
                    >,
                    EthEvmConfig,
                    BasicBlockExecutorProvider<EthExecutionStrategyFactory>,
                    Arc<dyn FullConsensus>,
                >,
            >,
            EthApi<
                BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
                Pool<
                    TransactionValidationTaskExecutor<
                        EthTransactionValidator<
                            BlockchainProvider<
                                NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>,
                            >,
                            EthPooledTransaction,
                        >,
                    >,
                    CoinbaseTipOrdering<EthPooledTransaction>,
                    DiskFileBlobStore,
                >,
                NetworkHandle,
                EthEvmConfig,
            >,
            EthereumEngineValidatorBuilder,
        >,
    >,
) {
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

    let node_handle = NodeBuilder::new(node_config)
        .with_database(database)
        .with_launch_context(tasks.executor())
        .node(EthereumNode::default())
        .extend_rpc_modules(|ctx| {
            // Add custom forwarder with transactions priority queue.
            let Some(queue) = queue else { return Ok(()) };
            let forwarder = BitfinityTransactionsForwarder::new(queue);
            ctx.registry.set_eth_raw_transaction_forwarder(Arc::new(forwarder));

            Ok(())
        })
        .launch()
        .await
        .unwrap();

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

/// Eth server mock utils.
pub mod eth_server {
<<<<<<< HEAD
    use alloy_rlp::{Bytes, Decodable};
    use discv5::enr::secp256k1::{Keypair, Secp256k1};
    use ethereum_json_rpc_client::{Block, CertifiedResult, H256};
||||||| 0e2237228

    use alloy_rlp::Bytes;
    use did::keccak;
    use ethereum_json_rpc_client::{Block, CertifiedResult, H256};
=======

    use alloy_rlp::Bytes;
    use did::keccak;
    use ethereum_json_rpc_client::CertifiedResult;
>>>>>>> bitfinity-archive-node
    use jsonrpsee::{core::RpcResult, proc_macros::rpc};
    use reth_primitives::{sign_message, TransactionSigned};
    use reth_rpc_types::{Signature, Transaction};
    use revm_primitives::{hex, Address, B256, U256};
    use tokio::sync::{mpsc::Sender, Mutex};

    #[rpc(server, namespace = "eth")]
    pub trait Eth {
        /// Returns gasPrice
        #[method(name = "gasPrice")]
        async fn gas_price(&self) -> RpcResult<U256>;

        /// Returns maxPriorityFeePerGas
        #[method(name = "maxPriorityFeePerGas")]
        async fn max_priority_fee_per_gas(&self) -> RpcResult<U256>;

        /// Returns sendRawTransaction
        #[method(name = "sendRawTransaction")]
        async fn send_raw_transaction(&self, tx: Bytes) -> RpcResult<B256>;

        /// Returns getTransactionByHash
        #[method(name = "getTransactionByHash")]
        async fn get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<Transaction>>;

        /// Returns getGenesisBalances
        #[method(name = "getGenesisBalances", aliases = ["ic_getGenesisBalances"])]
        async fn get_genesis_balances(&self) -> RpcResult<Vec<(Address, U256)>>;

        /// Returns getLastCertifiedBlock
        #[method(name = "getLastCertifiedBlock", aliases = ["ic_getLastCertifiedBlock"])]
<<<<<<< HEAD
        async fn get_last_certified_block(&self) -> RpcResult<CertifiedResult<Block<H256>>>;
||||||| 0e2237228
        async fn get_last_certified_block(&self) -> RpcResult<CertifiedResult<Block<H256>>>;

=======
        async fn get_last_certified_block(
            &self,
        ) -> RpcResult<CertifiedResult<did::Block<did::H256>>>;
>>>>>>> bitfinity-archive-node
    }

    /// Eth server mock.
    #[derive(Debug)]
    pub struct EthImpl {
        /// Current gas price
        pub gas_price: u128,

        /// Current `max_priority_fee_per_gas`
        pub max_priority_fee_per_gas: u128,

        /// List of received transactions.
        pub received_txs: Mutex<Vec<B256>>,

        /// The mock will send transactions to the sender, if present.
        pub txs_sender: Option<Sender<B256>>,
    }

    impl EthImpl {
        /// New mock instance.
        pub fn new(txs_sender: Option<Sender<B256>>) -> Self {
            Self {
                gas_price: rand::random(),
                max_priority_fee_per_gas: rand::random(),
                received_txs: Mutex::default(),
                txs_sender,
            }
        }
    }

    impl Default for EthImpl {
        fn default() -> Self {
            Self::new(None)
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
            self.received_txs.lock().await.push(hash);
            if let Some(sender) = &self.txs_sender {
                sender.send(hash).await.unwrap();
            }
            Ok(hash)
        }

        async fn get_transaction_by_hash(&self, hash: B256) -> RpcResult<Option<Transaction>> {
            if !self.received_txs.lock().await.contains(&hash) {
                return Ok(None);
            }

            // If tx present, ruturn it with some block number.
            let mut tx = Transaction {
                hash,
                nonce: 42,
                block_hash: Some(B256::random()),
                block_number: Some(42),
                transaction_index: Some(42),
                from: Address::random(),
                to: Some(Address::random()),
                value: Default::default(),
                gas_price: Default::default(),
                gas: Default::default(),
                max_fee_per_gas: Default::default(),
                max_priority_fee_per_gas: Default::default(),
                max_fee_per_blob_gas: Default::default(),
                input: Default::default(),
                signature: Default::default(),
                chain_id: Default::default(),
                blob_versioned_hashes: Default::default(),
                access_list: Default::default(),
                transaction_type: Default::default(),
                other: Default::default(),
            };
            let key_pair = Keypair::new(&Secp256k1::new(), &mut rand::thread_rng());
            let signature =
                sign_message(B256::from_slice(&key_pair.secret_bytes()[..]), tx.hash).unwrap();
            tx.signature = Some(Signature {
                r: signature.r,
                s: signature.s,
                v: U256::from(signature.v(None)),
                y_parity: None,
            });

            Ok(Some(tx))
        }

        async fn get_genesis_balances(&self) -> RpcResult<Vec<(Address, U256)>> {
            Ok(vec![
                (Address::from_slice(&[1u8; 20]), U256::from(10)),
                (Address::from_slice(&[2u8; 20]), U256::from(20)),
                (Address::from_slice(&[3u8; 20]), U256::from(30)),
            ])
        }

<<<<<<< HEAD
        async fn get_last_certified_block(&self) -> RpcResult<CertifiedResult<Block<H256>>> {
||||||| 0e2237228
        async fn get_last_certified_block(&self) -> RpcResult<CertifiedResult<Block<H256>>> {
            Ok(CertifiedResult { 
                data: Default::default(), 
                witness: vec![], 
                certificate: vec![1u8, 3, 11] 
=======
        async fn get_last_certified_block(
            &self,
        ) -> RpcResult<CertifiedResult<did::Block<did::H256>>> {
>>>>>>> bitfinity-archive-node
            Ok(CertifiedResult {
                data: Default::default(),
                witness: vec![],
                certificate: vec![1u8, 3, 11],
            })
        }
    }
}
