//! Per-configured-Graph-family read admission, not a process RSS or writer budget.
use crate::{GraphError, ReadBudget};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Debug, Clone, Copy)]
pub struct ReadLimits {
    pub snapshot: ReadBudget,
    pub max_retained_snapshots: usize,
}
pub(crate) struct Retention {
    pub(crate) limits: ReadLimits,
    count: AtomicUsize,
}
pub(crate) struct ReadLease(Arc<Retention>);
impl Retention {
    pub(crate) fn new(limits: ReadLimits) -> Result<Arc<Self>, GraphError> {
        if limits.max_retained_snapshots == 0
            || limits
                .max_retained_snapshots
                .checked_mul(limits.snapshot.max_source_bytes)
                .is_none()
        {
            return Err(GraphError::Invalid("invalid retained read limits".into()));
        }
        Ok(Arc::new(Self {
            limits,
            count: AtomicUsize::new(0),
        }))
    }
    pub(crate) fn acquire(self: &Arc<Self>) -> Result<ReadLease, GraphError> {
        self.count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count
                    .checked_add(1)
                    .filter(|next| *next <= self.limits.max_retained_snapshots)
            })
            .map_err(|_| zixcel_revision::Failure::Capacity)?;
        Ok(ReadLease(self.clone()))
    }
    pub(crate) fn count(&self) -> usize {
        self.count.load(Ordering::Acquire)
    }
}
impl Drop for ReadLease {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::Release);
    }
}
pub(crate) fn intersect(a: ReadBudget, b: ReadBudget) -> ReadBudget {
    ReadBudget {
        max_nodes: a.max_nodes.min(b.max_nodes),
        max_edges: a.max_edges.min(b.max_edges),
        max_index_entries: a.max_index_entries.min(b.max_index_entries),
        max_source_bytes: a.max_source_bytes.min(b.max_source_bytes),
    }
}
