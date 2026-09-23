#[cfg(feature = "health")]
use super::{CompactRing, ConsistentHashError, Node, Result, RingConfig};
#[cfg(feature = "health")]
use std::collections::HashMap;
#[cfg(feature = "health")]
use std::sync::Arc;
#[cfg(feature = "health")]
use std::time::Duration;
#[cfg(feature = "health")]
use tokio::sync::{watch, RwLock};
#[cfg(feature = "health")]
use tokio::time::{interval, timeout};

#[derive(Debug, Clone)]
pub struct HealthConfig {
    pub check_interval: Duration,
    pub timeout: Duration,
    pub healthy_threshold: u32,
    pub unhealthy_threshold: u32,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            check_interval: Duration::from_secs(10),
            timeout: Duration::from_secs(2),
            healthy_threshold: 2,
            unhealthy_threshold: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct NodeHealth {
    pub status: HealthStatus,
    pub consecutive_successes: u32,
    pub consecutive_failures: u32,
    pub last_check: Option<std::time::Instant>,
    pub last_error: Option<String>,
}

impl Default for NodeHealth {
    fn default() -> Self {
        Self {
            status: HealthStatus::Unknown,
            consecutive_successes: 0,
            consecutive_failures: 0,
            last_check: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HealthAwareStats {
    pub ring_stats: super::RingStats,
    pub healthy_count: usize,
    pub unhealthy_count: usize,
    pub unknown_count: usize,
    pub total_nodes: usize,
}

#[derive(Clone)]
pub struct HealthAwareRing<H, C>
where
    H: super::HashFunction + Clone + Send + Sync + 'static,
    C: Fn(
            &Node,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + Clone
        + 'static,
{
    ring: Arc<RwLock<CompactRing<H>>>,
    health: Arc<RwLock<HashMap<String, NodeHealth>>>,
    config: HealthConfig,
    checker: C,
    nodes: Arc<RwLock<Vec<Node>>>,
    tx: watch::Sender<u64>,
}

impl<H, C> HealthAwareRing<H, C>
where
    H: super::HashFunction + Clone + Send + Sync + 'static,
    C: Fn(
            &Node,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + Clone
        + 'static,
{
    pub fn new(
        nodes: Vec<Node>,
        config: RingConfig,
        health_config: HealthConfig,
        hash_fn: H,
        checker: C,
    ) -> Result<Self> {
        let ring = CompactRing::new(nodes.clone(), config, hash_fn.clone())?;
        let health = nodes
            .iter()
            .map(|n| (n.id.clone(), NodeHealth::default()))
            .collect();

        let (tx, _) = watch::channel(0u64);

        Ok(Self {
            ring: Arc::new(RwLock::new(ring)),
            health: Arc::new(RwLock::new(health)),
            config: health_config,
            checker,
            nodes: Arc::new(RwLock::new(nodes)),
            tx,
        })
    }

    pub fn builder(
        nodes: Vec<Node>,
        config: RingConfig,
        hash_fn: H,
    ) -> HealthAwareRingBuilder<H, C> {
        HealthAwareRingBuilder::new(nodes, config, hash_fn)
    }

    pub async fn get(&self, key: &[u8]) -> Result<Node> {
        let ring = self.ring.read().await;
        let node = ring.get(key)?;
        Ok(node.clone())
    }

    pub async fn get_healthy(&self, key: &[u8]) -> Result<Node> {
        let ring = self.ring.read().await;
        let health = self.health.read().await;

        let hash = ring.hash_function().hash(key);
        let points = ring.points();
        let nodes = ring.nodes();

        if points.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        let start_idx = points
            .binary_search_by_key(&hash, |p| p.hash)
            .unwrap_or_else(|i| i % points.len());

        for probe in 0..points.len() {
            let idx = (start_idx + probe) % points.len();
            let point = &points[idx];
            let node_idx = point.node_index() as usize;
            let node = &nodes[node_idx];

            if let Some(h) = health.get(&node.id) {
                if h.status == HealthStatus::Healthy {
                    return Ok(node.clone());
                }
            }
        }

        Err(ConsistentHashError::EmptyRing)
    }

    pub async fn add_node(&self, node: Node) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        nodes.push(node.clone());

        let mut health = self.health.write().await;
        health.insert(node.id.clone(), NodeHealth::default());

        self.rebuild_ring().await?;
        self.tx.send_replace(self.tx.borrow().wrapping_add(1));

        Ok(())
    }

    pub async fn remove_node(&self, node_id: &str) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        nodes.retain(|n| n.id != node_id);

        let mut health = self.health.write().await;
        health.remove(node_id);

        self.rebuild_ring().await?;
        self.tx.send_replace(self.tx.borrow().wrapping_add(1));

        Ok(())
    }

    pub async fn update_weight(&self, node_id: &str, weight: u64) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        if let Some(node) = nodes.iter_mut().find(|n| n.id == node_id) {
            node.weight = weight;
        } else {
            return Err(ConsistentHashError::NodeNotFound(node_id.to_string()));
        }

        self.rebuild_ring().await?;
        self.tx.send_replace(self.tx.borrow().wrapping_add(1));

        Ok(())
    }

    async fn rebuild_ring(&self) -> Result<()> {
        let nodes = self.nodes.read().await.clone();
        let ring = self.ring.read().await;
        let config = ring.config().clone();
        let hash_fn = ring.hash_function().clone();

        let new_ring = CompactRing::new(nodes, config, hash_fn)?;
        *self.ring.write().await = new_ring;
        Ok(())
    }

    pub async fn health_status(&self, node_id: &str) -> Option<HealthStatus> {
        let health = self.health.read().await;
        health.get(node_id).map(|h| h.status)
    }

    pub async fn all_health(&self) -> HashMap<String, HealthStatus> {
        let health = self.health.read().await;
        health.iter().map(|(k, v)| (k.clone(), v.status)).collect()
    }

    pub async fn start_health_checks(&self)
    where
        C: Fn(
                &Node,
            )
                -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'static>>
            + Send
            + Sync
            + Clone
            + 'static,
    {
        let mut interval = interval(self.config.check_interval);
        let health = self.health.clone();
        let checker = self.checker.clone();
        let nodes = self.nodes.clone();
        let config = self.config.clone();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            loop {
                interval.tick().await;

                let nodes = nodes.read().await.clone();

                for node in nodes {
                    let health = health.clone();
                    let checker = checker.clone();
                    let config = config.clone();
                    let node_id = node.id.clone();
                    let node_ref = node.clone();

                    tokio::spawn(async move {
                        let result = timeout(config.timeout, checker(&node_ref)).await;

                        let mut health = health.write().await;
                        if let Some(h) = health.get_mut(&node_id) {
                            h.last_check = Some(std::time::Instant::now());

                            match result {
                                Ok(res) => match res {
                                    Ok(()) => {
                                        h.consecutive_successes += 1;
                                        h.consecutive_failures = 0;
                                        h.last_error = None;

                                        if h.consecutive_successes >= config.healthy_threshold {
                                            h.status = HealthStatus::Healthy;
                                        }
                                    }
                                    Err(e) => {
                                        h.consecutive_failures += 1;
                                        h.consecutive_successes = 0;
                                        h.last_error = Some(e.to_string());

                                        if h.consecutive_failures >= config.unhealthy_threshold {
                                            h.status = HealthStatus::Unhealthy;
                                        }
                                    }
                                },
                                Err(_) => {
                                    h.consecutive_failures += 1;
                                    h.consecutive_successes = 0;
                                    h.last_error = Some("timeout".to_string());

                                    if h.consecutive_failures >= config.unhealthy_threshold {
                                        h.status = HealthStatus::Unhealthy;
                                    }
                                }
                            }
                        }
                    });
                }

                tx.send_replace(tx.borrow().wrapping_add(1));
            }
        });
    }

    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.tx.subscribe()
    }

    pub async fn stats(&self) -> HealthAwareStats {
        let ring = self.ring.read().await;
        let health = self.health.read().await;

        let ring_stats = ring.stats();
        let healthy_count = health
            .values()
            .filter(|h| h.status == HealthStatus::Healthy)
            .count();
        let unhealthy_count = health
            .values()
            .filter(|h| h.status == HealthStatus::Unhealthy)
            .count();
        let unknown_count = health
            .values()
            .filter(|h| h.status == HealthStatus::Unknown)
            .count();

        HealthAwareStats {
            ring_stats,
            healthy_count,
            unhealthy_count,
            unknown_count,
            total_nodes: health.len(),
        }
    }
}

pub struct HealthAwareRingBuilder<H, C>
where
    H: super::HashFunction + Clone + Send + Sync + 'static,
    C: Fn(
            &Node,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + Clone
        + 'static,
{
    nodes: Vec<Node>,
    config: RingConfig,
    health_config: HealthConfig,
    hash_fn: H,
    checker: Option<C>,
}

impl<H, C> HealthAwareRingBuilder<H, C>
where
    H: super::HashFunction + Clone + Send + Sync + 'static,
    C: Fn(
            &Node,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'static>>
        + Send
        + Sync
        + Clone
        + 'static,
{
    pub fn new(nodes: Vec<Node>, config: RingConfig, hash_fn: H) -> Self {
        Self {
            nodes,
            config,
            health_config: HealthConfig::default(),
            hash_fn,
            checker: None,
        }
    }

    pub fn health_config(mut self, config: HealthConfig) -> Self {
        self.health_config = config;
        self
    }

    pub fn checker(mut self, checker: C) -> Self {
        self.checker = Some(checker);
        self
    }

    pub fn build(self) -> Result<HealthAwareRing<H, C>> {
        let checker = self.checker.ok_or_else(|| {
            ConsistentHashError::SerializationError("Health checker required".into())
        })?;

        HealthAwareRing::new(
            self.nodes,
            self.config,
            self.health_config,
            self.hash_fn,
            checker,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DefaultHash, Node, RingConfig};

    #[tokio::test]
    #[ignore = "requires running tokio runtime"]
    async fn test_health_aware_basic() {
        let nodes = vec![Node::new("node1", 100), Node::new("node2", 200)];

        let checker = |_node: &Node| -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<()>> + Send>,
        > { Box::pin(async { Ok(()) }) };

        let ring = HealthAwareRing::new(
            nodes,
            RingConfig::default(),
            HealthConfig::default(),
            DefaultHash,
            checker,
        )
        .unwrap();

        let node = ring.get(b"test").await.unwrap();
        assert!(["node1", "node2"].contains(&node.id.as_str()));
    }

    #[tokio::test]
    #[ignore = "requires running tokio runtime"]
    async fn test_health_aware_add_remove() {
        let nodes = vec![Node::new("node1", 100)];

        let checker = |_node: &Node| -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<()>> + Send>,
        > { Box::pin(async { Ok(()) }) };

        let ring = HealthAwareRing::new(
            nodes,
            RingConfig::default(),
            HealthConfig::default(),
            DefaultHash,
            checker,
        )
        .unwrap();

        ring.add_node(Node::new("node2", 200)).await.unwrap();
        let node = ring.get(b"test").await.unwrap();
        assert!(["node1", "node2"].contains(&node.id.as_str()));

        ring.remove_node("node1").await.unwrap();
        let node = ring.get(b"test").await.unwrap();
        assert_eq!(node.id, "node2");
    }

    #[tokio::test]
    #[ignore = "requires running tokio runtime"]
    async fn test_health_aware_weight_update() {
        let nodes = vec![Node::new("node1", 100), Node::new("node2", 100)];

        let checker = |_node: &Node| -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<()>> + Send>,
        > { Box::pin(async { Ok(()) }) };

        let ring = HealthAwareRing::new(
            nodes,
            RingConfig::default(),
            HealthConfig::default(),
            DefaultHash,
            checker,
        )
        .unwrap();

        ring.update_weight("node1", 500).await.unwrap();

        let mut counts = std::collections::HashMap::new();
        for i in 0..1000 {
            let node = ring.get(format!("key-{}", i).as_bytes()).await.unwrap();
            *counts.entry(node.id.clone()).or_insert(0) += 1;
        }

        let node1_count = counts.get("node1").copied().unwrap_or(0);
        let node2_count = counts.get("node2").copied().unwrap_or(0);
        assert!(node1_count > node2_count * 2);
    }
}
