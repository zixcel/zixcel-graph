//! Bounded synthetic development measurement, never an existing database.
use std::{collections::BTreeMap, time::Instant};
use zixcel_graph::{Edge, Graph, GraphSpace, Node, PropertyValue};
fn metric(path: &str, key: &str) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name == key)
                .then(|| value.split_whitespace().next()?.parse().ok())
                .flatten()
        })
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let nodes: usize = args.first().map_or(Ok(1000), |v| v.parse())?;
    let spaces: usize = args.get(1).map_or(Ok(4), |v| v.parse())?;
    if args.len() > 2
        || !(2..=10000).contains(&nodes)
        || !(1..=8).contains(&spaces)
        || nodes.checked_mul(spaces).is_none_or(|n| n > 40000)
    {
        return Err(
            "fixture budget: 2..10000 nodes/space, 1..8 spaces, <=40000 total nodes".into(),
        );
    }
    let root = tempfile::tempdir()?;
    let path = root.path().join("synthetic.redb");
    let graph = Graph::create(&path)?;
    let import = Instant::now();
    for s in 0..spaces {
        let space = GraphSpace::new(format!("fixture-{s}"))?;
        let mut tx = graph.write(&space)?;
        for n in 0..nodes {
            let mut node = Node::new(format!("n{n:08}"), ["fixture.node"])?;
            node.properties =
                BTreeMap::from([("payload".into(), PropertyValue::String("x".repeat(128)))]);
            tx.upsert_node(node);
            if n > 0 {
                tx.upsert_edge(Edge::directed(
                    format!("e{n:08}"),
                    format!("n{:08}", n - 1),
                    format!("n{n:08}"),
                    "fixture.link",
                )?);
            }
        }
        tx.commit()?;
    }
    let imported = import.elapsed().as_micros();
    let target = GraphSpace::new("fixture-0")?;
    let mut reads = Vec::new();
    for _ in 0..20 {
        let begin = Instant::now();
        let snapshot = graph.read(&target)?;
        assert!(snapshot.get_node("n00000000").is_some());
        reads.push(begin.elapsed().as_micros());
    }
    reads.sort_unstable();
    let write_before = metric("/proc/self/io", "write_bytes");
    let update = Instant::now();
    let mut tx = graph.write(&target)?;
    let mut changed = Node::new("n00000000", ["fixture.node"])?;
    changed
        .properties
        .insert("payload".into(), PropertyValue::String("y".repeat(128)));
    tx.upsert_node(changed);
    tx.commit()?;
    let update_us = update.elapsed().as_micros();
    let write_after = metric("/proc/self/io", "write_bytes");
    drop(graph);
    let begin = Instant::now();
    let reopened = Graph::open_existing(&path)?;
    assert!(reopened.read(&target)?.get_node("n00000000").is_some());
    println!(
        "{}",
        serde_json::json!({"fixture":"chain-v1","seed":0,"profile":"debug",
        "nodes_per_space":nodes,"edges_per_space":nodes-1,"spaces":spaces,"payload_bytes":128,
        "import_us":imported,"read_samples":20,"read_p50_us":reads[9],"read_p95_us":reads[18],
        "single_update_us":update_us,"update_os_write_bytes":write_after.zip(write_before).map(|(a,b)|a.saturating_sub(b)),
        "reopen_and_read_us":begin.elapsed().as_micros(),"db_bytes":std::fs::metadata(path)?.len(),
        "process_peak_rss_kib":metric("/proc/self/status","VmHWM")})
    );
    Ok(())
}
