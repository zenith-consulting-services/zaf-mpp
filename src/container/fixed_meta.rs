//! Ported from MPXJ: src/main/java/org/mpxj/mpp/FixedMeta.java
//! Copyright (c) Packwood Software 2002-2003, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! `FixedMeta` describes the layout of a companion `FixedData` block: one
//! fixed-size record per entity, whose meaning MPXJ has only partially
//! reverse engineered. The important part for reading MPP14 is byte 0
//! (deleted flag) and the 4 byte item offset at byte 4, both consumed by
//! `FixedData`.

use crate::error::{corrupt, MppResult};
use crate::util::get_int;

const MAGIC: i32 = 0xFADFADBAu32 as i32;
const HEADER_SIZE: usize = 16;

/// Parsed `FixedMeta` stream: one fixed-size record per entity in the
/// companion `FixedData` block.
pub struct FixedMeta {
    items: Vec<Vec<u8>>,
    /// Item count as reported in the block header, kept for parity with
    /// MPXJ's `getItemCount` (distinct from `items.len()`, the adjusted count).
    pub reported_item_count: i32,
}

impl FixedMeta {
    /// Parse with a known, fixed item size. Mirrors `FixedMeta(InputStream,
    /// int)`.
    pub fn parse(data: &[u8], item_size: usize) -> MppResult<FixedMeta> {
        Self::parse_with_size_fn(data, |_file_size, _item_count| item_size)
    }

    /// Parse allowing several possible item sizes, picking the one that
    /// best matches the block size and, where possible, the item count of
    /// a related `FixedData` block. Mirrors `FixedMeta(InputStream,
    /// FixedData, int...)`.
    pub fn parse_with_candidates(
        data: &[u8],
        other_block_item_count: usize,
        item_sizes: &[usize],
    ) -> MppResult<FixedMeta> {
        let item_sizes = item_sizes.to_vec();
        Self::parse_with_size_fn(data, move |file_size, item_count| {
            let available = file_size.saturating_sub(HEADER_SIZE);
            let mut chosen = item_sizes[0];
            let mut best_distance = i64::MIN;

            for &candidate in &item_sizes {
                if candidate == 0 || available % candidate != 0 {
                    continue;
                }
                if available / candidate == other_block_item_count {
                    return candidate;
                }
                let test_distance = (item_count as i64 * candidate as i64) - available as i64;
                if test_distance <= 0 && test_distance > best_distance {
                    chosen = candidate;
                    best_distance = test_distance;
                }
            }
            chosen
        })
    }

    fn parse_with_size_fn(
        data: &[u8],
        item_size_fn: impl Fn(usize, usize) -> usize,
    ) -> MppResult<FixedMeta> {
        if data.len() < HEADER_SIZE {
            return Err(corrupt("FixedMeta block shorter than its 16 byte header"));
        }
        let magic = get_int(data, 0);
        if magic != MAGIC {
            return Err(corrupt(format!("bad FixedMeta magic number: {magic:#x}")));
        }
        let reported_item_count = get_int(data, 8);

        let file_size = data.len();
        let item_size = item_size_fn(file_size, reported_item_count.max(0) as usize);
        if item_size == 0 {
            return Err(corrupt("FixedMeta item size resolved to zero"));
        }

        let adjusted_item_count = (file_size - HEADER_SIZE) / item_size;
        let mut items = Vec::with_capacity(adjusted_item_count);
        let mut pos = HEADER_SIZE;
        for _ in 0..adjusted_item_count {
            items.push(data[pos..pos + item_size].to_vec());
            pos += item_size;
        }

        Ok(FixedMeta {
            items,
            reported_item_count,
        })
    }

    pub fn adjusted_item_count(&self) -> usize {
        self.items.len()
    }

    pub fn byte_array(&self, index: usize) -> Option<&[u8]> {
        self.items.get(index).map(|v| v.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(item_count: i32) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&MAGIC.to_le_bytes());
        h.extend_from_slice(&0i32.to_le_bytes());
        h.extend_from_slice(&item_count.to_le_bytes());
        h.extend_from_slice(&0i32.to_le_bytes());
        h
    }

    #[test]
    fn parses_fixed_item_size() {
        let mut data = header(2);
        data.extend_from_slice(&[1, 2, 3, 4]);
        data.extend_from_slice(&[5, 6, 7, 8]);
        let meta = FixedMeta::parse(&data, 4).unwrap();
        assert_eq!(meta.adjusted_item_count(), 2);
        assert_eq!(meta.byte_array(1), Some([5u8, 6, 7, 8].as_slice()));
    }

    #[test]
    fn bad_magic_is_error() {
        let mut data = header(1);
        data[0] = 0;
        assert!(FixedMeta::parse(&data, 4).is_err());
    }

    #[test]
    fn truncated_block_is_error_not_panic() {
        let data = vec![0u8; 4];
        assert!(FixedMeta::parse(&data, 4).is_err());
    }

    #[test]
    fn candidate_selection_falls_back_to_rule_of_thumb_distance() {
        // 15 bytes available: candidates 3 and 5 both divide it evenly (7
        // does not), but neither matches `other_block_item_count`, forcing
        // the "closest without going over" heuristic to pick between them.
        let mut data = header(2);
        data.extend_from_slice(&[0u8; 15]);
        let meta = FixedMeta::parse_with_candidates(&data, 99, &[3, 5, 7]).unwrap();
        // available/candidate: 15/3=5, 15/5=3; test_distance = item_count*candidate - available
        // favours the larger candidate (5) here, giving 3 items.
        assert_eq!(meta.adjusted_item_count(), 3);
    }

    #[test]
    fn candidate_selection_matching_other_block_count_wins_immediately() {
        let mut data = header(2);
        data.extend_from_slice(&[0u8; 15]);
        // available/candidate must equal other_block_item_count (5) for an
        // exact match: 15/3 = 5.
        let meta = FixedMeta::parse_with_candidates(&data, 5, &[3, 5, 7]).unwrap();
        assert_eq!(meta.adjusted_item_count(), 5);
    }

    #[test]
    fn zero_item_size_candidate_is_an_error() {
        let data = header(1);
        assert!(FixedMeta::parse_with_candidates(&data, 1, &[0]).is_err());
    }
}
