//! Variable expansion: port of the OSSP var library (C++ lib/var.cc) and of
//! the Director's lookup callbacks (C++ dird/expand.cc VariableExpansion).
//!
//! The engine works on bytes exactly like the C original, including its
//! quirks (prefix matching of built-in names, the `+` counter suffix, error
//! texts indexed by the negated return code). It is used by the `var`
//! console command and is meant for Pool LabelFormat as well.
//!
//! Return codes follow the C convention: a value >= 0 is the number of input
//! bytes consumed, a negative value is a `VAR_ERR_*` code.

use std::collections::HashMap;
use std::sync::Mutex;

// lib/var.h var_rc_t
const ERR_INCOMPLETE_NAMED_CHARACTER: i32 = -1;
const ERR_INCOMPLETE_HEX: i32 = -2;
const ERR_INVALID_HEX: i32 = -3;
const ERR_OCTAL_TOO_LARGE: i32 = -4;
const ERR_INVALID_OCTAL: i32 = -5;
const ERR_INCOMPLETE_OCTAL: i32 = -6;
const ERR_INCOMPLETE_GROUPED_HEX: i32 = -7;
const ERR_OUT_OF_MEMORY: i32 = -10;
const ERR_INCOMPLETE_VARIABLE_SPEC: i32 = -11;
pub const ERR_UNDEFINED_VARIABLE: i32 = -12;
const ERR_INPUT_ISNT_TEXT_NOR_VARIABLE: i32 = -13;
const ERR_UNKNOWN_COMMAND_CHAR: i32 = -14;
const ERR_MALFORMATTED_REPLACE: i32 = -15;
const ERR_UNKNOWN_REPLACE_FLAG: i32 = -16;
const ERR_INVALID_REGEX_IN_REPLACE: i32 = -17;
const ERR_MISSING_PARAMETER_IN_COMMAND: i32 = -18;
const ERR_EMPTY_SEARCH_STRING: i32 = -19;
const ERR_MISSING_START_OFFSET: i32 = -20;
const ERR_INVALID_OFFSET_DELIMITER: i32 = -21;
const ERR_RANGE_OUT_OF_BOUNDS: i32 = -22;
const ERR_OFFSET_OUT_OF_BOUNDS: i32 = -23;
const ERR_OFFSET_LOGIC: i32 = -24;
const ERR_MALFORMATTED_TRANSPOSE: i32 = -25;
const ERR_TRANSPOSE_CLASSES_MISMATCH: i32 = -26;
const ERR_EMPTY_TRANSPOSE_CLASS: i32 = -27;
const ERR_INCORRECT_TRANSPOSE_CLASS_SPEC: i32 = -28;
const ERR_MALFORMATTED_PADDING: i32 = -29;
const ERR_MISSING_PADDING_WIDTH: i32 = -30;
const ERR_EMPTY_PADDING_FILL_STRING: i32 = -31;
const ERR_UNKNOWN_QUOTED_PAIR_IN_REPLACE: i32 = -32;
const ERR_SUBMATCH_OUT_OF_RANGE: i32 = -33;
const ERR_INCOMPLETE_QUOTED_PAIR: i32 = -34;
const ERR_INVALID_ARGUMENT: i32 = -34;
const ERR_INVALID_CHAR_IN_INDEX_SPEC: i32 = -36;
const ERR_INCOMPLETE_INDEX_SPEC: i32 = -37;
const ERR_UNCLOSED_BRACKET_IN_INDEX: i32 = -39;
const ERR_DIVISION_BY_ZERO_IN_INDEX: i32 = -40;
const ERR_UNTERMINATED_LOOP_CONSTRUCT: i32 = -41;
const ERR_INVALID_CHAR_IN_LOOP_LIMITS: i32 = -42;
const ERR_MALFORMED_OPERATION_ARGUMENTS: i32 = -43;
pub const ERR_UNDEFINED_OPERATION: i32 = -44;

/// lib/var.cc var_errors[], indexed by the negated code (the table is not
/// aligned with every code, e.g. -36 reads "lookup function does not
/// support variable arrays" — kept as in the original).
const VAR_ERRORS: &[&str] = &[
    "everything ok",
    "incomplete named character",
    "incomplete hexadecimal value",
    "invalid hexadecimal value",
    "octal value too large",
    "invalid octal value",
    "incomplete octal value",
    "incomplete grouped hexadecimal value",
    "incorrect character class specification",
    "invalid expansion configuration",
    "out of memory",
    "incomplete variable specification",
    "undefined variable",
    "input is neither text nor variable",
    "unknown command character in variable",
    "malformatted search and replace operation",
    "unknown flag in search and replace operation",
    "invalid regex in search and replace operation",
    "missing parameter in command",
    "empty search string in search and replace operation",
    "start offset missing in cut operation",
    "offsets in cut operation delimited by unknown character",
    "range out of bounds in cut operation",
    "offset out of bounds in cut operation",
    "logic error in cut operation",
    "malformatted transpose operation",
    "source and target class mismatch in transpose operation",
    "empty character class in transpose operation",
    "incorrect character class in transpose operation",
    "malformatted padding operation",
    "width parameter missing in padding operation",
    "fill string missing in padding operation",
    "unknown quoted pair in search and replace operation",
    "sub-matching reference out of range",
    "invalid argument",
    "incomplete quoted pair",
    "lookup function does not support variable arrays",
    "index of array variable contains an invalid character",
    "index of array variable is incomplete",
    "bracket expression in array variable's index not closed",
    "division by zero error in index specification",
    "unterminated loop construct",
    "invalid character in loop limits",
    "malformed operation argument list",
    "undefined operation",
    "formatting failure",
];

/// lib/var.cc var_strerror()
pub fn strerror(rc: i32) -> &'static str {
    usize::try_from(-(rc as i64)).ok().and_then(|i| VAR_ERRORS.get(i)).copied().unwrap_or("unknown error")
}

// var_syntax_default
const ESCAPE: u8 = b'\\';
const DELIM_INIT: u8 = b'$';
const DELIM_OPEN: u8 = b'{';
const DELIM_CLOSE: u8 = b'}';
const INDEX_OPEN: u8 = b'[';
const INDEX_CLOSE: u8 = b']';
const INDEX_MARK: u8 = b'#';

/// Upper bound for an expansion result. The C library grows its buffer
/// until malloc fails (e.g. a loop with step 0); here that ends as
/// "out of memory" instead of exhausting the Director.
const OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// `*p` of the C code: the byte behind the end reads as the terminating NUL.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// Lookup and operation callbacks (var_cb_value_t / var_cb_operation_t).
pub trait Resolver {
    /// Value of variable `name`; `inc` is the `${name+}` form, `index` the
    /// array index (0 without one, -1 asks for the length).
    fn lookup(&mut self, name: &[u8], inc: bool, index: i32) -> Result<Vec<u8>, i32>;
    /// `${name:%op(arg)}` on the defined value `value`.
    fn operate(&mut self, op: &[u8], arg: Option<&[u8]>, value: &[u8]) -> Result<Vec<u8>, i32>;
}

/// tokenbuf_t: None is the undefined buffer (begin == end == NULL).
type Tok = Option<Vec<u8>>;

fn append(buf: &mut Tok, data: &[u8]) -> bool {
    let out = buf.get_or_insert_with(Vec::new);
    if out.len() + data.len() > OUTPUT_LIMIT {
        return false;
    }
    out.extend_from_slice(data);
    true
}

fn tok_len(buf: &Tok) -> usize {
    buf.as_ref().map_or(0, Vec::len)
}

/// var_parse_t
#[derive(Clone, Default)]
struct Ctx {
    force_expand: bool,
    rel_lookup_flag: bool,
    rel_lookup_cnt: i32,
    index_this: i32,
}

struct Var<'a> {
    resolver: &'a mut dyn Resolver,
}

/// Length of text up to (not including) a stop character; escapes skip the
/// next byte (parse_pattern/parse_substext/parse_exptext/parse_opargtext).
fn scan_text(s: &[u8], begin: usize, stop: impl Fn(u8) -> bool) -> i32 {
    let mut p = begin;
    while p < s.len() && !stop(s[p]) {
        if s[p] == ESCAPE {
            if p + 1 == s.len() {
                return ERR_INCOMPLETE_QUOTED_PAIR;
            }
            p += 1;
        }
        p += 1;
    }
    (p - begin) as i32
}

fn parse_pattern(s: &[u8], b: usize) -> i32 {
    scan_text(s, b, |c| c == b'/')
}

fn parse_substext(s: &[u8], b: usize) -> i32 {
    scan_text(s, b, |c| c == DELIM_INIT || c == b'/')
}

fn parse_exptext(s: &[u8], b: usize) -> i32 {
    scan_text(s, b, |c| c == DELIM_INIT || c == DELIM_CLOSE || c == b':')
}

fn parse_opargtext(s: &[u8], b: usize) -> i32 {
    scan_text(s, b, |c| c == DELIM_INIT || c == b')')
}

/// parse_integer(): digits only, C int arithmetic.
fn parse_integer(s: &[u8], b: usize) -> (usize, i32) {
    let mut p = b;
    let mut num: i32 = 0;
    while p < s.len() && s[p].is_ascii_digit() {
        num = num.wrapping_mul(10).wrapping_add((s[p] - b'0') as i32);
        p += 1;
    }
    (p - b, num)
}

fn parse_name(s: &[u8], b: usize) -> usize {
    s[b.min(s.len())..].iter().take_while(|c| is_name_char(**c)).count()
}

/// parse_class_description()
fn parse_class_description(src: &[u8]) -> Result<Tok, i32> {
    let mut dst: Tok = None;
    let mut p = 0;
    while p < src.len() {
        if src.len() - p >= 3 && src[p + 1] == b'-' {
            // `*p > p[2]` compares (signed) chars
            if (src[p] as i8) > (src[p + 2] as i8) {
                return Err(ERR_INCORRECT_TRANSPOSE_CLASS_SPEC);
            }
            for c in src[p]..=src[p + 2] {
                if !append(&mut dst, &[c]) {
                    return Err(ERR_OUT_OF_MEMORY);
                }
            }
            p += 3;
        } else {
            if !append(&mut dst, &src[p..p + 1]) {
                return Err(ERR_OUT_OF_MEMORY);
            }
            p += 1;
        }
    }
    Ok(dst)
}

/// parse_regex_replace(): `\N` refers to submatch N of `data`.
fn parse_regex_replace(data: &[u8], orig: &[u8], pmatch: &[libc::regmatch_t]) -> Result<Vec<u8>, i32> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < orig.len() {
        if orig[p] == b'\\' {
            if orig.len() - p <= 1 {
                return Err(ERR_INCOMPLETE_QUOTED_PAIR);
            }
            p += 1;
            if orig[p] == b'\\' {
                out.push(b'\\');
                p += 1;
                continue;
            }
            if !orig[p].is_ascii_digit() {
                return Err(ERR_UNKNOWN_QUOTED_PAIR_IN_REPLACE);
            }
            let m = &pmatch[(orig[p] - b'0') as usize];
            p += 1;
            if m.rm_so == -1 || m.rm_eo == -1 {
                return Err(ERR_SUBMATCH_OUT_OF_RANGE);
            }
            out.extend_from_slice(&data[m.rm_so as usize..m.rm_eo as usize]);
        } else {
            out.push(orig[p]);
            p += 1;
        }
    }
    Ok(out)
}

/// op_transpose(): `${x:y/a-z/A-Z/}`
fn op_transpose(data: &mut Vec<u8>, search: &[u8], replace: &[u8]) -> i32 {
    let src = match parse_class_description(search) { Ok(c) => c.unwrap_or_default(), Err(e) => return e };
    let dst = match parse_class_description(replace) { Ok(c) => c.unwrap_or_default(), Err(e) => return e };
    if src.is_empty() {
        return ERR_EMPTY_TRANSPOSE_CLASS;
    }
    if src.len() != dst.len() {
        return ERR_TRANSPOSE_CLASSES_MISMATCH;
    }
    for c in data.iter_mut() {
        if let Some(i) = src.iter().position(|s| s == c) {
            *c = dst[i];
        }
    }
    0
}

/// op_search_and_replace(): `${x:s/pattern/replace/flags}`
fn op_search_and_replace(data: &mut Tok, search: &[u8], replace: &Tok, flags: &[u8]) -> i32 {
    if search.is_empty() {
        return ERR_EMPTY_SEARCH_STRING;
    }
    let (mut multiline, mut case_insensitive, mut global, mut no_regex) = (false, false, false, false);
    for f in flags {
        match f.to_ascii_lowercase() {
            b'm' => multiline = true,
            b'i' => case_insensitive = true,
            b'g' => global = true,
            b't' => no_regex = true,
            _ => return ERR_UNKNOWN_REPLACE_FLAG,
        }
    }
    let input = data.clone().unwrap_or_default();
    let replace_bytes = replace.as_deref().unwrap_or(&[]);

    if no_regex {
        let mut tmp: Tok = None;
        let mut p = 0;
        while p < input.len() {
            let rest = &input[p..];
            let matched = rest.len() >= search.len() && if case_insensitive {
                rest[..search.len()].eq_ignore_ascii_case(search)
            } else {
                &rest[..search.len()] == search
            };
            if !matched {
                if !append(&mut tmp, &input[p..p + 1]) { return ERR_OUT_OF_MEMORY; }
                p += 1;
            } else {
                if !append(&mut tmp, replace_bytes) { return ERR_OUT_OF_MEMORY; }
                p += search.len();
                if !global {
                    if !append(&mut tmp, &input[p..]) { return ERR_OUT_OF_MEMORY; }
                    break;
                }
            }
        }
        *data = tmp;
        return 0;
    }

    // Regular expression (POSIX regcomp/regexec like the C original);
    // pattern and data are NUL-terminated copies.
    let pattern: Vec<u8> = search.split(|c| *c == 0).next().unwrap_or(&[]).to_vec();
    let Ok(cpattern) = std::ffi::CString::new(pattern) else { return ERR_INVALID_REGEX_IN_REPLACE };
    let mut mydata = input.clone();
    mydata.push(0);
    let end = input.len();
    let mut cflags = libc::REG_EXTENDED;
    if multiline { cflags |= libc::REG_NEWLINE; }
    if case_insensitive { cflags |= libc::REG_ICASE; }
    let mut preg: libc::regex_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::regcomp(&mut preg, cpattern.as_ptr(), cflags) } != 0 {
        return ERR_INVALID_REGEX_IN_REPLACE;
    }
    let mut pmatch = [libc::regmatch_t { rm_so: -1, rm_eo: -1 }; 10];
    let mut tmp: Tok = Some(Vec::new());
    let mut p = 0;
    let mut rc = 0;
    while p < end {
        let eflags = if p == 0 || mydata[p - 1] == b'\n' { 0 } else { libc::REG_NOTBOL };
        let r = unsafe {
            libc::regexec(&preg, mydata[p..].as_ptr() as *const libc::c_char, pmatch.len(), pmatch.as_mut_ptr(), eflags)
        };
        if r != 0 {
            append(&mut tmp, &input[p..]);
            break;
        }
        let (so, eo) = (pmatch[0].rm_so as usize, pmatch[0].rm_eo as usize);
        if multiline && p + so == end && eo == so {
            append(&mut tmp, &input[p..]);
            break;
        }
        if !append(&mut tmp, &input[p..p + so]) { rc = ERR_OUT_OF_MEMORY; break; }
        match parse_regex_replace(&mydata[p..], replace_bytes, &pmatch) {
            Ok(r) => if !append(&mut tmp, &r) { rc = ERR_OUT_OF_MEMORY; break; },
            Err(e) => { rc = e; break; }
        }
        p += eo;
        if eo == so {
            if p >= end { break; }
            if !append(&mut tmp, &input[p..p + 1]) { rc = ERR_OUT_OF_MEMORY; break; }
            p += 1;
        }
        if !global {
            if !append(&mut tmp, &input[p.min(end)..]) { rc = ERR_OUT_OF_MEMORY; }
            break;
        }
    }
    unsafe { libc::regfree(&mut preg) };
    if rc < 0 {
        return rc;
    }
    *data = tmp;
    0
}

/// op_offset(): `${x:oN,M}` (from N to M) and `${x:oN-L}` (L bytes from N).
fn op_offset(data: &mut Vec<u8>, num1: i32, num2: i32, isrange: bool) -> i32 {
    let len = data.len() as i64;
    let (n1, n2) = (num1 as i64, num2 as i64);
    if len < n1 {
        return ERR_OFFSET_OUT_OF_BOUNDS;
    }
    let p = n1;
    let take = if num2 == 0 {
        len - p
    } else if isrange {
        if p + n2 > len { return ERR_RANGE_OUT_OF_BOUNDS; }
        n2
    } else {
        if n2 < n1 { return ERR_OFFSET_LOGIC; }
        if n2 > len { return ERR_RANGE_OUT_OF_BOUNDS; }
        n2 - n1 + 1
    };
    // M == length copies the terminating NUL as the C code does.
    let mut res: Vec<u8> = (p..p + take).map(|i| at(data, i as usize)).collect();
    std::mem::swap(data, &mut res);
    0
}

fn repeat_fill(out: &mut Vec<u8>, fill: &[u8], times: i64) {
    for _ in 0..times.max(0) {
        out.extend_from_slice(fill);
    }
}

fn fill_prefix(fill: &[u8], n: i64) -> &[u8] {
    &fill[..(n.max(0) as usize).min(fill.len())]
}

/// op_padding(): `${x:p/width/fill/l|r|c}` (`l` appends, `r` prepends).
fn op_padding(data: &mut Vec<u8>, width: i32, fill: &[u8], position: u8) -> i32 {
    if fill.is_empty() {
        return ERR_EMPTY_PADDING_FILL_STRING;
    }
    if width as usize > OUTPUT_LIMIT {
        return ERR_OUT_OF_MEMORY;
    }
    let width = width as i64;
    let flen = fill.len() as i64;
    let len = data.len() as i64;
    match position {
        b'l' => {
            let i = width - len;
            if i > 0 {
                repeat_fill(data, fill, i / flen);
                let rest = (width - data.len() as i64) % flen;
                data.extend_from_slice(fill_prefix(fill, rest));
            }
        }
        b'r' => {
            let i = width - len;
            if i > 0 {
                let mut result = Vec::new();
                repeat_fill(&mut result, fill, i / flen);
                result.extend_from_slice(fill_prefix(fill, (width - len) % flen));
                result.extend_from_slice(data);
                *data = result;
            }
        }
        b'c' => {
            let i = (width - len) / 2;
            if i > 0 {
                let mut result = Vec::new();
                repeat_fill(&mut result, fill, i / flen);
                result.extend_from_slice(fill_prefix(fill, ((width - len) / 2) % flen));
                result.extend_from_slice(data);
                let i = width - result.len() as i64;
                repeat_fill(&mut result, fill, i / flen);
                let i = width - result.len() as i64;
                result.extend_from_slice(fill_prefix(fill, i));
                *data = result;
            }
        }
        _ => {}
    }
    0
}

impl Var<'_> {
    /// parse_*_or_variable(): text (by `text`) mixed with variables.
    fn text_or_variable(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, result: &mut Tok, text: fn(&[u8], usize) -> i32) -> i32 {
        *result = None;
        let mut p = b;
        if p == s.len() {
            return 0;
        }
        loop {
            let rc = text(s, p);
            if rc < 0 { *result = None; return rc; }
            if rc > 0 {
                if !append(result, &s[p..p + rc as usize]) { *result = None; return ERR_OUT_OF_MEMORY; }
                p += rc as usize;
            }
            let mut tmp: Tok = None;
            let rc = self.parse_variable(ctx, s, p, &mut tmp);
            if rc < 0 { *result = None; return rc; }
            if rc > 0 {
                p += rc as usize;
                if !append(result, tmp.as_deref().unwrap_or(&[])) { *result = None; return ERR_OUT_OF_MEMORY; }
            }
            if rc <= 0 {
                break;
            }
        }
        (p - b) as i32
    }

    /// parse_operation(): one `:x...` post-operation applied to `data`.
    fn parse_operation(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, data: &mut Tok) -> i32 {
        let rc = self.operation(ctx, s, b, data);
        if rc < 0 {
            *data = None;
        }
        rc
    }

    fn operation(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, data: &mut Tok) -> i32 {
        let mut p = b;
        if p == s.len() {
            return 0;
        }
        match s[p].to_ascii_lowercase() {
            b'l' => {
                if let Some(d) = data { d.make_ascii_lowercase(); }
                p += 1;
            }
            b'u' => {
                if let Some(d) = data { d.make_ascii_uppercase(); }
                p += 1;
            }
            b'o' => {
                p += 1;
                let (rc, num1) = parse_integer(s, p);
                if rc == 0 { return ERR_MISSING_START_OFFSET; }
                p += rc;
                let isrange = match at(s, p) {
                    b',' => false,
                    b'-' => true,
                    _ => return ERR_INVALID_OFFSET_DELIMITER,
                };
                p += 1;
                let (rc, num2) = parse_integer(s, p);
                p += rc;
                if let Some(d) = data {
                    let rc = op_offset(d, num1, num2, isrange);
                    if rc < 0 { return rc; }
                }
            }
            b'#' => {
                if let Some(d) = data {
                    *d = d.len().to_string().into_bytes();
                }
                p += 1;
            }
            b'-' | b'*' | b'+' => {
                let op = s[p];
                p += 1;
                let mut tmp: Tok = None;
                let rc = self.text_or_variable(ctx, s, p, &mut tmp, parse_exptext);
                if rc < 0 { return rc; }
                if rc == 0 { return ERR_MISSING_PARAMETER_IN_COMMAND; }
                p += rc as usize;
                match op {
                    // substitute parameter if data is undefined or empty
                    b'-' => if data.as_ref().is_none_or(|d| d.is_empty()) { *data = tmp; },
                    // empty string if data is not empty, parameter otherwise
                    b'*' => if let Some(d) = data {
                        if d.is_empty() { *data = tmp; } else { d.clear(); }
                    },
                    // substitute parameter if data is not empty
                    _ => if data.as_ref().is_some_and(|d| !d.is_empty()) { *data = tmp; },
                }
            }
            b's' => {
                p += 1;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_REPLACE; }
                p += 1;
                let rc = parse_pattern(s, p);
                if rc < 0 { return rc; }
                let search = s[p..p + rc as usize].to_vec();
                p += rc as usize;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_REPLACE; }
                p += 1;
                let mut replace: Tok = None;
                let rc = self.text_or_variable(ctx, s, p, &mut replace, parse_substext);
                if rc < 0 { return rc; }
                p += rc as usize;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_REPLACE; }
                p += 1;
                let rc = parse_exptext(s, p);
                if rc < 0 { return rc; }
                let flags = s[p..p + rc as usize].to_vec();
                p += rc as usize;
                if data.is_some() {
                    let rc = op_search_and_replace(data, &search, &replace, &flags);
                    if rc < 0 { return rc; }
                }
            }
            b'y' => {
                p += 1;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_TRANSPOSE; }
                p += 1;
                let mut search: Tok = None;
                let rc = self.text_or_variable(ctx, s, p, &mut search, parse_substext);
                if rc < 0 { return rc; }
                p += rc as usize;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_TRANSPOSE; }
                p += 1;
                let mut replace: Tok = None;
                let rc = self.text_or_variable(ctx, s, p, &mut replace, parse_substext);
                if rc < 0 { return rc; }
                p += rc as usize;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_TRANSPOSE; }
                p += 1;
                if let Some(d) = data {
                    let rc = op_transpose(d, search.as_deref().unwrap_or(&[]), replace.as_deref().unwrap_or(&[]));
                    if rc < 0 { return rc; }
                }
            }
            b'p' => {
                p += 1;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_PADDING; }
                p += 1;
                let (rc, width) = parse_integer(s, p);
                if rc == 0 { return ERR_MISSING_PADDING_WIDTH; }
                p += rc;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_PADDING; }
                p += 1;
                let mut fill: Tok = None;
                let rc = self.text_or_variable(ctx, s, p, &mut fill, parse_substext);
                if rc < 0 { return rc; }
                p += rc as usize;
                if at(s, p) != b'/' { return ERR_MALFORMATTED_PADDING; }
                p += 1;
                let position = at(s, p);
                if !matches!(position, b'l' | b'c' | b'r') { return ERR_MALFORMATTED_PADDING; }
                p += 1;
                if let Some(d) = data {
                    let rc = op_padding(d, width, fill.as_deref().unwrap_or(&[]), position);
                    if rc < 0 { return rc; }
                }
            }
            b'%' => {
                p += 1;
                let len = parse_name(s, p);
                let op = s[p..p + len].to_vec();
                p += len;
                let mut arg: Option<Vec<u8>> = None;
                if at(s, p) == b'(' {
                    p += 1;
                    let mut args: Tok = None;
                    let rc = self.text_or_variable(ctx, s, p, &mut args, parse_opargtext);
                    if rc < 0 { return rc; }
                    p += rc as usize;
                    arg = Some(args.unwrap_or_default());
                    if at(s, p) != b')' { return ERR_MALFORMED_OPERATION_ARGUMENTS; }
                    p += 1;
                }
                if let Some(d) = data {
                    match self.resolver.operate(&op, arg.as_deref(), d) {
                        Ok(v) => *d = v,
                        Err(e) => return e,
                    }
                }
            }
            _ => return ERR_UNKNOWN_COMMAND_CHAR,
        }
        (p - b) as i32
    }

    /// parse_numexp_operand()
    fn numexp_operand(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, result: &mut i32, failed: &mut bool) -> i32 {
        let mut p = b;
        if p == s.len() {
            return ERR_INCOMPLETE_INDEX_SPEC;
        }
        let c = s[p];
        if c == b'(' {
            p += 1;
            let rc = self.numexp(ctx, s, p, result, failed);
            if rc < 0 { return rc; }
            p += rc as usize;
            if p == s.len() { return ERR_INCOMPLETE_INDEX_SPEC; }
            if s[p] != b')' { return ERR_UNCLOSED_BRACKET_IN_INDEX; }
            p += 1;
        } else if c == DELIM_INIT {
            // variable with forced expansion
            let mut tmp: Tok = None;
            let mut forced = Ctx { force_expand: true, ..ctx.clone() };
            let rc = self.parse_variable(&mut forced, s, p, &mut tmp);
            if rc == ERR_UNDEFINED_VARIABLE {
                *failed = true;
                let mut lazy = Ctx { force_expand: false, ..ctx.clone() };
                let rc = self.parse_variable(&mut lazy, s, p, &mut tmp);
                if rc < 0 { return rc; }
                p += rc as usize;
                *result = 0;
            } else if rc < 0 {
                return rc;
            } else {
                p += rc as usize;
                let inner = tmp.unwrap_or_default();
                let rc = self.numexp(ctx, &inner, 0, result, failed);
                if rc < 0 { return rc; }
            }
        } else if c == INDEX_MARK {
            p += 1;
            *result = ctx.index_this;
            if ctx.rel_lookup_flag {
                ctx.rel_lookup_cnt += 1;
            }
        } else if c.is_ascii_digit() {
            let (rc, n) = parse_integer(s, p);
            *result = n;
            p += rc;
        } else if c == b'+' || c == b'-' {
            if s.len() - p > 1 && s[p + 1].is_ascii_digit() {
                p += 1;
                let (rc, n) = parse_integer(s, p);
                *result = if c == b'-' { n.wrapping_neg() } else { n };
                p += rc;
            } else {
                return ERR_INVALID_CHAR_IN_INDEX_SPEC;
            }
        } else {
            return ERR_INVALID_CHAR_IN_INDEX_SPEC;
        }
        (p - b) as i32
    }

    /// parse_numexp(): `+ -` bind right-recursively, `* / %` per operand.
    fn numexp(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, result: &mut i32, failed: &mut bool) -> i32 {
        let mut p = b;
        if p == s.len() {
            return ERR_INCOMPLETE_INDEX_SPEC;
        }
        let rc = self.numexp_operand(ctx, s, p, result, failed);
        if rc < 0 { return rc; }
        p += rc as usize;
        while p != s.len() {
            let op = s[p];
            let mut right = 0;
            if op == b'+' || op == b'-' {
                p += 1;
                let rc = self.numexp(ctx, s, p, &mut right, failed);
                if rc < 0 { return rc; }
                p += rc as usize;
                *result = if op == b'+' { result.wrapping_add(right) } else { result.wrapping_sub(right) };
            } else if op == b'*' || op == b'/' || op == b'%' {
                p += 1;
                let rc = self.numexp_operand(ctx, s, p, &mut right, failed);
                if rc < 0 { return rc; }
                p += rc as usize;
                if op == b'*' {
                    *result = result.wrapping_mul(right);
                } else if right == 0 {
                    if *failed { *result = 0; } else { return ERR_DIVISION_BY_ZERO_IN_INDEX; }
                } else if op == b'/' {
                    *result = result.wrapping_div(right);
                } else {
                    *result = result.wrapping_rem(right);
                }
            } else {
                break;
            }
        }
        (p - b) as i32
    }

    /// lookup_value(): undefined variables count down and read as empty
    /// inside a loop without limits.
    fn lookup_value(&mut self, ctx: &mut Ctx, name: &[u8], inc: bool, idx: i32) -> Result<Vec<u8>, i32> {
        let rc = self.resolver.lookup(name, inc, idx);
        if ctx.rel_lookup_flag && rc == Err(ERR_UNDEFINED_VARIABLE) {
            ctx.rel_lookup_cnt -= 1;
            return Ok(Vec::new());
        }
        rc
    }

    /// parse_variable_complex(): `{name[index]:ops}` after the `$` at b-1.
    fn variable_complex(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, result: &mut Tok) -> i32 {
        let rc = self.variable_complex_inner(ctx, s, b, result);
        if rc < 0 {
            *result = None;
        }
        rc
    }

    fn variable_complex_inner(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, result: &mut Tok) -> i32 {
        let len = s.len();
        let mut p = b;
        *result = None;
        if p == len || s[p] != DELIM_OPEN {
            return 0;
        }
        p += 1;
        if p == len {
            return ERR_INCOMPLETE_VARIABLE_SPEC;
        }

        // name of variable: name characters and embedded variables
        let mut name: Tok = None;
        loop {
            let n = parse_name(s, p);
            if n > 0 {
                append(&mut name, &s[p..p + n]);
                p += n;
            }
            let mut tmp: Tok = None;
            let rc = self.parse_variable(ctx, s, p, &mut tmp);
            if rc < 0 { return rc; }
            if rc > 0 {
                append(&mut name, tmp.as_deref().unwrap_or(&[]));
                p += rc as usize;
            }
            if rc <= 0 {
                break;
            }
        }

        let mut failed = false;
        // literal result range for a failed index lookup: [b-1, lit_end)
        let mut lit_end = 0usize;
        if tok_len(&name) == 0 {
            if ctx.force_expand {
                return ERR_INCOMPLETE_VARIABLE_SPEC;
            }
            // back-off: keep the text as it is
            *result = Some(s[b - 1..p].to_vec());
        } else {
            let name = name.unwrap_or_default();
            let mut idx = 0;
            if at(s, p) == INDEX_OPEN {
                p += 1;
                let rc = self.numexp(ctx, s, p, &mut idx, &mut failed);
                if rc < 0 { return rc; }
                if rc == 0 { return ERR_INCOMPLETE_INDEX_SPEC; }
                p += rc as usize;
                if p == len { return ERR_INCOMPLETE_INDEX_SPEC; }
                if s[p] != INDEX_CLOSE { return ERR_INVALID_CHAR_IN_INDEX_SPEC; }
                p += 1;
            }
            if p == len || (s[p] != DELIM_CLOSE && s[p] != b':' && s[p] != b'+') {
                return ERR_INCOMPLETE_VARIABLE_SPEC;
            }
            let inc = s[p] == b'+';
            p += 1;
            if inc {
                // skips the byte after '+' (normally the closing brace)
                p += 1;
            }
            if failed {
                lit_end = p;
            } else {
                match self.lookup_value(ctx, &name, inc, idx) {
                    Err(ERR_UNDEFINED_VARIABLE) => *result = None,
                    Err(e) => return e,
                    Ok(v) => *result = Some(v),
                }
            }
        }

        // optional post-operations
        if at(s, p.wrapping_sub(1)) == b':' {
            let mut tmp: Tok = None;
            p -= 1;
            while p != len && s[p] == b':' {
                p += 1;
                let rc = if failed {
                    self.parse_operation(ctx, s, p, &mut tmp)
                } else {
                    self.parse_operation(ctx, s, p, result)
                };
                if rc < 0 { return rc; }
                p += rc as usize;
                if failed { lit_end += rc as usize; }
            }
            if p == len || s[p] != DELIM_CLOSE {
                return ERR_INCOMPLETE_VARIABLE_SPEC;
            }
            p += 1;
            if failed { lit_end += 1; }
        } else if at(s, p.wrapping_sub(1)) == b'+' {
            p += 1;
        }
        // The C parser may step behind the end here (`${X+`); stop at it.
        p = p.min(len);

        if failed {
            *result = Some(s[b - 1..lit_end.min(len)].to_vec());
        } else if result.is_none() {
            // lazy handling of an undefined variable
            if ctx.force_expand {
                return ERR_UNDEFINED_VARIABLE;
            }
            *result = Some(s[b - 1..p].to_vec());
        }
        (p - b) as i32
    }

    /// ParseVariable(): `$name` or `${...}`.
    fn parse_variable(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, result: &mut Tok) -> i32 {
        let mut p = b;
        *result = None;
        if p >= s.len() || s[p] != DELIM_INIT {
            return 0;
        }
        p += 1;
        if p == s.len() {
            return ERR_INCOMPLETE_VARIABLE_SPEC;
        }
        let n = parse_name(s, p);
        if n > 0 {
            let inc = at(s, p + n) == b'+';
            return match self.lookup_value(ctx, &s[p..p + n], inc, 0) {
                Err(ERR_UNDEFINED_VARIABLE) if !ctx.force_expand => {
                    *result = Some(s[b..b + 1 + n].to_vec());
                    1 + n as i32
                }
                Err(e) => e,
                Ok(v) => {
                    *result = Some(v);
                    1 + n as i32
                }
            };
        }
        let rc = self.variable_complex(ctx, s, p, result);
        if rc > 0 { rc + 1 } else { rc }
    }

    /// parse_looplimits(): `{start,step,stop}` after a loop construct.
    #[allow(clippy::too_many_arguments)]
    fn looplimits(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, start: &mut i32, step: &mut i32, stop: &mut i32, open_stop: &mut bool) -> i32 {
        let mut p = b;
        if p >= s.len() || s[p] != DELIM_OPEN {
            return 0;
        }
        p += 1;

        let mut failed = false;
        let rc = self.numexp(ctx, s, p, start, &mut failed);
        if rc == ERR_INVALID_CHAR_IN_INDEX_SPEC { *start = 0; } else if rc < 0 { return rc; } else { p += rc as usize; }
        if failed { return ERR_UNDEFINED_VARIABLE; }
        if at(s, p) != b',' { return ERR_INVALID_CHAR_IN_LOOP_LIMITS; }
        p += 1;

        let mut failed = false;
        let rc = self.numexp(ctx, s, p, step, &mut failed);
        if rc == ERR_INVALID_CHAR_IN_INDEX_SPEC { *step = 1; } else if rc < 0 { return rc; } else { p += rc as usize; }
        if failed { return ERR_UNDEFINED_VARIABLE; }
        if at(s, p) != b',' {
            if at(s, p) != DELIM_CLOSE { return ERR_INVALID_CHAR_IN_LOOP_LIMITS; }
            p += 1;
            *stop = *step;
            *step = 1;
            *open_stop = rc <= 0;
            return (p - b) as i32;
        }
        p += 1;

        let mut failed = false;
        let rc = self.numexp(ctx, s, p, stop, &mut failed);
        if rc == ERR_INVALID_CHAR_IN_INDEX_SPEC {
            *stop = 0;
            *open_stop = true;
        } else if rc < 0 {
            return rc;
        } else {
            *open_stop = false;
            p += rc as usize;
        }
        if failed { return ERR_UNDEFINED_VARIABLE; }
        if at(s, p) != DELIM_CLOSE { return ERR_INVALID_CHAR_IN_LOOP_LIMITS; }
        p += 1;
        (p - b) as i32
    }

    /// parse_text(): plain text up to `$`, `[` or `]`.
    fn parse_text(s: &[u8], b: usize) -> i32 {
        let mut p = b;
        while p < s.len() {
            if s[p] == ESCAPE {
                p += 1;
                if p == s.len() {
                    return ERR_INCOMPLETE_QUOTED_PAIR;
                }
            } else if s[p] == DELIM_INIT || s[p] == INDEX_OPEN || s[p] == INDEX_CLOSE {
                break;
            }
            p += 1;
        }
        (p - b) as i32
    }

    /// parse_input(): text, variables and `[...]{b,s,e}` loop constructs.
    fn parse_input(&mut self, ctx: &mut Ctx, s: &[u8], b: usize, output: &mut Tok, level: u32) -> i32 {
        let len = s.len();
        let mut p = b;
        let mut rc: i32;
        loop {
            if p != len && s[p] == INDEX_OPEN {
                p += 1;
                let mut loop_limit_length: i32 = -1;
                let mut rel_lookup_cnt = ctx.rel_lookup_cnt;
                let mut open_stop = true;
                rc = 0;
                let (mut start, mut step, mut stop) = (0i32, 1i32, 0i32);
                let mut output_backup = 0usize;
                're_loop: loop {
                    let mut i = start;
                    while (open_stop && (loop_limit_length < 0 || rel_lookup_cnt > ctx.rel_lookup_cnt))
                        || (!open_stop && i <= stop)
                    {
                        output_backup = tok_len(output);
                        let mut inner = Ctx { force_expand: true, rel_lookup_flag: true, index_this: i, ..ctx.clone() };
                        rc = self.parse_input(&mut inner, s, p, output, level + 1);
                        rel_lookup_cnt = inner.rel_lookup_cnt;
                        if rc < 0 {
                            return Self::error_return(s, b, p, output, rc);
                        }
                        if at(s, p + rc as usize) != INDEX_CLOSE {
                            return Self::error_return(s, b, p, output, ERR_UNTERMINATED_LOOP_CONSTRUCT);
                        }
                        if loop_limit_length < 0 {
                            let rc2 = self.looplimits(ctx, s, p + rc as usize + 1, &mut start, &mut step, &mut stop, &mut open_stop);
                            if rc2 < 0 {
                                // C: `goto error_return` with rc still the
                                // (positive) length of the loop body.
                                return Self::error_return(s, b, p, output, rc);
                            } else if rc2 == 0 {
                                loop_limit_length = 0;
                            } else {
                                loop_limit_length = rc2;
                                if let Some(o) = output { o.truncate(output_backup); }
                                continue 're_loop;
                            }
                        }
                        i = i.wrapping_add(step);
                        if tok_len(output) >= OUTPUT_LIMIT {
                            return Self::error_return(s, b, p, output, ERR_OUT_OF_MEMORY);
                        }
                    }
                    break;
                }
                if open_stop {
                    if let Some(o) = output { o.truncate(output_backup); }
                }
                p += rc as usize + 1 + loop_limit_length.max(0) as usize;
                p = p.min(len);
                if !(p != len && rc > 0) { break; }
                continue;
            }

            rc = Self::parse_text(s, p);
            if rc > 0 {
                if !append(output, &s[p..p + rc as usize]) {
                    return Self::error_return(s, b, p, output, ERR_OUT_OF_MEMORY);
                }
                p += rc as usize;
                if !(p != len && rc > 0) { break; }
                continue;
            } else if rc < 0 {
                return Self::error_return(s, b, p, output, rc);
            }

            let mut result: Tok = None;
            rc = self.parse_variable(ctx, s, p, &mut result);
            if rc > 0 {
                if !append(output, result.as_deref().unwrap_or(&[])) {
                    return Self::error_return(s, b, p, output, ERR_OUT_OF_MEMORY);
                }
                p += rc as usize;
                if !(p != len && rc > 0) { break; }
                continue;
            }
            if rc < 0 {
                return Self::error_return(s, b, p, output, rc);
            }
            break;
        }

        if level == 0 && p != len {
            return Self::error_return(s, b, p, output, ERR_INPUT_ISNT_TEXT_NOR_VARIABLE);
        }
        (p - b) as i32
    }

    /// error_return of parse_input(): the output becomes the parsed input.
    fn error_return(s: &[u8], b: usize, p: usize, output: &mut Tok, rc: i32) -> i32 {
        *output = Some(s[b..p.min(s.len())].to_vec());
        rc
    }
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

fn hex_value(c: u8) -> u8 {
    (c as char).to_digit(16).unwrap_or(0) as u8
}

/// expand_simple_hex(): two digits, leaves `i` on the second.
fn expand_simple_hex(s: &[u8], i: &mut usize, dst: &mut Vec<u8>) -> Result<(), i32> {
    if s.len() - *i < 2 {
        return Err(ERR_INCOMPLETE_HEX);
    }
    if !is_hex(s[*i]) || !is_hex(s[*i + 1]) {
        return Err(ERR_INVALID_HEX);
    }
    let c = (hex_value(s[*i]) << 4).wrapping_add(hex_value(s[*i + 1]));
    *i += 1;
    dst.push(c);
    Ok(())
}

/// expand_hex(): `\xHH` or `\x{HHHH...}`.
fn expand_hex(s: &[u8], i: &mut usize, dst: &mut Vec<u8>) -> Result<(), i32> {
    if *i == s.len() {
        return Err(ERR_INCOMPLETE_HEX);
    }
    if s[*i] != b'{' {
        return expand_simple_hex(s, i, dst);
    }
    *i += 1;
    while *i < s.len() && s[*i] != b'}' {
        expand_simple_hex(s, i, dst)?;
        *i += 1;
    }
    if *i == s.len() {
        return Err(ERR_INCOMPLETE_GROUPED_HEX);
    }
    Ok(())
}

/// expand_octal(): three octal digits (max \377), leaves `i` on the last.
fn expand_octal(s: &[u8], i: &mut usize, dst: &mut Vec<u8>) -> Result<(), i32> {
    if s.len() - *i < 3 {
        return Err(ERR_INCOMPLETE_OCTAL);
    }
    let oct = |c: u8| (b'0'..=b'7').contains(&c);
    if !oct(s[*i]) || !oct(s[*i + 1]) || !oct(s[*i + 2]) {
        return Err(ERR_INVALID_OCTAL);
    }
    let first = s[*i] - b'0';
    if first > 3 {
        return Err(ERR_OCTAL_TOO_LARGE);
    }
    let c = first * 64 + (s[*i + 1] - b'0') * 8 + (s[*i + 2] - b'0');
    *i += 2;
    dst.push(c);
    Ok(())
}

/// var_unescape(): `all` = false keeps unknown escapes (and `\\`) for the
/// expansion step.
pub fn unescape(s: &[u8], all: bool) -> Result<Vec<u8>, i32> {
    let mut dst = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' {
            dst.push(s[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i == s.len() {
            return Err(ERR_INCOMPLETE_NAMED_CHARACTER);
        }
        match s[i] {
            b'\\' => {
                if !all { dst.push(b'\\'); }
                dst.push(b'\\');
            }
            b'n' => dst.push(b'\n'),
            b't' => dst.push(b'\t'),
            b'r' => dst.push(b'\r'),
            b'x' => {
                i += 1;
                expand_hex(s, &mut i, &mut dst)?;
            }
            c if c.is_ascii_digit() && s.len() - i >= 3 && s[i + 1].is_ascii_digit() && s[i + 2].is_ascii_digit() => {
                expand_octal(s, &mut i, &mut dst)?;
            }
            c => {
                if !all { dst.push(b'\\'); }
                dst.push(c);
            }
        }
        i += 1;
    }
    Ok(dst)
}

/// var_expand() without forced expansion.
pub fn expand(input: &[u8], resolver: &mut dyn Resolver) -> Result<Vec<u8>, i32> {
    if input.is_empty() {
        return Err(ERR_INVALID_ARGUMENT);
    }
    let mut var = Var { resolver };
    let mut ctx = Ctx::default();
    let mut output: Tok = None;
    let rc = var.parse_input(&mut ctx, input, 0, &mut output, 0);
    if rc < 0 {
        return Err(rc);
    }
    Ok(output.unwrap_or_default())
}

/// C string semantics (strlen): everything from the first NUL on is lost.
fn c_str(mut s: Vec<u8>) -> Vec<u8> {
    if let Some(n) = s.iter().position(|c| *c == 0) {
        s.truncate(n);
    }
    s
}

/// dird/expand.cc VariableExpansion(): unescape, expand, unescape again.
/// Err carries the job message the C++ Director emits (M_ERROR).
pub fn variable_expansion(input: &[u8], resolver: &mut dyn Resolver) -> Result<Vec<u8>, String> {
    let inp = c_str(unescape(input, false)
        .map_err(|rc| format!("Cannot unescape string: ERR={}\n", strerror(rc)))?);
    let out = expand(&inp, resolver).map_err(|rc| format!(
        "Cannot expand expression \"{}\": ERR={}\n", String::from_utf8_lossy(&inp), strerror(rc)))?;
    let out = unescape(&out, true)
        .map_err(|rc| format!("Cannot unescape string: ERR={}\n", strerror(rc)))?;
    Ok(c_str(out))
}

// ---------------------------------------------------------------------
// Director lookups (dird/expand.cc)
// ---------------------------------------------------------------------

/// Job values of the `JobControlRecord` the expansion runs for (job_item).
#[derive(Debug, Clone, Default)]
pub struct JobVars {
    pub job: String,
    pub dir: String,
    pub level: String,
    pub job_type: String,
    pub job_id: u32,
    pub client: String,
    pub num_vols: u32,
    pub pool: String,
    pub storage: String,
    pub catalog: String,
    pub media_type: String,
    pub job_name: String,
}

/// A Counter resource (dird_conf.cc counter_items).
#[derive(Debug, Clone)]
pub struct CounterDef {
    pub name: String,
    pub min: i32,
    pub max: i32,
    pub wrap_counter: String,
    pub catalog: String,
}

impl CounterDef {
    /// Counter resources from the raw configuration, in resource order.
    pub fn from_raw(raw: &[bareos_config::raw::ConfResource]) -> Vec<CounterDef> {
        raw.iter().filter(|r| r.norm_type() == "counter").filter_map(|r| {
            let value = |key: &str| r.directives.iter()
                .find(|d| d.norm_key() == key).and_then(|d| d.first()).map(str::to_string);
            Some(CounterDef {
                name: r.name()?.to_string(),
                min: value("minimum").and_then(|v| v.parse().ok()).unwrap_or(0),
                max: value("maximum").and_then(|v| v.parse().ok()).unwrap_or(i32::MAX),
                wrap_counter: value("wrapcounter").unwrap_or_default(),
                catalog: value("catalog").unwrap_or_default(),
            })
        }).collect()
    }
}

/// CounterResource::CurrentValue of every counter (process wide like the
/// resource). Unknown counters start at their minimum (check_catalog.cc).
static COUNTER_VALUES: Mutex<Option<HashMap<String, i32>>> = Mutex::new(None);

/// Catalog write of an incremented counter (BareosDb::UpdateCounterRecord).
pub type CounterUpdate<'a> = dyn FnMut(&CounterDef, i32) -> Result<(), String> + 'a;

/// dird/expand.cc lookup_var()/operate_var() for one job.
pub struct DirectorResolver<'a> {
    pub job: JobVars,
    pub counters: Vec<CounterDef>,
    /// None: counters are never written to the catalog.
    pub update_counter: Option<Box<CounterUpdate<'a>>>,
}

/// built_in_vars[] in table order.
const BUILT_IN: &[&str] = &[
    "Year", "Month", "Day", "Hour", "Minute", "Second", "WeekDay", "Job", "Dir", "Level", "Type",
    "JobId", "Client", "NumVols", "Pool", "Storage", "Catalog", "MediaType", "JobName",
];

impl DirectorResolver<'_> {
    /// lookup_built_in_var(): `bstrncmp(name, var, var_len)` — the variable
    /// only has to be a prefix of a table name (`${Y}` is the year).
    fn built_in(&self, name: &[u8]) -> Option<Vec<u8>> {
        let entry = BUILT_IN.iter().position(|t| t.as_bytes().starts_with(name))?;
        if entry < 7 {
            return Some(date_item(entry).to_string().into_bytes());
        }
        let j = &self.job;
        let value = match BUILT_IN[entry] {
            "Job" => j.job.clone(),
            "Dir" => j.dir.clone(),
            "Level" => j.level.clone(),
            "Type" => j.job_type.clone(),
            "JobId" => j.job_id.to_string(),
            "Client" => if j.client.is_empty() { " ".into() } else { j.client.clone() },
            "NumVols" => j.num_vols.to_string(),
            "Pool" => j.pool.clone(),
            "Storage" => j.storage.clone(),
            "Catalog" => j.catalog.clone(),
            "MediaType" => j.media_type.clone(),
            _ => j.job_name.clone(),
        };
        Some(value.into_bytes())
    }

    /// lookup_counter_var(); `${Counter+}` increments (and wraps) the counter
    /// and writes it to the catalog when the counter has one.
    fn counter(&mut self, name: &[u8], inc: bool, index: i32) -> Option<Vec<u8>> {
        let def = self.counters.iter().find(|c| c.name.as_bytes() == name)?.clone();
        let mut values = COUNTER_VALUES.lock().unwrap_or_else(|e| e.into_inner());
        let current = values.get_or_insert_with(HashMap::new).entry(def.name.clone()).or_insert(def.min);
        let value = current.to_string();
        if index == -1 {
            return Some(value.len().to_string().into_bytes());
        }
        if inc {
            *current = if *current == def.max { def.min } else { current.wrapping_add(1) };
            let new_value = *current;
            drop(values);
            if !def.catalog.is_empty() {
                if let Some(update) = self.update_counter.as_mut() {
                    if let Err(e) = update(&def, new_value) {
                        tracing::error!("Count not update counter {}: ERR={e}", def.name);
                    }
                }
            }
        }
        Some(value.into_bytes())
    }
}

/// date_item(): local time fields as plain numbers.
fn date_item(entry: usize) -> i32 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    match entry {
        0 => tm.tm_year + 1900,
        1 => tm.tm_mon + 1,
        2 => tm.tm_mday,
        3 => tm.tm_hour,
        4 => tm.tm_min,
        5 => tm.tm_sec,
        _ => tm.tm_wday,
    }
}

/// Environment lookup of lookup_var(): `|` separates array elements.
fn environment(name: &[u8], index: i32) -> Option<Vec<u8>> {
    let key = std::ffi::CString::new(name).ok()?;
    let ptr = unsafe { libc::getenv(key.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    let value = unsafe { std::ffi::CStr::from_ptr(ptr) }.to_bytes().to_vec();
    let count = 1 + value.iter().filter(|c| **c == b'|').count() as i64;
    if index == -1 {
        let len = if count == 1 { value.len() as i64 } else { count };
        return Some(len.to_string().into_bytes());
    }
    if index < -1 || index as i64 > count - 1 {
        return None;
    }
    value.split(|c| *c == b'|').nth(index as usize).map(<[u8]>::to_vec)
}

impl Resolver for DirectorResolver<'_> {
    fn lookup(&mut self, name: &[u8], inc: bool, index: i32) -> Result<Vec<u8>, i32> {
        if let Some(v) = self.built_in(name) {
            return Ok(v);
        }
        if let Some(v) = self.counter(name, inc, index) {
            return Ok(v);
        }
        environment(name, index).ok_or(ERR_UNDEFINED_VARIABLE)
    }

    /// operate_var() defines no operation. (For `%inc` the C++ Director
    /// dereferences a NULL argument and crashes; here it is an error.)
    fn operate(&mut self, _op: &[u8], _arg: Option<&[u8]>, _value: &[u8]) -> Result<Vec<u8>, i32> {
        Err(ERR_UNDEFINED_OPERATION)
    }
}

/// SQL of BareosDb::UpdateCounterRecord().
pub fn update_counter_sql(def: &CounterDef, value: i32) -> String {
    let esc = |s: &str| s.replace('\'', "''");
    format!("UPDATE Counters SET MinValue={},MaxValue={},CurrentValue={},WrapCounter='{}' WHERE Counter='{}'",
        def.min, def.max, value, esc(&def.wrap_counter), esc(&def.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver() -> DirectorResolver<'static> {
        DirectorResolver {
            job: JobVars {
                job: "backup-bareos-fd".into(), dir: "bareos-dir".into(), level: " ".into(),
                job_type: "Console".into(), client: "bareos-fd".into(), pool: "Incremental".into(),
                storage: "File".into(), catalog: "MyCatalog".into(), media_type: "File".into(),
                job_name: "-Console-.2026-10-15_20.56.25_04".into(), ..Default::default()
            },
            counters: Vec::new(),
            update_counter: None,
        }
    }

    fn x(input: &str) -> String {
        match variable_expansion(input.as_bytes(), &mut resolver()) {
            Ok(v) => String::from_utf8_lossy(&v).into_owned(),
            Err(e) => format!("ERR {e}"),
        }
    }

    #[test]
    fn plain_text_and_job_items() {
        assert_eq!(x("hello world"), "hello world");
        assert_eq!(x("${Dir}|${Type}|${Level}|${JobId}|${NumVols}|${Catalog}"), "bareos-dir|Console| |0|0|MyCatalog");
        assert_eq!(x("${Job}|${Client}|${Pool}|${Storage}|${MediaType}"), "backup-bareos-fd|bareos-fd|Incremental|File|File");
        assert_eq!(x("$Dir $Type x$Levelx"), "bareos-dir Console x$Levelx");
        assert_eq!(x("\"${Dir}\" 'x'"), "\"bareos-dir\" 'x'");
        // prefix match of the built-in table
        assert_eq!(x("${Di}/${J}/${Med}"), "bareos-dir/backup-bareos-fd/File");
    }

    #[test]
    fn undefined_variables_stay_literal() {
        assert_eq!(x("${Undefined}"), "${Undefined}");
        assert_eq!(x("$Undefined end"), "$Undefined end");
        assert_eq!(x("${}"), "${}");
        assert_eq!(x("${Undef:+set}|${Undef:*x}|"), "${Undef:+set}|${Undef:*x}|");
    }

    #[test]
    fn operations() {
        assert_eq!(x("${Dir:u} ${Dir:l} ${Dir:#} ${Dir:o1,3} ${Dir:o1-3}"), "BAREOS-DIR bareos-dir 10 are are");
        assert_eq!(x("${Dir:s/dir/DIR/} ${Dir:s/r/R/g} ${Dir:s/x/y/t} ${Dir:y/a-z/A-Z/}"), "bareos-DIR baReos-diR bareos-dir BAREOS-DIR");
        assert_eq!(x("${Dir:p/20/*/l}| ${Dir:p/20/*/r}| ${Dir:p/20/-=/c}|"),
            "bareos-dir**********| **********bareos-dir| -=-=-bareos-dir-=-=-|");
        assert_eq!(x("${Undef:-dflt} ${Dir:+set} ${Dir:*x}"), "dflt set ");
        assert_eq!(x("${Dir:s/(b)(a)/\\2\\1/}"), "abreos-dir");
    }

    #[test]
    fn escapes() {
        assert_eq!(x("a\\nb \\\\ \\x41 \\101 \\q"), "a\nb \\ A A q");
        assert_eq!(x("\\x{4142}"), "AB");
        assert_eq!(x("a\\x00b"), "a");
    }

    #[test]
    fn errors() {
        assert_eq!(x(""), "ERR Cannot expand expression \"\": ERR=invalid argument\n");
        assert_eq!(x("${"), "ERR Cannot expand expression \"${\": ERR=incomplete variable specification\n");
        assert_eq!(x("\\"), "ERR Cannot unescape string: ERR=incomplete named character\n");
        assert_eq!(x("${Dir:q}"), "ERR Cannot expand expression \"${Dir:q}\": ERR=unknown command character in variable\n");
        assert_eq!(x("${Dir:%inc}"), "ERR Cannot expand expression \"${Dir:%inc}\": ERR=undefined operation\n");
        assert_eq!(x("a]b"), "ERR Cannot expand expression \"a]b\": ERR=input is neither text nor variable\n");
        assert!(x("${Dir:o9,1}").starts_with("ERR "));
        assert!(x("${Dir[}").starts_with("ERR "));
    }

    #[test]
    fn loops_and_indexes() {
        // SAFETY: test-only variable that no other code reads.
        unsafe { std::env::set_var("ONESIMUS_EXPAND_TEST", "a|b|c") };
        assert_eq!(x("[${Dir}]"), "");
        assert_eq!(x("${ONESIMUS_EXPAND_TEST[1]} ${ONESIMUS_EXPAND_TEST[-1]} ${ONESIMUS_EXPAND_TEST[3]}"),
            "b 3 ${ONESIMUS_EXPAND_TEST[3]}");
        assert_eq!(x("[${ONESIMUS_EXPAND_TEST[#]},]"), "a,b,c,");
        assert_eq!(x("[x${ONESIMUS_EXPAND_TEST[#]}]{0,1,3}"), "xaxbxcx");
        assert_eq!(x("[${ONESIMUS_EXPAND_TEST[#+1]}]{0,2}"), "bc");
        // a bad limit keeps the text up to the loop start (C quirk)
        assert_eq!(x("ab[x]{z}"), "ab[");
    }

    #[test]
    fn counters_increment_only_with_plus() {
        let counters = vec![CounterDef { name: "ExpandTestCounter".into(), min: 5, max: 6, wrap_counter: String::new(), catalog: "MyCatalog".into() }];
        let mut writes = Vec::new();
        {
            let mut r = DirectorResolver { counters, update_counter: Some(Box::new(|d: &CounterDef, v| { writes.push(update_counter_sql(d, v)); Ok(()) })), ..resolver() };
            let mut run = |s: &str| String::from_utf8(variable_expansion(s.as_bytes(), &mut r).unwrap()).unwrap();
            assert_eq!(run("${ExpandTestCounter}"), "5");
            assert_eq!(run("${ExpandTestCounter+}"), "5");
            assert_eq!(run("${ExpandTestCounter}"), "6");
            assert_eq!(run("${ExpandTestCounter+}"), "6");
            // wrapped to the minimum
            assert_eq!(run("${ExpandTestCounter}"), "5");
        }
        assert_eq!(writes, [
            "UPDATE Counters SET MinValue=5,MaxValue=6,CurrentValue=6,WrapCounter='' WHERE Counter='ExpandTestCounter'",
            "UPDATE Counters SET MinValue=5,MaxValue=6,CurrentValue=5,WrapCounter='' WHERE Counter='ExpandTestCounter'",
        ]);
    }
}
