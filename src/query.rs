use crate::{Direction, Edge, GraphError, Node};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Query {
    NodesByLabel { label: String, limit: usize },
    EdgesByLabel { label: String, limit: usize },
    Traversal(Traversal),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QueryResult {
    Nodes { nodes: Vec<Node> },
    Edges { edges: Vec<Edge> },
    Traversal { result: TraversalResult },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Traversal {
    pub start: String,
    pub direction: Direction,
    pub maximum_depth: usize,
    pub maximum_nodes: usize,
    pub edge_label: Option<String>,
}

impl Traversal {
    #[must_use]
    pub fn new(
        start: impl Into<String>,
        direction: Direction,
        maximum_depth: usize,
        maximum_nodes: usize,
    ) -> Self {
        Self {
            start: start.into(),
            direction,
            maximum_depth,
            maximum_nodes,
            edge_label: None,
        }
    }

    #[must_use]
    pub fn with_edge_label(mut self, label: impl Into<String>) -> Self {
        self.edge_label = Some(label.into());
        self
    }

    pub(crate) fn validate(&self) -> Result<(), GraphError> {
        if self.maximum_depth == 0 || self.maximum_depth > 64 {
            return Err(GraphError::Invalid(
                "traversal maximum depth must be between 1 and 64".into(),
            ));
        }
        if self.maximum_nodes == 0 || self.maximum_nodes > 65_536 {
            return Err(GraphError::Invalid(
                "traversal maximum nodes must be between 1 and 65536".into(),
            ));
        }
        crate::validation::identifier(&self.start, 256, "traversal start")?;
        if let Some(label) = &self.edge_label {
            crate::validation::identifier(label, 256, "traversal edge label")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphPath {
    pub nodes: Vec<String>,
    pub edges: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraversalResult {
    pub visited: Vec<String>,
    pub paths: Vec<GraphPath>,
    pub truncated: bool,
}
