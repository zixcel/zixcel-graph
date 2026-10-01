//! A borrowed Graph-space retention port. No additional state or authority.
use crate::{Graph, GraphError, GraphSpace};
use zixcel_revision::{
    CommitReceipt, ExternalObjectRef, ExternalRegistration, ExternalRegistrationStatus,
    ExternalRetention, ExternalRoot, Failure, ReclamationPermit, Rejection, RetirementReceipt,
};

pub struct GraphExternalRetention<'a> {
    pub(crate) graph: &'a Graph,
    pub(crate) space: &'a GraphSpace,
}
fn failure(error: GraphError) -> Failure {
    match error {
        // The retention port operates on an already opened Graph. Its generic
        // failure contract has no startup lifecycle state; never call recovery.
        GraphError::Commit(error) => error,
        GraphError::Missing
        | GraphError::Unavailable
        | GraphError::Storage(_)
        | GraphError::InvalidPath(_) => Failure::Storage,
        GraphError::DependencyConflict { .. } | GraphError::Invalid(_) => {
            Failure::Rejected(Rejection::InvalidIntent)
        }
        GraphError::Codec(_)
        | GraphError::UnsupportedVersion
        | GraphError::MissingNode(_)
        | GraphError::CommitConflict(_)
        | GraphError::RevisionConflict { .. } => Failure::Corrupt,
    }
}
impl Graph {
    /// A read/lifecycle-only view. Merely constructing it performs no IO.
    #[must_use]
    pub fn external_retention<'a>(&'a self, space: &'a GraphSpace) -> GraphExternalRetention<'a> {
        GraphExternalRetention { graph: self, space }
    }
}
impl ExternalRetention for GraphExternalRetention<'_> {
    fn register_committed_external(
        &self,
        receipt: &CommitReceipt,
        objects: &[ExternalObjectRef],
    ) -> Result<Vec<ExternalRegistration>, Failure> {
        self.graph
            .register_committed_external(self.space, receipt, objects)
            .map_err(failure)
    }
    fn external_registration(
        &self,
        object: &ExternalObjectRef,
        prepared: &str,
    ) -> Result<Option<ExternalRegistration>, Failure> {
        self.graph
            .external_registration(self.space, object, prepared)
            .map_err(failure)
    }
    fn registration_status(
        &self,
        registration: &ExternalRegistration,
    ) -> Result<ExternalRegistrationStatus, Failure> {
        self.graph
            .registration_status(self.space, registration)
            .map_err(failure)
    }
    fn retire_external_registration(
        &self,
        registration: &ExternalRegistration,
    ) -> Result<RetirementReceipt, Failure> {
        self.graph
            .retire_external_registration(self.space, registration)
            .map_err(failure)
    }
    fn retain_registration(
        &self,
        registration: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        self.graph
            .retain_registration(self.space, registration, root)
            .map_err(failure)
    }
    fn release_registration(
        &self,
        registration: &ExternalRegistration,
        root: &ExternalRoot,
    ) -> Result<(), Failure> {
        self.graph
            .release_registration(self.space, registration, root)
            .map_err(failure)
    }
    fn claim_external_reclamation(
        &self,
        object: &ExternalObjectRef,
        epoch: u64,
    ) -> Result<Option<ReclamationPermit>, Failure> {
        self.graph
            .claim_external_reclamation(self.space, object, epoch)
            .map_err(failure)
    }
    fn complete_external_reclamation(&self, permit: &ReclamationPermit) -> Result<(), Failure> {
        self.graph
            .complete_external_reclamation(self.space, permit)
            .map_err(failure)
    }
}
