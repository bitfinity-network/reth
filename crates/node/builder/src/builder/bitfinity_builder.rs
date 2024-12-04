use reth_db::database::Database;
use reth_db::database_metrics::{DatabaseMetadata, DatabaseMetrics};
use reth_node_api::NodeTypes;

use crate::bitfinity_launch::{DefaultReadOnlyNodeLauncher, ReadOnlyLaunchNode};
use crate::components::NodeComponentsBuilder;
use crate::{Node, NodeHandle};

use super::{
    NodeAdapter, NodeBuilder, NodeBuilderWithComponents, RethFullAdapter, WithLaunchContext,
};

impl<DB> WithLaunchContext<NodeBuilder<DB>>
where
    DB: Database + DatabaseMetrics + DatabaseMetadata + Clone + Unpin + 'static,
{
    /// Launches a preconfigured [Node]
    ///
    /// This bootstraps the node internals, creates all the components with the given [Node]
    ///
    /// Returns a [`NodeHandle`] that can be used to interact with the node.
    pub async fn launch_readonly_node<N>(
        self,
        node: N,
    ) -> eyre::Result<
        NodeHandle<
            NodeAdapter<
                RethFullAdapter<DB, N>,
                <N::ComponentsBuilder as NodeComponentsBuilder<RethFullAdapter<DB, N>>>::Components,
            >,
        >,
    >
    where
        N: Node<RethFullAdapter<DB, N>>,
    {
        self.node(node).launch_readonly_node().await
    }
}

impl<T, DB, CB> WithLaunchContext<NodeBuilderWithComponents<RethFullAdapter<DB, T>, CB>>
where
    DB: Database + DatabaseMetrics + DatabaseMetadata + Clone + Unpin + 'static,
    T: NodeTypes,
    CB: NodeComponentsBuilder<RethFullAdapter<DB, T>>,
{
    /// Launches the node and returns a handle to it.
    pub async fn launch_readonly_node(
        self,
    ) -> eyre::Result<NodeHandle<NodeAdapter<RethFullAdapter<DB, T>, CB::Components>>> {
        let Self { builder, task_executor } = self;

        let launcher = DefaultReadOnlyNodeLauncher::new(task_executor, builder.config.datadir());

        builder
            .launch_with_fn(|builder| async move { launcher.launch_readonly_node(builder).await })
            .await
    }
}
