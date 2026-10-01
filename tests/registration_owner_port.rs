//! Owner port: lifecycle changes leave Graph revision, payload and receipt intact.
use zixcel_graph::{Graph, GraphChanges, GraphSpace, PublicationRequest};
use zixcel_revision::*;

fn scenario(graph: &Graph) {
    let space = GraphSpace::new("source-owner").unwrap();
    let object = ExternalObjectRef {
        owner: "source-owner".into(),
        object: "immutable-source".into(),
    };
    let request = PublicationRequest {
        operation_id: "publish".into(),
        expected_commit_revision: RevisionRef::default(),
        request_digest: "a".repeat(64),
        dependencies: Vec::new(),
    };
    assert!(
        graph
            .external_registration(&space, &object, "absent")
            .unwrap()
            .is_none()
    );
    let publication = graph
        .publish(
            &space,
            &request,
            |_| {
                Ok(GraphChanges {
                    output: b"owner-result".to_vec(),
                    ..GraphChanges::default()
                })
            },
            || Ok(()),
        )
        .unwrap();
    let CommitOutcome::Committed(receipt) = publication.outcome else {
        panic!("commit")
    };
    let before = graph.commit_revision(&space).unwrap();
    let graph_revision = graph.read(&space).unwrap().revision();
    let registration = graph
        .register_committed_external(&space, &receipt, std::slice::from_ref(&object))
        .unwrap()
        .remove(0);
    assert_eq!(
        graph
            .register_committed_external(&space, &receipt, std::slice::from_ref(&object))
            .unwrap(),
        vec![registration.clone()]
    );
    let root = ExternalRoot {
        kind: ExternalRootKind::InFlight,
        holder: "exact-reader".into(),
    };
    graph
        .retain_registration(&space, &registration, &root)
        .unwrap();
    graph
        .retire_external_registration(&space, &registration)
        .unwrap();
    assert_eq!(
        graph.registration_status(&space, &registration).unwrap(),
        ExternalRegistrationStatus::Retiring
    );
    assert!(
        graph
            .claim_external_reclamation(&space, &object, u64::MAX)
            .unwrap()
            .is_none()
    );
    graph
        .release_registration(&space, &registration, &root)
        .unwrap();
    let permit = graph
        .claim_external_reclamation(&space, &object, u64::MAX)
        .unwrap()
        .unwrap();
    graph
        .complete_external_reclamation(&space, &permit)
        .unwrap();
    assert_eq!(graph.commit_revision(&space).unwrap(), before);
    assert_eq!(graph.read(&space).unwrap().revision(), graph_revision);
    let replay = graph
        .publish(
            &space,
            &request,
            |_| panic!("must not run"),
            || panic!("must not prepare"),
        )
        .unwrap();
    assert_eq!(replay.outcome, CommitOutcome::NoChange(receipt));
    assert_eq!(replay.output, Some(b"owner-result".to_vec()));
}
#[test]
fn owner_port_metadata_does_not_publish_graph_domain_state() {
    scenario(&Graph::memory().unwrap());
    let dir = tempfile::tempdir().unwrap();
    scenario(&Graph::create(dir.path().join("graph.redb")).unwrap());
}

fn concurrent_publication(graph: &Graph) {
    let space = GraphSpace::new("concurrent-owner").unwrap();
    let first = PublicationRequest {
        operation_id: "first".into(),
        expected_commit_revision: RevisionRef::default(),
        request_digest: "a".repeat(64),
        dependencies: Vec::new(),
    };
    let CommitOutcome::Committed(receipt) = graph
        .publish(&space, &first, |_| Ok(GraphChanges::default()), || Ok(()))
        .unwrap()
        .outcome
    else {
        panic!("first publication")
    };
    let object = ExternalObjectRef {
        owner: "independent-source-owner".into(),
        object: "immutable-body".into(),
    };
    let port = graph.external_retention(&space);
    let registration = port
        .register_committed_external(&receipt, std::slice::from_ref(&object))
        .unwrap()
        .remove(0);
    let second = PublicationRequest {
        operation_id: "second".into(),
        expected_commit_revision: receipt.committed_revision.clone(),
        request_digest: "b".repeat(64),
        dependencies: Vec::new(),
    };
    let barrier = std::sync::Barrier::new(2);
    let current = std::thread::scope(|scope| {
        let retiring = scope.spawn(|| {
            barrier.wait();
            port.retire_external_registration(&registration).unwrap()
        });
        let publishing = scope.spawn(|| {
            barrier.wait();
            graph
                .publish(
                    &space,
                    &second,
                    |_| {
                        Ok(GraphChanges {
                            output: b"current".to_vec(),
                            ..GraphChanges::default()
                        })
                    },
                    || Ok(()),
                )
                .unwrap()
        });
        assert_eq!(retiring.join().unwrap().registration, registration);
        publishing.join().unwrap()
    });
    let CommitOutcome::Committed(newer) = current.outcome else {
        panic!("retirement must not conflict with canonical CAS")
    };
    assert_eq!(
        graph.commit_revision(&space).unwrap(),
        newer.committed_revision
    );
    assert_eq!(current.output, Some(b"current".to_vec()));
    let permit = port
        .claim_external_reclamation(&object, 0)
        .unwrap()
        .unwrap();
    port.complete_external_reclamation(&permit).unwrap();
    assert_eq!(
        port.retire_external_registration(&registration)
            .unwrap()
            .registration,
        registration
    );
    assert_eq!(
        graph
            .publish(&space, &first, |_| panic!("replay"), || panic!("replay"))
            .unwrap()
            .outcome,
        CommitOutcome::NoChange(receipt)
    );
    assert_eq!(
        graph.commit_revision(&space).unwrap(),
        newer.committed_revision
    );
}

#[test]
fn lifecycle_only_port_does_not_conflict_with_current_publication() {
    for _ in 0..8 {
        concurrent_publication(&Graph::memory().unwrap());
        let dir = tempfile::tempdir().unwrap();
        concurrent_publication(&Graph::create(dir.path().join("concurrent.redb")).unwrap());
    }
}
