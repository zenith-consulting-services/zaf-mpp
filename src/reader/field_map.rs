//! Ported from MPXJ: src/main/java/org/mpxj/mpp/FieldMap.java
//! (`createFieldMap`, `createTaskFieldMap`, `createResourceFieldMap`,
//! `createAssignmentFieldMap`) and src/main/java/org/mpxj/common/
//! MPPTaskField.java / MPPResourceField.java / MPPAssignmentField.java.
//! Copyright (c) Packwood Software 2011, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Real MS-Project-authored MPP14 files store their own field-to-offset
//! layout in the project `Props` block (one of `TASK_FIELD_MAP`,
//! `RESOURCE_FIELD_MAP`, `ASSIGNMENT_FIELD_MAP` and their `*_MAP2`
//! fallbacks): a sequence of 28 byte records, each naming a field by a
//! stable legacy integer ID (the same numbering MPX used, and the same
//! numbers used as the fifth constructor argument throughout
//! `FieldMap14`'s hardcoded tables) and giving its actual byte offset or
//! var-data key *in this file*.
//!
//! Every MPP14 file MS Project itself writes carries this block, and its
//! offsets often differ from `FieldMap14`'s hardcoded defaults (Project
//! 2013 and later move `WORK` from fixed-data offset 126 to offset 8, for
//! example: see `mpp14task-from2013.mpp` in the test corpus). `reader::mpp14`
//! looks every field up through a `FieldMap` built from this data, falling
//! back to the hardcoded default location only when the on-disk map is
//! absent or doesn't mention that field, which happens for the small
//! synthetic fixtures the property-based corruption tests build by hand
//! rather than for any real MPP14 file.

use std::collections::HashMap;

use crate::container::props::Props;
use crate::util::{get_int, get_short};

#[derive(Clone, Copy, Debug)]
enum FieldLoc {
    Fixed {
        block: usize,
        offset: usize,
    },
    Var {
        key: i32,
    },
    /// Boolean flags: parsed (to consume the entry and keep `block_index`
    /// tracking correct for later entries) but never queried. MPXJ's own
    /// `FieldMap.FieldItem.read` does the same and falls back to a
    /// hardcoded table for these instead, noting in a comment that nobody
    /// has worked out how to derive a byte/bit location from this data.
    #[allow(dead_code)]
    Meta {
        block: usize,
        mask: i32,
    },
}

/// A field map read from one project's `Props` block, keyed by the legacy
/// field ID MPXJ has used since the original MPX format.
pub struct FieldMap {
    entries: HashMap<i32, FieldLoc>,
}

impl FieldMap {
    /// Look up the on-disk map under `keys` in order (a primary key and,
    /// for some field classes, a secondary fallback key); fall back to an
    /// empty map, under which every lookup returns its caller-supplied
    /// default, if neither is present.
    pub fn from_props(props: &Props, keys: &[i32]) -> FieldMap {
        for &key in keys {
            if let Some(data) = props.byte_array(key) {
                return FieldMap {
                    entries: Self::parse(data),
                };
            }
        }
        FieldMap {
            entries: HashMap::new(),
        }
    }

    /// Ported from `FieldMap.createFieldMap`.
    fn parse(data: &[u8]) -> HashMap<i32, FieldLoc> {
        let mut entries = HashMap::new();
        let mut last_offset: i32 = 0;
        let mut block_index: usize = 0;

        let count = data.len() / 28;
        for i in 0..count {
            let base = i * 28;
            let mask = get_int(data, base);
            let data_block_offset = get_short(data, base + 4);
            let type_value = get_int(data, base + 12);
            let category = get_short(data, base + 20);
            // The upper bits of type_value are a field-class tag (e.g.
            // TASK_FIELD_BASE); the low 16 bits are the stable legacy ID
            // also used as the fifth argument throughout FieldMap14's
            // hardcoded tables.
            let legacy_id = type_value & 0xFFFF;

            let loc = match category {
                0x0B => Some(FieldLoc::Meta { block: 0, mask }),
                0x64 => Some(FieldLoc::Meta { block: 1, mask }),
                _ => {
                    if data_block_offset != -1 {
                        if data_block_offset < last_offset {
                            block_index += 1;
                        }
                        last_offset = data_block_offset;
                        Some(FieldLoc::Fixed {
                            block: block_index,
                            offset: data_block_offset as usize,
                        })
                    } else if legacy_id != 0 {
                        Some(FieldLoc::Var { key: legacy_id })
                    } else {
                        None
                    }
                }
            };

            if let Some(loc) = loc {
                entries.insert(legacy_id, loc);
            }
        }

        entries
    }

    /// Fixed-data location for `legacy_id`, or `(default_block,
    /// default_offset)` if the on-disk map is absent or has no entry (or a
    /// non-fixed entry) for this field.
    pub fn fixed(
        &self,
        legacy_id: i32,
        default_block: usize,
        default_offset: usize,
    ) -> (usize, usize) {
        match self.entries.get(&legacy_id) {
            Some(FieldLoc::Fixed { block, offset }) => (*block, *offset),
            _ => (default_block, default_offset),
        }
    }

    /// Var-data type key for `legacy_id`, or `default_key` if the on-disk
    /// map is absent or has no entry for this field. In practice the
    /// var-data key rarely differs from the legacy ID itself, but this
    /// still goes through the map for consistency and to honour an
    /// explicit override when one is present.
    pub fn var(&self, legacy_id: i32, default_key: i32) -> i32 {
        match self.entries.get(&legacy_id) {
            Some(FieldLoc::Var { key }) => *key,
            _ => default_key,
        }
    }
}

// PropsKey values, from src/main/java/org/mpxj/mpp/PropsKey.java, used to
// locate the on-disk field maps.
pub const TASK_FIELD_MAP: i32 = 131092;
pub const TASK_FIELD_MAP2: i32 = 50331668;
pub const RESOURCE_FIELD_MAP: i32 = 131093;
pub const RESOURCE_FIELD_MAP2: i32 = 50331669;
pub const ASSIGNMENT_FIELD_MAP: i32 = 131095;
pub const ASSIGNMENT_FIELD_MAP2: i32 = 50331671;

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(mask: i32, data_block_offset: i16, type_value: i32, category: i16) -> [u8; 28] {
        let mut e = [0u8; 28];
        e[0..4].copy_from_slice(&mask.to_le_bytes());
        e[4..6].copy_from_slice(&data_block_offset.to_le_bytes());
        e[12..16].copy_from_slice(&type_value.to_le_bytes());
        e[20..22].copy_from_slice(&category.to_le_bytes());
        e
    }

    #[test]
    fn empty_map_falls_back_to_defaults() {
        let map = FieldMap {
            entries: HashMap::new(),
        };
        assert_eq!(map.fixed(14, 0, 65535), (0, 65535));
        assert_eq!(map.var(14, 999), 999);
    }

    /// Build a `Props` block with a single entry: `key` mapped to `value`.
    /// Mirrors the layout `container::props::Props::parse` expects.
    fn build_props(key: i32, value: &[u8]) -> Props {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&1i16.to_le_bytes()); // item count
        data.extend_from_slice(&[0u8; 2]);
        data.extend_from_slice(&(value.len() as i32).to_le_bytes());
        data.extend_from_slice(&key.to_le_bytes());
        data.extend_from_slice(&0i32.to_le_bytes());
        data.extend_from_slice(value);
        Props::parse(&data)
    }

    #[test]
    fn from_props_falls_back_to_empty_when_no_key_present() {
        let props = build_props(999, &[1, 2, 3]);
        let map = FieldMap::from_props(&props, &[TASK_FIELD_MAP, TASK_FIELD_MAP2]);
        assert_eq!(map.fixed(0, 1, 2), (1, 2));
    }

    #[test]
    fn from_props_uses_first_matching_key() {
        // WORK (legacy id 0): fixed, block 0, offset 8.
        let blob = entry(0, 8, 0, 0x65);
        let props = build_props(TASK_FIELD_MAP, &blob);
        let map = FieldMap::from_props(&props, &[TASK_FIELD_MAP, TASK_FIELD_MAP2]);
        assert_eq!(map.fixed(0, 99, 99), (0, 8));
    }

    #[test]
    fn from_props_falls_through_to_secondary_key() {
        let blob = entry(0, 8, 0, 0x65);
        let props = build_props(TASK_FIELD_MAP2, &blob);
        let map = FieldMap::from_props(&props, &[TASK_FIELD_MAP, TASK_FIELD_MAP2]);
        assert_eq!(map.fixed(0, 99, 99), (0, 8));
    }

    #[test]
    fn entry_with_no_fixed_offset_and_zero_legacy_id_is_unresolvable() {
        // category not META, no fixed offset (-1), and a type_value that
        // masks to legacy id 0: MPXJ itself can't locate such a field, so
        // it's simply dropped rather than recorded.
        let data = entry(0, -1, 0, 0);
        let entries = FieldMap::parse(&data);
        let map = FieldMap { entries };
        assert_eq!(map.fixed(0, 1, 2), (1, 2));
        assert_eq!(map.var(0, 42), 42);
    }

    #[test]
    fn parses_fixed_and_var_entries() {
        let mut data = Vec::new();
        // WORK (legacy id 0): fixed, block 0, offset 8.
        data.extend_from_slice(&entry(0, 8, 0, 0x65));
        // NAME (legacy id 14): var data, key 14.
        data.extend_from_slice(&entry(0, -1, 14, 0x08));

        let entries = FieldMap::parse(&data);
        let map = FieldMap { entries };
        assert_eq!(map.fixed(0, 99, 99), (0, 8));
        assert_eq!(map.var(14, 0), 14);
    }

    #[test]
    fn second_data_block_detected_on_offset_wraparound() {
        let mut data = Vec::new();
        data.extend_from_slice(&entry(0, 50, 100, 0));
        // Offset drops back down: this belongs to a second fixed block.
        data.extend_from_slice(&entry(0, 10, 101, 0));

        let entries = FieldMap::parse(&data);
        let map = FieldMap { entries };
        assert_eq!(map.fixed(100, 0, 0), (0, 50));
        assert_eq!(map.fixed(101, 0, 0), (1, 10));
    }
}
