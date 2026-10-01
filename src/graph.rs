#[cfg(feature = "redb")]
use crate::backend::RedbStorage;
use crate::backend::{MemoryStorage, Storage};
use crate::{GraphError, GraphMetadata, GraphSpace, ReadTransaction, WriteTransaction};
#[cfg(feature = "redb")]
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct Graph {
    pub(crate) storage: Arc<dyn Storage>,
}

impl Graph {
    /// Explicit physical redb recovery. Does not initialize missing graph data or tables.
    /// # Errors
    /// Missing/invalid storage remains an error; domain state is never reconstructed.
    #[cfg(feature = "redb")]
    pub fn recover_existing(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        Ok(Self {
            storage: Arc::new(RedbStorage::recover_existing(path.as_ref())?),
        })
    }
    /// Opens an existing durable graph or explicitly creates a new database.
    /// Existing images are validated, never initialized or repaired.
    ///
    /// # Errors
    /// Returns an error when the path, database image, or logical versions are invalid.
    #[cfg(feature = "redb")]
    pub fn create(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        let storage = RedbStorage::create(path.as_ref())?;
        storage.metadata()?;
        Ok(Self {
            storage: Arc::new(storage),
        })
    }

    /// Opens an existing database without creating directories, data or tables.
    ///
    /// # Errors
    /// Returns an error for an absent, symlinked, incomplete or invalid database.
    #[cfg(feature = "redb")]
    pub fn open_existing(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        Ok(Self {
            storage: Arc::new(RedbStorage::open_existing(path.as_ref())?),
        })
    }

    /// Creates an isolated in-memory graph with the same transactional semantics.
    ///
    /// # Errors
    /// Reserved for backend initialization failures.
    pub fn memory() -> Result<Self, GraphError> {
        Ok(Self {
            storage: Arc::new(MemoryStorage::new()),
        })
    }

    /// Reads logical database and backend metadata.
    ///
    /// # Errors
    /// Returns an error when metadata cannot be read or uses unsupported versions.
    pub fn metadata(&self) -> Result<GraphMetadata, GraphError> {
        self.storage.metadata()
    }

    /// Opens an immutable point-in-time snapshot for one graph space.
    ///
    /// # Errors
    /// Returns an error when primary records or derived indexes are invalid.
    pub fn read(&self, space: &GraphSpace) -> Result<ReadTransaction, GraphError> {
        Ok(ReadTransaction::new(self.storage.load(space)?))
    }

    /// Read data and its covering canonical revision from the same physical snapshot.
    /// # Errors
    /// Invalid/unavailable state is never initialized or repaired.
    pub fn read_exact(
        &self,
        space: &GraphSpace,
    ) -> Result<(ReadTransaction, zixcel_revision::RevisionRef), GraphError> {
        let snapshot = self.storage.snapshot(space)?;
        let revision = snapshot.commits.head_revision(space.as_str());
        Ok((ReadTransaction::new(snapshot.graph), revision))
    }

    /// Opens an optimistic write transaction for one graph space.
    ///
    /// # Errors
    /// Returns an error when the current graph image cannot be read.
    pub fn write(&self, space: &GraphSpace) -> Result<WriteTransaction, GraphError> {
        let state = self.storage.snapshot(space)?;
        Ok(WriteTransaction::new(
            Arc::clone(&self.storage),
            space.clone(),
            state,
        ))
    }

    /// Rebuilds derived indexes atomically from primary node and edge records.
    ///
    /// # Errors
    /// Returns an error when primary records are invalid or storage cannot commit.
    pub fn rebuild_indexes(&self, space: &GraphSpace) -> Result<(), GraphError> {
        self.storage.rebuild_indexes(space)
    }
}
