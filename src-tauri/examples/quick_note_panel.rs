//! Checks the quick-note panel's window on this Mac without launching
//! noFriction or touching its data:
//!
//!     cargo run --example quick_note_panel
//!
//! The panel appears for a few seconds while the checks run, then the
//! result prints as JSON and the exit code says whether every check passed
//! (0 passed, 1 failed, 2 interrupted: someone clicked in another app
//! during the run, which closes the panel; run it again). Don't use the
//! Mac while it runs: for those seconds the panel has the keyboard.
//! It proves what unit tests can't: the panel takes the keyboard while
//! another app stays in front, Return / Esc / the editing keys reach the
//! field, ⌘Q typed into it never reaches the app's own menu, and it comes
//! up over a hidden app without bringing the app back.
//!
//! Keys are sent inside this process (`-[NSApplication sendEvent:]`), which
//! needs no permission. When the terminal is already allowed to send key
//! presses (Accessibility), one F13 is also sent through the window server,
//! which proves a key from the real keyboard arrives; otherwise that check
//! is skipped and says so. `HOLD_MS=8000` keeps the panel up that long
//! first so a person can type into it and see.
//! `RENDER=/path/to.png` saves the panel's own drawing (made inside this
//! process, so no permission is needed; the blurred background isn't in
//! it). `CLIPBOARD=1` also checks ⌘X and ⌘V, which use the real clipboard.
//! It takes no screenshot: asking macOS for one puts a Screen Recording
//! dialog on the screen, and that dialog takes the keyboard.

// The `objc` macros test a cfg this crate doesn't declare (as in lib.rs)
#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../src/quick_note/panel.rs"]
mod panel;

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(mac::run());
}

#[cfg(target_os = "macos")]
mod mac {
    use super::panel;
    use objc::declare::ClassDecl;
    use objc::runtime::{Object, Sel, BOOL, NO, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_void};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    type Id = *mut Object;
    const NIL: Id = std::ptr::null_mut();

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Point {
        x: f64,
        y: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Size {
        width: f64,
        height: f64,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Rect {
        origin: Point,
        size: Size,
    }
    unsafe impl objc::Encode for Point {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
        }
    }
    unsafe impl objc::Encode for Rect {
        fn encode() -> objc::Encoding {
            unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
        }
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// Whether this process may send key presses, without asking for it
        fn CGPreflightPostEventAccess() -> bool;
        fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
        fn CGEventPost(tap: u32, event: *mut c_void);
        /// What the window server knows about windows (no permission needed)
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(obj: *const c_void);
    }

    /// F13: a key no app is likely to act on, in case it lands elsewhere
    const F13: u16 = 105;
    /// F13 presses that arrived in this app's event queue
    static F13_SEEN: AtomicU32 = AtomicU32::new(0);

    /// ⌘Q reached the app's menu
    static QUIT_REACHED: AtomicU32 = AtomicU32::new(0);
    /// AppKit announced that the app became active
    static ACTIVATIONS: AtomicU32 = AtomicU32::new(0);

    extern "C" fn quit_reached(_this: &Object, _sel: Sel, _sender: Id) {
        QUIT_REACHED.fetch_add(1, Ordering::SeqCst);
    }

    extern "C" fn became_active(_this: &Object, _sel: Sel, _notification: Id) {
        ACTIVATIONS.fetch_add(1, Ordering::SeqCst);
    }

    /// Stands in for the app: the target of its Quit item, and the observer
    /// noFriction has on "the app became active" (notifications.rs).
    unsafe fn watcher() -> Id {
        let mut decl = ClassDecl::new("QNWatcher", class!(NSObject)).expect("declared once");
        decl.add_method(sel!(quitReached:), quit_reached as extern "C" fn(&Object, Sel, Id));
        decl.add_method(sel!(becameActive:), became_active as extern "C" fn(&Object, Sel, Id));
        let cls = decl.register();
        msg_send![cls, new]
    }

    static KEPT: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static CHANGED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static CLOSED: Mutex<u32> = Mutex::new(0);

    unsafe fn ns_string(s: &str) -> Id {
        let obj: Id = msg_send![class!(NSString), alloc];
        let obj: Id = msg_send![obj, initWithBytes: s.as_ptr() length: s.len() encoding: 4usize];
        msg_send![obj, autorelease]
    }

    unsafe fn rust_string(ns: Id) -> String {
        if ns.is_null() {
            return String::new();
        }
        let utf8: *const c_char = msg_send![ns, UTF8String];
        if utf8.is_null() {
            return String::new();
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    }

    /// Run the event loop for `ms`.
    unsafe fn pump(app: Id, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        let mode = ns_string("kCFRunLoopDefaultMode");
        while Instant::now() < end {
            let pool: Id = msg_send![class!(NSAutoreleasePool), new];
            let until: Id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.02f64];
            let event: Id = msg_send![app,
                nextEventMatchingMask: u64::MAX
                untilDate: until
                inMode: mode
                dequeue: YES];
            if !event.is_null() {
                let kind: usize = msg_send![event, type];
                if kind == 10 {
                    let code: u16 = msg_send![event, keyCode];
                    if code == F13 {
                        F13_SEEN.fetch_add(1, Ordering::SeqCst);
                    }
                }
                let _: () = msg_send![app, sendEvent: event];
            }
            let _: () = msg_send![pool, drain];
        }
    }

    /// The names of the app that gets key presses and the app that owns the menu bar.
    unsafe fn front() -> (String, String) {
        let ws: Id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let key_app: Id = msg_send![ws, frontmostApplication];
        let menu_app: Id = msg_send![ws, menuBarOwningApplication];
        let key_name: Id = if key_app.is_null() { NIL } else { msg_send![key_app, localizedName] };
        let menu_name: Id = if menu_app.is_null() { NIL } else { msg_send![menu_app, localizedName] };
        (rust_string(key_name), rust_string(menu_name))
    }

    /// A key press delivered to this app the way the window server's would be.
    unsafe fn press(app: Id, chars: &str, plain: &str, key_code: u16, flags: usize) {
        let window: Id = msg_send![app, keyWindow];
        let number: isize = if window.is_null() { 0 } else { msg_send![window, windowNumber] };
        for kind in [10usize, 11usize] {
            // NSEventTypeKeyDown, NSEventTypeKeyUp
            let event: Id = msg_send![class!(NSEvent),
                keyEventWithType: kind
                location: Point { x: 0.0, y: 0.0 }
                modifierFlags: flags
                timestamp: 0.0f64
                windowNumber: number
                context: NIL
                characters: ns_string(chars)
                charactersIgnoringModifiers: ns_string(plain)
                isARepeat: NO
                keyCode: key_code];
            if !event.is_null() {
                let _: () = msg_send![app, sendEvent: event];
            }
        }
        pump(app, 40);
    }

    unsafe fn type_text(app: Id, text: &str) {
        for ch in text.chars() {
            let s = ch.to_string();
            press(app, &s, &s, 0, 0);
        }
    }

    const COMMAND: usize = 1 << 20;
    /// NSEventModifierFlagNumericPad | NSEventModifierFlagFunction, as an arrow key carries
    const ARROW: usize = (1 << 21) | (1 << 23);

    pub fn run() -> i32 {
        let mut checks: Vec<(&str, bool, String)> = Vec::new();
        unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            // Accessory: no Dock icon, like a helper. noFriction itself is a
            // regular app; the panel must not bring either kind forward.
            let policy: isize = if std::env::var("POLICY").as_deref() == Ok("regular") { 0 } else { 1 };
            let _: BOOL = msg_send![app, setActivationPolicy: policy];

            // A Quit item on ⌘Q, as the app has. It only records that it was reached.
            let watcher = watcher();
            let bar: Id = msg_send![class!(NSMenu), new];
            let holder: Id = msg_send![class!(NSMenuItem), new];
            let _: () = msg_send![bar, addItem: holder];
            let menu: Id = msg_send![class!(NSMenu), new];
            let quit: Id = msg_send![class!(NSMenuItem), alloc];
            let quit: Id = msg_send![quit,
                initWithTitle: ns_string("Quit")
                action: sel!(quitReached:)
                keyEquivalent: ns_string("q")];
            let _: () = msg_send![quit, setTarget: watcher];
            let _: () = msg_send![menu, addItem: quit];
            let _: () = msg_send![holder, setSubmenu: menu];
            let _: () = msg_send![app, setMainMenu: bar];
            let _: () = msg_send![app, finishLaunching];
            pump(app, 150);
            let center: Id = msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![center,
                addObserver: watcher
                selector: sel!(becameActive:)
                name: ns_string("NSApplicationDidBecomeActiveNotification")
                object: NIL];

            panel::set_handlers(panel::Handlers {
                keep: |text| KEPT.lock().unwrap().push(text),
                changed: |text| CHANGED.lock().unwrap().push(text),
                closed: || *CLOSED.lock().unwrap() += 1,
            });

            let before = front();

            // 1. The field, over whatever is in front
            panel::show("BIO 101 · Return keeps it at this moment", false);
            pump(app, 350);
            if let Some(ms) = std::env::var("HOLD_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
                pump(app, ms);
                let typed = panel::probe().map(|p| p.text).unwrap_or_default();
                checks.push(("a person typed into it (HOLD_MS)", !typed.is_empty(), format!("{} characters", typed.chars().count())));
                panel::clear();
            }
            let open = panel::probe().expect("the panel exists");
            let during = front();
            checks.push(("the panel is on screen", open.visible, format!("{:?}", open.frame)));
            checks.push(("the panel has the keyboard", open.key, String::new()));
            checks.push(("its app did not become active", !open.app_active, String::new()));
            checks.push(("the field is being edited", open.editing, String::new()));
            checks.push((
                "the menu bar still belongs to the app that was in front",
                during.1 == before.1,
                format!("{} → {}", before.1, during.1),
            ));
            checks.push(("the caption says which recording", open.caption.starts_with("BIO 101"), open.caption.clone()));

            let activations = ACTIVATIONS.load(Ordering::SeqCst);
            checks.push((
                "AppKit did not announce that the app became active",
                activations == 0,
                format!("{} announcements", activations),
            ));
            // What the window server has for the panel: 0 is "not shared",
            // which is what leaves a window out of screen captures
            let window: Id = msg_send![app, keyWindow];
            let mut sharing: Option<i32> = None;
            if !window.is_null() {
                let number: isize = msg_send![window, windowNumber];
                // kCGWindowListOptionIncludingWindow
                let list = CGWindowListCopyWindowInfo(1 << 3, number as u32) as Id;
                if !list.is_null() {
                    let count: usize = msg_send![list, count];
                    if count > 0 {
                        let info: Id = msg_send![list, objectAtIndex: 0usize];
                        let value: Id = msg_send![info, objectForKey: ns_string("kCGWindowSharingState")];
                        if !value.is_null() {
                            sharing = Some(msg_send![value, intValue]);
                        }
                    }
                    CFRelease(list as *const c_void);
                }
            }
            checks.push(("the window server has the panel as not shared", sharing == Some(0), format!("sharing state {:?}", sharing)));

            // A key pressed on the real keyboard, sent through the window server,
            // when this process is already allowed to send one (never asks)
            let real_keys = std::env::var("NO_REAL_KEYS").is_err() && CGPreflightPostEventAccess();
            if real_keys {
                for down in [true, false] {
                    let event = CGEventCreateKeyboardEvent(std::ptr::null(), F13, down);
                    if !event.is_null() {
                        // kCGHIDEventTap: where the keyboard's own events enter
                        CGEventPost(0, event);
                        CFRelease(event);
                    }
                }
                pump(app, 400);
                let seen = F13_SEEN.load(Ordering::SeqCst);
                checks.push(("a key from the window server arrives while another app is in front", seen == 1, format!("{} F13 presses arrived", seen)));
            } else {
                eprintln!("skipped: sending a real key press (this process isn't allowed to, or NO_REAL_KEYS is set)");
            }

            // 2. Typing reaches the field and is reported
            type_text(app, "ask about proof 2");
            let typed = panel::probe().unwrap();
            checks.push(("typed keys reach the field", typed.text == "ask about proof 2", typed.text.clone()));
            let reports = CHANGED.lock().unwrap().clone();
            checks.push((
                "typing is reported",
                reports.last().map(String::as_str) == Some("ask about proof 2"),
                format!("{} reports", reports.len()),
            ));

            // The panel's own drawing, saved as a PNG. Made inside this process, so
            // it needs no permission; the blurred background isn't in it.
            if let Ok(path) = std::env::var("RENDER") {
                let window: Id = msg_send![app, keyWindow];
                let view: Id = if window.is_null() { NIL } else { msg_send![window, contentView] };
                let mut ok = false;
                if !view.is_null() {
                    let bounds: Rect = msg_send![view, bounds];
                    let rep: Id = msg_send![view, bitmapImageRepForCachingDisplayInRect: bounds];
                    if !rep.is_null() {
                        let _: () = msg_send![view, cacheDisplayInRect: bounds toBitmapImageRep: rep];
                        let props: Id = msg_send![class!(NSDictionary), dictionary];
                        // NSBitmapImageFileTypePNG
                        let data: Id = msg_send![rep, representationUsingType: 4usize properties: props];
                        if !data.is_null() {
                            let wrote: BOOL = msg_send![data, writeToFile: ns_string(&path) atomically: YES];
                            ok = wrote != NO;
                        }
                    }
                }
                checks.push(("the panel's drawing was saved (RENDER)", ok, path));
            }

            // 3. ⌘Q stays in the panel (this process is still here); ⌘A selects the line
            press(app, "q", "q", 12, COMMAND);
            let quits = QUIT_REACHED.load(Ordering::SeqCst);
            checks.push(("⌘Q did not reach the app's menu", quits == 0, format!("reached {} times", quits)));
            press(app, "a", "a", 0, COMMAND);
            type_text(app, "proof 2?");
            let replaced = panel::probe().unwrap();
            checks.push(("⌘A selects the line (typing replaces it)", replaced.text == "proof 2?", replaced.text.clone()));
            // The field's own ⌘ keys: start of line, end of line, delete to the start
            press(app, "\u{F702}", "\u{F702}", 123, COMMAND | ARROW);
            type_text(app, ">");
            press(app, "\u{F703}", "\u{F703}", 124, COMMAND | ARROW);
            type_text(app, "<");
            let moved = panel::text();
            checks.push(("⌘← and ⌘→ move in the field", moved == ">proof 2?<", moved));
            press(app, "\u{7F}", "\u{7F}", 51, COMMAND);
            let deleted = panel::text();
            checks.push(("⌘⌫ deletes to the start of the line", deleted.is_empty(), deleted));
            // A Russian layout: the key types ф alone and a with ⌘ held
            type_text(app, "draft");
            press(app, "a", "ф", 0, COMMAND);
            type_text(app, "proof 2?");
            let cyrillic = panel::text();
            checks.push(("⌘A works on a layout that isn't Latin", cyrillic == "proof 2?", cyrillic));
            // ⌘X and ⌘V use the real clipboard, so only when asked: text on it
            // is put back afterwards, anything else on it is lost
            if std::env::var("CLIPBOARD").is_ok() {
                let pasteboard: Id = msg_send![class!(NSPasteboard), generalPasteboard];
                let kind = ns_string("public.utf8-plain-text");
                let saved: Id = msg_send![pasteboard, stringForType: kind];
                let saved: Id = if saved.is_null() { NIL } else { msg_send![saved, copy] };
                press(app, "a", "a", 0, COMMAND);
                press(app, "x", "x", 7, COMMAND);
                let cut = panel::probe().unwrap();
                checks.push(("⌘A then ⌘X empties the field", cut.text.is_empty(), cut.text.clone()));
                press(app, "v", "v", 9, COMMAND);
                let pasted = panel::probe().unwrap();
                checks.push(("⌘V pastes it back", pasted.text == "proof 2?", pasted.text.clone()));
                let _: isize = msg_send![pasteboard, clearContents];
                if !saved.is_null() {
                    let _: BOOL = msg_send![pasteboard, setString: saved forType: kind];
                    let _: () = msg_send![saved, release];
                }
            }

            // 4. Return reports the line; the panel stays until it is told the line was kept
            press(app, "\r", "\r", 36, 0);
            let kept = KEPT.lock().unwrap().clone();
            checks.push(("Return reports the line once", kept == vec!["proof 2?".to_string()], format!("{:?}", kept)));
            checks.push(("the panel waits to hear the line was kept", panel::is_open(), String::new()));
            // While the line is on its way, the field takes no keys
            panel::set_busy(true);
            type_text(app, "zz");
            press(app, "\r", "\r", 36, 0);
            let busy = panel::text();
            let kept_while_busy = KEPT.lock().unwrap().len();
            checks.push(("while a line is being kept, typing changes nothing", busy == "proof 2?", busy));
            checks.push(("and a second Return is not a second note", kept_while_busy == 1, format!("{} lines reported", kept_while_busy)));
            // It couldn't be kept: the field is back, with the cursor after the line
            panel::set_busy(false);
            type_text(app, "!");
            let again = panel::text();
            checks.push(("after a failure the field takes keys again, after the line", again == "proof 2?!", again));
            panel::set_caption("Couldn't keep that note: the disk is full", true);
            let failed = panel::probe().unwrap();
            checks.push(("a failure is shown and the text is still there", failed.caption.starts_with("Couldn't") && !failed.text.is_empty(), failed.caption));
            panel::clear();
            panel::hide();
            pump(app, 150);
            let after_keep = front();
            checks.push(("kept: the panel is gone", !panel::is_open(), String::new()));
            let closed = *CLOSED.lock().unwrap();
            checks.push(("closing is reported once", closed == 1, format!("{}", closed)));
            checks.push((
                "the app in front is still the one that was",
                after_keep.0 == before.0,
                format!("{} → {}", before.0, after_keep.0),
            ));
            if real_keys {
                let seen_before = F13_SEEN.load(Ordering::SeqCst);
                for down in [true, false] {
                    let event = CGEventCreateKeyboardEvent(std::ptr::null(), F13, down);
                    if !event.is_null() {
                        CGEventPost(0, event);
                        CFRelease(event);
                    }
                }
                pump(app, 400);
                let arrived = F13_SEEN.load(Ordering::SeqCst) - seen_before;
                checks.push(("with the panel gone, a key from the window server no longer comes here", arrived == 0, format!("{} F13 presses arrived", arrived)));
            }

            // 5. A draft survives Esc, and the cursor is after it
            panel::show("BIO 101", false);
            pump(app, 200);
            type_text(app, "half a thou");
            press(app, "\u{1b}", "\u{1b}", 53, 0);
            checks.push(("Esc closes it", !panel::is_open(), String::new()));
            panel::show("BIO 101", true);
            pump(app, 200);
            type_text(app, "ght");
            let draft = panel::probe().unwrap();
            checks.push(("the draft is still there and typing continues after it", draft.text == "half a thought", draft.text.clone()));
            press(app, "w", "w", 13, COMMAND);
            checks.push(("⌘W closes it", !panel::is_open(), String::new()));
            // Esc still closes it while a line is on its way
            panel::show("BIO 101", true);
            pump(app, 200);
            panel::set_busy(true);
            press(app, "\u{1b}", "\u{1b}", 53, 0);
            checks.push(("Esc closes it while a line is being kept", !panel::is_open(), String::new()));
            panel::set_busy(false);
            panel::clear();

            // 6. Nothing recording: a notice that leaves the keyboard alone
            panel::show_notice("Nothing is recording", "Start a recording, then press ⌃⌥⌘N to type a note.");
            pump(app, 500);
            let notice = panel::probe().unwrap();
            let during_notice = front();
            checks.push(("the notice is on screen", notice.visible, String::new()));
            checks.push(("the notice does not take the keyboard", !notice.key && !notice.editing, String::new()));
            checks.push((
                "the app in front is unchanged while the notice is up",
                during_notice.0 == before.0,
                format!("{} → {}", before.0, during_notice.0),
            ));
            if real_keys {
                let seen_before = F13_SEEN.load(Ordering::SeqCst);
                for down in [true, false] {
                    let event = CGEventCreateKeyboardEvent(std::ptr::null(), F13, down);
                    if !event.is_null() {
                        CGEventPost(0, event);
                        CFRelease(event);
                    }
                }
                pump(app, 400);
                let arrived = F13_SEEN.load(Ordering::SeqCst) - seen_before;
                checks.push(("a key pressed while the notice is up does not come here", arrived == 0, format!("{} F13 presses arrived", arrived)));
            }
            panel::hide();
            pump(app, 100);
            checks.push(("the notice is gone", !panel::is_open(), String::new()));

            // 7. The app hidden with ⌘H (people hide it and go to their slides)
            let _: () = msg_send![app, hide: NIL];
            pump(app, 300);
            let hidden: BOOL = msg_send![app, isHidden];
            panel::show("BIO 101", false);
            pump(app, 300);
            let over_hidden = panel::probe().unwrap();
            let still_hidden: BOOL = msg_send![app, isHidden];
            checks.push((
                "with its app hidden (⌘H) the panel appears, takes the keyboard, and the app stays hidden",
                hidden != NO && still_hidden != NO && over_hidden.visible && over_hidden.key,
                format!(
                    "app hidden before: {}, after: {}; panel visible: {}, key: {}",
                    hidden != NO,
                    still_hidden != NO,
                    over_hidden.visible,
                    over_hidden.key
                ),
            ));
            panel::hide();
            let _: () = msg_send![app, unhideWithoutActivation];
            pump(app, 150);

            let passed = checks.iter().all(|c| c.1);
            // Someone used the Mac during the run: a click in another app
            // closes the panel, as it should, and the checks after it mean nothing
            let interrupted = !passed && (after_keep.0 != before.0 || during_notice.0 != before.0 || front().0 != before.0);
            let status = if passed {
                "PASS"
            } else if interrupted {
                "INTERRUPTED: the app in front changed during the run. Leave the Mac alone for a few seconds and run it again."
            } else {
                "FAIL"
            };
            println!("{{");
            println!("  \"status\": \"{}\",", status);
            println!("  \"in front before\": {:?},", before);
            println!("  \"in front while the panel had the keyboard\": {:?},", during);
            println!("  \"checks\": [");
            for (i, (name, ok, detail)) in checks.iter().enumerate() {
                let comma = if i + 1 == checks.len() { "" } else { "," };
                println!("    {{\"ok\": {}, \"check\": {:?}, \"detail\": {:?}}}{}", ok, name, detail, comma);
            }
            println!("  ]");
            println!("}}");
            if passed {
                0
            } else if interrupted {
                2
            } else {
                1
            }
        }
    }
}
