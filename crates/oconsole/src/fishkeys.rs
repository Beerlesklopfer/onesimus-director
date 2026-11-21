//! Key bindings like fish for the interactive console
//! (docs/plans/OCONSOLE-COMPLETION.md, D7), on rustyline's
//! `bind_sequence` with conditional handlers.
//!
//! The decisions are plain functions of the line, the cursor and the
//! suggestion ([`right_action`], [`word_action`], [`esc_action`]), tested
//! without a terminal; the handlers only adapt them to rustyline. Keys
//! without a binding here keep rustyline's Emacs default: Tab completes
//! (list), Ctrl-R searches the history incrementally, Ctrl-C clears the
//! line (the main loop keeps the session), Ctrl-D on an empty line quits.

#![warn(clippy::missing_docs_in_private_items)]

use std::sync::{Arc, Mutex};

use rustyline::{
    At, Cmd, ConditionalEventHandler, Editor, Event, EventContext, EventHandler, KeyCode, KeyEvent, Modifiers,
    Movement, RepeatCount, Word,
};

/// What a key does, decided from the line, the cursor and the suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Accept the whole suggestion.
    AcceptSuggestion,
    /// Insert this text: the next word of the suggestion.
    InsertWord(String),
    /// Let rustyline do its default for the key.
    Default,
}

/// Right, Ctrl-F, End: with the cursor at the end and a suggestion to
/// insert (`hint`, rustyline's hint completion), accept it.
pub fn right_action(line: &str, pos: usize, hint: Option<&str>) -> Action {
    match hint {
        Some(text) if pos == line.len() && !text.is_empty() => Action::AcceptSuggestion,
        _ => Action::Default,
    }
}

/// Alt-Right, Alt-F: with a suggestion at the end of the line, its next
/// word (leading blanks included, up to the next blank).
pub fn word_action(line: &str, pos: usize, hint: Option<&str>) -> Action {
    let Some(text) = hint.filter(|t| pos == line.len() && !t.is_empty()) else { return Action::Default };
    let blanks = text.len() - text.trim_start().len();
    let word_end = text[blanks..].find(char::is_whitespace).map(|i| blanks + i).unwrap_or(text.len());
    Action::InsertWord(text[..word_end].to_string())
}

/// Esc: hide a shown suggestion (true) until the line changes.
pub fn esc_action(has_hint: bool) -> bool {
    has_hint
}

/// The line for which Esc hid the suggestion; shared between the key
/// handler and the Hinter.
#[derive(Debug, Clone, Default)]
pub struct HiddenHint(Arc<Mutex<Option<String>>>);

impl HiddenHint {
    /// Hide the suggestion while the line is `line`.
    pub fn hide(&self, line: &str) {
        if let Ok(mut hidden) = self.0.lock() {
            *hidden = Some(line.to_string());
        }
    }

    /// Whether the suggestion for `line` is hidden (Esc on this very text).
    pub fn is_hidden(&self, line: &str) -> bool {
        self.0.lock().map(|hidden| hidden.as_deref() == Some(line)).unwrap_or(false)
    }
}

/// The history as the Up/Down search sees it: the entries of the history
/// file and every line added in this session, oldest first. rustyline's
/// handlers get no history, so the main loop keeps this copy in step with
/// `add_history_entry`.
#[derive(Debug, Clone, Default)]
pub struct SharedHistory(Arc<Mutex<Vec<String>>>);

impl SharedHistory {
    /// Append a line (a repeat of the newest entry is not stored twice,
    /// as in rustyline's history with `history_ignore_dups`).
    pub fn push(&self, line: &str) {
        if let Ok(mut entries) = self.0.lock() {
            if entries.last().map(String::as_str) != Some(line) {
                entries.push(line.to_string());
            }
        }
    }

    /// Replace the entries (after loading the history file).
    pub fn set(&self, entries: impl IntoIterator<Item = String>) {
        if let Ok(mut all) = self.0.lock() {
            *all = entries.into_iter().collect();
        }
    }
}

/// A running Up/Down search: the text typed before the first key (the
/// prefix), the position in the history and the line shown last. A line
/// different from the shown one starts a new search.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    /// What was typed when the search started.
    prefix: String,
    /// Index of the entry shown last; the history length before the first.
    index: usize,
    /// The line the last key put in the editor.
    shown: Option<String>,
}

/// Up (`backward`) and Down like fish: through the entries that start with
/// the text typed before the first key (all entries on an empty line),
/// skipping entries equal to the line on screen. Down past the newest match
/// brings back the typed text. None: no further match (the line stays).
pub fn history_search(history: &[String], state: &mut SearchState, line: &str, backward: bool) -> Option<String> {
    if state.shown.as_deref() != Some(line) {
        *state = SearchState { prefix: line.to_string(), index: history.len(), shown: None };
    }
    let matches = |i: &usize| history[*i].starts_with(&state.prefix) && history[*i] != line;
    let next = if backward {
        (0..state.index).rev().find(matches)
    } else {
        (state.index + 1..history.len()).find(matches)
    };
    let text = match next {
        Some(i) => {
            state.index = i;
            history[i].clone()
        }
        None if !backward && state.index < history.len() => {
            state.index = history.len();
            state.prefix.clone()
        }
        None => return None,
    };
    state.shown = Some(text.clone());
    Some(text)
}

/// Up and Down ([`history_search`]).
struct SearchHistory {
    /// The entries to search.
    history: SharedHistory,
    /// The running search, shared by Up and Down.
    state: Arc<Mutex<SearchState>>,
    /// Up (true) or Down.
    backward: bool,
}

impl ConditionalEventHandler for SearchHistory {
    fn handle(&self, _evt: &Event, _n: RepeatCount, _positive: bool, ctx: &EventContext) -> Option<Cmd> {
        let history = self.history.0.lock().ok()?;
        let mut state = self.state.lock().ok()?;
        match history_search(&history, &mut state, ctx.line(), self.backward) {
            Some(text) => Some(Cmd::Replace(Movement::WholeLine, Some(text))),
            None => Some(Cmd::Noop),
        }
    }
}

/// The bound keys and what they do (oconsole(1) KEYS lists them; the test
/// checks that [`bind`] binds each).
#[cfg_attr(not(test), allow(dead_code))]
pub fn bindings() -> Vec<(KeyEvent, &'static str)> {
    vec![
        (KeyEvent(KeyCode::Up, Modifiers::NONE), "previous history entry starting with the typed text (all on an empty line)"),
        (KeyEvent(KeyCode::Down, Modifiers::NONE), "next history entry starting with the typed text, at the end the typed text"),
        (KeyEvent(KeyCode::Right, Modifiers::NONE), "accept the suggestion, else move right"),
        (KeyEvent::ctrl('F'), "accept the suggestion, else move right"),
        (KeyEvent(KeyCode::End, Modifiers::NONE), "accept the suggestion, else move to the end"),
        (KeyEvent(KeyCode::Right, Modifiers::ALT), "accept the next word of the suggestion, else move a word"),
        (KeyEvent::alt('f'), "accept the next word of the suggestion, else move a word"),
        (KeyEvent(KeyCode::Esc, Modifiers::NONE), "hide the suggestion until the line changes"),
    ]
}

/// Right, Ctrl-F, End: accept the suggestion ([`right_action`]).
struct AcceptHint;

impl ConditionalEventHandler for AcceptHint {
    fn handle(&self, _evt: &Event, _n: RepeatCount, _positive: bool, ctx: &EventContext) -> Option<Cmd> {
        match right_action(ctx.line(), ctx.pos(), ctx.hint_text()) {
            Action::AcceptSuggestion => Some(Cmd::CompleteHint),
            _ => None, // default binding (move right / to the end)
        }
    }
}

/// Alt-Right, Alt-F: accept the next word of the suggestion ([`word_action`]).
struct AcceptWord;

impl ConditionalEventHandler for AcceptWord {
    fn handle(&self, _evt: &Event, _n: RepeatCount, _positive: bool, ctx: &EventContext) -> Option<Cmd> {
        match word_action(ctx.line(), ctx.pos(), ctx.hint_text()) {
            Action::InsertWord(text) => Some(Cmd::Insert(1, text)),
            _ => Some(Cmd::Move(Movement::ForwardWord(1, At::AfterEnd, Word::Emacs))),
        }
    }
}

/// Esc: hide the suggestion for the current line ([`esc_action`]).
struct HideHint(HiddenHint);

impl ConditionalEventHandler for HideHint {
    fn handle(&self, _evt: &Event, _n: RepeatCount, _positive: bool, ctx: &EventContext) -> Option<Cmd> {
        if esc_action(ctx.has_hint()) {
            self.0.hide(ctx.line());
            Some(Cmd::Repaint)
        } else {
            Some(Cmd::Noop)
        }
    }
}

/// Bind the keys of [`bindings`]; `hidden` is the Hinter's [`HiddenHint`],
/// `history` the copy the main loop keeps.
pub fn bind<H: rustyline::Helper, I: rustyline::history::History>(rl: &mut Editor<H, I>, hidden: HiddenHint,
                                                                   history: SharedHistory) {
    let state = Arc::new(Mutex::new(SearchState::default()));
    for (key, backward) in [(KeyCode::Up, true), (KeyCode::Down, false)] {
        let handler = SearchHistory { history: history.clone(), state: state.clone(), backward };
        rl.bind_sequence(KeyEvent(key, Modifiers::NONE), EventHandler::Conditional(Box::new(handler)));
    }
    for key in [KeyEvent(KeyCode::Right, Modifiers::NONE), KeyEvent::ctrl('F'), KeyEvent(KeyCode::End, Modifiers::NONE)] {
        rl.bind_sequence(key, EventHandler::Conditional(Box::new(AcceptHint)));
    }
    for key in [KeyEvent(KeyCode::Right, Modifiers::ALT), KeyEvent::alt('f')] {
        rl.bind_sequence(key, EventHandler::Conditional(Box::new(AcceptWord)));
    }
    rl.bind_sequence(KeyEvent(KeyCode::Esc, Modifiers::NONE), EventHandler::Conditional(Box::new(HideHint(hidden))));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_accepts_only_at_the_end_with_a_hint() {
        assert_eq!(right_action("purge vol", 9, Some("ume")), Action::AcceptSuggestion);
        assert_eq!(right_action("purge vol", 3, Some("ume")), Action::Default, "cursor inside the line");
        assert_eq!(right_action("purge vol", 9, None), Action::Default, "no hint or a usage hint");
        assert_eq!(right_action("purge vol", 9, Some("")), Action::Default);
    }

    #[test]
    fn alt_right_takes_one_word() {
        assert_eq!(word_action("purge", 5, Some(" volume all pool=Full")), Action::InsertWord(" volume".into()));
        assert_eq!(word_action("purge vol", 9, Some("ume all")), Action::InsertWord("ume".into()));
        assert_eq!(word_action("list", 4, Some(" jobs")), Action::InsertWord(" jobs".into()));
        assert_eq!(word_action("list", 2, Some(" jobs")), Action::Default);
    }

    #[test]
    fn without_hint_keys_keep_their_default() {
        assert_eq!(right_action("", 0, None), Action::Default);
        assert_eq!(word_action("status", 6, None), Action::Default);
        assert!(!esc_action(false));
        assert!(esc_action(true));
    }

    fn h(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn up_on_an_empty_line_walks_the_whole_history() {
        let history = h(&["status dir", "list jobs", "version"]);
        let mut s = SearchState::default();
        assert_eq!(history_search(&history, &mut s, "", true).as_deref(), Some("version"));
        assert_eq!(history_search(&history, &mut s, "version", true).as_deref(), Some("list jobs"));
        assert_eq!(history_search(&history, &mut s, "list jobs", true).as_deref(), Some("status dir"));
        assert_eq!(history_search(&history, &mut s, "status dir", true), None, "oldest reached");
        assert_eq!(history_search(&history, &mut s, "status dir", false).as_deref(), Some("list jobs"));
        assert_eq!(history_search(&history, &mut s, "list jobs", false).as_deref(), Some("version"));
        assert_eq!(history_search(&history, &mut s, "version", false).as_deref(), Some(""), "back to the typed text");
        assert_eq!(history_search(&history, &mut s, "", false), None);
    }

    #[test]
    fn up_keeps_the_typed_prefix_and_skips_the_line_itself() {
        let history = h(&["list volumes", "status dir", "list jobs", "list jobs", "version"]);
        let mut s = SearchState::default();
        assert_eq!(history_search(&history, &mut s, "list", true).as_deref(), Some("list jobs"));
        assert_eq!(history_search(&history, &mut s, "list jobs", true).as_deref(), Some("list volumes"),
            "the duplicate equal to the shown line is skipped");
        assert_eq!(history_search(&history, &mut s, "list volumes", false).as_deref(), Some("list jobs"));
        // editing the recalled line starts a new search with it as prefix
        assert_eq!(history_search(&history, &mut s, "list jobs ", true), None);
        assert_eq!(history_search(&history, &mut s, "st", true).as_deref(), Some("status dir"));
    }

    #[test]
    fn shared_history_drops_repeats_of_the_newest_entry() {
        let shared = SharedHistory::default();
        shared.set(h(&["a"]));
        shared.push("b");
        shared.push("b");
        shared.push("a");
        assert_eq!(*shared.0.lock().unwrap(), h(&["a", "b", "a"]));
    }

    #[test]
    fn esc_hides_until_the_line_changes() {
        let hidden = HiddenHint::default();
        assert!(!hidden.is_hidden("list "));
        hidden.hide("list ");
        assert!(hidden.is_hidden("list "));
        assert!(!hidden.is_hidden("list j"), "the next key shows suggestions again");
        let shared = hidden.clone();
        shared.hide("x");
        assert!(hidden.is_hidden("x"), "shared with the Hinter");
    }

    #[test]
    fn every_documented_key_is_bound_once() {
        let keys: Vec<KeyEvent> = bindings().into_iter().map(|(k, _)| k).collect();
        for (i, key) in keys.iter().enumerate() {
            assert!(!keys[i + 1..].contains(key), "{key:?} twice");
        }
        let mut rl: Editor<(), rustyline::history::DefaultHistory> = Editor::new().unwrap();
        bind(&mut rl, HiddenHint::default(), SharedHistory::default());
        for key in keys {
            assert!(rl.bind_sequence(key, EventHandler::Simple(Cmd::Noop)).is_some(), "{key:?} not bound");
        }
    }
}
