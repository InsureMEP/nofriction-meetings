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
//   - A note is no AI, no network, no setting. Typed notes are free, so
//     this is.
//
// ⌃⌥⌘A opens the same panel for a question (Ask from any app,
// docs/TOPICS_AND_CHAT.md). Return sends one question about the recording
// that is running to the user's AI, through `chat::ask_recording` (the
// same request, consent and Pro gate as the panel in the app), and gives
// the keyboard back to the app in front. The answer shows on the panel as
// a card that stays up while it is read. Nothing asks by itself.

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

/// The key that opens the same panel for a question (Ask)
pub const ASK_KEY_LABEL: &str = "⌃⌥⌘A";
pub const ASK_MENU_ACCELERATOR: &str = "Ctrl+Alt+Cmd+A";

/// The line under the field while a question is typed
const ASK_HINT: &str = "Return asks your AI · empty asks what you missed";
/// What Return with nothing typed asks: the question people have when they
/// look up from something else
const ASK_DEFAULT: &str = "What did I miss in the last few minutes?";
const ASKING: &str = "Asking your AI…";
/// Under an answer
const ANSWER_HINT: &str = "⌃⌥⌘A asks again · twice closes · the full answer is in noFriction";
/// Return on a card whose recording has since stopped
const RECORDING_ENDED: &str = "That recording has stopped. Ask about it in noFriction.";
/// Return while the last question is still with the AI
const ONE_AT_A_TIME: &str = "Still answering your last question.";
const ASK_NOT_RECORDING_HINT: &str = "Start a recording, then press ⌃⌥⌘A to ask about it.";
/// How long an answer stays up when the user is back in their call
const ANSWER_STAYS_MS: u64 = 120_000;
/// The most an answer card shows; the whole answer is in the app
const CARD_CHARS: usize = 900;

/// What a press of the note key or the Ask key does to the panel.
#[derive(Debug, PartialEq, Clone, Copy)]
enum Press {
    /// Open it for this key; if it is open for the other one, change over
    Open,
    /// An answer is up and the keyboard is elsewhere: take the keyboard
    /// for the next question and leave the answer where it is
    NextQuestion,
    /// Close it (nothing happens if it is closed)
    Close,
}

/// `open`: the panel is on screen. `was_ask`: it was last opened for a
/// question. `ask`: this press is the Ask key. `reading`: an answer card
/// (or a question on its way) for the recording that is still running is
/// on screen without the keyboard. `saving` / `asking`: a note is being
/// kept / a question is with the AI.
fn press(open: bool, was_ask: bool, ask: bool, reading: bool, saving: bool, asking: bool) -> Press {
    if !open {
        // The field still holds the line being kept: it is not a question
        return if saving && ask { Press::Close } else { Press::Open };
    }
    if ask && was_ask && reading && !asking {
        return Press::NextQuestion;
    }
    if ask != was_ask && !saving && !asking {
        return Press::Open;
    }
    Press::Close
}

/// ⌃⌥⌘A: Ask, beside ⌃⌥⌘N (note) and ⌃⌥⌘M (mark).
pub fn ask_hotkey() -> tauri_plugin_global_shortcut::Shortcut {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER), Code::KeyA)
}

/// The line under the field in Ask: which recording the question is about.
pub fn ask_caption(title: Option<&str>) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("{} · {}", t, ASK_HINT),
        None => ASK_HINT.to_string(),
    }
}

/// An answer's Markdown as the plain lines the card shows: no emphasis
/// marks, no source numbers (they can't be opened there), bullets as "•",
/// at most `CARD_CHARS` characters.
pub fn plain_answer(markdown: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in markdown.lines() {
        let mut line = raw.trim().to_string();
        // A code fence is not a line of the answer
        if line.is_empty() || line.starts_with("```") {
            continue;
        }
        // Headings and quotes are just lines here ("#1 priority" and ">50%" are not either)
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
            line = line[hashes..].trim().to_string();
        }
        if let Some(rest) = line.strip_prefix("> ") {
            line = rest.trim().to_string();
        }
        // A bullet with nothing after it is nothing
        if matches!(line.as_str(), "-" | "*" | "+" | ">") {
            continue;
        }
        for bullet in ["- ", "* ", "+ "] {
            if let Some(rest) = line.strip_prefix(bullet) {
                line = format!("• {}", rest.trim());
                break;
            }
        }
        line = line.replace("**", "").replace("__", "").replace('`', "");
        // *emphasis*: a star with a word on one side only. "5 * 3" and "2*3" keep theirs
        let chars: Vec<char> = line.chars().collect();
        line = chars
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                let word = |j: Option<&char>| j.is_some_and(|x| x.is_alphanumeric());
                **c != '*' || word(i.checked_sub(1).and_then(|j| chars.get(j))) == word(chars.get(i + 1))
            })
            .map(|(_, c)| *c)
            .collect();
        // [3] and [1, 2] are sources (shown in the app); [words](link) is its words
        let mut out = String::with_capacity(line.len());
        let mut rest = line.as_str();
        while let Some(open) = rest.find('[') {
            let Some(close) = rest[open..].find(']') else { break };
            let inner = &rest[open + 1..open + close];
            let mut after = &rest[open + close + 1..];
            let link = after.starts_with('(') && after.contains(')');
            if link {
                after = &after[after.find(')').unwrap_or(0) + 1..];
            }
            let cite = !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit() || c == ',' || c == ' ');
            out.push_str(&rest[..open]);
            if cite {
                // The space a source number left before a full stop or comma
                if out.ends_with(' ') && after.chars().next().is_none_or(|c| ".,;:!?)".contains(c)) {
                    out.pop();
                }
            } else if link {
                out.push_str(inner);
            } else {
                out.push_str(&rest[open..open + close + 1]);
            }
            rest = after;
        }
        out.push_str(rest);
        let tidy = out.split_whitespace().collect::<Vec<_>>().join(" ");
        if !tidy.is_empty() && tidy != "•" {
            lines.push(tidy);
        }
    }
    let text = lines.join("\n");
    if text.chars().count() <= CARD_CHARS {
        return text;
    }
    let cut: String = text.chars().take(CARD_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// The app's own code at the front of an error ("AI_UNREACHABLE"), for
/// the log. Anything else may be an endpoint's words: it is not logged.
pub fn error_code(e: &str) -> &str {
    let code = e.split(':').next().unwrap_or("");
    let own = !code.is_empty() && code.len() <= 32 && code.chars().all(|c| c.is_ascii_uppercase() || c == '_');
    if own {
        code
    } else {
        "error"
    }
}

/// Why a question couldn't be answered, in a line for the card. The card
/// is over another app, so it can't show the app's own dialogs: it says
/// where to go.
pub fn ask_error(e: &str) -> String {
    if e.starts_with("CONSENT_REQUIRED") {
        "Your AI needs your permission first. Open noFriction and ask there once.".to_string()
    } else if e.starts_with("PRO_REQUIRED") {
        "Ask is part of noFriction Pro. Open noFriction to see the plans.".to_string()
    } else if e.starts_with("AI_NO_PROVIDER") || e.starts_with("AI_NO_KEY") {
        "Ask needs an AI. Set one up in noFriction under Settings → AI.".to_string()
    } else if e.starts_with("AI_WRONG_KEY") {
        "Your AI didn't accept its key. Check it in noFriction under Settings → AI.".to_string()
    } else if e.starts_with("AI_NO_CREDIT") {
        "Your AI account is out of credit.".to_string()
    } else if e.starts_with("AI_BAD_URL") {
        "Your AI's address isn't right. Check it in noFriction under Settings → AI.".to_string()
    } else if e.starts_with("AI_UNREACHABLE") {
        "Couldn't reach your AI. Try again.".to_string()
    } else if e.starts_with("AI_TRUNCATED") {
        "The answer was cut short. Try again, or ask in noFriction.".to_string()
    } else {
        // An endpoint's own words can be long, and are not for a small card
        "Your AI couldn't answer that. Try again, or ask in noFriction.".to_string()
    }
}

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
    let result = app.global_shortcut().on_shortcut(ask_hotkey(), |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle_ask(app);
        }
    });
    match result {
        Ok(()) => log::info!("Ask hotkey {} registered", ASK_KEY_LABEL),
        Err(e) => log::warn!("Ask hotkey {} not available ({}); File → Ask About This Recording still works", ASK_KEY_LABEL, e),
    }
}

/// The key, File → Type a Note and the tray item: open the panel, or close
/// it if it is open.
pub fn toggle(app: &AppHandle) {
    open(app, false);
}

/// The Ask key, File → Ask About This Recording and the tray item: the same
/// panel, for a question about the recording that is running. The answer
/// shows on the panel, over whatever app is in front. Only ever from this
/// key or those menu items: nothing asks the AI by itself.
pub fn toggle_ask(app: &AppHandle) {
    open(app, true);
}

/// A recording's chat messages were purged: nothing of them stays on the
/// answer card. Safe from any thread, and when the panel was never made.
pub fn answer_purged(meeting_id: &str) {
    #[cfg(target_os = "macos")]
    mac::answer_purged(meeting_id.to_string());
    #[cfg(not(target_os = "macos"))]
    let _ = meeting_id;
}

fn open(app: &AppHandle, ask: bool) {
    if !accept(&LAST_ASK, chrono::Utc::now().timestamp_millis()) {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        if let Err(e) = app.run_on_main_thread(move || mac::toggle(&handle, ask)) {
            log::warn!("Quick note: could not reach the main thread: {}", e);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, ask);
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{
        ask_caption, ask_error, caption, count_caption, error_code, panel, plain_answer, settle, Press, Tell, ANSWER_HINT, ANSWER_STAYS_MS, ASKING,
        ASK_DEFAULT, ASK_NOT_RECORDING_HINT, NOTICE_MS, NOT_RECORDING, NOT_RECORDING_HINT, ONE_AT_A_TIME, RECORDING_ENDED,
    };
    use crate::markers;
    use crate::AppState;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::OnceLock;
    use tauri::{AppHandle, Emitter, Manager};

    static APP: OnceLock<AppHandle> = OnceLock::new();

    /// The recording the panel was last opened in
    struct Session {
        meeting: Option<String>,
        /// Its name, once read
        title: Option<String>,
        /// A line that couldn't be kept while the panel was closed: said at the next open
        failure: Option<String>,
        /// A note that was being typed when the panel was opened for a
        /// question. The field is one; a note and a question are not, so
        /// the note waits here and comes back with the note key.
        note_draft: String,
    }
    static SESSION: Mutex<Session> = Mutex::new(Session { meeting: None, title: None, failure: None, note_draft: String::new() });
    /// A line is on its way to being kept. The field takes no keys until it
    /// is settled, so a second Return can't be a second note. Main thread only.
    static SAVING: AtomicBool = AtomicBool::new(false);
    /// The panel was opened for a question (Ask), not a note. Main thread only.
    static ASK_MODE: AtomicBool = AtomicBool::new(false);
    /// A question is with the AI: one at a time
    static ASKING_NOW: AtomicBool = AtomicBool::new(false);
    /// The recording that question is about
    static ASKING_FOR: Mutex<Option<String>> = Mutex::new(None);
    /// That recording's chat was purged while the question was with the
    /// AI: its answer is not shown
    static PURGED_MEANWHILE: AtomicBool = AtomicBool::new(false);
    /// Counts every change of what the card holds (a question sent, an
    /// answer shown, the next question, closing), so the timer that puts an
    /// answer away can tell whether it is still looking at its own answer
    static ANSWER_SEQ: AtomicU64 = AtomicU64::new(0);
    /// Counts every time the panel is shown, so a timer or a save that
    /// lands later can tell whether it is still looking at its own showing
    static SHOWN: AtomicU64 = AtomicU64::new(0);

    pub fn init(app: &AppHandle) {
        let _ = APP.set(app.clone());
        panel::set_handlers(panel::Handlers { keep: on_keep, changed: on_changed, closed: on_closed });
    }

    /// On the main thread.
    pub fn toggle(app: &AppHandle, ask: bool) {
        let was_ask = ASK_MODE.load(Ordering::SeqCst);
        let open = panel::is_open();
        // An answer card is "being read" only while its recording is still
        // the one running: the next question is about that recording
        let running = app.try_state::<AppState>().and_then(|state| markers::commands::recording_meeting(&state));
        let same_recording = running.is_some() && running == SESSION.lock().meeting;
        let reading = open && was_ask && panel::pinned() && same_recording && panel::probe().is_some_and(|p| !p.key);
        match super::press(open, was_ask, ask, reading, SAVING.load(Ordering::SeqCst), ASKING_NOW.load(Ordering::SeqCst)) {
            Press::Close => {
                panel::hide();
                return;
            }
            Press::NextQuestion => {
                SHOWN.fetch_add(1, Ordering::SeqCst);
                ANSWER_SEQ.fetch_add(1, Ordering::SeqCst);
                let title = SESSION.lock().title.clone();
                // Takes the keyboard; the answer stays until the next one
                panel::show_ask(&ask_caption(title.as_deref()), true);
                return;
            }
            Press::Open => {}
        }
        ASK_MODE.store(ask, Ordering::SeqCst);
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
                        session.note_draft.clear();
                    }
                    // The one field changes hands: a note being typed is put
                    // aside for a question, and put back for the note key. A
                    // question is never left in the field as a note.
                    if ask && !was_ask && same {
                        session.note_draft = panel::text();
                    } else if !ask && was_ask {
                        panel::set_text(&std::mem::take(&mut session.note_draft));
                    }
                    // A note that couldn't be kept is said at the next note, not lost to a question
                    let failure = if ask { None } else { session.failure.take() };
                    (same, session.title.clone(), failure)
                };
                if ask {
                    ANSWER_SEQ.fetch_add(1, Ordering::SeqCst);
                    // A question starts empty; a note's draft is kept for the note key
                    panel::show_ask(&ask_caption(title.as_deref()), false);
                    let waiting = ASKING_NOW.load(Ordering::SeqCst) && ASKING_FOR.lock().as_deref() == Some(meeting.as_str());
                    if waiting {
                        // Opened again while this recording's question is with the AI:
                        // the card waits for it, and the keyboard stays with the call
                        panel::set_busy(true);
                        panel::set_caption(ASKING, false);
                        panel::release_keyboard();
                    }
                    if title.is_none() {
                        read_title(app.clone(), meeting, shown);
                    }
                    return;
                }
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
                    session.note_draft.clear();
                }
                panel::show_notice(NOT_RECORDING, if ask { ASK_NOT_RECORDING_HINT } else { NOT_RECORDING_HINT });
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
                let asking = ASK_MODE.load(Ordering::SeqCst);
                let (bare, named) = if asking { (ask_caption(None), ask_caption(Some(&title))) } else { (caption(None), caption(Some(&title))) };
                let plain = panel::probe().is_some_and(|p| p.caption == bare);
                if SHOWN.load(Ordering::SeqCst) == shown && panel::is_open() && plain {
                    panel::set_caption(&named, false);
                }
            });
        });
    }

    /// Typing: how much room is left near the limit, else which recording.
    /// Either way an earlier error is gone.
    fn on_changed(text: String) {
        if ASK_MODE.load(Ordering::SeqCst) {
            // Typing a question: an earlier error is gone; an answer's hint stays
            if !ASKING_NOW.load(Ordering::SeqCst) && panel::answer_probe().is_some_and(|(answer, _)| answer.is_empty()) {
                let title = SESSION.lock().title.clone();
                panel::set_caption(&ask_caption(title.as_deref()), false);
            }
            return;
        }
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
        if ASK_MODE.load(Ordering::SeqCst) {
            on_ask(text);
            return;
        }
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

    /// The panel left the screen. A question typed and not sent goes with
    /// it (a note's draft stays: it is kept for the note key), and a timer
    /// for an answer has nothing left to put away.
    fn on_closed() {
        ANSWER_SEQ.fetch_add(1, Ordering::SeqCst);
        if ASK_MODE.load(Ordering::SeqCst) {
            panel::clear();
        }
    }

    /// Return in Ask: one question to the user's AI about the recording
    /// the panel was opened in, if it is still the one running. The
    /// keyboard goes back to the app in front at once; the card stays up
    /// while the question is answered and read.
    fn on_ask(text: String) {
        let Some(app) = APP.get().cloned() else { return };
        let Some(meeting) = SESSION.lock().meeting.clone() else {
            panel::hide();
            return;
        };
        let running = app.try_state::<AppState>().and_then(|state| markers::commands::recording_meeting(&state));
        if running.as_deref() != Some(meeting.as_str()) {
            panel::set_caption(RECORDING_ENDED, true);
            return;
        }
        let typed = text.trim();
        let question: String = if typed.is_empty() { ASK_DEFAULT.to_string() } else { typed.chars().take(600).collect() };
        if ASKING_NOW.swap(true, Ordering::SeqCst) {
            panel::set_caption(ONE_AT_A_TIME, true);
            return;
        }
        *ASKING_FOR.lock() = Some(meeting.clone());
        PURGED_MEANWHILE.store(false, Ordering::SeqCst);
        ANSWER_SEQ.fetch_add(1, Ordering::SeqCst);
        // The card says what was asked (also when nothing was typed), in the
        // field's grey line: the field itself is empty for the next question
        panel::clear();
        panel::set_placeholder(&question);
        panel::set_busy(true);
        panel::set_caption(ASKING, false);
        panel::release_keyboard();
        tauri::async_runtime::spawn(async move {
            let result = match app.try_state::<AppState>() {
                Some(state) => {
                    let pool = state.database.pool();
                    // The same conversation as the panel in the app
                    let thread: Option<String> = sqlx::query_scalar(
                        "SELECT id FROM chat_threads WHERE scope_kind = 'meeting' AND scope_value = ? ORDER BY updated_at DESC LIMIT 1",
                    )
                    .bind(&meeting)
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten();
                    let mode = crate::chat::live::Mode { recording: true, related: true, glance: true };
                    crate::chat::ask_recording(pool, &crate::study::LiveCompleter, thread.as_deref(), &meeting, &question, mode)
                        .await
                        .map(|turn| turn.assistant.content)
                }
                None => Err("noFriction is still starting".to_string()),
            };
            // The panel in the app shows the same conversation
            let _ = app.emit(ASK_ANSWERED_EVENT, serde_json::json!({ "meetingId": meeting }));
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || answered_on_main(&handle, result, &meeting));
        });
    }

    /// `{meetingId}`: a question asked from the card was answered (or failed)
    const ASK_ANSWERED_EVENT: &str = "ask_answered";

    /// The answer landed. It goes on the card if the card is up for a
    /// question about that recording (also when it was closed and opened
    /// again meanwhile). Otherwise the card is left alone: the answer is
    /// in the app's conversation.
    fn answered_on_main(app: &AppHandle, result: Result<String, String>, meeting: &str) {
        ASKING_NOW.store(false, Ordering::SeqCst);
        *ASKING_FOR.lock() = None;
        let purged = PURGED_MEANWHILE.swap(false, Ordering::SeqCst);
        let open = panel::is_open() && ASK_MODE.load(Ordering::SeqCst);
        let here = open && SESSION.lock().meeting.as_deref() == Some(meeting);
        if !here {
            // A card for another recording that said "still answering" can ask now
            if open && panel::probe().is_some_and(|p| p.caption == ONE_AT_A_TIME) {
                let title = SESSION.lock().title.clone();
                panel::set_caption(&ask_caption(title.as_deref()), false);
            }
            return;
        }
        if purged {
            // What it was asked about has been removed since
            panel::hide();
            return;
        }
        match result {
            Ok(markdown) => {
                panel::show_answer(&plain_answer(&markdown));
                panel::set_caption(ANSWER_HINT, false);
            }
            Err(e) => {
                // Never the endpoint's own words in the log or on the card
                log::warn!("Ask from the card failed ({})", error_code(&e));
                panel::set_caption(&ask_error(&e), true);
            }
        }
        panel::set_busy(false);
        // It puts itself away after a while, unless something else happened to it since
        let seq = ANSWER_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
        let handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ANSWER_STAYS_MS)).await;
            let _ = handle.run_on_main_thread(move || {
                if ANSWER_SEQ.load(Ordering::SeqCst) != seq || !panel::is_open() || !ASK_MODE.load(Ordering::SeqCst) {
                    return;
                }
                if panel::probe().is_some_and(|p| p.key) {
                    // Being used (a line is being copied): it closes when the user leaves it
                    panel::set_pinned(false);
                } else {
                    panel::hide();
                }
            });
        });
    }

    /// A recording's chat was purged (Delete, or a Strike in it): an answer
    /// about it that is still on the card goes too.
    pub fn answer_purged(meeting: String) {
        let Some(app) = APP.get() else { return };
        let _ = app.run_on_main_thread(move || {
            if ASKING_FOR.lock().as_deref() == Some(meeting.as_str()) {
                PURGED_MEANWHILE.store(true, Ordering::SeqCst);
            }
            let about = SESSION.lock().meeting.as_deref() == Some(meeting.as_str());
            if about && ASK_MODE.load(Ordering::SeqCst) && panel::is_open() {
                panel::hide();
            }
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
    fn the_ask_key_is_its_own_and_says_which_recording() {
        assert_ne!(ask_hotkey(), hotkey());
        assert_ne!(ask_hotkey(), crate::markers::hotkey());
        assert_eq!(ask_caption(None), "Return asks your AI · empty asks what you missed");
        assert_eq!(ask_caption(Some(" Acme sync ")), "Acme sync · Return asks your AI · empty asks what you missed");
        assert!(ASK_DEFAULT.ends_with('?'));
    }

    #[test]
    fn each_key_opens_its_own_panel_and_the_other_key_changes_it_over() {
        // (open, was_ask, ask, reading, saving, asking)
        // Closed: either key opens it
        assert_eq!(press(false, false, false, false, false, false), Press::Open);
        assert_eq!(press(false, false, true, false, false, false), Press::Open);
        assert_eq!(press(false, true, false, false, false, false), Press::Open);
        // Open: the same key closes it
        assert_eq!(press(true, false, false, false, false, false), Press::Close);
        assert_eq!(press(true, true, true, false, false, false), Press::Close);
        // Open for the other one: it changes over
        assert_eq!(press(true, false, true, false, false, false), Press::Open);
        assert_eq!(press(true, true, false, false, false, false), Press::Open);
        assert_eq!(press(true, true, false, true, false, false), Press::Open);
        // An answer is up while the user is back in their call: the Ask key
        // takes the keyboard for the next question
        assert_eq!(press(true, true, true, true, false, false), Press::NextQuestion);
        // While a question is with the AI the Ask key closes the card (the
        // answer is kept in the app), and no key changes it over
        assert_eq!(press(true, true, true, true, false, true), Press::Close);
        assert_eq!(press(true, true, false, true, false, true), Press::Close);
        // The line being kept is never taken for a question
        assert_eq!(press(true, false, true, false, true, false), Press::Close);
        assert_eq!(press(false, false, true, false, true, false), Press::Close);
        assert_eq!(press(false, false, false, false, true, false), Press::Open);
        // A question still with the AI: the card can be opened again to wait
        assert_eq!(press(false, true, true, false, false, true), Press::Open);
    }

    #[test]
    fn an_answer_is_shown_as_plain_lines() {
        let md = "They asked **you** for the `SOC 2` report by Friday [7].\n\n- Rollout starts in Denver [4]\n* Ana needs the *review* first [4, 5]\n### Next\n> Send it [2][3].";
        assert_eq!(
            plain_answer(md),
            "They asked you for the SOC 2 report by Friday.\n• Rollout starts in Denver\n• Ana needs the review first\nNext\nSend it."
        );
        // Brackets that aren't source numbers are text
        assert_eq!(plain_answer("See [the plan] and [stricken from the record]."), "See [the plan] and [stricken from the record].");
        assert_eq!(plain_answer("An open [bracket stays"), "An open [bracket stays");
        assert_eq!(plain_answer("\n\n  \n"), "");
        assert_eq!(plain_answer("- \n- Kept"), "• Kept");
        // What a model's Markdown really holds
        assert_eq!(plain_answer("It is built on .NET [3] and costs 5 * 3 dollars."), "It is built on .NET and costs 5 * 3 dollars.");
        assert_eq!(plain_answer("2*3 is *six*, not *seven*."), "2*3 is six, not seven.");
        assert_eq!(plain_answer("#1 priority is >50% uptime"), "#1 priority is >50% uptime");
        assert_eq!(plain_answer(">50% of sign-ups"), ">50% of sign-ups");
        assert_eq!(plain_answer("See [the plan](plan.md) and [2](#source-2)."), "See the plan and.");
        assert_eq!(plain_answer("```rust\nlet x = 1;\n```"), "let x = 1;");
        assert_eq!(plain_answer("  - nested *point*\n1. First [1]\n2. Second"), "• nested point\n1. First\n2. Second");
        assert_eq!(plain_answer("Ünïcode — “quoted” [1] ✓"), "Ünïcode — “quoted” ✓");
        // A long answer is cut for the card; the whole of it is in the app
        let long = plain_answer(&"word ".repeat(400));
        assert_eq!(long.chars().count(), CARD_CHARS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn a_failure_on_the_card_says_where_to_go_and_never_the_endpoints_words() {
        assert!(ask_error("CONSENT_REQUIRED:custom").contains("Open noFriction and ask there once"));
        assert!(ask_error("PRO_REQUIRED: AI notes are part of noFriction Pro.").contains("noFriction Pro"));
        assert!(ask_error("AI_NO_PROVIDER: No AI provider is set up.").contains("Settings → AI"));
        assert!(ask_error("AI_UNREACHABLE: Can't reach the AI provider (timed out)").starts_with("Couldn't reach your AI"));
        assert!(ask_error("AI_WRONG_KEY: 401").contains("didn't accept its key"));
        assert!(ask_error("AI_NO_CREDIT: 402").contains("out of credit"));
        assert!(ask_error("AI_BAD_URL: nope").contains("address isn't right"));
        assert!(ask_error("AI_TRUNCATED: length").contains("cut short"));
        let other = ask_error("AI_ERROR: 400 {\"error\": \"some long text from an endpoint\"}");
        assert!(!other.contains("some long text") && other.contains("Try again"));
        // Words in an endpoint's reply are not the app's own answer
        assert!(!ask_error("AI_ERROR: 400 CONSENT_REQUIRED:x").contains("permission"));
        // The log gets the app's own code and nothing an endpoint wrote
        assert_eq!(error_code("AI_UNREACHABLE: Can't reach the AI provider (timed out)"), "AI_UNREACHABLE");
        assert_eq!(error_code("CONSENT_REQUIRED:custom"), "CONSENT_REQUIRED");
        assert_eq!(error_code("the model said: something from the meeting"), "error");
        assert_eq!(error_code("Database busy"), "error");
        assert_eq!(error_code(""), "error");
    }

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
