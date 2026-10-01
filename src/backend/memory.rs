// Graph-only commit and dependency validation share one writer fence; no domain callbacks.
use super::{Publish, Storage, StoredSpace};
use crate::GraphPublication;
use crate::state::SpaceState;
use crate::{
    DATABASE_FORMAT_VERSION, GRAPH_SCHEMA_VERSION, GraphError, GraphMetadata, GraphSpace,
    INDEX_VERSION,
};
use std::collections::BTreeMap;
use std::sync::RwLock;
use zixcel_revision::{CommitOutcome, CommitState};

pub(crate) struct MemoryStorage {
    spaces: RwLock<BTreeMap<String, StoredSpace>>,
}

impl MemoryStorage {
    pub(crate) fn new() -> Self {
        Self {
            spaces: RwLock::new(BTreeMap::new()),
        }
    }
}

impl Storage for MemoryStorage {
    fn lifecycle(
        &self,
        space: &GraphSpace,
        change: super::LifecycleChange<'_>,
    ) -> Result<(), GraphError> {
        let mut spaces = self
            .spaces
            .write()
            .map_err(|_| zixcel_revision::Failure::Storage)?;
        let mut proposed = spaces.get(space.as_str()).cloned().unwrap_or_default();
        #[cfg(test)]
        super::failure_tests::checkpoint(super::failure_tests::Point::Before)?;
        change(&mut proposed.commits)?;
        proposed.commits.validate()?;
        #[cfg(test)]
        super::failure_tests::checkpoint(super::failure_tests::Point::During)?;
        spaces.insert(space.as_str().to_owned(), proposed);
        #[cfg(test)]
        super::failure_tests::checkpoint(super::failure_tests::Point::After)?;

        Ok(())
    }
    fn snapshot(&self, space: &GraphSpace) -> Result<StoredSpace, GraphError> {
        let spaces = self
            .spaces
            .read()
            .map_err(|_| zixcel_revision::Failure::Storage)?;
        Ok(spaces.get(space.as_str()).cloned().unwrap_or_default())
    }
    fn load_commits(&self, space: &GraphSpace) -> Result<CommitState, GraphError> {
        let spaces = self
            .spaces
            .read()
            .map_err(|_| zixcel_revision::Failure::Storage)?;
        Ok(spaces
            .get(space.as_str())
            .map(|s| s.commits.clone())
            .unwrap_or_default())
    }
    fn publish(
        &self,
        space: &GraphSpace,
        dependencies: &[crate::GraphReadDependency],
        publish: Publish<'_>,
    ) -> Result<GraphPublication, GraphError> {
        let mut spaces = self
            .spaces
            .write()
            .map_err(|_| zixcel_revision::Failure::Storage)?;
        let mut proposed = spaces.get(space.as_str()).cloned().unwrap_or_default();
        #[cfg(test)]
        super::failure_tests::checkpoint(super::failure_tests::Point::Before)?;
        let actual: Vec<_> = dependencies
            .iter()
            .map(|d| {
                spaces
                    .get(d.space.as_str())
                    .map(|s| s.commits.head_revision(d.space.as_str()))
                    .unwrap_or_default()
            })
            .collect();
        let result = publish(&mut proposed, &actual)?;
        if matches!(result.outcome, CommitOutcome::Committed(_)) {
            proposed.graph.validate()?;
            proposed.commits.validate()?;
            #[cfg(test)]
            super::failure_tests::checkpoint(super::failure_tests::Point::During)?;
            spaces.insert(space.as_str().to_owned(), proposed);
            #[cfg(test)]
            super::failure_tests::checkpoint(super::failure_tests::Point::After)?;
        }
        Ok(result)
    }
    fn metadata(&self) -> Result<GraphMetadata, GraphError> {
        Ok(GraphMetadata {
            format_version: DATABASE_FORMAT_VERSION,
            graph_schema_version: GRAPH_SCHEMA_VERSION,
            index_version: INDEX_VERSION,
            backend: "memory".to_owned(),
            migration_history: Vec::new(),
        })
    }

    fn load(&self, space: &GraphSpace) -> Result<SpaceState, GraphError> {
        let spaces = self
            .spaces
            .read()
            .map_err(|_| zixcel_revision::Failure::Storage)?;
        Ok(spaces
            .get(space.as_str())
            .map(|s| s.graph.clone())
            .unwrap_or_default())
    }

    fn rebuild_indexes(&self, space: &GraphSpace) -> Result<(), GraphError> {
        let mut spaces = self
            .spaces
            .write()
            .map_err(|_| zixcel_revision::Failure::Storage)?;
        if let Some(state) = spaces.get_mut(space.as_str()) {
            state.graph.rebuild_indexes();
            state.graph.validate()?;
        }
        Ok(())
    }
}
