use crate::{Edge, EdgeDirection, GraphError, Node};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, PartialEq)]
pub(crate) struct SpaceState {
    pub revision: u64,
    pub nodes: BTreeMap<String, Node>,
    pub edges: BTreeMap<String, Edge>,
    pub node_labels: BTreeMap<String, BTreeSet<String>>,
    pub edge_labels: BTreeMap<String, BTreeSet<String>>,
    pub outgoing: BTreeMap<String, BTreeSet<String>>,
    pub incoming: BTreeMap<String, BTreeSet<String>>,
}

impl SpaceState {
    pub fn rebuild_indexes(&mut self) {
        self.node_labels.clear();
        self.edge_labels.clear();
        self.outgoing.clear();
        self.incoming.clear();
        for node in self.nodes.values() {
            for label in &node.labels {
                self.node_labels
                    .entry(label.clone())
                    .or_default()
                    .insert(node.id.clone());
            }
        }
        for edge in self.edges.values() {
            self.edge_labels
                .entry(edge.label.clone())
                .or_default()
                .insert(edge.id.clone());
            self.outgoing
                .entry(edge.from.clone())
                .or_default()
                .insert(edge.id.clone());
            if edge.direction == EdgeDirection::Undirected {
                self.outgoing
                    .entry(edge.to.clone())
                    .or_default()
                    .insert(edge.id.clone());
            } else {
                self.incoming
                    .entry(edge.to.clone())
                    .or_default()
                    .insert(edge.id.clone());
            }
        }
    }

    pub fn validate(&self) -> Result<(), GraphError> {
        for edge in self.edges.values() {
            if !self.nodes.contains_key(&edge.from) {
                return Err(GraphError::MissingNode(edge.from.clone()));
            }
            if !self.nodes.contains_key(&edge.to) {
                return Err(GraphError::MissingNode(edge.to.clone()));
            }
        }
        let mut rebuilt = Self {
            revision: self.revision,
            nodes: self.nodes.clone(),
            edges: self.edges.clone(),
            ..Self::default()
        };
        rebuilt.rebuild_indexes();
        if rebuilt.node_labels != self.node_labels
            || rebuilt.edge_labels != self.edge_labels
            || rebuilt.outgoing != self.outgoing
            || rebuilt.incoming != self.incoming
        {
            return Err(GraphError::Storage(
                "derived graph index is inconsistent".into(),
            ));
        }
        Ok(())
    }
}
