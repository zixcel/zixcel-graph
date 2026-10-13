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
    pub(crate) read_retention: Option<Arc<crate::read_retention::Retention>>,
}

impl Graph {
    /// Configure per-read admission and retained snapshot count for this handle and subsequent clones.
    /// Previously created unconfigured handles, writer staging and caller-owned query outputs are outside this policy.
    /// # Errors
    /// Refuses zero/overflowing limits and reconfiguration of an already configured family.
    pub fn with_read_limits(mut self, limits: crate::ReadLimits) -> Result<Self, GraphError> {
        if self.read_retention.is_some() {
            return Err(GraphError::Invalid("read limits already configured".into()));
        }
        self.read_retention = Some(crate::read_retention::Retention::new(limits)?);
        Ok(self)
    }
    /// Number of retained or in-progress reads admitted by this configured handle family.
    #[must_use]
    pub fn retained_reads(&self) -> Option<usize> {
        self.read_retention
            .as_ref()
            .map(|retention| retention.count())
    }

    /// Explicit physical redb recovery. Does not initialize missing graph data or tables.
    /// # Errors
    /// Missing/invalid storage remains an error; domain state is never reconstructed.
    #[cfg(feature = "redb")]
    pub fn recover_existing(path: impl AsRef<Path>) -> Result<Self, GraphError> {
        Ok(Self {
            storage: Arc::new(RedbStorage::recover_existing(path.as_ref())?),
            read_retention: None,
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
            read_retention: None,
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
            read_retention: None,
        })
    }

    /// Open existing regular storage under explicit file-length and logical validation limits.
    /// No creation, recovery or memory fallback occurs. This does not bound backend cache/RSS.
    /// # Errors
    /// Rejects cancellation, missing/symlinked paths, file-length or validation capacity overflow.
    #[cfg(feature = "redb")]
    pub fn open_existing_budgeted(
        path: impl AsRef<Path>,
        budget: crate::StorageOpeningBudget,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<Self, GraphError> {
        crate::storage_budget::check_cancelled(cancelled)?;
        let metadata = std::fs::symlink_metadata(path.as_ref()).map_err(crate::error::storage)?;
        if !metadata.file_type().is_file() {
            return Err(GraphError::InvalidPath(path.as_ref().to_owned()));
        }
        if metadata.len() > budget.max_file_bytes {
            return Err(zixcel_revision::Failure::Capacity.into());
        }
        let graph = Self::open_existing(path)?;
        graph.validate_storage_budgeted(budget.validation, cancelled)?;
        Ok(graph)
    }

    /// Creates an isolated in-memory graph with the same transactional semantics.
    ///
    /// # Errors
    /// Reserved for backend initialization failures.
    pub fn memory() -> Result<Self, GraphError> {
        Ok(Self {
            storage: Arc::new(MemoryStorage::new()),
            read_retention: None,
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
        if let Some(retention) = &self.read_retention {
            return self.read_budgeted(space, retention.limits.snapshot);
        }
        Ok(ReadTransaction::new(self.storage.load(space)?))
    }

    /// Materialize one space only within explicit record/index/source-byte limits.
    /// Redb checks stored bytes before decoding; memory checks before cloning.
    /// # Errors
    /// Exceeding a limit returns an error, never a partial snapshot or fallback.
    /// This bounds input size/counts, not allocator overhead or total process RSS.
    pub fn read_budgeted(
        &self,
        space: &GraphSpace,
        budget: crate::ReadBudget,
    ) -> Result<ReadTransaction, GraphError> {
        let lease = self
            .read_retention
            .as_ref()
            .map(crate::read_retention::Retention::acquire)
            .transpose()?;
        let budget = self.read_retention.as_ref().map_or(budget, |retention| {
            crate::read_retention::intersect(budget, retention.limits.snapshot)
        });
        let mut result = ReadTransaction::new(self.storage.load_budgeted(space, budget)?);
        result.retain(lease);
        Ok(result)
    }

    /// Read data and its covering canonical revision from the same physical snapshot.
    /// # Errors
    /// Invalid/unavailable state is never initialized or repaired.
    pub fn read_exact(
        &self,
        space: &GraphSpace,
    ) -> Result<(ReadTransaction, zixcel_revision::RevisionRef), GraphError> {
        if let Some(retention) = &self.read_retention {
            return self.read_exact_budgeted(space, retention.limits.snapshot);
        }
        let snapshot = self.storage.snapshot(space)?;
        let revision = snapshot.commits.head_revision(space.as_str());
        Ok((ReadTransaction::new(snapshot.graph), revision))
    }

    /// Read bounded data and canonical revision from the same physical snapshot.
    /// # Errors
    /// Capacity exhaustion returns no data or revision and never repairs storage.
    /// Limits cover source bytes/records, not process RSS or retained snapshots.
    pub fn read_exact_budgeted(
        &self,
        space: &GraphSpace,
        budget: crate::ReadBudget,
    ) -> Result<(ReadTransaction, zixcel_revision::RevisionRef), GraphError> {
        let lease = self
            .read_retention
            .as_ref()
            .map(crate::read_retention::Retention::acquire)
            .transpose()?;
        let budget = self.read_retention.as_ref().map_or(budget, |retention| {
            crate::read_retention::intersect(budget, retention.limits.snapshot)
        });
        let snapshot = self.storage.snapshot_budgeted(space, budget)?;
        let revision = snapshot.commits.head_revision(space.as_str());
        let mut result = ReadTransaction::new(snapshot.graph);
        result.retain(lease);
        Ok((result, revision))
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

    /// Begin a write with bounded snapshot and cumulative mutation admission.
    /// # Errors
    /// Oversized existing state is rejected before cloning/decoding. Later mutation
    /// refusal poisons the whole transaction; no prefix is committed.
    pub fn write_budgeted(
        &self,
        space: &GraphSpace,
        budget: crate::WriteBudget,
    ) -> Result<crate::BudgetedWriteTransaction, GraphError> {
        let snapshot = self.storage.snapshot_budgeted(space, budget.snapshot)?;
        let mut transaction =
            WriteTransaction::new(Arc::clone(&self.storage), space.clone(), snapshot);
        transaction.budget = Some(budget.snapshot);
        Ok(crate::BudgetedWriteTransaction::new(transaction, budget))
    }

    /// Validate all existing spaces within one cumulative storage budget.
    /// # Errors
    /// Quota exhaustion/cancellation returns no acceptance and never repairs data.
    /// Checks cancellation between spaces; it does not interrupt a single record
    /// decode or backend lock acquisition. No cache/RSS or physical disk bound is implied.
    pub fn validate_storage_budgeted(
        &self,
        budget: crate::StorageValidationBudget,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(), GraphError> {
        self.storage.validate_budgeted(budget, cancelled)
    }

    /// Rebuilds derived indexes atomically from primary node and edge records.
    ///
    /// # Errors
    /// Returns an error when primary records are invalid or storage cannot commit.
    pub fn rebuild_indexes(&self, space: &GraphSpace) -> Result<(), GraphError> {
        self.storage.rebuild_indexes(space)
    }
}
