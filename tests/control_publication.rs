// Hatter reconstruction 2026: C3 storage port acceptance, independent of Hatter semantics.
#![cfg(all(feature = "graph", feature = "redb"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use zixcel_graph::{
    Graph, GraphChanges, GraphError, GraphSpace, Node, PropertyValue, PublicationRequest,
};
use zixcel_revision::{CommitOutcome, Conflict, RevisionRef};

fn request(id: &str, base: RevisionRef, digest: char) -> PublicationRequest {
    PublicationRequest {
        operation_id: id.into(),
        expected_commit_revision: base,
        request_digest: digest.to_string().repeat(64),
        dependencies: Vec::new(),
    }
}
fn changes(value: &str) -> GraphChanges {
    GraphChanges {
        nodes: vec![
            Node::new("role", ["control".to_string()])
                .unwrap()
                .with_property("reference", PropertyValue::String(value.into()))
                .unwrap(),
        ],
        output: value.as_bytes().to_vec(),
        ..GraphChanges::default()
    }
}
fn committed(outcome: &CommitOutcome) -> &zixcel_revision::CommitReceipt {
    match outcome {
        CommitOutcome::Committed(receipt) | CommitOutcome::NoChange(receipt) => receipt,
        other => panic!("unexpected {other:?}"),
    }
}
fn formal_input_is_bounded() {
    let input = zixcel_graph::PublicationIntent {
        operation_id: "typed-input".into(),
        expected_commit_revision: RevisionRef::default(),
    };
    let formal = input.request("register", &("owner", "value")).unwrap();
    let again = input.request("register", &("owner", "value")).unwrap();
    assert_eq!(formal.request_digest, again.request_digest);
    assert_eq!(formal.expected_commit_revision, RevisionRef::default());
    assert_ne!(
        formal.request_digest,
        input
            .request("register", &("other-owner", "value"))
            .unwrap()
            .request_digest
    );
    assert_ne!(
        formal.request_digest,
        input
            .request("delete", &("owner", "value"))
            .unwrap()
            .request_digest
    );
    let oversized = "x".repeat(zixcel_revision::MAX_PAYLOAD_BYTES + 1);
    assert!(input.request("register", &oversized).is_err());
}
fn scenario(graph: &Graph, space: &GraphSpace) {
    formal_input_is_bounded();
    let calculated = AtomicUsize::new(0);
    let prepared = AtomicUsize::new(0);
    let first = request("observe", RevisionRef::default(), 'a');
    let result = graph
        .publish(
            space,
            &first,
            |read| {
                assert!(read.get_node("role").is_none());
                calculated.fetch_add(1, Ordering::SeqCst);
                Ok(changes("first"))
            },
            || {
                prepared.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
    let receipt = committed(&result.outcome).clone();
    assert_eq!(result.output, Some(b"first".to_vec()));
    let second = request("decide", receipt.committed_revision.clone(), 'b');
    let later = graph
        .publish(space, &second, |_| Ok(changes("later")), || Ok(()))
        .unwrap();
    let later_receipt = committed(&later.outcome).clone();
    exact_receipt_lookup(graph, space, &receipt, &later_receipt);
    for _ in 0..100 {
        let replay = graph
            .publish(
                space,
                &first,
                |_| panic!("replay must not authorize or calculate"),
                || panic!("replay must not prepare"),
            )
            .unwrap();
        assert!(matches!(replay.outcome, CommitOutcome::NoChange(_)));
        assert_eq!(committed(&replay.outcome), &receipt);
        assert_eq!(replay.output, Some(b"first".to_vec()));
        let preflight = graph.replay(space, &first).unwrap().unwrap();
        assert_eq!(preflight.outcome, replay.outcome);
        let original = graph.committed_changes(space, &receipt).unwrap();
        assert_eq!(
            original.nodes[0].properties.get("reference"),
            Some(&PropertyValue::String("first".into()))
        );
        let reused = graph
            .publish(
                space,
                &request("observe", RevisionRef::default(), 'c'),
                |_| panic!("reuse before authorization"),
                || panic!("reuse cannot prepare"),
            )
            .unwrap();
        assert!(matches!(
            reused.outcome,
            CommitOutcome::Conflict(Conflict::OperationReuse { .. })
        ));
        let stale = graph
            .publish(
                space,
                &request("stale", RevisionRef::default(), 'a'),
                |_| panic!("stale before calculation"),
                || panic!("stale cannot prepare"),
            )
            .unwrap();
        assert_eq!(
            stale.outcome,
            CommitOutcome::Conflict(Conflict::Stale {
                expected: RevisionRef::default(),
                actual: later_receipt.committed_revision.clone()
            })
        );
        assert!(stale.output.is_none());
        let checked = graph
            .replay(space, &request("stale", RevisionRef::default(), 'a'))
            .unwrap()
            .unwrap();
        assert_eq!(checked.outcome, stale.outcome);
    }
    assert_eq!(calculated.load(Ordering::SeqCst), 1);
    assert_eq!(prepared.load(Ordering::SeqCst), 1);
    assert_eq!(
        graph
            .read(space)
            .unwrap()
            .get_node("role")
            .unwrap()
            .properties
            .get("reference"),
        Some(&PropertyValue::String("later".into()))
    );
    rejected_preparation(graph, space, &later_receipt.committed_revision);
}

fn exact_receipt_lookup(
    graph: &Graph,
    space: &GraphSpace,
    first: &zixcel_revision::CommitReceipt,
    second: &zixcel_revision::CommitReceipt,
) {
    for receipt in [first, second] {
        assert_eq!(
            graph
                .committed_receipt(space, &receipt.commit_ref)
                .unwrap()
                .as_ref(),
            Some(receipt)
        );
    }
    assert!(
        graph
            .committed_receipt(space, &"0".repeat(64))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        graph.commit_revision(space).unwrap(),
        second.committed_revision
    );
}

fn rejected_preparation(graph: &Graph, space: &GraphSpace, base: &RevisionRef) {
    let before = graph.read(space).unwrap().revision();
    for _ in 0..100 {
        let result = graph.publish(
            space,
            &request("failure", base.clone(), 'd'),
            |_| Ok(changes("invalid")),
            || {
                Err(GraphError::Storage(
                    "injected immutable prepare failure".into(),
                ))
            },
        );
        assert!(result.is_err());
        let result = graph.publish(
            space,
            &request("invalid", base.clone(), 'e'),
            |_| Err(GraphError::Invalid("owner rejects input".into())),
            || panic!("invalid input cannot prepare"),
        );
        assert!(result.is_err());
        let invalid = graph.publish(
            space,
            &request("invalid-node", base.clone(), 'a'),
            |_| {
                let mut data = changes("invalid");
                data.nodes[0].id.clear();
                Ok(data)
            },
            || panic!("invalid node cannot prepare"),
        );
        assert!(matches!(invalid, Err(GraphError::Invalid(_))));
        let oversized = graph
            .publish(
                space,
                &request("too-large", base.clone(), 'a'),
                |_| {
                    let mut data = changes("oversized");
                    data.output = vec![b'x'; zixcel_revision::MAX_PAYLOAD_BYTES];
                    Ok(data)
                },
                || panic!("oversized cannot prepare"),
            )
            .unwrap();
        assert!(matches!(
            oversized.outcome,
            CommitOutcome::Rejected(zixcel_revision::Rejection::Capacity)
        ));
        assert_eq!(graph.read(space).unwrap().revision(), before);
    }
    assert_eq!(graph.commit_revision(space).unwrap(), *base);
}
#[test]
fn memory_and_durable_reference_publication_replay_restart_and_failure() {
    let space = GraphSpace::new("control").unwrap();
    let memory = Graph::memory().unwrap();
    scenario(&memory, &space);
    writers(&memory, &space);
    prepared_input(&memory, &space);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.redb");
    scenario(&Graph::create(&path).unwrap(), &space);
    let graph = Graph::open_existing(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let base = graph.commit_revision(&space).unwrap();
    for _ in 0..100 {
        let replay = graph
            .publish(
                &space,
                &request("observe", RevisionRef::default(), 'a'),
                |_| panic!("restart replay"),
                || panic!("restart prepare"),
            )
            .unwrap();
        assert!(matches!(replay.outcome, CommitOutcome::NoChange(_)));
        assert_eq!(replay.output, Some(b"first".to_vec()));
        assert_eq!(graph.commit_revision(&space).unwrap(), base);
    }
    assert!(
        std::fs::read(&path).unwrap() == before,
        "replay wrote database bytes"
    );
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified
    );
    writers(&graph, &space);
    prepared_input(&graph, &space);
    let input = request(
        "restart-preparation",
        graph.commit_revision(&space).unwrap(),
        'f',
    );
    let original = changes("preserved-before-restart");
    let reference = graph
        .prepare_publication(&space, &input, original.clone(), &[])
        .unwrap()
        .reference()
        .to_owned();
    drop(graph);
    let graph = Graph::open_existing(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let (found, data) = graph.prepared_changes(&space, &input).unwrap().unwrap();
    assert_eq!(found, reference);
    assert_eq!(
        serde_json::to_value(data).unwrap(),
        serde_json::to_value(&original).unwrap()
    );
    assert_eq!(
        graph.commit_revision(&space).unwrap(),
        input.expected_commit_revision
    );
    assert!(
        std::fs::read(&path).unwrap() == before,
        "preparation inspection wrote database"
    );
    graph.abandon_prepared(&space, &reference, 1).unwrap();
    let before = std::fs::read(&path).unwrap();
    // Inspection is not revival: the existing noncanonical input can still be
    // identified, but the owner must restage and abandoned identity rejects.
    assert_eq!(
        graph.prepared_changes(&space, &input).unwrap().unwrap().0,
        reference
    );
    assert!(
        graph
            .prepare_publication(&space, &input, original, &[])
            .is_err()
    );
    assert_eq!(
        graph.commit_revision(&space).unwrap(),
        input.expected_commit_revision
    );
    assert!(
        std::fs::read(&path).unwrap() == before,
        "abandoned inspection changed database bytes"
    );
}

fn prepared_input(graph: &Graph, space: &GraphSpace) {
    let base = graph.commit_revision(space).unwrap();
    let input = request("prepared-original", base.clone(), 'd');
    assert!(graph.prepared_changes(space, &input).unwrap().is_none());
    let original = changes("original-owner-generated-value");
    let prepared = graph
        .prepare_publication(space, &input, original.clone(), &[])
        .unwrap();
    let current = graph.read(space).unwrap().get_node("role");
    for _ in 0..2 {
        let (reference, preserved) = graph.prepared_changes(space, &input).unwrap().unwrap();
        assert_eq!(reference, prepared.reference());
        assert_eq!(
            serde_json::to_value(preserved).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        assert_eq!(graph.commit_revision(space).unwrap(), base);
    }
    let changed = request("prepared-original", base.clone(), 'e');
    assert!(graph.prepared_changes(space, &changed).is_err());
    let changed_base = request("prepared-original", RevisionRef::default(), 'd');
    assert!(graph.prepared_changes(space, &changed_base).is_err());
    assert_eq!(graph.read(space).unwrap().get_node("role"), current);
    let result = graph.publish_prepared(space, &prepared).unwrap();
    let receipt = committed(&result.outcome);
    assert!(graph.prepared_changes(space, &input).unwrap().is_none());
    assert_eq!(
        graph.committed_changes(space, receipt).unwrap().output,
        original.output
    );
}

fn writers(graph: &Graph, space: &GraphSpace) {
    let base = graph.commit_revision(space).unwrap();
    let calculated = AtomicUsize::new(0);
    let prepared = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..100)
            .map(|n| {
                let base = base.clone();
                let calculated = &calculated;
                let prepared = &prepared;
                scope.spawn(move || {
                    graph
                        .publish(
                            space,
                            &request(&format!("writer-{n}"), base, 'f'),
                            |_| {
                                calculated.fetch_add(1, Ordering::SeqCst);
                                Ok(changes("winner"))
                            },
                            || {
                                prepared.fetch_add(1, Ordering::SeqCst);
                                Ok(())
                            },
                        )
                        .unwrap()
                        .outcome
                })
            })
            .collect();
        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, CommitOutcome::Committed(_)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|o| matches!(o, CommitOutcome::Conflict(Conflict::Stale { .. })))
                .count(),
            99
        );
    });
    // Speculative callbacks are not canonical acceptance. A losing final CAS
    // may leave inert owner-managed objects; the canonical winner count is 1.
    assert!((1..=100).contains(&calculated.load(Ordering::SeqCst)));
    assert!((1..=100).contains(&prepared.load(Ordering::SeqCst)));
    assert_eq!(
        graph.commit_revision(space).unwrap().sequence,
        base.sequence + 1
    );
}
