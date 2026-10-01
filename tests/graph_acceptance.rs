use std::collections::BTreeSet;
use tempfile::tempdir;
use zixcel_graph::{Direction, Edge, Graph, GraphError, GraphSpace, Node, Traversal};

#[test]
fn embedded_commit_reopen_and_rollback_are_atomic() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("graph.redb");
    assert!(Graph::open_existing(&path).is_err());
    assert!(!path.exists());
    let absent = directory.path().join("absent/graph.redb");
    assert!(Graph::open_existing(&absent).is_err());
    assert!(!directory.path().join("absent").exists());
    assert!(Graph::open_existing(directory.path()).is_err());
    let zero = directory.path().join("zero.redb");
    std::fs::write(&zero, []).unwrap();
    assert!(Graph::open_existing(&zero).is_err());
    assert!(Graph::create(&zero).is_err());
    assert_eq!(std::fs::metadata(&zero).unwrap().len(), 0);
    let empty = directory.path().join("empty.redb");
    drop(redb::Database::create(&empty).unwrap());
    assert!(Graph::open_existing(&empty).is_err());
    assert!(
        Graph::create(&empty).is_err(),
        "existing incomplete storage is not an initialization request"
    );
    let space = GraphSpace::new("acceptance").unwrap();
    {
        let graph = Graph::create(&path).unwrap();
        let mut tx = graph.write(&space).unwrap();
        tx.upsert_node(Node::new("a", ["fixture.node"]).unwrap());
        tx.upsert_node(Node::new("b", ["fixture.node"]).unwrap());
        tx.upsert_edge(Edge::directed("a-b", "a", "b", "fixture.link").unwrap());
        assert_eq!(tx.commit().unwrap(), 1);

        let mut dropped = graph.write(&space).unwrap();
        dropped.upsert_node(Node::new("not-committed", ["fixture.node"]).unwrap());
    }
    let reopened = Graph::open_existing(&path).unwrap();
    let read = reopened.read(&space).unwrap();
    assert!(read.get_node("a").is_some());
    assert!(read.get_edge("a-b").is_some());
    assert!(read.get_node("not-committed").is_none());
    assert_eq!(read.revision(), 1);
    drop(reopened);
    let database = redb::Database::open(&path).unwrap();
    let write = database.begin_write().unwrap();
    {
        let mut metadata = write
            .open_table(redb::TableDefinition::<&str, &str>::new(
                "zixcel_graph_metadata",
            ))
            .unwrap();
        metadata
            .insert("migration_history", "explicit-reviewed-migration")
            .unwrap();
    }
    write.commit().unwrap();
    drop(database);
    let graph = Graph::create(&path).unwrap();
    assert_eq!(
        graph.metadata().unwrap().migration_history,
        vec!["explicit-reviewed-migration"]
    );
    assert_eq!(graph.read(&space).unwrap().revision(), 1);
    drop(graph);
    #[cfg(unix)]
    {
        let linked = directory.path().join("linked.redb");
        std::os::unix::fs::symlink(&path, &linked).unwrap();
        assert!(matches!(
            Graph::open_existing(&linked),
            Err(GraphError::InvalidPath(_))
        ));
        assert!(matches!(
            Graph::create(&linked),
            Err(GraphError::InvalidPath(_))
        ));
    }
    let database = redb::Database::open(&path).unwrap();
    let write = database.begin_write().unwrap();
    write
        .delete_table(redb::TableDefinition::<&str, u8>::new(
            "zixcel_graph_incoming",
        ))
        .unwrap();
    write.commit().unwrap();
    drop(database);
    assert!(Graph::open_existing(&path).is_err());
    assert!(
        Graph::create(&path).is_err(),
        "read must not repair a missing index table"
    );
}

#[test]
fn parallel_directional_edges_and_bounded_traversal_are_exact() {
    let graph = Graph::memory().unwrap();
    let space = GraphSpace::new("relations").unwrap();
    let mut tx = graph.write(&space).unwrap();
    for id in ["a", "b", "c"] {
        tx.upsert_node(Node::new(id, ["fixture.node"]).unwrap());
    }
    tx.upsert_edge(Edge::directed("owns", "a", "b", "owns").unwrap());
    tx.upsert_edge(Edge::directed("manages", "a", "b", "manages").unwrap());
    tx.upsert_edge(Edge::undirected("link", "b", "c", "linked-to").unwrap());
    tx.commit().unwrap();

    let read = graph.read(&space).unwrap();
    assert_eq!(
        ids(read.outgoing_edges("a", None)),
        BTreeSet::from(["manages".to_owned(), "owns".to_owned()])
    );
    assert_eq!(
        ids(read.incoming_edges("b", None)),
        BTreeSet::from(["manages".to_owned(), "owns".to_owned()])
    );
    assert_eq!(read.neighbors("c", Direction::Outgoing, None), vec!["b"]);
    let traversal = read
        .traverse(&Traversal::new("a", Direction::Outgoing, 2, 16))
        .unwrap();
    assert_eq!(traversal.visited, vec!["a", "b", "c"]);
    assert_eq!(traversal.paths.last().unwrap().nodes, vec!["a", "b", "c"]);
}

#[test]
fn memory_and_redb_match_and_indexes_rebuild_from_primary_records() {
    let directory = tempdir().unwrap();
    let memory = Graph::memory().unwrap();
    let redb = Graph::create(directory.path().join("parity.redb")).unwrap();
    let space = GraphSpace::new("parity").unwrap();
    for graph in [&memory, &redb] {
        write_fixture(graph, &space);
        graph.rebuild_indexes(&space).unwrap();
    }
    let memory_read = memory.read(&space).unwrap();
    let redb_read = redb.read(&space).unwrap();
    assert_eq!(
        memory_read.nodes_by_label("fixture.router", 16),
        redb_read.nodes_by_label("fixture.router", 16)
    );
    assert_eq!(
        memory_read.outgoing_edges("router-a", Some("fixture.link")),
        redb_read.outgoing_edges("router-a", Some("fixture.link"))
    );
    assert_eq!(memory.metadata().unwrap().format_version, 1);
    assert_eq!(redb.metadata().unwrap().backend, "redb");

    let first = redb.write(&space).unwrap();
    let mut second = redb.write(&space).unwrap();
    second.upsert_node(Node::new("new", ["fixture.node"]).unwrap());
    second.commit().unwrap();
    assert!(matches!(
        first.commit(),
        Err(GraphError::CommitConflict(
            zixcel_revision::Conflict::Stale { .. }
        ))
    ));
}

fn write_fixture(graph: &Graph, space: &GraphSpace) {
    let mut tx = graph.write(space).unwrap();
    tx.upsert_node(Node::new("router-a", ["fixture.router"]).unwrap());
    tx.upsert_node(Node::new("router-b", ["fixture.router"]).unwrap());
    tx.upsert_edge(Edge::directed("route", "router-a", "router-b", "fixture.link").unwrap());
    tx.commit().unwrap();
}

fn ids(edges: Vec<Edge>) -> BTreeSet<String> {
    edges.into_iter().map(|edge| edge.id).collect()
}
