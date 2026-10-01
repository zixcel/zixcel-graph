//! Property graph transactions over the independent Zixcel revision foundation.

#[cfg(feature = "graph")]
mod backend;
#[cfg(feature = "graph")]
mod error;
#[cfg(feature = "graph")]
mod external_port;
#[cfg(feature = "graph")]
pub use external_port::GraphExternalRetention;
#[cfg(feature = "graph")]
mod graph;
#[cfg(feature = "graph")]
mod model;
#[cfg(feature = "graph")]
mod publication;
#[cfg(feature = "graph")]
mod query;
#[cfg(all(feature = "graph", feature = "redb"))]
mod startup;
#[cfg(feature = "graph")]
mod state;
#[cfg(all(feature = "graph", feature = "redb"))]
pub use startup::{StartedGraph, StoragePhase};
#[cfg(feature = "graph")]
mod transaction;
#[cfg(feature = "graph")]
mod validation;

#[cfg(feature = "graph")]
pub use error::GraphError;
#[cfg(feature = "graph")]
pub use graph::Graph;
#[cfg(feature = "graph")]
pub use model::{Direction, Edge, EdgeDirection, GraphMetadata, GraphSpace, Node, PropertyValue};
#[cfg(feature = "graph")]
pub use publication::{
    GraphChanges, GraphPublication, GraphReadDependency, MAX_READ_DEPENDENCIES, PublicationIntent,
    PublicationRequest,
};
#[cfg(feature = "graph")]
pub use query::{GraphPath, Query, QueryResult, Traversal, TraversalResult};
#[cfg(feature = "graph")]
pub use transaction::{ReadTransaction, WriteTransaction};

pub const DATABASE_FORMAT_VERSION: u32 = 1;
pub const GRAPH_SCHEMA_VERSION: u32 = 1;
pub const INDEX_VERSION: u32 = 1;
