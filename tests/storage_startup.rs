//! Graph startup is an explicit mutation boundary; queries never recover storage.
#![cfg(all(feature = "graph", feature = "redb"))]
use zixcel_graph::{Graph, GraphError, GraphSpace, Node, StoragePhase};
use zixcel_revision::Failure;

#[test]
fn clean_start_is_read_only_and_invalid_logical_state_is_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.redb");
    let space = GraphSpace::new("startup").unwrap();
    {
        let graph = Graph::create(&path).unwrap();
        let mut write = graph.write(&space).unwrap();
        write.upsert_node(Node::new("a", ["fixture"]).unwrap());
        write.commit().unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let mut phases = vec![];
    let started = Graph::start_existing(&path, |p| phases.push(p)).unwrap();
    assert_eq!(phases, [StoragePhase::Opening, StoragePhase::Ready]);
    assert!(!started.recovery_performed);
    assert!(started.graph.read(&space).unwrap().get_node("a").is_some());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    drop(started);
    {
        let db = redb::Database::open(&path).unwrap();
        let tx = db.begin_write().unwrap();
        tx.open_table(redb::TableDefinition::<&str, u64>::new(
            "zixcel_graph_spaces",
        ))
        .unwrap()
        .insert("startup", 99)
        .unwrap();
        tx.commit().unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    phases.clear();
    assert!(matches!(
        Graph::start_existing(&path, |p| phases.push(p)),
        Err(GraphError::Commit(Failure::Corrupt))
    ));
    assert_eq!(phases, [StoragePhase::Opening, StoragePhase::Unavailable]);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(matches!(
        Graph::start_existing(dir.path().join("missing"), |_| {}),
        Err(GraphError::Missing)
    ));
    assert!(matches!(
        Graph::open_existing(dir.path().join("missing")),
        Err(GraphError::Missing)
    ));
    assert!(!dir.path().join("missing").exists());
    assert!(!dir.path().join("missing.lifecycle.lock").exists());
}

#[test]
fn unknown_and_orphan_storage_is_not_skipped_and_lifecycle_aliases_are_rejected() {
    for orphan in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.redb");
        drop(Graph::create(&path).unwrap());
        {
            let db = redb::Database::open(&path).unwrap();
            let tx = db.begin_write().unwrap();
            let table = redb::TableDefinition::<&str, &[u8]>::new(if orphan {
                "zixcel_graph_nodes"
            } else {
                "unknown"
            });
            tx.open_table(table)
                .unwrap()
                .insert("orphan\0node", b"{}".as_slice())
                .unwrap();
            tx.commit().unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(
            Graph::start_existing(&path, |_| {}),
            Err(GraphError::Commit(Failure::Corrupt))
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.redb");
        drop(Graph::create(&path).unwrap());
        let alias = dir.path().join("alias.redb");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(matches!(
            Graph::start_existing(&alias, |_| {}),
            Err(GraphError::InvalidPath(_))
        ));
        assert!(!dir.path().join("alias.redb.lifecycle.lock").exists());
    }
}
