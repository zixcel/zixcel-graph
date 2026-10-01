// Hatter C4 2026: real Graph consumer, not a second lifecycle implementation.
#![cfg(all(feature = "graph", feature = "redb"))]
use zixcel_graph::{Graph, GraphSpace, Node};
use zixcel_graph::{GraphChanges, PublicationIntent};
use zixcel_revision::*;

fn request(graph: &Graph, space: &GraphSpace, id: &str) -> zixcel_graph::PublicationRequest {
    PublicationIntent {
        operation_id: id.into(),
        expected_commit_revision: graph.commit_revision(space).unwrap(),
    }
    .request("test", &id)
    .unwrap()
}
fn changes(id: &str) -> GraphChanges {
    GraphChanges {
        nodes: vec![Node::new(id, ["fixture"]).unwrap()],
        output: id.as_bytes().to_vec(),
        ..Default::default()
    }
}
fn child(root: &std::path::Path, phase: &str) {
    let mut process = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "durable_prepared_restart_replay_and_external_owner_reclamation",
        ])
        .env("GRAPH_C4_HOME", root)
        .env("GRAPH_C4_PHASE", phase)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(status) = process.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = process.kill();
            let _ = process.wait();
            panic!("test child timeout");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
fn durable_prepared_restart_replay_and_external_owner_reclamation() {
    if let Ok(home) = std::env::var("GRAPH_C4_HOME") {
        child_phase(std::path::Path::new(&home));
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let path = home.join("graph.redb");
    let space = GraphSpace::new("external").unwrap();
    let graph = Graph::create(&path).unwrap();
    let object = ExternalObjectRef {
        owner: "fixture".into(),
        object: "canonical".into(),
    };
    let first = graph
        .prepare_publication(
            &space,
            &request(&graph, &space, "first"),
            changes("first"),
            std::slice::from_ref(&object),
        )
        .unwrap();
    std::fs::write(home.join("canonical.object"), b"owner bytes").unwrap();
    std::fs::write(
        home.join("first.prepared"),
        serde_json::to_vec(&first).unwrap(),
    )
    .unwrap();
    std::fs::write(home.join("prepared.ref"), first.reference()).unwrap();
    drop(graph);
    child(home, "prepared");
    let graph = Graph::open_existing(&path).unwrap();
    assert!(matches!(
        graph.publish_prepared(&space, &first).unwrap().outcome,
        CommitOutcome::Committed(_)
    ));
    graph
        .publish(
            &space,
            &request(&graph, &space, "later"),
            |_| Ok(changes("later")),
            || Ok(()),
        )
        .unwrap();
    assert!(
        graph
            .claim_external_reclamation(&space, &object, u64::MAX)
            .unwrap()
            .is_none(),
        "receipt/canonical roots protect the owner object"
    );
    drop(graph);
    child(home, "replay");
    let graph = Graph::open_existing(&path).unwrap();
    reclamation_phase(home, graph, &space);
}

fn reclamation_phase(home: &std::path::Path, graph: Graph, space: &GraphSpace) {
    let path = home.join("graph.redb");
    let orphan = ExternalObjectRef {
        owner: "fixture".into(),
        object: "orphan".into(),
    };
    let pending = graph
        .prepare_publication(
            space,
            &request(&graph, space, "pending"),
            changes("unpublished"),
            std::slice::from_ref(&orphan),
        )
        .unwrap();
    std::fs::write(home.join("orphan.object"), b"unpublished owner bytes").unwrap();
    graph
        .publish(
            space,
            &request(&graph, space, "intervening"),
            |_| Ok(changes("intervening")),
            || Ok(()),
        )
        .unwrap();
    assert!(matches!(
        graph.publish_prepared(space, &pending).unwrap().outcome,
        CommitOutcome::Conflict(_)
    ));
    assert!(graph.read(space).unwrap().get_node("unpublished").is_none());
    assert!(
        home.join("orphan.object").exists(),
        "failed publication must not delete owner bytes"
    );
    std::fs::write(
        home.join("abandoned.prepared"),
        serde_json::to_vec(&pending).unwrap(),
    )
    .unwrap();
    let hold = ExternalRoot {
        kind: ExternalRootKind::InFlight,
        holder: "operation:1".into(),
    };
    graph.retain_external(space, &orphan, &hold).unwrap();
    graph
        .abandon_prepared(space, pending.reference(), 10)
        .unwrap();
    assert!(
        graph
            .claim_external_reclamation(space, &orphan, 100)
            .unwrap()
            .is_none()
    );
    assert!(home.join("orphan.object").exists());
    graph.release_external(space, &orphan, &hold).unwrap();
    assert!(
        graph
            .claim_external_reclamation(space, &orphan, 9)
            .unwrap()
            .is_none()
    );
    let permit = graph
        .claim_external_reclamation(space, &orphan, 10)
        .unwrap()
        .unwrap();
    std::fs::write(
        home.join("permit.json"),
        serde_json::to_vec(&permit).unwrap(),
    )
    .unwrap();
    drop(graph);
    child(home, "reclaim");
    assert!(!home.join("orphan.object").exists());
    let graph = Graph::open_existing(&path).unwrap();
    assert!(graph.read(space).unwrap().get_node("unpublished").is_none());
}

#[test]
fn graph_transactions_publish_foundation_identity_and_inspection_is_physically_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.redb");
    let space = GraphSpace::new("consumer").unwrap();
    for graph in [Graph::memory().unwrap(), Graph::create(&path).unwrap()] {
        let mut write = graph.write(&space).unwrap();
        write.upsert_node(Node::new("a", ["test"]).unwrap());
        assert_eq!(write.commit().unwrap(), 1);
        let revision = graph.commit_revision(&space).unwrap();
        assert_eq!(
            revision.sequence, 1,
            "old Graph CAS must not bypass Foundation"
        );
        assert!(revision.commit.is_some());
    }
    let before = std::fs::read(&path).unwrap();
    {
        let graph = Graph::open_existing(&path).unwrap();
        assert_eq!(graph.read(&space).unwrap().revision(), 1);
        assert_eq!(graph.commit_revision(&space).unwrap().sequence, 1);
    }
    assert!(
        std::fs::read(&path).unwrap() == before,
        "inspection must not open a writable redb handle"
    );
}

fn child_phase(home: &std::path::Path) {
    let graph = Graph::open_existing(home.join("graph.redb")).unwrap();
    let space = GraphSpace::new("external").unwrap();
    let reference = std::fs::read_to_string(home.join("prepared.ref")).unwrap();
    match std::env::var("GRAPH_C4_PHASE").unwrap().as_str() {
        "prepared" => {
            assert!(graph.read(&space).unwrap().get_node("first").is_none());
            assert_eq!(
                graph.commit_revision(&space).unwrap(),
                RevisionRef::default()
            );
            assert!(
                graph
                    .prepared_publication(&space, &reference)
                    .unwrap()
                    .is_some()
            );
        }
        "replay" => {
            let first: PreparedCommit =
                serde_json::from_slice(&std::fs::read(home.join("first.prepared")).unwrap())
                    .unwrap();
            let before = graph.commit_revision(&space).unwrap();
            for _ in 0..100 {
                let result = graph.publish_prepared(&space, &first).unwrap();
                assert!(matches!(result.outcome, CommitOutcome::NoChange(_)));
                assert_eq!(result.output, Some(b"first".to_vec()));
            }
            assert_eq!(graph.commit_revision(&space).unwrap(), before);
            assert!(graph.read(&space).unwrap().get_node("later").is_some());
        }
        "reclaim" => {
            let object = ExternalObjectRef {
                owner: "fixture".into(),
                object: "orphan".into(),
            };
            let permit = graph
                .claim_external_reclamation(&space, &object, 10)
                .unwrap()
                .unwrap();
            let expected: ReclamationPermit =
                serde_json::from_slice(&std::fs::read(home.join("permit.json")).unwrap()).unwrap();
            assert_eq!(permit, expected);
            assert!(
                graph
                    .retain_external(
                        &space,
                        &object,
                        &ExternalRoot {
                            kind: ExternalRootKind::Retained,
                            holder: "late".into()
                        }
                    )
                    .is_err()
            );
            // ONLY the external owner fixture deletes its own object, after exact permit confirmation.
            std::fs::remove_file(home.join("orphan.object")).unwrap();
            graph
                .complete_external_reclamation(&space, &permit)
                .unwrap();
            assert_eq!(graph.reclaim_prepared(&space, 10).unwrap(), 1);
            let abandoned: PreparedCommit =
                serde_json::from_slice(&std::fs::read(home.join("abandoned.prepared")).unwrap())
                    .unwrap();
            assert!(graph.publish_prepared(&space, &abandoned).is_err());
            assert!(home.join("canonical.object").exists());
        }
        _ => panic!("invalid child phase"),
    }
}
