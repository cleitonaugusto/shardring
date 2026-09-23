use super::{RingConfig, RingStats};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveConfig {
    pub target_cv: f64,
    pub max_cv: f64,
    pub min_hashes_per_node: usize,
    pub max_hashes_per_node: usize,
    pub collision_threshold: f64,
    pub weight_skew_threshold: f64,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            target_cv: 0.08,
            max_cv: 0.15,
            min_hashes_per_node: 1,
            max_hashes_per_node: 100_000,
            collision_threshold: 0.01,
            weight_skew_threshold: 100.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct OptimalHashCount {
    pub base_hashes: usize,
    pub weighted_hashes: usize,
    pub estimated_cv: f64,
    pub collision_probability: f64,
    pub memory_bytes: usize,
}

pub fn calculate_optimal_hashes(
    node_count: usize,
    total_weight: u64,
    weight_distribution: &[u64],
    config: &AdaptiveConfig,
) -> OptimalHashCount {
    if node_count == 0 {
        return OptimalHashCount {
            base_hashes: config.min_hashes_per_node,
            weighted_hashes: 0,
            estimated_cv: 0.0,
            collision_probability: 0.0,
            memory_bytes: 0,
        };
    }

    let avg_weight = total_weight as f64 / node_count as f64;
    let max_weight = *weight_distribution.iter().max().unwrap_or(&1) as f64;
    let weight_skew = max_weight / avg_weight;

    let base_hashes = super::optimal_hashes_for_cv(node_count, config.target_cv)
        .clamp(config.min_hashes_per_node, config.max_hashes_per_node);

    let weighted_hashes = if weight_skew > config.weight_skew_threshold {
        (base_hashes as f64 * weight_skew.ln()).ceil() as usize
    } else {
        base_hashes
    };

    let weighted_hashes = weighted_hashes.min(config.max_hashes_per_node);

    let total_points = (node_count as f64 * weighted_hashes as f64) as usize;
    let collision_prob = super::collision_probability(total_points, 32);
    let estimated_cv = super::coefficient_of_variation(node_count, weighted_hashes);

    OptimalHashCount {
        base_hashes,
        weighted_hashes,
        estimated_cv,
        collision_probability: collision_prob,
        memory_bytes: total_points * 6,
    }
}

pub fn recommend_ring_config(
    node_count: usize,
    weights: &[u64],
    constraints: Option<&AdaptiveConfig>,
) -> RingConfig {
    let default_config = AdaptiveConfig::default();
    let config = constraints.unwrap_or(&default_config);
    let total_weight: u64 = weights.iter().sum();

    let optimal = calculate_optimal_hashes(node_count, total_weight, weights, config);

    let mut ring_config = RingConfig {
        base_hashes_per_node: optimal.base_hashes,
        target_cv: config.target_cv,
        max_hashes_per_node: config.max_hashes_per_node,
        min_hashes_per_node: config.min_hashes_per_node,
        use_v2_format: true,
        ..Default::default()
    };

    if optimal.collision_probability > config.collision_threshold {
        ring_config.base_hashes_per_node = (optimal.base_hashes as f64 * 0.8).ceil() as usize;
    }

    ring_config
}

pub fn analyze_ring_efficiency(stats: &RingStats, config: &AdaptiveConfig) -> EfficiencyReport {
    let mut issues = Vec::new();
    let mut recommendations = Vec::new();

    if stats.coefficient_of_variation > config.max_cv {
        issues.push(format!(
            "CV {:.2}% exceeds max {:.2}% - consider increasing hashes per node",
            stats.coefficient_of_variation * 100.0,
            config.max_cv * 100.0
        ));
        recommendations.push("Increase target_cv or base_hashes_per_node".into());
    }

    if stats.collision_probability > config.collision_threshold {
        issues.push(format!(
            "Collision probability {:.2}% exceeds threshold {:.2}%",
            stats.collision_probability * 100.0,
            config.collision_threshold * 100.0
        ));
        recommendations.push("Reduce hashes per node or use 64-bit hashes".into());
    }

    let memory_mb = stats.memory_bytes as f64 / (1024.0 * 1024.0);
    if memory_mb > 100.0 {
        issues.push(format!("Memory usage {:.1}MB is high", memory_mb));
        recommendations.push("Consider adaptive hash count or v2 format".into());
    }

    if stats.avg_hashes_per_node < config.min_hashes_per_node as f64 {
        issues.push("Average hashes per node below minimum".into());
        recommendations.push("Increase min_hashes_per_node".into());
    }

    let is_efficient = issues.is_empty();
    let suggested_config = if is_efficient {
        None
    } else {
        Some(AdaptiveConfig {
            target_cv: config.target_cv * 0.9,
            ..*config
        })
    };

    EfficiencyReport {
        is_efficient,
        issues,
        recommendations,
        current_stats: stats.clone(),
        suggested_config,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EfficiencyReport {
    pub is_efficient: bool,
    pub issues: Vec<String>,
    pub recommendations: Vec<String>,
    pub current_stats: RingStats,
    pub suggested_config: Option<AdaptiveConfig>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimal_hashes_uniform() {
        let weights = vec![100, 100, 100, 100];
        let config = AdaptiveConfig::default();

        let optimal = calculate_optimal_hashes(4, 400, &weights, &config);

        assert!(optimal.base_hashes >= config.min_hashes_per_node);
        assert!(optimal.base_hashes <= config.max_hashes_per_node);
        assert!(optimal.estimated_cv <= config.target_cv * 1.5);
    }

    #[test]
    fn test_optimal_hashes_skewed() {
        let weights = vec![1, 1, 1, 10000];
        let config = AdaptiveConfig::default();

        let optimal = calculate_optimal_hashes(4, 10003, &weights, &config);

        assert!(optimal.weighted_hashes >= optimal.base_hashes);
    }

    #[test]
    fn test_recommend_config() {
        let weights = vec![100, 200, 300];
        let config = recommend_ring_config(3, &weights, None);

        assert!(config.base_hashes_per_node > 0);
        assert!(config.target_cv > 0.0);
    }

    #[test]
    fn test_efficiency_analysis() {
        let stats = super::super::RingStats {
            node_count: 100,
            total_points: 16000,
            avg_hashes_per_node: 160.0,
            coefficient_of_variation: 0.08,
            collision_probability: 0.005,
            memory_bytes: 96000,
        };

        let config = AdaptiveConfig::default();
        let report = analyze_ring_efficiency(&stats, &config);

        assert!(report.is_efficient);
    }

    #[test]
    fn test_efficiency_warnings() {
        let stats = super::super::RingStats {
            node_count: 100,
            total_points: 1000000,
            avg_hashes_per_node: 10000.0,
            coefficient_of_variation: 0.01,
            collision_probability: 0.15,
            memory_bytes: 6_000_000,
        };

        let config = AdaptiveConfig::default();
        let report = analyze_ring_efficiency(&stats, &config);

        assert!(!report.is_efficient);
        assert!(!report.issues.is_empty());
    }
}
