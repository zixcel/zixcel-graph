#![cfg(all(feature = "graph", feature = "redb"))]
use std::sync::atomic::AtomicBool;
use zixcel_graph::{Graph, GraphError, GraphSpace, Node, ReadBudget, StorageValidationBudget};
use zixcel_revision::{Failure, durable::Handle};
fn budget() -> StorageValidationBudget {
    StorageValidationBudget {
        max_spaces: 1,
        records: ReadBudget {
            max_nodes: 2,
            max_edges: 0,
            max_index_entries: 4,
            max_source_bytes: 65536,
        },
    }
}
fn seed(path: &std::path::Path, space: &GraphSpace) -> u64 {
    let graph = Graph::create(path).unwrap();
    let mut write = graph.write(space).unwrap();
    write.upsert_node(Node::new("first", ["fixture"]).unwrap());
    write.commit().unwrap()
}
#[test]
fn shared_read_only_handle_preserves_values_and_canonical_revision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.redb");
    let space = GraphSpace::new("shared").unwrap();
    let revision = seed(&path, &space);
    let before = std::fs::read(&path).unwrap();
    let handle = Handle::open(&path).unwrap();
    let graph =
        Graph::from_existing_durable_handle(handle, budget(), &AtomicBool::new(false)).unwrap();
    let read = graph.read(&space).unwrap();
    assert!(read.get_node("first").is_some());
    assert_eq!(read.revision(), revision);
    drop(read);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut write = graph.write(&space).unwrap();
    write.upsert_node(Node::new("second", ["fixture"]).unwrap());
    assert_eq!(write.commit().unwrap(), revision + 1);
}
#[test]
fn capacity_cancel_and_blank_store_refusals_never_initialize_or_mutate_logical_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.redb");
    let space = GraphSpace::new("refusal").unwrap();
    seed(&path, &space);
    let before = std::fs::read(&path).unwrap();
    let mut small = budget();
    small.records.max_nodes = 0;
    assert!(matches!(
        Graph::from_existing_durable_handle(
            Handle::open(&path).unwrap(),
            small,
            &AtomicBool::new(false)
        ),
        Err(GraphError::Commit(Failure::Capacity))
    ));
    assert!(
        Graph::from_existing_durable_handle(
            Handle::open(&path).unwrap(),
            budget(),
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let blank = dir.path().join("blank.redb");
    drop(redb::Database::create(&blank).unwrap());
    let before = std::fs::read(&blank).unwrap();
    assert!(
        Graph::from_existing_durable_handle(
            Handle::open(&blank).unwrap(),
            budget(),
            &AtomicBool::new(false)
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&blank).unwrap(), before);
}
