use shardring::{CompactRing, DefaultHash, DualRing, MigrationStrategy, Node, RingConfig};

fn main() {
    let nodes = vec![
        Node::new("node-1", 100),
        Node::new("node-2", 200),
        Node::new("node-3", 300),
    ];

    // Old configuration
    let old_config = RingConfig {
        base_hashes_per_node: 100,
        target_cv: 0.1,
        ..Default::default()
    };

    // New configuration (more hashes for better distribution)
    let new_config = RingConfig {
        base_hashes_per_node: 160,
        target_cv: 0.08,
        ..Default::default()
    };

    let old_ring = CompactRing::new(nodes.clone(), old_config, DefaultHash).unwrap();
    let new_ring = CompactRing::new(nodes, new_config, DefaultHash).unwrap();

    // Create dual ring for migration
    let mut dual = DualRing::builder(old_ring, new_ring)
        .strategy(MigrationStrategy::HashBased)
        .rollout_percentage(0.1) // Start with 10%
        .enable_shadow_mode(true)
        .build()
        .unwrap();

    println!("Initial rollout: {:.0}%", dual.rollout_percentage() * 100.0);

    // Simulate traffic
    let test_keys: Vec<Vec<u8>> = (0..1000)
        .map(|i| format!("key-{}", i).into_bytes())
        .collect();

    // Check shadow mismatches
    let mut mismatches = 0;
    for key in &test_keys {
        let (old_node, new_node) = dual.get_shadow(key).unwrap();
        if old_node.id != new_node.id {
            mismatches += 1;
        }
    }
    println!(
        "Shadow mismatches: {}/{} ({:.1}%)",
        mismatches,
        test_keys.len(),
        mismatches as f64 / test_keys.len() as f64 * 100.0
    );

    // Gradual rollout
    for pct in [0.25, 0.5, 0.75, 1.0] {
        dual.set_rollout(pct);
        println!("Rollout: {:.0}%", pct * 100.0);

        // Check distribution
        let mut counts = std::collections::HashMap::new();
        for key in &test_keys {
            let node = dual.get(key).unwrap();
            *counts.entry(node.id.clone()).or_insert(0) += 1;
        }
        for (id, count) in counts {
            println!("  {}: {}", id, count);
        }
    }

    // Datacenter-aware migration
    println!("\n--- Datacenter Migration ---");
    let nodes_dc = vec![
        Node::new("dc1-node-1", 100),
        Node::new("dc1-node-2", 100),
        Node::new("dc2-node-1", 100),
    ];

    let dc_old = CompactRing::new(nodes_dc.clone(), RingConfig::default(), DefaultHash).unwrap();
    let dc_new = CompactRing::new(
        nodes_dc,
        RingConfig {
            base_hashes_per_node: 200,
            ..Default::default()
        },
        DefaultHash,
    )
    .unwrap();

    let dual_dc = DualRing::builder(dc_old, dc_new)
        .strategy(MigrationStrategy::Datacenter)
        .datacenter_allowlist(vec!["dc1".to_string()])
        .build()
        .unwrap();

    let key = b"test-key";
    let node_dc1 = dual_dc.get_with_datacenter(key, "dc1").unwrap();
    let node_dc2 = dual_dc.get_with_datacenter(key, "dc2").unwrap();

    println!("DC1 routes to: {}", node_dc1.id); // Should use new ring
    println!("DC2 routes to: {}", node_dc2.id); // Should use old ring
}
