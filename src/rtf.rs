//! Simplified equivalent of MPXJ's `org.mpxj.common.RtfHelper.strip`.
//!
//! MS Project stores task and resource notes as RTF. MPXJ strips this with
//! a full RTF parser (the `rtfparserkit` library). Pulling in a full RTF
//! parser here would mean a dependency beyond `cfb`/`thiserror`/`serde`, so
//! this module implements a small hand-written stripper instead: it walks
//! RTF control words and groups well enough to recover the plain text of
//! the simple, single-author notes Microsoft Project actually writes, but
//! it does not handle every RTF destination (embedded objects, custom font
//! tables with unusual names, and so on) the way a full parser would.

/// Strip RTF control words and groups, returning plain text. If `text`
/// does not start with an RTF header (`{\rtf`), it is assumed to already be
/// plain text and is returned unchanged, matching MPXJ's `isFormalRTF`
/// heuristic.
pub(crate) fn strip(text: &str) -> String {
    if !text.starts_with("{\\rtf") {
        return text.to_string();
    }

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut depth = 0i32;
    // Depth at which a skipped destination group (fonttbl, colortbl, ...)
    // started; content at or below this depth is dropped until the group
    // closes.
    let mut skip_until_depth: Option<i32> = None;

    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if let Some(d) = skip_until_depth {
                    if depth <= d {
                        skip_until_depth = None;
                    }
                }
                i += 1;
            }
            b'\\' => {
                let (control, len) = read_control(&bytes[i..]);
                i += len;
                if skip_until_depth.is_some() {
                    continue;
                }
                match control {
                    Control::Word(name, param) => match name {
                        "par" | "line" => out.push('\n'),
                        "tab" => out.push('\t'),
                        "u" => {
                            if let Some(code) = param.and_then(|p| char::from_u32(p as u32)) {
                                out.push(code);
                            }
                            // \uN is followed by one fallback character for
                            // readers that can't render the Unicode code
                            // point; skip it so it doesn't get duplicated.
                            if i < bytes.len()
                                && bytes[i] != b'\\'
                                && bytes[i] != b'{'
                                && bytes[i] != b'}'
                            {
                                i += 1;
                            }
                        }
                        "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "object"
                        | "themedata" | "datastore" | "generator" => {
                            skip_until_depth = Some(depth - 1);
                        }
                        _ => {}
                    },
                    Control::Symbol('\'') => {
                        if i + 2 <= bytes.len() {
                            if let Ok(byte) = u8::from_str_radix(&text[i..i + 2], 16) {
                                out.push(cp1252_to_char(byte));
                            }
                            i += 2;
                        }
                    }
                    Control::Symbol(c @ ('\\' | '{' | '}')) => out.push(c),
                    Control::Symbol(_) => {}
                }
            }
            // Bare CR/LF bytes in the source are formatting for the RTF
            // markup itself, not real line breaks (those come from \par
            // and \line); skip them like a real RTF reader would.
            b'\r' | b'\n' => {
                i += 1;
            }
            c => {
                if skip_until_depth.is_none() {
                    out.push(c as char);
                }
                i += 1;
            }
        }
    }

    out.trim_matches('\n').to_string()
}

enum Control<'a> {
    Word(&'a str, Option<i32>),
    Symbol(char),
}

/// Decodes a `\'XX` RTF hex-escape byte under Windows-1252 — the default
/// codepage real Microsoft Project/Word output uses — rather than treating
/// it as Latin-1. The two encodings agree everywhere except 0x80-0x9F,
/// where cp1252 assigns printable characters (curly quotes, en/em dash,
/// ellipsis, trademark sign, etc.) to code points Latin-1 reserves for C1
/// control characters; five of those 32 slots are themselves undefined in
/// cp1252 and fall back to the C1 control point, matching the WHATWG
/// windows-1252 decoder table.
fn cp1252_to_char(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}', // €
        0x82 => '\u{201A}', // ‚
        0x83 => '\u{0192}', // ƒ
        0x84 => '\u{201E}', // „
        0x85 => '\u{2026}', // …
        0x86 => '\u{2020}', // †
        0x87 => '\u{2021}', // ‡
        0x88 => '\u{02C6}', // ˆ
        0x89 => '\u{2030}', // ‰
        0x8A => '\u{0160}', // Š
        0x8B => '\u{2039}', // ‹
        0x8C => '\u{0152}', // Œ
        0x8E => '\u{017D}', // Ž
        0x91 => '\u{2018}', // '
        0x92 => '\u{2019}', // '
        0x93 => '\u{201C}', // "
        0x94 => '\u{201D}', // "
        0x95 => '\u{2022}', // •
        0x96 => '\u{2013}', // –
        0x97 => '\u{2014}', // —
        0x98 => '\u{02DC}', // ˜
        0x99 => '\u{2122}', // ™
        0x9A => '\u{0161}', // š
        0x9B => '\u{203A}', // ›
        0x9C => '\u{0153}', // œ
        0x9E => '\u{017E}', // ž
        0x9F => '\u{0178}', // Ÿ
        // 0x81, 0x8D, 0x8F, 0x90, 0x9D are undefined in cp1252; every other
        // byte agrees with Latin-1.
        other => other as char,
    }
}

/// Parse one control word or control symbol starting at `data[0] == b'\\'`.
/// Returns the control and the number of bytes consumed, including the
/// leading backslash and any trailing space that terminates a word.
fn read_control(data: &[u8]) -> (Control<'_>, usize) {
    debug_assert_eq!(data[0], b'\\');
    if data.len() < 2 {
        return (Control::Symbol(' '), 1);
    }

    let next = data[1];
    if !next.is_ascii_alphabetic() {
        return (Control::Symbol(next as char), 2);
    }

    let mut end = 1;
    while end < data.len() && data[end].is_ascii_alphabetic() {
        end += 1;
    }
    let name = std::str::from_utf8(&data[1..end]).unwrap_or("");

    let param_start = end;
    let mut param_end = end;
    if param_end < data.len() && (data[param_end] == b'-' || data[param_end].is_ascii_digit()) {
        param_end += 1;
        while param_end < data.len() && data[param_end].is_ascii_digit() {
            param_end += 1;
        }
    }
    let param = std::str::from_utf8(&data[param_start..param_end])
        .ok()
        .and_then(|s| s.parse().ok());

    let mut consumed = param_end;
    if consumed < data.len() && data[consumed] == b' ' {
        consumed += 1;
    }

    (Control::Word(name, param), consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_returned_unchanged() {
        assert_eq!(strip("just some notes"), "just some notes");
    }

    #[test]
    fn strips_basic_formatting() {
        let rtf = r"{\rtf1\ansi\deff0{\fonttbl{\f0 Arial;}}\f0\pard Hello \b world\b0!\par}";
        assert_eq!(strip(rtf), "Hello world!");
    }

    #[test]
    fn converts_par_to_newline() {
        let rtf = r"{\rtf1 line one\par line two}";
        assert_eq!(strip(rtf), "line one\nline two");
    }

    #[test]
    fn ignores_bare_source_line_breaks() {
        // MS Project wraps the raw RTF source itself across lines; those
        // line breaks are not real paragraph breaks and must not appear
        // in the stripped text.
        let rtf = "{\\rtf1\r\nNotes Example\n\r\n\r}";
        assert_eq!(strip(rtf), "Notes Example");
    }

    #[test]
    fn decodes_hex_escape() {
        let rtf = r"{\rtf1 caf\'e9}";
        assert_eq!(strip(rtf), "caf\u{e9}");
    }

    #[test]
    fn decodes_hex_escape_as_windows_1252_not_latin1() {
        // 0x93/0x94 are curly double-quotes under cp1252 (real MS Project
        // output); under Latin-1 they'd be the C1 control points U+0093/94.
        let rtf = r"{\rtf1 \'93quoted\'94}";
        assert_eq!(strip(rtf), "\u{201C}quoted\u{201D}");
    }

    #[test]
    fn hex_escape_above_0x9f_matches_latin1() {
        // cp1252 and Latin-1 agree everywhere outside 0x80-0x9F.
        let rtf = r"{\rtf1 caf\'e9 na\'efve}";
        assert_eq!(strip(rtf), "caf\u{e9} na\u{ef}ve");
    }

    #[test]
    fn never_panics_on_malformed_rtf() {
        let _ = strip("{\\rtf1 \\");
        let _ = strip("{\\rtf1 {{{{");
        let _ = strip("{\\rtf1 \\'zz}");
    }

    #[test]
    fn unicode_control_word_inserts_char_and_skips_fallback_char() {
        // \u233 is U+00E9 (e acute); the '?' immediately after it is the
        // ANSI fallback for readers that can't render Unicode and must be
        // skipped, not duplicated into the output.
        let rtf = "{\\rtf1 caf\\u233?}";
        assert_eq!(strip(rtf), "caf\u{e9}");
    }

    #[test]
    fn unicode_control_word_fallback_is_not_skipped_before_a_control() {
        // If the fallback character position is itself a backslash or
        // brace, nothing should be consumed as a fallback.
        let rtf = "{\\rtf1 \\u233\\par}";
        assert_eq!(strip(rtf), "\u{e9}");
    }

    #[test]
    fn escaped_brace_and_backslash_are_literal() {
        let rtf = r"{\rtf1 a\{b\}c\\d}";
        assert_eq!(strip(rtf), r"a{b}c\d");
    }

    #[test]
    fn unrecognised_control_symbol_is_dropped() {
        // \~ (non-breaking space) is a control symbol zaf-mpp doesn't
        // special-case; it should simply be dropped, not appear literally.
        let rtf = "{\\rtf1 a\\~b}";
        assert_eq!(strip(rtf), "ab");
    }
}
