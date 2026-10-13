#![cfg(feature = "graph")]
use zixcel_graph::{Graph, GraphError, GraphSpace, Node, ReadBudget, ReadLimits};
use zixcel_revision::Failure;
fn limits(max_nodes: usize) -> ReadLimits {
    ReadLimits {
        snapshot: ReadBudget {
            max_nodes,
            max_edges: 2,
            max_index_entries: 8,
            max_source_bytes: 65536,
        },
        max_retained_snapshots: 2,
    }
}
fn seed(graph: &Graph, space: &GraphSpace) {
    let mut writer = graph.write(space).unwrap();
    writer.upsert_node(Node::new("first", ["fixture"]).unwrap());
    writer.commit().unwrap();
}
fn journey(graph: Graph) {
    let space = GraphSpace::new("retained").unwrap();
    seed(&graph, &space);
    let graph = graph.with_read_limits(limits(1)).unwrap();
    let peer = graph.clone();
    let old = graph.read(&space).unwrap();
    let (exact, revision) = peer.read_exact(&space).unwrap();
    assert_eq!(graph.retained_reads(), Some(2));
    assert!(matches!(
        peer.read(&space),
        Err(GraphError::Commit(Failure::Capacity))
    ));
    assert!(matches!(
        peer.read_budgeted(&space, limits(99).snapshot),
        Err(GraphError::Commit(Failure::Capacity))
    ));
    assert_eq!(exact.revision(), revision.sequence);
    drop(exact);
    let current = peer
        .read_exact_budgeted(&space, limits(99).snapshot)
        .unwrap();
    assert!(old.get_node("first").is_some());
    assert_eq!(current.1, revision);
    drop(current);
    drop(old);
    assert_eq!(peer.retained_reads(), Some(0));
    let mut writer = graph.write(&space).unwrap();
    writer.upsert_node(Node::new("second", ["fixture"]).unwrap());
    writer.commit().unwrap();
    // A larger caller budget cannot relax the configured family limit, nor leak a reservation.
    assert!(matches!(
        peer.read_budgeted(&space, limits(99).snapshot),
        Err(GraphError::Commit(Failure::Capacity))
    ));
    assert_eq!(graph.retained_reads(), Some(0));
    assert!(matches!(
        graph.read_exact(&space),
        Err(GraphError::Commit(Failure::Capacity))
    ));
    assert_eq!(graph.retained_reads(), Some(0));
}
#[test]
fn memory_retention_release_exact_reads_and_refusal() {
    journey(Graph::memory().unwrap());
}
#[cfg(feature = "redb")]
#[test]
fn durable_retention_release_exact_reads_and_refusal() {
    let dir = tempfile::tempdir().unwrap();
    journey(Graph::create(dir.path().join("fixture.redb")).unwrap());
}
#[test]
fn concurrent_clones_share_admission_and_configuration_cannot_reset_it() {
    let space = GraphSpace::new("concurrent").unwrap();
    let graph = Graph::memory().unwrap();
    seed(&graph, &space);
    let mut policy = limits(1);
    policy.max_retained_snapshots = 1;
    let graph = graph.with_read_limits(policy).unwrap();
    let held = graph.read(&space).unwrap();
    let peer = graph.clone();
    let target = space.clone();
    let denied = std::thread::spawn(move || peer.read(&target))
        .join()
        .unwrap();
    assert!(matches!(denied, Err(GraphError::Commit(Failure::Capacity))));
    assert!(graph.clone().with_read_limits(limits(99)).is_err());
    assert_eq!(graph.retained_reads(), Some(1));
    drop(held);
    assert!(graph.read(&space).is_ok());
    policy.max_retained_snapshots = 0;
    assert!(Graph::memory().unwrap().with_read_limits(policy).is_err());
    policy.max_retained_snapshots = usize::MAX;
    assert!(Graph::memory().unwrap().with_read_limits(policy).is_err());
}
