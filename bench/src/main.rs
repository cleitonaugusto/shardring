use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use shardring::{
    adaptive::recommend_ring_config,
    migration::{DualRing, MigrationStrategy},
    CompactRing, DefaultHash, Node, RingConfig,
};
use std::hint::black_box as std_black_box;

fn generate_nodes(count: usize, weight_range: (u64, u64)) -> Vec<Node> {
    let mut rng = SmallRng::from_entropy();
    (0..count)
        .map(|i| {
            Node::new(
                format!("node-{}", i),
                rng.gen_range(weight_range.0..=weight_range.1),
            )
        })
        .collect()
}

fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("get");

    for node_count in [10, 100, 1000, 10_000] {
        let nodes = generate_nodes(node_count, (1, 1000));
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .build()
            .unwrap();

        let keys: Vec<Vec<u8>> = (0..1000)
            .map(|i| format!("key-{}", i).into_bytes())
            .collect();

        group.throughput(Throughput::Elements(1000));
        group.bench_with_input(BenchmarkId::new("nodes", node_count), &keys, |b, keys| {
            b.iter(|| {
                for key in keys {
                    std_black_box(ring.get(key).unwrap());
                }
            });
        });
    }
    group.finish();
}

fn bench_get_n(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_n");

    let nodes = generate_nodes(100, (1, 1000));
    let ring = CompactRing::<DefaultHash>::builder()
        .add_nodes(nodes)
        .build()
        .unwrap();

    let keys: Vec<Vec<u8>> = (0..1000)
        .map(|i| format!("key-{}", i).into_bytes())
        .collect();

    for n in [1, 3, 5, 10] {
        group.throughput(Throughput::Elements(1000));
        group.bench_with_input(BenchmarkId::new("replicas", n), &keys, |b, keys| {
            b.iter(|| {
                for key in keys {
                    std_black_box(ring.get_n(key, n).unwrap());
                }
            });
        });
    }
    group.finish();
}

fn bench_ring_creation(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring_creation");

    for node_count in [10, 100, 1000, 10_000] {
        let nodes = generate_nodes(node_count, (1, 1000));

        group.bench_with_input(BenchmarkId::new("nodes", node_count), &nodes, |b, nodes| {
            b.iter(|| {
                std_black_box(
                    CompactRing::<DefaultHash>::builder()
                        .add_nodes(nodes.clone())
                        .build()
                        .unwrap(),
                );
            });
        });
    }
    group.finish();
}

fn bench_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialization");

    for node_count in [100, 1000, 10_000] {
        let nodes = generate_nodes(node_count, (1, 1000));
        let ring = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes.clone())
            .build()
            .unwrap();

        group.bench_with_input(
            BenchmarkId::new("serialize", node_count),
            &ring,
            |b, ring| {
                b.iter(|| std_black_box(ring.to_bytes()));
            },
        );

        let bytes = ring.to_bytes();
        group.bench_with_input(
            BenchmarkId::new("deserialize", node_count),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    std_black_box(
                        CompactRing::<DefaultHash>::from_bytes(
                            bytes,
                            RingConfig::default(),
                            DefaultHash,
                        )
                        .unwrap(),
                    );
                });
            },
        );
    }
    group.finish();
}

fn bench_migration(c: &mut Criterion) {
    let mut group = c.benchmark_group("migration");

    let nodes = generate_nodes(1000, (1, 1000));

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

    let old_ring = CompactRing::new(nodes.clone(), old_config.clone(), DefaultHash).unwrap();
    let new_ring = CompactRing::new(nodes, new_config.clone(), DefaultHash).unwrap();

    let dual = DualRing::builder(old_ring, new_ring)
        .strategy(MigrationStrategy::HashBased)
        .rollout_percentage(0.5)
        .build()
        .unwrap();

    let keys: Vec<Vec<u8>> = (0..10000)
        .map(|i| format!("key-{}", i).into_bytes())
        .collect();

    group.throughput(Throughput::Elements(10000));
    group.bench_function("hash_based_50pct", |b| {
        b.iter(|| {
            for key in &keys {
                std_black_box(dual.get(key).unwrap());
            }
        });
    });

    let dual_dc = DualRing::builder(
        CompactRing::new(
            generate_nodes(1000, (1, 1000)),
            old_config.clone(),
            DefaultHash,
        )
        .unwrap(),
        CompactRing::new(
            generate_nodes(1000, (1, 1000)),
            new_config.clone(),
            DefaultHash,
        )
        .unwrap(),
    )
    .strategy(MigrationStrategy::Datacenter)
    .datacenter_allowlist(vec!["dc1".into(), "dc2".into()])
    .build()
    .unwrap();

    group.bench_function("datacenter_based", |b| {
        b.iter(|| {
            for key in &keys {
                std_black_box(dual_dc.get_with_datacenter(key, "dc1").unwrap());
            }
        });
    });

    group.finish();
}

fn bench_adaptive_config(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_config");

    for node_count in [10, 100, 1000, 10_000] {
        let mut rng = SmallRng::from_entropy();
        let weights: Vec<u64> = (0..node_count)
            .map(|_| rng.gen::<u64>() % 1000 + 1)
            .collect();

        group.bench_with_input(
            BenchmarkId::new("recommend", node_count),
            &weights,
            |b, weights| {
                b.iter(|| std_black_box(recommend_ring_config(weights.len(), weights, None)));
            },
        );
    }
    group.finish();
}

fn bench_memory_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_comparison");

    for node_count in [100, 1000, 10_000] {
        let nodes = generate_nodes(node_count, (1, 1000));

        let ring_v1 = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes.clone())
            .config(RingConfig {
                use_v2_format: false,
                ..Default::default()
            })
            .build()
            .unwrap();

        let ring_v2 = CompactRing::<DefaultHash>::builder()
            .add_nodes(nodes)
            .config(RingConfig {
                use_v2_format: true,
                ..Default::default()
            })
            .build()
            .unwrap();

        let stats_v1 = ring_v1.stats();
        let stats_v2 = ring_v2.stats();

        println!(
            "Nodes: {}, V1: {} bytes ({} pts), V2: {} bytes ({} pts), Savings: {:.1}%",
            node_count,
            stats_v1.memory_bytes,
            stats_v1.total_points,
            stats_v2.memory_bytes,
            stats_v2.total_points,
            (1.0 - stats_v2.memory_bytes as f64 / stats_v1.memory_bytes as f64) * 100.0
        );

        group.bench_with_input(
            BenchmarkId::new("v1_get", node_count),
            &ring_v1,
            |b, ring| {
                let keys: Vec<Vec<u8>> =
                    (0..1000).map(|i| format!("k{}", i).into_bytes()).collect();
                b.iter(|| {
                    for key in &keys {
                        std_black_box(ring.get(key).unwrap());
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("v2_get", node_count),
            &ring_v2,
            |b, ring| {
                let keys: Vec<Vec<u8>> =
                    (0..1000).map(|i| format!("k{}", i).into_bytes()).collect();
                b.iter(|| {
                    for key in &keys {
                        std_black_box(ring.get(key).unwrap());
                    }
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_get,
    bench_get_n,
    bench_ring_creation,
    bench_serialization,
    bench_migration,
    bench_adaptive_config,
    bench_memory_comparison
);
criterion_main!(benches);
