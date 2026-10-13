#![cfg(feature = "graph")]
use zixcel_graph::{Graph, GraphError, GraphSpace, Node, ReadBudget, WriteBudget};
fn limits() -> WriteBudget {
    WriteBudget {
        snapshot: ReadBudget {
            max_nodes: 2,
            max_edges: 1,
            max_index_entries: 5,
            max_source_bytes: 65536,
        },
        max_updates: 3,
        max_update_bytes: 4096,
    }
}
fn capacity<T>(result: &Result<T, GraphError>) {
    assert!(matches!(
        result,
        Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
    ));
}
fn verify(graph: &Graph) {
    let space = GraphSpace::new("bounded-write").unwrap();
    let mut tx = graph.write_budgeted(&space, limits()).unwrap();
    tx.upsert_node(Node::new("a", ["fixture"]).unwrap())
        .unwrap();
    tx.upsert_node(Node::new("b", ["fixture"]).unwrap())
        .unwrap();
    tx.upsert_edge(zixcel_graph::Edge::directed("ab", "a", "b", "link").unwrap())
        .unwrap();
    let revision = tx.commit().unwrap();
    let (accepted, identity) = graph
        .read_exact_budgeted(&space, limits().snapshot)
        .unwrap();
    assert_eq!(revision, identity.sequence);
    assert!(accepted.get_edge("ab").is_some());
    let mut tx = graph
        .write_budgeted(
            &space,
            WriteBudget {
                max_updates: 1,
                ..limits()
            },
        )
        .unwrap();
    tx.remove_edge("ab").unwrap();
    capacity(&tx.upsert_node(Node::new("new", ["fixture"]).unwrap()));
    capacity(&tx.commit());
    assert_eq!(graph.read_exact(&space).unwrap().1, identity);
    assert!(graph.read(&space).unwrap().get_edge("ab").is_some());
    let mut tx = graph
        .write_budgeted(
            &space,
            WriteBudget {
                max_update_bytes: 1,
                ..limits()
            },
        )
        .unwrap();
    capacity(&tx.remove_edge("ab"));
    capacity(&tx.commit());
    assert_eq!(graph.read_exact(&space).unwrap().1, identity);
    let mut tx = graph.write_budgeted(&space, limits()).unwrap();
    tx.upsert_node(Node::new("too-many", ["fixture"]).unwrap())
        .unwrap();
    capacity(&tx.commit());
    assert_eq!(graph.read_exact(&space).unwrap().1, identity);
    assert!(graph.read(&space).unwrap().get_node("too-many").is_none());
    let mut tx = graph.write_budgeted(&space, limits()).unwrap();
    assert!(tx.remove_node("a").is_err());
    capacity(&tx.commit());
    assert_eq!(graph.read_exact(&space).unwrap().1, identity);
    let mut tx = graph.write_budgeted(&space, limits()).unwrap();
    tx.remove_edge("ab").unwrap();
    tx.remove_node("a").unwrap();
    tx.commit().unwrap();
    assert!(graph.read(&space).unwrap().get_node("a").is_none());
    assert!(accepted.get_node("a").is_some());
}
#[test]
fn memory_mutation_refusal_never_commits_a_successful_prefix() {
    verify(&Graph::memory().unwrap());
}
#[test]
#[cfg(feature = "redb")]
fn durable_mutation_refusal_never_commits_a_successful_prefix() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("bounded.redb");
    let graph = Graph::create(&file).unwrap();
    verify(&graph);
    drop(graph);
    let graph = Graph::open_existing(file).unwrap();
    assert!(
        graph
            .read(&GraphSpace::new("bounded-write").unwrap())
            .unwrap()
            .get_node("a")
            .is_none()
    );
}
#[test]
fn final_receipt_bytes_refuse_before_canonical_publication() {
    let graph = Graph::memory().unwrap();
    let space = GraphSpace::new("receipt-quota").unwrap();
    let small = WriteBudget {
        snapshot: ReadBudget {
            max_source_bytes: 1,
            ..limits().snapshot
        },
        ..limits()
    };
    let mut tx = graph.write_budgeted(&space, small).unwrap();
    tx.upsert_node(Node::new("fixture", ["fixture"]).unwrap())
        .unwrap();
    capacity(&tx.commit());
    assert_eq!(graph.read(&space).unwrap().revision(), 0);
    assert!(graph.read(&space).unwrap().get_node("fixture").is_none());
}
#[test]
fn existing_state_and_zero_update_capacity_are_explicit() {
    let graph = Graph::memory().unwrap();
    let space = GraphSpace::new("admission").unwrap();
    let mut tx = graph
        .write_budgeted(
            &space,
            WriteBudget {
                max_updates: 0,
                ..limits()
            },
        )
        .unwrap();
    capacity(&tx.upsert_node(Node::new("x", ["fixture"]).unwrap()));
    capacity(&tx.commit());
    let mut tx = graph.write(&space).unwrap();
    for id in ["a", "b", "c"] {
        tx.upsert_node(Node::new(id, ["fixture"]).unwrap());
    }
    tx.commit().unwrap();
    capacity(&graph.write_budgeted(&space, limits()));
}
#[test]
fn bounded_writer_preserves_the_concurrent_writers_revision() {
    let graph = Graph::memory().unwrap();
    let space = GraphSpace::new("concurrent").unwrap();
    let mut old = graph.write_budgeted(&space, limits()).unwrap();
    old.upsert_node(Node::new("old", ["fixture"]).unwrap())
        .unwrap();
    let mut current = graph.write_budgeted(&space, limits()).unwrap();
    current
        .upsert_node(Node::new("current", ["fixture"]).unwrap())
        .unwrap();
    current.commit().unwrap();
    let identity = graph.read_exact(&space).unwrap().1;
    assert!(matches!(old.commit(), Err(GraphError::CommitConflict(_))));
    assert_eq!(graph.read_exact(&space).unwrap().1, identity);
    assert!(graph.read(&space).unwrap().get_node("old").is_none());
}
