//! Timers, reminders and the calendar.

use super::*;


pub(super) fn minutes_label(minutes: f64) -> String {
    let secs = (minutes * 60.0).round() as u64;
    match secs {
        s if s < 60 => format!("{s}-second"),
        s if s % 3600 == 0 => format!("{}-hour", s / 3600),
        s if s % 60 == 0 => format!("{}-minute", s / 60),
        s => format!("{}-minute {}-second", s / 60, s % 60),
    }
}

pub(super) fn timer(action: &str, minutes: Option<f64>, label: &str, host: &dyn Host) -> Result<String> {
    match action {
        "start" => {
            let minutes = minutes.filter(|m| *m > 0.0).ok_or_else(|| anyhow!("How long should the timer be?"))?;
            let seconds = (minutes * 60.0).round().max(1.0) as u64;
            host.start_timer(seconds, label.to_string());
            Ok(format!("Started a {} timer{}", minutes_label(minutes), if label.is_empty() { String::new() } else { format!(" for {label}") }))
        }
        "cancel" | "stop" => Ok(match host.cancel_timers() {
            0 => "There were no timers".into(),
            n => format!("Cancelled {n} timer{}", if n == 1 { "" } else { "s" }),
        }),
        _ => {
            let timers = host.timers();
            if timers.is_empty() {
                return Ok("No timers are running".into());
            }
            Ok(timers
                .iter()
                .map(|(label, left)| format!("{}: {} min {} s left", if label.is_empty() { "Timer" } else { label }, left / 60, left % 60))
                .collect::<Vec<_>>()
                .join("\n"))
        }
    }
}

/// "2026-10-03 17:30", "2026-10-03T17:30", or a date alone (9 in the morning).
pub(super) fn parse_when(when: &str) -> Option<chrono::NaiveDateTime> {
    use chrono::{NaiveDate, NaiveDateTime};
    ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S"]
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(when, f).ok())
        .or_else(|| NaiveDate::parse_from_str(when, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(9, 0, 0)))
}

pub(super) fn add_reminder(title: &str, when: &str) -> Result<String> {
    use chrono::{Datelike, Timelike};
    if title.is_empty() {
        bail!("No reminder text");
    }
    let script = r#"on run argv
        set t to item 1 of argv
        tell application "Reminders"
            if (count of argv) is 1 then
                tell default list to make new reminder with properties {name:t}
            else
                set d to current date
                set day of d to 1
                set year of d to (item 2 of argv) as integer
                set month of d to (item 3 of argv) as integer
                set day of d to (item 4 of argv) as integer
                set time of d to (item 5 of argv) as integer
                tell default list to make new reminder with properties {name:t, remind me date:d}
            end if
        end tell
        return "ok"
    end run"#;
    if when.is_empty() {
        osascript(script, &[title])?;
        return Ok(format!("Added the reminder “{title}”"));
    }
    let at = parse_when(when).ok_or_else(|| anyhow!("Couldn't understand the time {when}; use YYYY-MM-DD HH:MM"))?;
    let parts = [at.year().to_string(), at.month().to_string(), at.day().to_string(), at.num_seconds_from_midnight().to_string()];
    osascript(script, &[title, &parts[0], &parts[1], &parts[2], &parts[3]])?;
    Ok(format!("Added the reminder “{title}” for {}", at.format("%A %-d %B at %-I:%M %p")))
}

/// Calendar events from EventKit, which (unlike AppleScript) includes repeating events.
pub(super) fn calendar(date: &str, days: u64) -> Result<String> {
    use chrono::{Local, NaiveDate, TimeZone};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObjectProtocol};
    use objc2::sel;
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore};
    use objc2_foundation::{NSDate, NSError};

    let first = if date.is_empty() { Local::now().date_naive() } else { NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| anyhow!("Use the date format YYYY-MM-DD"))? };
    let days = days.clamp(1, 14);
    let start = Local.from_local_datetime(&first.and_hms_opt(0, 0, 0).unwrap()).earliest().ok_or_else(|| anyhow!("bad date"))?;
    let end = start + chrono::Duration::days(days as i64);

    let events = unsafe {
        let store: Retained<EKEventStore> = EKEventStore::new();
        let status = EKEventStore::authorizationStatusForEntityType(EKEntityType::Event);
        if status != EKAuthorizationStatus::FullAccess {
            if status != EKAuthorizationStatus::NotDetermined {
                bail!("Murmur isn't allowed to see the calendar. Allow it in System Settings → Privacy & Security → Calendars");
            }
            let (tx, rx) = std::sync::mpsc::channel::<bool>();
            let tx = Mutex::new(tx);
            let done = block2::RcBlock::new(move |granted: Bool, _error: *mut NSError| {
                let _ = tx.lock().unwrap().send(granted.as_bool());
            });
            // macOS 14 split calendar access into full and write-only.
            if store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
                store.requestFullAccessToEventsWithCompletion(&*done as *const _ as *mut _);
            } else {
                #[allow(deprecated)]
                store.requestAccessToEntityType_completion(EKEntityType::Event, &*done as *const _ as *mut _);
            }
            if !rx.recv_timeout(Duration::from_secs(120)).unwrap_or(false) {
                bail!("Murmur wasn't given access to the calendar");
            }
        }
        let from = NSDate::dateWithTimeIntervalSince1970(start.timestamp() as f64);
        let to = NSDate::dateWithTimeIntervalSince1970(end.timestamp() as f64);
        let predicate = store.predicateForEventsWithStartDate_endDate_calendars(&from, &to, None);
        let mut events: Vec<(i64, i64, bool, String, String)> = store
            .eventsMatchingPredicate(&predicate)
            .iter()
            .map(|e| {
                (
                    e.startDate().timeIntervalSince1970() as i64,
                    e.endDate().timeIntervalSince1970() as i64,
                    e.isAllDay(),
                    e.title().to_string(),
                    e.location().map(|l| l.to_string()).unwrap_or_default(),
                )
            })
            .collect();
        events.sort();
        events
    };

    let span = if days == 1 { first.format("%A %-d %B").to_string() } else { format!("the {days} days from {}", first.format("%A %-d %B")) };
    if events.is_empty() {
        return Ok(format!("No events on {span}"));
    }
    let time = |t: i64| Local.timestamp_opt(t, 0).single().map(|d| d.format("%-I:%M %p").to_string()).unwrap_or_default();
    let day = |t: i64| Local.timestamp_opt(t, 0).single().map(|d| d.format("%a %-d %b").to_string()).unwrap_or_default();
    let mut out = format!("{} event{} on {span}:\n", events.len(), if events.len() == 1 { "" } else { "s" });
    for (from, to, all_day, title, place) in events.iter().take(30) {
        let when = if *all_day { format!("{}, all day", day(*from)) } else { format!("{}, {} to {}", day(*from), time(*from), time(*to)) };
        let place = if place.is_empty() { String::new() } else { format!(" ({place})") };
        out.push_str(&format!("{when}: {title}{place}\n"));
    }
    Ok(out)
}
