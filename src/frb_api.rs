//! Entry points for `flutter_rust_bridge` (FRB) Dart bindings. These are
//! plain functions with no FRB dependency of their own: the actual codegen
//! target and `#[frb(...)]` annotations live in the thin bridge crate a
//! consuming Flutter app generates alongside its build (via
//! `flutter_rust_bridge_codegen integrate`), which depends on zaf-mpp and
//! calls straight through to the functions below. Keeping them here, rather
//! than duplicated in every consuming app, means the bridge-facing surface
//! is versioned and documented alongside the rest of the crate.
//!
//! `Project` and its nested types are exposed to Dart by value: FRB mirrors
//! every public field of every type reachable from a bridge function's
//! signature, so `model.rs` needs no FRB-specific annotations to stay a
//! plain, bridge-agnostic data model.

use crate::error::MppError;
use crate::model::Project;

/// Parse an MPP14 file already loaded into memory. Mirrors
/// [`crate::read_mpp_bytes`] with an owned buffer: FRB always marshals a
/// byte argument as a `Vec<u8>` copied across the bridge, so there is no
/// borrowed-slice form to offer here.
pub fn parse_mpp_bytes(bytes: Vec<u8>) -> Result<Project, MppError> {
    crate::read_mpp_bytes(&bytes)
}

/// Parse an MPP14 file from a filesystem path. Mirrors [`crate::read_mpp`].
pub fn parse_mpp_file(path: String) -> Result<Project, MppError> {
    crate::read_mpp(path)
}

/// Parse a Primavera P6 XER export already loaded into memory. Mirrors
/// [`crate::read_xer_bytes`].
pub fn parse_xer_bytes(bytes: Vec<u8>) -> Result<Project, MppError> {
    crate::read_xer_bytes(&bytes)
}

/// Parse a Primavera P6 XER export from a filesystem path. Mirrors
/// [`crate::read_xer`].
pub fn parse_xer_file(path: String) -> Result<Project, MppError> {
    crate::read_xer(path)
}

/// Parse a Primavera P6 PMXML export already loaded into memory. Mirrors
/// [`crate::read_pmxml_bytes`].
pub fn parse_pmxml_bytes(bytes: Vec<u8>) -> Result<Project, MppError> {
    crate::read_pmxml_bytes(&bytes)
}

/// Parse a Primavera P6 PMXML export from a filesystem path. Mirrors
/// [`crate::read_pmxml`].
pub fn parse_pmxml_file(path: String) -> Result<Project, MppError> {
    crate::read_pmxml(path)
}

/// Parse a schedule file of any supported format (MPP14, XER, PMXML),
/// detected from its content. Mirrors [`crate::read_project_bytes`].
pub fn parse_project_bytes(bytes: Vec<u8>) -> Result<Project, MppError> {
    crate::read_project_bytes(&bytes)
}

/// Parse a schedule file of any supported format from a filesystem path.
/// Mirrors [`crate::read_project`].
pub fn parse_project_file(path: String) -> Result<Project, MppError> {
    crate::read_project(path)
}
