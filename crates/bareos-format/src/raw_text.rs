// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Bytes that need not be UTF-8 (file names in a `SQL_ASCII` catalog, console
//! lines) carried losslessly in a Rust `String`.
//!
//! Bareos keeps file names as the bytes the file system gave it: the catalog
//! is `SQL_ASCII` (cats/postgresql.cc:203, connection with `client_encoding`
//! `SQL_ASCII`, :281) and the console gets the bytes as they are. Onesimus
//! passes text as `String`; [`decode`] keeps every valid UTF-8 sequence and
//! maps each byte of an invalid one to one character of the last 256 code
//! points of plane 16 (private use, U+10FF00 + byte). [`encode`] reverses it
//! at the edges (catalog, BNET, terminal), so a name round-trips byte for
//! byte. A name that really contains one of those 256 characters would be
//! written as the single byte; they are reserved for private use and do not
//! occur in file names in practice.

use std::borrow::Cow;

/// First code point of the byte block (U+10FF00 stands for byte 0x00).
const BASE: u32 = 0x10_FF00;

/// The character that stands for the invalid byte `b`.
fn byte_char(b: u8) -> char {
    char::from_u32(BASE + u32::from(b)).expect("U+10FF00..U+10FFFF are valid scalar values")
}

/// The byte a character of the block stands for, or None for any other char.
pub fn char_byte(c: char) -> Option<u8> {
    (c as u32).checked_sub(BASE).and_then(|b| u8::try_from(b).ok())
}

/// Bytes as text: valid UTF-8 unchanged, each byte of an invalid sequence as
/// its block character.
pub fn decode(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(text);
    }
    let mut text = String::with_capacity(bytes.len() + 8);
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        text.extend(chunk.invalid().iter().map(|&b| byte_char(b)));
    }
    Cow::Owned(text)
}

/// Owned form of [`decode`].
pub fn decode_owned(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => decode(e.as_bytes()).into_owned(),
    }
}

/// True when `text` carries at least one raw byte.
pub fn has_raw(text: &str) -> bool {
    text.chars().any(|c| char_byte(c).is_some())
}

/// Text back to the bytes it came from.
pub fn encode(text: &str) -> Cow<'_, [u8]> {
    if !has_raw(text) {
        return Cow::Borrowed(text.as_bytes());
    }
    let mut bytes = Vec::with_capacity(text.len());
    let mut buf = [0u8; 4];
    for c in text.chars() {
        match char_byte(c) {
            Some(b) => bytes.push(b),
            None => bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes()),
        }
    }
    Cow::Owned(bytes)
}

/// SQL string literal of `text` with its raw bytes: `'…'` (quotes doubled)
/// when there are none, else an escape string `E'…'` with `\xNN` for each
/// raw byte. The server must accept those bytes, i.e. a `SQL_ASCII`
/// database.
pub fn sql_literal(text: &str) -> String {
    if !has_raw(text) {
        return format!("'{}'", text.replace('\'', "''"));
    }
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str("E'");
    for c in text.chars() {
        match (char_byte(c), c) {
            (Some(b), _) => out.push_str(&format!("\\x{b:02x}")),
            (None, '\'') => out.push_str("''"),
            (None, '\\') => out.push_str("\\\\"),
            (None, c) => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Content for a `'…'` SQL literal (PQescapeStringConn with
/// standard_conforming_strings on): quotes doubled. A run of raw bytes is
/// spliced in as `'||E'\xNN…'||'`, so `'{}'` stays one valid expression
/// (`||` binds tighter than `=`, `LIKE` and `IN` in PostgreSQL) whose value
/// has the bytes. Needs a `SQL_ASCII` database, like [`sql_literal`].
pub fn sql_escape(text: &str) -> Cow<'_, str> {
    if !has_raw(text) {
        return if text.contains('\'') { Cow::Owned(text.replace('\'', "''")) } else { Cow::Borrowed(text) };
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut in_raw = false;
    for c in text.chars() {
        match char_byte(c) {
            Some(b) => {
                if !in_raw {
                    out.push_str("'||E'");
                    in_raw = true;
                }
                out.push_str(&format!("\\x{b:02x}"));
            }
            None => {
                if in_raw {
                    out.push_str("'||'");
                    in_raw = false;
                }
                if c == '\'' {
                    out.push_str("''");
                } else {
                    out.push(c);
                }
            }
        }
    }
    if in_raw {
        out.push_str("'||'");
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_utf8_is_unchanged() {
        assert!(matches!(decode("café/ü.txt".as_bytes()), Cow::Borrowed("café/ü.txt")));
        assert!(matches!(encode("café"), Cow::Borrowed(b) if b == "café".as_bytes()));
        assert_eq!(sql_literal("it's"), "'it''s'");
    }

    #[test]
    fn invalid_bytes_round_trip() {
        for bytes in [&b"caf\xe9.txt"[..], b"\xff\xfe", b"a\xc3", b"\xe9\xc3\xa9\x80", b"\x80\\'x"] {
            let text = decode(bytes);
            assert!(has_raw(&text), "{bytes:?}");
            assert_eq!(&*encode(&text), bytes);
            assert_eq!(decode_owned(bytes.to_vec()), text);
        }
        assert_eq!(decode(b"\xe9\xc3\xa9"), "\u{10ffe9}é");
    }

    #[test]
    fn literal_escapes_raw_bytes() {
        assert_eq!(sql_literal(&decode(b"caf\xe9.txt")), r"E'caf\xe9.txt'");
        assert_eq!(sql_literal(&decode(b"a\\b'\xe9")), r"E'a\\b''\xe9'");
        assert_eq!(char_byte('a'), None);
        assert_eq!(sql_escape("it's"), "it''s");
        assert_eq!(format!("'{}'", sql_escape(&decode(b"caf\xe9\xff'.txt"))), r"'caf'||E'\xe9\xff'||'''.txt'");
        assert_eq!(format!("'{}'", sql_escape(&decode(b"\xe9"))), r"''||E'\xe9'||''");
        assert_eq!(char_byte('\u{10ff00}'), Some(0));
    }
}
