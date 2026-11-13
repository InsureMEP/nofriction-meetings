//! The notebook page (docs/design/FADELL_V2.md §3 C and §6): everything a
//! notebook's recordings hold, in one place.
//!
//! - **Items**: what each recording's newest notes list as action items
//!   (meeting), announcements and deadlines (class) or to-dos (personal),
//!   with a checkbox.
//! - **Marked**: every ✎ and ? mark in the notebook, with its note and time.
//! - **Review everything**: the flashcards and quiz questions of every
//!   stored study guide in the notebook, as one deck and one quiz.
//!
//! Nothing here calls the AI and nothing here stores recording content a
//! second time: the page is read from the notes, marks and guides that
//! exist, and a checkbox is kept as a hash of the item's text
//! (`notebook_item_done`, deleted with its recording). So a Delete or
//! Strike that rewrites the notes or deletes a guide changes this page with
//! no purge step of its own.

use crate::recording_kind::RecordingKind;
use crate::study::parse::{self, StudyKind};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::sqlite::SqliteConnection;
use sqlx::{Pool, Row, Sqlite};
use std::collections::{HashMap, HashSet};

/// Items shown per recording (notes rarely list more; a runaway list is cut).
const MAX_ITEMS_PER_RECORDING: usize = 40;

// ═══════════════════════════════════════════════════════════════════════════
// Schema (runs inside DatabaseManager::run_migrations on its one connection)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn ensure_schema(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS notebook_item_done (
            meeting_id TEXT NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
            item_key TEXT NOT NULL,
            done_at TEXT NOT NULL,
            PRIMARY KEY (meeting_id, item_key)
        )
        "#,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Items in the notes (pure)
// ═══════════════════════════════════════════════════════════════════════════

/// One thing to do, as the notes say it.
#[derive(Debug, Clone, PartialEq)]
pub struct NoteItem {
    pub task: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The checkbox key for an item: a hash of its text with case, spacing and
/// trailing punctuation ignored, so the same item keeps its tick when the
/// notes are made again. The text itself is never stored with the tick.
pub fn item_key(task: &str) -> String {
    let norm = one_line(task).to_lowercase();
    let norm = norm.trim_end_matches(|c: char| matches!(c, '.' | '!' | ';' | ',' | ':'));
    let mut h = Sha256::new();
    h.update(norm.as_bytes());
    h.finalize().iter().take(8).map(|b| format!("{:02x}", b)).collect()
}

/// "None recorded." and friends: what a notes list says when it is empty.
fn is_placeholder(task: &str) -> bool {
    let t = task.trim().trim_end_matches('.').to_lowercase();
    matches!(t.as_str(), "none" | "none recorded" | "none mentioned" | "none stated" | "n/a" | "nothing")
}

/// Items from the Mac's structured notes (`meeting_notes.action_items`).
pub fn items_from_json(action_items: Option<&str>) -> Vec<NoteItem> {
    let Some(list) = action_items.and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok()) else {
        return Vec::new();
    };
    let field = |v: &serde_json::Value, key: &str| {
        v.get(key).and_then(|x| x.as_str()).map(one_line).filter(|s| !s.is_empty() && s.to_lowercase() != "null")
    };
    list.iter()
        .filter_map(|v| {
            let task = match v {
                serde_json::Value::String(s) => one_line(s),
                other => field(other, "task").or_else(|| field(other, "text"))?,
            };
            (!task.is_empty() && !is_placeholder(&task)).then(|| NoteItem {
                task,
                assignee: field(v, "assignee"),
                due: field(v, "due_date"),
            })
        })
        .take(MAX_ITEMS_PER_RECORDING)
        .collect()
}

/// The headings a notes document puts its items under, by type.
const ITEM_HEADINGS: &[&str] = &["action items", "announcements and deadlines", "to-dos and reminders"];

/// Items from notes kept as Markdown (made on the iPhone and synced): the
/// bullets under "Action items", "Announcements and deadlines" or "To-dos
/// and reminders", written as `## Heading` or `**Heading**`.
pub fn items_from_markdown(md: &str) -> Vec<NoteItem> {
    let mut out = Vec::new();
    let mut inside = false;
    for raw in md.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let heading = line
            .strip_prefix('#')
            .map(|r| r.trim_start_matches('#').trim())
            .or_else(|| line.strip_prefix("**").and_then(|r| r.trim_end_matches(':').strip_suffix("**")).map(str::trim));
        if let Some(h) = heading {
            inside = ITEM_HEADINGS.contains(&h.trim_end_matches(':').to_lowercase().as_str());
            continue;
        }
        if !inside {
            continue;
        }
        let bullet = ['•', '-', '*'].iter().find_map(|m| line.strip_prefix(*m)).map(str::trim);
        let Some(text) = bullet else { continue };
        let task = one_line(&text.replace("**", ""));
        if !task.is_empty() && !is_placeholder(&task) {
            out.push(NoteItem { task, assignee: None, due: None });
        }
        if out.len() >= MAX_ITEMS_PER_RECORDING {
            break;
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// The page
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotebookRecording {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub duration_seconds: Option<i64>,
    pub recording_kind: String,
    pub has_notes: bool,
    pub has_guide: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotebookItem {
    pub meeting_id: String,
    pub meeting_title: String,
    pub started_at: String,
    /// Checkbox key ([`item_key`])
    pub key: String,
    pub task: String,
    pub assignee: Option<String>,
    pub due: Option<String>,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotebookMark {
    pub id: String,
    pub meeting_id: String,
    pub meeting_title: String,
    pub started_at: String,
    /// "test" | "question"
    pub kind: String,
    /// The kind's label in that recording's type ("On the test", "Follow up"…)
    pub label: String,
    pub note: Option<String>,
    pub offset_ms: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NotebookOverview {
    /// The notebook's name as its newest recording spells it
    pub name: String,
    /// The type of its newest recording: names the items section
    pub recording_kind: String,
    pub total_seconds: i64,
    /// Newest first
    pub recordings: Vec<NotebookRecording>,
    /// Newest recording first, in the order its notes list them
    pub items: Vec<NotebookItem>,
    /// Newest recording first, in time order inside a recording
    pub marked: Vec<NotebookMark>,
    /// What "Review everything" holds
    pub cards: usize,
    pub questions: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReviewCard {
    pub front: String,
    pub back: String,
    pub meeting_id: String,
    pub meeting_title: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReviewQuestion {
    pub question: String,
    pub choices: Vec<String>,
    pub answer: usize,
    pub explanation: Option<String>,
    pub at_ms: Option<i64>,
    pub meeting_id: String,
    pub meeting_title: String,
}

/// A recording the review draws on.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReviewRecording {
    pub id: String,
    pub title: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct NotebookReview {
    /// The recordings with a card or a question, oldest first (the review
    /// can start from any of them: "since the midterm")
    pub recordings: Vec<ReviewRecording>,
    pub cards: Vec<ReviewCard>,
    pub questions: Vec<ReviewQuestion>,
}

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(", ")
}

struct Rec {
    id: String,
    title: String,
    started_at: String,
    duration_seconds: Option<i64>,
    kind: RecordingKind,
    notebook: String,
}

async fn recordings_in(conn: &mut SqliteConnection, notebook: &str) -> Result<Vec<Rec>, String> {
    let name = crate::notebooks::normalize(notebook).ok_or("That notebook has no name.")?;
    let rows = sqlx::query(
        "SELECT id, title, started_at, duration_seconds, recording_kind, class_name FROM meetings \
         WHERE class_name = ? COLLATE NOCASE ORDER BY started_at DESC",
    )
    .bind(&name)
    .fetch_all(&mut *conn)
    .await
    .map_err(err("Couldn't read the notebook"))?;
    Ok(rows
        .iter()
        .map(|r| Rec {
            id: r.get("id"),
            title: r.get("title"),
            started_at: r.get("started_at"),
            duration_seconds: r.get("duration_seconds"),
            kind: RecordingKind::from_stored(r.get::<Option<String>, _>("recording_kind").as_deref()),
            notebook: r.get::<Option<String>, _>("class_name").unwrap_or_default(),
        })
        .collect())
}

/// Each recording's stored guide parts of `kind`, re-validated.
async fn guide_parts(
    conn: &mut SqliteConnection,
    recs: &[Rec],
    kind: StudyKind,
) -> Result<HashMap<String, serde_json::Value>, String> {
    if recs.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT meeting_id, json FROM study_materials WHERE kind = ? AND json IS NOT NULL AND meeting_id IN ({}) \
         ORDER BY created_at ASC",
        placeholders(recs.len())
    );
    let mut q = sqlx::query(&sql).bind(kind.as_str());
    for r in recs {
        q = q.bind(&r.id);
    }
    let rows = q.fetch_all(&mut *conn).await.map_err(err("Couldn't read the guides"))?;
    let duration: HashMap<&str, i64> =
        recs.iter().map(|r| (r.id.as_str(), r.duration_seconds.unwrap_or(0).max(0) * 1000)).collect();
    let mut out = HashMap::new();
    for r in rows {
        let id: String = r.get("meeting_id");
        // A recording with no stored length: times can't be bounded, so allow a day
        let d = duration.get(id.as_str()).copied().filter(|d| *d > 0).unwrap_or(86_400_000);
        if let Some(v) = parse::revalidate(kind, &r.get::<String, _>("json"), d) {
            out.insert(id, v);
        }
    }
    Ok(out)
}

pub async fn overview(pool: &Pool<Sqlite>, notebook: &str) -> Result<NotebookOverview, String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    let recs = recordings_in(&mut conn, notebook).await?;
    let Some(newest) = recs.first() else {
        return Err("No recordings in that notebook.".into());
    };
    let ids: Vec<&str> = recs.iter().map(|r| r.id.as_str()).collect();

    // The newest notes of each recording
    let sql = format!(
        "SELECT meeting_id, summary, action_items, model_used FROM meeting_notes WHERE meeting_id IN ({}) \
         ORDER BY generated_at DESC, rowid DESC",
        placeholders(ids.len())
    );
    let mut q = sqlx::query(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    let mut notes: HashMap<String, Vec<NoteItem>> = HashMap::new();
    for r in q.fetch_all(&mut *conn).await.map_err(err("Couldn't read the notes"))? {
        let id: String = r.get("meeting_id");
        if notes.contains_key(&id) {
            continue; // an older run of the same recording's notes
        }
        let model: Option<String> = r.get("model_used");
        let items = if model.as_deref() == Some(crate::sync::store::SYNCED_NOTES_MODEL) {
            items_from_markdown(&r.get::<Option<String>, _>("summary").unwrap_or_default())
        } else {
            items_from_json(r.get::<Option<String>, _>("action_items").as_deref())
        };
        notes.insert(id, items);
    }

    // Ticks; ones whose item is gone (the notes were made again) are dropped
    let sql = format!(
        "SELECT meeting_id, item_key FROM notebook_item_done WHERE meeting_id IN ({})",
        placeholders(ids.len())
    );
    let mut q = sqlx::query(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    let done: HashSet<(String, String)> = q
        .fetch_all(&mut *conn)
        .await
        .map_err(err("Couldn't read the notebook"))?
        .iter()
        .map(|r| (r.get("meeting_id"), r.get("item_key")))
        .collect();

    let mut items = Vec::new();
    let mut live: HashSet<(String, String)> = HashSet::new();
    for rec in &recs {
        let mut seen = HashSet::new();
        for it in notes.get(&rec.id).map(Vec::as_slice).unwrap_or_default() {
            let key = item_key(&it.task);
            if !seen.insert(key.clone()) {
                continue; // the notes said it twice
            }
            let k = (rec.id.clone(), key.clone());
            items.push(NotebookItem {
                meeting_id: rec.id.clone(),
                meeting_title: rec.title.clone(),
                started_at: rec.started_at.clone(),
                done: done.contains(&k),
                key,
                task: it.task.clone(),
                assignee: it.assignee.clone(),
                due: it.due.clone(),
            });
            live.insert(k);
        }
    }
    for (meeting_id, key) in done.difference(&live) {
        // Only for a recording that has notes now: a tick outlives a Make again
        // that is still running, never one whose item the new notes dropped
        if notes.contains_key(meeting_id) {
            let _ = sqlx::query("DELETE FROM notebook_item_done WHERE meeting_id = ? AND item_key = ?")
                .bind(meeting_id)
                .bind(key)
                .execute(&mut *conn)
                .await;
        }
    }

    // ✎ and ? marks
    let sql = format!(
        "SELECT id, meeting_id, ts, kind, note FROM meeting_markers WHERE kind IN ('test', 'question') \
         AND meeting_id IN ({}) ORDER BY ts ASC, created_at ASC",
        placeholders(ids.len())
    );
    let mut q = sqlx::query(&sql);
    for id in &ids {
        q = q.bind(*id);
    }
    let mut by_rec: HashMap<String, Vec<NotebookMark>> = HashMap::new();
    let rec_of: HashMap<&str, &Rec> = recs.iter().map(|r| (r.id.as_str(), r)).collect();
    for r in q.fetch_all(&mut *conn).await.map_err(err("Couldn't read the marks"))? {
        let meeting_id: String = r.get("meeting_id");
        let Some(rec) = rec_of.get(meeting_id.as_str()) else { continue };
        let kind: String = r.get("kind");
        let offset_ms = match (parse_ts(&r.get::<String, _>("ts")), parse_ts(&rec.started_at)) {
            (Some(t), Some(s)) => (t - s).num_milliseconds().max(0),
            _ => 0,
        };
        by_rec.entry(meeting_id.clone()).or_default().push(NotebookMark {
            id: r.get("id"),
            meeting_id,
            meeting_title: rec.title.clone(),
            started_at: rec.started_at.clone(),
            label: crate::markers::label(&kind, rec.kind).to_string(),
            kind,
            note: r.get::<Option<String>, _>("note").filter(|n| !n.trim().is_empty()),
            offset_ms,
        });
    }
    let marked: Vec<NotebookMark> = recs.iter().flat_map(|r| by_rec.remove(&r.id).unwrap_or_default()).collect();

    let cards = guide_parts(&mut conn, &recs, StudyKind::Flashcards).await?;
    let quiz = guide_parts(&mut conn, &recs, StudyKind::Quiz).await?;
    let count = |m: &HashMap<String, serde_json::Value>, key: &str| -> usize {
        m.values().map(|v| v[key].as_array().map_or(0, Vec::len)).sum()
    };
    let with_guide: HashSet<&String> = cards.keys().chain(quiz.keys()).collect();

    Ok(NotebookOverview {
        name: newest.notebook.clone(),
        recording_kind: newest.kind.as_str().to_string(),
        total_seconds: recs.iter().filter_map(|r| r.duration_seconds).filter(|d| *d > 0).sum(),
        recordings: recs
            .iter()
            .map(|r| NotebookRecording {
                id: r.id.clone(),
                title: r.title.clone(),
                started_at: r.started_at.clone(),
                duration_seconds: r.duration_seconds,
                recording_kind: r.kind.as_str().to_string(),
                has_notes: notes.contains_key(&r.id),
                has_guide: with_guide.contains(&r.id),
            })
            .collect(),
        items,
        marked,
        cards: count(&cards, "cards"),
        questions: count(&quiz, "questions"),
    })
}

/// Every stored flashcard and quiz question in the notebook, oldest
/// recording first (the order the course was taught, the project ran).
pub async fn review(pool: &Pool<Sqlite>, notebook: &str) -> Result<NotebookReview, String> {
    let mut conn = pool.acquire().await.map_err(err("Database busy"))?;
    let mut recs = recordings_in(&mut conn, notebook).await?;
    recs.reverse();
    let cards = guide_parts(&mut conn, &recs, StudyKind::Flashcards).await?;
    let quiz = guide_parts(&mut conn, &recs, StudyKind::Quiz).await?;
    let text = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let mut out = NotebookReview::default();
    let mut fronts = HashSet::new();
    for rec in &recs {
        let before = (out.cards.len(), out.questions.len());
        for c in cards.get(&rec.id).and_then(|v| v["cards"].as_array()).map(Vec::as_slice).unwrap_or_default() {
            let (Some(front), Some(back)) = (text(&c["front"]), text(&c["back"])) else { continue };
            // The same card from two lectures is asked once
            if fronts.insert(parse::norm_key(&front)) {
                out.cards.push(ReviewCard { front, back, meeting_id: rec.id.clone(), meeting_title: rec.title.clone() });
            }
        }
        for q in quiz.get(&rec.id).and_then(|v| v["questions"].as_array()).map(Vec::as_slice).unwrap_or_default() {
            let Some(question) = text(&q["question"]) else { continue };
            let choices: Vec<String> = q["choices"].as_array().map(|a| a.iter().filter_map(text).collect()).unwrap_or_default();
            let Some(answer) = q["answer"].as_u64().map(|n| n as usize).filter(|n| *n < choices.len()) else { continue };
            out.questions.push(ReviewQuestion {
                question,
                choices,
                answer,
                explanation: text(&q["explanation"]),
                at_ms: q["at_ms"].as_i64(),
                meeting_id: rec.id.clone(),
                meeting_title: rec.title.clone(),
            });
        }
        if (out.cards.len(), out.questions.len()) != before {
            out.recordings.push(ReviewRecording {
                id: rec.id.clone(),
                title: rec.title.clone(),
                started_at: rec.started_at.clone(),
            });
        }
    }
    Ok(out)
}

/// One recording's items with their ticks, in the order its newest notes
/// list them: the Notes view shows the same checkbox as the notebook page.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecordingItem {
    pub key: String,
    pub task: String,
    pub done: bool,
}

pub async fn recording_items(pool: &Pool<Sqlite>, meeting_id: &str) -> Result<Vec<RecordingItem>, String> {
    let row = sqlx::query(
        "SELECT summary, action_items, model_used FROM meeting_notes WHERE meeting_id = ? \
         ORDER BY generated_at DESC, rowid DESC LIMIT 1",
    )
    .bind(meeting_id)
    .fetch_optional(pool)
    .await
    .map_err(err("Couldn't read the notes"))?;
    let Some(r) = row else { return Ok(Vec::new()) };
    let model: Option<String> = r.get("model_used");
    let items = if model.as_deref() == Some(crate::sync::store::SYNCED_NOTES_MODEL) {
        items_from_markdown(&r.get::<Option<String>, _>("summary").unwrap_or_default())
    } else {
        items_from_json(r.get::<Option<String>, _>("action_items").as_deref())
    };
    let done: HashSet<String> = sqlx::query_scalar("SELECT item_key FROM notebook_item_done WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_all(pool)
        .await
        .map_err(err("Couldn't read the notes"))?
        .into_iter()
        .collect();
    let mut seen = HashSet::new();
    Ok(items
        .into_iter()
        .filter_map(|it| {
            let key = item_key(&it.task);
            seen.insert(key.clone()).then(|| RecordingItem { done: done.contains(&key), key, task: it.task })
        })
        .collect())
}

/// Tick or untick an item. Only the key is kept.
pub async fn set_done(pool: &Pool<Sqlite>, meeting_id: &str, key: &str, done: bool) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() || key.len() > 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("That item can't be ticked.".into());
    }
    if done {
        sqlx::query("INSERT OR REPLACE INTO notebook_item_done (meeting_id, item_key, done_at) VALUES (?, ?, ?)")
            .bind(meeting_id)
            .bind(key)
            .bind(Utc::now().to_rfc3339())
            .execute(pool)
            .await
            .map_err(err("Couldn't save the tick"))?;
    } else {
        sqlx::query("DELETE FROM notebook_item_done WHERE meeting_id = ? AND item_key = ?")
            .bind(meeting_id)
            .bind(key)
            .execute(pool)
            .await
            .map_err(err("Couldn't save the tick"))?;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Tauri commands
// ═══════════════════════════════════════════════════════════════════════════

pub mod commands {
    use super::*;
    use crate::AppState;
    use tauri::State;

    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_notebook_overview(state: State<'_, AppState>, notebook: String) -> Result<NotebookOverview, String> {
        overview(state.database.pool(), &notebook).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_notebook_review(state: State<'_, AppState>, notebook: String) -> Result<NotebookReview, String> {
        review(state.database.pool(), &notebook).await
    }

    /// The items in one recording's notes, with their ticks (Notes view).
    #[tauri::command(rename_all = "camelCase")]
    pub async fn get_recording_items(state: State<'_, AppState>, meeting_id: String) -> Result<Vec<RecordingItem>, String> {
        recording_items(state.database.pool(), &meeting_id).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn set_notebook_item_done(
        state: State<'_, AppState>,
        meeting_id: String,
        key: String,
        done: bool,
    ) -> Result<(), String> {
        set_done(state.database.pool(), &meeting_id, &key, done).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;
    use serde_json::json;

    #[test]
    fn item_keys_ignore_case_spacing_and_trailing_punctuation() {
        assert_eq!(item_key("Read chapter 7."), item_key("  read   CHAPTER 7 "));
        assert_ne!(item_key("Read chapter 7"), item_key("Read chapter 8"));
        assert_eq!(item_key("x").len(), 16);
        assert!(item_key("Read chapter 7").chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn items_come_from_structured_notes_and_from_markdown() {
        let json = r#"[{"task": " Quiz on  membranes", "assignee": null, "due_date": "Next Wednesday", "priority": "high"},
            {"task": "Send the agreement", "assignee": "Alex", "due_date": "null"}, "Book the room",
            {"task": "None recorded."}, {"assignee": "nobody"}, 7]"#;
        assert_eq!(
            items_from_json(Some(json)),
            vec![
                NoteItem { task: "Quiz on membranes".into(), assignee: None, due: Some("Next Wednesday".into()) },
                NoteItem { task: "Send the agreement".into(), assignee: Some("Alex".into()), due: None },
                NoteItem { task: "Book the room".into(), assignee: None, due: None },
            ]
        );
        assert!(items_from_json(None).is_empty());
        assert!(items_from_json(Some("not json")).is_empty());
        assert!(items_from_json(Some("{}")).is_empty());

        // The iPhone's notes, all three styles and both heading forms
        let class = "## Summary\nMembranes.\n## Key concepts\n- Bilayer\n## Announcements and deadlines\n- Read chapter 7 by **Friday**\n- Quiz Wednesday\n";
        assert_eq!(
            items_from_markdown(class).iter().map(|i| i.task.as_str()).collect::<Vec<_>>(),
            vec!["Read chapter 7 by Friday", "Quiz Wednesday"]
        );
        let meeting = "**Summary**\nWe agreed.\n\n**Decisions**\n• Ship Oct 14\n\n**Action items**\n• Alex: Send the agreement · Friday\n";
        assert_eq!(items_from_markdown(meeting).iter().map(|i| i.task.as_str()).collect::<Vec<_>>(), vec!["Alex: Send the agreement · Friday"]);
        let personal = "## Summary\nDoctor.\n## To-dos and reminders\n- None mentioned.\n";
        assert!(items_from_markdown(personal).is_empty());
        // Bullets under other headings are not items; a later heading ends the list
        let mixed = "## Action items\n- [owner not stated] Draft the plan\n## Follow-up\n- not an item\n";
        assert_eq!(items_from_markdown(mixed).len(), 1);
        assert!(items_from_markdown("").is_empty());
    }

    struct Fx {
        db: DatabaseManager,
        dir: std::path::PathBuf,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Three recordings: two lectures in "BIO 101" (the newer one spelled
    /// "bio 101") and a meeting in another notebook.
    async fn fx() -> Fx {
        let dir = std::env::temp_dir().join(format!("nf-notebook-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = DatabaseManager::new(&dir.join("t.db")).await.unwrap();
        db.run_migrations().await.unwrap();
        for (id, title, started, notebook, kind) in [
            ("old", "Cell biology", "2026-11-04T09:00:00+00:00", "BIO 101", "class"),
            ("new", "Membrane transport", "2026-11-11T09:00:00+00:00", "bio 101", "class"),
            ("other", "Acme sync", "2026-11-10T15:00:00+00:00", "Acme project", "meeting"),
        ] {
            db.create_meeting(id, title).await.unwrap();
            sqlx::query("UPDATE meetings SET started_at = ?, duration_seconds = 3000, class_name = ?, recording_kind = ? WHERE id = ?")
                .bind(started)
                .bind(notebook)
                .bind(kind)
                .bind(id)
                .execute(db.pool())
                .await
                .unwrap();
        }
        Fx { db, dir }
    }

    async fn guide(f: &Fx, meeting: &str, kind: &str, value: serde_json::Value) {
        sqlx::query("INSERT INTO study_materials (id, meeting_id, kind, json, created_at, generated_at) VALUES (?, ?, ?, ?, ?, datetime('now'))")
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(meeting)
            .bind(kind)
            .bind(value.to_string())
            .bind(Utc::now().to_rfc3339())
            .execute(f.db.pool())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn the_page_gathers_items_marks_and_guides_across_the_notebook() {
        let f = fx().await;
        let pool = f.db.pool();
        f.db.save_meeting_notes("n-old", "old", Some("Cells."), None, None, Some(r#"[{"task": "Buy the lab manual", "due_date": "Monday"}]"#), None, Some("lecture-notes")).await.unwrap();
        // Made on the iPhone and synced: Markdown
        f.db.save_meeting_notes("n-new", "new", Some("## Summary\nTransport.\n## Announcements and deadlines\n- Quiz on membranes Wednesday\n- Read chapter 7\n- read chapter 7.\n"), None, None, None, None, Some(crate::sync::store::SYNCED_NOTES_MODEL)).await.unwrap();
        f.db.save_meeting_notes("n-other", "other", Some("Sync."), None, None, Some(r#"[{"task": "Send the agreement", "assignee": "Alex"}]"#), None, Some("default")).await.unwrap();

        let start = |s: &str| parse_ts(s).unwrap();
        let t_old = start("2026-11-04T09:00:00+00:00");
        let t_new = start("2026-11-11T09:00:00+00:00");
        crate::markers::add(pool, "old", t_old + chrono::Duration::seconds(600), Some("test"), Some("phases of mitosis"), false).await.unwrap();
        crate::markers::add(pool, "new", t_new + chrono::Duration::seconds(120), Some("question"), None, false).await.unwrap();
        crate::markers::add(pool, "new", t_new + chrono::Duration::seconds(60), Some("test"), Some("the pump"), false).await.unwrap();
        // Not on the page: ★ marks, typed notes, another notebook's marks
        crate::markers::add(pool, "new", t_new + chrono::Duration::seconds(30), Some("important"), None, false).await.unwrap();
        crate::markers::add(pool, "new", t_new + chrono::Duration::seconds(40), Some("note"), Some("my own line"), false).await.unwrap();
        crate::markers::add(pool, "other", Utc::now(), Some("test"), Some("budget"), false).await.unwrap();

        guide(&f, "old", "flashcards", json!({"cards": [{"front": "What is a cell?", "back": "The unit of life"}, {"front": "ATP?", "back": "Energy"}]})).await;
        guide(&f, "new", "flashcards", json!({"cards": [{"front": "atp?", "back": "Energy currency"}, {"front": "Na/K pump?", "back": "3 out, 2 in"}]})).await;
        guide(&f, "new", "quiz", json!({"questions": [{"question": "What moves sodium out?", "choices": ["The pump", "Osmosis"], "answer": 0, "explanation": "Active transport.", "at_ms": 60000}]})).await;
        guide(&f, "other", "flashcards", json!({"cards": [{"front": "Budget?", "back": "Friday"}]})).await;

        let page = overview(pool, "Bio 101").await.unwrap();
        assert_eq!(page.name, "bio 101", "spelled like the newest recording");
        assert_eq!(page.recording_kind, "class");
        assert_eq!(page.total_seconds, 6000);
        assert_eq!(page.recordings.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["new", "old"]);
        assert!(page.recordings.iter().all(|r| r.has_notes && r.has_guide));
        // Items: newest recording first, the repeated one once
        assert_eq!(
            page.items.iter().map(|i| (i.meeting_id.as_str(), i.task.as_str(), i.due.as_deref())).collect::<Vec<_>>(),
            vec![("new", "Quiz on membranes Wednesday", None), ("new", "Read chapter 7", None), ("old", "Buy the lab manual", Some("Monday"))]
        );
        assert!(page.items.iter().all(|i| !i.done));
        // Marked: ✎ and ? only, newest recording first, in time order
        assert_eq!(
            page.marked.iter().map(|m| (m.meeting_id.as_str(), m.kind.as_str(), m.label.as_str(), m.note.as_deref(), m.offset_ms)).collect::<Vec<_>>(),
            vec![
                ("new", "test", "On the test", Some("the pump"), 60_000),
                ("new", "question", "Question", None, 120_000),
                ("old", "test", "On the test", Some("phases of mitosis"), 600_000),
            ]
        );
        assert_eq!((page.cards, page.questions), (4, 1));

        // Review everything: oldest lecture first, the same card once
        let r = review(pool, "BIO 101").await.unwrap();
        assert_eq!(r.cards.iter().map(|c| c.front.as_str()).collect::<Vec<_>>(), vec!["What is a cell?", "ATP?", "Na/K pump?"]);
        assert_eq!(r.cards[0].meeting_title, "Cell biology");
        assert_eq!(r.recordings.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["old", "new"]);
        assert_eq!(r.questions.len(), 1);
        assert_eq!((r.questions[0].answer, r.questions[0].at_ms, r.questions[0].meeting_id.as_str()), (0, Some(60_000), "new"));

        // Another notebook sees only its own
        let acme = overview(pool, "acme project").await.unwrap();
        assert_eq!(acme.recording_kind, "meeting");
        assert_eq!(acme.items.len(), 1);
        assert_eq!(acme.items[0].assignee.as_deref(), Some("Alex"));
        assert_eq!(acme.marked[0].label, "Follow up");
        assert_eq!((acme.cards, acme.questions), (1, 0));
        // No such notebook
        assert!(overview(pool, "CHEM 201").await.is_err());
        assert!(overview(pool, "   ").await.is_err());
    }

    #[tokio::test]
    async fn a_tick_is_kept_as_a_hash_survives_make_again_and_goes_with_its_item_or_recording() {
        let f = fx().await;
        let pool = f.db.pool();
        let notes = r#"[{"task": "Read chapter 7"}, {"task": "Quiz Wednesday"}]"#;
        f.db.save_meeting_notes("n1", "new", Some("x"), None, None, Some(notes), None, Some("lecture-notes")).await.unwrap();
        let key = item_key("Read chapter 7");
        set_done(pool, "new", &key, true).await.unwrap();
        let page = overview(pool, "BIO 101").await.unwrap();
        assert_eq!(page.items.iter().map(|i| i.done).collect::<Vec<_>>(), vec![true, false]);
        // Only the key is stored
        let stored: Vec<(String, String)> = sqlx::query_as("SELECT meeting_id, item_key FROM notebook_item_done").fetch_all(pool).await.unwrap();
        assert_eq!(stored, vec![("new".to_string(), key.clone())]);

        // Made again with the same item worded the same: still ticked. The dropped item's tick goes.
        set_done(pool, "new", &item_key("Quiz Wednesday"), true).await.unwrap();
        f.db.save_meeting_notes("n2", "new", Some("x"), None, None, Some(r#"[{"task": "read chapter 7."}, {"task": "Bring a calculator"}]"#), None, Some("lecture-notes")).await.unwrap();
        let page = overview(pool, "BIO 101").await.unwrap();
        assert_eq!(page.items.iter().map(|i| (i.task.as_str(), i.done)).collect::<Vec<_>>(), vec![("read chapter 7.", true), ("Bring a calculator", false)]);
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notebook_item_done").fetch_one(pool).await.unwrap();
        assert_eq!(left, 1);

        // The recording's own Notes view reads the same ticks, in the notes' order
        assert_eq!(
            recording_items(pool, "new").await.unwrap(),
            vec![
                RecordingItem { key: key.clone(), task: "read chapter 7.".into(), done: true },
                RecordingItem { key: item_key("Bring a calculator"), task: "Bring a calculator".into(), done: false },
            ]
        );
        assert!(recording_items(pool, "old").await.unwrap().is_empty(), "no notes, no items");

        // Untick; a bad key is refused
        set_done(pool, "new", &key, false).await.unwrap();
        assert!(!overview(pool, "BIO 101").await.unwrap().items[0].done);
        assert!(set_done(pool, "new", "Read chapter 7", true).await.is_err(), "text is never accepted as a key");
        assert!(set_done(pool, "new", "", true).await.is_err());

        // Deleting the recording takes its ticks
        set_done(pool, "new", &key, true).await.unwrap();
        f.db.delete_meeting("new").await.unwrap();
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notebook_item_done").fetch_one(pool).await.unwrap();
        assert_eq!(left, 0);
    }
}
