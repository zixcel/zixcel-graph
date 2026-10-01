use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("Graph source dependency changed: {space:?}, expected {expected:?}, actual {actual:?}")]
    DependencyConflict {
        space: crate::GraphSpace,
        expected: zixcel_revision::RevisionRef,
        actual: zixcel_revision::RevisionRef,
    },
    #[error("graph database is missing")]
    Missing,
    #[error("graph storage lifecycle is owned by another caller")]
    Unavailable,
    #[error("commit conflict: {0:?}")]
    CommitConflict(zixcel_revision::Conflict),
    #[error(transparent)]
    Commit(#[from] zixcel_revision::Failure),
    #[error("invalid graph value: {0}")]
    Invalid(String),
    #[error("graph storage failed: {0}")]
    Storage(String),
    #[error("graph codec failed: {0}")]
    Codec(String),
    #[error("graph file has an unsupported logical version")]
    UnsupportedVersion,
    #[error("graph write revision conflict: expected {expected}, current {current}")]
    RevisionConflict { expected: u64, current: u64 },
    #[error("graph edge references a missing node: {0}")]
    MissingNode(String),
    #[error("graph path is not a regular file: {0}")]
    InvalidPath(PathBuf),
}

#[cfg(feature = "redb")]
pub(crate) fn storage(_error: impl std::fmt::Display) -> GraphError {
    zixcel_revision::Failure::Storage.into()
}

#[cfg(feature = "redb")]
pub(crate) fn codec(_error: impl std::fmt::Display) -> GraphError {
    zixcel_revision::Failure::Corrupt.into()
}
