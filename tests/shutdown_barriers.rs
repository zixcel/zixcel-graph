//! Actual durable owner barriers. No process-timing inference and no read repair.
#![cfg(all(feature = "graph", feature = "redb", target_os = "linux"))]
use std::{
    io::{BufRead, Write},
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use zixcel_graph::{Graph, GraphChanges, GraphError, GraphSpace, Node, PublicationIntent};
use zixcel_revision::*;

fn prepare(graph: &Graph, space: &GraphSpace, id: &str) -> PreparedCommit {
    let request = PublicationIntent {
        operation_id: id.into(),
        expected_commit_revision: graph.commit_revision(space).unwrap(),
    }
    .request("shutdown-barrier", &id)
    .unwrap();
    graph
        .prepare_publication(
            space,
            &request,
            GraphChanges {
                nodes: vec![Node::new(id, ["fixture"]).unwrap()],
                output: id.as_bytes().to_vec(),
                ..Default::default()
            },
            &[],
        )
        .unwrap()
}
fn writer(path: &Path, barrier: &str) {
    let graph = Graph::create(path.join("graph.redb")).unwrap();
    let space = GraphSpace::new("shutdown").unwrap();
    let first = prepare(&graph, &space, "first");
    graph.publish_prepared(&space, &first).unwrap();
    let pending = if barrier == "prepared" || barrier == "committed" {
        Some(prepare(&graph, &space, "second"))
    } else {
        None
    };
    if barrier == "committed" {
        graph
            .publish_prepared(&space, pending.as_ref().unwrap())
            .unwrap();
    }
    // An unmodified optimistic write snapshot is held only in before-write;
    // graceful EOF explicitly abandons it, never prepares or commits it.
    let uncommitted = (barrier == "before-write").then(|| graph.write(&space).unwrap());
    let head = graph.commit_revision(&space).unwrap();
    std::fs::write(
        path.join("owner.json"),
        serde_json::to_vec(&(head, first, pending)).unwrap(),
    )
    .unwrap();
    println!("GRAPH_BARRIER:{barrier}");
    std::io::stdout().flush().unwrap();
    // Parent EOF, not an elapsed-time guess, releases normal owner shutdown.
    let mut command = String::new();
    std::io::stdin().read_line(&mut command).unwrap();
    if command.trim() == "crash" {
        std::process::exit(23);
    }
    drop(uncommitted);
    drop(graph);
    println!("GRAPH_CLOSED");
}
struct Owned(Child);
impl Drop for Owned {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
fn exact_barrier_shutdown_and_explicit_recovery_preserve_original_receipts() {
    if let Ok(path) = std::env::var("GRAPH_SHUTDOWN_CHILD") {
        writer(
            Path::new(&path),
            &std::env::var("GRAPH_SHUTDOWN_BARRIER").unwrap(),
        );
        return;
    }
    for barrier in ["idle", "before-write", "prepared", "committed"] {
        for disposition in ["eof", "term", "kill", "crash"] {
            run_case(barrier, disposition);
        }
    }
}

fn run_case(barrier: &str, disposition: &str) {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Owned(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "exact_barrier_shutdown_and_explicit_recovery_preserve_original_receipts",
                "--nocapture",
            ])
            .env("GRAPH_SHUTDOWN_CHILD", dir.path())
            .env("GRAPH_SHUTDOWN_BARRIER", barrier)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = child.0.stdout.take().unwrap();
    let (send, receive) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut closed = false;
        for line in std::io::BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if line.starts_with("GRAPH_BARRIER:") {
                send.send(line.clone()).unwrap();
            }
            if line == "GRAPH_CLOSED" {
                closed = true;
            }
        }
        closed
    });
    assert_eq!(
        receive.recv_timeout(Duration::from_secs(10)).unwrap(),
        format!("GRAPH_BARRIER:{barrier}")
    );
    match disposition {
        "eof" => {
            drop(child.0.stdin.take());
        }
        "kill" => child.0.kill().unwrap(),
        "term" => {
            assert!(
                Command::new("/bin/kill")
                    .args(["-TERM", &child.0.id().to_string()])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        "crash" => {
            child
                .0
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"crash\n")
                .unwrap();
        }
        _ => unreachable!(),
    }
    let until = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < until, "owner failed to exit");
        std::thread::sleep(Duration::from_millis(5));
    };
    let closed = reader.join().unwrap();
    assert_eq!(closed, disposition == "eof");
    assert_eq!(status.success(), disposition == "eof");
    verify_reopen(dir.path(), barrier, disposition, closed);
}

fn verify_reopen(dir: &Path, barrier: &str, disposition: &str, closed: bool) {
    let file = dir.join("graph.redb");
    let (head, first, pending): (RevisionRef, PreparedCommit, Option<PreparedCommit>) =
        serde_json::from_slice(&std::fs::read(dir.join("owner.json")).unwrap()).unwrap();
    let space = GraphSpace::new("shutdown").unwrap();
    let before = std::fs::read(&file).unwrap();
    for _ in 0..2 {
        if disposition == "eof" {
            assert_eq!(
                Graph::open_existing(&file)
                    .unwrap()
                    .commit_revision(&space)
                    .unwrap(),
                head
            );
        } else {
            assert!(matches!(
                Graph::open_existing(&file),
                Err(GraphError::Commit(Failure::RecoveryRequired))
            ));
        }
        assert!(
            std::fs::read(&file).unwrap() == before,
            "read mutated evidence"
        );
    }
    let graph = if disposition == "eof" {
        Graph::open_existing(&file).unwrap()
    } else {
        Graph::recover_existing(&file).unwrap()
    };
    assert_eq!(graph.commit_revision(&space).unwrap(), head);
    let original = if barrier == "committed" {
        pending.as_ref().unwrap()
    } else {
        &first
    };
    let replay = graph.publish_prepared(&space, original).unwrap();
    assert!(matches!(replay.outcome, CommitOutcome::NoChange(_)));
    assert_eq!(graph.commit_revision(&space).unwrap(), head);
    assert_eq!(head.sequence, if barrier == "committed" { 2 } else { 1 });
    assert_eq!(
        graph.read(&space).unwrap().get_node("second").is_some(),
        barrier == "committed"
    );
    if barrier == "prepared" {
        assert!(
            graph
                .prepared_publication(&space, pending.as_ref().unwrap().reference())
                .unwrap()
                .is_some()
        );
    }
    drop(graph);
    assert_eq!(
        Graph::open_existing(&file)
            .unwrap()
            .commit_revision(&space)
            .unwrap(),
        head
    );
    println!(
        "GRAPH_MATRIX {}",
        serde_json::json!({"barrier":barrier,"shutdown":disposition,"writerClosed":closed,"head":head,"readMutations":0,"explicitRecovery":disposition!="eof","duplicateEffect":0})
    );
}
