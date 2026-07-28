//! Owned representation of an MPP14 project. Field selection and semantics
//! follow MPXJ's `ProjectFile` object model, but the shapes here are
//! zaf-mpp's own: `Option` instead of null sentinels, no field-number
//! getters, plain enums instead of MPXJ's `FieldType` maps.

use serde::Serialize;

use crate::util::{MppDate, MppDateTime, MppDuration};

/// A fully parsed MPP14 project: properties, calendars, tasks, resources,
/// and assignments. Returned by [`crate::read_mpp`] and
/// [`crate::read_mpp_bytes`].
#[derive(Debug, Clone, Serialize)]
pub struct Project {
    pub properties: ProjectProperties,
    pub calendars: Vec<Calendar>,
    pub tasks: Vec<Task>,
    pub resources: Vec<Resource>,
    pub assignments: Vec<Assignment>,
}

/// Project-level settings. Ported from `ProjectPropertiesReader`, restricted
/// to the fields actually stored in the MPP14 `Props` block (see
/// `container::props`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ProjectProperties {
    pub guid: Option<[u8; 16]>,
    pub start_date: Option<MppDateTime>,
    pub finish_date: Option<MppDateTime>,
    pub status_date: Option<MppDateTime>,
    pub default_start_time_seconds: Option<u32>,
    pub default_end_time_seconds: Option<u32>,
    pub minutes_per_day: i32,
    pub minutes_per_week: i32,
    pub days_per_month: i32,
    pub currency_symbol: Option<String>,
    pub currency_code: Option<String>,
    pub currency_digits: i32,
    pub week_start_day: i32,
    pub default_calendar_name: Option<String>,
    pub honor_constraints: bool,
    /// The project title as set in the Project Information dialog. MS
    /// Project also writes this to the OLE `SummaryInformation` property
    /// set; zaf-mpp reads only the `Props` copy (see
    /// `reader::mpp14::props_key::TITLE`), which is present whenever the
    /// title has been set through the Project Information dialog.
    pub title: Option<String>,
    /// Below this amount of total slack, a task counts as critical. Stored
    /// directly in days (Microsoft Project's default is 0).
    pub critical_slack_limit_days: i32,
    /// Application version detected from the `\1CompObj` stream, e.g. 14.
    pub application_version: Option<u32>,
}

/// Task scheduling mode: whether MS Project auto-schedules dates and
/// duration, or the user has pinned them manually.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TaskMode {
    AutoScheduled,
    ManuallyScheduled,
}

/// Task constraint type. Ordinal values match MPXJ's `ConstraintType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ConstraintType {
    AsSoonAsPossible,
    AsLateAsPossible,
    MustStartOn,
    MustFinishOn,
    StartNoEarlierThan,
    StartNoLaterThan,
    FinishNoEarlierThan,
    FinishNoLaterThan,
    StartOn,
    FinishOn,
}

impl ConstraintType {
    pub(crate) fn from_mpp(value: i32) -> ConstraintType {
        match value {
            1 => ConstraintType::AsLateAsPossible,
            2 => ConstraintType::MustStartOn,
            3 => ConstraintType::MustFinishOn,
            4 => ConstraintType::StartNoEarlierThan,
            5 => ConstraintType::StartNoLaterThan,
            6 => ConstraintType::FinishNoEarlierThan,
            7 => ConstraintType::FinishNoLaterThan,
            8 => ConstraintType::StartOn,
            9 => ConstraintType::FinishOn,
            _ => ConstraintType::AsSoonAsPossible,
        }
    }
}

/// Values for baseline 0 (the primary baseline). Baselines 1-10 use
/// [`Task::baselines`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct Baseline {
    pub start: Option<MppDateTime>,
    pub finish: Option<MppDateTime>,
    pub duration: Option<MppDuration>,
    pub work: Option<MppDuration>,
    pub cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub unique_id: i32,
    pub id: i32,
    pub name: Option<String>,
    pub outline_level: i32,
    pub parent_task_unique_id: Option<i32>,
    pub wbs: Option<String>,

    pub start: Option<MppDateTime>,
    pub finish: Option<MppDateTime>,
    pub actual_start: Option<MppDateTime>,
    pub actual_finish: Option<MppDateTime>,
    pub duration: Option<MppDuration>,
    pub actual_duration: Option<MppDuration>,
    pub work: Option<MppDuration>,
    pub actual_work: Option<MppDuration>,
    pub percent_complete: f64,

    /// Earliest possible start date given dependencies and constraints, as
    /// computed by MS Project's own scheduler and stored in the file (not
    /// recomputed by zaf-mpp).
    pub early_start: Option<MppDateTime>,
    pub early_finish: Option<MppDateTime>,
    pub late_start: Option<MppDateTime>,
    pub late_finish: Option<MppDateTime>,
    pub free_slack: Option<MppDuration>,
    pub start_slack: Option<MppDuration>,
    pub finish_slack: Option<MppDuration>,
    /// The lesser of `start_slack` and `finish_slack`, or `finish_slack`
    /// alone once the task has actually started. Ported from
    /// `MicrosoftSlackCalculator.calculateTotalSlack`.
    pub total_slack: Option<MppDuration>,
    /// True if the task has no room to slip without delaying the project
    /// finish. Ported from `Task.calculateCritical`; see that method for
    /// the full rule this is a close approximation of.
    pub critical: bool,

    pub milestone: bool,
    /// True once this task's children have been linked up during reading;
    /// a task with children is a "summary" task.
    pub summary: bool,
    pub task_mode: TaskMode,
    pub constraint_type: ConstraintType,
    pub constraint_date: Option<MppDateTime>,

    pub calendar_unique_id: Option<i32>,
    pub cost: Option<f64>,
    pub actual_cost: Option<f64>,
    /// Plain-text notes with RTF formatting stripped; see `crate::rtf`.
    pub notes: Option<String>,

    pub predecessors: Vec<Relation>,

    /// Baseline 0 (the primary baseline).
    pub baseline: Baseline,
    /// Baselines 1 through 10, indexed 0..=9.
    pub baselines: [Baseline; 10],
}

/// Task dependency type. Ordinal values match MPXJ's `RelationType`
/// (0 = FF, 1 = FS, 2 = SF, 3 = SS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RelationType {
    FinishFinish,
    FinishStart,
    StartFinish,
    StartStart,
}

impl RelationType {
    pub(crate) fn from_mpp(value: i32) -> RelationType {
        match value {
            0 => RelationType::FinishFinish,
            2 => RelationType::StartFinish,
            3 => RelationType::StartStart,
            _ => RelationType::FinishStart,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Relation {
    pub unique_id: i32,
    pub predecessor_task_unique_id: i32,
    pub successor_task_unique_id: i32,
    pub relation_type: RelationType,
    pub lag: MppDuration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ResourceType {
    Work,
    Material,
    Cost,
}

#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    pub unique_id: i32,
    pub id: i32,
    pub name: Option<String>,
    pub initials: Option<String>,
    pub group: Option<String>,
    pub email_address: Option<String>,
    pub resource_type: ResourceType,
    pub standard_rate_per_hour: f64,
    pub overtime_rate_per_hour: f64,
    pub max_units: f64,
    pub cost: Option<f64>,
    pub work: Option<MppDuration>,
    pub calendar_unique_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Assignment {
    pub unique_id: i32,
    pub task_unique_id: i32,
    pub resource_unique_id: Option<i32>,
    pub start: Option<MppDateTime>,
    pub finish: Option<MppDateTime>,
    pub work: Option<MppDuration>,
    pub actual_work: Option<MppDuration>,
    pub units: f64,
    pub cost: Option<f64>,
    pub actual_cost: Option<f64>,
}

/// A single working-time range within a day, in seconds since midnight.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TimeRange {
    pub start_seconds: u32,
    pub end_seconds: u32,
}

/// Working hours for one day of the week. `None` means the calendar has no
/// override for this day and the parent (base) calendar's day applies.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DayHours {
    pub working: bool,
    pub ranges: Vec<TimeRange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalendarException {
    pub from_date: Option<MppDate>,
    pub to_date: Option<MppDate>,
    pub working: bool,
    pub ranges: Vec<TimeRange>,
    pub name: Option<String>,
}

/// Working time calendar. Days are indexed Sunday = 0 through Saturday = 6,
/// matching MPXJ's day-of-week ordering for MPP calendar blocks.
#[derive(Debug, Clone, Serialize)]
pub struct Calendar {
    pub unique_id: i32,
    pub name: Option<String>,
    pub base_calendar_unique_id: Option<i32>,
    /// Always exactly 7 entries, Sunday first. A `Vec` rather than a fixed
    /// array so the type mirrors cleanly across an FRB bridge (fixed-size
    /// arrays of `Option<T>` are not well supported by flutter_rust_bridge
    /// 2.11's codegen).
    pub days: Vec<Option<DayHours>>,
    pub exceptions: Vec<CalendarException>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_type_from_mpp_covers_every_stored_value() {
        assert_eq!(
            ConstraintType::from_mpp(0),
            ConstraintType::AsSoonAsPossible
        );
        assert_eq!(
            ConstraintType::from_mpp(1),
            ConstraintType::AsLateAsPossible
        );
        assert_eq!(ConstraintType::from_mpp(2), ConstraintType::MustStartOn);
        assert_eq!(ConstraintType::from_mpp(3), ConstraintType::MustFinishOn);
        assert_eq!(
            ConstraintType::from_mpp(4),
            ConstraintType::StartNoEarlierThan
        );
        assert_eq!(
            ConstraintType::from_mpp(5),
            ConstraintType::StartNoLaterThan
        );
        assert_eq!(
            ConstraintType::from_mpp(6),
            ConstraintType::FinishNoEarlierThan
        );
        assert_eq!(
            ConstraintType::from_mpp(7),
            ConstraintType::FinishNoLaterThan
        );
        assert_eq!(ConstraintType::from_mpp(8), ConstraintType::StartOn);
        assert_eq!(ConstraintType::from_mpp(9), ConstraintType::FinishOn);
    }

    #[test]
    fn constraint_type_from_mpp_defaults_to_asap_for_unknown_values() {
        assert_eq!(
            ConstraintType::from_mpp(10),
            ConstraintType::AsSoonAsPossible
        );
        assert_eq!(
            ConstraintType::from_mpp(-1),
            ConstraintType::AsSoonAsPossible
        );
        assert_eq!(
            ConstraintType::from_mpp(9999),
            ConstraintType::AsSoonAsPossible
        );
    }

    #[test]
    fn relation_type_from_mpp_covers_every_stored_value() {
        assert_eq!(RelationType::from_mpp(0), RelationType::FinishFinish);
        assert_eq!(RelationType::from_mpp(1), RelationType::FinishStart);
        assert_eq!(RelationType::from_mpp(2), RelationType::StartFinish);
        assert_eq!(RelationType::from_mpp(3), RelationType::StartStart);
    }

    #[test]
    fn relation_type_from_mpp_defaults_to_finish_start_for_unknown_values() {
        assert_eq!(RelationType::from_mpp(4), RelationType::FinishStart);
        assert_eq!(RelationType::from_mpp(-1), RelationType::FinishStart);
    }
}
