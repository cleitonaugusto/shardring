use shardring::{CompactRing, DefaultHash, Node, RingConfig, SERIALIZATION_VERSION};

fn main() {
    let nodes = vec![
        Node::new("node-1", 100),
        Node::new("node-2", 200),
        Node::new("node-3", 150),
    ];

    let ring = CompactRing::<DefaultHash>::builder()
        .add_nodes(nodes.clone())
        .config(RingConfig {
            base_hashes_per_node: 160,
            ..Default::default()
        })
        .build()
        .unwrap();

    println!("Original ring stats: {:?}", ring.stats());
    println!("Serialization version: {}", SERIALIZATION_VERSION);

    // Serialize to bytes
    let bytes = ring.to_bytes();
    println!("Serialized size: {} bytes", bytes.len());

    // Deserialize
    let restored =
        CompactRing::<DefaultHash>::from_bytes(&bytes, RingConfig::default(), DefaultHash).unwrap();
    println!("Restored ring stats: {:?}", restored.stats());

    // Verify they produce same results
    let test_keys = vec!["key1", "key2", "key3", "user:123", "session:abc"];
    for key in test_keys {
        let orig = ring.get(key.as_bytes()).unwrap();
        let rest = restored.get(key.as_bytes()).unwrap();
        assert_eq!(orig.id, rest.id, "Mismatch for key: {}", key);
        println!("{} -> {} (OK)", key, orig.id);
    }

    println!("\nSerialization roundtrip successful!");
}
