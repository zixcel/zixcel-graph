// Hatter reconstruction 2026: generic revision-fenced object durability preparation.
#[cfg(test)]
mod failure_tests;
mod memory;
#[cfg(feature = "redb")]
mod redb;

pub(crate) use memory::MemoryStorage;
#[cfg(feature = "redb")]
pub(crate) use redb::RedbStorage;

use crate::GraphPublication;
use crate::state::SpaceState;
use zixcel_revision::CommitState;

#[derive(Clone, Default)]
pub(crate) struct StoredSpace {
    pub graph: SpaceState,
    pub commits: CommitState,
}
pub(crate) type Publish<'a> = Box<
    dyn FnOnce(
            &mut StoredSpace,
            &[zixcel_revision::RevisionRef],
        ) -> Result<GraphPublication, GraphError>
        + 'a,
>;
pub(crate) type LifecycleChange<'a> =
    Box<dyn FnOnce(&mut CommitState) -> Result<(), GraphError> + 'a>;
use crate::{GraphError, GraphMetadata, GraphSpace};

pub(crate) trait Storage: Send + Sync {
    fn lifecycle(&self, space: &GraphSpace, change: LifecycleChange<'_>) -> Result<(), GraphError>;
    fn snapshot(&self, space: &GraphSpace) -> Result<StoredSpace, GraphError>;
    fn load_commits(&self, space: &GraphSpace) -> Result<CommitState, GraphError>;
    fn publish(
        &self,
        space: &GraphSpace,
        dependencies: &[crate::GraphReadDependency],
        publish: Publish<'_>,
    ) -> Result<GraphPublication, GraphError>;
    fn metadata(&self) -> Result<GraphMetadata, GraphError>;
    fn load(&self, space: &GraphSpace) -> Result<SpaceState, GraphError>;
    fn rebuild_indexes(&self, space: &GraphSpace) -> Result<(), GraphError>;
}
