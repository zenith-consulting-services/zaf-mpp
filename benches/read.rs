//! Warm in-process parse time for `read_project_bytes`, one benchmark per
//! input file. Run with `cargo bench --bench read`.
//!
//! Inputs, each skipped if absent:
//! - MPXJ corpus files (`scripts/fetch-test-data.sh`): the largest MPP14
//!   outputs, the largest container, and the P6 XER/PMXML fixtures.
//! - The PathProof sample schedules (fictional, public), looked up in
//!   `$ZAF_MPP_BENCH_SAMPLES` or `../zaf-pathproof/website/public/samples`.
//!
//! The file is read into memory once; only parsing is timed.

use std::path::{Path, PathBuf};

use divan::{black_box, Bencher};

fn main() {
    divan::main();
}

const CORPUS: &[&str] = &[
    "generated/task-baselines/task-baselines-project2016-mpp14.mpp",
    "taskFlags-mpp14Project2013.mpp",
    "calendar-exception-precedence.mpp",
    "mpp14task.mpp",
    "generated/task-percentcomplete/percent-complete-8.4.xer",
    "generated/task-percentcomplete/percent-complete-8.4.pmxml",
];

const SAMPLES: &[&str] = &[
    "PathProof Example Project.mpp",
    "PathProof Example Project.xer",
];

fn corpus_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/mpxj/junit/data")
        .join(name)
}

fn sample_path(name: &str) -> PathBuf {
    std::env::var_os("ZAF_MPP_BENCH_SAMPLES")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../zaf-pathproof/website/public/samples")
        })
        .join(name)
}

fn bench_file(bencher: Bencher, path: &Path) {
    let Ok(bytes) = std::fs::read(path) else {
        eprintln!("skipping {}: not found", path.display());
        return;
    };
    // Fail loudly rather than timing an error path.
    zaf_mpp::read_project_bytes(&bytes).expect("benchmark input must parse");
    bencher.bench_local(|| zaf_mpp::read_project_bytes(black_box(&bytes)));
}

#[divan::bench(args = CORPUS)]
fn corpus(bencher: Bencher, name: &str) {
    bench_file(bencher, &corpus_path(name));
}

#[divan::bench(args = SAMPLES)]
fn sample(bencher: Bencher, name: &str) {
    bench_file(bencher, &sample_path(name));
}
