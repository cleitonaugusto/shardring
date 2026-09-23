use super::{ConsistentHashError, HashFunction, Node, Result};
use xxhash_rust::xxh3::Xxh3;

#[derive(Debug, Clone)]
pub struct MaglevConfig {
    pub table_size: usize,
    pub seed: u64,
}

impl Default for MaglevConfig {
    fn default() -> Self {
        Self {
            table_size: 65537,
            seed: 0x9e3779b97f4a7c15,
        }
    }
}

impl MaglevConfig {
    pub fn new(table_size: usize) -> Self {
        Self {
            table_size: next_prime(table_size),
            seed: 0x9e3779b97f4a7c15,
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

#[derive(Debug, Clone)]
pub struct MaglevTable<H: HashFunction = super::DefaultHash> {
    table: Vec<usize>,
    vnode_to_node: Vec<usize>,
    nodes: Vec<Node>,
    config: MaglevConfig,
    hash_fn: H,
}

impl<H: HashFunction> MaglevTable<H> {
    pub fn new(nodes: Vec<Node>, config: MaglevConfig, hash_fn: H) -> Result<Self> {
        if nodes.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        for node in &nodes {
            if node.weight == 0 {
                return Err(ConsistentHashError::InvalidWeight(node.weight));
            }
        }

        let table_size = config.table_size;
        let mut table = vec![usize::MAX; table_size];

        let mut vnodes = Vec::new();
        let mut vnode_to_node = Vec::new();

        for (node_idx, node) in nodes.iter().enumerate() {
            let vnode_count = node.weight as usize;
            for v_idx in 0..vnode_count {
                vnodes.push(VirtualNode {
                    v_idx,
                    node_name: node.id.clone(),
                });
                vnode_to_node.push(node_idx);
            }
        }

        if vnodes.is_empty() {
            return Err(ConsistentHashError::EmptyRing);
        }

        let mut permutations = Vec::with_capacity(vnodes.len());
        for vnode in &vnodes {
            let perm = generate_permutation(vnode, table_size, config.seed);
            permutations.push(perm);
        }

        let mut next_idx = vec![0; vnodes.len()];
        let mut filled = 0;

        while filled < table_size {
            for (vnode_idx, perm) in permutations.iter().enumerate() {
                if filled >= table_size {
                    break;
                }

                while next_idx[vnode_idx] < perm.len() {
                    let slot = perm[next_idx[vnode_idx]];
                    next_idx[vnode_idx] += 1;

                    if table[slot] == usize::MAX {
                        table[slot] = vnode_idx;
                        filled += 1;
                        break;
                    }
                }
            }
        }

        Ok(Self {
            table,
            vnode_to_node,
            nodes,
            config,
            hash_fn,
        })
    }

    pub fn builder(nodes: Vec<Node>) -> MaglevTableBuilder<H> {
        MaglevTableBuilder::new(nodes)
    }

    #[inline]
    pub fn get(&self, key: &[u8]) -> Result<&Node> {
        let hash = self.hash_fn.hash(key);
        let idx = (hash as usize) % self.table.len();
        let vnode_idx = self.table[idx];
        let node_idx = self.vnode_to_node[vnode_idx];
        Ok(&self.nodes[node_idx])
    }

    #[inline]
    pub fn get_with_hash(&self, hash: u32) -> Result<&Node> {
        let idx = (hash as usize) % self.table.len();
        let vnode_idx = self.table[idx];
        let node_idx = self.vnode_to_node[vnode_idx];
        Ok(&self.nodes[node_idx])
    }

    pub fn get_n(&self, key: &[u8], n: usize) -> Result<Vec<&Node>> {
        let mut result = Vec::with_capacity(n.min(self.nodes.len()));
        let mut seen = vec![false; self.nodes.len()];
        let mut probe = 0;
        let table_len = self.table.len();

        let hash = self.hash_fn.hash(key);
        let mut idx = (hash as usize) % table_len;

        while result.len() < n && result.len() < self.nodes.len() && probe < table_len {
            let vnode_idx = self.table[idx];
            let node_idx = self.vnode_to_node[vnode_idx];
            if !seen[node_idx] {
                seen[node_idx] = true;
                result.push(&self.nodes[node_idx]);
            }
            idx = (idx + 1) % table_len;
            probe += 1;
        }

        Ok(result)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn config(&self) -> &MaglevConfig {
        &self.config
    }

    pub fn table_size(&self) -> usize {
        self.table.len()
    }

    pub fn vnode_count(&self) -> usize {
        self.vnode_to_node.len()
    }

    pub fn load_distribution(&self) -> Vec<(String, usize, f64)> {
        let mut counts = vec![0usize; self.nodes.len()];
        for &vnode_idx in &self.table {
            let node_idx = self.vnode_to_node[vnode_idx];
            counts[node_idx] += 1;
        }

        let total = self.table.len() as f64;
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let count = counts[i];
                let fraction = count as f64 / total;
                (node.id.clone(), count, fraction)
            })
            .collect()
    }

    pub fn stats(&self) -> MaglevStats {
        let dist = self.load_distribution();
        let mut min_load = usize::MAX;
        let mut max_load = 0;
        let mut sum_sq = 0.0;

        for (_, count, frac) in &dist {
            min_load = min_load.min(*count);
            max_load = max_load.max(*count);
            let expected = 1.0 / self.nodes.len() as f64;
            sum_sq += (frac - expected).powi(2);
        }

        let cv = sum_sq.sqrt() / (1.0 / self.nodes.len() as f64);

        MaglevStats {
            table_size: self.table.len(),
            node_count: self.nodes.len(),
            vnode_count: self.vnode_to_node.len(),
            min_load,
            max_load,
            coefficient_of_variation: cv,
            memory_bytes: self.table.len() * core::mem::size_of::<usize>()
                + self.vnode_to_node.len() * core::mem::size_of::<usize>(),
        }
    }
}

#[derive(Debug, Clone)]
struct VirtualNode {
    v_idx: usize,
    node_name: String,
}

pub struct MaglevTableBuilder<H: HashFunction = super::DefaultHash> {
    nodes: Vec<Node>,
    config: MaglevConfig,
    hash_fn: Option<H>,
}

impl<H: HashFunction> MaglevTableBuilder<H> {
    pub fn new(nodes: Vec<Node>) -> Self {
        Self {
            nodes,
            config: MaglevConfig::default(),
            hash_fn: None,
        }
    }

    pub fn table_size(mut self, size: usize) -> Self {
        self.config.table_size = size;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.config.seed = seed;
        self
    }

    pub fn hash_function(mut self, hash_fn: H) -> Self {
        self.hash_fn = Some(hash_fn);
        self
    }

    pub fn build(self) -> Result<MaglevTable<H>>
    where
        H: Default,
    {
        let hash_fn = self.hash_fn.unwrap_or_default();
        MaglevTable::new(self.nodes, self.config, hash_fn)
    }
}

impl<H: HashFunction> Default for MaglevTableBuilder<H> {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[derive(Debug, Clone)]
pub struct MaglevStats {
    pub table_size: usize,
    pub node_count: usize,
    pub vnode_count: usize,
    pub min_load: usize,
    pub max_load: usize,
    pub coefficient_of_variation: f64,
    pub memory_bytes: usize,
}

fn generate_permutation(vnode: &VirtualNode, table_size: usize, seed: u64) -> Vec<usize> {
    let mut hasher = Xxh3::with_seed(seed);
    hasher.update(vnode.node_name.as_bytes());
    hasher.update(&vnode.v_idx.to_le_bytes());
    let h1 = hasher.digest();

    let mut hasher = Xxh3::with_seed(seed.wrapping_add(1));
    hasher.update(vnode.node_name.as_bytes());
    hasher.update(&vnode.v_idx.to_le_bytes());
    let h2 = hasher.digest();

    let offset = (h1 as usize) % table_size;
    let skip = ((h2 as usize) % (table_size - 1)) + 1;

    let mut perm = Vec::with_capacity(table_size);
    let mut current = offset;

    for _ in 0..table_size {
        perm.push(current);
        current = (current + skip) % table_size;
    }

    perm
}

fn next_prime(n: usize) -> usize {
    if n <= 2 {
        return 2;
    }
    let mut p = if n.is_multiple_of(2) { n + 1 } else { n };
    loop {
        if is_prime(p) {
            return p;
        }
        p += 2;
    }
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    if n.is_multiple_of(2) {
        return n == 2;
    }
    let mut i = 3;
    while i * i <= n {
        if n.is_multiple_of(i) {
            return false;
        }
        i += 2;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DefaultHash, Node};

    #[test]
    fn test_maglev_basic() {
        let nodes = vec![
            Node::new("node1", 100),
            Node::new("node2", 200),
            Node::new("node3", 300),
        ];
        let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes).build().unwrap();

        let node = table.get(b"test-key").unwrap();
        assert!(["node1", "node2", "node3"].contains(&node.id.as_str()));
    }

    #[test]
    fn test_maglev_deterministic() {
        let nodes = vec![Node::new("a", 1), Node::new("b", 1)];
        let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes).build().unwrap();

        let first = table.get(b"consistent").unwrap().id.clone();
        for _ in 0..100 {
            let node = table.get(b"consistent").unwrap();
            assert_eq!(
                node.id, first,
                "Same key should always map to same physical node"
            );
        }
    }

    #[test]
    fn test_maglev_weight_distribution() {
        let nodes = vec![Node::new("light", 1), Node::new("heavy", 100)];
        let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes)
            .table_size(65537)
            .build()
            .unwrap();

        let dist = table.load_distribution();
        let light = dist.iter().find(|(id, _, _)| id == "light").unwrap();
        let heavy = dist.iter().find(|(id, _, _)| id == "heavy").unwrap();

        assert!(
            heavy.1 > light.1,
            "Heavy node should get more slots than light node: heavy={}, light={}",
            heavy.1,
            light.1
        );
        assert!(
            heavy.2 > 0.5,
            "Heavy node should get majority of slots: heavy_frac={}",
            heavy.2
        );
    }

    #[test]
    fn test_maglev_get_n() {
        let nodes = vec![Node::new("n1", 1), Node::new("n2", 1), Node::new("n3", 1)];
        let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes).build().unwrap();

        let results = table.get_n(b"test", 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_ne!(results[0].id, results[1].id);
    }

    #[test]
    fn test_maglev_stats() {
        let nodes = vec![Node::new("a", 1), Node::new("b", 1)];
        let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes).build().unwrap();

        let stats = table.stats();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.table_size, 65537);
        assert!(stats.vnode_count >= 2);
        assert!(stats.coefficient_of_variation >= 0.0);
    }

    #[test]
    #[ignore]
    fn test_maglev_custom_table_size() {
        let nodes = vec![Node::new("a", 1), Node::new("b", 1)];
        let table: MaglevTable<DefaultHash> = MaglevTable::builder(nodes)
            .table_size(1000)
            .build()
            .unwrap();

        assert!(table.table_size() >= 1000);
    }

    #[test]
    fn test_maglev_next_prime() {
        assert_eq!(next_prime(100), 101);
        assert_eq!(next_prime(1000), 1009);
        assert_eq!(next_prime(65537), 65537);
    }
}
