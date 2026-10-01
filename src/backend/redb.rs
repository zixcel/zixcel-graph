// Graph 0.10.0: private structural application and dependencies share one writer;
// arbitrary domain callbacks and immutable preparation run outside storage locks.
use super::{Publish, Storage, StoredSpace};
use crate::GraphPublication;
use crate::error::{codec, storage};
use crate::state::SpaceState;
use crate::{
    DATABASE_FORMAT_VERSION, GRAPH_SCHEMA_VERSION, GraphError, GraphMetadata, GraphSpace,
    INDEX_VERSION,
};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, TableHandle};
use std::path::Path;
use zixcel_revision::{CommitOutcome, CommitState, Failure};

const COMMITS: TableDefinition<&str, &[u8]> = TableDefinition::new("zixcel_graph_commits");
const META: TableDefinition<&str, &str> = TableDefinition::new("zixcel_graph_metadata");
const SPACES: TableDefinition<&str, u64> = TableDefinition::new("zixcel_graph_spaces");
const NODES: TableDefinition<&str, &[u8]> = TableDefinition::new("zixcel_graph_nodes");
const EDGES: TableDefinition<&str, &[u8]> = TableDefinition::new("zixcel_graph_edges");
const NODE_LABELS: TableDefinition<&str, u8> = TableDefinition::new("zixcel_graph_node_labels");
const EDGE_LABELS: TableDefinition<&str, u8> = TableDefinition::new("zixcel_graph_edge_labels");
const OUTGOING: TableDefinition<&str, u8> = TableDefinition::new("zixcel_graph_outgoing");
const INCOMING: TableDefinition<&str, u8> = TableDefinition::new("zixcel_graph_incoming");

#[test]
fn dependency_with_orphan_revision_image_is_corrupt_not_empty() {
    let directory = tempfile::tempdir().unwrap();
    let backend = RedbStorage::create(&directory.path().join("graph.redb")).unwrap();
    let source = GraphSpace::new("source/orphan").unwrap();
    backend
        .database
        .write(|database| -> Result<(), GraphError> {
            let write = database.begin_write().map_err(storage)?;
            {
                let mut table = write.open_table(COMMITS).map_err(storage)?;
                let bytes = serde_json::to_vec(&CommitState::default()).unwrap();
                table
                    .insert(source.as_str(), bytes.as_slice())
                    .map_err(storage)?;
            }
            write.commit().map_err(storage)?;
            Ok(())
        })
        .unwrap();
    let result = backend.publish(
        &GraphSpace::new("target").unwrap(),
        &[crate::GraphReadDependency {
            space: source,
            expected_revision: zixcel_revision::RevisionRef::default(),
        }],
        Box::new(|_, _| panic!("corrupt source must not reach target application")),
    );
    assert!(matches!(result, Err(GraphError::Commit(Failure::Corrupt))));
}

pub(crate) struct RedbStorage {
    database: zixcel_revision::durable::Handle,
}

impl RedbStorage {
    pub(crate) fn create(path: &Path) -> Result<Self, GraphError> {
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Self::open_existing(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage(error)),
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(storage)?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(storage)?;
        let database = Database::builder().create_file(file).map_err(storage)?;
        let storage = Self {
            database: zixcel_revision::durable::Handle::created(path, database),
        };
        storage.initialize()?;
        Ok(storage)
    }

    pub(crate) fn open_existing(path: &Path) -> Result<Self, GraphError> {
        let _reader = crate::startup::read_lease(path)?;
        Self::existing(path, false)
    }
    pub(crate) fn recover_existing(path: &Path) -> Result<Self, GraphError> {
        let _owner = crate::startup::exclusive_lease(path)?;
        Self::existing(path, true)
    }
    pub(crate) fn open_at_startup(path: &Path) -> Result<Self, GraphError> {
        Self::existing(path, false)
    }
    pub(crate) fn recover_at_startup(path: &Path) -> Result<Self, GraphError> {
        Self::existing(path, true)
    }
    pub(crate) fn validate_all(&self) -> Result<(), GraphError> {
        self.database.read(|read| {
            let allowed = [
                COMMITS.name(),
                META.name(),
                SPACES.name(),
                NODES.name(),
                EDGES.name(),
                NODE_LABELS.name(),
                EDGE_LABELS.name(),
                OUTGOING.name(),
                INCOMING.name(),
            ];
            if read
                .list_multimap_tables()
                .map_err(storage)?
                .next()
                .is_some()
            {
                return Err(Failure::Corrupt.into());
            }
            for table in read.list_tables().map_err(storage)? {
                if !allowed.contains(&table.name()) {
                    return Err(Failure::Corrupt.into());
                }
            }
            let spaces = read.open_table(SPACES).map_err(storage)?;
            for entry in spaces.iter().map_err(storage)? {
                let (id, _) = entry.map_err(storage)?;
                Self::load_from_read(read, &GraphSpace::new(id.value())?, true)?;
            }
            // Orphan revision images must not disappear from startup validation.
            let commits = read.open_table(COMMITS).map_err(storage)?;
            for entry in commits.iter().map_err(storage)? {
                let (id, _) = entry.map_err(storage)?;
                read_commits(read, &GraphSpace::new(id.value())?)?;
            }
            // No primary/index row may hide outside the set of validated spaces.
            for table in [NODES, EDGES] {
                validate_key_spaces(read, table, &spaces)?;
            }
            for table in [NODE_LABELS, EDGE_LABELS, OUTGOING, INCOMING] {
                validate_key_spaces(read, table, &spaces)?;
            }
            Ok(())
        })
    }
    fn existing(path: &Path, recover: bool) -> Result<Self, GraphError> {
        if !std::fs::symlink_metadata(path)
            .map_err(storage)?
            .file_type()
            .is_file()
        {
            return Err(GraphError::InvalidPath(path.to_path_buf()));
        }
        let backend = Self {
            database: if recover {
                zixcel_revision::durable::Handle::recover(path)?
            } else {
                zixcel_revision::durable::Handle::open(path)?
            },
        };
        backend.metadata()?;
        backend.database.read(|read| {
            read.open_table(COMMITS).map_err(|_| Failure::Corrupt)?;
            read.open_table(SPACES).map_err(storage)?;
            read.open_table(NODES).map_err(storage)?;
            read.open_table(EDGES).map_err(storage)?;
            read.open_table(NODE_LABELS).map_err(storage)?;
            read.open_table(EDGE_LABELS).map_err(storage)?;
            read.open_table(OUTGOING).map_err(storage)?;
            read.open_table(INCOMING).map_err(storage)?;
            Ok::<_, GraphError>(())
        })?;
        Ok(backend)
    }

    fn initialize(&self) -> Result<(), GraphError> {
        self.database.write(|database| {
            let write = database.begin_write().map_err(storage)?;
            {
                let mut metadata = write.open_table(META).map_err(storage)?;
                initialize_version(&mut metadata, "format", DATABASE_FORMAT_VERSION)?;
                initialize_version(&mut metadata, "graph_schema", GRAPH_SCHEMA_VERSION)?;
                initialize_version(&mut metadata, "index", INDEX_VERSION)?;
                metadata.insert("migration_history", "").map_err(storage)?;
                write.open_table(COMMITS).map_err(storage)?;
                write.open_table(SPACES).map_err(storage)?;
                write.open_table(NODES).map_err(storage)?;
                write.open_table(EDGES).map_err(storage)?;
                write.open_table(NODE_LABELS).map_err(storage)?;
                write.open_table(EDGE_LABELS).map_err(storage)?;
                write.open_table(OUTGOING).map_err(storage)?;
                write.open_table(INCOMING).map_err(storage)?;
            }
            write.commit().map_err(storage)
        })
    }

    fn load_from_read(
        read: &redb::ReadTransaction,
        space: &GraphSpace,
        load_indexes: bool,
    ) -> Result<SpaceState, GraphError> {
        read_commits(read, space)?;
        let revision = read
            .open_table(SPACES)
            .map_err(storage)?
            .get(space.as_str())
            .map_err(storage)?
            .map_or(0, |value| value.value());
        let mut state = SpaceState {
            revision,
            ..SpaceState::default()
        };
        let prefix = key_prefix(space);
        {
            let table = read.open_table(NODES).map_err(storage)?;
            for entry in table.iter().map_err(storage)? {
                let (key, value) = entry.map_err(storage)?;
                if let Some(id) = key.value().strip_prefix(&prefix) {
                    let node = serde_json::from_slice(value.value()).map_err(codec)?;
                    state.nodes.insert(id.to_owned(), node);
                }
            }
        }
        {
            let table = read.open_table(EDGES).map_err(storage)?;
            for entry in table.iter().map_err(storage)? {
                let (key, value) = entry.map_err(storage)?;
                if let Some(id) = key.value().strip_prefix(&prefix) {
                    let edge = serde_json::from_slice(value.value()).map_err(codec)?;
                    state.edges.insert(id.to_owned(), edge);
                }
            }
        }
        if load_indexes {
            load_set_index(read, NODE_LABELS, &prefix, &mut state.node_labels)?;
            load_set_index(read, EDGE_LABELS, &prefix, &mut state.edge_labels)?;
            load_set_index(read, OUTGOING, &prefix, &mut state.outgoing)?;
            load_set_index(read, INCOMING, &prefix, &mut state.incoming)?;
            state.validate()?;
        } else {
            state.rebuild_indexes();
        }
        Ok(state)
    }
}

fn validate_key_spaces<V: redb::Value + 'static>(
    read: &redb::ReadTransaction,
    definition: TableDefinition<&str, V>,
    spaces: &redb::ReadOnlyTable<&str, u64>,
) -> Result<(), GraphError> {
    for entry in read
        .open_table(definition)
        .map_err(storage)?
        .iter()
        .map_err(storage)?
    {
        let (key, _) = entry.map_err(storage)?;
        let (space, suffix) = key.value().split_once('\0').ok_or(Failure::Corrupt)?;
        if suffix.is_empty() || spaces.get(space).map_err(storage)?.is_none() {
            return Err(Failure::Corrupt.into());
        }
    }
    Ok(())
}

impl Storage for RedbStorage {
    fn lifecycle(
        &self,
        space: &GraphSpace,
        change: super::LifecycleChange<'_>,
    ) -> Result<(), GraphError> {
        self.database.write(|database| {
            let write = database.begin_write().map_err(storage)?;
            let read = database.begin_read().map_err(storage)?;
            let mut commits = read_commits(&read, space)?;
            let graph = Self::load_from_read(&read, space, true)?;
            let before = serde_json::to_vec(&commits).map_err(codec)?;
            drop(read);
            #[cfg(test)]
            super::failure_tests::checkpoint(super::failure_tests::Point::Before)?;
            change(&mut commits)?;
            commits.validate()?;
            let bytes = serde_json::to_vec(&commits).map_err(codec)?;
            if before == bytes {
                return Ok(());
            }
            write_state(&write, space, &graph)?;
            {
                let mut table = write.open_table(COMMITS).map_err(storage)?;
                table
                    .insert(space.as_str(), bytes.as_slice())
                    .map_err(storage)?;
            }
            // Prepared bytes are now in the transaction, but not durably published.
            #[cfg(test)]
            super::failure_tests::checkpoint(super::failure_tests::Point::During)?;
            write.commit().map_err(storage)?;
            #[cfg(test)]
            super::failure_tests::checkpoint(super::failure_tests::Point::After)?;
            Ok(())
        })
    }
    fn snapshot(&self, space: &GraphSpace) -> Result<StoredSpace, GraphError> {
        self.database.read(|read| {
            Ok(StoredSpace {
                graph: Self::load_from_read(read, space, true)?,
                commits: read_commits(read, space)?,
            })
        })
    }
    fn load_commits(&self, space: &GraphSpace) -> Result<CommitState, GraphError> {
        self.database.read(|read| read_commits(read, space))
    }
    fn publish(
        &self,
        space: &GraphSpace,
        dependencies: &[crate::GraphReadDependency],
        publish: Publish<'_>,
    ) -> Result<GraphPublication, GraphError> {
        self.database.write(|database| {
            // The database writer excludes both old graph writers and new control publications.
            let write = database.begin_write().map_err(storage)?;
            let read = database.begin_read().map_err(storage)?;
            let mut proposed = StoredSpace {
                graph: Self::load_from_read(&read, space, true)?,
                commits: read_commits(&read, space)?,
            };
            drop(read);
            #[cfg(test)]
            super::failure_tests::checkpoint(super::failure_tests::Point::Before)?;
            // Source revisions are read from the exact target writer transaction,
            // not an auxiliary physical handle or an earlier read transaction.
            let actual = {
                let table = write.open_table(COMMITS).map_err(storage)?;
                dependencies
                    .iter()
                    .map(|d| {
                        let value = table.get(d.space.as_str()).map_err(storage)?;
                        let physical_revision = write
                            .open_table(SPACES)
                            .map_err(storage)?
                            .get(d.space.as_str())
                            .map_err(storage)?
                            .map(|v| v.value());
                        let state: CommitState = match value {
                            Some(bytes) => {
                                if physical_revision.is_none() {
                                    return Err(Failure::Corrupt.into());
                                }
                                if bytes.value().len() > 96 * 1024 * 1024 {
                                    return Err(Failure::Capacity.into());
                                }
                                serde_json::from_slice(bytes.value()).map_err(codec)?
                            }
                            None if physical_revision.is_none() => CommitState::default(),
                            None => return Err(Failure::Corrupt.into()),
                        };
                        state.validate()?;
                        if physical_revision
                            .is_some_and(|v| v != state.head_revision(d.space.as_str()).sequence)
                        {
                            return Err(Failure::Corrupt.into());
                        }
                        Ok(state.head_revision(d.space.as_str()))
                    })
                    .collect::<Result<Vec<_>, GraphError>>()?
            };
            let result = publish(&mut proposed, &actual)?;
            if matches!(result.outcome, CommitOutcome::Committed(_)) {
                proposed.graph.validate()?;
                proposed.commits.validate()?;
                write_state(&write, space, &proposed.graph)?;
                #[cfg(test)]
                super::failure_tests::checkpoint(super::failure_tests::Point::During)?;

                let bytes = serde_json::to_vec(&proposed.commits).map_err(codec)?;
                {
                    let mut table = write.open_table(COMMITS).map_err(storage)?;
                    table
                        .insert(space.as_str(), bytes.as_slice())
                        .map_err(storage)?;
                }
                write.commit().map_err(storage)?;
                #[cfg(test)]
                super::failure_tests::checkpoint(super::failure_tests::Point::After)?;
            }
            Ok(result)
        })
    }
    fn metadata(&self) -> Result<GraphMetadata, GraphError> {
        self.database.read(|read| {
            let table = read.open_table(META).map_err(storage)?;
            let format_version = read_version(&table, "format")?;
            let graph_schema_version = read_version(&table, "graph_schema")?;
            let index_version = read_version(&table, "index")?;
            if format_version != DATABASE_FORMAT_VERSION
                || graph_schema_version != GRAPH_SCHEMA_VERSION
                || index_version != INDEX_VERSION
            {
                return Err(GraphError::UnsupportedVersion);
            }
            let migration_history = table
                .get("migration_history")
                .map_err(storage)?
                .ok_or(GraphError::UnsupportedVersion)?
                .value()
                .split(',')
                .filter(|item| !item.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            Ok(GraphMetadata {
                format_version,
                graph_schema_version,
                index_version,
                backend: "redb".to_owned(),
                migration_history,
            })
        })
    }

    fn load(&self, space: &GraphSpace) -> Result<SpaceState, GraphError> {
        self.database
            .read(|read| Self::load_from_read(read, space, true))
    }

    fn rebuild_indexes(&self, space: &GraphSpace) -> Result<(), GraphError> {
        self.database.write(|database| {
            let write = database.begin_write().map_err(storage)?;
            let read = database.begin_read().map_err(storage)?;
            let state = Self::load_from_read(&read, space, false)?;
            drop(read);
            write_indexes(&write, space, &state)?;
            write.commit().map_err(storage)
        })
    }
}

fn initialize_version(
    table: &mut redb::Table<'_, &str, &str>,
    key: &str,
    value: u32,
) -> Result<(), GraphError> {
    let encoded = value.to_string();
    if let Some(current) = table.get(key).map_err(storage)? {
        if current.value() != encoded {
            return Err(GraphError::UnsupportedVersion);
        }
    } else {
        table.insert(key, encoded.as_str()).map_err(storage)?;
    }
    Ok(())
}

fn read_version(table: &redb::ReadOnlyTable<&str, &str>, key: &str) -> Result<u32, GraphError> {
    table
        .get(key)
        .map_err(storage)?
        .ok_or(GraphError::UnsupportedVersion)?
        .value()
        .parse()
        .map_err(codec)
}

fn key_prefix(space: &GraphSpace) -> String {
    format!("{}\0", space.as_str())
}

fn index_key(space: &GraphSpace, first: &str, second: &str) -> String {
    format!("{}\0{first}\0{second}", space.as_str())
}

fn load_set_index(
    read: &redb::ReadTransaction,
    definition: TableDefinition<&str, u8>,
    prefix: &str,
    target: &mut std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> Result<(), GraphError> {
    let table = read.open_table(definition).map_err(storage)?;
    for entry in table.iter().map_err(storage)? {
        let (key, _) = entry.map_err(storage)?;
        if let Some(remainder) = key.value().strip_prefix(prefix) {
            let (first, second) = remainder.split_once('\0').ok_or(Failure::Corrupt)?;
            target
                .entry(first.to_owned())
                .or_default()
                .insert(second.to_owned());
        }
    }
    Ok(())
}

fn write_state(
    write: &redb::WriteTransaction,
    space: &GraphSpace,
    state: &SpaceState,
) -> Result<(), GraphError> {
    let prefix = key_prefix(space);
    {
        let mut commits = write.open_table(COMMITS).map_err(storage)?;
        let present = commits.get(space.as_str()).map_err(storage)?.is_some();
        if !present {
            // Explicit creation of a new graph space, never read-side repair.
            let spaces = write.open_table(SPACES).map_err(storage)?;
            if spaces.get(space.as_str()).map_err(storage)?.is_some() {
                return Err(Failure::Corrupt.into());
            }
            let empty = serde_json::to_vec(&CommitState::default()).map_err(codec)?;
            commits
                .insert(space.as_str(), empty.as_slice())
                .map_err(storage)?;
        }
    }
    {
        let mut spaces = write.open_table(SPACES).map_err(storage)?;
        spaces
            .insert(space.as_str(), state.revision)
            .map_err(storage)?;
    }
    {
        let mut nodes = write.open_table(NODES).map_err(storage)?;
        nodes
            .retain(|key, _| !key.starts_with(&prefix))
            .map_err(storage)?;
        for node in state.nodes.values() {
            let key = format!("{prefix}{}", node.id);
            let value = serde_json::to_vec(node).map_err(codec)?;
            nodes
                .insert(key.as_str(), value.as_slice())
                .map_err(storage)?;
        }
    }
    {
        let mut edges = write.open_table(EDGES).map_err(storage)?;
        edges
            .retain(|key, _| !key.starts_with(&prefix))
            .map_err(storage)?;
        for edge in state.edges.values() {
            let key = format!("{prefix}{}", edge.id);
            let value = serde_json::to_vec(edge).map_err(codec)?;
            edges
                .insert(key.as_str(), value.as_slice())
                .map_err(storage)?;
        }
    }
    write_indexes(write, space, state)
}

fn write_indexes(
    write: &redb::WriteTransaction,
    space: &GraphSpace,
    state: &SpaceState,
) -> Result<(), GraphError> {
    write_set_index(write, NODE_LABELS, space, &state.node_labels)?;
    write_set_index(write, EDGE_LABELS, space, &state.edge_labels)?;
    write_set_index(write, OUTGOING, space, &state.outgoing)?;
    write_set_index(write, INCOMING, space, &state.incoming)
}

fn write_set_index(
    write: &redb::WriteTransaction,
    definition: TableDefinition<&str, u8>,
    space: &GraphSpace,
    values: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> Result<(), GraphError> {
    let prefix = key_prefix(space);
    let mut table = write.open_table(definition).map_err(storage)?;
    table
        .retain(|key, _| !key.starts_with(&prefix))
        .map_err(storage)?;
    for (first, seconds) in values {
        for second in seconds {
            let key = index_key(space, first, second);
            table.insert(key.as_str(), 0).map_err(storage)?;
        }
    }
    Ok(())
}

fn read_commits(
    read: &redb::ReadTransaction,
    space: &GraphSpace,
) -> Result<CommitState, GraphError> {
    let table = read.open_table(COMMITS).map_err(|_| Failure::Corrupt)?;
    let bytes = table.get(space.as_str()).map_err(storage)?;
    if let Some(bytes) = bytes {
        if read
            .open_table(SPACES)
            .map_err(storage)?
            .get(space.as_str())
            .map_err(storage)?
            .is_none()
        {
            return Err(Failure::Corrupt.into());
        }
        if bytes.value().len() > 96 * 1024 * 1024 {
            return Err(Failure::Capacity.into());
        }
        let state: CommitState =
            serde_json::from_slice(bytes.value()).map_err(|_| Failure::Corrupt)?;
        state.validate()?;
        let revision = read
            .open_table(SPACES)
            .map_err(storage)?
            .get(space.as_str())
            .map_err(storage)?
            .ok_or(Failure::Corrupt)?
            .value();
        if state.head_revision(space.as_str()).sequence != revision {
            return Err(Failure::Corrupt.into());
        }
        Ok(state)
    } else {
        let spaces = read.open_table(SPACES).map_err(storage)?;
        if spaces.get(space.as_str()).map_err(storage)?.is_some() {
            return Err(Failure::Corrupt.into());
        }
        // An absent space is empty; this read creates no tables, files or records.
        Ok(CommitState::default())
    }
}
