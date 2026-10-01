// Hatter reconstruction 2026: Graph-owned atomic reference and Foundation receipt publication.
// Domain authorization/result encoding remain with callers. No semantic or Hatter types.
use crate::backend::StoredSpace;
use crate::{Edge, Graph, GraphError, GraphSpace, Node, ReadTransaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::sync::Mutex;
use zixcel_revision::{
    Backend, CommitIntent, CommitOutcome, CommitState, CommitStore, Failure, OperationId,
    Rejection, RevisionRef,
};

/// Generic control input. The caller supplies the complete typed command to
/// fingerprint; presentation labels and semantic policy remain caller concerns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationIntent {
    pub operation_id: OperationId,
    pub expected_commit_revision: RevisionRef,
}
impl PublicationIntent {
    /// Fingerprints the complete typed input without allocating a serialized body.
    /// This fingerprint identifies request equality, never the CAS base revision.
    /// # Errors
    /// Invalid encoding or inputs exceeding the Foundation payload bound reject.
    pub fn request(
        &self,
        kind: &str,
        input: &impl Serialize,
    ) -> Result<PublicationRequest, GraphError> {
        struct Fingerprint {
            digest: Sha256,
            bytes: usize,
        }
        impl Write for Fingerprint {
            fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
                if value.len() > zixcel_revision::MAX_PAYLOAD_BYTES.saturating_sub(self.bytes) {
                    return Err(std::io::Error::other("publication input capacity exceeded"));
                }
                self.bytes += value.len();
                self.digest.update(value);
                Ok(value.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Fingerprint {
            digest: Sha256::new(),
            bytes: 0,
        };
        serde_json::to_writer(&mut writer, &(kind, input))
            .map_err(|_| GraphError::Invalid("publication input is invalid or too large".into()))?;
        Ok(PublicationRequest {
            operation_id: self.operation_id.clone(),
            expected_commit_revision: self.expected_commit_revision.clone(),
            request_digest: format!("{:x}", writer.digest.finalize()),
            dependencies: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationRequest {
    pub operation_id: OperationId,
    pub expected_commit_revision: RevisionRef,
    /// Caller-owned canonical input fingerprint. It is NOT a concurrency revision.
    pub request_digest: String,
    pub dependencies: Vec<GraphReadDependency>,
}

/// Exact source snapshot validation, never ownership of source meaning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphReadDependency {
    pub space: GraphSpace,
    pub expected_revision: RevisionRef,
}
pub const MAX_READ_DEPENDENCIES: usize = zixcel_revision::MAX_PARENTS;
impl PublicationRequest {
    /// Bind a bounded canonical dependency set once, including it in request identity.
    /// # Errors
    /// Duplicate spaces, invalid revisions, rebinding and excess dependencies reject.
    pub fn with_dependencies(
        mut self,
        mut dependencies: Vec<GraphReadDependency>,
    ) -> Result<Self, GraphError> {
        if !self.dependencies.is_empty() {
            return Err(GraphError::Invalid("dependency rebinding".into()));
        }
        dependencies.sort_by(|a, b| a.space.as_str().cmp(b.space.as_str()));
        validate_dependencies(&dependencies, None)?;
        if !dependencies.is_empty() {
            let bytes = serde_json::to_vec(&(
                "graph/read/dependencies",
                &self.request_digest,
                &dependencies,
            ))
            .map_err(|_| Failure::Corrupt)?;
            self.request_digest = format!("{:x}", Sha256::digest(bytes));
        }
        self.dependencies = dependencies;
        Ok(self)
    }
}
fn validate_dependencies(
    dependencies: &[GraphReadDependency],
    target: Option<&GraphSpace>,
) -> Result<(), GraphError> {
    if dependencies.len() > MAX_READ_DEPENDENCIES
        || dependencies
            .windows(2)
            .any(|w| w[0].space.as_str() >= w[1].space.as_str())
    {
        return Err(GraphError::Invalid("invalid dependency set".into()));
    }
    for d in dependencies {
        GraphSpace::new(d.space.as_str())?;
        if target == Some(&d.space)
            || d.expected_revision.commit.as_ref().is_some_and(|c| {
                c.len() != 64
                    || !c
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            || (d.expected_revision.sequence == 0) != d.expected_revision.commit.is_none()
        {
            return Err(GraphError::Invalid(
                "invalid dependency revision or target space".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphChanges {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub remove_nodes: Vec<String>,
    pub remove_edges: Vec<String>,
    /// Bounded owner result references, not a copy of external domain memory.
    pub output: Vec<u8>,
}

#[derive(Debug)]
pub struct GraphPublication {
    pub outcome: CommitOutcome,
    /// Exact original owner result only for Committed/NoChange.
    pub output: Option<Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    request_digest: String,
    dependencies: Vec<GraphReadDependency>,
    changes: GraphChanges,
}

impl Graph {
    /// Bounded identities of existing noncanonical preparations, never a head
    /// selection or a cleanup operation.
    /// # Errors
    /// Missing/corrupt storage is not an empty proposal set.
    pub fn prepared_references(&self, space: &GraphSpace) -> Result<Vec<String>, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        Ok(CommitStore::new(&backend).prepared_references(space.as_str())?)
    }
    /// Resolve an original staged or committed request by its exact operation.
    /// Only immutable control references are returned, never a selected latest
    /// revision. Owners must compare the current formal input fingerprint before
    /// continuing; this lookup alone does not authorize publication.
    /// # Errors
    /// Invalid stored publication or conflicting operation identities reject.
    pub fn original_publication_request(
        &self,
        space: &GraphSpace,
        operation: &str,
    ) -> Result<Option<PublicationRequest>, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        let store = CommitStore::new(&backend);
        let prepared = if let Some(receipt) = store.receipt(space.as_str(), operation)? {
            Some(
                store
                    .committed(&receipt.commit_ref)?
                    .ok_or(Failure::Corrupt)?,
            )
        } else {
            find_prepared(&store, space, operation)?
        };
        prepared
            .map(|item| {
                let original: Publication =
                    serde_json::from_slice(item.payload()).map_err(|_| Failure::Corrupt)?;
                Ok(PublicationRequest {
                    operation_id: item.operation_id().into(),
                    expected_commit_revision: item.base_revision().clone(),
                    request_digest: original.request_digest,
                    dependencies: original.dependencies,
                })
            })
            .transpose()
    }
    /// Read original noncanonical changes for the exact formal request.
    /// This does not reserve, publish, revive or authorize a preparation. Owners
    /// must revalidate/stage before persisting their corresponding external data.
    /// A lost preparation reply must not cause owner-generated values to change.
    /// # Errors
    /// Substituted requests and corrupt/unavailable storage remain failures.
    pub fn prepared_changes(
        &self,
        space: &GraphSpace,
        request: &PublicationRequest,
    ) -> Result<Option<(String, GraphChanges)>, GraphError> {
        validate_dependencies(&request.dependencies, Some(space))?;
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        let store = CommitStore::new(&backend);
        let Some(prepared) = find_prepared(&store, space, &request.operation_id)? else {
            return Ok(None);
        };
        let original: Publication =
            serde_json::from_slice(prepared.payload()).map_err(|_| Failure::Corrupt)?;
        if prepared.base_revision() != &request.expected_commit_revision
            || original.request_digest != request.request_digest
            || original.dependencies != request.dependencies
        {
            return Err(Failure::Rejected(Rejection::InvalidIntent).into());
        }
        Ok(Some((prepared.reference().into(), original.changes)))
    }
    /// Pure exact registration inspection; never creates a Graph space.
    /// # Errors
    /// Missing/corrupt storage or malformed owner references remain explicit.
    pub fn external_registration(
        &self,
        space: &GraphSpace,
        object: &zixcel_revision::ExternalObjectRef,
        prepared: &str,
    ) -> Result<Option<zixcel_revision::ExternalRegistration>, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        Ok(CommitStore::new(&backend).external_registration(object, prepared)?)
    }
    /// Pure exact generation status, separate from domain publication state.
    /// # Errors
    /// Unknown/substituted generations and backend failures reject.
    pub fn registration_status(
        &self,
        space: &GraphSpace,
        registration: &zixcel_revision::ExternalRegistration,
    ) -> Result<zixcel_revision::ExternalRegistrationStatus, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        Ok(CommitStore::new(&backend).registration_status(registration)?)
    }
    /// ZG1 owner-controlled lifecycle metadata; no domain publication.
    /// # Errors
    /// Unknown, stale, reclaiming or excessive registrations fail closed.
    pub fn register_committed_external(
        &self,
        space: &GraphSpace,
        receipt: &zixcel_revision::CommitReceipt,
        objects: &[zixcel_revision::ExternalObjectRef],
    ) -> Result<Vec<zixcel_revision::ExternalRegistration>, GraphError> {
        self.change_lifecycle(space, |s| s.register_committed_external(receipt, objects))
    }
    /// ZG1 owner-controlled lifecycle metadata; no domain publication.
    /// # Errors
    /// Unknown, stale, reclaiming or excessive registrations fail closed.
    pub fn retire_external_registration(
        &self,
        space: &GraphSpace,
        registration: &zixcel_revision::ExternalRegistration,
    ) -> Result<zixcel_revision::RetirementReceipt, GraphError> {
        self.change_lifecycle(space, |s| s.retire_external_registration(registration))
    }
    /// ZG1 owner-controlled lifecycle metadata; no domain publication.
    /// # Errors
    /// Unknown, stale, reclaiming or excessive registrations fail closed.
    pub fn reregister_external(
        &self,
        space: &GraphSpace,
        registration: &zixcel_revision::ExternalRegistration,
    ) -> Result<zixcel_revision::ExternalRegistration, GraphError> {
        self.change_lifecycle(space, |s| s.reregister_external(registration))
    }
    /// ZG1 owner-controlled lifecycle metadata; no domain publication.
    /// # Errors
    /// Unknown, stale, reclaiming or excessive registrations fail closed.
    pub fn retain_registration(
        &self,
        space: &GraphSpace,
        registration: &zixcel_revision::ExternalRegistration,
        root: &zixcel_revision::ExternalRoot,
    ) -> Result<(), GraphError> {
        self.change_lifecycle(space, |s| s.retain_registration(registration, root))
    }
    /// ZG1 owner-controlled lifecycle metadata; no domain publication.
    /// # Errors
    /// Unknown, stale, reclaiming or excessive registrations fail closed.
    pub fn release_registration(
        &self,
        space: &GraphSpace,
        registration: &zixcel_revision::ExternalRegistration,
        root: &zixcel_revision::ExternalRoot,
    ) -> Result<(), GraphError> {
        self.change_lifecycle(space, |s| s.release_registration(registration, root))
    }
    /// Durably stage noncanonical graph changes and external ownership references.
    /// External owners create immutable bytes only AFTER this method succeeds.
    /// # Errors
    /// Invalid topology/input, reused preparation and backend failure are preserved.
    pub fn prepare_publication(
        &self,
        space: &GraphSpace,
        request: &PublicationRequest,
        changes: GraphChanges,
        objects: &[zixcel_revision::ExternalObjectRef],
    ) -> Result<zixcel_revision::PreparedCommit, GraphError> {
        validate_dependencies(&request.dependencies, Some(space))?;
        validate_changes(&changes)?;
        let prepared = intent(space, request, changes).map_err(Failure::Rejected)?;
        self.change_lifecycle(space, |store| {
            if objects.is_empty() {
                store.stage(&prepared)
            } else {
                store.reserve_external(&prepared, objects)
            }
        })?;
        Ok(prepared)
    }
    /// Publish a durable Prepared after its owner has made required objects durable.
    /// No external callback or distributed transaction is implied.
    /// # Errors
    /// Abandoned, substituted, unavailable or stale preparations fail closed.
    pub fn publish_prepared(
        &self,
        space: &GraphSpace,
        prepared: &zixcel_revision::PreparedCommit,
    ) -> Result<GraphPublication, GraphError> {
        if prepared.domain() != space.as_str() {
            return Err(Failure::Rejected(Rejection::InvalidIntent).into());
        }
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        let store = CommitStore::new(&backend);
        let known = store
            .prepared(prepared.reference())?
            .or(store.committed_by_prepared(prepared.reference())?);
        if known.as_ref() != Some(prepared) {
            return Err(Failure::Rejected(Rejection::NotPrepared).into());
        }
        let original: Publication =
            serde_json::from_slice(prepared.payload()).map_err(|_| Failure::Corrupt)?;
        let request = PublicationRequest {
            operation_id: prepared.operation_id().into(),
            expected_commit_revision: prepared.base_revision().clone(),
            request_digest: original.request_digest,
            dependencies: original.dependencies,
        };
        self.publish(space, &request, |_| Ok(original.changes), || Ok(()))
    }
    /// # Errors
    /// Reads only existing noncanonical state; missing/corrupt backend remains an error.
    pub fn prepared_publication(
        &self,
        space: &GraphSpace,
        reference: &str,
    ) -> Result<Option<zixcel_revision::PreparedCommit>, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        Ok(CommitStore::new(&backend).prepared(reference)?)
    }
    /// # Errors
    /// Only existing noncanonical preparations may be explicitly abandoned.
    pub fn abandon_prepared(
        &self,
        space: &GraphSpace,
        reference: &str,
        epoch: u64,
    ) -> Result<(), GraphError> {
        self.change_lifecycle(space, |s| s.abandon(reference, epoch))
    }
    /// # Errors
    /// New protection is rejected once owner reclamation has been fenced.
    pub fn retain_external(
        &self,
        space: &GraphSpace,
        object: &zixcel_revision::ExternalObjectRef,
        root: &zixcel_revision::ExternalRoot,
    ) -> Result<(), GraphError> {
        self.change_lifecycle(space, |s| s.retain_external(object, root))
    }
    /// # Errors
    /// Unknown objects and fenced reclamation reject without releasing other roots.
    pub fn release_external(
        &self,
        space: &GraphSpace,
        object: &zixcel_revision::ExternalObjectRef,
        root: &zixcel_revision::ExternalRoot,
    ) -> Result<(), GraphError> {
        self.change_lifecycle(space, |s| s.release_external(object, root))
    }
    /// # Errors
    /// Owner claim does not delete foreign bytes. Backend failures retain eligibility.
    pub fn claim_external_reclamation(
        &self,
        space: &GraphSpace,
        object: &zixcel_revision::ExternalObjectRef,
        epoch: u64,
    ) -> Result<Option<zixcel_revision::ReclamationPermit>, GraphError> {
        self.change_lifecycle(space, |s| s.claim_external_reclamation(object, epoch))
    }
    /// # Errors
    /// Only an exact owner permit may acknowledge physical reclamation.
    pub fn complete_external_reclamation(
        &self,
        space: &GraphSpace,
        permit: &zixcel_revision::ReclamationPermit,
    ) -> Result<(), GraphError> {
        self.change_lifecycle(space, |s| s.complete_external_reclamation(permit))
    }
    /// # Errors
    /// Reclaims only Foundation-owned abandoned preparations, never external objects.
    pub fn reclaim_prepared(&self, space: &GraphSpace, epoch: u64) -> Result<usize, GraphError> {
        self.change_lifecycle(space, |s| s.reclaim(epoch))
    }
    fn change_lifecycle<T>(
        &self,
        space: &GraphSpace,
        change: impl FnOnce(&CommitStore<&ImageBackend>) -> Result<T, Failure>,
    ) -> Result<T, GraphError> {
        let mut result = None;
        self.storage.lifecycle(
            space,
            Box::new(|state| {
                let backend = ImageBackend(Mutex::new(state.clone()));
                result = Some(change(&CommitStore::new(&backend))?);
                *state = backend.0.into_inner().map_err(|_| Failure::Storage)?;
                Ok(())
            }),
        )?;
        result.ok_or_else(|| Failure::Corrupt.into())
    }
    /// Reads the Foundation revision without initializing a space.
    /// # Errors
    /// Missing/corrupt storage preserves its actual error.
    pub fn commit_revision(&self, space: &GraphSpace) -> Result<RevisionRef, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        let store = CommitStore::new(&backend);
        Ok(store
            .current(space.as_str())?
            .map(|p| {
                store
                    .receipt(space.as_str(), p.operation_id())
                    .and_then(|r| r.ok_or(Failure::Corrupt))
                    .map(|r| r.committed_revision)
            })
            .transpose()?
            .unwrap_or_default())
    }

    /// Checks original acceptance or rejects a new stale request before callers restore owner state or authorize again.
    /// No write transaction, initialization or owner callback is performed.
    /// # Errors
    /// Corrupt or unavailable commit storage never becomes an unknown operation.
    pub fn replay(
        &self,
        space: &GraphSpace,
        request: &PublicationRequest,
    ) -> Result<Option<GraphPublication>, GraphError> {
        validate_dependencies(&request.dependencies, Some(space))?;
        let state = self.storage.load_commits(space)?;
        state.validate()?;
        let backend = ImageBackend(Mutex::new(state));
        let store = CommitStore::new(&backend);
        if let Some(original) = replay(&store, space, request)? {
            return Ok(Some(original));
        }
        if let Err(error) = intent(space, request, GraphChanges::default()) {
            return Ok(Some(finished(CommitOutcome::Rejected(error), None)));
        }
        Ok(store
            .check_base(space.as_str(), &request.expected_commit_revision)?
            .map(|c| finished(CommitOutcome::Conflict(c), None)))
    }

    /// Resolve an exact committed identity through its canonical receipt, without
    /// initialization, current-head selection or a parallel source index.
    /// # Errors
    /// Corrupt or unavailable canonical storage preserves its owner error.
    pub fn committed_receipt(
        &self,
        space: &GraphSpace,
        reference: &str,
    ) -> Result<Option<zixcel_revision::CommitReceipt>, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        let store = CommitStore::new(&backend);
        let Some(prepared) = store.committed(reference)? else {
            return Ok(None);
        };
        if prepared.domain() != space.as_str() {
            return Ok(None);
        }
        let receipt = store
            .receipt(space.as_str(), prepared.operation_id())?
            .ok_or(Failure::Corrupt)?;
        if receipt.commit_ref != reference {
            return Err(Failure::Corrupt.into());
        }
        Ok(Some(receipt))
    }

    /// Reads the exact graph mutation payload of an accepted receipt, never current head.
    /// # Errors
    /// An unknown/forged receipt or corrupt publication is rejected.
    pub fn committed_changes(
        &self,
        space: &GraphSpace,
        receipt: &zixcel_revision::CommitReceipt,
    ) -> Result<GraphChanges, GraphError> {
        let backend = ImageBackend(Mutex::new(self.storage.load_commits(space)?));
        let store = CommitStore::new(&backend);
        if receipt.domain != space.as_str()
            || store
                .receipt(space.as_str(), &receipt.operation_id)?
                .as_ref()
                != Some(receipt)
        {
            return Err(Failure::Corrupt.into());
        }
        let original = store
            .committed(&receipt.commit_ref)?
            .ok_or(Failure::Corrupt)?;
        let value: Publication =
            serde_json::from_slice(original.payload()).map_err(|_| Failure::Corrupt)?;
        Ok(value.changes)
    }

    /// Publishes graph references and the exact Foundation receipt in ONE transaction.
    /// The input fingerprint must cover the complete caller command (including Subject,
    /// decisions and semantic references). The caller alone owns those semantics.
    /// Known replay, reuse and stale-base checks precede `calculate`.
    /// `calculate` uses a detached target snapshot, covered by exact target CAS.
    /// Cross-space reads must declare exact dependencies. Callbacks run without storage locks.
    /// `prepare` is bounded immutable durability, not acceptance or mutable owner state.
    /// A losing final CAS may leave inert owner-managed objects. A failed prepare creates no graph commit;
    /// any external orphan remains owned by its repository.
    /// # Errors
    /// Owner validation, graph constraints and storage failures abort atomically.
    pub fn publish<C, P>(
        &self,
        space: &GraphSpace,
        request: &PublicationRequest,
        calculate: C,
        prepare: P,
    ) -> Result<GraphPublication, GraphError>
    where
        C: FnOnce(&ReadTransaction) -> Result<GraphChanges, GraphError>,
        P: FnOnce() -> Result<(), GraphError>,
    {
        if request.request_digest.len() != 64
            || !request
                .request_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Ok(finished(
                CommitOutcome::Rejected(Rejection::InvalidIntent),
                None,
            ));
        }
        if let Some(original) = self.replay(space, request)? {
            return Ok(original);
        }
        // The detached immutable target snapshot is covered by the target base CAS.
        // No consumer closure runs while Graph/revision storage ownership is held.
        let snapshot = self.storage.snapshot(space)?;
        if snapshot.commits.head_revision(space.as_str()) != request.expected_commit_revision {
            return self
                .replay(space, request)?
                .ok_or_else(|| Failure::Corrupt.into());
        }
        let changes = calculate(&ReadTransaction::new(snapshot.graph.clone()))?;
        let mut preview = snapshot;
        let checked = publish_changes(&mut preview, space, request, changes.clone())?;
        if !matches!(checked.outcome, CommitOutcome::Committed(_)) {
            return Ok(checked);
        }
        // Only immutable object preparation belongs here. Domain owners must not
        // treat a callback as canonical acceptance: final CAS may still lose.
        prepare()?;
        self.storage.publish(
            space,
            &request.dependencies,
            Box::new(move |stored, actual| {
                let backend = ImageBackend(Mutex::new(stored.commits.clone()));
                if let Some(original) = replay(&CommitStore::new(&backend), space, request)? {
                    return Ok(original);
                }
                for (dependency, actual) in request.dependencies.iter().zip(actual) {
                    if &dependency.expected_revision != actual {
                        return Err(GraphError::DependencyConflict {
                            space: dependency.space.clone(),
                            expected: dependency.expected_revision.clone(),
                            actual: actual.clone(),
                        });
                    }
                }
                publish_changes(stored, space, request, changes)
            }),
        )
    }
}

fn intent(
    space: &GraphSpace,
    request: &PublicationRequest,
    changes: GraphChanges,
) -> Result<zixcel_revision::PreparedCommit, Rejection> {
    if request.request_digest.len() != 64
        || !request
            .request_digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Rejection::InvalidIntent);
    }
    CommitIntent {
        domain: space.as_str().to_owned(),
        operation_id: request.operation_id.clone(),
        expected_revision: request.expected_commit_revision.clone(),
        parents: request
            .expected_commit_revision
            .commit
            .iter()
            .cloned()
            .collect(),
        payload: Publication {
            request_digest: request.request_digest.clone(),
            dependencies: request.dependencies.clone(),
            changes,
        },
    }
    .prepare_with(|p| encode(&p))
}

fn finished(outcome: CommitOutcome, output: Option<Vec<u8>>) -> GraphPublication {
    let accepted = matches!(
        outcome,
        CommitOutcome::Committed(_) | CommitOutcome::NoChange(_)
    );
    GraphPublication {
        outcome,
        output: if accepted { output } else { None },
    }
}

fn publish_changes(
    stored: &mut StoredSpace,
    space: &GraphSpace,
    request: &PublicationRequest,
    changes: GraphChanges,
) -> Result<GraphPublication, GraphError> {
    stored.commits.validate()?;
    let backend = ImageBackend(Mutex::new(stored.commits.clone()));
    let store = CommitStore::new(&backend);
    if let Some(replayed) = replay(&store, space, request)? {
        return Ok(replayed);
    }
    if let Err(e) = intent(space, request, GraphChanges::default()) {
        return Ok(finished(CommitOutcome::Rejected(e), None));
    }
    if let Some(c) = store.check_base(space.as_str(), &request.expected_commit_revision)? {
        return Ok(finished(CommitOutcome::Conflict(c), None));
    }
    validate_changes(&changes)?;
    let prepared = match intent(space, request, changes.clone()) {
        Ok(p) => p,
        Err(e) => return Ok(finished(CommitOutcome::Rejected(e), None)),
    };
    let mut next = stored.graph.clone();
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or_else(|| GraphError::Invalid("graph revision exhausted".into()))?;
    for id in &changes.remove_edges {
        next.edges.remove(id);
    }
    for id in &changes.remove_nodes {
        next.nodes.remove(id);
    }
    for node in &changes.nodes {
        let mut node = node.clone();
        node.revision = next.revision;
        next.nodes.insert(node.id.clone(), node);
    }
    for edge in &changes.edges {
        let mut edge = edge.clone();
        edge.revision = next.revision;
        next.edges.insert(edge.id.clone(), edge);
    }
    next.rebuild_indexes();
    next.validate()?;
    let outcome = store.commit(&prepared);
    if matches!(outcome, CommitOutcome::Committed(_)) {
        stored.commits = backend.0.into_inner().map_err(|_| Failure::Storage)?;
        stored.graph = next;
    }
    Ok(finished(outcome, Some(changes.output)))
}

// An image adapter inside the owning graph write transaction. The Foundation alone
// interprets revisions/receipts; the graph backend supplies atomic persistence.
struct ImageBackend(Mutex<CommitState>);

fn find_prepared(
    store: &CommitStore<&ImageBackend>,
    space: &GraphSpace,
    operation: &str,
) -> Result<Option<zixcel_revision::PreparedCommit>, GraphError> {
    let mut found = None;
    for reference in store.prepared_references(space.as_str())? {
        let item = store.prepared(&reference)?.ok_or(Failure::Corrupt)?;
        if item.operation_id() == operation {
            if found.is_some() {
                return Err(Failure::Corrupt.into());
            }
            found = Some(item);
        }
    }
    Ok(found)
}
impl Backend for ImageBackend {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        let state = self.0.lock().map_err(|_| Failure::Storage)?;
        Ok(read(&state))
    }
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let mut state = self.0.lock().map_err(|_| Failure::Storage)?;
        let mut next = state.clone();
        let result = write(&mut next)?;
        next.validate()?;
        *state = next;
        Ok(result)
    }
}
impl Backend for &ImageBackend {
    fn read<T>(&self, read: impl FnOnce(&CommitState) -> T) -> Result<T, Failure> {
        (**self).read(read)
    }
    fn write<T>(
        &self,
        write: impl FnOnce(&mut CommitState) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        (**self).write(write)
    }
}

fn encode(publication: &Publication) -> Result<Vec<u8>, Rejection> {
    struct Bounded(Vec<u8>);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > zixcel_revision::MAX_PAYLOAD_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    serde_json::to_writer(&mut output, publication).map_err(|_| Rejection::Capacity)?;
    Ok(output.0)
}

fn validate_changes(changes: &GraphChanges) -> Result<(), GraphError> {
    use crate::validation::{identifier, property_key, property_value};
    for values in [&changes.remove_nodes, &changes.remove_edges] {
        let mut ids = std::collections::BTreeSet::new();
        for id in values {
            identifier(id, 256, "removed identity")?;
            if !ids.insert(id) {
                return Err(GraphError::Invalid("duplicate removal".into()));
            }
        }
    }
    let mut node_ids = std::collections::BTreeSet::new();
    let mut edge_ids = std::collections::BTreeSet::new();
    for node in &changes.nodes {
        Node::new(&node.id, node.labels.iter().cloned())?;
        if !node_ids.insert(&node.id)
            || changes.remove_nodes.contains(&node.id)
            || node.properties.len() > 128
        {
            return Err(GraphError::Invalid("invalid node update set".into()));
        }
        for (key, value) in &node.properties {
            property_key(key)?;
            property_value(value)?;
        }
    }
    for edge in &changes.edges {
        for value in [&edge.id, &edge.from, &edge.to, &edge.label] {
            identifier(value, 256, "edge identity")?;
        }
        if !edge_ids.insert(&edge.id)
            || changes.remove_edges.contains(&edge.id)
            || edge.properties.len() > 128
            || (edge.direction == crate::EdgeDirection::Undirected && edge.from > edge.to)
        {
            return Err(GraphError::Invalid("invalid edge update set".into()));
        }
        for (key, value) in &edge.properties {
            property_key(key)?;
            property_value(value)?;
        }
    }
    Ok(())
}

fn replay<B: Backend>(
    store: &CommitStore<B>,
    space: &GraphSpace,
    request: &PublicationRequest,
) -> Result<Option<GraphPublication>, GraphError> {
    let Some(receipt) = store.receipt(space.as_str(), &request.operation_id)? else {
        return Ok(None);
    };
    let original = store
        .committed(&receipt.commit_ref)?
        .ok_or(Failure::Corrupt)?;
    let original: Publication =
        serde_json::from_slice(original.payload()).map_err(|_| Failure::Corrupt)?;
    let output = original.changes.output.clone();
    let prepared = match intent(space, request, original.changes) {
        Ok(p) => p,
        Err(e) => return Ok(Some(finished(CommitOutcome::Rejected(e), None))),
    };
    Ok(Some(finished(store.commit(&prepared), Some(output))))
}
