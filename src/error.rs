//! Error type for zaf-mpp.

use std::fmt;

/// Errors produced while reading an MPP file.
#[derive(thiserror::Error, Debug)]
pub enum MppError {
    /// The file is not a valid OLE2 / CFB compound document, so it cannot
    /// be an MPP file at all.
    #[error("not an OLE2 compound file: {source}")]
    NotACompoundFile {
        #[source]
        source: std::io::Error,
    },

    /// The compound document was read, but it is not MPP14 (Project 2010-365).
    /// `detected` names the format that was found, e.g. "MPP12 (Project 2007)".
    #[error("unsupported file format: {detected}")]
    UnsupportedVersion {
        /// Human readable name of the detected format.
        detected: String,
    },

    /// The file is protected with a password zaf-mpp cannot bypass.
    #[error("file is password protected")]
    PasswordProtected,

    /// The file's structure does not match what MPXJ's MPP14 reader expects.
    /// Carries a short description of what was expected and where.
    #[error("corrupt file: {0}")]
    Corrupt(String),

    /// Low level I/O failure (short read, seek past end, etc).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl MppError {
    /// Stable, machine readable tag for FFI consumers. See `ffi.rs`.
    pub fn kind(&self) -> &'static str {
        match self {
            MppError::NotACompoundFile { .. } => "not_a_compound_file",
            MppError::UnsupportedVersion { .. } => "unsupported_version",
            MppError::PasswordProtected => "password_protected",
            MppError::Corrupt(_) => "corrupt",
            MppError::Io(_) => "io",
        }
    }
}

/// Helper for building `Corrupt` errors with context, mirroring the
/// defensive checks MPXJ performs throughout its MPP14 reader.
pub(crate) fn corrupt(context: impl fmt::Display) -> MppError {
    MppError::Corrupt(context.to_string())
}

/// Convenience alias used throughout the crate's internals. Not named
/// `Result`: flutter_rust_bridge's generated glue assumes a bare `Result`
/// in scope is always `std::result::Result` (2 generic parameters), and a
/// 1-parameter alias of the same name breaks that generated code wherever
/// it ends up in scope alongside a mirrored type from this module.
pub type MppResult<T> = std::result::Result<T, MppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_matches_each_variant() {
        assert_eq!(
            MppError::NotACompoundFile {
                source: std::io::Error::from(std::io::ErrorKind::InvalidData)
            }
            .kind(),
            "not_a_compound_file"
        );
        assert_eq!(
            MppError::UnsupportedVersion {
                detected: "MPP12".to_string()
            }
            .kind(),
            "unsupported_version"
        );
        assert_eq!(MppError::PasswordProtected.kind(), "password_protected");
        assert_eq!(MppError::Corrupt("x".to_string()).kind(), "corrupt");
        assert_eq!(
            MppError::Io(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)).kind(),
            "io"
        );
    }

    #[test]
    fn display_messages_include_context() {
        assert_eq!(
            MppError::NotACompoundFile {
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, "bad magic")
            }
            .to_string(),
            "not an OLE2 compound file: bad magic"
        );
        assert_eq!(
            MppError::UnsupportedVersion {
                detected: "MPP12 (Project 2007)".to_string()
            }
            .to_string(),
            "unsupported file format: MPP12 (Project 2007)"
        );
        assert_eq!(
            MppError::PasswordProtected.to_string(),
            "file is password protected"
        );
        assert_eq!(
            MppError::Corrupt("short read".to_string()).to_string(),
            "corrupt file: short read"
        );
    }

    #[test]
    fn io_error_converts_via_from() {
        let io_err = std::io::Error::from(std::io::ErrorKind::NotFound);
        let err: MppError = io_err.into();
        assert_eq!(err.kind(), "io");
        assert!(err.to_string().starts_with("I/O error:"));
    }

    #[test]
    fn not_a_compound_file_preserves_the_source_error() {
        // Unlike `Corrupt`/`UnsupportedVersion` (synthesized messages with
        // no underlying error object), this variant always wraps a real
        // `io::Error` from the `cfb` crate, so `Error::source()` should
        // expose it for callers that want the full chain (`anyhow`,
        // structured logging, and so on), not just the flattened message.
        use std::error::Error as _;

        let io_err = std::io::Error::new(std::io::ErrorKind::InvalidData, "bad magic");
        let err = MppError::NotACompoundFile { source: io_err };
        let source = err.source().expect("source should be present");
        assert_eq!(source.to_string(), "bad magic");
    }

    #[test]
    fn io_variant_also_preserves_the_source_error() {
        use std::error::Error as _;

        let err: MppError = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert_eq!(err.source().unwrap().to_string(), "gone");
    }

    #[test]
    fn corrupt_helper_formats_context() {
        let err = corrupt(format!("bad offset {}", 42));
        assert_eq!(err.to_string(), "corrupt file: bad offset 42");
    }
}
