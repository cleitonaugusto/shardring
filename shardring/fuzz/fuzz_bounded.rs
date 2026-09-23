#![no_main]
use libfuzzer_sys::fuzz_target;
use shardring::{BoundedRing, BoundedConfig, Node, RingConfig, DefaultHash};

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    
    let node_count = (data[0] as usize % 8) + 2;
    let mut nodes = Vec::with_capacity(node_count);
    
    for i in 0..node_count {
        let weight = ((data.get(i + 1).copied().unwrap_or(1) as u64) % 100) + 1;
        nodes.push(Node::new(format!("node{}", i), weight));
    }
    
    let config = RingConfig::default();
    let bounded_config = BoundedConfig {
        enabled: true,
        max_load_factor: 1.25,
        max_probes: 100,
    };
    
    let ring = match BoundedRing::new(nodes, config, bounded_config, DefaultHash) {
        Ok(r) => r,
        Err(_) => return,
    };
    
    if data.len() > 10 {
        let key = &data[10..];
        let _ = ring.get(key);
        let _ = ring.get_n(key, 3);
        
        // Simulate load
        for i in 0..data.len().min(20) {
            let k = &data[i..i+1];
            let _ = ring.get(k);
        }
    }
    
    // Test health changes
    ring.set_healthy("node0", false);
    let _ = ring.get(b"test");
    ring.set_healthy("node0", true);
});