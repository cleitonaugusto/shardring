use super::{
    coefficient_of_variation, collision_probability, optimal_hashes_for_cv, CompactPointV2,
    ConsistentHashError, HashFunction, Result, SERIALIZATION_VERSION,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "serde")]
use serde_json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingConfig {
    pub base_hashes_per_node: usize,
    pub target_cv: f64,
    pub max_hashes_per_node: usize,
    pub min_hashes_per_node: usize,
    pub use_v2_format: bool,
    pub hash_function: HashFunctionType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HashFunctionType {
    Default,
    Ketama,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            base_hashes_per_node: 160,
            target_cv: 0.08,
            max_hashes_per_node: 100_000,
            min_hashes_per_node: 1,
            use_v2_format: true,
            hash_function: HashFunctionType::Default,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub weight: u64,
    #[cfg(feature = "serde")]
    pub metadata: Option<serde_json::Value>,
    #[cfg(not(feature = "serde"))]
    pub metadata: Option<()>,
}

impl Node {
    pub fn new(id: impl Into<String>, weight: u64) -> Self {
        Self {
            id: id.into(),
            weight,
            metadata: None,
        }
    }

    #[cfg(feature = "serde")]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}

#[derive(Debug, Clone)]
pub struct CompactRing<H: HashFunction = super::DefaultHash> {
    points: Vec<CompactPointV2>,
    nodes: Vec<Node>,
    config: RingConfig,
    hash_fn: H,
}

impl<H: HashFunction> CompactRing<H> {
    pub fn new(nodes: Vec<Node>, config: RingConfig, hash_fn: H) -> Result<Self> {
        if nodes.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        for node in &nodes {
            if node.weight == 0 {
                return Err(ConsistentHashError::InvalidWeight(node.weight));
            }
        }

        let total_weight: u64 = nodes.iter().map(|n| n.weight).sum();
        let n_nodes = nodes.len();

        let adaptive_hashes = optimal_hashes_for_cv(n_nodes, config.target_cv)
            .clamp(config.min_hashes_per_node, config.max_hashes_per_node);

        let hashes_per_node = adaptive_hashes.min(config.max_hashes_per_node);

        if n_nodes > u16::MAX as usize + 1 {
            return Err(ConsistentHashError::SerializationError(format!(
                "Too many nodes: {} (max {})",
                n_nodes,
                u16::MAX as usize + 1
            )));
        }

        let mut points = Vec::with_capacity(n_nodes * hashes_per_node);

        for (node_idx, node) in nodes.iter().enumerate() {
            // Points scale with weight relative to the average weight, so an
            // average-weight node gets `hashes_per_node` points.
            let node_hashes = (hashes_per_node as u128 * node.weight as u128 * n_nodes as u128
                / total_weight as u128)
                .max(1) as usize;

            // Seeds depend only on node id and point index, so every process
            // building a ring from the same nodes gets the same ring.
            for i in 0..node_hashes {
                let seed = format!("{}-{}", node.id, i);
                let hash = hash_fn.hash(seed.as_bytes());
                points.push(CompactPointV2::new(hash, node_idx as u16));
            }
        }

        // Tie-break on node index so hash collisions resolve identically everywhere.
        points.sort_unstable_by_key(|p| (p.hash, p.node_index));

        Ok(Self {
            points,
            nodes,
            config,
            hash_fn,
        })
    }

    pub fn builder() -> RingBuilder<H> {
        RingBuilder::new()
    }

    #[inline]
    pub fn get(&self, key: &[u8]) -> Result<&Node> {
        if self.points.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        let hash = self.hash_fn.hash(key);
        let idx = self.points.binary_search_by_key(&hash, |p| p.hash);

        let point_idx = match idx {
            Ok(i) => i,
            Err(i) => i % self.points.len(),
        };

        let node_idx = self.points[point_idx].node_index() as usize;
        Ok(&self.nodes[node_idx])
    }

    #[inline]
    pub fn get_with_hash(&self, hash: u32) -> Result<&Node> {
        if self.points.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        let idx = self.points.binary_search_by_key(&hash, |p| p.hash);
        let point_idx = match idx {
            Ok(i) => i,
            Err(i) => i % self.points.len(),
        };

        let node_idx = self.points[point_idx].node_index() as usize;
        Ok(&self.nodes[node_idx])
    }

    pub fn get_n(&self, key: &[u8], n: usize) -> Result<Vec<&Node>> {
        if self.points.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        let hash = self.hash_fn.hash(key);
        let idx = self.points.binary_search_by_key(&hash, |p| p.hash);
        let start = match idx {
            Ok(i) => i,
            Err(i) => i % self.points.len(),
        };

        let mut result = Vec::with_capacity(n.min(self.nodes.len()));
        let mut seen = vec![false; self.nodes.len()];
        let mut i = start;

        while result.len() < n && result.len() < self.nodes.len() {
            let point = &self.points[i];
            let node_idx = point.node_index() as usize;
            if !seen[node_idx] {
                seen[node_idx] = true;
                result.push(&self.nodes[node_idx]);
            }
            i = (i + 1) % self.points.len();
        }

        Ok(result)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn points(&self) -> &[CompactPointV2] {
        &self.points
    }

    pub fn config(&self) -> &RingConfig {
        &self.config
    }

    pub fn hash_function(&self) -> &H {
        &self.hash_fn
    }

    pub fn stats(&self) -> RingStats {
        let n_nodes = self.nodes.len();
        let n_points = self.points.len();
        let avg_hashes = n_points as f64 / n_nodes as f64;
        let cv = coefficient_of_variation(n_nodes, avg_hashes as usize);
        let collision_prob = collision_probability(n_points, 32);

        RingStats {
            node_count: n_nodes,
            total_points: n_points,
            avg_hashes_per_node: avg_hashes,
            coefficient_of_variation: cv,
            collision_probability: collision_prob,
            memory_bytes: n_points * CompactPointV2::SIZE,
        }
    }

    /// Serializes the ring, including its nodes, as:
    /// `version: u32 | n_nodes: u32 | (weight: u64, id_len: u32, id)* | n_points: u32 | (hash: u32, node_index: u16)*`
    /// (all little-endian). Node metadata is not serialized.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&SERIALIZATION_VERSION.to_le_bytes());
        buf.extend_from_slice(&(self.nodes.len() as u32).to_le_bytes());
        for node in &self.nodes {
            buf.extend_from_slice(&node.weight.to_le_bytes());
            buf.extend_from_slice(&(node.id.len() as u32).to_le_bytes());
            buf.extend_from_slice(node.id.as_bytes());
        }
        buf.extend_from_slice(&(self.points.len() as u32).to_le_bytes());
        for point in &self.points {
            buf.extend_from_slice(&point.hash.to_le_bytes());
            buf.extend_from_slice(&point.node_index.to_le_bytes());
        }
        buf
    }

    pub fn from_bytes(bytes: &[u8], config: RingConfig, hash_fn: H) -> Result<Self> {
        let mut reader = ByteReader { bytes, offset: 0 };

        let version = reader.u32()?;
        if version != SERIALIZATION_VERSION {
            return Err(ConsistentHashError::VersionMismatch {
                expected: SERIALIZATION_VERSION,
                got: version,
            });
        }

        let n_nodes = reader.u32()? as usize;
        let mut nodes = Vec::with_capacity(n_nodes.min(bytes.len()));
        for _ in 0..n_nodes {
            let weight = reader.u64()?;
            let id_len = reader.u32()? as usize;
            let id = core::str::from_utf8(reader.take(id_len)?)
                .map_err(|e| ConsistentHashError::DeserializationError(e.to_string()))?;
            nodes.push(Node::new(id, weight));
        }

        let n_points = reader.u32()? as usize;
        let mut points = Vec::with_capacity(n_points.min(bytes.len() / CompactPointV2::SIZE));
        for _ in 0..n_points {
            let hash = reader.u32()?;
            let node_index = u16::from_le_bytes(reader.take(2)?.try_into().unwrap());
            if node_index as usize >= nodes.len() {
                return Err(ConsistentHashError::DeserializationError(format!(
                    "Point references node {} but ring has {} nodes",
                    node_index,
                    nodes.len()
                )));
            }
            points.push(CompactPointV2::new(hash, node_index));
        }

        Ok(Self {
            points,
            nodes,
            config,
            hash_fn,
        })
    }
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> ByteReader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len());
        let Some(end) = end else {
            return Err(ConsistentHashError::DeserializationError(format!(
                "Unexpected end of buffer at offset {} (needed {} bytes, have {})",
                self.offset,
                len,
                self.bytes.len() - self.offset
            )));
        };
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RingStats {
    pub node_count: usize,
    pub total_points: usize,
    pub avg_hashes_per_node: f64,
    pub coefficient_of_variation: f64,
    pub collision_probability: f64,
    pub memory_bytes: usize,
}

pub struct RingBuilder<H: HashFunction = super::DefaultHash> {
    nodes: Vec<Node>,
    config: RingConfig,
    hash_fn: Option<H>,
}

impl<H: HashFunction> RingBuilder<H> {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            config: RingConfig::default(),
            hash_fn: None,
        }
    }

    pub fn add_node(mut self, node: Node) -> Self {
        self.nodes.push(node);
        self
    }

    pub fn add_nodes(mut self, nodes: Vec<Node>) -> Self {
        self.nodes.extend(nodes);
        self
    }

    pub fn config(mut self, config: RingConfig) -> Self {
        self.config = config;
        self
    }

    pub fn base_hashes(mut self, n: usize) -> Self {
        self.config.base_hashes_per_node = n;
        self
    }

    pub fn target_cv(mut self, cv: f64) -> Self {
        self.config.target_cv = cv;
        self
    }

    pub fn hash_function(mut self, hash_fn: H) -> Self {
        self.hash_fn = Some(hash_fn);
        self
    }

    pub fn use_v2_format(mut self, use_v2: bool) -> Self {
        self.config.use_v2_format = use_v2;
        self
    }

    pub fn build(self) -> Result<CompactRing<H>>
    where
        H: Default,
    {
        let hash_fn = self.hash_fn.unwrap_or_default();
        CompactRing::new(self.nodes, self.config, hash_fn)
    }
}

impl<H: HashFunction> Default for RingBuilder<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: HashFunction + Default> Default for CompactRing<H> {
    fn default() -> Self {
        Self::builder().build().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DefaultHash;

    #[test]
    fn test_basic_ring() {
        let nodes = vec![
            Node::new("node1", 100),
            Node::new("node2", 200),
            Node::new("node3", 300),
        ];
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .build()
            .unwrap();

        let node = ring.get(b"test-key").unwrap();
        assert!(["node1", "node2", "node3"].contains(&node.id.as_str()));
    }

    #[test]
    fn test_deterministic() {
        let nodes = vec![Node::new("a", 1), Node::new("b", 1)];
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .build()
            .unwrap();

        let first = ring.get(b"consistent").unwrap().id.clone();
        for _ in 0..100 {
            let node = ring.get(b"consistent").unwrap();
            assert_eq!(node.id, first, "Same key should always map to same node");
        }
    }

    #[test]
    fn test_weight_distribution() {
        let nodes = vec![Node::new("light", 1), Node::new("heavy", 100)];
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .config(RingConfig {
                base_hashes_per_node: 160,
                ..Default::default()
            })
            .build()
            .unwrap();

        let mut counts = std::collections::HashMap::new();
        for i in 0..10000 {
            let key = format!("key-{}", i);
            let node = ring.get(key.as_bytes()).unwrap();
            *counts.entry(node.id.clone()).or_insert(0) += 1;
        }

        let light = counts.get("light").copied().unwrap_or(0);
        let heavy = counts.get("heavy").copied().unwrap_or(0);
        assert!(heavy > light * 10);
    }

    #[test]
    fn test_get_n() {
        let nodes = vec![Node::new("n1", 1), Node::new("n2", 1), Node::new("n3", 1)];
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .build()
            .unwrap();

        let results = ring.get_n(b"test", 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_ne!(results[0].id, results[1].id);
    }

    #[test]
    fn test_serialization() {
        let nodes = vec![Node::new("a", 1), Node::new("b", 2)];
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes.clone())
            .build()
            .unwrap();

        let bytes = ring.to_bytes();
        let restored =
            CompactRing::<DefaultHash>::from_bytes(&bytes, RingConfig::default(), DefaultHash)
                .unwrap();

        assert_eq!(ring.nodes().len(), restored.nodes().len());
        for (a, b) in nodes.iter().zip(restored.nodes()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.weight, b.weight);
        }
        for i in 0..1000 {
            let key = format!("key-{}", i);
            assert_eq!(
                ring.get(key.as_bytes()).unwrap().id,
                restored.get(key.as_bytes()).unwrap().id
            );
        }
        assert_eq!(ring.points.len(), restored.points.len());
        for (a, b) in ring.points.iter().zip(restored.points.iter()) {
            assert_eq!(a.hash(), b.hash());
            assert_eq!(a.node_index(), b.node_index());
        }
    }

    fn equal_nodes(n: usize) -> Vec<Node> {
        (0..n)
            .map(|i| Node::new(format!("node-{}", i), 1))
            .collect()
    }

    #[test]
    fn test_independent_rings_agree() {
        let a =
            CompactRing::<DefaultHash>::new(equal_nodes(20), RingConfig::default(), DefaultHash)
                .unwrap();
        let b =
            CompactRing::<DefaultHash>::new(equal_nodes(20), RingConfig::default(), DefaultHash)
                .unwrap();

        for i in 0..10_000 {
            let key = format!("key-{}", i);
            assert_eq!(
                a.get(key.as_bytes()).unwrap().id,
                b.get(key.as_bytes()).unwrap().id
            );
        }
    }

    #[test]
    fn test_distribution_cv_with_many_nodes() {
        for n in [10, 50, 200] {
            let ring =
                CompactRing::<DefaultHash>::new(equal_nodes(n), RingConfig::default(), DefaultHash)
                    .unwrap();

            let keys = n * 2_000;
            let mut counts = std::collections::HashMap::new();
            for i in 0..keys {
                let key = format!("key-{}", i);
                *counts
                    .entry(ring.get(key.as_bytes()).unwrap().id.clone())
                    .or_insert(0usize) += 1;
            }
            assert_eq!(counts.len(), n, "every node should own keys");

            let mean = keys as f64 / n as f64;
            let variance = counts
                .values()
                .map(|&c| (c as f64 - mean).powi(2))
                .sum::<f64>()
                / n as f64;
            let cv = variance.sqrt() / mean;
            // target_cv defaults to 0.08; allow headroom for key-sampling noise.
            assert!(cv < 0.15, "CV {:.3} too high for {} nodes", cv, n);
        }
    }

    #[test]
    fn test_adding_node_moves_few_keys() {
        let before =
            CompactRing::<DefaultHash>::new(equal_nodes(10), RingConfig::default(), DefaultHash)
                .unwrap();
        let after =
            CompactRing::<DefaultHash>::new(equal_nodes(11), RingConfig::default(), DefaultHash)
                .unwrap();

        let keys = 20_000;
        let mut moved = 0;
        for i in 0..keys {
            let key = format!("key-{}", i);
            let old = &before.get(key.as_bytes()).unwrap().id;
            let new = &after.get(key.as_bytes()).unwrap().id;
            if old != new {
                moved += 1;
            }
        }
        // Ideal is 1/11 (~9%). Points per node also grow slightly with n, so allow some slack.
        let moved_frac = moved as f64 / keys as f64;
        assert!(moved_frac < 0.2, "moved {:.1}% of keys", moved_frac * 100.0);
    }

    #[test]
    fn test_from_bytes_rejects_bad_input() {
        let ring =
            CompactRing::<DefaultHash>::new(equal_nodes(3), RingConfig::default(), DefaultHash)
                .unwrap();
        let bytes = ring.to_bytes();

        let mut wrong_version = bytes.clone();
        wrong_version[0] ^= 0xff;
        assert!(matches!(
            CompactRing::<DefaultHash>::from_bytes(
                &wrong_version,
                RingConfig::default(),
                DefaultHash
            ),
            Err(ConsistentHashError::VersionMismatch { .. })
        ));

        let truncated = &bytes[..bytes.len() - 1];
        assert!(CompactRing::<DefaultHash>::from_bytes(
            truncated,
            RingConfig::default(),
            DefaultHash
        )
        .is_err());
    }

    #[test]
    fn test_stats() {
        let nodes = vec![Node::new("a", 1), Node::new("b", 1)];
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .build()
            .unwrap();

        let stats = ring.stats();
        assert_eq!(stats.node_count, 2);
        assert!(stats.total_points > 0);
        assert!(stats.memory_bytes > 0);
    }
}
