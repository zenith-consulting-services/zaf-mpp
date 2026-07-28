//! Ported from MPXJ: src/main/java/org/mpxj/mpp/MPPUtility.java
//! Copyright (c) Packwood Software, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Byte-level decode helpers shared by every MPP14 block reader: dates,
//! times, timestamps, durations, and UTF-16LE / Latin-1 strings.
//!
//! MPP epoch: MPXJ's `MicrosoftProjectConstants.EPOCH_DATE`, 1983-12-31
//! 00:00:00. Dates are stored as a count of days since this epoch, times as
//! tenths of a minute since midnight.

use std::fmt;
use std::time::Duration as StdDuration;

/// Naive (no timezone) date, matching MPXJ's use of `java.time.LocalDate`.
///
/// Serialises as a plain ISO 8601 date string (`"2006-08-23"`) rather than
/// a `{year, month, day}` object, so consumers deserializing the JSON
/// output (a .NET `DateTime`/`DateOnly` via `System.Text.Json`, for
/// example) get it for free without a custom converter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MppDate {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl fmt::Display for MppDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl serde::Serialize for MppDate {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

const DAYS_IN_MONTH: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> i64 {
    if month == 2 && is_leap_year(year) {
        29
    } else {
        DAYS_IN_MONTH[(month - 1) as usize]
    }
}

impl MppDate {
    /// Construct a date directly from its components, with no validation.
    pub const fn new(year: i32, month: u8, day: u8) -> Self {
        MppDate { year, month, day }
    }

    /// Add (or subtract, if negative) a number of days.
    pub fn plus_days(self, days: i64) -> MppDate {
        let mut year = self.year;
        let mut month = self.month;
        let mut day = self.day as i64 + days;

        loop {
            if day < 1 {
                month = if month == 1 { 12 } else { month - 1 };
                if month == 12 {
                    year -= 1;
                }
                day += days_in_month(year, month);
            } else {
                let dim = days_in_month(year, month);
                if day > dim {
                    day -= dim;
                    month = if month == 12 { 1 } else { month + 1 };
                    if month == 1 {
                        year += 1;
                    }
                } else {
                    break;
                }
            }
        }
        MppDate {
            year,
            month,
            day: day as u8,
        }
    }
}

/// Naive local date+time, matching MPXJ's use of `java.time.LocalDateTime`.
///
/// Serialises as a plain ISO 8601 timestamp string (`"2006-08-23T08:00:00"`)
/// rather than a `{date, seconds_since_midnight}` object; see
/// [`MppDate`]'s docs for why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MppDateTime {
    pub date: MppDate,
    pub seconds_since_midnight: u32,
}

impl MppDateTime {
    /// Add a day offset and a signed second offset to a base date,
    /// normalising the seconds into a valid day (carrying into the date if
    /// they overflow a day in either direction).
    pub fn plus_days_and_seconds(base: MppDate, days: i64, seconds: i64) -> MppDateTime {
        let extra_days = seconds.div_euclid(86400);
        let secs = seconds.rem_euclid(86400);
        MppDateTime {
            date: base.plus_days(days + extra_days),
            seconds_since_midnight: secs as u32,
        }
    }

    /// The hour component of the time of day (0-23).
    pub fn hour(&self) -> u32 {
        self.seconds_since_midnight / 3600
    }

    /// The minute component of the time of day (0-59).
    pub fn minute(&self) -> u32 {
        (self.seconds_since_midnight / 60) % 60
    }

    /// The seconds component of the time of day (0-59).
    pub fn second(&self) -> u32 {
        self.seconds_since_midnight % 60
    }
}

impl fmt::Display for MppDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}T{:02}:{:02}:{:02}",
            self.date,
            self.hour(),
            self.minute(),
            self.second()
        )
    }
}

impl serde::Serialize for MppDateTime {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Epoch used by every date/time field in an MPP file: 1983-12-31.
pub const EPOCH_DATE: MppDate = MppDate::new(1983, 12, 31);

/// Ported from `MPPUtility.getByte`.
pub fn get_byte(data: &[u8], offset: usize) -> u8 {
    data.get(offset).copied().unwrap_or(0)
}

/// Ported from `ByteArrayHelper.getShort`: little-endian i16, widened to i32
/// as MPXJ's helper returns an `int`.
pub fn get_short(data: &[u8], offset: usize) -> i32 {
    if offset + 2 > data.len() {
        return 0;
    }
    i16::from_le_bytes([data[offset], data[offset + 1]]) as i32
}

/// Ported from `ByteArrayHelper.getInt`: little-endian i32.
pub fn get_int(data: &[u8], offset: usize) -> i32 {
    if offset + 4 > data.len() {
        return 0;
    }
    i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Ported from `ByteArrayHelper.getLong`: little-endian i64.
pub fn get_long(data: &[u8], offset: usize) -> i64 {
    if offset + 8 > data.len() {
        return 0;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&data[offset..offset + 8]);
    i64::from_le_bytes(buf)
}

/// Ported from `MPPUtility.getDouble`: little-endian f64, NaN mapped to 0.0
/// exactly as MPXJ does.
pub fn get_double(data: &[u8], offset: usize) -> f64 {
    let bits = get_long(data, offset) as u64;
    let result = f64::from_bits(bits);
    if result.is_nan() {
        0.0
    } else {
        result
    }
}

/// Ported from `MPPUtility.getGUID`. MPP stores GUIDs with the first three
/// fields byte-swapped (Windows GUID wire format); this returns the 16 raw
/// bytes in that stored order, still useful as an opaque identifier.
/// Returns `None` if the bytes are all-zero or out of range, matching MPXJ.
pub fn get_guid(data: &[u8], offset: usize) -> Option<[u8; 16]> {
    if data.len() < offset + 16 {
        return None;
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&data[offset..offset + 16]);
    if buf.iter().all(|&b| b == 0) {
        None
    } else {
        Some(buf)
    }
}

/// Ported from `MPPUtility.getDate`. Stored as a count of days since
/// [`EPOCH_DATE`]; 65535 means "not available".
pub fn get_date(data: &[u8], offset: usize) -> Option<MppDate> {
    let days = get_short(data, offset) as i64 & 0xFFFF;
    if days == 65535 {
        None
    } else {
        Some(EPOCH_DATE.plus_days(days))
    }
}

/// Ported from `MPPUtility.getTime`. Stored as tenths of a minute since
/// midnight; returns seconds since midnight.
pub fn get_time_seconds(data: &[u8], offset: usize) -> u32 {
    let raw = get_short(data, offset) as i64 & 0xFFFF;
    let mut seconds = (raw / 10) * 60;
    if seconds > 86399 {
        seconds %= 86400;
    }
    seconds as u32
}

/// Ported from `MPPUtility.getDuration`. Stored as tenths of a minute;
/// returns the duration in milliseconds.
pub fn get_duration_millis(data: &[u8], offset: usize) -> i64 {
    let raw = get_short(data, offset) as i64;
    (raw * 60_000) / 10
}

/// Ported from `MPPUtility.getTimestamp`. A combined date (at offset + 2)
/// and time (at offset) value. Values with fewer than 100 days since the
/// epoch and a non-zero seconds component are treated as "not available",
/// matching MPXJ's heuristic for distinguishing real dates from `NA`.
pub fn get_timestamp(data: &[u8], offset: usize) -> Option<MppDateTime> {
    let days = get_short(data, offset + 2) as i64 & 0xFFFF;
    if days <= 1 || days == 65535 {
        return None;
    }
    let mut time = get_short(data, offset) as i64 & 0xFFFF;
    if time == 65535 {
        time = 0;
    }
    let result = MppDateTime::plus_days_and_seconds(EPOCH_DATE, days, time * 6);
    if days < 100 && result.second() != 0 {
        None
    } else {
        Some(result)
    }
}

/// Length in bytes of a nul-terminated UTF-16LE string starting at `offset`.
fn unicode_string_length_bytes(data: &[u8], offset: usize) -> usize {
    if data.is_empty() || offset >= data.len() {
        return 0;
    }
    let mut loop_idx = offset;
    while loop_idx + 1 < data.len() {
        if data[loop_idx] == 0 && data[loop_idx + 1] == 0 {
            return loop_idx - offset;
        }
        loop_idx += 2;
    }
    data.len() - offset
}

/// Ported from `MPPUtility.getUnicodeString(byte[], int)`.
pub fn get_unicode_string(data: &[u8], offset: usize) -> String {
    let len = unicode_string_length_bytes(data, offset);
    if len == 0 {
        return String::new();
    }
    let units: Vec<u16> = data[offset..offset + len]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Ported from `MPPUtility.getUnicodeString(byte[], int, int)`: as above,
/// but capped at `max_length` bytes.
pub fn get_unicode_string_max(data: &[u8], offset: usize, max_length: usize) -> String {
    let mut len = unicode_string_length_bytes(data, offset);
    if max_length > 0 && len > max_length {
        len = max_length;
    }
    if len == 0 {
        return String::new();
    }
    let units: Vec<u16> = data[offset..offset + len]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Ported from `MPPUtility.getString`: single byte characters, nul or
/// end-of-array terminated.
pub fn get_string(data: &[u8], offset: usize) -> String {
    let mut result = String::new();
    let mut i = offset;
    while i < data.len() {
        let c = data[i];
        if c == 0 {
            break;
        }
        result.push(c as char);
        i += 1;
    }
    result
}

/// MppDuration time units, matching the subset of MPXJ's `TimeUnit` enum this
/// crate needs to interpret stored durations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum TimeUnit {
    Minutes,
    ElapsedMinutes,
    Hours,
    ElapsedHours,
    Days,
    ElapsedDays,
    Weeks,
    ElapsedWeeks,
    Months,
    ElapsedMonths,
    Percent,
    ElapsedPercent,
}

const DURATION_UNITS_MASK: i32 = 0x1F;

/// Ported from `MPPUtility.getDurationTimeUnits(int, TimeUnit)`.
pub fn get_duration_time_units(raw: i32, project_default: TimeUnit) -> TimeUnit {
    match raw & DURATION_UNITS_MASK {
        3 => TimeUnit::Minutes,
        4 => TimeUnit::ElapsedMinutes,
        5 => TimeUnit::Hours,
        6 => TimeUnit::ElapsedHours,
        7 => TimeUnit::Days,
        8 => TimeUnit::ElapsedDays,
        9 => TimeUnit::Weeks,
        10 => TimeUnit::ElapsedWeeks,
        11 => TimeUnit::Months,
        12 => TimeUnit::ElapsedMonths,
        19 => TimeUnit::Percent,
        20 => TimeUnit::ElapsedPercent,
        21 => project_default,
        _ => TimeUnit::Days,
    }
}

/// A duration, normalised to whole milliseconds plus the unit it was
/// originally expressed in (kept so callers can round-trip display
/// formatting the way MPXJ's `MppDuration` type does).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct MppDuration {
    pub value: f64,
    pub units: TimeUnit,
}

/// A standard 8-hour day / 5-day (40-hour) week / 20-day month calendar.
/// Only correct for projects whose calendar actually uses these values;
/// real callers should read `ProjectProperties::minutes_per_day` /
/// `minutes_per_week` / `days_per_month` from the parsed project instead of
/// reaching for these. Provided for tests and synthetic fixtures that don't
/// have a real project's calendar in scope.
pub const STANDARD_MINUTES_PER_DAY: f64 = 480.0;
pub const STANDARD_MINUTES_PER_WEEK: f64 = 2400.0;
pub const STANDARD_DAYS_PER_MONTH: f64 = 20.0;

impl MppDuration {
    /// Re-express this duration in a different unit, using the project's
    /// actual calendar parameters (minutes per working day/week, working
    /// days per month) to convert "working time" units (`Days`, `Weeks`,
    /// `Months`) exactly the way MPXJ's `Duration.convertUnits(duration,
    /// fromUnits, toUnits, minutesPerDay, minutesPerWeek, daysPerMonth)`
    /// does. `Minutes`/`Hours` (and all `Elapsed*` variants, which measure
    /// real 24-hour clock time rather than working time) are calendar
    /// independent and convert via fixed ratios either way.
    ///
    /// Converting to or from `Percent`/`ElapsedPercent` is not meaningful
    /// (percentages have no time basis) and passes the value through
    /// unscaled, matching prior behaviour; callers shouldn't rely on the
    /// result in that case.
    pub fn to_unit(
        self,
        target: TimeUnit,
        minutes_per_day: f64,
        minutes_per_week: f64,
        days_per_month: f64,
    ) -> MppDuration {
        if self.units == target {
            return self;
        }

        if matches!(self.units, TimeUnit::Percent | TimeUnit::ElapsedPercent)
            || matches!(target, TimeUnit::Percent | TimeUnit::ElapsedPercent)
        {
            return MppDuration {
                value: self.value,
                units: target,
            };
        }

        // Step 1: express `self.value` in minutes, per `fromUnits`.
        let mut minutes = self.value;
        match self.units {
            TimeUnit::Months => minutes *= minutes_per_day * days_per_month,
            TimeUnit::ElapsedMonths => minutes *= 60.0 * 24.0 * 30.0,
            TimeUnit::Weeks => minutes *= minutes_per_week,
            TimeUnit::ElapsedWeeks => minutes *= 60.0 * 24.0 * 7.0,
            TimeUnit::Days => minutes *= minutes_per_day,
            TimeUnit::ElapsedDays => minutes *= 60.0 * 24.0,
            TimeUnit::Hours | TimeUnit::ElapsedHours => minutes *= 60.0,
            TimeUnit::Minutes | TimeUnit::ElapsedMinutes => {}
            TimeUnit::Percent | TimeUnit::ElapsedPercent => unreachable!("handled above"),
        }

        // Step 2: convert minutes into `toUnits`.
        let value = match target {
            TimeUnit::Minutes | TimeUnit::ElapsedMinutes => minutes,
            TimeUnit::Hours | TimeUnit::ElapsedHours => minutes / 60.0,
            TimeUnit::Days => {
                if minutes_per_day != 0.0 {
                    minutes / minutes_per_day
                } else {
                    0.0
                }
            }
            TimeUnit::ElapsedDays => minutes / (60.0 * 24.0),
            TimeUnit::Weeks => {
                if minutes_per_week != 0.0 {
                    minutes / minutes_per_week
                } else {
                    0.0
                }
            }
            TimeUnit::ElapsedWeeks => minutes / (60.0 * 24.0 * 7.0),
            TimeUnit::Months => {
                if minutes_per_day != 0.0 && days_per_month != 0.0 {
                    minutes / (minutes_per_day * days_per_month)
                } else {
                    0.0
                }
            }
            TimeUnit::ElapsedMonths => minutes / (60.0 * 24.0 * 30.0),
            TimeUnit::Percent | TimeUnit::ElapsedPercent => unreachable!("handled above"),
        };

        MppDuration {
            value,
            units: target,
        }
    }

    /// Convenience wrapper for callers without a real project calendar in
    /// scope (synthetic fixtures, tests): converts using a standard 8hr
    /// day / 40hr week / 20-day month calendar. Real callers with a parsed
    /// `Project` should call `to_unit` directly with its actual
    /// `ProjectProperties::minutes_per_day` / `minutes_per_week` /
    /// `days_per_month` instead.
    pub fn to_unit_standard(self, target: TimeUnit) -> MppDuration {
        self.to_unit(
            target,
            STANDARD_MINUTES_PER_DAY,
            STANDARD_MINUTES_PER_WEEK,
            STANDARD_DAYS_PER_MONTH,
        )
    }
}

/// Tenths-of-a-minute per unit of `units`, i.e. the divisor
/// `duration_from_raw` applies. `Percent`/`ElapsedPercent` have no time
/// basis, so they pass through unscaled (divisor 1).
fn tenths_of_minute_per_unit(units: TimeUnit) -> f64 {
    match units {
        TimeUnit::Minutes | TimeUnit::ElapsedMinutes => 10.0,
        TimeUnit::Hours | TimeUnit::ElapsedHours => 600.0,
        TimeUnit::Days => 4800.0,
        TimeUnit::ElapsedDays => 14400.0,
        TimeUnit::Weeks => 24000.0,
        TimeUnit::ElapsedWeeks => 100800.0,
        TimeUnit::Months => 96000.0,
        TimeUnit::ElapsedMonths => 432000.0,
        TimeUnit::Percent | TimeUnit::ElapsedPercent => 1.0,
    }
}

/// Ported from `MPPUtility.getDuration(double, TimeUnit)`. `raw` is stored
/// in tenths of a minute; this rescales it into the given unit.
pub fn duration_from_raw(raw: f64, units: TimeUnit) -> MppDuration {
    MppDuration {
        value: raw / tenths_of_minute_per_unit(units),
        units,
    }
}

/// Convenience wrapper matching `std::time::Duration` for callers that only
/// need wall-clock milliseconds, ignoring display units.
pub fn duration_as_std(raw_tenths_of_minute: i64) -> StdDuration {
    let millis = (raw_tenths_of_minute * 60_000) / 10;
    StdDuration::from_millis(millis.unsigned_abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_plus_zero_is_epoch() {
        assert_eq!(EPOCH_DATE.plus_days(0), EPOCH_DATE);
    }

    #[test]
    fn mpp_date_displays_as_iso8601() {
        assert_eq!(MppDate::new(2006, 8, 23).to_string(), "2006-08-23");
        // Zero-padded month and day.
        assert_eq!(MppDate::new(2006, 1, 5).to_string(), "2006-01-05");
    }

    #[test]
    fn mpp_date_serializes_as_a_plain_string() {
        let json = serde_json::to_string(&MppDate::new(2006, 8, 23)).unwrap();
        assert_eq!(json, "\"2006-08-23\"");
    }

    #[test]
    fn mpp_date_time_displays_as_iso8601() {
        let dt = MppDateTime {
            date: MppDate::new(2006, 8, 23),
            seconds_since_midnight: 28800, // 08:00:00
        };
        assert_eq!(dt.to_string(), "2006-08-23T08:00:00");
        assert_eq!(dt.hour(), 8);
        assert_eq!(dt.minute(), 0);
        assert_eq!(dt.second(), 0);
    }

    #[test]
    fn mpp_date_time_serializes_as_a_plain_string() {
        let dt = MppDateTime {
            date: MppDate::new(2006, 8, 23),
            seconds_since_midnight: 3661, // 01:01:01
        };
        let json = serde_json::to_string(&dt).unwrap();
        assert_eq!(json, "\"2006-08-23T01:01:01\"");
    }

    #[test]
    fn date_roundtrip_regular_days() {
        // 1 day after 1983-12-31 is 1984-01-01.
        assert_eq!(EPOCH_DATE.plus_days(1), MppDate::new(1984, 1, 1));
        // Crossing a leap year boundary: 1983-12-31 + 60 days.
        let d = EPOCH_DATE.plus_days(60);
        assert_eq!(d, MppDate::new(1984, 2, 29));
    }

    #[test]
    fn get_date_na_sentinel_is_none() {
        let data = [0xFF, 0xFF];
        assert_eq!(get_date(&data, 0), None);
    }

    #[test]
    fn get_unicode_string_reads_nul_terminated() {
        let mut data = Vec::new();
        for c in "Hi".encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&[0, 0]);
        assert_eq!(get_unicode_string(&data, 0), "Hi");
    }

    #[test]
    fn get_double_nan_becomes_zero() {
        let data = f64::NAN.to_le_bytes();
        assert_eq!(get_double(&data, 0), 0.0);
    }

    #[test]
    fn duration_from_raw_days() {
        // 4800 tenths-of-a-minute per day (8 hour day).
        let d = duration_from_raw(4800.0, TimeUnit::Days);
        assert!((d.value - 1.0).abs() < 1e-9);
    }

    #[test]
    fn plus_days_crosses_month_and_year_boundary_backwards() {
        // 1984-01-01 minus 1 day is back to the epoch, 1983-12-31.
        assert_eq!(MppDate::new(1984, 1, 1).plus_days(-1), EPOCH_DATE);
        // 1984-03-01 minus 61 days crosses back over the Feb 1984 leap day.
        assert_eq!(
            MppDate::new(1984, 3, 1).plus_days(-61),
            MppDate::new(1983, 12, 31)
        );
    }

    #[test]
    fn get_byte_in_and_out_of_range() {
        let data = [1u8, 2, 3];
        assert_eq!(get_byte(&data, 1), 2);
        assert_eq!(get_byte(&data, 10), 0);
    }

    #[test]
    fn get_short_truncated_data_is_zero() {
        let data = [1u8];
        assert_eq!(get_short(&data, 0), 0);
        let data = [0xFFu8, 0xFF];
        assert_eq!(get_short(&data, 0), -1);
    }

    #[test]
    fn get_int_truncated_data_is_zero() {
        let data = [1u8, 2, 3];
        assert_eq!(get_int(&data, 0), 0);
        let data = 42i32.to_le_bytes();
        assert_eq!(get_int(&data, 0), 42);
    }

    #[test]
    fn get_long_truncated_data_is_zero() {
        let data = [1u8; 4];
        assert_eq!(get_long(&data, 0), 0);
        let data = 12345i64.to_le_bytes();
        assert_eq!(get_long(&data, 0), 12345);
    }

    #[test]
    fn get_double_roundtrips_a_normal_value() {
        let data = 3.5f64.to_le_bytes();
        assert_eq!(get_double(&data, 0), 3.5);
    }

    #[test]
    fn get_guid_zero_bytes_is_none() {
        let data = [0u8; 16];
        assert_eq!(get_guid(&data, 0), None);
    }

    #[test]
    fn get_guid_out_of_range_is_none() {
        let data = [1u8; 8];
        assert_eq!(get_guid(&data, 0), None);
    }

    #[test]
    fn get_guid_nonzero_bytes_roundtrip() {
        let mut data = [0u8; 16];
        data[0] = 0xAB;
        assert_eq!(get_guid(&data, 0), Some(data));
    }

    #[test]
    fn get_date_valid_value() {
        // 1 day since the epoch: 1983-12-31 -> 1984-01-01.
        let data = 1i16.to_le_bytes();
        assert_eq!(get_date(&data, 0), Some(MppDate::new(1984, 1, 1)));
    }

    #[test]
    fn get_time_seconds_normal_and_wraparound() {
        // 600 tenths-of-a-minute = 60 minutes = 3600 seconds (1:00 AM).
        let data = 600i16.to_le_bytes();
        assert_eq!(get_time_seconds(&data, 0), 3600);

        // A raw value large enough that (raw/10)*60 exceeds one day's
        // worth of seconds must wrap back into a single day.
        let data = i16::MAX.to_le_bytes();
        let seconds = get_time_seconds(&data, 0);
        assert!(seconds < 86400);
    }

    #[test]
    fn get_duration_millis_converts_tenths_of_minute() {
        // 10 tenths-of-a-minute = 1 minute = 60_000 ms.
        let data = 10i16.to_le_bytes();
        assert_eq!(get_duration_millis(&data, 0), 60_000);
    }

    #[test]
    fn get_timestamp_valid_value() {
        let mut data = [0u8; 4];
        data[0..2].copy_from_slice(&0i16.to_le_bytes()); // time: midnight
        data[2..4].copy_from_slice(&100i16.to_le_bytes()); // days: >= 100
        let ts = get_timestamp(&data, 0).expect("valid timestamp");
        assert_eq!(ts.date, EPOCH_DATE.plus_days(100));
    }

    #[test]
    fn get_timestamp_na_sentinels_are_none() {
        // days <= 1
        let mut data = [0u8; 4];
        data[2..4].copy_from_slice(&1i16.to_le_bytes());
        assert_eq!(get_timestamp(&data, 0), None);

        // days == 65535
        let mut data = [0u8; 4];
        data[2..4].copy_from_slice(&(-1i16).to_le_bytes());
        assert_eq!(get_timestamp(&data, 0), None);
    }

    #[test]
    fn get_timestamp_time_sentinel_defaults_to_midnight() {
        let mut data = [0u8; 4];
        data[0..2].copy_from_slice(&(-1i16).to_le_bytes()); // time: 65535 sentinel
        data[2..4].copy_from_slice(&200i16.to_le_bytes());
        let ts = get_timestamp(&data, 0).expect("valid timestamp");
        assert_eq!(ts.seconds_since_midnight, 0);
    }

    #[test]
    fn get_timestamp_under_100_days_with_nonzero_seconds_is_none() {
        // MPXJ's heuristic: a suspiciously small day count together with a
        // non-zero seconds component looks like "NA" rather than a real
        // date, so this is treated as absent.
        let mut data = [0u8; 4];
        data[0..2].copy_from_slice(&1i16.to_le_bytes()); // time: 6 seconds
        data[2..4].copy_from_slice(&50i16.to_le_bytes()); // days: < 100
        assert_eq!(get_timestamp(&data, 0), None);
    }

    #[test]
    fn get_timestamp_under_100_days_with_zero_seconds_is_valid() {
        let mut data = [0u8; 4];
        data[0..2].copy_from_slice(&0i16.to_le_bytes()); // time: midnight, zero seconds
        data[2..4].copy_from_slice(&50i16.to_le_bytes()); // days: < 100
        let ts = get_timestamp(&data, 0).expect("valid timestamp");
        assert_eq!(ts.date, EPOCH_DATE.plus_days(50));
    }

    #[test]
    fn get_unicode_string_handles_empty_and_unterminated_data() {
        assert_eq!(get_unicode_string(&[], 0), "");

        // No nul terminator before the end of the buffer: reads to the end.
        let data: Vec<u8> = "Hi".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        assert_eq!(get_unicode_string(&data, 0), "Hi");
    }

    #[test]
    fn get_unicode_string_max_caps_length_and_handles_empty() {
        let data: Vec<u8> = "Hello"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        assert_eq!(get_unicode_string_max(&data, 0, 4), "He");
        assert_eq!(get_unicode_string_max(&data, 0, 0), "Hello");
        assert_eq!(get_unicode_string_max(&[], 0, 4), "");
    }

    #[test]
    fn get_string_reads_single_byte_chars() {
        assert_eq!(get_string(b"Hi\0ignored", 0), "Hi");
        // No nul terminator: reads to the end of the buffer.
        assert_eq!(get_string(b"Hi", 0), "Hi");
        assert_eq!(get_string(b"", 0), "");
    }

    #[test]
    fn get_duration_time_units_covers_every_stored_code() {
        let cases = [
            (3, TimeUnit::Minutes),
            (4, TimeUnit::ElapsedMinutes),
            (5, TimeUnit::Hours),
            (6, TimeUnit::ElapsedHours),
            (7, TimeUnit::Days),
            (8, TimeUnit::ElapsedDays),
            (9, TimeUnit::Weeks),
            (10, TimeUnit::ElapsedWeeks),
            (11, TimeUnit::Months),
            (12, TimeUnit::ElapsedMonths),
            (19, TimeUnit::Percent),
            (20, TimeUnit::ElapsedPercent),
            (0, TimeUnit::Days),
            // 15 has no assigned meaning; masked by DURATION_UNITS_MASK it
            // still falls through to the default.
            (15, TimeUnit::Days),
        ];
        for (raw, expected) in cases {
            assert_eq!(get_duration_time_units(raw, TimeUnit::Weeks), expected);
        }
        // Code 21 defers to the project's own default duration units.
        assert_eq!(
            get_duration_time_units(21, TimeUnit::Weeks),
            TimeUnit::Weeks
        );
    }

    #[test]
    fn duration_from_raw_covers_every_unit() {
        let cases: [(TimeUnit, f64); 10] = [
            (TimeUnit::Minutes, 10.0),
            (TimeUnit::ElapsedMinutes, 10.0),
            (TimeUnit::Hours, 600.0),
            (TimeUnit::ElapsedHours, 600.0),
            (TimeUnit::ElapsedDays, 14400.0),
            (TimeUnit::Weeks, 24000.0),
            (TimeUnit::ElapsedWeeks, 100800.0),
            (TimeUnit::Months, 96000.0),
            (TimeUnit::ElapsedMonths, 432000.0),
            (TimeUnit::Percent, 1.0),
        ];
        for (units, raw) in cases {
            let d = duration_from_raw(raw, units);
            assert!((d.value - 1.0).abs() < 1e-9, "{units:?} gave {}", d.value);
        }
        let d = duration_from_raw(50.0, TimeUnit::ElapsedPercent);
        assert_eq!(d.value, 50.0);
    }

    #[test]
    fn to_unit_converts_days_to_hours() {
        // 1 day == 8 hours, per the same fixed 8 hour day duration_from_raw uses.
        let d = MppDuration {
            value: 1.0,
            units: TimeUnit::Days,
        };
        let converted = d.to_unit_standard(TimeUnit::Hours);
        assert_eq!(converted.units, TimeUnit::Hours);
        assert!((converted.value - 8.0).abs() < 1e-9);
    }

    #[test]
    fn to_unit_converts_weeks_to_days() {
        // 1 week == 5 working days.
        let d = MppDuration {
            value: 1.0,
            units: TimeUnit::Weeks,
        };
        let converted = d.to_unit_standard(TimeUnit::Days);
        assert!((converted.value - 5.0).abs() < 1e-9);
    }

    #[test]
    fn to_unit_same_unit_is_a_no_op() {
        let d = MppDuration {
            value: 3.5,
            units: TimeUnit::Hours,
        };
        assert_eq!(d.to_unit_standard(TimeUnit::Hours), d);
    }

    #[test]
    fn to_unit_round_trips() {
        let original = MppDuration {
            value: 2.0,
            units: TimeUnit::Days,
        };
        let round_tripped = original
            .to_unit_standard(TimeUnit::Minutes)
            .to_unit_standard(TimeUnit::Days);
        assert!((round_tripped.value - original.value).abs() < 1e-9);
    }

    #[test]
    fn to_unit_uses_the_supplied_calendar_not_a_hardcoded_one() {
        // A calendar with a 6-hour (360-minute) working day: 1 "day" of
        // duration should come out to 360/480 = 0.75 standard days' worth
        // of minutes, i.e. converting to Hours should give 6, not 8.
        let d = MppDuration {
            value: 1.0,
            units: TimeUnit::Days,
        };
        let converted = d.to_unit(TimeUnit::Hours, 360.0, 1800.0, 20.0);
        assert!((converted.value - 6.0).abs() < 1e-9);

        // And expressed in standard (480-min) days, that's 0.75 days.
        let converted_days = converted.to_unit(TimeUnit::Days, 480.0, 2400.0, 20.0);
        assert!((converted_days.value - 0.75).abs() < 1e-9);

        // Weeks use minutesPerWeek directly, not 5x minutesPerDay: a
        // calendar with a 6-day, 360-min/day week (2160 min/week) should
        // convert 1 week to exactly 2160 minutes, not 1800 (5 * 360).
        let w = MppDuration {
            value: 1.0,
            units: TimeUnit::Weeks,
        };
        let w_minutes = w.to_unit(TimeUnit::Minutes, 360.0, 2160.0, 20.0);
        assert!((w_minutes.value - 2160.0).abs() < 1e-9);

        // Elapsed units are calendar-independent: 1 elapsed day is always
        // 1440 minutes regardless of the working-day length.
        let e = MppDuration {
            value: 1.0,
            units: TimeUnit::ElapsedDays,
        };
        let e_minutes = e.to_unit(TimeUnit::ElapsedMinutes, 360.0, 1800.0, 20.0);
        assert!((e_minutes.value - 1440.0).abs() < 1e-9);
    }

    #[test]
    fn duration_as_std_converts_tenths_of_minute_to_millis() {
        assert_eq!(duration_as_std(10).as_millis(), 60_000);
        // Negative raw values still produce a non-negative std MppDuration.
        assert_eq!(duration_as_std(-10).as_millis(), 60_000);
    }
}
