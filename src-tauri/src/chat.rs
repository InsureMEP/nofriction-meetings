//! Chat with your recordings (docs/TOPICS_AND_CHAT.md): a conversation
//! that answers from the user's own recordings, with citations that jump
//! to the moment.
//!
//! - Scope: all recordings, one Notebook, one Topic or one recording. The
//!   scope is resolved to a SQL filter (`retrieval::Filter`) and named in
//!   every answer.
//! - Retrieval, local only: FTS5 over the transcripts (bm25), plus the saved
//!   notes and marker notes matched by the question's terms, all read from
//!   the app's SQLite database. The best passages that fit the model's
//!   window are sent, each with the recording's title, date, type, Notebook
//!   and time. No embeddings, no index service.
//! - AI: one request through the same `Completer` the Review guide uses
//!   (`ai::complete_text`: consent, endpoint policy and, in the Mac App
//!   Store build, noFriction Pro). The answer is streamed: the window gets
//!   the text so far while the model writes (`STREAM_EVENT`), then the
//!   stored turn.
//! - Answer: Markdown with `[n]` citations; `citations` maps each number to
//!   the passage (recording, time, excerpt). The UI renders the text as
//!   text, never HTML.
//! - Memory: the thread's last `HISTORY_TURNS` messages go into the prompt.
//!   Threads and messages are stored locally (`chat_threads`,
//!   `chat_messages`, `chat_message_sources`), listable and deletable.
//! - Purge: when a recording is deleted or its transcript edited, every
//!   assistant message whose sources included it is deleted and the thread
//!   flagged (`purge_for_meeting`, in `redaction::redact_ai_outputs` and
//!   `DatabaseManager::delete_meeting`). See docs/REDACTION.md. Transcript
//!   text and model output are never logged.

pub mod live;
pub mod prompt;
pub mod retrieval;
#[cfg(test)]
mod tests;

use crate::ai::{AiError, Msg, Opts};
use crate::study::Completer;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnection;
use sqlx::{Acquire, Pool, Row, Sqlite};

pub use retrieval::{Passage, Scope, ScopeKind};

/// Messages of the thread sent back to the model with each question.
pub const HISTORY_TURNS: usize = 8;
/// Passages at most per answer.
pub const MAX_PASSAGES: usize = 12;
/// Shared with iOS.
const TITLE_CHARS: usize = 48;

pub const FLAG_REMOVED: &str =
    "Some answers were removed from this chat because a recording they drew on was deleted or edited.";

// ═══════════════════════════════════════════════════════════════════════════
// Schema (runs inside DatabaseManager::run_migrations on its one connection)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn ensure_schema(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS chat_threads (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            scope_kind TEXT NOT NULL,
            scope_value TEXT,
            flag TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
            content TEXT NOT NULL,
            citations TEXT NOT NULL DEFAULT '[]',
            scope_label TEXT,
            created_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_chat_messages_thread ON chat_messages(thread_id, created_at)")
        .execute(&mut *conn)
        .await?;
    // Every recording whose passages were sent for an answer (cited or not)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS chat_message_sources (
            message_id TEXT NOT NULL REFERENCES chat_messages(id) ON DELETE CASCADE,
            meeting_id TEXT NOT NULL,
            PRIMARY KEY (message_id, meeting_id)
        )
        "#,
    )
    .execute(&mut *conn)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_chat_message_sources_meeting ON chat_message_sources(meeting_id)")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Purges, counted per library (the database file, so a purge in an app
/// backup or in another test's database is not this one's). An answer
/// takes seconds to write: one that was being written while text was
/// removed from a recording may hold that text, so it stops being shown
/// (`stream_sink`) and is not kept (`ask_as`).
static PURGES: once_cell::sync::Lazy<parking_lot::Mutex<std::collections::HashMap<String, u64>>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

/// The file of the database this connection is on ("" if it can't be read).
async fn library_of(conn: &mut SqliteConnection) -> String {
    sqlx::query_scalar::<_, String>("SELECT file FROM pragma_database_list WHERE name = 'main'")
        .fetch_optional(&mut *conn)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Whether text has been removed from any recording of a library (Delete,
/// Strike, a recording deleted) since this was started.
#[derive(Debug, Clone)]
pub struct PurgeWatch {
    library: String,
    mark: u64,
}

impl PurgeWatch {
    pub async fn start(conn: &mut SqliteConnection) -> Self {
        let library = library_of(conn).await;
        let mark = PURGES.lock().get(&library).copied().unwrap_or(0);
        Self { library, mark }
    }
    pub async fn of(pool: &Pool<Sqlite>) -> Result<Self, String> {
        let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
        Ok(Self::start(&mut conn).await)
    }
    pub fn purged(&self) -> bool {
        PURGES.lock().get(&self.library).copied().unwrap_or(0) != self.mark
    }
}

/// What a question gets when text was removed while it was being answered.
pub const PURGED_MEANWHILE: &str =
    "Text was removed from a recording while this was being answered, so the answer wasn't kept. Ask again.";

/// Wait for a write that is being committed right now (a Delete or Strike
/// counts its purge before it commits): after this, what is read is what
/// that write left.
async fn wait_for_writers(conn: &mut SqliteConnection) -> Result<(), String> {
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await.map_err(err("Database busy"))?;
    sqlx::query("COMMIT").execute(&mut *conn).await.map_err(err("Database busy"))?;
    Ok(())
}

async fn has_table(conn: &mut SqliteConnection, name: &str) -> Result<bool, sqlx::Error> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?")
        .bind(name)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(exists.is_some())
}

/// Purge: delete every assistant answer that drew on this meeting (its
/// passages were in the prompt, cited or not) and flag their threads, and
/// flag threads scoped to the meeting. Called for every Delete/Strike of
/// transcript text, in the action's transaction, on the live database and
/// on each app backup (`redaction::redact_ai_outputs`), and when the meeting
/// is deleted. Schema-tolerant: an old backup may not have the tables.
/// Returns the number of messages deleted.
pub async fn purge_for_meeting(conn: &mut SqliteConnection, meeting_id: &str) -> Result<u64, sqlx::Error> {
    // An answer about this recording that is still on the card over another
    // app (quick_note.rs) is one more copy: it goes first
    crate::quick_note::answer_purged(meeting_id);
    // And one still being written stops being shown and is not kept
    let library = library_of(conn).await;
    *PURGES.lock().entry(library).or_insert(0) += 1;
    if !has_table(conn, "chat_message_sources").await? || !has_table(conn, "chat_messages").await? {
        return Ok(0);
    }
    sqlx::query(
        "UPDATE chat_threads SET flag = ? WHERE id IN (SELECT thread_id FROM chat_messages WHERE id IN \
         (SELECT message_id FROM chat_message_sources WHERE meeting_id = ?)) \
         OR (scope_kind = 'meeting' AND scope_value = ?)",
    )
    .bind(FLAG_REMOVED)
    .bind(meeting_id)
    .bind(meeting_id)
    .execute(&mut *conn)
    .await?;
    let n = sqlx::query(
        "DELETE FROM chat_messages WHERE id IN (SELECT message_id FROM chat_message_sources WHERE meeting_id = ?)",
    )
    .bind(meeting_id)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM chat_message_sources WHERE meeting_id = ?")
        .bind(meeting_id)
        .execute(&mut *conn)
        .await?;
    Ok(n)
}

/// Answers a purge would delete (for the Delete/Strike preview).
pub async fn count_for_meeting(conn: &mut SqliteConnection, meeting_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(DISTINCT message_id) FROM chat_message_sources WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or(0)
}

// ═══════════════════════════════════════════════════════════════════════════
// Records
// ═══════════════════════════════════════════════════════════════════════════

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatThread {
    pub id: String,
    pub title: String,
    pub scope: Scope,
    /// Set when answers were removed by a purge (`FLAG_REMOVED`)
    pub flag: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// One passage the answer may cite: `[n]` in the text → this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Citation {
    pub n: usize,
    pub meeting_id: String,
    pub title: String,
    /// ms from the recording's start; None for notes (no time)
    pub timestamp_ms: Option<i64>,
    pub excerpt: String,
    /// "transcript" | "notes" | "marker"
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    pub id: String,
    pub thread_id: String,
    /// "user" | "assistant"
    pub role: String,
    /// Markdown (assistant) or the user's text; shown as text, never HTML
    pub content: String,
    pub citations: Vec<Citation>,
    /// The scope the answer was drawn from ("All recordings", "Notebook BIO 101"…)
    pub scope_label: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatThreadDetail {
    pub thread: ChatThread,
    pub messages: Vec<ChatMessage>,
}

/// What `ask` returns: the thread (new or existing) and the two messages.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatTurn {
    pub thread: ChatThread,
    pub user: ChatMessage,
    pub assistant: ChatMessage,
}

fn thread_from_row(r: &sqlx::sqlite::SqliteRow) -> ChatThread {
    ChatThread {
        id: r.get("id"),
        title: r.get("title"),
        scope: Scope::from_stored(&r.get::<String, _>("scope_kind"), r.get::<Option<String>, _>("scope_value")),
        flag: r.get("flag"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn message_from_row(r: &sqlx::sqlite::SqliteRow) -> ChatMessage {
    let citations: Vec<Citation> = serde_json::from_str(&r.get::<String, _>("citations")).unwrap_or_default();
    ChatMessage {
        id: r.get("id"),
        thread_id: r.get("thread_id"),
        role: r.get("role"),
        content: r.get("content"),
        citations,
        scope_label: r.get("scope_label"),
        created_at: r.get("created_at"),
    }
}

const THREAD_COLS: &str = "id, title, scope_kind, scope_value, flag, created_at, updated_at";
const MESSAGE_COLS: &str = "id, thread_id, role, content, citations, scope_label, created_at";

pub async fn list_threads(pool: &Pool<Sqlite>) -> Result<Vec<ChatThread>, String> {
    let rows = sqlx::query(&format!("SELECT {} FROM chat_threads ORDER BY updated_at DESC LIMIT 200", THREAD_COLS))
        .fetch_all(pool)
        .await
        .map_err(err("Couldn't read the chats"))?;
    Ok(rows.iter().map(thread_from_row).collect())
}

async fn get_thread_conn(conn: &mut SqliteConnection, id: &str) -> Result<ChatThread, String> {
    let row = sqlx::query(&format!("SELECT {} FROM chat_threads WHERE id = ?", THREAD_COLS))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(err("Couldn't read the chat"))?
        .ok_or("That chat no longer exists")?;
    Ok(thread_from_row(&row))
}

pub async fn get_thread(pool: &Pool<Sqlite>, id: &str) -> Result<ChatThreadDetail, String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    let thread = get_thread_conn(&mut conn, id).await?;
    let rows = sqlx::query(&format!(
        "SELECT {} FROM chat_messages WHERE thread_id = ? ORDER BY created_at ASC, rowid ASC",
        MESSAGE_COLS
    ))
    .bind(id)
    .fetch_all(&mut *conn)
    .await
    .map_err(err("Couldn't read the chat"))?;
    Ok(ChatThreadDetail { thread, messages: rows.iter().map(message_from_row).collect() })
}

pub async fn delete_thread(pool: &Pool<Sqlite>, id: &str) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(err("Database busy"))?;
    // Explicit, so the delete never depends on foreign-key enforcement
    sqlx::query("DELETE FROM chat_message_sources WHERE message_id IN (SELECT id FROM chat_messages WHERE thread_id = ?)")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err("Couldn't delete the chat"))?;
    sqlx::query("DELETE FROM chat_messages WHERE thread_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err("Couldn't delete the chat"))?;
    sqlx::query("DELETE FROM chat_threads WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(err("Couldn't delete the chat"))?;
    tx.commit().await.map_err(err("Couldn't delete the chat"))
}

/// The thread's last `HISTORY_TURNS` messages, oldest first.
async fn history(conn: &mut SqliteConnection, thread_id: &str) -> Result<Vec<(String, String)>, String> {
    let rows = sqlx::query(
        "SELECT role, content FROM chat_messages WHERE thread_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?",
    )
    .bind(thread_id)
    .bind(HISTORY_TURNS as i64)
    .fetch_all(&mut *conn)
    .await
    .map_err(err("Couldn't read the chat"))?;
    Ok(rows.iter().rev().map(|r| (r.get("role"), r.get("content"))).collect())
}

/// A thread title from the first question.
pub fn title_of(question: &str) -> String {
    let one = question.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= TITLE_CHARS {
        one
    } else {
        let cut: String = one.chars().take(TITLE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Asking
// ═══════════════════════════════════════════════════════════════════════════

/// Everything an answer is made of, before and after the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub content: String,
    pub passages: Vec<Passage>,
    pub scope_label: String,
}

impl Answer {
    pub fn citations(&self) -> Vec<Citation> {
        self.passages
            .iter()
            .map(|p| Citation {
                n: p.n,
                meeting_id: p.meeting_id.clone(),
                title: p.title.clone(),
                timestamp_ms: p.timestamp_ms,
                excerpt: p.excerpt.clone(),
                source: p.source.to_string(),
            })
            .collect()
    }
}

/// The citation numbers an answer uses, in order of first use.
pub fn cited_numbers(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '[' {
            let mut j = i + 1;
            let mut nums: Vec<usize> = Vec::new();
            let mut cur = String::new();
            let mut ok = true;
            while j < bytes.len() && bytes[j] != ']' {
                let c = bytes[j];
                if c.is_ascii_digit() {
                    cur.push(c);
                } else if c == ',' || c == ' ' {
                    if !cur.is_empty() {
                        nums.push(cur.parse().unwrap_or(0));
                        cur.clear();
                    }
                } else {
                    ok = false;
                    break;
                }
                j += 1;
            }
            if ok && j < bytes.len() {
                if !cur.is_empty() {
                    nums.push(cur.parse().unwrap_or(0));
                }
                for n in nums {
                    if n > 0 && !out.contains(&n) {
                        out.push(n);
                    }
                }
                i = j;
            }
        }
        i += 1;
    }
    out
}

/// Answer `question` within `scope` with `c`, using `history` (role, text)
/// as memory. Retrieval and the prompt are local; only the request goes to
/// the user's AI.
pub async fn answer(
    conn: &mut SqliteConnection,
    c: &dyn Completer,
    scope: &Scope,
    history: &[(String, String)],
    question: &str,
) -> Result<Answer, String> {
    let question = question.trim();
    if question.is_empty() {
        return Err("Ask something first.".into());
    }
    let info = retrieval::resolve(conn, scope).await?;
    let ctx = c.context_tokens();
    let max_tokens = prompt::max_tokens(ctx);
    let hist = prompt::history_messages(history, ctx);
    let fixed = prompt::user_message(&info, &[], question) + &hist.iter().map(|m| m.text()).collect::<Vec<_>>().join("\n");
    let budget = prompt::passage_budget(ctx, max_tokens, &prompt::system_prompt(), &fixed);
    let passages = retrieval::retrieve(conn, &info, question, budget, MAX_PASSAGES).await?;
    let mut msgs = vec![Msg::system(prompt::system_prompt())];
    msgs.extend(hist);
    msgs.push(Msg::user(prompt::user_message(&info, &passages, question)));
    let opts = Opts { max_tokens, temperature: Some(0.2) };
    let content = match c.complete(msgs, opts).await {
        Ok(raw) => prompt::clean_answer(&raw),
        Err(AiError::Truncated) => return Err(AiError::Truncated.to_string()),
        Err(e) => return Err(e.to_string()),
    };
    if content.is_empty() {
        return Err("The model sent an empty answer. Try again.".into());
    }
    Ok(Answer { content, passages, scope_label: info.label })
}

/// Ask in a thread (a new one when `thread_id` is None) and store both
/// messages. The thread's scope is the one asked with (it can change
/// between questions; every answer names its own).
pub async fn ask(
    pool: &Pool<Sqlite>,
    c: &dyn Completer,
    thread_id: Option<&str>,
    scope: &Scope,
    question: &str,
) -> Result<ChatTurn, String> {
    ask_as(pool, c, thread_id, scope, question, None).await
}

/// Ask about one recording in the moment (`live::answer`): what was just
/// said, who is there, what the user wrote. Stored like any other turn, in
/// a thread scoped to that recording, so it is in Chat afterwards and is
/// removed with the recording's text.
pub async fn ask_recording(
    pool: &Pool<Sqlite>,
    c: &dyn Completer,
    thread_id: Option<&str>,
    meeting_id: &str,
    question: &str,
    mode: live::Mode,
) -> Result<ChatTurn, String> {
    ask_as(pool, c, thread_id, &Scope::meeting(meeting_id), question, Some(mode)).await
}

async fn ask_as(
    pool: &Pool<Sqlite>,
    c: &dyn Completer,
    thread_id: Option<&str>,
    scope: &Scope,
    question: &str,
    in_the_moment: Option<live::Mode>,
) -> Result<ChatTurn, String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    // Before anything of a recording is read
    wait_for_writers(&mut conn).await?;
    let watch = PurgeWatch::start(&mut conn).await;
    let (existing, hist) = match thread_id {
        Some(id) => {
            let t = get_thread_conn(&mut conn, id).await?;
            let h = history(&mut conn, id).await?;
            (Some(t), h)
        }
        None => (None, Vec::new()),
    };
    let ans = match (in_the_moment, scope.value.as_deref()) {
        (Some(mode), Some(meeting_id)) => live::answer(&mut conn, c, meeting_id, &hist, question, Utc::now(), mode).await?,
        _ => answer(&mut conn, c, scope, &hist, question).await?,
    };
    // The model took its time. If text was removed from a recording
    // meanwhile, this answer may hold it: it is not kept
    if watch.purged() {
        return Err(PURGED_MEANWHILE.into());
    }
    let now = Utc::now().to_rfc3339();
    let mut tx = conn.begin().await.map_err(err("Database busy"))?;
    let thread = match existing {
        Some(mut t) => {
            sqlx::query("UPDATE chat_threads SET scope_kind = ?, scope_value = ?, updated_at = ? WHERE id = ?")
                .bind(scope.kind.as_str())
                .bind(&scope.value)
                .bind(&now)
                .bind(&t.id)
                .execute(&mut *tx)
                .await
                .map_err(err("Couldn't save the chat"))?;
            t.scope = scope.clone();
            t.updated_at = now.clone();
            t
        }
        None => {
            let t = ChatThread {
                id: uuid::Uuid::new_v4().simple().to_string(),
                title: title_of(question),
                scope: scope.clone(),
                flag: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            sqlx::query(
                "INSERT INTO chat_threads (id, title, scope_kind, scope_value, flag, created_at, updated_at) VALUES (?, ?, ?, ?, NULL, ?, ?)",
            )
            .bind(&t.id)
            .bind(&t.title)
            .bind(scope.kind.as_str())
            .bind(&scope.value)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(err("Couldn't save the chat"))?;
            t
        }
    };
    let user = ChatMessage {
        id: uuid::Uuid::new_v4().simple().to_string(),
        thread_id: thread.id.clone(),
        role: "user".into(),
        content: question.trim().to_string(),
        citations: Vec::new(),
        scope_label: None,
        created_at: now.clone(),
    };
    // The answer sorts after the question even within one clock tick
    let later = (Utc::now() + chrono::Duration::milliseconds(1)).to_rfc3339();
    let assistant = ChatMessage {
        id: uuid::Uuid::new_v4().simple().to_string(),
        thread_id: thread.id.clone(),
        role: "assistant".into(),
        content: ans.content.clone(),
        citations: ans.citations(),
        scope_label: Some(ans.scope_label.clone()),
        created_at: later,
    };
    for m in [&user, &assistant] {
        sqlx::query(
            "INSERT INTO chat_messages (id, thread_id, role, content, citations, scope_label, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&m.id)
        .bind(&m.thread_id)
        .bind(&m.role)
        .bind(&m.content)
        .bind(serde_json::to_string(&m.citations).unwrap_or_else(|_| "[]".into()))
        .bind(&m.scope_label)
        .bind(&m.created_at)
        .execute(&mut *tx)
        .await
        .map_err(err("Couldn't save the chat"))?;
    }
    let mut seen: Vec<&str> = Vec::new();
    // An answer about a recording is that recording's, even when it was made
    // from the people and the event alone: it goes when the recording does
    let about: Vec<&str> = match (in_the_moment, scope.value.as_deref()) {
        (Some(_), Some(id)) => vec![id],
        _ => Vec::new(),
    };
    for id in &about {
        seen.push(id);
        sqlx::query("INSERT OR IGNORE INTO chat_message_sources (message_id, meeting_id) VALUES (?, ?)")
            .bind(&assistant.id)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(err("Couldn't save the chat"))?;
    }
    for p in &ans.passages {
        if seen.contains(&p.meeting_id.as_str()) {
            continue;
        }
        seen.push(&p.meeting_id);
        sqlx::query("INSERT OR IGNORE INTO chat_message_sources (message_id, meeting_id) VALUES (?, ?)")
            .bind(&assistant.id)
            .bind(&p.meeting_id)
            .execute(&mut *tx)
            .await
            .map_err(err("Couldn't save the chat"))?;
    }
    // The write lock is this transaction's now. A purge has either finished
    // (and is counted: nothing is kept) or runs after this commit and
    // deletes the answer by its sources
    if watch.purged() {
        return Err(PURGED_MEANWHILE.into());
    }
    tx.commit().await.map_err(err("Couldn't save the chat"))?;
    Ok(ChatTurn { thread, user, assistant })
}

// ═══════════════════════════════════════════════════════════════════════════
// Tauri commands
// ═══════════════════════════════════════════════════════════════════════════

/// `{ requestId, text }`: the answer to a question while it is written.
/// `text` is the whole answer so far, cleaned like the finished one. The
/// finished turn is the command's own result; this event carries nothing
/// that isn't in it, and is never logged.
pub const STREAM_EVENT: &str = "chat_stream";

/// Lets a call through at most once per `every` (the first one at once).
/// An answer so far replaces the one before it, so a skipped one is no loss.
pub struct Throttle {
    every: std::time::Duration,
    last: parking_lot::Mutex<Option<std::time::Instant>>,
}

impl Throttle {
    pub fn new(every: std::time::Duration) -> Self {
        Self { every, last: parking_lot::Mutex::new(None) }
    }
    pub fn ready(&self) -> bool {
        let mut last = self.last.lock();
        if last.is_some_and(|t| t.elapsed() < self.every) {
            return false;
        }
        *last = Some(std::time::Instant::now());
        true
    }
}

/// Where an answer goes while it is written: `show` gets the answer so far
/// (cleaned like the finished one), a few times a second at most. Once text
/// was removed from a recording of the library (`watch`) the answer may
/// hold it: `show` gets one empty answer, which takes it off the screen,
/// and then nothing more (`ask_as` drops the turn).
pub fn stream_sink(every: std::time::Duration, watch: PurgeWatch, show: impl Fn(String) + Send + Sync) -> impl Fn(&str) + Send + Sync {
    stream_sink_until(every, move || watch.purged(), show)
}

/// `stream_sink` with any "stop now".
pub fn stream_sink_until(
    every: std::time::Duration,
    stop: impl Fn() -> bool + Send + Sync,
    show: impl Fn(String) + Send + Sync,
) -> impl Fn(&str) + Send + Sync {
    let throttle = Throttle::new(every);
    let stopped = std::sync::atomic::AtomicBool::new(false);
    move |so_far: &str| {
        if stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if stop() {
            // What is on screen of it goes too: an empty answer so far
            stopped.store(true, std::sync::atomic::Ordering::SeqCst);
            show(String::new());
            return;
        }
        if !throttle.ready() {
            return;
        }
        let text = prompt::clean_partial(so_far);
        if !text.is_empty() {
            show(text);
        }
    }
}

pub mod commands {
    use super::*;
    use crate::study::StreamingCompleter;
    use crate::AppState;
    use tauri::{AppHandle, Emitter, State};

    /// The window redraws the answer this often at most while it is written
    const STREAM_EVERY: std::time::Duration = std::time::Duration::from_millis(40);

    /// The sink for a question asked with a `requestId`: the answer so far
    /// goes to the window as `STREAM_EVENT`.
    fn to_window(app: AppHandle, watch: PurgeWatch, request_id: Option<String>) -> impl Fn(&str) + Send + Sync {
        stream_sink(STREAM_EVERY, watch, move |text| {
            if let Some(id) = &request_id {
                let _ = app.emit(STREAM_EVENT, serde_json::json!({ "requestId": id, "text": text }));
            }
        })
    }

    /// This recording is the one being recorded right now.
    fn recording_now(state: &AppState, meeting_id: &str) -> bool {
        state.capture_engine.read().is_recording() && state.state_builder.read().current_meeting_id().as_deref() == Some(meeting_id)
    }

    /// Ask a question in a thread (None starts a new one). Consent and Pro
    /// are checked by `ai::complete_text`; the UI turns their errors into
    /// the dialogs.
    ///
    /// `request_id`: the window's name for this question. The answer so far
    /// is sent to it as `STREAM_EVENT` while the model writes; without one,
    /// nothing is sent until the turn is returned.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn chat_ask(
        app: AppHandle,
        state: State<'_, AppState>,
        thread_id: Option<String>,
        scope: Scope,
        message: String,
        request_id: Option<String>,
    ) -> Result<ChatTurn, String> {
        log::info!("Chat: question of {} chars, scope {}", message.chars().count(), scope.kind.as_str());
        let sink = to_window(app, PurgeWatch::of(state.database.pool()).await?, request_id);
        let c = StreamingCompleter { on_text: &sink };
        // One recording: answered the way Ask does (its people, the user's
        // notes, the last minutes), and from that recording alone: the user
        // narrowed the chat to it
        if let (ScopeKind::Meeting, Some(meeting_id)) = (scope.kind, scope.value.as_deref()) {
            let mode = live::Mode { recording: recording_now(&state, meeting_id), related: false, glance: false };
            return ask_recording(state.database.pool(), &c, thread_id.as_deref(), meeting_id, &message, mode).await;
        }
        ask(state.database.pool(), &c, thread_id.as_deref(), &scope, &message).await
    }

    /// Ask about one recording in the moment, also while it is still being
    /// recorded. Only ever on the user's own question: nothing calls this
    /// by itself. Consent and Pro as for `chat_ask`.
    /// `request_id` as for `chat_ask`.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn chat_ask_recording(
        app: AppHandle,
        state: State<'_, AppState>,
        thread_id: Option<String>,
        meeting_id: String,
        message: String,
        request_id: Option<String>,
    ) -> Result<ChatTurn, String> {
        log::info!("Ask: question of {} chars about one recording", message.chars().count());
        let mode = live::Mode { recording: recording_now(&state, &meeting_id), related: true, glance: false };
        let sink = to_window(app, PurgeWatch::of(state.database.pool()).await?, request_id);
        let c = StreamingCompleter { on_text: &sink };
        ask_recording(state.database.pool(), &c, thread_id.as_deref(), &meeting_id, &message, mode).await
    }

    /// What a question about this recording would draw on (no AI call).
    #[tauri::command(rename_all = "camelCase")]
    pub async fn chat_recording_knows(state: State<'_, AppState>, meeting_id: String) -> Result<live::Knows, String> {
        let recording = recording_now(&state, &meeting_id);
        let mut conn = state.database.pool().acquire().await.map_err(err("Database busy"))?;
        live::knows(&mut conn, &meeting_id, recording).await
    }

    /// The newest chat about this recording, to pick the conversation up
    /// where it was left (the Ask panel).
    #[tauri::command(rename_all = "camelCase")]
    pub async fn chat_thread_for_recording(state: State<'_, AppState>, meeting_id: String) -> Result<Option<ChatThreadDetail>, String> {
        let id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM chat_threads WHERE scope_kind = 'meeting' AND scope_value = ? ORDER BY updated_at DESC LIMIT 1",
        )
        .bind(&meeting_id)
        .fetch_optional(state.database.pool())
        .await
        .map_err(err("Database busy"))?;
        match id {
            Some(id) => Ok(Some(get_thread(state.database.pool(), &id).await?)),
            None => Ok(None),
        }
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn list_chat_threads(state: State<'_, AppState>) -> Result<Vec<ChatThread>, String> {
        list_threads(state.database.pool()).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_chat_thread(state: State<'_, AppState>, thread_id: String) -> Result<ChatThreadDetail, String> {
        get_thread(state.database.pool(), &thread_id).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn delete_chat_thread(state: State<'_, AppState>, thread_id: String) -> Result<(), String> {
        delete_thread(state.database.pool(), &thread_id).await
    }

    /// What a scope covers, for the scope picker and the suggested
    /// questions (built locally, no AI call).
    #[tauri::command(rename_all = "camelCase")]
    pub async fn chat_scope_summary(state: State<'_, AppState>, scope: Scope) -> Result<retrieval::ScopeSummary, String> {
        let mut conn = state.database.pool().acquire().await.map_err(err("Database busy"))?;
        retrieval::summary(&mut conn, &scope).await
    }
}
