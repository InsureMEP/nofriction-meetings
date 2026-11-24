//! Links into the app: `nofriction://settings/<page>` opens Settings at
//! that page (the scheme is declared in Info.plist). Made so that anything
//! that tells the user to go somewhere in the app can give the link that
//! lands there, first for Settings → Storage (docs/STORAGE.md).
//!
//! A link only ever opens a page. It carries no data into the app, starts
//! nothing, changes nothing, and anything that isn't one of the pages below
//! is ignored.

use parking_lot::Mutex;
use tauri::{AppHandle, Emitter};

/// The Settings pages a link can open (`SettingsCategory` in
/// src/lib/navigation.ts).
pub const SETTINGS_PAGES: [&str; 7] = ["recording", "transcription", "ai", "sync", "storage", "subscription", "about"];

/// `{}`: the window asks for the page with `take_app_link`.
pub const EVENT: &str = "app_link";

/// The page a link asked for, until the window takes it. A link can launch
/// the app, and then it arrives before the window is listening.
static PENDING: Mutex<Option<&'static str>> = Mutex::new(None);

/// The Settings page `url` asks for, if it is exactly
/// `nofriction://settings/<one of the pages>` (a trailing slash is fine).
pub fn settings_page(url: &str) -> Option<&'static str> {
    let rest = url.strip_prefix("nofriction://settings/")?;
    let page = rest.strip_suffix('/').unwrap_or(rest);
    SETTINGS_PAGES.iter().copied().find(|p| *p == page)
}

/// A link was opened (macOS hands it over at launch or while running).
pub fn open(app: &AppHandle, url: &str) {
    let Some(page) = settings_page(url) else { return };
    *PENDING.lock() = Some(page);
    // The page is one of the fixed names above, never text from the link
    log::info!("Link: Settings → {}", page);
    crate::notifications::focus_main_window(app);
    let _ = app.emit(EVENT, ());
}

/// The page a link asked for, once. The window calls this when it comes up
/// and when `EVENT` arrives.
#[tauri::command]
pub fn take_app_link() -> Option<&'static str> {
    PENDING.lock().take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_opens_a_settings_page_and_nothing_else() {
        assert_eq!(settings_page("nofriction://settings/storage"), Some("storage"));
        assert_eq!(settings_page("nofriction://settings/storage/"), Some("storage"));
        assert_eq!(settings_page("nofriction://settings/ai"), Some("ai"));
        for page in SETTINGS_PAGES {
            assert_eq!(settings_page(&format!("nofriction://settings/{}", page)), Some(page));
        }
        for not_a_page in [
            "nofriction://settings/",
            "nofriction://settings",
            "nofriction://settings/storage/remove",
            "nofriction://settings/storage?remove=all",
            "nofriction://settings/Storage",
            "nofriction://settings/../storage",
            "nofriction://record",
            "nofriction://recordings/delete/abc",
            "https://settings/storage",
            "nofriction:settings/storage",
            "",
        ] {
            assert_eq!(settings_page(not_a_page), None, "{}", not_a_page);
        }
        // Taken once
        *PENDING.lock() = Some("storage");
        assert_eq!(take_app_link(), Some("storage"));
        assert_eq!(take_app_link(), None);
    }
}
