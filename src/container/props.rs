//! Ported from MPXJ: src/main/java/org/mpxj/mpp/Props.java and
//! src/main/java/org/mpxj/mpp/Props14.java
//! Copyright (c) Packwood Software 2003, 2010, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! `Props` streams hold a flat map of integer key to byte-array value; the
//! MPP14 "Props14" layout is a 16 byte header (item count as a `short` at
//! byte 12) followed by `(size, key, unused)` triples and their payloads,
//! each payload padded to a two byte boundary.

use std::collections::HashMap;

use crate::util::{get_double, get_int, get_short, get_timestamp, get_unicode_string, MppDateTime};

/// Parsed `Props` stream: a flat map of integer key to byte-array value.
pub struct Props {
    map: HashMap<i32, Vec<u8>>,
}

impl Props {
    /// Parse a "Props14" stream. Mirrors `Props14(ProjectFile, InputStream)`.
    pub fn parse(data: &[u8]) -> Props {
        let mut map = HashMap::new();
        if data.len() < 16 {
            return Props { map };
        }

        let header_count = get_short(data, 12) as usize;
        let mut found_count = 0usize;
        let mut pos = 16usize;

        while found_count < header_count {
            if pos + 12 > data.len() {
                break;
            }
            let attrib1 = get_int(data, pos) as i64;
            let attrib2 = get_int(data, pos + 4);
            // attrib3 (pos + 8) is unused, matching MPXJ.
            pos += 12;

            if attrib1 < 1 || pos as i64 + attrib1 > data.len() as i64 {
                break;
            }
            let len = attrib1 as usize;
            let value = data[pos..pos + len].to_vec();
            pos += len;
            found_count += 1;
            map.insert(attrib2, value);

            // Align to a two byte boundary.
            if !len.is_multiple_of(2) {
                pos += 1;
            }
        }

        Props { map }
    }

    pub fn byte_array(&self, key: i32) -> Option<&[u8]> {
        self.map.get(&key).map(|v| v.as_slice())
    }

    pub fn byte(&self, key: i32) -> u8 {
        self.map
            .get(&key)
            .and_then(|v| v.first())
            .copied()
            .unwrap_or(0)
    }

    pub fn short(&self, key: i32) -> i32 {
        match self.map.get(&key) {
            Some(v) if v.len() >= 2 => get_short(v, 0),
            _ => 0,
        }
    }

    pub fn int(&self, key: i32) -> i32 {
        match self.map.get(&key) {
            Some(v) if v.len() >= 4 => get_int(v, 0),
            _ => 0,
        }
    }

    pub fn double(&self, key: i32) -> f64 {
        match self.map.get(&key) {
            Some(v) if v.len() >= 8 => get_double(v, 0),
            _ => 0.0,
        }
    }

    pub fn boolean(&self, key: i32) -> bool {
        self.short(key) != 0
    }

    pub fn unicode_string(&self, key: i32) -> Option<String> {
        self.map.get(&key).map(|v| get_unicode_string(v, 0))
    }

    pub fn timestamp(&self, key: i32) -> Option<MppDateTime> {
        let v = self.map.get(&key)?;
        get_timestamp(v, 0)
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

    #[test]
    fn parses_single_property() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 12]); // unused header bytes
        data.extend_from_slice(&le16(1)); // item count
        data.extend_from_slice(&[0u8; 2]); // padding to 16 byte header
        data.extend_from_slice(&le32(4)); // attrib1: length
        data.extend_from_slice(&le32(37748738)); // attrib2: key (PROJECT_START_DATE)
        data.extend_from_slice(&le32(0)); // attrib3: unused
        data.extend_from_slice(&le32(99)); // payload

        let props = Props::parse(&data);
        assert_eq!(props.int(37748738), 99);
        assert_eq!(props.int(999), 0);
    }

    #[test]
    fn truncated_header_yields_empty_props_not_panic() {
        let props = Props::parse(&[0u8; 4]);
        assert_eq!(props.int(1), 0);
    }

    #[test]
    fn truncated_mid_record_stops_cleanly() {
        // Header claims 2 properties, but only 12 bytes (the fixed
        // attrib1/attrib2/attrib3 header) of the second record follow: not
        // enough to even read its length, so parsing stops without error.
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&le16(2)); // item count
        data.extend_from_slice(&[0u8; 2]);
        data.extend_from_slice(&le32(4));
        data.extend_from_slice(&le32(1));
        data.extend_from_slice(&le32(0));
        data.extend_from_slice(&le32(99));
        data.extend_from_slice(&[0u8; 8]); // second record header, truncated

        let props = Props::parse(&data);
        assert_eq!(props.int(1), 99);
    }

    #[test]
    fn short_and_double_default_to_zero_when_absent_or_too_short() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&le16(1));
        data.extend_from_slice(&[0u8; 2]);
        data.extend_from_slice(&le32(1)); // length: 1 byte, too short for short/double
        data.extend_from_slice(&le32(1));
        data.extend_from_slice(&le32(0));
        data.push(7);

        let props = Props::parse(&data);
        assert_eq!(props.short(1), 0);
        assert_eq!(props.double(1), 0.0);

        // Entirely absent key.
        assert_eq!(props.short(999), 0);
        assert_eq!(props.double(999), 0.0);
    }

    #[test]
    fn double_reads_a_full_value() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 12]);
        data.extend_from_slice(&le16(1));
        data.extend_from_slice(&[0u8; 2]);
        data.extend_from_slice(&le32(8));
        data.extend_from_slice(&le32(1));
        data.extend_from_slice(&le32(0));
        data.extend_from_slice(&3.5f64.to_le_bytes());

        let props = Props::parse(&data);
        assert_eq!(props.double(1), 3.5);
    }

    #[test]
    fn byte_defaults_to_zero_when_absent() {
        let props = Props::parse(&[0u8; 16]);
        assert_eq!(props.byte(1), 0);
    }
}
