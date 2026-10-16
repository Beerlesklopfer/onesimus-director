//! Shared pieces of the Storage daemon label/relabel exchange: the
//! acceptance of the SD reply and BashSpaces().
//!
//! Used by `purge volume action=truncate` (ua_purge.cc:604
//! do_truncate_on_purge, compatibility log C-049) and by `truncate` /
//! `label` / `relabel` (ua_label.cc native_send_label_request, C-053).
//! Both accept the two reply forms of 18be77e6:
//! `3000 OK label. VolFiles=%lu VolBytes=%llu ` and
//! `3000 OK label. VolBytes=%llu `.

/// C++ BashSpaces(): every space becomes 0x01 for the SD command line.
pub fn bash_spaces(text: &str) -> String {
    text.replace(' ', "\u{1}")
}

/// The successful label reply of the SD: `Some((Some(VolFiles), VolBytes))`
/// for `3000 OK label. VolFiles=%lu VolBytes=%llu ` (both conversions),
/// `Some((None, VolBytes))` for `3000 OK label. VolBytes=%llu `, None
/// otherwise — the two sscanf() checks in that order. VolFiles is a
/// uint32_t read with `%lu`: its low 32 bits.
pub fn parse_label_ok(msg: &str) -> Option<(Option<u32>, u64)> {
    let first = scan(msg, "3000 OK label. VolFiles=% VolBytes=% ");
    if let [files, bytes] = first[..] {
        return Some((Some(files as u32), bytes));
    }
    match scan(msg, "3000 OK label. VolBytes=% ")[..] {
        [bytes] => Some((None, bytes)),
        _ => None,
    }
}

/// A small sscanf(): whitespace in `format` skips any whitespace (also
/// none), `%` is an unsigned conversion (`%lu`/`%llu`: leading whitespace,
/// optional sign, strtoull wrapping and saturation), anything else must
/// match literally. Returns the converted values up to the first failure.
fn scan(input: &str, format: &str) -> Vec<u64> {
    let input = input.as_bytes();
    let mut pos = 0;
    let mut values = Vec::new();
    for f in format.bytes() {
        if f.is_ascii_whitespace() {
            while pos < input.len() && input[pos].is_ascii_whitespace() { pos += 1; }
        } else if f == b'%' {
            while pos < input.len() && input[pos].is_ascii_whitespace() { pos += 1; }
            let negative = match input.get(pos) {
                Some(b'-') => { pos += 1; true }
                Some(b'+') => { pos += 1; false }
                _ => false,
            };
            let start = pos;
            let mut value: u64 = 0;
            let mut overflow = false;
            while pos < input.len() && input[pos].is_ascii_digit() {
                match value.checked_mul(10).and_then(|v| v.checked_add((input[pos] - b'0') as u64)) {
                    Some(v) => value = v,
                    None => overflow = true,
                }
                pos += 1;
            }
            if pos == start { return values; }
            if overflow {
                value = u64::MAX;
            } else if negative {
                value = value.wrapping_neg();
            }
            values.push(value);
        } else {
            if input.get(pos) != Some(&f) { return values; }
            pos += 1;
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_reply_forms_are_accepted() {
        assert_eq!(parse_label_ok("3000 OK label. VolFiles=0 VolBytes=225 Volume=\"v1\" Device=\"FileStorage\" (/var/lib/bareos/storage)\n"),
            Some((Some(0), 225)));
        assert_eq!(parse_label_ok("3000 OK label. VolBytes=64512 Volume=\"v1\" Device=\"d\"\n"), Some((None, 64512)));
        // sscanf: format whitespace matches none, conversions skip it.
        assert_eq!(parse_label_ok("3000OK label.VolBytes=  7"), Some((None, 7)));
        assert_eq!(parse_label_ok("3000 OK label. VolFiles=4294967297 VolBytes=1"), Some((Some(1), 1)));
        assert_eq!(parse_label_ok("3000 OK label. VolBytes=-1"), Some((None, u64::MAX)));
        assert_eq!(parse_label_ok("3000 OK label. VolBytes=99999999999999999999999"), Some((None, u64::MAX)));
    }

    #[test]
    fn other_replies_are_rejected() {
        assert_eq!(parse_label_ok("3910 Unable to open device \"d\"\n"), None);
        assert_eq!(parse_label_ok("3000 OK label. VolFiles=3 VolBytes=x"), None);
        assert_eq!(parse_label_ok("3000 OK label. VolBytes="), None);
        assert_eq!(parse_label_ok(""), None);
    }

    #[test]
    fn spaces_are_bashed() {
        assert_eq!(bash_spaces("a b  c"), "a\u{1}b\u{1}\u{1}c");
    }
}
