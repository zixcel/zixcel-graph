// Hatter reconstruction 2026: one validated commit with an optional immutable-object fence.
use crate::backend::Storage;
use crate::query::{GraphPath, Query, QueryResult, Traversal, TraversalResult};
use crate::state::SpaceState;
use crate::{Direction, Edge, EdgeDirection, GraphError, GraphSpace, Node};
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

pub struct ReadTransaction {
    state: SpaceState,
}

impl ReadTransaction {
    pub(crate) fn new(state: SpaceState) -> Self {
        Self { state }
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.state.revision
    }

    #[must_use]
    pub fn get_node(&self, id: &str) -> Option<Node> {
        self.state.nodes.get(id).cloned()
    }

    #[must_use]
    pub fn get_edge(&self, id: &str) -> Option<Edge> {
        self.state.edges.get(id).cloned()
    }

    #[must_use]
    pub fn nodes_by_label(&self, label: &str, limit: usize) -> Vec<Node> {
        self.state
            .node_labels
            .get(label)
            .into_iter()
            .flatten()
            .take(limit)
            .filter_map(|id| self.state.nodes.get(id).cloned())
            .collect()
    }

    #[must_use]
    pub fn edges_by_label(&self, label: &str, limit: usize) -> Vec<Edge> {
        self.state
            .edge_labels
            .get(label)
            .into_iter()
            .flatten()
            .take(limit)
            .filter_map(|id| self.state.edges.get(id).cloned())
            .collect()
    }

    #[must_use]
    pub fn outgoing_edges(&self, node: &str, label: Option<&str>) -> Vec<Edge> {
        self.edges_for(node, Direction::Outgoing, label)
    }

    #[must_use]
    pub fn incoming_edges(&self, node: &str, label: Option<&str>) -> Vec<Edge> {
        self.edges_for(node, Direction::Incoming, label)
    }

    #[must_use]
    pub fn neighbors(&self, node: &str, direction: Direction, label: Option<&str>) -> Vec<String> {
        let mut neighbors = BTreeSet::new();
        for edge in self.edges_for(node, direction, label) {
            if edge.from == node {
                neighbors.insert(edge.to);
            } else if edge.to == node {
                neighbors.insert(edge.from);
            }
        }
        neighbors.into_iter().collect()
    }

    /// Executes a typed query against this immutable snapshot.
    ///
    /// # Errors
    /// Returns an error when a traversal query is invalid.
    pub fn query(&self, query: &Query) -> Result<QueryResult, GraphError> {
        match query {
            Query::NodesByLabel { label, limit } => Ok(QueryResult::Nodes {
                nodes: self.nodes_by_label(label, *limit),
            }),
            Query::EdgesByLabel { label, limit } => Ok(QueryResult::Edges {
                edges: self.edges_by_label(label, *limit),
            }),
            Query::Traversal(traversal) => Ok(QueryResult::Traversal {
                result: self.traverse(traversal)?,
            }),
        }
    }

    /// Performs a deterministic bounded breadth-first traversal.
    ///
    /// # Errors
    /// Returns an error when bounds are invalid or the start node is absent.
    pub fn traverse(&self, traversal: &Traversal) -> Result<TraversalResult, GraphError> {
        traversal.validate()?;
        if !self.state.nodes.contains_key(&traversal.start) {
            return Err(GraphError::MissingNode(traversal.start.clone()));
        }
        let mut visited = BTreeSet::from([traversal.start.clone()]);
        let mut visit_order = vec![traversal.start.clone()];
        let initial = GraphPath {
            nodes: vec![traversal.start.clone()],
            edges: Vec::new(),
        };
        let mut queue = VecDeque::from([(traversal.start.clone(), 0, initial)]);
        let mut paths = Vec::new();
        let mut truncated = false;
        while let Some((node, depth, path)) = queue.pop_front() {
            if depth == traversal.maximum_depth {
                continue;
            }
            for (neighbor, edge_id) in
                self.neighbor_steps(&node, traversal.direction, traversal.edge_label.as_deref())
            {
                if visited.contains(&neighbor) {
                    continue;
                }
                if visited.len() == traversal.maximum_nodes {
                    truncated = true;
                    break;
                }
                visited.insert(neighbor.clone());
                visit_order.push(neighbor.clone());
                let mut next = path.clone();
                next.nodes.push(neighbor.clone());
                next.edges.push(edge_id);
                paths.push(next.clone());
                queue.push_back((neighbor, depth + 1, next));
            }
            if truncated {
                break;
            }
        }
        Ok(TraversalResult {
            visited: visit_order,
            paths,
            truncated,
        })
    }

    fn edges_for(&self, node: &str, direction: Direction, label: Option<&str>) -> Vec<Edge> {
        let mut ids = BTreeSet::new();
        if matches!(direction, Direction::Outgoing | Direction::Both)
            && let Some(outgoing) = self.state.outgoing.get(node)
        {
            ids.extend(outgoing.iter().cloned());
        }
        if matches!(direction, Direction::Incoming | Direction::Both)
            && let Some(incoming) = self.state.incoming.get(node)
        {
            ids.extend(incoming.iter().cloned());
        }
        ids.into_iter()
            .filter_map(|id| self.state.edges.get(&id))
            .filter(|edge| label.is_none_or(|expected| edge.label == expected))
            .cloned()
            .collect()
    }

    fn neighbor_steps(
        &self,
        node: &str,
        direction: Direction,
        label: Option<&str>,
    ) -> Vec<(String, String)> {
        let mut result = BTreeSet::new();
        for edge in self.edges_for(node, direction, label) {
            match edge.direction {
                EdgeDirection::Undirected => {
                    let neighbor = if edge.from == node {
                        edge.to
                    } else {
                        edge.from
                    };
                    result.insert((neighbor, edge.id));
                }
                EdgeDirection::Directed
                    if edge.from == node
                        && matches!(direction, Direction::Outgoing | Direction::Both) =>
                {
                    result.insert((edge.to, edge.id));
                }
                EdgeDirection::Directed
                    if edge.to == node
                        && matches!(direction, Direction::Incoming | Direction::Both) =>
                {
                    result.insert((edge.from, edge.id));
                }
                EdgeDirection::Directed => {}
            }
        }
        result.into_iter().collect()
    }
}

pub struct WriteTransaction {
    storage: Arc<dyn Storage>,
    space: GraphSpace,
    base_revision: zixcel_revision::RevisionRef,
    state: SpaceState,
    touched_nodes: BTreeSet<String>,
    touched_edges: BTreeSet<String>,
}

impl WriteTransaction {
    pub(crate) fn new(
        storage: Arc<dyn Storage>,
        space: GraphSpace,
        state: crate::backend::StoredSpace,
    ) -> Self {
        Self {
            base_revision: state.commits.head_revision(space.as_str()),
            storage,
            space,
            state: state.graph,
            touched_nodes: BTreeSet::new(),
            touched_edges: BTreeSet::new(),
        }
    }

    /// Returns the immutable graph revision from which this write began.
    #[must_use]
    pub fn base_revision(&self) -> u64 {
        self.base_revision.sequence
    }

    pub fn upsert_node(&mut self, node: Node) {
        self.touched_nodes.insert(node.id.clone());
        self.state.nodes.insert(node.id.clone(), node);
    }

    pub fn upsert_edge(&mut self, edge: Edge) {
        self.touched_edges.insert(edge.id.clone());
        self.state.edges.insert(edge.id.clone(), edge);
    }

    pub fn remove_edge(&mut self, id: &str) -> Option<Edge> {
        self.touched_edges.insert(id.to_owned());
        self.state.edges.remove(id)
    }

    /// Removes a node only when no edge still refers to it.
    ///
    /// # Errors
    /// Returns an error when the node has attached edges.
    pub fn remove_node(&mut self, id: &str) -> Result<Option<Node>, GraphError> {
        if self
            .state
            .edges
            .values()
            .any(|edge| edge.from == id || edge.to == id)
        {
            return Err(GraphError::Invalid(
                "node with attached edges cannot be removed".into(),
            ));
        }
        self.touched_nodes.insert(id.to_owned());
        Ok(self.state.nodes.remove(id))
    }

    /// Publishes graph changes through the same Foundation as every other consumer.
    /// # Errors
    /// Exact stale/reuse conflicts and storage failures retain their typed outcomes.
    pub fn commit(self) -> Result<u64, GraphError> {
        let mut changes = crate::GraphChanges::default();
        for id in self.touched_nodes {
            if let Some(n) = self.state.nodes.get(&id) {
                changes.nodes.push(n.clone());
            } else {
                changes.remove_nodes.push(id);
            }
        }
        for id in self.touched_edges {
            if let Some(e) = self.state.edges.get(&id) {
                changes.edges.push(e.clone());
            } else {
                changes.remove_edges.push(id);
            }
        }
        // Proposal fingerprint, not a second commit identity or replay store.
        let mut intent = crate::PublicationIntent {
            operation_id: "graph/write".into(),
            expected_commit_revision: self.base_revision,
        };
        let digest = intent
            .request("graph/write", &(&intent.expected_commit_revision, &changes))?
            .request_digest;
        intent.operation_id = format!("graph/write/{digest}");
        let request = intent.request("graph/write", &changes)?;
        let graph = crate::Graph {
            storage: self.storage,
        };
        let result = graph.publish(&self.space, &request, |_| Ok(changes), || Ok(()))?;
        match result.outcome {
            zixcel_revision::CommitOutcome::Committed(r)
            | zixcel_revision::CommitOutcome::NoChange(r) => Ok(r.committed_revision.sequence),
            zixcel_revision::CommitOutcome::Conflict(c) => Err(GraphError::CommitConflict(c)),
            zixcel_revision::CommitOutcome::Rejected(r) => {
                Err(zixcel_revision::Failure::Rejected(r).into())
            }
            zixcel_revision::CommitOutcome::Failure(f) => Err(f.into()),
        }
    }
}
