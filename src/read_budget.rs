//! Admission limits for one materialized read, not a process RSS guarantee.
use crate::{GraphError, state::SpaceState};
#[derive(Debug, Clone, Copy)]
pub struct ReadBudget {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_index_entries: usize,
    /// Commit image, node/edge JSON and complete storage-key bytes.
    pub max_source_bytes: usize,
}
pub(crate) struct Counter<'a> {
    cancelled: Option<&'a std::sync::atomic::AtomicBool>,
    cancellation_observed: bool,
    limits: ReadBudget,
    nodes: usize,
    edges: usize,
    indexes: usize,
    bytes: usize,
}
impl<'a> Counter<'a> {
    pub(crate) fn new(limits: ReadBudget) -> Self {
        Self {
            limits,
            cancelled: None,
            cancellation_observed: false,
            nodes: 0,
            edges: 0,
            indexes: 0,
            bytes: 0,
        }
    }
    #[cfg(feature = "redb")]
    pub(crate) fn unlimited() -> Self {
        Self::new(ReadBudget {
            max_nodes: usize::MAX,
            max_edges: usize::MAX,
            max_index_entries: usize::MAX,
            max_source_bytes: usize::MAX,
        })
    }
    pub(crate) fn with_cancellation(
        mut self,
        cancelled: &'a std::sync::atomic::AtomicBool,
    ) -> Self {
        self.cancelled = Some(cancelled);
        self
    }
    fn check_cancelled(&mut self) -> Result<(), GraphError> {
        if let Some(cancelled) = self.cancelled {
            self.cancellation_observed |= cancelled.load(std::sync::atomic::Ordering::Relaxed);
        }
        if self.cancellation_observed {
            return Err(GraphError::Invalid("storage validation cancelled".into()));
        }
        Ok(())
    }
    fn charge(value: &mut usize, amount: usize, limit: usize) -> Result<(), GraphError> {
        *value = value
            .checked_add(amount)
            .filter(|n| *n <= limit)
            .ok_or(GraphError::Commit(zixcel_revision::Failure::Capacity))?;
        Ok(())
    }
    pub(crate) fn bytes(&mut self, amount: usize) -> Result<(), GraphError> {
        self.check_cancelled()?;
        Self::charge(&mut self.bytes, amount, self.limits.max_source_bytes)
    }
    pub(crate) fn node(&mut self) -> Result<(), GraphError> {
        self.check_cancelled()?;
        Self::charge(&mut self.nodes, 1, self.limits.max_nodes)
    }
    pub(crate) fn edge(&mut self) -> Result<(), GraphError> {
        self.check_cancelled()?;
        Self::charge(&mut self.edges, 1, self.limits.max_edges)
    }
    pub(crate) fn index(&mut self, key_bytes: usize) -> Result<(), GraphError> {
        self.check_cancelled()?;
        Self::charge(&mut self.indexes, 1, self.limits.max_index_entries)?;
        self.bytes(key_bytes)
    }
    pub(crate) fn encoded(&mut self, value: &impl serde::Serialize) -> Result<(), GraphError> {
        let result = serde_json::to_writer(&mut *self, value);
        self.check_cancelled()?;
        result.map_err(|_| GraphError::Commit(zixcel_revision::Failure::Capacity))
    }
    pub(crate) fn stored(
        &mut self,
        space: &str,
        state: &SpaceState,
        commits: &zixcel_revision::CommitState,
    ) -> Result<(), GraphError> {
        self.encoded(commits)?;
        for (id, node) in &state.nodes {
            self.node()?;
            self.bytes(space.len() + 1 + id.len())?;
            self.encoded(node)?;
        }
        for (id, edge) in &state.edges {
            self.edge()?;
            self.bytes(space.len() + 1 + id.len())?;
            self.encoded(edge)?;
        }
        for map in [
            &state.node_labels,
            &state.edge_labels,
            &state.outgoing,
            &state.incoming,
        ] {
            for (first, seconds) in map {
                for second in seconds {
                    self.index(space.len() + 2 + first.len() + second.len())?;
                }
            }
        }
        Ok(())
    }
}
impl std::io::Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes(bytes.len()).map_err(std::io::Error::other)?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use serde::ser::SerializeSeq;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct StopDuringStream<'a>(&'a AtomicBool);
    impl serde::Serialize for StopDuringStream<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element(&1)?;
            self.0.store(true, Ordering::Relaxed);
            sequence.serialize_element(&2)?;
            sequence.end()
        }
    }
    #[test]
    fn cancellation_during_serialization_keeps_cancel_identity() {
        let stop = AtomicBool::new(false);
        let mut counter = Counter::new(ReadBudget {
            max_nodes: 2,
            max_edges: 2,
            max_index_entries: 2,
            max_source_bytes: 65536,
        })
        .with_cancellation(&stop);
        assert!(matches!(
            counter.encoded(&StopDuringStream(&stop)),
            Err(GraphError::Invalid(_))
        ));
        stop.store(false, Ordering::Relaxed);
        assert!(matches!(counter.node(), Err(GraphError::Invalid(_))));
        assert!(matches!(counter.index(1), Err(GraphError::Invalid(_))));
    }
}
