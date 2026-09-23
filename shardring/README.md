# shardring

> Repository landing page with diagrams: <https://github.com/cleitonaugusto/shardring>

Consistent hashing in Rust with **bounded loads**, **live migration between ring configurations**, and a compact in-memory layout. Works on `wasm32`.

[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

```toml
[dependencies]
shardring = "0.1"
```

> **Status**: 0.1, API unstable. The core is tested (unit, property-based and fuzz targets) but has not been run in production. Benchmarks are included; published numbers are not — see [Performance](#performance).

## Why another one

Most consistent hashing crates give you a ring and a `get()`. The parts that hurt in practice are the ones around it:

- **A hot key or an uneven ring overloads one node.** Plain consistent hashing has no back pressure, so tail load is expected — especially with few nodes. `BoundedRing` implements *Consistent Hashing with Bounded Loads* (Mirrokni, Thorup, Zadimoghaddam, 2016): a per-node cap of `(1+ε)·mean`, overflowing to the next node on the ring.
- **Changing the ring reshuffles everything at once.** `DualRing` runs the old and the new ring side by side, moves a configurable share of keys, and can answer from both at once (shadow mode) so you can compare before committing.
- **Every process must agree.** Placement is derived only from node ids, so independently built rings map keys identically — verified by a test that builds two rings separately and compares 10,000 keys.

## Quick start

```rust
use shardring::{CompactRing, DefaultHash, Node};

let ring = CompactRing::<DefaultHash>::builder()
    .add_nodes(vec![
        Node::new("node-a", 100),
        Node::new("node-b", 100),
        Node::new("node-c", 200), // twice the share
    ])
    .build()?;

let node = ring.get(b"session:12345")?;   // owner of the key
let replicas = ring.get_n(b"session:12345", 3)?; // owner + next distinct nodes
# Ok::<(), shardring::ConsistentHashError>(())
```

## What's in it

| Module | What it does |
|---|---|
| `ring` | `CompactRing`: ring with virtual nodes, 6 bytes per point (`u32` hash + `u16` node index) |
| `bounded` | `BoundedRing`: load-aware routing with a per-node cap and overflow |
| `migration` | `DualRing`: hash-based, percentage and datacenter rollout, plus shadow mode |
| `maglev` | `MaglevTable`: Google's Maglev — faster lookups, more memory, fixed table |
| `jump` | `jump_consistent_hash`: O(1), no allocation, but buckets must be `0..n` |
| `adaptive` | Picks the number of virtual nodes per node from a target coefficient of variation |
| `health` | `HealthAwareRing`: skips unhealthy nodes (async, Tokio) |

### Picking an algorithm

| You need | Use |
|---|---|
| Weighted nodes, nodes come and go by name | `CompactRing` |
| The above, plus protection against a hot node | `BoundedRing` |
| Fixed bucket count, minimum memory | `jump` |
| Highest lookup throughput, memory is cheap | `MaglevTable` |

## Bounded loads

```rust
use shardring::{BoundedRing, CompactRing, DefaultHash, Node};

let ring = CompactRing::<DefaultHash>::builder()
    .add_nodes(vec![Node::new("a", 1), Node::new("b", 1), Node::new("c", 1)])
    .build()?;

let bounded = BoundedRing::builder(ring)
    .max_load_factor(1.25) // never more than 1.25x the mean load
    .build()?;

let node = bounded.get(b"key")?.id.clone();
bounded.acquire(&node);  // count the in-flight work
// ... serve the request ...
bounded.release(&node);
# Ok::<(), shardring::ConsistentHashError>(())
```

The cap is what keeps one node from absorbing a burst while still moving only about `1/N` of the keys when the node set changes.

## Migration between ring configurations

```rust
use shardring::{DualRing, MigrationStrategy};

let mut dual = DualRing::builder(old_ring, new_ring)
    .strategy(MigrationStrategy::HashBased)
    .rollout_percentage(0.10) // start with 10% of keys on the new ring
    .build()?;

let (old_node, new_node) = dual.get_shadow(b"key")?; // compare without switching
dual.set_rollout(0.50);                              // then move more
# Ok::<(), shardring::ConsistentHashError>(())
```

A key does not flip between rings while the rollout is unchanged, so a session stays where it is until you decide otherwise.

## Serialization

`to_bytes()` produces a self-contained snapshot — version, nodes and ring points — and `from_bytes()` validates the version, the length and every node index before returning a ring. A snapshot taken on one process rebuilds a ring that routes keys identically on another.

## WebAssembly

The crate builds for `wasm32-unknown-unknown`, so the same placement logic can run in the browser or in an edge runtime:

```bash
wasm-pack build --target web --features "std,adaptive-ring,migration,serialization"
```

## Features

| Feature | Default | What it enables |
|---|---|---|
| `std` | ✅ | Standard library (required by `bounded` and `health`) |
| `adaptive-ring` | ✅ | Virtual node count derived from a target CV |
| `migration` | | `DualRing` |
| `serialization` | | Postcard helpers |
| `health` | | `HealthAwareRing` (pulls in Tokio) |
| `serde` | | JSON metadata on nodes |

## Performance

Criterion benchmarks live in `shardring-bench`:

```bash
cargo bench -p shardring-bench
```

**No performance or memory numbers are published here on purpose.** Earlier drafts of this README carried figures that turned out to come from a bug in how virtual nodes were counted. Numbers will be added when they come from a reproducible benchmark on a known machine.

What is measured today, by tests rather than benchmarks: rings built independently from the same nodes agree on 10,000 keys; the distribution stays under a 0.15 coefficient of variation for 10, 50 and 200 nodes; adding one node to a ring of ten moves under 20% of keys.

## Testing

```bash
cargo test --all-features                       # unit + property tests
cd shardring/fuzz && cargo +nightly fuzz run fuzz_ring    # ring, migration and bounded targets
```

## What this is not

- Not a datastore. It decides *where* something belongs; moving or replicating data is yours.
- Not a cluster. There is no membership, gossip or consensus here.
- Not `no_std`. The crate uses `alloc` types throughout; the `std` feature flag does not currently make it `no_std`-compatible.

## License

MIT OR Apache-2.0, at your option.
