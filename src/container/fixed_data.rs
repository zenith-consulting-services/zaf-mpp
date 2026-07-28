//! Ported from MPXJ: src/main/java/org/mpxj/mpp/FixedData.java
//! Copyright (c) Packwood Software 2002-2003, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! `FixedData` holds one variable-size record per entity. Record
//! boundaries are derived from the item offsets recorded in the companion
//! `FixedMeta` block (byte offset 4 of each meta entry), since MS Project
//! does not store record sizes directly.

use std::collections::HashMap;

use crate::util::get_int;

use super::fixed_meta::FixedMeta;

/// Parsed `FixedData` stream: one variable-size record per entity, looked
/// up by index or by the byte offset recorded in a `FixedMeta` entry.
pub struct FixedData<'a> {
    items: Vec<Option<&'a [u8]>>,
    /// Maps each record's starting offset back to its index, so relation
    /// and constraint lookups (which address records by offset, not index)
    /// don't need a linear scan per lookup.
    offset_to_index: HashMap<i32, usize>,
}

impl<'a> FixedData<'a> {
    /// Parse using a companion `FixedMeta` block to locate each record.
    /// Mirrors `FixedData(FixedMeta, InputStream, int)`.
    pub fn parse_with_meta(
        meta: &FixedMeta,
        data: &'a [u8],
        max_expected_size: usize,
    ) -> FixedData<'a> {
        let item_count = meta.adjusted_item_count();
        let mut items = Vec::with_capacity(item_count);
        let mut offset_to_index = HashMap::with_capacity(item_count);

        for loop_idx in 0..item_count {
            let meta_data = meta.byte_array(loop_idx);
            let item_offset = match meta_data {
                Some(m) if m.len() >= 8 => get_int(m, 4),
                _ => {
                    items.push(None);
                    continue;
                }
            };

            if item_offset < 0 || item_offset as usize > data.len() {
                items.push(None);
                continue;
            }
            let item_offset = item_offset as usize;

            let mut item_size = if loop_idx + 1 == item_count {
                data.len() - item_offset
            } else {
                let next_offset = meta
                    .byte_array(loop_idx + 1)
                    .filter(|m| m.len() >= 8)
                    .map(|m| get_int(m, 4))
                    .unwrap_or(item_offset as i32);
                (next_offset - item_offset as i32).max(0) as usize
            };

            let available = data.len() - item_offset;
            if item_size > available {
                item_size = if max_expected_size == 0 {
                    available
                } else {
                    max_expected_size.min(available)
                };
            }
            if max_expected_size != 0 && item_size > max_expected_size {
                item_size = max_expected_size;
            }

            if item_size > 0 {
                items.push(Some(&data[item_offset..item_offset + item_size]));
                offset_to_index
                    .entry(item_offset as i32)
                    .or_insert(loop_idx);
            } else {
                items.push(None);
            }
        }

        FixedData {
            items,
            offset_to_index,
        }
    }

    /// Parse a block whose records all have the same, known size. Mirrors
    /// `FixedData(int, InputStream)`.
    pub fn parse_fixed_size(data: &'a [u8], item_size: usize) -> FixedData<'a> {
        if item_size == 0 {
            return FixedData {
                items: Vec::new(),
                offset_to_index: HashMap::new(),
            };
        }
        let item_count = data.len() / item_size;
        let mut items = Vec::with_capacity(item_count);
        let mut offset_to_index = HashMap::with_capacity(item_count);
        for loop_idx in 0..item_count {
            let offset = loop_idx * item_size;
            items.push(Some(&data[offset..offset + item_size]));
            offset_to_index.insert(offset as i32, loop_idx);
        }
        FixedData {
            items,
            offset_to_index,
        }
    }

    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    pub fn byte_array(&self, index: usize) -> Option<&'a [u8]> {
        self.items.get(index).copied().flatten()
    }

    /// Index of the item whose record starts at the given data offset, if
    /// any. Mirrors `getIndexFromOffset`.
    pub fn index_from_offset(&self, offset: i32) -> Option<usize> {
        self.offset_to_index.get(&offset).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixed_size_splits_into_chunks() {
        let data = [1u8, 2, 3, 4, 5, 6];
        let fd = FixedData::parse_fixed_size(&data, 2);
        assert_eq!(fd.item_count(), 3);
        assert_eq!(fd.byte_array(1), Some([3u8, 4].as_slice()));
    }

    #[test]
    fn parse_fixed_size_zero_item_size_is_empty_not_panic() {
        let data = [1u8, 2, 3];
        let fd = FixedData::parse_fixed_size(&data, 0);
        assert_eq!(fd.item_count(), 0);
    }

    #[test]
    fn index_from_offset_finds_each_record() {
        let data = [1u8, 2, 3, 4, 5, 6];
        let fd = FixedData::parse_fixed_size(&data, 2);
        assert_eq!(fd.index_from_offset(0), Some(0));
        assert_eq!(fd.index_from_offset(2), Some(1));
        assert_eq!(fd.index_from_offset(4), Some(2));
        assert_eq!(fd.index_from_offset(6), None);
    }

    /// Build a `FixedMeta` block whose entries encode `offsets[i]` at byte
    /// offset 4 of each `item_size`-byte record, matching the layout
    /// `FixedData::parse_with_meta` expects.
    fn build_fixed_meta(offsets: &[i32], item_size: usize) -> FixedMeta {
        let mut data = Vec::new();
        data.extend_from_slice(&(0xFADFADBAu32 as i32).to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(&(offsets.len() as i32).to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
        for &offset in offsets {
            let mut entry = vec![0u8; item_size];
            if item_size >= 8 {
                entry[4..8].copy_from_slice(&offset.to_le_bytes());
            }
            data.extend_from_slice(&entry);
        }
        FixedMeta::parse(&data, item_size).unwrap()
    }

    #[test]
    fn meta_entry_too_short_for_an_offset_is_none() {
        // item_size 4: too short to hold the offset field at bytes 4..8.
        let meta = build_fixed_meta(&[0], 4);
        let data = [0u8; 16];
        let fd = FixedData::parse_with_meta(&meta, &data, 0);
        assert_eq!(fd.byte_array(0), None);
    }

    #[test]
    fn negative_or_out_of_range_offset_is_none() {
        let meta = build_fixed_meta(&[-1, 1000], 8);
        let data = [0u8; 16];
        let fd = FixedData::parse_with_meta(&meta, &data, 0);
        assert_eq!(fd.byte_array(0), None);
        assert_eq!(fd.byte_array(1), None);
    }

    #[test]
    fn oversized_item_is_capped_to_available_data_with_no_max() {
        // Meta implies item 0 spans 100 bytes (offset 100 - offset 0), but
        // only 10 bytes actually exist: with no max_expected_size, the
        // item is capped to whatever's available.
        let meta = build_fixed_meta(&[0, 100], 8);
        let data = [0u8; 10];
        let fd = FixedData::parse_with_meta(&meta, &data, 0);
        assert_eq!(fd.byte_array(0).map(<[u8]>::len), Some(10));
    }

    #[test]
    fn oversized_item_is_capped_to_max_expected_size() {
        let meta = build_fixed_meta(&[0, 100], 8);
        let data = [0u8; 10];
        let fd = FixedData::parse_with_meta(&meta, &data, 5);
        assert_eq!(fd.byte_array(0).map(<[u8]>::len), Some(5));
    }

    #[test]
    fn zero_size_item_is_none() {
        // Two entries at the same offset: the computed size for the first
        // is next_offset - this_offset = 0.
        let meta = build_fixed_meta(&[0, 0], 8);
        let data = [0u8; 16];
        let fd = FixedData::parse_with_meta(&meta, &data, 0);
        assert_eq!(fd.byte_array(0), None);
    }
}
