# Changelog

All notable changes to this project are documented here, following
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `CompactRing`: consistent hashing ring with weighted virtual nodes and a
  6-byte point layout (`u32` hash + `u16` node index).
- `BoundedRing`: per-node load cap with overflow, following *Consistent
  Hashing with Bounded Loads* (Mirrokni, Thorup, Zadimoghaddam, 2016).
- `DualRing`: migration between ring configurations with hash-based,
  percentage and datacenter rollout, plus shadow mode.
- `MaglevTable` and `jump_consistent_hash` as alternative placement strategies.
- `HealthAwareRing` behind the `health` feature.
- Self-contained serialization: snapshots carry the format version and the
  node set, and are validated on read.
