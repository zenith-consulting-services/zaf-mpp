//! Ported from MPXJ: src/main/java/org/mpxj/mpp/CompObj.java
//! Copyright (c) Packwood Software 2002-2003, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Reads the `\1CompObj` stream of an OLE2 compound document. This stream
//! carries the application name and, for MPP files, the file format string
//! used to distinguish MPP8/9/12/14 and choose the correct reader.

use crate::error::{corrupt, MppResult};

/// Parsed contents of the `\1CompObj` stream.
pub struct CompObj {
    pub application_name: String,
    pub application_version: Option<u32>,
    pub file_format: Option<String>,
}

impl CompObj {
    /// Parse a `\1CompObj` stream. Mirrors `CompObj(InputStream)`: skip a
    /// fixed 28 byte header, then read three length-prefixed, nul-terminated
    /// strings (application name, file format, application id). Only the
    /// first two are needed here.
    pub fn parse(data: &[u8]) -> MppResult<CompObj> {
        let mut r = ByteReader::new(data);
        r.skip(28)?;

        let name_len = r.read_i32_le()? as usize;
        let application_name = r.read_nul_terminated_str(name_len)?;

        let mut application_version = None;
        if let Some(caps) = parse_version_suffix(&application_name) {
            application_version = Some(caps);
        }

        let mut file_format = None;
        if application_name == "Microsoft Project 4.0" {
            file_format = Some("MSProject.MPP4".to_string());
        } else if let Ok(fmt_len) = r.read_i32_le() {
            if fmt_len > 0 {
                file_format = Some(r.read_nul_terminated_str(fmt_len as usize)?);
                // application id string follows; not needed publicly.
            }
        }

        Ok(CompObj {
            application_name,
            application_version,
            file_format,
        })
    }
}

/// Matches MPXJ's regex `Microsoft.Project.(\d+)\.0` against the
/// application name, e.g. "Microsoft Project 14.0" -> 14.
fn parse_version_suffix(name: &str) -> Option<u32> {
    // MPXJ's pattern is `Microsoft.Project.(\d+)\.0`: the two middle dots
    // are regex wildcards, not literal characters, so real-world CompObj
    // names like "Microsoft.Project 14.0" and "Microsoft Project.14.0" both
    // match. We match the same way: any single character in each gap.
    let bytes = name.as_bytes();
    let head = b"Microsoft";
    let mid = b"Project";
    if bytes.len() < head.len() + 1 + mid.len() + 1 {
        return None;
    }
    if &bytes[..head.len()] != head {
        return None;
    }
    let mut pos = head.len() + 1; // skip "Microsoft" + one wildcard char
    if &bytes[pos..pos + mid.len()] != mid {
        return None;
    }
    pos += mid.len() + 1; // skip "Project" + one wildcard char

    let digits_start = pos;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        pos += 1;
    }
    if pos == digits_start {
        return None;
    }
    let digits = &name[digits_start..pos];
    if &bytes[pos..] != b".0" {
        return None;
    }
    digits.parse().ok()
}

/// Minimal cursor over a byte slice with the primitives the container
/// block parsers need. Every parser in this module works on a fully
/// buffered `&[u8]` rather than a stream, since cfb streams are read into
/// memory up front (see `container/mod.rs`).
pub(crate) struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        ByteReader { data, pos: 0 }
    }

    pub(crate) fn skip(&mut self, n: usize) -> MppResult<()> {
        if self.pos + n > self.data.len() {
            return Err(corrupt(format!(
                "unexpected end of stream skipping {n} bytes at {}",
                self.pos
            )));
        }
        self.pos += n;
        Ok(())
    }

    pub(crate) fn read_bytes(&mut self, n: usize) -> MppResult<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(corrupt(format!(
                "unexpected end of stream reading {n} bytes at {}",
                self.pos
            )));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    pub(crate) fn read_i32_le(&mut self) -> MppResult<i32> {
        let b = self.read_bytes(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Read `len` bytes and interpret them as a nul-terminated ASCII/Latin-1
    /// string, dropping the trailing nul as MPXJ does (`length - 1`).
    fn read_nul_terminated_str(&mut self, len: usize) -> MppResult<String> {
        if len == 0 {
            return Ok(String::new());
        }
        let b = self.read_bytes(len)?;
        let trimmed = &b[..len - 1];
        Ok(trimmed.iter().map(|&c| c as char).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(v: i32) -> [u8; 4] {
        v.to_le_bytes()
    }

    #[test]
    fn parses_application_name_and_format() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 28]);
        let name = b"Microsoft Project 14.0\0";
        data.extend_from_slice(&le32(name.len() as i32));
        data.extend_from_slice(name);
        let format = b"MSProject.MPP14\0";
        data.extend_from_slice(&le32(format.len() as i32));
        data.extend_from_slice(format);
        let id = b"MSProject.Project.14\0";
        data.extend_from_slice(&le32(id.len() as i32));
        data.extend_from_slice(id);

        let compobj = CompObj::parse(&data).unwrap();
        assert_eq!(compobj.application_name, "Microsoft Project 14.0");
        assert_eq!(compobj.application_version, Some(14));
        assert_eq!(compobj.file_format.as_deref(), Some("MSProject.MPP14"));
    }

    #[test]
    fn parses_version_from_dotted_application_name() {
        // Real MPP14 files store "Microsoft.Project 14.0" (or similar
        // punctuation), not "Microsoft Project 14.0": MPXJ's regex uses
        // unescaped dots as wildcards, so both forms must parse.
        assert_eq!(parse_version_suffix("Microsoft.Project 14.0"), Some(14));
        assert_eq!(parse_version_suffix("Microsoft Project.15.0"), Some(15));
        assert_eq!(parse_version_suffix("Microsoft Project 16.0"), Some(16));
        assert_eq!(parse_version_suffix("Something Else"), None);
    }

    #[test]
    fn truncated_header_is_corrupt_not_panic() {
        let data = vec![0u8; 10];
        assert!(CompObj::parse(&data).is_err());
    }

    #[test]
    fn version_suffix_rejects_mismatched_middle_word() {
        // "Project" must appear (any single wildcard char before it), so a
        // completely different middle word never matches.
        assert_eq!(parse_version_suffix("Microsoft XSomething 14.0"), None);
    }

    #[test]
    fn version_suffix_rejects_missing_digits() {
        assert_eq!(parse_version_suffix("Microsoft.Project .0"), None);
    }

    #[test]
    fn version_suffix_rejects_missing_dot_zero_suffix() {
        assert_eq!(parse_version_suffix("Microsoft.Project 14x"), None);
    }

    #[test]
    fn truncated_file_format_length_is_corrupt_not_panic() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 28]);
        let name = b"Microsoft Project 14.0\0";
        data.extend_from_slice(&le32(name.len() as i32));
        data.extend_from_slice(name);
        // Claims a file-format string longer than the remaining bytes.
        data.extend_from_slice(&le32(1000));
        data.extend_from_slice(b"short");

        assert!(CompObj::parse(&data).is_err());
    }

    #[test]
    fn zero_length_file_format_is_absent() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 28]);
        let name = b"Some Other App\0";
        data.extend_from_slice(&le32(name.len() as i32));
        data.extend_from_slice(name);
        data.extend_from_slice(&le32(0)); // zero-length file format string

        let compobj = CompObj::parse(&data).unwrap();
        assert_eq!(compobj.file_format, None);
    }

    #[test]
    fn zero_length_application_name_is_empty_string() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 28]);
        data.extend_from_slice(&le32(0)); // zero-length application name

        let compobj = CompObj::parse(&data).unwrap();
        assert_eq!(compobj.application_name, "");
        assert_eq!(compobj.application_version, None);
    }
}
