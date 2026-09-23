use super::{ConsistentHashError, Result};

pub fn jump_consistent_hash(key: u64, num_buckets: u32) -> Result<u32> {
    if num_buckets == 0 {
        return Err(ConsistentHashError::EmptyRing);
    }

    let mut b: i64 = -1;
    let mut j: i64 = 0;
    let mut key = key;

    while j < num_buckets as i64 {
        b = j;
        key = key.wrapping_mul(2862933555777941757).wrapping_add(1);
        j = ((b + 1) as f64 * ((1u64 << 31) as f64 / ((key >> 33) + 1) as f64)).floor() as i64;
    }

    Ok(b as u32)
}

pub fn jump_consistent_hash_u64(key: u64, num_buckets: u64) -> Result<u64> {
    if num_buckets == 0 {
        return Err(ConsistentHashError::EmptyRing);
    }

    let mut b: i64 = -1;
    let mut j: i64 = 0;
    let mut key = key;

    while j < num_buckets as i64 {
        b = j;
        key = key.wrapping_mul(2862933555777941757).wrapping_add(1);
        j = ((b + 1) as f64 * ((1u64 << 31) as f64 / ((key >> 33) + 1) as f64)).floor() as i64;
    }

    Ok(b as u64)
}

pub struct JumpHash;

impl JumpHash {
    pub fn hash(key: u64, num_buckets: u32) -> Result<u32> {
        jump_consistent_hash(key, num_buckets)
    }

    pub fn hash_u64(key: u64, num_buckets: u64) -> Result<u64> {
        jump_consistent_hash_u64(key, num_buckets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jump_basic() {
        let result = jump_consistent_hash(12345, 10).unwrap();
        assert!(result < 10);
    }

    #[test]
    fn test_jump_deterministic() {
        for _ in 0..100 {
            let r1 = jump_consistent_hash(42, 100).unwrap();
            let r2 = jump_consistent_hash(42, 100).unwrap();
            assert_eq!(r1, r2);
        }
    }

    #[test]
    fn test_jump_distribution() {
        let mut counts = vec![0u32; 10];
        for i in 0..10000 {
            let bucket = jump_consistent_hash(i, 10).unwrap();
            counts[bucket as usize] += 1;
        }

        for count in counts {
            assert!(
                count > 800 && count < 1200,
                "Bucket count {} out of expected range",
                count
            );
        }
    }

    #[test]
    fn test_jump_monotonic() {
        let prev = jump_consistent_hash(12345, 10).unwrap();
        let next = jump_consistent_hash(12345, 11).unwrap();
        assert!(next >= prev || next == 10);
    }

    #[test]
    fn test_jump_minimal_remapping() {
        let total_keys = 10000;
        let mut remapped = 0;

        for i in 0..total_keys {
            let b10 = jump_consistent_hash(i, 10).unwrap();
            let b11 = jump_consistent_hash(i, 11).unwrap();

            if b10 != b11 && b11 != 10 {
                remapped += 1;
            }
        }

        let pct = remapped as f64 / total_keys as f64 * 100.0;
        assert!(pct < 15.0, "Remapped {:.1}%, expected <10%", pct);
    }

    #[test]
    fn test_jump_u64() {
        let result = jump_consistent_hash_u64(12345, 1000).unwrap();
        assert!(result < 1000);
    }

    #[test]
    fn test_jump_empty() {
        assert!(jump_consistent_hash(12345, 0).is_err());
    }
}
