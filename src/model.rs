use crate::GraphError;
use crate::validation::{identifier, property_key};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GraphSpace(String);

impl GraphSpace {
    /// Creates a validated technical storage namespace.
    ///
    /// # Errors
    /// Returns an error for an empty, oversized, or non-canonical identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        identifier(&value, 128, "graph space")?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub labels: BTreeSet<String>,
    pub properties: BTreeMap<String, PropertyValue>,
    pub revision: u64,
}

impl Node {
    /// Creates a graph node with one or more labels.
    ///
    /// # Errors
    /// Returns an error when identifiers or the label count are invalid.
    pub fn new<I, S>(id: impl Into<String>, labels: I) -> Result<Self, GraphError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let id = id.into();
        identifier(&id, 256, "node id")?;
        let labels = labels.into_iter().map(Into::into).collect::<BTreeSet<_>>();
        if labels.is_empty() || labels.len() > 32 {
            return Err(GraphError::Invalid(
                "node labels must contain between 1 and 32 values".into(),
            ));
        }
        for label in &labels {
            identifier(label, 256, "node label")?;
        }
        Ok(Self {
            id,
            labels,
            properties: BTreeMap::new(),
            revision: 0,
        })
    }

    /// Adds or replaces a validated node property.
    ///
    /// # Errors
    /// Returns an error when the key, value, or property count is invalid.
    pub fn with_property(
        mut self,
        key: impl Into<String>,
        value: PropertyValue,
    ) -> Result<Self, GraphError> {
        let key = key.into();
        property_key(&key)?;
        crate::validation::property_value(&value)?;
        if self.properties.insert(key, value).is_none() && self.properties.len() > 128 {
            return Err(GraphError::Invalid(
                "node has more than 128 properties".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeDirection {
    Directed,
    Undirected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    pub direction: EdgeDirection,
    pub properties: BTreeMap<String, PropertyValue>,
    pub revision: u64,
}

impl Edge {
    /// Creates a directed edge.
    ///
    /// # Errors
    /// Returns an error when an identifier is invalid.
    pub fn directed(
        id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, GraphError> {
        Self::new(id, from, to, label, EdgeDirection::Directed)
    }

    /// Creates an undirected logical edge with canonical endpoint order.
    ///
    /// # Errors
    /// Returns an error when an identifier is invalid.
    pub fn undirected(
        id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        label: impl Into<String>,
    ) -> Result<Self, GraphError> {
        Self::new(id, from, to, label, EdgeDirection::Undirected)
    }

    fn new(
        id: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
        label: impl Into<String>,
        direction: EdgeDirection,
    ) -> Result<Self, GraphError> {
        let (id, mut from, mut to, label) = (id.into(), from.into(), to.into(), label.into());
        for (value, name) in [
            (&id, "edge id"),
            (&from, "edge from"),
            (&to, "edge to"),
            (&label, "edge label"),
        ] {
            identifier(value, 256, name)?;
        }
        if direction == EdgeDirection::Undirected && from > to {
            std::mem::swap(&mut from, &mut to);
        }
        Ok(Self {
            id,
            from,
            to,
            label,
            direction,
            properties: BTreeMap::new(),
            revision: 0,
        })
    }

    /// Adds or replaces a validated edge property.
    ///
    /// # Errors
    /// Returns an error when the key, value, or property count is invalid.
    pub fn with_property(
        mut self,
        key: impl Into<String>,
        value: PropertyValue,
    ) -> Result<Self, GraphError> {
        let key = key.into();
        property_key(&key)?;
        crate::validation::property_value(&value)?;
        if self.properties.insert(key, value).is_none() && self.properties.len() > 128 {
            return Err(GraphError::Invalid(
                "edge has more than 128 properties".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphMetadata {
    pub format_version: u32,
    pub graph_schema_version: u32,
    pub index_version: u32,
    pub backend: String,
    pub migration_history: Vec<String>,
}
