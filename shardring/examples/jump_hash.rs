use shardring::{jump_consistent_hash, jump_consistent_hash_u64, JumpHash};

fn main() {
    println!("=== Jump Consistent Hash ===");

    // Basic usage
    let num_buckets = 10;
    for key in [1, 2, 3, 100, 1000, 12345] {
        let bucket = jump_consistent_hash(key, num_buckets).unwrap();
        println!("Key {} -> Bucket {}", key, bucket);
    }

    // Deterministic
    println!("\n=== Deterministic ===");
    for _ in 0..5 {
        let bucket = jump_consistent_hash(42, 100).unwrap();
        println!("Key 42 -> Bucket {}", bucket);
    }

    // Distribution test
    println!("\n=== Distribution (10000 keys, 10 buckets) ===");
    let mut counts = [0; 10];
    for i in 0..10000 {
        let bucket = jump_consistent_hash(i as u64, 10).unwrap();
        counts[bucket as usize] += 1;
    }
    for (i, count) in counts.iter().enumerate() {
        println!("  Bucket {}: {} ({:.1}%)", i, count, *count as f64 / 100.0);
    }

    // Minimal remapping when adding bucket
    println!("\n=== Minimal Remapping (10 -> 11 buckets) ===");
    let mut remapped = 0;
    for i in 0..10000 {
        let b10 = jump_consistent_hash(i as u64, 10).unwrap();
        let b11 = jump_consistent_hash(i as u64, 11).unwrap();
        if b10 != b11 && b11 != 10 {
            remapped += 1;
        }
    }
    println!("Remapped: {} ({:.1}%)", remapped, remapped as f64 / 100.0);

    // U64 version
    println!("\n=== U64 Version ===");
    let bucket = jump_consistent_hash_u64(12345, 1000).unwrap();
    println!("Key 12345 -> Bucket {} (out of 1000)", bucket);

    // Using JumpHash struct
    println!("\n=== JumpHash Struct ===");
    let bucket = JumpHash::hash(12345, 10).unwrap();
    println!("JumpHash::hash(12345, 10) = {}", bucket);
}
