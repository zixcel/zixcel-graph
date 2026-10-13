//! Synthetic write/read/refusal playground; no existing DB or cloud account.
use zixcel_graph::{
    BoundedWriter, Graph, GraphSpace, Node, ReadBudget, ReadLimits, WriteBudget, WriteLimits,
};
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
    let writer = BoundedWriter::new(
        graph.clone(),
        WriteLimits {
            transaction: budget,
            max_pending_transactions: 1,
        },
    )?;
    let held = writer.write(&space)?;
    assert!(writer.clone().write(&space).is_err());
    drop(held);
    let mut write = writer.write(&space)?;
    write.upsert_node(Node::new("example", ["example"])?)?;
    let committed = write.commit()?;
    assert_eq!(writer.pending_transactions(), 0);
    let graph = graph.with_read_limits(ReadLimits {
        snapshot: budget.snapshot,
        max_retained_snapshots: 1,
    })?;
    let (snapshot, revision) = graph.read_exact_budgeted(&space, budget.snapshot)?;
    assert_eq!(graph.retained_reads(), Some(1));
    assert!(graph.read(&space).is_err());
    assert_eq!(committed, revision.sequence);
    assert!(snapshot.get_node("example").is_some());
    drop(snapshot);
    let mut refused = writer.write_budgeted(
        &space,
        WriteBudget {
            max_updates: 0,
            ..budget
        },
    )?;
    assert!(refused.remove_node("example").is_err());
    assert!(refused.commit().is_err());
    assert_eq!(writer.pending_transactions(), 0);
    assert_eq!(
        graph.read_exact_budgeted(&space, budget.snapshot)?.1,
        revision
    );
    println!("bounded synthetic write/read/refusal accepted; no canonical prefix on refusal");
    Ok(())
}
