#![cfg(feature = "graph")]
use zixcel_graph::{Graph, GraphError, GraphSpace, Node, ReadBudget};
fn limits() -> ReadBudget {
    ReadBudget {
        max_nodes: 2,
        max_edges: 0,
        max_index_entries: 2,
        max_source_bytes: 8192,
    }
}
fn verify(graph: &Graph) {
    let space = GraphSpace::new("synthetic").unwrap();
    let mut tx = graph.write(&space).unwrap();
    tx.upsert_node(Node::new("a", ["fixture"]).unwrap());
    tx.upsert_node(Node::new("b", ["fixture"]).unwrap());
    tx.commit().unwrap();
    let accepted = graph.read_budgeted(&space, limits()).unwrap();
    assert_eq!(accepted.nodes_by_label("fixture", 3).len(), 2);
    for rejected in [
        ReadBudget {
            max_nodes: 1,
            ..limits()
        },
        ReadBudget {
            max_index_entries: 1,
            ..limits()
        },
        ReadBudget {
            max_source_bytes: 1,
            ..limits()
        },
    ] {
        assert!(matches!(
            graph.read_budgeted(&space, rejected),
            Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
        ));
    }
    assert_eq!(graph.read(&space).unwrap().revision(), accepted.revision());
    assert_eq!(
        graph
            .read_budgeted(&space, limits())
            .unwrap()
            .nodes_by_label("fixture", 3)
            .len(),
        2
    );
}
#[test]
fn memory_checks_before_clone_and_never_returns_a_partial_snapshot() {
    verify(&Graph::memory().unwrap());
}
#[test]
#[cfg(feature = "redb")]
fn durable_checks_stored_bytes_before_decode_and_keeps_committed_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.redb");
    let graph = Graph::create(&path).unwrap();
    verify(&graph);
    drop(graph);
    let graph = Graph::open_existing(path).unwrap();
    assert!(
        graph
            .read_budgeted(&GraphSpace::new("synthetic").unwrap(), limits())
            .is_ok()
    );
}
#[test]
fn zero_record_budgets_admit_only_an_absent_space() {
    let graph = Graph::memory().unwrap();
    let zero = ReadBudget {
        max_nodes: 0,
        max_edges: 0,
        max_index_entries: 0,
        max_source_bytes: 0,
    };
    assert_eq!(
        graph
            .read_budgeted(&GraphSpace::new("absent").unwrap(), zero)
            .unwrap()
            .revision(),
        0
    );
}

#[test]
fn an_edge_limit_refuses_the_complete_snapshot() {
    let graph = Graph::memory().unwrap();
    let space = GraphSpace::new("edges").unwrap();
    let mut tx = graph.write(&space).unwrap();
    tx.upsert_node(Node::new("a", ["fixture"]).unwrap());
    tx.upsert_node(Node::new("b", ["fixture"]).unwrap());
    tx.upsert_edge(zixcel_graph::Edge::directed("link", "a", "b", "fixture.edge").unwrap());
    tx.commit().unwrap();
    assert!(matches!(
        graph.read_budgeted(&space, limits()),
        Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
    ));
    let accepted = ReadBudget {
        max_edges: 1,
        max_index_entries: 5,
        ..limits()
    };
    assert!(
        graph
            .read_budgeted(&space, accepted)
            .unwrap()
            .get_edge("link")
            .is_some()
    );
}
