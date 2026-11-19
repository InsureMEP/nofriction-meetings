//! Asks a real question of the real AI on this Mac and shows the answer
//! arriving, without launching noFriction or touching its data:
//!
//!     cargo run --example stream_answer
//!
//! It makes a throwaway library in the temp folder (one made-up meeting),
//! asks Chat a question about it the way the app does (`chat::ask` with the
//! streaming completer and the window's sink), and prints when the first
//! words came, how often the answer grew and when it was whole. With no AI
//! set up in that empty library the app's own rule applies: Apple's
//! on-device model, if this Mac has it. Nothing leaves the Mac.
//!
//! It checks what unit tests can't (they never call a real model): that the
//! on-device model's words reach the sink while it is still writing, and
//! that the stored answer is the one that was shown.

use nofriction_meetings_lib::chat::{self, Scope};
use nofriction_meetings_lib::database::DatabaseManager;
use nofriction_meetings_lib::study::StreamingCompleter;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LINES: &[&str] = &[
    "Thanks for joining. Three things today: the pilot, the budget and the launch date.",
    "On the pilot, we agreed it starts in March with two sites, Denver and Austin.",
    "Maya will own the Denver site and Jordan takes Austin.",
    "The budget stays at ten thousand dollars for the pilot, reviewed the week after it starts.",
    "For the launch date we could not agree. Marketing wants June, engineering says September.",
    "Let's decide the launch date at the next meeting, once the pilot numbers are in.",
    "Action items: Maya sends the site checklist by Friday, Jordan books the Austin venue.",
];

#[tokio::main]
async fn main() {
    let (available, reason) = nofriction_meetings_lib::store::apple_model_availability();
    if !available {
        println!("SKIP: Apple's on-device model is not available here ({}), and this example sets up no other AI.", reason);
        return;
    }
    let dir = std::env::temp_dir().join(format!("nf-stream-answer-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = DatabaseManager::new(&dir.join("nofriction_meetings.db")).await.expect("a database");
    db.run_migrations().await.expect("its tables");
    db.create_meeting("m1", "Pilot planning").await.unwrap();
    let t0 = chrono::Utc::now() - chrono::Duration::hours(1);
    for (i, line) in LINES.iter().enumerate() {
        db.add_transcript_at("m1", line, None, true, 0.9, t0 + chrono::Duration::seconds(20 * i as i64)).await.unwrap();
    }

    let mut failed = 0;
    for question in ["What was agreed about the pilot, and what is still open?", "Who owns which site?"] {
        let start = Instant::now();
        let seen: Mutex<Vec<(f32, String)>> = Mutex::new(Vec::new());
        // The window's sink, at the window's pace
        let turn = {
            let watch = chat::PurgeWatch::of(db.pool()).await.expect("the library");
            let sink = chat::stream_sink(Duration::from_millis(40), watch, |text| seen.lock().unwrap().push((start.elapsed().as_secs_f32(), text)));
            let c = StreamingCompleter { on_text: &sink };
            chat::ask(db.pool(), &c, None, &Scope::all(), question).await
        };
        let total = start.elapsed().as_secs_f32();
        let seen = seen.into_inner().unwrap();
        println!("\nQ: {}", question);
        match turn {
            Ok(turn) => {
                let answer = turn.assistant.content;
                let first = seen.first().map(|s| s.0).unwrap_or(total);
                println!(
                    "   first words after {:.2}s, whole after {:.2}s, {} updates on the way, {} characters, {} sources",
                    first,
                    total,
                    seen.len(),
                    answer.chars().count(),
                    turn.assistant.citations.len()
                );
                for (at, text) in seen.iter().step_by((seen.len() / 5).max(1)) {
                    println!("   {:>5.2}s  {} chars", at, text.chars().count());
                }
                println!("A: {}", answer);
                let mut check = |ok: bool, what: &str| {
                    println!("   {} {}", if ok { "ok  " } else { "FAIL" }, what);
                    if !ok {
                        failed += 1;
                    }
                };
                check(seen.len() >= 3, "the answer arrived in pieces, not at once");
                check(first < total * 0.8, "the first words came well before the end");
                check(seen.windows(2).all(|w| w[1].1.len() >= w[0].1.len()), "it only ever grew");
                check(seen.iter().all(|(_, t)| answer.starts_with(t.trim_end())), "everything shown is the start of the stored answer");
            }
            Err(e) => {
                println!("   FAIL the question failed: {}", e);
                failed += 1;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if failed > 0 {
        println!("\n{} check(s) failed", failed);
        std::process::exit(1);
    }
    println!("\nAll checks passed.");
}
