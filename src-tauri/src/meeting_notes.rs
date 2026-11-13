//! Notes for one recording (docs/TIMED_RECORDING_AND_NOTEBOOKS.md "Notes by
//! type", docs/design/FADELL_V2.md §6 "Notes generation").
//!
//! - **The whole recording.** The prompt gets the entire stored transcript,
//!   each line with its time. A transcript that doesn't fit beside the
//!   prompt and the answer is first condensed in parts (the study guide's
//!   map step, `study::condense_to_budget`); nothing is cut off the end.
//! - **Built on what the user did while listening.** The marks (★ ? ✎) and
//!   the notes they typed are listed with their times, and the model is
//!   asked to cover those first and to write, for each typed note, what was
//!   being said then (`your_notes`, stored as `note_details`).
//! - **One shape for every type.** Meeting, lecture and personal notes keep
//!   the same JSON (`summary`, `key_topics`, `decisions`, `action_items`,
//!   `participants`); the type only picks the prompt (recording_kind.rs).
//! - **AI.** Every request goes through `ai::complete_text` (the study
//!   module's `Completer`), so consent and, in the Mac App Store build, the
//!   Pro check apply unchanged.
//! - **Untrusted output.** The answer is parsed tolerantly and cleaned; an
//!   unusable answer is asked for once more. Notes made from a transcript
//!   that was edited meanwhile are not saved. Transcript text and model
//!   output are never logged, and never put in an error message.

use crate::database::DatabaseManager;
use crate::study::prompt::{self, StudyInput};
use crate::study::{Completer, GenError, Progress};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::SqliteConnection;
use sqlx::{Pool, Row, Sqlite};
use std::sync::Arc;
use uuid::Uuid;

pub const NO_TRANSCRIPT: &str = "This recording has no transcript to make notes from.";
pub const TRANSCRIPT_CHANGED: &str =
    "The transcript was edited while the notes were being made, so they weren't saved. Make notes again.";
pub const STILL_RECORDING: &str = "Stop the recording to make notes from it.";
pub const UNUSABLE: &str =
    "Couldn't make the notes: the AI's answer wasn't usable (asked twice). Try again, or choose a larger model in Settings → AI.";

/// Answer budget for the notes JSON, scaled down for small context windows.
pub const NOTES_MAX_TOKENS: u32 = crate::ai_client::REPORT_MAX_TOKENS;
/// Typed notes listed in the prompt (the rest still steer the condensing).
const MAX_PROMPT_NOTES: usize = 40;
/// A typed note as the prompt shows it.
const PROMPT_NOTE_CHARS: usize = 240;

/// Generated meeting notes structure
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GeneratedNotes {
    pub summary: String,
    pub key_topics: Vec<String>,
    pub decisions: Vec<Decision>,
    pub action_items: Vec<ActionItem>,
    pub participants: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub text: String,
    pub made_by: Option<String>,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ActionItem {
    pub task: String,
    pub assignee: Option<String>,
    pub due_date: Option<String>,
    pub priority: Option<String>,
}

/// What was being said when the user typed a note, written by the AI.
/// Stored in `meeting_notes.note_details` as a JSON list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteDetail {
    pub marker_id: String,
    pub detail: String,
}

/// A note the user typed while listening.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedNote {
    pub marker_id: String,
    pub ms: i64,
    pub text: String,
}

/// Everything the notes prompt is built from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NotesInput {
    /// Title, type, notebook, the transcript (speaker-prefixed when a line
    /// has one) and every marker, typed notes included: the condensing step
    /// keeps what was said near all of them.
    pub study: StudyInput,
    pub typed: Vec<TypedNote>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Schema (runs inside DatabaseManager::run_migrations on its one connection)
// ═══════════════════════════════════════════════════════════════════════════

/// `meeting_notes` is created in database.rs; this adds `note_details`.
/// It is AI output: redaction rewrites it with the other notes columns
/// (`redaction::AI_TABLES`).
pub async fn ensure_schema(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    crate::database::ensure_columns(conn, "meeting_notes", &[("note_details", "TEXT")]).await?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Input
// ═══════════════════════════════════════════════════════════════════════════

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

pub async fn load_input(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<NotesInput, String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    load_input_conn(&mut conn, meeting_id).await
}

pub async fn load_input_conn(conn: &mut SqliteConnection, meeting_id: &str) -> Result<NotesInput, String> {
    // Title, type, notebook, lines with times, markers (study.rs)
    let mut study = crate::study::load_input_conn(conn, meeting_id).await?;
    // Notes keep who spoke, when the transcript knows
    let speakers: Vec<Option<String>> =
        sqlx::query("SELECT speaker FROM transcripts WHERE meeting_id = ? ORDER BY timestamp ASC, id ASC")
            .bind(meeting_id)
            .fetch_all(&mut *conn)
            .await
            .map_err(err("Couldn't read the transcript"))?
            .iter()
            .map(|r| r.get::<Option<String>, _>("speaker"))
            .collect();
    if speakers.len() == study.lines.len() {
        for (line, speaker) in study.lines.iter_mut().zip(speakers) {
            let Some(sp) = speaker.as_deref().map(str::trim).filter(|s| !s.is_empty()) else { continue };
            if !line.text.trim().is_empty() {
                line.text = format!("{}: {}", sp, line.text);
            }
        }
    }
    let rows = sqlx::query(
        "SELECT k.id, k.ts, k.note, m.started_at FROM meeting_markers k JOIN meetings m ON m.id = k.meeting_id \
         WHERE k.meeting_id = ? AND k.kind = 'note' ORDER BY k.ts ASC, k.created_at ASC",
    )
    .bind(meeting_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(err("Couldn't read the notes you typed"))?;
    let parse = |s: String| chrono::DateTime::parse_from_rfc3339(&s).ok().map(|d| d.with_timezone(&chrono::Utc));
    let typed = rows
        .iter()
        .filter_map(|r| {
            let text: String = r.get::<Option<String>, _>("note").unwrap_or_default();
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let ms = match (parse(r.get("ts")), parse(r.get("started_at"))) {
                (Some(t), Some(s)) => (t - s).num_milliseconds().max(0),
                _ => 0,
            };
            Some(TypedNote { marker_id: r.get("id"), ms, text })
        })
        .collect();
    Ok(NotesInput { study, typed })
}

// ═══════════════════════════════════════════════════════════════════════════
// Prompt (pure)
// ═══════════════════════════════════════════════════════════════════════════

/// Answer budget for a model with this context window.
pub fn max_tokens(context_tokens: usize) -> u32 {
    NOTES_MAX_TOKENS.min((context_tokens * 3 / 10).max(256) as u32)
}

/// How the transcript is laid out. Added to every type's prompt.
const READING_RULES: &str = "Each line of the transcript starts with its time in the recording as [m:ss]. \
Text shown as [stricken from the record] was removed by the user: never guess at or mention what it said.";

/// Added when the user marked moments or typed notes.
fn weave_rules(marks: bool, typed: bool) -> String {
    let mut s = String::new();
    if marks {
        s.push_str(
            "The user marked moments while listening (MARKS, with their times). What was said at those moments \
matters most to them: make sure the notes cover it.\n",
        );
    }
    if typed {
        s.push_str(
            "The user also typed their own notes while listening (TYPED NOTES, numbered, with the time each was \
typed). Build the notes around them: cover what each one is about. Add one more field to the JSON:\n\
\"your_notes\": [{\"n\": 1, \"detail\": \"one or two sentences saying what was being said when note 1 was typed\"}]\n\
with one entry per typed note, in order. Use only what the transcript says around that time; never repeat the \
note itself, and if nothing relevant was said use an empty string.\n",
        );
    }
    s
}

fn marks_block(input: &NotesInput) -> String {
    input
        .study
        .marks
        .iter()
        .filter(|m| !crate::markers::is_note(&m.kind))
        .map(|m| {
            let mut s = format!(
                "[{}] {} {}",
                prompt::clock(m.ms),
                crate::markers::symbol(&m.kind),
                crate::markers::label(&m.kind, input.study.kind)
            );
            if let Some(n) = m.note.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                s.push_str(&format!(": {}", n));
            }
            s
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The typed notes the prompt lists, in order (`n` is the position, from 1).
pub fn prompt_notes(input: &NotesInput) -> &[TypedNote] {
    &input.typed[..input.typed.len().min(MAX_PROMPT_NOTES)]
}

fn typed_block(input: &NotesInput) -> String {
    prompt_notes(input)
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let text: String = n.text.chars().take(PROMPT_NOTE_CHARS).collect();
            format!("{}. [{}] {}", i + 1, prompt::clock(n.ms), text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The one user message: the type's prompt, the rules, the header, the
/// marks and typed notes, then the material (the transcript, or notes
/// condensed from it). Marker ids never appear.
pub fn build_prompt(base_prompt: &str, input: &NotesInput, condensed: bool, body: &str) -> String {
    let marks = marks_block(input);
    let typed = typed_block(input);
    let mut s = format!("{}\n\n{}\n", base_prompt.trim_end(), READING_RULES);
    s.push_str(&weave_rules(!marks.is_empty(), !typed.is_empty()));
    s.push('\n');
    s.push_str(&prompt::header(&input.study));
    if !marks.is_empty() {
        s.push_str(&format!("\nMARKS:\n{}\n", marks));
    }
    if !typed.is_empty() {
        s.push_str(&format!("\nTYPED NOTES:\n{}\n", typed));
    }
    let material = if condensed { "NOTES (condensed from the whole transcript, with times)" } else { "TRANSCRIPT" };
    s.push_str(&format!("\n{}:\n{}\n\nJSON RESPONSE:", material, body));
    s
}

// ═══════════════════════════════════════════════════════════════════════════
// Parsing the answer (untrusted)
// ═══════════════════════════════════════════════════════════════════════════

const MAX_ITEMS: usize = 40;
const MAX_TEXT: usize = 600;
const MAX_SUMMARY: usize = 2_000;
const MAX_DETAIL: usize = 600;

fn text(v: Option<&Value>, max: usize) -> Option<String> {
    crate::study::parse::clean_text(v?, max)
}

fn items<'a>(o: &'a serde_json::Map<String, Value>, key: &str) -> Vec<&'a Value> {
    match o.get(key) {
        Some(Value::Array(a)) => a.iter().take(MAX_ITEMS).collect(),
        _ => Vec::new(),
    }
}

/// The notes and the per-note details (as `(n, detail)`, n from 1) in a
/// model answer. An answer with no summary and nothing in any list is an
/// error; anything missing or mis-shaped inside a usable answer is dropped.
pub fn parse_answer(raw: &str) -> Result<(GeneratedNotes, Vec<(usize, String)>), String> {
    let v = crate::study::parse::extract_json(raw)?;
    let o = v.as_object().ok_or("The answer wasn't a JSON object")?;
    let strings = |key: &str| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for x in items(o, key) {
            if let Some(t) = text(Some(x), MAX_TEXT) {
                if !out.iter().any(|y| y.eq_ignore_ascii_case(&t)) {
                    out.push(t);
                }
            }
        }
        out
    };
    let decisions = items(o, "decisions")
        .into_iter()
        .filter_map(|x| match x {
            Value::Object(d) => Some(Decision {
                text: text(d.get("text").or_else(|| d.get("decision")), MAX_TEXT)?,
                made_by: text(d.get("made_by"), 120),
                context: text(d.get("context"), MAX_TEXT),
            }),
            other => Some(Decision { text: text(Some(other), MAX_TEXT)?, made_by: None, context: None }),
        })
        .collect();
    let action_items = items(o, "action_items")
        .into_iter()
        .filter_map(|x| match x {
            Value::Object(a) => Some(ActionItem {
                task: text(a.get("task").or_else(|| a.get("text")), MAX_TEXT)?,
                assignee: text(a.get("assignee").or_else(|| a.get("owner")), 120),
                due_date: text(a.get("due_date").or_else(|| a.get("due")), 120),
                priority: text(a.get("priority"), 20).map(|p| p.to_lowercase()),
            }),
            other => Some(ActionItem { task: text(Some(other), MAX_TEXT)?, ..Default::default() }),
        })
        .collect();
    let notes = GeneratedNotes {
        summary: text(o.get("summary"), MAX_SUMMARY).unwrap_or_default(),
        key_topics: strings("key_topics"),
        decisions,
        action_items,
        participants: strings("participants"),
    };
    if notes.summary.is_empty() && notes.key_topics.is_empty() && notes.decisions.is_empty() && notes.action_items.is_empty() {
        return Err("The answer had no notes in it".into());
    }
    let mut details: Vec<(usize, String)> = Vec::new();
    for (i, x) in items(o, "your_notes").into_iter().enumerate() {
        let (n, d) = match x {
            Value::Object(e) => (
                e.get("n").and_then(|n| n.as_u64().or_else(|| n.as_str()?.trim().parse().ok())).map(|n| n as usize),
                text(e.get("detail").or_else(|| e.get("text")), MAX_DETAIL),
            ),
            // A bare string: the entry for the note at this position
            other => (Some(i + 1), text(Some(other), MAX_DETAIL)),
        };
        if let (Some(n), Some(d)) = (n, d) {
            if n >= 1 && !details.iter().any(|(m, _)| *m == n) {
                details.push((n, d));
            }
        }
    }
    Ok((notes, details))
}

// ═══════════════════════════════════════════════════════════════════════════
// Generation
// ═══════════════════════════════════════════════════════════════════════════

/// Write the notes for `input` with the type's `base_prompt`. AI access
/// errors (consent, Pro, no provider, network) stop at once; an unusable
/// answer is asked for once more.
pub async fn write(
    c: &dyn Completer,
    input: &NotesInput,
    base_prompt: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<(GeneratedNotes, Vec<NoteDetail>), GenError> {
    if !input.study.has_transcript() {
        return Err(GenError::Failed(NO_TRANSCRIPT.into()));
    }
    let ctx = c.context_tokens();
    let max = max_tokens(ctx);
    // Everything but the material, to see how much room the material has
    let fixed = build_prompt(base_prompt, input, true, "");
    let budget = prompt::body_budget(ctx, max, "", &fixed);
    let mut total = 1usize;
    let mut done = 0usize;
    let (body, condensed) =
        crate::study::condense_to_budget(c, &input.study, budget, &mut total, &mut done, progress).await?;
    let user = build_prompt(base_prompt, input, condensed, &body);

    progress(Progress { done, total, label: "Writing the notes…".into() });
    let mut why = String::new();
    for attempt in 0..2 {
        let text = if attempt == 0 {
            user.clone()
        } else {
            format!(
                "{}\n\nYour previous answer couldn't be used ({}). Answer again with only the JSON object in the \
shape described, and nothing else.",
                user, why
            )
        };
        let opts = crate::ai::Opts { max_tokens: max, temperature: Some(if attempt == 0 { 0.3 } else { 0.4 }) };
        match c.complete(vec![crate::ai::Msg::user(text)], opts).await {
            Ok(raw) => match parse_answer(&raw) {
                Ok((notes, details)) => {
                    let listed = prompt_notes(input);
                    let details = details
                        .into_iter()
                        .filter_map(|(n, detail)| {
                            Some(NoteDetail { marker_id: listed.get(n.checked_sub(1)?)?.marker_id.clone(), detail })
                        })
                        .collect();
                    progress(Progress { done: total, total, label: "Done".into() });
                    return Ok((notes, details));
                }
                Err(e) => why = e,
            },
            Err(e) if crate::study::answer_error(&e) => why = crate::study::short(&e),
            Err(e) => return Err(GenError::Ai(e)),
        }
    }
    log::warn!("Notes: the answer was unusable twice");
    Err(GenError::Failed(UNUSABLE.into()))
}

/// Make the notes for a recording and save them. `meeting_prompt` and
/// `meeting_label` are used for a Meeting; a Class and a Personal recording
/// have their own prompt and label (recording_kind.rs).
pub async fn generate_and_save(
    c: &dyn Completer,
    database: &Arc<DatabaseManager>,
    meeting_id: &str,
    meeting_prompt: &str,
    meeting_label: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<GeneratedNotes, String> {
    let pool = database.pool();
    let input = load_input(pool, meeting_id).await?;
    if !input.study.has_transcript() {
        return Err(NO_TRANSCRIPT.into());
    }
    let read = input.study.transcript_lines();
    let (prompt_text, label) = crate::recording_kind::report_prompt(
        input.study.kind,
        input.study.notebook.as_deref(),
        meeting_prompt,
        meeting_label,
    );
    let (notes, details) = write(c, &input, &prompt_text, progress).await.map_err(String::from)?;

    save_checked(pool, meeting_id, &read, &notes, &details, label).await?;

    // Topics follow the notes (docs/TOPICS_AND_CHAT.md), in the background
    crate::topics::find_after_notes(database.clone(), meeting_id.to_string());
    Ok(notes)
}

/// Save the notes if the transcript still holds every line they were made
/// from, in one `BEGIN IMMEDIATE` transaction: the write lock is taken
/// before the check, so no Delete or Strike can commit between the check
/// and the insert (notes made from text that was removed meanwhile must
/// never be written after the purge ran). Lines that only arrived since
/// (the transcriber's last ones land a few seconds after Stop) are not a
/// reason to throw the notes away. Details are kept only for typed notes
/// that still exist.
async fn save_checked(
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    read: &[String],
    notes: &GeneratedNotes,
    details: &[NoteDetail],
    label: &str,
) -> Result<(), String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await.map_err(err("Database busy"))?;
    let result = save_locked(&mut conn, meeting_id, read, notes, details, label).await;
    let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
    match sqlx::query(end).execute(&mut *conn).await {
        Ok(_) => result,
        Err(e) => {
            // Never hand a connection with an open transaction back to the pool
            use sqlx::Connection;
            let _ = conn.detach().close().await;
            result.and(Err(format!("Failed to save notes: {}", e)))
        }
    }
}

async fn save_locked(
    conn: &mut SqliteConnection,
    meeting_id: &str,
    read: &[String],
    notes: &GeneratedNotes,
    details: &[NoteDetail],
    label: &str,
) -> Result<(), String> {
    if crate::study::has_pending_delete(conn, meeting_id).await?
        || lost_lines(read, &load_input_conn(conn, meeting_id).await?.study.transcript_lines()) > 0
    {
        return Err(TRANSCRIPT_CHANGED.into());
    }
    let alive = typed_note_ids(conn, meeting_id).await.map_err(err("Failed to save notes"))?;
    let kept: Vec<&NoteDetail> = details.iter().filter(|d| alive.contains(&d.marker_id)).collect();
    sqlx::query(
        "INSERT INTO meeting_notes \
         (id, meeting_id, summary, key_topics, decisions, action_items, participants, model_used, generated_at, note_details) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, datetime('now'), ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(meeting_id)
    .bind(&notes.summary)
    .bind(to_json(&notes.key_topics))
    .bind(to_json(&notes.decisions))
    .bind(to_json(&notes.action_items))
    .bind(to_json(&notes.participants))
    .bind(label)
    .bind((!kept.is_empty()).then(|| to_json(&kept)))
    .execute(&mut *conn)
    .await
    .map_err(err("Failed to save notes"))?;
    Ok(())
}

async fn typed_note_ids(conn: &mut SqliteConnection, meeting_id: &str) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    Ok(sqlx::query_scalar::<_, String>("SELECT id FROM meeting_markers WHERE meeting_id = ? AND kind = 'note'")
        .bind(meeting_id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .collect())
}

/// A typed note was added, changed or removed in this recording. What the
/// notes wrote under a note that no longer exists is dropped (it must not
/// outlive the note, in the live database or a backup), and the notes row
/// is touched so sync sends the notes again: their "What you wrote" section
/// carries the typed lines. Does nothing for a database without the column
/// (an older backup) or a recording without notes.
pub async fn typed_notes_changed(conn: &mut SqliteConnection, meeting_id: &str) -> Result<(), sqlx::Error> {
    let has_column: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM pragma_table_info('meeting_notes') WHERE name = 'note_details'")
            .fetch_optional(&mut *conn)
            .await?;
    if has_column.is_none() {
        return Ok(());
    }
    let alive = typed_note_ids(conn, meeting_id).await?;
    let rows: Vec<(String, Option<String>)> = sqlx::query_as("SELECT id, note_details FROM meeting_notes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_all(&mut *conn)
        .await?;
    for (id, stored) in rows {
        let kept: Option<String> = stored.as_deref().and_then(|j| serde_json::from_str::<Vec<NoteDetail>>(j).ok()).and_then(|d| {
            let kept: Vec<NoteDetail> = d.into_iter().filter(|x| alive.contains(&x.marker_id)).collect();
            (!kept.is_empty()).then(|| to_json(&kept))
        });
        // Written even when nothing was dropped: the update is what tells sync
        sqlx::query("UPDATE meeting_notes SET note_details = ? WHERE id = ?")
            .bind(kept)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// How many of the lines the notes were made from are no longer in the
/// transcript as they were (deleted, stricken or edited). Each line counts
/// as often as it occurs; added lines don't count.
pub fn lost_lines(read: &[String], now: &[String]) -> usize {
    let mut have: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for l in now {
        *have.entry(l.as_str()).or_default() += 1;
    }
    read.iter()
        .filter(|l| match have.get_mut(l.as_str()) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
        .count()
}

/// The notes lists as stored; a list that can't be written is stored empty.
fn to_json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

/// A recording gets notes by itself when it stops if it ran six minutes, or
/// one minute when the user typed a note or marked a moment in it.
pub fn auto_notes_due(duration_seconds: i64, marks_and_notes: i64) -> bool {
    duration_seconds >= 360 || (duration_seconds >= 60 && marks_and_notes > 0)
}

// ═══════════════════════════════════════════════════════════════════════════
// Running it (commands and the automatic run after Stop)
// ═══════════════════════════════════════════════════════════════════════════

pub mod run {
    use super::*;
    use crate::AppState;
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use tauri::{AppHandle, Emitter, Manager};

    /// `{meetingId, label, done, total}` while notes are being made.
    pub const PROGRESS_EVENT: &str = "notes_progress";
    /// `{meetingId, ok}` when a run ends (the Notes view reloads).
    pub const DONE_EVENT: &str = "notes_done";

    /// Recordings whose notes are being made, with what the run is doing.
    static RUNNING: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

    struct RunGuard(String);
    impl Drop for RunGuard {
        fn drop(&mut self) {
            RUNNING.lock().remove(&self.0);
        }
    }

    /// What the run for this recording is doing, if one is going.
    pub fn status(meeting_id: &str) -> Option<String> {
        RUNNING.lock().get(meeting_id).cloned()
    }

    fn recording_meeting(state: &AppState) -> Option<String> {
        if !state.capture_engine.read().is_recording() {
            return None;
        }
        state.state_builder.read().current_meeting_id()
    }

    /// The transcriber's last lines, and the trim of what it heard after a
    /// detected end (10 s after Stop), land after the recording stops.
    /// Automatic notes wait for both.
    const SETTLE_AFTER_STOP: std::time::Duration = std::time::Duration::from_secs(12);

    const WAIT_STEP: std::time::Duration = std::time::Duration::from_secs(5);
    /// A recording that outlasts this isn't waited for (make the notes by hand).
    const MAX_WAIT_FOR_STOP: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

    /// Take the recording's one notes slot (released when the guard drops)
    /// and tell the window a run has begun.
    fn reserve(app: &AppHandle, meeting_id: &str) -> Result<RunGuard, String> {
        {
            let mut running = RUNNING.lock();
            if running.contains_key(meeting_id) {
                return Err("Notes are already being made for this recording.".into());
            }
            running.insert(meeting_id.to_string(), "Making notes…".into());
        }
        let _ = app.emit(
            PROGRESS_EVENT,
            serde_json::json!({ "meetingId": meeting_id, "label": "Making notes…", "done": 0, "total": 1 }),
        );
        Ok(RunGuard(meeting_id.to_string()))
    }

    /// Make and save the notes in a slot already taken, telling the window
    /// what the run is doing and when it ends.
    /// `automatic`: nobody clicked for this run, so each of its requests
    /// waits while a recording is running.
    async fn run_reserved(
        app: &AppHandle,
        state: &AppState,
        meeting_id: &str,
        meeting_prompt: &str,
        meeting_label: &str,
        automatic: bool,
    ) -> Result<GeneratedNotes, String> {
        let emit = {
            let app = app.clone();
            let id = meeting_id.to_string();
            move |p: Progress| {
                if let Some(l) = RUNNING.lock().get_mut(&id) {
                    *l = p.label.clone();
                }
                let _ = app.emit(
                    PROGRESS_EVENT,
                    serde_json::json!({ "meetingId": id, "label": p.label, "done": p.done, "total": p.total }),
                );
            }
        };
        if automatic {
            let c = crate::study::BackgroundCompleter;
            generate_and_save(&c, &state.database, meeting_id, meeting_prompt, meeting_label, &emit).await
        } else {
            let c = crate::study::LiveCompleter;
            generate_and_save(&c, &state.database, meeting_id, meeting_prompt, meeting_label, &emit).await
        }
    }

    /// Make and save the notes, one run per recording at a time (Make notes
    /// / Make again).
    pub async fn make(
        app: &AppHandle,
        state: &AppState,
        meeting_id: &str,
        meeting_prompt: &str,
        meeting_label: &str,
    ) -> Result<GeneratedNotes, String> {
        if recording_meeting(state).as_deref() == Some(meeting_id) {
            return Err(STILL_RECORDING.into());
        }
        let guard = reserve(app, meeting_id)?;
        let result = run_reserved(app, state, meeting_id, meeting_prompt, meeting_label, false).await;
        drop(guard);
        let _ = app.emit(DONE_EVENT, serde_json::json!({ "meetingId": meeting_id, "ok": result.is_ok() }));
        result
    }

    /// After Stop: make the notes in the background when automatic notes
    /// are on, an AI is ready and the recording is worth it
    /// ([`auto_notes_due`]). The window shows "Making notes…" at once; the
    /// run starts when the transcript has settled. Says "Notes are ready"
    /// when the window isn't in front. Never raises a dialog: a missing AI,
    /// consent or Pro just means no automatic notes.
    pub fn after_stop(app: AppHandle, meeting_id: String) {
        tauri::async_runtime::spawn(async move {
            let Some(state) = app.try_state::<AppState>() else { return };
            let settings = match state.settings.get_all().await {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("Automatic notes: couldn't load settings: {}", e);
                    return;
                }
            };
            if !settings.auto_generate_report || !crate::ai::is_ready(crate::ai::Kind::Text) {
                return;
            }
            let duration = match state.database.get_meeting(&meeting_id).await {
                Ok(Some(m)) => m.duration_seconds.unwrap_or(0),
                _ => return,
            };
            let marked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_markers WHERE meeting_id = ?")
                .bind(&meeting_id)
                .fetch_one(state.database.pool())
                .await
                .unwrap_or(0);
            if !auto_notes_due(duration, marked) {
                log::info!("Automatic notes: skipped ({} s, {} marks or notes)", duration, marked);
                return;
            }
            // The user may already have pressed Make notes: then that run is the one
            let Ok(guard) = reserve(&app, &meeting_id) else { return };
            tokio::time::sleep(SETTLE_AFTER_STOP).await;
            // Nothing automatic runs while a recording does (a new segment or
            // the next meeting may already have started): wait for it to stop
            let mut waited = std::time::Duration::ZERO;
            while state.capture_engine.read().is_recording() {
                if waited.is_zero() {
                    let label = "Waiting for the recording to stop…";
                    if let Some(l) = RUNNING.lock().get_mut(&meeting_id) {
                        *l = label.to_string();
                    }
                    let _ = app.emit(
                        PROGRESS_EVENT,
                        serde_json::json!({ "meetingId": meeting_id, "label": label, "done": 0, "total": 1 }),
                    );
                }
                tokio::time::sleep(WAIT_STEP).await;
                waited += WAIT_STEP;
                if waited >= MAX_WAIT_FOR_STOP {
                    drop(guard);
                    let _ = app.emit(DONE_EVENT, serde_json::json!({ "meetingId": meeting_id, "ok": false }));
                    return;
                }
            }
            let result = run_reserved(&app, &state, &meeting_id, &settings.meeting_report_prompt, "auto-report", true).await;
            drop(guard);
            let _ = app.emit(DONE_EVENT, serde_json::json!({ "meetingId": meeting_id, "ok": result.is_ok() }));
            match result {
                Ok(_) => {
                    log::info!("Automatic notes: made for {}", meeting_id);
                    let front = app
                        .get_webview_window("main")
                        .map(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
                        .unwrap_or(false);
                    if !front && settings.show_notifications {
                        let title = state
                            .database
                            .get_meeting(&meeting_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|m| m.title)
                            .unwrap_or_default();
                        crate::notifications::post(&app, "Notes are ready", &notes_ready_body(&title));
                    }
                }
                // The kind of failure only: a provider's error text can echo what it was sent
                Err(e) => log::warn!("Automatic notes: not made ({})", failure_kind(&e)),
            }
        });
    }

    /// What kind of failure it was, for the log. Never the message itself: a
    /// provider's error body can repeat the transcript it was sent.
    pub fn failure_kind(e: &str) -> &'static str {
        match e {
            NO_TRANSCRIPT => "no transcript",
            TRANSCRIPT_CHANGED => "the transcript changed",
            UNUSABLE => "the answer was unusable",
            _ if e.starts_with("PRO_REQUIRED") => "noFriction Pro is needed",
            _ if e.starts_with("CONSENT_REQUIRED") => "permission to send is needed",
            _ => "the AI or the database returned an error",
        }
    }

    /// The recording's title (a calendar event or notebook name, never
    /// transcript text) and what clicking does.
    pub fn notes_ready_body(title: &str) -> String {
        let t = title.trim();
        if t.is_empty() {
            "Click to read them.".to_string()
        } else {
            format!("{}. Click to read them.", t)
        }
    }

    /// What the notes run for this recording is doing, or null.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_notes_status(meeting_id: String) -> Result<Option<String>, String> {
        Ok(status(&meeting_id))
    }
}

// ─── Follow-up email (same prompt rules as iOS MeetingAI.emailSystem) ───────

pub const FOLLOWUP_EMAIL_SYSTEM: &str = "Draft a short, friendly follow-up email to the people in this meeting: thank them, \
recap what was decided, and list next steps. Plain text, no Markdown. Start with a \
\"Subject:\" line. Only state facts that are in the transcript; never invent owners or dates. \
Text shown as [stricken from the record] was removed by the user: never guess at or mention what it said.";

/// A drafted follow-up email.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUpEmail {
    pub subject: String,
    pub body: String,
    /// Attendee emails (calendar-linked meetings), excluding the user
    pub to: Vec<String>,
}

/// The user message for the follow-up email: meeting facts + transcript
/// (stricken spans already rendered as `[stricken from the record]`).
pub fn followup_context(
    title: &str,
    when: &str,
    attendees: &[(String, Option<String>)],
    transcript: &str,
) -> String {
    let mut s = format!("Meeting: {}\nWhen: {}\n", title.trim(), when);
    if !attendees.is_empty() {
        let list = attendees
            .iter()
            .map(|(name, company)| match company {
                Some(c) if !c.trim().is_empty() => format!("{} ({})", name, c.trim()),
                _ => name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!("Attendees: {}\n", list));
    }
    s.push_str("\nTranscript:\n");
    s.push_str(transcript);
    s
}

/// Split a drafted email into (subject, body). Accepts "Subject: …" on the
/// first non-empty line (any case, optional Markdown bold); without one the
/// subject falls back to "Follow-up: <title>".
pub fn parse_followup_email(raw: &str, title: &str) -> (String, String) {
    let text = raw.trim().trim_start_matches("```").trim_end_matches("```").trim();
    let mut lines = text.lines();
    let mut subject = None;
    let mut rest: Vec<&str> = Vec::new();
    for line in lines.by_ref() {
        let l = line.trim().trim_matches('*').trim();
        if l.is_empty() {
            continue;
        }
        if l.len() >= 8 && l[..8].eq_ignore_ascii_case("subject:") {
            subject = Some(l[8..].trim().trim_matches('*').trim().to_string());
        } else {
            rest.push(line);
        }
        break;
    }
    rest.extend(lines);
    let body = rest.join("\n").trim().to_string();
    let subject = subject
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("Follow-up: {}", title.trim()));
    (subject, body)
}

/// The transcript block sent to the AI. `get_transcripts` already renders
/// stricken spans as `[stricken from the record]`; render again here so a
/// caller holding marked rows can never leak a marker id (or skip it).
pub fn transcript_for_prompt(transcripts: &[crate::database::Transcript]) -> String {
    transcripts
        .iter()
        .map(|t| {
            let text = crate::redaction::render_plain(&t.text);
            match t.speaker {
                Some(ref speaker) => format!("{}: {}", speaker, text),
                None => text,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod followup_tests {
    #[test]
    fn followup_email_parsing() {
        let (s, b) = super::parse_followup_email("Subject: Thanks for today\n\nHi all,\nThanks!", "Weekly");
        assert_eq!(s, "Thanks for today");
        assert_eq!(b, "Hi all,\nThanks!");
        let (s, b) = super::parse_followup_email("**Subject:** Recap\nBody", "Weekly");
        assert_eq!(s, "Recap");
        assert_eq!(b, "Body");
        let (s, b) = super::parse_followup_email("Hi team,\nthanks", "Weekly sync");
        assert_eq!(s, "Follow-up: Weekly sync");
        assert_eq!(b, "Hi team,\nthanks");
    }

    #[test]
    fn followup_context_lists_attendees_and_keeps_stricken_marker() {
        let c = super::followup_context(
            "Weekly",
            "Thu 2 Oct 2026 10:00",
            &[("Ana".into(), Some("Acme".into())), ("Bo".into(), None)],
            "We agreed to ship. [stricken from the record]",
        );
        assert!(c.starts_with("Meeting: Weekly\nWhen: Thu 2 Oct 2026 10:00\nAttendees: Ana (Acme), Bo\n"));
        assert!(c.ends_with("[stricken from the record]"));
        assert!(super::FOLLOWUP_EMAIL_SYSTEM.contains("never invent owners or dates"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiError, Msg, Opts};
    use crate::recording_kind::RecordingKind;
    use crate::study::prompt::{StudyLine, StudyMark};
    use parking_lot::Mutex;
    use std::path::PathBuf;

    const GOOD: &str = r#"{"summary": "Membranes and transport.", "key_topics": ["Bilayer", "Pumps"],
        "decisions": [{"text": "Osmosis: water follows solute", "made_by": null, "context": "a red blood cell"}],
        "action_items": [{"task": "Quiz on membranes", "assignee": null, "due_date": "Wednesday", "priority": "HIGH"}],
        "participants": ["Dr. Reyes"],
        "your_notes": [{"n": 2, "detail": "Two potassium ions come in for each ATP."}, {"n": "1", "detail": "The pump moves three sodium ions out."}, {"n": 99, "detail": "no such note"}]}"#;

    struct Mock {
        ctx: usize,
        calls: Mutex<Vec<String>>,
        /// Answers to give to the notes request, in order (the last repeats)
        answers: Mutex<Vec<String>>,
        error: Option<AiError>,
        /// SQL run once, during the notes request (an edit made meanwhile)
        edit: Mutex<Option<(Arc<DatabaseManager>, &'static str)>>,
    }

    impl Mock {
        fn new(ctx: usize) -> Self {
            Self { ctx, calls: Mutex::new(Vec::new()), answers: Mutex::new(vec![GOOD.into()]), error: None, edit: Mutex::new(None) }
        }
        fn answering(ctx: usize, answers: &[&str]) -> Self {
            let m = Self::new(ctx);
            *m.answers.lock() = answers.iter().map(|a| a.to_string()).collect();
            m
        }
        fn requests(&self) -> Vec<String> {
            self.calls.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl Completer for Mock {
        async fn complete(&self, msgs: Vec<Msg>, _opts: Opts) -> Result<String, AiError> {
            let text = msgs.iter().map(|m| m.text()).collect::<Vec<_>>().join("\n");
            self.calls.lock().push(text.clone());
            if let Some(e) = &self.error {
                return Err(e.clone());
            }
            if text.starts_with("You condense") {
                // Keep the first and last line of the chunk, as a condenser would keep its facts
                let lines: Vec<&str> = text.lines().filter(|l| l.starts_with('[')).collect();
                let first = lines.first().copied().unwrap_or("[0:00] nothing");
                let last = lines.last().copied().unwrap_or(first);
                return Ok(format!("{}\n{}", first, last));
            }
            let edit = self.edit.lock().take();
            if let Some((db, sql)) = edit {
                sqlx::query(sql).execute(db.pool()).await.unwrap();
            }
            let mut a = self.answers.lock();
            Ok(if a.len() > 1 { a.remove(0) } else { a[0].clone() })
        }
        fn context_tokens(&self) -> usize {
            self.ctx
        }
    }

    fn quiet() -> impl Fn(Progress) + Send + Sync {
        |_p: Progress| {}
    }

    fn input(lines: &[(i64, &str)], marks: &[(i64, &str, Option<&str>)], typed: &[(i64, &str)]) -> NotesInput {
        let mut all: Vec<StudyMark> =
            marks.iter().map(|(ms, k, n)| StudyMark { ms: *ms, kind: k.to_string(), note: n.map(String::from) }).collect();
        all.extend(typed.iter().map(|(ms, t)| StudyMark { ms: *ms, kind: "note".into(), note: Some(t.to_string()) }));
        NotesInput {
            study: StudyInput {
                title: "Membrane transport".into(),
                kind: RecordingKind::Class,
                notebook: Some("BIO 101".into()),
                duration_ms: lines.iter().map(|l| l.0).max().unwrap_or(0) + 5_000,
                lines: lines.iter().map(|(ms, t)| StudyLine { ms: *ms, text: t.to_string() }).collect(),
                marks: all,
            },
            typed: typed
                .iter()
                .enumerate()
                .map(|(i, (ms, t))| TypedNote { marker_id: format!("marker-id-{}", i + 1), ms: *ms, text: t.to_string() })
                .collect(),
        }
    }

    #[test]
    fn the_prompt_lists_marks_and_numbered_notes_and_never_an_id() {
        let i = input(
            &[(0, "Dr. Reyes: Let's pick up at the membrane."), (61_000, "[stricken from the record]"), (90_000, "The pump moves sodium.")],
            &[(10_000, "question", None), (90_500, "test", Some("on the exam"))],
            &[(92_000, "pump 3 na out"), (95_000, "2 k in")],
        );
        let p = build_prompt("LECTURE PROMPT", &i, false, &i.study.transcript_lines().join("\n"));
        assert!(p.starts_with("LECTURE PROMPT\n\nEach line of the transcript starts with its time"), "{}", p);
        assert!(p.contains("never guess at or mention what it said"));
        assert!(p.contains("\nMARKS:\n[0:10] ? Question\n[1:30] ✎ On the test: on the exam\n"), "{}", p);
        assert!(p.contains("\nTYPED NOTES:\n1. [1:32] pump 3 na out\n2. [1:35] 2 k in\n"), "{}", p);
        assert!(p.contains("\"your_notes\""));
        assert!(p.contains("Lecture: Membrane transport\nClass: BIO 101\n"));
        assert!(p.contains("\nTRANSCRIPT:\n[0:00] Dr. Reyes: Let's pick up at the membrane.\n[1:01] [stricken from the record]\n"), "{}", p);
        assert!(p.ends_with("JSON RESPONSE:"));
        assert!(!p.contains("marker-id"), "marker ids never go in a prompt");
        // A typed note is listed once, under TYPED NOTES, not again as a mark
        assert_eq!(p.matches("pump 3 na out").count(), 1);

        // Nothing marked or typed: no blocks and no extra field asked for
        let plain = input(&[(0, "Hello.")], &[], &[]);
        let p = build_prompt("PROMPT", &plain, false, "[0:00] Hello.");
        assert!(!p.contains("MARKS:") && !p.contains("TYPED NOTES:") && !p.contains("your_notes"), "{}", p);
        // Condensed material is named as such
        assert!(build_prompt("PROMPT", &plain, true, "[0:00] hello").contains("NOTES (condensed from the whole transcript, with times):"));
    }

    #[test]
    fn answers_are_parsed_tolerantly_and_cleaned() {
        let (n, d) = parse_answer(&format!("<think>plan</think>Here you go:\n```json\n{}\n```", GOOD)).unwrap();
        assert_eq!(n.summary, "Membranes and transport.");
        assert_eq!(n.key_topics, vec!["Bilayer", "Pumps"]);
        assert_eq!(n.decisions[0].context.as_deref(), Some("a red blood cell"));
        assert_eq!(n.decisions[0].made_by, None);
        assert_eq!(
            (n.action_items[0].task.as_str(), n.action_items[0].due_date.as_deref(), n.action_items[0].priority.as_deref()),
            ("Quiz on membranes", Some("Wednesday"), Some("high"))
        );
        assert_eq!(d.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![2, 1, 99]);

        // Small-model shapes: strings where objects were asked for, lists of bare details
        let (n, d) = parse_answer(
            r#"{"summary": " A   talk. ", "key_topics": ["- one", "One", 3], "decisions": ["Ship in May"],
               "action_items": ["Book the room", {"text": "Call Ana", "owner": "Bo", "due": "Friday"}, {"assignee": "nobody"}],
               "your_notes": ["first detail", "", "third detail"],}"#,
        )
        .unwrap();
        assert_eq!(n.summary, "A talk.");
        assert_eq!(n.key_topics, vec!["one", "3"], "list markers dropped, duplicates folded");
        assert_eq!(n.decisions, vec![Decision { text: "Ship in May".into(), made_by: None, context: None }]);
        assert_eq!(n.action_items.len(), 2, "an item with no task is dropped");
        assert_eq!((n.action_items[1].assignee.as_deref(), n.action_items[1].due_date.as_deref()), (Some("Bo"), Some("Friday")));
        assert_eq!(d, vec![(1, "first detail".to_string()), (3, "third detail".to_string())]);

        // Unusable answers are errors, never panics, and never an empty note
        for bad in ["", "I can't help with that.", "{", "[1, 2]", "null", "{\"summary\": \"\", \"key_topics\": []}", "{\"other\": true}"] {
            assert!(parse_answer(bad).is_err(), "accepted {:?}", bad);
        }
        let deep = "[".repeat(5000);
        assert!(parse_answer(&deep).is_err());
        // Counts and lengths are capped
        let many = serde_json::json!({"summary": "x".repeat(9000), "key_topics": (0..200).map(|i| format!("t{}", i)).collect::<Vec<_>>()});
        let (n, _) = parse_answer(&many.to_string()).unwrap();
        assert!(n.summary.chars().count() <= MAX_SUMMARY);
        assert_eq!(n.key_topics.len(), MAX_ITEMS);
    }

    #[tokio::test]
    async fn a_short_recording_is_one_request_with_the_whole_transcript() {
        let i = input(&[(0, "first line"), (30_000, "middle line"), (60_000, "the very last line")], &[], &[(31_000, "pump"), (32_000, "gradient")]);
        let m = Mock::new(32_768);
        let (notes, details) = write(&m, &i, "PROMPT", &quiet()).await.unwrap();
        let reqs = m.requests();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].contains("[0:00] first line") && reqs[0].contains("[1:00] the very last line"));
        assert_eq!(notes.summary, "Membranes and transport.");
        // Details land on the typed notes they answer; an unknown number is dropped
        assert_eq!(
            details,
            vec![
                NoteDetail { marker_id: "marker-id-2".into(), detail: "Two potassium ions come in for each ATP.".into() },
                NoteDetail { marker_id: "marker-id-1".into(), detail: "The pump moves three sodium ions out.".into() },
            ]
        );
    }

    #[tokio::test]
    async fn a_long_recording_is_read_to_the_end_in_parts_that_fit() {
        // About 50 minutes of speech: far more than the 8,000 characters the
        // notes used to read, and more than a 4K-token model holds
        let mut lines: Vec<(i64, String)> = (0..600)
            .map(|n| (n * 5_000, format!("line {} of the lecture, with enough words in it to take up real room in a prompt", n)))
            .collect();
        lines.push((600 * 5_000, "THE-LAST-THING-SAID before everyone left".into()));
        let refs: Vec<(i64, &str)> = lines.iter().map(|(ms, t)| (*ms, t.as_str())).collect();
        let i = input(&refs, &[(2_999_000, "test", None)], &[(3_000_500, "what was the last thing")]);
        assert!(i.study.transcript_lines().join("\n").chars().count() > 40_000);

        let ctx = 4_096;
        let m = Mock::new(ctx);
        let (notes, details) = write(&m, &i, &crate::recording_kind::lecture_notes_prompt(Some("BIO 101")), &quiet()).await.unwrap();
        assert_eq!(notes.summary, "Membranes and transport.");
        assert!(details.is_empty() || details.iter().all(|d| d.marker_id == "marker-id-1"));

        let reqs = m.requests();
        let condense: Vec<&String> = reqs.iter().filter(|r| r.starts_with("You condense")).collect();
        assert!(condense.len() > 3, "read in parts ({} requests)", condense.len());
        // Every request fits the window with the answer's budget beside it
        let room = ((ctx - max_tokens(ctx) as usize) as f64 * prompt::CHARS_PER_TOKEN) as usize;
        for r in &reqs {
            assert!(r.chars().count() <= room, "a request of {} characters doesn't fit {}", r.chars().count(), room);
        }
        // The end of the recording was read, and reached the notes request
        assert!(condense.iter().any(|r| r.contains("THE-LAST-THING-SAID")), "the tail was never sent");
        let last = reqs.last().unwrap();
        assert!(last.contains("NOTES (condensed from the whole transcript, with times):"));
        assert!(last.contains("THE-LAST-THING-SAID"), "the notes request never saw the end");
        assert!(last.contains("line 0 of the lecture"), "the notes request never saw the start");
        assert!(last.contains("TYPED NOTES:\n1. [50:00] what was the last thing"));
    }

    #[tokio::test]
    async fn a_bad_answer_is_asked_again_and_a_second_one_is_reported_without_quoting_it() {
        let i = input(&[(0, "cells have organelles")], &[], &[]);
        let m = Mock::answering(32_768, &["Sure! SECRET-TRANSCRIPT-WORDS but no json", GOOD]);
        assert!(write(&m, &i, "PROMPT", &quiet()).await.is_ok());
        assert_eq!(m.requests().len(), 2);
        assert!(m.requests()[1].contains("Your previous answer couldn't be used"));

        let m = Mock::answering(32_768, &["SECRET-TRANSCRIPT-WORDS, again not json"]);
        let e = String::from(write(&m, &i, "PROMPT", &quiet()).await.unwrap_err());
        assert_eq!(e, UNUSABLE);
        assert!(!e.contains("SECRET"), "the model's words never go in an error");
        assert_eq!(m.requests().len(), 2);
    }

    #[tokio::test]
    async fn access_errors_stop_the_run_and_an_empty_recording_never_calls_the_ai() {
        let i = input(&[(0, "cells have organelles")], &[], &[]);
        let mut m = Mock::new(32_768);
        m.error = Some(AiError::ProRequired);
        assert!(matches!(write(&m, &i, "PROMPT", &quiet()).await, Err(GenError::Ai(AiError::ProRequired))));
        assert_eq!(m.requests().len(), 1);

        let empty = input(&[(0, "[stricken from the record]")], &[], &[(1_000, "a note")]);
        let m = Mock::new(32_768);
        assert_eq!(String::from(write(&m, &empty, "PROMPT", &quiet()).await.unwrap_err()), NO_TRANSCRIPT);
        assert!(m.requests().is_empty());
    }

    #[test]
    fn only_lines_that_were_read_and_are_gone_count_as_lost() {
        let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let read = l(&["[0:00] a", "[0:05] b", "[0:05] b"]);
        assert_eq!(lost_lines(&read, &read), 0);
        assert_eq!(lost_lines(&read, &l(&["[0:00] a", "[0:05] b", "[0:05] b", "[0:09] late line"])), 0, "added lines are fine");
        assert_eq!(lost_lines(&read, &l(&["[0:00] a", "[0:05] b"])), 1, "one of the two identical lines went");
        assert_eq!(lost_lines(&read, &l(&["[0:00] a", "[0:05] [stricken from the record]", "[0:05] b"])), 1);
        assert_eq!(lost_lines(&read, &[]), 3);
        assert_eq!(lost_lines(&[], &read), 0);
    }

    #[test]
    fn automatic_notes_rule() {
        assert!(auto_notes_due(360, 0), "six minutes is enough on its own");
        assert!(!auto_notes_due(359, 0));
        assert!(auto_notes_due(60, 1), "a minute, when the user typed or marked");
        assert!(!auto_notes_due(59, 5));
        assert!(!auto_notes_due(0, 0));
        assert_eq!(run::notes_ready_body("BIO 101 — Nov 12"), "BIO 101 — Nov 12. Click to read them.");
        // The log gets the kind of failure, never a provider's words
        assert_eq!(run::failure_kind(TRANSCRIPT_CHANGED), "the transcript changed");
        assert_eq!(run::failure_kind("PRO_REQUIRED: noFriction Pro unlocks the AI features."), "noFriction Pro is needed");
        let echoed = "HTTP 400: could not process 'the launch code is tangerine'";
        assert!(!run::failure_kind(echoed).contains("tangerine"));
        assert_eq!(run::notes_ready_body("  "), "Click to read them.");
        // Small models get a smaller answer budget
        assert_eq!(max_tokens(128_000), NOTES_MAX_TOKENS);
        assert!(max_tokens(4_096) < 1_300);
    }

    // ── With the database ────────────────────────────────────────────────

    struct Fx {
        db: Arc<DatabaseManager>,
        dir: PathBuf,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn fx() -> Fx {
        let dir = std::env::temp_dir().join(format!("nf-notes-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = DatabaseManager::new(&dir.join("t.db")).await.unwrap();
        db.run_migrations().await.unwrap();
        db.create_meeting("m1", "Membrane transport").await.unwrap();
        db.set_meeting_kind("m1", RecordingKind::Class).await.unwrap();
        Fx { db: Arc::new(db), dir }
    }

    async fn start(f: &Fx) -> chrono::DateTime<chrono::Utc> {
        let s: String = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = 'm1'").fetch_one(f.db.pool()).await.unwrap();
        chrono::DateTime::parse_from_rfc3339(&s).unwrap().with_timezone(&chrono::Utc)
    }

    async fn line(f: &Fx, secs: i64, speaker: Option<&str>, text: &str) {
        let t = start(f).await + chrono::Duration::seconds(secs);
        f.db.add_transcript_at("m1", text, speaker, true, 0.9, t).await.unwrap();
    }

    #[tokio::test]
    async fn notes_are_saved_with_what_was_said_at_each_typed_note() {
        let f = fx().await;
        let t0 = start(&f).await;
        line(&f, 0, Some("Dr. Reyes"), "The sodium potassium pump uses ATP.").await;
        line(&f, 20, None, "Three sodium out, two potassium in.").await;
        let pool = f.db.pool();
        let n1 = crate::markers::add(pool, "m1", t0 + chrono::Duration::seconds(21), Some("note"), Some("pump 3 na out"), false).await.unwrap();
        let n2 = crate::markers::add(pool, "m1", t0 + chrono::Duration::seconds(25), Some("note"), Some("2 k in"), false).await.unwrap();
        crate::markers::add(pool, "m1", t0 + chrono::Duration::seconds(22), Some("test"), None, false).await.unwrap();

        let loaded = load_input(pool, "m1").await.unwrap();
        assert_eq!(loaded.study.transcript_lines()[0], "[0:00] Dr. Reyes: The sodium potassium pump uses ATP.");
        assert_eq!(loaded.study.transcript_lines()[1], "[0:20] Three sodium out, two potassium in.");
        assert_eq!(loaded.typed.iter().map(|t| (t.ms, t.text.as_str())).collect::<Vec<_>>(), vec![(21_000, "pump 3 na out"), (25_000, "2 k in")]);

        let m = Mock::new(32_768);
        let notes = generate_and_save(&m, &f.db, "m1", "MEETING PROMPT", "auto-report", &quiet()).await.unwrap();
        assert_eq!(notes.key_topics, vec!["Bilayer", "Pumps"]);
        // A class gets the lecture prompt, whatever the meeting prompt is
        assert!(m.requests()[0].contains("Write lecture notes"), "{}", &m.requests()[0][..200]);
        assert!(m.requests()[0].contains("MARKS:\n[0:22] ✎ On the test"));

        let saved = f.db.get_meeting_notes("m1").await.unwrap().unwrap();
        assert_eq!(saved.model_used.as_deref(), Some("lecture-notes"));
        assert!((chrono::Utc::now() - saved.generated_at).num_seconds().abs() < 120, "the stored time is read back, not replaced by now");
        let details: Vec<NoteDetail> = serde_json::from_str(saved.note_details.as_deref().unwrap()).unwrap();
        assert_eq!(
            details,
            vec![
                NoteDetail { marker_id: n2.id.clone(), detail: "Two potassium ions come in for each ATP.".into() },
                NoteDetail { marker_id: n1.id.clone(), detail: "The pump moves three sodium ions out.".into() },
            ]
        );

        // Deleting a typed note takes what the notes wrote under it, and the
        // notes row is touched so sync sends the notes again
        let seq = |pool: sqlx::Pool<sqlx::Sqlite>| async move {
            sqlx::query_scalar::<_, i64>("SELECT seq FROM sync_meta WHERE entity = 'notes' AND record_id = 'm1'")
                .fetch_one(&pool)
                .await
                .unwrap()
        };
        let before = seq(pool.clone()).await;
        crate::markers::delete(pool, &n1.id).await.unwrap();
        let after_delete = f.db.get_meeting_notes("m1").await.unwrap().unwrap();
        let left: Vec<NoteDetail> = serde_json::from_str(after_delete.note_details.as_deref().unwrap()).unwrap();
        assert_eq!(left.iter().map(|d| d.marker_id.as_str()).collect::<Vec<_>>(), vec![n2.id.as_str()]);
        assert!(!after_delete.note_details.unwrap().contains("three sodium"));
        assert!(seq(pool.clone()).await > before, "the notes are sent again");
        // Editing one sends the notes again too (their section quotes the line)
        let before = seq(pool.clone()).await;
        crate::markers::update(pool, &n2.id, None, Some(Some("2 K in per ATP"))).await.unwrap();
        assert!(seq(pool.clone()).await > before);

        // Striking a distinctive word rewrites the details like the rest of the notes
        let mut conn = pool.acquire().await.unwrap();
        crate::redaction::redact_ai_outputs(&mut conn, "m1", "potassium", "[stricken from the record]").await.unwrap();
        drop(conn);
        let after = f.db.get_meeting_notes("m1").await.unwrap().unwrap();
        assert!(after.stale_after_edit);
        let d = after.note_details.unwrap();
        assert!(!d.to_lowercase().contains("potassium"), "{}", d);
        assert!(d.contains("[stricken from the record]"));
    }

    #[tokio::test]
    async fn notes_made_from_text_edited_meanwhile_are_not_saved() {
        let f = fx().await;
        line(&f, 0, None, "The launch code is tangerine.").await;
        let m = Mock::new(32_768);
        // The user removes the word while the AI is writing
        *m.edit.lock() =
            Some((f.db.clone(), "UPDATE transcripts SET text = 'The launch code is.' WHERE meeting_id = 'm1'"));
        let e = generate_and_save(&m, &f.db, "m1", "PROMPT", "default", &quiet()).await.unwrap_err();
        assert_eq!(e, TRANSCRIPT_CHANGED);
        assert!(f.db.get_meeting_notes("m1").await.unwrap().is_none());

        // A line that arrives while the AI is writing (the transcriber's last
        // ones land after Stop) doesn't cost the notes
        let f3 = fx().await;
        line(&f3, 0, None, "Cells have organelles.").await;
        let m = Mock::new(32_768);
        *m.edit.lock() = Some((
            f3.db.clone(),
            "INSERT INTO transcripts (meeting_id, text, timestamp, is_final, confidence) \
             SELECT 'm1', 'Mitochondria make ATP.', started_at, 1, 0.9 FROM meetings WHERE id = 'm1'",
        ));
        generate_and_save(&m, &f3.db, "m1", "PROMPT", "default", &quiet()).await.unwrap();
        assert!(f3.db.get_meeting_notes("m1").await.unwrap().is_some());

        // No transcript: nothing is asked
        let f2 = fx().await;
        let m = Mock::new(32_768);
        assert_eq!(generate_and_save(&m, &f2.db, "m1", "PROMPT", "default", &quiet()).await.unwrap_err(), NO_TRANSCRIPT);
        assert!(m.requests().is_empty());
    }
}
