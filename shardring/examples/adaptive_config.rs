use shardring::{
    adaptive::{analyze_ring_efficiency, recommend_ring_config, AdaptiveConfig},
    CompactRing, DefaultHash, Node,
};

fn main() {
    // Example 1: Recommend config for a cluster
    println!("=== Config Recommendation ===");
    let weights = vec![100, 200, 150, 300, 250, 100, 500];
    let config = recommend_ring_config(weights.len(), &weights, None);
    println!("Nodes: {}", weights.len());
    println!(
        "Recommended base_hashes_per_node: {}",
        config.base_hashes_per_node
    );
    println!("Target CV: {:.2}", config.target_cv);

    // Example 2: Custom adaptive config
    println!("\n=== Custom Adaptive Config ===");
    let custom_config = AdaptiveConfig {
        target_cv: 0.05, // Stricter distribution
        max_cv: 0.1,
        min_hashes_per_node: 50,
        max_hashes_per_node: 50000,
        collision_threshold: 0.005,
        weight_skew_threshold: 50.0,
    };
    let config2 = recommend_ring_config(weights.len(), &weights, Some(&custom_config));
    println!(
        "Custom config base_hashes_per_node: {}",
        config2.base_hashes_per_node
    );

    // Example 3: Analyze existing ring efficiency
    println!("\n=== Ring Efficiency Analysis ===");
    let nodes: Vec<Node> = weights
        .iter()
        .enumerate()
        .map(|(i, w)| Node::new(format!("node-{}", i), *w))
        .collect();

    let ring = CompactRing::<DefaultHash>::new(nodes, config, DefaultHash).unwrap();
    let stats = ring.stats();
    println!("Ring stats: {:?}", stats);

    let report = analyze_ring_efficiency(&stats, &custom_config);
    println!("Efficient: {}", report.is_efficient);
    for issue in &report.issues {
        println!("  Issue: {}", issue);
    }
    for rec in &report.recommendations {
        println!("  Recommendation: {}", rec);
    }

    // Example 4: Skewed weights
    println!("\n=== Skewed Weights ===");
    let skewed_weights = vec![1, 1, 1, 1, 10000]; // One very heavy node
    let skewed_config = recommend_ring_config(skewed_weights.len(), &skewed_weights, None);
    println!(
        "Skewed weights base_hashes_per_node: {}",
        skewed_config.base_hashes_per_node
    );

    let nodes_skewed: Vec<Node> = skewed_weights
        .iter()
        .enumerate()
        .map(|(i, w)| Node::new(format!("node-{}", i), *w))
        .collect();
    let ring_skewed =
        CompactRing::<DefaultHash>::new(nodes_skewed, skewed_config, DefaultHash).unwrap();
    let stats_skewed = ring_skewed.stats();
    println!("Skewed ring stats: {:?}", stats_skewed);
}
