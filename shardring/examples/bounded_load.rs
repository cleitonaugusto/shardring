use shardring::{BoundedRing, CompactRing, DefaultHash, Node};

fn main() {
    let nodes = vec![
        Node::new("node-1", 100),
        Node::new("node-2", 100),
        Node::new("node-3", 100),
    ];

    let ring = CompactRing::<DefaultHash>::builder()
        .add_nodes(nodes)
        .build()
        .unwrap();

    // Create bounded ring with max load factor of 1.25 (25% above average)
    let bounded = BoundedRing::builder(ring)
        .max_load_factor(1.25)
        .max_probes(100)
        .build()
        .unwrap();

    println!("=== Bounded Load Example ===");

    // Simulate some nodes being overloaded
    bounded.acquire("node-1");
    bounded.acquire("node-1");
    bounded.acquire("node-1");
    bounded.acquire("node-1");
    bounded.acquire("node-1");

    println!("Initial loads: {:?}", bounded.loads());

    // Test routing - node-1 should be avoided for new requests
    let mut counts = std::collections::HashMap::new();
    for i in 0..1000 {
        let key = format!("hot-key-{}", i);
        let node = bounded.get(key.as_bytes()).unwrap();
        *counts.entry(node.id.clone()).or_insert(0) += 1;
    }

    println!("\nRouting with overloaded node-1:");
    for (id, count) in counts {
        println!("  {}: {}", id, count);
    }

    println!("\nLoads after routing: {:?}", bounded.loads());

    // Test health-based failover
    println!("\n=== Health Failover ===");
    bounded.set_healthy("node-2", false);
    bounded.set_healthy("node-3", false);

    let node = bounded.get(b"test-key").unwrap();
    println!(
        "With node-2 and node-3 unhealthy, key routes to: {}",
        node.id
    );

    // Restore health
    bounded.set_healthy("node-2", true);
    bounded.set_healthy("node-3", true);

    // Test with bounded disabled
    println!("\n=== Disabled Bounded Mode ===");
    let ring2 = CompactRing::<DefaultHash>::builder()
        .add_nodes(vec![Node::new("a", 100), Node::new("b", 100)])
        .build()
        .unwrap();

    let unbounded = BoundedRing::builder(ring2).disabled().build().unwrap();
    unbounded.acquire("a");
    unbounded.acquire("a");
    unbounded.acquire("a");

    let node = unbounded.get(b"test").unwrap();
    println!("With bounded disabled, key routes to: {}", node.id);

    // Print stats
    println!("\n=== Final Stats ===");
    let stats = bounded.stats();
    println!("Healthy nodes: {}", stats.healthy_count);
    println!("Avg load: {:.2}", stats.avg_load);
    println!("Max allowed load: {}", stats.max_allowed_load);
    println!("Total load: {}", stats.total_load);
    for node_stat in stats.node_loads {
        println!(
            "  {}: load={}, healthy={}, overloaded={}",
            node_stat.node_id, node_stat.load, node_stat.healthy, node_stat.overloaded
        );
    }
}
