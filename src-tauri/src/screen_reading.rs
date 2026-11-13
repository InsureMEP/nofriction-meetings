// noFriction - Read the screens (docs/design/FADELL_V2.md §12).
//
// The one place a picture leaves this Mac for an AI. The user presses Read
// the screens on a recording, sees how many screens will go and where, and
// confirms. Each chosen screen is then sent as one downscaled picture with
// the prompt for its kind (screen_prompts.rs), and what the AI read from it
// is kept as text beside the screen.
//
// Rules:
//   - Only on that click, for the number of screens the user was shown
//     (`read` refuses if the count it would send differs). Never by itself,
//     never on a timer, never while that recording is still running.
//   - Few, and the ones that matter: the screens showing when the user
//     marked a moment or typed a note, then the rest spread evenly, at most
//     `MAX_SCREENS` a click. A screen is read once.
//   - The gates of every AI call apply (Pro in the Mac App Store build,
//     consent for a public endpoint), and the model must read images:
//     Apple's on-device model can't, so nothing is sent with it.
//   - What was read is stored as `text_snapshots` rows with
//     `source = 'screen_reading'` on the screen's own state id, like a
//     browser address captured on screen (meeting_links.rs). Every screen
//     purge removes text snapshots by screen and by time whatever their
//     source (docs/REDACTION.md), so deleting or striking a screen removes
//     its reading with no new purge code. A reading is saved only if its
//     screen still exists when the answer arrives.
//   - The answer is untrusted: parsed tolerantly, capped, and never logged.

use crate::ai::{AiError, Kind, Msg, Opts};
use crate::recording_kind::RecordingKind;
use crate::screen_prompts::{self, ScreenKind};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::collections::HashSet;

/// `text_snapshots.source` of what the AI read from a screen
pub const SOURCE: &str = "screen_reading";
/// The most screens one click sends
pub const MAX_SCREENS: usize = 12;
/// What was being said around a screen, given to the AI as context
const SAID_BEFORE_MS: i64 = 30_000;
const SAID_AFTER_MS: i64 = 60_000;
const SAID_CHARS: usize = 420;
const READ_MAX_TOKENS: u32 = 700;

pub const STILL_RECORDING: &str = "Stop the recording to read its screens.";
pub const NOTHING_TO_READ: &str = "Every screen of this recording has been read.";
pub const SCREENS_CHANGED: &str = "The screens changed since you looked. Check the number again.";
pub const NEEDS_IMAGE_MODEL: &str =
    "Reading a screen needs a model that can read images. Choose one under Settings → AI → Screens.";
pub const APPLE_CANT: &str =
    "Apple's on-device model can't read pictures. Choose a model that can under Settings → AI → Screens.";

/// One captured screen of a recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub state_id: String,
    /// From the start of the recording
    pub ms: i64,
    pub end_ms: Option<i64>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub keyframe_path: String,
}

/// Which screens one click sends: indexes into `screens` (which is in time
/// order), in time order, at most `max`. First the screen showing at each
/// moment the user marked or typed a note; then the others, spread evenly
/// over the recording. Screens already read, and noFriction's own window,
/// are never chosen.
pub fn choose(screens: &[Screen], moments: &[i64], already: &HashSet<String>, max: usize) -> Vec<usize> {
    let eligible = |s: &Screen| {
        !already.contains(&s.state_id)
            && !s.keyframe_path.trim().is_empty()
            && !s.app_name.as_deref().is_some_and(|a| a.trim().eq_ignore_ascii_case("noFriction"))
    };
    let mut chosen: Vec<usize> = Vec::new();
    let mut sorted_moments = moments.to_vec();
    sorted_moments.sort_unstable();
    for m in sorted_moments {
        if chosen.len() >= max {
            break;
        }
        // The last screen that started at or before the moment
        let showing = screens.iter().rposition(|s| s.ms <= m).filter(|i| {
            let s = &screens[*i];
            s.end_ms.map_or(true, |end| m <= end + 5_000)
        });
        if let Some(i) = showing {
            if eligible(&screens[i]) && !chosen.contains(&i) {
                chosen.push(i);
            }
        }
    }
    let rest: Vec<usize> = (0..screens.len()).filter(|i| eligible(&screens[*i]) && !chosen.contains(i)).collect();
    let room = max.saturating_sub(chosen.len());
    if rest.len() <= room {
        chosen.extend(rest);
    } else if room > 0 {
        // Evenly over the ones left: the middle of each of `room` equal runs
        for k in 0..room {
            chosen.push(rest[(k * rest.len() + rest.len() / 2) / room]);
        }
    }
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}

/// What was being said while a screen was shown, for the prompt.
pub fn said_around(lines: &[crate::study::prompt::StudyLine], screen: &Screen) -> String {
    let from = screen.ms - SAID_BEFORE_MS;
    let to = screen.end_ms.unwrap_or(screen.ms).max(screen.ms) + SAID_AFTER_MS;
    let text = lines
        .iter()
        .filter(|l| l.ms >= from && l.ms <= to)
        .map(|l| l.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    text.chars().take(SAID_CHARS).collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// The answer (untrusted)
// ═══════════════════════════════════════════════════════════════════════════

const MAX_POINTS: usize = 8;
const MAX_TERMS: usize = 6;

/// What the AI read from one screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub kind: ScreenKind,
    pub title: Option<String>,
    pub points: Vec<String>,
    pub terms: Vec<(String, String)>,
    /// Wording kept exactly: a formula, a few lines of code
    pub text: Option<String>,
}

fn clip(v: Option<&Value>, max: usize) -> Option<String> {
    crate::study::parse::clean_text(v?, max)
}

/// The reading in a model answer. `Ok(None)` when the AI said to skip the
/// screen or found nothing on it; an answer that isn't a JSON object is an
/// error. `expected` is the kind the prompt was for: it is kept unless the
/// prompt asked the AI to decide (`ScreenKind::Other`).
pub fn parse(raw: &str, expected: ScreenKind) -> Result<Option<Reading>, String> {
    let v = crate::study::parse::extract_json(raw)?;
    let o = v.as_object().ok_or("The answer wasn't a JSON object")?;
    if o.get("skip").and_then(|s| s.as_bool()).unwrap_or(false) {
        return Ok(None);
    }
    let mut points: Vec<String> = Vec::new();
    if let Some(Value::Array(a)) = o.get("points") {
        for x in a.iter().take(40) {
            if let Some(p) = clip(Some(x), 300) {
                if !points.iter().any(|y| y.eq_ignore_ascii_case(&p)) {
                    points.push(p);
                }
            }
        }
    }
    points.truncate(MAX_POINTS);
    let mut terms: Vec<(String, String)> = Vec::new();
    if let Some(Value::Array(a)) = o.get("terms") {
        for x in a.iter().take(40) {
            let Value::Object(e) = x else { continue };
            let term = clip(e.get("term").or_else(|| e.get("name")), 80);
            let meaning = clip(e.get("meaning").or_else(|| e.get("definition")), 300);
            if let (Some(t), Some(m)) = (term, meaning) {
                if !terms.iter().any(|(y, _)| y.eq_ignore_ascii_case(&t)) {
                    terms.push((t, m));
                }
            }
        }
    }
    terms.truncate(MAX_TERMS);
    let title = clip(o.get("title"), 160).filter(|t| !t.eq_ignore_ascii_case("null"));
    let text = clip(o.get("text"), 600).filter(|t| !t.eq_ignore_ascii_case("null"));
    if title.is_none() && points.is_empty() && terms.is_empty() && text.is_none() {
        return Ok(None);
    }
    let kind = if expected == ScreenKind::Other {
        o.get("kind").and_then(|k| k.as_str()).and_then(ScreenKind::from_answer).unwrap_or(ScreenKind::Other)
    } else {
        expected
    };
    Ok(Some(Reading { kind, title, points, terms, text }))
}

/// The reading as the plain text that is stored and shown:
/// "Slide: The cell membrane", then its points, terms and exact wording.
pub fn to_text(r: &Reading) -> String {
    let mut s = match &r.title {
        Some(t) => format!("{}: {}", r.kind.label(), t),
        None => r.kind.label().to_string(),
    };
    for p in &r.points {
        s.push_str(&format!("\n- {}", p));
    }
    if !r.terms.is_empty() {
        s.push_str("\nTerms:");
        for (t, m) in &r.terms {
            s.push_str(&format!("\n- {}: {}", t, m));
        }
    }
    if let Some(t) = &r.text {
        s.push_str(&format!("\nExact wording: {}", t));
    }
    s
}

// ═══════════════════════════════════════════════════════════════════════════
// The plan (what a click would send) and the stored readings
// ═══════════════════════════════════════════════════════════════════════════

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

fn parse_ts(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&chrono::Utc))
}

/// A recording's screens that still have their picture, in time order.
pub async fn screens(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<Vec<Screen>, String> {
    let started: Option<String> = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(err("Couldn't read the recording"))?;
    let Some(started) = started.as_deref().and_then(parse_ts) else { return Ok(Vec::new()) };
    let rows = sqlx::query(
        "SELECT state_id, start_ts, end_ts, app_name, window_title, keyframe_path FROM screen_states \
         WHERE meeting_id = ? AND keyframe_path IS NOT NULL AND keyframe_path != '' ORDER BY start_ts ASC, state_id ASC",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(err("Couldn't read the screens"))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let ms = (parse_ts(&r.get::<String, _>("start_ts"))? - started).num_milliseconds().max(0);
            let end_ms = r
                .get::<Option<String>, _>("end_ts")
                .as_deref()
                .and_then(parse_ts)
                .map(|t| (t - started).num_milliseconds().max(ms));
            let path: String = r.get("keyframe_path");
            // A picture that was removed from disk has nothing to send
            std::path::Path::new(&path).is_file().then(|| Screen {
                state_id: r.get("state_id"),
                ms,
                end_ms,
                app_name: r.get("app_name"),
                window_title: r.get("window_title"),
                keyframe_path: path,
            })
        })
        .collect())
}

/// The screens of a recording the AI has already looked at (read or skipped).
async fn already_read(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<HashSet<String>, String> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT state_id FROM text_snapshots WHERE source = ?1 AND state_id IN \
         (SELECT state_id FROM screen_states WHERE meeting_id = ?2)",
    )
    .bind(SOURCE)
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(err("Couldn't read the screens"))?
    .into_iter()
    .collect())
}

/// One screen a click would send, for the confirmation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PlanItem {
    pub state_id: String,
    pub ms: i64,
    /// "Slide", "Document"…: which prompt of the library it gets
    pub kind: &'static str,
    pub app_name: Option<String>,
}

/// What Read the screens would do right now. Nothing is sent to make it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Plan {
    /// The screens one click would send, in time order
    pub send: Vec<PlanItem>,
    /// Screens of this recording that have a picture
    pub total: usize,
    /// Of those, how many the AI has already looked at
    pub already: usize,
    /// Where the pictures would go (the host), when an image model is set up
    pub destination: Option<String>,
    pub model: Option<String>,
    /// The endpoint is on this Mac or the local network
    pub local: bool,
    /// Why it can't run now, in words for the user; None when it can
    pub blocked: Option<String>,
}

/// Where pictures would go, without the side effects of resolving a
/// request (no consent prompt is raised by looking).
fn destination() -> (Option<String>, Option<String>, bool, Option<String>) {
    let cfg = crate::ai::config::snapshot();
    let Some(sel) = cfg.selection(Kind::Vision) else {
        return (None, None, false, Some(NEEDS_IMAGE_MODEL.to_string()));
    };
    if sel.provider == "apple" {
        return (None, None, true, Some(APPLE_CANT.to_string()));
    }
    let host = cfg
        .base_url(&sel.provider)
        .and_then(|u| url::Url::parse(&u).ok())
        .and_then(|u| u.host_str().map(str::to_string));
    let blocked = host.is_none().then(|| NEEDS_IMAGE_MODEL.to_string());
    (host, Some(sel.model), cfg.is_local(&sel.provider), blocked)
}

pub async fn plan(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<Plan, String> {
    let all = screens(pool, meeting_id).await?;
    let already = already_read(pool, meeting_id).await?;
    let input = crate::study::load_input(pool, meeting_id).await?;
    let moments: Vec<i64> = input.marks.iter().map(|m| m.ms).collect();
    let send: Vec<PlanItem> = choose(&all, &moments, &already, MAX_SCREENS)
        .into_iter()
        .map(|i| {
            let s = &all[i];
            PlanItem {
                state_id: s.state_id.clone(),
                ms: s.ms,
                kind: screen_prompts::kind_for(s.app_name.as_deref(), s.window_title.as_deref()).label(),
                app_name: s.app_name.clone(),
            }
        })
        .collect();
    let (destination, model, local, mut blocked) = destination();
    if blocked.is_none() && send.is_empty() && !all.is_empty() {
        blocked = Some(NOTHING_TO_READ.to_string());
    }
    Ok(Plan { send, total: all.len(), already: all.iter().filter(|s| already.contains(&s.state_id)).count(), destination, model, local, blocked })
}

/// A stored reading, for the Notes view.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StoredReading {
    pub state_id: String,
    pub ms: i64,
    pub text: String,
}

/// What was read from a recording's screens, in time order. Screens the AI
/// skipped are remembered (so they aren't sent again) but have no text and
/// aren't listed.
pub async fn readings(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<Vec<StoredReading>, String> {
    let started: Option<String> = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(err("Couldn't read the recording"))?;
    let Some(started) = started.as_deref().and_then(parse_ts) else { return Ok(Vec::new()) };
    let rows = sqlx::query(
        "SELECT t.state_id, s.start_ts, t.text FROM text_snapshots t JOIN screen_states s ON s.state_id = t.state_id \
         WHERE t.source = ?1 AND s.meeting_id = ?2 AND TRIM(t.text) != '' ORDER BY s.start_ts ASC, t.created_at ASC",
    )
    .bind(SOURCE)
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(err("Couldn't read the screens"))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            Some(StoredReading {
                state_id: r.get("state_id"),
                ms: (parse_ts(&r.get::<String, _>("start_ts"))? - started).num_milliseconds().max(0),
                text: r.get("text"),
            })
        })
        .collect())
}

/// Keep what was read from a screen, only if that screen still exists with
/// the same picture (it may have been deleted or struck while the AI was
/// answering). One statement, so nothing can land in between. An empty
/// `text` records that the AI looked and found nothing to keep.
async fn store(pool: &Pool<Sqlite>, screen: &Screen, text: &str) -> Result<bool, String> {
    let hash: String = Sha256::digest(text.as_bytes()).iter().take(16).map(|b| format!("{:02x}", b)).collect();
    let done = sqlx::query(
        "INSERT INTO text_snapshots \
         (snapshot_id, episode_id, state_id, meeting_id, ts, text, text_hash, quality_score, source, word_count, app_name, window_title) \
         SELECT ?1, NULL, s.state_id, s.meeting_id, s.start_ts, ?2, ?3, 1.0, ?4, ?5, s.app_name, s.window_title \
         FROM screen_states s WHERE s.state_id = ?6 AND s.keyframe_path = ?7 \
         AND NOT EXISTS (SELECT 1 FROM text_snapshots t WHERE t.state_id = s.state_id AND t.source = ?4)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(text)
    .bind(hash)
    .bind(SOURCE)
    .bind(text.split_whitespace().count() as i64)
    .bind(&screen.state_id)
    .bind(&screen.keyframe_path)
    .execute(pool)
    .await
    .map_err(err("Couldn't keep what was read"))?;
    Ok(done.rows_affected() > 0)
}

// ═══════════════════════════════════════════════════════════════════════════
// Reading
// ═══════════════════════════════════════════════════════════════════════════

/// Sends one picture with its prompt. Abstracted so tests never reach an
/// AI; the live one is the only caller of `Msg::user_with_image` in the app.
#[async_trait::async_trait]
pub trait Reader: Send + Sync {
    async fn read(&self, image_path: &str, prompt: &str) -> Result<String, AiError>;
}

/// The user's image model. `ai::complete_vision` checks Pro, consent and
/// the endpoint policy, and refuses Apple's on-device model.
pub struct LiveReader;

#[async_trait::async_trait]
impl Reader for LiveReader {
    async fn read(&self, image_path: &str, prompt: &str) -> Result<String, AiError> {
        let path = image_path.to_string();
        // Long edge at most 1568 px, JPEG: what leaves is a copy, never the file
        let (mime, b64) = tokio::task::spawn_blocking(move || crate::vlm_client::encode_image_for_upload(&path))
            .await
            .map_err(|e| AiError::Other(format!("Couldn't prepare the picture: {}", e)))?
            .map_err(AiError::Other)?;
        crate::ai::complete_vision(
            vec![Msg::user_with_image(prompt.to_string(), &mime, b64)],
            Opts { max_tokens: READ_MAX_TOKENS, temperature: Some(0.1) },
        )
        .await
    }
}

/// How one click went.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Summary {
    /// Screens whose reading was kept
    pub read: usize,
    /// Screens the AI had nothing to keep from (private, empty, or unreadable twice)
    pub skipped: usize,
    /// Screens that were deleted or changed while the AI was answering
    pub gone: usize,
}

/// Read the screens a click was shown (`expect` of them). Stops at the
/// first AI access error (consent, Pro, no model, unreachable); what was
/// read before it is kept.
pub async fn read_screens(
    reader: &dyn Reader,
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    expect: usize,
    progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<Summary, String> {
    let all = screens(pool, meeting_id).await?;
    let already = already_read(pool, meeting_id).await?;
    let input = crate::study::load_input(pool, meeting_id).await?;
    let moments: Vec<i64> = input.marks.iter().map(|m| m.ms).collect();
    let chosen = choose(&all, &moments, &already, MAX_SCREENS);
    if chosen.is_empty() {
        return Err(NOTHING_TO_READ.into());
    }
    // The user confirmed a number: never send a different one
    if chosen.len() != expect {
        return Err(SCREENS_CHANGED.into());
    }
    let recording: RecordingKind = input.kind;
    let mut summary = Summary::default();
    for (n, i) in chosen.iter().enumerate() {
        progress(n, chosen.len());
        let screen = &all[*i];
        let kind = screen_prompts::kind_for(screen.app_name.as_deref(), screen.window_title.as_deref());
        let prompt = screen_prompts::prompt_for(kind, recording, &said_around(&input.lines, screen));
        let reading = match reader.read(&screen.keyframe_path, &prompt).await {
            Ok(raw) => parse(&raw, kind).ok().flatten(),
            // A cut-off or malformed answer is this screen's problem, not the run's
            Err(e) if crate::study::answer_error(&e) => None,
            Err(e) => return Err(e.to_string()),
        };
        let text = reading.as_ref().map(to_text).unwrap_or_default();
        if !store(pool, screen, &text).await? {
            summary.gone += 1;
        } else if text.is_empty() {
            summary.skipped += 1;
        } else {
            summary.read += 1;
        }
    }
    progress(chosen.len(), chosen.len());
    Ok(summary)
}

// ═══════════════════════════════════════════════════════════════════════════
// Commands
// ═══════════════════════════════════════════════════════════════════════════

pub mod commands {
    use super::*;
    use crate::AppState;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use tauri::{AppHandle, Emitter, State};

    /// `{meetingId, done, total}` while screens are being read.
    pub const PROGRESS_EVENT: &str = "screens_progress";

    /// Recordings whose screens are being read: one run each.
    static RUNNING: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

    struct RunGuard(String);
    impl Drop for RunGuard {
        fn drop(&mut self) {
            RUNNING.lock().remove(&self.0);
        }
    }

    fn recording_meeting(state: &AppState) -> Option<String> {
        if !state.capture_engine.read().is_recording() {
            return None;
        }
        state.state_builder.read().current_meeting_id()
    }

    /// What Read the screens would send for this recording, and where.
    /// Looking sends nothing.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn plan_screen_reading(state: State<'_, AppState>, meeting_id: String) -> Result<Plan, String> {
        let mut plan = plan(state.database.pool(), &meeting_id).await?;
        if recording_meeting(&state).as_deref() == Some(meeting_id.as_str()) {
            plan.blocked = Some(STILL_RECORDING.to_string());
        }
        Ok(plan)
    }

    /// Read the screens (the confirmation's button). `expect` is the number
    /// of screens the user was shown; a different number is refused.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn read_screens(
        app: AppHandle,
        state: State<'_, AppState>,
        meeting_id: String,
        expect: usize,
    ) -> Result<Summary, String> {
        if recording_meeting(&state).as_deref() == Some(meeting_id.as_str()) {
            return Err(STILL_RECORDING.into());
        }
        if !RUNNING.lock().insert(meeting_id.clone()) {
            return Err("This recording's screens are already being read.".into());
        }
        let _guard = RunGuard(meeting_id.clone());
        let emit = {
            let app = app.clone();
            let id = meeting_id.clone();
            move |done: usize, total: usize| {
                let _ = app.emit(PROGRESS_EVENT, serde_json::json!({ "meetingId": id, "done": done, "total": total }));
            }
        };
        super::read_screens(&LiveReader, state.database.pool(), &meeting_id, expect, &emit).await
    }

    /// What was read from this recording's screens.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_screen_readings(state: State<'_, AppState>, meeting_id: String) -> Result<Vec<StoredReading>, String> {
        readings(state.database.pool(), &meeting_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;
    use parking_lot::Mutex;
    use std::sync::Arc;

    fn screen(id: &str, ms: i64, app: &str) -> Screen {
        Screen { state_id: id.into(), ms, end_ms: None, app_name: Some(app.into()), window_title: None, keyframe_path: format!("/tmp/{id}.png") }
    }

    #[test]
    fn the_screens_at_marks_and_notes_go_first_then_the_rest_evenly() {
        let screens: Vec<Screen> = (0..30).map(|n| screen(&format!("s{n}"), n * 60_000, "Keynote")).collect();
        let none = HashSet::new();
        // Nothing marked: evenly over the recording, never more than the cap
        let even = choose(&screens, &[], &none, 6);
        assert_eq!(even, vec![2, 7, 12, 17, 22, 27]);
        // A mark at 10:30 and a note at 25:10: the screens showing then are in
        let picked = choose(&screens, &[25 * 60_000 + 10_000, 10 * 60_000 + 30_000], &none, 6);
        assert!(picked.contains(&10) && picked.contains(&25), "{picked:?}");
        assert_eq!(picked.len(), 6);
        assert!(picked.windows(2).all(|w| w[0] < w[1]), "in time order");
        // Fewer screens than the cap: all of them
        assert_eq!(choose(&screens[..4], &[], &none, 12), vec![0, 1, 2, 3]);
        // More marks than the cap: only the cap
        let marks: Vec<i64> = (0..30).map(|n| n * 60_000 + 5_000).collect();
        assert_eq!(choose(&screens, &marks, &none, 12).len(), 12);
    }

    #[test]
    fn a_screen_is_read_once_and_the_apps_own_window_never() {
        let mut screens: Vec<Screen> = (0..5).map(|n| screen(&format!("s{n}"), n * 60_000, "Keynote")).collect();
        screens[1].app_name = Some("noFriction".into());
        screens[3].keyframe_path = " ".into();
        let already: HashSet<String> = ["s0".to_string()].into_iter().collect();
        assert_eq!(choose(&screens, &[0, 60_000], &already, 12), vec![2, 4]);
        // A mark after a screen ended picks nothing for it
        let mut ended = vec![screen("a", 0, "Keynote")];
        ended[0].end_ms = Some(10_000);
        assert_eq!(choose(&ended, &[600_000], &HashSet::new(), 1), vec![0], "it is still chosen as one of the rest");
        assert_eq!(choose(&ended, &[600_000], &["a".to_string()].into_iter().collect(), 1), Vec::<usize>::new());
    }

    #[test]
    fn answers_are_parsed_tolerantly_and_skips_keep_nothing() {
        let raw = r#"Here you go: {"skip": false, "kind": "slide", "title": " The cell membrane ",
            "points": ["A phospholipid bilayer.", "a phospholipid bilayer.", "Proteins move through it.", 7],
            "terms": [{"term": "Osmosis", "meaning": "Water moving across a membrane."}, {"term": "osmosis", "meaning": "again"}, "junk"],
            "text": "ΔG = ΔH − TΔS"}"#;
        let r = parse(raw, ScreenKind::Slide).unwrap().unwrap();
        assert_eq!(r.title.as_deref(), Some("The cell membrane"));
        assert_eq!(r.points, vec!["A phospholipid bilayer.", "Proteins move through it.", "7"]);
        assert_eq!(r.terms, vec![("Osmosis".to_string(), "Water moving across a membrane.".to_string())]);
        assert_eq!(
            to_text(&r),
            "Slide: The cell membrane\n- A phospholipid bilayer.\n- Proteins move through it.\n- 7\nTerms:\n- Osmosis: Water moving across a membrane.\nExact wording: ΔG = ΔH − TΔS"
        );
        // The prompt for an unrecognised screen lets the AI say what it is
        let other = parse(r#"{"kind": "chart", "title": "Sales by quarter", "points": []}"#, ScreenKind::Other).unwrap().unwrap();
        assert_eq!(other.kind, ScreenKind::Chart);
        assert_eq!(to_text(&other), "Chart: Sales by quarter");
        // A known kind is kept whatever the answer calls it
        assert_eq!(parse(r#"{"kind": "chart", "title": "T"}"#, ScreenKind::Document).unwrap().unwrap().kind, ScreenKind::Document);
        // Skip, or nothing in it: nothing is kept
        assert_eq!(parse(r#"{"skip": true, "title": "Inbox"}"#, ScreenKind::WebPage).unwrap(), None);
        assert_eq!(parse(r#"{"title": null, "points": [], "text": "null"}"#, ScreenKind::Slide).unwrap(), None);
        assert!(parse("I can't read this.", ScreenKind::Slide).is_err());
        // Caps
        let many = format!(r#"{{"points":[{}]}}"#, (0..30).map(|n| format!("\"point {n}\"")).collect::<Vec<_>>().join(","));
        assert_eq!(parse(&many, ScreenKind::Slide).unwrap().unwrap().points.len(), MAX_POINTS);
    }

    struct Mock {
        answers: Mutex<Vec<Result<String, AiError>>>,
        sent: Mutex<Vec<(String, String)>>,
        /// SQL run once, during the first request
        meanwhile: Mutex<Option<(Arc<DatabaseManager>, String)>>,
    }

    impl Mock {
        fn new(answers: Vec<Result<&str, AiError>>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().map(|a| a.map(str::to_string)).collect()),
                sent: Mutex::new(Vec::new()),
                meanwhile: Mutex::new(None),
            }
        }
    }

    #[async_trait::async_trait]
    impl Reader for Mock {
        async fn read(&self, image_path: &str, prompt: &str) -> Result<String, AiError> {
            self.sent.lock().push((image_path.to_string(), prompt.to_string()));
            let meanwhile = self.meanwhile.lock().take();
            if let Some((db, sql)) = meanwhile {
                sqlx::query(&sql).execute(db.pool()).await.unwrap();
            }
            let mut a = self.answers.lock();
            if a.len() > 1 {
                a.remove(0)
            } else {
                a[0].clone()
            }
        }
    }

    struct Fx {
        db: Arc<DatabaseManager>,
        dir: std::path::PathBuf,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    const SLIDE: &str = r#"{"kind":"slide","title":"The cell membrane","points":["A phospholipid bilayer."]}"#;

    /// A class recording with `n` screens a minute apart (pictures on disk),
    /// one line said at each, and a typed note at 2:10.
    async fn fx(n: i64) -> Fx {
        let dir = std::env::temp_dir().join(format!("nf-screens-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = DatabaseManager::new(&dir.join("t.db")).await.unwrap();
        db.run_migrations().await.unwrap();
        db.create_meeting("m1", "Membrane transport").await.unwrap();
        db.set_meeting_kind("m1", RecordingKind::Class).await.unwrap();
        let start: String = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = 'm1'").fetch_one(db.pool()).await.unwrap();
        let start = parse_ts(&start).unwrap();
        for i in 0..n {
            let path = dir.join(format!("s{i}.png"));
            std::fs::write(&path, b"png").unwrap();
            let at = start + chrono::Duration::seconds(i * 60);
            sqlx::query(
                "INSERT INTO screen_states (state_id, meeting_id, start_ts, end_ts, app_name, window_title, phash, keyframe_path) \
                 VALUES (?, 'm1', ?, ?, 'Keynote', 'Lecture 7', 'p', ?)",
            )
            .bind(format!("s{i}"))
            .bind(at.to_rfc3339())
            .bind((at + chrono::Duration::seconds(60)).to_rfc3339())
            .bind(path.to_str().unwrap())
            .execute(db.pool())
            .await
            .unwrap();
            db.add_transcript_at("m1", &format!("This is what was said at screen {i}."), None, true, 0.9, at + chrono::Duration::seconds(5))
                .await
                .unwrap();
        }
        crate::markers::add(db.pool(), "m1", start + chrono::Duration::seconds(130), Some("note"), Some("pump"), false).await.unwrap();
        Fx { db: Arc::new(db), dir }
    }

    fn quiet() -> impl Fn(usize, usize) + Send + Sync {
        |_, _| {}
    }

    #[tokio::test]
    async fn one_click_sends_the_number_shown_and_keeps_what_was_read() {
        let f = fx(3).await;
        let pool = f.db.pool();
        let p = plan(pool, "m1").await.unwrap();
        assert_eq!((p.send.len(), p.total, p.already), (3, 3, 0));
        assert_eq!(p.send[0].kind, "Slide");

        // A different number than the user was shown is refused, and nothing is sent
        let m = Mock::new(vec![Ok(SLIDE)]);
        assert_eq!(read_screens(&m, pool, "m1", 2, &quiet()).await.unwrap_err(), SCREENS_CHANGED);
        assert!(m.sent.lock().is_empty());

        let m = Mock::new(vec![Ok(SLIDE), Ok(r#"{"skip": true}"#), Ok("not json")]);
        let s = read_screens(&m, pool, "m1", 3, &quiet()).await.unwrap();
        assert_eq!(s, Summary { read: 1, skipped: 2, gone: 0 });
        let sent = m.sent.lock().clone();
        assert_eq!(sent.len(), 3, "one picture per request");
        assert!(sent[0].0.ends_with("s0.png"));
        // The slide prompt, with what was being said then, and never an id
        assert!(sent[0].1.contains("This is a presentation slide") && sent[0].1.contains("what was said at screen 0"));
        assert!(sent[2].1.contains("what was said at screen 2"));

        let kept = readings(pool, "m1").await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].state_id.as_str(), kept[0].ms), ("s0", 0));
        assert_eq!(kept[0].text, "Slide: The cell membrane\n- A phospholipid bilayer.");

        // Every screen has been looked at: nothing is sent again
        let again = plan(pool, "m1").await.unwrap();
        assert_eq!((again.send.len(), again.already), (0, 3));
        let m = Mock::new(vec![Ok(SLIDE)]);
        assert_eq!(read_screens(&m, pool, "m1", 0, &quiet()).await.unwrap_err(), NOTHING_TO_READ);
        assert!(m.sent.lock().is_empty());
    }

    #[tokio::test]
    async fn a_reading_never_outlives_its_screen() {
        // The screen is deleted while the AI is answering: its reading isn't kept
        let f = fx(1).await;
        let pool = f.db.pool();
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.meanwhile.lock() = Some((f.db.clone(), "DELETE FROM screen_states WHERE state_id = 's0'".into()));
        assert_eq!(read_screens(&m, pool, "m1", 1, &quiet()).await.unwrap(), Summary { read: 0, skipped: 0, gone: 1 });
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM text_snapshots").fetch_one(pool).await.unwrap();
        assert_eq!(n, 0);

        // Its picture is replaced (a strike blanks it): not kept either
        let f = fx(1).await;
        let pool = f.db.pool();
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.meanwhile.lock() = Some((f.db.clone(), "UPDATE screen_states SET keyframe_path = '/elsewhere.png' WHERE state_id = 's0'".into()));
        assert_eq!(read_screens(&m, pool, "m1", 1, &quiet()).await.unwrap().gone, 1);

        // A kept reading is a text snapshot on the screen: deleting the screen takes it
        let f = fx(2).await;
        let pool = f.db.pool();
        read_screens(&Mock::new(vec![Ok(SLIDE)]), pool, "m1", 2, &quiet()).await.unwrap();
        assert_eq!(readings(pool, "m1").await.unwrap().len(), 2);
        let (source, state): (String, String) =
            sqlx::query_as("SELECT source, state_id FROM text_snapshots ORDER BY ts LIMIT 1").fetch_one(pool).await.unwrap();
        assert_eq!((source.as_str(), state.as_str()), (SOURCE, "s0"));
        sqlx::query("PRAGMA foreign_keys = ON").execute(pool).await.unwrap();
        sqlx::query("DELETE FROM screen_states WHERE state_id = 's0'").execute(pool).await.unwrap();
        let left = readings(pool, "m1").await.unwrap();
        assert!(left.iter().all(|r| r.state_id != "s0"));
    }

    #[tokio::test]
    async fn an_access_error_stops_the_run_and_keeps_what_was_read() {
        let f = fx(3).await;
        let pool = f.db.pool();
        let m = Mock::new(vec![Ok(SLIDE), Err(AiError::ConsentRequired("custom".into())), Ok(SLIDE)]);
        let e = read_screens(&m, pool, "m1", 3, &quiet()).await.unwrap_err();
        assert!(e.starts_with("CONSENT_REQUIRED"), "{e}");
        assert_eq!(m.sent.lock().len(), 2, "nothing more is sent after the refusal");
        assert_eq!(readings(pool, "m1").await.unwrap().len(), 1);
        // The rest can be read on the next click
        assert_eq!(plan(pool, "m1").await.unwrap().send.len(), 2);
    }

    #[test]
    fn what_was_said_is_taken_from_around_the_screen_and_capped() {
        use crate::study::prompt::StudyLine;
        let lines: Vec<StudyLine> = (0..20).map(|n| StudyLine { ms: n * 30_000, text: format!("line {n}.") }).collect();
        let mut s = screen("a", 120_000, "Keynote");
        s.end_ms = Some(150_000);
        // From 30 s before it appeared to 60 s after it went
        assert_eq!(said_around(&lines, &s), "line 3. line 4. line 5. line 6. line 7.");
        let long: Vec<StudyLine> = vec![StudyLine { ms: 120_000, text: "x".repeat(2_000) }];
        assert_eq!(said_around(&long, &s).chars().count(), SAID_CHARS);
        assert_eq!(said_around(&[], &s), "");
    }
}
