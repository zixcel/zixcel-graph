//! Explicit Graph lifecycle policy. Never invoked by a query or a Scene.
use crate::{Graph, GraphError, backend::RedbStorage};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use zixcel_revision::Failure;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoragePhase {
    Opening,
    RecoveryRequired,
    Recovering,
    Ready,
    Unavailable,
}

pub struct StartedGraph {
    pub graph: Graph,
    pub recovery_performed: bool,
    pub recovery_duration: Duration,
}

impl Graph {
    /// Explicit owner startup, distinct from `open_existing` and ordinary reads.
    /// Recovers only physical `RecoveryRequired` state, validates all logical
    /// spaces, closes the writer, then reopens read-only before reporting Ready.
    /// # Errors
    /// Missing/corrupt/unsupported state and lock contention remain unavailable;
    /// no database, table, space, revision or domain record is created/repaired.
    pub fn start_existing(
        path: impl AsRef<Path>,
        mut observe: impl FnMut(StoragePhase),
    ) -> Result<StartedGraph, GraphError> {
        observe(StoragePhase::Opening);
        let result = (|| {
            let path = existing_path(path.as_ref())?;
            let _owner = exclusive_lease(&path)?;
            let mut recovery_performed = false;
            let mut recovery_duration = Duration::ZERO;
            let backend = match RedbStorage::open_at_startup(&path) {
                Ok(backend) => backend,
                Err(GraphError::Commit(Failure::RecoveryRequired)) => {
                    observe(StoragePhase::RecoveryRequired);
                    observe(StoragePhase::Recovering);
                    let start = Instant::now();
                    let recovered = RedbStorage::recover_at_startup(&path)?;
                    recovered.validate_all()?;
                    drop(recovered);
                    recovery_duration = start.elapsed();
                    recovery_performed = true;
                    RedbStorage::open_at_startup(&path)?
                }
                Err(error) => return Err(error),
            };
            backend.validate_all()?;
            Ok(StartedGraph {
                graph: Graph {
                    storage: Arc::new(backend),
                },
                recovery_performed,
                recovery_duration,
            })
        })();
        observe(if result.is_ok() {
            StoragePhase::Ready
        } else {
            StoragePhase::Unavailable
        });
        result
    }
}

fn existing_path(path: &Path) -> Result<PathBuf, GraphError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GraphError::Missing
        } else {
            crate::error::storage(error)
        }
    })?;
    if !metadata.file_type().is_file() {
        return Err(GraphError::InvalidPath(path.into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // Aliased database names must not manufacture independent lifecycle leases.
        if metadata.nlink() != 1 {
            return Err(GraphError::InvalidPath(path.into()));
        }
    }
    std::fs::canonicalize(path).map_err(crate::error::storage)
}
fn lease_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".lifecycle.lock");
    PathBuf::from(name)
}
fn validate_lease(path: &Path, file: &File) -> Result<(), GraphError> {
    let named = std::fs::symlink_metadata(path).map_err(crate::error::storage)?;
    let opened = file.metadata().map_err(crate::error::storage)?;
    if !named.is_file() || !opened.is_file() || opened.len() != 0 {
        return Err(GraphError::InvalidPath(path.into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if named.ino() != opened.ino() || named.dev() != opened.dev() || opened.nlink() != 1 {
            return Err(GraphError::InvalidPath(path.into()));
        }
    }
    Ok(())
}
pub(crate) fn exclusive_lease(path: &Path) -> Result<File, GraphError> {
    let path = lease_path(&existing_path(path)?);
    // Empty persistent lock inode only: no lifecycle state or recovery journal.
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if !std::fs::symlink_metadata(&path)
                .map_err(crate::error::storage)?
                .is_file()
            {
                return Err(GraphError::InvalidPath(path));
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .map_err(crate::error::storage)?
        }
        Err(e) => return Err(crate::error::storage(e)),
    };
    validate_lease(&path, &file)?;
    file.try_lock().map_err(|e| match e {
        std::fs::TryLockError::WouldBlock => GraphError::Unavailable,
        std::fs::TryLockError::Error(e) => crate::error::storage(e),
    })?;
    Ok(file)
}
pub(crate) fn read_lease(path: &Path) -> Result<Option<File>, GraphError> {
    let path = lease_path(&existing_path(path)?);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(crate::error::storage(e)),
    };
    validate_lease(&path, &file)?;
    file.try_lock_shared().map_err(|e| match e {
        std::fs::TryLockError::WouldBlock => GraphError::Unavailable,
        std::fs::TryLockError::Error(e) => crate::error::storage(e),
    })?;
    Ok(Some(file))
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
