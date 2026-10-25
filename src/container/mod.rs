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
mod direct;
pub mod fixed_data;
pub mod fixed_meta;
pub mod props;
pub mod var2_data;
pub mod var_meta;

use std::borrow::Cow;
use std::io::{Cursor, Read, Seek};

use crate::error::{MppError, MppResult};

/// An open OLE2 compound document, with streams read on demand. The
/// default type parameter owns its bytes ([`OleContainer::open`]);
/// [`OleContainer::open_slice`] borrows them instead, avoiding a copy.
pub struct OleContainer<F = Cursor<Vec<u8>>> {
    file: cfb::CompoundFile<F>,
}

impl<'a> OleContainer<Cursor<&'a [u8]>> {
    /// As [`OleContainer::open`], reading from borrowed bytes.
    pub fn open_slice(bytes: &'a [u8]) -> MppResult<Self> {
        Self::open_reader(Cursor::new(bytes))
    }
}

impl OleContainer {
    /// Open an in-memory OLE2 compound document. Fails with
    /// [`MppError::NotACompoundFile`] if `bytes` isn't a valid CFB
    /// container, before any MPP-specific parsing happens.
    pub fn open(bytes: Vec<u8>) -> MppResult<OleContainer> {
        Self::open_reader(Cursor::new(bytes))
    }
}

impl<F: Read + Seek> OleContainer<F> {
    fn open_reader(reader: F) -> MppResult<Self> {
        let file = cfb::CompoundFile::open(reader)
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
        // Size the buffer up front: growing it while reading costs a
        // realloc and copy per doubling. Capped so a corrupt length field
        // cannot request a huge allocation before the read fails.
        let capacity = usize::try_from(stream.len()).unwrap_or(0).min(64 << 20);
        let mut buf = Vec::with_capacity(capacity);
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

/// An OLE2 compound document read from borrowed bytes: opened and
/// validated by `cfb`, with stream contents served zero-copy by
/// [`direct::DirectReader`] where it can, and by `cfb` otherwise.
pub(crate) struct SliceContainer<'a> {
    ole: OleContainer<Cursor<&'a [u8]>>,
    direct: Option<direct::DirectReader<'a>>,
}

impl<'a> SliceContainer<'a> {
    pub(crate) fn open(bytes: &'a [u8]) -> MppResult<Self> {
        let ole = OleContainer::open_slice(bytes)?;
        Ok(SliceContainer {
            ole,
            direct: direct::DirectReader::new(bytes),
        })
    }

    /// Stream contents, borrowed from the input where possible. Same
    /// bytes and same errors as [`OleContainer::read_stream`].
    pub(crate) fn stream(&mut self, path: &str) -> MppResult<Cow<'a, [u8]>> {
        if let Some(bytes) = self.direct.as_ref().and_then(|d| d.stream(path)) {
            return Ok(bytes);
        }
        self.ole.read_stream(path).map(Cow::Owned)
    }

    pub(crate) fn storage_exists(&self, path: &str) -> bool {
        self.ole.storage_exists(path)
    }
}

#[cfg(test)]
mod tests {

    /// Every stream in every corpus compound file reads back identically
    /// through the direct reader and through `cfb`.
    #[test]
    fn direct_reader_matches_cfb_for_every_corpus_stream() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mpxj/junit/data");
        if !root.exists() {
            eprintln!("skipping: corpus not fetched");
            return;
        }
        let mut stack = vec![root];
        let (mut files, mut streams, mut direct_hits) = (0, 0, 0);
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let bytes = std::fs::read(&path).unwrap();
                let Ok(mut ole) = OleContainer::open_slice(&bytes) else {
                    continue;
                };
                files += 1;
                let direct = direct::DirectReader::new(&bytes)
                    .unwrap_or_else(|| panic!("{}: direct reader declined", path.display()));
                let stream_paths: Vec<String> = ole
                    .file
                    .walk()
                    .filter(|e| e.is_stream())
                    .map(|e| e.path().to_string_lossy().into_owned())
                    .collect();
                for stream_path in stream_paths {
                    streams += 1;
                    let expected = ole.read_stream(&stream_path).unwrap();
                    if let Some(actual) = direct.stream(&stream_path) {
                        direct_hits += 1;
                        assert_eq!(
                            actual.as_ref(),
                            expected.as_slice(),
                            "{} {stream_path}",
                            path.display()
                        );
                    }
                }
            }
        }
        eprintln!("{files} files, {streams} streams, {direct_hits} served directly");
        assert_eq!(streams, direct_hits, "some streams fell back to cfb");
    }

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
