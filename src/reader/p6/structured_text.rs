//! Ported from MPXJ: src/main/java/org/mpxj/primavera/StructuredTextParser.java
//! and StructuredTextRecord.java. Copyright (c) Packwood Software, Jon Iles.
//! Licensed under the GNU Lesser General Public License, version 2.1 or later.
//!
//! Parser for P6's "structured text" format, used for the `clndr_data`
//! column of the XER CALENDAR table (and a handful of other columns this
//! crate doesn't read). The format is a nest of parenthesised records:
//!
//! ```text
//! (0||CalendarData()(
//!   (0||DaysOfWeek()(
//!     (0||1()())
//!     (0||2()(
//!       (0||0(s|08:00|f|16:00)())))
//!   ))
//!   (0||Exceptions()(
//!     (0||0(d|41274)())))
//! ))
//! ```
//!
//! Each record is `(<number>||<name>(attr|value|attr|value...)(<children>))`.

use std::collections::HashMap;

/// One parsed structured-text record: a name, a flat attribute map, and
/// child records in document order.
#[derive(Debug, Clone, Default)]
pub(crate) struct StructuredTextRecord {
    pub record_number: Option<String>,
    pub record_name: Option<String>,
    pub attributes: HashMap<String, String>,
    pub children: Vec<StructuredTextRecord>,
}

impl StructuredTextRecord {
    /// First child with the given record name, if any.
    pub fn child(&self, name: &str) -> Option<&StructuredTextRecord> {
        self.children
            .iter()
            .find(|c| c.record_name.as_deref() == Some(name))
    }

    pub fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }
}

/// Parse structured text. Mirrors MPXJ's parser with
/// `setRaiseExceptionOnParseError(false)`: on malformed input it returns
/// whatever was successfully parsed up to that point rather than failing,
/// which is what both MPXJ readers use for `clndr_data`.
pub(crate) fn parse(text: &str) -> StructuredTextRecord {
    let mut parser = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    parser.parse_children_lenient()
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

/// Internal marker for "ran out of input / structure mismatch"; the public
/// entry point converts this into a best-effort partial result.
struct ParseAbort;

type ParseResult<T> = std::result::Result<T, ParseAbort>;

impl Parser {
    /// Top level: parse sibling records until input is exhausted or a
    /// structural error occurs, returning a synthetic root that holds them.
    fn parse_children_lenient(&mut self) -> StructuredTextRecord {
        let mut root = StructuredTextRecord::default();
        // MPXJ parses the top level as an anonymous record body: a sequence
        // of `(...)` records terminated by `)` or end of input.
        loop {
            match self.skip_ws_and_read() {
                Err(_) => break,
                Ok(')') => break,
                Ok('(') => match self.parse_record() {
                    Ok(child) => root.children.push(child),
                    Err(_) => break,
                },
                Ok(_) => break,
            }
        }
        root
    }

    /// Parse one record; the opening `(` has already been consumed.
    fn parse_record(&mut self) -> ParseResult<StructuredTextRecord> {
        let mut record = StructuredTextRecord::default();

        // Record number: a run of digits.
        let mut number = String::new();
        let mut c = self.read()?;
        while c.is_ascii_digit() {
            number.push(c);
            c = self.read()?;
        }
        if number.is_empty() {
            return Err(ParseAbort);
        }
        record.record_number = Some(number);

        // Separator `||` (possibly with whitespace before it).
        c = self.skip_ws_from(c)?;
        let mut bars = 0;
        while c == '|' {
            bars += 1;
            c = self.read()?;
        }
        if bars != 2 {
            return Err(ParseAbort);
        }

        // Record name up to the attribute list's `(`.
        c = self.skip_ws_from(c)?;
        let mut name = String::new();
        while c != '(' {
            name.push(c);
            c = self.read()?;
        }
        if !name.is_empty() {
            record.record_name = Some(name);
        }

        // Attributes: `name|value|name|value...` until `)`.
        c = self.read()?;
        while c != ')' {
            let mut field_name = String::new();
            while c != '|' {
                field_name.push(c);
                c = self.read()?;
            }
            let mut value = String::new();
            c = self.read()?;
            while c != '|' && c != ')' {
                value.push(c);
                c = self.read()?;
            }
            if c == '|' {
                c = self.read()?;
            }
            record.attributes.insert(field_name, value);
        }

        // Child records: `(` then zero or more records then `)`.
        if self.skip_ws_and_read()? != '(' {
            return Err(ParseAbort);
        }
        loop {
            let c = self.skip_ws_and_read()?;
            if c != '(' {
                // Not a child record start: put it back and stop.
                self.pos -= 1;
                break;
            }
            record.children.push(self.parse_record()?);
        }
        if self.skip_ws_and_read()? != ')' {
            return Err(ParseAbort);
        }

        // Closing `)` of the record itself.
        if self.skip_ws_and_read()? != ')' {
            return Err(ParseAbort);
        }

        Ok(record)
    }

    fn read(&mut self) -> ParseResult<char> {
        let c = self.chars.get(self.pos).copied().ok_or(ParseAbort)?;
        self.pos += 1;
        Ok(c)
    }

    fn skip_ws_and_read(&mut self) -> ParseResult<char> {
        let c = self.read()?;
        self.skip_ws_from(c)
    }

    fn skip_ws_from(&mut self, mut c: char) -> ParseResult<char> {
        while c.is_whitespace() || c.is_control() {
            c = self.read()?;
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed-down but structurally faithful clndr_data value.
    const SAMPLE: &str = "(0||CalendarData()(\
        (0||DaysOfWeek()(\
          (0||1()())\
          (0||2()((0||0(s|08:00|f|16:00)())(0||1(s|17:00|f|19:00)())))\
        ))\
        (0||Exceptions()(\
          (0||0(d|41274)())\
          (0||1(d|41275)((0||0(s|08:00|f|12:00)())))\
        ))\
      ))";

    #[test]
    fn parses_nested_calendar_structure() {
        let root = parse(SAMPLE);
        let cal = root.child("CalendarData").expect("CalendarData");
        let days = cal.child("DaysOfWeek").expect("DaysOfWeek");
        assert_eq!(days.children.len(), 2);

        // Day "1" (Sunday): no hour records -> non-working.
        let sunday = &days.children[0];
        assert_eq!(sunday.record_name.as_deref(), Some("1"));
        assert!(sunday.children.is_empty());

        // Day "2" (Monday): two working ranges.
        let monday = &days.children[1];
        assert_eq!(monday.children.len(), 2);
        assert_eq!(monday.children[0].attribute("s"), Some("08:00"));
        assert_eq!(monday.children[0].attribute("f"), Some("16:00"));
        assert_eq!(monday.children[1].attribute("s"), Some("17:00"));

        let exceptions = cal.child("Exceptions").expect("Exceptions");
        assert_eq!(exceptions.children.len(), 2);
        assert_eq!(exceptions.children[0].attribute("d"), Some("41274"));
        assert!(exceptions.children[0].children.is_empty());
        assert_eq!(exceptions.children[1].children.len(), 1);
    }

    #[test]
    fn empty_input_yields_empty_root() {
        let root = parse("");
        assert!(root.children.is_empty());
    }

    #[test]
    fn malformed_input_returns_partial_result() {
        // Second record is truncated mid-attributes: the first record must
        // still be returned, matching MPXJ's lenient mode.
        let root = parse("(0||First()())(0||Second(a|b");
        assert_eq!(root.children.len(), 1);
        assert_eq!(root.children[0].record_name.as_deref(), Some("First"));
    }

    #[test]
    fn whitespace_between_records_is_ignored() {
        let root = parse("(0||A()(\n  (0||B()())\r\n))");
        let a = root.child("A").expect("A");
        assert_eq!(a.children.len(), 1);
        assert_eq!(a.children[0].record_name.as_deref(), Some("B"));
    }

    #[test]
    fn attributes_with_empty_values_parse() {
        let root = parse("(0||R(a||b|2)())");
        let r = root.child("R").expect("R");
        assert_eq!(r.attribute("a"), Some(""));
        assert_eq!(r.attribute("b"), Some("2"));
    }
}
