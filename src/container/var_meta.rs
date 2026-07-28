//! Ported from MPXJ: src/main/java/org/mpxj/mpp/VarMeta12.java and
//! src/main/java/org/mpxj/mpp/AbstractVarMeta.java
//! Copyright (c) Packwood Software 2002-2005, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! `VarMeta` describes the layout of a companion `Var2Data` block: for each
//! entity unique ID it records, per field type, the byte offset within the
//! `Var2Data` stream where that field's value lives.

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::error::{corrupt, MppResult};
use crate::util::get_int;

const MAGIC: i32 = 0xFADFADBAu32 as i32;

/// Parsed `VarMeta` stream (MPP14 uses the "VarMeta12" layout throughout).
pub struct VarMeta {
    table: HashMap<i32, BTreeMap<i32, i32>>,
    offsets: Vec<i32>,
}

impl VarMeta {
    /// Parse a `VarMeta` stream. Mirrors `VarMeta12(InputStream)`.
    pub fn parse(data: &[u8]) -> MppResult<VarMeta> {
        if data.len() < 24 {
            return Err(corrupt("VarMeta block shorter than its 24 byte header"));
        }
        let magic = get_int(data, 0);
        // MPXJ tolerates a zero magic number: some otherwise valid files use it.
        if magic != 0 && magic != MAGIC {
            return Err(corrupt(format!("bad VarMeta magic number: {magic:#x}")));
        }

        let item_count = get_int(data, 8) as usize;
        let mut table: HashMap<i32, BTreeMap<i32, i32>> = HashMap::new();
        let mut offsets = Vec::with_capacity(item_count);

        let mut pos = 24usize;
        for _ in 0..item_count {
            if pos + 12 > data.len() {
                break;
            }
            let unique_id = get_int(data, pos);
            let offset = get_int(data, pos + 4);
            let ty = crate::util::get_short(data, pos + 8);
            // 2 bytes unknown at pos + 10, skipped.
            pos += 12;

            table.entry(unique_id).or_default().insert(ty, offset);
            offsets.push(offset);
        }

        offsets.sort_unstable();

        Ok(VarMeta { table, offsets })
    }

    /// Byte offset within the companion `Var2Data` block for the given
    /// entity unique ID and field type, if present.
    pub fn offset(&self, unique_id: i32, field_type: i32) -> Option<i32> {
        self.table
            .get(&unique_id)
            .and_then(|m| m.get(&field_type))
            .copied()
    }

    /// All field type keys recorded for the given entity unique ID.
    pub fn types(&self, unique_id: i32) -> Vec<i32> {
        self.table
            .get(&unique_id)
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default()
    }

    /// The set of entity unique IDs described by this block.
    pub fn unique_ids(&self) -> Vec<i32> {
        let mut ids: Vec<i32> = self.table.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    /// True if the given unique ID has any entries in this block.
    pub fn contains_unique_id(&self, unique_id: i32) -> bool {
        self.table.contains_key(&unique_id)
    }

    /// Sorted list of data offsets referenced by this block. Not
    /// deduplicated: two different (unique ID, type) entries can
    /// legitimately point at the same offset (MS Project deduplicates
    /// identical values when writing), so the same offset may appear more
    /// than once here — harmless downstream, since `Var2Data::parse`
    /// already skips an offset it's seen before.
    pub(crate) fn offsets(&self) -> &[i32] {
        &self.offsets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(v: i32) -> [u8; 4] {
        v.to_le_bytes()
    }
    fn le16(v: i16) -> [u8; 2] {
        v.to_le_bytes()
    }

    fn sample_bytes() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&le32(MAGIC)); // magic
        data.extend_from_slice(&le32(0)); // unknown1
        data.extend_from_slice(&le32(1)); // item count
        data.extend_from_slice(&le32(0)); // unknown2
        data.extend_from_slice(&le32(0)); // unknown3
        data.extend_from_slice(&le32(0)); // data size
        data.extend_from_slice(&le32(42)); // unique id
        data.extend_from_slice(&le32(100)); // offset
        data.extend_from_slice(&le16(14)); // type (e.g. NAME)
        data.extend_from_slice(&le16(0)); // unknown
        data
    }

    #[test]
    fn parses_single_entry() {
        let vm = VarMeta::parse(&sample_bytes()).unwrap();
        assert_eq!(vm.offset(42, 14), Some(100));
        assert_eq!(vm.offset(42, 99), None);
        assert_eq!(vm.unique_ids(), vec![42]);
    }

    #[test]
    fn bad_magic_is_error() {
        let mut data = sample_bytes();
        data[0] = 0x01;
        assert!(VarMeta::parse(&data).is_err());
    }

    #[test]
    fn zero_magic_is_tolerated() {
        let mut data = sample_bytes();
        data[0..4].copy_from_slice(&le32(0));
        assert!(VarMeta::parse(&data).is_ok());
    }

    #[test]
    fn header_shorter_than_24_bytes_is_error() {
        assert!(VarMeta::parse(&[0u8; 20]).is_err());
    }

    #[test]
    fn truncated_entry_list_stops_cleanly() {
        // Header claims 2 entries but only one full 12 byte entry follows.
        let mut data = Vec::new();
        data.extend_from_slice(&le32(MAGIC));
        data.extend_from_slice(&le32(0));
        data.extend_from_slice(&le32(2));
        data.extend_from_slice(&le32(0));
        data.extend_from_slice(&le32(0));
        data.extend_from_slice(&le32(0));
        data.extend_from_slice(&le32(42));
        data.extend_from_slice(&le32(100));
        data.extend_from_slice(&le16(14));
        data.extend_from_slice(&le16(0));

        let vm = VarMeta::parse(&data).unwrap();
        assert_eq!(vm.unique_ids(), vec![42]);
    }

    #[test]
    fn types_lists_every_field_type_for_a_unique_id() {
        let mut data = sample_bytes();
        // Append a second entry for the same unique ID with a different type.
        data.extend_from_slice(&le32(42));
        data.extend_from_slice(&le32(120));
        data.extend_from_slice(&le16(16));
        data.extend_from_slice(&le16(0));
        // Bump the item count to 2 so both entries are read.
        data[8..12].copy_from_slice(&le32(2));

        let vm = VarMeta::parse(&data).unwrap();
        let mut types = vm.types(42);
        types.sort_unstable();
        assert_eq!(types, vec![14, 16]);
        assert!(vm.types(999).is_empty());
    }

    #[test]
    fn contains_unique_id_reflects_presence() {
        let vm = VarMeta::parse(&sample_bytes()).unwrap();
        assert!(vm.contains_unique_id(42));
        assert!(!vm.contains_unique_id(999));
    }
}
