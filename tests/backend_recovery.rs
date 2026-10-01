//! C4 physical recovery is explicit; logical corruption is never repaired on read.
#![cfg(all(feature = "graph", feature = "redb"))]
use zixcel_graph::{Graph, GraphChanges, GraphError, GraphSpace, Node, PublicationIntent};
use zixcel_revision::*;

fn proposal(id: &str, revision: RevisionRef) -> PreparedCommit {
    CommitIntent {
        domain: "crash".into(),
        operation_id: id.into(),
        parents: revision.commit.iter().cloned().collect(),
        expected_revision: revision,
        payload: id.as_bytes().to_vec(),
    }
    .prepare()
    .unwrap()
}
fn crash_child(path: &std::path::Path) {
    let graph = Graph::create(path.join("graph.redb")).unwrap();
    let space = GraphSpace::new("crash").unwrap();
    let request = PublicationIntent {
        operation_id: "first".into(),
        expected_commit_revision: RevisionRef::default(),
    }
    .request("fixture", &"first")
    .unwrap();
    let first = graph
        .prepare_publication(
            &space,
            &request,
            GraphChanges {
                nodes: vec![Node::new("first", ["fixture"]).unwrap()],
                output: b"original".to_vec(),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    graph.publish_prepared(&space, &first).unwrap();
    let request = PublicationIntent {
        operation_id: "pending".into(),
        expected_commit_revision: graph.commit_revision(&space).unwrap(),
    }
    .request("fixture", &"pending")
    .unwrap();
    let pending = graph
        .prepare_publication(
            &space,
            &request,
            GraphChanges {
                nodes: vec![Node::new("pending", ["fixture"]).unwrap()],
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    std::fs::write(
        path.join("prepared.json"),
        serde_json::to_vec(&[first, pending]).unwrap(),
    )
    .unwrap();
    let store = CommitStore::new(RedbBackend::create(path.join("foundation.redb")).unwrap());
    let CommitOutcome::Committed(receipt) =
        store.commit(&proposal("first", RevisionRef::default()))
    else {
        panic!("commit")
    };
    store
        .stage(&proposal("pending", receipt.committed_revision))
        .unwrap();
    // No destructors: leave both writable redb handles requiring physical recovery.
    std::process::exit(0);
}
#[test]
fn crash_requires_explicit_recovery_and_preserves_prepared_and_receipt() {
    if let Ok(path) = std::env::var("GRAPH_C4_CRASH") {
        crash_child(std::path::Path::new(&path));
    }
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crash_requires_explicit_recovery_and_preserves_prepared_and_receipt",
        ])
        .env("GRAPH_C4_CRASH", dir.path())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("crash child timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let graph_path = dir.path().join("graph.redb");
    let foundation_path = dir.path().join("foundation.redb");
    for _ in 0..3 {
        let before = std::fs::read(&graph_path).unwrap();
        assert!(matches!(
            Graph::open_existing(&graph_path),
            Err(GraphError::Commit(Failure::RecoveryRequired))
        ));
        assert!(
            std::fs::read(&graph_path).unwrap() == before,
            "read must not alter backend bytes"
        );
        let before = std::fs::read(&foundation_path).unwrap();
        assert!(matches!(
            RedbBackend::open_existing(&foundation_path),
            Err(Failure::RecoveryRequired)
        ));
        assert_eq!(Failure::RecoveryRequired.retry_class(), RetryClass::None);
        assert!(
            std::fs::read(&foundation_path).unwrap() == before,
            "read must not alter backend bytes"
        );
    }
    let graph = Graph::recover_existing(&graph_path).unwrap();
    let space = GraphSpace::new("crash").unwrap();
    let prepared: Vec<PreparedCommit> =
        serde_json::from_slice(&std::fs::read(dir.path().join("prepared.json")).unwrap()).unwrap();
    assert_eq!(graph.commit_revision(&space).unwrap().sequence, 1);
    assert!(graph.read(&space).unwrap().get_node("pending").is_none());
    assert!(
        graph
            .prepared_publication(&space, prepared[1].reference())
            .unwrap()
            .is_some()
    );
    let replay = graph.publish_prepared(&space, &prepared[0]).unwrap();
    assert!(matches!(replay.outcome, CommitOutcome::NoChange(_)));
    assert_eq!(replay.output, Some(b"original".to_vec()));
    let store = CommitStore::new(RedbBackend::recover_existing(&foundation_path).unwrap());
    assert_eq!(store.current("crash").unwrap().unwrap().payload(), b"first");
    assert_eq!(store.stats().unwrap().prepared, 1);
    assert!(matches!(
        store.commit(&proposal("first", RevisionRef::default())),
        CommitOutcome::NoChange(_)
    ));
    drop(store);
    drop(graph);
    assert!(Graph::open_existing(&graph_path).is_ok());
    assert!(RedbBackend::open_existing(&foundation_path).is_ok());
    assert!(Graph::recover_existing(dir.path().join("missing")).is_err());
    assert!(!dir.path().join("missing").exists());
}

#[test]
fn mismatched_graph_foundation_revision_is_typed_corruption_without_read_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mismatch.redb");
    let space = GraphSpace::new("fixture").unwrap();
    {
        let graph = Graph::create(&path).unwrap();
        let mut write = graph.write(&space).unwrap();
        write.upsert_node(Node::new("a", ["fixture"]).unwrap());
        write.commit().unwrap();
    }
    {
        let db = redb::Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        tx.open_table(redb::TableDefinition::<&str, u64>::new(
            "zixcel_graph_spaces",
        ))
        .unwrap()
        .insert("fixture", 99)
        .unwrap();
        tx.commit().unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    {
        let graph = Graph::open_existing(&path).unwrap();
        assert!(matches!(
            graph.read(&space),
            Err(GraphError::Commit(Failure::Corrupt))
        ));
        assert!(matches!(
            graph.write(&space),
            Err(GraphError::Commit(Failure::Corrupt))
        ));
        assert!(matches!(
            graph.commit_revision(&space),
            Err(GraphError::Commit(Failure::Corrupt))
        ));
    }
    assert!(
        std::fs::read(&path).unwrap() == before,
        "read must not alter backend bytes"
    );
}
