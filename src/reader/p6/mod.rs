//! Ported from MPXJ's Primavera reader family:
//! src/main/java/org/mpxj/primavera/ (TableProjectReader, TableContextReader,
//! XerProjectReader, XmlProjectReader, XmlReaderHelper, and the small
//! `*Helper` enum-mapping classes). Copyright (c) Packwood Software, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Shared semantic layer for the two Primavera P6 formats this crate reads:
//! XER (`reader::p6::xer`) and PMXML (`reader::p6::pmxml`). Both parsers
//! produce the intermediate [`P6Data`] representation defined here, and
//! [`build_project`] maps that onto the crate's [`crate::model::Project`] —
//! so the P6-to-MPP semantic decisions live in exactly one place and the two
//! formats agree by construction.
//!
//! Scope: like the MPP14 reader, this is a deliberate subset of MPXJ,
//! restricted to the fields `model::Project` carries. Activity codes, UDFs,
//! expense items, steps, roles, shifts and cross-project relations are out
//! of scope. Where MPXJ needs a working-time engine this crate doesn't have
//! (actual-duration and at-completion-duration calculations against the
//! task calendar), simpler arithmetic on the stored hour counts is used
//! instead — each such divergence is documented at the point it happens.

pub(crate) mod pmxml;
pub(crate) mod structured_text;
pub(crate) mod xer;

mod build;

pub(crate) use build::build_project;

use crate::model::{ConstraintType, RelationType, ResourceType};
use crate::util::{MppDate, MppDateTime, MppDuration, TimeUnit};

/// Everything extracted from a P6 file, before mapping to the MPP-shaped
/// model. Field names follow the XER column names, since those are the
/// terser of the two formats' vocabularies; the PMXML parser translates its
/// element names into these.
#[derive(Debug, Default)]
pub(crate) struct P6Data {
    pub projects: Vec<P6Project>,
    pub calendars: Vec<P6Calendar>,
    pub wbs: Vec<P6Wbs>,
    pub activities: Vec<P6Activity>,
    pub relations: Vec<P6Relation>,
    pub resources: Vec<P6Resource>,
    pub assignments: Vec<P6Assignment>,
    /// Global preferences (PMXML only): hours per day/week/month, week
    /// start day. XER has no global equivalent; the builder falls back to
    /// the default calendar's hour counts.
    pub hours_per_day: Option<f64>,
    pub hours_per_week: Option<f64>,
    pub hours_per_month: Option<f64>,
    pub week_start_day: Option<i32>,
    /// Default currency (XER `currtype` row matching the header currency,
    /// PMXML base currency).
    pub currency_symbol: Option<String>,
    pub currency_code: Option<String>,
    pub currency_digits: Option<i32>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Project {
    pub proj_id: Option<i32>,
    pub short_name: Option<String>,
    pub export_flag: bool,
    /// P6 "data date" — maps to the MPP status date.
    pub last_recalc_date: Option<MppDateTime>,
    pub plan_start_date: Option<MppDateTime>,
    pub scd_end_date: Option<MppDateTime>,
    /// Total-float threshold (hours) below which an activity is critical.
    pub critical_drtn_hr_cnt: Option<f64>,
    /// "Longest path" rather than total-float criticality, if true.
    pub critical_path_type_is_longest_path: bool,
    pub clndr_id: Option<i32>,
    pub wbs_code_separator: Option<String>,
    pub guid: Option<[u8; 16]>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Wbs {
    pub wbs_id: i32,
    pub parent_wbs_id: Option<i32>,
    pub proj_id: Option<i32>,
    pub name: Option<String>,
    pub short_name: Option<String>,
    pub seq_num: Option<i32>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Activity {
    pub task_id: i32,
    pub proj_id: Option<i32>,
    pub wbs_id: Option<i32>,
    pub name: Option<String>,
    /// Activity ID as displayed in P6 (e.g. "A1010").
    pub task_code: Option<String>,
    /// XER `task_type` code (`TT_Task`, `TT_Mile`, ...). The PMXML parser
    /// translates the XML names ("Task Dependent", "Start Milestone", ...)
    /// into these codes.
    pub task_type: Option<String>,
    /// XER `status_code` (`TK_NotStart`, `TK_Active`, `TK_Complete`).
    pub status_code: Option<String>,
    pub clndr_id: Option<i32>,

    pub target_drtn_hr_cnt: Option<f64>,
    pub remain_drtn_hr_cnt: Option<f64>,
    /// At-completion duration in hours (PMXML only; XER derives it).
    pub at_completion_drtn_hr_cnt: Option<f64>,
    /// Duration % complete 0-100 (PMXML only; XER derives it).
    pub duration_pct_complete: Option<f64>,

    pub act_start_date: Option<MppDateTime>,
    pub act_end_date: Option<MppDateTime>,
    /// Remaining early start ("restart_date") / finish ("reend_date").
    pub restart_date: Option<MppDateTime>,
    pub reend_date: Option<MppDateTime>,
    pub target_start_date: Option<MppDateTime>,
    pub target_end_date: Option<MppDateTime>,
    pub early_start_date: Option<MppDateTime>,
    pub early_end_date: Option<MppDateTime>,
    pub late_start_date: Option<MppDateTime>,
    pub late_end_date: Option<MppDateTime>,
    /// Start/Finish as exported (PMXML only; XER derives them).
    pub start_date: Option<MppDateTime>,
    pub finish_date: Option<MppDateTime>,

    pub cstr_type: Option<String>,
    pub cstr_date: Option<MppDateTime>,

    pub total_float_hr_cnt: Option<f64>,
    pub free_float_hr_cnt: Option<f64>,
    /// On the longest ("driving") path, per P6's own scheduler.
    pub driving_path: bool,

    /// Labor + nonlabor unit counts, all in hours.
    pub act_work_qty: Option<f64>,
    pub act_equip_qty: Option<f64>,
    pub remain_work_qty: Option<f64>,
    pub remain_equip_qty: Option<f64>,
    pub target_work_qty: Option<f64>,
    pub target_equip_qty: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Relation {
    pub task_pred_id: Option<i32>,
    /// Successor activity.
    pub task_id: i32,
    /// Predecessor activity.
    pub pred_task_id: i32,
    /// XER `pred_type` code (`PR_FS`, ...); PMXML names are translated.
    pub pred_type: Option<String>,
    pub lag_hr_cnt: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Resource {
    pub rsrc_id: i32,
    pub name: Option<String>,
    pub short_name: Option<String>,
    pub email_addr: Option<String>,
    /// XER `rsrc_type` (`RT_Labor`, `RT_Mat`, `RT_Equip`); PMXML names are
    /// translated.
    pub rsrc_type: Option<String>,
    pub clndr_id: Option<i32>,
    /// Standard rate per hour, from the most recent rate-table entry.
    pub cost_per_qty: Option<f64>,
    /// Max units per hour as a fraction (1.0 == 100%), from the most
    /// recent rate-table entry.
    pub max_qty_per_hr: Option<f64>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Assignment {
    pub taskrsrc_id: i32,
    pub task_id: i32,
    pub rsrc_id: Option<i32>,
    pub act_start_date: Option<MppDateTime>,
    pub act_end_date: Option<MppDateTime>,
    pub restart_date: Option<MppDateTime>,
    pub reend_date: Option<MppDateTime>,
    pub target_start_date: Option<MppDateTime>,
    pub target_end_date: Option<MppDateTime>,
    /// Unit counts in hours.
    pub remain_qty: Option<f64>,
    pub act_reg_qty: Option<f64>,
    pub act_ot_qty: Option<f64>,
    pub target_cost: Option<f64>,
    pub remain_cost: Option<f64>,
    pub act_reg_cost: Option<f64>,
    pub act_ot_cost: Option<f64>,
    /// Planned units per hour as a fraction (1.0 == 100%).
    pub target_qty_per_hr: Option<f64>,
}

/// Working hours for one day: list of (start, end) second-of-day ranges.
/// `None` end-of-list means non-working.
#[derive(Debug, Default, Clone)]
pub(crate) struct P6CalendarDay {
    /// Explicitly present in the source data. When false the builder
    /// applies the format's default (XER: non-working; PMXML: Mon-Fri
    /// 08:00-16:00 working, weekend non-working), matching MPXJ.
    pub present: bool,
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug)]
pub(crate) struct P6CalendarException {
    pub date: MppDate,
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Debug, Default)]
pub(crate) struct P6Calendar {
    pub clndr_id: i32,
    pub name: Option<String>,
    pub base_clndr_id: Option<i32>,
    pub is_default: bool,
    /// Days indexed Sunday = 0 through Saturday = 6, matching
    /// [`crate::model::Calendar`].
    pub days: [P6CalendarDay; 7],
    pub exceptions: Vec<P6CalendarException>,
    pub day_hr_cnt: Option<f64>,
    pub week_hr_cnt: Option<f64>,
    pub month_hr_cnt: Option<f64>,
}

/// Duration expressed in hours, the unit P6 stores every duration in.
pub(crate) fn hours(value: f64) -> MppDuration {
    MppDuration {
        value,
        units: TimeUnit::Hours,
    }
}

/// Ported from `RelationTypeHelper.getInstanceFromXer` (the XML names are
/// translated to XER codes by the PMXML parser). MPXJ truncates codes to 5
/// characters before the lookup to tolerate vendor variants like
/// "PR_FS1"; unknown codes default to finish-start.
pub(crate) fn relation_type_from_xer(value: Option<&str>) -> RelationType {
    let code = value.map(|v| if v.len() > 5 { &v[..5] } else { v });
    match code {
        Some("PR_FF") => RelationType::FinishFinish,
        Some("PR_SS") => RelationType::StartStart,
        Some("PR_SF") => RelationType::StartFinish,
        _ => RelationType::FinishStart,
    }
}

/// Ported from `ConstraintTypeHelper.getInstanceFromXer`. `None` (no
/// constraint) maps to as-soon-as-possible, the model's default.
pub(crate) fn constraint_type_from_xer(value: Option<&str>) -> ConstraintType {
    match value {
        Some("CS_MSO") => ConstraintType::StartOn,
        Some("CS_MSOB") => ConstraintType::StartNoLaterThan,
        Some("CS_MSOA") => ConstraintType::StartNoEarlierThan,
        Some("CS_MEO") => ConstraintType::FinishOn,
        Some("CS_MEOB") => ConstraintType::FinishNoLaterThan,
        Some("CS_MEOA") => ConstraintType::FinishNoEarlierThan,
        Some("CS_ALAP") => ConstraintType::AsLateAsPossible,
        Some("CS_MANDSTART") => ConstraintType::MustStartOn,
        Some("CS_MANDFIN") => ConstraintType::MustFinishOn,
        _ => ConstraintType::AsSoonAsPossible,
    }
}

/// Ported from `ResourceTypeHelper.getInstanceFromXer`, with one deliberate
/// divergence: MPXJ maps `RT_Equip` to its NON_LABOR type, which this
/// crate's model (restricted to MS Project's Work/Material/Cost) doesn't
/// have. Equipment behaves like labor for scheduling purposes (it has a
/// calendar and hourly units), so it maps to `Work` here.
pub(crate) fn resource_type_from_xer(value: Option<&str>) -> ResourceType {
    match value {
        Some("RT_Mat") => ResourceType::Material,
        _ => ResourceType::Work,
    }
}

/// Milestone flag by activity type, ported from the `MILESTONE_MAP` in
/// `TableProjectReader` / `XmlProjectReader`.
pub(crate) fn is_milestone(task_type: Option<&str>) -> bool {
    matches!(task_type, Some("TT_Mile") | Some("TT_FinMile"))
}

/// Translate a PMXML activity type name to the XER code used throughout
/// this module. Ported from `ActivityTypeHelper`.
pub(crate) fn activity_type_from_xml(value: &str) -> Option<&'static str> {
    match value {
        "Task Dependent" => Some("TT_Task"),
        "Resource Dependent" => Some("TT_Rsrc"),
        "Level of Effort" => Some("TT_LOE"),
        "Start Milestone" => Some("TT_Mile"),
        "Finish Milestone" => Some("TT_FinMile"),
        "WBS Summary" => Some("TT_WBS"),
        _ => None,
    }
}

/// Translate a PMXML activity status name to the XER code. Ported from
/// `ActivityStatusHelper`.
pub(crate) fn activity_status_from_xml(value: &str) -> Option<&'static str> {
    match value {
        "Not Started" => Some("TK_NotStart"),
        "In Progress" => Some("TK_Active"),
        "Completed" => Some("TK_Complete"),
        _ => None,
    }
}

/// Translate a PMXML relationship type name to the XER code. Ported from
/// `RelationTypeHelper`.
pub(crate) fn relation_type_from_xml(value: &str) -> Option<&'static str> {
    match value {
        "Finish to Start" => Some("PR_FS"),
        "Finish to Finish" => Some("PR_FF"),
        "Start to Start" => Some("PR_SS"),
        "Start to Finish" => Some("PR_SF"),
        _ => None,
    }
}

/// Translate a PMXML constraint type name to the XER code. Ported from
/// `ConstraintTypeHelper`'s XML map.
pub(crate) fn constraint_type_from_xml(value: &str) -> Option<&'static str> {
    match value {
        "Start On" => Some("CS_MSO"),
        "Start On or Before" => Some("CS_MSOB"),
        "Start On or After" => Some("CS_MSOA"),
        "Finish On" => Some("CS_MEO"),
        "Finish On or Before" => Some("CS_MEOB"),
        "Finish On or After" => Some("CS_MEOA"),
        "As Late As Possible" => Some("CS_ALAP"),
        "Mandatory Start" => Some("CS_MANDSTART"),
        "Mandatory Finish" => Some("CS_MANDFIN"),
        _ => None,
    }
}

/// Translate a PMXML resource type name to the XER code. Ported from
/// `ResourceTypeHelper`'s XML map.
pub(crate) fn resource_type_from_xml(value: &str) -> Option<&'static str> {
    match value {
        "Labor" => Some("RT_Labor"),
        "Material" => Some("RT_Mat"),
        "Nonlabor" => Some("RT_Equip"),
        _ => None,
    }
}

/// Parse a P6 timestamp. Accepts the XER form (`2023-04-17 08:00`, with
/// optional seconds and `.` accepted in place of `:`, per MPXJ's pattern
/// `yyyy-M-dd[ HH[:][.]mm[[:][.]ss]]`) and the PMXML ISO form
/// (`2023-04-17T08:00:00`).
pub(crate) fn parse_datetime(text: &str) -> Option<MppDateTime> {
    let text = text.trim();
    let (date_part, time_part) = match text.find([' ', 'T']) {
        Some(idx) => (&text[..idx], Some(&text[idx + 1..])),
        None => (text, None),
    };

    let date = parse_date(date_part)?;
    let seconds = match time_part {
        Some(t) if !t.is_empty() => parse_time_of_day_24h(t)?,
        _ => 0,
    };
    Some(MppDateTime {
        date,
        seconds_since_midnight: seconds,
    })
}

/// Parse a `YYYY-M-D` date.
pub(crate) fn parse_date(text: &str) -> Option<MppDate> {
    let mut parts = text.trim().split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(MppDate::new(year, month, day))
}

/// Parse `HH:MM`, `HH.MM`, `HH:MM:SS` into seconds since midnight.
fn parse_time_of_day_24h(text: &str) -> Option<u32> {
    let mut parts = text.trim().split([':', '.']);
    let hour: u32 = parts.next()?.parse().ok()?;
    let minute: u32 = parts.next()?.parse().ok()?;
    let second: u32 = match parts.next() {
        Some(s) => s.parse().ok()?,
        None => 0,
    };
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(hour * 3600 + minute * 60 + second)
}

/// Parse a calendar working-hours time: 24-hour `H:mm` when there is no
/// AM/PM suffix, 12-hour `h:mm a` when there is. Ported from
/// `TableContextReader`'s twin formatters.
pub(crate) fn parse_calendar_time(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(idx) = text.find(' ') {
        let (time, suffix) = text.split_at(idx);
        let seconds = parse_time_of_day_24h(time)?;
        let hour = seconds / 3600;
        let rest = seconds % 3600;
        match suffix.trim().to_ascii_uppercase().as_str() {
            "AM" => {
                let hour = if hour == 12 { 0 } else { hour };
                if hour > 11 {
                    return None;
                }
                Some(hour * 3600 + rest)
            }
            "PM" => {
                let hour = if hour == 12 { 12 } else { hour + 12 };
                if hour > 23 {
                    return None;
                }
                Some(hour * 3600 + rest)
            }
            _ => None,
        }
    } else {
        parse_time_of_day_24h(text)
    }
}

/// Epoch for XER calendar exception dates (`d` attribute): days since
/// 1899-12-30, matching `TableContextReader.EXCEPTION_EPOCH`.
pub(crate) const EXCEPTION_EPOCH: MppDate = MppDate::new(1899, 12, 30);

/// Maps colliding unique IDs into fresh ones. P6 keeps WBS IDs and
/// activity IDs in separate namespaces, but the MPP model puts both kinds
/// of row in one task list, so a collision must remap the activity to a
/// new ID. Ported from MPXJ's `ClashMap`.
#[derive(Debug, Default)]
pub(crate) struct ClashMap {
    used: std::collections::HashSet<i32>,
    remapped: std::collections::HashMap<i32, i32>,
    next: i32,
}

impl ClashMap {
    /// Register `id`, remapping it if already taken. Returns the ID to use.
    pub fn add(&mut self, id: i32) -> i32 {
        if self.used.insert(id) {
            if id >= self.next {
                self.next = id + 1;
            }
            id
        } else {
            let fresh = self.next;
            self.next += 1;
            self.used.insert(fresh);
            self.remapped.insert(id, fresh);
            fresh
        }
    }

    /// Resolve a foreign-key reference to a possibly remapped ID.
    #[cfg(test)]
    pub fn get(&self, id: i32) -> i32 {
        self.remapped.get(&id).copied().unwrap_or(id)
    }
}

/// The most recent rate-table entry per resource: (start date, rate per
/// hour, max units per hour).
pub(crate) type LatestRates =
    std::collections::HashMap<i32, (Option<MppDateTime>, Option<f64>, Option<f64>)>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xer_datetimes() {
        let dt = parse_datetime("2023-04-17 08:30").unwrap();
        assert_eq!(dt.to_string(), "2023-04-17T08:30:00");
        let dt = parse_datetime("2023-4-7 08:30:15").unwrap();
        assert_eq!(dt.to_string(), "2023-04-07T08:30:15");
        // Date only.
        let dt = parse_datetime("2023-04-17").unwrap();
        assert_eq!(dt.to_string(), "2023-04-17T00:00:00");
    }

    #[test]
    fn parses_pmxml_iso_datetimes() {
        let dt = parse_datetime("2023-04-17T08:00:00").unwrap();
        assert_eq!(dt.to_string(), "2023-04-17T08:00:00");
    }

    #[test]
    fn rejects_garbage_datetimes() {
        assert!(parse_datetime("").is_none());
        assert!(parse_datetime("not a date").is_none());
        assert!(parse_datetime("2023-13-01").is_none());
        assert!(parse_datetime("2023-04-17 25:00").is_none());
    }

    #[test]
    fn parses_calendar_times_in_both_conventions() {
        assert_eq!(parse_calendar_time("8:00"), Some(8 * 3600));
        assert_eq!(parse_calendar_time("16:30"), Some(16 * 3600 + 1800));
        assert_eq!(parse_calendar_time("8:00 AM"), Some(8 * 3600));
        assert_eq!(parse_calendar_time("4:00 PM"), Some(16 * 3600));
        assert_eq!(parse_calendar_time("12:00 AM"), Some(0));
        assert_eq!(parse_calendar_time("12:00 PM"), Some(12 * 3600));
        assert_eq!(parse_calendar_time("nope"), None);
    }

    #[test]
    fn relation_type_defaults_and_truncates() {
        assert_eq!(relation_type_from_xer(None), RelationType::FinishStart);
        assert_eq!(
            relation_type_from_xer(Some("PR_SS")),
            RelationType::StartStart
        );
        // MPXJ truncates over-long codes to five characters.
        assert_eq!(
            relation_type_from_xer(Some("PR_FF1")),
            RelationType::FinishFinish
        );
        assert_eq!(
            relation_type_from_xer(Some("bogus")),
            RelationType::FinishStart
        );
    }

    #[test]
    fn constraint_type_covers_all_xer_codes() {
        assert_eq!(
            constraint_type_from_xer(Some("CS_MANDSTART")),
            ConstraintType::MustStartOn
        );
        assert_eq!(
            constraint_type_from_xer(None),
            ConstraintType::AsSoonAsPossible
        );
    }

    #[test]
    fn milestone_map_matches_mpxj() {
        assert!(is_milestone(Some("TT_Mile")));
        assert!(is_milestone(Some("TT_FinMile")));
        assert!(!is_milestone(Some("TT_Task")));
        assert!(!is_milestone(None));
    }

    #[test]
    fn clash_map_remaps_second_use_of_an_id() {
        let mut map = ClashMap::default();
        assert_eq!(map.add(100), 100);
        assert_eq!(map.add(200), 200);
        // 100 is taken (a WBS row used it): the activity gets a fresh ID
        // above the highest seen so far, and lookups follow it there.
        let remapped = map.add(100);
        assert_eq!(remapped, 201);
        assert_eq!(map.get(100), 201);
        assert_eq!(map.get(200), 200);
    }

    #[test]
    fn xml_name_translations() {
        assert_eq!(activity_type_from_xml("Start Milestone"), Some("TT_Mile"));
        assert_eq!(activity_status_from_xml("Completed"), Some("TK_Complete"));
        assert_eq!(relation_type_from_xml("Start to Start"), Some("PR_SS"));
        assert_eq!(constraint_type_from_xml("Mandatory Finish"), Some("CS_MANDFIN"));
        assert_eq!(resource_type_from_xml("Nonlabor"), Some("RT_Equip"));
        assert_eq!(resource_type_from_xml("???"), None);
    }
}
