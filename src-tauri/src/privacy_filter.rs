// noFriction - what is private on screen, decided on this Mac.
//
// Two uses:
//   - While recording, a display or window is not captured at all while a
//     password manager, or a browser window whose title says it is private,
//     is in front of it (`never_captured`; capture_engine.rs).
//   - Read the screens never sends a wider set of screens to an AI
//     (screen_prompts.rs `never_sent`, which uses the helpers here).
//
// Both go by the app's name and the window's title. What that can't see:
//   - Safari doesn't mark private windows in their titles.
//   - A private window that is only visible behind another one.
//   - An app that isn't on the lists below.
// Don't describe either use as more than this in user-facing text.

/// List of window title patterns that indicate private browsing
const PRIVATE_PATTERNS: &[&str] = &[
    "Private",          // Safari Private Browsing
    "Incognito",        // Chrome Incognito
    "InPrivate",        // Edge InPrivate
    "Private Browsing", // Firefox
    "Private Window",   // Generic
];

/// Browsers, by a whole word of the app's name (lower case). A window
/// title says "private" about a browser window; in another app it is just a
/// word in a document's name.
pub const BROWSERS: &[&str] = &["safari", "chrome", "chromium", "firefox", "arc", "brave", "edge", "opera", "vivaldi", "duckduckgo", "orion"];

/// Password apps, by whole words of the app's name (lower case): their
/// windows are never captured while recording.
const PASSWORD_APPS: &[&str] = &[
    "1password", "keychain access", "bitwarden", "lastpass", "dashlane", "keepassxc", "authy", "passwords", "enpass",
    "nordpass", "proton pass", "strongbox", "keeper", "roboform",
    // Apple's Passwords app under its names in other languages
    "passwörter", "contraseñas", "mots de passe", "wachtwoorden", "senhas", "パスワード", "密码",
];

/// A name as words between single spaces (" proton pass "), so a short name
/// matches as a whole word and never inside another ("keeper", not
/// "Bookkeeper").
fn spaced(name: &str) -> String {
    let lowered = name.to_lowercase();
    let only: String = lowered.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    format!(" {} ", only.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn names(spaced_name: &str, list: &[&str]) -> bool {
    list.iter().any(|n| spaced_name.contains(&format!(" {} ", n)))
}

/// While recording, no picture is taken of this window, or of a display
/// this window is in front of: a password app, or a browser window whose
/// title says it is private (Chrome, Edge and Firefox say Incognito,
/// InPrivate or Private Browsing; Safari's titles don't say).
///
/// Deliberately narrower than what Read the screens never sends: mail, chat
/// and terminals are part of many recordings and are still captured. A
/// picture the user takes by hand (the snapshot button) is their own
/// choice and isn't refused.
pub fn never_captured(app_name: &str, window_title: &str) -> bool {
    let app = spaced(app_name);
    names(&app, PASSWORD_APPS) || (names(&app, BROWSERS) && is_private_title(window_title))
}

/// Why a capture was skipped, for the log (never a title or an app name)
pub const PRIVATE_IN_FRONT: &str = "a private window is in front";

/// Apps Read the screens never sends (screen_prompts.rs `never_sent`):
/// password managers, and Terminal, which may show keys and tokens
const BLOCKED_APPS: &[&str] = &[
    "1Password",
    "Keychain Access",
    "Bitwarden",
    "LastPass",
    "Dashlane",
    "KeePassXC",
    "Authy",
    "Terminal", // May contain sensitive commands
];

/// (owner app name, window title) for every normal on-screen window, front
/// to back. Titles are empty without Screen Recording permission, so callers
/// must treat "no titled windows" as "signal unavailable". None if the
/// window server can't be queried.
#[cfg(target_os = "macos")]
pub fn on_screen_windows() -> Option<Vec<(String, String)>> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerName,
    };

    let windows = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )?;
    let (k_layer, k_name, k_owner) = unsafe {
        (
            CFString::wrap_under_get_rule(kCGWindowLayer),
            CFString::wrap_under_get_rule(kCGWindowName),
            CFString::wrap_under_get_rule(kCGWindowOwnerName),
        )
    };
    let mut out = Vec::new();
    for raw in windows.get_all_values() {
        if raw.is_null() {
            continue;
        }
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(raw as CFDictionaryRef) };
        let layer = dict
            .find(&k_layer)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64());
        if layer != Some(0) {
            continue;
        }
        let text = |k: &CFString| {
            dict.find(k)
                .and_then(|v| v.downcast::<CFString>())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        out.push((text(&k_owner), text(&k_name)));
    }
    Some(out)
}

#[cfg(not(target_os = "macos"))]
pub fn on_screen_windows() -> Option<Vec<(String, String)>> {
    None
}

/// How much of a window is on a display: the width and height of the part
/// they share. Rectangles are (x, y, width, height) in the window server's
/// global points.
fn shared(window: (f64, f64, f64, f64), display: (f64, f64, f64, f64)) -> (f64, f64) {
    let (x, y, w, h) = window;
    let (dx, dy, dw, dh) = display;
    let width = (x + w).min(dx + dw) - x.max(dx);
    let height = (y + h).min(dy + dh) - y.max(dy);
    (width.max(0.0), height.max(0.0))
}

/// A window counts as being on a display when a part of it that someone
/// could be looking at is there: at least 80 by 60 points. A window lying
/// across two displays is on both (which errs toward leaving a screen
/// out); a helper window of a few points, or a sliver at the edge, is on
/// neither.
fn window_is_on(window: (f64, f64, f64, f64), display: (f64, f64, f64, f64)) -> bool {
    let (width, height) = shared(window, display);
    width >= 80.0 && height >= 60.0
}

/// The window takes up most of the display (60% or more of its area).
fn window_fills(window: (f64, f64, f64, f64), display: (f64, f64, f64, f64)) -> bool {
    let (width, height) = shared(window, display);
    let area = display.2 * display.3;
    area > 0.0 && (width * height) / area >= 0.6
}

/// The name `front_window_on` gives when what is in front of a display is
/// noFriction itself, filling it (screen_prompts.rs `never_sent` leaves
/// that screen out: it is a picture of the user's notes).
pub const OWN_APP: &str = "noFriction";

/// The app and title of the frontmost window on one display, for a capture
/// of that whole display (which has no app of its own). Kept beside the
/// screen so a display with a private app in front can be left out of Read
/// the screens (screen_prompts.rs `never_sent`).
///
/// "In front" is the first normal window, front to back, that is on this
/// display and visible. One of noFriction's own windows is passed over
/// (the notes window being in front says nothing about what else the
/// display showed) unless it fills the display: then the picture is of
/// noFriction, and that is what is returned (`OWN_APP`, no title).
/// None when there is no such window or the window server can't say; the
/// title is empty without Screen Recording permission.
#[cfg(target_os = "macos")]
pub fn front_window_on(display_id: u32) -> Option<(String, String)> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::CGRect;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowOwnerName,
        kCGWindowOwnerPID,
    };

    let screen = CGDisplay::new(display_id).bounds();
    let display = (screen.origin.x, screen.origin.y, screen.size.width, screen.size.height);
    let windows = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    )?;
    let (k_layer, k_name, k_owner, k_pid, k_alpha, k_bounds) = unsafe {
        (
            CFString::wrap_under_get_rule(kCGWindowLayer),
            CFString::wrap_under_get_rule(kCGWindowName),
            CFString::wrap_under_get_rule(kCGWindowOwnerName),
            CFString::wrap_under_get_rule(kCGWindowOwnerPID),
            CFString::wrap_under_get_rule(kCGWindowAlpha),
            CFString::wrap_under_get_rule(kCGWindowBounds),
        )
    };
    let me = std::process::id() as i64;
    for raw in windows.get_all_values() {
        if raw.is_null() {
            continue;
        }
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(raw as CFDictionaryRef) };
        let num = |k: &CFString| dict.find(k).and_then(|v| v.downcast::<CFNumber>());
        // A normal window (not a menu, the Dock or an overlay) that can be seen
        if num(&k_layer).and_then(|n| n.to_i64()) != Some(0) {
            continue;
        }
        if num(&k_alpha).and_then(|n| n.to_f64()).is_some_and(|a| a < 0.05) {
            continue;
        }
        let bounds = dict
            .find(&k_bounds)
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|d| CGRect::from_dict_representation(&d));
        let Some(b) = bounds else { continue };
        let window = (b.origin.x, b.origin.y, b.size.width, b.size.height);
        if !window_is_on(window, display) {
            continue;
        }
        if num(&k_pid).and_then(|n| n.to_i64()) == Some(me) {
            // Our own window: it is what the picture shows only if it fills the display
            if window_fills(window, display) {
                return Some((OWN_APP.to_string(), String::new()));
            }
            continue;
        }
        let text = |k: &CFString| {
            dict.find(k)
                .and_then(|v| v.downcast::<CFString>())
                .map(|s| s.to_string())
                .unwrap_or_default()
        };
        return Some((text(&k_owner), text(&k_name)));
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub fn front_window_on(_display_id: u32) -> Option<(String, String)> {
    None
}

/// A stored window title says private or incognito browsing
/// (screen_prompts.rs `never_sent`: such a screen is never sent to an AI).
pub fn is_private_title(title: &str) -> bool {
    let title = title.to_lowercase();
    PRIVATE_PATTERNS.iter().any(|p| {
        let p = p.to_lowercase();
        // Whole words: "Private Browsing", not "Privateer" or "Deprivation"
        title.match_indices(&p).any(|(i, _)| {
            let before = title[..i].chars().next_back();
            let after = title[i + p.len()..].chars().next();
            !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
        })
    })
}

/// An app Read the screens never sends a window of
pub fn is_blocked_app(app_name: &str) -> bool {
    for blocked in BLOCKED_APPS {
        if app_name.to_lowercase().contains(&blocked.to_lowercase()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_apps_and_private_browser_windows_are_never_captured() {
        for (app, title) in [
            ("1Password 8", ""),
            ("1Password", "Vault — Personal"),
            ("Keychain Access", ""),
            ("Passwords", ""),
            ("Bitwarden", "My vault"),
            ("Proton Pass", ""),
            ("Keeper Password Manager", ""),
            ("Passwörter", ""),
            ("Google Chrome", "New Incognito Tab - Google Chrome"),
            ("Microsoft Edge", "InPrivate"),
            ("Firefox", "Mozilla Firefox Private Browsing"),
            ("Brave Browser", "Private Window"),
        ] {
            assert!(never_captured(app, title), "{app:?} {title:?}");
        }
        for (app, title) in [
            // Ordinary windows
            ("Keynote", "Lecture 7"),
            ("Google Chrome", "Lecture 7 - Google Slides"),
            ("Safari", "Cell membrane - Wikipedia"),
            // Mail, chat and terminals are part of many recordings: captured
            ("Mail", "Inbox"),
            ("Slack", "#general"),
            ("Terminal", "zsh"),
            // "Private" in a document's name is not private browsing
            ("Microsoft Word", "Private equity notes.docx"),
            ("Keynote", "Private markets"),
            ("Preview", "Incognito.pdf"),
            // A whole word, never part of another app's name
            ("Bookkeeper", ""),
            ("Passwordless Demo", ""),
            // Safari doesn't say so in the title: it can't be recognised
            ("Safari", "Start Page"),
            ("", ""),
        ] {
            assert!(!never_captured(app, title), "{app:?} {title:?}");
        }
    }

    #[test]
    fn test_blocked_apps() {
        assert!(is_blocked_app("1Password 8"));
        assert!(is_blocked_app("Keychain Access"));
        assert!(!is_blocked_app("Visual Studio Code"));
    }

    #[test]
    fn a_window_is_on_every_display_it_can_be_seen_on() {
        let main = (0.0, 0.0, 1512.0, 982.0);
        let projector = (1512.0, 0.0, 1920.0, 1080.0);
        // A window in the middle of the main display
        assert!(window_is_on((200.0, 100.0, 900.0, 600.0), main));
        assert!(!window_is_on((200.0, 100.0, 900.0, 600.0), projector));
        // Full screen on the projector
        assert!(window_is_on((1512.0, 0.0, 1920.0, 1080.0), projector));
        assert!(!window_is_on((1512.0, 0.0, 1920.0, 1080.0), main));
        // Lying across the edge: on both, wherever its centre is
        assert!(window_is_on((1000.0, 100.0, 800.0, 600.0), main));
        assert!(window_is_on((1000.0, 100.0, 800.0, 600.0), projector));
        // Mostly off the right edge of the only display, still in front of it
        assert!(window_is_on((1200.0, 100.0, 800.0, 600.0), main));
        // Only a sliver shows: not something on that display
        assert!(!window_is_on((1450.0, 100.0, 800.0, 600.0), main));
        // A display above the main one has negative coordinates
        assert!(window_is_on((100.0, -900.0, 800.0, 600.0), (0.0, -1080.0, 1920.0, 1080.0)));
        // A helper window of a few points isn't something on screen
        assert!(!window_is_on((10.0, 10.0, 1.0, 1.0), main));
        assert!(!window_is_on((10.0, 10.0, 300.0, 20.0), main));
    }

    #[test]
    fn our_own_window_is_the_picture_only_when_it_fills_the_display() {
        let main = (0.0, 0.0, 1512.0, 982.0);
        // Full screen, or maximised under the menu bar
        assert!(window_fills((0.0, 0.0, 1512.0, 982.0), main));
        assert!(window_fills((0.0, 38.0, 1512.0, 944.0), main));
        // A notes window beside the slides: the display shows more than noFriction
        assert!(!window_fills((0.0, 38.0, 700.0, 944.0), main));
        assert!(!window_fills((900.0, 300.0, 500.0, 400.0), main));
        // Filling another display says nothing about this one
        assert!(!window_fills((1512.0, 0.0, 1920.0, 1080.0), main));
        assert!(!window_fills((0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 0.0, 0.0)));
    }

    /// The lookup runs on every captured frame of a whole display, so it
    /// is run here for real: against the window server, for the main
    /// display and for a display that doesn't exist. It reads which windows
    /// are open (their names only with Screen Recording permission), takes
    /// no picture and shows no prompt. Nothing it returns is printed.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_front_window_lookup_runs_against_the_window_server() {
        let main = core_graphics::display::CGDisplay::main().id;
        for _ in 0..50 {
            // Whatever it answers, it answers: no crash, no hang, a sane name
            if let Some((app, _)) = front_window_on(main) {
                assert!(app.len() < 400);
            }
        }
        // A display that isn't connected has no bounds, so nothing is on it
        assert_eq!(front_window_on(0xFFFF_FFF0), None);
    }

    #[test]
    fn a_stored_title_says_private_browsing() {
        assert!(is_private_title("Private Browsing — Start Page"));
        assert!(is_private_title("New Incognito Tab - Google Chrome"));
        assert!(is_private_title("InPrivate"));
        assert!(!is_private_title("Deprivation studies.pdf"));
        assert!(!is_private_title("Lecture 7"));
        assert!(!is_private_title(""));
    }
}
