//! zaf-mpp is a pure-Rust, read-only parser for project schedule files:
//! Microsoft Project MPP14 (Project 2010 through 365) and Primavera P6
//! exports in both XER and PMXML form. It is a deliberate port of the read
//! path of [MPXJ](https://github.com/joniles/mpxj), and it is licensed
//! under the GNU Lesser General Public License, version 2.1 or later, the
//! same terms as MPXJ itself. See `README.md` for the full licence
//! statement and scope.
//!
//! Every format parses into the same [`model::Project`] shape, so
//! consumers are format-agnostic: use [`read_project`] /
//! [`read_project_bytes`] to auto-detect, or the per-format entry points
//! ([`read_mpp`], [`read_xer`], [`read_pmxml`]) when the format is known.

pub mod container;
pub mod error;
pub mod ffi;
pub mod frb_api;
mod lookup;
pub mod model;
mod reader;
mod rtf;
pub mod util;

use std::fs;
use std::path::Path;

pub use error::MppError;
pub use model::Project;

/// Read and parse an MPP14 file from disk.
pub fn read_mpp(path: impl AsRef<Path>) -> Result<Project, MppError> {
    let bytes = fs::read(path)?;
    read_mpp_bytes(&bytes)
}

/// Read and parse an MPP14 file already loaded into memory.
pub fn read_mpp_bytes(bytes: &[u8]) -> Result<Project, MppError> {
    reader::read_mpp_bytes(bytes)
}

/// Read and parse a Primavera P6 XER export from disk.
pub fn read_xer(path: impl AsRef<Path>) -> Result<Project, MppError> {
    let bytes = fs::read(path)?;
    read_xer_bytes(&bytes)
}

/// Read and parse a Primavera P6 XER export already loaded into memory.
pub fn read_xer_bytes(bytes: &[u8]) -> Result<Project, MppError> {
    reader::p6::xer::read_xer_bytes(bytes)
}

/// Read and parse a Primavera P6 PMXML export from disk.
pub fn read_pmxml(path: impl AsRef<Path>) -> Result<Project, MppError> {
    let bytes = fs::read(path)?;
    read_pmxml_bytes(&bytes)
}

/// Read and parse a Primavera P6 PMXML export already loaded into memory.
pub fn read_pmxml_bytes(bytes: &[u8]) -> Result<Project, MppError> {
    reader::p6::pmxml::read_pmxml_bytes(bytes)
}

/// Read and parse a schedule file of any supported format, detected from
/// its content: OLE2 magic bytes for MPP, an `ERMHDR` header for XER,
/// leading XML for PMXML.
pub fn read_project(path: impl AsRef<Path>) -> Result<Project, MppError> {
    let bytes = fs::read(path)?;
    read_project_bytes(&bytes)
}

/// Read and parse a schedule file of any supported format from memory.
/// See [`read_project`] for how the format is detected.
pub fn read_project_bytes(bytes: &[u8]) -> Result<Project, MppError> {
    const OLE2_MAGIC: [u8; 4] = [0xD0, 0xCF, 0x11, 0xE0];
    if bytes.starts_with(&OLE2_MAGIC) {
        return read_mpp_bytes(bytes);
    }

    // Skip a UTF-8 BOM and leading whitespace before sniffing text formats.
    let stripped = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    let text_start: &[u8] = match stripped.iter().position(|b| !b.is_ascii_whitespace()) {
        Some(idx) => &stripped[idx..],
        None => &[],
    };

    if text_start.starts_with(b"ERMHDR") {
        return read_xer_bytes(bytes);
    }
    if text_start.starts_with(b"<") {
        return read_pmxml_bytes(bytes);
    }
    Err(MppError::UnsupportedVersion {
        detected: "unrecognised file content (not MPP, XER or PMXML)".to_string(),
    })
}
