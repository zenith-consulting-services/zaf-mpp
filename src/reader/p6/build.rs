//! Ported from MPXJ: src/main/java/org/mpxj/primavera/TableProjectReader.java,
//! TableContextReader.java and XmlProjectReader.java (the parts that map P6
//! data onto the MPP-shaped object model). Copyright (c) Packwood Software,
//! Jon Iles. Licensed under the GNU Lesser General Public License, version
//! 2.1 or later.
//!
//! Assembles a [`Project`] from the format-neutral [`P6Data`] produced by
//! the XER and PMXML parsers.

use std::collections::HashMap;

use super::*;
use crate::error::{corrupt, MppResult};
use crate::model::{
    Assignment, Baseline, Calendar, CalendarException, DayHours, Project, ProjectProperties,
    Relation, Resource, Task, TaskMode, TimeRange,
};

pub(crate) fn build_project(data: P6Data) -> MppResult<Project> {
    // An XER export can contain several projects (the exported one plus any
    // it has cross-project relations into). Like MPXJ's single-project
    // `read()`, this returns the exported project: the one whose export
    // flag is set, or the first if none is flagged. Cross-project relations
    // are dropped (MPXJ only links them on request via `readAll`).
    let project = data
        .projects
        .iter()
        .find(|p| p.export_flag)
        .or_else(|| data.projects.first())
        .ok_or_else(|| corrupt("P6 file contains no project rows"))?;
    let proj_id = project.proj_id;

    let calendars: Vec<Calendar> = data.calendars.iter().map(build_calendar).collect();

    let default_calendar = project
        .clndr_id
        .and_then(|id| data.calendars.iter().find(|c| c.clndr_id == id))
        .or_else(|| data.calendars.iter().find(|c| c.is_default));

    let (minutes_per_day, minutes_per_week, days_per_month) =
        calendar_period_minutes(&data, default_calendar);

    let mut properties = ProjectProperties {
        guid: project.guid,
        status_date: project.last_recalc_date,
        start_date: project.plan_start_date,
        finish_date: project.scd_end_date,
        minutes_per_day,
        minutes_per_week,
        days_per_month,
        currency_symbol: data.currency_symbol.clone(),
        currency_code: data.currency_code.clone(),
        currency_digits: data.currency_digits.unwrap_or(2),
        // XER carries no week-start setting; 1 (Sunday) matches the MPP
        // default used when the Props value is absent.
        week_start_day: data.week_start_day.unwrap_or(1),
        default_calendar_name: default_calendar.and_then(|c| c.name.clone()),
        honor_constraints: true,
        title: project.short_name.clone(),
        // P6's critical threshold is stored in hours; the model field is
        // whole days (MS Project semantics). Round to the nearest day.
        critical_slack_limit_days: project
            .critical_drtn_hr_cnt
            .map(|h| (h / (minutes_per_day as f64 / 60.0)).round() as i32)
            .unwrap_or(0),
        ..ProjectProperties::default()
    };

    // ---- WBS hierarchy -------------------------------------------------
    let wbs_separator = project.wbs_code_separator.as_deref().unwrap_or(".");

    let wbs_rows: Vec<&P6Wbs> = data
        .wbs
        .iter()
        .filter(|w| proj_id.is_none() || w.proj_id == proj_id || w.proj_id.is_none())
        .collect();
    let wbs_ids: std::collections::HashSet<i32> = wbs_rows.iter().map(|w| w.wbs_id).collect();

    let mut clash = ClashMap::default();
    for w in &wbs_rows {
        clash.add(w.wbs_id);
    }

    // Children of each WBS node, ordered by sequence number (MPXJ receives
    // XER WBS rows pre-sorted and sorts PMXML rows with a comparator that
    // is also seq_num based).
    let mut wbs_children: HashMap<Option<i32>, Vec<&P6Wbs>> = HashMap::new();
    for w in &wbs_rows {
        let parent = w.parent_wbs_id.filter(|p| wbs_ids.contains(p));
        wbs_children.entry(parent).or_default().push(w);
    }
    for children in wbs_children.values_mut() {
        children.sort_by_key(|w| (w.seq_num.unwrap_or(i32::MAX), w.wbs_id));
    }

    // Activities of the chosen project, grouped by parent WBS.
    let mut activity_children: HashMap<i32, Vec<&P6Activity>> = HashMap::new();
    let mut orphan_activities: Vec<&P6Activity> = Vec::new();
    for a in &data.activities {
        if proj_id.is_some() && a.proj_id != proj_id && a.proj_id.is_some() {
            continue;
        }
        match a.wbs_id.filter(|w| wbs_ids.contains(w)) {
            Some(wbs_id) => activity_children.entry(wbs_id).or_default().push(a),
            None => orphan_activities.push(a),
        }
    }
    let activity_order = |a: &&P6Activity, b: &&P6Activity| {
        // MPXJ's ActivitySorter orders sibling activities by Activity ID
        // using an alphanumeric comparator, so "A9" sorts before "A10"
        // (plain string order would put "A10" first).
        natural_cmp(
            a.task_code.as_deref().unwrap_or(""),
            b.task_code.as_deref().unwrap_or(""),
        )
        .then(a.task_id.cmp(&b.task_id))
    };
    for children in activity_children.values_mut() {
        children.sort_by(activity_order);
    }
    orphan_activities.sort_by(activity_order);

    // Remap any activity IDs that collide with WBS IDs, and remember the
    // mapping for predecessor / assignment foreign keys.
    let mut activity_uid: HashMap<i32, i32> = HashMap::new();
    for children in activity_children.values() {
        for a in children {
            activity_uid.insert(a.task_id, clash.add(a.task_id));
        }
    }
    for a in &orphan_activities {
        activity_uid.insert(a.task_id, clash.add(a.task_id));
    }

    // Longest-path projects mark criticality with the driving-path flag.
    // MPXJ forces `critical` to false here (it has no per-task longest
    // path data in its generic model at read time); this port instead uses
    // P6's own driving-path flag, which is what the P6 UI highlights.
    let longest_path = project.critical_path_type_is_longest_path;
    let critical_limit_hours = project.critical_drtn_hr_cnt.unwrap_or(0.0);

    // Depth-first walk: emit WBS summary tasks and their activities in
    // outline order, assigning sequential IDs and outline levels exactly
    // like MPXJ's updateStructure().
    let mut tasks: Vec<Task> = Vec::new();

    #[allow(clippy::too_many_arguments)]
    fn walk(
        parent_wbs: Option<i32>,
        parent_uid: Option<i32>,
        level: i32,
        parent_path: &str,
        separator: &str,
        wbs_children: &HashMap<Option<i32>, Vec<&P6Wbs>>,
        activity_children: &HashMap<i32, Vec<&P6Activity>>,
        activity_uid: &HashMap<i32, i32>,
        longest_path: bool,
        critical_limit_hours: f64,
        tasks: &mut Vec<Task>,
    ) {
        if let Some(children) = wbs_children.get(&parent_wbs) {
            for w in children {
                let path = if parent_path.is_empty() {
                    w.short_name.clone().unwrap_or_default()
                } else {
                    format!(
                        "{parent_path}{separator}{}",
                        w.short_name.as_deref().unwrap_or("")
                    )
                };
                let uid = w.wbs_id;
                tasks.push(new_wbs_task(w, uid, parent_uid, level, &path));

                for a in activity_children
                    .get(&w.wbs_id)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    let auid = activity_uid[&a.task_id];
                    tasks.push(new_activity_task(
                        a,
                        auid,
                        Some(uid),
                        level + 1,
                        &path,
                        longest_path,
                        critical_limit_hours,
                    ));
                }

                walk(
                    Some(w.wbs_id),
                    Some(uid),
                    level + 1,
                    &path,
                    separator,
                    wbs_children,
                    activity_children,
                    activity_uid,
                    longest_path,
                    critical_limit_hours,
                    tasks,
                );
            }
        }
    }
    walk(
        None,
        None,
        1,
        "",
        wbs_separator,
        &wbs_children,
        &activity_children,
        &activity_uid,
        longest_path,
        critical_limit_hours,
        &mut tasks,
    );

    // Activities with no resolvable WBS parent land at the top level,
    // mirroring MPXJ's `m_project.addTask()` fallback.
    for a in &orphan_activities {
        let auid = activity_uid[&a.task_id];
        tasks.push(new_activity_task(
            a,
            auid,
            None,
            1,
            "",
            longest_path,
            critical_limit_hours,
        ));
    }

    // Sequential IDs in outline order, 0-based to match the MPP14 reader's
    // row numbering (where MS Project's project-summary row takes ID 0):
    // the root WBS plays that row's role in a P6 file.
    for (idx, task) in tasks.iter_mut().enumerate() {
        task.id = idx as i32;
    }

    // The project short name is a code like "PROJ-1"; the display name
    // lives on the sole root WBS row, matching MPXJ's late name fix-up.
    let root_count = tasks.iter().filter(|t| t.outline_level == 1).count();
    if root_count == 1 {
        if let Some(root) = tasks.first().filter(|t| t.summary) {
            if let Some(name) = &root.name {
                properties.title = Some(name.clone());
            }
        }
    }

    let task_index: HashMap<i32, usize> = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.unique_id, i))
        .collect();

    // ---- Relations -----------------------------------------------------
    let mut synthetic_relation_id = 1;
    for r in &data.relations {
        let pred = activity_uid.get(&r.pred_task_id).copied();
        let succ = activity_uid.get(&r.task_id).copied();
        let (Some(pred), Some(succ)) = (pred, succ) else {
            // One end is in another project in the file (or missing):
            // treated as an external relation and dropped, see above.
            continue;
        };
        let unique_id = r.task_pred_id.unwrap_or_else(|| {
            let id = synthetic_relation_id;
            synthetic_relation_id += 1;
            id
        });
        let relation = Relation {
            unique_id,
            predecessor_task_unique_id: pred,
            successor_task_unique_id: succ,
            relation_type: relation_type_from_xer(r.pred_type.as_deref()),
            lag: hours(r.lag_hr_cnt.unwrap_or(0.0)),
        };
        if let Some(&idx) = task_index.get(&succ) {
            tasks[idx].predecessors.push(relation);
        }
    }

    // ---- Resources -----------------------------------------------------
    let resources: Vec<Resource> = data
        .resources
        .iter()
        .enumerate()
        .map(|(idx, r)| Resource {
            unique_id: r.rsrc_id,
            id: idx as i32 + 1,
            name: r.name.clone(),
            initials: r.short_name.clone(),
            group: None,
            email_address: r.email_addr.clone(),
            resource_type: resource_type_from_xer(r.rsrc_type.as_deref()),
            standard_rate_per_hour: r.cost_per_qty.unwrap_or(0.0),
            overtime_rate_per_hour: 0.0,
            // Stored as a fraction of an hour per hour; the model uses MS
            // Project's percentage convention (100.0 == one full unit).
            max_units: r.max_qty_per_hr.map(|q| q * 100.0).unwrap_or(100.0),
            cost: None,
            work: None,
            calendar_unique_id: r.clndr_id,
        })
        .collect();

    // ---- Assignments ---------------------------------------------------
    let mut assignments: Vec<Assignment> = Vec::new();
    for a in &data.assignments {
        let Some(&task_uid) = activity_uid.get(&a.task_id) else {
            continue;
        };
        let actual_work_hours = sum_options(&[a.act_reg_qty, a.act_ot_qty]);
        let work_hours = sum_options(&[actual_work_hours, a.remain_qty]);
        let actual_cost = sum_options(&[a.act_reg_cost, a.act_ot_cost]);
        let cost = sum_options(&[actual_cost, a.remain_cost]);
        assignments.push(Assignment {
            unique_id: a.taskrsrc_id,
            task_unique_id: task_uid,
            resource_unique_id: a.rsrc_id,
            start: a.act_start_date.or(a.restart_date).or(a.target_start_date),
            finish: a.act_end_date.or(a.reend_date).or(a.target_end_date),
            work: work_hours.map(hours),
            actual_work: actual_work_hours.map(hours),
            units: a.target_qty_per_hr.map(|q| q * 100.0).unwrap_or(100.0),
            cost,
            actual_cost,
        });
    }

    // ---- Cost / baseline-cost rollup to activities ---------------------
    // The XER TASK table carries no cost columns and P6 omits most cost
    // totals from PMXML activities, so like MPXJ (`RollupHelper`) costs are
    // rolled up from resource assignments.
    let mut task_cost: HashMap<i32, (f64, f64, f64)> = HashMap::new();
    for (a, src) in assignments.iter().zip(data.assignments.iter()) {
        let entry = task_cost.entry(a.task_unique_id).or_default();
        entry.0 += a.cost.unwrap_or(0.0);
        entry.1 += a.actual_cost.unwrap_or(0.0);
        entry.2 += src.target_cost.unwrap_or(0.0);
    }
    for task in &mut tasks {
        if let Some(&(cost, actual, target)) = task_cost.get(&task.unique_id) {
            task.cost = Some(cost);
            task.actual_cost = Some(actual);
            // Keep the baseline coherent: no planned cost on a task whose
            // baseline is otherwise absent (see the presence gate above).
            if task.baseline.start.is_some()
                || task.baseline.finish.is_some()
                || task.baseline.duration.is_some()
            {
                task.baseline.cost = Some(target);
            }
        }
    }

    // ---- Summary rollup ------------------------------------------------
    // MPXJ leaves P6 WBS rows without dates/durations (its consumers
    // compute them on demand). This model is plain data, so summary rows
    // are populated here: dates as min/max over descendants, work/cost as
    // sums, percent complete as a planned-duration-weighted average.
    rollup_summaries(&mut tasks);

    // Summary durations: MS Project stores a scheduler-computed duration
    // on every summary row; P6 stores none, so measure working time
    // between the rolled-up start and finish on the default calendar
    // (whole-day resolution — start/finish times within the day are
    // ignored, which matches how phase-level durations read in practice).
    if let Some(cal) = default_calendar {
        for task in tasks.iter_mut().filter(|t| t.summary) {
            if task.duration.is_none() {
                if let (Some(start), Some(finish)) = (task.start, task.finish) {
                    task.duration =
                        Some(hours(working_hours_between(cal, start.date, finish.date)));
                }
            }
        }
    }

    // Project start/finish fall back to the task span when the project row
    // didn't carry them.
    if properties.start_date.is_none() {
        properties.start_date = tasks.iter().filter_map(|t| t.start).min();
    }
    if properties.finish_date.is_none() {
        properties.finish_date = tasks.iter().filter_map(|t| t.finish).max();
    }

    Ok(Project {
        properties,
        calendars,
        tasks,
        resources,
        assignments,
    })
}

fn new_wbs_task(
    w: &P6Wbs,
    unique_id: i32,
    parent_uid: Option<i32>,
    level: i32,
    path: &str,
) -> Task {
    Task {
        name: w.name.clone(),
        outline_level: level,
        parent_task_unique_id: parent_uid,
        wbs: Some(path.to_string()),
        summary: true,
        ..blank_task(unique_id)
    }
}

fn new_activity_task(
    a: &P6Activity,
    unique_id: i32,
    parent_uid: Option<i32>,
    level: i32,
    parent_path: &str,
    longest_path: bool,
    critical_limit_hours: f64,
) -> Task {
    let completed = a.status_code.as_deref() == Some("TK_Complete");

    // Percent complete: P6's duration % complete. PMXML carries it
    // directly; for XER it is derived from planned vs remaining duration,
    // ported from `TableProjectReader.calculateDurationPercentComplete`.
    let percent_complete = a.duration_pct_complete.unwrap_or_else(|| {
        let target = a.target_drtn_hr_cnt.unwrap_or(0.0);
        let remain = a.remain_drtn_hr_cnt.unwrap_or(0.0);
        if target == 0.0 {
            if remain == 0.0 && completed {
                100.0
            } else {
                0.0
            }
        } else if remain < target {
            ((target - remain) * 100.0) / target
        } else {
            0.0
        }
    });

    // Start/finish selection chains, ported from the populateField calls
    // in TableProjectReader / XmlProjectReader: exported value (PMXML
    // only), then actual, remaining-early, planned, early.
    let mut start = a
        .start_date
        .or(a.act_start_date)
        .or(a.restart_date)
        .or(a.target_start_date)
        .or(a.early_start_date);
    let mut finish = a
        .finish_date
        .or(a.act_end_date)
        .or(a.reend_date)
        .or(a.target_end_date)
        .or(a.early_end_date);

    // For milestones P6 populates the meaningful end only; mirror it to
    // the other, ported from XmlProjectReader's milestone date fix-up.
    let milestone = is_milestone(a.task_type.as_deref());
    if milestone {
        match a.task_type.as_deref() {
            Some("TT_Mile") => finish = start,
            _ => start = finish,
        }
    }

    // Durations. MPXJ computes actual and at-completion durations by
    // measuring working time between dates on the task calendar; this
    // crate has no working-time engine, so both come from the stored hour
    // counts instead: actual = planned - remaining (never negative),
    // at-completion = actual + remaining. For unstarted and completed
    // activities the results agree with MPXJ; for in-progress activities
    // that are running long they can differ from a calendar measurement.
    let target = a.target_drtn_hr_cnt;
    let remain = a.remain_drtn_hr_cnt;
    let actual_duration = a
        .act_start_date
        .map(|_| (target.unwrap_or(0.0) - remain.unwrap_or(0.0)).max(0.0));
    let duration_hours = a.at_completion_drtn_hr_cnt.or_else(|| {
        sum_options(&[
            actual_duration,
            if a.act_end_date.is_some() {
                None
            } else {
                remain
            },
        ])
        .or(target)
    });

    let actual_work = sum_options(&[a.act_work_qty, a.act_equip_qty]);
    let remaining_work = sum_options(&[a.remain_work_qty, a.remain_equip_qty]);
    let work = sum_options(&[actual_work, remaining_work]);
    let target_work = sum_options(&[a.target_work_qty, a.target_equip_qty]);

    // Slack: P6 exports only total float; start/finish slack are inferred
    // the way MPXJ's SlackHelper.inferSlack does.
    let total = a.total_float_hr_cnt;
    let (start_slack, finish_slack, total_slack) = match (a.act_start_date, a.act_end_date) {
        (_, Some(_)) => (Some(0.0), Some(0.0), Some(0.0)),
        (Some(_), None) => (Some(0.0), total, total),
        (None, None) => (total, total, total),
    };

    // Critical flag. For total-float projects: float at or under the
    // project threshold (ported from the standard P6 definition MPXJ's
    // Task.calculateCritical approximates); completed activities are never
    // critical. For longest-path projects MPXJ forces false; this port
    // uses P6's own driving-path flag instead (see build_project).
    let critical = if completed {
        false
    } else if longest_path {
        a.driving_path
    } else {
        a.total_float_hr_cnt
            .map(|f| f <= critical_limit_hours)
            .unwrap_or(false)
    };

    Task {
        name: a.name.clone(),
        outline_level: level,
        parent_task_unique_id: parent_uid,
        // Activities inherit their parent WBS path, matching
        // MatchPrimaveraWBS = true (MPXJ's default).
        wbs: Some(parent_path.to_string()),
        start,
        finish,
        actual_start: a.act_start_date,
        actual_finish: a.act_end_date,
        duration: duration_hours.map(hours),
        actual_duration: actual_duration.map(hours),
        work: work.map(hours),
        actual_work: actual_work.map(hours),
        percent_complete,
        early_start: a.early_start_date.or(a.restart_date),
        early_finish: a.early_end_date.or(a.reend_date),
        late_start: a.late_start_date,
        late_finish: a.late_end_date,
        free_slack: a.free_float_hr_cnt.map(hours),
        start_slack: start_slack.map(hours),
        finish_slack: finish_slack.map(hours),
        total_slack: total_slack.map(hours),
        critical,
        milestone,
        constraint_type: constraint_type_from_xer(a.cstr_type.as_deref()),
        constraint_date: a.cstr_date,
        calendar_unique_id: a.clndr_id,
        // P6's planned ("target") values play the baseline role when no
        // separate baseline project is attached, matching MPXJ's
        // PLANNED_ATTRIBUTES baseline strategy. Genuine P6 exports always
        // carry planned dates; the presence gate only matters for sparse
        // synthetic files, where an activity without planned dates should
        // read as "no baseline" rather than inheriting a baseline duration.
        baseline: if a.target_start_date.is_some() || a.target_end_date.is_some() {
            Baseline {
                start: a.target_start_date,
                finish: a.target_end_date,
                duration: a.target_drtn_hr_cnt.map(hours),
                work: target_work.map(hours),
                cost: None, // rolled up from assignments later
            }
        } else {
            Baseline::default()
        },
        ..blank_task(unique_id)
    }
}

fn blank_task(unique_id: i32) -> Task {
    Task {
        unique_id,
        id: 0,
        name: None,
        outline_level: 1,
        parent_task_unique_id: None,
        wbs: None,
        start: None,
        finish: None,
        actual_start: None,
        actual_finish: None,
        duration: None,
        actual_duration: None,
        work: None,
        actual_work: None,
        percent_complete: 0.0,
        early_start: None,
        early_finish: None,
        late_start: None,
        late_finish: None,
        free_slack: None,
        start_slack: None,
        finish_slack: None,
        total_slack: None,
        critical: false,
        milestone: false,
        summary: false,
        // P6 has no manual scheduling concept; everything is scheduled.
        task_mode: TaskMode::AutoScheduled,
        constraint_type: crate::model::ConstraintType::AsSoonAsPossible,
        constraint_date: None,
        calendar_unique_id: None,
        cost: None,
        actual_cost: None,
        notes: None,
        predecessors: Vec::new(),
        baseline: Baseline::default(),
        baselines: Default::default(),
    }
}

/// Sum the `Some` entries; `None` only if every entry is `None`.
fn sum_options(values: &[Option<f64>]) -> Option<f64> {
    if values.iter().all(Option::is_none) {
        None
    } else {
        Some(values.iter().flatten().sum())
    }
}

/// Populate summary (WBS) rows from their descendants: dates min/max,
/// duration/work/cost sums of leaf values, percent complete weighted by
/// planned duration. Summaries keep `critical: false`: P6 has no WBS-level
/// criticality (MS Project's scheduler computes summary slack from summary
/// dates, which P6 data cannot reproduce), and inferring it from children
/// over-marks bands that merely contain critical work. Bottom-up: children
/// always appear after their parent in outline order, so a reverse pass
/// accumulates correctly.
fn rollup_summaries(tasks: &mut [Task]) {
    #[derive(Default, Clone)]
    struct Acc {
        start: Option<crate::util::MppDateTime>,
        finish: Option<crate::util::MppDateTime>,
        work: f64,
        has_work: bool,
        cost: f64,
        has_cost: bool,
        actual_cost: f64,
        baseline_cost: f64,
        has_baseline_cost: bool,
        weighted_pct: f64,
        weight: f64,
        baseline_start: Option<crate::util::MppDateTime>,
        baseline_finish: Option<crate::util::MppDateTime>,
    }

    let mut accs: HashMap<i32, Acc> = HashMap::new();

    for idx in (0..tasks.len()).rev() {
        let (uid, parent) = (tasks[idx].unique_id, tasks[idx].parent_task_unique_id);

        // A summary's own weight towards ITS parent is the accumulated
        // weight of its subtree (summaries carry no duration of their
        // own), so a multi-level hierarchy propagates percent complete all
        // the way to the root. The exact (unrounded) percentage travels
        // upward so rounding never compounds across levels.
        let mut subtree_weight = 0.0;
        let mut subtree_pct_exact = 0.0;

        if tasks[idx].summary {
            if let Some(acc) = accs.remove(&uid) {
                let t = &mut tasks[idx];
                t.start = min_option(t.start, acc.start);
                t.finish = max_option(t.finish, acc.finish);
                if acc.has_work {
                    t.work = Some(hours(acc.work));
                }
                if acc.has_cost {
                    t.cost = Some(acc.cost);
                    t.actual_cost = Some(acc.actual_cost);
                }
                if acc.has_baseline_cost {
                    t.baseline.cost = Some(acc.baseline_cost);
                }
                // Summaries inherit the baseline span of their subtree,
                // mirroring the stored summary baselines MS Project files
                // carry (deliverable variance and planned-% need them).
                t.baseline.start = acc.baseline_start;
                t.baseline.finish = acc.baseline_finish;
                if acc.weight > 0.0 {
                    // Whole-percent rounding matches MS Project, which
                    // stores every percent complete as an integer — the
                    // same schedule then reads identically from either
                    // format's file. Only the stored value is rounded;
                    // the exact fraction continues up the hierarchy.
                    subtree_pct_exact = acc.weighted_pct / acc.weight;
                    subtree_weight = acc.weight;
                    t.percent_complete = subtree_pct_exact.round();
                }
            }
        }

        let Some(parent) = parent else { continue };
        let t = &tasks[idx];
        let up = accs.entry(parent).or_default();
        up.start = min_option(up.start, t.start);
        up.finish = max_option(up.finish, t.finish);
        if let Some(w) = t.work {
            up.work += w.value;
            up.has_work = true;
        }
        if let Some(c) = t.cost {
            up.cost += c;
            up.has_cost = true;
            up.actual_cost += t.actual_cost.unwrap_or(0.0);
        }
        if let Some(c) = t.baseline.cost {
            up.baseline_cost += c;
            up.has_baseline_cost = true;
        }
        up.baseline_start = min_option(up.baseline_start, t.baseline.start);
        up.baseline_finish = max_option(up.baseline_finish, t.baseline.finish);
        // Weight by the current at-completion duration (percent x duration
        // is then the actual duration, so the rollup reproduces MS
        // Project's sum(actual)/sum(duration) exactly); planned duration
        // only as a fallback.
        let weight = if t.summary {
            subtree_weight
        } else {
            t.duration
                .or(t.baseline.duration)
                .map(|d| d.value)
                .unwrap_or(0.0)
        };
        if weight > 0.0 {
            let pct = if t.summary {
                subtree_pct_exact
            } else {
                t.percent_complete
            };
            up.weighted_pct += pct * weight;
            up.weight += weight;
        }
    }
}

/// Days from 1970-01-01 (Howard Hinnant's days-from-civil algorithm).
fn days_from_civil(date: crate::util::MppDate) -> i64 {
    let (mut y, m, d) = (date.year as i64, date.month as i64, date.day as i64);
    if m <= 2 {
        y -= 1;
    }
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Day of week, Sunday = 0 (1970-01-01 was a Thursday).
fn weekday_index(date: crate::util::MppDate) -> usize {
    ((days_from_civil(date) + 4).rem_euclid(7)) as usize
}

/// Working hours between two dates (inclusive) on a P6 calendar: sum of
/// each day's working ranges, with single-day exceptions overriding the
/// weekday pattern.
fn working_hours_between(
    cal: &P6Calendar,
    from: crate::util::MppDate,
    to: crate::util::MppDate,
) -> f64 {
    if to < from {
        return 0.0;
    }
    let range_hours = |ranges: &[(u32, u32)]| -> f64 {
        ranges
            .iter()
            .map(|&(s, e)| (e.saturating_sub(s)) as f64 / 3600.0)
            .sum()
    };
    // A calendar with no day data at all reads as a standard Mon-Fri
    // 08:00-16:00 week, the same fallback build_calendar applies.
    let default_week: [Vec<(u32, u32)>; 7] = std::array::from_fn(|i| {
        if (1..=5).contains(&i) {
            vec![(8 * 3600, 16 * 3600)]
        } else {
            Vec::new()
        }
    });
    let no_day_data = cal.days.iter().all(|d| !d.present);
    let day_ranges = |weekday: usize| -> &[(u32, u32)] {
        if no_day_data {
            &default_week[weekday]
        } else {
            &cal.days[weekday].ranges
        }
    };
    let exceptions: HashMap<crate::util::MppDate, f64> = cal
        .exceptions
        .iter()
        .map(|e| (e.date, range_hours(&e.ranges)))
        .collect();

    let mut total = 0.0;
    let mut day = from;
    loop {
        total += exceptions
            .get(&day)
            .copied()
            .unwrap_or_else(|| range_hours(day_ranges(weekday_index(day))));
        if day >= to {
            break;
        }
        day = day.plus_days(1);
    }
    total
}

/// Alphanumeric ("natural") string ordering: digit runs compare as
/// numbers, other runs as text. Ported in spirit from MPXJ's
/// AlphanumComparator, which ActivitySorter uses for Activity IDs.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                if x.is_ascii_digit() && y.is_ascii_digit() {
                    let mut na = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() {
                            na.push(c);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    let mut nb = String::new();
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() {
                            nb.push(c);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    // Compare as numbers: longer (trimmed) digit run wins,
                    // then lexicographic on equal length.
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    let ord = ta
                        .len()
                        .cmp(&tb.len())
                        .then_with(|| ta.cmp(tb))
                        .then_with(|| na.len().cmp(&nb.len()));
                    if ord != std::cmp::Ordering::Equal {
                        return ord;
                    }
                } else {
                    if x != y {
                        return x.cmp(&y);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

fn min_option<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (x, None) | (None, x) => x,
    }
}

fn max_option<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (x, None) | (None, x) => x,
    }
}

/// Convert a parsed P6 calendar to the model shape.
fn build_calendar(c: &P6Calendar) -> Calendar {
    let any_day_present = c.days.iter().any(|d| d.present);

    let days: Vec<Option<DayHours>> = (0..7)
        .map(|i| {
            let day = &c.days[i];
            if !any_day_present {
                // No day data at all (e.g. an XER calendar with an empty
                // clndr_data): fall back to a standard Mon-Fri 08:00-16:00
                // week, matching ProjectCalendarHelper.ensureWorkingTime.
                if (1..=5).contains(&i) {
                    return Some(DayHours {
                        working: true,
                        ranges: vec![TimeRange {
                            start_seconds: 8 * 3600,
                            end_seconds: 16 * 3600,
                        }],
                    });
                }
                return Some(DayHours {
                    working: false,
                    ranges: Vec::new(),
                });
            }
            Some(DayHours {
                working: !day.ranges.is_empty(),
                ranges: day
                    .ranges
                    .iter()
                    .map(|&(s, e)| TimeRange {
                        start_seconds: s,
                        end_seconds: e,
                    })
                    .collect(),
            })
        })
        .collect();

    Calendar {
        unique_id: c.clndr_id,
        name: c.name.clone(),
        base_calendar_unique_id: c.base_clndr_id,
        days,
        exceptions: c
            .exceptions
            .iter()
            .map(|e| CalendarException {
                from_date: Some(e.date),
                to_date: Some(e.date),
                working: !e.ranges.is_empty(),
                ranges: e
                    .ranges
                    .iter()
                    .map(|&(s, e)| TimeRange {
                        start_seconds: s,
                        end_seconds: e,
                    })
                    .collect(),
                name: None,
            })
            .collect(),
    }
}

/// Work out minutes-per-day / minutes-per-week / days-per-month, in this
/// order of preference: global preferences (PMXML), the default calendar's
/// stored hour counts (XER), a computation from the default calendar's
/// working week, then MS Project's standard 480/2400/20.
fn calendar_period_minutes(
    data: &P6Data,
    default_calendar: Option<&P6Calendar>,
) -> (i32, i32, i32) {
    let computed = default_calendar.map(|c| {
        let mut minutes_per_week = 0.0;
        let mut working_days = 0;
        for day in &c.days {
            let day_minutes: f64 = day
                .ranges
                .iter()
                .map(|&(s, e)| (e.saturating_sub(s)) as f64 / 60.0)
                .sum();
            if day_minutes > 0.0 {
                working_days += 1;
                minutes_per_week += day_minutes;
            }
        }
        if working_days == 0 {
            (480.0, 2400.0)
        } else {
            (minutes_per_week / working_days as f64, minutes_per_week)
        }
    });

    let minutes_per_day = data
        .hours_per_day
        .or(default_calendar.and_then(|c| c.day_hr_cnt))
        .map(|h| h * 60.0)
        .or(computed.map(|c| c.0))
        .unwrap_or(480.0);
    let minutes_per_week = data
        .hours_per_week
        .or(default_calendar.and_then(|c| c.week_hr_cnt))
        .map(|h| h * 60.0)
        .or(computed.map(|c| c.1))
        .unwrap_or(2400.0);
    let days_per_month = data
        .hours_per_month
        .or(default_calendar.and_then(|c| c.month_hr_cnt))
        .map(|month_hours| {
            let day_hours = minutes_per_day / 60.0;
            if day_hours > 0.0 {
                (month_hours / day_hours).round()
            } else {
                20.0
            }
        })
        .unwrap_or(20.0);

    (
        minutes_per_day.round() as i32,
        minutes_per_week.round() as i32,
        days_per_month as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_index_matches_known_dates() {
        // 2026-03-02 is a Monday, 1970-01-01 a Thursday, 1899-12-30 a Saturday.
        assert_eq!(weekday_index(crate::util::MppDate::new(2026, 3, 2)), 1);
        assert_eq!(weekday_index(crate::util::MppDate::new(1970, 1, 1)), 4);
        assert_eq!(weekday_index(crate::util::MppDate::new(1899, 12, 30)), 6);
    }

    #[test]
    fn working_hours_between_counts_a_standard_week() {
        let mut cal = P6Calendar::default();
        for i in 1..=5 {
            cal.days[i].present = true;
            cal.days[i].ranges = vec![(8 * 3600, 16 * 3600)];
        }
        // Mon 2026-03-02 through Sun 2026-03-08: five 8h days.
        let from = crate::util::MppDate::new(2026, 3, 2);
        let to = crate::util::MppDate::new(2026, 3, 8);
        assert!((working_hours_between(&cal, from, to) - 40.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod natural_cmp_tests {
    use super::natural_cmp;
    use std::cmp::Ordering;

    #[test]
    fn digit_runs_compare_numerically() {
        assert_eq!(natural_cmp("A9", "A10"), Ordering::Less);
        assert_eq!(natural_cmp("A10", "A9"), Ordering::Greater);
        assert_eq!(natural_cmp("A1000", "A1010"), Ordering::Less);
        assert_eq!(natural_cmp("A2", "A2"), Ordering::Equal);
        assert_eq!(natural_cmp("A02", "A2"), Ordering::Greater);
        assert_eq!(natural_cmp("B1", "A2"), Ordering::Greater);
    }
}
