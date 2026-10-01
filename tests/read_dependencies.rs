#![cfg(all(feature = "graph", feature = "redb"))]
use zixcel_graph::{
    Graph, GraphChanges, GraphError, GraphReadDependency, GraphSpace, Node, PublicationIntent,
};
use zixcel_revision::{CommitOutcome, RevisionRef};

fn request(
    graph: &Graph,
    s: &GraphSpace,
    id: &str,
    dependencies: Vec<GraphReadDependency>,
) -> zixcel_graph::PublicationRequest {
    PublicationIntent {
        operation_id: id.into(),
        expected_commit_revision: graph.commit_revision(s).unwrap(),
    }
    .request("test", &id)
    .unwrap()
    .with_dependencies(dependencies)
    .unwrap()
}
fn changes(id: &str) -> GraphChanges {
    GraphChanges {
        nodes: vec![Node::new(id, ["test"]).unwrap()],
        ..Default::default()
    }
}
fn put(graph: &Graph, s: &GraphSpace, id: &str) {
    assert!(matches!(
        graph
            .publish(
                s,
                &request(graph, s, id, vec![]),
                |_| Ok(changes(id)),
                || Ok(())
            )
            .unwrap()
            .outcome,
        CommitOutcome::Committed(_)
    ));
}
fn dep(graph: &Graph, s: &GraphSpace) -> GraphReadDependency {
    let (view, revision) = graph.read_exact(s).unwrap();
    assert_eq!(view.revision(), revision.sequence);
    GraphReadDependency {
        space: s.clone(),
        expected_revision: revision,
    }
}
fn matrix(graph: &Graph) {
    let source_a = GraphSpace::new("source/source_a").unwrap();
    let source_b = GraphSpace::new("source/source_b").unwrap();
    let target = GraphSpace::new("target").unwrap();
    put(graph, &source_a, "a1");
    put(graph, &source_b, "b1");
    let da = dep(graph, &source_a);
    let db = dep(graph, &source_b);
    let first = request(graph, &target, "target/first", vec![db.clone(), da.clone()]);
    let sorted = request(graph, &target, "target/first", vec![da.clone(), db.clone()]);
    assert_eq!(first.request_digest, sorted.request_digest);
    let accepted = graph
        .publish(
            &target,
            &first,
            |_| {
                graph.read(&source_a)?;
                Ok(changes("accepted"))
            },
            || {
                graph.read(&source_b)?;
                Ok(())
            },
        )
        .unwrap();
    let CommitOutcome::Committed(receipt) = accepted.outcome else {
        panic!("not committed")
    };
    put(graph, &source_a, "a2");
    let replay = graph
        .publish(
            &target,
            &first,
            |_| panic!("replay calculation"),
            || panic!("replay preparation"),
        )
        .unwrap();
    assert!(matches!(replay.outcome,CommitOutcome::NoChange(ref r) if r==&receipt));
    let stale = request(
        graph,
        &target,
        "target/stale-source_a",
        vec![da.clone(), db.clone()],
    );
    let before = graph.commit_revision(&target).unwrap();
    assert!(
        matches!(graph.publish(&target,&stale, |_| Ok(changes("bad")), || Ok(())),Err(GraphError::DependencyConflict {space,expected,actual}) if space==source_a && expected==da.expected_revision && actual==graph.commit_revision(&source_a).unwrap())
    );
    assert_eq!(graph.commit_revision(&target).unwrap(), before);
    assert!(graph.read(&target).unwrap().get_node("bad").is_none());
    assert!(graph.prepared_references(&target).unwrap().is_empty());
    let changed = PublicationIntent {
        operation_id: first.operation_id.clone(),
        expected_commit_revision: RevisionRef::default(),
    }
    .request("test", &"target/first")
    .unwrap()
    .with_dependencies(vec![dep(graph, &source_a), db.clone()])
    .unwrap();
    assert_ne!(first.request_digest, changed.request_digest);
    assert!(matches!(
        graph
            .publish(&target, &changed, |_| panic!(), || panic!())
            .unwrap()
            .outcome,
        CommitOutcome::Conflict(_)
    ));
    put(graph, &source_b, "b2");
    let stale_b = request(
        graph,
        &target,
        "target/stale-source_b",
        vec![dep(graph, &source_a), db],
    );
    assert!(
        matches!(graph.publish(&target,&stale_b, |_|Ok(changes("bad-source_b")),||Ok(())),Err(GraphError::DependencyConflict {space,..}) if space==source_b)
    );
    invalid_dependencies(graph, &source_a, &target, &da, before);
    concurrent_source_change(graph, &source_a, &target, &receipt);
}
#[test]
fn exact_dependencies_replay_and_reentrant_free_calculation() {
    matrix(&Graph::memory().unwrap());
    let d = tempfile::tempdir().unwrap();
    matrix(&Graph::create(d.path().join("graph.redb")).unwrap());
}

fn invalid_dependencies(
    graph: &Graph,
    source_a: &GraphSpace,
    target: &GraphSpace,
    da: &GraphReadDependency,
    before: RevisionRef,
) {
    let bad = request(graph, target, "target/self", vec![dep(graph, target)]);
    assert!(
        graph
            .publish(target, &bad, |_| panic!(), || panic!())
            .is_err()
    );
    assert!(
        PublicationIntent {
            operation_id: "duplicates".into(),
            expected_commit_revision: before.clone()
        }
        .request("test", &0)
        .unwrap()
        .with_dependencies(vec![da.clone(), dep(graph, source_a)])
        .is_err()
    );
    assert!(
        PublicationIntent {
            operation_id: "limit".into(),
            expected_commit_revision: before
        }
        .request("test", &0)
        .unwrap()
        .with_dependencies(
            (0..9)
                .map(|i| GraphReadDependency {
                    space: GraphSpace::new(format!("s/{i}")).unwrap(),
                    expected_revision: RevisionRef::default()
                })
                .collect()
        )
        .is_err()
    );
}

fn concurrent_source_change(
    graph: &Graph,
    source_a: &GraphSpace,
    target: &GraphSpace,
    receipt: &zixcel_revision::CommitReceipt,
) {
    let barrier = std::sync::Barrier::new(2);
    let concurrent = request(
        graph,
        target,
        "target/concurrent",
        vec![dep(graph, source_a)],
    );
    std::thread::scope(|scope| {
        let source = scope.spawn(|| {
            barrier.wait();
            put(graph, source_a, "a3");
            barrier.wait();
        });
        let result = graph.publish(
            target,
            &concurrent,
            |_| {
                barrier.wait();
                barrier.wait();
                Ok(changes("raced"))
            },
            || Ok(()),
        );
        assert!(matches!(result, Err(GraphError::DependencyConflict { .. })));
        source.join().unwrap();
    });
    assert!(graph.read(target).unwrap().get_node("raced").is_none());
    assert_eq!(
        graph.commit_revision(target).unwrap(),
        receipt.committed_revision
    );
}
