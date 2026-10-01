use zixcel_graph::{Direction, Edge, Graph, GraphSpace, Node, Traversal};

#[test]
fn network_topology_uses_only_generic_graph_primitives() {
    let graph = Graph::memory().unwrap();
    let space = GraphSpace::new("network-fixture").unwrap();
    let mut tx = graph.write(&space).unwrap();
    tx.upsert_node(Node::new("router-a", ["fixture.router"]).unwrap());
    tx.upsert_node(Node::new("interface-a1", ["fixture.interface"]).unwrap());
    tx.upsert_node(Node::new("interface-b1", ["fixture.interface"]).unwrap());
    tx.upsert_node(Node::new("router-b", ["fixture.router"]).unwrap());
    tx.upsert_edge(
        Edge::directed(
            "router-a-interface",
            "router-a",
            "interface-a1",
            "fixture.has-interface",
        )
        .unwrap(),
    );
    tx.upsert_edge(
        Edge::undirected(
            "physical-link",
            "interface-a1",
            "interface-b1",
            "fixture.linked-to",
        )
        .unwrap(),
    );
    tx.upsert_edge(
        Edge::directed(
            "router-b-interface",
            "router-b",
            "interface-b1",
            "fixture.has-interface",
        )
        .unwrap(),
    );
    tx.commit().unwrap();

    let read = graph.read(&space).unwrap();
    assert_eq!(
        read.neighbors(
            "router-a",
            Direction::Outgoing,
            Some("fixture.has-interface")
        ),
        vec!["interface-a1"]
    );
    assert_eq!(
        read.neighbors(
            "interface-a1",
            Direction::Outgoing,
            Some("fixture.linked-to")
        ),
        vec!["interface-b1"]
    );
    let result = read
        .traverse(&Traversal::new("router-a", Direction::Both, 3, 16))
        .unwrap();
    assert_eq!(
        result.paths.last().unwrap().nodes,
        vec!["router-a", "interface-a1", "interface-b1", "router-b"]
    );
}
