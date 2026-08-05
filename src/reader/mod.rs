//! Ported from MPXJ: src/main/java/org/mpxj/mpp/MPPReader.java
//! Copyright (c) Packwood Software 2005, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Top level entry point: detect the MPP sub-format from the `\1CompObj`
//! stream and dispatch to the MPP14 reader. MPP8/9/12 are recognised only
//! well enough to report [`crate::MppError::UnsupportedVersion`] with a
//! human readable name, matching the scope of this crate.

mod field_map;
mod mpp14;
pub(crate) mod p6;

use crate::container::comp_obj::CompObj;
use crate::container::OleContainer;
use crate::error::{MppError, MppResult};
use crate::model::Project;

pub fn read_mpp_bytes(bytes: &[u8]) -> MppResult<Project> {
    let mut container = OleContainer::open(bytes.to_vec())?;

    let compobj_bytes = container.read_stream("/\u{1}CompObj")?;
    let compobj = CompObj::parse(&compobj_bytes)?;
    let format = compobj.file_format.clone().unwrap_or_default();

    match format.as_str() {
        "MSProject.MPP14" | "MSProject.MPT14" | "MSProject.GLOBAL14" => {
            mpp14::read(&mut container, compobj.application_version)
        }
        "MSProject.MPP12" | "MSProject.MPT12" | "MSProject.GLOBAL12" => {
            Err(MppError::UnsupportedVersion {
                detected: "MPP12 (Project 2007)".to_string(),
            })
        }
        "MSProject.MPP9" | "MSProject.MPT9" | "MSProject.GLOBAL9" => {
            Err(MppError::UnsupportedVersion {
                detected: "MPP9 (Project 2000-2003)".to_string(),
            })
        }
        "MSProject.MPP8" | "MSProject.MPT8" | "MSProject.MPP4" => {
            Err(MppError::UnsupportedVersion {
                detected: "MPP8 or earlier (Project 98 and prior)".to_string(),
            })
        }
        "" => Err(MppError::UnsupportedVersion {
            detected: format!("unrecognised file format ({})", compobj.application_name),
        }),
        other => Err(MppError::UnsupportedVersion {
            detected: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    /// Build a minimal in-memory OLE2 compound document containing only a
    /// `\1CompObj` stream, with the given application name and file format
    /// strings encoded the way `container::comp_obj::CompObj::parse`
    /// expects (28 byte header skip, then two length-prefixed,
    /// nul-terminated strings).
    fn compound_file_with_comp_obj(application_name: &str, file_format: &str) -> Vec<u8> {
        let mut compobj = vec![0u8; 28];

        let mut name_bytes = application_name.as_bytes().to_vec();
        name_bytes.push(0);
        compobj.extend_from_slice(&(name_bytes.len() as i32).to_le_bytes());
        compobj.extend_from_slice(&name_bytes);

        let mut format_bytes = file_format.as_bytes().to_vec();
        format_bytes.push(0);
        compobj.extend_from_slice(&(format_bytes.len() as i32).to_le_bytes());
        compobj.extend_from_slice(&format_bytes);

        // Application ID string: empty is fine, CompObj::parse doesn't use it.
        compobj.extend_from_slice(&0i32.to_le_bytes());

        let cursor = Cursor::new(Vec::new());
        let mut file = cfb::CompoundFile::create(cursor).unwrap();
        let mut stream = file.create_stream("/\u{1}CompObj").unwrap();
        stream.write_all(&compobj).unwrap();
        drop(stream);
        file.into_inner().into_inner()
    }

    #[test]
    fn mpp14_format_dispatches_to_mpp14_reader() {
        // No further streams exist, so the MPP14 reader itself will fail,
        // but it proves dispatch picked the right branch rather than
        // reporting UnsupportedVersion.
        let bytes = compound_file_with_comp_obj("Microsoft Project 14.0", "MSProject.MPP14");
        let err = read_mpp_bytes(&bytes).unwrap_err();
        assert_ne!(err.kind(), "unsupported_version");
    }

    #[test]
    fn mpp12_format_is_reported_by_name() {
        let bytes = compound_file_with_comp_obj("Microsoft Project 12.0", "MSProject.MPP12");
        let err = read_mpp_bytes(&bytes).unwrap_err();
        assert_eq!(err.kind(), "unsupported_version");
        assert!(err.to_string().contains("MPP12 (Project 2007)"));
    }

    #[test]
    fn mpp9_format_is_reported_by_name() {
        let bytes = compound_file_with_comp_obj("Microsoft Project 9.0", "MSProject.GLOBAL9");
        let err = read_mpp_bytes(&bytes).unwrap_err();
        assert_eq!(err.kind(), "unsupported_version");
        assert!(err.to_string().contains("MPP9 (Project 2000-2003)"));
    }

    #[test]
    fn mpp8_format_is_reported_by_name() {
        let bytes = compound_file_with_comp_obj("Microsoft Project 4.0", "MSProject.MPP4");
        let err = read_mpp_bytes(&bytes).unwrap_err();
        assert_eq!(err.kind(), "unsupported_version");
        assert!(err
            .to_string()
            .contains("MPP8 or earlier (Project 98 and prior)"));
    }

    #[test]
    fn missing_file_format_names_the_application() {
        let bytes = compound_file_with_comp_obj("Some Other Application", "");
        let err = read_mpp_bytes(&bytes).unwrap_err();
        assert_eq!(err.kind(), "unsupported_version");
        assert!(err.to_string().contains("Some Other Application"));
    }

    #[test]
    fn unrecognised_nonempty_format_is_reported_verbatim() {
        let bytes = compound_file_with_comp_obj("Weird App", "Something.Else");
        let err = read_mpp_bytes(&bytes).unwrap_err();
        assert_eq!(err.kind(), "unsupported_version");
        assert!(err.to_string().contains("Something.Else"));
    }

    #[test]
    fn not_a_compound_file_is_reported() {
        let err = read_mpp_bytes(&[0u8; 32]).unwrap_err();
        assert_eq!(err.kind(), "not_a_compound_file");
    }
}
