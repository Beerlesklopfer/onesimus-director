//! Three-second admission gate for terminal commands and dialog replies.
use std::{
    io::{self, Write},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub const INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Default)]
pub struct CommandRateGuard {
    last: Arc<Mutex<Option<Instant>>>,
}
impl CommandRateGuard {
    pub fn remaining(&self) -> Duration {
        self.remaining_at(Instant::now())
    }
    fn remaining_at(&self, now: Instant) -> Duration {
        self.last
            .lock()
            .unwrap()
            .map(|last| INTERVAL.saturating_sub(now.saturating_duration_since(last)))
            .unwrap_or_default()
    }
    fn check_at(&self, now: Instant) -> Result<(), Duration> {
        let mut last = self.last.lock().unwrap();
        if let Some(previous) = *last {
            let remaining = INTERVAL.saturating_sub(now.saturating_duration_since(previous));
            if !remaining.is_zero() {
                return Err(remaining);
            }
        }
        *last = Some(now);
        Ok(())
    }
    pub fn admit(&self) -> bool {
        self.admit_at(Instant::now(), &mut io::stderr().lock())
    }
    fn admit_at(&self, now: Instant, stderr: &mut impl Write) -> bool {
        match self.check_at(now) {
            Ok(()) => true,
            Err(remaining) => {
                let milliseconds = remaining.as_nanos().div_ceil(1_000_000);
                let _ = writeln!(
                    stderr,
                    "oconsole: Command Rate Guard violation: input discarded; at most one command every 3 seconds (retry in {milliseconds} ms)."
                );
                let _ = stderr.flush();
                false
            }
        }
    }
    pub fn prompt(&self, text: &str, color: bool) -> String {
        self.prompt_at(text, color, Instant::now())
    }
    fn prompt_at(&self, text: &str, color: bool, now: Instant) -> String {
        if !color {
            return text.to_string();
        }
        let code = if self.remaining_at(now).is_zero() {
            32
        } else {
            31
        };
        format!("\x1b[{code}m{text}\x1b[0m")
    }
}

/// One bounded-lifetime worker redraws an idle readline prompt when it turns green.
/// The external-printer API preserves the edited input and cursor on redraw.
#[derive(Default)]
pub struct PromptRefresh {
    shared: Option<Arc<(Mutex<RefreshState>, Condvar)>>,
}
#[derive(Default)]
struct RefreshState {
    deadline: Option<Instant>,
    stop: bool,
}
impl PromptRefresh {
    pub fn new(mut printer: impl rustyline::ExternalPrinter + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(RefreshState::default()), Condvar::new()));
        let worker = shared.clone();
        std::thread::spawn(move || {
            let (lock, wake) = &*worker;
            let mut state = lock.lock().unwrap();
            loop {
                if state.stop {
                    break;
                }
                let Some(deadline) = state.deadline else {
                    state = wake.wait(state).unwrap();
                    continue;
                };
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    state.deadline = None;
                    drop(state);
                    // An empty external message asks readline to redraw its prompt.
                    let _ = printer.print(String::new());
                    state = lock.lock().unwrap();
                } else {
                    state = wake.wait_timeout(state, remaining).unwrap().0;
                }
            }
        });
        Self {
            shared: Some(shared),
        }
    }
    pub fn arm(&self, remaining: Duration) {
        if let Some(shared) = &self.shared {
            let (lock, wake) = &**shared;
            lock.lock().unwrap().deadline = if remaining.is_zero() {
                None
            } else {
                Some(Instant::now() + remaining)
            };
            wake.notify_one();
        }
    }
    pub fn disarm(&self) {
        self.arm(Duration::ZERO);
    }
}
impl Drop for PromptRefresh {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            let (lock, wake) = &**shared;
            lock.lock().unwrap().stop = true;
            wake.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_command_immediate_then_exact_three_second_boundary() {
        let guard = CommandRateGuard::default();
        let now = Instant::now();
        assert_eq!(guard.check_at(now), Ok(()));
        assert_eq!(
            guard.check_at(now + Duration::from_millis(2999)),
            Err(Duration::from_millis(1))
        );
        assert_eq!(guard.check_at(now + INTERVAL), Ok(()));
        assert_eq!(guard.check_at(now + INTERVAL), Err(INTERVAL));
    }
    #[test]
    fn spam_is_discarded_and_does_not_extend_cooldown() {
        let guard = CommandRateGuard::default();
        let now = Instant::now();
        let mut stderr = vec![];
        assert!(guard.admit_at(now, &mut stderr));
        for milliseconds in 0..3000 {
            assert!(!guard.admit_at(now + Duration::from_millis(milliseconds), &mut stderr));
        }
        assert!(guard.admit_at(now + INTERVAL, &mut stderr));
        assert_eq!(
            String::from_utf8(stderr)
                .unwrap()
                .matches("Command Rate Guard violation")
                .count(),
            3000
        );
    }
    #[test]
    fn shared_guard_covers_main_prompt_and_dialogs_and_reports_only_to_supplied_stderr() {
        let main = CommandRateGuard::default();
        let dialog = main.clone();
        let now = Instant::now();
        let mut stderr = vec![];
        assert!(main.admit_at(now, &mut stderr));
        assert!(stderr.is_empty());
        assert!(!dialog.admit_at(now + Duration::from_secs(1), &mut stderr));
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .contains("retry in 2000 ms")
        );
        assert!(dialog.admit_at(now + INTERVAL, &mut vec![]));
        assert!(main.check_at(now + INTERVAL).is_err());
    }
    #[test]
    fn prompt_is_green_ready_red_blocked_and_plain_without_color() {
        let guard = CommandRateGuard::default();
        let now = Instant::now();
        assert_eq!(guard.prompt_at("dir> ", true, now), "\x1b[32mdir> \x1b[0m");
        guard.check_at(now).unwrap();
        assert_eq!(guard.prompt_at("dir> ", true, now), "\x1b[31mdir> \x1b[0m");
        assert_eq!(guard.prompt_at("yes/no> ", false, now), "yes/no> ");
        assert_eq!(
            guard.prompt_at("dir> ", true, now + INTERVAL),
            "\x1b[32mdir> \x1b[0m"
        );
    }
}
