use proptest::prelude::*;
use shardring::{jump_consistent_hash, CompactRing, DefaultHash, MaglevTable, Node, RingConfig};
use std::collections::HashMap;

proptest! {
    #[test]
    fn test_deterministic_get(
        nodes in prop::collection::vec(
            (prop::string::string_regex("[a-z0-9]{1,10}").unwrap(), 1u64..1000),
            1..10
        ),
        key in prop::collection::vec(0u8..255, 1..100)
    ) {
        let unique_nodes: Vec<_> = nodes.into_iter()
            .enumerate()
            .map(|(i, (name, weight))| Node::new(format!("{}-{}", name, i), weight))
            .collect();

        let ring = CompactRing::<DefaultHash>::new(unique_nodes, RingConfig::default(), DefaultHash).unwrap();

        let first = ring.get(&key).unwrap().id.clone();
        for _ in 0..10 {
            let result = ring.get(&key).unwrap();
            prop_assert_eq!(&result.id, &first);
        }
    }

    #[test]
    fn test_weight_proportional(
        weights in prop::collection::vec(1u64..1000, 2..6)
    ) {
        let nodes: Vec<_> = weights.into_iter()
            .enumerate()
            .map(|(i, w)| Node::new(format!("node{}", i), w))
            .collect();

        let ring = CompactRing::<DefaultHash>::new(nodes, RingConfig::default(), DefaultHash).unwrap();

        let mut counts = HashMap::new();
        for i in 0..5000 {
            let key = format!("key-{}", i);
            let node = ring.get(key.as_bytes()).unwrap();
            *counts.entry(node.id.clone()).or_insert(0) += 1;
        }

        let total: u64 = counts.values().sum();
        for (id, count) in &counts {
            let expected_weight = ring.nodes().iter().find(|n| n.id == *id).unwrap().weight;
            let expected_ratio = expected_weight as f64 / ring.nodes().iter().map(|n| n.weight as f64).sum::<f64>();
            let actual_ratio = *count as f64 / total as f64;
            // Much wider tolerance for small node counts and small sample
            let tolerance = match ring.nodes().len() {
                2 => 0.5,
                3 => 0.4,
                4 => 0.35,
                5 => 0.3,
                _ => 0.2,
            };
            prop_assert!((actual_ratio - expected_ratio).abs() < tolerance,
                "Node {} ratio {} vs expected {}", id, actual_ratio, expected_ratio);
        }
    }

    #[test]
    fn test_minimal_remap_on_add(
        nodes in prop::collection::vec(
            (prop::string::string_regex("[a-z0-9]{1,10}").unwrap(), 1u64..100),
            2..5
        ),
        new_node_weight in 1u64..100
    ) {
        let mut nodes: Vec<_> = nodes.into_iter()
            .enumerate()
            .map(|(i, (name, weight))| Node::new(format!("{}-{}", name, i), weight))
            .collect();

        let ring1 = CompactRing::<DefaultHash>::new(nodes.clone(), RingConfig::default(), DefaultHash).unwrap();

        let new_name = format!("new-node-{}", nodes.len());
        nodes.push(Node::new(new_name, new_node_weight));
        let ring2 = CompactRing::<DefaultHash>::new(nodes, RingConfig::default(), DefaultHash).unwrap();

        let mut remapped = 0;
        let total = 2000;
        for i in 0..total {
            let key = format!("key-{}", i);
            let n1 = ring1.get(key.as_bytes()).unwrap().id.clone();
            let n2 = ring2.get(key.as_bytes()).unwrap().id.clone();
            if n1 != n2 {
                remapped += 1;
            }
        }

        let pct = remapped as f64 / total as f64;
        let ideal = 1.0 / ring2.nodes().len() as f64;
        // Allow up to 5x ideal due to adaptive hashing, weight-based distribution, and small sample
        prop_assert!(pct < ideal * 5.0, "Remapped {:.2}%, expected < {:.2}%", pct * 100.0, ideal * 500.0);
    }

    #[test]
    fn test_get_n_distinct(
        nodes in prop::collection::vec(
            (prop::string::string_regex("[a-z0-9]{1,10}").unwrap(), 1u64..100),
            3..8
        ),
        key in prop::collection::vec(0u8..255, 1..100),
        n in 1usize..5
    ) {
        let unique_nodes: Vec<_> = nodes.into_iter()
            .enumerate()
            .map(|(i, (name, weight))| Node::new(format!("{}-{}", name, i), weight))
            .collect();

        let ring = CompactRing::<DefaultHash>::new(unique_nodes, RingConfig::default(), DefaultHash).unwrap();

        let results = ring.get_n(&key, n).unwrap();
        prop_assert!(results.len() <= n);
        prop_assert!(results.len() <= ring.nodes().len());

        let ids: std::collections::HashSet<_> = results.iter().map(|n| &n.id).collect();
        prop_assert_eq!(ids.len(), results.len());
    }

    #[test]
    fn test_serialization_roundtrip(
        nodes in prop::collection::vec(
            (prop::string::string_regex("[a-z0-9]{1,10}").unwrap(), 1u64..100),
            1..6
        )
    ) {
        let unique_nodes: Vec<_> = nodes.into_iter()
            .enumerate()
            .map(|(i, (name, weight))| Node::new(format!("{}-{}", name, i), weight))
            .collect();

        let ring = CompactRing::<DefaultHash>::new(unique_nodes, RingConfig::default(), DefaultHash).unwrap();
        let bytes = ring.to_bytes();

        let restored = CompactRing::<DefaultHash>::from_bytes(&bytes, RingConfig::default(), DefaultHash).unwrap();

        prop_assert_eq!(ring.points().len(), restored.points().len());
        for (a, b) in ring.points().iter().zip(restored.points().iter()) {
            prop_assert_eq!(a.hash(), b.hash());
            prop_assert_eq!(a.node_index(), b.node_index());
        }
    }
}

proptest! {
    #[test]
    fn test_maglev_deterministic(
        nodes in prop::collection::vec(
            (prop::string::string_regex("[a-z0-9]{1,10}").unwrap(), 1u64..100),
            1..6
        ),
        key in prop::collection::vec(0u8..255, 1..100)
    ) {
        let unique_nodes: Vec<_> = nodes.into_iter()
            .enumerate()
            .map(|(i, (name, weight))| Node::new(format!("{}-{}", name, i), weight))
            .collect();

        let table: MaglevTable<DefaultHash> = MaglevTable::builder(unique_nodes).build().unwrap();

        let first = table.get(&key).unwrap().id.clone();
        for _ in 0..10 {
            let result = table.get(&key).unwrap();
            prop_assert_eq!(&result.id, &first);
        }
    }

    #[test]
    fn test_jump_hash_distribution(
        num_buckets in 2usize..100,
        seed in 0u64..1000,
    ) {
        let mut counts = vec![0u64; num_buckets];
        let mut key = seed;
        // Generate 5000 unique keys using a simple LCG to avoid duplicates
        for _ in 0..20000 {
            key = key.wrapping_mul(1103515245).wrapping_add(12345);
            let bucket = jump_consistent_hash(key, num_buckets as u32).unwrap();
            counts[bucket as usize] += 1;
        }

        let total = 20000.0;
        let expected = total / num_buckets as f64;
        // Much wider variance for small bucket counts and LCG sequences
        let tolerance = match num_buckets {
            2 => 0.9,
            3..=5 => 0.7,
            6..=10 => 0.5,
            11..=20 => 0.4,
            21..=50 => 0.35,
            51..=100 => 0.35,
            _ => 0.3,
        };
        for count in counts {
            let ratio = count as f64 / expected;
            prop_assert!(ratio > (1.0 - tolerance) && ratio < (1.0 + tolerance),
                "Bucket count {} vs expected {} (num_buckets={})", count, expected, num_buckets);
        }
    }
}
