//! Output-stability guard: parse every schedule file in the MPXJ test
//! corpus with `read_project_bytes`, serialise the full parsed model (or
//! the error) to JSON, and compare a length + FNV-1a 64 digest of that
//! JSON against `tests/snapshots/corpus_digests.txt`.
//!
//! The digest file was generated from the reader as it stood before the
//! performance work (origin/main at e9d404f), so any optimisation that
//! changes a single byte of parsed output for any corpus file fails here.
//!
//! - Skips cleanly if the corpus has not been fetched
//!   (`scripts/fetch-test-data.sh`).
//! - `ZAF_MPP_SNAPSHOT_UPDATE=1` rewrites the digest file (only do this
//!   for an intentional output change).
//! - `ZAF_MPP_SNAPSHOT_DUMP=<dir>` also writes each file's full JSON into
//!   `<dir>`, for diffing two builds when a digest mismatches.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &["mpp", "mpt", "xer", "pmxml", "xml"];

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mpxj/junit/data")
}

fn digest_file() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/corpus_digests.txt")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        {
            out.push(path);
        }
    }
}

/// Full parsed output for one file: the serialised model, or the error.
fn render(bytes: &[u8]) -> String {
    match std::panic::catch_unwind(|| zaf_mpp::read_project_bytes(bytes)) {
        Ok(Ok(project)) => serde_json::to_string(&project).expect("serialise"),
        Ok(Err(err)) => format!("ERR {}: {err}", err.kind()),
        Err(_) => "PANIC".to_string(),
    }
}

#[test]
fn corpus_output_matches_pre_optimisation_snapshot() {
    let root = corpus_root();
    if !root.exists() {
        eprintln!("skipping: corpus not fetched, run scripts/fetch-test-data.sh first");
        return;
    }
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(!files.is_empty(), "no corpus files found");

    let dump_dir = std::env::var_os("ZAF_MPP_SNAPSHOT_DUMP").map(PathBuf::from);
    if let Some(dir) = &dump_dir {
        std::fs::create_dir_all(dir).expect("create dump dir");
    }

    let mut actual: BTreeMap<String, String> = BTreeMap::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(path).expect("read corpus file");
        let output = render(&bytes);
        if let Some(dir) = &dump_dir {
            std::fs::write(dir.join(rel.replace('/', "__") + ".json"), &output)
                .expect("write dump");
        }
        actual.insert(
            rel,
            format!("{}\t{:016x}", output.len(), fnv1a64(output.as_bytes())),
        );
    }

    if std::env::var_os("ZAF_MPP_SNAPSHOT_UPDATE").is_some() {
        let mut text = String::new();
        for (rel, digest) in &actual {
            writeln!(text, "{rel}\t{digest}").unwrap();
        }
        std::fs::create_dir_all(digest_file().parent().unwrap()).unwrap();
        std::fs::write(digest_file(), text).expect("write digest file");
        return;
    }

    let expected_text = std::fs::read_to_string(digest_file()).expect("read digest file");
    let expected: BTreeMap<String, String> = expected_text
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (rel, digest) = l.split_once('\t').expect("digest line");
            (rel.to_string(), digest.to_string())
        })
        .collect();

    let mut mismatches = Vec::new();
    for (rel, digest) in &expected {
        match actual.get(rel) {
            Some(d) if d == digest => {}
            Some(d) => mismatches.push(format!("{rel}: expected {digest}, got {d}")),
            None => mismatches.push(format!("{rel}: missing from corpus")),
        }
    }
    for rel in actual.keys() {
        if !expected.contains_key(rel) {
            mismatches.push(format!("{rel}: not in digest file"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} corpus file(s) changed output:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}
