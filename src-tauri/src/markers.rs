//! Moment markers (docs/STUDY_TOOLS.md): while recording, the user marks a
//! moment as ★ Important, ? Question or ✎ (labelled by the recording's
//! type: On the test for a class, Follow up for a meeting, Remember for
//! personal; stored as `test` for all three), with an optional short note.
//! One tap marks ★; the kind and note can be changed afterwards.
//!
//! A fourth stored kind, `note`, is a line the user typed while listening
//! (docs/design/FADELL_V2.md §6): the same row, the same clock, so Rewind,
//! time-range Delete, the study guide, Chat, Export and sync handle it the
//! way they handle a mark. It is never one of the kinds a mark is offered
//! or changed to, and it always has text.
//!
//! Markers live in `meeting_markers` (this module owns the table) and go
//! with their meeting (`ON DELETE CASCADE`). A note is user content: a
//! time-range Delete/Strike removes the markers inside the range
//! (`redaction::time_range::find_range_extras` / `purge_range_extras`, at
//! commit, so the 5-second undo keeps them). Word and line edits leave
//! markers alone, like user comments (docs/REDACTION.md).
//!
//! Notes and marker text are never logged.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnection;
use sqlx::{Pool, Row, Sqlite};

/// The mark kinds, in the order the UI offers them.
pub const KINDS: &[&str] = &["important", "question", "test"];
/// A line the user typed while listening. Stored beside the marks, never
/// offered as a mark kind.
pub const NOTE_KIND: &str = "note";
/// What a one-tap mark is.
pub const DEFAULT_KIND: &str = "important";
/// Long enough for "ask about the second proof", short enough to stay a note.
pub const MAX_NOTE_CHARS: usize = 280;
/// A typed note is one line of the user's own notes: a sentence or two.
pub const MAX_TYPED_NOTE_CHARS: usize = 1000;
/// A mark this soon after the previous one in the same meeting is the same
/// mark (the hotkey and the menu accelerator, or a double tap).
pub const DEBOUNCE_MS: i64 = 800;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Marker {
    pub id: String,
    pub meeting_id: String,
    /// Wall clock, RFC 3339 (same clock as transcript lines and screens)
    pub ts: String,
    /// "important" | "question" | "test", or "note" for a typed note
    pub kind: String,
    pub note: Option<String>,
    pub created_at: String,
    /// ms from the meeting start: the Recordings timeline's clock
    pub offset_ms: i64,
}

/// ★ / ? / ✎ (and ✍ for a typed note), for prompts and exports.
pub fn symbol(kind: &str) -> &'static str {
    match kind {
        "question" => "?",
        "test" => "✎",
        NOTE_KIND => "✍",
        _ => "★",
    }
}

/// The kind's label in a recording of type `rec`: the stored `test` kind is
/// "On the test" in a class, "Follow up" in a meeting, "Remember" in a
/// personal recording.
pub fn label(kind: &str, rec: crate::recording_kind::RecordingKind) -> &'static str {
    match kind {
        "question" => "Question",
        "test" => rec.third_mark_label(),
        NOTE_KIND => "Note",
        _ => "Important",
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Schema (runs inside DatabaseManager::run_migrations on its one connection)
// ═══════════════════════════════════════════════════════════════════════════

fn create_table_sql(name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {} (
            id TEXT PRIMARY KEY,
            meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            ts TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('important', 'question', 'test', 'note')),
            note TEXT,
            created_at TEXT NOT NULL
        )
        "#,
        name
    )
}

pub async fn ensure_schema(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    use sqlx::Connection;
    sqlx::query(&create_table_sql("meeting_markers")).execute(&mut *conn).await?;
    // Tables made before typed notes only allow the three mark kinds. A
    // CHECK can't be altered, so the table is rebuilt once, in one
    // transaction: copy into a new table, drop the old one, rename the new
    // one into place (the order SQLite documents, so nothing that names
    // `meeting_markers` is rewritten). Dropping the old table drops its
    // sync triggers; sync::store::ensure_schema runs after this and makes
    // them again.
    let sql: Option<String> =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'meeting_markers'")
            .fetch_optional(&mut *conn)
            .await?;
    if sql.map_or(false, |s| !s.contains("'note'")) {
        let mut tx = conn.begin().await?;
        sqlx::query("DROP TABLE IF EXISTS meeting_markers_rebuild").execute(&mut *tx).await?;
        sqlx::query(&create_table_sql("meeting_markers_rebuild")).execute(&mut *tx).await?;
        let n = sqlx::query(
            "INSERT INTO meeting_markers_rebuild (id, meeting_id, ts, kind, note, created_at) \
             SELECT id, meeting_id, ts, kind, note, created_at FROM meeting_markers",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        sqlx::query("DROP TABLE meeting_markers").execute(&mut *tx).await?;
        sqlx::query("ALTER TABLE meeting_markers_rebuild RENAME TO meeting_markers").execute(&mut *tx).await?;
        tx.commit().await?;
        log::info!("Markers: table rebuilt to allow typed notes ({} kept)", n);
    }
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_meeting_markers_meeting ON meeting_markers(meeting_id, ts)")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Validation (pure)
// ═══════════════════════════════════════════════════════════════════════════

/// `None` → ★ (a one-tap mark). Unknown kinds are refused, and so is
/// `note`: a typed note is never a kind a mark can be given.
pub fn normalize_kind(kind: Option<&str>) -> Result<&'static str, String> {
    match kind.map(str::trim).filter(|k| !k.is_empty()) {
        None => Ok(DEFAULT_KIND),
        Some(k) => KINDS
            .iter()
            .find(|x| x.eq_ignore_ascii_case(k))
            .copied()
            .ok_or_else(|| "Unknown marker type".to_string()),
    }
}

/// Like [`normalize_kind`], and also accepts `note`.
fn stored_kind(kind: Option<&str>) -> Result<&'static str, String> {
    match kind.map(str::trim) {
        Some(k) if k.eq_ignore_ascii_case(NOTE_KIND) => Ok(NOTE_KIND),
        other => normalize_kind(other),
    }
}

pub fn is_note(kind: &str) -> bool {
    kind == NOTE_KIND
}

fn clean_text(note: Option<&str>, max: usize) -> Result<Option<String>, String> {
    let Some(n) = note else { return Ok(None) };
    let n = n.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        return Ok(None);
    }
    if n.chars().count() > max {
        return Err(format!("Keep the note under {} characters", max));
    }
    Ok(Some(n))
}

/// A mark's note: trimmed, whitespace runs closed up to one space, at most
/// [`MAX_NOTE_CHARS`]; empty → no note.
pub fn clean_note(note: Option<&str>) -> Result<Option<String>, String> {
    clean_text(note, MAX_NOTE_CHARS)
}

/// A typed note: one line, at most [`MAX_TYPED_NOTE_CHARS`], never empty.
pub fn clean_typed_note(text: &str) -> Result<String, String> {
    clean_text(Some(text), MAX_TYPED_NOTE_CHARS)?.ok_or_else(|| "Type something first.".to_string())
}

/// The note as it may be stored for `kind`.
fn clean_for(kind: &str, note: Option<&str>) -> Result<Option<String>, String> {
    if is_note(kind) {
        clean_typed_note(note.unwrap_or_default()).map(Some)
    } else {
        clean_note(note)
    }
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

// ═══════════════════════════════════════════════════════════════════════════
// Store
// ═══════════════════════════════════════════════════════════════════════════

async fn meeting_start(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<DateTime<Utc>, String> {
    let s: Option<String> = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(err("Database busy"))?;
    s.as_deref().and_then(parse_ts).ok_or_else(|| "That meeting no longer exists".to_string())
}

fn from_row(r: &sqlx::sqlite::SqliteRow, start: Option<DateTime<Utc>>) -> Marker {
    let ts: String = r.get("ts");
    let offset_ms = match (parse_ts(&ts), start) {
        (Some(t), Some(s)) => (t - s).num_milliseconds().max(0),
        _ => 0,
    };
    Marker {
        id: r.get("id"),
        meeting_id: r.get("meeting_id"),
        ts,
        kind: r.get("kind"),
        note: r.get("note"),
        created_at: r.get("created_at"),
        offset_ms,
    }
}

async fn get(pool: &Pool<Sqlite>, id: &str) -> Result<Marker, String> {
    let row = sqlx::query(
        "SELECT k.id, k.meeting_id, k.ts, k.kind, k.note, k.created_at, m.started_at \
         FROM meeting_markers k JOIN meetings m ON m.id = k.meeting_id WHERE k.id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(err("Database busy"))?
    .ok_or("That marker no longer exists")?;
    let start = parse_ts(&row.get::<String, _>("started_at"));
    Ok(from_row(&row, start))
}

/// Add a marker at `ts` (wall clock). With `debounce`, a mark within
/// [`DEBOUNCE_MS`] after the meeting's latest mark returns that one
/// instead. A typed note (`kind` = `note`) needs text and is never folded
/// into another row.
pub async fn add(
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    ts: DateTime<Utc>,
    kind: Option<&str>,
    note: Option<&str>,
    debounce: bool,
) -> Result<Marker, String> {
    let kind = stored_kind(kind)?;
    let note = clean_for(kind, note)?;
    meeting_start(pool, meeting_id).await?;
    if debounce && !is_note(kind) {
        let last: Option<(String, String)> = sqlx::query_as(
            "SELECT id, ts FROM meeting_markers WHERE meeting_id = ? AND kind != 'note' ORDER BY ts DESC LIMIT 1",
        )
                .bind(meeting_id)
                .fetch_optional(pool)
                .await
                .map_err(err("Database busy"))?;
        if let Some((id, last_ts)) = last {
            if parse_ts(&last_ts).map_or(false, |t| (ts - t).num_milliseconds().abs() < DEBOUNCE_MS) {
                return get(pool, &id).await;
            }
        }
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query("INSERT INTO meeting_markers (id, meeting_id, ts, kind, note, created_at) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(meeting_id)
        .bind(ts.to_rfc3339())
        .bind(kind)
        .bind(&note)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(err("Couldn't save the marker"))?;
    if is_note(kind) {
        typed_notes_changed(pool, meeting_id).await;
    }
    get(pool, &id).await
}

/// Tell the notes a typed note changed (meeting_notes.rs): details of notes
/// that are gone are dropped and the notes are sent again. Best effort: the
/// note itself is already saved.
async fn typed_notes_changed(pool: &Pool<Sqlite>, meeting_id: &str) {
    let Ok(mut conn) = pool.acquire().await else { return };
    if let Err(e) = crate::meeting_notes::typed_notes_changed(&mut conn, meeting_id).await {
        log::warn!("Markers: couldn't update the notes after a typed note changed: {}", e);
    }
}

/// Add a marker `offset_ms` after the meeting start (marking while
/// reviewing a recording).
pub async fn add_at_offset(
    pool: &Pool<Sqlite>,
    meeting_id: &str,
    offset_ms: i64,
    kind: Option<&str>,
    note: Option<&str>,
) -> Result<Marker, String> {
    let start = meeting_start(pool, meeting_id).await?;
    add(pool, meeting_id, start + chrono::Duration::milliseconds(offset_ms.max(0)), kind, note, false).await
}

/// Change a marker's kind and/or note. `note: Some(None)` (or an empty
/// note) removes a mark's note; `None` keeps it. A typed note can have its
/// text changed but not emptied (delete it instead) and not its kind.
pub async fn update(
    pool: &Pool<Sqlite>,
    id: &str,
    kind: Option<&str>,
    note: Option<Option<&str>>,
) -> Result<Marker, String> {
    let current = get(pool, id).await?;
    // A typed note stays a note and a mark stays a mark
    let kind = match kind {
        Some(_) if is_note(&current.kind) => return Err("A note can't be changed into a mark.".into()),
        Some(k) => normalize_kind(Some(k))?.to_string(),
        None => current.kind.clone(),
    };
    let note = match note {
        Some(n) => clean_for(&kind, n)?,
        None => current.note.clone(),
    };
    sqlx::query("UPDATE meeting_markers SET kind = ?, note = ? WHERE id = ?")
        .bind(&kind)
        .bind(&note)
        .bind(id)
        .execute(pool)
        .await
        .map_err(err("Couldn't save the marker"))?;
    if is_note(&kind) {
        typed_notes_changed(pool, &current.meeting_id).await;
    }
    get(pool, id).await
}

pub async fn delete(pool: &Pool<Sqlite>, id: &str) -> Result<(), String> {
    let row: Option<(String, String)> = sqlx::query_as("SELECT meeting_id, kind FROM meeting_markers WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(err("Database busy"))?;
    sqlx::query("DELETE FROM meeting_markers WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(err("Couldn't delete the marker"))?;
    if let Some((meeting_id, kind)) = row {
        if is_note(&kind) {
            typed_notes_changed(pool, &meeting_id).await;
        }
    }
    Ok(())
}

/// When a note typed at `at` (the moment Return was pressed) should be
/// stamped: that moment when it is a sane one (not in the future, not more
/// than a minute old), else now.
pub fn note_time(at: Option<&str>, now: DateTime<Utc>) -> DateTime<Utc> {
    at.and_then(parse_ts)
        .filter(|t| *t <= now + chrono::Duration::seconds(2) && now - *t <= chrono::Duration::seconds(60))
        .map(|t| t.min(now))
        .unwrap_or(now)
}

/// A recording that stopped this recently still takes a note that was
/// typed in it (Return, then Stop, and the save arrives after the stop).
pub const LATE_NOTE_SECS: i64 = 120;

/// A meeting's markers in time order.
pub async fn list(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<Vec<Marker>, String> {
    let start = parse_ts(
        &sqlx::query_scalar::<_, String>("SELECT started_at FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_optional(pool)
            .await
            .map_err(err("Database busy"))?
            .unwrap_or_default(),
    );
    let rows = sqlx::query(
        "SELECT id, meeting_id, ts, kind, note, created_at FROM meeting_markers WHERE meeting_id = ? ORDER BY ts ASC, created_at ASC",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(err("Database busy"))?;
    Ok(rows.iter().map(|r| from_row(r, start)).collect())
}

// ═══════════════════════════════════════════════════════════════════════════
// Tauri commands
// ═══════════════════════════════════════════════════════════════════════════

pub mod commands {
    use super::*;
    use crate::AppState;
    use tauri::{AppHandle, Emitter, Manager, State};

    /// Event carrying the new [`Marker`] (CaptureBar shows it and offers the
    /// kind/note chooser; the Recordings view refreshes).
    pub const MARKER_ADDED_EVENT: &str = "marker_added";
    /// Event when a mark was asked for but nothing is being recorded.
    pub const MARKER_FAILED_EVENT: &str = "marker_failed";
    /// Event carrying a typed note just saved (the live transcript shows it
    /// where it was written). Separate from [`MARKER_ADDED_EVENT`], which
    /// opens the capture bar's kind chooser.
    pub const NOTE_ADDED_EVENT: &str = "note_added";

    /// The meeting being recorded right now, if any.
    fn recording_meeting(state: &AppState) -> Option<String> {
        if !state.capture_engine.read().is_recording() {
            return None;
        }
        state.state_builder.read().current_meeting_id()
    }

    /// Mark now in the meeting being recorded (Mark button, menu item and
    /// the global hotkey). Debounced, so the hotkey and the menu accelerator
    /// firing for one key press make one marker.
    pub async fn mark_now(
        app: &AppHandle,
        state: &AppState,
        kind: Option<&str>,
        note: Option<&str>,
    ) -> Result<Marker, String> {
        let meeting_id = recording_meeting(state).ok_or("Start a recording to mark a moment.")?;
        // One of the three mark kinds (a typed note goes through `add_note`)
        let kind = normalize_kind(kind)?;
        let m = add(state.database.pool(), &meeting_id, Utc::now(), Some(kind), note, true).await?;
        let _ = app.emit(MARKER_ADDED_EVENT, &m);
        Ok(m)
    }

    /// Global hotkey / menu: mark ★ in the meeting being recorded.
    pub fn mark_from_shortcut(app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let Some(state) = app.try_state::<AppState>() else { return };
            if let Err(e) = mark_now(&app, &state, None, None).await {
                let _ = app.emit(MARKER_FAILED_EVENT, e);
            }
        });
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn mark_moment(
        app: AppHandle,
        state: State<'_, AppState>,
        kind: Option<String>,
        note: Option<String>,
    ) -> Result<Marker, String> {
        mark_now(&app, &state, kind.as_deref(), note.as_deref()).await
    }

    /// Keep a line the user typed in the recording in progress (the note
    /// field under the live transcript), stamped with the moment Return was
    /// pressed (`at`). `meeting_id` is the recording it was typed in: if
    /// that recording stopped between the keypress and this call, the line
    /// is still kept on it, never lost and never put on another recording.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn add_note(
        app: AppHandle,
        state: State<'_, AppState>,
        text: String,
        meeting_id: Option<String>,
        at: Option<String>,
    ) -> Result<Marker, String> {
        let now = Utc::now();
        let pool = state.database.pool();
        let live = recording_meeting(&state);
        let (target, ts) = match (&live, &meeting_id) {
            (Some(l), Some(m)) if l == m => (l.clone(), note_time(at.as_deref(), now)),
            (Some(l), None) => (l.clone(), note_time(at.as_deref(), now)),
            // Typed in a recording that has just stopped
            (_, Some(m)) => {
                let ended: Option<Option<String>> = sqlx::query_scalar("SELECT ended_at FROM meetings WHERE id = ?")
                    .bind(m)
                    .fetch_optional(pool)
                    .await
                    .map_err(err("Database busy"))?;
                let ended = ended.flatten().as_deref().and_then(parse_ts);
                match ended {
                    Some(end) if (now - end).num_seconds() <= LATE_NOTE_SECS => (m.clone(), note_time(at.as_deref(), now).min(end)),
                    _ => return Err("That recording has ended. Add the note in its Notes.".into()),
                }
            }
            (None, None) => return Err("Start a recording to type a note.".into()),
        };
        let m = add(pool, &target, ts, Some(NOTE_KIND), Some(&text), false).await?;
        let _ = app.emit(NOTE_ADDED_EVENT, &m);
        Ok(m)
    }

    /// Mark a moment of a recorded meeting (Recordings view). `kind: "note"`
    /// with text adds a typed note at that moment.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn add_marker(
        state: State<'_, AppState>,
        meeting_id: String,
        offset_ms: i64,
        kind: Option<String>,
        note: Option<String>,
    ) -> Result<Marker, String> {
        add_at_offset(state.database.pool(), &meeting_id, offset_ms, kind.as_deref(), note.as_deref()).await
    }

    /// `clearNote: true` removes the note; otherwise `note` replaces it when given.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn update_marker(
        state: State<'_, AppState>,
        id: String,
        kind: Option<String>,
        note: Option<String>,
        clear_note: Option<bool>,
    ) -> Result<Marker, String> {
        let note_arg: Option<Option<&str>> = if clear_note.unwrap_or(false) {
            Some(None)
        } else {
            note.as_deref().map(Some)
        };
        update(state.database.pool(), &id, kind.as_deref(), note_arg).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn delete_marker(state: State<'_, AppState>, id: String) -> Result<(), String> {
        delete(state.database.pool(), &id).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn list_markers(state: State<'_, AppState>, meeting_id: String) -> Result<Vec<Marker>, String> {
        list(state.database.pool(), &meeting_id).await
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Global hotkey
// ═══════════════════════════════════════════════════════════════════════════

/// ⌃⌥⌘M marks ★ while another app (slides, a video call, a browser) is in
/// front. Chosen not to collide: the app menu uses ⌘, ⌘N ⌘. ⌘1 ⌘2 ⌘⇧I ⌘⇧P
/// ⌘K, the tray ⌘⇧N, the capture bar ⌘⇧S; macOS itself uses ⌥⌘M (minimize
/// all) and VoiceOver ⌃⌥ + letter, and ⌃⌥⌘M is unused. The same key is the
/// File → Mark Moment accelerator when the app is in front (debounced, so
/// the two never make two markers).
pub fn hotkey() -> tauri_plugin_global_shortcut::Shortcut {
    use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER), Code::KeyM)
}

/// The same key as a menu accelerator.
pub const MENU_ACCELERATOR: &str = "Ctrl+Alt+Cmd+M";

/// Register the global ⌃⌥⌘M at launch. If another app owns the key, the
/// app still starts (File → Mark Moment and the Mark button work); the
/// failure is logged. (Registering inside the plugin builder instead would
/// make a taken key fail the whole app's setup.)
pub fn register_hotkey(app: &tauri::AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let result = app.global_shortcut().on_shortcut(hotkey(), |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            commands::mark_from_shortcut(app);
        }
    });
    match result {
        Ok(()) => log::info!("Mark hotkey ⌃⌥⌘M registered"),
        Err(e) => log::warn!("Mark hotkey ⌃⌥⌘M not available ({}); File → Mark Moment still works", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;

    async fn db() -> (DatabaseManager, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("nf-markers-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = DatabaseManager::new(&dir.join("t.db")).await.unwrap();
        db.run_migrations().await.unwrap();
        db.create_meeting("m1", "Biology 101").await.unwrap();
        (db, dir)
    }

    #[test]
    fn kinds_and_notes_are_validated() {
        assert_eq!(normalize_kind(None).unwrap(), "important");
        assert_eq!(normalize_kind(Some("  ")).unwrap(), "important");
        assert_eq!(normalize_kind(Some("TEST")).unwrap(), "test");
        assert_eq!(normalize_kind(Some("question")).unwrap(), "question");
        assert!(normalize_kind(Some("urgent")).is_err());
        assert_eq!(clean_note(Some("  ask   about\n proof 2 ")).unwrap().as_deref(), Some("ask about proof 2"));
        assert_eq!(clean_note(Some("   ")).unwrap(), None);
        assert_eq!(clean_note(None).unwrap(), None);
        assert!(clean_note(Some(&"x".repeat(MAX_NOTE_CHARS + 1))).is_err());
        assert!(clean_note(Some(&"é".repeat(MAX_NOTE_CHARS))).unwrap().is_some(), "counts characters, not bytes");
        assert_eq!((symbol("important"), symbol("question"), symbol("test")), ("★", "?", "✎"));
    }

    #[test]
    fn labels_follow_the_recording_type_and_the_stored_kind_does_not() {
        use crate::recording_kind::RecordingKind::*;
        assert_eq!(label("test", Class), "On the test");
        assert_eq!(label("test", Meeting), "Follow up");
        assert_eq!(label("test", Personal), "Remember");
        for rec in [Class, Meeting, Personal] {
            assert_eq!(label("important", rec), "Important");
            assert_eq!(label("question", rec), "Question");
        }
        // One stored value for the third mark, whatever the type
        assert_eq!(KINDS, &["important", "question", "test"]);
    }

    #[tokio::test]
    async fn marker_crud_and_offsets() {
        let (db, dir) = db().await;
        let pool = db.pool();
        let start = parse_ts(&sqlx::query_scalar::<_, String>("SELECT started_at FROM meetings WHERE id = 'm1'")
            .fetch_one(pool)
            .await
            .unwrap())
        .unwrap();

        // One tap: ★, no note
        let a = add(pool, "m1", start + chrono::Duration::seconds(90), None, None, false).await.unwrap();
        assert_eq!((a.kind.as_str(), a.note.as_deref(), a.offset_ms), ("important", None, 90_000));
        // Typed, with a note; and one added while reviewing, by offset
        let b = add(pool, "m1", start + chrono::Duration::seconds(30), Some("question"), Some("why?"), false)
            .await
            .unwrap();
        let c = add_at_offset(pool, "m1", 600_000, Some("test"), None).await.unwrap();
        assert_eq!(c.offset_ms, 600_000);
        let all = list(pool, "m1").await.unwrap();
        assert_eq!(all.iter().map(|m| m.id.clone()).collect::<Vec<_>>(), vec![b.id.clone(), a.id.clone(), c.id.clone()]);

        // Change the type afterwards; set, keep and clear the note
        let a2 = update(pool, &a.id, Some("test"), None).await.unwrap();
        assert_eq!((a2.kind.as_str(), a2.note.as_deref()), ("test", None));
        let a3 = update(pool, &a.id, None, Some(Some("chapter 4"))).await.unwrap();
        assert_eq!((a3.kind.as_str(), a3.note.as_deref()), ("test", Some("chapter 4")));
        let a4 = update(pool, &a.id, Some("important"), None).await.unwrap();
        assert_eq!(a4.note.as_deref(), Some("chapter 4"), "a kind change keeps the note");
        let a5 = update(pool, &a.id, None, Some(None)).await.unwrap();
        assert_eq!(a5.note, None);
        assert!(update(pool, &a.id, Some("nope"), None).await.is_err());

        delete(pool, &b.id).await.unwrap();
        assert_eq!(list(pool, "m1").await.unwrap().len(), 2);
        assert!(update(pool, &b.id, Some("test"), None).await.is_err());
        // Unknown meeting
        assert!(add(pool, "nope", Utc::now(), None, None, false).await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn debounce_folds_a_double_press_into_one_marker() {
        let (db, dir) = db().await;
        let pool = db.pool();
        let t = Utc::now();
        let a = add(pool, "m1", t, None, None, true).await.unwrap();
        let b = add(pool, "m1", t + chrono::Duration::milliseconds(300), None, None, true).await.unwrap();
        assert_eq!(a.id, b.id);
        let c = add(pool, "m1", t + chrono::Duration::milliseconds(DEBOUNCE_MS + 50), None, None, true).await.unwrap();
        assert_ne!(a.id, c.id);
        assert_eq!(list(pool, "m1").await.unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn meeting_delete_removes_its_markers() {
        let (db, dir) = db().await;
        let pool = db.pool();
        db.create_meeting("m2", "Other").await.unwrap();
        add(pool, "m1", Utc::now(), Some("question"), Some("private note"), false).await.unwrap();
        add(pool, "m2", Utc::now(), None, None, false).await.unwrap();
        db.delete_meeting("m1").await.unwrap();
        let left: Vec<String> = sqlx::query_scalar("SELECT meeting_id FROM meeting_markers").fetch_all(pool).await.unwrap();
        assert_eq!(left, vec!["m2".to_string()]);
        let notes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_markers WHERE note = 'private note'")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(notes, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_typed_note_is_not_a_mark_kind() {
        // Offered kinds are still the three marks
        assert_eq!(KINDS, &["important", "question", "test"]);
        assert!(normalize_kind(Some("note")).is_err(), "a mark can't be given the note kind");
        assert_eq!(stored_kind(Some("NOTE")).unwrap(), "note");
        assert_eq!(stored_kind(None).unwrap(), "important");
        assert_eq!(symbol("note"), "✍");
        for rec in [
            crate::recording_kind::RecordingKind::Class,
            crate::recording_kind::RecordingKind::Meeting,
            crate::recording_kind::RecordingKind::Personal,
        ] {
            assert_eq!(label("note", rec), "Note");
        }
        // One line, longer than a mark's note, never empty
        assert_eq!(clean_typed_note("  the pump   moves\n3 Na out ").unwrap(), "the pump moves 3 Na out");
        assert!(clean_typed_note("   ").is_err());
        assert!(clean_typed_note(&"x".repeat(MAX_TYPED_NOTE_CHARS)).is_ok());
        assert!(clean_typed_note(&"x".repeat(MAX_TYPED_NOTE_CHARS + 1)).is_err());
        assert!(MAX_TYPED_NOTE_CHARS > MAX_NOTE_CHARS);
    }

    #[test]
    fn a_note_is_stamped_with_the_moment_return_was_pressed_when_that_is_sane() {
        let now = Utc::now();
        let iso = |d: DateTime<Utc>| d.to_rfc3339();
        let pressed = now - chrono::Duration::milliseconds(800);
        assert_eq!(note_time(Some(&iso(pressed)), now), pressed);
        // Missing, unreadable, too old or in the future: now
        assert_eq!(note_time(None, now), now);
        assert_eq!(note_time(Some("yesterday"), now), now);
        assert_eq!(note_time(Some(&iso(now - chrono::Duration::seconds(300))), now), now);
        assert_eq!(note_time(Some(&iso(now + chrono::Duration::seconds(30))), now), now);
    }

    #[tokio::test]
    async fn typed_notes_are_stored_beside_marks_and_never_fold_into_one() {
        let (db, dir) = db().await;
        let pool = db.pool();
        let t = Utc::now();
        let mark = add(pool, "m1", t, None, None, true).await.unwrap();
        // Typed within the debounce window of a mark, and of each other: all kept
        let n1 = add(pool, "m1", t + chrono::Duration::milliseconds(100), Some("note"), Some("sodium out"), true)
            .await
            .unwrap();
        let n2 = add(pool, "m1", t + chrono::Duration::milliseconds(200), Some("note"), Some("potassium in"), true)
            .await
            .unwrap();
        assert_eq!((n1.kind.as_str(), n1.note.as_deref()), ("note", Some("sodium out")));
        assert_ne!(n1.id, n2.id);
        assert_ne!(n1.id, mark.id);
        // A mark right after a note is still its own mark (the debounce looks at marks only)
        let mark2 = add(pool, "m1", t + chrono::Duration::milliseconds(DEBOUNCE_MS + 300), None, None, true).await.unwrap();
        assert_ne!(mark2.id, n2.id);
        assert_eq!(mark2.kind, "important");
        assert_eq!(list(pool, "m1").await.unwrap().len(), 4);

        // A note needs text
        assert!(add(pool, "m1", t, Some("note"), None, false).await.is_err());
        assert!(add(pool, "m1", t, Some("note"), Some("  "), false).await.is_err());
        // A long line fits a note but not a mark
        let long = "y".repeat(MAX_NOTE_CHARS + 50);
        assert!(add(pool, "m1", t, Some("note"), Some(&long), false).await.is_ok());
        assert!(add(pool, "m1", t, Some("important"), Some(&long), false).await.is_err());

        // Its text can change; its kind can't, and it can't be emptied
        let n1b = update(pool, &n1.id, None, Some(Some("sodium out, three of them"))).await.unwrap();
        assert_eq!((n1b.kind.as_str(), n1b.note.as_deref()), ("note", Some("sodium out, three of them")));
        assert!(update(pool, &n1.id, Some("important"), None).await.is_err());
        assert!(update(pool, &n1.id, None, Some(None)).await.is_err());
        // And a mark can't become a note
        assert!(update(pool, &mark.id, Some("note"), None).await.is_err());
        delete(pool, &n2.id).await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn an_older_markers_table_is_rebuilt_once_and_keeps_its_rows() {
        let dir = std::env::temp_dir().join(format!("nf-markers-old-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = DatabaseManager::new(&dir.join("t.db")).await.unwrap();
        db.run_migrations().await.unwrap();
        db.create_meeting("m1", "Biology 101").await.unwrap();
        let pool = db.pool();
        // Put the table back the way 3.9.0 made it, with a row and an index
        sqlx::query("DROP TABLE meeting_markers").execute(pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE meeting_markers (id TEXT PRIMARY KEY, \
             meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE, ts TEXT NOT NULL, \
             kind TEXT NOT NULL CHECK (kind IN ('important', 'question', 'test')), note TEXT, created_at TEXT NOT NULL)",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO meeting_markers VALUES ('old1', 'm1', ?, 'test', 'chapter 4', ?)")
            .bind(Utc::now().to_rfc3339())
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .unwrap();
        assert!(add(pool, "m1", Utc::now(), Some("note"), Some("x"), false).await.is_err(), "the old CHECK refuses a note");

        db.run_migrations().await.unwrap();
        let kept = list(pool, "m1").await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].id.as_str(), kept[0].kind.as_str(), kept[0].note.as_deref()), ("old1", "test", Some("chapter 4")));
        add(pool, "m1", Utc::now(), Some("note"), Some("now it fits"), false).await.unwrap();
        // The sync triggers were made again on the rebuilt table: the new row is tracked
        let tracked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_meta WHERE entity = 'mark'")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(tracked >= 1, "a note written after the rebuild is logged for sync");
        let leftovers: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'meeting_markers_rebuild'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(leftovers, 0);
        // Running again changes nothing
        db.run_migrations().await.unwrap();
        assert_eq!(list(pool, "m1").await.unwrap().len(), 2);
        // The meeting still takes its markers with it
        db.delete_meeting("m1").await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_markers").fetch_one(pool).await.unwrap();
        assert_eq!(left, 0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn schema_rejects_unknown_kinds() {
        let (db, dir) = db().await;
        let r = sqlx::query(
            "INSERT INTO meeting_markers (id, meeting_id, ts, kind, note, created_at) VALUES ('x', 'm1', ?, 'urgent', NULL, ?)",
        )
        .bind(Utc::now().to_rfc3339())
        .bind(Utc::now().to_rfc3339())
        .execute(db.pool())
        .await;
        assert!(r.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
