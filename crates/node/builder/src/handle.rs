use crate::node::FullNode;
use reth_node_api::{FullNodeComponents, FullNodeTypes};
use reth_node_core::{args::BitfinityImportArgs, exit::NodeExitFuture};
use reth_provider::ProviderFactory;
use std::fmt;

/// A Handle to the launched node.
#[must_use = "Needs to await the node exit future"]
pub struct NodeHandle<Node: FullNodeComponents> {
    /// All node components.
    pub node: FullNode<Node>,
    /// The exit future of the node.
    pub node_exit_future: NodeExitFuture,
    /// Bitfinity import command.
    pub bitfinity_import: Option<(ProviderFactory<<Node as FullNodeTypes>::DB>, BitfinityImportArgs)>,
}

impl<Node: FullNodeComponents> NodeHandle<Node> {
    /// Waits for the node to exit, if it was configured to exit.
    pub async fn wait_for_node_exit(self) -> eyre::Result<()> {
        self.node_exit_future.await
    }
}

impl<Node: FullNodeComponents> fmt::Debug for NodeHandle<Node> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeHandle")
            .field("node", &"...")
            .field("node_exit_future", &self.node_exit_future)
            .finish()
    }
}
