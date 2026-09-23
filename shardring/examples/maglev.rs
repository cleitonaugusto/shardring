use shardring::{DefaultHash, MaglevTable, Node};

fn main() {
    let nodes = vec![
        Node::new("us-east-1", 100),
        Node::new("us-west-2", 200),
        Node::new("eu-west-1", 150),
        Node::new("ap-southeast-1", 50),
    ];

    // Create Maglev table with default config (65537 slots, prime)
    let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes.clone()).build().unwrap();

    println!("=== Maglev Table ===");
    println!("Table size: {}", table.table_size());
    println!("VNode count: {}", table.vnode_count());
    println!("Stats: {:?}", table.stats());

    // Load distribution
    println!("\nLoad Distribution:");
    for (id, count, fraction) in table.load_distribution() {
        println!("  {}: {} slots ({:.2}%)", id, count, fraction * 100.0);
    }

    // Test key mapping
    let test_keys = vec![
        "user:1001",
        "user:1002",
        "session:abc",
        "cache:key1",
        "data:item42",
    ];
    println!("\nKey Mapping:");
    for key in test_keys {
        let node = table.get(key.as_bytes()).unwrap();
        println!("  {} -> {}", key, node.id);
    }

    // Get replicas
    let key = b"important:data";
    let replicas = table.get_n(key, 3).unwrap();
    println!("\nReplicas for {:?}:", key);
    for (i, node) in replicas.iter().enumerate() {
        println!("  {}: {}", i + 1, node.id);
    }

    // Custom table size
    println!("\n=== Custom Table Size ===");
    let table2: MaglevTable<DefaultHash> = MaglevTable::builder(nodes)
        .table_size(10000)
        .build()
        .unwrap();
    println!(
        "Requested 10000, got prime table size: {}",
        table2.table_size()
    );
    println!("Stats: {:?}", table2.stats());
}
