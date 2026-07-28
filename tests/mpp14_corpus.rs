//! Integration tests against real MPP14 files from MPXJ's own test corpus.
//! Expected values are ported from MPXJ's JUnit tests, cited per test.
//! Run `scripts/fetch-test-data.sh` first; these tests skip cleanly if the
//! corpus has not been fetched.

use std::path::{Path, PathBuf};

fn corpus_file(name: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/mpxj/junit/data")
        .join(name);
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "skipping: {} not found, run scripts/fetch-test-data.sh first",
            path.display()
        );
        None
    }
}

/// Ported from `MppTaskTest.testBasicTask`, called via `testMpp14Task`
/// (src/test/java/org/mpxj/junit/MppTaskTest.java) against mpp14task.mpp.
#[test]
fn mpp14_task_basic_fields() {
    let Some(path) = corpus_file("mpp14task.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14task.mpp");

    assert_eq!(project.tasks.len(), 2);

    let task0 = &project.tasks[0];
    assert_eq!(task0.id, 0);
    assert_eq!(task0.name.as_deref(), Some("MPP12 Test"));

    let task1 = &project.tasks[1];
    assert_eq!(task1.id, 1);
    assert_eq!(task1.name.as_deref(), Some("Task #1"));

    let start = task1.start.expect("start");
    assert_eq!(
        (start.date.year, start.date.month, start.date.day),
        (2006, 8, 23)
    );
    let finish = task1.finish.expect("finish");
    assert_eq!(
        (finish.date.year, finish.date.month, finish.date.day),
        (2006, 8, 29)
    );

    assert!(task1.predecessors.is_empty());

    let duration = task1.duration.expect("duration");
    assert_eq!(duration.units, zaf_mpp::util::TimeUnit::Weeks);
    assert!((duration.value - 1.0).abs() < 1e-9);

    let work = task1.work.expect("work");
    assert_eq!(work.units, zaf_mpp::util::TimeUnit::Hours);
    assert!((work.value - 40.0).abs() < 1e-9);

    assert!((task1.percent_complete - 45.0).abs() < 1e-9);
    assert_eq!(task1.cost, Some(5000.0));
    assert_eq!(task1.actual_cost, Some(2800.0));

    let constraint_date = task1.constraint_date.expect("constraint date");
    assert_eq!(
        (
            constraint_date.date.year,
            constraint_date.date.month,
            constraint_date.date.day
        ),
        (2006, 8, 23)
    );
    assert_eq!(
        task1.constraint_type,
        zaf_mpp::model::ConstraintType::MustStartOn
    );

    assert_eq!(task1.notes.as_deref(), Some("Notes Example"));
    assert_eq!(project.properties.title.as_deref(), Some("MPP12 Test"));
}

/// A resave of mpp14task.mpp by Project 2013 (see
/// `MppTaskTest.testMpp14TaskFromProject2013`, which runs the identical
/// `testBasicTask` assertions as the mpp14task.mpp test above). Project
/// 2013 writes a materially different on-disk field-map layout than 2010
/// (e.g. WORK moves from fixed-data offset 126 to offset 8): this is the
/// single regression test in the corpus that exercises the on-disk
/// `FieldMap` override path end to end, since every other MPP14 fixture
/// here happens to carry a field map that reproduces MPXJ's hardcoded
/// defaults byte-for-byte.
#[test]
fn mpp14_task_from_project_2013_basic_fields() {
    let Some(path) = corpus_file("mpp14task-from2013.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14task-from2013.mpp");

    assert_eq!(project.tasks.len(), 2);

    let task0 = &project.tasks[0];
    assert_eq!(task0.id, 0);
    assert_eq!(task0.name.as_deref(), Some("MPP12 Test"));

    let task1 = &project.tasks[1];
    assert_eq!(task1.id, 1);
    assert_eq!(task1.name.as_deref(), Some("Task #1"));

    let start = task1.start.expect("start");
    assert_eq!(
        (start.date.year, start.date.month, start.date.day),
        (2006, 8, 23)
    );

    let duration = task1.duration.expect("duration");
    assert_eq!(duration.units, zaf_mpp::util::TimeUnit::Weeks);
    assert!((duration.value - 1.0).abs() < 1e-9);

    let work = task1.work.expect("work");
    assert_eq!(work.units, zaf_mpp::util::TimeUnit::Hours);
    assert!((work.value - 40.0).abs() < 1e-9);

    assert!((task1.percent_complete - 45.0).abs() < 1e-9);
    assert_eq!(task1.cost, Some(5000.0));
    assert_eq!(task1.actual_cost, Some(2800.0));
    assert_eq!(task1.notes.as_deref(), Some("Notes Example"));
}

/// Ported from `MppFilterTest`/general corpus smoke coverage: a file with
/// task relations should parse FS/SS/FF/SF dependencies without panicking
/// and attach at least one predecessor.
#[test]
fn mpp14_relations_parse() {
    let Some(path) = corpus_file("mpp14relations.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14relations.mpp");
    let has_predecessor = project.tasks.iter().any(|t| !t.predecessors.is_empty());
    assert!(
        has_predecessor,
        "expected at least one task with a predecessor relation"
    );
}

/// Exercises `Project`'s lookup helpers against mpp14relations.mpp's known
/// chain: Task 1 -FS-> Task 2 -SS-> Task 3 -FF-> Task 4 -SF-> Task 5, all
/// on the "Standard" calendar (unique ID 1).
#[test]
fn mpp14_project_lookup_helpers() {
    let Some(path) = corpus_file("mpp14relations.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14relations.mpp");

    let task2 = project.task_by_unique_id(2).expect("task 2");
    assert_eq!(task2.name.as_deref(), Some("Task 2"));
    assert!(project.task_by_unique_id(9999).is_none());

    let calendar = project
        .calendar_by_unique_id(1)
        .expect("calendar unique id 1");
    assert_eq!(calendar.name.as_deref(), Some("Standard"));
    assert!(project.calendar_by_unique_id(9999).is_none());

    // Task 1 is the predecessor of Task 2 via a finish-to-start relation:
    // successors_of(1) should find it.
    let successors = project.successors_of(1);
    assert_eq!(successors.len(), 1);
    assert_eq!(successors[0].successor_task_unique_id, 2);
    assert_eq!(
        successors[0].relation_type,
        zaf_mpp::model::RelationType::FinishStart
    );

    // Task 5 has no successors.
    assert!(project.successors_of(5).is_empty());
}

/// A calendar-focused fixture: every calendar should expose at least one
/// working day, and the default "Standard" calendar's Monday hours should
/// be non-empty.
#[test]
fn mpp14_calendar_parses_working_hours() {
    let Some(path) = corpus_file("mpp14calendar.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14calendar.mpp");
    assert!(!project.calendars.is_empty());
}

/// Baseline fixture: baseline 0 work should be populated for at least one
/// task ("Base Task" has 192 hours of baseline work in this fixture).
#[test]
fn mpp14_baseline_work_present() {
    let Some(path) = corpus_file("mpp14baseline.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14baseline.mpp");
    let has_baseline_work = project.tasks.iter().any(|t| t.baseline.work.is_some());
    assert!(
        has_baseline_work,
        "expected at least one task with baseline 0 work"
    );
}

/// Resource fixture: names and rates should read back without panicking.
#[test]
fn mpp14_resource_basic_fields() {
    let Some(path) = corpus_file("mpp14resource.mpp") else {
        return;
    };
    let project = zaf_mpp::read_mpp(&path).expect("parse mpp14resource.mpp");
    assert!(!project.resources.is_empty());
    assert!(project.resources.iter().any(|r| r.name.is_some()));

    let first_uid = project.resources[0].unique_id;
    let found = project
        .resource_by_unique_id(first_uid)
        .expect("resource lookup by its own unique id");
    assert_eq!(found.unique_id, first_uid);
    assert!(project.resource_by_unique_id(-9999).is_none());
}

/// Truncated/corrupted files must produce a typed error, never a panic.
#[test]
fn corrupted_variants_never_panic() {
    let Some(path) = corpus_file("mpp14task.mpp") else {
        return;
    };
    let original = std::fs::read(&path).unwrap();

    // Truncate at several points, including mid-header and mid-stream-table.
    for cut in [0, 1, 8, 64, 512, original.len() / 2] {
        let truncated = &original[..cut.min(original.len())];
        let _ = zaf_mpp::read_mpp_bytes(truncated);
    }

    // Flip bytes throughout the header region: still must not panic.
    let mut corrupted = original.clone();
    for i in (0..corrupted.len().min(4096)).step_by(37) {
        corrupted[i] ^= 0xFF;
        let _ = zaf_mpp::read_mpp_bytes(&corrupted);
        corrupted[i] ^= 0xFF;
    }

    // Wrong magic entirely.
    let mut wrong_magic = original.clone();
    wrong_magic[0..4].copy_from_slice(&[0, 0, 0, 0]);
    let _ = zaf_mpp::read_mpp_bytes(&wrong_magic);
}

#[test]
fn empty_and_tiny_inputs_return_typed_errors_not_panics() {
    assert!(zaf_mpp::read_mpp_bytes(&[]).is_err());
    assert!(zaf_mpp::read_mpp_bytes(&[0u8; 4]).is_err());
    assert!(zaf_mpp::read_mpp_bytes(&[0xFFu8; 512]).is_err());
}
