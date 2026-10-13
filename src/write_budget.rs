//! Finite mutation input and atomic stored-state admission; not an RSS quota.
use crate::{Edge, GraphError, Node, ReadBudget, WriteTransaction, read_budget::Counter};

#[derive(Debug, Clone, Copy)]
pub struct WriteBudget {
    /// Applies to initial, concurrent and final stored state, including receipts.
    pub snapshot: ReadBudget,
    /// Cumulative mutation calls, including replacements and removals.
    pub max_updates: usize,
    /// Cumulative JSON-encoded mutation input bytes before retaining each input.
    pub max_update_bytes: usize,
}

pub struct BudgetedWriteTransaction {
    inner: WriteTransaction,
    counter: Counter,
    remaining: usize,
    refused: bool,
}
impl BudgetedWriteTransaction {
    pub(crate) fn new(inner: WriteTransaction, budget: WriteBudget) -> Self {
        Self {
            inner,
            counter: Counter::new(ReadBudget {
                max_nodes: 0,
                max_edges: 0,
                max_index_entries: 0,
                max_source_bytes: budget.max_update_bytes,
            }),
            remaining: budget.max_updates,
            refused: false,
        }
    }
    #[must_use]
    pub fn base_revision(&self) -> u64 {
        self.inner.base_revision()
    }

    fn admit(&mut self, input: &impl serde::Serialize) -> Result<(), GraphError> {
        if self.refused || self.remaining == 0 {
            self.refused = true;
            return Err(zixcel_revision::Failure::Capacity.into());
        }
        self.remaining -= 1;
        if let Err(error) = self.counter.encoded(input) {
            self.refused = true;
            return Err(error);
        }
        Ok(())
    }
    /// # Errors
    /// Exceeding cumulative input bounds poisons the entire write.
    pub fn upsert_node(&mut self, node: Node) -> Result<(), GraphError> {
        self.admit(&("node", &node))?;
        self.inner.upsert_node(node);
        Ok(())
    }
    /// # Errors
    /// Exceeding cumulative input bounds poisons the entire write.
    pub fn upsert_edge(&mut self, edge: Edge) -> Result<(), GraphError> {
        self.admit(&("edge", &edge))?;
        self.inner.upsert_edge(edge);
        Ok(())
    }
    /// # Errors
    /// Exceeding cumulative input bounds poisons the entire write.
    pub fn remove_edge(&mut self, id: &str) -> Result<Option<Edge>, GraphError> {
        self.admit(&("remove-edge", id))?;
        Ok(self.inner.remove_edge(id))
    }
    /// # Errors
    /// Attached edges or input exhaustion poison the entire write.
    pub fn remove_node(&mut self, id: &str) -> Result<Option<Node>, GraphError> {
        self.admit(&("remove-node", id))?;
        match self.inner.remove_node(id) {
            Ok(node) => Ok(node),
            Err(error) => {
                self.refused = true;
                Err(error)
            }
        }
    }
    /// # Errors
    /// A refused mutation, final stored-state quota, conflict or storage failure
    /// leaves canonical state unchanged. No successful prefix is published.
    pub fn commit(self) -> Result<u64, GraphError> {
        if self.refused {
            return Err(zixcel_revision::Failure::Capacity.into());
        }
        self.inner.commit()
    }
}
