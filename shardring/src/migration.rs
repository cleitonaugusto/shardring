use super::{CompactRing, ConsistentHashError, HashFunction, Node, Result, RingConfig, RingStats};
use serde::{Deserialize, Serialize};

#[cfg(feature = "std")]
use parking_lot::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    pub strategy: MigrationStrategy,
    pub rollout_percentage: f64,
    pub datacenter_allowlist: Vec<String>,
    pub enable_shadow_mode: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MigrationStrategy {
    HashBased,
    Percentage,
    Datacenter,
}

impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            strategy: MigrationStrategy::HashBased,
            rollout_percentage: 0.0,
            datacenter_allowlist: Vec::new(),
            enable_shadow_mode: false,
        }
    }
}

#[derive(Debug)]
pub struct DualRing<H: HashFunction = super::DefaultHash> {
    old_ring: CompactRing<H>,
    new_ring: CompactRing<H>,
    config: MigrationConfig,
    #[cfg(feature = "std")]
    metrics: RwLock<MigrationMetrics>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MigrationMetrics {
    pub old_ring_requests: u64,
    pub new_ring_requests: u64,
    pub shadow_mismatches: u64,
    pub errors: u64,
}

impl<H: HashFunction> DualRing<H> {
    pub fn new(
        old_ring: CompactRing<H>,
        new_ring: CompactRing<H>,
        config: MigrationConfig,
    ) -> Result<Self> {
        if old_ring.nodes().len() != new_ring.nodes().len() {
            return Err(ConsistentHashError::MigrationInProgress(
                "Node count mismatch between rings".into(),
            ));
        }

        for (old, new) in old_ring.nodes().iter().zip(new_ring.nodes().iter()) {
            if old.id != new.id {
                return Err(ConsistentHashError::MigrationInProgress(
                    "Node ID mismatch between rings".into(),
                ));
            }
        }

        Ok(Self {
            old_ring,
            new_ring,
            config,
            #[cfg(feature = "std")]
            metrics: RwLock::new(MigrationMetrics::default()),
        })
    }

    #[inline]
    fn should_use_new_ring(&self, key: &[u8], datacenter: Option<&str>) -> bool {
        match self.config.strategy {
            MigrationStrategy::HashBased => {
                let mut hasher = xxhash_rust::xxh3::Xxh3::new();
                hasher.update(key);
                if let Some(dc) = datacenter {
                    hasher.update(dc.as_bytes());
                }
                let hash = hasher.digest();
                (hash as f64 / u64::MAX as f64) < self.config.rollout_percentage
            }
            MigrationStrategy::Percentage => {
                let mut hasher = xxhash_rust::xxh3::Xxh3::new();
                hasher.update(key);
                let hash = hasher.digest();
                (hash as f64 / u64::MAX as f64) < self.config.rollout_percentage
            }
            MigrationStrategy::Datacenter => datacenter
                .is_some_and(|dc| self.config.datacenter_allowlist.contains(&dc.to_string())),
        }
    }

    pub fn get(&self, key: &[u8]) -> Result<&Node> {
        let use_new = self.should_use_new_ring(key, None);

        #[cfg(feature = "std")]
        {
            let mut metrics = self.metrics.write();
            if use_new {
                metrics.new_ring_requests += 1;
            } else {
                metrics.old_ring_requests += 1;
            }
        }

        if use_new {
            self.new_ring.get(key)
        } else {
            self.old_ring.get(key)
        }
    }

    pub fn get_with_datacenter(&self, key: &[u8], datacenter: &str) -> Result<&Node> {
        let use_new = self.should_use_new_ring(key, Some(datacenter));

        #[cfg(feature = "std")]
        {
            let mut metrics = self.metrics.write();
            if use_new {
                metrics.new_ring_requests += 1;
            } else {
                metrics.old_ring_requests += 1;
            }
        }

        if use_new {
            self.new_ring.get(key)
        } else {
            self.old_ring.get(key)
        }
    }

    pub fn get_shadow(&self, key: &[u8]) -> Result<(&Node, &Node)> {
        let old_node = self.old_ring.get(key)?;
        let new_node = self.new_ring.get(key)?;

        #[cfg(feature = "std")]
        if self.config.enable_shadow_mode && old_node.id != new_node.id {
            self.metrics.write().shadow_mismatches += 1;
        }

        Ok((old_node, new_node))
    }

    pub fn update_config(&mut self, config: MigrationConfig) {
        self.config = config;
    }

    pub fn rollout_percentage(&self) -> f64 {
        self.config.rollout_percentage
    }

    pub fn set_rollout(&mut self, percentage: f64) {
        self.config.rollout_percentage = percentage.clamp(0.0, 1.0);
    }

    #[cfg(feature = "std")]
    pub fn metrics(&self) -> MigrationMetrics {
        self.metrics.read().clone()
    }

    #[cfg(feature = "std")]
    pub fn reset_metrics(&self) {
        *self.metrics.write() = MigrationMetrics::default();
    }

    pub fn old_ring(&self) -> &CompactRing<H> {
        &self.old_ring
    }

    pub fn new_ring(&self) -> &CompactRing<H> {
        &self.new_ring
    }

    pub fn into_inner(self) -> (CompactRing<H>, CompactRing<H>) {
        (self.old_ring, self.new_ring)
    }

    pub fn is_fully_migrated(&self) -> bool {
        self.config.rollout_percentage >= 1.0
            || matches!(self.config.strategy, MigrationStrategy::Datacenter)
                && !self.config.datacenter_allowlist.is_empty()
    }
}

impl<H: HashFunction> DualRing<H> {
    pub fn builder(old_ring: CompactRing<H>, new_ring: CompactRing<H>) -> DualRingBuilder<H> {
        DualRingBuilder::new(old_ring, new_ring)
    }
}

pub struct DualRingBuilder<H: HashFunction> {
    old_ring: CompactRing<H>,
    new_ring: CompactRing<H>,
    config: MigrationConfig,
}

impl<H: HashFunction> DualRingBuilder<H> {
    pub fn new(old_ring: CompactRing<H>, new_ring: CompactRing<H>) -> Self {
        Self {
            old_ring,
            new_ring,
            config: MigrationConfig::default(),
        }
    }

    pub fn strategy(mut self, strategy: MigrationStrategy) -> Self {
        self.config.strategy = strategy;
        self
    }

    pub fn rollout_percentage(mut self, pct: f64) -> Self {
        self.config.rollout_percentage = pct.clamp(0.0, 1.0);
        self
    }

    pub fn datacenter_allowlist(mut self, dcs: Vec<String>) -> Self {
        self.config.datacenter_allowlist = dcs;
        self
    }

    pub fn enable_shadow_mode(mut self, enable: bool) -> Self {
        self.config.enable_shadow_mode = enable;
        self
    }

    pub fn build(self) -> Result<DualRing<H>> {
        DualRing::new(self.old_ring, self.new_ring, self.config)
    }
}

pub fn generate_migration_plan(
    old_config: &RingConfig,
    new_config: &RingConfig,
    nodes: &[Node],
) -> MigrationPlan {
    let old_ring = CompactRing::<super::DefaultHash>::new(
        nodes.to_vec(),
        old_config.clone(),
        super::DefaultHash,
    )
    .unwrap();

    let new_ring = CompactRing::<super::DefaultHash>::new(
        nodes.to_vec(),
        new_config.clone(),
        super::DefaultHash,
    )
    .unwrap();

    let mut changes = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let old_points: Vec<_> = old_ring
            .points()
            .iter()
            .filter(|p| p.node_index() == i as u16)
            .collect();
        let new_points: Vec<_> = new_ring
            .points()
            .iter()
            .filter(|p| p.node_index() == i as u16)
            .collect();

        changes.push(NodeMigrationChange {
            node_id: node.id.clone(),
            old_hash_count: old_points.len(),
            new_hash_count: new_points.len(),
            hash_delta: new_points.len() as i64 - old_points.len() as i64,
        });
    }

    MigrationPlan {
        old_stats: old_ring.stats(),
        new_stats: new_ring.stats(),
        node_changes: changes,
        estimated_keys_moved_pct: estimate_keys_moved(&old_ring, &new_ring),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub old_stats: RingStats,
    pub new_stats: RingStats,
    pub node_changes: Vec<NodeMigrationChange>,
    pub estimated_keys_moved_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMigrationChange {
    pub node_id: String,
    pub old_hash_count: usize,
    pub new_hash_count: usize,
    pub hash_delta: i64,
}

fn estimate_keys_moved<H: HashFunction>(old: &CompactRing<H>, new: &CompactRing<H>) -> f64 {
    const SAMPLE_SIZE: usize = 10_000;
    let mut moved = 0;

    for i in 0..SAMPLE_SIZE {
        let key = format!("sample-key-{}", i);
        let old_node = old.get(key.as_bytes()).ok();
        let new_node = new.get(key.as_bytes()).ok();

        if old_node.map(|n| &n.id) != new_node.map(|n| &n.id) {
            moved += 1;
        }
    }

    moved as f64 / SAMPLE_SIZE as f64 * 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompactRing, DefaultHash, Node, RingConfig};

    fn create_test_rings() -> (CompactRing<DefaultHash>, CompactRing<DefaultHash>) {
        let nodes = vec![
            Node::new("n1", 100),
            Node::new("n2", 200),
            Node::new("n3", 300),
        ];

        let old_config = RingConfig {
            base_hashes_per_node: 100,
            target_cv: 0.1,
            ..Default::default()
        };

        let new_config = RingConfig {
            base_hashes_per_node: 160,
            target_cv: 0.08,
            ..Default::default()
        };

        let old = CompactRing::new(nodes.clone(), old_config, DefaultHash).unwrap();
        let new = CompactRing::new(nodes, new_config, DefaultHash).unwrap();

        (old, new)
    }

    #[test]
    fn test_dual_ring_creation() {
        let (old, new) = create_test_rings();
        let dual = DualRing::new(old, new, MigrationConfig::default()).unwrap();

        let node = dual.get(b"test").unwrap();
        assert!(["n1", "n2", "n3"].contains(&node.id.as_str()));
    }

    #[test]
    fn test_hash_based_rollout() {
        let (old, new) = create_test_rings();
        let config = MigrationConfig {
            strategy: MigrationStrategy::HashBased,
            rollout_percentage: 0.5,
            ..Default::default()
        };

        let dual = DualRing::new(old, new, config).unwrap();

        let keys = 10_000;
        for i in 0..keys {
            let key = format!("key-{}", i);
            dual.get(key.as_bytes()).unwrap();
        }

        let metrics = dual.metrics();
        assert_eq!(metrics.old_ring_requests + metrics.new_ring_requests, keys);
        let new_share = metrics.new_ring_requests as f64 / keys as f64;
        assert!(
            (0.45..0.55).contains(&new_share),
            "rollout of 0.5 routed {:.1}% to the new ring",
            new_share * 100.0
        );
    }

    #[test]
    fn test_rollout_is_stable_per_key() {
        let (old, new) = create_test_rings();
        let dual = DualRing::builder(old, new)
            .strategy(MigrationStrategy::HashBased)
            .rollout_percentage(0.3)
            .build()
            .unwrap();

        // The same key must not flip between rings while the rollout is unchanged.
        for i in 0..1_000 {
            let key = format!("key-{}", i);
            let first = dual.get(key.as_bytes()).unwrap().id.clone();
            assert_eq!(dual.get(key.as_bytes()).unwrap().id, first);
        }
    }

    #[test]
    fn test_datacenter_rollout() {
        let (old, new) = create_test_rings();
        let config = MigrationConfig {
            strategy: MigrationStrategy::Datacenter,
            datacenter_allowlist: vec!["dc1".to_string(), "dc2".to_string()],
            ..Default::default()
        };

        let dual = DualRing::new(old, new, config).unwrap();

        for i in 0..500 {
            let key = format!("key-{}", i);
            dual.get_with_datacenter(key.as_bytes(), "dc1").unwrap();
            dual.get_with_datacenter(key.as_bytes(), "dc3").unwrap();
        }

        // Allowlisted datacenters go to the new ring, everyone else stays on the old one.
        let metrics = dual.metrics();
        assert_eq!(metrics.new_ring_requests, 500);
        assert_eq!(metrics.old_ring_requests, 500);
    }

    #[test]
    fn test_shadow_mode() {
        let (old, new) = create_test_rings();
        let config = MigrationConfig {
            enable_shadow_mode: true,
            ..Default::default()
        };

        let dual = DualRing::new(old, new, config).unwrap();

        // Shadow mode answers from both rings so callers can compare them.
        let (old_node, new_node) = dual.get_shadow(b"test").unwrap();
        assert_eq!(old_node.id, dual.old_ring().get(b"test").unwrap().id);
        assert_eq!(new_node.id, dual.new_ring().get(b"test").unwrap().id);
    }

    #[test]
    fn test_migration_plan() {
        let nodes = vec![Node::new("n1", 100), Node::new("n2", 200)];

        let old_config = RingConfig {
            base_hashes_per_node: 100,
            ..Default::default()
        };
        let new_config = RingConfig {
            base_hashes_per_node: 160,
            ..Default::default()
        };

        let plan = generate_migration_plan(&old_config, &new_config, &nodes);

        assert_eq!(plan.node_changes.len(), 2);
        assert!(plan.estimated_keys_moved_pct >= 0.0 && plan.estimated_keys_moved_pct <= 100.0);
    }
}
