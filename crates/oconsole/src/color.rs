//! NO_COLOR applies to console decoration and terminal escape sequences in replies.
use std::borrow::Cow;

pub fn enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}

pub fn output(text: &str) -> Cow<'_, str> {
    without_sgr(text, !enabled())
}

/// Console decoration for stdout: styling only with colour enabled and stdout on a terminal.
pub fn stdout(text: &str) -> Cow<'_, str> {
    use std::io::IsTerminal;
    for_terminal(text, std::io::stdout().is_terminal())
}

/// Console decoration for stderr: styling only with colour enabled and stderr on a terminal.
pub fn stderr(text: &str) -> Cow<'_, str> {
    use std::io::IsTerminal;
    for_terminal(text, std::io::stderr().is_terminal())
}

/// Styling stays only when colour is enabled and the stream is a terminal.
fn for_terminal(text: &str, is_terminal: bool) -> Cow<'_, str> {
    without_sgr(text, !enabled() || !is_terminal)
}

fn without_sgr(text: &str, disabled: bool) -> Cow<'_, str> {
    if !disabled || !text.contains('\x1b') {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            let mut sequence = String::from("\x1b[");
            chars.next();
            while let Some(next) = chars.peek().copied() {
                sequence.push(next);
                chars.next();
                if ('@'..='~').contains(&next) {
                    // Only remove styling, preserving other terminal controls.
                    if next != 'm' { output.push_str(&sequence); }
                    break;
                }
                if chars.peek().is_none() { output.push_str(&sequence); }
            }
        } else {
            output.push(c);
        }
    }
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    #[test]
    fn no_styling_without_a_terminal() {
        assert_eq!(super::for_terminal("\x1b[2m(97 commands)\x1b[0m", false), "(97 commands)");
        assert_eq!(super::for_terminal("plain", false), "plain");
    }

    use super::*;

    #[test]
    fn removes_style_without_changing_response_bytes() {
        let text = "\x1b[1;36mGröße\x1b[0m\t42\r\n";
        assert_eq!(without_sgr(text, true), "Größe\t42\r\n");
        assert_eq!(without_sgr(text, false), text);
        assert_eq!(without_sgr("a\x1b[2Kb", true), "a\x1b[2Kb");
    }
}
