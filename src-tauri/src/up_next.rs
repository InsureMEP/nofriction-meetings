//! Up next (docs/design/FADELL_V2.md §3 A and §6 "The routine"): one
//! suggestion to record what is starting, with the answers already filled
//! in. Three signals, in this order:
//!
//! 1. **Calendar**: an event that is on now or begins within 15 minutes
//!    (only when Calendar access was already granted; this never prompts).
//! 2. **Routine**: a notebook the user records on this weekday at about
//!    this time. Learned from the recordings, never set up.
//! 3. **Call**: a call app has taken the microphone (macOS 14.2+).
//!
//! The suggestion is shown on the idle Record screen and in the tray. With
//! the window away it is also a notification ("Remind me when something
//! starts", on by default), and with "Record by itself" on (off by default:
//! recording people is the user's act) it starts the recording.
//!
//! Everything is computed on this Mac from what is already stored. Nothing
//! here reads or keeps transcript text, and nothing is sent anywhere.

use crate::recording_kind::RecordingKind;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDateTime, Timelike, Utc};
use serde::Serialize;

/// How far back a habit is looked for.
pub const LOOKBACK_DAYS: i64 = 42;
/// A past recording matches now from this long before its start time of day…
pub const BEFORE_MIN: i64 = 10;
/// …to this long after it.
pub const AFTER_MIN: i64 = 25;
/// Recordings that must match before a notebook is a routine.
pub const MIN_MATCHES: usize = 2;
/// A call app must hold the microphone this long before it counts (a blip
/// from dictation or a permission check isn't a call).
pub const CALL_HOLD_SECS: u64 = 10;

/// Setting: post a notification when something starts ("true" unless set).
pub const REMIND_SETTING: &str = "remind_when_something_starts";
/// Setting: start the recording instead of suggesting it ("false" unless set).
pub const AUTO_START_SETTING: &str = "auto_start_recording";
/// Setting: `key|YYYY-MM-DD` lines the user said "Not now" to today.
pub const DISMISSED_SETTING: &str = "up_next_dismissed";

/// Emitted when the suggestion changes (payload: `UpNext` or null).
pub const CHANGED_EVENT: &str = "up_next";
/// Emitted to start the suggested recording (tray item, Record by itself).
pub const START_EVENT: &str = "up_next_start";

// ═══════════════════════════════════════════════════════════════════════════
// The routine (pure)
// ═══════════════════════════════════════════════════════════════════════════

/// A past recording that has a notebook. `started` is local time.
#[derive(Debug, Clone, PartialEq)]
pub struct Past {
    pub notebook: String,
    pub kind: RecordingKind,
    pub started: NaiveDateTime,
    pub duration_seconds: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Routine {
    /// The notebook, as its most recent recording spells it
    pub notebook: String,
    /// The type of its most recent recording
    pub kind: RecordingKind,
    /// "15" | "30" | "60" | "90" | "none"
    pub duration: &'static str,
    pub matches: usize,
    /// Median start of the matching recordings, in minutes of the day
    pub usual_minute: i64,
}

fn minute_of_day(t: NaiveDateTime) -> i64 {
    (t.hour() * 60 + t.minute()) as i64
}

fn median(mut v: Vec<i64>) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[(v.len() - 1) / 2])
}

/// The picker length that holds `minutes`: rounded up to 15/30/60/90, and
/// no limit past that (a recording is never cut short by a guess).
pub fn length_choice(minutes: Option<i64>) -> &'static str {
    match minutes {
        Some(m) if m <= 0 => "none",
        Some(m) if m <= 15 => "15",
        Some(m) if m <= 30 => "30",
        Some(m) if m <= 60 => "60",
        Some(m) if m <= 90 => "90",
        _ => "none",
    }
}

/// The notebook the user usually records about now, if any (§6).
pub fn routine(past: &[Past], now: NaiveDateTime) -> Option<Routine> {
    use std::collections::BTreeMap;
    let today = now.date();
    let now_min = minute_of_day(now);
    let mut by_notebook: BTreeMap<String, Vec<&Past>> = BTreeMap::new();
    for p in past {
        let name = p.notebook.trim();
        if !name.is_empty() {
            by_notebook.entry(name.to_lowercase()).or_default().push(p);
        }
    }
    let mut best: Option<(usize, NaiveDateTime, Routine)> = None;
    for recs in by_notebook.values() {
        if recs.iter().any(|p| p.started.date() == today) {
            continue; // already recorded today
        }
        let matching: Vec<&&Past> = recs
            .iter()
            .filter(|p| {
                let days = (today - p.started.date()).num_days();
                let s = minute_of_day(p.started);
                (1..=LOOKBACK_DAYS).contains(&days)
                    && p.started.weekday() == today.weekday()
                    && now_min >= s - BEFORE_MIN
                    && now_min <= s + AFTER_MIN
            })
            .collect();
        if matching.len() < MIN_MATCHES {
            continue;
        }
        let Some(latest) = recs.iter().max_by_key(|p| p.started) else { continue };
        let minutes = median(
            matching
                .iter()
                .filter_map(|p| p.duration_seconds.filter(|d| *d > 0))
                .map(|d| (d + 59) / 60)
                .collect(),
        );
        let found = Routine {
            notebook: latest.notebook.trim().to_string(),
            kind: latest.kind,
            duration: length_choice(minutes),
            matches: matching.len(),
            usual_minute: median(matching.iter().map(|p| minute_of_day(p.started)).collect()).unwrap_or(now_min),
        };
        let better = match &best {
            None => true,
            Some((n, at, _)) => (matching.len(), latest.started) > (*n, *at),
        };
        if better {
            best = Some((matching.len(), latest.started, found));
        }
    }
    best.map(|(_, _, r)| r)
}

/// 545 → "9:05 AM".
pub fn clock_label(minute_of_day: i64) -> String {
    let m = minute_of_day.rem_euclid(24 * 60);
    let (h24, min) = (m / 60, m % 60);
    let h12 = match h24 % 12 {
        0 => 12,
        h => h,
    };
    format!("{}:{:02} {}", h12, min, if h24 < 12 { "AM" } else { "PM" })
}

fn weekday_plural(d: chrono::Weekday) -> &'static str {
    match d {
        chrono::Weekday::Mon => "Mondays",
        chrono::Weekday::Tue => "Tuesdays",
        chrono::Weekday::Wed => "Wednesdays",
        chrono::Weekday::Thu => "Thursdays",
        chrono::Weekday::Fri => "Fridays",
        chrono::Weekday::Sat => "Saturdays",
        chrono::Weekday::Sun => "Sundays",
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// The suggestion (pure)
// ═══════════════════════════════════════════════════════════════════════════

/// What to record, and how. The window's button says "Record <title>".
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UpNext {
    /// "calendar" | "routine" | "call"
    pub source: String,
    /// Identifies this suggestion for "Not now"
    pub key: String,
    pub title: String,
    /// One quiet line under the title: why it is offered
    pub detail: String,
    pub notebook: Option<String>,
    /// "meeting" | "class" | "personal"
    pub recording_kind: String,
    /// "15" | "30" | "60" | "90" | "none"
    pub duration: String,
}

/// A calendar event that is on now or about to start.
#[derive(Debug, Clone, PartialEq)]
pub struct EventNow {
    pub id: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// It has other people on it or a call link, or it was recorded before
    pub meeting_like: bool,
    /// The notebook and type the last recording with this title had
    pub notebook: Option<String>,
    pub kind: Option<RecordingKind>,
    /// A recording of this event already exists
    pub recorded: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signals {
    pub event: Option<EventNow>,
    pub routine: Option<Routine>,
    /// The call app holding the microphone ("Zoom"), once it has held it
    /// long enough and wasn't already on when a recording stopped
    pub call_app: Option<String>,
}

/// The one suggestion for these signals: calendar, then routine, then call.
/// `now_local` and `today` phrase the detail line.
pub fn choose(signals: &Signals, now: DateTime<Utc>, now_local: NaiveDateTime) -> Option<UpNext> {
    if let Some(e) = signals.event.as_ref().filter(|e| e.meeting_like && !e.recorded && e.end > now) {
        let end_local = e.end.with_timezone(&Local).naive_local();
        let detail = if e.start > now {
            let mins = ((e.start - now).num_seconds() + 59) / 60;
            format!("On your calendar, starts in {} min", mins.max(1))
        } else {
            format!("On your calendar until {}", clock_label(minute_of_day(end_local)))
        };
        return Some(UpNext {
            source: "calendar".into(),
            key: format!("calendar:{}:{}", e.id, e.start.timestamp()),
            title: e.title.trim().to_string(),
            detail,
            notebook: e.notebook.clone(),
            recording_kind: e.kind.unwrap_or(RecordingKind::Meeting).as_str().to_string(),
            // It stops when the event is over (meeting_end.rs)
            duration: "none".into(),
        });
    }
    if let Some(r) = &signals.routine {
        return Some(UpNext {
            source: "routine".into(),
            key: format!("routine:{}", r.notebook.to_lowercase()),
            title: r.notebook.clone(),
            detail: format!(
                "You usually record this on {} around {}",
                weekday_plural(now_local.weekday()),
                clock_label(r.usual_minute)
            ),
            notebook: Some(r.notebook.clone()),
            recording_kind: r.kind.as_str().to_string(),
            duration: r.duration.to_string(),
        });
    }
    signals.call_app.as_ref().map(|app| UpNext {
        source: "call".into(),
        key: format!("call:{}", app.to_lowercase()),
        title: "this call".into(),
        detail: format!("{} is using the microphone", app),
        notebook: None,
        recording_kind: RecordingKind::Meeting.as_str().to_string(),
        duration: "none".into(),
    })
}

/// The notification for a suggestion: (title, body). Names only (a calendar
/// title, a notebook, an app), never anything that was said.
pub fn reminder(s: &UpNext) -> (String, String) {
    let title = match s.source.as_str() {
        "calendar" => format!("{} is starting", s.title),
        "routine" => format!("{} usually starts about now", s.title),
        _ => s.detail.clone(),
    };
    (title, "Click to record it.".to_string())
}

/// The notification when Record by itself began a recording.
pub fn started_notice(s: &UpNext) -> (String, String) {
    let what = if s.source == "call" { "this call".to_string() } else { s.title.clone() };
    (format!("Recording {}", what), "Tell people you're recording. Click to open noFriction.".to_string())
}

// ── "Not now" ───────────────────────────────────────────────────────────

/// The stored dismissals that are still for `today` ("YYYY-MM-DD").
pub fn dismissed_today(stored: Option<&str>, today: &str) -> Vec<String> {
    stored
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.rsplit_once('|'))
        .filter(|(_, day)| *day == today)
        .map(|(key, _)| key.to_string())
        .collect()
}

/// `stored` with `key` dismissed for `today`; other days' lines are dropped.
pub fn dismiss(stored: Option<&str>, key: &str, today: &str) -> String {
    let mut keys = dismissed_today(stored, today);
    if !keys.iter().any(|k| k == key) {
        keys.push(key.to_string());
    }
    keys.iter().map(|k| format!("{}|{}", k, today)).collect::<Vec<_>>().join("\n")
}

// ── The call signal over time ───────────────────────────────────────────

/// Tracks whether a call app's hold on the microphone is a new call the
/// user hasn't been offered (or declined) yet.
#[derive(Debug, Default)]
pub struct CallTracker {
    /// When the current hold was first seen
    since: Option<std::time::Instant>,
    /// This call was on while a recording ran, or the user said Not now:
    /// say nothing until the microphone is released
    blocked: bool,
}

impl CallTracker {
    /// Feed one sample; returns the call app to offer, if any.
    pub fn tick(&mut self, apps: &[String], recording: bool, now: std::time::Instant) -> Option<String> {
        let Some(first) = apps.first() else {
            self.since = None;
            self.blocked = false;
            return None;
        };
        if recording {
            self.blocked = true;
            self.since = None;
            return None;
        }
        if self.blocked {
            return None;
        }
        let since = *self.since.get_or_insert(now);
        (now.duration_since(since).as_secs() >= CALL_HOLD_SECS).then(|| first.clone())
    }

    /// The user said Not now to this call.
    pub fn dismiss(&mut self) {
        self.blocked = true;
        self.since = None;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Reading the signals, and the monitor
// ═══════════════════════════════════════════════════════════════════════════

pub mod live {
    use super::*;
    use crate::AppState;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use sqlx::Row;
    use tauri::{AppHandle, Emitter, Manager};

    static CALLS: Lazy<Mutex<CallTracker>> = Lazy::new(|| Mutex::new(CallTracker::default()));
    /// The call app to offer, from the monitor's last sample
    static CALL_APP: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
    /// What the window and the tray show now
    static CURRENT: Lazy<Mutex<Option<UpNext>>> = Lazy::new(|| Mutex::new(None));

    pub fn current() -> Option<UpNext> {
        CURRENT.lock().clone()
    }

    fn is_recording(state: &AppState) -> bool {
        state.capture_engine.read().is_recording()
    }

    async fn setting_on(state: &AppState, key: &str, default: bool) -> bool {
        match state.settings.get(key).await {
            Ok(Some(v)) => v == "true",
            _ => default,
        }
    }

    fn today() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    async fn past_recordings(state: &AppState) -> Vec<Past> {
        let since = (Utc::now() - Duration::days(LOOKBACK_DAYS + 1)).to_rfc3339();
        let rows = sqlx::query(
            "SELECT class_name, recording_kind, started_at, duration_seconds FROM meetings \
             WHERE class_name IS NOT NULL AND TRIM(class_name) != '' AND started_at >= ? ORDER BY started_at DESC",
        )
        .bind(since)
        .fetch_all(state.database.pool())
        .await
        .unwrap_or_default();
        rows.iter()
            .filter_map(|r| {
                let started = DateTime::parse_from_rfc3339(&r.get::<String, _>("started_at")).ok()?;
                Some(Past {
                    notebook: r.get("class_name"),
                    kind: RecordingKind::from_stored(r.get::<Option<String>, _>("recording_kind").as_deref()),
                    started: started.with_timezone(&Local).naive_local(),
                    duration_seconds: r.get("duration_seconds"),
                })
            })
            .collect()
    }

    /// The calendar event on now or about to start, if Calendar access was
    /// already granted. Never asks for it.
    async fn event_now(state: &AppState) -> Option<EventNow> {
        use crate::calendar_client::{CalendarAccessStatus, CalendarClient};
        if CalendarClient::check_access() != CalendarAccessStatus::Authorized {
            return None;
        }
        let client = state.calendar_client.clone();
        // EventKit off the async threads (it can take a moment the first time)
        let event = tokio::task::spawn_blocking(move || client.read().get_current_event()).await.ok()??;
        if event.is_all_day || event.title.trim().is_empty() {
            return None;
        }
        let pool = state.database.pool();
        let last = sqlx::query(
            "SELECT class_name, recording_kind FROM meetings WHERE title = ? COLLATE NOCASE ORDER BY started_at DESC LIMIT 1",
        )
        .bind(event.title.trim())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let recorded: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM meetings WHERE calendar_event_id = ? AND started_at >= ? AND started_at <= ? LIMIT 1",
        )
        .bind(&event.event_id)
        .bind((event.start_time - Duration::minutes(30)).to_rfc3339())
        .bind(event.end_time.to_rfc3339())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let others = event.participants.iter().any(|p| !p.is_self) || event.attendees.len() > 1;
        Some(EventNow {
            id: event.event_id.clone(),
            title: event.title.clone(),
            start: event.start_time,
            end: event.end_time,
            meeting_like: others || event.meeting_url.is_some() || last.is_some(),
            notebook: last.as_ref().and_then(|r| r.get::<Option<String>, _>("class_name")).filter(|n| !n.trim().is_empty()),
            kind: last.as_ref().map(|r| RecordingKind::from_stored(r.get::<Option<String>, _>("recording_kind").as_deref())),
            recorded: recorded.is_some(),
        })
    }

    fn call_apps_now() -> Vec<String> {
        let own = std::process::id() as i32;
        crate::core_audio::audio_processes()
            .map(|p| crate::meeting_end::active_call_apps(&p, own).into_iter().collect())
            .unwrap_or_default()
    }

    /// Sample who holds the microphone and advance the call tracker (the
    /// monitor only: the tracker counts time between samples).
    async fn sample_calls(state: &AppState) {
        let recording = is_recording(state);
        let apps = tokio::task::spawn_blocking(call_apps_now).await.unwrap_or_default();
        let offer = CALLS.lock().tick(&apps, recording, std::time::Instant::now());
        *CALL_APP.lock() = offer;
    }

    /// Work out the suggestion now, from the calendar, the recordings and
    /// the last call sample.
    async fn compute(state: &AppState) -> Option<UpNext> {
        let recording = is_recording(state);
        let call_app = CALL_APP.lock().clone();
        if recording {
            return None;
        }
        let now_local = Local::now().naive_local();
        let signals = Signals {
            event: event_now(state).await,
            routine: routine(&past_recordings(state).await, now_local),
            call_app,
        };
        let dismissed = dismissed_today(state.settings.get(DISMISSED_SETTING).await.ok().flatten().as_deref(), &today());
        // A dismissed calendar event or routine doesn't hide what is behind it
        let mut s = signals.clone();
        loop {
            let pick = choose(&s, Utc::now(), now_local)?;
            if !dismissed.contains(&pick.key) {
                return Some(pick);
            }
            match pick.source.as_str() {
                "calendar" => s.event = None,
                "routine" => s.routine = None,
                _ => return None,
            }
        }
    }

    fn window_in_front(app: &AppHandle) -> bool {
        app.get_webview_window("main")
            .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
            .unwrap_or(false)
    }

    /// The tray's first item: "Record BIO 101" while something is up next.
    async fn refresh_tray(app: &AppHandle, s: Option<&UpNext>) {
        match s {
            Some(s) => crate::tray_builder::set_start_item_text(app, &format!("Record {}", s.title)),
            None => crate::timed_recording::refresh_start_label(app).await,
        }
    }

    /// Watch for something starting. Call once from setup.
    pub fn start_monitor(app: AppHandle) {
        tauri::async_runtime::spawn(async move {
            // Let startup finish (database, permissions, the tray)
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            let mut last_key: Option<String> = None;
            loop {
                if let Some(state) = app.try_state::<AppState>() {
                    sample_calls(&state).await;
                    let next = compute(&state).await;
                    let key = next.as_ref().map(|s| s.key.clone());
                    if key != last_key {
                        *CURRENT.lock() = next.clone();
                        let _ = app.emit(CHANGED_EVENT, &next);
                        refresh_tray(&app, next.as_ref()).await;
                        if let Some(s) = &next {
                            if setting_on(&state, AUTO_START_SETTING, false).await {
                                log::info!("Up next: starting by itself ({})", s.source);
                                let _ = app.emit(START_EVENT, s);
                                let (t, b) = started_notice(s);
                                crate::notifications::post(&app, &t, &b);
                            } else if !window_in_front(&app) && setting_on(&state, REMIND_SETTING, true).await {
                                let (t, b) = reminder(s);
                                crate::notifications::post(&app, &t, &b);
                            }
                        }
                        last_key = key;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }

    /// Tray: start what is up next, if anything is. Returns false when
    /// there is no suggestion (the caller starts the remembered choice).
    pub fn start_from_tray(app: &AppHandle) -> bool {
        match current() {
            Some(s) => app.emit(START_EVENT, &s).is_ok(),
            None => false,
        }
    }

    /// What to record now, or null (the idle Record screen asks on mount).
    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_up_next(state: tauri::State<'_, AppState>) -> Result<Option<UpNext>, String> {
        Ok(compute(&state).await)
    }

    /// "Not now": hide this suggestion until tomorrow (a call: until the
    /// microphone is released).
    #[tauri::command(rename_all = "camelCase")]
    pub async fn dismiss_up_next(app: AppHandle, state: tauri::State<'_, AppState>, key: String) -> Result<(), String> {
        if key.starts_with("call:") {
            CALLS.lock().dismiss();
            *CALL_APP.lock() = None;
        } else {
            let stored = state.settings.get(DISMISSED_SETTING).await.ok().flatten();
            state
                .settings
                .set(DISMISSED_SETTING, &dismiss(stored.as_deref(), &key, &today()))
                .await
                .map_err(|e| format!("Couldn't save that: {}", e))?;
        }
        let next = compute(&state).await;
        *CURRENT.lock() = next.clone();
        let _ = app.emit(CHANGED_EVENT, &next);
        refresh_tray(&app, next.as_ref()).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};
    use RecordingKind::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    fn rec(notebook: &str, kind: RecordingKind, started: NaiveDateTime, minutes: i64) -> Past {
        Past { notebook: notebook.into(), kind, started, duration_seconds: Some(minutes * 60) }
    }

    // 2026-11-11 is a Wednesday
    const WED: (i32, u32, u32) = (2026, 11, 11);

    fn bio_wednesdays() -> Vec<Past> {
        vec![
            rec("BIO 101", Class, at(2026, 11, 4, 9, 2), 50),
            rec("BIO 101", Class, at(2026, 10, 28, 9, 0), 49),
            rec("BIO 101", Class, at(2026, 10, 21, 8, 58), 52),
        ]
    }

    #[test]
    fn a_notebook_recorded_at_this_time_on_this_weekday_is_a_routine() {
        let past = bio_wednesdays();
        let r = routine(&past, at(WED.0, WED.1, WED.2, 9, 0)).unwrap();
        assert_eq!((r.notebook.as_str(), r.kind, r.duration, r.matches), ("BIO 101", Class, "60", 3));
        assert_eq!(clock_label(r.usual_minute), "9:00 AM");
        // From ten minutes before…
        assert!(routine(&past, at(WED.0, WED.1, WED.2, 8, 52)).is_some());
        assert!(routine(&past, at(WED.0, WED.1, WED.2, 8, 40)).is_none());
        // …to twenty-five after (two of the three still match at 9:25)
        assert_eq!(routine(&past, at(WED.0, WED.1, WED.2, 9, 25)).unwrap().matches, 2);
        assert!(routine(&past, at(WED.0, WED.1, WED.2, 9, 40)).is_none());
        // Another weekday at the same time: nothing
        assert!(routine(&past, at(2026, 11, 12, 9, 0)).is_none());
    }

    #[test]
    fn once_is_not_a_habit_and_an_old_habit_fades() {
        let once = vec![rec("BIO 101", Class, at(2026, 11, 4, 9, 0), 50)];
        assert!(routine(&once, at(WED.0, WED.1, WED.2, 9, 0)).is_none());
        // Two matches, both older than six weeks
        let old = vec![rec("BIO 101", Class, at(2026, 9, 23, 9, 0), 50), rec("BIO 101", Class, at(2026, 9, 16, 9, 0), 50)];
        assert!(routine(&old, at(WED.0, WED.1, WED.2, 9, 0)).is_none());
        // Exactly 42 days back still counts
        let edge = vec![rec("BIO 101", Class, at(2026, 9, 30, 9, 0), 50), rec("BIO 101", Class, at(2026, 11, 4, 9, 0), 50)];
        assert!(routine(&edge, at(WED.0, WED.1, WED.2, 9, 0)).is_some());
    }

    #[test]
    fn already_recorded_today_means_no_suggestion() {
        let mut past = bio_wednesdays();
        past.push(rec("bio 101", Class, at(WED.0, WED.1, WED.2, 8, 55), 3));
        assert!(routine(&past, at(WED.0, WED.1, WED.2, 9, 0)).is_none(), "the name matches ignoring case");
    }

    #[test]
    fn the_usual_type_spelling_and_length_come_from_the_recordings() {
        // The newest recording is spelled differently and was changed to Personal
        let mut past = bio_wednesdays();
        past.push(rec("Bio 101", Personal, at(2026, 11, 9, 14, 0), 20));
        let r = routine(&past, at(WED.0, WED.1, WED.2, 9, 0)).unwrap();
        assert_eq!((r.notebook.as_str(), r.kind), ("Bio 101", Personal));
        assert_eq!(r.duration, "60", "the median of the matching recordings (50 min), rounded up");

        assert_eq!(length_choice(Some(12)), "15");
        assert_eq!(length_choice(Some(15)), "15");
        assert_eq!(length_choice(Some(31)), "60");
        assert_eq!(length_choice(Some(75)), "90");
        assert_eq!(length_choice(Some(91)), "none");
        assert_eq!(length_choice(Some(0)), "none");
        assert_eq!(length_choice(None), "none");
        // No lengths stored: no limit
        let unknown: Vec<Past> = bio_wednesdays().into_iter().map(|p| Past { duration_seconds: None, ..p }).collect();
        assert_eq!(routine(&unknown, at(WED.0, WED.1, WED.2, 9, 0)).unwrap().duration, "none");
    }

    #[test]
    fn two_routines_at_once_the_stronger_then_the_more_recent_wins() {
        let mut past = bio_wednesdays();
        past.extend([
            rec("Standup", Meeting, at(2026, 11, 4, 9, 5), 15),
            rec("Standup", Meeting, at(2026, 10, 28, 9, 5), 15),
        ]);
        assert_eq!(routine(&past, at(WED.0, WED.1, WED.2, 9, 0)).unwrap().notebook, "BIO 101", "three matches beat two");
        // Equal matches: the one recorded most recently
        let tie = vec![
            rec("BIO 101", Class, at(2026, 10, 28, 9, 0), 50),
            rec("BIO 101", Class, at(2026, 10, 21, 9, 0), 50),
            rec("Standup", Meeting, at(2026, 11, 4, 9, 5), 15),
            rec("Standup", Meeting, at(2026, 10, 28, 9, 5), 15),
        ];
        let r = routine(&tie, at(WED.0, WED.1, WED.2, 9, 0)).unwrap();
        assert_eq!((r.notebook.as_str(), r.duration), ("Standup", "15"));
        // Recordings without a notebook are never a routine
        let none = vec![rec("  ", Meeting, at(2026, 11, 4, 9, 0), 30), rec("", Meeting, at(2026, 10, 28, 9, 0), 30)];
        assert!(routine(&none, at(WED.0, WED.1, WED.2, 9, 0)).is_none());
    }

    fn event(meeting_like: bool, recorded: bool, start_in_min: i64) -> (EventNow, DateTime<Utc>) {
        let now = Utc.with_ymd_and_hms(2026, 11, 11, 15, 0, 0).unwrap();
        let start = now + Duration::minutes(start_in_min);
        (
            EventNow {
                id: "evt-1".into(),
                title: " Acme project sync ".into(),
                start,
                end: start + Duration::minutes(30),
                meeting_like,
                notebook: Some("Acme project".into()),
                kind: None,
                recorded,
            },
            now,
        )
    }

    #[test]
    fn the_calendar_comes_first_then_the_routine_then_a_call() {
        let (e, now) = event(true, false, 5);
        let local = at(WED.0, WED.1, WED.2, 9, 0);
        let r = routine(&bio_wednesdays(), local);
        let all = Signals { event: Some(e.clone()), routine: r.clone(), call_app: Some("Zoom".into()) };
        let s = choose(&all, now, local).unwrap();
        assert_eq!((s.source.as_str(), s.title.as_str(), s.detail.as_str()), ("calendar", "Acme project sync", "On your calendar, starts in 5 min"));
        assert_eq!((s.notebook.as_deref(), s.recording_kind.as_str(), s.duration.as_str()), (Some("Acme project"), "meeting", "none"));
        assert_eq!(s.key, format!("calendar:evt-1:{}", e.start.timestamp()));

        let s = choose(&Signals { event: None, ..all.clone() }, now, local).unwrap();
        assert_eq!((s.source.as_str(), s.title.as_str(), s.key.as_str()), ("routine", "BIO 101", "routine:bio 101"));
        assert_eq!(s.detail, "You usually record this on Wednesdays around 9:00 AM");
        assert_eq!((s.recording_kind.as_str(), s.duration.as_str()), ("class", "60"));

        let s = choose(&Signals { call_app: Some("Zoom".into()), ..Default::default() }, now, local).unwrap();
        assert_eq!((s.source.as_str(), s.title.as_str(), s.detail.as_str(), s.key.as_str()), ("call", "this call", "Zoom is using the microphone", "call:zoom"));
        assert_eq!(choose(&Signals::default(), now, local), None);
    }

    #[test]
    fn a_calendar_event_is_offered_only_when_it_is_a_meeting_not_yet_recorded_and_not_over() {
        let local = at(WED.0, WED.1, WED.2, 9, 0);
        // "Lunch": nobody else on it, no link, never recorded
        let (e, now) = event(false, false, -5);
        assert_eq!(choose(&Signals { event: Some(e), ..Default::default() }, now, local), None);
        // Already being or been recorded
        let (e, now) = event(true, true, -5);
        assert_eq!(choose(&Signals { event: Some(e), ..Default::default() }, now, local), None);
        // Over
        let (e, now) = event(true, false, -45);
        assert_eq!(choose(&Signals { event: Some(e), ..Default::default() }, now, local), None);
        // On now
        let (e, now) = event(true, false, -5);
        let s = choose(&Signals { event: Some(e), ..Default::default() }, now, local).unwrap();
        assert!(s.detail.starts_with("On your calendar until "), "{}", s.detail);
    }

    #[test]
    fn reminders_name_the_thing_and_never_more() {
        let local = at(WED.0, WED.1, WED.2, 9, 0);
        let (e, now) = event(true, false, 0);
        let cal = choose(&Signals { event: Some(e), ..Default::default() }, now, local).unwrap();
        assert_eq!(reminder(&cal), ("Acme project sync is starting".to_string(), "Click to record it.".to_string()));
        let rt = choose(&Signals { routine: routine(&bio_wednesdays(), local), ..Default::default() }, now, local).unwrap();
        assert_eq!(reminder(&rt).0, "BIO 101 usually starts about now");
        let call = choose(&Signals { call_app: Some("Zoom".into()), ..Default::default() }, now, local).unwrap();
        assert_eq!(reminder(&call).0, "Zoom is using the microphone");
        assert_eq!(started_notice(&call).0, "Recording this call");
        assert_eq!(started_notice(&rt), ("Recording BIO 101".to_string(), "Tell people you're recording. Click to open noFriction.".to_string()));
        assert_eq!(clock_label(0), "12:00 AM");
        assert_eq!(clock_label(13 * 60 + 5), "1:05 PM");
        assert_eq!(clock_label(12 * 60), "12:00 PM");
    }

    #[test]
    fn not_now_lasts_until_the_next_day() {
        let stored = dismiss(None, "routine:bio 101", "2026-11-11");
        assert_eq!(stored, "routine:bio 101|2026-11-11");
        let stored = dismiss(Some(&stored), "calendar:evt|1:99", "2026-11-11");
        assert_eq!(dismissed_today(Some(&stored), "2026-11-11"), vec!["routine:bio 101", "calendar:evt|1:99"], "a key may contain the separator");
        // Dismissing twice keeps one line
        assert_eq!(dismiss(Some(&stored), "routine:bio 101", "2026-11-11"), stored);
        // The next day nothing is hidden, and yesterday's lines are dropped on the next write
        assert!(dismissed_today(Some(&stored), "2026-11-12").is_empty());
        assert_eq!(dismiss(Some(&stored), "routine:chem", "2026-11-12"), "routine:chem|2026-11-12");
        assert!(dismissed_today(Some("garbage\n\n|"), "2026-11-11").is_empty());
    }

    #[test]
    fn a_call_is_offered_once_it_has_held_the_microphone_and_never_twice() {
        use std::time::{Duration as D, Instant};
        let t0 = Instant::now();
        let zoom = vec!["Zoom".to_string()];
        let mut c = CallTracker::default();
        assert_eq!(c.tick(&[], false, t0), None);
        // A blip isn't a call
        assert_eq!(c.tick(&zoom, false, t0), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(5)), None);
        assert_eq!(c.tick(&[], false, t0 + D::from_secs(6)), None);
        // Held long enough
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(20)), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(20 + CALL_HOLD_SECS)), Some("Zoom".to_string()));
        // Not now: quiet until the microphone is released, then a new call counts again
        c.dismiss();
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(60)), None);
        assert_eq!(c.tick(&[], false, t0 + D::from_secs(70)), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(80)), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(80 + CALL_HOLD_SECS)), Some("Zoom".to_string()));

        // A call that was on while recording is never offered when the recording stops
        let mut c = CallTracker::default();
        assert_eq!(c.tick(&zoom, true, t0), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(100)), None, "the user just stopped: don't start again");
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(200)), None);
        assert_eq!(c.tick(&[], false, t0 + D::from_secs(210)), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(220)), None);
        assert_eq!(c.tick(&zoom, false, t0 + D::from_secs(220 + CALL_HOLD_SECS)), Some("Zoom".to_string()));
    }
}
