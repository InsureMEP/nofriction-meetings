//! Checks the answer card of the quick-note panel (Ask from any app) on
//! this Mac without launching noFriction or touching its data:
//!
//!     cargo run --example ask_card
//!
//! The card appears for under a second and does NOT take the keyboard (it
//! is shown the way the "nothing is recording" notice is). It checks what
//! unit tests can't: the native panel grows downward to fit an answer, its
//! top edge stays where it was, a long answer is held to the limit, and
//! taking the answer away gives the one-line height back.
//! `RENDER=/path/to.png` saves the card's own drawing (made inside this
//! process: no screenshot, no permission). `KEYBOARD=1` also checks that
//! Return hands the keyboard back; that run takes the keyboard for about
//! a tenth of a second, so don't use it while someone is typing.

// The `objc` macros test a cfg this crate doesn't declare (as in lib.rs)
#![allow(unexpected_cfgs)]
#![allow(dead_code)]

#[cfg(target_os = "macos")]
#[path = "../src/quick_note/panel.rs"]
mod panel;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use objc::runtime::{Object, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::time::{Duration, Instant};
    type Id = *mut Object;
    const NIL: Id = std::ptr::null_mut();

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct R {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    unsafe impl objc::Encode for R {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    unsafe fn ns(s: &str) -> Id {
        let c = std::ffi::CString::new(s).unwrap();
        msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]
    }
    unsafe fn pump(app: Id, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        let mode = ns("kCFRunLoopDefaultMode");
        while Instant::now() < end {
            let pool: Id = msg_send![class!(NSAutoreleasePool), new];
            let until: Id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.02f64];
            let event: Id = msg_send![app, nextEventMatchingMask: u64::MAX untilDate: until inMode: mode dequeue: YES];
            if !event.is_null() {
                let _: () = msg_send![app, sendEvent: event];
            }
            let _: () = msg_send![pool, drain];
        }
    }

    let mut checks: Vec<(&str, bool, String)> = Vec::new();
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        // An accessory app: no Dock icon, never in front
        let _: BOOL = msg_send![app, setActivationPolicy: 1isize];
        panel::set_handlers(panel::Handlers { keep: |_| {}, changed: |_| {}, closed: || {} });

        // Up without the keyboard, the way a notice is
        panel::show_notice("Ask about what's being said", "Acme sync · Return asks your AI");
        pump(app, 120);
        let before = panel::probe().expect("the panel exists");
        let top_before = before.frame.1 + before.frame.3;
        // Typing goes to the app in front: this app isn't active and nothing is being edited
        checks.push((
            "the card is up and did not take the keyboard",
            before.visible && !before.app_active && !before.editing,
            format!("visible {} app active {} editing {}", before.visible, before.app_active, before.editing),
        ));
        checks.push(("it starts one line tall", (before.frame.3 - panel::HEIGHT).abs() < 0.5, format!("{}", before.frame.3)));

        let answer = "They asked you for the SOC 2 report by Friday.\n• Rollout starts with the Denver office\n• Ana needs your security review first\n• You said the review takes two weeks";
        panel::show_answer(answer);
        pump(app, 150);
        let after = panel::probe().expect("the panel exists");
        let (text, height) = panel::answer_probe().expect("the answer label exists");
        checks.push(("the answer is on the card", text == answer, format!("{} characters", text.chars().count())));
        checks.push(("the card grew to fit it", height > panel::HEIGHT + 40.0 && height < panel::HEIGHT + 200.0, format!("{height}")));
        checks.push(("its top edge stayed where it was", ((after.frame.1 + after.frame.3) - top_before).abs() < 0.5, format!("{} -> {}", top_before, after.frame.1 + after.frame.3)));
        checks.push(("it still has not taken the keyboard", !after.app_active && !after.editing, format!("app active {} editing {}", after.app_active, after.editing)));
        checks.push(("the caption is still there", after.caption == before.caption, after.caption.clone()));

        if let Ok(path) = std::env::var("RENDER") {
            let windows: Id = msg_send![app, windows];
            let count: usize = msg_send![windows, count];
            let mut ok = false;
            for i in 0..count {
                let window: Id = msg_send![windows, objectAtIndex: i];
                let visible: BOOL = msg_send![window, isVisible];
                if visible == NO {
                    continue;
                }
                let view: Id = msg_send![window, contentView];
                let bounds: R = msg_send![view, bounds];
                let rep: Id = msg_send![view, bitmapImageRepForCachingDisplayInRect: bounds];
                if rep.is_null() {
                    continue;
                }
                let _: () = msg_send![view, cacheDisplayInRect: bounds toBitmapImageRep: rep];
                let props: Id = msg_send![class!(NSDictionary), dictionary];
                let data: Id = msg_send![rep, representationUsingType: 4usize properties: props];
                if !data.is_null() {
                    let wrote: BOOL = msg_send![data, writeToFile: ns(&path) atomically: YES];
                    ok = wrote != NO;
                }
            }
            checks.push(("the card's drawing was saved (RENDER)", ok, path));
        }

        // A very long answer is held to the limit
        panel::show_answer(&"A long line of answer text that wraps. ".repeat(80));
        pump(app, 120);
        let (_, tall) = panel::answer_probe().unwrap();
        checks.push(("a long answer is held to the limit", tall <= panel::HEIGHT + 320.0, format!("{tall}")));
        let long = panel::probe().unwrap();
        checks.push(("and the top edge still hasn't moved", ((long.frame.1 + long.frame.3) - top_before).abs() < 0.5, format!("{}", long.frame.1 + long.frame.3)));

        // Taking the answer away gives the one-line height back
        panel::show_answer("");
        pump(app, 80);
        let (gone, short) = panel::answer_probe().unwrap();
        checks.push(("without an answer it is one line again", gone.is_empty() && (short - panel::HEIGHT).abs() < 0.5, format!("{short}")));

        // KEYBOARD=1 (opt-in: it takes the keyboard for about a tenth of a
        // second): Return hands the keyboard back and the card stays up
        if std::env::var("KEYBOARD").is_ok() {
            panel::show_ask("Acme sync · Return asks your AI", false);
            pump(app, 60);
            let asking = panel::probe().unwrap();
            panel::clear();
            panel::set_placeholder("What did I miss?");
            panel::set_busy(true);
            panel::release_keyboard();
            pump(app, 80);
            let waiting = panel::probe().unwrap();
            checks.push(("the card took the keyboard for the question", asking.visible && asking.key, format!("key {}", asking.key)));
            checks.push((
                "after Return it is still up, pinned, and no longer has the keyboard",
                waiting.visible && !waiting.key && !waiting.editing && panel::pinned() && !waiting.app_active,
                format!("visible {} key {} editing {} pinned {} app active {}", waiting.visible, waiting.key, waiting.editing, panel::pinned(), waiting.app_active),
            ));
            checks.push(("and it says what was asked, with the field itself empty", panel::placeholder() == "What did I miss?" && panel::text().is_empty(), panel::placeholder()));
            panel::show_answer("Two lines of answer.\n• And a point");
            panel::set_busy(false);
            pump(app, 60);
            let answered = panel::probe().unwrap();
            checks.push(("the answer lands without taking the keyboard back", answered.visible && !answered.key && !answered.editing, format!("key {} editing {}", answered.key, answered.editing)));
        }

        // A closed card holds no answer
        panel::show_answer("Something said in the recording.");
        pump(app, 60);
        panel::hide();
        pump(app, 60);
        checks.push(("it closes", !panel::is_open(), String::new()));
        let (left, closed_height) = panel::answer_probe().unwrap();
        checks.push(("and a closed card holds no answer", left.is_empty() && (closed_height - panel::HEIGHT).abs() < 0.5, format!("{closed_height}")));
        // A note put aside for a question comes back as it was
        panel::set_text("half a note");
        checks.push(("a note put aside can be put back", panel::text() == "half a note", panel::text()));
        panel::clear();
        let _ = NIL;
    }
    let failed = checks.iter().filter(|c| !c.1).count();
    for (name, ok, detail) in &checks {
        println!("{} {}{}", if *ok { "PASS" } else { "FAIL" }, name, if detail.is_empty() { String::new() } else { format!(" ({detail})") });
    }
    println!("{} checks, {} failed", checks.len(), failed);
    std::process::exit(if failed == 0 { 0 } else { 1 });
}
