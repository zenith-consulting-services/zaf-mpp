//! Ported from MPXJ: src/main/java/org/mpxj/mpp/VarMeta12.java and
//! src/main/java/org/mpxj/mpp/AbstractVarMeta.java
//! Copyright (c) Packwood Software 2002-2005, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! `VarMeta` describes the layout of a companion `Var2Data` block: for each
//! entity unique ID it records, per field type, the byte offset within the
//! `Var2Data` stream where that field's value lives.

use crate::error::{corrupt, MppResult};
use crate::util::get_int;

const MAGIC: i32 = 0xFADFADBAu32 as i32;

/// Parsed `VarMeta` stream (MPP14 uses the "VarMeta12" layout throughout).
pub struct VarMeta {
    /// `(unique_id, field_type, offset)`, sorted by `(unique_id,
    /// field_type)` with one entry per pair. Where the stream repeats a
    /// pair, the later entry wins, as it did with MPXJ's map insert.
    entries: Vec<(i32, i32, i32)>,
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
        let available = (data.len() - 24) / 12;
        let mut entries = Vec::with_capacity(item_count.min(available));

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
            entries.push((unique_id, ty, offset));
        }

        crate::util::sort_dedup_keep_last(&mut entries, |&(id, ty, _)| (id, ty));

        Ok(VarMeta { entries })
    }

    /// Every `(unique_id, field_type, offset)` entry for one unique ID,
    /// sorted by field type.
    pub(crate) fn entity(&self, unique_id: i32) -> &[(i32, i32, i32)] {
        let start = self.entries.partition_point(|e| e.0 < unique_id);
        let len = self.entries[start..].partition_point(|e| e.0 == unique_id);
        &self.entries[start..start + len]
    }

    /// Byte offset within the companion `Var2Data` block for the given
    /// entity unique ID and field type, if present.
    pub fn offset(&self, unique_id: i32, field_type: i32) -> Option<i32> {
        self.entries
            .binary_search_by(|e| (e.0, e.1).cmp(&(unique_id, field_type)))
            .ok()
            .map(|i| self.entries[i].2)
    }

    /// All field type keys recorded for the given entity unique ID.
    pub fn types(&self, unique_id: i32) -> Vec<i32> {
        self.entity(unique_id).iter().map(|e| e.1).collect()
    }

    /// The set of entity unique IDs described by this block.
    pub fn unique_ids(&self) -> Vec<i32> {
        let mut ids: Vec<i32> = self.entries.iter().map(|e| e.0).collect();
        ids.dedup();
        ids
    }

    /// True if the given unique ID has any entries in this block.
    pub fn contains_unique_id(&self, unique_id: i32) -> bool {
        !self.entity(unique_id).is_empty()
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
        assert_eq!(vm.types(42), vec![14, 16]);
        assert!(vm.types(999).is_empty());
    }

    #[test]
    fn repeated_id_and_type_keeps_the_later_offset() {
        let mut data = sample_bytes();
        data.extend_from_slice(&le32(42));
        data.extend_from_slice(&le32(200));
        data.extend_from_slice(&le16(14));
        data.extend_from_slice(&le16(0));
        data[8..12].copy_from_slice(&le32(2));

        let vm = VarMeta::parse(&data).unwrap();
        assert_eq!(vm.offset(42, 14), Some(200));
        assert_eq!(vm.types(42), vec![14]);
    }

    #[test]
    fn contains_unique_id_reflects_presence() {
        let vm = VarMeta::parse(&sample_bytes()).unwrap();
        assert!(vm.contains_unique_id(42));
        assert!(!vm.contains_unique_id(999));
    }
}
