//! Finite concurrent transaction staging for an explicit writer family, not an RSS quota.
use crate::{BudgetedWriteTransaction, Graph, GraphError, GraphSpace, WriteBudget};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug, Clone, Copy)]
pub struct WriteLimits {
    pub transaction: WriteBudget,
    pub max_pending_transactions: usize,
}
struct Admission {
    limits: WriteLimits,
    count: AtomicUsize,
}
pub(crate) struct WriteLease(Arc<Admission>);
impl Drop for WriteLease {
    fn drop(&mut self) {
        self.0.count.fetch_sub(1, Ordering::Release);
    }
}

/// Owns a Graph handle and shares finite write admission with subsequent clones.
/// Other Graph aliases, publication APIs, backend cache and caller-owned inputs remain outside this policy.
#[derive(Clone)]
pub struct BoundedWriter {
    graph: Graph,
    admission: Arc<Admission>,
}
impl BoundedWriter {
    /// # Errors
    /// Rejects zero pending capacity and aggregate encoded-source-byte overflow.
    pub fn new(graph: Graph, limits: WriteLimits) -> Result<Self, GraphError> {
        if limits.max_pending_transactions == 0
            || limits
                .transaction
                .snapshot
                .max_source_bytes
                .checked_add(limits.transaction.max_update_bytes)
                .and_then(|bytes| bytes.checked_mul(limits.max_pending_transactions))
                .is_none()
        {
            return Err(GraphError::Invalid("invalid pending write limits".into()));
        }
        Ok(Self {
            graph,
            admission: Arc::new(Admission {
                limits,
                count: AtomicUsize::new(0),
            }),
        })
    }
    /// Counts in-progress snapshot loads, pending transactions and commits in this family.
    #[must_use]
    pub fn pending_transactions(&self) -> usize {
        self.admission.count.load(Ordering::Acquire)
    }
    /// # Errors
    /// Reserves a slot before materialization; all failures release it. No eviction occurs.
    pub fn write(&self, space: &GraphSpace) -> Result<BudgetedWriteTransaction, GraphError> {
        self.write_budgeted(space, self.admission.limits.transaction)
    }
    /// # Errors
    /// Caller limits can only tighten the configured snapshot and cumulative input limits.
    pub fn write_budgeted(
        &self,
        space: &GraphSpace,
        budget: WriteBudget,
    ) -> Result<BudgetedWriteTransaction, GraphError> {
        let limits = self.admission.limits;
        self.admission
            .count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count
                    .checked_add(1)
                    .filter(|next| *next <= limits.max_pending_transactions)
            })
            .map_err(|_| zixcel_revision::Failure::Capacity)?;
        let lease = WriteLease(self.admission.clone());
        let budget = WriteBudget {
            snapshot: crate::read_retention::intersect(
                budget.snapshot,
                limits.transaction.snapshot,
            ),
            max_updates: budget.max_updates.min(limits.transaction.max_updates),
            max_update_bytes: budget
                .max_update_bytes
                .min(limits.transaction.max_update_bytes),
        };
        let mut transaction = self.graph.write_budgeted(space, budget)?;
        transaction.lease = Some(lease);
        Ok(transaction)
    }
}
