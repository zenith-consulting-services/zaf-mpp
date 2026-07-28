//! Ported from MPXJ: src/main/java/org/mpxj/mpp/MPP14Reader.java,
//! ProjectPropertiesReader.java, FieldMap14.java, ConstraintFactory.java,
//! AbstractCalendarFactory.java and AbstractCalendarAndExceptionFactory.java.
//! Copyright (c) Packwood Software, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Reads the directory layout MPXJ's `MPP14Reader` expects:
//!
//! ```text
//! /Props14                       high level document properties
//! /   114/Props                  project properties
//! /   114/TBkndCal/...           calendars
//! /   114/TBkndTask/...          tasks
//! /   114/TBkndCons/...          task relations ("constraints")
//! /   114/TBkndRsc/...           resources
//! /   114/TBkndAssn/...          assignments
//! ```
//!
//! Field offsets below are transcribed from `FieldMap14`'s
//! `getDefaultTaskData`/`getDefaultResourceData`/`getDefaultAssignmentData`
//! static tables and from the `MppBitFlag` tables in `MPP14Reader`, cited
//! inline. Only fields in zaf-mpp's documented scope are read: enterprise
//! and custom fields, hyperlinks, recurring tasks, subprojects, and outline
//! codes are intentionally not ported (out of scope, see README).

use std::collections::HashMap;

use super::field_map::{
    FieldMap, ASSIGNMENT_FIELD_MAP, ASSIGNMENT_FIELD_MAP2, RESOURCE_FIELD_MAP, RESOURCE_FIELD_MAP2,
    TASK_FIELD_MAP, TASK_FIELD_MAP2,
};
use crate::container::fixed_data::FixedData;
use crate::container::fixed_meta::FixedMeta;
use crate::container::props::Props;
use crate::container::var2_data::Var2Data;
use crate::container::var_meta::VarMeta;
use crate::container::OleContainer;
use crate::error::{MppError, MppResult};
use crate::model::{
    Assignment, Baseline, Calendar, CalendarException, ConstraintType, DayHours, Project,
    ProjectProperties, Relation, RelationType, Resource, ResourceType, Task, TaskMode, TimeRange,
};
use crate::util::{
    duration_from_raw, get_date, get_double, get_duration_millis, get_duration_time_units,
    get_guid, get_int, get_short, get_time_seconds, get_timestamp, get_unicode_string, MppDuration,
    TimeUnit,
};

const PROJECT_DIR: &str = "/   114";

// PropsKey values, from src/main/java/org/mpxj/mpp/PropsKey.java.
mod props_key {
    pub const PASSWORD_FLAG: i32 = 893386752;
    pub const PROTECTION_PASSWORD_HASH: i32 = 893386756;
    pub const GUID: i32 = 37748777;
    pub const PROJECT_START_DATE: i32 = 37748738;
    pub const PROJECT_FINISH_DATE: i32 = 37748739;
    pub const START_TIME: i32 = 37748764;
    pub const END_TIME: i32 = 37748769;
    pub const STATUS_DATE: i32 = 37748805;
    pub const MINUTES_PER_DAY: i32 = 37748765;
    pub const MINUTES_PER_WEEK: i32 = 37748766;
    pub const CURRENCY_DIGITS: i32 = 37748754;
    pub const CURRENCY_SYMBOL: i32 = 37748752;
    pub const CURRENCY_CODE: i32 = 37753787;
    pub const WEEK_START_DAY: i32 = 37748773;
    pub const DAYS_PER_MONTH: i32 = 37753743;
    pub const HONOR_CONSTRAINTS: i32 = 37748794;
    pub const DEFAULT_CALENDAR_HOURS: i32 = 37753736;
    pub const DEFAULT_CALENDAR_NAME: i32 = 37748750;
    pub const TITLE: i32 = 37748744;
    pub const CRITICAL_SLACK_LIMIT: i32 = 37748756;
}

pub fn read(container: &mut OleContainer, application_version: Option<u32>) -> MppResult<Project> {
    let root_props_bytes = container.read_stream("/Props14")?;
    let root_props = Props::parse(&root_props_bytes);

    let password_flag = root_props.byte(props_key::PASSWORD_FLAG);
    let password_required_to_read = password_flag & 0x1 != 0;
    let encryption_xml_present = root_props
        .byte_array(props_key::PROTECTION_PASSWORD_HASH)
        .is_some();
    if password_required_to_read && encryption_xml_present {
        return Err(MppError::PasswordProtected);
    }

    let project_props_bytes = container.read_stream(&format!("{PROJECT_DIR}/Props"))?;
    let project_props = Props::parse(&project_props_bytes);

    let properties = read_properties(&project_props, application_version);

    let mut resource_calendar_map: HashMap<i32, i32> = HashMap::new();
    let calendars = read_calendars(container, &project_props, &mut resource_calendar_map)?;

    let app_version = application_version.unwrap_or(14);
    let mut tasks = read_tasks(
        container,
        &project_props,
        app_version,
        properties.critical_slack_limit_days,
    )?;
    read_relations(container, &mut tasks, app_version)?;
    link_hierarchy(&mut tasks);
    generate_auto_wbs(&mut tasks);

    let resources = read_resources(
        container,
        &project_props,
        app_version,
        &resource_calendar_map,
    )?;
    let assignments = read_assignments(container, &project_props)?;

    Ok(Project {
        properties,
        calendars,
        tasks,
        resources,
        assignments,
    })
}

/// Ported from `ProjectPropertiesReader.process`, restricted to fields
/// actually stored in the MPP14 `Props` block.
fn read_properties(props: &Props, application_version: Option<u32>) -> ProjectProperties {
    ProjectProperties {
        guid: props
            .byte_array(props_key::GUID)
            .and_then(|v| get_guid(v, 0)),
        start_date: props.timestamp(props_key::PROJECT_START_DATE),
        finish_date: props.timestamp(props_key::PROJECT_FINISH_DATE),
        status_date: props.timestamp(props_key::STATUS_DATE),
        default_start_time_seconds: props
            .byte_array(props_key::START_TIME)
            .map(|v| get_time_seconds(v, 0)),
        default_end_time_seconds: props
            .byte_array(props_key::END_TIME)
            .map(|v| get_time_seconds(v, 0)),
        minutes_per_day: props.int(props_key::MINUTES_PER_DAY),
        minutes_per_week: props.int(props_key::MINUTES_PER_WEEK),
        days_per_month: props.short(props_key::DAYS_PER_MONTH),
        currency_symbol: props.unicode_string(props_key::CURRENCY_SYMBOL),
        currency_code: props.unicode_string(props_key::CURRENCY_CODE),
        currency_digits: props.short(props_key::CURRENCY_DIGITS),
        week_start_day: props.short(props_key::WEEK_START_DAY),
        default_calendar_name: props.unicode_string(props_key::DEFAULT_CALENDAR_NAME),
        honor_constraints: !props.boolean(props_key::HONOR_CONSTRAINTS),
        title: props.unicode_string(props_key::TITLE),
        critical_slack_limit_days: props.int(props_key::CRITICAL_SLACK_LIMIT),
        application_version,
    }
}

// ---------------------------------------------------------------------
// Calendars. Ported from AbstractCalendarFactory / AbstractCalendarAndExceptionFactory
// / MPP14CalendarFactory.
// ---------------------------------------------------------------------

fn read_calendars(
    container: &mut OleContainer,
    project_props: &Props,
    resource_calendar_map: &mut HashMap<i32, i32>,
) -> MppResult<Vec<Calendar>> {
    let dir = format!("{PROJECT_DIR}/TBkndCal");
    if !container.storage_exists(&dir) {
        return Ok(Vec::new());
    }

    let var_meta_bytes = container.read_stream(&format!("{dir}/VarMeta"))?;
    let var_meta = VarMeta::parse(&var_meta_bytes)?;
    let var_data_bytes = container.read_stream(&format!("{dir}/Var2Data"))?;
    let var_data = Var2Data::parse(&var_meta, &var_data_bytes);

    let fixed_meta_bytes = container.read_stream(&format!("{dir}/FixedMeta"))?;
    let fixed_meta = FixedMeta::parse(&fixed_meta_bytes, 10)?;
    let fixed_data_bytes = container.read_stream(&format!("{dir}/FixedData"))?;
    let fixed_data = FixedData::parse_with_meta(&fixed_meta, &fixed_data_bytes, 12);

    const CALENDAR_NAME_TYPE: i32 = 1;
    const CALENDAR_DATA_TYPE: i32 = 8;

    let default_calendar_data = project_props.byte_array(props_key::DEFAULT_CALENDAR_HOURS);
    let default_calendar_days = default_calendar_data.map(parse_calendar_day_hours);

    let mut calendars: Vec<Calendar> = Vec::new();
    let mut seen: HashMap<i32, usize> = HashMap::new();
    let mut base_links: Vec<(usize, i32)> = Vec::new();

    let items = fixed_data.item_count();
    for loop_idx in 0..items {
        let Some(record) = fixed_data.byte_array(loop_idx) else {
            continue;
        };
        if record.len() < 8 {
            continue;
        }

        let mut offset = 0usize;
        while offset + 12 <= record.len() {
            let calendar_id = get_int(record, offset);
            let base_calendar_id = get_int(record, offset + 4);
            let resource_id = get_int(record, offset + 8);

            if calendar_id > 0 && !seen.contains_key(&calendar_id) {
                let calendar_data = var_data.byte_array(calendar_id, CALENDAR_DATA_TYPE);
                let name = var_data.unicode_string(calendar_id, CALENDAR_NAME_TYPE);

                let is_base = base_calendar_id <= 0 || base_calendar_id == calendar_id;
                let days = match (calendar_data, default_calendar_days.as_ref()) {
                    (Some(data), _) => parse_calendar_day_hours(data),
                    (None, _) if is_base => vec![None; 7],
                    (None, Some(default_days)) => default_days.clone(),
                    (None, None) => vec![None; 7],
                };
                let exceptions = calendar_data
                    .map(parse_calendar_exceptions)
                    .unwrap_or_default();

                let index = calendars.len();
                calendars.push(Calendar {
                    unique_id: calendar_id,
                    name,
                    base_calendar_unique_id: if is_base {
                        None
                    } else {
                        Some(base_calendar_id)
                    },
                    days,
                    exceptions,
                });
                seen.insert(calendar_id, index);

                if !is_base {
                    base_links.push((index, base_calendar_id));
                }

                if resource_id > 0 {
                    resource_calendar_map
                        .entry(resource_id)
                        .or_insert(calendar_id);
                }
            }

            offset += 12;
        }
    }

    // Resolve derived calendars whose base calendar was defined later in
    // the file than the derived calendar itself.
    for (index, base_id) in base_links {
        if let Some(&base_index) = seen.get(&base_id) {
            if base_index != index {
                calendars[index].base_calendar_unique_id = Some(base_id);
            }
        }
    }

    Ok(calendars)
}

/// Ported from `AbstractCalendarFactory.processCalendarHours`. Returns
/// working hours for each of the 7 days (Sunday = index 0), where a "1"
/// default flag day is represented as `None` here (inherits from the base
/// calendar) rather than resolving the inheritance itself, which zaf-mpp
/// leaves to the caller.
fn parse_calendar_day_hours(data: &[u8]) -> Vec<Option<DayHours>> {
    (0..7)
        .map(|index| {
            let offset = 60 * index;
            if offset + 4 > data.len() {
                return None;
            }
            let default_flag = get_short(data, offset);
            if default_flag == 1 {
                return None;
            }

            let period_count = get_short(data, offset + 2).max(0) as usize;
            let mut ranges = Vec::with_capacity(period_count);
            for period_index in 0..period_count {
                let start_offset = offset + 8 + period_index * 2;
                let duration_offset = offset + 20 + period_index * 4;
                if duration_offset + 4 > data.len() || start_offset + 2 > data.len() {
                    break;
                }
                let start = get_time_seconds(data, start_offset);
                let duration_ms = get_duration_millis(data, duration_offset);
                let end = start + (duration_ms / 1000) as u32;
                ranges.push(TimeRange {
                    start_seconds: start,
                    end_seconds: end,
                });
            }

            Some(DayHours {
                working: !ranges.is_empty(),
                ranges,
            })
        })
        .collect()
}

/// Ported from `AbstractCalendarAndExceptionFactory.processCalendarExceptions`.
/// Recurring exception details (`readRecurringData`) are not decoded: each
/// exception is recorded as a plain date range, which is sufficient to
/// normalise working time for the affected days.
fn parse_calendar_exceptions(data: &[u8]) -> Vec<CalendarException> {
    let mut exceptions = Vec::new();
    if data.len() <= 420 {
        return exceptions;
    }

    let mut offset = 420usize;
    let exception_count = get_short(data, offset).max(0) as usize;
    offset += 4;

    for _ in 0..exception_count {
        if offset + 92 > data.len() {
            break;
        }

        let from_date = get_date(data, offset);
        let to_date = get_date(data, offset + 2);
        let period_count = get_short(data, offset + 14).max(0) as usize;

        let mut ranges = Vec::with_capacity(period_count);
        for period_index in 0..period_count {
            let start_offset = offset + 20 + period_index * 2;
            let duration_offset = offset + 32 + period_index * 4;
            if duration_offset + 4 > data.len() || start_offset + 2 > data.len() {
                break;
            }
            let start = get_time_seconds(data, start_offset);
            let duration_ms = get_duration_millis(data, duration_offset);
            ranges.push(TimeRange {
                start_seconds: start,
                end_seconds: start + (duration_ms / 1000) as u32,
            });
        }

        let mut name_len = get_int(data, offset + 88) as usize;
        if !name_len.is_multiple_of(4) {
            name_len = (name_len / 4 + 1) * 4;
        }
        let name = if name_len != 0 && offset + 92 <= data.len() {
            Some(get_unicode_string(data, offset + 92))
        } else {
            None
        };

        exceptions.push(CalendarException {
            from_date,
            to_date,
            working: !ranges.is_empty(),
            ranges,
            name,
        });

        offset += 92 + name_len;
    }

    exceptions
}

// ---------------------------------------------------------------------
// Tasks. Ported from MPP14Reader.processTaskData / createTaskMap and
// FieldMap14.getDefaultTaskData.
// ---------------------------------------------------------------------

const NULL_TASK_BLOCK_SIZE: usize = 16;

/// Legacy field IDs used both as the fifth argument throughout
/// `FieldMap14.getDefaultTaskData` and as the field identifier in the
/// on-disk field map (see `field_map`). MPXJ's own `FieldMap` class reads
/// META_DATA (boolean flag) locations from a hardcoded table rather than
/// the on-disk map (its authors note in `FieldMap.FieldItem.read` that
/// they haven't worked out how to derive a byte/bit location from that
/// data), so MILESTONE and TASK_MODE are deliberately not looked up here;
/// see `task_bit_flags`.
mod task_field {
    pub const UNIQUE_ID: i32 = 86;
    pub const ID: i32 = 23;
    pub const PARENT_TASK_UNIQUE_ID: i32 = 160;
    pub const OUTLINE_LEVEL: i32 = 249;
    pub const SCHEDULED_DURATION: i32 = 29;
    pub const ACTUAL_DURATION_UNITS: i32 = 181;
    pub const ACTUAL_DURATION: i32 = 28;
    pub const CONSTRAINT_TYPE: i32 = 17;
    pub const CONSTRAINT_DATE: i32 = 18;
    pub const SCHEDULED_START: i32 = 35;
    pub const SCHEDULED_FINISH: i32 = 36;
    pub const ACTUAL_START: i32 = 41;
    pub const ACTUAL_FINISH: i32 = 42;
    pub const PERCENT_COMPLETE: i32 = 32;
    pub const CALENDAR_UNIQUE_ID: i32 = 401;
    pub const WORK: i32 = 0;
    pub const ACTUAL_WORK: i32 = 2;
    pub const COST: i32 = 5;
    pub const ACTUAL_COST: i32 = 7;
    pub const EARLY_FINISH: i32 = 38;
    pub const LATE_START: i32 = 39;
    pub const EARLY_START: i32 = 37;
    pub const LATE_FINISH: i32 = 40;
    pub const FREE_SLACK: i32 = 21;
    pub const START_SLACK: i32 = 438;
    pub const FINISH_SLACK: i32 = 439;
    pub const START: i32 = 1283;
    pub const FINISH: i32 = 1284;
    pub const MANUAL_DURATION: i32 = 1288;
    pub const NAME: i32 = 14;
    pub const WBS: i32 = 16;
    pub const NOTES: i32 = 15;
    pub const BASELINE_START: i32 = 1299;
    pub const BASELINE_FINISH: i32 = 1300;
    pub const BASELINE_DURATION: i32 = 1301;
    pub const BASELINE_WORK: i32 = 1;
    pub const BASELINE_COST: i32 = 6;
    // (start, finish, duration, work, cost) per baseline 1-10.
    pub const BASELINE_N: [(i32, i32, i32, i32, i32); 10] = [
        (1302, 1303, 1304, 485, 484),
        (1305, 1306, 1307, 496, 495),
        (1308, 1309, 1310, 507, 506),
        (1311, 1312, 1313, 518, 517),
        (1314, 1315, 1316, 529, 528),
        (1317, 1318, 1319, 547, 546),
        (1320, 1321, 1322, 558, 557),
        (1323, 1324, 1325, 569, 568),
        (1326, 1327, 1328, 580, 579),
        (1329, 1330, 1331, 591, 590),
    ];

    // "Estimated" baseline fields: MS Project writes these instead of the
    // plain BASELINE_START/FINISH/DURATION fields above whenever the
    // baseline was captured from a task that didn't have hard dates set
    // (e.g. an auto-scheduled task with no manual baseline override).
    // MPXJ's MPPReader.copyEstimatedBaselineFields falls back to these
    // (TASK_ESTIMATED_BASELINE_FIELDS in MPPReader.java) whenever the
    // plain field is absent; see FieldMap14.java for these key values.
    pub const BASELINE_ESTIMATED_START: i32 = 43;
    pub const BASELINE_ESTIMATED_FINISH: i32 = 44;
    pub const BASELINE_ESTIMATED_DURATION: i32 = 27;
    // (estimated start, estimated finish, estimated duration) per baseline 1-10.
    pub const BASELINE_N_ESTIMATED: [(i32, i32, i32); 10] = [
        (482, 483, 487),
        (493, 494, 498),
        (504, 505, 509),
        (515, 516, 520),
        (526, 527, 531),
        (544, 545, 549),
        (555, 556, 560),
        (566, 567, 571),
        (577, 578, 582),
        (588, 589, 593),
    ];
}

/// Pick block 0 (`data`) or block 1 (`data2`) as resolved by the field map.
fn pick<'a>(data: &'a [u8], data2: &'a [u8], block: usize) -> &'a [u8] {
    if block == 0 {
        data
    } else {
        data2
    }
}

struct TaskBitFlags {
    milestone_meta_offset: usize,
    milestone_mask: i32,
    task_mode_meta2_offset: usize,
    task_mode_mask: i32,
}

/// Bit flag locations vary by MS Project product year. Project 2010 uses
/// one layout; 2013 and 2016-365 share field positions that differ from
/// 2010 (they don't fully match each other, but the two fields zaf-mpp
/// reads, MILESTONE and TASK_MODE, happen to share the same offsets for
/// both). See `MPP14Reader.PROJECT2010_TASK_META_DATA_BIT_FLAGS` and
/// siblings.
fn task_bit_flags(application_version: u32) -> TaskBitFlags {
    if application_version <= 14 {
        TaskBitFlags {
            milestone_meta_offset: 8,
            milestone_mask: 0x20,
            task_mode_meta2_offset: 8,
            task_mode_mask: 0x08,
        }
    } else {
        TaskBitFlags {
            milestone_meta_offset: 10,
            milestone_mask: 0x02,
            task_mode_meta2_offset: 8,
            task_mode_mask: 0x80,
        }
    }
}

fn read_tasks(
    container: &mut OleContainer,
    project_props: &Props,
    application_version: u32,
    critical_slack_limit_days: i32,
) -> MppResult<Vec<Task>> {
    let dir = format!("{PROJECT_DIR}/TBkndTask");
    if !container.storage_exists(&dir) {
        return Ok(Vec::new());
    }

    let field_map = FieldMap::from_props(project_props, &[TASK_FIELD_MAP, TASK_FIELD_MAP2]);
    let (uid_block, uid_off) = field_map.fixed(task_field::UNIQUE_ID, 0, 0);
    let (id_block, id_off) = field_map.fixed(task_field::ID, 0, 4);
    let (parent_block, parent_off) = field_map.fixed(task_field::PARENT_TASK_UNIQUE_ID, 0, 36);
    let (outline_block, outline_off) = field_map.fixed(task_field::OUTLINE_LEVEL, 0, 40);
    let (sched_dur_block, sched_dur_off) = field_map.fixed(task_field::SCHEDULED_DURATION, 0, 42);
    let (dur_units_block, dur_units_off) =
        field_map.fixed(task_field::ACTUAL_DURATION_UNITS, 0, 46);
    let (actual_dur_block, actual_dur_off) = field_map.fixed(task_field::ACTUAL_DURATION, 0, 48);
    let (constraint_type_block, constraint_type_off) =
        field_map.fixed(task_field::CONSTRAINT_TYPE, 0, 56);
    let (constraint_date_block, constraint_date_off) =
        field_map.fixed(task_field::CONSTRAINT_DATE, 0, 80);
    let (sched_start_block, sched_start_off) = field_map.fixed(task_field::SCHEDULED_START, 0, 64);
    let (sched_finish_block, sched_finish_off) =
        field_map.fixed(task_field::SCHEDULED_FINISH, 0, 68);
    let (actual_start_block, actual_start_off) = field_map.fixed(task_field::ACTUAL_START, 0, 72);
    let (actual_finish_block, actual_finish_off) =
        field_map.fixed(task_field::ACTUAL_FINISH, 0, 76);
    let (percent_block, percent_off) = field_map.fixed(task_field::PERCENT_COMPLETE, 0, 90);
    let (calendar_block, calendar_off) = field_map.fixed(task_field::CALENDAR_UNIQUE_ID, 0, 118);
    let (work_block, work_off) = field_map.fixed(task_field::WORK, 0, 126);
    let (actual_work_block, actual_work_off) = field_map.fixed(task_field::ACTUAL_WORK, 0, 134);
    let (cost_block, cost_off) = field_map.fixed(task_field::COST, 0, 150);
    let (actual_cost_block, actual_cost_off) = field_map.fixed(task_field::ACTUAL_COST, 0, 166);
    let (early_finish_block, early_finish_off) = field_map.fixed(task_field::EARLY_FINISH, 0, 8);
    let (late_start_block, late_start_off) = field_map.fixed(task_field::LATE_START, 0, 12);
    let (early_start_block, early_start_off) = field_map.fixed(task_field::EARLY_START, 0, 106);
    let (late_finish_block, late_finish_off) = field_map.fixed(task_field::LATE_FINISH, 0, 110);
    let (free_slack_block, free_slack_off) = field_map.fixed(task_field::FREE_SLACK, 0, 24);
    let (start_slack_block, start_slack_off) = field_map.fixed(task_field::START_SLACK, 0, 28);
    let (finish_slack_block, finish_slack_off) = field_map.fixed(task_field::FINISH_SLACK, 0, 32);
    let (start_block, start_off) = field_map.fixed(task_field::START, 1, 50);
    let (finish_block, finish_off) = field_map.fixed(task_field::FINISH, 1, 54);
    let (manual_dur_block, manual_dur_off) = field_map.fixed(task_field::MANUAL_DURATION, 1, 58);

    let name_key = field_map.var(task_field::NAME, task_field::NAME);
    let wbs_key = field_map.var(task_field::WBS, task_field::WBS);
    let notes_key = field_map.var(task_field::NOTES, task_field::NOTES);
    let baseline_start_key = field_map.var(task_field::BASELINE_START, task_field::BASELINE_START);
    let baseline_finish_key =
        field_map.var(task_field::BASELINE_FINISH, task_field::BASELINE_FINISH);
    let baseline_duration_key =
        field_map.var(task_field::BASELINE_DURATION, task_field::BASELINE_DURATION);
    let baseline_work_key = field_map.var(task_field::BASELINE_WORK, task_field::BASELINE_WORK);
    let baseline_cost_key = field_map.var(task_field::BASELINE_COST, task_field::BASELINE_COST);
    let baseline_n_keys: [(i32, i32, i32, i32, i32); 10] = std::array::from_fn(|i| {
        let (s, f, d, w, c) = task_field::BASELINE_N[i];
        (
            field_map.var(s, s),
            field_map.var(f, f),
            field_map.var(d, d),
            field_map.var(w, w),
            field_map.var(c, c),
        )
    });
    let baseline_estimated_start_key = field_map.var(
        task_field::BASELINE_ESTIMATED_START,
        task_field::BASELINE_ESTIMATED_START,
    );
    let baseline_estimated_finish_key = field_map.var(
        task_field::BASELINE_ESTIMATED_FINISH,
        task_field::BASELINE_ESTIMATED_FINISH,
    );
    let baseline_estimated_duration_key = field_map.var(
        task_field::BASELINE_ESTIMATED_DURATION,
        task_field::BASELINE_ESTIMATED_DURATION,
    );
    let baseline_n_estimated_keys: [(i32, i32, i32); 10] = std::array::from_fn(|i| {
        let (s, f, d) = task_field::BASELINE_N_ESTIMATED[i];
        (
            field_map.var(s, s),
            field_map.var(f, f),
            field_map.var(d, d),
        )
    });

    let var_meta_bytes = container.read_stream(&format!("{dir}/VarMeta"))?;
    let var_meta = VarMeta::parse(&var_meta_bytes)?;
    let var_data_bytes = container.read_stream(&format!("{dir}/Var2Data"))?;
    let var_data = Var2Data::parse(&var_meta, &var_data_bytes);

    let fixed_meta_bytes = container.read_stream(&format!("{dir}/FixedMeta"))?;
    let fixed_meta = FixedMeta::parse(&fixed_meta_bytes, 47)?;
    let fixed_data_bytes = container.read_stream(&format!("{dir}/FixedData"))?;
    // No cap on record size: the real per-file layout, not a hardcoded
    // default, determines how large a task record is (see `field_map`).
    let fixed_data = FixedData::parse_with_meta(&fixed_meta, &fixed_data_bytes, 0);

    let fixed2_meta_bytes = container.read_stream(&format!("{dir}/Fixed2Meta"))?;
    let fixed2_meta = FixedMeta::parse_with_candidates(
        &fixed2_meta_bytes,
        fixed_data.item_count(),
        &[92, 93, 94, 95, 96],
    )?;
    let fixed2_data_bytes = container.read_stream(&format!("{dir}/Fixed2Data"))?;
    let fixed2_data = FixedData::parse_with_meta(&fixed2_meta, &fixed2_data_bytes, 0);

    let bit_flags = task_bit_flags(application_version);

    // Smallest record we trust as real, complete task data: enough to
    // cover every field this reader looks up in fixed block 0. Real
    // per-file layouts can place these fields well beyond MPXJ's own
    // hardcoded default offsets, so this is computed from the resolved
    // layout rather than a fixed constant.
    let min_block0_size = [
        (uid_block, uid_off),
        (id_block, id_off),
        (parent_block, parent_off),
        (outline_block, outline_off),
        (sched_dur_block, sched_dur_off),
        (dur_units_block, dur_units_off),
        (actual_dur_block, actual_dur_off),
        (constraint_type_block, constraint_type_off),
        (constraint_date_block, constraint_date_off),
        (sched_start_block, sched_start_off),
        (sched_finish_block, sched_finish_off),
        (actual_start_block, actual_start_off),
        (actual_finish_block, actual_finish_off),
        (percent_block, percent_off),
        (calendar_block, calendar_off),
        (work_block, work_off),
        (actual_work_block, actual_work_off),
        (cost_block, cost_off),
        (actual_cost_block, actual_cost_off),
        (early_finish_block, early_finish_off),
        (late_start_block, late_start_off),
        (early_start_block, early_start_off),
        (late_finish_block, late_finish_off),
        (free_slack_block, free_slack_off),
        (start_slack_block, start_slack_off),
        (finish_slack_block, finish_slack_off),
    ]
    .iter()
    .filter(|(block, _)| *block == 0)
    .map(|(_, off)| off + 8)
    .max()
    .unwrap_or(64);

    // Ported from `MPP14Reader.createTaskMap`: walk backwards so that,
    // where duplicate unique IDs exist, the later (higher index) record
    // wins. The 75%-of-max-size heuristic MPXJ uses to reject incomplete
    // records is replaced with a simple minimum big enough to cover every
    // field this reader reads, since the true per-file maximum record size
    // (computed from every field MS Project knows about, not just the
    // ones zaf-mpp reads) isn't reproduced here.
    let mut task_map: HashMap<i32, usize> = HashMap::new();
    let item_count = fixed_meta.adjusted_item_count();
    for loop_idx in (3..item_count).rev() {
        let (Some(data), Some(_data2)) = (
            fixed_data.byte_array(loop_idx),
            fixed2_data.byte_array(loop_idx),
        ) else {
            continue;
        };
        let Some(meta) = fixed_meta.byte_array(loop_idx) else {
            continue;
        };
        if meta.len() < 4 {
            continue;
        }
        let flags = get_int(meta, 0);
        if flags & 0x02 != 0 {
            continue; // deleted
        }
        if data.len() == NULL_TASK_BLOCK_SIZE {
            continue; // null task placeholder: not represented in the output
        }
        if data.len() < min_block0_size {
            continue; // too little data to trust
        }
        let unique_id = get_int(data, uid_off);
        task_map.entry(unique_id).or_insert(loop_idx);
    }

    let mut unique_ids: Vec<i32> = task_map.keys().copied().collect();
    unique_ids.sort_unstable();

    let mut tasks = Vec::with_capacity(unique_ids.len());
    for unique_id in unique_ids {
        let index = task_map[&unique_id];
        let Some(data) = fixed_data.byte_array(index) else {
            continue;
        };
        let data2 = fixed2_data.byte_array(index).unwrap_or(&[]);
        let meta = fixed_meta.byte_array(index).unwrap_or(&[]);
        let meta2 = fixed2_meta.byte_array(index).unwrap_or(&[]);

        let id = get_int(pick(data, data2, id_block), id_off);
        let outline_level = get_short(pick(data, data2, outline_block), outline_off);
        let parent_task_unique_id = {
            let v = get_int(pick(data, data2, parent_block), parent_off);
            if v == 0 {
                None
            } else {
                Some(v)
            }
        };

        let duration_units_raw = get_short(pick(data, data2, dur_units_block), dur_units_off);
        let duration_units = get_duration_time_units(duration_units_raw, TimeUnit::Days);
        let scheduled_duration = duration_from_raw(
            get_int(pick(data, data2, sched_dur_block), sched_dur_off) as f64,
            duration_units,
        );
        let actual_duration = duration_from_raw(
            get_int(pick(data, data2, actual_dur_block), actual_dur_off) as f64,
            duration_units,
        );

        let task_mode_bit =
            get_int(meta2, bit_flags.task_mode_meta2_offset) & bit_flags.task_mode_mask != 0;
        let task_mode = if task_mode_bit {
            TaskMode::ManuallyScheduled
        } else {
            TaskMode::AutoScheduled
        };

        let scheduled_start = get_timestamp(pick(data, data2, sched_start_block), sched_start_off);
        let scheduled_finish =
            get_timestamp(pick(data, data2, sched_finish_block), sched_finish_off);
        let block1_start = get_timestamp(pick(data, data2, start_block), start_off);
        let block1_finish = get_timestamp(pick(data, data2, finish_block), finish_off);
        let manual_duration_field = pick(data, data2, manual_dur_block);
        let manual_duration_raw = if manual_duration_field.len() >= manual_dur_off + 4 {
            get_int(manual_duration_field, manual_dur_off)
        } else {
            0
        };
        let manual_duration = duration_from_raw(manual_duration_raw as f64, duration_units);

        let start = if block1_start.is_none()
            || (scheduled_start.is_some() && task_mode == TaskMode::AutoScheduled)
        {
            scheduled_start
        } else {
            block1_start
        };
        let finish = if block1_finish.is_none()
            || (scheduled_finish.is_some() && task_mode == TaskMode::AutoScheduled)
        {
            scheduled_finish
        } else {
            block1_finish
        };
        let duration = if task_mode == TaskMode::ManuallyScheduled {
            Some(manual_duration)
        } else {
            Some(scheduled_duration)
        };

        let milestone =
            get_int(meta, bit_flags.milestone_meta_offset) & bit_flags.milestone_mask != 0;

        let calendar_unique_id = {
            let v = get_int(pick(data, data2, calendar_block), calendar_off);
            if v <= 0 {
                None
            } else {
                Some(v)
            }
        };

        // MS Project only writes the plain BASELINE_START/FINISH/DURATION
        // var-data fields when a task's baseline was explicitly captured
        // with hard dates; otherwise it records the "estimated" variants
        // instead. MPXJ's MPPReader.copyEstimatedBaselineFields performs
        // this same fallback as a post-processing pass (see
        // TASK_ESTIMATED_BASELINE_FIELDS in MPPReader.java); we do it
        // inline here.
        let baseline = Baseline {
            start: var_data
                .timestamp(unique_id, baseline_start_key)
                .or_else(|| var_data.timestamp(unique_id, baseline_estimated_start_key)),
            finish: var_data
                .timestamp(unique_id, baseline_finish_key)
                .or_else(|| var_data.timestamp(unique_id, baseline_estimated_finish_key)),
            duration: var_data
                .byte_array(unique_id, baseline_duration_key)
                .map(|v| duration_from_raw(get_int(v, 0) as f64, TimeUnit::Days))
                .or_else(|| {
                    var_data
                        .byte_array(unique_id, baseline_estimated_duration_key)
                        .map(|v| duration_from_raw(get_int(v, 0) as f64, TimeUnit::Days))
                }),
            work: work_hours(var_data.double(unique_id, baseline_work_key)),
            cost: hundredths(var_data.double(unique_id, baseline_cost_key)),
        };

        let baselines: [Baseline; 10] = std::array::from_fn(|i| {
            let (start_t, finish_t, duration_t, work_t, cost_t) = baseline_n_keys[i];
            let (est_start_t, est_finish_t, est_duration_t) = baseline_n_estimated_keys[i];
            Baseline {
                start: var_data
                    .timestamp(unique_id, start_t)
                    .or_else(|| var_data.timestamp(unique_id, est_start_t)),
                finish: var_data
                    .timestamp(unique_id, finish_t)
                    .or_else(|| var_data.timestamp(unique_id, est_finish_t)),
                duration: var_data
                    .byte_array(unique_id, duration_t)
                    .map(|v| duration_from_raw(get_int(v, 0) as f64, TimeUnit::Days))
                    .or_else(|| {
                        var_data
                            .byte_array(unique_id, est_duration_t)
                            .map(|v| duration_from_raw(get_int(v, 0) as f64, TimeUnit::Days))
                    }),
                work: work_hours(var_data.double(unique_id, work_t)),
                cost: hundredths(var_data.double(unique_id, cost_t)),
            }
        });

        let name = var_data.unicode_string(unique_id, name_key);
        let wbs = var_data.unicode_string(unique_id, wbs_key);

        let percent_complete = get_short(pick(data, data2, percent_block), percent_off) as f64;
        let cost = hundredths(get_double(pick(data, data2, cost_block), cost_off));
        let actual_cost = hundredths(get_double(
            pick(data, data2, actual_cost_block),
            actual_cost_off,
        ));
        let work = work_hours(get_double(pick(data, data2, work_block), work_off));
        let actual_work = work_hours(get_double(
            pick(data, data2, actual_work_block),
            actual_work_off,
        ));
        let actual_finish =
            get_timestamp(pick(data, data2, actual_finish_block), actual_finish_off);
        let notes = var_data
            .string(unique_id, notes_key)
            .map(|s| crate::rtf::strip(&s));

        let early_finish = get_timestamp(pick(data, data2, early_finish_block), early_finish_off);
        let late_start = get_timestamp(pick(data, data2, late_start_block), late_start_off);
        let early_start = get_timestamp(pick(data, data2, early_start_block), early_start_off);
        let late_finish = get_timestamp(pick(data, data2, late_finish_block), late_finish_off);
        let free_slack = Some(duration_from_raw(
            get_int(pick(data, data2, free_slack_block), free_slack_off) as f64,
            duration_units,
        ));
        let start_slack = Some(duration_from_raw(
            get_int(pick(data, data2, start_slack_block), start_slack_off) as f64,
            duration_units,
        ));
        let finish_slack = Some(duration_from_raw(
            get_int(pick(data, data2, finish_slack_block), finish_slack_off) as f64,
            duration_units,
        ));

        // Ported from `MicrosoftSlackCalculator.calculateTotalSlack`: once
        // a task has actually started, its total slack tracks finish
        // slack alone; otherwise it's whichever of start/finish slack is
        // smaller. zaf-mpp compares raw values without the calendar-aware
        // unit conversion MPXJ applies when start and finish slack differ
        // in units, since in practice both share the task's duration
        // units.
        let actual_start_for_slack =
            get_timestamp(pick(data, data2, actual_start_block), actual_start_off);
        let total_slack = if actual_start_for_slack.is_some() {
            finish_slack
        } else {
            match (start_slack, finish_slack) {
                (Some(s), Some(f)) if s.value < f.value => Some(s),
                (Some(_), Some(f)) => Some(f),
                (a, b) => a.or(b),
            }
        };

        // Ported from `Task.calculateCritical`, dropping the "no text
        // override" clause (`Show*Text` fields are out of scope).
        let critical = actual_finish.is_none()
            && total_slack.is_some_and(|s| s.value <= critical_slack_limit_days as f64)
            && (percent_complete as i64) != 100
            && task_mode == TaskMode::AutoScheduled;

        // Skip tasks that never got a real name and have every plausible
        // date at the epoch: these are the "phantom" null tasks MPXJ
        // filters out at the end of `processTaskData`.
        if name.is_none() && start.is_none() && finish.is_none() {
            continue;
        }

        tasks.push(Task {
            unique_id,
            id,
            name,
            outline_level,
            parent_task_unique_id,
            wbs,
            start,
            finish,
            actual_start: actual_start_for_slack,
            actual_finish,
            duration,
            actual_duration: Some(actual_duration),
            work,
            actual_work,
            percent_complete,
            early_start,
            early_finish,
            late_start,
            late_finish,
            free_slack,
            start_slack,
            finish_slack,
            total_slack,
            critical,
            milestone,
            summary: false,
            task_mode,
            constraint_type: ConstraintType::from_mpp(get_short(
                pick(data, data2, constraint_type_block),
                constraint_type_off,
            )),
            constraint_date: get_timestamp(
                pick(data, data2, constraint_date_block),
                constraint_date_off,
            ),
            calendar_unique_id,
            cost,
            actual_cost,
            notes,
            predecessors: Vec::new(),
            baseline,
            baselines,
        });
    }

    Ok(tasks)
}

/// Ported from `FieldMap`'s `CURRENCY`/`UNITS` case: raw fixed-data doubles
/// for cost and percentage-unit fields are stored scaled by 100, with
/// anything under half that (0.1 in real units) treated as absent.
fn hundredths(raw: f64) -> Option<f64> {
    if raw.abs() < 0.1 {
        None
    } else {
        Some(raw / 100.0)
    }
}

/// Ported from `FieldMap`'s `WORK` case: raw fixed-data doubles for work
/// fields are stored in milliseconds and always reported in hours,
/// regardless of the project's default work units.
fn work_hours(raw: f64) -> Option<MppDuration> {
    if raw.abs() < 1000.0 {
        None
    } else {
        Some(MppDuration {
            value: raw / 60000.0,
            units: TimeUnit::Hours,
        })
    }
}

/// Sets `Task::summary` for every task that has at least one child, and
/// sorts tasks by `id` to match on-disk presentation order. Ported from the
/// summary-flag portion of `MPPReader.read` (outline number generation and
/// external-task handling are out of scope).
fn link_hierarchy(tasks: &mut [Task]) {
    let mut has_children: HashMap<i32, bool> = HashMap::new();
    for task in tasks.iter() {
        if let Some(parent) = task.parent_task_unique_id {
            has_children.insert(parent, true);
        }
    }
    for task in tasks.iter_mut() {
        task.summary = has_children.get(&task.unique_id).copied().unwrap_or(false);
    }
    tasks.sort_by_key(|t| t.id);
}

/// Ported from `Task.generateWBS`: real MPP files rarely store an explicit
/// per-task WBS value (that field is only present in var-data if the user
/// customized the WBS mask), so MPXJ falls back to auto-numbering tasks by
/// creation order — position among siblings, dotted with the parent's own
/// WBS — and only keeps an explicit value when one was actually read.
/// `tasks` must already be in creation/display order (sorted by `id`, as
/// `link_hierarchy` above leaves them) since child counters are position
/// counts, not a function of the tree shape alone.
fn generate_auto_wbs(tasks: &mut [Task]) {
    let mut child_counts: HashMap<i32, i32> = HashMap::new();
    let mut resolved: HashMap<i32, String> = HashMap::new();

    for task in tasks.iter_mut() {
        let auto_wbs = if task.unique_id == 0 {
            "0".to_string()
        } else {
            match task.parent_task_unique_id {
                None => {
                    let count = child_counts.entry(-1).or_insert(0);
                    *count += 1;
                    count.to_string()
                }
                Some(parent_uid) => {
                    let parent_wbs = resolved.get(&parent_uid).cloned().unwrap_or_default();
                    let count = child_counts.entry(parent_uid).or_insert(0);
                    *count += 1;
                    if parent_wbs == "0" {
                        count.to_string()
                    } else {
                        format!("{parent_wbs}.{count}")
                    }
                }
            }
        };

        let final_wbs = task.wbs.clone().unwrap_or(auto_wbs);
        if task.wbs.is_none() {
            task.wbs = Some(final_wbs.clone());
        }
        resolved.insert(task.unique_id, final_wbs);
    }
}

// ---------------------------------------------------------------------
// Relations. Ported from ConstraintFactory.process.
// ---------------------------------------------------------------------

fn read_relations(
    container: &mut OleContainer,
    tasks: &mut [Task],
    application_version: u32,
) -> MppResult<()> {
    let dir = format!("{PROJECT_DIR}/TBkndCons");
    if !container.storage_exists(&dir) {
        return Ok(());
    }

    let fixed_meta_bytes = container.read_stream(&format!("{dir}/FixedMeta"))?;
    let fixed_meta = FixedMeta::parse(&fixed_meta_bytes, 10)?;
    let fixed_data_bytes = container.read_stream(&format!("{dir}/FixedData"))?;
    let fixed_data = FixedData::parse_fixed_size(&fixed_data_bytes, 20);

    let mut by_unique_id: HashMap<i32, usize> = HashMap::new();
    for (i, t) in tasks.iter().enumerate() {
        by_unique_id.insert(t.unique_id, i);
    }

    let count = fixed_meta.adjusted_item_count();
    // Ported from `ConstraintFactory.process`: Project 2013 and later
    // swapped the position of the lag value and its units within the
    // relation record relative to 2010.
    let (duration_offset, duration_units_offset) = if application_version > 14 {
        (14usize, 18usize)
    } else {
        (16usize, 14usize)
    };

    let mut pending: Vec<(usize, Relation)> = Vec::new();

    for loop_idx in 0..count {
        let Some(meta) = fixed_meta.byte_array(loop_idx) else {
            continue;
        };
        if meta.len() < 8 || get_short(meta, 0) != 0 {
            continue; // deleted
        }
        let Some(index) = fixed_data.index_from_offset(get_int(meta, 4)) else {
            continue;
        };
        let Some(data) = fixed_data.byte_array(index) else {
            continue;
        };
        if data.len() < 14 {
            continue;
        }

        let task_id1 = get_int(data, 4); // predecessor
        let task_id2 = get_int(data, 8); // successor
        if task_id1 == 0 || task_id2 == 0 || task_id1 == task_id2 {
            continue;
        }

        let Some(&successor_idx) = by_unique_id.get(&task_id2) else {
            continue;
        };
        if !by_unique_id.contains_key(&task_id1) {
            continue;
        }

        let relation_type = RelationType::from_mpp(get_short(data, 12));
        let units = get_duration_time_units(get_short(data, duration_units_offset), TimeUnit::Days);
        let lag = duration_from_raw(get_int(data, duration_offset) as f64, units);

        pending.push((
            successor_idx,
            Relation {
                unique_id: get_int(data, 0),
                predecessor_task_unique_id: task_id1,
                successor_task_unique_id: task_id2,
                relation_type,
                lag,
            },
        ));
    }

    for (successor_idx, relation) in pending {
        tasks[successor_idx].predecessors.push(relation);
    }

    Ok(())
}

// ---------------------------------------------------------------------
// Resources. Ported from MPP14Reader.processResourceData / createResourceMap
// and FieldMap14.getDefaultResourceData.
// ---------------------------------------------------------------------

mod resource_field {
    pub const UNIQUE_ID: i32 = 27;
    pub const ID: i32 = 0;
    pub const NAME: i32 = 1;
    pub const INITIALS: i32 = 2;
    pub const GROUP: i32 = 3;
    pub const EMAIL_ADDRESS: i32 = 35;
    pub const MAX_UNITS: i32 = 4;
    pub const STANDARD_RATE: i32 = 6;
    pub const OVERTIME_RATE: i32 = 7;
    pub const COST: i32 = 12;
    pub const WORK: i32 = 13;
}

fn read_resources(
    container: &mut OleContainer,
    project_props: &Props,
    application_version: u32,
    resource_calendar_map: &HashMap<i32, i32>,
) -> MppResult<Vec<Resource>> {
    let dir = format!("{PROJECT_DIR}/TBkndRsc");
    if !container.storage_exists(&dir) {
        return Ok(Vec::new());
    }

    let field_map = FieldMap::from_props(project_props, &[RESOURCE_FIELD_MAP, RESOURCE_FIELD_MAP2]);
    let (uid_block, uid_off) = field_map.fixed(resource_field::UNIQUE_ID, 0, 0);
    let (id_block, id_off) = field_map.fixed(resource_field::ID, 0, 4);
    let (max_units_block, max_units_off) = field_map.fixed(resource_field::MAX_UNITS, 0, 44);
    let (std_rate_block, std_rate_off) = field_map.fixed(resource_field::STANDARD_RATE, 0, 28);
    let (ot_rate_block, ot_rate_off) = field_map.fixed(resource_field::OVERTIME_RATE, 0, 36);
    let (cost_block, cost_off) = field_map.fixed(resource_field::COST, 0, 140);
    let (work_block, work_off) = field_map.fixed(resource_field::WORK, 0, 52);
    let name_key = field_map.var(resource_field::NAME, resource_field::NAME);
    let initials_key = field_map.var(resource_field::INITIALS, resource_field::INITIALS);
    let group_key = field_map.var(resource_field::GROUP, resource_field::GROUP);
    let email_key = field_map.var(resource_field::EMAIL_ADDRESS, resource_field::EMAIL_ADDRESS);

    let var_meta_bytes = container.read_stream(&format!("{dir}/VarMeta"))?;
    let var_meta = VarMeta::parse(&var_meta_bytes)?;
    let var_data_bytes = container.read_stream(&format!("{dir}/Var2Data"))?;
    let var_data = Var2Data::parse(&var_meta, &var_data_bytes);

    let fixed_meta_bytes = container.read_stream(&format!("{dir}/FixedMeta"))?;
    let fixed_meta = FixedMeta::parse(&fixed_meta_bytes, 37)?;
    let fixed_data_bytes = container.read_stream(&format!("{dir}/FixedData"))?;
    // No cap: see the equivalent note in `read_tasks`.
    let fixed_data = FixedData::parse_with_meta(&fixed_meta, &fixed_data_bytes, 0);

    let (resource_type_offset, resource_type_mask) = if application_version > 14 {
        (12, 0x10)
    } else {
        (9, 0x02)
    };

    // Smallest record trusted as real resource data: enough to cover
    // every field this reader reads (see the equivalent note in
    // `read_tasks` for why this replaces MPXJ's hardcoded max size).
    let min_size = [
        (uid_block, uid_off),
        (id_block, id_off),
        (max_units_block, max_units_off),
        (std_rate_block, std_rate_off),
        (ot_rate_block, ot_rate_off),
        (cost_block, cost_off),
        (work_block, work_off),
    ]
    .iter()
    .filter(|(block, _)| *block == 0)
    .map(|(_, off)| off + 8)
    .max()
    .unwrap_or(48);

    let mut resource_map: HashMap<i32, usize> = HashMap::new();
    let item_count = fixed_meta.adjusted_item_count();
    for loop_idx in 0..item_count {
        let Some(data) = fixed_data.byte_array(loop_idx) else {
            continue;
        };
        if data.len() < min_size {
            continue;
        }
        let unique_id = get_short(data, uid_off);
        resource_map.entry(unique_id).or_insert(loop_idx);
    }

    let mut ids: Vec<i32> = resource_map.keys().copied().collect();
    ids.sort_unstable();

    let mut resources = Vec::with_capacity(ids.len());
    for id in ids {
        let index = resource_map[&id];
        let Some(data) = fixed_data.byte_array(index) else {
            continue;
        };
        let meta = fixed_meta.byte_array(index).unwrap_or(&[]);

        let resource_type = if meta.len() > resource_type_offset
            && (meta[resource_type_offset] as i32 & resource_type_mask) != 0
        {
            ResourceType::Work
        } else {
            ResourceType::Material
        };

        // Standard/overtime rate is stored as an amount per minute; MPXJ's
        // `convertRateFromHours` rescales using the paired *_UNITS field.
        // zaf-mpp always reports a per-hour rate for simplicity.
        let standard_rate_raw = get_double(data, std_rate_off);
        let overtime_rate_raw = get_double(data, ot_rate_off);

        resources.push(Resource {
            unique_id: id,
            id: get_int(data, id_off),
            name: var_data.unicode_string(id, name_key),
            initials: var_data.unicode_string(id, initials_key),
            group: var_data.unicode_string(id, group_key),
            email_address: var_data.unicode_string(id, email_key),
            resource_type,
            standard_rate_per_hour: standard_rate_raw,
            overtime_rate_per_hour: overtime_rate_raw,
            max_units: hundredths(get_double(data, max_units_off)).unwrap_or(0.0),
            cost: hundredths(get_double(data, cost_off)),
            work: work_hours(get_double(data, work_off)),
            calendar_unique_id: resource_calendar_map.get(&id).copied(),
        });
    }

    Ok(resources)
}

// ---------------------------------------------------------------------
// Assignments. Ported from MPP14Reader.processAssignmentData and
// FieldMap14.getDefaultAssignmentData. MPXJ's `ResourceAssignmentFactory`
// additionally reads timephased data, which is out of scope.
// ---------------------------------------------------------------------

mod assignment_field {
    pub const UNIQUE_ID: i32 = 0;
    pub const TASK_UNIQUE_ID: i32 = 1;
    pub const RESOURCE_UNIQUE_ID: i32 = 2;
    pub const START: i32 = 20;
    pub const FINISH: i32 = 21;
    pub const ASSIGNMENT_UNITS: i32 = 7;
    pub const WORK: i32 = 8;
    pub const ACTUAL_WORK: i32 = 10;
    pub const COST: i32 = 26;
    pub const ACTUAL_COST: i32 = 28;
}

fn read_assignments(
    container: &mut OleContainer,
    project_props: &Props,
) -> MppResult<Vec<Assignment>> {
    let dir = format!("{PROJECT_DIR}/TBkndAssn");
    if !container.storage_exists(&dir) {
        return Ok(Vec::new());
    }

    // Unlike tasks and resources, MPP14 assignment records always have a
    // fixed 110 byte size regardless of the on-disk field map; only the
    // offsets of individual fields within that record can differ.
    let field_map = FieldMap::from_props(
        project_props,
        &[ASSIGNMENT_FIELD_MAP, ASSIGNMENT_FIELD_MAP2],
    );
    let (_, uid_off) = field_map.fixed(assignment_field::UNIQUE_ID, 0, 0);
    let (_, task_uid_off) = field_map.fixed(assignment_field::TASK_UNIQUE_ID, 0, 4);
    let (_, resource_uid_off) = field_map.fixed(assignment_field::RESOURCE_UNIQUE_ID, 0, 8);
    let (_, start_off) = field_map.fixed(assignment_field::START, 0, 12);
    let (_, finish_off) = field_map.fixed(assignment_field::FINISH, 0, 16);
    let (_, units_off) = field_map.fixed(assignment_field::ASSIGNMENT_UNITS, 0, 46);
    let (_, work_off) = field_map.fixed(assignment_field::WORK, 0, 54);
    let (_, actual_work_off) = field_map.fixed(assignment_field::ACTUAL_WORK, 0, 62);
    let (_, cost_off) = field_map.fixed(assignment_field::COST, 0, 86);
    let (_, actual_cost_off) = field_map.fixed(assignment_field::ACTUAL_COST, 0, 94);

    let fixed_data_bytes = container.read_stream(&format!("{dir}/FixedData"))?;
    let fixed_data = FixedData::parse_fixed_size(&fixed_data_bytes, 110);

    let mut assignments = Vec::new();
    for i in 0..fixed_data.item_count() {
        let Some(data) = fixed_data.byte_array(i) else {
            continue;
        };
        if data.len() < 110 {
            continue;
        }

        let unique_id = get_int(data, uid_off);
        let task_unique_id = get_int(data, task_uid_off);
        let resource_unique_id = {
            let v = get_int(data, resource_uid_off);
            if v <= 0 {
                None
            } else {
                Some(v)
            }
        };

        assignments.push(Assignment {
            unique_id,
            task_unique_id,
            resource_unique_id,
            start: get_timestamp(data, start_off),
            finish: get_timestamp(data, finish_off),
            work: work_hours(get_double(data, work_off)),
            actual_work: work_hours(get_double(data, actual_work_off)),
            units: hundredths(get_double(data, units_off)).unwrap_or(0.0),
            cost: hundredths(get_double(data, cost_off)),
            actual_cost: hundredths(get_double(data, actual_cost_off)),
        });
    }

    Ok(assignments)
}
