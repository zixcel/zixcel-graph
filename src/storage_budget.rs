//! Whole-storage validation admission, independent of product startup/authority.
use crate::{GraphError, ReadBudget};
use std::sync::atomic::{AtomicBool, Ordering};
#[derive(Debug, Clone, Copy)]
pub struct StorageValidationBudget {
    pub max_spaces: usize,
    /// Cumulative over all validated spaces, including canonical receipt images.
    pub records: ReadBudget,
}
pub(crate) fn check_cancelled(cancelled: &AtomicBool) -> Result<(), GraphError> {
    if cancelled.load(Ordering::Relaxed) {
        Err(GraphError::Invalid("storage validation cancelled".into()))
    } else {
        Ok(())
    }
}
