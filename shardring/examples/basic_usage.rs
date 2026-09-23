use shardring::{CompactRing, DefaultHash, Node};

fn main() {
    // Create nodes with different weights
    let nodes = vec![
        Node::new("us-east-1", 100),
        Node::new("us-west-2", 200),
        Node::new("eu-west-1", 150),
        Node::new("ap-southeast-1", 50),
    ];

    // Build ring with default config (adaptive sizing enabled)
    let ring = CompactRing::<DefaultHash>::builder()
        .add_nodes(nodes)
        .build()
        .unwrap();

    println!("Ring stats: {:?}", ring.stats());

    // Test key distribution
    let test_keys = vec![
        "user:1001",
        "user:1002",
        "session:abc",
        "cache:key1",
        "data:item42",
    ];

    for key in test_keys {
        let node = ring.get(key.as_bytes()).unwrap();
        println!("{} -> {}", key, node.id);
    }

    // Get replicas for replication
    let key = b"important:data";
    let replicas = ring.get_n(key, 3).unwrap();
    println!("\nReplicas for {:?}:", key);
    for (i, node) in replicas.iter().enumerate() {
        println!("  {}: {}", i + 1, node.id);
    }
}
