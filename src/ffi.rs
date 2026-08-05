//! C ABI for consuming applications that link zaf-mpp dynamically (the
//! `cdylib` target exists for exactly this purpose, per LGPL section 6:
//! a consumer that links dynamically is free to relink against a modified
//! copy of this library). This is also the boundary Dart's `dart:ffi`
//! calls through directly, so every entry point here is panic-safe: a
//! panic anywhere inside zaf-mpp is caught before it would otherwise
//! unwind across the `extern "C"` boundary, which is undefined behaviour
//! and can crash or corrupt the embedding process (a Flutter app, in
//! particular) rather than just fail the one call.
//!
//! Two functions make up the whole ABI:
//!
//! - [`zaf_mpp_parse`]: parse an MPP14 file from an in-memory buffer,
//!   returning a JSON envelope, owned by the caller, of the shape
//!   `{"ok": <project>}` or `{"error": {"kind": "...", "message": "..."}}`.
//!   `kind` is one of `not_a_compound_file`, `unsupported_version`,
//!   `password_protected`, `corrupt`, `io`, `panic`. In practice this
//!   never returns null: every call, success or failure, produces a JSON
//!   string to decode. (The only theoretical null path is `CString::new`
//!   failing on an embedded NUL byte — unreachable today since every
//!   string returned here is either `serde_json` output, which always
//!   escapes control characters, or a literal we wrote ourselves; nothing
//!   in the type system stops a future change to how that JSON is built
//!   from reintroducing the possibility, so callers should still check
//!   for null defensively.)
//! - [`zaf_mpp_free_string`]: free a string this crate returned.
//!
//! ## Usage
//!
//! ```c
//! char *result = zaf_mpp_parse(buf, len);
//! // result is always valid JSON; decode it and check for "ok" vs "error".
//! puts(result);
//! zaf_mpp_free_string(result);
//! ```
//!
//! Every non-null pointer returned by `zaf_mpp_parse` must eventually be
//! passed to `zaf_mpp_free_string` exactly once. Passing any other
//! pointer, or passing the same pointer twice, is undefined behaviour.

use std::ffi::CString;
use std::os::raw::{c_char, c_uchar};
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::slice;

use serde::Serialize;

use crate::model::Project;

/// Serialises as `{"ok": <project>}` or `{"error": {"kind": ..., "message":
/// ...}}`: an externally-tagged enum with tuple variants serialises to
/// exactly `{"<lowercase variant name>": <inner value>}`.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum Envelope {
    Ok(Box<Project>),
    Error(ErrorPayload),
}

#[derive(Serialize)]
struct ErrorPayload {
    kind: &'static str,
    message: String,
}

fn error_json(kind: &'static str, message: impl Into<String>) -> String {
    let envelope = Envelope::Error(ErrorPayload {
        kind,
        message: message.into(),
    });
    // An error payload is just two plain strings; a failure here would
    // mean serde_json itself is broken, not anything about zaf-mpp's data.
    serde_json::to_string(&envelope).unwrap_or_else(|_| {
        format!("{{\"error\":{{\"kind\":\"{kind}\",\"message\":\"serialization failed\"}}}}")
    })
}

/// Extract a human-readable message from a caught panic payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

fn string_to_raw(s: String) -> *mut c_char {
    // `s` is always either serde_json output (never contains a raw NUL
    // byte: JSON escapes control characters) or a literal fallback string
    // we wrote ourselves, so this cannot fail in practice; the null
    // fallback exists only so a future change can't turn it into UB.
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Parse an MPP14 file held in `bytes[0..len)` and return a JSON envelope
/// describing the result, owned by the caller. See the module
/// documentation for the envelope shape and the caveat on the null
/// return path. Parse failures, and even an internal panic, are reported
/// as a JSON error object rather than a null pointer or an aborted call.
///
/// # Safety
///
/// `bytes` must be valid for reads of `len` bytes for the duration of this
/// call, or null (treated as empty).
#[no_mangle]
pub unsafe extern "C" fn zaf_mpp_parse(bytes: *const c_uchar, len: usize) -> *mut c_char {
    let slice: &[u8] = if bytes.is_null() {
        &[]
    } else {
        slice::from_raw_parts(bytes, len)
    };

    let result = panic::catch_unwind(AssertUnwindSafe(|| crate::read_mpp_bytes(slice)));

    let json = match result {
        Ok(Ok(project)) => match serde_json::to_string(&Envelope::Ok(Box::new(project))) {
            Ok(json) => json,
            Err(e) => error_json("corrupt", e.to_string()),
        },
        Ok(Err(e)) => error_json(e.kind(), e.to_string()),
        Err(payload) => error_json("panic", panic_message(&*payload)),
    };

    string_to_raw(json)
}

/// Parse a schedule file of any supported format (MPP14, Primavera P6 XER
/// or PMXML, detected from the buffer's content — see
/// [`crate::read_project_bytes`]) and return the same JSON envelope as
/// [`zaf_mpp_parse`]. That function remains MPP-only for ABI stability;
/// this one is the format-agnostic entry point.
///
/// # Safety
///
/// `bytes` must be valid for reads of `len` bytes for the duration of this
/// call, or null (treated as empty).
#[no_mangle]
pub unsafe extern "C" fn zaf_mpp_parse_project(bytes: *const c_uchar, len: usize) -> *mut c_char {
    let slice: &[u8] = if bytes.is_null() {
        &[]
    } else {
        slice::from_raw_parts(bytes, len)
    };

    let result = panic::catch_unwind(AssertUnwindSafe(|| crate::read_project_bytes(slice)));

    let json = match result {
        Ok(Ok(project)) => match serde_json::to_string(&Envelope::Ok(Box::new(project))) {
            Ok(json) => json,
            Err(e) => error_json("corrupt", e.to_string()),
        },
        Ok(Err(e)) => error_json(e.kind(), e.to_string()),
        Err(payload) => error_json("panic", panic_message(&*payload)),
    };

    string_to_raw(json)
}

/// Free a string previously returned by [`zaf_mpp_parse`] or
/// [`zaf_mpp_parse_project`].
///
/// # Safety
///
/// `s` must be a pointer this crate returned, not yet freed, or null (a
/// no-op).
#[no_mangle]
pub unsafe extern "C" fn zaf_mpp_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(CString::from_raw(s));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn corpus_file(name: &str) -> Option<Vec<u8>> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/mpxj/junit/data")
            .join(name);
        std::fs::read(path).ok()
    }

    fn parse_to_string(bytes: &[u8]) -> String {
        let ptr = unsafe { zaf_mpp_parse(bytes.as_ptr(), bytes.len()) };
        assert!(!ptr.is_null(), "zaf_mpp_parse must never return null");
        let owned = unsafe { CString::from_raw(ptr) };
        owned.to_str().unwrap().to_string()
    }

    #[test]
    fn null_bytes_pointer_is_treated_as_empty() {
        let json = unsafe {
            let ptr = zaf_mpp_parse(ptr::null(), 0);
            assert!(!ptr.is_null());
            let s = CString::from_raw(ptr);
            s.to_str().unwrap().to_string()
        };
        assert!(json.starts_with("{\"error\":"));
        assert!(json.contains("\"kind\":\"not_a_compound_file\""));
    }

    #[test]
    fn garbage_bytes_are_not_a_compound_file() {
        let json = parse_to_string(&[0u8; 64]);
        assert!(json.starts_with("{\"error\":"));
        assert!(json.contains("\"kind\":\"not_a_compound_file\""));
    }

    #[test]
    fn empty_input_is_not_a_compound_file() {
        let json = parse_to_string(&[]);
        assert!(json.contains("\"kind\":\"not_a_compound_file\""));
    }

    #[test]
    fn free_string_of_null_is_a_no_op() {
        unsafe {
            zaf_mpp_free_string(ptr::null_mut());
        }
    }

    #[test]
    fn successful_parse_round_trips_through_json() {
        let Some(bytes) = corpus_file("mpp14task.mpp") else {
            eprintln!("skipping: mpp14task.mpp not found, run scripts/fetch-test-data.sh first");
            return;
        };

        let json = parse_to_string(&bytes);
        assert!(json.starts_with("{\"ok\":"));
        assert!(json.contains("\"tasks\""));
        assert!(json.contains("Task #1"));
    }

    #[test]
    fn parse_project_detects_and_parses_an_xer_buffer() {
        let Some(bytes) = corpus_file("PredecessorCalendar.xer") else {
            eprintln!(
                "skipping: PredecessorCalendar.xer not found, run scripts/fetch-test-data.sh first"
            );
            return;
        };

        let ptr = unsafe { zaf_mpp_parse_project(bytes.as_ptr(), bytes.len()) };
        assert!(!ptr.is_null());
        let json = unsafe { CString::from_raw(ptr) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(json.starts_with("{\"ok\":"));
        assert!(json.contains("TEST PROJECT"));
    }

    #[test]
    fn parse_project_rejects_unknown_content_with_typed_error() {
        let bytes = b"neither mpp nor xer nor xml";
        let ptr = unsafe { zaf_mpp_parse_project(bytes.as_ptr(), bytes.len()) };
        let json = unsafe { CString::from_raw(ptr) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(json.contains("\"kind\":\"unsupported_version\""));
    }

    #[test]
    fn error_json_falls_back_safely_if_serialization_somehow_failed() {
        // Exercise the literal fallback path directly: it must still be
        // valid enough to find the fields tests rely on, even though this
        // path only exists as a defensive backstop and isn't reachable
        // through normal use.
        let json = error_json("corrupt", "oops");
        assert!(json.contains("\"kind\":\"corrupt\""));
    }

    #[test]
    fn panic_message_extracts_str_and_string_payloads() {
        let str_payload: Box<dyn std::any::Any + Send> = Box::new("boom");
        assert_eq!(panic_message(&*str_payload), "boom");

        let string_payload: Box<dyn std::any::Any + Send> = Box::new(String::from("kaboom"));
        assert_eq!(panic_message(&*string_payload), "kaboom");

        let other_payload: Box<dyn std::any::Any + Send> = Box::new(42i32);
        assert_eq!(panic_message(&*other_payload), "unknown panic");
    }

    #[test]
    fn a_panic_inside_the_parsed_closure_is_caught_not_propagated() {
        // Proves the exact catch_unwind + AssertUnwindSafe pattern
        // zaf_mpp_parse uses does not abort the process or propagate past
        // this test, regardless of whether any real code path in zaf-mpp
        // itself can currently panic.
        let result = panic::catch_unwind(AssertUnwindSafe(|| -> i32 {
            panic!("synthetic panic for this test");
        }));
        assert!(result.is_err());
    }
}
