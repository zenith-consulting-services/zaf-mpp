//! Integration tests against real Primavera P6 files from MPXJ's own test
//! corpus. Percent-complete expectations are ported from MPXJ's
//! `TaskPercentCompleteTest.testPrimaveraPercentComplete`; the structural
//! assertions are zaf-mpp's own, checked against the file contents.
//! Run `scripts/fetch-test-data.sh` first; these tests skip cleanly if the
//! corpus has not been fetched.

use std::path::{Path, PathBuf};

use zaf_mpp::Project;

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

fn task_by_name<'a>(project: &'a Project, name: &str) -> &'a zaf_mpp::model::Task {
    project
        .tasks
        .iter()
        .find(|t| t.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no task named {name:?}"))
}

/// Ported from `TaskPercentCompleteTest.testPrimaveraPercentComplete`
/// (src/test/java/org/mpxj/junit/task/TaskPercentCompleteTest.java): the
/// four "Duration N%" activities must report N as their duration percent
/// complete. (The Physical/Units rows exercise percent-complete fields
/// this crate does not model.)
fn assert_duration_percent_complete(project: &Project) {
    for expected in [0.0, 25.0, 75.0, 100.0] {
        let task = task_by_name(project, &format!("Duration {expected}%"));
        assert!(
            (task.percent_complete - expected).abs() < 1e-9,
            "{}: expected {expected}, got {}",
            task.name.as_deref().unwrap(),
            task.percent_complete
        );
    }
}

#[test]
fn xer_percent_complete() {
    let Some(path) = corpus_file("generated/task-percentcomplete/percent-complete-8.4.xer") else {
        return;
    };
    let project = zaf_mpp::read_xer(&path).expect("parse percent-complete-8.4.xer");
    assert_duration_percent_complete(&project);

    // 12 activities under the project WBS root.
    assert_eq!(project.tasks.iter().filter(|t| !t.summary).count(), 12);
    assert_eq!(project.tasks.iter().filter(|t| t.summary).count(), 1);
}

#[test]
fn pmxml_percent_complete() {
    let Some(path) = corpus_file("generated/task-percentcomplete/percent-complete-8.4.pmxml")
    else {
        return;
    };
    let project = zaf_mpp::read_pmxml(&path).expect("parse percent-complete-8.4.pmxml");
    assert_duration_percent_complete(&project);
    assert_eq!(project.tasks.iter().filter(|t| !t.summary).count(), 12);
}

#[test]
fn xer_and_pmxml_agree_on_percent_complete_fixture() {
    let (Some(xer_path), Some(pmxml_path)) = (
        corpus_file("generated/task-percentcomplete/percent-complete-8.4.xer"),
        corpus_file("generated/task-percentcomplete/percent-complete-8.4.pmxml"),
    ) else {
        return;
    };
    let from_xer = zaf_mpp::read_xer(&xer_path).expect("parse xer");
    let from_pmxml = zaf_mpp::read_pmxml(&pmxml_path).expect("parse pmxml");

    // Same schedule exported both ways: activity-level fields the shared
    // builder produces must agree across formats.
    for xer_task in from_xer.tasks.iter().filter(|t| !t.summary) {
        let name = xer_task.name.as_deref().unwrap();
        let pmxml_task = task_by_name(&from_pmxml, name);
        assert!(
            (xer_task.percent_complete - pmxml_task.percent_complete).abs() < 1e-9,
            "{name}: percent complete differs (xer {}, pmxml {})",
            xer_task.percent_complete,
            pmxml_task.percent_complete
        );
        assert_eq!(
            xer_task.milestone, pmxml_task.milestone,
            "{name}: milestone flag differs"
        );
        assert_eq!(
            xer_task.actual_start.is_some(),
            pmxml_task.actual_start.is_some(),
            "{name}: actual start presence differs"
        );
    }
}

/// Structural read of a genuine P6 8.2 XER export (CP1252-encoded, so this
/// also exercises the non-UTF-8 fallback: the currency table includes a
/// "£" symbol).
#[test]
fn xer_real_export_structure() {
    let Some(path) = corpus_file("PredecessorCalendar.xer") else {
        return;
    };
    let project = zaf_mpp::read_xer(&path).expect("parse PredecessorCalendar.xer");

    // One WBS root, one completed activity.
    assert_eq!(project.tasks.len(), 2);
    let root = &project.tasks[0];
    assert!(root.summary);
    assert_eq!(root.name.as_deref(), Some("TEST PROJECT"));
    let activity = &project.tasks[1];
    assert_eq!(activity.name.as_deref(), Some("Test"));
    assert!(activity.actual_finish.is_some());
    assert!(!activity.critical); // completed
    assert_eq!(activity.parent_task_unique_id, Some(root.unique_id));

    // The "5 Day" project calendar decoded from clndr_data: Monday works
    // 08:00-12:00 and 13:00-17:00, Sunday does not work.
    let calendar = project
        .calendars
        .iter()
        .find(|c| c.name.as_deref() == Some("5 Day"))
        .expect("5 Day calendar");
    let monday = calendar.days[1].as_ref().unwrap();
    assert!(monday.working);
    assert_eq!(monday.ranges.len(), 2);
    assert_eq!(monday.ranges[0].start_seconds, 8 * 3600);
    assert_eq!(monday.ranges[0].end_seconds, 12 * 3600);
    assert_eq!(monday.ranges[1].start_seconds, 13 * 3600);
    assert_eq!(monday.ranges[1].end_seconds, 17 * 3600);
    assert!(!calendar.days[0].as_ref().unwrap().working);

    // Calendar-derived period settings from the file's day/week counts.
    assert_eq!(project.properties.minutes_per_day, 480);
    assert_eq!(project.properties.minutes_per_week, 2400);
    assert_eq!(project.properties.currency_code.as_deref(), Some("USD"));
}

/// Structural read of the matching PMXML export.
#[test]
fn pmxml_real_export_structure() {
    let Some(path) = corpus_file("PredecessorCalendar.xml") else {
        return;
    };
    let project = zaf_mpp::read_pmxml(&path).expect("parse PredecessorCalendar.xml");

    assert!(!project.tasks.is_empty());
    let activity = project
        .tasks
        .iter()
        .find(|t| t.name.as_deref() == Some("Test"))
        .expect("Test activity");
    assert!(!activity.summary);
    assert!(!project.calendars.is_empty());
}

/// The format sniffer must route each real file to the right reader.
#[test]
fn read_project_detects_formats() {
    if let Some(path) = corpus_file("PredecessorCalendar.xer") {
        let project = zaf_mpp::read_project(&path).expect("detect + parse xer");
        assert!(!project.tasks.is_empty());
    }
    if let Some(path) = corpus_file("PredecessorCalendar.xml") {
        let project = zaf_mpp::read_project(&path).expect("detect + parse pmxml");
        assert!(!project.tasks.is_empty());
    }
    if let Some(path) = corpus_file("mpp14task.mpp") {
        let project = zaf_mpp::read_project(&path).expect("detect + parse mpp");
        assert!(!project.tasks.is_empty());
    }
}

/// Every P6 file in the corpus (including ones exercising vendor quirks
/// and deliberately invalid data) must parse or fail with a typed error —
/// never panic.
#[test]
fn all_corpus_p6_files_parse_or_fail_cleanly() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mpxj/junit/data");
    if !root.exists() {
        eprintln!("skipping: corpus not fetched");
        return;
    }

    let mut checked = 0;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            match ext.to_ascii_lowercase().as_str() {
                "xer" => {
                    checked += 1;
                    if let Err(err) = zaf_mpp::read_xer(&path) {
                        eprintln!("{} -> {err}", path.display());
                    }
                }
                "pmxml" => {
                    checked += 1;
                    if let Err(err) = zaf_mpp::read_pmxml(&path) {
                        eprintln!("{} -> {err}", path.display());
                    }
                }
                _ => {}
            }
        }
    }
    assert!(checked > 0, "no P6 files found in corpus");
}
