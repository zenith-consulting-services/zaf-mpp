//! Ported from MPXJ: src/main/java/org/mpxj/mpp/Var2Data.java
//! Copyright (c) Packwood Software 2002-2003, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! `Var2Data` holds variable-length field values. Each value is a 4 byte
//! little-endian size prefix followed by that many bytes, and the byte
//! offset of each value is given by the companion `VarMeta` block.
//!
//! MPXJ reads this from a `Stream` and has to cope with offsets that are
//! out of order or repeated (`is.reset()` / re-skip). Since this crate reads
//! the whole stream into memory up front, we index by offset directly
//! instead, which sidesteps that complexity while producing the same map.

use std::collections::BTreeMap;

use crate::util::{
    get_int, get_long, get_short, get_string, get_timestamp, get_unicode_string, MppDateTime,
};

use super::var_meta::VarMeta;

/// Parsed `Var2Data` stream: variable-length field values, indexed by the
/// byte offsets recorded in a companion `VarMeta` block.
pub struct Var2Data<'a> {
    meta: &'a VarMeta,
    map: BTreeMap<i32, &'a [u8]>,
}

impl<'a> Var2Data<'a> {
    /// Parse a `Var2Data` stream. Mirrors `Var2Data(ProjectFile, VarMeta,
    /// InputStream)`: for every offset recorded in `meta`, read a 4 byte
    /// size prefix followed by that many bytes of value data.
    pub fn parse(meta: &'a VarMeta, data: &'a [u8]) -> Var2Data<'a> {
        let mut map = BTreeMap::new();

        for &item_offset in meta.offsets() {
            if item_offset < 0 {
                continue;
            }
            let item_offset = item_offset as usize;
            if item_offset >= data.len() || map.contains_key(&(item_offset as i32)) {
                continue;
            }
            if item_offset + 4 > data.len() {
                continue;
            }
            let size = get_int(data, item_offset);
            if size < 0 {
                continue;
            }
            let size = size as usize;
            let start = item_offset + 4;
            let end = start + size;
            if end > data.len() {
                continue;
            }
            map.insert(item_offset as i32, &data[start..end]);
        }

        Var2Data { meta, map }
    }

    fn bytes_at_offset(&self, offset: Option<i32>) -> Option<&'a [u8]> {
        offset.and_then(|o| self.map.get(&o).copied())
    }

    /// Raw bytes for the given entity unique ID and field type.
    pub fn byte_array(&self, id: i32, field_type: i32) -> Option<&'a [u8]> {
        self.bytes_at_offset(self.meta.offset(id, field_type))
    }

    pub fn unicode_string(&self, id: i32, field_type: i32) -> Option<String> {
        self.byte_array(id, field_type)
            .map(|v| get_unicode_string(v, 0))
    }

    pub fn string(&self, id: i32, field_type: i32) -> Option<String> {
        self.byte_array(id, field_type).map(|v| get_string(v, 0))
    }

    pub fn timestamp(&self, id: i32, field_type: i32) -> Option<MppDateTime> {
        let v = self.byte_array(id, field_type)?;
        if v.len() < 4 {
            return None;
        }
        get_timestamp(v, 0)
    }

    pub fn short(&self, id: i32, field_type: i32) -> i32 {
        match self.byte_array(id, field_type) {
            Some(v) if v.len() >= 2 => get_short(v, 0),
            _ => 0,
        }
    }

    pub fn int(&self, id: i32, field_type: i32) -> i32 {
        match self.byte_array(id, field_type) {
            Some(v) if v.len() >= 4 => get_int(v, 0),
            _ => 0,
        }
    }

    pub fn long(&self, id: i32, field_type: i32) -> i64 {
        match self.byte_array(id, field_type) {
            Some(v) if v.len() >= 8 => get_long(v, 0),
            _ => 0,
        }
    }

    pub fn double(&self, id: i32, field_type: i32) -> f64 {
        let bits = self.long(id, field_type);
        let result = f64::from_bits(bits as u64);
        if result.is_nan() {
            0.0
        } else {
            result
        }
    }

    pub fn var_meta(&self) -> &VarMeta {
        self.meta
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

    fn build_var_meta_and_data() -> (Vec<u8>, Vec<u8>) {
        let mut meta = Vec::new();
        meta.extend_from_slice(&le32(0xFADFADBAu32 as i32));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(1));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(7)); // unique id
        meta.extend_from_slice(&le32(0)); // offset
        meta.extend_from_slice(&le16(14)); // type
        meta.extend_from_slice(&le16(0));

        let mut data = Vec::new();
        let name: Vec<u8> = "Hi\0"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        data.extend_from_slice(&le32(name.len() as i32));
        data.extend_from_slice(&name);

        (meta, data)
    }

    #[test]
    fn reads_unicode_string_by_offset() {
        let (meta_bytes, data_bytes) = build_var_meta_and_data();
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data_bytes);
        assert_eq!(var_data.unicode_string(7, 14).as_deref(), Some("Hi"));
    }

    #[test]
    fn missing_offset_returns_none() {
        let (meta_bytes, data_bytes) = build_var_meta_and_data();
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data_bytes);
        assert_eq!(var_data.unicode_string(999, 14), None);
    }

    /// Build a `VarMeta` stream with arbitrary (unique_id, offset, type)
    /// entries, for exercising `Var2Data::parse`'s defensive checks.
    fn build_var_meta(entries: &[(i32, i32, i16)]) -> Vec<u8> {
        let mut meta = Vec::new();
        meta.extend_from_slice(&le32(0xFADFADBAu32 as i32));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(entries.len() as i32));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(0));
        meta.extend_from_slice(&le32(0));
        for &(id, offset, ty) in entries {
            meta.extend_from_slice(&le32(id));
            meta.extend_from_slice(&le32(offset));
            meta.extend_from_slice(&le16(ty));
            meta.extend_from_slice(&le16(0));
        }
        meta
    }

    #[test]
    fn negative_offset_is_skipped() {
        let meta_bytes = build_var_meta(&[(1, -1, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &[]);
        assert_eq!(var_data.byte_array(1, 5), None);
    }

    #[test]
    fn offset_past_end_of_data_is_skipped() {
        let meta_bytes = build_var_meta(&[(1, 100, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let data = vec![0u8; 10];
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.byte_array(1, 5), None);
    }

    #[test]
    fn truncated_size_prefix_is_skipped() {
        // Offset 8 with only 2 bytes remaining: not enough for a 4 byte
        // size prefix.
        let meta_bytes = build_var_meta(&[(1, 8, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let data = vec![0u8; 10];
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.byte_array(1, 5), None);
    }

    #[test]
    fn negative_size_prefix_is_skipped() {
        let mut data = Vec::new();
        data.extend_from_slice(&le32(-1)); // size: negative
        let meta_bytes = build_var_meta(&[(1, 0, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.byte_array(1, 5), None);
    }

    #[test]
    fn size_prefix_larger_than_remaining_data_is_skipped() {
        let mut data = Vec::new();
        data.extend_from_slice(&le32(100)); // size claims 100 bytes, none follow
        let meta_bytes = build_var_meta(&[(1, 0, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.byte_array(1, 5), None);
    }

    #[test]
    fn two_fields_sharing_the_same_offset_both_resolve() {
        // Two different (id, type) entries can point at the same offset
        // when MS Project deduplicates identical values.
        let mut data = Vec::new();
        data.extend_from_slice(&le32(4));
        data.extend_from_slice(b"abcd");

        let meta_bytes = build_var_meta(&[(1, 0, 5), (2, 0, 6)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.byte_array(1, 5), Some(b"abcd".as_slice()));
        assert_eq!(var_data.byte_array(2, 6), Some(b"abcd".as_slice()));
    }

    #[test]
    fn timestamp_with_too_few_bytes_is_none() {
        let mut data = Vec::new();
        data.extend_from_slice(&le32(2));
        data.extend_from_slice(&[0, 0]); // only 2 bytes: too short for a timestamp

        let meta_bytes = build_var_meta(&[(1, 0, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.timestamp(1, 5), None);
    }

    #[test]
    fn timestamp_with_enough_bytes_delegates_to_get_timestamp() {
        // days = 100, time = 0: a valid, decodable timestamp.
        let mut payload = [0u8; 4];
        payload[2..4].copy_from_slice(&100i16.to_le_bytes());
        let mut data = Vec::new();
        data.extend_from_slice(&le32(4));
        data.extend_from_slice(&payload);

        let meta_bytes = build_var_meta(&[(1, 0, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert!(var_data.timestamp(1, 5).is_some());
    }

    #[test]
    fn short_int_long_default_to_zero_when_absent_or_too_short() {
        let mut data = Vec::new();
        data.extend_from_slice(&le32(1));
        data.extend_from_slice(&[0]); // 1 byte: too short for short/int/long

        let meta_bytes = build_var_meta(&[(1, 0, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.short(1, 5), 0);
        assert_eq!(var_data.int(1, 5), 0);
        assert_eq!(var_data.long(1, 5), 0);

        // Entirely absent field.
        assert_eq!(var_data.short(1, 999), 0);
        assert_eq!(var_data.int(1, 999), 0);
        assert_eq!(var_data.long(1, 999), 0);
    }

    #[test]
    fn double_nan_becomes_zero() {
        let mut data = Vec::new();
        let nan_bytes = f64::NAN.to_le_bytes();
        data.extend_from_slice(&le32(8));
        data.extend_from_slice(&nan_bytes);

        let meta_bytes = build_var_meta(&[(1, 0, 5)]);
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data);
        assert_eq!(var_data.double(1, 5), 0.0);
    }

    #[test]
    fn var_meta_returns_the_underlying_meta() {
        let (meta_bytes, data_bytes) = build_var_meta_and_data();
        let meta = VarMeta::parse(&meta_bytes).unwrap();
        let var_data = Var2Data::parse(&meta, &data_bytes);
        assert_eq!(var_data.var_meta().offset(7, 14), Some(0));
    }
}
