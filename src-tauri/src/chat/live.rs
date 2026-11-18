//! Ask: a question about the recording you are in, answered in the moment
//! (docs/TOPICS_AND_CHAT.md "Ask"; owner decision 2026-11-17).
//!
//! The Chat tab searches recordings; this answers about one, the way
//! someone sitting in the room would: it knows what was just said, who is
//! there, and what you wrote down. It runs only when the user asks (never
//! by itself, also while the recording is still running), and everything it
//! is given is put together here, on the Mac, for that one question:
//!
//! - **The recording**: title, type, Notebook, when it started and how far
//!   in it is.
//! - **The people**: the calendar event's attendees with company and role,
//!   which of them is the user, and the event's title, place and notes.
//! - **Which side spoke**: lines are tagged (mic) or (computer audio) when
//!   that could be told (audio_source.rs). Never a name: the transcript
//!   doesn't know voices, and the prompt says so.
//! - **Your notes and marks** in this recording, with their times.
//! - **The last minutes**, in order, newest last. Always given: "what did
//!   they just say?" has no words to search for.
//! - **Earlier in this recording**: lines matched to the question (or, when
//!   it has no words to match, lines spread over what came before), and
//!   the summary of the notes already made from it.
//! - **Earlier recordings** in the same Notebook or with the same people:
//!   their saved notes, and lines matched to the question.
//!
//! Every numbered source can be cited and jumped to. The answer is stored
//! like any chat answer, with this recording (and any other it drew on) as
//! its sources, so deleting or striking any of them removes the answer
//! (`chat::purge_for_meeting`). Stricken text is never in a source. Nothing
//! here is logged.

use super::retrieval::{self, clip, Filter, Passage, Scope};
use super::{prompt, Answer};
use crate::ai::{AiError, Msg, Opts};
use crate::study::prompt::clock;
use crate::study::Completer;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::sqlite::SqliteConnection;
use sqlx::Row;

/// The last minutes that are always given, counted back from the newest line
const RECENT_MS: i64 = 6 * 60 * 1000;
/// Lines of one side are joined up to this many characters per source
const CHUNK_CHARS: usize = 420;
const MAX_PEOPLE: usize = 14;
const MAX_NOTES: usize = 16;
/// Earlier recordings whose saved notes are offered
const MAX_RELATED: usize = 3;
/// An answer in the moment is short
const MAX_ANSWER_TOKENS: u32 = 700;
/// An answer on the card over another app is shorter still
const GLANCE_ANSWER_TOKENS: u32 = 320;
/// Added to the system prompt for that card
const GLANCE_RULE: &str = "\n- This answer is shown on a small card over the user's call, read at a glance: at most three short \
lines or bullets, the most useful thing first, no introduction.";
/// Lines given from before the last minutes
const SAMPLED_LINES: usize = 6;

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

/// How a question is being asked.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    /// The recording is running right now (the capture engine says so; a
    /// recording with no end time is not proof of it)
    pub recording: bool,
    /// Earlier recordings may be drawn on: Ask does, and says so in "What
    /// it's given"; a Chat narrowed to one recording stays in it
    pub related: bool,
    /// The answer is read at a glance, on the small card over another app
    /// (quick_note.rs): a few lines, no lists of sources
    pub glance: bool,
}

/// The recording being asked about.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub id: String,
    pub title: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    /// "meeting" | "class" | "personal"
    pub kind: String,
    pub notebook: Option<String>,
}

impl Head {
    fn passage(&self, source: &'static str, timestamp_ms: Option<i64>, excerpt: String, score: f64) -> Passage {
        Passage {
            n: 0,
            meeting_id: self.id.clone(),
            title: self.title.clone(),
            started_at: self.started_at.clone(),
            kind: self.kind.clone(),
            notebook: self.notebook.clone(),
            source,
            timestamp_ms,
            excerpt,
            score,
        }
    }

    fn offset_ms(&self, at: &str) -> Option<i64> {
        Some((parse_ts(at)? - parse_ts(&self.started_at)?).num_milliseconds().max(0))
    }
}

pub async fn head(conn: &mut SqliteConnection, meeting_id: &str) -> Result<Head, String> {
    let r = sqlx::query("SELECT id, title, started_at, ended_at, recording_kind, class_name FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(err("Database busy"))?
        .ok_or("That recording no longer exists")?;
    Ok(Head {
        id: r.get("id"),
        title: r.get("title"),
        started_at: r.get("started_at"),
        ended_at: r.get("ended_at"),
        kind: crate::recording_kind::RecordingKind::from_stored(r.get::<Option<String>, _>("recording_kind").as_deref())
            .as_str()
            .to_string(),
        notebook: r.get::<Option<String>, _>("class_name").filter(|n| !n.trim().is_empty()),
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// The people and the event
// ═══════════════════════════════════════════════════════════════════════════

/// Someone in the recording's calendar event.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Who {
    pub name: String,
    /// The company, or the email's domain when no company is known
    pub company: Option<String>,
    /// "organizer" when they called it
    pub role: Option<String>,
    /// This is the user
    pub you: bool,
    /// How many recordings they are in
    #[serde(skip)]
    pub met: i64,
}

async fn has_table(conn: &mut SqliteConnection, name: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?")
        .bind(name)
        .fetch_one(&mut *conn)
        .await
        .map(|n| n > 0)
        .unwrap_or(false)
}

pub async fn people(conn: &mut SqliteConnection, meeting_id: &str) -> Result<Vec<Who>, String> {
    if !has_table(conn, "people").await || !has_table(conn, "meeting_attendees").await {
        return Ok(Vec::new());
    }
    let rows = sqlx::query(
        "SELECT a.name AS listed, a.email, a.company AS listed_company, a.role, p.name, p.company, p.company_domain, \
                COALESCE(p.is_self, 0) AS is_self, \
                (SELECT COUNT(DISTINCT x.meeting_id) FROM meeting_attendees x WHERE lower(x.email) = lower(a.email)) AS met \
         FROM meeting_attendees a LEFT JOIN people p ON p.id = lower(a.email) \
         WHERE a.meeting_id = ? \
         ORDER BY COALESCE(p.is_self, 0) DESC, (a.role = 'organizer') DESC, COALESCE(p.name, a.name) COLLATE NOCASE LIMIT ?",
    )
    .bind(meeting_id)
    .bind(MAX_PEOPLE as i64)
    .fetch_all(&mut *conn)
    .await
    .map_err(err("Couldn't read the people"))?;
    let text = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Ok(rows
        .iter()
        .map(|r| {
            let email: String = r.get("email");
            let name = text(r.get("name")).or_else(|| text(r.get("listed"))).unwrap_or_else(|| email.clone());
            // A personal mail domain says nothing about a company
            let domain = text(r.get("company_domain")).filter(|d| !crate::attendee_intel::is_personal_domain(d));
            Who {
                name,
                company: text(r.get("company")).or_else(|| text(r.get("listed_company"))).or(domain),
                role: text(r.get("role")).filter(|role| role == "organizer"),
                you: r.get::<i64, _>("is_self") != 0,
                met: r.get("met"),
            }
        })
        .collect())
}

/// The calendar event the recording is linked to: (title, place, notes).
async fn event(conn: &mut SqliteConnection, meeting_id: &str) -> Option<(Option<String>, Option<String>, Option<String>)> {
    if !has_table(conn, "meeting_details").await {
        return None;
    }
    let r = sqlx::query("SELECT event_title, location, notes FROM meeting_details WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_optional(&mut *conn)
        .await
        .ok()??;
    let text = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    Some((text(r.get("event_title")), text(r.get("location")), text(r.get("notes"))))
}

fn kind_word(kind: &str) -> &'static str {
    match kind {
        "class" => "class",
        "personal" => "personal recording",
        _ => "meeting",
    }
}

fn local_time(rfc3339: &str) -> String {
    parse_ts(rfc3339).map(|d| d.with_timezone(&chrono::Local).format("%a %-d %b %Y, %-I:%M %p").to_string()).unwrap_or_default()
}

/// On a call the event says is between the user and one other person,
/// the computer's audio is most likely that person. Only then, and only
/// when lines are tagged with a side, may an answer name them for the
/// other side, marked "(likely)": it is worked out from the calendar, not
/// heard (`askLogic.ts::otherSide` says the same thing in the panel).
pub fn other_side(who: &[Who], tagged: bool) -> Option<&Who> {
    let mut others = who.iter().filter(|w| !w.you);
    match (tagged && who.len() == 2, others.next(), others.next()) {
        // A name, not the address that stands in when the invite has none
        (true, Some(other), None) if !other.name.trim().is_empty() && !other.name.contains('@') => Some(other),
        _ => None,
    }
}

/// What the assistant is told about the recording before any source: where
/// and when this is, who is there, and how to read the side tags. It is
/// context, not something to cite.
pub fn brief_text(
    h: &Head,
    who: &[Who],
    event: Option<&(Option<String>, Option<String>, Option<String>)>,
    tagged: bool,
    now: DateTime<Utc>,
    recording: bool,
) -> String {
    let mut out = format!("RECORDING: \"{}\", a {}", h.title.trim(), kind_word(&h.kind));
    if let Some(nb) = &h.notebook {
        out.push_str(&format!(", Notebook \"{}\"", nb));
    }
    out.push_str(&format!(". Started {}.", local_time(&h.started_at)));
    let started = parse_ts(&h.started_at);
    match (recording, h.ended_at.as_deref().and_then(parse_ts), started) {
        (true, _, Some(start)) => out.push_str(&format!(
            " It is STILL BEING RECORDED: {} min in. The user is in it right now.",
            ((now - start).num_seconds().max(0)) / 60
        )),
        (false, Some(end), Some(start)) => out.push_str(&format!(" It ended after {} min.", ((end - start).num_seconds().max(0) + 30) / 60)),
        _ => {}
    }
    if let Some((title, place, notes)) = event {
        if let Some(t) = title {
            out.push_str(&format!("\nCALENDAR EVENT: {}", clip(t, 160)));
            if let Some(p) = place {
                out.push_str(&format!(" ({})", clip(p, 100)));
            }
        }
        if let Some(n) = notes {
            out.push_str(&format!("\nEVENT NOTES: {}", clip(n, 600)));
        }
    }
    if !who.is_empty() {
        out.push_str("\nPEOPLE IN IT:");
        for w in who {
            let mut line = format!("\n- {}", w.name);
            let mut facts: Vec<String> = Vec::new();
            if w.you {
                facts.push("the user".into());
            }
            if let Some(c) = &w.company {
                facts.push(c.clone());
            }
            if let Some(r) = &w.role {
                facts.push(r.clone());
            }
            if !w.you && w.met > 1 {
                facts.push(format!("in {} of the user's recordings", w.met));
            }
            if !facts.is_empty() {
                line.push_str(&format!(" ({})", facts.join(", ")));
            }
            out.push_str(&line);
        }
    }
    // A call between two people: the other side is most likely the other one
    if let Some(other) = other_side(who, tagged) {
        out.push_str(&format!(
            "\nTHE OTHER SIDE: the event has only the user and {name}, so a (computer audio) line is most likely {name}. \
This is worked out from the calendar, not from a voice. You may name them for such a line, written \"{name} (likely)\" the \
first time in an answer, unless the words show someone else is speaking.",
            name = other.name
        ));
    }
    out.push_str(if tagged {
        "\nWHO SPOKE: the transcript does not know voices. A line tagged (mic) was picked up by the user's microphone: \
the user, or someone in the room with them. A line tagged (computer audio) came out of the user's computer: the other \
people on the call, or a video being played. An untagged line could be either."
    } else {
        "\nWHO SPOKE: the transcript does not say who spoke each line."
    });
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Sources
// ═══════════════════════════════════════════════════════════════════════════

/// A line as it may be shown: stricken spans as the placeholder, and a line
/// that is nothing but the placeholder left out.
fn plain(text: &str) -> Option<String> {
    let p = crate::redaction::render_plain(text);
    let t = p.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() || t == crate::redaction::STRICKEN_PLACEHOLDER {
        return None;
    }
    Some(t)
}

/// One transcript line of the recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub ms: i64,
    pub text: String,
    /// `audio_source::MIC` / `SYSTEM`
    pub side: Option<String>,
}

/// The last minutes as sources: consecutive lines of one side joined into a
/// chunk, in order, each starting with its side tag when it has one.
pub fn recent_chunks(h: &Head, lines: &[Line]) -> Vec<Passage> {
    let mut out: Vec<Passage> = Vec::new();
    let mut open: Option<(i64, Option<String>, String)> = None;
    let close = |open: &mut Option<(i64, Option<String>, String)>, out: &mut Vec<Passage>| {
        if let Some((ms, side, text)) = open.take() {
            let excerpt = match crate::audio_source::label(side.as_deref()) {
                Some(tag) => format!("({}) {}", tag, text),
                None => text,
            };
            out.push(h.passage("transcript", Some(ms), excerpt, 0.0));
        }
    };
    for l in lines {
        let fits = open
            .as_ref()
            .is_some_and(|(_, side, text)| *side == l.side && text.chars().count() + l.text.chars().count() < CHUNK_CHARS);
        if fits {
            if let Some((_, _, text)) = open.as_mut() {
                text.push(' ');
                text.push_str(&l.text);
            }
        } else {
            close(&mut open, &mut out);
            open = Some((l.ms, l.side.clone(), clip(&l.text, CHUNK_CHARS)));
        }
    }
    close(&mut open, &mut out);
    out
}

/// The newest of `chunks` that fit `budget` characters, still in order.
fn newest_that_fit(chunks: Vec<Passage>, budget: usize) -> Vec<Passage> {
    let mut used = 0usize;
    let mut kept: Vec<Passage> = Vec::new();
    for p in chunks.into_iter().rev() {
        let cost = p.cost();
        if used + cost > budget && !kept.is_empty() {
            break;
        }
        used += cost;
        kept.push(p);
    }
    kept.reverse();
    kept
}

/// The best of `cands` (by score) that fit `budget` characters.
fn best_that_fit(mut cands: Vec<Passage>, budget: usize, max: usize, taken: &[Passage]) -> Vec<Passage> {
    cands.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut used = 0usize;
    let mut out: Vec<Passage> = Vec::new();
    for p in cands {
        if out.len() >= max {
            break;
        }
        let same = |o: &Passage| o.meeting_id == p.meeting_id && (o.excerpt == p.excerpt || (o.source == "transcript" && o.excerpt.contains(&p.excerpt)));
        if taken.iter().any(same) || out.iter().any(same) {
            continue;
        }
        if used + p.cost() > budget {
            continue;
        }
        used += p.cost();
        out.push(p);
    }
    out
}

async fn lines_since(conn: &mut SqliteConnection, h: &Head, since_ms: i64) -> Result<Vec<Line>, String> {
    let rows = sqlx::query("SELECT timestamp, text, source FROM transcripts WHERE meeting_id = ? AND is_final = 1 ORDER BY timestamp ASC, id ASC")
        .bind(&h.id)
        .fetch_all(&mut *conn)
        .await
        .map_err(err("Couldn't read the transcript"))?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let ms = h.offset_ms(&r.get::<String, _>("timestamp"))?;
            if ms < since_ms {
                return None;
            }
            Some(Line { ms, text: plain(&r.get::<String, _>("text"))?, side: r.get("source") })
        })
        .collect())
}

/// The user's typed notes and marks in this recording, newest first.
async fn own_notes(conn: &mut SqliteConnection, h: &Head, terms: &[String]) -> Result<Vec<Passage>, String> {
    let rows = sqlx::query("SELECT ts, kind, note FROM meeting_markers WHERE meeting_id = ? ORDER BY ts DESC LIMIT ?")
        .bind(&h.id)
        .bind(MAX_NOTES as i64)
        .fetch_all(&mut *conn)
        .await
        .map_err(err("Couldn't read the notes"))?;
    let rec = crate::recording_kind::RecordingKind::from_stored(Some(&h.kind));
    Ok(rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let kind: String = r.get("kind");
            let note = r.get::<Option<String>, _>("note").map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
            let label = crate::markers::label(&kind, rec);
            let text = match note {
                Some(n) => format!("{} {}: {}", crate::markers::symbol(&kind), label, n),
                None => format!("{} {} (marked, no note)", crate::markers::symbol(&kind), label),
            };
            // The user's own emphasis always matters; a match with the question more
            let score = 0.6 + 0.3 * retrieval::term_score(&text, terms) - i as f64 * 0.001;
            h.passage("marker", h.offset_ms(&r.get::<String, _>("ts")), clip(&text, retrieval::EXCERPT_CHARS), score)
        })
        .collect())
}

/// Other recordings to draw on: the same Notebook, or the same people.
async fn related_ids(conn: &mut SqliteConnection, h: &Head) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = Vec::new();
    if let Some(nb) = &h.notebook {
        ids.extend(
            sqlx::query_scalar::<_, String>(
                "SELECT id FROM meetings WHERE class_name = ? COLLATE NOCASE AND id != ? AND started_at < ? ORDER BY started_at DESC LIMIT 12",
            )
            .bind(nb)
            .bind(&h.id)
            .bind(&h.started_at)
            .fetch_all(&mut *conn)
            .await
            .map_err(err("Couldn't read the recordings"))?,
        );
    }
    if has_table(conn, "people").await && has_table(conn, "meeting_attendees").await {
        // "The same people" needs to know which attendee is the user: without
        // that, the user's own address would make every recording related. An
        // address that is in most of the user's recordings (a room, a team
        // list, an unflagged alias) says nothing either.
        let with_people: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT m.id FROM meetings m JOIN meeting_attendees x ON x.meeting_id = m.id \
             WHERE m.id != ?1 AND m.started_at < ?2 \
               AND EXISTS (SELECT 1 FROM meeting_attendees s JOIN people sp ON sp.id = lower(s.email) \
                           WHERE s.meeting_id = ?1 AND sp.is_self = 1) \
               AND lower(x.email) IN ( \
                 SELECT lower(a.email) FROM meeting_attendees a LEFT JOIN people p ON p.id = lower(a.email) \
                 WHERE a.meeting_id = ?1 AND COALESCE(p.is_self, 0) = 0 \
                   AND (SELECT COUNT(DISTINCT y.meeting_id) FROM meeting_attendees y WHERE lower(y.email) = lower(a.email)) * 2 \
                       <= MAX(6, (SELECT COUNT(DISTINCT z.meeting_id) FROM meeting_attendees z))) \
             ORDER BY m.started_at DESC LIMIT 12",
        )
        .bind(&h.id)
        .bind(&h.started_at)
        .fetch_all(&mut *conn)
        .await
        .map_err(err("Couldn't read the recordings"))?;
        for id in with_people {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

/// The saved notes of the newest related recordings (what was decided and
/// promised before), and their lines matched to the question.
async fn related(conn: &mut SqliteConnection, ids: &[String], question: &str, terms: &[String]) -> Result<Vec<Passage>, String> {
    let mut out: Vec<Passage> = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let filter = Filter::Meeting(id.clone());
        if i < MAX_RELATED {
            // Its summary, whatever the question: it is what came before
            let r = sqlx::query(
                "SELECT n.summary, m.title, m.started_at, m.recording_kind, m.class_name FROM meeting_notes n JOIN meetings m ON m.id = n.meeting_id \
                 WHERE n.meeting_id = ? AND n.summary IS NOT NULL AND TRIM(n.summary) != '' ORDER BY n.generated_at DESC LIMIT 1",
            )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(err("Couldn't read the notes"))?;
            if let Some(r) = r {
                let h = Head {
                    id: id.clone(),
                    title: r.get("title"),
                    started_at: r.get("started_at"),
                    ended_at: None,
                    kind: crate::recording_kind::RecordingKind::from_stored(r.get::<Option<String>, _>("recording_kind").as_deref())
                        .as_str()
                        .to_string(),
                    notebook: r.get::<Option<String>, _>("class_name").filter(|n| !n.trim().is_empty()),
                };
                let summary: String = r.get("summary");
                let text = format!("Summary: {}", crate::redaction::render_plain(&summary));
                out.push(h.passage("notes", None, clip(&text, retrieval::EXCERPT_CHARS + 160), 0.35 - i as f64 * 0.02));
            }
        }
        for mut p in retrieval::notes_hits(conn, &filter, terms).await? {
            p.score *= 0.8;
            out.push(p);
        }
        for mut p in retrieval::transcript_hits(conn, &filter, question).await? {
            p.score *= 0.7;
            out.push(p);
        }
    }
    Ok(out)
}

/// Everything a question about this recording is answered from.
#[derive(Debug, Clone, PartialEq)]
pub struct Sources {
    pub brief: String,
    /// The user's notes and marks in this recording
    pub notes: Vec<Passage>,
    /// Lines of this recording, before the last minutes, matched to the question
    pub earlier: Vec<Passage>,
    /// From other recordings
    pub related: Vec<Passage>,
    /// The last minutes, in order
    pub recent: Vec<Passage>,
}

impl Sources {
    /// Every source, numbered from 1 in the order the prompt shows them.
    pub fn numbered(mut self) -> Sources {
        let mut n = 0usize;
        for p in self.notes.iter_mut().chain(self.earlier.iter_mut()).chain(self.related.iter_mut()).chain(self.recent.iter_mut()) {
            n += 1;
            p.n = n;
        }
        self
    }

    pub fn all(&self) -> Vec<Passage> {
        self.notes.iter().chain(&self.earlier).chain(&self.related).chain(&self.recent).cloned().collect()
    }
}

/// Put the sources together for `question`, within `budget` characters.
pub async fn sources(
    conn: &mut SqliteConnection,
    h: &Head,
    question: &str,
    budget: usize,
    now: DateTime<Utc>,
    mode: Mode,
) -> Result<Sources, String> {
    let terms = crate::database::search_terms(question);
    let all = lines_since(conn, h, 0).await?;
    let newest = all.last().map(|l| l.ms).unwrap_or(0);
    let recent_from = (newest - RECENT_MS).max(0);
    let recent_lines: Vec<Line> = all.iter().filter(|l| l.ms >= recent_from).cloned().collect();
    let tagged = all.iter().any(|l| crate::audio_source::label(l.side.as_deref()).is_some());

    // Half for the last minutes; the rest shared by what the user wrote,
    // what was said earlier and what came before this recording
    let recent = newest_that_fit(recent_chunks(h, &recent_lines), budget / 2);
    let spent: usize = recent.iter().map(Passage::cost).sum();
    let rest = budget.saturating_sub(spent);
    // "Earlier" is everything before the first line that is given as recent:
    // in a small window that is less than six minutes back, and what falls
    // between must still be findable
    let recent_from = recent.first().and_then(|p| p.timestamp_ms).unwrap_or(recent_from);

    let notes = best_that_fit(own_notes(conn, h, &terms).await?, rest * 3 / 10, MAX_NOTES, &recent);
    let mut earlier_cands: Vec<Passage> = retrieval::transcript_hits(conn, &Filter::Meeting(h.id.clone()), question)
        .await?
        .into_iter()
        .filter(|p| p.timestamp_ms.is_some_and(|ms| ms < recent_from))
        .collect();
    // A question with no words to match ("what have we covered?"): lines
    // spread over what came before the last minutes
    let before: Vec<&Line> = all.iter().filter(|l| l.ms < recent_from).collect();
    if earlier_cands.len() < 2 && !before.is_empty() {
        let want = before.len().min(SAMPLED_LINES);
        for k in 0..want {
            let l = before[(k * before.len() + before.len() / 2) / want];
            earlier_cands.push(h.passage("transcript", Some(l.ms), clip(&l.text, retrieval::EXCERPT_CHARS), 0.2));
        }
    }
    // Tag the matched lines the same way the recent ones are
    for p in earlier_cands.iter_mut() {
        if let Some(l) = all.iter().find(|l| Some(l.ms) == p.timestamp_ms) {
            if let Some(tag) = crate::audio_source::label(l.side.as_deref()) {
                p.excerpt = format!("({}) {}", tag, p.excerpt);
            }
        }
    }
    // The notes already made from this recording, when there are any
    let saved: Option<String> = sqlx::query_scalar(
        "SELECT summary FROM meeting_notes WHERE meeting_id = ? AND summary IS NOT NULL AND TRIM(summary) != '' ORDER BY generated_at DESC LIMIT 1",
    )
    .bind(&h.id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(err("Couldn't read the notes"))?;
    if let Some(summary) = saved {
        let text = format!("The saved notes' summary of this recording: {}", crate::redaction::render_plain(&summary));
        earlier_cands.push(h.passage("notes", None, clip(&text, retrieval::EXCERPT_CHARS + 240), 0.95));
    }
    // What the notes made from this recording say about the question:
    // decisions, action items, key points, what Fill in the blanks added
    earlier_cands.extend(retrieval::notes_hits(conn, &Filter::Meeting(h.id.clone()), &terms).await?);
    let earlier = best_that_fit(earlier_cands, rest * 4 / 10, SAMPLED_LINES + 4, &recent);
    let ids = if mode.related { related_ids(conn, h).await? } else { Vec::new() };
    let mut taken = recent.clone();
    taken.extend(notes.iter().cloned());
    taken.extend(earlier.iter().cloned());
    let related = best_that_fit(related(conn, &ids, question, &terms).await?, rest * 3 / 10, 5, &taken);

    let who = people(conn, &h.id).await?;
    let ev = event(conn, &h.id).await;
    let mut earlier_sorted = earlier;
    earlier_sorted.sort_by_key(|p| p.timestamp_ms.unwrap_or(0));
    let mut notes_sorted = notes;
    notes_sorted.sort_by_key(|p| p.timestamp_ms.unwrap_or(0));
    Ok(Sources { brief: brief_text(h, &who, ev.as_ref(), tagged, now, mode.recording), notes: notes_sorted, earlier: earlier_sorted, related, recent }
        .numbered())
}

// ═══════════════════════════════════════════════════════════════════════════
// The prompt
// ═══════════════════════════════════════════════════════════════════════════

pub fn system_prompt() -> String {
    "You are noFriction's assistant, sitting in on the user's recording with them. They ask you things in the moment, \
often while people are still talking, so answer fast: the answer first, in one or two sentences, then at most four short \
bullets if they add something. No preamble, no closing line.\n\
\n\
What you are given: CONTEXT (the recording, the people in it and their companies; do not cite it) and numbered SOURCES \
(the user's own notes and marks, what was said in this recording, the last minutes in order, and earlier recordings).\n\
\n\
Rules:\n\
- What was said comes only from the sources. Cite it as [n] right after the fact, using only the numbers given, and prefer \
the one best source. If the sources don't have it, say so in a few words; never fill in what someone might have said.\n\
- When asked what something means, or about a name, company, product or term that came up: first say how it is being \
talked about here, with a citation. Then add what it is in general, in a sentence or two, and mark that part \
\"(background, not said in the recording)\". If you don't know it, say so.\n\
- Who said it: the transcript does not know voices. Use the (mic) and (computer audio) tags and the people list. Say \
\"you\" for the user's side only when a (mic) line clearly is the user, and name another person only when the words \
themselves make it clear (they are addressed by name, or introduce themselves), or as the context allows under THE OTHER \
SIDE, marked \"(likely)\". Otherwise say \"someone on the call\" or \"someone in the room\". Never guess a name.\n\
- Use the people list for who is who: names, companies and who organised it.\n\
- \"Just now\", \"what did they say\", \"what did I miss\" mean the last minutes: read them in order; the newest is last.\n\
- The transcript comes from speech recognition and can be wrong, most of all on names and numbers. Say when something \
looks misheard instead of repeating it as fact.\n\
- Text shown as [stricken from the record] was removed by the user: never guess at it or mention it.\n\
- Markdown: short paragraphs or bullets, bold for the key point, no headings, no tables, no HTML."
        .to_string()
}

fn source_line(p: &Passage, own: bool) -> String {
    if !own {
        return prompt::passage_line(p);
    }
    match (p.source, p.timestamp_ms) {
        ("marker", Some(ms)) => format!("[{}] at {} — {}", p.n, clock(ms), p.excerpt),
        (_, Some(ms)) => format!("[{}] {} — {}", p.n, clock(ms), p.excerpt),
        _ => format!("[{}] {}", p.n, p.excerpt),
    }
}

fn block(out: &mut String, heading: &str, items: &[Passage], own: bool) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("\n\n{}\n", heading));
    out.push_str(&items.iter().map(|p| source_line(p, own)).collect::<Vec<_>>().join(if own { "\n" } else { "\n\n" }));
}

/// The user message: the context, the sources by kind, and the question.
pub fn user_message(s: &Sources, question: &str) -> String {
    let mut out = format!("CONTEXT\n{}", s.brief);
    out.push_str("\n\nSOURCES");
    block(&mut out, "THE USER'S OWN NOTES AND MARKS IN THIS RECORDING:", &s.notes, true);
    block(&mut out, "EARLIER IN THIS RECORDING:", &s.earlier, true);
    block(&mut out, "FROM EARLIER RECORDINGS:", &s.related, false);
    if s.recent.is_empty() {
        out.push_str("\n\nTHE LAST MINUTES: (nothing has been transcribed yet)");
    } else {
        block(&mut out, "THE LAST MINUTES OF THIS RECORDING, IN ORDER (the newest is last):", &s.recent, true);
    }
    out.push_str(&format!("\n\nQUESTION: {}", question.trim()));
    out
}

/// Answer `question` about the recording `meeting_id`, as of `now`.
pub async fn answer(
    conn: &mut SqliteConnection,
    c: &dyn Completer,
    meeting_id: &str,
    history: &[(String, String)],
    question: &str,
    now: DateTime<Utc>,
    mode: Mode,
) -> Result<Answer, String> {
    let question = question.trim();
    if question.is_empty() {
        return Err("Ask something first.".into());
    }
    let h = head(conn, meeting_id).await?;
    let mut label = retrieval::resolve(conn, &Scope::meeting(meeting_id)).await?.label;
    let ctx = c.context_tokens();
    let max_tokens = prompt::max_tokens(ctx).min(if mode.glance { GLANCE_ANSWER_TOKENS } else { MAX_ANSWER_TOKENS });
    let hist = prompt::history_messages(history, ctx);
    let system = if mode.glance { system_prompt() + GLANCE_RULE } else { system_prompt() };
    // What is there whatever the sources: the context, the question, the memory
    let who = people(conn, &h.id).await?;
    let ev = event(conn, &h.id).await;
    // The headings and the question line are part of what is always there
    let fixed = brief_text(&h, &who, ev.as_ref(), true, now, mode.recording)
        + &user_message(&Sources { brief: String::new(), notes: vec![], earlier: vec![], related: vec![], recent: vec![] }, question)
        + &"x".repeat(240)
        + &hist.iter().map(|m| m.text()).collect::<Vec<_>>().join("\n");
    let budget = prompt::passage_budget(ctx, max_tokens, &system, &fixed);
    let s = sources(conn, &h, question, budget, now, mode).await?;
    // The answer says what it drew on
    if !s.related.is_empty() {
        label.push_str(" and earlier recordings");
    }
    let mut msgs = vec![Msg::system(system)];
    msgs.extend(hist);
    msgs.push(Msg::user(user_message(&s, question)));
    let content = match c.complete(msgs, Opts { max_tokens, temperature: Some(0.2) }).await {
        Ok(raw) => prompt::clean_answer(&raw),
        Err(AiError::Truncated) => return Err(AiError::Truncated.to_string()),
        Err(e) => return Err(e.to_string()),
    };
    if content.is_empty() {
        return Err("The model sent an empty answer. Try again.".into());
    }
    Ok(Answer { content, passages: s.all(), scope_label: label })
}

// ═══════════════════════════════════════════════════════════════════════════
// What Ask knows, for the panel (no AI call)
// ═══════════════════════════════════════════════════════════════════════════

/// What a question about this recording would draw on right now. Built
/// locally; shown in the panel so the user sees what their AI is given.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Knows {
    pub meeting_id: String,
    pub title: String,
    /// "meeting" | "class" | "personal"
    pub kind: String,
    pub notebook: Option<String>,
    /// Still being recorded (no end time yet)
    pub recording: bool,
    /// Transcript lines so far
    pub lines: i64,
    /// Lines that say which side spoke
    pub tagged: i64,
    pub people: Vec<Who>,
    pub event: Option<String>,
    /// The event has a place, or notes, that go with a question
    pub event_details: bool,
    /// Notes were already made from this recording
    pub saved_notes: bool,
    /// The user's typed notes and marks
    pub notes: i64,
    /// Earlier recordings it can draw on (same Notebook or people)
    pub related: i64,
}

pub async fn knows(conn: &mut SqliteConnection, meeting_id: &str, recording: bool) -> Result<Knows, String> {
    let h = head(conn, meeting_id).await?;
    let (lines, tagged): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(source IN ('mic', 'system')), 0) FROM transcripts WHERE meeting_id = ? AND is_final = 1",
    )
    .bind(meeting_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(err("Couldn't read the transcript"))?;
    let notes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_markers WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(err("Couldn't read the notes"))?;
    let related = related_ids(conn, &h).await?.len() as i64;
    let who = people(conn, meeting_id).await?;
    let ev = event(conn, meeting_id).await;
    let event_details = ev.as_ref().is_some_and(|(_, place, notes)| place.is_some() || notes.is_some());
    let ev = ev.and_then(|(title, _, _)| title);
    let saved_notes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_notes WHERE meeting_id = ?")
        .bind(meeting_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(err("Couldn't read the notes"))?;
    Ok(Knows {
        meeting_id: h.id.clone(),
        title: h.title.clone(),
        kind: h.kind.clone(),
        notebook: h.notebook.clone(),
        recording,
        lines,
        tagged,
        people: who,
        event: ev,
        event_details,
        saved_notes: saved_notes > 0,
        notes,
        related,
    })
}
