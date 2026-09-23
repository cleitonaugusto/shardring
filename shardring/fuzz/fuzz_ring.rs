#![no_main]
use libfuzzer_sys::fuzz_target;
use shardring::{CompactRing, Node, RingConfig, DefaultHash};

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    
    let node_count = (data[0] as usize % 10) + 1;
    let mut nodes = Vec::with_capacity(node_count);
    
    for i in 0..node_count {
        let weight = ((data.get(i + 1).copied().unwrap_or(1) as u64) % 100) + 1;
        nodes.push(Node::new(format!("node{}", i), weight));
    }
    
    let config = RingConfig::default();
    let ring = match CompactRing::<DefaultHash>::new(nodes, config, DefaultHash) {
        Ok(r) => r,
        Err(_) => return,
    };
    
    if data.len() > 10 {
        let key = &data[10..];
        let _ = ring.get(key);
        let _ = ring.get_n(key, 3);
    }
});