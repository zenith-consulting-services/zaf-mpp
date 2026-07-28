//! OLE2 / CFB compound document access, built on the `cfb` crate. MPP files
//! are OLE2 compound documents; this module provides a thin path-based
//! wrapper used to load the named streams MPXJ's MPP14 reader expects
//! (`CompObj`, `Props14`, `TBkndTask/VarMeta`, and so on) fully into memory.
//!
//! Every block parser in this module (`comp_obj`, `props`, `var_meta`,
//! `var2_data`, `fixed_meta`, `fixed_data`) works on an in-memory `&[u8]`
//! rather than a stream, unlike MPXJ's `InputStream`-based originals: cfb
//! streams are small enough in practice to buffer, and doing so avoids
//! porting MPXJ's stream `reset`/skip bookkeeping.

pub mod comp_obj;
pub mod fixed_data;
pub mod fixed_meta;
pub mod props;
pub mod var2_data;
pub mod var_meta;

use std::io::{Cursor, Read};

use crate::error::{MppError, MppResult};

/// An open OLE2 compound document, with streams read on demand.
pub struct OleContainer {
    file: cfb::CompoundFile<Cursor<Vec<u8>>>,
}

impl OleContainer {
    /// Open an in-memory OLE2 compound document. Fails with
    /// [`MppError::NotACompoundFile`] if `bytes` isn't a valid CFB
    /// container, before any MPP-specific parsing happens.
    pub fn open(bytes: Vec<u8>) -> MppResult<OleContainer> {
        let cursor = Cursor::new(bytes);
        let file = cfb::CompoundFile::open(cursor)
            .map_err(|e| MppError::NotACompoundFile { source: e })?;
        Ok(OleContainer { file })
    }

    /// Read an entire stream into memory. `path` uses `/` separators, e.g.
    /// `"/\u{1}CompObj"` or `"/   114/TBkndTask/VarMeta"`.
    pub fn read_stream(&mut self, path: &str) -> MppResult<Vec<u8>> {
        let mut stream = self
            .file
            .open_stream(path)
            .map_err(|e| MppError::Corrupt(format!("missing stream {path}: {e}")))?;
        let mut buf = Vec::new();
        stream
            .read_to_end(&mut buf)
            .map_err(|e| MppError::Corrupt(format!("failed to read stream {path}: {e}")))?;
        Ok(buf)
    }

    /// True if `path` names a stream (not a storage/directory) in this
    /// document.
    pub fn stream_exists(&self, path: &str) -> bool {
        self.file.is_stream(path)
    }

    /// True if `path` names a storage (directory) in this document. Used
    /// to detect optional sections, e.g. a project with no task relations
    /// has no `TBkndCons` storage at all.
    pub fn storage_exists(&self, path: &str) -> bool {
        self.file.is_storage(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample_document() -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut file = cfb::CompoundFile::create(cursor).unwrap();
        file.create_storage("/Storage").unwrap();
        let mut stream = file.create_stream("/Storage/Stream").unwrap();
        stream.write_all(b"hello").unwrap();
        drop(stream);
        file.into_inner().into_inner()
    }

    #[test]
    fn open_rejects_non_compound_file_bytes() {
        let err = OleContainer::open(vec![0u8; 32]).err().unwrap();
        assert_eq!(err.kind(), "not_a_compound_file");
    }

    #[test]
    fn read_stream_round_trips_written_bytes() {
        let mut container = OleContainer::open(sample_document()).unwrap();
        let bytes = container.read_stream("/Storage/Stream").unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn read_stream_missing_path_is_corrupt() {
        let mut container = OleContainer::open(sample_document()).unwrap();
        let err = container.read_stream("/does/not/exist").err().unwrap();
        assert_eq!(err.kind(), "corrupt");
    }

    #[test]
    fn stream_exists_and_storage_exists_distinguish_kinds() {
        let container = OleContainer::open(sample_document()).unwrap();
        assert!(container.stream_exists("/Storage/Stream"));
        assert!(!container.stream_exists("/Storage"));
        assert!(container.storage_exists("/Storage"));
        assert!(!container.storage_exists("/Storage/Stream"));
        assert!(!container.stream_exists("/nope"));
        assert!(!container.storage_exists("/nope"));
    }
}
