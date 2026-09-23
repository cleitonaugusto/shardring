use bytemuck::{Pod, Zeroable};
use thiserror::Error;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

pub mod adaptive;
pub mod jump;
pub mod maglev;
pub mod migration;
pub mod ring;

#[cfg(feature = "std")]
pub mod bounded;

#[cfg(feature = "health")]
pub mod health;

pub use adaptive::{AdaptiveConfig, OptimalHashCount};
pub use jump::{jump_consistent_hash, jump_consistent_hash_u64, JumpHash};
pub use maglev::{MaglevConfig, MaglevStats, MaglevTable, MaglevTableBuilder};
pub use migration::{DualRing, MigrationConfig, MigrationStrategy};
pub use ring::{CompactRing, Node, RingBuilder, RingConfig, RingStats};

#[cfg(feature = "std")]
pub use bounded::{BoundedConfig, BoundedRing, BoundedStats, LoadTracker, NodeLoadStat};

#[cfg(feature = "health")]
pub use health::{
    HealthAwareRing, HealthAwareRingBuilder, HealthAwareStats, HealthConfig, HealthStatus,
    NodeHealth,
};

pub use metrics::{AtomicMetrics, NoOpMetrics, RingMetrics};

#[derive(Debug, Error)]
pub enum ConsistentHashError {
    #[error("Ring is empty, cannot select node")]
    EmptyRing,
    #[error("Invalid weight: {0} (must be > 0)")]
    InvalidWeight(u64),
    #[error("Node not found: {0}")]
    NodeNotFound(String),
    #[error("Migration in progress: {0}")]
    MigrationInProgress(String),
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
    #[error("Version mismatch: expected {expected}, got {got}")]
    VersionMismatch { expected: u32, got: u32 },
}

pub type Result<T> = core::result::Result<T, ConsistentHashError>;

/// Current serialization format version
pub const SERIALIZATION_VERSION: u32 = 1;

#[repr(C, packed)]
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Pod,
    Zeroable,
    FromBytes,
    IntoBytes,
    KnownLayout,
    Immutable,
    Unaligned,
)]
pub struct CompactPoint {
    hash: u32,
    node_index: u16,
    _padding: u16,
}

impl CompactPoint {
    pub const SIZE: usize = 8;

    #[inline]
    pub fn new(hash: u32, node_index: u16) -> Self {
        Self {
            hash,
            node_index,
            _padding: 0,
        }
    }

    #[inline]
    pub fn hash(&self) -> u32 {
        self.hash
    }

    #[inline]
    pub fn node_index(&self) -> u16 {
        self.node_index
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8; 8]) -> Self {
        *bytemuck::from_bytes(bytes)
    }

    #[inline]
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(bytemuck::bytes_of(self));
        bytes
    }
}

#[repr(C, packed)]
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Pod,
    Zeroable,
    FromBytes,
    IntoBytes,
    KnownLayout,
    Immutable,
    Unaligned,
)]
pub struct CompactPointV2 {
    hash: u32,
    node_index: u16,
}

impl CompactPointV2 {
    pub const SIZE: usize = 6;

    #[inline]
    pub fn new(hash: u32, node_index: u16) -> Self {
        Self { hash, node_index }
    }

    #[inline]
    pub fn hash(&self) -> u32 {
        self.hash
    }

    #[inline]
    pub fn node_index(&self) -> u16 {
        self.node_index
    }
}

pub trait HashFunction: Send + Sync {
    fn hash(&self, data: &[u8]) -> u32;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultHash;

impl HashFunction for DefaultHash {
    #[inline]
    fn hash(&self, data: &[u8]) -> u32 {
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        hasher.update(data);
        hasher.digest() as u32
    }
}

#[derive(Clone, Copy, Debug)]
pub struct KetamaHash;

impl HashFunction for KetamaHash {
    #[inline]
    fn hash(&self, data: &[u8]) -> u32 {
        use md5::Digest;
        let mut hasher = md5::Md5::new();
        hasher.update(data);
        let result = hasher.finalize();
        u32::from_le_bytes([result[0], result[1], result[2], result[3]])
    }
}

pub fn coefficient_of_variation(n_nodes: usize, hashes_per_node: usize) -> f64 {
    if n_nodes <= 1 {
        return 0.0;
    }
    let n = n_nodes as f64;
    let k = hashes_per_node as f64;
    ((n - 1.0) / (n + 1.0) / k).sqrt()
}

pub fn optimal_hashes_for_cv(n_nodes: usize, target_cv: f64) -> usize {
    if n_nodes <= 1 || target_cv <= 0.0 {
        return 1;
    }
    let n = n_nodes as f64;
    let k = (n - 1.0) / (n + 1.0) / (target_cv * target_cv);
    k.ceil().max(1.0) as usize
}

pub fn collision_probability(total_points: usize, hash_bits: u32) -> f64 {
    let n = total_points as f64;
    let m = 2.0_f64.powi(hash_bits as i32);
    1.0 - (-n * (n - 1.0) / (2.0 * m)).exp()
}

pub mod metrics {

    use core::sync::atomic::{AtomicU64, Ordering};

    pub trait RingMetrics: Send + Sync {
        fn record_get(&self, latency_ns: u64);
        fn record_get_healthy(&self, latency_ns: u64);
        fn record_migration(&self, strategy: &str);
        fn record_rebalance(&self, nodes_affected: u64);
        fn record_error(&self, error_type: &str);
    }

    pub struct NoOpMetrics;

    impl RingMetrics for NoOpMetrics {
        fn record_get(&self, _latency_ns: u64) {}
        fn record_get_healthy(&self, _latency_ns: u64) {}
        fn record_migration(&self, _strategy: &str) {}
        fn record_rebalance(&self, _nodes_affected: u64) {}
        fn record_error(&self, _error_type: &str) {}
    }

    // Atomic counters for no-std/no-metrics environments
    pub struct AtomicMetrics {
        gets_total: AtomicU64,
        gets_latency_sum: AtomicU64,
        gets_latency_count: AtomicU64,
        healthy_gets_total: AtomicU64,
        healthy_gets_latency_sum: AtomicU64,
        healthy_gets_latency_count: AtomicU64,
        migrations_total: AtomicU64,
        rebalances_total: AtomicU64,
        errors_total: AtomicU64,
    }

    impl Default for AtomicMetrics {
        fn default() -> Self {
            Self {
                gets_total: AtomicU64::new(0),
                gets_latency_sum: AtomicU64::new(0),
                gets_latency_count: AtomicU64::new(0),
                healthy_gets_total: AtomicU64::new(0),
                healthy_gets_latency_sum: AtomicU64::new(0),
                healthy_gets_latency_count: AtomicU64::new(0),
                migrations_total: AtomicU64::new(0),
                rebalances_total: AtomicU64::new(0),
                errors_total: AtomicU64::new(0),
            }
        }
    }

    impl RingMetrics for AtomicMetrics {
        fn record_get(&self, latency_ns: u64) {
            self.gets_total.fetch_add(1, Ordering::Relaxed);
            self.gets_latency_sum
                .fetch_add(latency_ns, Ordering::Relaxed);
            self.gets_latency_count.fetch_add(1, Ordering::Relaxed);
        }

        fn record_get_healthy(&self, latency_ns: u64) {
            self.healthy_gets_total.fetch_add(1, Ordering::Relaxed);
            self.healthy_gets_latency_sum
                .fetch_add(latency_ns, Ordering::Relaxed);
            self.healthy_gets_latency_count
                .fetch_add(1, Ordering::Relaxed);
        }

        fn record_migration(&self, _strategy: &str) {
            self.migrations_total.fetch_add(1, Ordering::Relaxed);
        }

        fn record_rebalance(&self, nodes_affected: u64) {
            self.rebalances_total
                .fetch_add(nodes_affected, Ordering::Relaxed);
        }

        fn record_error(&self, _error_type: &str) {
            self.errors_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl AtomicMetrics {
        pub fn gets_per_sec(&self, window_secs: u64) -> f64 {
            self.gets_total.load(Ordering::Relaxed) as f64 / window_secs as f64
        }

        pub fn avg_get_latency_ns(&self) -> Option<u64> {
            let count = self.gets_latency_count.load(Ordering::Relaxed);
            if count == 0 {
                return None;
            }
            Some(self.gets_latency_sum.load(Ordering::Relaxed) / count)
        }

        pub fn avg_healthy_get_latency_ns(&self) -> Option<u64> {
            let count = self.healthy_gets_latency_count.load(Ordering::Relaxed);
            if count == 0 {
                return None;
            }
            Some(self.healthy_gets_latency_sum.load(Ordering::Relaxed) / count)
        }
    }
}
