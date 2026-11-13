// noFriction - the quick-note panel: type a note while another app is in
// front (docs/design/FADELL_V2.md §10).
//
// ⌃⌥⌘N while a recording is on opens one field over whatever is in front
// (slides, a call, a browser). Return keeps the line at that moment and the
// panel is gone; Esc, a click elsewhere or the key again closes it. The app
// in front stays in front the whole time: the panel takes the keyboard
// without bringing noFriction forward (quick_note/panel.rs).
//
// It is the note field under the live transcript with another way in, and
// nothing else:
//   - A line is kept by `markers::commands::keep_note`, the same call the
//     in-app field makes, so it is a mark of kind `note` on the recording
//     the panel was opened in, and the late-note rule applies unchanged.
//   - Nothing typed is stored until Return. Closing without Return leaves
//     the line in the field (memory only) while the same recording is on; a
//     different recording, or none, starts empty.
//   - After Return the field takes no keys until the line is kept or not.
//     The save can land after the panel was closed or reopened, so it only
//     touches what is still its own (`settle`).
//   - With nothing recording the key shows a notice for a moment and takes
//     no keys. It never starts a recording.
//   - No AI, no network, no setting. Typed notes are free, so this is.

#[cfg(target_os = "macos")]
pub mod panel;

use std::sync::atomic::{AtomicI64, Ordering};
use tauri::AppHandle;

/// How the key is written for people
pub const KEY_LABEL: &str = "⌃⌥⌘N";
/// The same key as a menu accelerator (File → Type a Note)
pub const MENU_ACCELERATOR: &str = "Ctrl+Alt+Cmd+N";

/// The line under the field
const HINT: &str = "Return keeps it at this moment";
const NOT_RECORDING: &str = "Nothing is recording";
const NOT_RECORDING_HINT: &str = "Start a recording, then press ⌃⌥⌘N to type a note.";
/// How long the "Nothing is recording" notice stays
#[cfg(target_os = "macos")]
const NOTICE_MS: u64 = 2500;
/// Two asks this close together are one key press (the global key and the
/// menu accelerator can both fire for it)
const DEBOUNCE_MS: i64 = 250;
/// Say how much room is left once the line is this close to the limit
const COUNT_FROM: usize = 100;

/// ⌃⌥⌘N, beside ⌃⌥⌘M (mark). The same reasoning as `markers::hotkey`: the
/// app's own keys are ⌘-based, macOS uses ⌥⌘N and ⇧⌘N in Finder and
/// VoiceOver uses ⌃⌥ + letter; ⌃⌥⌘N is unused.
pub fn hotkey() -> tauri_plugin_global_shortcut::Shortcut {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER), Code::KeyN)
}

/// The line under the field: which recording the note goes to.
pub fn caption(title: Option<&str>) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("{} · {}", t, HINT),
        None => HINT.to_string(),
    }
}

/// What the line under the field says instead once the note is near or
/// past its limit, and whether that is an error.
pub fn count_caption(text: &str) -> Option<(String, bool)> {
    let max = crate::markers::MAX_TYPED_NOTE_CHARS;
    let length = text.chars().count();
    if length > max {
        Some((format!("{} over. Keep a note under {} characters.", length - max, max), true))
    } else if max - length <= COUNT_FROM {
        Some((format!("{} left", max - length), false))
    } else {
        None
    }
}

/// Whether an ask at `now_ms` is a new key press, not the echo of the last.
fn accept(last: &AtomicI64, now_ms: i64) -> bool {
    let before = last.load(Ordering::SeqCst);
    if now_ms >= before && now_ms - before < DEBOUNCE_MS {
        return false;
    }
    last.store(now_ms, Ordering::SeqCst);
    true
}

/// Where a failure to keep a line is said.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Tell {
    Nothing,
    /// In the panel, now
    Now,
    /// The panel is closed: the next time it opens in this recording
    NextOpen,
    /// The field no longer holds the line; it can only be logged
    Lost,
}

/// What the panel does when it hears back about a line.
#[derive(Debug, PartialEq, Clone, Copy)]
struct Settle {
    /// Empty the field
    clear: bool,
    /// Close the panel
    hide: bool,
    tell: Tell,
}

/// The save can land after the panel was closed, or closed and opened
/// again, so what it may touch depends on what is there now:
/// `same_showing` is "still the showing Return was pressed in", `open` is
/// "on screen", `holds_line` is "the field still has exactly that line".
fn settle(kept: bool, same_showing: bool, open: bool, holds_line: bool) -> Settle {
    if kept {
        // A kept line must not stay behind as a draft; a later showing stays open
        Settle { clear: holds_line, hide: same_showing && open, tell: Tell::Nothing }
    } else {
        let tell = match (holds_line, open) {
            (true, true) => Tell::Now,
            (true, false) => Tell::NextOpen,
            (false, _) => Tell::Lost,
        };
        Settle { clear: false, hide: false, tell }
    }
}

static LAST_ASK: AtomicI64 = AtomicI64::new(0);

/// Register the global ⌃⌥⌘N at launch. If another app owns the key the app
/// still starts; File → Type a Note and the tray item still open the panel.
pub fn init(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    mac::init(app);
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let result = app.global_shortcut().on_shortcut(hotkey(), |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle(app);
        }
    });
    match result {
        Ok(()) => log::info!("Quick-note hotkey {} registered", KEY_LABEL),
        Err(e) => log::warn!("Quick-note hotkey {} not available ({}); File → Type a Note still works", KEY_LABEL, e),
    }
}

/// The key, File → Type a Note and the tray item: open the panel, or close
/// it if it is open.
pub fn toggle(app: &AppHandle) {
    if !accept(&LAST_ASK, chrono::Utc::now().timestamp_millis()) {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        if let Err(e) = app.run_on_main_thread(move || mac::toggle(&handle)) {
            log::warn!("Quick note: could not reach the main thread: {}", e);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{caption, count_caption, panel, settle, Tell, NOTICE_MS, NOT_RECORDING, NOT_RECORDING_HINT};
    use crate::markers;
    use crate::AppState;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::OnceLock;
    use tauri::{AppHandle, Manager};

    static APP: OnceLock<AppHandle> = OnceLock::new();

    /// The recording the panel was last opened in
    struct Session {
        meeting: Option<String>,
        /// Its name, once read
        title: Option<String>,
        /// A line that couldn't be kept while the panel was closed: said at the next open
        failure: Option<String>,
    }
    static SESSION: Mutex<Session> = Mutex::new(Session { meeting: None, title: None, failure: None });
    /// A line is on its way to being kept. The field takes no keys until it
    /// is settled, so a second Return can't be a second note. Main thread only.
    static SAVING: AtomicBool = AtomicBool::new(false);
    /// Counts every time the panel is shown, so a timer or a save that
    /// lands later can tell whether it is still looking at its own showing
    static SHOWN: AtomicU64 = AtomicU64::new(0);

    pub fn init(app: &AppHandle) {
        let _ = APP.set(app.clone());
        panel::set_handlers(panel::Handlers { keep: on_keep, changed: on_changed, closed: || {} });
    }

    /// On the main thread.
    pub fn toggle(app: &AppHandle) {
        if panel::is_open() {
            panel::hide();
            return;
        }
        let Some(state) = app.try_state::<AppState>() else { return };
        let shown = SHOWN.fetch_add(1, Ordering::SeqCst) + 1;
        match markers::commands::recording_meeting(&state) {
            Some(meeting) => {
                let (same, title, failure) = {
                    let mut session = SESSION.lock();
                    let same = session.meeting.as_deref() == Some(meeting.as_str());
                    if !same {
                        session.meeting = Some(meeting.clone());
                        session.title = None;
                        session.failure = None;
                    }
                    (same, session.title.clone(), session.failure.take())
                };
                // On screen first: the next key pressed must land in it
                panel::show(&caption(title.as_deref()), same);
                if SAVING.load(Ordering::SeqCst) {
                    // Reopened before the last line was settled
                    panel::set_busy(true);
                } else if let Some(failure) = failure {
                    panel::set_caption(&failure, true);
                } else if same {
                    // A draft near the limit says so again
                    let draft = panel::text();
                    if !draft.is_empty() {
                        on_changed(draft);
                    }
                }
                if title.is_none() {
                    read_title(app.clone(), meeting, shown);
                }
            }
            None => {
                {
                    let mut session = SESSION.lock();
                    session.meeting = None;
                    session.title = None;
                    session.failure = None;
                }
                panel::show_notice(NOT_RECORDING, NOT_RECORDING_HINT);
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(NOTICE_MS)).await;
                    let _ = handle.run_on_main_thread(move || {
                        if SHOWN.load(Ordering::SeqCst) == shown {
                            panel::hide();
                        }
                    });
                });
            }
        }
    }

    /// The recording's name, for the line under the field.
    fn read_title(app: AppHandle, meeting: String, shown: u64) {
        tauri::async_runtime::spawn(async move {
            let title: Option<String> = match app.try_state::<AppState>() {
                Some(state) => sqlx::query_scalar("SELECT title FROM meetings WHERE id = ?")
                    .bind(&meeting)
                    .fetch_optional(state.database.pool())
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            let Some(title) = title else { return };
            let _ = app.run_on_main_thread(move || {
                {
                    let mut session = SESSION.lock();
                    if session.meeting.as_deref() != Some(meeting.as_str()) {
                        return;
                    }
                    session.title = Some(title.clone());
                }
                // Still the same showing, and the line says nothing more pressing
                // (an error, or how much room is left)
                let plain = panel::probe().is_some_and(|p| p.caption == caption(None));
                if SHOWN.load(Ordering::SeqCst) == shown && panel::is_open() && plain {
                    panel::set_caption(&caption(Some(&title)), false);
                }
            });
        });
    }

    /// Typing: how much room is left near the limit, else which recording.
    /// Either way an earlier error is gone.
    fn on_changed(text: String) {
        match count_caption(&text) {
            Some((line, over)) => panel::set_caption(&line, over),
            None => {
                let title = SESSION.lock().title.clone();
                panel::set_caption(&caption(title.as_deref()), false);
            }
        }
    }

    /// Return: keep the line on the recording the panel was opened in. The
    /// panel closes when the line is kept; if it can't be, it stays with
    /// the line and the reason.
    fn on_keep(text: String) {
        let Some(app) = APP.get().cloned() else { return };
        if text.trim().is_empty() {
            panel::hide();
            return;
        }
        let line = match markers::clean_typed_note(&text) {
            Ok(line) => line,
            Err(e) => {
                panel::set_caption(&format!("{}.", e), true);
                return;
            }
        };
        if SAVING.swap(true, Ordering::SeqCst) {
            return;
        }
        // Nothing more is typed until this line is settled
        panel::set_busy(true);
        // The moment, the recording and the showing are taken now, not when the save lands
        let meeting = SESSION.lock().meeting.clone();
        let at = chrono::Utc::now().to_rfc3339();
        let shown = SHOWN.load(Ordering::SeqCst);
        tauri::async_runtime::spawn(async move {
            let result = match app.try_state::<AppState>() {
                Some(state) => markers::commands::keep_note(&app, &state, &line, meeting.as_deref(), Some(&at))
                    .await
                    .map(|_| ()),
                None => Err("noFriction is still starting".to_string()),
            };
            let _ = app.run_on_main_thread(move || settle_on_main(result, &text, meeting.as_deref(), shown));
        });
    }

    /// The save landed. The panel may have been closed, or closed and
    /// opened again, since Return (`settle`).
    fn settle_on_main(result: Result<(), String>, typed: &str, meeting: Option<&str>, shown: u64) {
        SAVING.store(false, Ordering::SeqCst);
        let open = panel::is_open();
        let same_showing = SHOWN.load(Ordering::SeqCst) == shown;
        let holds_line = panel::text() == typed;
        let plan = settle(result.is_ok(), same_showing, open, holds_line);
        if plan.clear {
            panel::clear();
        }
        if plan.hide {
            panel::hide();
        }
        // The field takes keys again, for the next line or another try
        panel::set_busy(false);
        let failure = match result {
            Ok(()) => {
                if open && !plan.hide {
                    // A later showing: back to which recording it is
                    on_changed(panel::text());
                }
                return;
            }
            Err(e) => format!("Couldn't keep that note: {}", e),
        };
        match plan.tell {
            Tell::Now => panel::set_caption(&failure, true),
            Tell::NextOpen => {
                let mut session = SESSION.lock();
                if session.meeting.as_deref() == meeting {
                    session.failure = Some(failure);
                }
            }
            // Never the line itself
            Tell::Lost => log::warn!("Quick note: a line could not be kept and is no longer in the panel"),
            Tell::Nothing => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_beside_the_mark_key_and_is_not_it() {
        assert_ne!(hotkey(), crate::markers::hotkey());
        assert_ne!(MENU_ACCELERATOR, crate::markers::MENU_ACCELERATOR);
        assert!(MENU_ACCELERATOR.ends_with("+N"));
        assert!(NOT_RECORDING_HINT.contains(KEY_LABEL), "the notice names the key");
    }

    #[test]
    fn the_caption_says_which_recording() {
        assert_eq!(caption(Some("BIO 101")), "BIO 101 · Return keeps it at this moment");
        assert_eq!(caption(Some("  ")), "Return keeps it at this moment");
        assert_eq!(caption(None), "Return keeps it at this moment");
    }

    #[test]
    fn room_left_is_said_near_the_limit_and_over_it_is_an_error() {
        let max = crate::markers::MAX_TYPED_NOTE_CHARS;
        assert_eq!(count_caption("ask about proof 2"), None);
        assert_eq!(count_caption(&"x".repeat(max - 101)), None);
        assert_eq!(count_caption(&"x".repeat(max - 100)), Some(("100 left".to_string(), false)));
        assert_eq!(count_caption(&"x".repeat(max)), Some(("0 left".to_string(), false)));
        // Characters, not bytes, like the stored limit
        assert_eq!(count_caption(&"é".repeat(max)), Some(("0 left".to_string(), false)));
        let (line, error) = count_caption(&"x".repeat(max + 3)).unwrap();
        assert!(error);
        assert_eq!(line, format!("3 over. Keep a note under {} characters.", max));
        // What the panel says is what the store enforces
        assert!(crate::markers::clean_typed_note(&"x".repeat(max)).is_ok());
        assert!(crate::markers::clean_typed_note(&"x".repeat(max + 1)).is_err());
    }

    #[test]
    fn a_save_that_lands_late_touches_only_what_is_still_its_own() {
        // The usual case: Return, kept, the panel as it was
        assert_eq!(settle(true, true, true, true), Settle { clear: true, hide: true, tell: Tell::Nothing });
        // Closed before the save landed: the kept line isn't left as a draft
        assert_eq!(settle(true, true, false, true), Settle { clear: true, hide: false, tell: Tell::Nothing });
        // Closed and opened again: that showing stays open
        assert_eq!(settle(true, false, true, true), Settle { clear: true, hide: false, tell: Tell::Nothing });
        // Opened again in another recording (the field was emptied): nothing of it is touched
        assert_eq!(settle(true, false, true, false), Settle { clear: false, hide: false, tell: Tell::Nothing });
        // Not kept: the line stays, and the reason is said where it will be seen
        assert_eq!(settle(false, true, true, true), Settle { clear: false, hide: false, tell: Tell::Now });
        assert_eq!(settle(false, true, false, true), Settle { clear: false, hide: false, tell: Tell::NextOpen });
        assert_eq!(settle(false, false, true, true), Settle { clear: false, hide: false, tell: Tell::Now });
        assert_eq!(settle(false, false, true, false).tell, Tell::Lost);
        // A failure never empties the field or closes the panel
        for same in [true, false] {
            for open in [true, false] {
                for holds in [true, false] {
                    let plan = settle(false, same, open, holds);
                    assert!(!plan.clear && !plan.hide);
                }
            }
        }
    }

    #[test]
    fn one_key_press_is_one_ask() {
        let last = AtomicI64::new(0);
        assert!(accept(&last, 10_000));
        assert!(!accept(&last, 10_100), "the menu accelerator firing for the same press");
        assert!(accept(&last, 10_000 + DEBOUNCE_MS), "the next press");
        // The clock stepping back never locks the key out
        assert!(accept(&last, 5_000));
    }
}
