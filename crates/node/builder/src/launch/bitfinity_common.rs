//! Helper types that can be used by launchers.

use crate::{
    components::{NodeComponents, NodeComponentsBuilder},
    hooks::OnComponentInitializedHook,
    BuilderContext, NodeAdapter,
};
use backon::{ConstantBuilder, Retryable};
use eyre::Context;
use rayon::ThreadPoolBuilder;
use reth_auto_seal_consensus::MiningMode;
use reth_beacon_consensus::EthBeaconConsensus;
use reth_blockchain_tree::{
    noop::NoopBlockchainTree, BlockchainTree, BlockchainTreeConfig, ShareableBlockchainTree,
    TreeExternals,
};
use reth_chainspec::{Chain, ChainSpec};
use reth_config::{config::EtlConfig, PruneConfig};
use reth_consensus::Consensus;
use reth_db_api::{database::Database, database_metrics::DatabaseMetrics};
use reth_db_common::init::{init_genesis, InitDatabaseError};
use reth_downloaders::{bodies::noop::NoopBodiesDownloader, headers::noop::NoopHeaderDownloader};
use reth_evm::noop::NoopBlockExecutorProvider;
use reth_network_p2p::headers::client::HeadersClient;
use reth_node_api::FullNodeTypes;
use reth_node_core::{
    dirs::{ChainPath, DataDirPath},
    node_config::NodeConfig,
};
use reth_primitives::{BlockNumber, Head, B256};
use reth_provider::{
    providers::{BlockchainProvider, StaticFileProvider},
    CanonStateNotificationSender, ProviderFactory, StaticFileProviderFactory,
};
use reth_prune::{PruneModes, PrunerBuilder};
use reth_rpc_builder::config::RethRpcServerConfig;
use reth_rpc_layer::JwtSecret;
use reth_stages::{sets::DefaultStages, MetricEvent, Pipeline, PipelineTarget};
use reth_static_file::StaticFileProducer;
use reth_tasks::TaskExecutor;
use reth_tracing::tracing::{debug, error, info, warn};
use std::{marker::PhantomData, sync::Arc, thread::available_parallelism};
use tokio::sync::{
    mpsc::{unbounded_channel, Receiver, UnboundedSender},
    oneshot, watch,
};

use super::common::{Attached, LaunchContextWith, WithConfigs};

impl<DB> LaunchContextWith<Attached<WithConfigs, DB>>
where
    DB: Database + Clone + 'static,
{
    /// Returns the [`ProviderFactory`] for the attached storage after executing a consistent check
    /// between the database and static files. **It may execute a pipeline unwind if it fails this
    /// check.**
    pub async fn create_readonly_provider_factory(&self) -> eyre::Result<ProviderFactory<DB>> {
        let factory = ProviderFactory::new(
            self.right().clone(),
            self.chain_spec(),
            StaticFileProvider::read_only(self.data_dir().static_files())?,
        )
        .with_static_files_metrics();

        let has_receipt_pruning =
            self.toml_config().prune.as_ref().map_or(false, |a| a.has_receipts_pruning());

        info!(target: "reth::cli", "Verifying storage consistency.");

        // Check for consistency between database and static files. If it fails, it unwinds to
        // the first block that's consistent between database and static files.
        if let Some(unwind_target) = factory
            .static_file_provider()
            .check_consistency(&factory.provider()?, has_receipt_pruning)?
        {
            // Highly unlikely to happen, and given its destructive nature, it's better to panic
            // instead.
            assert_ne!(unwind_target, PipelineTarget::Unwind(0), "A static file <> database inconsistency was found that would trigger an unwind to block 0");

            info!(target: "reth::cli", unwind_target = %unwind_target, "Executing an unwind after a failed storage consistency check.");

            let (_tip_tx, tip_rx) = watch::channel(B256::ZERO);

            // Builds an unwind-only pipeline
            let pipeline = Pipeline::builder()
                .add_stages(DefaultStages::new(
                    factory.clone(),
                    tip_rx,
                    Arc::new(EthBeaconConsensus::new(self.chain_spec())),
                    NoopHeaderDownloader::default(),
                    NoopBodiesDownloader::default(),
                    NoopBlockExecutorProvider::default(),
                    self.toml_config().stages.clone(),
                    self.prune_modes().unwrap_or_default(),
                ))
                .build(
                    factory.clone(),
                    StaticFileProducer::new(
                        factory.clone(),
                        self.prune_modes().unwrap_or_default(),
                    ),
                );

            // Unwinds to block
            let (tx, rx) = oneshot::channel();

            // Pipeline should be run as blocking and panic if it fails.
            self.task_executor().spawn_critical_blocking(
                "pipeline task",
                Box::pin(async move {
                    let (_, result) = pipeline.run_as_fut(Some(unwind_target)).await;
                    let _ = tx.send(result);
                }),
            );
            rx.await??;
        }

        Ok(factory)
    }

    /// Creates a new [`ProviderFactory`] and attaches it to the launch context.
    pub async fn with_readonly_provider_factory(
        self,
    ) -> eyre::Result<LaunchContextWith<Attached<WithConfigs, ProviderFactory<DB>>>> {
        let factory = self.create_readonly_provider_factory().await?;
        let ctx = LaunchContextWith {
            inner: self.inner,
            attachment: self.attachment.map_right(|_| factory),
        };

        Ok(ctx)
    }
}
