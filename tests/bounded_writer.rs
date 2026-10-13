#![cfg(feature = "graph")]
use zixcel_graph::{
    BoundedWriter, Graph, GraphError, GraphSpace, Node, ReadBudget, WriteBudget, WriteLimits,
};
fn limits() -> WriteLimits {
    WriteLimits {
        transaction: WriteBudget {
            snapshot: ReadBudget {
                max_nodes: 2,
                max_edges: 1,
                max_index_entries: 8,
                max_source_bytes: 65536,
            },
            max_updates: 1,
            max_update_bytes: 4096,
        },
        max_pending_transactions: 2,
    }
}
fn capacity<T>(result: &Result<T, GraphError>) {
    assert!(matches!(
        result,
        Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
    ));
}
fn journey(graph: &Graph) {
    let space = GraphSpace::new("pending").unwrap();
    let writer = BoundedWriter::new(graph.clone(), limits()).unwrap();
    let peer = writer.clone();
    let mut first = writer.write(&space).unwrap();
    let stale = peer.write(&space).unwrap();
    assert_eq!(writer.pending_transactions(), 2);
    capacity(&peer.write(&space));
    first
        .upsert_node(Node::new("first", ["fixture"]).unwrap())
        .unwrap();
    first.commit().unwrap();
    assert_eq!(writer.pending_transactions(), 1);
    assert!(stale.commit().is_err());
    assert_eq!(writer.pending_transactions(), 0);
    // Enlarging a per-call budget cannot relax the configured family ceiling.
    let mut relaxed = limits().transaction;
    relaxed.max_updates = usize::MAX;
    relaxed.max_update_bytes = usize::MAX;
    let mut refused = peer.write_budgeted(&space, relaxed).unwrap();
    refused
        .upsert_node(Node::new("second", ["fixture"]).unwrap())
        .unwrap();
    capacity(&refused.upsert_node(Node::new("third", ["fixture"]).unwrap()));
    capacity(&refused.commit());
    assert_eq!(writer.pending_transactions(), 0);
    let snapshot = graph.read(&space).unwrap();
    assert!(snapshot.get_node("first").is_some());
    assert!(snapshot.get_node("second").is_none());
    let held = writer.write(&space).unwrap();
    assert_eq!(writer.pending_transactions(), 1);
    drop(held);
    assert_eq!(writer.pending_transactions(), 0);
    let mut strict = limits().transaction;
    strict.snapshot.max_nodes = 0;
    capacity(&writer.write_budgeted(&space, strict));
    assert_eq!(writer.pending_transactions(), 0);
}
#[test]
fn memory_pending_writes_release_and_preserve_atomic_state() {
    journey(&Graph::memory().unwrap());
}
#[cfg(feature = "redb")]
#[test]
fn durable_pending_writes_release_and_preserve_atomic_state() {
    let dir = tempfile::tempdir().unwrap();
    journey(&Graph::create(dir.path().join("fixture.redb")).unwrap());
}
#[test]
fn clones_cannot_exceed_concurrent_slots_or_reset_limits() {
    let mut policy = limits();
    policy.max_pending_transactions = 1;
    let writer = BoundedWriter::new(Graph::memory().unwrap(), policy).unwrap();
    let space = GraphSpace::new("concurrent").unwrap();
    let held = writer.write(&space).unwrap();
    let peer = writer.clone();
    let result = std::thread::spawn(move || peer.write(&space))
        .join()
        .unwrap();
    capacity(&result);
    drop(held);
    assert_eq!(writer.pending_transactions(), 0);
    policy.max_pending_transactions = 0;
    assert!(BoundedWriter::new(Graph::memory().unwrap(), policy).is_err());
    policy.max_pending_transactions = usize::MAX;
    assert!(BoundedWriter::new(Graph::memory().unwrap(), policy).is_err());
}
