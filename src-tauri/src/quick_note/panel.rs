//! The quick-note panel's window (AppKit). One text field in a floating
//! panel that takes the keyboard without bringing noFriction to the front,
//! so the app the user is in (slides, a call, a browser) stays where it is.
//! What the panel does with a line is in the parent module; this file only
//! shows the field and reports Return, typing and closing.
//!
//! Main thread only. It depends on nothing but `objc`, so
//! `examples/quick_note_panel.rs` can run it without the app or its data.
//!
//! Why a native panel and not a second web view: a panel is non-activating
//! only if it is created that way (`NSWindowStyleMaskNonactivatingPanel` at
//! init; setting it later leaves the window server out of step), it costs
//! no web process, and it is on screen before the next key is pressed.

use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, NO, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::cell::Cell;
use std::ffi::CStr;
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

type Id = *mut Object;
const NIL: Id = std::ptr::null_mut();

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

pub const WIDTH: f64 = 560.0;
pub const HEIGHT: f64 = 76.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Rect {
    origin: Point,
    size: Size,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Range {
    location: usize,
    length: usize,
}

unsafe impl objc::Encode for Point {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGPoint=dd}") }
    }
}
unsafe impl objc::Encode for Size {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGSize=dd}") }
    }
}
unsafe impl objc::Encode for Rect {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") }
    }
}
unsafe impl objc::Encode for Range {
    fn encode() -> objc::Encoding {
        unsafe { objc::Encoding::from_str("{_NSRange=QQ}") }
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
    Rect { origin: Point { x, y }, size: Size { width, height } }
}

// NSWindowStyleMask
const STYLE_BORDERLESS: usize = 0;
const STYLE_NONACTIVATING_PANEL: usize = 1 << 7;
// NSWindowCollectionBehavior: every Space, beside a full-screen app, not in ⌘`
const JOIN_ALL_SPACES: usize = 1 << 0;
const IGNORES_CYCLE: usize = 1 << 6;
const FULL_SCREEN_AUXILIARY: usize = 1 << 8;
/// NSStatusWindowLevel: above other apps' windows and floating panels
const LEVEL: isize = 25;
// NSEventModifierFlags
const SHIFT: usize = 1 << 17;
const CONTROL: usize = 1 << 18;
const OPTION: usize = 1 << 19;
const COMMAND: usize = 1 << 20;

/// What the panel reports. Called on the main thread.
pub struct Handlers {
    /// Return was pressed; the field's text
    pub keep: fn(String),
    /// The text changed
    pub changed: fn(String),
    /// The panel left the screen (Esc, a click elsewhere, the key again, or [`hide`])
    pub closed: fn(),
}

static HANDLERS: OnceLock<Handlers> = OnceLock::new();

/// Set once, before the first [`show`].
pub fn set_handlers(handlers: Handlers) {
    let _ = HANDLERS.set(handlers);
}

#[derive(Clone, Copy)]
struct Refs {
    panel: Id,
    field: Id,
    caption: Id,
    /// The answer to a question (Ask), under the field; empty when hidden
    answer: Id,
}

thread_local! {
    static REFS: Cell<Option<Refs>> = const { Cell::new(None) };
    static HIDING: Cell<bool> = const { Cell::new(false) };
    /// The panel stays on screen when it loses the keyboard: an answer is
    /// on its way or being read while the user is back in their call
    static PINNED: Cell<bool> = const { Cell::new(false) };
}

/// The answer card: how tall the text may get, and the space around it
const ANSWER_MAX_HEIGHT: f64 = 300.0;
const ANSWER_GAP: f64 = 12.0;
/// As many lines as fit in that height at the answer's size
const ANSWER_MAX_LINES: isize = 17;

/// Run `f` inside an autorelease pool.
fn pooled<R>(f: impl FnOnce() -> R) -> R {
    unsafe {
        let pool: Id = msg_send![class!(NSAutoreleasePool), new];
        let out = f();
        let _: () = msg_send![pool, drain];
        out
    }
}

/// An autoreleased NSString.
unsafe fn ns_string(s: &str) -> Id {
    const UTF8: usize = 4;
    let obj: Id = msg_send![class!(NSString), alloc];
    let obj: Id = msg_send![obj, initWithBytes: s.as_ptr() length: s.len() encoding: UTF8];
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

/// A callback from AppKit must never unwind into it. This stops that in a
/// development build only: the shipped build aborts on a panic
/// (`panic = "abort"` in Cargo.toml), which would end the app
/// mid-recording. So nothing a callback reaches may panic: no `unwrap` or
/// `expect` on anything that can fail, no indexing, no poisoning lock.
fn guarded(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

// ── The panel class: a borderless panel that may take the keyboard ──

extern "C" fn can_become_key(_this: &Object, _sel: Sel) -> BOOL {
    YES
}

extern "C" fn can_become_main(_this: &Object, _sel: Sel) -> BOOL {
    NO
}

/// What a ⌘ key does while the panel has the keyboard.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CommandKey {
    Paste,
    Copy,
    Cut,
    SelectAll,
    Undo,
    Redo,
    /// ⌘W
    Close,
    /// ⌘←, ⌘→, ⌘⌫ and the like: the field's own editing keys
    Edit,
    /// Anything else: it does nothing, and goes no further
    Swallow,
}

/// `typed` is what the key types with ⌘ held (`-[NSEvent characters]`: a
/// Latin letter even on a Russian or Greek layout) and `plain` what it
/// types without (`charactersIgnoringModifiers`). A letter counts if
/// either is it, so the editing keys work on every layout.
pub fn command_key(typed: &str, plain: &str, shift: bool) -> CommandKey {
    let typed = typed.to_lowercase();
    let plain = plain.to_lowercase();
    let is = |letter: &str| typed == letter || plain == letter;
    if is("v") {
        CommandKey::Paste
    } else if is("c") {
        CommandKey::Copy
    } else if is("x") {
        CommandKey::Cut
    } else if is("a") {
        CommandKey::SelectAll
    } else if is("z") {
        if shift {
            CommandKey::Redo
        } else {
            CommandKey::Undo
        }
    } else if is("w") {
        CommandKey::Close
    } else {
        // Arrows, Home and End, delete and forward delete
        match plain.chars().next() {
            Some('\u{F700}'..='\u{F703}' | '\u{F729}' | '\u{F72B}' | '\u{7F}' | '\u{F728}') => CommandKey::Edit,
            _ => CommandKey::Swallow,
        }
    }
}

/// While the panel has the keyboard, ⌘ keys stay in it. Without this they
/// would reach noFriction's own menu (⌘Q would quit it mid-recording) even
/// though another app is in front. The editing keys work and ⌘W closes the
/// panel; the rest, the panel's own ⌃⌥⌘N included (the global key is what
/// acts on that), do nothing.
extern "C" fn perform_key_equivalent(this: &Object, _sel: Sel, event: Id) -> BOOL {
    let mut handled = NO;
    guarded(|| unsafe {
        let flags: usize = msg_send![event, modifierFlags];
        if flags & COMMAND == 0 {
            return;
        }
        handled = YES;
        let typed: Id = msg_send![event, characters];
        let plain: Id = msg_send![event, charactersIgnoringModifiers];
        // With ⌃ or ⌥ also down it is a chord for something else, never an editing key
        let chord = flags & (CONTROL | OPTION) != 0;
        let key = if chord {
            CommandKey::Swallow
        } else {
            command_key(&rust_string(typed), &rust_string(plain), flags & SHIFT != 0)
        };
        let action = match key {
            CommandKey::Paste => sel!(paste:),
            CommandKey::Copy => sel!(copy:),
            CommandKey::Cut => sel!(cut:),
            CommandKey::SelectAll => sel!(selectAll:),
            CommandKey::Undo => sel!(undo:),
            CommandKey::Redo => sel!(redo:),
            CommandKey::Close => return hide(),
            CommandKey::Edit => {
                // Straight to the field, never by way of the app's menu
                let responder: Id = msg_send![this, firstResponder];
                let from = this as *const Object as Id;
                if !responder.is_null() && responder != from {
                    let _: () = msg_send![responder, keyDown: event];
                }
                return;
            }
            CommandKey::Swallow => return,
        };
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let from = this as *const Object as Id;
        let _: BOOL = msg_send![app, sendAction: action to: NIL from: from];
    });
    handled
}

/// Esc when the field isn't taking keys (a line is on its way to being kept).
extern "C" fn cancel(_this: &Object, _sel: Sel, _sender: Id) {
    guarded(hide);
}

fn panel_class() -> &'static Class {
    const NAME: &str = "NFQuickNotePanel";
    if let Some(cls) = Class::get(NAME) {
        return cls;
    }
    let mut decl = ClassDecl::new(NAME, class!(NSPanel)).expect("NFQuickNotePanel is declared once");
    unsafe {
        decl.add_method(sel!(canBecomeKeyWindow), can_become_key as extern "C" fn(&Object, Sel) -> BOOL);
        decl.add_method(sel!(canBecomeMainWindow), can_become_main as extern "C" fn(&Object, Sel) -> BOOL);
        decl.add_method(
            sel!(performKeyEquivalent:),
            perform_key_equivalent as extern "C" fn(&Object, Sel, Id) -> BOOL,
        );
        decl.add_method(sel!(cancelOperation:), cancel as extern "C" fn(&Object, Sel, Id));
    }
    decl.register()
}

// ── The delegate: Return, Esc, typing, and losing the keyboard ──

extern "C" fn keep(_this: &Object, _sel: Sel, _sender: Id) {
    guarded(|| {
        let Some(refs) = REFS.with(|r| r.get()) else { return };
        let text = unsafe {
            let value: Id = msg_send![refs.field, stringValue];
            rust_string(value)
        };
        if let Some(h) = HANDLERS.get() {
            (h.keep)(text);
        }
    });
}

extern "C" fn do_command(_this: &Object, _sel: Sel, _control: Id, _text_view: Id, command: Sel) -> BOOL {
    if command == sel!(cancelOperation:) {
        guarded(hide);
        return YES;
    }
    NO
}

extern "C" fn text_changed(_this: &Object, _sel: Sel, _notification: Id) {
    guarded(|| {
        let Some(refs) = REFS.with(|r| r.get()) else { return };
        let text = unsafe {
            let value: Id = msg_send![refs.field, stringValue];
            rust_string(value)
        };
        if let Some(h) = HANDLERS.get() {
            (h.changed)(text);
        }
    });
}

extern "C" fn resigned_key(_this: &Object, _sel: Sel, _notification: Id) {
    // While an answer is awaited or shown the card stays where it is: the
    // user went back to their call and will glance at it
    if PINNED.with(|p| p.get()) {
        return;
    }
    guarded(hide);
}

fn delegate_class() -> &'static Class {
    const NAME: &str = "NFQuickNoteDelegate";
    if let Some(cls) = Class::get(NAME) {
        return cls;
    }
    let mut decl = ClassDecl::new(NAME, class!(NSObject)).expect("NFQuickNoteDelegate is declared once");
    unsafe {
        decl.add_method(sel!(keep:), keep as extern "C" fn(&Object, Sel, Id));
        decl.add_method(
            sel!(control:textView:doCommandBySelector:),
            do_command as extern "C" fn(&Object, Sel, Id, Id, Sel) -> BOOL,
        );
        decl.add_method(sel!(controlTextDidChange:), text_changed as extern "C" fn(&Object, Sel, Id));
        decl.add_method(sel!(windowDidResignKey:), resigned_key as extern "C" fn(&Object, Sel, Id));
    }
    decl.register()
}

// ── Building it ──

unsafe fn create() -> Refs {
    let frame = rect(0.0, 0.0, WIDTH, HEIGHT);
    let panel: Id = msg_send![panel_class(), alloc];
    let panel: Id = msg_send![panel,
        initWithContentRect: frame
        styleMask: STYLE_BORDERLESS | STYLE_NONACTIVATING_PANEL
        backing: 2usize
        defer: YES];
    let _: () = msg_send![panel, setReleasedWhenClosed: NO];
    let _: () = msg_send![panel, setFloatingPanel: YES];
    let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: NO];
    // A panel hides when its app isn't active unless told not to, and this
    // one is only ever used while another app is
    let _: () = msg_send![panel, setHidesOnDeactivate: NO];
    // Hiding noFriction (⌘H) doesn't hide the panel, so showing it doesn't
    // bring the whole app back: without this the main window reappeared
    // over the slides when the key was pressed
    let _: () = msg_send![panel, setCanHide: NO];
    let _: () = msg_send![panel, setWorksWhenModal: YES];
    let _: () = msg_send![panel, setLevel: LEVEL];
    let _: () = msg_send![panel, setCollectionBehavior: JOIN_ALL_SPACES | IGNORES_CYCLE | FULL_SCREEN_AUXILIARY];
    let _: () = msg_send![panel, setOpaque: NO];
    let clear: Id = msg_send![class!(NSColor), clearColor];
    let _: () = msg_send![panel, setBackgroundColor: clear];
    let _: () = msg_send![panel, setHasShadow: YES];
    let _: () = msg_send![panel, setMovableByWindowBackground: NO];
    // No animation (NSWindowAnimationBehaviorNone): it is there when the key is pressed
    let _: () = msg_send![panel, setAnimationBehavior: 2isize];
    // Ask to be left out of screen captures and shared screens
    // (NSWindowSharingNone). macOS decides whether a capture honors it.
    let _: () = msg_send![panel, setSharingType: 0usize];
    let _: () = msg_send![panel, setTitle: ns_string("Type a note")];
    // The app is dark whatever the system is set to
    let dark: Id = msg_send![class!(NSAppearance), appearanceNamed: ns_string("NSAppearanceNameVibrantDark")];
    if !dark.is_null() {
        let _: () = msg_send![panel, setAppearance: dark];
    }

    // Background: the system's dark HUD material, rounded
    let effect: Id = msg_send![class!(NSVisualEffectView), alloc];
    let effect: Id = msg_send![effect, initWithFrame: frame];
    let _: () = msg_send![effect, setMaterial: 13isize];
    let _: () = msg_send![effect, setBlendingMode: 0isize];
    let _: () = msg_send![effect, setState: 1isize];
    let _: () = msg_send![effect, setWantsLayer: YES];
    let layer: Id = msg_send![effect, layer];
    if !layer.is_null() {
        let _: () = msg_send![layer, setCornerRadius: 14.0f64];
        let _: () = msg_send![layer, setMasksToBounds: YES];
        let _: () = msg_send![layer, setBorderWidth: 1.0f64];
        let edge: Id = msg_send![class!(NSColor), colorWithWhite: 1.0f64 alpha: 0.14f64];
        let edge: Id = msg_send![edge, CGColor];
        let _: () = msg_send![layer, setBorderColor: edge];
    }
    let _: () = msg_send![panel, setContentView: effect];
    let _: () = msg_send![effect, release];

    let delegate: Id = msg_send![delegate_class(), new];

    // The field: one line, no chrome
    let field: Id = msg_send![class!(NSTextField), alloc];
    let field: Id = msg_send![field, initWithFrame: rect(20.0, 32.0, WIDTH - 40.0, 28.0)];
    let _: () = msg_send![field, setBezeled: NO];
    let _: () = msg_send![field, setBordered: NO];
    let _: () = msg_send![field, setDrawsBackground: NO];
    let _: () = msg_send![field, setFocusRingType: 1usize];
    let font: Id = msg_send![class!(NSFont), systemFontOfSize: 20.0f64];
    let _: () = msg_send![field, setFont: font];
    let label: Id = msg_send![class!(NSColor), labelColor];
    let _: () = msg_send![field, setTextColor: label];
    let _: () = msg_send![field, setUsesSingleLineMode: YES];
    let cell: Id = msg_send![field, cell];
    let _: () = msg_send![cell, setScrollable: YES];
    let _: () = msg_send![cell, setWraps: NO];
    // Return keeps the line; losing the keyboard must not
    let _: () = msg_send![cell, setSendsActionOnEndEditing: NO];
    let _: () = msg_send![field, setTarget: delegate];
    let _: () = msg_send![field, setAction: sel!(keep:)];
    let _: () = msg_send![field, setDelegate: delegate];
    let _: () = msg_send![field, setAccessibilityLabel: ns_string("Type a note. Return keeps it at this moment.")];
    // Stays at the top when the panel grows downward for an answer
    // (NSViewMinYMargin: the space under it is what stretches)
    let _: () = msg_send![field, setAutoresizingMask: 8usize];
    let _: () = msg_send![effect, addSubview: field];
    let _: () = msg_send![field, release];

    // The line under it: which recording, or what went wrong
    let caption: Id = msg_send![class!(NSTextField), labelWithString: ns_string("")];
    let _: () = msg_send![caption, setFrame: rect(22.0, 11.0, WIDTH - 44.0, 16.0)];
    let small: Id = msg_send![class!(NSFont), systemFontOfSize: 11.0f64];
    let _: () = msg_send![caption, setFont: small];
    // NSLineBreakByTruncatingTail
    let _: () = msg_send![caption, setLineBreakMode: 4usize];
    let _: () = msg_send![caption, setAutoresizingMask: 8usize];
    let _: () = msg_send![effect, addSubview: caption];

    // The answer to a question, under the caption: wrapped text, selectable
    // so a line can be copied, hidden until there is one
    let answer: Id = msg_send![class!(NSTextField), wrappingLabelWithString: ns_string("")];
    let _: () = msg_send![answer, setFrame: rect(22.0, ANSWER_GAP, WIDTH - 44.0, 0.0)];
    let body: Id = msg_send![class!(NSFont), systemFontOfSize: 13.5f64];
    let _: () = msg_send![answer, setFont: body];
    let ink: Id = msg_send![class!(NSColor), labelColor];
    let _: () = msg_send![answer, setTextColor: ink];
    let _: () = msg_send![answer, setSelectable: YES];
    // A long answer ends in an ellipsis instead of being cut mid-line
    let _: () = msg_send![answer, setMaximumNumberOfLines: ANSWER_MAX_LINES];
    let answer_cell: Id = msg_send![answer, cell];
    let _: () = msg_send![answer_cell, setTruncatesLastVisibleLine: YES];
    let _: () = msg_send![answer, setHidden: YES];
    let _: () = msg_send![answer, setAccessibilityLabel: ns_string("Answer")];
    let _: () = msg_send![effect, addSubview: answer];

    // The panel doesn't retain its delegate; it lives as long as the app
    let _: () = msg_send![panel, setDelegate: delegate];
    let _: () = msg_send![panel, setInitialFirstResponder: field];

    // The caret in the app's yellow (#facc15)
    let editor: Id = msg_send![panel, fieldEditor: YES forObject: field];
    if !editor.is_null() {
        let yellow: Id = msg_send![class!(NSColor),
            colorWithSRGBRed: 0.980f64 green: 0.800f64 blue: 0.082f64 alpha: 1.0f64];
        let _: () = msg_send![editor, setInsertionPointColor: yellow];
    }

    Refs { panel, field, caption, answer }
}

/// Resize the panel to `height`, keeping its top edge where it is.
unsafe fn set_height(refs: Refs, height: f64) {
    let frame: Rect = msg_send![refs.panel, frame];
    if (frame.size.height - height).abs() < 0.5 {
        return;
    }
    let top = frame.origin.y + frame.size.height;
    let next = rect(frame.origin.x, top - height, frame.size.width, height);
    let _: () = msg_send![refs.panel, setFrame: next display: YES];
}

/// Take the answer away and give the panel its one-line height back.
unsafe fn drop_answer(refs: Refs) {
    let _: () = msg_send![refs.answer, setStringValue: ns_string("")];
    let _: () = msg_send![refs.answer, setHidden: YES];
    set_height(refs, HEIGHT);
}

fn refs() -> Refs {
    REFS.with(|cell| match cell.get() {
        Some(r) => r,
        None => {
            let r = unsafe { create() };
            cell.set(Some(r));
            r
        }
    })
}

/// Top centre of the screen the pointer is on, where Spotlight sits.
unsafe fn place(panel: Id) {
    let mouse: Point = msg_send![class!(NSEvent), mouseLocation];
    let screens: Id = msg_send![class!(NSScreen), screens];
    let count: usize = if screens.is_null() { 0 } else { msg_send![screens, count] };
    let mut target: Id = msg_send![class!(NSScreen), mainScreen];
    for i in 0..count {
        let screen: Id = msg_send![screens, objectAtIndex: i];
        let f: Rect = msg_send![screen, frame];
        let inside = mouse.x >= f.origin.x
            && mouse.x < f.origin.x + f.size.width
            && mouse.y >= f.origin.y
            && mouse.y < f.origin.y + f.size.height;
        if inside {
            target = screen;
            break;
        }
    }
    if target.is_null() {
        return;
    }
    let v: Rect = msg_send![target, visibleFrame];
    let (x, y) = origin((v.origin.x, v.origin.y, v.size.width, v.size.height), (WIDTH, HEIGHT));
    let _: () = msg_send![panel, setFrameOrigin: Point { x, y }];
}

/// Where the panel's bottom-left corner goes on a screen whose usable area
/// is `visible` (x, y, width, height; y grows upward): centred, its top a
/// fifth of the way down.
pub fn origin(visible: (f64, f64, f64, f64), size: (f64, f64)) -> (f64, f64) {
    let (vx, vy, vw, vh) = visible;
    let (w, h) = size;
    let x = vx + ((vw - w) / 2.0).max(0.0);
    let y = vy + (vh * 0.8 - h).max(0.0);
    (x.round(), y.round())
}

unsafe fn write_caption(refs: Refs, text: &str, error: bool) {
    let _: () = msg_send![refs.caption, setStringValue: ns_string(text)];
    let color: Id = if error {
        msg_send![class!(NSColor), systemRedColor]
    } else {
        msg_send![class!(NSColor), secondaryLabelColor]
    };
    let _: () = msg_send![refs.caption, setTextColor: color];
}

// ── What the parent module calls ──

/// Show the field over whatever is in front and give it the keyboard.
/// `keep_draft` leaves what was typed before; otherwise the field is empty.
pub fn show(caption: &str, keep_draft: bool) {
    show_field("Type a note", "Type a note. Return keeps it at this moment.", caption, keep_draft, false);
}

/// The same field, for a question (Ask): Return sends it. `keep_answer`
/// leaves the last answer on the card while the next question is typed.
pub fn show_ask(caption: &str, keep_answer: bool) {
    show_field("Ask about what's being said", "Ask about what is being said. Return asks.", caption, false, keep_answer);
}

fn show_field(placeholder: &str, spoken: &str, caption: &str, keep_draft: bool, keep_answer: bool) {
    pooled(|| unsafe {
        let refs = refs();
        PINNED.with(|p| p.set(false));
        if !keep_draft {
            let _: () = msg_send![refs.field, setStringValue: ns_string("")];
        }
        let already: BOOL = msg_send![refs.panel, isVisible];
        if !keep_answer {
            drop_answer(refs);
        }
        let _: () = msg_send![refs.field, setPlaceholderString: ns_string(placeholder)];
        let _: () = msg_send![refs.field, setAccessibilityLabel: ns_string(spoken)];
        let _: () = msg_send![refs.field, setEditable: YES];
        let _: () = msg_send![refs.field, setSelectable: YES];
        write_caption(refs, caption, false);
        // A card that is already up with its answer stays where it is
        if !(keep_answer && already != NO) {
            place(refs.panel);
        }
        let _: () = msg_send![refs.panel, orderFrontRegardless];
        let _: () = msg_send![refs.panel, makeKeyWindow];
        let _: BOOL = msg_send![refs.panel, makeFirstResponder: refs.field];
        cursor_to_end(refs);
    });
}

/// Keep the panel on screen when it loses the keyboard (an answer is on
/// its way or being read), or let it close as usual.
pub fn set_pinned(pinned: bool) {
    PINNED.with(|p| p.set(pinned));
}

/// Whether the panel stays up without the keyboard (an answer card).
pub fn pinned() -> bool {
    PINNED.with(|p| p.get())
}

/// Give the keyboard back to the app in front and stay on screen: the
/// question is sent and the user is in their call again. Until the card is
/// closed or takes the keyboard for the next question, a Return or a ⌘ key
/// goes to their app, not to it.
pub fn release_keyboard() {
    pooled(|| unsafe {
        let Some(refs) = REFS.with(|r| r.get()) else { return };
        let visible: BOOL = msg_send![refs.panel, isVisible];
        if visible == NO {
            return;
        }
        PINNED.with(|p| p.set(true));
        // Nothing in the card is being edited any more, whoever calls this
        let _: BOOL = msg_send![refs.panel, makeFirstResponder: NIL];
        let key: BOOL = msg_send![refs.panel, isKeyWindow];
        if key == NO {
            return;
        }
        // Ordering out hands the keyboard back to the window that had it;
        // ordering front again shows the card without taking it. Nothing
        // here is a close: no handler runs.
        let was_hiding = HIDING.with(|h| h.replace(true));
        let _: () = msg_send![refs.panel, orderOut: NIL];
        let _: () = msg_send![refs.panel, orderFrontRegardless];
        HIDING.with(|h| h.set(was_hiding));
    });
}

/// Show an answer under the field. The panel grows downward to fit it, up
/// to a limit; longer text ends in an ellipsis (the whole answer is in the
/// app). An empty text takes the answer away.
pub fn show_answer(text: &str) {
    pooled(|| unsafe {
        let Some(refs) = REFS.with(|r| r.get()) else { return };
        if text.trim().is_empty() {
            drop_answer(refs);
            return;
        }
        let _: () = msg_send![refs.answer, setStringValue: ns_string(text)];
        let width = WIDTH - 44.0;
        let cell: Id = msg_send![refs.answer, cell];
        let fits: Size = msg_send![cell, cellSizeForBounds: rect(0.0, 0.0, width, 10_000.0)];
        let height = fits.height.ceil().clamp(18.0, ANSWER_MAX_HEIGHT);
        // The panel first, so the views pinned to its top move with it
        set_height(refs, HEIGHT + height + ANSWER_GAP + 4.0);
        let _: () = msg_send![refs.answer, setFrame: rect(22.0, ANSWER_GAP, width, height)];
        let _: () = msg_send![refs.answer, setHidden: NO];
    });
}

/// What the answer label holds, and the panel's height (for the example
/// and the tests of the parent module).
pub fn answer_probe() -> Option<(String, f64)> {
    REFS.with(|r| r.get()).map(|refs| {
        pooled(|| unsafe {
            let value: Id = msg_send![refs.answer, stringValue];
            let frame: Rect = msg_send![refs.panel, frame];
            (rust_string(value), frame.size.height)
        })
    })
}

/// Becoming first responder selects the text; put the cursor after it.
unsafe fn cursor_to_end(refs: Refs) {
    let editor: Id = msg_send![refs.field, currentEditor];
    if !editor.is_null() {
        let value: Id = msg_send![refs.field, stringValue];
        let length: usize = msg_send![value, length];
        let _: () = msg_send![editor, setSelectedRange: Range { location: length, length: 0 }];
    }
}

/// Show a line of text and nothing to type in (nothing is recording). It
/// doesn't take the keyboard: what is typed next still goes to the app in
/// front. The caller hides it.
pub fn show_notice(text: &str, caption: &str) {
    pooled(|| unsafe {
        let refs = refs();
        PINNED.with(|p| p.set(false));
        drop_answer(refs);
        let _: () = msg_send![refs.field, setStringValue: ns_string("")];
        let _: () = msg_send![refs.field, setPlaceholderString: ns_string(text)];
        // End any editing left from the last time it was open
        let _: BOOL = msg_send![refs.panel, makeFirstResponder: NIL];
        let _: () = msg_send![refs.field, setEditable: NO];
        let _: () = msg_send![refs.field, setSelectable: NO];
        write_caption(refs, caption, false);
        place(refs.panel);
        let _: () = msg_send![refs.panel, orderFrontRegardless];
    });
}

/// The line under the field.
pub fn set_caption(text: &str, error: bool) {
    pooled(|| unsafe {
        if let Some(refs) = REFS.with(|r| r.get()) {
            write_caption(refs, text, error);
        }
    });
}

/// A line is on its way to being kept: nothing more can be typed until it
/// is, or isn't. `false` gives the field back, with the cursor after the text.
pub fn set_busy(busy: bool) {
    pooled(|| unsafe {
        let Some(refs) = REFS.with(|r| r.get()) else { return };
        if busy {
            // End the editing first, or the keys would still reach the editor
            let _: BOOL = msg_send![refs.panel, makeFirstResponder: NIL];
            let _: () = msg_send![refs.field, setEditable: NO];
            let _: () = msg_send![refs.field, setSelectable: NO];
            return;
        }
        let _: () = msg_send![refs.field, setEditable: YES];
        let _: () = msg_send![refs.field, setSelectable: YES];
        let key: BOOL = msg_send![refs.panel, isKeyWindow];
        if key != NO {
            let _: BOOL = msg_send![refs.panel, makeFirstResponder: refs.field];
            cursor_to_end(refs);
        }
    });
}

/// What is in the field.
pub fn text() -> String {
    REFS.with(|r| r.get()).map_or_else(String::new, |refs| {
        pooled(|| unsafe {
            let value: Id = msg_send![refs.field, stringValue];
            rust_string(value)
        })
    })
}

/// The field's grey line when it is empty (the question that was asked).
/// The next showing sets its own.
pub fn set_placeholder(text: &str) {
    pooled(|| unsafe {
        if let Some(refs) = REFS.with(|r| r.get()) {
            let _: () = msg_send![refs.field, setPlaceholderString: ns_string(text)];
        }
    });
}

/// The field's grey line (for the example).
pub fn placeholder() -> String {
    REFS.with(|r| r.get()).map_or_else(String::new, |refs| {
        pooled(|| unsafe {
            let value: Id = msg_send![refs.field, placeholderString];
            rust_string(value)
        })
    })
}

/// Put `text` in the field (a note that was put aside for a question).
pub fn set_text(text: &str) {
    pooled(|| unsafe {
        if let Some(refs) = REFS.with(|r| r.get()) {
            let _: () = msg_send![refs.field, setStringValue: ns_string(text)];
        }
    });
}

/// Empty the field (the line was kept).
pub fn clear() {
    pooled(|| unsafe {
        if let Some(refs) = REFS.with(|r| r.get()) {
            let _: () = msg_send![refs.field, setStringValue: ns_string("")];
        }
    });
}

pub fn is_open() -> bool {
    REFS.with(|r| r.get()).is_some_and(|refs| unsafe {
        let visible: BOOL = msg_send![refs.panel, isVisible];
        visible != NO
    })
}

/// Take the panel off the screen. The keyboard goes back to the app in
/// front, which never stopped being in front.
pub fn hide() {
    let Some(refs) = REFS.with(|r| r.get()) else { return };
    // Ordering out makes the panel resign key, which asks to hide again
    if HIDING.with(|h| h.replace(true)) {
        return;
    }
    let was_open = is_open();
    PINNED.with(|p| p.set(false));
    pooled(|| unsafe {
        let _: () = msg_send![refs.panel, orderOut: NIL];
        // A closed card holds no answer: nothing of a recording stays here
        drop_answer(refs);
    });
    HIDING.with(|h| h.set(false));
    if was_open {
        if let Some(h) = HANDLERS.get() {
            (h.closed)();
        }
    }
}

/// What the example reads to check the panel took the keyboard without
/// bringing its app forward.
#[derive(Debug, Clone, PartialEq)]
pub struct Probe {
    pub visible: bool,
    pub key: bool,
    /// The app is the one in front, as the system sees it
    pub app_active: bool,
    /// The field is being edited (typing goes into it)
    pub editing: bool,
    pub text: String,
    pub caption: String,
    pub frame: (f64, f64, f64, f64),
}

pub fn probe() -> Option<Probe> {
    let refs = REFS.with(|r| r.get())?;
    Some(pooled(|| unsafe {
        let visible: BOOL = msg_send![refs.panel, isVisible];
        let key: BOOL = msg_send![refs.panel, isKeyWindow];
        // The system's view. `-[NSApplication isActive]` is no use here: it
        // says yes whenever one of the app's windows has the keyboard.
        let me: Id = msg_send![class!(NSRunningApplication), currentApplication];
        let active: BOOL = msg_send![me, isActive];
        let editor: Id = msg_send![refs.field, currentEditor];
        let text: Id = msg_send![refs.field, stringValue];
        let caption: Id = msg_send![refs.caption, stringValue];
        let f: Rect = msg_send![refs.panel, frame];
        Probe {
            visible: visible != NO,
            key: key != NO,
            app_active: active != NO,
            editing: !editor.is_null(),
            text: rust_string(text),
            caption: rust_string(caption),
            frame: (f.origin.x, f.origin.y, f.size.width, f.size.height),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_sits_top_centre_of_the_usable_screen() {
        // A 1440×900 screen with a 25pt menu bar and a 70pt Dock
        let (x, y) = origin((0.0, 70.0, 1440.0, 805.0), (WIDTH, HEIGHT));
        assert_eq!(x, 440.0, "centred");
        assert_eq!(y, 70.0 + 644.0 - 76.0, "its top is a fifth of the way down");
        // A second screen to the left, above the main one
        let (x, y) = origin((-1920.0, 200.0, 1920.0, 1080.0), (WIDTH, HEIGHT));
        assert_eq!((x, y), (-1920.0 + 680.0, 200.0 + 864.0 - 76.0));
        // A screen smaller than the panel: never off its left or bottom edge
        assert_eq!(origin((10.0, 20.0, 300.0, 60.0), (WIDTH, HEIGHT)), (10.0, 20.0));
    }

    #[test]
    fn command_keys_edit_close_or_do_nothing() {
        use CommandKey::*;
        assert_eq!(command_key("v", "v", false), Paste);
        assert_eq!(command_key("c", "c", false), Copy);
        assert_eq!(command_key("x", "x", false), Cut);
        assert_eq!(command_key("a", "a", false), SelectAll);
        assert_eq!(command_key("z", "z", false), Undo);
        assert_eq!(command_key("Z", "z", true), Redo);
        assert_eq!(command_key("w", "w", false), Close);
        // A Russian layout: ⌘ gives the Latin letter, the key alone doesn't
        assert_eq!(command_key("v", "м", false), Paste);
        assert_eq!(command_key("a", "ф", false), SelectAll);
        // ⌘← ⌘→ ⌘⌫ and forward delete go to the field
        for key in ["\u{F702}", "\u{F703}", "\u{7F}", "\u{F728}"] {
            assert_eq!(command_key(key, key, false), Edit);
        }
        // ⌘Q, ⌘H, ⌘N, ⌘, and the rest never reach the app's menu
        for key in ["q", "h", "n", ",", "m", "1", "e", "k", ""] {
            assert_eq!(command_key(key, key, false), Swallow, "{key:?}");
        }
    }
}
