//! Independent immutable-archive consumer. Generic Graph inspection only;
//! database path, Graph space and label are supplied by the external scenario.
use serde_json::json;
use zixcel_graph::{Graph, GraphSpace};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err("expected mode path space label".into());
    }
    let mut phases = Vec::new();
    let graph = match args[1].as_str() {
        "start" => {
            Graph::start_existing(&args[2], |phase| phases.push(format!("{phase:?}")))?.graph
        }
        "read" => Graph::open_existing(&args[2])?,
        _ => return Err("unknown mode".into()),
    };
    let space = GraphSpace::new(&args[3])?;
    let preparations = graph
        .prepared_references(&space)?
        .into_iter()
        .map(|reference| {
            graph
                .prepared_publication(&space, &reference)
                .and_then(|value| {
                    value.ok_or(zixcel_graph::GraphError::Commit(
                        zixcel_revision::Failure::Corrupt,
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    println!(
        "{}",
        json!({"revision":graph.commit_revision(&space)?,
        "nodes":graph.read(&space)?.nodes_by_label(&args[4],257),
        "preparations":preparations,"phases":phases})
    );
    Ok(())
}
