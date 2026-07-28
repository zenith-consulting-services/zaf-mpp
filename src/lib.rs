//! zaf-mpp is a pure-Rust, read-only parser for Microsoft Project MPP14
//! files (Project 2010 through 365). It is a deliberate port of the read
//! path of [MPXJ](https://github.com/joniles/mpxj), and it is licensed
//! under the GNU Lesser General Public License, version 2.1 or later, the
//! same terms as MPXJ itself. See `README.md` for the full licence
//! statement and scope.

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
