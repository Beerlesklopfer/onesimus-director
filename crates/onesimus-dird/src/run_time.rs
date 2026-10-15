//! When a Schedule Run directive triggers — port of C++ dird/date_time.cc,
//! dird/date_time_mask.cc, RunResource::NextScheduleTime (dird_conf.cc) and
//! lib/btime.cc TmWoy(). Everything works in local time (localtime_r /
//! mktime) like bareos-dir, so DST changes behave the same.

use bareos_config::resources::{DateTimeMask, RunDirective};

fn localtime(t: libc::time_t) -> libc::tm {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm
}

fn mktime(tm: &mut libc::tm) -> libc::time_t {
    unsafe { libc::mktime(tm) }
}

/// lib/btime.cc TmWoy(): week of the year 0-53 (week 1 contains 4 January).
pub fn tm_woy(t: libc::time_t) -> i32 {
    let mut tm = localtime(t);
    let tm_yday = tm.tm_yday;
    tm.tm_mon = 0;
    tm.tm_mday = 4;
    tm.tm_isdst = 0; // 4 Jan is not DST
    let time4 = mktime(&mut tm);
    let tm = localtime(time4);
    let mut fty = 1 - tm.tm_wday;
    if fty <= 0 {
        fty += 7;
    }
    let woy = tm_yday - fty + 4;
    if woy < 0 {
        return 0;
    }
    1 + woy / 7
}

/// C++ dird/date_time.h DateTime.
struct DateTime {
    year: i32,
    month: i32,
    week_of_year: i32,
    day_of_year: i32,
    day_of_month: i32,
    day_of_week: i32,
    hour: i32,
}

impl DateTime {
    fn new(t: libc::time_t) -> Self {
        let tm = localtime(t);
        Self {
            day_of_month: tm.tm_mday - 1,
            day_of_week: tm.tm_wday,
            week_of_year: tm_woy(t),
            month: tm.tm_mon,
            day_of_year: tm.tm_yday,
            hour: tm.tm_hour,
            year: tm.tm_year,
        }
    }

    fn week_of_month(&self) -> i32 {
        self.day_of_month / 7
    }

    /// C++ passes tm_year (years since 1900) to IsLeapYear; kept as is.
    fn on_last_7days_of_month(&self) -> bool {
        const LEAP: [i32; 12] = [30, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365];
        const NORMAL: [i32; 12] = [30, 58, 89, 119, 150, 180, 211, 242, 272, 303, 333, 364];
        let y = self.year;
        let leap = y % 400 == 0 || (y % 100 != 0 && y % 4 == 0);
        let last_day = if leap { LEAP } else { NORMAL }[self.month as usize];
        last_day - 7 < self.day_of_year
    }
}

/// DateTimeMask::TriggersOnDay()
pub fn triggers_on_day(mask: &DateTimeMask, t: libc::time_t) -> bool {
    let dt = DateTime::new(t);
    mask.has_mday(dt.day_of_month)
        && mask.has_wday(dt.day_of_week)
        && mask.has_month(dt.month)
        && (mask.has_wom(dt.week_of_month()) || (dt.on_last_7days_of_month() && mask.last_7days_of_month))
        && mask.has_woy(dt.week_of_year)
}

/// DateTimeMask::TriggersOnDayAndHour()
pub fn triggers_on_day_and_hour(mask: &DateTimeMask, t: libc::time_t) -> bool {
    triggers_on_day(mask, t) && mask.has_hour(DateTime::new(t).hour)
}

/// The run time inside the hour of `t`: local time of `t` with the run's
/// minute and second 0 (ua_status.cc show_scheduled_preview,
/// scheduler_private.cc).
pub fn run_time_in_hour(run: &RunDirective, t: libc::time_t) -> libc::time_t {
    let mut tm = localtime(t);
    tm.tm_min = run.minute as i32;
    tm.tm_sec = 0;
    mktime(&mut tm)
}

/// RunResource::NextScheduleTime(): the first trigger on the day of
/// `start` (from its hour on, minutes not compared) or one of the next
/// `ndays` days.
pub fn next_schedule_time(run: &RunDirective, mut start: libc::time_t, ndays: u32) -> Option<libc::time_t> {
    for d in 0..=ndays {
        if triggers_on_day(&run.mask, start) {
            let mut tm = localtime(start);
            let first = if d == 0 { tm.tm_hour } else { 0 };
            for h in first..24 {
                if run.mask.has_hour(h) {
                    tm.tm_hour = h;
                    tm.tm_min = run.minute as i32;
                    tm.tm_sec = 0;
                    return Some(mktime(&mut tm));
                }
            }
        }
        start += 24 * 60 * 60;
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests that change TZ hold this lock.
    pub(crate) static TZ_LOCK: Mutex<()> = Mutex::new(());

    unsafe extern "C" {
        fn tzset();
    }

    pub(crate) fn with_tz<R>(tz: &str, f: impl FnOnce() -> R) -> R {
        let _guard = TZ_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var("TZ").ok();
        unsafe { std::env::set_var("TZ", tz) };
        unsafe { tzset() };
        let r = f();
        match old {
            Some(v) => unsafe { std::env::set_var("TZ", v) },
            None => unsafe { std::env::remove_var("TZ") },
        }
        unsafe { tzset() };
        r
    }

    fn run(spec: &str) -> RunDirective {
        let input = format!("Schedule {{\n Name = s\n Run = {spec}\n}}\n");
        match &bareos_config::parser::parse_config(&input).unwrap()[0] {
            bareos_config::Resource::Schedule(s) => s.runs[0].clone(),
            _ => unreachable!(),
        }
    }

    /// Local time -> time_t through mktime (tm_isdst = -1).
    fn at(y: i32, mo: i32, d: i32, h: i32, mi: i32) -> libc::time_t {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = y - 1900;
        tm.tm_mon = mo - 1;
        tm.tm_mday = d;
        tm.tm_hour = h;
        tm.tm_min = mi;
        tm.tm_isdst = -1;
        mktime(&mut tm)
    }

    fn fmt(t: libc::time_t) -> String {
        crate::localtime::strftime("%Y-%m-%d %H:%M %a", t)
    }

    #[test]
    fn weekdays_and_positions() {
        with_tz("UTC", || {
            let incr = run("Incremental mon-fri at 21:00");
            // Thu 15-Oct-2026 20:47 -> today 21:00
            let now = at(2026, 10, 15, 20, 47);
            assert_eq!(fmt(next_schedule_time(&incr, now, 1).unwrap()), "2026-10-15 21:00 Thu");
            // Fri 16-Oct 22:00 -> no run left on Friday, Sat/Sun excluded
            let now = at(2026, 10, 16, 22, 0);
            assert_eq!(next_schedule_time(&incr, now, 1), None);
            assert_eq!(fmt(next_schedule_time(&incr, now, 3).unwrap()), "2026-10-19 21:00 Mon");
            // The current hour still counts (minutes are not compared)
            let now = at(2026, 10, 15, 21, 30);
            assert_eq!(fmt(next_schedule_time(&incr, now, 0).unwrap()), "2026-10-15 21:00 Thu");

            let first_sat = run("Full 1st sat at 21:00");
            let other_sat = run("Differential 2nd-5th sat at 21:00");
            let sat = |d| triggers_on_day(&first_sat.mask, at(2026, 10, d, 12, 0));
            assert!(sat(3) && !sat(10) && !sat(17) && !sat(4));
            let sat2 = |d| triggers_on_day(&other_sat.mask, at(2026, 10, d, 12, 0));
            assert!(!sat2(3) && sat2(10) && sat2(17) && sat2(31));
            assert!(triggers_on_day_and_hour(&first_sat.mask, at(2026, 10, 3, 21, 59)));
            assert!(!triggers_on_day_and_hour(&first_sat.mask, at(2026, 10, 3, 20, 59)));
        });
    }

    #[test]
    fn month_and_year_boundaries() {
        with_tz("UTC", || {
            let last = run("Full last fri at 23:00");
            assert!(triggers_on_day(&last.mask, at(2026, 10, 30, 0, 0)));
            assert!(!triggers_on_day(&last.mask, at(2026, 10, 23, 0, 0)));
            // From 31-Oct the next "last fri" is 27-Nov
            assert_eq!(fmt(next_schedule_time(&last, at(2026, 10, 31, 0, 0), 40).unwrap()),
                       "2026-11-27 23:00 Fri");
            let first = run("Full 1 at 0:05");
            assert_eq!(fmt(next_schedule_time(&first, at(2026, 12, 31, 23, 0), 2).unwrap()),
                       "2027-01-01 00:05 Fri");
            let monthly = run("Full on 31 at 1:00");
            assert_eq!(fmt(next_schedule_time(&monthly, at(2026, 11, 1, 0, 0), 60).unwrap()),
                       "2026-12-31 01:00 Thu");
            // Week of year: 1-Jan-2027 (Friday) belongs to week 0, 4-Jan to week 1
            assert_eq!(tm_woy(at(2027, 1, 1, 12, 0)), 0);
            assert_eq!(tm_woy(at(2027, 1, 4, 12, 0)), 1);
            let w01 = run("Full w01 mon at 3:00");
            assert_eq!(fmt(next_schedule_time(&w01, at(2026, 12, 28, 12, 0), 14).unwrap()),
                       "2027-01-04 03:00 Mon");
        });
    }

    #[test]
    fn daylight_saving_changes() {
        with_tz("Europe/Berlin", || {
            // 29-Mar-2026: 02:00 -> 03:00 CEST. The daily 21:00 stays 21:00 local.
            let daily = run("Incremental daily at 21:00");
            let t = next_schedule_time(&daily, at(2026, 3, 28, 22, 0), 2).unwrap();
            assert_eq!(fmt(t), "2026-03-29 21:00 Sun");
            assert_eq!(t - at(2026, 3, 28, 21, 0), 23 * 3600);
            // 25-Oct-2026: 03:00 -> 02:00 CET
            let t = next_schedule_time(&daily, at(2026, 10, 24, 22, 0), 2).unwrap();
            assert_eq!(fmt(t), "2026-10-25 21:00 Sun");
            assert_eq!(t - at(2026, 10, 24, 21, 0), 25 * 3600);
            // Hourly stepping through the spring gap never sees hour 2
            let at2 = run("Full daily at 2:30");
            let mut seen = Vec::new();
            let mut t = at(2026, 3, 29, 0, 0);
            while t < at(2026, 3, 29, 6, 0) {
                if triggers_on_day_and_hour(&at2.mask, t) {
                    seen.push(fmt(run_time_in_hour(&at2, t)));
                }
                t += 3600;
            }
            assert!(seen.is_empty());
            // ... and sees hour 2 twice in autumn
            let mut seen = Vec::new();
            let mut t = at(2026, 10, 25, 0, 0);
            while t < at(2026, 10, 25, 6, 0) {
                if triggers_on_day_and_hour(&at2.mask, t) {
                    seen.push(run_time_in_hour(&at2, t));
                }
                t += 3600;
            }
            assert_eq!(seen.len(), 2);
            assert_eq!(seen[1] - seen[0], 3600);
        });
    }
}
