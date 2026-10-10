//! Run ids: UUIDv7 (RFC 9562), so ids sort by the time they were made, and the short
//! handles people type.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The fewest hex characters a handle has.
const MIN_HANDLE: usize = 4;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The handle of `id` among `ids`: the shortest suffix of its hex digits, at least
/// [`MIN_HANDLE`] long, that no other id ends with. An id's last 15 hex digits are random
/// (its first 12 are the time), so handles of runs made close together still differ.
pub fn handle(id: &str, ids: &[String]) -> String {
    let digits = hex_digits(id);
    let others: Vec<String> = ids
        .iter()
        .filter(|other| other.as_str() != id)
        .map(|other| hex_digits(other))
        .collect();
    for len in MIN_HANDLE..=digits.len() {
        let suffix = &digits[digits.len() - len..];
        if !others.iter().any(|other| other.ends_with(suffix)) {
            return suffix.to_string();
        }
    }
    digits
}

/// The hex digits of an id, without its dashes, in lowercase.
pub fn hex_digits(id: &str) -> String {
    id.chars()
        .filter(|c| *c != '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The time an id was made, in milliseconds since the Unix epoch.
pub fn millis(id: &str) -> u64 {
    u64::from_str_radix(hex_digits(id).get(..12).unwrap_or("0"), 16).unwrap_or(0)
}

/// A new id for a run made now.
pub fn new() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    with_time(millis, random())
}

/// 128 random bits from the process's randomly keyed hasher, never the same twice in one
/// process.
fn random() -> u128 {
    let mut out = 0u128;
    for half in 0..2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u32(std::process::id());
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        out |= (hasher.finish() as u128) << (64 * half);
    }
    out
}

fn with_time(millis: u64, random: u128) -> String {
    let mut bits = random & ((1u128 << 80) - 1);
    bits |= ((millis as u128) & ((1u128 << 48) - 1)) << 80;
    // Version 7 in bits 76..80, variant 0b10 in bits 62..64.
    bits = (bits & !(0xf << 76)) | (0x7 << 76);
    bits = (bits & !(0x3 << 62)) | (0x2 << 62);
    let hex = format!("{:032x}", bits);
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handles_are_the_shortest_unique_suffix() {
        let ids: Vec<String> = [
            "0192aaaa-0000-7000-8000-00000000abcd",
            "0192aaaa-0000-7000-8000-0000000fabcd",
            "0192aaaa-0000-7000-8000-000000001234",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(handle(&ids[0], &ids), "0abcd");
        assert_eq!(handle(&ids[1], &ids), "fabcd");
        assert_eq!(handle(&ids[2], &ids), "1234");
    }

    #[test]
    fn test_ids_are_version_7_and_sort_by_time() {
        let early = with_time(1_700_000_000_000, u128::MAX);
        let late = with_time(1_700_000_000_001, 0);
        assert!(early < late);
        assert_eq!(early.len(), 36);
        assert_eq!(&early[14..15], "7");
        assert!(matches!(&early[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(millis(&early), 1_700_000_000_000);
        assert_ne!(new(), new());
    }
}
