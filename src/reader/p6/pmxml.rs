//! Ported from MPXJ: src/main/java/org/mpxj/primavera/PrimaveraPMFileReader.java,
//! XmlProjectReader.java, XmlContextReader.java and XmlReaderHelper.java.
//! Copyright (c) Packwood Software, Jon Iles. Licensed under the GNU Lesser
//! General Public License, version 2.1 or later.
//!
//! Reader for Primavera P6 PMXML exports (`APIBusinessObjects` documents).
//! Element values are translated into the XER-flavoured vocabulary of the
//! shared [`P6Data`] representation, so the semantic mapping in
//! `reader::p6::build` is common to both formats.

use roxmltree::Node;

use super::*;
use crate::error::{MppError, MppResult};
use crate::model::Project;

/// Read and parse a PMXML file already loaded into memory.
pub(crate) fn read_pmxml_bytes(bytes: &[u8]) -> MppResult<Project> {
    build_project(parse(bytes)?)
}

fn parse(bytes: &[u8]) -> MppResult<P6Data> {
    let text = String::from_utf8_lossy(bytes);
    let doc = roxmltree::Document::parse(&text).map_err(|e| MppError::UnsupportedVersion {
        detected: format!("not a PMXML file ({e})"),
    })?;

    let root = doc.root_element();
    if root.tag_name().name() != "APIBusinessObjects" {
        return Err(MppError::UnsupportedVersion {
            detected: format!(
                "not a PMXML file (root element is <{}>)",
                root.tag_name().name()
            ),
        });
    }

    let mut data = P6Data::default();

    // Global preferences.
    if let Some(prefs) = child(root, "GlobalPreferences") {
        data.hours_per_day = child_f64(prefs, "HoursPerDay");
        data.hours_per_week = child_f64(prefs, "HoursPerWeek");
        data.hours_per_month = child_f64(prefs, "HoursPerMonth");
        data.week_start_day = child_i32(prefs, "StartDayOfWeek");

        // Base currency.
        if let Some(base_id) = child_i32(prefs, "BaseCurrencyObjectId") {
            for currency in children(root, "Currency") {
                if child_i32(currency, "ObjectId") == Some(base_id) {
                    data.currency_symbol = child_string(currency, "Symbol");
                    data.currency_code = child_string(currency, "Id");
                    data.currency_digits = child_i32(currency, "DecimalPlaces");
                }
            }
        }
    }

    // Calendars: global ones live directly under the root, project ones
    // inside each <Project>.
    for calendar in children(root, "Calendar") {
        data.calendars.push(parse_calendar(calendar));
    }

    // Resources and their rates (global).
    for resource in children(root, "Resource") {
        let Some(rsrc_id) = child_i32(resource, "ObjectId") else {
            continue;
        };
        data.resources.push(P6Resource {
            rsrc_id,
            name: child_string(resource, "Name"),
            short_name: child_string(resource, "Id"),
            email_addr: child_string(resource, "EmailAddress"),
            rsrc_type: child_str(resource, "ResourceType")
                .and_then(resource_type_from_xml)
                .map(str::to_string),
            clndr_id: child_i32(resource, "CalendarObjectId"),
            cost_per_qty: None,
            max_qty_per_hr: child_f64(resource, "MaxUnitsPerTime"),
        });
    }
    let mut latest_rate = LatestRates::new();
    for rate in children(root, "ResourceRate") {
        let Some(rsrc_id) = child_i32(rate, "ResourceObjectId") else {
            continue;
        };
        let effective = child_datetime(rate, "EffectiveDate");
        let entry = latest_rate.entry(rsrc_id).or_insert((None, None, None));
        if entry.0.is_none() || effective >= entry.0 {
            *entry = (
                effective,
                child_f64(rate, "PricePerUnit"),
                child_f64(rate, "MaxUnitsPerTime"),
            );
        }
    }
    for resource in &mut data.resources {
        if let Some((_, rate, max_units)) = latest_rate.get(&resource.rsrc_id) {
            resource.cost_per_qty = *rate;
            if max_units.is_some() {
                resource.max_qty_per_hr = *max_units;
            }
        }
    }

    // Projects. Baseline projects (<BaselineProject>) are out of scope,
    // matching this crate's single-schedule model; the planned-value
    // baseline is populated by the builder instead.
    for project in children(root, "Project") {
        let proj_id = child_i32(project, "ObjectId");

        for calendar in children(project, "Calendar") {
            data.calendars.push(parse_calendar(calendar));
        }

        data.projects.push(P6Project {
            proj_id,
            short_name: child_string(project, "Id"),
            // PMXML marks the projects a file references but does not
            // contain with External=true; the exported project is the
            // non-external one, ported from XmlProjectReader.
            export_flag: !child_bool(project, "External"),
            last_recalc_date: child_datetime(project, "DataDate"),
            plan_start_date: child_datetime(project, "PlannedStartDate"),
            scd_end_date: child_datetime(project, "ScheduledFinishDate"),
            critical_drtn_hr_cnt: child_f64(project, "CriticalActivityFloatLimit"),
            critical_path_type_is_longest_path: child_str(project, "CriticalActivityPathType")
                == Some("Longest Path"),
            clndr_id: child_i32(project, "ActivityDefaultCalendarObjectId"),
            wbs_code_separator: child_string(project, "WBSCodeSeparator"),
            guid: parse_guid(child_str(project, "GUID")),
        });

        for wbs in children(project, "WBS") {
            let Some(wbs_id) = child_i32(wbs, "ObjectId") else {
                continue;
            };
            data.wbs.push(P6Wbs {
                wbs_id,
                parent_wbs_id: child_i32(wbs, "ParentObjectId"),
                proj_id,
                name: child_string(wbs, "Name"),
                short_name: child_string(wbs, "Code"),
                seq_num: child_i32(wbs, "SequenceNumber"),
            });
        }

        for activity in children(project, "Activity") {
            let Some(task_id) = child_i32(activity, "ObjectId") else {
                continue;
            };
            data.activities.push(P6Activity {
                task_id,
                proj_id: child_i32(activity, "ProjectObjectId").or(proj_id),
                wbs_id: child_i32(activity, "WBSObjectId"),
                name: child_string(activity, "Name"),
                task_code: child_string(activity, "Id"),
                task_type: child_str(activity, "Type")
                    .and_then(activity_type_from_xml)
                    .map(str::to_string),
                status_code: child_str(activity, "Status")
                    .and_then(activity_status_from_xml)
                    .map(str::to_string),
                clndr_id: child_i32(activity, "CalendarObjectId"),
                target_drtn_hr_cnt: child_f64(activity, "PlannedDuration"),
                remain_drtn_hr_cnt: child_f64(activity, "RemainingDuration"),
                at_completion_drtn_hr_cnt: child_f64(activity, "AtCompletionDuration"),
                // PMXML stores percentages as fractions (0.6 = 60%),
                // ported from XmlProjectReader.reversePercentage.
                duration_pct_complete: child_f64(activity, "DurationPercentComplete")
                    .map(|f| f * 100.0),
                act_start_date: child_datetime(activity, "ActualStartDate"),
                act_end_date: child_datetime(activity, "ActualFinishDate"),
                restart_date: child_datetime(activity, "RemainingEarlyStartDate"),
                reend_date: child_datetime(activity, "RemainingEarlyFinishDate"),
                target_start_date: child_datetime(activity, "PlannedStartDate"),
                target_end_date: child_datetime(activity, "PlannedFinishDate"),
                // P6 leaves early/late dates out of PMXML; the remaining
                // early/late dates are their equivalents for incomplete
                // work, ported from XmlProjectReader's comment on this.
                early_start_date: child_datetime(activity, "RemainingEarlyStartDate"),
                early_end_date: child_datetime(activity, "RemainingEarlyFinishDate"),
                late_start_date: child_datetime(activity, "RemainingLateStartDate"),
                late_end_date: child_datetime(activity, "RemainingLateFinishDate"),
                start_date: child_datetime(activity, "StartDate"),
                finish_date: child_datetime(activity, "FinishDate"),
                cstr_type: child_str(activity, "PrimaryConstraintType")
                    .and_then(constraint_type_from_xml)
                    .map(str::to_string),
                cstr_date: child_datetime(activity, "PrimaryConstraintDate"),
                total_float_hr_cnt: child_f64(activity, "TotalFloat"),
                free_float_hr_cnt: child_f64(activity, "FreeFloat"),
                driving_path: child_bool(activity, "IsLongestPath"),
                act_work_qty: child_f64(activity, "ActualLaborUnits"),
                act_equip_qty: child_f64(activity, "ActualNonLaborUnits"),
                remain_work_qty: child_f64(activity, "RemainingLaborUnits"),
                remain_equip_qty: child_f64(activity, "RemainingNonLaborUnits"),
                target_work_qty: child_f64(activity, "PlannedLaborUnits"),
                target_equip_qty: child_f64(activity, "PlannedNonLaborUnits"),
            });
        }

        for relation in children(project, "Relationship") {
            let (Some(pred), Some(succ)) = (
                child_i32(relation, "PredecessorActivityObjectId"),
                child_i32(relation, "SuccessorActivityObjectId"),
            ) else {
                continue;
            };
            data.relations.push(P6Relation {
                task_pred_id: child_i32(relation, "ObjectId"),
                task_id: succ,
                pred_task_id: pred,
                pred_type: child_str(relation, "Type")
                    .and_then(relation_type_from_xml)
                    .map(str::to_string),
                lag_hr_cnt: child_f64(relation, "Lag"),
            });
        }

        for assignment in children(project, "ResourceAssignment") {
            let (Some(taskrsrc_id), Some(task_id)) = (
                child_i32(assignment, "ObjectId"),
                child_i32(assignment, "ActivityObjectId"),
            ) else {
                continue;
            };
            data.assignments.push(P6Assignment {
                taskrsrc_id,
                task_id,
                rsrc_id: child_i32(assignment, "ResourceObjectId"),
                act_start_date: child_datetime(assignment, "ActualStartDate"),
                act_end_date: child_datetime(assignment, "ActualFinishDate"),
                restart_date: child_datetime(assignment, "RemainingStartDate"),
                reend_date: child_datetime(assignment, "RemainingFinishDate"),
                target_start_date: child_datetime(assignment, "PlannedStartDate"),
                target_end_date: child_datetime(assignment, "PlannedFinishDate"),
                remain_qty: child_f64(assignment, "RemainingUnits"),
                // PMXML's ActualUnits / ActualCost already include
                // overtime, so the overtime slots stay empty (the XER
                // reader sums regular + overtime instead).
                act_reg_qty: child_f64(assignment, "ActualUnits"),
                act_ot_qty: None,
                target_cost: child_f64(assignment, "PlannedCost"),
                remain_cost: child_f64(assignment, "RemainingCost"),
                act_reg_cost: child_f64(assignment, "ActualCost"),
                act_ot_cost: None,
                target_qty_per_hr: child_f64(assignment, "PlannedUnitsPerTime"),
            });
        }
    }

    Ok(data)
}

/// Parse one `<Calendar>` element, ported from
/// XmlReaderHelper.processCalendar.
fn parse_calendar(calendar: Node) -> P6Calendar {
    let mut result = P6Calendar {
        clndr_id: child_i32(calendar, "ObjectId").unwrap_or(0),
        name: child_string(calendar, "Name"),
        base_clndr_id: child_i32(calendar, "BaseCalendarObjectId"),
        is_default: child_bool(calendar, "IsDefault"),
        day_hr_cnt: child_f64(calendar, "HoursPerDay"),
        week_hr_cnt: child_f64(calendar, "HoursPerWeek"),
        month_hr_cnt: child_f64(calendar, "HoursPerMonth"),
        ..P6Calendar::default()
    };

    if let Some(work_week) = child(calendar, "StandardWorkWeek") {
        for hours in children(work_week, "StandardWorkHours") {
            let Some(index) = child_str(hours, "DayOfWeek").and_then(day_index) else {
                continue;
            };
            let slot = &mut result.days[index];
            slot.present = true;
            slot.ranges = work_time_ranges(hours);
        }
    }
    // Days without a StandardWorkHours entry default to a working Mon-Fri
    // 08:00-16:00 / non-working weekend, ported from
    // XmlReaderHelper.processCalendar's fallback.
    for (index, slot) in result.days.iter_mut().enumerate() {
        if !slot.present {
            slot.present = true;
            slot.ranges = if (1..=5).contains(&index) {
                vec![(8 * 3600, 16 * 3600)]
            } else {
                Vec::new()
            };
        }
    }

    if let Some(exceptions) = child(calendar, "HolidayOrExceptions") {
        for exception in children(exceptions, "HolidayOrException") {
            let Some(date) = child_str(exception, "Date")
                .and_then(parse_datetime)
                .map(|dt| dt.date)
            else {
                continue;
            };
            let mut ranges = work_time_ranges(exception);
            // A single 00:00-23:59 range is P6's encoding of an all-day
            // non-working exception.
            if ranges.len() == 1 && ranges[0].0 == 0 && ranges[0].1 >= 24 * 3600 {
                ranges.clear();
            }
            result.exceptions.push(P6CalendarException { date, ranges });
        }
    }

    result
}

/// Collect `<WorkTime><Start>...<Finish>...</WorkTime>` ranges. P6 writes
/// finish times as the last minute of the range (e.g. 15:59 for a range
/// that ends at 16:00), so one minute is added back, ported from
/// XmlReaderHelper.getEndTime.
fn work_time_ranges(node: Node) -> Vec<(u32, u32)> {
    children(node, "WorkTime")
        .filter_map(|work| {
            let start = parse_calendar_time(child_str(work, "Start")?)?;
            let finish = parse_calendar_time(child_str(work, "Finish")?)? + 60;
            Some((start, finish.min(24 * 3600)))
        })
        .collect()
}

/// Day-of-week name to Sunday-first index.
fn day_index(name: &str) -> Option<usize> {
    match name {
        "Sunday" => Some(0),
        "Monday" => Some(1),
        "Tuesday" => Some(2),
        "Wednesday" => Some(3),
        "Thursday" => Some(4),
        "Friday" => Some(5),
        "Saturday" => Some(6),
        _ => None,
    }
}

/// Parse a PMXML GUID (`{A1B2...}` or bare hex-with-dashes) into raw bytes.
fn parse_guid(value: Option<&str>) -> Option<[u8; 16]> {
    let hex: String = value?
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect();
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    if bytes.iter().all(|&b| b == 0) {
        None
    } else {
        Some(bytes)
    }
}

// ---- roxmltree convenience wrappers -----------------------------------

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

fn children<'a, 'input: 'a>(
    node: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn child_str<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    let text = child(node, name)?.text()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn child_string(node: Node, name: &str) -> Option<String> {
    child_str(node, name).map(str::to_string)
}

fn child_i32(node: Node, name: &str) -> Option<i32> {
    child_str(node, name)?.parse().ok()
}

fn child_f64(node: Node, name: &str) -> Option<f64> {
    child_str(node, name)?.parse().ok()
}

fn child_bool(node: Node, name: &str) -> bool {
    matches!(child_str(node, name), Some("true") | Some("1"))
}

fn child_datetime(node: Node, name: &str) -> Option<MppDateTime> {
    parse_datetime(child_str(node, name)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pmxml() -> String {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<APIBusinessObjects xmlns="http://xmlns.oracle.com/Primavera/P6/V23.12/API/BusinessObjects">
  <GlobalPreferences>
    <HoursPerDay>8</HoursPerDay>
    <HoursPerWeek>40</HoursPerWeek>
    <HoursPerMonth>172</HoursPerMonth>
    <StartDayOfWeek>1</StartDayOfWeek>
    <BaseCurrencyObjectId>1</BaseCurrencyObjectId>
  </GlobalPreferences>
  <Currency>
    <ObjectId>1</ObjectId>
    <Id>USD</Id>
    <Symbol>$</Symbol>
    <DecimalPlaces>2</DecimalPlaces>
  </Currency>
  <Calendar>
    <ObjectId>500</ObjectId>
    <Name>Standard 5 Day</Name>
    <IsDefault>true</IsDefault>
    <HoursPerDay>8</HoursPerDay>
    <HoursPerWeek>40</HoursPerWeek>
    <StandardWorkWeek>
      <StandardWorkHours>
        <DayOfWeek>Sunday</DayOfWeek>
      </StandardWorkHours>
      <StandardWorkHours>
        <DayOfWeek>Monday</DayOfWeek>
        <WorkTime><Start>08:00:00</Start><Finish>15:59:00</Finish></WorkTime>
      </StandardWorkHours>
      <StandardWorkHours>
        <DayOfWeek>Saturday</DayOfWeek>
      </StandardWorkHours>
    </StandardWorkWeek>
    <HolidayOrExceptions>
      <HolidayOrException>
        <Date>2024-01-01T00:00:00</Date>
        <WorkTime><Start>00:00:00</Start><Finish>23:59:00</Finish></WorkTime>
      </HolidayOrException>
    </HolidayOrExceptions>
  </Calendar>
  <Resource>
    <ObjectId>4000</ObjectId>
    <Id>EXC</Id>
    <Name>Excavator Crew</Name>
    <EmailAddress>crew@example.com</EmailAddress>
    <ResourceType>Labor</ResourceType>
    <CalendarObjectId>500</CalendarObjectId>
    <MaxUnitsPerTime>1</MaxUnitsPerTime>
  </Resource>
  <ResourceRate>
    <ObjectId>4500</ObjectId>
    <ResourceObjectId>4000</ResourceObjectId>
    <EffectiveDate>2023-01-01T00:00:00</EffectiveDate>
    <PricePerUnit>95.5</PricePerUnit>
  </ResourceRate>
  <Project>
    <ObjectId>100</ObjectId>
    <Id>PRJ1</Id>
    <Name>Sample Project</Name>
    <DataDate>2023-04-17T08:00:00</DataDate>
    <PlannedStartDate>2023-04-17T08:00:00</PlannedStartDate>
    <ScheduledFinishDate>2023-06-30T16:00:00</ScheduledFinishDate>
    <CriticalActivityFloatLimit>0</CriticalActivityFloatLimit>
    <CriticalActivityPathType>Critical Float</CriticalActivityPathType>
    <ActivityDefaultCalendarObjectId>500</ActivityDefaultCalendarObjectId>
    <WBSCodeSeparator>.</WBSCodeSeparator>
    <WBS>
      <ObjectId>1000</ObjectId>
      <Code>PRJ1</Code>
      <Name>Sample Project</Name>
      <SequenceNumber>1</SequenceNumber>
    </WBS>
    <WBS>
      <ObjectId>1001</ObjectId>
      <ParentObjectId>1000</ParentObjectId>
      <Code>P1</Code>
      <Name>Phase One</Name>
      <SequenceNumber>1</SequenceNumber>
    </WBS>
    <Activity>
      <ObjectId>2000</ObjectId>
      <Id>A1000</Id>
      <Name>Dig foundations</Name>
      <WBSObjectId>1001</WBSObjectId>
      <Type>Task Dependent</Type>
      <Status>In Progress</Status>
      <CalendarObjectId>500</CalendarObjectId>
      <PlannedDuration>40</PlannedDuration>
      <RemainingDuration>16</RemainingDuration>
      <AtCompletionDuration>40</AtCompletionDuration>
      <DurationPercentComplete>0.6</DurationPercentComplete>
      <ActualStartDate>2023-04-17T08:00:00</ActualStartDate>
      <PlannedStartDate>2023-04-17T08:00:00</PlannedStartDate>
      <PlannedFinishDate>2023-04-21T16:00:00</PlannedFinishDate>
      <RemainingEarlyStartDate>2023-04-20T08:00:00</RemainingEarlyStartDate>
      <RemainingEarlyFinishDate>2023-04-21T16:00:00</RemainingEarlyFinishDate>
      <StartDate>2023-04-17T08:00:00</StartDate>
      <FinishDate>2023-04-21T16:00:00</FinishDate>
      <TotalFloat>0</TotalFloat>
      <FreeFloat>0</FreeFloat>
      <ActualLaborUnits>24</ActualLaborUnits>
      <RemainingLaborUnits>16</RemainingLaborUnits>
      <PlannedLaborUnits>40</PlannedLaborUnits>
    </Activity>
    <Activity>
      <ObjectId>2001</ObjectId>
      <Id>A1010</Id>
      <Name>Project complete</Name>
      <WBSObjectId>1001</WBSObjectId>
      <Type>Finish Milestone</Type>
      <Status>Not Started</Status>
      <PlannedFinishDate>2023-04-21T16:00:00</PlannedFinishDate>
      <FinishDate>2023-04-21T16:00:00</FinishDate>
      <PrimaryConstraintType>Finish On</PrimaryConstraintType>
      <PrimaryConstraintDate>2023-04-21T16:00:00</PrimaryConstraintDate>
      <TotalFloat>8</TotalFloat>
      <FreeFloat>8</FreeFloat>
    </Activity>
    <Relationship>
      <ObjectId>3000</ObjectId>
      <PredecessorActivityObjectId>2000</PredecessorActivityObjectId>
      <SuccessorActivityObjectId>2001</SuccessorActivityObjectId>
      <Type>Finish to Start</Type>
      <Lag>0</Lag>
    </Relationship>
    <ResourceAssignment>
      <ObjectId>5000</ObjectId>
      <ActivityObjectId>2000</ActivityObjectId>
      <ResourceObjectId>4000</ResourceObjectId>
      <PlannedUnits>40</PlannedUnits>
      <RemainingUnits>16</RemainingUnits>
      <ActualUnits>24</ActualUnits>
      <PlannedCost>3820</PlannedCost>
      <RemainingCost>1528</RemainingCost>
      <ActualCost>2292</ActualCost>
      <PlannedUnitsPerTime>1</PlannedUnitsPerTime>
      <ActualStartDate>2023-04-17T08:00:00</ActualStartDate>
      <PlannedStartDate>2023-04-17T08:00:00</PlannedStartDate>
      <PlannedFinishDate>2023-04-21T16:00:00</PlannedFinishDate>
    </ResourceAssignment>
  </Project>
</APIBusinessObjects>"#
            .to_string()
    }

    #[test]
    fn parses_the_sample_end_to_end() {
        let project = read_pmxml_bytes(sample_pmxml().as_bytes()).expect("parse");

        let p = &project.properties;
        assert_eq!(p.title.as_deref(), Some("Sample Project"));
        assert_eq!(p.status_date.unwrap().to_string(), "2023-04-17T08:00:00");
        assert_eq!(p.minutes_per_day, 480);
        assert_eq!(p.minutes_per_week, 2400);
        assert_eq!(p.currency_code.as_deref(), Some("USD"));

        assert_eq!(project.tasks.len(), 4);
        let dig = &project.tasks[2];
        assert_eq!(dig.unique_id, 2000);
        assert!((dig.percent_complete - 60.0).abs() < 1e-9);
        assert!((dig.duration.unwrap().value - 40.0).abs() < 1e-9);
        assert!((dig.work.unwrap().value - 40.0).abs() < 1e-9);
        assert!(dig.critical);
        assert_eq!(dig.start.unwrap().to_string(), "2023-04-17T08:00:00");
        assert!((dig.cost.unwrap() - 3820.0).abs() < 1e-9);

        let milestone = &project.tasks[3];
        assert!(milestone.milestone);
        assert!(!milestone.critical);
        assert_eq!(milestone.start, milestone.finish);
        assert_eq!(milestone.predecessors.len(), 1);

        // Calendar: Monday 08:00-16:00 (15:59 finish + 1 minute).
        let cal = &project.calendars[0];
        let monday = cal.days[1].as_ref().unwrap();
        assert!(monday.working);
        assert_eq!(monday.ranges[0].start_seconds, 8 * 3600);
        assert_eq!(monday.ranges[0].end_seconds, 16 * 3600);
        // Sunday present but empty -> non-working; Tuesday absent ->
        // defaulted to working.
        assert!(!cal.days[0].as_ref().unwrap().working);
        assert!(cal.days[2].as_ref().unwrap().working);
        // All-day exception is non-working.
        assert_eq!(cal.exceptions.len(), 1);
        assert!(!cal.exceptions[0].working);
        assert_eq!(
            cal.exceptions[0].from_date.unwrap().to_string(),
            "2024-01-01"
        );

        // Resource rate came from the ResourceRate table.
        let rsrc = &project.resources[0];
        assert!((rsrc.standard_rate_per_hour - 95.5).abs() < 1e-9);
        assert!((rsrc.max_units - 100.0).abs() < 1e-9);
    }

    #[test]
    fn non_pmxml_xml_is_rejected() {
        let err = read_pmxml_bytes(b"<project/>").unwrap_err();
        assert!(matches!(err, MppError::UnsupportedVersion { .. }));
        let err = read_pmxml_bytes(b"not xml at all").unwrap_err();
        assert!(matches!(err, MppError::UnsupportedVersion { .. }));
    }

    #[test]
    fn guid_parsing() {
        assert_eq!(parse_guid(None), None);
        assert_eq!(parse_guid(Some("{}")), None);
        let guid = parse_guid(Some("{01020304-0506-0708-090A-0B0C0D0E0F10}")).unwrap();
        assert_eq!(guid[0], 0x01);
        assert_eq!(guid[15], 0x10);
    }
}
