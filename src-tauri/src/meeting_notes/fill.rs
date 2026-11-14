// noFriction - Fill in the blanks (docs/design/FADELL_V2.md §11).
//
// One button on a recording's Notes. It reads the whole recording again,
// beside the notes that already exist and the lines the user typed, and
// writes two things:
//   - What you missed: things that were said and aren't in the notes, each
//     with the time it was said.
//   - More detail: the notes' key ideas explained further, from what was
//     said. For a Class only, the AI may also add what a textbook would say,
//     and every such entry is marked as background, never as something that
//     was said.
//
// Rules:
//   - It runs only on the user's click, with the same gates as Make notes
//     (Pro on the Mac App Store build, consent, an AI that is set up), and
//     never for the recording that is still running.
//   - The user's own lines are never rewritten. Typed notes are only shown
//     to the AI so it knows what is already covered.
//   - The result is kept on the notes row it was made for (`filled`), under
//     the same check as the notes: one transaction, refused if a line it
//     read is gone or a Delete is pending.
//   - It never outlives what it was made from. Delete or Strike of
//     transcript text removes it outright (it paraphrases the recording, so
//     word matching can't clean it); making the notes again starts a row
//     without it; changing or deleting a typed note it was made beside
//     removes it.
//   - The answer is untrusted: sizes are capped, a time outside the
//     recording is dropped, and background is refused outside a Class.

use super::*;
use crate::recording_kind::RecordingKind;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const NEEDS_NOTES: &str = "Make the notes first, then fill in the blanks.";
pub const NOTES_CHANGED: &str = "The notes changed while this was being written. Fill in the blanks again.";

/// How many missed points and explanations are kept
pub const MAX_MISSED: usize = 12;
pub const MAX_MORE: usize = 8;
const MAX_POINT: usize = 400;
const MAX_ABOUT: usize = 160;
const MAX_MORE_DETAIL: usize = 700;
/// The notes the user already has are shown to the AI up to this many
/// characters, less on a small model: about a quarter of its window
/// (`have_cap`), so the recording itself keeps most of the room.
const MAX_HAVE_CHARS: usize = 6_000;
const MIN_HAVE_CHARS: usize = 800;

/// How much of NOTES SO FAR a model with this context window is shown.
pub fn have_cap(context_tokens: usize) -> usize {
    (context_tokens * 3 / 4).clamp(MIN_HAVE_CHARS, MAX_HAVE_CHARS)
}

/// Something that was said and the notes don't have.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Missed {
    /// When it was said, from the start of the recording
    pub at_ms: Option<i64>,
    pub text: String,
}

/// One of the notes' key ideas, explained further.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct More {
    pub about: String,
    pub detail: String,
    pub at_ms: Option<i64>,
    /// Not said in the recording: what a textbook says, added by the AI (Class only)
    #[serde(default)]
    pub background: bool,
}

/// `meeting_notes.filled`, as JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Filled {
    #[serde(default)]
    pub missed: Vec<Missed>,
    #[serde(default)]
    pub more: Vec<More>,
    /// The typed notes this was made beside: id → a hash of the line. If one
    /// of them is changed or deleted, this goes with it.
    #[serde(default)]
    pub typed: BTreeMap<String, String>,
}

impl Filled {
    pub fn is_empty(&self) -> bool {
        self.missed.is_empty() && self.more.is_empty()
    }
}

/// `filled` is AI output made from the transcript. It is removed outright
/// when transcript text is deleted or struck (`clear_for_meeting`, called
/// from `redaction::redact_ai_outputs`).
pub async fn ensure_schema(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    crate::database::ensure_columns(conn, "meeting_notes", &[("filled", "TEXT")]).await?;
    Ok(())
}

async fn has_column(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    let has: Option<i64> = sqlx::query_scalar("SELECT 1 FROM pragma_table_info('meeting_notes') WHERE name = 'filled'")
        .fetch_optional(&mut *conn)
        .await?;
    Ok(has.is_some())
}

/// Remove what was filled in for a recording (transcript text was deleted
/// or struck). Returns how many rows had it. Safe on a database without
/// the column (an older backup).
pub async fn clear_for_meeting(conn: &mut SqliteConnection, meeting_id: &str) -> Result<u64, sqlx::Error> {
    if !has_column(conn).await? {
        return Ok(0);
    }
    let cleared = sqlx::query("UPDATE meeting_notes SET filled = NULL WHERE meeting_id = ? AND filled IS NOT NULL")
        .bind(meeting_id)
        .execute(&mut *conn)
        .await?
        .rows_affected();
    Ok(cleared + strip_synced(conn, meeting_id).await?)
}

/// Notes that came from the iPhone are Markdown in `summary`, and may carry
/// the two sections the iPhone (or this Mac, on the way back) wrote. They
/// are removed there too. Returns how many rows had them.
async fn strip_synced(conn: &mut SqliteConnection, meeting_id: &str) -> Result<u64, sqlx::Error> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, summary FROM meeting_notes WHERE meeting_id = ? AND model_used = ? AND summary IS NOT NULL")
            .bind(meeting_id)
            .bind(crate::sync::store::SYNCED_NOTES_MODEL)
            .fetch_all(&mut *conn)
            .await?;
    let mut changed = 0;
    for (id, md) in rows {
        let stripped = strip_sections(&md);
        if stripped != md.trim_end() {
            sqlx::query("UPDATE meeting_notes SET summary = ? WHERE id = ?").bind(stripped).bind(id).execute(&mut *conn).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// A typed line as `load_input` reads it (trimmed), hashed.
/// Whether a recording's notes have anything filled in (for the Delete and
/// Strike confirmation).
pub async fn exists_for_meeting(conn: &mut SqliteConnection, meeting_id: &str) -> bool {
    if !has_column(conn).await.unwrap_or(false) {
        return false;
    }
    sqlx::query_scalar::<_, i64>("SELECT 1 FROM meeting_notes WHERE meeting_id = ? AND filled IS NOT NULL LIMIT 1")
        .bind(meeting_id)
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten()
        .is_some()
}

fn line_hash(text: &str) -> String {
    let digest = Sha256::digest(text.trim().as_bytes());
    digest.iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

/// id → hash for a recording's typed notes. All of them: when a long
/// recording is condensed, every one is in what the AI is given.
pub fn typed_hashes(typed: &[TypedNote]) -> BTreeMap<String, String> {
    typed.iter().map(|n| (n.marker_id.clone(), line_hash(&n.text))).collect()
}

/// A typed note was added, changed or removed (`typed_notes_changed`).
/// Whatever was filled in beside a note that is now gone or reads
/// differently is removed; a note added since changes nothing.
pub async fn typed_notes_changed(conn: &mut SqliteConnection, meeting_id: &str) -> Result<(), sqlx::Error> {
    if !has_column(conn).await? {
        return Ok(());
    }
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT id, filled FROM meeting_notes WHERE meeting_id = ? AND filled IS NOT NULL")
            .bind(meeting_id)
            .fetch_all(&mut *conn)
            .await?;
    // Sections inside notes from the iPhone don't say which typed notes they
    // were made beside, so any typed-note change takes them
    strip_synced(conn, meeting_id).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let now: BTreeMap<String, String> = sqlx::query_as::<_, (String, String)>(
        "SELECT id, COALESCE(note, '') FROM meeting_markers WHERE meeting_id = ? AND kind = 'note'",
    )
    .bind(meeting_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .filter(|(_, text)| !text.trim().is_empty())
    .map(|(id, text)| (id, line_hash(&text)))
    .collect();
    for (id, stored) in rows {
        // Unreadable is treated as changed: it can't be shown to be safe
        let still = serde_json::from_str::<Filled>(&stored)
            .map(|f| f.typed.iter().all(|(note, hash)| now.get(note) == Some(hash)))
            .unwrap_or(false);
        if !still {
            sqlx::query("UPDATE meeting_notes SET filled = NULL WHERE id = ?").bind(id).execute(&mut *conn).await?;
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// The prompt
// ═══════════════════════════════════════════════════════════════════════════

/// What the pass is asked to do, by recording type. Only a Class may get
/// background, and it must be marked.
pub fn base_prompt(kind: RecordingKind, notebook: Option<&str>) -> String {
    let named = |what: &str| match notebook.and_then(crate::notebooks::normalize) {
        Some(n) => format!("{} (\"{}\")", what, n),
        None => what.to_string(),
    };
    let shape = r#"Your output MUST be valid JSON with this exact schema:
{
  "missed": [{"at": "m:ss", "point": "one thing that was said and NOTES SO FAR don't have"}],
  "more": [{"about": "a key idea or term from NOTES SO FAR", "detail": "two to four sentences that explain it further", "at": "m:ss", "from": "recording"}]
}"#;
    match kind {
        RecordingKind::Class => format!(
            r#"You are helping a student complete their notes from {what}. NOTES SO FAR is what they already have: the notes made from the recording and the lines they typed themselves. Below it is everything that was said. Find what the notes are missing, and explain the key ideas further so the student can study from them.

{shape}

Rules:
- missed: only things actually said, each with the time of the line it comes from. Look for definitions, the steps of a method, formulas, examples, corrections, what the instructor said matters or will be on a test, and answers to students' questions. Skip anything NOTES SO FAR already says. In the order they were said, at most {missed}.
- more: take the key concepts and terms in NOTES SO FAR, at most {more}, and explain each one further from what the instructor said: "from": "recording", with the time.
- If the recording does not explain a concept well enough to study from, you may add what a standard textbook says about it: a definition, the formula, a simple example, in one to three sentences. Mark that entry "from": "background" and set "at" to null. Background must be well-established knowledge. Never present background as something that was said in class.
- Never rewrite or repeat the lines the student typed.
- Write plain sentences a student can study from.
- If there is nothing for a list, return an empty array.
- Do NOT invent anything that was said.
"#,
            what = named("a class"),
            shape = shape,
            missed = MAX_MISSED,
            more = MAX_MORE,
        ),
        RecordingKind::Meeting => format!(
            r#"You are helping someone complete their notes from a meeting. NOTES SO FAR is what they already have: the notes made from the recording and the lines they typed themselves. Below it is everything that was said. Find what the notes are missing, and say more about the key topics.

{shape}

Rules:
- missed: only things actually said, each with the time of the line it comes from. Look for decisions, numbers, dates, names, commitments, objections, open questions and the reasons given. Skip anything NOTES SO FAR already says. In the order they were said, at most {missed}.
- more: take the key topics in NOTES SO FAR, at most {more}, and say what was said about each that the notes leave out: the reasoning, the alternatives that were discussed, who raised what. Every entry is "from": "recording", with the time.
- Use only what was said. Do not add outside knowledge, advice or opinions.
- Never rewrite or repeat the lines the user typed.
- If there is nothing for a list, return an empty array.
- Do NOT invent anything that was said.
"#,
            shape = shape,
            missed = MAX_MISSED,
            more = MAX_MORE,
        ),
        RecordingKind::Personal => format!(
            r#"You are helping someone complete their notes from {what}: a conversation, appointment, talk or idea. NOTES SO FAR is what they already have: the notes made from the recording and the lines they typed themselves. Below it is everything that was said. Find what the notes are missing, and say more about the key points.

{shape}

Rules:
- missed: only things actually said, each with the time of the line it comes from. Look for facts, numbers, names, dates, instructions and things to do. Skip anything NOTES SO FAR already says. In the order they were said, at most {missed}.
- more: take the key points in NOTES SO FAR, at most {more}, and say what was said about each that the notes leave out. Every entry is "from": "recording", with the time.
- Use only what was said. Do not add outside knowledge, advice or opinions.
- Never rewrite or repeat the lines the user typed.
- If there is nothing for a list, return an empty array.
- Do NOT invent anything that was said.
"#,
            what = named("a personal recording"),
            shape = shape,
            missed = MAX_MISSED,
            more = MAX_MORE,
        ),
    }
}

fn list_strings(json: Option<&str>) -> Vec<Value> {
    json.and_then(|j| serde_json::from_str::<Vec<Value>>(j).ok()).unwrap_or_default()
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn field_text(v: &Value, keys: &[&str]) -> Option<String> {
    match v {
        Value::String(s) => Some(one_line(s)).filter(|s| !s.is_empty()),
        Value::Object(o) => keys.iter().find_map(|k| o.get(*k).and_then(|x| x.as_str()).map(one_line).filter(|s| !s.is_empty())),
        _ => None,
    }
}

/// NOTES SO FAR: the saved notes and the user's own lines, as plain text.
/// Notes that came from the iPhone are Markdown in `summary` and are shown
/// as they are.
pub fn have_block(notes: &crate::database::MeetingNotes, typed: &[TypedNote], cap: usize) -> String {
    let mut s = String::new();
    if let Some(summary) = notes.summary.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        if notes.model_used.as_deref() == Some(crate::sync::store::SYNCED_NOTES_MODEL) {
            // What an earlier fill wrote is not what the user already has
            s.push_str(&strip_sections(summary));
            s.push('\n');
        } else {
            s.push_str(&format!("Summary: {}\n", one_line(summary)));
        }
    }
    let mut section = |title: &str, lines: Vec<String>| {
        if !lines.is_empty() {
            s.push_str(&format!("{}:\n", title));
            for l in lines {
                s.push_str(&format!("- {}\n", l));
            }
        }
    };
    section(
        "Key points",
        list_strings(notes.key_topics.as_deref()).iter().filter_map(|v| field_text(v, &["text", "topic"])).collect(),
    );
    section(
        "Definitions and decisions",
        list_strings(notes.decisions.as_deref())
            .iter()
            .filter_map(|v| {
                let text = field_text(v, &["text", "decision"])?;
                Some(match v.get("context").and_then(|c| c.as_str()).map(one_line).filter(|c| !c.is_empty()) {
                    Some(c) => format!("{} ({})", text, c),
                    None => text,
                })
            })
            .collect(),
    );
    section(
        "To do, announcements and deadlines",
        list_strings(notes.action_items.as_deref())
            .iter()
            .filter_map(|v| {
                let task = field_text(v, &["task", "text"])?;
                Some(match v.get("due_date").and_then(|c| c.as_str()).map(one_line).filter(|c| !c.is_empty()) {
                    Some(d) => format!("{} (due {})", task, d),
                    None => task,
                })
            })
            .collect(),
    );
    section(
        "Typed by the user while listening",
        prompt_notes_of(typed)
            .iter()
            .map(|n| format!("[{}] {}", prompt::clock(n.ms), n.text.chars().take(PROMPT_NOTE_CHARS).collect::<String>()))
            .collect(),
    );
    if s.chars().count() > cap {
        s = s.chars().take(cap).collect::<String>() + "\n(more notes follow)\n";
    }
    s
}

/// The typed notes shown to the AI (the same cap the notes prompt uses).
fn prompt_notes_of(typed: &[TypedNote]) -> &[TypedNote] {
    &typed[..typed.len().min(MAX_PROMPT_NOTES)]
}

/// The one user message. Marker ids never appear.
pub fn build_prompt(input: &NotesInput, have: &str, condensed: bool, body: &str) -> String {
    let base = base_prompt(input.study.kind, input.study.notebook.as_deref());
    let mut s = format!("{}\n{}\n\n", base.trim_end(), READING_RULES);
    s.push_str(&prompt::header(&input.study));
    s.push_str(&format!("\nNOTES SO FAR:\n{}\n", if have.trim().is_empty() { "(none)" } else { have.trim_end() }));
    let material = if condensed { "NOTES (condensed from the whole transcript, with times)" } else { "TRANSCRIPT" };
    s.push_str(&format!("\n{}:\n{}\n\nJSON RESPONSE:", material, body));
    s
}

// ═══════════════════════════════════════════════════════════════════════════
// Parsing the answer (untrusted)
// ═══════════════════════════════════════════════════════════════════════════

fn clip(v: Option<&Value>, max: usize) -> Option<String> {
    crate::study::parse::clean_text(v?, max)
}

/// A time the answer gave, if it is inside the recording.
fn moment(v: Option<&Value>, duration_ms: i64) -> Option<i64> {
    let ms = match v? {
        Value::String(s) => crate::study::parse::parse_clock(s)?,
        Value::Number(n) => (n.as_f64()? * 1000.0).round() as i64,
        _ => return None,
    };
    // A few seconds past the end is the transcriber's last line
    (ms >= 0 && (duration_ms <= 0 || ms <= duration_ms + 5_000)).then_some(ms)
}

fn array<'a>(o: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Vec<&'a Value> {
    keys.iter()
        .find_map(|k| match o.get(*k) {
            Some(Value::Array(a)) => Some(a.iter().take(60).collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// What a model answer holds. Anything missing or mis-shaped is dropped;
/// an answer that isn't a JSON object is an error. Empty lists are a valid
/// answer (the notes already cover the recording).
pub fn parse(raw: &str, kind: RecordingKind, duration_ms: i64) -> Result<Filled, String> {
    let v = crate::study::parse::extract_json(raw)?;
    let o = v.as_object().ok_or("The answer wasn't a JSON object")?;
    if !o.contains_key("missed") && !o.contains_key("more") {
        return Err("The answer had neither list in it".into());
    }
    let mut missed: Vec<Missed> = Vec::new();
    for x in array(o, &["missed"]) {
        let (text, at) = match x {
            Value::Object(e) => (
                clip(e.get("point").or_else(|| e.get("text")), MAX_POINT),
                moment(e.get("at").or_else(|| e.get("time")), duration_ms),
            ),
            other => (clip(Some(other), MAX_POINT), None),
        };
        if let Some(text) = text {
            if !missed.iter().any(|m| m.text.eq_ignore_ascii_case(&text)) {
                missed.push(Missed { at_ms: at, text });
            }
        }
    }
    // In the order said; the ones without a time keep their place at the end
    missed.sort_by_key(|m| m.at_ms.unwrap_or(i64::MAX));
    missed.truncate(MAX_MISSED);

    let mut more: Vec<More> = Vec::new();
    for x in array(o, &["more"]) {
        let Value::Object(e) = x else { continue };
        let about = clip(e.get("about").or_else(|| e.get("term")).or_else(|| e.get("concept")), MAX_ABOUT);
        let detail = clip(e.get("detail").or_else(|| e.get("text")).or_else(|| e.get("explanation")), MAX_MORE_DETAIL);
        let (Some(about), Some(detail)) = (about, detail) else { continue };
        let at = moment(e.get("at").or_else(|| e.get("time")), duration_ms);
        // Said in the recording only when the answer says so, or gives its
        // time and says nothing else. Anything else ("textbook", "outside
        // knowledge", a flag) is background: it must never pass as said.
        let from = e.get("from").or_else(|| e.get("source")).and_then(|f| f.as_str()).map(|f| f.trim().to_lowercase());
        let flagged = match e.get("background") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(t)) => !matches!(t.trim().to_lowercase().as_str(), "" | "false" | "no"),
            _ => false,
        };
        let said = match from.as_deref() {
            Some(f) => ["recording", "transcript", "lecture", "class", "meeting", "said"].iter().any(|w| f.starts_with(w)),
            None => at.is_some(),
        };
        let background = flagged || !said;
        // Outside a class nothing may come from anywhere but the recording
        if background && kind != RecordingKind::Class {
            continue;
        }
        if more.iter().any(|m| m.about.eq_ignore_ascii_case(&about)) {
            continue;
        }
        more.push(More { about, detail, at_ms: if background { None } else { at }, background });
    }
    more.truncate(MAX_MORE);
    Ok(Filled { missed, more, typed: BTreeMap::new() })
}

// ═══════════════════════════════════════════════════════════════════════════
// Generation and saving
// ═══════════════════════════════════════════════════════════════════════════

/// Ask the AI once the transcript fits (condensed in parts when it is
/// long), and once more if the answer can't be used. AI access errors stop
/// at once.
pub async fn write(
    c: &dyn Completer,
    input: &NotesInput,
    have: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Filled, GenError> {
    if !input.study.has_transcript() {
        return Err(GenError::Failed(NO_TRANSCRIPT.into()));
    }
    let ctx = c.context_tokens();
    let max = max_tokens(ctx);
    let fixed = build_prompt(input, have, true, "");
    let budget = prompt::body_budget(ctx, max, "", &fixed);
    let mut total = 1usize;
    let mut done = 0usize;
    let (body, condensed) =
        crate::study::condense_to_budget(c, &input.study, budget, &mut total, &mut done, progress).await?;
    let user = build_prompt(input, have, condensed, &body);

    progress(Progress { done, total, label: "Finding what the notes are missing…".into() });
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
            Ok(raw) => match parse(&raw, input.study.kind, input.study.duration_ms) {
                Ok(filled) => {
                    progress(Progress { done: total, total, label: "Done".into() });
                    return Ok(filled);
                }
                Err(e) => why = e,
            },
            Err(e) if crate::study::answer_error(&e) => why = crate::study::short(&e),
            Err(e) => return Err(GenError::Ai(e)),
        }
    }
    log::warn!("Fill in the blanks: the answer was unusable twice");
    Err(GenError::Failed(UNUSABLE.into()))
}

/// Fill in the blanks for a recording whose notes exist, and keep the
/// result on that notes row.
pub async fn fill_and_save(
    c: &dyn Completer,
    database: &Arc<DatabaseManager>,
    meeting_id: &str,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Filled, String> {
    let pool = database.pool();
    let input = load_input(pool, meeting_id).await?;
    if !input.study.has_transcript() {
        return Err(NO_TRANSCRIPT.into());
    }
    let notes = database
        .get_meeting_notes(meeting_id)
        .await
        .map_err(err("Couldn't read the notes"))?
        .ok_or_else(|| NEEDS_NOTES.to_string())?;
    // Text in a Delete's undo window is about to be removed: it isn't sent
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    if crate::study::has_pending_delete(&mut conn, meeting_id).await? {
        return Err(crate::study::PENDING_EDIT.into());
    }
    drop(conn);
    let read = input.study.transcript_lines();
    let have = have_block(&notes, &input.typed, have_cap(c.context_tokens()));
    let mut filled = write(c, &input, &have, progress).await.map_err(String::from)?;
    filled.typed = typed_hashes(&input.typed);
    save_checked(pool, meeting_id, &notes.id, &read, &filled).await?;
    Ok(filled)
}

/// Keep `filled` on the notes row it was made for, in one `BEGIN IMMEDIATE`
/// transaction with its checks, like the notes themselves
/// (`meeting_notes::save_checked`): no Delete pending, every line that was
/// read still there, the notes row still the recording's latest, and the
/// typed notes it was made beside unchanged.
async fn save_checked(
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    notes_id: &str,
    read: &[String],
    filled: &Filled,
) -> Result<(), String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await.map_err(err("Database busy"))?;
    let result = save_locked(&mut conn, meeting_id, notes_id, read, filled).await;
    let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
    match sqlx::query(end).execute(&mut *conn).await {
        Ok(_) => result,
        Err(e) => {
            // Never hand a connection with an open transaction back to the pool
            use sqlx::Connection;
            let _ = conn.detach().close().await;
            result.and(Err(format!("Failed to save: {}", e)))
        }
    }
}

async fn save_locked(
    conn: &mut SqliteConnection,
    meeting_id: &str,
    notes_id: &str,
    read: &[String],
    filled: &Filled,
) -> Result<(), String> {
    let now = load_input_conn(conn, meeting_id).await?;
    if crate::study::has_pending_delete(conn, meeting_id).await? || lost_lines(read, &now.study.transcript_lines()) > 0 {
        return Err(TRANSCRIPT_CHANGED.into());
    }
    let latest: Option<String> = sqlx::query_scalar(
        "SELECT id FROM meeting_notes WHERE meeting_id = ? ORDER BY generated_at DESC, rowid DESC LIMIT 1",
    )
    .bind(meeting_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err("Failed to save"))?;
    let typed_now = typed_hashes(&now.typed);
    if latest.as_deref() != Some(notes_id) || filled.typed.iter().any(|(id, hash)| typed_now.get(id) != Some(hash)) {
        return Err(NOTES_CHANGED.into());
    }
    // This Mac's result replaces any the notes carried in their own text
    strip_synced(conn, meeting_id).await.map_err(err("Failed to save"))?;
    sqlx::query("UPDATE meeting_notes SET filled = ? WHERE id = ?")
        .bind(to_json_object(filled))
        .bind(notes_id)
        .execute(&mut *conn)
        .await
        .map_err(err("Failed to save"))?;
    Ok(())
}

fn to_json_object(filled: &Filled) -> String {
    serde_json::to_string(filled).unwrap_or_else(|_| "{}".into())
}

/// The two sections as Markdown, for the notes sent to the iPhone
/// (sync/store.rs). The iPhone writes the same headings when it fills in
/// the blanks itself. None when nothing was filled in.
pub fn markdown(filled_json: Option<&str>) -> Option<String> {
    let filled: Filled = serde_json::from_str(filled_json?).ok()?;
    let mut parts: Vec<String> = Vec::new();
    if !filled.missed.is_empty() {
        let bullets: Vec<String> = filled
            .missed
            .iter()
            .map(|m| match m.at_ms {
                Some(ms) => format!("- [{}] {}", prompt::clock(ms), one_line(&m.text)),
                None => format!("- {}", one_line(&m.text)),
            })
            .collect();
        parts.push(format!("## {}\n{}", MISSED_HEADING, bullets.join("\n")));
    }
    if !filled.more.is_empty() {
        let bullets: Vec<String> = filled
            .more
            .iter()
            .map(|m| {
                let about = one_line(&m.about).replace("**", "");
                let tail = match (m.background, m.at_ms) {
                    (true, _) => format!(" ({})", BACKGROUND_NOTE),
                    (false, Some(ms)) => format!(" [{}]", prompt::clock(ms)),
                    (false, None) => String::new(),
                };
                format!("- **{}**: {}{}", about, one_line(&m.detail), tail)
            })
            .collect();
        parts.push(format!("## {}\n{}", MORE_HEADING, bullets.join("\n")));
    }
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// The two sections as plain lines, (What you missed, More detail), for
/// the places that lay notes out themselves: Export, the Obsidian export
/// and Chat. A line carries its time when it has one, and a More detail
/// line that is background says so, exactly as the notes view does: that
/// marking must never be lost on the way out of the app.
pub fn lines(filled_json: Option<&str>) -> (Vec<String>, Vec<String>) {
    let Some(filled) = filled_json.and_then(|j| serde_json::from_str::<Filled>(j).ok()) else {
        return (Vec::new(), Vec::new());
    };
    let missed = filled
        .missed
        .iter()
        .map(|m| match m.at_ms {
            Some(ms) => format!("[{}] {}", prompt::clock(ms), one_line(&m.text)),
            None => one_line(&m.text),
        })
        .collect();
    let more = filled
        .more
        .iter()
        .map(|m| {
            let tail = match (m.background, m.at_ms) {
                (true, _) => format!(" ({})", BACKGROUND_NOTE),
                (false, Some(ms)) => format!(" [{}]", prompt::clock(ms)),
                (false, None) => String::new(),
            };
            format!("{}: {}{}", one_line(&m.about).replace("**", ""), one_line(&m.detail), tail)
        })
        .collect();
    (missed, more)
}

/// Notes Markdown without the two sections (from their `## ` heading to the
/// next heading or the end). Notes that came from the iPhone may carry
/// them: they are taken out before this Mac's own are added, and before
/// the notes are shown to the AI as what the user already has.
pub fn strip_sections(md: &str) -> String {
    // A heading is a `#` line or a line that is only **bold** (how the
    // frontend reads notes Markdown, syncLogic.ts)
    let heading = |line: &str| -> Option<String> {
        let t = line.trim();
        if t.starts_with('#') {
            Some(t.trim_matches('#').trim().trim_matches('*').trim().to_string())
        } else if t.len() > 4 && t.starts_with("**") && t.ends_with("**") && !t[2..t.len() - 2].contains("**") {
            Some(t[2..t.len() - 2].trim().to_string())
        } else {
            None
        }
    };
    let mut out: Vec<&str> = Vec::new();
    let mut skipping = false;
    for line in md.lines() {
        if let Some(h) = heading(line) {
            skipping = h.eq_ignore_ascii_case(MISSED_HEADING) || h.eq_ignore_ascii_case(MORE_HEADING);
        }
        if !skipping {
            out.push(line);
        }
    }
    // No blank lines left hanging where a section was
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// The headings, the same on every platform
pub const MISSED_HEADING: &str = "What you missed";
pub const MORE_HEADING: &str = "More detail";
/// How an entry that wasn't said is marked, the same on every platform
pub const BACKGROUND_NOTE: &str = "background, not said in the recording";
