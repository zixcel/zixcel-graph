//! Actual child processes, physical recovery barrier, exact whole-space oracles.
use super::*;
use crate::{Edge, GraphChanges, GraphSpace, Node, PublicationIntent};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread::JoinHandle,
};
use zixcel_revision::{CommitOutcome, PreparedCommit};

fn identity(graph: &Graph) -> Value {
    let space = GraphSpace::new("lifecycle").unwrap();
    let state = graph.storage.snapshot(&space).unwrap();
    let graph = json!({"revision": state.graph.revision, "nodes": state.graph.nodes,
        "edges":state.graph.edges,"nodeLabels":state.graph.node_labels,
        "edgeLabels":state.graph.edge_labels,"outgoing":state.graph.outgoing,"incoming":state.graph.incoming});
    let revision = serde_json::to_value(state.commits).unwrap();
    json!({"graphDigest":format!("{:x}",Sha256::digest(serde_json::to_vec(&graph).unwrap())),
        "revisionDigest":format!("{:x}",Sha256::digest(serde_json::to_vec(&revision).unwrap())),
        "graph":graph,"revision":revision})
}
fn original_requests(graph: &Graph) {
    let space = GraphSpace::new("lifecycle").unwrap();
    for op in ["a", "b", "c"] {
        if let Some(request) = graph.original_publication_request(&space, op).unwrap() {
            let expected = PublicationIntent {
                operation_id: op.into(),
                expected_commit_revision: request.expected_commit_revision.clone(),
            }
            .request("lifecycle-fixture", &op)
            .unwrap();
            assert_eq!(request.request_digest, expected.request_digest);
            assert_eq!(request.operation_id, op);
            // Original identity is read-only and survives a later current head.
            assert_eq!(
                request.expected_commit_revision.sequence,
                match op {
                    "a" => 0,
                    "b" => 1,
                    "c" => 2,
                    _ => unreachable!(),
                }
            );
        }
    }
    assert!(
        graph
            .original_publication_request(&space, "unknown")
            .unwrap()
            .is_none()
    );
}
fn prepare(graph: &Graph, id: &str) -> PreparedCommit {
    let space = GraphSpace::new("lifecycle").unwrap();
    let request = PublicationIntent {
        operation_id: id.into(),
        expected_commit_revision: graph.commit_revision(&space).unwrap(),
    }
    .request("lifecycle-fixture", &id)
    .unwrap();
    let changes = GraphChanges {
        nodes: vec![
            Node::new(id, ["fixture"]).unwrap(),
            Node::new(format!("{id}-target"), ["fixture"]).unwrap(),
        ],
        edges: vec![
            Edge::directed(format!("{id}-edge"), id, format!("{id}-target"), "link").unwrap(),
        ],
        output: id.as_bytes().to_vec(),
        ..Default::default()
    };
    graph
        .prepare_publication(&space, &request, changes, &[])
        .unwrap()
}
fn child(path: &Path, mode: &str) {
    if mode == "start" {
        match Graph::start_existing(path, |phase| {
            println!("PHASE:{phase:?}");
            std::io::stdout().flush().unwrap();
        }) {
            Ok(started) => {
                println!("IDENTITY:{}", identity(&started.graph));
            }
            Err(GraphError::Unavailable) => println!("UNAVAILABLE"),
            Err(error) => println!("START_FAILED:{error:?}"),
        }
        return;
    }
    let graph = Graph::recover_existing(path).unwrap();
    let space = GraphSpace::new("lifecycle").unwrap();
    if mode == "prepared" || mode == "committed" || mode == "history" {
        let second = prepare(&graph, "b");
        if mode != "prepared" {
            graph.publish_prepared(&space, &second).unwrap();
        }
        if mode == "history" {
            prepare(&graph, "c");
        }
    }
    println!("IDENTITY:{}", identity(&graph));
    println!("BARRIER:{mode}");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "release");
    drop(graph);
}
struct Owned {
    child: Child,
    receiver: mpsc::Receiver<String>,
    reader: Option<JoinHandle<()>>,
}
impl Owned {
    fn spawn(path: &Path, mode: &str, preload: Option<&Path>) -> Self {
        Self::spawn_action(path, mode, preload, "barrier")
    }
    fn spawn_action(path: &Path, mode: &str, preload: Option<&Path>, action: &str) -> Self {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "startup::tests::process_matrix", "--nocapture"])
            .env("GRAPH_LIFECYCLE_FILE", path)
            .env("GRAPH_LIFECYCLE_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(library) = preload {
            cmd.env("LD_PRELOAD", library)
                .env("GRAPH_RECOVERY_FILE", path)
                .env("GRAPH_RECOVERY_ACTION", action);
        }
        let mut child = cmd.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, receiver) = mpsc::sync_channel(32);
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let line = line.unwrap();
                assert!(line.len() < 262_144);
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            receiver,
            reader: Some(reader),
        }
    }
    fn until(&self, prefix: &str) -> String {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let line = self
                .receiver
                .recv_timeout(end.saturating_duration_since(Instant::now()))
                .unwrap();
            if line.starts_with(prefix) {
                return line;
            }
        }
    }
    fn release(&mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"release\n")
            .unwrap();
    }
    fn finish(&mut self, killed: bool) {
        if killed {
            self.child.kill().unwrap();
        }
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.success(), !killed);
                break;
            }
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        self.reader.take().unwrap().join().unwrap();
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        while self.receiver.try_recv().is_ok() {}
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
fn base(path: &Path) {
    let graph = Graph::create(path).unwrap();
    let a = prepare(&graph, "a");
    assert!(matches!(
        graph
            .publish_prepared(&GraphSpace::new("lifecycle").unwrap(), &a)
            .unwrap()
            .outcome,
        CommitOutcome::Committed(_)
    ));
}
fn raw_unchanged(path: &Path, dirty: bool) {
    let before = std::fs::read(path).unwrap();
    for _ in 0..2 {
        match Graph::open_existing(path) {
            Err(GraphError::Commit(Failure::RecoveryRequired)) if dirty => {}
            Ok(_) if !dirty => {}
            other => panic!("unexpected raw read: {}", other.is_ok()),
        }
        assert_eq!(std::fs::read(path).unwrap(), before);
    }
}
fn run_case(id: &str, mode: &str, killed: bool, rounds: usize) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.redb");
    base(&path);
    let mut last = None;
    for _ in 0..rounds {
        let mut writer = Owned::spawn(&path, mode, None);
        let expected: Value =
            serde_json::from_str(writer.until("IDENTITY:").strip_prefix("IDENTITY:").unwrap())
                .unwrap();
        writer.until("BARRIER:");
        if !killed {
            writer.release();
        }
        writer.finish(killed);
        raw_unchanged(&path, killed);
        let mut phases = vec![];
        let ready = Graph::start_existing(&path, |phase| phases.push(phase)).unwrap();
        assert_eq!(identity(&ready.graph), expected);
        original_requests(&ready.graph);
        assert_eq!(ready.recovery_performed, killed);
        assert_eq!(
            phases,
            if killed {
                vec![
                    StoragePhase::Opening,
                    StoragePhase::RecoveryRequired,
                    StoragePhase::Recovering,
                    StoragePhase::Ready,
                ]
            } else {
                vec![StoragePhase::Opening, StoragePhase::Ready]
            }
        );
        let before = std::fs::read(&path).unwrap();
        assert_eq!(identity(&Graph::open_existing(&path).unwrap()), expected);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        if let Some(prior) = last {
            assert_eq!(expected, prior);
        }
        last = Some(expected);
        println!(
            "GRAPH_STARTUP_CASE:{}",
            json!({"id":id,"mode":mode,"killed":killed,"phases":format!("{phases:?}"),"logicalEquality":true,"oracle":last})
        );
    }
}
#[test]
fn process_matrix() {
    if let Ok(file) = std::env::var("GRAPH_LIFECYCLE_FILE") {
        child(
            Path::new(&file),
            &std::env::var("GRAPH_LIFECYCLE_MODE").unwrap(),
        );
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let clean = dir.path().join("clean.redb");
    base(&clean);
    let original = identity(&Graph::open_existing(&clean).unwrap());
    let ready = Graph::start_existing(&clean, |_| {}).unwrap();
    assert!(!ready.recovery_performed);
    assert_eq!(identity(&ready.graph), original);
    drop(ready);
    println!(
        "GRAPH_STARTUP_CASE:{}",
        json!({"id":"A","logicalEquality":true,"oracle":original})
    );
    for (id, mode, killed) in [
        ("B", "writer", false),
        ("C", "writer", true),
        ("D", "prepared", false),
        ("E", "prepared", true),
        ("F", "committed", false),
        ("G", "committed", true),
        ("H", "history", true),
    ] {
        run_case(id, mode, killed, 1);
    }
    run_case("I", "writer", true, 3);
    recovery_interruption_and_contenders(dir.path());
}
fn recovery_interruption_and_contenders(dir: &Path) {
    let library = dir.join("barrier.so");
    let compiled = Command::new("cc")
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&library)
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/recovery_barrier.c"))
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    assert!(compiled.stderr.is_empty());
    let path = dir.join("recovery.redb");
    base(&path);
    let mut writer = Owned::spawn(&path, "history", None);
    let expected: Value =
        serde_json::from_str(writer.until("IDENTITY:").strip_prefix("IDENTITY:").unwrap()).unwrap();
    writer.until("BARRIER:");
    writer.finish(true);
    let mut recovery = Owned::spawn(&path, "start", Some(&library));
    recovery.until("GRAPH_RECOVERY_SYNC");
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        Graph::open_existing(&path),
        Err(GraphError::Unavailable)
    ));
    assert!(matches!(
        Graph::start_existing(&path, |_| {}),
        Err(GraphError::Unavailable)
    ));
    let mut contender = Owned::spawn(&path, "start", None);
    contender.until("UNAVAILABLE");
    contender.finish(false);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    recovery.finish(true);
    let next = Graph::start_existing(&path, |_| {}).unwrap();
    assert_eq!(identity(&next.graph), expected);
    drop(next);
    let mut writer = Owned::spawn(&path, "writer", None);
    writer.until("BARRIER:");
    writer.finish(true);
    let mut failure = Owned::spawn_action(&path, "start", Some(&library), "fail");
    failure.until("PHASE:Unavailable");
    assert_eq!(
        failure.until("START_FAILED:"),
        "START_FAILED:Commit(Storage)"
    );
    failure.finish(false);
    let next = Graph::start_existing(&path, |_| {}).unwrap();
    assert_eq!(identity(&next.graph), expected);
    println!(
        "GRAPH_STARTUP_CASE:{}",
        json!({"id":"J/K/recovery-interruption","actualRecoverySync":true,"readerUnavailable":true,"contenderUnavailable":true,"recoveryFailureUnavailable":true,"logicalEquality":true,"oracle":expected})
    );
}
