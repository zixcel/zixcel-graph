#![cfg(all(feature = "graph", feature = "redb"))]
use zixcel_graph::{Edge, Graph, GraphSpace, Node};
#[test]
fn shared_prefix_spaces_keep_exact_indexes_during_updates_deletes_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("synthetic.redb");
    let graph = Graph::create(&path).unwrap();
    for name in ["a", "a/b", "a-1", "aa"] {
        let space = GraphSpace::new(name).unwrap();
        let mut tx = graph.write(&space).unwrap();
        tx.upsert_node(Node::new("first", ["old.label"]).unwrap());
        tx.upsert_node(Node::new("second", ["old.label"]).unwrap());
        tx.upsert_edge(Edge::directed("link", "first", "second", "old.edge").unwrap());
        tx.commit().unwrap();
    }
    let target = GraphSpace::new("a").unwrap();
    let retained = graph.read(&target).unwrap();
    let mut tx = graph.write(&target).unwrap();
    tx.remove_edge("link");
    tx.remove_node("second").unwrap();
    tx.upsert_node(Node::new("first", ["new.label"]).unwrap());
    tx.commit().unwrap();
    assert!(retained.get_edge("link").is_some());
    drop(retained);
    drop(graph);
    let graph = Graph::open_existing(&path).unwrap();
    let read = graph.read(&target).unwrap();
    assert!(read.get_node("second").is_none());
    assert!(read.get_edge("link").is_none());
    assert!(read.nodes_by_label("old.label", 10).is_empty());
    assert_eq!(read.nodes_by_label("new.label", 10).len(), 1);
    for name in ["a/b", "a-1", "aa"] {
        let read = graph.read(&GraphSpace::new(name).unwrap()).unwrap();
        assert_eq!(read.nodes_by_label("old.label", 10).len(), 2);
        assert!(read.get_edge("link").is_some());
    }
}
