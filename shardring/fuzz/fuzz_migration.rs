#![no_main]
use libfuzzer_sys::fuzz_target;
use shardring::{CompactRing, DualRing, MigrationConfig, MigrationStrategy, Node, RingConfig, DefaultHash};

fuzz_target!(|data: &[u8]| {
    if data.len() < 8 {
        return;
    }
    
    let node_count = (data[0] as usize % 8) + 2;
    let mut nodes = Vec::with_capacity(node_count);
    
    for i in 0..node_count {
        let weight = ((data.get(i + 1).copied().unwrap_or(1) as u64) % 100) + 1;
        nodes.push(Node::new(format!("node{}", i), weight));
    }
    
    let config1 = RingConfig {
        base_hashes_per_node: (data[2] as usize % 200) + 50,
        target_cv: 0.08,
        ..Default::default()
    };
    
    let config2 = RingConfig {
        base_hashes_per_node: (data[3] as usize % 200) + 50,
        target_cv: 0.08,
        ..Default::default()
    };
    
    let ring1 = match CompactRing::<DefaultHash>::new(nodes.clone(), config1, DefaultHash) {
        Ok(r) => r,
        Err(_) => return,
    };
    
    let ring2 = match CompactRing::<DefaultHash>::new(nodes, config2, DefaultHash) {
        Ok(r) => r,
        Err(_) => return,
    };
    
    let strategy = match data[4] % 3 {
        0 => MigrationStrategy::HashBased,
        1 => MigrationStrategy::Percentage,
        _ => MigrationStrategy::Datacenter,
    };
    
    let mut config = MigrationConfig {
        strategy,
        rollout_percentage: (data[5] as f64 / 255.0).clamp(0.0, 1.0),
        datacenter_allowlist: vec!["dc1".to_string(), "dc2".to_string()],
        enable_shadow_mode: data[6] % 2 == 0,
    };
    
    let dual = match DualRing::new(ring1, ring2, config) {
        Ok(d) => d,
        Err(_) => return,
    };
    
    if data.len() > 10 {
        let key = &data[10..];
        let _ = dual.get(key);
        let _ = dual.get_shadow(key);
    }
});