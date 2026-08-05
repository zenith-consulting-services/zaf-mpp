//! Ported from MPXJ: src/main/java/org/mpxj/primavera/XerFile.java,
//! XerContextReader.java, XerProjectReader.java and
//! org/mpxj/common/Tokenizer.java. Copyright (c) Packwood Software, Jon
//! Iles. Licensed under the GNU Lesser General Public License, version 2.1
//! or later.
//!
//! Reader for Primavera P6 XER exports: a tab-delimited text format where
//! `%T` records open a table, `%F` records name its columns, `%R` records
//! carry rows, and `%E` ends the file. Parsed rows for the tables this
//! crate models are translated into the shared [`P6Data`] representation;
//! everything else is skipped.

use std::collections::HashMap;

use super::structured_text;
use super::*;
use crate::error::{corrupt, MppError, MppResult};
use crate::model::Project;

/// Read and parse an XER file already loaded into memory.
pub(crate) fn read_xer_bytes(bytes: &[u8]) -> MppResult<Project> {
    build_project(parse(bytes)?)
}

/// Character decoding. MPXJ defaults to Windows-1252 and lets callers
/// override; with no caller-supplied charset this port instead tries UTF-8
/// first (newer P6 versions can export UTF-8) and falls back to
/// Windows-1252, which cannot fail (every byte maps somewhere).
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| cp1252_char(b)).collect(),
    }
}

/// Windows-1252 to Unicode. Identical to Latin-1 outside 0x80..=0x9F.
fn cp1252_char(b: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}',
        '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        other => other as char,
    }
}

/// One parsed `%R` row: column name -> raw field text (empty = absent).
struct Row<'a> {
    columns: &'a HashMap<String, usize>,
    fields: Vec<String>,
    decimal_symbol: char,
    group_symbol: char,
}

impl Row<'_> {
    fn str(&self, name: &str) -> Option<&str> {
        let idx = *self.columns.get(name)?;
        // Field 0 of the record is the "%R" tag itself; the stored fields
        // start at record index 1, and the column map is built the same
        // way, so indexes line up directly.
        let value = self.fields.get(idx)?.as_str();
        if value.is_empty() {
            None
        } else {
            Some(value)
        }
    }

    fn string(&self, name: &str) -> Option<String> {
        self.str(name).map(unescape_quotes)
    }

    fn i32(&self, name: &str) -> Option<i32> {
        self.str(name)?.trim().parse().ok()
    }

    /// Numeric field, honouring the file's currency separators the way
    /// XerFile's DecimalFormat reparse does.
    fn f64(&self, name: &str) -> Option<f64> {
        let raw = self.str(name)?.trim();
        let normalised: String = raw
            .chars()
            .filter(|&c| c != self.group_symbol)
            .map(|c| if c == self.decimal_symbol { '.' } else { c })
            .collect();
        normalised.parse().ok()
    }

    fn datetime(&self, name: &str) -> Option<MppDateTime> {
        parse_datetime(self.str(name)?)
    }

    fn flag(&self, name: &str) -> bool {
        self.str(name) == Some("Y")
    }
}

/// XER escapes a literal `"` as `""`, ported from XerFile.unescapeQuotes.
fn unescape_quotes(value: &str) -> String {
    if value.contains("\"\"") {
        value.replace("\"\"", "\"")
    } else {
        value.to_string()
    }
}

/// A table accumulated during the scan.
#[derive(Default)]
struct Table {
    columns: HashMap<String, usize>,
    rows: Vec<Vec<String>>,
}

fn parse(bytes: &[u8]) -> MppResult<P6Data> {
    let text = decode(bytes);

    let mut tables: HashMap<String, Table> = HashMap::new();
    let mut current: Option<String> = None;
    let mut header_currency: Option<String> = None;
    let mut saw_header = false;

    for line in text.split(['\n']) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let record = tokenize(line);
        if record.is_empty() {
            continue;
        }

        match record[0].as_str() {
            "ERMHDR" => {
                saw_header = true;
                header_currency = record.get(8).cloned();
            }
            "%T" => {
                let name = record
                    .get(1)
                    .map(|n| n.to_lowercase())
                    .unwrap_or_default();
                if REQUIRED_TABLES.contains(&name.as_str()) {
                    tables.entry(name.clone()).or_default();
                    current = Some(name);
                } else {
                    current = None;
                }
            }
            "%F" => {
                if let Some(table) = current.as_ref().and_then(|n| tables.get_mut(n)) {
                    table.columns = record
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(i, name)| (name.to_lowercase(), i))
                        .collect();
                }
            }
            "%R" => {
                if let Some(table) = current.as_ref().and_then(|n| tables.get_mut(n)) {
                    table.rows.push(record);
                }
            }
            "%E" => break,
            // A field containing a line break spills onto a following
            // line whose first token is not a record tag; MPXJ treats
            // such lines as (misaligned) data rows. Skipping them keeps
            // every properly-tagged row intact, which is the part that
            // matters for the tables this crate reads.
            _ => {}
        }

        if !saw_header {
            return Err(corrupt("not an XER file: missing ERMHDR record"));
        }
    }

    if !saw_header {
        return Err(MppError::UnsupportedVersion {
            detected: "not an XER file (no ERMHDR header)".to_string(),
        });
    }

    // Default currency: the currtype row whose short name matches the
    // header's currency, ported from XerFile.processCurrency. It supplies
    // both the project currency properties and the separators used to
    // parse every numeric field.
    let mut decimal_symbol = '.';
    let mut group_symbol = ',';
    let mut currency_symbol = None;
    let mut currency_code = None;
    let mut currency_digits = None;
    if let Some(currtype) = tables.get("currtype") {
        for fields in &currtype.rows {
            let row = Row {
                columns: &currtype.columns,
                fields: fields.clone(),
                decimal_symbol: '.',
                group_symbol: ',',
            };
            let short_name = row.string("curr_short_name");
            let is_default = match (&short_name, &header_currency) {
                (Some(s), Some(h)) => s.eq_ignore_ascii_case(h),
                _ => false,
            };
            if is_default || (header_currency.is_none() && currency_code.is_none()) {
                decimal_symbol = row
                    .str("decimal_symbol")
                    .and_then(|s| s.chars().next())
                    .unwrap_or('.');
                group_symbol = row
                    .str("digit_group_symbol")
                    .and_then(|s| s.chars().next())
                    .unwrap_or(',');
                currency_symbol = row.string("curr_symbol");
                currency_code = short_name;
                currency_digits = row.i32("decimal_digit_cnt");
            }
        }
    }
    if currency_code.is_none() {
        currency_code = header_currency;
    }

    let mut data = P6Data {
        currency_symbol,
        currency_code,
        currency_digits,
        ..P6Data::default()
    };

    let for_each = |name: &str, mut f: Box<dyn FnMut(&Row) + '_>| {
        if let Some(table) = tables.get(name) {
            for fields in &table.rows {
                let row = Row {
                    columns: &table.columns,
                    fields: fields.clone(),
                    decimal_symbol,
                    group_symbol,
                };
                f(&row);
            }
        }
    };

    for_each(
        "project",
        Box::new(|row| {
            data.projects.push(P6Project {
                proj_id: row.i32("proj_id"),
                short_name: row.string("proj_short_name"),
                export_flag: row.flag("export_flag"),
                last_recalc_date: row.datetime("last_recalc_date"),
                plan_start_date: row.datetime("plan_start_date"),
                scd_end_date: row.datetime("scd_end_date"),
                critical_drtn_hr_cnt: row.f64("critical_drtn_hr_cnt"),
                // "CT_Drivpath" = longest path, "CT_TotFloat" = total float,
                // ported from CriticalActivityTypeHelper.
                critical_path_type_is_longest_path: row.str("critical_path_type")
                    == Some("CT_Drivpath"),
                clndr_id: row.i32("clndr_id"),
                wbs_code_separator: row.string("name_sep_char"),
                guid: None,
            });
        }),
    );

    for_each(
        "calendar",
        Box::new(|row| {
            let Some(clndr_id) = row.i32("clndr_id") else {
                return;
            };
            let mut calendar = P6Calendar {
                clndr_id,
                name: row.string("clndr_name"),
                base_clndr_id: row.i32("base_clndr_id"),
                is_default: row.flag("default_flag"),
                day_hr_cnt: row.f64("day_hr_cnt"),
                week_hr_cnt: row.f64("week_hr_cnt"),
                month_hr_cnt: row.f64("month_hr_cnt"),
                ..P6Calendar::default()
            };
            if let Some(clndr_data) = row.str("clndr_data") {
                apply_calendar_data(&mut calendar, clndr_data);
            }
            data.calendars.push(calendar);
        }),
    );

    for_each(
        "projwbs",
        Box::new(|row| {
            let Some(wbs_id) = row.i32("wbs_id") else {
                return;
            };
            data.wbs.push(P6Wbs {
                wbs_id,
                parent_wbs_id: row.i32("parent_wbs_id"),
                proj_id: row.i32("proj_id"),
                name: row.string("wbs_name"),
                short_name: row.string("wbs_short_name"),
                seq_num: row.i32("seq_num"),
            });
        }),
    );

    for_each(
        "task",
        Box::new(|row| {
            let Some(task_id) = row.i32("task_id") else {
                return;
            };
            data.activities.push(P6Activity {
                task_id,
                proj_id: row.i32("proj_id"),
                wbs_id: row.i32("wbs_id"),
                name: row.string("task_name"),
                task_code: row.string("task_code"),
                task_type: row.string("task_type"),
                status_code: row.string("status_code"),
                clndr_id: row.i32("clndr_id"),
                target_drtn_hr_cnt: row.f64("target_drtn_hr_cnt"),
                remain_drtn_hr_cnt: row.f64("remain_drtn_hr_cnt"),
                at_completion_drtn_hr_cnt: None,
                duration_pct_complete: None,
                act_start_date: row.datetime("act_start_date"),
                act_end_date: row.datetime("act_end_date"),
                restart_date: row.datetime("restart_date"),
                reend_date: row.datetime("reend_date"),
                target_start_date: row.datetime("target_start_date"),
                target_end_date: row.datetime("target_end_date"),
                early_start_date: row.datetime("early_start_date"),
                early_end_date: row.datetime("early_end_date"),
                late_start_date: row.datetime("late_start_date"),
                late_end_date: row.datetime("late_end_date"),
                start_date: None,
                finish_date: None,
                cstr_type: row.string("cstr_type"),
                cstr_date: row.datetime("cstr_date"),
                total_float_hr_cnt: row.f64("total_float_hr_cnt"),
                free_float_hr_cnt: row.f64("free_float_hr_cnt"),
                driving_path: row.flag("driving_path_flag"),
                act_work_qty: row.f64("act_work_qty"),
                act_equip_qty: row.f64("act_equip_qty"),
                remain_work_qty: row.f64("remain_work_qty"),
                remain_equip_qty: row.f64("remain_equip_qty"),
                target_work_qty: row.f64("target_work_qty"),
                target_equip_qty: row.f64("target_equip_qty"),
            });
        }),
    );

    for_each(
        "taskpred",
        Box::new(|row| {
            let (Some(task_id), Some(pred_task_id)) =
                (row.i32("task_id"), row.i32("pred_task_id"))
            else {
                return;
            };
            data.relations.push(P6Relation {
                task_pred_id: row.i32("task_pred_id"),
                task_id,
                pred_task_id,
                pred_type: row.string("pred_type"),
                lag_hr_cnt: row.f64("lag_hr_cnt"),
            });
        }),
    );

    for_each(
        "rsrc",
        Box::new(|row| {
            let Some(rsrc_id) = row.i32("rsrc_id") else {
                return;
            };
            data.resources.push(P6Resource {
                rsrc_id,
                name: row.string("rsrc_name"),
                short_name: row.string("rsrc_short_name"),
                email_addr: row.string("email_addr"),
                rsrc_type: row.string("rsrc_type"),
                clndr_id: row.i32("clndr_id"),
                cost_per_qty: None,
                max_qty_per_hr: None,
            });
        }),
    );

    // Rate tables: P6 defines entries by start date; the entry with the
    // latest start date carries the resource's current rate and max units
    // (MPXJ keeps the whole table; this model keeps one value).
    let mut latest_rate = LatestRates::new();
    for_each(
        "rsrcrate",
        Box::new(|row| {
            let Some(rsrc_id) = row.i32("rsrc_id") else {
                return;
            };
            let start = row.datetime("start_date");
            let entry = latest_rate.entry(rsrc_id).or_insert((None, None, None));
            if entry.0.is_none() || start >= entry.0 {
                *entry = (start, row.f64("cost_per_qty"), row.f64("max_qty_per_hr"));
            }
        }),
    );
    for resource in &mut data.resources {
        if let Some((_, rate, max_units)) = latest_rate.get(&resource.rsrc_id) {
            resource.cost_per_qty = *rate;
            resource.max_qty_per_hr = *max_units;
        }
    }

    for_each(
        "taskrsrc",
        Box::new(|row| {
            let (Some(taskrsrc_id), Some(task_id)) =
                (row.i32("taskrsrc_id"), row.i32("task_id"))
            else {
                return;
            };
            data.assignments.push(P6Assignment {
                taskrsrc_id,
                task_id,
                rsrc_id: row.i32("rsrc_id"),
                act_start_date: row.datetime("act_start_date"),
                act_end_date: row.datetime("act_end_date"),
                restart_date: row.datetime("restart_date"),
                reend_date: row.datetime("reend_date"),
                target_start_date: row.datetime("target_start_date"),
                target_end_date: row.datetime("target_end_date"),
                remain_qty: row.f64("remain_qty"),
                act_reg_qty: row.f64("act_reg_qty"),
                act_ot_qty: row.f64("act_ot_qty"),
                target_cost: row.f64("target_cost"),
                remain_cost: row.f64("remain_cost"),
                act_reg_cost: row.f64("act_reg_cost"),
                act_ot_cost: row.f64("act_ot_cost"),
                target_qty_per_hr: row.f64("target_qty_per_hr"),
            });
        }),
    );

    Ok(data)
}

/// Tables this reader consumes; every other `%T` section is skipped
/// wholesale, mirroring XerFile's required-tables filter.
const REQUIRED_TABLES: &[&str] = &[
    "project", "calendar", "projwbs", "task", "taskpred", "rsrc", "rsrcrate", "taskrsrc",
    "currtype",
];

/// Split one line into fields on tabs, honouring XER's quoting rules,
/// ported from org.mpxj.common.Tokenizer: a `"` opens a quoted section
/// only at the start of a field; inside it `""` is a literal quote and a
/// tab does not split.
fn tokenize(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut buffer = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if !quoted && buffer.is_empty() {
                    quoted = true;
                } else if quoted {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        buffer.push('"');
                    } else {
                        quoted = false;
                    }
                } else {
                    buffer.push('"');
                }
            }
            '\t' if !quoted => {
                fields.push(std::mem::take(&mut buffer));
            }
            _ => buffer.push(c),
        }
    }
    fields.push(buffer);
    fields
}

/// Decode a `clndr_data` structured-text blob into the calendar's day and
/// exception tables. Ported from TableContextReader.processCalendar and
/// friends. Day records are named "1" (Sunday) through "7" (Saturday);
/// exceptions carry `d` = days since 1899-12-30.
fn apply_calendar_data(calendar: &mut P6Calendar, clndr_data: &str) {
    let root = structured_text::parse(clndr_data);
    // The root record is CalendarData; tolerate blobs where the day /
    // exception lists are at the top level.
    let base = root.child("CalendarData").unwrap_or(&root);

    if let Some(days) = base.child("DaysOfWeek") {
        for day in &days.children {
            let Some(index) = day
                .record_name
                .as_deref()
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|n| (1..=7).contains(n))
            else {
                continue;
            };
            let slot = &mut calendar.days[index - 1];
            slot.present = true;
            slot.ranges = hour_ranges(&day.children);
        }
        // Days absent from the data are explicitly non-working in P6,
        // matching MPXJ's pass over all seven days.
        for slot in &mut calendar.days {
            if !slot.present {
                slot.present = true;
                slot.ranges = Vec::new();
            }
        }
    }

    if let Some(exceptions) = base.child("Exceptions") {
        for exception in &exceptions.children {
            let Some(days_from_epoch) = exception
                .attribute("d")
                .and_then(|d| d.parse::<i64>().ok())
            else {
                continue;
            };
            calendar.exceptions.push(P6CalendarException {
                date: EXCEPTION_EPOCH.plus_days(days_from_epoch),
                ranges: hour_ranges(&exception.children),
            });
        }
    }
}

/// Extract (start, end) second-of-day ranges from hour records with `s`
/// and `f` attributes. Incomplete or unparseable records are skipped,
/// matching MPXJ's lenient calendar handling.
fn hour_ranges(records: &[structured_text::StructuredTextRecord]) -> Vec<(u32, u32)> {
    records
        .iter()
        .filter_map(|r| {
            let start = parse_calendar_time(r.attribute("s")?)?;
            let finish = parse_calendar_time(r.attribute("f")?)?;
            // A finish of 00:00 means end-of-day.
            let finish = if finish == 0 { 24 * 3600 } else { finish };
            Some((start, finish))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_xer() -> String {
        let mut s = String::new();
        s.push_str("ERMHDR\t19.12\t2023-04-17\tProject\tadmin\tAdmin\tdbxDatabaseNoName\tProject Management\tUSD\r\n");
        s.push_str("%T\tCURRTYPE\r\n");
        s.push_str("%F\tcurr_id\tdecimal_digit_cnt\tcurr_symbol\tdecimal_symbol\tdigit_group_symbol\tcurr_short_name\r\n");
        s.push_str("%R\t1\t2\t$\t.\t,\tUSD\r\n");
        s.push_str("%T\tCALENDAR\r\n");
        s.push_str("%F\tclndr_id\tdefault_flag\tclndr_name\tbase_clndr_id\tclndr_data\tday_hr_cnt\tweek_hr_cnt\tmonth_hr_cnt\r\n");
        s.push_str("%R\t500\tY\tStandard 5 Day\t\t(0||CalendarData()((0||DaysOfWeek()((0||1()())(0||2()((0||0(s|08:00|f|16:00)())))(0||3()((0||0(s|08:00|f|16:00)())))(0||4()((0||0(s|08:00|f|16:00)())))(0||5()((0||0(s|08:00|f|16:00)())))(0||6()((0||0(s|08:00|f|16:00)())))(0||7()())))(0||Exceptions()((0||0(d|45292)())))))\t8\t40\t172\r\n");
        s.push_str("%T\tPROJECT\r\n");
        s.push_str("%F\tproj_id\tproj_short_name\texport_flag\tlast_recalc_date\tplan_start_date\tscd_end_date\tcritical_drtn_hr_cnt\tcritical_path_type\tclndr_id\tname_sep_char\r\n");
        s.push_str("%R\t100\tPRJ1\tY\t2023-04-17 08:00\t2023-04-17 08:00\t2023-06-30 16:00\t0\tCT_TotFloat\t500\t.\r\n");
        s.push_str("%T\tPROJWBS\r\n");
        s.push_str("%F\twbs_id\tparent_wbs_id\tproj_id\twbs_name\twbs_short_name\tseq_num\r\n");
        s.push_str("%R\t1000\t\t100\tSample Project\tPRJ1\t1\r\n");
        s.push_str("%R\t1001\t1000\t100\tPhase One\tP1\t1\r\n");
        s.push_str("%T\tTASK\r\n");
        s.push_str("%F\ttask_id\tproj_id\twbs_id\ttask_name\ttask_code\ttask_type\tstatus_code\tclndr_id\ttarget_drtn_hr_cnt\tremain_drtn_hr_cnt\tact_start_date\tact_end_date\ttarget_start_date\ttarget_end_date\tearly_start_date\tearly_end_date\tlate_start_date\tlate_end_date\tcstr_type\tcstr_date\ttotal_float_hr_cnt\tfree_float_hr_cnt\tdriving_path_flag\tact_work_qty\tremain_work_qty\ttarget_work_qty\trestart_date\treend_date\r\n");
        s.push_str("%R\t2000\t100\t1001\tDig foundations\tA1000\tTT_Task\tTK_Active\t500\t40\t16\t2023-04-17 08:00\t\t2023-04-17 08:00\t2023-04-21 16:00\t2023-04-17 08:00\t2023-04-21 16:00\t2023-04-17 08:00\t2023-04-21 16:00\t\t\t0\t0\tY\t24\t16\t40\t2023-04-20 08:00\t2023-04-21 16:00\r\n");
        s.push_str("%R\t2001\t100\t1001\tProject complete\tA1010\tTT_FinMile\tTK_NotStart\t500\t0\t0\t\t\t\t2023-04-21 16:00\t\t2023-04-21 16:00\t\t2023-04-21 16:00\tCS_MEO\t2023-04-21 16:00\t8\t8\tN\t\t\t\t\t\r\n");
        s.push_str("%T\tTASKPRED\r\n");
        s.push_str("%F\ttask_pred_id\ttask_id\tpred_task_id\tpred_type\tlag_hr_cnt\r\n");
        s.push_str("%R\t3000\t2001\t2000\tPR_FS\t0\r\n");
        s.push_str("%T\tRSRC\r\n");
        s.push_str("%F\trsrc_id\trsrc_name\trsrc_short_name\temail_addr\trsrc_type\tclndr_id\r\n");
        s.push_str("%R\t4000\tExcavator Crew\tEXC\tcrew@example.com\tRT_Labor\t500\r\n");
        s.push_str("%T\tRSRCRATE\r\n");
        s.push_str("%F\trsrcrate_id\trsrc_id\tstart_date\tcost_per_qty\tmax_qty_per_hr\r\n");
        s.push_str("%R\t4500\t4000\t2023-01-01 00:00\t95.5\t1\r\n");
        s.push_str("%T\tTASKRSRC\r\n");
        s.push_str("%F\ttaskrsrc_id\ttask_id\trsrc_id\ttarget_qty\tremain_qty\tact_reg_qty\tact_ot_qty\ttarget_cost\tremain_cost\tact_reg_cost\tact_ot_cost\ttarget_qty_per_hr\ttarget_start_date\ttarget_end_date\tact_start_date\r\n");
        s.push_str("%R\t5000\t2000\t4000\t40\t16\t24\t0\t3820\t1528\t2292\t0\t1\t2023-04-17 08:00\t2023-04-21 16:00\t2023-04-17 08:00\r\n");
        s.push_str("%E\r\n");
        s
    }

    #[test]
    fn parses_the_sample_end_to_end() {
        let project = read_xer_bytes(sample_xer().as_bytes()).expect("parse");

        // Properties.
        let p = &project.properties;
        assert_eq!(p.title.as_deref(), Some("Sample Project"));
        assert_eq!(p.status_date.unwrap().to_string(), "2023-04-17T08:00:00");
        assert_eq!(p.minutes_per_day, 480);
        assert_eq!(p.minutes_per_week, 2400);
        assert_eq!(p.currency_code.as_deref(), Some("USD"));
        assert_eq!(p.currency_symbol.as_deref(), Some("$"));
        assert_eq!(p.default_calendar_name.as_deref(), Some("Standard 5 Day"));

        // Task tree: 2 WBS summaries + 2 activities in outline order.
        assert_eq!(project.tasks.len(), 4);
        let root = &project.tasks[0];
        assert!(root.summary);
        assert_eq!(root.name.as_deref(), Some("Sample Project"));
        assert_eq!(root.outline_level, 1);
        assert_eq!(root.id, 1);

        let phase = &project.tasks[1];
        assert!(phase.summary);
        assert_eq!(phase.parent_task_unique_id, Some(1000));
        assert_eq!(phase.wbs.as_deref(), Some("PRJ1.P1"));

        let dig = &project.tasks[2];
        assert_eq!(dig.unique_id, 2000);
        assert_eq!(dig.name.as_deref(), Some("Dig foundations"));
        assert!(!dig.summary);
        assert_eq!(dig.outline_level, 3);
        assert_eq!(dig.wbs.as_deref(), Some("PRJ1.P1"));
        assert_eq!(dig.actual_start.unwrap().to_string(), "2023-04-17T08:00:00");
        // Duration % complete: (40 - 16) * 100 / 40 = 60.
        assert!((dig.percent_complete - 60.0).abs() < 1e-9);
        // Actual duration 24h, at-completion 40h.
        assert!((dig.actual_duration.unwrap().value - 24.0).abs() < 1e-9);
        assert!((dig.duration.unwrap().value - 40.0).abs() < 1e-9);
        // Work: 24 actual + 16 remaining.
        assert!((dig.work.unwrap().value - 40.0).abs() < 1e-9);
        // Slack: started, not finished -> start slack 0, finish = total.
        assert_eq!(dig.start_slack.unwrap().value, 0.0);
        assert_eq!(dig.total_slack.unwrap().value, 0.0);
        assert!(dig.critical);
        // Baseline from planned values.
        assert_eq!(
            dig.baseline.start.unwrap().to_string(),
            "2023-04-17T08:00:00"
        );
        assert!((dig.baseline.duration.unwrap().value - 40.0).abs() < 1e-9);
        // Costs rolled up from the assignment: 2292 actual + 1528 remaining.
        assert!((dig.cost.unwrap() - 3820.0).abs() < 1e-9);
        assert!((dig.actual_cost.unwrap() - 2292.0).abs() < 1e-9);
        assert!((dig.baseline.cost.unwrap() - 3820.0).abs() < 1e-9);

        let milestone = &project.tasks[3];
        assert_eq!(milestone.unique_id, 2001);
        assert!(milestone.milestone);
        assert!(!milestone.critical); // 8h float > 0h limit
        assert_eq!(
            milestone.constraint_type,
            crate::model::ConstraintType::FinishOn
        );
        // Finish milestone: start mirrors finish.
        assert_eq!(
            milestone.start.unwrap().to_string(),
            "2023-04-21T16:00:00"
        );
        assert_eq!(milestone.start, milestone.finish);
        // Predecessor list.
        assert_eq!(milestone.predecessors.len(), 1);
        let rel = &milestone.predecessors[0];
        assert_eq!(rel.predecessor_task_unique_id, 2000);
        assert_eq!(rel.successor_task_unique_id, 2001);
        assert_eq!(rel.relation_type, crate::model::RelationType::FinishStart);

        // Summary rollup.
        assert_eq!(phase.start, dig.start.min(milestone.start));
        assert_eq!(phase.finish.unwrap().to_string(), "2023-04-21T16:00:00");
        assert!((phase.cost.unwrap() - 3820.0).abs() < 1e-9);

        // Resource with its latest rate.
        assert_eq!(project.resources.len(), 1);
        let rsrc = &project.resources[0];
        assert_eq!(rsrc.name.as_deref(), Some("Excavator Crew"));
        assert!((rsrc.standard_rate_per_hour - 95.5).abs() < 1e-9);
        assert!((rsrc.max_units - 100.0).abs() < 1e-9);

        // Assignment.
        assert_eq!(project.assignments.len(), 1);
        let a = &project.assignments[0];
        assert_eq!(a.task_unique_id, 2000);
        assert_eq!(a.resource_unique_id, Some(4000));
        assert!((a.work.unwrap().value - 40.0).abs() < 1e-9);
        assert!((a.units - 100.0).abs() < 1e-9);
        assert!((a.cost.unwrap() - 3820.0).abs() < 1e-9);

        // Calendar decoded from clndr_data.
        assert_eq!(project.calendars.len(), 1);
        let cal = &project.calendars[0];
        assert_eq!(cal.unique_id, 500);
        let monday = cal.days[1].as_ref().unwrap();
        assert!(monday.working);
        assert_eq!(monday.ranges[0].start_seconds, 8 * 3600);
        assert_eq!(monday.ranges[0].end_seconds, 16 * 3600);
        let sunday = cal.days[0].as_ref().unwrap();
        assert!(!sunday.working);
        // Exception: 45292 days after 1899-12-30 = 2024-01-01, non-working.
        assert_eq!(cal.exceptions.len(), 1);
        assert_eq!(cal.exceptions[0].from_date.unwrap().to_string(), "2024-01-01");
        assert!(!cal.exceptions[0].working);
    }

    #[test]
    fn non_xer_input_is_rejected() {
        let err = read_xer_bytes(b"this is not an XER file").unwrap_err();
        assert!(matches!(
            err,
            MppError::Corrupt(_) | MppError::UnsupportedVersion { .. }
        ));
    }

    #[test]
    fn quoted_fields_with_tabs_and_escaped_quotes() {
        let fields = tokenize("%R\t1\t\"has\ttab\"\t\"say \"\"hi\"\"\"\tplain");
        assert_eq!(fields[2], "has\ttab");
        assert_eq!(fields[3], "say \"hi\"");
        assert_eq!(fields[4], "plain");
    }

    #[test]
    fn cp1252_fallback_decodes_high_bytes() {
        // 0x93/0x94 are curly quotes in CP1252 and invalid UTF-8 alone.
        let bytes = [0x93u8, b'x', 0x94];
        let decoded = decode(&bytes);
        assert_eq!(decoded, "\u{201C}x\u{201D}");
    }

    #[test]
    fn export_flag_selects_the_exported_project() {
        // Two projects; the second is flagged for export.
        let mut s = String::new();
        s.push_str("ERMHDR\t19.12\t2023-04-17\tProject\tadmin\tAdmin\tdb\tPM\tUSD\r\n");
        s.push_str("%T\tPROJECT\r\n");
        s.push_str("%F\tproj_id\tproj_short_name\texport_flag\r\n");
        s.push_str("%R\t1\tOTHER\tN\r\n");
        s.push_str("%R\t2\tMAIN\tY\r\n");
        s.push_str("%T\tPROJWBS\r\n");
        s.push_str("%F\twbs_id\tparent_wbs_id\tproj_id\twbs_name\twbs_short_name\tseq_num\r\n");
        s.push_str("%R\t10\t\t1\tOther Root\tOTHER\t1\r\n");
        s.push_str("%R\t20\t\t2\tMain Root\tMAIN\t1\r\n");
        s.push_str("%E\r\n");
        let project = read_xer_bytes(s.as_bytes()).expect("parse");
        assert_eq!(project.properties.title.as_deref(), Some("Main Root"));
        assert_eq!(project.tasks.len(), 1);
    }
}
