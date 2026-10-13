#![cfg(feature = "graph")]
use std::sync::atomic::{AtomicBool, Ordering};
use zixcel_graph::{Graph, GraphError, GraphSpace, Node, ReadBudget, StorageValidationBudget};
fn budget() -> StorageValidationBudget {
    StorageValidationBudget {
        max_spaces: 2,
        records: ReadBudget {
            max_nodes: 2,
            max_edges: 0,
            max_index_entries: 2,
            max_source_bytes: 65536,
        },
    }
}
fn verify(graph: &Graph) {
    let stop = AtomicBool::new(false);
    for id in ["a", "a/b"] {
        let space = GraphSpace::new(id).unwrap();
        let mut tx = graph.write(&space).unwrap();
        tx.upsert_node(Node::new("fixture", ["fixture"]).unwrap());
        tx.commit().unwrap();
    }
    graph.validate_storage_budgeted(budget(), &stop).unwrap();
    for limits in [
        StorageValidationBudget {
            max_spaces: 1,
            ..budget()
        },
        StorageValidationBudget {
            records: ReadBudget {
                max_nodes: 1,
                ..budget().records
            },
            ..budget()
        },
        StorageValidationBudget {
            records: ReadBudget {
                max_source_bytes: 1,
                ..budget().records
            },
            ..budget()
        },
    ] {
        assert!(matches!(
            graph.validate_storage_budgeted(limits, &stop),
            Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
        ));
    }
    stop.store(true, Ordering::Relaxed);
    assert!(matches!(
        graph.validate_storage_budgeted(budget(), &stop),
        Err(GraphError::Invalid(_))
    ));
    stop.store(false, Ordering::Relaxed);
    graph.validate_storage_budgeted(budget(), &stop).unwrap();
    for id in ["a", "a/b"] {
        assert!(
            graph
                .read(&GraphSpace::new(id).unwrap())
                .unwrap()
                .get_node("fixture")
                .is_some()
        );
    }
}
#[test]
fn memory_storage_admission_is_cumulative_and_cancellation_never_mutates() {
    verify(&Graph::memory().unwrap());
}
#[test]
#[cfg(feature = "redb")]
fn durable_storage_admission_is_cumulative_and_cancellation_never_mutates() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("validation.redb");
    let graph = Graph::create(&file).unwrap();
    verify(&graph);
    drop(graph);
    Graph::open_existing(file)
        .unwrap()
        .validate_storage_budgeted(budget(), &AtomicBool::new(false))
        .unwrap();
}
#[test]
fn empty_storage_accepts_a_zero_budget_without_initializing_a_space() {
    Graph::memory()
        .unwrap()
        .validate_storage_budgeted(
            StorageValidationBudget {
                max_spaces: 0,
                records: ReadBudget {
                    max_nodes: 0,
                    max_edges: 0,
                    max_index_entries: 0,
                    max_source_bytes: 0,
                },
            },
            &AtomicBool::new(false),
        )
        .unwrap();
}

#[cfg(feature = "redb")]
#[test]
fn opening_admits_file_and_logical_limits_without_repair_or_creation() {
    use zixcel_graph::StorageOpeningBudget;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("opening.redb");
    let graph = Graph::create(&file).unwrap();
    verify(&graph);
    drop(graph);
    let original = std::fs::read(&file).unwrap();
    let stop = AtomicBool::new(false);
    let allowed = StorageOpeningBudget {
        max_file_bytes: original.len() as u64,
        validation: budget(),
    };
    let graph = Graph::open_existing_budgeted(&file, allowed, &stop).unwrap();
    let (_, before) = graph.read_exact(&GraphSpace::new("a").unwrap()).unwrap();
    drop(graph);
    let mut refused = allowed;
    refused.max_file_bytes -= 1;
    assert!(matches!(
        Graph::open_existing_budgeted(&file, refused, &stop),
        Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
    ));
    refused = allowed;
    refused.validation.records.max_nodes = 1;
    assert!(matches!(
        Graph::open_existing_budgeted(&file, refused, &stop),
        Err(GraphError::Commit(zixcel_revision::Failure::Capacity))
    ));
    stop.store(true, Ordering::Relaxed);
    assert!(matches!(
        Graph::open_existing_budgeted(&file, allowed, &stop),
        Err(GraphError::Invalid(_))
    ));
    stop.store(false, Ordering::Relaxed);
    let missing = root.path().join("missing.redb");
    assert!(Graph::open_existing_budgeted(&missing, allowed, &stop).is_err());
    assert!(!missing.exists());
    assert_eq!(std::fs::read(&file).unwrap(), original);
    let graph = Graph::open_existing_budgeted(&file, allowed, &stop).unwrap();
    assert_eq!(
        graph.read_exact(&GraphSpace::new("a").unwrap()).unwrap().1,
        before
    );
}
