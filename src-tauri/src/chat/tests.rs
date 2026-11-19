//! Chat with your recordings: scope, retrieval, citations and the purge
//! (docs/TOPICS_AND_CHAT.md, docs/REDACTION.md). The AI is always a mock
//! here; no endpoint is called.

use super::prompt::*;
use super::retrieval::*;
use super::*;
use crate::ai::{AiError, Msg, Opts};
use crate::database::DatabaseManager;
use crate::redaction::{self as rd, RedactionEnv, WordTarget};
use parking_lot::Mutex;
use sqlx::{ConnectOptions, Connection};
use std::path::PathBuf;
use std::sync::Arc;

// ═══════════════════════════════════════════════════════════════════════════
// Pure pieces
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn citation_numbers_are_read_in_order_of_first_use() {
    assert_eq!(cited_numbers("Ship Friday [2]. Budget agreed [1], see [2] and [1, 3]."), vec![2, 1, 3]);
    assert_eq!(cited_numbers("[12] then [x] and [] and [0]"), vec![12]);
    assert_eq!(cited_numbers("no citations"), Vec::<usize>::new());
    assert_eq!(cited_numbers("[1"), Vec::<usize>::new());
}

#[test]
fn answers_are_cleaned_and_titles_clipped() {
    assert_eq!(clean_answer("<think>plan</think>\n```markdown\n**Yes** [1]\n```"), "**Yes** [1]");
    assert_eq!(clean_answer("  Plain [1].  "), "Plain [1].");
    assert_eq!(clean_answer("```\nnot closed"), "```\nnot closed");
    assert_eq!(title_of("  What did we   decide? "), "What did we decide?");
    let t = title_of(&"word ".repeat(30));
    assert!(t.chars().count() <= 48 && t.ends_with('…'));
}

#[test]
fn an_answer_being_written_is_cleaned_like_the_finished_one() {
    assert_eq!(clean_partial("  They decided to **ship"), "They decided to **ship");
    // A whole-answer fence: its opening line never shows, nor its closing one
    assert_eq!(clean_partial("```"), "");
    assert_eq!(clean_partial("```mark"), "");
    assert_eq!(clean_partial("```markdown\n**Yes** [1]"), "**Yes** [1]");
    assert_eq!(clean_partial("```markdown\n**Yes** [1]\n```"), "**Yes** [1]");
    assert_eq!(clean_partial("```markdown\n**Yes** [1]\n```"), clean_answer("```markdown\n**Yes** [1]\n```"));
    // Code inside a longer answer is not a wrapper
    let mixed = "```js\nlet x = 1;\n```\nThat sets x.";
    assert_eq!(clean_partial(mixed), mixed);
}

#[test]
fn the_answer_so_far_is_shown_a_few_times_a_second_and_stops_at_a_purge() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    let t = Throttle::new(Duration::from_secs(3600));
    assert!(t.ready(), "the first one at once");
    assert!(!t.ready());
    assert!(Throttle::new(Duration::ZERO).ready());

    let shown: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (to, flag) = (shown.clone(), stop.clone());
    let sink = stream_sink_until(Duration::ZERO, move || flag.load(Ordering::SeqCst), move |text| to.lock().push(text));
    sink("```markdown");
    sink("```markdown\nShip");
    sink("```markdown\nShip **Friday**");
    assert_eq!(*shown.lock(), vec!["Ship", "Ship **Friday**"], "nothing while there is nothing to read");
    // Text was removed from a recording: what is on screen goes, and no more comes
    stop.store(true, Ordering::SeqCst);
    sink("```markdown\nShip **Friday** [1]");
    stop.store(false, Ordering::SeqCst);
    sink("```markdown\nShip **Friday** [1], as agreed");
    assert_eq!(*shown.lock(), vec!["Ship", "Ship **Friday**", ""]);

    // The real sink: updates inside its interval are skipped, never queued
    let shown: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let to = shown.clone();
    let sink = stream_sink_until(Duration::from_secs(3600), || false, move |text| to.lock().push(text));
    sink("One");
    sink("One two");
    assert_eq!(*shown.lock(), vec!["One"]);
}

/// A model that is still writing when text is removed from a recording.
struct PurgedMeanwhile {
    pool: Pool<Sqlite>,
    meeting: &'static str,
}

#[async_trait::async_trait]
impl Completer for PurgedMeanwhile {
    async fn complete(&self, _msgs: Vec<Msg>, _opts: Opts) -> Result<String, AiError> {
        let mut conn = self.pool.acquire().await.unwrap();
        purge_for_meeting(&mut conn, self.meeting).await.unwrap();
        Ok("They decided to **ship Friday** [1].".into())
    }
    fn context_tokens(&self) -> usize {
        32_768
    }
}

#[tokio::test]
async fn an_answer_written_while_text_was_removed_is_not_kept() {
    let f = fx().await;
    let other = fx().await;
    let mut conn = f.db.pool().acquire().await.unwrap();
    let watch = PurgeWatch::start(&mut conn).await;
    drop(conn);
    let elsewhere = PurgeWatch::of(other.db.pool()).await.unwrap();
    assert!(!watch.purged());
    // The recording asked about, and one the answer never drew on: either
    // way text left the library while the model was writing
    for (meeting, question) in [("m2", "What did we decide about the launch?"), ("m3", "What did we decide about the launch?")] {
        let c = PurgedMeanwhile { pool: f.db.pool().clone(), meeting };
        let e = ask(f.db.pool(), &c, None, &Scope::notebook("Acme"), question).await.unwrap_err();
        assert_eq!(e, PURGED_MEANWHILE);
        let e = ask_recording(f.db.pool(), &c, None, "m2", question, live::Mode { recording: false, related: true, glance: false }).await.unwrap_err();
        assert_eq!(e, PURGED_MEANWHILE);
    }
    assert!(list_threads(f.db.pool()).await.unwrap().is_empty(), "no thread, no question, no answer");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(n, 0);
    assert!(watch.purged(), "and what was on screen of it goes");
    // Another library (an app backup, another test) is not this one
    assert!(!elsewhere.purged());
    // Afterwards a question is answered and kept as ever
    let turn = ask(f.db.pool(), &Mock::new("Friday [1]."), None, &Scope::notebook("Acme"), "When do we ship?").await.unwrap();
    assert_eq!(turn.assistant.content, "Friday [1].");
}

#[test]
fn scopes_parse_and_fall_back_to_all() {
    let s: Scope = serde_json::from_str(r#"{"kind": "notebook", "value": "BIO 101"}"#).unwrap();
    assert_eq!(s, Scope::notebook("BIO 101"));
    let s: Scope = serde_json::from_str(r#"{"kind": "all"}"#).unwrap();
    assert_eq!(s, Scope::all());
    assert!(serde_json::from_str::<Scope>(r#"{"kind": "galaxy"}"#).is_err());
    assert_eq!(Scope::from_stored("galaxy", Some("x".into())), Scope::all());
    assert_eq!(Scope::from_stored("meeting", Some(" ".into())).value, None);
    assert_eq!(Scope::from_stored("topic", Some("q4 roadmap".into())), Scope::topic("q4 roadmap"));
    assert_eq!(Filter::Notebook("x".into()).clause("t.meeting_id"), " AND t.meeting_id IN (SELECT id FROM meetings WHERE class_name = ? COLLATE NOCASE)");
    assert_eq!(Filter::All.clause("id"), "");
}

#[test]
fn packing_ranks_caps_per_recording_and_fits_the_budget() {
    let p = |m: &str, text: &str, score: f64| Passage {
        n: 0,
        meeting_id: m.into(),
        title: m.into(),
        started_at: "2026-10-06T10:00:00Z".into(),
        kind: "meeting".into(),
        notebook: None,
        source: "transcript",
        timestamp_ms: Some(0),
        excerpt: text.into(),
        score,
    };
    let all = vec![p("a", "x1", 0.1), p("a", "x2", 0.9), p("a", "x3", 0.8), p("b", "y1", 0.5), p("a", "x2", 0.9)];
    let got = pack(all.clone(), 10_000, 12, 2);
    assert_eq!(got.iter().map(|g| g.excerpt.as_str()).collect::<Vec<_>>(), vec!["x2", "x3", "y1"], "ranked, duplicate and third 'a' passage dropped");
    assert_eq!(got.iter().map(|g| g.n).collect::<Vec<_>>(), vec![1, 2, 3]);
    let got = pack(all.clone(), 200, 12, 12);
    assert_eq!(got.len(), 2, "two passages of ~92 chars fit in 200");
    assert_eq!(pack(all, 10, 12, 12).len(), 1, "the best passage always fits");
    assert_eq!(term_score("The Q4 Roadmap is set", &["q4".into(), "roadmap".into(), "budget".into()]), 2.0 / 3.0);
    assert_eq!(clip("a  b\n c", 10), "a b c");
    assert_eq!(clip("abcdefghij", 5), "abcd…");
}

#[test]
fn history_is_clipped_for_small_windows() {
    let h: Vec<(String, String)> = (0..8).map(|i| ((if i % 2 == 0 { "user" } else { "assistant" }).to_string(), format!("turn {} {}", i, "x".repeat(900)))).collect();
    let big = history_messages(&h, 32_768);
    assert_eq!(big.len(), 8);
    assert_eq!(big[1].role, crate::ai::Role::Assistant);
    let small = history_messages(&h, 4_096);
    assert_eq!(small.len(), 4);
    assert!(small[0].text().starts_with("turn 4"));
    assert!(small.iter().all(|m| m.text().chars().count() <= 500));
    assert!(max_tokens(4_096) < max_tokens(32_768));
}

// ═══════════════════════════════════════════════════════════════════════════
// Database fixture
// ═══════════════════════════════════════════════════════════════════════════

struct Mock {
    ctx: usize,
    calls: Mutex<Vec<Vec<Msg>>>,
    reply: Mutex<String>,
    error: Option<AiError>,
}

impl Mock {
    fn new(reply: &str) -> Self {
        Self { ctx: 32_768, calls: Mutex::new(Vec::new()), reply: Mutex::new(reply.into()), error: None }
    }
    fn last_prompt(&self) -> String {
        self.calls.lock().last().unwrap().iter().map(|m| m.text()).collect::<Vec<_>>().join("\n")
    }
}

#[async_trait::async_trait]
impl Completer for Mock {
    async fn complete(&self, msgs: Vec<Msg>, _opts: Opts) -> Result<String, AiError> {
        self.calls.lock().push(msgs);
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        Ok(self.reply.lock().clone())
    }
    fn context_tokens(&self) -> usize {
        self.ctx
    }
}

struct Fx {
    db: Arc<DatabaseManager>,
    dir: PathBuf,
    env: RedactionEnv,
    db_path: PathBuf,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn start_of(f: &Fx, meeting: &str) -> chrono::DateTime<chrono::Utc> {
    let s: String = sqlx::query_scalar("SELECT started_at FROM meetings WHERE id = ?").bind(meeting).fetch_one(f.db.pool()).await.unwrap();
    chrono::DateTime::parse_from_rfc3339(&s).unwrap().with_timezone(&chrono::Utc)
}

async fn line_at(f: &Fx, meeting: &str, secs: i64, text: &str) -> i64 {
    let t0 = start_of(f, meeting).await;
    f.db.add_transcript_at(meeting, text, None, true, 0.9, t0 + chrono::Duration::seconds(secs)).await.unwrap()
}

/// Three recordings: m1 (class, Notebook BIO 101, topic "cell membrane"),
/// m2 (meeting, Notebook Acme, with notes), m3 (personal, topic "cell membrane").
async fn fx() -> Fx {
    let dir = std::env::temp_dir().join(format!("nf-chat-{}", uuid::Uuid::new_v4()));
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let db_path = data.join("nofriction_meetings.db");
    let db = DatabaseManager::new(&db_path).await.unwrap();
    db.run_migrations().await.unwrap();
    db.create_meeting("m1", "Biology 101").await.unwrap();
    db.create_meeting("m2", "Acme sync").await.unwrap();
    db.create_meeting("m3", "Dentist").await.unwrap();
    db.set_meeting_notebook("m1", Some("BIO 101")).await.unwrap();
    db.set_meeting_notebook("m2", Some("Acme")).await.unwrap();
    db.set_meeting_kind("m1", crate::recording_kind::RecordingKind::Class).await.unwrap();
    db.set_meeting_kind("m3", crate::recording_kind::RecordingKind::Personal).await.unwrap();
    let f = Fx {
        env: RedactionEnv { app_data_dir: data, cache_dir: dir.join("cache"), video_enabled: false, recording_meetings: Vec::new() },
        db: Arc::new(db),
        dir,
        db_path,
    };
    line_at(&f, "m1", 10, "the cell membrane is selectively permeable").await;
    line_at(&f, "m1", 20, "mitochondria make ATP for the cell").await;
    line_at(&f, "m1", 30, "osmosis moves water across the membrane").await;
    line_at(&f, "m2", 20, "we decided to ship the launch on Friday").await;
    line_at(&f, "m2", 40, "the budget stays at ten thousand").await;
    line_at(&f, "m3", 5, "the dentist said floss every night").await;
    f.db
        .save_meeting_notes("n2", "m2", Some("Launch planning sync."), Some("[\"launch\"]"), Some("[{\"text\": \"Ship the launch Friday\", \"made_by\": \"Ana\"}]"), Some("[{\"task\": \"Send the budget\", \"assignee\": \"Bo\"}]"), None, Some("default"))
        .await
        .unwrap();
    crate::markers::add_at_offset(f.db.pool(), "m2", 40_000, Some("test"), Some("budget follow up")).await.unwrap();
    crate::topics::set_for_meeting(f.db.pool(), "m1", &["Cell membranes".into()]).await.unwrap();
    crate::topics::set_for_meeting(f.db.pool(), "m3", &["cell membrane".into(), "Flossing".into()]).await.unwrap();
    f
}

fn meetings_of(p: &[Passage]) -> Vec<&str> {
    let mut v: Vec<&str> = p.iter().map(|x| x.meeting_id.as_str()).collect();
    v.dedup();
    v.sort();
    v.dedup();
    v
}

// ═══════════════════════════════════════════════════════════════════════════
// Retrieval
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn retrieval_stays_inside_the_scope_and_the_budget() {
    let f = fx().await;
    let mut conn = f.db.pool().acquire().await.unwrap();
    // All: the transcript line about mitochondria ranks first, with its time
    let info = resolve(&mut conn, &Scope::all()).await.unwrap();
    assert_eq!((info.label.as_str(), info.count), ("All recordings", 3));
    let p = retrieve(&mut conn, &info, "what makes ATP in the cell?", 4_000, 12).await.unwrap();
    assert_eq!(p[0].meeting_id, "m1");
    assert_eq!(p[0].timestamp_ms, Some(20_000));
    assert_eq!((p[0].n, p[0].source, p[0].kind.as_str(), p[0].notebook.as_deref()), (1, "transcript", "class", Some("BIO 101")));
    assert!(p[0].excerpt.contains("mitochondria"));
    // Notebook Acme: nothing about ATP there, so its own recent passages come instead
    let info = resolve(&mut conn, &Scope::notebook("acme")).await.unwrap();
    assert_eq!((info.label.as_str(), info.count), ("Notebook · acme", 1));
    let p = retrieve(&mut conn, &info, "what makes ATP in the cell?", 4_000, 12).await.unwrap();
    assert_eq!(meetings_of(&p), vec!["m2"]);
    assert!(p.iter().any(|x| x.source == "notes" && x.excerpt.starts_with("Summary:")), "{:?}", p);
    // Notes and marker notes match by term, with the marker's time
    let p = retrieve(&mut conn, &info, "who sends the budget?", 4_000, 12).await.unwrap();
    assert!(p.iter().any(|x| x.source == "notes" && x.excerpt.contains("Send the budget (Bo)")), "{:?}", p);
    assert!(p.iter().any(|x| x.source == "marker" && x.timestamp_ms == Some(40_000) && x.excerpt.contains("Follow up: budget")), "{:?}", p);
    assert!(p.iter().any(|x| x.source == "transcript" && x.timestamp_ms == Some(40_000)));
    // Fill in the blanks is part of the notes: its lines are found by their
    // words, and background keeps its marking
    let filled = r#"{"missed":[{"at_ms":41000,"text":"The contingency is two thousand."}],"more":[{"about":"Contingency","detail":"Money set aside for surprises.","at_ms":null,"background":true}],"typed":{}}"#;
    sqlx::query("UPDATE meeting_notes SET filled = ? WHERE meeting_id = 'm2'").bind(filled).execute(&mut *conn).await.unwrap();
    let p = retrieve(&mut conn, &info, "how big is the contingency?", 4_000, 12).await.unwrap();
    assert!(p.iter().any(|x| x.source == "notes" && x.excerpt.contains("[0:41] The contingency is two thousand.")), "{:?}", p);
    assert!(
        p.iter().any(|x| x.source == "notes" && x.excerpt.contains("Contingency: Money set aside for surprises. (background, not said in the recording)")),
        "{:?}",
        p
    );
    // Topic: m1 and m3 share "cell membrane"; m2 never appears
    let info = resolve(&mut conn, &Scope::topic("cell membrane")).await.unwrap();
    assert_eq!((info.label.as_str(), info.count), ("Topic · Cell membranes", 2));
    let p = retrieve(&mut conn, &info, "membrane dentist budget launch", 4_000, 12).await.unwrap();
    assert_eq!(meetings_of(&p), vec!["m1", "m3"]);
    // One recording: only it, and a question with no matching terms samples its lines
    let info = resolve(&mut conn, &Scope::meeting("m2")).await.unwrap();
    assert_eq!(info.label, "Recording · Acme sync");
    let p = retrieve(&mut conn, &info, "summarize this", 4_000, 12).await.unwrap();
    assert_eq!(meetings_of(&p), vec!["m2"]);
    assert!(p.iter().any(|x| x.source == "transcript") && p.iter().any(|x| x.source == "notes"));
    // The budget bounds the total; a tiny window still gets one passage
    let info = resolve(&mut conn, &Scope::all()).await.unwrap();
    let p = retrieve(&mut conn, &info, "membrane cell water ATP osmosis", 150, 12).await.unwrap();
    assert_eq!(p.len(), 1);
    let p = retrieve(&mut conn, &info, "membrane cell water ATP osmosis", 600, 12).await.unwrap();
    assert!(p.len() >= 2 && p.iter().map(|x| x.cost()).sum::<usize>() <= 600, "{:?}", p);
    // Unknown scopes are refused
    assert!(resolve(&mut conn, &Scope::meeting("nope")).await.is_err());
    assert!(resolve(&mut conn, &Scope { kind: ScopeKind::Notebook, value: None }).await.is_err());
    // The summary feeds the suggested questions
    let s = summary(&mut conn, &Scope::all()).await.unwrap();
    assert_eq!(s.count, 3);
    assert_eq!(s.recent_titles.len(), 3);
    assert_eq!(s.topics[0], "Cell membranes");
    assert!(s.notebooks.contains(&"BIO 101".to_string()) && s.notebooks.contains(&"Acme".to_string()));
    assert_eq!(s.kinds.len(), 3);
    let s = summary(&mut conn, &Scope::notebook("BIO 101")).await.unwrap();
    assert_eq!((s.count, s.recent_titles[0].as_str(), s.kinds.as_slice()), (1, "Biology 101", &["class".to_string()][..]));
}

// ═══════════════════════════════════════════════════════════════════════════
// Asking, memory, citations
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn an_answer_carries_its_scope_and_citations_and_the_thread_remembers() {
    let f = fx().await;
    let m = Mock::new("They decided to **ship Friday** [1].\n\n- Budget: ten thousand [2]");
    let turn = ask(f.db.pool(), &m, None, &Scope::notebook("Acme"), "What did we decide about the launch?").await.unwrap();
    assert_eq!(turn.thread.title, "What did we decide about the launch?");
    assert_eq!(turn.thread.scope, Scope::notebook("Acme"));
    assert_eq!(turn.user.content, "What did we decide about the launch?");
    assert_eq!(turn.assistant.scope_label.as_deref(), Some("Notebook · Acme"));
    assert!(turn.assistant.content.starts_with("They decided"));
    let c1 = turn.assistant.citations.iter().find(|c| c.n == 1).unwrap();
    assert_eq!((c1.meeting_id.as_str(), c1.title.as_str(), c1.timestamp_ms), ("m2", "Acme sync", Some(20_000)));
    assert!(c1.excerpt.contains("ship the launch on Friday"));
    assert_eq!(cited_numbers(&turn.assistant.content), vec![1, 2]);
    assert!(turn.assistant.citations.iter().all(|c| c.meeting_id == "m2"));
    // The prompt: system rules, scope, numbered sources with recording facts, the question
    let p = m.last_prompt();
    assert!(p.starts_with(&system_prompt()));
    assert!(p.contains("SCOPE: Notebook · Acme (1 recording)\n"), "{}", p);
    assert!(p.contains("[1] Acme sync — ") && p.contains(", Meeting, Notebook Acme — said at 0:20\nwe decided to ship the launch on Friday"), "{}", p);
    assert!(p.ends_with("QUESTION: What did we decide about the launch?"));
    // A second question in the thread carries the first exchange
    let m2 = Mock::new("Friday [1].");
    let t2 = ask(f.db.pool(), &m2, Some(&turn.thread.id), &Scope::all(), "And when exactly?").await.unwrap();
    let msgs = m2.calls.lock()[0].clone();
    assert_eq!(msgs.len(), 4, "system, user, assistant, user");
    assert_eq!(msgs[1].text(), "What did we decide about the launch?");
    assert!(msgs[2].text().starts_with("They decided"));
    assert_eq!(t2.thread.id, turn.thread.id);
    assert_eq!(t2.thread.scope, Scope::all(), "the thread follows the latest scope");
    assert_eq!(t2.assistant.scope_label.as_deref(), Some("All recordings"));
    let detail = get_thread(f.db.pool(), &turn.thread.id).await.unwrap();
    assert_eq!(detail.messages.iter().map(|x| x.role.as_str()).collect::<Vec<_>>(), vec!["user", "assistant", "user", "assistant"]);
    assert_eq!(detail.messages[1].citations.len(), turn.assistant.citations.len());
    assert_eq!(list_threads(f.db.pool()).await.unwrap().len(), 1);
    // AI access errors come back as they are, and nothing is stored
    let mut bad = Mock::new("");
    bad.error = Some(AiError::ProRequired);
    let e = ask(f.db.pool(), &bad, None, &Scope::all(), "anything").await.unwrap_err();
    assert!(e.starts_with("PRO_REQUIRED"));
    assert_eq!(list_threads(f.db.pool()).await.unwrap().len(), 1);
    assert!(ask(f.db.pool(), &m, None, &Scope::all(), "   ").await.is_err());
    // Delete
    delete_thread(f.db.pool(), &turn.thread.id).await.unwrap();
    assert!(list_threads(f.db.pool()).await.unwrap().is_empty());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_message_sources").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn stricken_words_never_reach_the_prompt() {
    let f = fx().await;
    let text = "the secret code is Zanzibar";
    let id = line_at(&f, "m1", 50, text).await;
    let b = text.find("Zanzibar").unwrap();
    let s = text[..b].encode_utf16().count();
    rd::strike_words(f.db.pool(), &f.env, &WordTarget { meeting_id: "m1".into(), transcript_id: id, start: s, end: s + 8, expected_text: None, whole_line: false }, None)
        .await
        .unwrap();
    let m = Mock::new("No idea.");
    ask(f.db.pool(), &m, None, &Scope::meeting("m1"), "What is the secret code?").await.unwrap();
    let p = m.last_prompt();
    assert!(!p.contains("Zanzibar") && !p.contains("strickenid"), "{}", p);
    assert!(p.contains("[stricken from the record]"), "{}", p);
}

// ═══════════════════════════════════════════════════════════════════════════
// Purge
// ═══════════════════════════════════════════════════════════════════════════

async fn counts(pool: &Pool<Sqlite>, thread: &str) -> (i64, i64) {
    let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE thread_id = ? AND role = 'user'").bind(thread).fetch_one(pool).await.unwrap();
    let answers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE thread_id = ? AND role = 'assistant'").bind(thread).fetch_one(pool).await.unwrap();
    (users, answers)
}

#[tokio::test]
async fn transcript_edits_and_meeting_delete_remove_the_answers_that_used_it() {
    let f = fx().await;
    let m = Mock::new("ATP [1].");
    let bio = ask(f.db.pool(), &m, None, &Scope::meeting("m1"), "what makes ATP?").await.unwrap();
    let acme = ask(f.db.pool(), &Mock::new("Friday [1]."), None, &Scope::notebook("Acme"), "when do we launch?").await.unwrap();
    // Answers drew on the recording even when the text doesn't cite it
    let uncited = ask(f.db.pool(), &Mock::new("Not sure."), Some(&bio.thread.id), &Scope::all(), "and osmosis?").await.unwrap();
    assert!(uncited.assistant.citations.iter().any(|c| c.meeting_id == "m1"));
    // A backup made now
    rd::wal_checkpoint_truncate(f.db.pool()).await.unwrap();
    let bdir = f.env.app_data_dir.join("backups");
    std::fs::create_dir_all(&bdir).unwrap();
    let copy = bdir.join("copy.db");
    std::fs::copy(&f.db_path, &copy).unwrap();
    // Striking words in m1: the preview says so; both answers of the Bio thread go, the questions stay, the thread is flagged
    let text = "mitochondria make ATP for the cell";
    let id: i64 = sqlx::query_scalar("SELECT id FROM transcripts WHERE meeting_id = 'm1' AND text = ?").bind(text).fetch_one(f.db.pool()).await.unwrap();
    let target = WordTarget { meeting_id: "m1".into(), transcript_id: id, start: 0, end: 12, expected_text: None, whole_line: false };
    let preview = rd::preview_words(f.db.pool(), &f.env, &target).await.unwrap();
    assert!(preview.iter().any(|i| i.contains("2 CHAT answers that drew on this recording")), "{:?}", preview);
    let out = rd::strike_words(f.db.pool(), &f.env, &target, None).await.unwrap();
    assert_eq!(counts(f.db.pool(), &bio.thread.id).await, (2, 0));
    assert_eq!(counts(f.db.pool(), &acme.thread.id).await, (1, 1), "the other thread is untouched");
    let t = get_thread(f.db.pool(), &bio.thread.id).await.unwrap().thread;
    assert_eq!(t.flag.as_deref(), Some(FLAG_REMOVED));
    assert!(get_thread(f.db.pool(), &acme.thread.id).await.unwrap().thread.flag.is_none());
    assert_eq!((out.backups_purged, out.backups_deleted), (1, 0), "{:?}", out.warnings);
    let mut c = sqlx::sqlite::SqliteConnectOptions::new().filename(&copy).connect().await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE role = 'assistant' AND thread_id = ?").bind(&bio.thread.id).fetch_one(&mut c).await.unwrap();
    assert_eq!(n, 0, "purged from the backup too");
    // An older backup without the chat tables is left alone
    sqlx::query("DROP TABLE chat_message_sources").execute(&mut c).await.unwrap();
    assert_eq!(purge_for_meeting(&mut c, "m2").await.unwrap(), 0);
    assert!(rd::redact_ai_outputs(&mut c, "m2", "launch Friday", "[stricken from the record]").await.is_ok());
    c.close().await.unwrap();
    // Deleting m2 removes the Acme answer and flags that thread
    f.db.delete_meeting("m2").await.unwrap();
    assert_eq!(counts(f.db.pool(), &acme.thread.id).await, (1, 0));
    assert_eq!(get_thread(f.db.pool(), &acme.thread.id).await.unwrap().thread.flag.as_deref(), Some(FLAG_REMOVED));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_message_sources WHERE meeting_id = 'm2'").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(n, 0);
    // A thread scoped to a deleted recording is flagged even without answers
    let scoped = ask(f.db.pool(), &Mock::new("Floss [1]."), None, &Scope::meeting("m3"), "what did the dentist say?").await.unwrap();
    f.db.delete_meeting("m3").await.unwrap();
    assert_eq!(get_thread(f.db.pool(), &scoped.thread.id).await.unwrap().thread.flag.as_deref(), Some(FLAG_REMOVED));
}

// ═══════════════════════════════════════════════════════════════════════════
// Ask: a question about the recording you are in (chat/live.rs)
// ═══════════════════════════════════════════════════════════════════════════

/// m2 (Acme sync) as a call that is still being recorded, 20 minutes in:
/// an event with two people, lines on both sides, a typed note, and an
/// earlier recording with the same Notebook and its notes.
async fn live_fx() -> Fx {
    let f = fx().await;
    let pool = f.db.pool();
    f.db.add_meeting_attendee("m2", "Ana Ruiz", "ana@acme.example", Some("Acme Corp"), "organizer").await.unwrap();
    f.db.add_meeting_attendee("m2", "Casey", "casey@gmail.com", None, "attendee").await.unwrap();
    sqlx::query(
        "INSERT INTO people (id, email, name, company, company_domain, notes, is_self) VALUES \
         ('ana@acme.example', 'ana@acme.example', 'Ana Ruiz', 'Acme Corp', 'acme.example', 'Runs procurement. Prefers email.', 0), \
         ('casey@gmail.com', 'casey@gmail.com', 'Casey', NULL, 'gmail.com', NULL, 1)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO meeting_details (meeting_id, event_title, location, notes) VALUES ('m2', 'Acme renewal call', 'Zoom', 'Agenda: renewal price, rollout date')")
        .execute(pool)
        .await
        .unwrap();
    // Said long ago in this recording, on the other side
    let old = line_at(&f, "m2", 120, "the renewal price would go up eight percent in January").await;
    f.db.set_transcript_source(old, crate::audio_source::SYSTEM).await.unwrap();
    // The last minutes: the other side, then the user, then the other side again
    for (secs, text, side) in [
        (1080, "so the rollout would start with the Denver office", crate::audio_source::SYSTEM),
        (1090, "and we would need your security review first", crate::audio_source::SYSTEM),
        (1110, "we can have the review done in two weeks", crate::audio_source::MIC),
        (1190, "Casey, can you send the SOC two report by Friday", crate::audio_source::SYSTEM),
    ] {
        let id = line_at(&f, "m2", secs, text).await;
        f.db.set_transcript_source(id, side).await.unwrap();
    }
    // A line whose side couldn't be told
    line_at(&f, "m2", 1195, "sounds good").await;
    crate::markers::add_at_offset(pool, "m2", 1_112_000, Some(crate::markers::NOTE_KIND), Some("review in 2 wks: ask Dana")).await.unwrap();
    // An earlier Acme recording, with notes
    f.db.create_meeting("m0", "Acme kickoff").await.unwrap();
    f.db.set_meeting_notebook("m0", Some("Acme")).await.unwrap();
    let start = start_of(&f, "m2").await;
    sqlx::query("UPDATE meetings SET started_at = ?, ended_at = ? WHERE id = 'm0'")
        .bind((start - chrono::Duration::days(7)).to_rfc3339())
        .bind((start - chrono::Duration::days(7) + chrono::Duration::minutes(30)).to_rfc3339())
        .execute(pool)
        .await
        .unwrap();
    f.db.save_meeting_notes("n0", "m0", Some("Acme wants a pilot in Denver before a full rollout."), None, None, None, None, Some("test")).await.unwrap();
    f
}

/// Asked from the Ask panel while the recording runs.
const ASK: live::Mode = live::Mode { recording: true, related: true, glance: false };
/// Asked in the Chat tab, narrowed to the one recording, after it ended.
const ONE_RECORDING: live::Mode = live::Mode { recording: false, related: false, glance: false };

fn at_20_min(start: chrono::DateTime<chrono::Utc>) -> chrono::DateTime<chrono::Utc> {
    start + chrono::Duration::seconds(1200)
}

#[tokio::test]
async fn ask_is_given_the_recording_the_people_the_notes_and_the_last_minutes() {
    let f = live_fx().await;
    let mut conn = f.db.pool().acquire().await.unwrap();
    let now = at_20_min(start_of(&f, "m2").await);
    let m = Mock::new("They asked **you** for the SOC 2 report by Friday [7].");
    let ans = live::answer(&mut conn, &m, "m2", &[], "what did they just ask me for?", now, ASK).await.unwrap();
    let p = m.last_prompt();
    assert!(p.starts_with(&live::system_prompt()), "{p}");

    // The context: where this is, that it is still running, who is there
    assert!(p.contains("RECORDING: \"Acme sync\", a meeting, Notebook \"Acme\"."), "{p}");
    assert!(p.contains("It is STILL BEING RECORDED: 20 min in."), "{p}");
    assert!(p.contains("CALENDAR EVENT: Acme renewal call (Zoom)") && p.contains("EVENT NOTES: Agenda: renewal price, rollout date"), "{p}");
    assert!(p.contains("- Casey (the user)"), "a personal mail domain is not a company: {p}");
    assert!(p.contains("- Ana Ruiz (Acme Corp, organizer)\n"), "{p}");
    // Nothing in the app writes notes about a person, so none are sent
    assert!(!p.contains("Runs procurement"), "{p}");
    assert!(p.contains("A line tagged (mic) was picked up by the user's microphone"), "{p}");
    // A call between two people: the other side has a name
    assert!(p.contains("THE OTHER SIDE: the event has only the user and Ana Ruiz, so a (computer audio) line is most likely Ana Ruiz."), "{p}");
    // …as something worked out, and marked so in the answer
    assert!(p.contains("written \"Ana Ruiz (likely)\" the first time") && p.contains("not from a voice"), "{p}");
    assert!(p.contains("or as the context allows under THE OTHER SIDE, marked \"(likely)\""), "{p}");
    assert!(!p.contains("what the user noted about them"), "nothing writes notes about people: {p}");
    assert!(!p.contains("read at a glance"), "the card's rule is for the card: {p}");
    // No email address goes: the name and the company are what an answer needs
    assert!(!p.contains("ana@acme.example") && !p.contains("gmail.com"), "{p}");

    // The last minutes, in order, each side tagged, the newest last
    let last = p.split("THE LAST MINUTES OF THIS RECORDING, IN ORDER (the newest is last):\n").nth(1).unwrap();
    let last = last.split("\n\nQUESTION:").next().unwrap();
    let lines: Vec<&str> = last.lines().collect();
    assert!(lines[0].contains("18:00 — (computer audio) so the rollout would start with the Denver office and we would need your security review first"), "{last}");
    assert!(lines[1].contains("18:30 — (mic) we can have the review done in two weeks"), "{last}");
    assert!(lines[2].contains("19:50 — (computer audio) Casey, can you send the SOC two report by Friday"), "{last}");
    assert!(lines[3].ends_with("19:55 — sounds good"), "an untagged line has no tag: {last}");
    assert_eq!(lines.len(), 4);
    // What was said 18 minutes ago is not "the last minutes"
    assert!(!last.contains("eight percent"), "{last}");

    // The user's own note, with its time
    assert!(
        p.contains("THE USER'S OWN NOTES AND MARKS IN THIS RECORDING:\n[1] at 0:40 — ✎ Follow up: budget follow up\n[2] at 18:32 — ✍ Note: review in 2 wks: ask Dana"),
        "{p}"
    );
    // What came before this recording: the earlier Acme recording's notes
    assert!(p.contains("FROM EARLIER RECORDINGS:\n") && p.contains("Acme kickoff — ") && p.contains("Summary: Acme wants a pilot in Denver before a full rollout."), "{p}");
    assert!(p.ends_with("QUESTION: what did they just ask me for?"), "{p}");

    // Every source is numbered once, and a citation leads to its moment
    let ns: Vec<usize> = ans.passages.iter().map(|x| x.n).collect();
    assert_eq!(ns, (1..=ans.passages.len()).collect::<Vec<_>>());
    let cited = ans.passages.iter().find(|x| x.excerpt.contains("SOC two report")).unwrap();
    assert_eq!((cited.meeting_id.as_str(), cited.timestamp_ms, cited.source), ("m2", Some(1_190_000), "transcript"));
    // The answer says what it drew on
    assert_eq!(ans.scope_label, "Recording · Acme sync and earlier recordings");

    // Asked from the card over another app: the same sources, a shorter answer
    let card = Mock::new("ok");
    live::answer(&mut conn, &card, "m2", &[], "what did they just ask me for?", now, live::Mode { glance: true, ..ASK }).await.unwrap();
    let cp = card.last_prompt();
    assert!(cp.contains("read at a glance: at most three short lines or bullets"), "{cp}");
    assert!(cp.contains("THE LAST MINUTES OF THIS RECORDING") && cp.contains("FROM EARLIER RECORDINGS"), "{cp}");
    // With a third person in the event nobody is named for the other side
    f.db.add_meeting_attendee("m2", "Bo Chen", "bo@acme.example", Some("Acme Corp"), "attendee").await.unwrap();
    let three = Mock::new("ok");
    live::answer(&mut conn, &three, "m2", &[], "who said that?", now, ASK).await.unwrap();
    assert!(!three.last_prompt().contains("THE OTHER SIDE: the event has only"), "{}", three.last_prompt());
    // The rule itself: two people, one of them the user, and lines with a side
    let who = |name: &str, you: bool| live::Who { name: name.into(), company: None, role: None, you, met: 0 };
    let pair = [who("Casey", true), who("Ana Ruiz", false)];
    assert_eq!(live::other_side(&pair, true).map(|w| w.name.as_str()), Some("Ana Ruiz"));
    assert!(live::other_side(&pair, false).is_none(), "no side tags, no name");
    assert!(live::other_side(&[who("Bo", false), who("Ana Ruiz", false)], true).is_none(), "nobody is known to be the user");
    assert!(live::other_side(&[who("Casey", true), who(" ", false)], true).is_none());
    assert!(live::other_side(&[who("Casey", true)], true).is_none());
    assert!(live::other_side(&[who("Casey", true), who("ana@acme.example", false)], true).is_none(), "an address is not a name");

    // The same question in a Chat narrowed to this recording stays in it,
    // and a recording that isn't running isn't said to be
    let one = Mock::new("ok");
    let ans = live::answer(&mut conn, &one, "m2", &[], "what did they just ask me for?", now, ONE_RECORDING).await.unwrap();
    let p = one.last_prompt();
    assert!(!p.contains("FROM EARLIER RECORDINGS") && !p.contains("Acme kickoff") && !p.contains("pilot in Denver"), "{p}");
    assert!(!p.contains("STILL BEING RECORDED"), "{p}");
    assert!(ans.passages.iter().all(|x| x.meeting_id == "m2"));
    assert_eq!(ans.scope_label, "Recording · Acme sync");
}

#[tokio::test]
async fn ask_finds_what_was_said_earlier_and_never_what_was_struck() {
    let f = live_fx().await;
    let now = at_20_min(start_of(&f, "m2").await);
    // A word struck from a recent line
    let text = "the discount code is Zanzibar";
    let id = line_at(&f, "m2", 1150, text).await;
    let b = text.find("Zanzibar").unwrap();
    let s = text[..b].encode_utf16().count();
    rd::strike_words(f.db.pool(), &f.env, &WordTarget { meeting_id: "m2".into(), transcript_id: id, start: s, end: s + 8, expected_text: None, whole_line: false }, None)
        .await
        .unwrap();
    let mut conn = f.db.pool().acquire().await.unwrap();
    let m = Mock::new("Eight percent, in January [2].");
    live::answer(&mut conn, &m, "m2", &[], "what did they say about the renewal price?", now, ASK).await.unwrap();
    let p = m.last_prompt();
    // Matched to the question from 18 minutes earlier, with its side
    let earlier = p.split("EARLIER IN THIS RECORDING:\n").nth(1).expect(&p);
    let earlier = earlier.split("\n\n").next().unwrap();
    assert!(earlier.contains("2:00 — (computer audio) the renewal price would go up eight percent in January"), "{p}");
    assert_eq!(earlier.matches("eight percent").count(), 1, "a line is given once: {earlier}");
    // In time order, and other lines from before the last minutes come with it
    let times: Vec<&str> = earlier.lines().filter_map(|l| l.split("] ").nth(1)).map(|l| l.split(" — ").next().unwrap()).collect();
    assert_eq!(times, vec!["The saved notes' summary of this recording: Launch planning sync.", "0:20", "0:40", "2:00"], "{earlier}");
    // The struck word is nowhere; the placeholder stands in the line
    assert!(!p.contains("Zanzibar") && !p.contains("strickenid"), "{p}");
    assert!(p.contains("the discount code is [stricken from the record]"), "{p}");
    // A recording that doesn't exist is an error, and nothing is asked
    let none = Mock::new("x");
    assert!(live::answer(&mut conn, &none, "nope", &[], "anything?", now, ASK).await.is_err());
    assert!(live::answer(&mut conn, &none, "m2", &[], "   ", now, ASK).await.is_err());
    assert!(none.calls.lock().is_empty());
    // What the notes made from this recording say is found by the question's
    // words: an action item, not only the summary
    let m = Mock::new("Bo sends it [1].");
    live::answer(&mut conn, &m, "m2", &[], "who sends the budget?", now, ONE_RECORDING).await.unwrap();
    assert!(m.last_prompt().contains("Action item: Send the budget (Bo)"), "{}", m.last_prompt());
}

#[tokio::test]
async fn ask_fits_a_small_model_and_keeps_the_newest_line() {
    let f = live_fx().await;
    let start = start_of(&f, "m2").await;
    // A long, busy stretch right before now
    for i in 0..60 {
        line_at(&f, "m2", 900 + i * 4, &format!("point number {i} about the rollout plan and the security review timeline")).await;
    }
    line_at(&f, "m2", 1199, "the very last thing said").await;
    let mut conn = f.db.pool().acquire().await.unwrap();
    let mut m = Mock::new("ok");
    m.ctx = 4_096; // Apple's on-device model
    live::answer(&mut conn, &m, "m2", &[], "what did I miss?", at_20_min(start), ASK).await.unwrap();
    let p = m.last_prompt();
    // 4,096 tokens less the answer (614) and the reserve, at 3.2 characters a token
    assert!(p.chars().count() < 11_000, "the prompt fits the window: {} chars", p.chars().count());
    // In a small window fewer than six minutes fit as "the last minutes";
    // what was said between is still found by a question's words
    let q = Mock::new("ok");
    let mut q = q;
    q.ctx = 4_096;
    live::answer(&mut conn, &q, "m2", &[], "what was point number 3 about?", at_20_min(start), ASK).await.unwrap();
    assert!(q.last_prompt().contains("point number 3 about"), "{}", q.last_prompt());
    assert!(p.contains("the very last thing said"), "the newest line is always there: {p}");
    assert!(p.contains("STILL BEING RECORDED") && p.contains("PEOPLE IN IT:"), "{p}");
    // With room, all of the last six minutes is given
    let big = Mock::new("ok");
    live::answer(&mut conn, &big, "m2", &[], "what did I miss?", at_20_min(start), ASK).await.unwrap();
    assert!(big.last_prompt().contains("point number 30") && big.last_prompt().contains("point number 59"));
}

#[tokio::test]
async fn an_ask_is_kept_as_a_chat_about_that_recording_and_goes_with_its_text() {
    let f = live_fx().await;
    let pool = f.db.pool();
    let m = Mock::new("Denver first [3], as agreed at kickoff [2].");
    let turn = ask_recording(pool, &m, None, "m2", "where does the rollout start?", ASK).await.unwrap();
    assert_eq!(turn.thread.scope, Scope::meeting("m2"));
    assert_eq!(turn.assistant.scope_label.as_deref(), Some("Recording · Acme sync and earlier recordings"));
    assert!(m.last_prompt().starts_with(&live::system_prompt()));
    // The next question in the same thread carries the first exchange
    let m2 = Mock::new("In two weeks [4].");
    let t2 = ask_recording(pool, &m2, Some(&turn.thread.id), "m2", "and the review?", ASK).await.unwrap();
    assert_eq!(t2.thread.id, turn.thread.id);
    assert_eq!(m2.calls.lock()[0].len(), 4, "system, question, answer, question");
    // It drew on this recording and on the earlier one: both are its sources
    let sources: Vec<String> = sqlx::query_scalar("SELECT meeting_id FROM chat_message_sources WHERE message_id = ? ORDER BY meeting_id")
        .bind(&turn.assistant.id)
        .fetch_all(pool)
        .await
        .unwrap();
    assert_eq!(sources, vec!["m0", "m2"]);
    // What Ask knows, without asking
    let mut conn = pool.acquire().await.unwrap();
    let k = live::knows(&mut conn, "m2", true).await.unwrap();
    assert_eq!((k.recording, k.lines, k.tagged, k.notes, k.related), (true, 8, 5, 2, 1));
    assert!(k.event_details && k.saved_notes);
    assert!(!live::knows(&mut conn, "m2", false).await.unwrap().recording, "running is what the capture engine says");
    // An answer made before anything was said is still this recording's
    f.db.create_meeting("m9", "Just started").await.unwrap();
    let early = ask_recording(pool, &Mock::new("Nothing yet."), None, "m9", "who is here?", ASK).await.unwrap();
    let own: Vec<String> = sqlx::query_scalar("SELECT meeting_id FROM chat_message_sources WHERE message_id = ?").bind(&early.assistant.id).fetch_all(pool).await.unwrap();
    assert_eq!(own, vec!["m9"]);
    assert_eq!(k.people.iter().map(|w| (w.name.as_str(), w.company.as_deref(), w.you)).collect::<Vec<_>>(), vec![("Casey", None, true), ("Ana Ruiz", Some("Acme Corp"), false)]);
    assert_eq!(k.event.as_deref(), Some("Acme renewal call"));
    drop(conn);
    // Deleting the earlier recording removes the answers that drew on it; the questions stay
    f.db.delete_meeting("m0").await.unwrap();
    assert_eq!(counts(pool, &turn.thread.id).await, (2, 0));
    assert_eq!(get_thread(pool, &turn.thread.id).await.unwrap().thread.flag.as_deref(), Some(FLAG_REMOVED));
}
