use super::{CompactRing, ConsistentHashError, Node, Result};
use core::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundedConfig {
    pub enabled: bool,
    pub max_load_factor: f64,
    pub max_probes: usize,
}

impl Default for BoundedConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_load_factor: 1.25,
            max_probes: 100,
        }
    }
}

impl BoundedConfig {
    pub fn new(max_load_factor: f64) -> Self {
        Self {
            enabled: true,
            max_load_factor,
            max_probes: 100,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            max_load_factor: 1.0,
            max_probes: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadTracker {
    loads: Arc<HashMap<String, AtomicU64>>,
    total_load: Arc<AtomicU64>,
}

impl LoadTracker {
    pub fn new(node_ids: &[String]) -> Self {
        let mut loads = HashMap::new();
        for id in node_ids {
            loads.insert(id.clone(), AtomicU64::new(0));
        }
        Self {
            loads: Arc::new(loads),
            total_load: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn acquire(&self, node_id: &str) {
        if let Some(load) = self.loads.get(node_id) {
            load.fetch_add(1, Ordering::Relaxed);
            self.total_load.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn release(&self, node_id: &str) {
        if let Some(load) = self.loads.get(node_id) {
            load.fetch_sub(1, Ordering::Relaxed);
            self.total_load.fetch_sub(1, Ordering::Relaxed);
        }
    }

    pub fn get_load(&self, node_id: &str) -> u64 {
        self.loads
            .get(node_id)
            .map(|l| l.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    pub fn get_all_loads(&self) -> HashMap<String, u64> {
        self.loads
            .iter()
            .map(|(k, v)| (k.clone(), v.load(Ordering::Relaxed)))
            .collect()
    }

    pub fn avg_load(&self, healthy_count: usize) -> f64 {
        if healthy_count == 0 {
            return 0.0;
        }
        self.total_load.load(Ordering::Relaxed) as f64 / healthy_count as f64
    }

    pub fn total_load(&self) -> u64 {
        self.total_load.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub struct BoundedRing<H: super::HashFunction> {
    ring: CompactRing<H>,
    config: BoundedConfig,
    load_tracker: LoadTracker,
    healthy: Arc<RwLock<HashMap<String, bool>>>,
}

impl<H: super::HashFunction> BoundedRing<H> {
    pub fn new(ring: CompactRing<H>, config: BoundedConfig) -> Result<Self> {
        let node_ids: Vec<String> = ring.nodes().iter().map(|n| n.id.clone()).collect();
        let load_tracker = LoadTracker::new(&node_ids);
        let healthy = Arc::new(RwLock::new(
            node_ids.iter().map(|id| (id.clone(), true)).collect(),
        ));

        Ok(Self {
            ring,
            config,
            load_tracker,
            healthy,
        })
    }

    pub fn builder(ring: CompactRing<H>) -> BoundedRingBuilder<H> {
        BoundedRingBuilder::new(ring)
    }

    pub fn get(&self, key: &[u8]) -> Result<&Node> {
        if !self.config.enabled {
            return self.ring.get(key);
        }

        let hash = self.ring.hash_function().hash(key);
        let points = self.ring.points();
        let nodes = self.ring.nodes();

        if points.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        let healthy_guard = self.healthy.read().unwrap();
        let healthy_count = healthy_guard.values().filter(|&&h| h).count();
        let avg_load = self.load_tracker.avg_load(healthy_count);
        let max_load = (avg_load * self.config.max_load_factor) as u64;

        let start_idx = points
            .binary_search_by_key(&hash, |p| p.hash)
            .unwrap_or_else(|i| i % points.len());

        for probe in 0..self.config.max_probes {
            let idx = (start_idx + probe) % points.len();
            let point = &points[idx];
            let node_idx = point.node_index() as usize;
            let node = &nodes[node_idx];

            if !*healthy_guard.get(&node.id).unwrap_or(&false) {
                continue;
            }

            let current_load = self.load_tracker.get_load(&node.id);
            if current_load <= max_load {
                self.load_tracker.acquire(&node.id);
                return Ok(node);
            }
        }

        let fallback = self.find_least_loaded(&healthy_guard)?;
        self.load_tracker.acquire(&fallback.id);
        Ok(fallback)
    }

    pub fn get_n(&self, key: &[u8], n: usize) -> Result<Vec<&Node>> {
        if !self.config.enabled {
            return self.ring.get_n(key, n);
        }

        let mut result = Vec::with_capacity(n);
        let mut seen = std::collections::HashSet::new();
        let mut probe = 0;

        while result.len() < n && probe < self.config.max_probes {
            let single = self.get(key)?;
            if seen.insert(single.id.clone()) {
                result.push(single);
            }
            probe += 1;
        }

        if result.len() < n {
            let all_nodes: Vec<&Node> = self
                .ring
                .nodes()
                .iter()
                .filter(|n| *self.healthy.read().unwrap().get(&n.id).unwrap_or(&false))
                .filter(|n| !seen.contains(&n.id))
                .collect();

            for node in all_nodes {
                if result.len() >= n {
                    break;
                }
                result.push(node);
            }
        }

        Ok(result)
    }

    fn find_least_loaded(&self, healthy_guard: &HashMap<String, bool>) -> Result<&Node> {
        let nodes = self.ring.nodes();
        let mut best_node = None;
        let mut best_load = u64::MAX;

        for node in nodes {
            if !*healthy_guard.get(&node.id).unwrap_or(&false) {
                continue;
            }
            let load = self.load_tracker.get_load(&node.id);
            if load < best_load {
                best_load = load;
                best_node = Some(node);
            }
        }

        best_node.ok_or(ConsistentHashError::EmptyRing)
    }

    pub fn set_healthy(&self, node_id: &str, healthy: bool) {
        self.healthy
            .write()
            .unwrap()
            .insert(node_id.to_string(), healthy);
    }

    pub fn is_healthy(&self, node_id: &str) -> bool {
        *self.healthy.read().unwrap().get(node_id).unwrap_or(&false)
    }

    pub fn acquire(&self, node_id: &str) {
        self.load_tracker.acquire(node_id);
    }

    pub fn release(&self, node_id: &str) {
        self.load_tracker.release(node_id);
    }

    pub fn loads(&self) -> HashMap<String, u64> {
        self.load_tracker.get_all_loads()
    }

    pub fn stats(&self) -> BoundedStats {
        let healthy_guard = self.healthy.read().unwrap();
        let healthy_count = healthy_guard.values().filter(|&&h| h).count();
        let avg_load = self.load_tracker.avg_load(healthy_count);
        let max_load = (avg_load * self.config.max_load_factor) as u64;

        let node_loads: Vec<_> = self
            .ring
            .nodes()
            .iter()
            .map(|n| {
                let load = self.load_tracker.get_load(&n.id);
                let healthy = *healthy_guard.get(&n.id).unwrap_or(&false);
                NodeLoadStat {
                    node_id: n.id.clone(),
                    load,
                    healthy,
                    overloaded: load > max_load && healthy,
                }
            })
            .collect();

        BoundedStats {
            config: self.config,
            healthy_count,
            avg_load,
            max_allowed_load: max_load,
            total_load: self.load_tracker.total_load(),
            node_loads,
        }
    }

    pub fn inner_ring(&self) -> &CompactRing<H> {
        &self.ring
    }

    pub fn into_inner(self) -> CompactRing<H> {
        self.ring
    }
}

pub struct BoundedRingBuilder<H: super::HashFunction> {
    ring: CompactRing<H>,
    config: BoundedConfig,
}

impl<H: super::HashFunction> BoundedRingBuilder<H> {
    pub fn new(ring: CompactRing<H>) -> Self {
        Self {
            ring,
            config: BoundedConfig::default(),
        }
    }

    pub fn max_load_factor(mut self, factor: f64) -> Self {
        self.config.max_load_factor = factor;
        self
    }

    pub fn max_probes(mut self, probes: usize) -> Self {
        self.config.max_probes = probes;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.config = BoundedConfig::disabled();
        self
    }

    pub fn build(self) -> Result<BoundedRing<H>> {
        BoundedRing::new(self.ring, self.config)
    }
}

#[derive(Debug, Clone)]
pub struct BoundedStats {
    pub config: BoundedConfig,
    pub healthy_count: usize,
    pub avg_load: f64,
    pub max_allowed_load: u64,
    pub total_load: u64,
    pub node_loads: Vec<NodeLoadStat>,
}

#[derive(Debug, Clone)]
pub struct NodeLoadStat {
    pub node_id: String,
    pub load: u64,
    pub healthy: bool,
    pub overloaded: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompactRing, DefaultHash, Node};

    fn create_test_ring() -> CompactRing<DefaultHash> {
        let nodes = vec![
            Node::new("node1", 100),
            Node::new("node2", 100),
            Node::new("node3", 100),
        ];
        CompactRing::builder().add_nodes(nodes).build().unwrap()
    }

    #[test]
    fn test_bounded_load_basic() {
        let ring = create_test_ring();
        let bounded = BoundedRing::builder(ring)
            .max_load_factor(1.5)
            .build()
            .unwrap();

        let node = bounded.get(b"test-key").unwrap();
        assert!(["node1", "node2", "node3"].contains(&node.id.as_str()));
    }

    #[test]
    fn test_bounded_load_overload_protection() {
        let ring = create_test_ring();
        let bounded = BoundedRing::builder(ring)
            .max_load_factor(1.0)
            .max_probes(100)
            .build()
            .unwrap();

        bounded.acquire("node1");
        bounded.acquire("node1");
        bounded.acquire("node1");

        let mut skipped = 0;
        for i in 0..100 {
            let key = format!("hot-key-{}", i);
            let node = bounded.get(key.as_bytes()).unwrap();
            if node.id != "node1" {
                skipped += 1;
            }
        }
        assert!(
            skipped > 50,
            "Overloaded node should be skipped for most keys"
        );
    }

    #[test]
    fn test_bounded_load_unhealthy() {
        let ring = create_test_ring();
        let bounded = BoundedRing::builder(ring).build().unwrap();

        bounded.set_healthy("node1", false);
        bounded.set_healthy("node2", false);

        let node = bounded.get(b"test").unwrap();
        assert_eq!(node.id, "node3");
    }

    #[test]
    fn test_bounded_load_disabled() {
        let ring = create_test_ring();
        let bounded = BoundedRing::builder(ring).disabled().build().unwrap();

        bounded.acquire("node1");
        bounded.acquire("node1");

        let _node = bounded.get(b"test").unwrap();
    }

    #[test]
    fn test_bounded_stats() {
        let ring = create_test_ring();
        let bounded = BoundedRing::builder(ring)
            .max_load_factor(1.5)
            .build()
            .unwrap();

        bounded.acquire("node1");
        bounded.acquire("node1");

        let stats = bounded.stats();
        assert_eq!(stats.healthy_count, 3);
        assert_eq!(stats.total_load, 2);
        assert!(stats
            .node_loads
            .iter()
            .any(|n| n.node_id == "node1" && n.load == 2));
    }

    #[test]
    fn test_get_n_distinct() {
        let ring = create_test_ring();
        let bounded = BoundedRing::builder(ring).build().unwrap();

        let nodes = bounded.get_n(b"test", 2).unwrap();
        assert_eq!(nodes.len(), 2);
        assert_ne!(nodes[0].id, nodes[1].id);
    }
}
