// noFriction - Read the screens (docs/design/FADELL_V2.md §12).
//
// The one place a picture leaves this Mac for an AI. The user presses Read
// the screens on a recording, sees the pictures that would go and where,
// and confirms. Each confirmed picture is then sent, downscaled, with the
// prompt for its kind (screen_prompts.rs), and what the AI read from it is
// kept as text beside the screen.
//
// Rules:
//   - Only on that click, and only the pictures the user was shown and left
//     ticked: the confirmation passes back their ids and the destination it
//     named, and `read_screens` sends those ids and nothing else. It never
//     works the list out again. If the destination is no longer the one
//     named, nothing (more) is sent: the request itself is made only on a
//     target that was resolved and compared in the same step
//     (`ai::complete_vision_if`), so no change of settings can slip between
//     the check and the send.
//   - Never by itself, never on a timer, never while that recording is
//     still running, never while a Delete is in its undo window.
//   - Few, and the ones that matter: the screens showing when the user
//     marked a moment or typed a note, then the rest spread evenly, at most
//     `MAX_SCREENS` a click. A screen is read once.
//   - Private screens are never offered (`screen_prompts::never_sent`):
//     mail, messages, password apps, terminals, private browsing, key files
//     and noFriction's own window. That is decided on the Mac, from the
//     window a screen was captured from, or for a whole-display capture
//     from the window that was in front of it. A private window that was
//     only visible behind another can't be recognised: the confirmation
//     shows every picture whole, and the user can leave any out.
//   - The gates of every AI call apply (Pro in the Mac App Store build,
//     consent for a public endpoint), and the model must read images:
//     Apple's on-device model can't, so nothing is sent with it.
//   - What leaves is a downscaled copy of the picture and a fixed prompt.
//     No transcript, title, app or window name goes with it, so a reading
//     can only repeat what its own picture shows.
//   - What was read is stored as `text_snapshots` rows with
//     `source = 'screen_reading'` on the screen's own state id. Every screen
//     purge removes text snapshots by screen and by time whatever their
//     source (docs/REDACTION.md), so deleting or striking a screen removes
//     its reading with no new purge code. A reading is saved only if its
//     screen still exists when the answer arrives. Other readers of
//     `text_snapshots` (Links, the timeline, chat search, Obsidian export)
//     leave these rows out: they are the AI's words, not text captured from
//     the screen.
//   - A screen counts as read only when the AI answered about it: a reading,
//     or a plain "nothing to keep". An endpoint error stops the run and
//     leaves the screen unread; an answer that can't be used leaves it
//     unread too. What was read can be removed again (`forget`), and the
//     screen is then unread.
//   - The answer is untrusted: parsed tolerantly, capped, and never logged.

use crate::ai::{AiError, Kind};
use crate::recording_kind::RecordingKind;
use crate::screen_prompts::{self, ScreenKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

/// `text_snapshots.source` of what the AI read from a screen
pub const SOURCE: &str = "screen_reading";
/// The most screens one click sends
pub const MAX_SCREENS: usize = 12;
/// Room for a full answer: 8 points, 6 terms and the exact wording
const READ_MAX_TOKENS: u32 = 1_400;

pub const STILL_RECORDING: &str = "Stop the recording to read its screens.";
pub const NOTHING_TO_READ: &str = "Every screen of this recording has been read.";
pub const NONE_CHOSEN: &str = "No screens were chosen.";
pub const TOO_MANY: &str = "That is more screens than one click sends. Check the list again.";
pub const DESTINATION_CHANGED: &str =
    "Your AI settings changed since you looked, so nothing more was sent. Check the list again.";
pub const ALREADY_RUNNING: &str = "This recording's screens are already being read.";
pub const DELETE_WAITING: &str =
    "Something in this recording is being deleted. Try again when its undo time has passed.";
pub const NEEDS_IMAGE_MODEL: &str =
    "Reading a screen needs a model that can read images. Choose one under Settings → AI → Screens.";
pub const APPLE_CANT: &str =
    "Apple's on-device model can't read pictures. Choose a model that can under Settings → AI → Screens.";

/// One picture of a recording: a captured screen, or a photo from the iPhone.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub state_id: String,
    /// From the start of the recording
    pub ms: i64,
    pub end_ms: Option<i64>,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub keyframe_path: String,
    /// A photo taken with the iPhone, not a capture of a screen
    pub photo: bool,
    /// A capture of a whole display: everything that was on it
    pub whole_display: bool,
    /// Whole-display captures: the app and title of the window in front
    pub front_app: Option<String>,
    pub front_title: Option<String>,
}

impl Screen {
    /// Which prompt of the library it gets
    pub fn kind(&self) -> ScreenKind {
        if self.photo {
            ScreenKind::Photo
        } else {
            screen_prompts::kind_for(self.app_name.as_deref(), self.window_title.as_deref())
        }
    }

    /// Never sent, whatever the user presses. A capture of one window is
    /// judged by that window; a capture of a whole display by the window
    /// that was in front of it.
    pub fn private(&self) -> bool {
        if self.photo {
            return false;
        }
        if screen_prompts::never_sent(self.app_name.as_deref(), self.window_title.as_deref()) {
            return true;
        }
        self.app_name.is_none()
            && self.front_app.is_some()
            && screen_prompts::never_sent(self.front_app.as_deref(), self.front_title.as_deref())
    }

    fn has_picture(&self) -> bool {
        !self.keyframe_path.trim().is_empty()
    }
}

/// Which screens one click offers: indexes into `screens` (which is in time
/// order), in time order, at most `max`. First the screen showing at each
/// moment the user marked or typed a note; then the others, spread evenly
/// over the recording. Screens already read, and private ones, are never
/// chosen.
pub fn choose(screens: &[Screen], moments: &[i64], already: &HashSet<String>, max: usize) -> Vec<usize> {
    let eligible = |s: &Screen| !already.contains(&s.state_id) && s.has_picture() && !s.private();
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

/// The AI said to skip the screen. Small models write the flag as a word or
/// a number as often as a JSON boolean; any of them means skip, because
/// keeping what it was told to leave out is the worse mistake.
fn says_skip(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|x| x != 0.0),
        Some(Value::String(s)) => matches!(s.trim().to_lowercase().as_str(), "true" | "yes" | "y" | "1" | "skip"),
        _ => false,
    }
}

/// The reading in a model answer. `Ok(None)` when the AI said to skip the
/// screen or found nothing on it; an answer that isn't a JSON object is an
/// error. `expected` is the kind the prompt was for: it is kept unless the
/// prompt asked the AI to decide (`ScreenKind::ai_decides`).
pub fn parse(raw: &str, expected: ScreenKind) -> Result<Option<Reading>, String> {
    let v = crate::study::parse::extract_json(raw)?;
    let o = v.as_object().ok_or("The answer wasn't a JSON object")?;
    if says_skip(o.get("skip")) {
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
    // A model that never saw the picture tends to hand back the example in
    // the instructions. That is not a reading of anything.
    let echoed = |t: &str| screen_prompts::ANSWER_PLACEHOLDERS.iter().any(|e| t.trim().eq_ignore_ascii_case(e));
    if title.as_deref().is_some_and(echoed)
        || text.as_deref().is_some_and(echoed)
        || points.iter().any(|p| echoed(p))
        || terms.iter().any(|(t, m)| echoed(t) || echoed(m))
    {
        return Err("The answer repeated the instructions instead of reading the picture".into());
    }
    let kind = if expected.ai_decides() {
        o.get("kind").and_then(|k| k.as_str()).and_then(ScreenKind::from_answer).unwrap_or(expected)
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

/// A recording's pictures that are still on disk, in time order; only the
/// one with `only` as its id when that is given.
async fn pictures(pool: &Pool<Sqlite>, meeting_id: &str, only: Option<&str>) -> Result<Vec<Screen>, String> {
    let started: Option<String> = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(err("Couldn't read the recording"))?;
    let Some(started) = started.as_deref().and_then(parse_ts) else { return Ok(Vec::new()) };
    let rows = sqlx::query(
        "SELECT state_id, start_ts, end_ts, app_name, window_title, keyframe_path, state_type, source_key, front_app, front_title \
         FROM screen_states \
         WHERE meeting_id = ?1 AND keyframe_path IS NOT NULL AND keyframe_path != '' AND (?2 IS NULL OR state_id = ?2) \
         ORDER BY start_ts ASC, state_id ASC",
    )
    .bind(meeting_id)
    .bind(only)
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
                photo: r.get::<Option<String>, _>("state_type").as_deref() == Some("photo"),
                whole_display: r.get::<Option<String>, _>("source_key").is_some_and(|k| k.starts_with("display:")),
                front_app: r.get("front_app"),
                front_title: r.get("front_title"),
            })
        })
        .collect())
}

/// A recording's screens and photos that still have their picture, in time
/// order.
pub async fn screens(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<Vec<Screen>, String> {
    pictures(pool, meeting_id, None).await
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

/// A Delete of anything in this recording (a screen, transcript text) is
/// still in its undo window: its rows and files are still in place, so
/// nothing is offered or sent until it is committed or undone.
async fn delete_waiting(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<bool, String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    crate::study::has_pending_delete(&mut conn, meeting_id).await
}

/// One picture a click would send, for the confirmation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PlanItem {
    pub state_id: String,
    pub ms: i64,
    /// "Slide", "Document", "Photo"…: which prompt of the library it gets
    pub kind: &'static str,
    pub app_name: Option<String>,
    /// The picture's file, so the confirmation can show it
    pub image_path: String,
    /// A capture of a whole display: it shows everything that was on it
    pub whole_display: bool,
}

/// What Read the screens would do right now. Nothing is sent to make it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Plan {
    /// The pictures one click would send, in time order
    pub send: Vec<PlanItem>,
    /// Pictures of this recording that may be read (private ones aren't counted)
    pub total: usize,
    /// Of those, how many the AI has already looked at
    pub already: usize,
    /// Of those, how many a later click can send (not read, not in `send`)
    pub more: usize,
    /// Pictures that are never sent because they look private
    pub left_out: usize,
    /// Where the pictures would go (the host), when an image model is set up
    pub destination: Option<String>,
    pub model: Option<String>,
    /// The endpoint is on this Mac or the local network
    pub local: bool,
    /// Why it can't run now, in words for the user; None when it can
    pub blocked: Option<String>,
    /// A run for this recording is under way
    pub running: bool,
}

/// Where pictures would go right now.
#[derive(Debug, Clone, PartialEq, Default)]
struct Destination {
    host: Option<String>,
    model: Option<String>,
    local: bool,
    /// Why no picture can go there
    blocked: Option<String>,
}

/// The server a base URL names, as the confirmation shows it and as it is
/// compared before every picture: the host, with the port when the URL
/// gives one ("localhost:11434" and "localhost:1234" are two servers).
fn host_of(base_url: &str) -> Option<String> {
    let url = url::Url::parse(base_url.trim()).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}:{}", host, port),
        None => host.to_string(),
    })
}

/// Where pictures would go, without the side effects of resolving a
/// request (no consent prompt is raised by looking).
fn destination() -> Destination {
    let cfg = crate::ai::config::snapshot();
    let Some(sel) = cfg.selection(Kind::Vision) else {
        return Destination { blocked: Some(NEEDS_IMAGE_MODEL.to_string()), ..Default::default() };
    };
    if sel.provider == "apple" {
        return Destination { local: true, blocked: Some(APPLE_CANT.to_string()), ..Default::default() };
    }
    let host = cfg.base_url(&sel.provider).and_then(|u| host_of(&u));
    let blocked = host.is_none().then(|| NEEDS_IMAGE_MODEL.to_string());
    Destination { host, model: Some(sel.model), local: cfg.is_local(&sel.provider), blocked }
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
                kind: s.kind().label(),
                app_name: s.app_name.clone(),
                image_path: s.keyframe_path.clone(),
                whole_display: s.whole_display,
            }
        })
        .collect();
    let readable: Vec<&Screen> = all.iter().filter(|s| !s.private()).collect();
    let total = readable.len();
    let read = readable.iter().filter(|s| already.contains(&s.state_id)).count();
    let dest = destination();
    // The reason that is true of the recording comes before the one about
    // the AI settings: with nothing left to read there is no model to set up
    let blocked = if send.is_empty() && total > 0 {
        Some(NOTHING_TO_READ.to_string())
    } else if delete_waiting(pool, meeting_id).await? {
        Some(DELETE_WAITING.to_string())
    } else {
        dest.blocked
    };
    Ok(Plan {
        more: total - read - send.len(),
        send,
        total,
        already: read,
        left_out: all.len() - total,
        destination: dest.host,
        model: dest.model,
        local: dest.local,
        blocked,
        running: false,
    })
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
/// answering) and no Delete of this recording is waiting. One statement, so
/// nothing can land in between. An empty `text` records that the AI looked
/// and found nothing to keep.
///
/// The row carries no app or window name: it is the AI's text, and the
/// screen it belongs to already has them.
async fn store(pool: &Pool<Sqlite>, screen: &Screen, text: &str) -> Result<bool, String> {
    let hash: String = Sha256::digest(text.as_bytes()).iter().take(16).map(|b| format!("{:02x}", b)).collect();
    let done = sqlx::query(
        "INSERT INTO text_snapshots \
         (snapshot_id, episode_id, state_id, meeting_id, ts, text, text_hash, quality_score, source, word_count, app_name, window_title) \
         SELECT ?1, NULL, s.state_id, s.meeting_id, s.start_ts, ?2, ?3, 1.0, ?4, ?5, NULL, NULL \
         FROM screen_states s WHERE s.state_id = ?6 AND s.keyframe_path = ?7 \
         AND NOT EXISTS (SELECT 1 FROM text_snapshots t WHERE t.state_id = s.state_id AND t.source = ?4) \
         AND NOT EXISTS (SELECT 1 FROM redactions r WHERE r.meeting_id = s.meeting_id AND r.action = 'delete' \
                         AND r.pending_payload IS NOT NULL)",
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

/// What the user confirmed: the pictures they were shown and left ticked,
/// and where the confirmation said they would go.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Confirmed {
    pub state_ids: Vec<String>,
    /// The host the confirmation named
    pub destination: String,
    pub model: String,
}

/// Sends one picture with its prompt. Abstracted so tests never reach an
/// AI. Private to this file: the only one that sends is `commands::Live`,
/// and only `commands::read_screens` (the confirmation's button) runs it.
#[async_trait::async_trait]
trait Reader: Send + Sync {
    /// Where a picture would go right now: (host, model). None when no
    /// picture can be sent. For stopping early; `read` is what enforces it.
    fn destination(&self) -> Option<(String, String)>;
    /// Send one picture to `named` (host, model) and nowhere else. The
    /// destination is compared on the very target the request is made
    /// with; if it isn't `named`, nothing is sent (`ReadError::Changed`).
    async fn read(&self, image_path: &str, prompt: &str, named: &(String, String)) -> Result<String, ReadError>;
}

/// Why one picture wasn't read.
#[derive(Debug, Clone)]
enum ReadError {
    /// The AI settings no longer name the confirmed destination. Nothing was sent.
    Changed,
    /// The picture couldn't be prepared on this Mac. Nothing was sent.
    Picture,
    /// The request was made and the endpoint refused or failed
    Ai(AiError),
}

/// How one click went.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Summary {
    /// Screens whose reading was kept
    pub read: usize,
    /// Screens the AI answered about with nothing to keep (private, or empty)
    pub skipped: usize,
    /// Screens that weren't sent, or whose answer wasn't kept, because they
    /// were removed, changed or read meanwhile
    pub gone: usize,
    /// Screens whose answer couldn't be used (cut off, or not in the form
    /// asked for). They stay unread and can be sent again.
    pub failed: usize,
    /// Why the run ended before the last screen, as the AI error or a
    /// sentence; None when it reached the end
    pub stopped: Option<String>,
    /// The time (from the start of the recording) of the screen it stopped
    /// at, so the user can leave that one out and go on
    pub stopped_at_ms: Option<i64>,
    /// The user stopped it
    pub cancelled: bool,
}

/// The confirmed screen as it is now, if it may still be sent: it belongs
/// to this recording, its picture is on disk, it isn't private, and the AI
/// hasn't looked at it.
async fn still_sendable(pool: &Pool<Sqlite>, meeting_id: &str, state_id: &str) -> Result<Option<Screen>, String> {
    let Some(screen) = pictures(pool, meeting_id, Some(state_id)).await?.into_iter().next() else { return Ok(None) };
    if screen.private() || !screen.has_picture() {
        return Ok(None);
    }
    let read: Option<i64> = sqlx::query_scalar("SELECT 1 FROM text_snapshots WHERE state_id = ?1 AND source = ?2 LIMIT 1")
        .bind(state_id)
        .bind(SOURCE)
        .fetch_optional(pool)
        .await
        .map_err(err("Couldn't read the screens"))?;
    Ok(read.is_none().then_some(screen))
}

/// Send the confirmed screens, one request each, and keep what was read.
///
/// Exactly `confirmed.state_ids` are considered, in that order: the list is
/// never worked out again, so a mark, a synced photo or an earlier run
/// can't change what goes. Before each one: the user hasn't stopped the
/// run, the destination is still the one confirmed, no Delete is waiting,
/// and the screen may still be sent.
///
/// An AI error on the first request is returned as the error (nothing was
/// kept; for consent and Pro nothing was sent either, and the caller may
/// ask and call again with the same confirmation). After that, an error
/// ends the run with `stopped` set, and what was read is kept: a partly
/// completed run is never repeated by itself.
async fn read_screens(
    reader: &dyn Reader,
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    confirmed: &Confirmed,
    cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Send + Sync),
) -> Result<Summary, String> {
    let mut ids: Vec<&str> = Vec::new();
    for id in &confirmed.state_ids {
        if !ids.contains(&id.as_str()) {
            ids.push(id);
        }
    }
    if ids.is_empty() {
        return Err(NONE_CHOSEN.into());
    }
    if ids.len() > MAX_SCREENS {
        return Err(TOO_MANY.into());
    }
    let recording: RecordingKind = RecordingKind::from_stored(
        sqlx::query_scalar::<_, Option<String>>("SELECT recording_kind FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .map_err(err("Couldn't read the recording"))?
            .flatten()
            .as_deref(),
    );
    let named = (confirmed.destination.clone(), confirmed.model.clone());
    let mut summary = Summary::default();
    // Requests made to the endpoint so far, and how many of them it answered
    let mut requests = 0usize;
    let mut answered = 0usize;
    // The last request failed in a way that may be that picture's own problem
    let mut doubtful = false;
    for (n, id) in ids.iter().enumerate() {
        progress(n, ids.len());
        if cancel.load(Ordering::SeqCst) {
            summary.cancelled = true;
            break;
        }
        // Why nothing (more) is sent, when something stands in the way
        let refusal = if reader.destination().as_ref() != Some(&named) {
            Some(DESTINATION_CHANGED)
        } else if delete_waiting(pool, meeting_id).await? {
            Some(DELETE_WAITING)
        } else {
            None
        };
        if let Some(why) = refusal {
            if requests == 0 {
                return Err(why.into());
            }
            summary.stopped = Some(why.into());
            break;
        }
        let Some(screen) = still_sendable(pool, meeting_id, id).await? else {
            summary.gone += 1;
            continue;
        };
        let kind = screen.kind();
        let prompt = screen_prompts::prompt_for(kind, recording);
        let result = match reader.read(&screen.keyframe_path, &prompt, &named).await {
            // Nothing was sent for this picture: it stays unread, the run goes on
            Err(ReadError::Picture) => {
                summary.failed += 1;
                continue;
            }
            // The settings changed between the check above and the send:
            // the send didn't happen
            Err(ReadError::Changed) => {
                if requests == 0 {
                    return Err(DESTINATION_CHANGED.into());
                }
                summary.stopped = Some(DESTINATION_CHANGED.into());
                break;
            }
            Ok(raw) => Ok(raw),
            Err(ReadError::Ai(e)) => Err(e),
        };
        requests += 1;
        match result {
            Ok(raw) => {
                answered += 1;
                doubtful = false;
                match parse(&raw, kind) {
                    // The AI answered: a reading, or nothing to keep
                    Ok(reading) => {
                        let text = reading.as_ref().map(to_text).unwrap_or_default();
                        if !store(pool, &screen, &text).await? {
                            summary.gone += 1;
                        } else if text.is_empty() {
                            summary.skipped += 1;
                        } else {
                            summary.read += 1;
                        }
                    }
                    // Not in the form asked for: unread, and it can be sent again
                    Err(_) => summary.failed += 1,
                }
            }
            // Cut off: this screen's problem, not the endpoint's
            Err(AiError::Truncated) => summary.failed += 1,
            // The endpoint has read other pictures in this run and rejects
            // this one (too large for it, filtered, an empty answer): most
            // likely the picture. It stays unread and the run goes on, once:
            // a second rejection in a row ends the run.
            Err(AiError::Other(_)) if answered > 0 && !doubtful => {
                summary.failed += 1;
                doubtful = true;
            }
            // The endpoint refused or failed: the next picture would meet the
            // same, so none is sent. This screen stays unread.
            Err(e) => {
                if requests == 1 {
                    return Err(e.to_string());
                }
                summary.stopped = Some(e.to_string());
                summary.stopped_at_ms = Some(screen.ms);
                break;
            }
        }
    }
    progress(ids.len(), ids.len());
    Ok(summary)
}

/// Remove what was read from one screen. The screen is then unread and can
/// be sent again. For a reading that is wrong, or that a model made up.
pub async fn forget(pool: &Pool<Sqlite>, meeting_id: &str, state_id: &str) -> Result<bool, String> {
    let done = sqlx::query(
        "DELETE FROM text_snapshots WHERE source = ?1 AND state_id = ?2 \
         AND state_id IN (SELECT state_id FROM screen_states WHERE meeting_id = ?3)",
    )
    .bind(SOURCE)
    .bind(state_id)
    .bind(meeting_id)
    .execute(pool)
    .await
    .map_err(err("Couldn't remove what was read"))?;
    Ok(done.rows_affected() > 0)
}

// ═══════════════════════════════════════════════════════════════════════════
// Commands
// ═══════════════════════════════════════════════════════════════════════════

pub mod commands {
    use super::*;
    use crate::ai::{Msg, Opts};
    use crate::AppState;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tauri::{AppHandle, Emitter, State};

    /// `{meetingId, done, total}` while screens are being read, then
    /// `{meetingId, finished: true}` once, however the run ended.
    pub const PROGRESS_EVENT: &str = "screens_progress";

    /// Recordings whose screens are being read (one run each), with the
    /// flag that stops the run.
    static RUNNING: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

    struct RunGuard(String);
    impl Drop for RunGuard {
        fn drop(&mut self) {
            RUNNING.lock().remove(&self.0);
        }
    }

    /// The user's image model: the only sender of a picture in the app.
    /// `ai::complete_vision` checks Pro, consent and the endpoint policy,
    /// and refuses Apple's on-device model.
    struct Live;

    #[async_trait::async_trait]
    impl Reader for Live {
        fn destination(&self) -> Option<(String, String)> {
            let d = super::destination();
            if d.blocked.is_some() {
                return None;
            }
            Some((d.host?, d.model?))
        }

        async fn read(&self, image_path: &str, prompt: &str, named: &(String, String)) -> Result<String, ReadError> {
            let path = image_path.to_string();
            // Turned upright, at most 1568 px on its long edge, JPEG: what
            // leaves is a new encoding, never the file
            let (mime, b64) = tokio::task::spawn_blocking(move || crate::vlm_client::encode_image_for_upload(&path))
                .await
                .map_err(|_| ReadError::Picture)?
                .map_err(|_| ReadError::Picture)?;
            // Resolved, compared and sent on one target: only to `named`
            let (host, model) = named;
            crate::ai::complete_vision_if(
                vec![Msg::user_with_image(prompt.to_string(), &mime, b64)],
                Opts { max_tokens: READ_MAX_TOKENS, temperature: Some(0.1) },
                |base_url, to_model| host_of(base_url).as_deref() == Some(host.as_str()) && to_model == model,
            )
            .await
            .map_err(ReadError::Ai)?
            .ok_or(ReadError::Changed)
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
        plan.running = RUNNING.lock().contains_key(&meeting_id);
        if recording_meeting(&state).as_deref() == Some(meeting_id.as_str()) {
            plan.blocked = Some(STILL_RECORDING.to_string());
        } else if plan.running {
            plan.blocked = Some(ALREADY_RUNNING.to_string());
        }
        Ok(plan)
    }

    /// Read the screens (the confirmation's button). `confirmed` is what the
    /// user was shown and left ticked, and where it said they would go;
    /// nothing else is sent, and nowhere else.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn read_screens(
        app: AppHandle,
        state: State<'_, AppState>,
        meeting_id: String,
        confirmed: Confirmed,
    ) -> Result<Summary, String> {
        if recording_meeting(&state).as_deref() == Some(meeting_id.as_str()) {
            return Err(STILL_RECORDING.into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = RUNNING.lock();
            if running.contains_key(&meeting_id) {
                return Err(ALREADY_RUNNING.into());
            }
            running.insert(meeting_id.clone(), cancel.clone());
        }
        let _guard = RunGuard(meeting_id.clone());
        let emit = {
            let app = app.clone();
            let id = meeting_id.clone();
            move |done: usize, total: usize| {
                let _ = app.emit(PROGRESS_EVENT, serde_json::json!({ "meetingId": id, "done": done, "total": total }));
            }
        };
        let outcome = super::read_screens(&Live, state.database.pool(), &meeting_id, &confirmed, &cancel, &emit).await;
        // However it ended: a view that didn't start this run (the user left
        // and came back) learns here that it is over
        let _ = app.emit(PROGRESS_EVENT, serde_json::json!({ "meetingId": meeting_id, "finished": true }));
        outcome
    }

    /// Remove what was read from one screen, so it can be read again.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn forget_screen_reading(state: State<'_, AppState>, meeting_id: String, state_id: String) -> Result<bool, String> {
        if RUNNING.lock().contains_key(&meeting_id) {
            return Err(ALREADY_RUNNING.into());
        }
        forget(state.database.pool(), &meeting_id, &state_id).await
    }

    /// Stop reading this recording's screens. The request under way
    /// finishes (its picture has already left); no other picture is sent.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn stop_screen_reading(meeting_id: String) -> Result<bool, String> {
        Ok(match RUNNING.lock().get(&meeting_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        })
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
        Screen {
            state_id: id.into(),
            ms,
            end_ms: None,
            app_name: Some(app.into()),
            window_title: None,
            keyframe_path: format!("/tmp/{id}.png"),
            photo: false,
            whole_display: false,
            front_app: None,
            front_title: None,
        }
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
    fn a_screen_is_read_once_and_a_private_one_never() {
        let mut screens: Vec<Screen> = (0..8).map(|n| screen(&format!("s{n}"), n * 60_000, "Keynote")).collect();
        screens[1].app_name = Some("noFriction".into());
        screens[3].keyframe_path = " ".into();
        screens[5].app_name = Some("Mail".into());
        screens[5].window_title = Some("Re: slides.pptx".into());
        screens[6].app_name = Some("1Password 8".into());
        screens[7].app_name = Some("Visual Studio Code".into());
        screens[7].window_title = Some("server.key".into());
        let already: HashSet<String> = ["s0".to_string()].into_iter().collect();
        // Marks on the private ones don't bring them in
        assert_eq!(choose(&screens, &[0, 60_000, 5 * 60_000, 6 * 60_000, 7 * 60_000], &already, 12), vec![2, 4]);
        // A mark after a screen ended picks nothing for it
        let mut ended = vec![screen("a", 0, "Keynote")];
        ended[0].end_ms = Some(10_000);
        assert_eq!(choose(&ended, &[600_000], &HashSet::new(), 1), vec![0], "it is still chosen as one of the rest");
        assert_eq!(choose(&ended, &[600_000], &["a".to_string()].into_iter().collect(), 1), Vec::<usize>::new());
        // A photo is read as a photo whatever app name it carries, and is never "private" by a name
        let mut photo = screen("p", 0, "Mail");
        photo.photo = true;
        assert_eq!(photo.kind(), ScreenKind::Photo);
        assert!(!photo.private());
        assert_eq!(screen("k", 0, "Keynote").kind(), ScreenKind::Slide);
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
        // So does the prompt for a photo; without a kind it stays a photo
        assert_eq!(parse(r#"{"kind": "whiteboard", "title": "Pump"}"#, ScreenKind::Photo).unwrap().unwrap().kind, ScreenKind::Whiteboard);
        assert_eq!(to_text(&parse(r#"{"title": "Pump"}"#, ScreenKind::Photo).unwrap().unwrap()), "Photo: Pump");
        // A known kind is kept whatever the answer calls it
        assert_eq!(parse(r#"{"kind": "chart", "title": "T"}"#, ScreenKind::Document).unwrap().unwrap().kind, ScreenKind::Document);
        // Skip, or nothing in it: nothing is kept
        assert_eq!(parse(r#"{"skip": true, "title": "Inbox"}"#, ScreenKind::WebPage).unwrap(), None);
        assert_eq!(parse(r#"{"title": null, "points": [], "text": "null"}"#, ScreenKind::Slide).unwrap(), None);
        // Skip written as a word or a number is still skip
        for skip in [r#""true""#, r#""True""#, r#""yes""#, "1", r#""skip""#] {
            let raw = format!(r#"{{"skip": {skip}, "title": "Inbox", "points": ["A message from someone"]}}"#);
            assert_eq!(parse(&raw, ScreenKind::WebPage).unwrap(), None, "{skip}");
        }
        for keep in ["false", r#""false""#, r#""no""#, "0", "null"] {
            let raw = format!(r#"{{"skip": {keep}, "title": "Lecture 7"}}"#);
            assert!(parse(&raw, ScreenKind::Slide).unwrap().is_some(), "{keep}");
        }
        assert!(parse("I can't read this.", ScreenKind::Slide).is_err());
        // Caps
        let many = format!(r#"{{"points":[{}]}}"#, (0..30).map(|n| format!("\"point {n}\"")).collect::<Vec<_>>().join(","));
        assert_eq!(parse(&many, ScreenKind::Slide).unwrap().unwrap().points.len(), MAX_POINTS);
    }

    #[test]
    fn the_destination_is_the_host_and_its_port() {
        assert_eq!(host_of("https://api.example.com/v1").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("https://api.example.com:443/v1").as_deref(), Some("api.example.com"), "the default port isn't shown");
        assert_eq!(host_of("http://localhost:11434/v1").as_deref(), Some("localhost:11434"));
        assert_ne!(host_of("http://localhost:11434/v1"), host_of("http://localhost:1234/v1"), "two servers on one Mac");
        assert_eq!(host_of(" http://192.168.1.20:8080 ").as_deref(), Some("192.168.1.20:8080"));
        assert_eq!(host_of("not a url"), None);
        assert_eq!(host_of(""), None);
    }

    const HOST: &str = "ai.example.com";
    const MODEL: &str = "vision-1";

    struct Mock {
        answers: Mutex<Vec<Result<String, AiError>>>,
        sent: Mutex<Vec<(String, String)>>,
        /// Where a picture would go; changed by a test to stand for the
        /// user editing their AI settings
        dest: Mutex<Option<(String, String)>>,
        /// SQL run once, during the first request
        meanwhile: Mutex<Option<(Arc<DatabaseManager>, String)>>,
        /// Set during the first request: the user pressed Stop
        stop: Mutex<Option<Arc<AtomicBool>>>,
        /// The destination after the first request
        then_dest: Mutex<Option<Option<(String, String)>>>,
        /// (n, destination): once n pictures have been sent, the destination
        /// changes after the run's own check and before the next send (the
        /// user edits the settings at that moment)
        switch_at_send: Mutex<Option<(usize, Option<(String, String)>)>>,
        /// The next picture can't be prepared on this Mac
        unreadable_file: Mutex<bool>,
    }

    impl Mock {
        fn new(answers: Vec<Result<&str, AiError>>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().map(|a| a.map(str::to_string)).collect()),
                sent: Mutex::new(Vec::new()),
                dest: Mutex::new(Some((HOST.into(), MODEL.into()))),
                meanwhile: Mutex::new(None),
                stop: Mutex::new(None),
                then_dest: Mutex::new(None),
                switch_at_send: Mutex::new(None),
                unreadable_file: Mutex::new(false),
            }
        }

        fn sent_files(&self) -> Vec<String> {
            self.sent.lock().iter().map(|(p, _)| p.rsplit('/').next().unwrap_or("").to_string()).collect()
        }
    }

    #[async_trait::async_trait]
    impl Reader for Mock {
        fn destination(&self) -> Option<(String, String)> {
            self.dest.lock().clone()
        }

        async fn read(&self, image_path: &str, prompt: &str, named: &(String, String)) -> Result<String, ReadError> {
            if std::mem::take(&mut *self.unreadable_file.lock()) {
                return Err(ReadError::Picture);
            }
            let due = self.switch_at_send.lock().as_ref().is_some_and(|(n, _)| *n == self.sent.lock().len());
            if due {
                if let Some((_, next)) = self.switch_at_send.lock().take() {
                    *self.dest.lock() = next;
                }
            }
            // Like the live sender: the request is made only on the named target
            if self.dest.lock().as_ref() != Some(named) {
                return Err(ReadError::Changed);
            }
            self.sent.lock().push((image_path.to_string(), prompt.to_string()));
            let meanwhile = self.meanwhile.lock().take();
            if let Some((db, sql)) = meanwhile {
                sqlx::query(&sql).execute(db.pool()).await.unwrap();
            }
            if let Some(flag) = self.stop.lock().take() {
                flag.store(true, Ordering::SeqCst);
            }
            if let Some(next) = self.then_dest.lock().take() {
                *self.dest.lock() = next;
            }
            let mut a = self.answers.lock();
            let answer = if a.len() > 1 { a.remove(0) } else { a[0].clone() };
            answer.map_err(ReadError::Ai)
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
    const SAID: &str = "This is what was said at screen";

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
            db.add_transcript_at("m1", &format!("{SAID} {i}."), None, true, 0.9, at + chrono::Duration::seconds(5))
                .await
                .unwrap();
        }
        crate::markers::add(db.pool(), "m1", start + chrono::Duration::seconds(130), Some("note"), Some("pump"), false).await.unwrap();
        Fx { db: Arc::new(db), dir }
    }

    fn quiet() -> impl Fn(usize, usize) + Send + Sync {
        |_, _| {}
    }

    fn confirmed(ids: &[&str]) -> Confirmed {
        Confirmed { state_ids: ids.iter().map(|s| s.to_string()).collect(), destination: HOST.into(), model: MODEL.into() }
    }

    async fn run(m: &Mock, f: &Fx, ids: &[&str]) -> Result<Summary, String> {
        read_screens(m, f.db.pool(), "m1", &confirmed(ids), &AtomicBool::new(false), &quiet()).await
    }

    async fn looked(f: &Fx) -> Vec<String> {
        sqlx::query_scalar("SELECT state_id FROM text_snapshots WHERE source = 'screen_reading' ORDER BY state_id")
            .fetch_all(f.db.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn one_click_sends_the_screens_shown_and_keeps_what_was_read() {
        let f = fx(3).await;
        let pool = f.db.pool();
        let p = plan(pool, "m1").await.unwrap();
        assert_eq!((p.send.len(), p.total, p.already, p.more, p.left_out), (3, 3, 0, 0, 0));
        assert_eq!(p.send[0].kind, "Slide");
        assert!(p.send[0].image_path.ends_with("s0.png"));

        let m = Mock::new(vec![Ok(SLIDE), Ok(r#"{"skip": true}"#), Ok(r#"{"points": []}"#)]);
        let s = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap();
        assert_eq!(s, Summary { read: 1, skipped: 2, ..Default::default() });
        let sent = m.sent.lock().clone();
        assert_eq!(m.sent_files(), vec!["s0.png", "s1.png", "s2.png"], "one picture per request, in the order shown");
        // The slide prompt for a class, and nothing of the recording with it:
        // no words that were said, no title, no app or window name, no id
        for (_, prompt) in &sent {
            assert!(prompt.contains("This is a presentation slide") && prompt.contains("a student's recording of a class"));
            for leak in [SAID, "Membrane transport", "Lecture 7", "Keynote", "pump", "s0", "m1"] {
                assert!(!prompt.contains(leak), "{leak}");
            }
        }
        assert_eq!(sent[0].1, sent[2].1, "the same fixed prompt for the same kind of screen");

        let kept = readings(pool, "m1").await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].state_id.as_str(), kept[0].ms), ("s0", 0));
        assert_eq!(kept[0].text, "Slide: The cell membrane\n- A phospholipid bilayer.");
        // The stored row is the AI's text only: no app or window name on it
        let (app, title): (Option<String>, Option<String>) =
            sqlx::query_as("SELECT app_name, window_title FROM text_snapshots WHERE state_id = 's0'").fetch_one(pool).await.unwrap();
        assert_eq!((app, title), (None, None));

        // Every screen has been looked at: nothing is offered or sent again
        let again = plan(pool, "m1").await.unwrap();
        assert_eq!((again.send.len(), again.already, again.more), (0, 3, 0));
        assert_eq!(again.blocked.as_deref(), Some(NOTHING_TO_READ));
        let m = Mock::new(vec![Ok(SLIDE)]);
        assert_eq!(run(&m, &f, &["s0", "s1", "s2"]).await.unwrap(), Summary { gone: 3, ..Default::default() });
        assert!(m.sent.lock().is_empty());
    }

    #[tokio::test]
    async fn only_the_confirmed_screens_go_whatever_changed_since() {
        // 30 screens; the user is shown 12 and unticks all but three
        let f = fx(30).await;
        let pool = f.db.pool();
        let p = plan(pool, "m1").await.unwrap();
        assert_eq!((p.send.len(), p.total, p.more), (MAX_SCREENS, 30, 18));
        let shown: Vec<String> = p.send.iter().map(|i| i.state_id.clone()).collect();
        assert!(shown.contains(&"s2".to_string()), "the screen at the typed note is in: {shown:?}");

        // Since they looked: a new mark, and another screen arrives
        let start: String = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = 'm1'").fetch_one(pool).await.unwrap();
        let start = parse_ts(&start).unwrap();
        crate::markers::add(pool, "m1", start + chrono::Duration::seconds(29 * 60 + 5), Some("important"), None, false).await.unwrap();
        let late = f.dir.join("late.png");
        std::fs::write(&late, b"png").unwrap();
        sqlx::query("INSERT INTO screen_states (state_id, meeting_id, start_ts, phash, keyframe_path) VALUES ('late', 'm1', ?, 'p', ?)")
            .bind((start + chrono::Duration::seconds(10)).to_rfc3339())
            .bind(late.to_str().unwrap())
            .execute(pool)
            .await
            .unwrap();
        assert_ne!(plan(pool, "m1").await.unwrap().send.iter().map(|i| i.state_id.clone()).collect::<Vec<_>>(), shown);

        let pick = [shown[0].as_str(), shown[5].as_str(), shown[11].as_str()];
        let m = Mock::new(vec![Ok(SLIDE)]);
        // Sent twice in the list by a broken caller: still once
        let s = run(&m, &f, &[pick[0], pick[1], pick[2], pick[0]]).await.unwrap();
        assert_eq!(s, Summary { read: 3, ..Default::default() });
        assert_eq!(m.sent_files(), pick.iter().map(|id| format!("{id}.png")).collect::<Vec<_>>());
        assert_eq!(looked(&f).await.len(), 3);

        // More than one click's worth, or none: refused before anything is sent
        let m = Mock::new(vec![Ok(SLIDE)]);
        let too_many: Vec<String> = (0..=MAX_SCREENS).map(|n| format!("s{n}")).collect();
        let refs: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert_eq!(run(&m, &f, &refs).await.unwrap_err(), TOO_MANY);
        assert_eq!(run(&m, &f, &[]).await.unwrap_err(), NONE_CHOSEN);
        // An id of another recording, or one that doesn't exist, sends nothing
        f.db.create_meeting("m2", "Another").await.unwrap();
        sqlx::query("INSERT INTO screen_states (state_id, meeting_id, start_ts, phash, keyframe_path) VALUES ('other', 'm2', ?, 'p', ?)")
            .bind(start.to_rfc3339())
            .bind(late.to_str().unwrap())
            .execute(pool)
            .await
            .unwrap();
        assert_eq!(run(&m, &f, &["other", "nope"]).await.unwrap(), Summary { gone: 2, ..Default::default() });
        assert!(m.sent.lock().is_empty());
    }

    #[tokio::test]
    async fn pictures_go_only_to_the_destination_the_confirmation_named() {
        let f = fx(3).await;
        // The AI settings changed between looking and pressing: nothing is sent
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.dest.lock() = Some(("elsewhere.example.net".into(), MODEL.into()));
        assert_eq!(run(&m, &f, &["s0", "s1", "s2"]).await.unwrap_err(), DESTINATION_CHANGED);
        let m2 = Mock::new(vec![Ok(SLIDE)]);
        *m2.dest.lock() = Some((HOST.into(), "another-model".into()));
        assert_eq!(run(&m2, &f, &["s0"]).await.unwrap_err(), DESTINATION_CHANGED);
        let m3 = Mock::new(vec![Ok(SLIDE)]);
        *m3.dest.lock() = None;
        assert_eq!(run(&m3, &f, &["s0"]).await.unwrap_err(), DESTINATION_CHANGED);
        assert!(m.sent.lock().is_empty() && m2.sent.lock().is_empty() && m3.sent.lock().is_empty());

        // They change while the run is under way: it stops there, and keeps what was read
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.then_dest.lock() = Some(Some(("elsewhere.example.net".into(), MODEL.into())));
        let s = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap();
        assert_eq!(s, Summary { read: 1, stopped: Some(DESTINATION_CHANGED.into()), ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s0.png"]);
        assert_eq!(looked(&f).await, vec!["s0"]);
    }

    #[tokio::test]
    async fn an_endpoint_error_stops_the_run_and_marks_nothing_read() {
        // The endpoint can't take a picture: one request, an error, nothing marked
        let f = fx(3).await;
        let m = Mock::new(vec![Err(AiError::Other("HTTP 400: this model does not support image input".into()))]);
        let e = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap_err();
        assert!(e.contains("does not support image input"), "{e}");
        assert_eq!(m.sent.lock().len(), 1, "the other pictures are not uploaded to an endpoint that just refused one");
        assert!(looked(&f).await.is_empty(), "a screen the AI never answered about is not read");
        assert_eq!(plan(f.db.pool(), "m1").await.unwrap().send.len(), 3, "all three can still be read");

        // Consent is refused before anything is sent: the same, as an error the caller can act on
        let m = Mock::new(vec![Err(AiError::ConsentRequired("custom".into()))]);
        let e = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap_err();
        assert!(e.starts_with("CONSENT_REQUIRED"), "{e}");
        assert_eq!(m.sent.lock().len(), 1);

        // Mid-run: the run ends with the reason, what was read is kept, and it is
        // reported as a result, so nothing repeats it
        let m = Mock::new(vec![Ok(SLIDE), Err(AiError::Unreachable("timed out".into())), Ok(SLIDE)]);
        let s = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap();
        assert_eq!((s.read, s.skipped, s.failed, s.gone), (1, 0, 0, 0));
        assert!(s.stopped.as_deref().is_some_and(|w| w.contains("timed out")), "{:?}", s.stopped);
        assert_eq!(m.sent.lock().len(), 2, "nothing more is sent after the failure");
        assert_eq!(looked(&f).await, vec!["s0"]);
        // The rest can be read on the next click
        let ids: Vec<String> = plan(f.db.pool(), "m1").await.unwrap().send.into_iter().map(|i| i.state_id).collect();
        assert_eq!(ids, vec!["s1", "s2"]);
    }

    #[tokio::test]
    async fn an_answer_that_cant_be_used_leaves_its_screen_unread() {
        let f = fx(4).await;
        let m = Mock::new(vec![Ok("Sorry, I can't help with that."), Err(AiError::Truncated), Ok(r#"{"title": "Half a"#), Ok(SLIDE)]);
        let s = run(&m, &f, &["s0", "s1", "s2", "s3"]).await.unwrap();
        assert_eq!(s, Summary { read: 1, failed: 3, ..Default::default() });
        assert_eq!(m.sent.lock().len(), 4, "one screen's bad answer doesn't end the run");
        assert_eq!(looked(&f).await, vec!["s3"]);
        let ids: Vec<String> = plan(f.db.pool(), "m1").await.unwrap().send.into_iter().map(|i| i.state_id).collect();
        assert_eq!(ids, vec!["s0", "s1", "s2"], "they are offered again");
    }

    #[tokio::test]
    async fn nothing_is_offered_or_sent_while_a_delete_waits() {
        let f = fx(3).await;
        let pool = f.db.pool();
        let wait = "INSERT INTO redactions (id, meeting_id, kind, action, created_at, item_count, pending_payload) \
                    VALUES ('d1', 'm1', 'screen', 'delete', '2026-11-13T00:00:00Z', 1, '{\"Screens\":{\"ids\":[\"s1\"]}}')";
        sqlx::query(wait).execute(pool).await.unwrap();
        let p = plan(pool, "m1").await.unwrap();
        assert_eq!(p.blocked.as_deref(), Some(DELETE_WAITING));
        let m = Mock::new(vec![Ok(SLIDE)]);
        assert_eq!(run(&m, &f, &["s0", "s1", "s2"]).await.unwrap_err(), DELETE_WAITING);
        assert!(m.sent.lock().is_empty());
        sqlx::query("DELETE FROM redactions").execute(pool).await.unwrap();

        // A Delete that starts while the AI is answering: that answer isn't
        // kept, and no other picture is sent
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.meanwhile.lock() = Some((f.db.clone(), wait.into()));
        let s = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap();
        assert_eq!(s, Summary { gone: 1, stopped: Some(DELETE_WAITING.into()), ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s0.png"]);
        assert!(looked(&f).await.is_empty());
    }

    #[tokio::test]
    async fn stop_ends_the_run_before_the_next_picture() {
        let f = fx(3).await;
        let cancel = Arc::new(AtomicBool::new(false));
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.stop.lock() = Some(cancel.clone());
        let s = read_screens(&m, f.db.pool(), "m1", &confirmed(&["s0", "s1", "s2"]), &cancel, &quiet()).await.unwrap();
        assert_eq!(s, Summary { read: 1, cancelled: true, ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s0.png"], "the request under way finishes; no other picture leaves");
        // Stopped before it began: nothing at all
        let m = Mock::new(vec![Ok(SLIDE)]);
        let s = read_screens(&m, f.db.pool(), "m1", &confirmed(&["s1"]), &AtomicBool::new(true), &quiet()).await.unwrap();
        assert_eq!(s, Summary { cancelled: true, ..Default::default() });
        assert!(m.sent.lock().is_empty());
    }

    #[tokio::test]
    async fn private_screens_and_photos() {
        let f = fx(4).await;
        let pool = f.db.pool();
        // s1 is a mail window, s2 a terminal; s3 is a photo from the iPhone
        sqlx::query("UPDATE screen_states SET app_name = 'Mail', window_title = 'Re: slides.pptx' WHERE state_id = 's1'").execute(pool).await.unwrap();
        sqlx::query("UPDATE screen_states SET app_name = 'Terminal', window_title = 'zsh' WHERE state_id = 's2'").execute(pool).await.unwrap();
        sqlx::query("UPDATE screen_states SET app_name = NULL, window_title = NULL, state_type = 'photo', source_key = 'sync' WHERE state_id = 's3'")
            .execute(pool)
            .await
            .unwrap();
        let p = plan(pool, "m1").await.unwrap();
        let offered: Vec<(&str, &str)> = p.send.iter().map(|i| (i.state_id.as_str(), i.kind)).collect();
        assert_eq!(offered, vec![("s0", "Slide"), ("s3", "Photo")]);
        assert_eq!((p.total, p.left_out, p.more), (2, 2, 0));

        // A caller that names a private screen anyway: it is not sent
        let m = Mock::new(vec![Ok(SLIDE), Ok(r#"{"kind": "whiteboard", "title": "The pump", "points": ["3 Na+ out, 2 K+ in"]}"#)]);
        let s = run(&m, &f, &["s0", "s1", "s2", "s3"]).await.unwrap();
        assert_eq!(s, Summary { read: 2, gone: 2, ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s0.png", "s3.png"]);
        let photo_prompt = m.sent.lock()[1].1.clone();
        assert!(photo_prompt.contains("This is a photo taken during the recording") && photo_prompt.contains("Ignore people in a photo"));
        let kept = readings(pool, "m1").await.unwrap();
        assert_eq!(kept[1].text, "Whiteboard: The pump\n- 3 Na+ out, 2 K+ in");
        // Everything that may be read has been: the private ones don't keep the button alive
        let after = plan(pool, "m1").await.unwrap();
        assert_eq!((after.send.len(), after.already, after.total, after.more, after.left_out), (0, 2, 2, 0, 2));
    }

    #[tokio::test]
    async fn settings_changed_between_the_check_and_the_send_send_nothing() {
        // The run's own check passes, then the settings change before the
        // request is made: the request isn't made
        let f = fx(3).await;
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.switch_at_send.lock() = Some((0, Some(("elsewhere.example.net".into(), MODEL.into()))));
        assert_eq!(run(&m, &f, &["s0", "s1", "s2"]).await.unwrap_err(), DESTINATION_CHANGED);
        assert!(m.sent.lock().is_empty());
        assert!(looked(&f).await.is_empty());

        // The same for the second picture of a run: the first is kept, the
        // second isn't sent, and the run ends there
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.switch_at_send.lock() = Some((1, Some((HOST.into(), "another-model".into()))));
        let s = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap();
        assert_eq!(s, Summary { read: 1, stopped: Some(DESTINATION_CHANGED.into()), ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s0.png"]);
        assert_eq!(looked(&f).await, vec!["s0"]);
    }

    #[tokio::test]
    async fn one_bad_picture_doesnt_end_the_run_but_a_failing_endpoint_does() {
        // A picture that can't be prepared on this Mac: nothing is sent for
        // it, it stays unread, and the others are read
        let f = fx(3).await;
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.unreadable_file.lock() = true;
        let s = run(&m, &f, &["s0", "s1", "s2"]).await.unwrap();
        assert_eq!(s, Summary { read: 2, failed: 1, ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s1.png", "s2.png"]);
        assert_eq!(looked(&f).await, vec!["s1", "s2"]);

        // The endpoint read a picture, then rejects one: that one stays
        // unread and the run goes on
        let f = fx(4).await;
        let reject = || Err(AiError::Other("HTTP 400: image rejected".into()));
        let m = Mock::new(vec![Ok(SLIDE), reject(), Ok(SLIDE), Ok(SLIDE)]);
        let s = run(&m, &f, &["s0", "s1", "s2", "s3"]).await.unwrap();
        assert_eq!(s, Summary { read: 3, failed: 1, ..Default::default() });
        assert_eq!(looked(&f).await, vec!["s0", "s2", "s3"]);

        // Two rejections in a row: it is the endpoint. The run ends and says where
        let f = fx(4).await;
        let m = Mock::new(vec![Ok(SLIDE), reject(), reject(), Ok(SLIDE)]);
        let s = run(&m, &f, &["s0", "s1", "s2", "s3"]).await.unwrap();
        assert_eq!((s.read, s.failed), (1, 1));
        assert!(s.stopped.as_deref().is_some_and(|w| w.contains("image rejected")), "{:?}", s.stopped);
        assert_eq!(s.stopped_at_ms, Some(120_000), "the screen it stopped at");
        assert_eq!(m.sent.lock().len(), 3, "the fourth picture isn't sent");
        assert_eq!(looked(&f).await, vec!["s0"]);
    }

    #[tokio::test]
    async fn an_echo_of_the_instructions_is_not_a_reading_and_a_reading_can_be_removed() {
        let f = fx(2).await;
        let pool = f.db.pool();
        // A model that never saw the picture hands back the example
        let echo = r#"{"skip": false, "kind": "slide", "title": "the title, or null", "points": ["one statement from the screen"]}"#;
        let s = run(&Mock::new(vec![Ok(echo), Ok(SLIDE)]), &f, &["s0", "s1"]).await.unwrap();
        assert_eq!(s, Summary { read: 1, failed: 1, ..Default::default() });
        assert_eq!(looked(&f).await, vec!["s1"], "the echoed one stays unread");

        // What was read can be removed; the screen is then offered again
        assert!(!forget(pool, "m2", "s1").await.unwrap(), "not through another recording");
        assert_eq!(looked(&f).await, vec!["s1"]);
        assert!(forget(pool, "m1", "s1").await.unwrap());
        assert!(!forget(pool, "m1", "s1").await.unwrap());
        assert!(readings(pool, "m1").await.unwrap().is_empty());
        let ids: Vec<String> = plan(pool, "m1").await.unwrap().send.into_iter().map(|i| i.state_id).collect();
        assert_eq!(ids, vec!["s0", "s1"]);
        // Only readings are removed that way, never text captured from the screen
        sqlx::query(
            "INSERT INTO text_snapshots (snapshot_id, state_id, meeting_id, ts, text, text_hash, source, word_count) \
             SELECT 'ocr1', state_id, meeting_id, start_ts, 'captured text', 'h', 'ocr', 2 FROM screen_states WHERE state_id = 's0'",
        )
        .execute(pool)
        .await
        .unwrap();
        assert!(!forget(pool, "m1", "s0").await.unwrap());
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM text_snapshots WHERE snapshot_id = 'ocr1'").fetch_one(pool).await.unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn a_whole_display_is_judged_by_what_was_in_front() {
        let f = fx(4).await;
        let pool = f.db.pool();
        // All four were captures of the whole display (no app of their own)
        sqlx::query("UPDATE screen_states SET app_name = NULL, window_title = 'Built-in Retina Display', source_key = 'display:1'")
            .execute(pool)
            .await
            .unwrap();
        // In front: Keynote, Mail, a private browser window, and (an older capture) nothing recorded
        for (id, app, title) in [("s0", "Keynote", "Lecture 7"), ("s1", "Mail", "Inbox"), ("s2", "Safari", "Private Browsing")] {
            f.db.set_screen_state_front(id, app, title).await.unwrap();
        }
        let p = plan(pool, "m1").await.unwrap();
        let offered: Vec<(&str, &str, bool)> = p.send.iter().map(|i| (i.state_id.as_str(), i.kind, i.whole_display)).collect();
        // A whole display gets the prompt that lets the AI say what it is
        assert_eq!(offered, vec![("s0", "Screen", true), ("s3", "Screen", true)]);
        assert_eq!((p.total, p.left_out), (2, 2));
        // Named anyway by a caller: still not sent
        let m = Mock::new(vec![Ok(SLIDE)]);
        let s = run(&m, &f, &["s0", "s1", "s2", "s3"]).await.unwrap();
        assert_eq!(s, Summary { read: 2, gone: 2, ..Default::default() });
        assert_eq!(m.sent_files(), vec!["s0.png", "s3.png"]);
        // What was in front is never in what is sent
        for (_, prompt) in m.sent.lock().iter() {
            assert!(!prompt.contains("Keynote") && !prompt.contains("Lecture 7") && !prompt.contains("Built-in"));
        }
    }

    #[tokio::test]
    async fn a_reading_is_the_ais_words_and_nothing_else_reads_it_as_screen_text() {
        let f = fx(1).await;
        let pool = f.db.pool();
        // The AI writes an address and a distinctive word into its reading
        let answer = r#"{"kind":"slide","title":"Zymurgy handout","points":["See https://reading.example.org/handout for more."]}"#;
        assert_eq!(run(&Mock::new(vec![Ok(answer)]), &f, &["s0"]).await.unwrap().read, 1);
        assert_eq!(readings(pool, "m1").await.unwrap().len(), 1, "it is shown under On the screens");

        // Links: an address a model wrote is not a link that was on screen
        let links = crate::meeting_links::list_links(pool, "m1").await.unwrap();
        assert!(!format!("{links:?}").contains("reading.example.org"), "{links:?}");
        // The timeline, the accessibility view and the Obsidian export
        assert!(f.db.get_text_snapshots_by_meeting("m1").await.unwrap().is_empty());
        // Chat search over screen text
        let hits = f.db.search_activity_text(&["Zymurgy".to_string()], 10).await.unwrap();
        assert!(hits.is_empty(), "{hits:?}");

        // Text that really was captured from the screen is still read by all three
        sqlx::query(
            "INSERT INTO text_snapshots (snapshot_id, state_id, meeting_id, ts, text, text_hash, source, word_count) \
             SELECT 'ocr1', state_id, meeting_id, start_ts, 'Zymurgy notes at https://screen.example.org/page', 'h', 'ocr', 4 \
             FROM screen_states WHERE state_id = 's0'",
        )
        .execute(pool)
        .await
        .unwrap();
        assert!(format!("{:?}", crate::meeting_links::list_links(pool, "m1").await.unwrap()).contains("screen.example.org"));
        assert_eq!(f.db.get_text_snapshots_by_meeting("m1").await.unwrap().len(), 1);
        assert_eq!(f.db.search_activity_text(&["Zymurgy".to_string()], 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_reading_never_outlives_its_screen() {
        // The screen is deleted while the AI is answering: its reading isn't kept
        let f = fx(1).await;
        let pool = f.db.pool();
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.meanwhile.lock() = Some((f.db.clone(), "DELETE FROM screen_states WHERE state_id = 's0'".into()));
        assert_eq!(run(&m, &f, &["s0"]).await.unwrap(), Summary { gone: 1, ..Default::default() });
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM text_snapshots").fetch_one(pool).await.unwrap();
        assert_eq!(n, 0);

        // Its picture is replaced (a strike blanks it): not kept either
        let f = fx(1).await;
        let m = Mock::new(vec![Ok(SLIDE)]);
        *m.meanwhile.lock() = Some((f.db.clone(), "UPDATE screen_states SET keyframe_path = '/elsewhere.png' WHERE state_id = 's0'".into()));
        assert_eq!(run(&m, &f, &["s0"]).await.unwrap().gone, 1);

        // A kept reading is a text snapshot on the screen: deleting the screen takes it
        let f = fx(2).await;
        let pool = f.db.pool();
        run(&Mock::new(vec![Ok(SLIDE)]), &f, &["s0", "s1"]).await.unwrap();
        assert_eq!(readings(pool, "m1").await.unwrap().len(), 2);
        let (source, state): (String, String) =
            sqlx::query_as("SELECT source, state_id FROM text_snapshots ORDER BY ts LIMIT 1").fetch_one(pool).await.unwrap();
        assert_eq!((source.as_str(), state.as_str()), (SOURCE, "s0"));
        sqlx::query("DELETE FROM screen_states WHERE state_id = 's0'").execute(pool).await.unwrap();
        let left = readings(pool, "m1").await.unwrap();
        assert!(left.iter().all(|r| r.state_id != "s0"));
    }
}
