//! Synthetic write/read/refusal playground; no existing DB or cloud account.
use zixcel_graph::{Graph, GraphSpace, Node, ReadBudget, WriteBudget};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = Graph::memory()?;
    let space = GraphSpace::new("playground")?;
    let budget = WriteBudget {
        snapshot: ReadBudget {
            max_nodes: 2,
            max_edges: 0,
            max_index_entries: 2,
            max_source_bytes: 65536,
        },
        max_updates: 2,
        max_update_bytes: 2048,
    };
    let mut write = graph.write_budgeted(&space, budget)?;
    write.upsert_node(Node::new("example", ["example"])?)?;
    let committed = write.commit()?;
    let (snapshot, revision) = graph.read_exact_budgeted(&space, budget.snapshot)?;
    assert_eq!(committed, revision.sequence);
    assert!(snapshot.get_node("example").is_some());
    let mut refused = graph.write_budgeted(
        &space,
        WriteBudget {
            max_updates: 0,
            ..budget
        },
    )?;
    assert!(refused.remove_node("example").is_err());
    assert!(refused.commit().is_err());
    assert_eq!(
        graph.read_exact_budgeted(&space, budget.snapshot)?.1,
        revision
    );
    println!("bounded synthetic write/read/refusal accepted; no canonical prefix on refusal");
    Ok(())
}
