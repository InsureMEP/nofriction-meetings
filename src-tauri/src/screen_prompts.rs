// noFriction - the prompt library for reading a screen
// (docs/design/FADELL_V2.md §12, docs/AI_PROVIDERS.md "Read the screens").
//
// When the user presses Read the screens, each chosen screen goes to their
// AI as one picture with one prompt. Which prompt depends on what kind of
// screen it is, and that is decided here, on the device, from the app and
// window the screen came from: no extra request is made to classify it.
// The prompt is fixed text: it carries nothing from the recording (no
// transcript, title, app or window name), so what the AI answers can only
// come from the picture.
//
// Some screens are never sent at all (`never_sent`): mail, messages,
// password apps, terminals, private browsing, files that look like keys.
// That is decided here too, before the user is shown the list.
//
// Every prompt asks for the same JSON, so one parser reads every answer:
//   {"skip": false, "kind": "slide", "title": "...", "points": ["..."],
//    "terms": [{"term": "...", "meaning": "..."}], "text": "..."}
//
// The library is data. To add a kind: add a `ScreenKind`, its entry in
// `LIBRARY`, and the apps that show it in `kind_for`. The shared rules are
// added to every prompt by `prompt_for`; an entry only says what to look
// for on that kind of screen. A rule that protects someone (people on a
// call, a password on screen) belongs in the shared rules, because a screen
// captured as a whole display has no app name and gets the `Other` prompt.

use crate::recording_kind::RecordingKind;

/// What a captured screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenKind {
    /// A presentation slide
    Slide,
    /// Handwriting or drawing on a board or canvas
    Whiteboard,
    /// A page of prose: a document, a handout, a PDF, an article
    Document,
    /// Source code or a terminal
    Code,
    /// A spreadsheet or table of numbers
    Table,
    /// A chart, graph or diagram
    Chart,
    /// A web page or web app
    WebPage,
    /// A video call: what is being shared, not who is in it
    Call,
    /// A photo the user took with their iPhone during the recording
    Photo,
    /// Anything else
    Other,
}

impl ScreenKind {
    /// The word stored with a reading and shown beside it
    pub fn label(self) -> &'static str {
        match self {
            ScreenKind::Slide => "Slide",
            ScreenKind::Whiteboard => "Whiteboard",
            ScreenKind::Document => "Document",
            ScreenKind::Code => "Code",
            ScreenKind::Table => "Table",
            ScreenKind::Chart => "Chart",
            ScreenKind::WebPage => "Web page",
            ScreenKind::Call => "Call",
            ScreenKind::Photo => "Photo",
            ScreenKind::Other => "Screen",
        }
    }

    /// The prompt for this kind asks the AI to say what the picture is, so
    /// the kind in its answer is kept.
    pub fn ai_decides(self) -> bool {
        matches!(self, ScreenKind::Other | ScreenKind::Photo)
    }

    /// The kind an answer names (`"kind": "slide"`), if it is one of ours.
    pub fn from_answer(s: &str) -> Option<ScreenKind> {
        let s = s.trim().to_lowercase();
        ALL.iter().copied().find(|k| {
            let l = k.label().to_lowercase();
            s == l || s == l.replace(' ', "") || s == l.replace(' ', "_") || (*k == ScreenKind::WebPage && s == "web")
        })
    }
}

pub const ALL: &[ScreenKind] = &[
    ScreenKind::Slide,
    ScreenKind::Whiteboard,
    ScreenKind::Document,
    ScreenKind::Code,
    ScreenKind::Table,
    ScreenKind::Chart,
    ScreenKind::WebPage,
    ScreenKind::Call,
    ScreenKind::Photo,
    ScreenKind::Other,
];

/// One entry of the library.
pub struct ScreenPrompt {
    pub kind: ScreenKind,
    /// When this prompt is used, for people reading the library
    pub when: &'static str,
    /// What to look for on this kind of screen
    pub look_for: &'static str,
}

pub const LIBRARY: &[ScreenPrompt] = &[
    ScreenPrompt {
        kind: ScreenKind::Slide,
        when: "Keynote, PowerPoint, Google Slides, or a PDF shown full screen",
        look_for: "This is a presentation slide. Give its title, then each bullet or statement on it as one point, in the order shown. \
Copy formulas, numbers, dates and names exactly. If the slide has a diagram or picture, add one point saying what it shows. \
Put any term the slide defines in \"terms\".",
    },
    ScreenPrompt {
        kind: ScreenKind::Whiteboard,
        when: "Miro, FigJam, Freeform, Excalidraw or another drawing canvas; photos of a board",
        look_for: "This is a whiteboard or drawing canvas. Read the handwriting and labels. Give each written line, list or labelled \
shape as one point, and say how they connect when arrows or lines show it (\"A leads to B\"). Copy formulas exactly. \
If a word can't be read, write [unreadable] in its place; never guess a word.",
    },
    ScreenPrompt {
        kind: ScreenKind::Document,
        when: "Word, Pages, Google Docs, Notion, Preview, a PDF reader, a notes app",
        look_for: "This is a page of a document. Give the document's or section's title, then the main statements on the visible \
part as points: definitions, claims, steps, deadlines. Copy numbers, dates and names exactly. Put defined terms in \"terms\". \
Don't summarise text that is cut off at the edge of the screen.",
    },
    ScreenPrompt {
        kind: ScreenKind::Code,
        when: "Xcode, VS Code, a JetBrains editor (a terminal window is never sent)",
        look_for: "This is source code. Give the file being shown as the title. As points, say what the visible code does, \
function by function, and any error or output message. Put the few lines that matter most, copied exactly, in \"text\".",
    },
    ScreenPrompt {
        kind: ScreenKind::Table,
        when: "Excel, Numbers, Google Sheets, Airtable",
        look_for: "This is a spreadsheet or table. Give its title or sheet name. As points, name the columns, then the rows or \
totals that stand out, with their numbers copied exactly and their units. Don't list every cell.",
    },
    ScreenPrompt {
        kind: ScreenKind::Chart,
        when: "A chart, graph or diagram fills the screen",
        look_for: "This is a chart, graph or diagram. Give its title. As points: what each axis or part is, the unit, what the \
chart shows (the trend, the largest and smallest values, any labelled point), with numbers copied exactly. For a diagram, \
name the parts and how they connect.",
    },
    ScreenPrompt {
        kind: ScreenKind::WebPage,
        when: "Safari, Chrome, Firefox, Arc, Brave, Edge",
        look_for: "This is a web page. Give the page's title or heading, then the main statements of the content as points. \
Ignore navigation, menus, advertisements and cookie notices. Copy numbers, dates and names exactly. \
If the page is really slides, a document or a table, read it as that.",
    },
    ScreenPrompt {
        kind: ScreenKind::Call,
        when: "Zoom, Microsoft Teams, Google Meet, FaceTime, Webex",
        look_for: "This is a video call. Read only what is being shared on the call (slides, a document, a screen): give its \
title and its content as points. If nothing is being shared, answer with \"skip\": true.",
    },
    ScreenPrompt {
        kind: ScreenKind::Photo,
        when: "A photo taken with the iPhone during the recording",
        look_for: "This is a photo taken during the recording, most likely of a slide, a whiteboard, a handout or a page of a \
book. Say which in \"kind\". Read what is written on it: give its title, then each line, bullet or labelled part as one point. \
Copy formulas exactly. If a word can't be read, write [unreadable] in its place; never guess a word. If the photo is mainly of \
people or a place, with nothing written to read, answer with \"skip\": true.",
    },
    ScreenPrompt {
        kind: ScreenKind::Other,
        when: "Anything not recognised",
        look_for: "First decide what this screen is: a slide, a whiteboard, a document, code, a table, a chart, a web page or \
something else, and say so in \"kind\". Then give its title and what it says as points, copying formulas, numbers, dates and \
names exactly.",
    },
];

/// The library entry for a kind.
pub fn entry(kind: ScreenKind) -> &'static ScreenPrompt {
    LIBRARY.iter().find(|p| p.kind == kind).unwrap_or(&LIBRARY[LIBRARY.len() - 1])
}

fn has(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// A name as words between single spaces (" visual studio code "), so
/// "arc" matches Arc and not Search, and "word" not 1Password.
fn words(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    format!(" {} ", spaced.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// One of `names` appears in `spaced` (from `words`) as whole words.
fn is_app(spaced: &str, names: &[&str]) -> bool {
    names.iter().any(|n| spaced.contains(&format!(" {} ", n)))
}

/// `title` names a file with this extension: ".key" matches "talk.key" and
/// "talk.key — Edited", never "server.keychain".
fn has_ext(title: &str, ext: &str) -> bool {
    title.match_indices(ext).any(|(i, _)| !title[i + ext.len()..].chars().next().is_some_and(|c| c.is_alphanumeric()))
}

const BROWSERS: &[&str] = &["safari", "chrome", "chromium", "firefox", "arc", "brave", "edge", "opera", "vivaldi", "duckduckgo", "orion"];

/// Apps whose windows are never sent: mail, messages, passwords, money,
/// and terminals (which show commands, keys and tokens).
const PRIVATE_APPS: &[&str] = &[
    "mail", "outlook", "spark", "airmail", "mimestream", "superhuman", "thunderbird",
    "messages", "whatsapp", "signal", "telegram", "messenger", "slack", "discord", "wechat", "line",
    "passwords", "keychain access", "wallet", "health",
    "terminal", "iterm", "iterm2", "warp", "kitty", "alacritty", "ghostty", "hyper", "wezterm",
];

/// Window titles (whole words) that say the same about a browser tab or a
/// document.
const PRIVATE_TITLE_WORDS: &[&str] = &[
    "gmail", "inbox", "outlook", "proton mail", "protonmail", "whatsapp", "messenger", "telegram",
    "password", "passwords", "1password", "bitwarden", "lastpass", "dashlane",
    "sign in", "log in", "login", "two factor", "verification code",
    "bank", "banking", "paypal", "venmo", "tax return", "payslip", "pay stub",
    "private key", "api key", "api keys", "secrets", "credentials", "id rsa", "id ed25519",
];

/// Files that hold keys or secrets.
const PRIVATE_EXTENSIONS: &[&str] = &[".pem", ".p8", ".p12", ".pfx", ".keychain", ".env", ".kdbx", ".ovpn"];

/// This screen is never sent to an AI, whatever the user presses. Decided
/// on the device from the app and window it was captured from, before the
/// user is shown what would go. It errs toward leaving a screen out.
///
/// A screen captured as a whole display has no app and no window title, so
/// nothing here can recognise it: the confirmation shows every picture and
/// lets the user leave any out, and the shared rules tell the AI to skip
/// private content.
pub fn never_sent(app_name: Option<&str>, window_title: Option<&str>) -> bool {
    let app_raw = app_name.unwrap_or("").trim();
    let title_raw = window_title.unwrap_or("").trim();
    // noFriction's own window: the recording's notes, not a screen of it
    if app_raw.eq_ignore_ascii_case("noFriction") {
        return true;
    }
    // Password managers, and private or incognito browser windows
    if crate::privacy_filter::is_blocked_app(app_raw) || crate::privacy_filter::is_private_title(title_raw) {
        return true;
    }
    let app = words(&app_raw.to_lowercase());
    let title = title_raw.to_lowercase();
    if is_app(&app, PRIVATE_APPS) || is_app(&words(&title), PRIVATE_TITLE_WORDS) {
        return true;
    }
    if PRIVATE_EXTENSIONS.iter().any(|e| has_ext(&title, e)) {
        return true;
    }
    // "server.key" is a private key unless Keynote is showing it
    has_ext(&title, ".key") && !is_app(&app, &["keynote"])
}

/// What kind of screen this is, from the app and window it came from.
/// Decided on the device; nothing is sent to find out.
///
/// The app decides when it is one that shows one kind of thing. Only for a
/// browser, or an app that isn't recognised, does the window title decide
/// (a tab showing Google Slides is a slide).
pub fn kind_for(app_name: Option<&str>, window_title: Option<&str>) -> ScreenKind {
    let app = words(&app_name.unwrap_or("").trim().to_lowercase());
    let title = window_title.unwrap_or("").trim().to_lowercase();
    if is_app(&app, &["keynote", "powerpoint"]) {
        return ScreenKind::Slide;
    }
    if is_app(&app, &["miro", "figjam", "freeform", "excalidraw", "whiteboard"]) {
        return ScreenKind::Whiteboard;
    }
    if is_app(&app, &["zoom", "teams", "meet", "facetime", "webex"]) {
        return ScreenKind::Call;
    }
    if is_app(&app, &["xcode", "visual studio code", "code", "intellij", "pycharm", "webstorm", "rustrover", "cursor", "zed"]) {
        return ScreenKind::Code;
    }
    if is_app(&app, &["excel", "numbers", "airtable"]) {
        return ScreenKind::Table;
    }
    if is_app(&app, &["word", "pages", "preview", "acrobat", "notion", "obsidian", "notes", "textedit", "books"]) {
        return ScreenKind::Document;
    }
    // A browser tab, or a window of an app this library doesn't know
    if has(&title, &["google slides", "- slides", "powerpoint"]) || has_ext(&title, ".pptx") {
        return ScreenKind::Slide;
    }
    if has(&title, &["google sheets", "- sheets"]) || [".xlsx", ".csv", ".numbers"].iter().any(|e| has_ext(&title, e)) {
        return ScreenKind::Table;
    }
    if has(&title, &["google docs", "- docs"]) || [".docx", ".pdf", ".md"].iter().any(|e| has_ext(&title, e)) {
        return ScreenKind::Document;
    }
    if has(&title, &["miro", "figjam", "excalidraw", "whiteboard", "jamboard"]) {
        return ScreenKind::Whiteboard;
    }
    if is_app(&app, BROWSERS) {
        ScreenKind::WebPage
    } else {
        ScreenKind::Other
    }
}

/// What every prompt ends with: the rules that hold for any picture,
/// whichever prompt it got.
const SHARED_RULES: &str = "Rules:\n\
- Use only what is visible in the picture. Never guess at text that is too small, cut off or blurred.\n\
- Copy formulas, numbers, dates, names and quoted wording exactly as shown.\n\
- Never describe a person's appearance, and never name a person from their face. Ignore people in a photo.\n\
- If the picture shows a video call, read only what is being shared on it. Never describe or name the people in video \
tiles, and never read the chat.\n\
- Never include anything that looks like a password, key or token: write [secret] in its place.\n\
- If the picture shows something private that is not the subject of the recording (passwords, keys, messages, email, \
banking, medical records), answer only {\"skip\": true}.\n\
- If there is nothing worth keeping (a blank screen, a desktop, a loading page), answer only {\"skip\": true}.\n\
- At most 8 points, each one sentence. At most 6 terms.\n\
\n\
Answer with JSON only, in this shape:\n\
{\"skip\": false, \"kind\": \"slide\", \"title\": \"the title, or null\", \"points\": [\"one statement from the screen\"], \
\"terms\": [{\"term\": \"a term the screen defines\", \"meaning\": \"its meaning as the screen gives it\"}], \
\"text\": \"wording that must be kept exactly (a formula, a few lines of code), or null\"}";

/// The whole prompt sent with one picture. It is fixed text chosen by the
/// kind of screen and the type of recording: nothing from the recording
/// (what was said, its title, the app or window name) is put in it, so a
/// reading can only repeat what its own picture shows.
pub fn prompt_for(kind: ScreenKind, recording: RecordingKind) -> String {
    let who = match recording {
        RecordingKind::Class => "a student's recording of a class",
        RecordingKind::Meeting => "a recording of a meeting",
        RecordingKind::Personal => "a personal recording",
    };
    let what = if kind == ScreenKind::Photo { "one photo that was taken" } else { "one screen that was shown" };
    format!(
        "You read {what} during {who}, so its content can be kept with the notes.\n\n{look}\n\n{rules}",
        what = what,
        who = who,
        look = entry(kind).look_for,
        rules = SHARED_RULES
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_has_one_prompt_for_every_kind() {
        for kind in ALL {
            assert_eq!(LIBRARY.iter().filter(|p| p.kind == *kind).count(), 1, "{kind:?}");
            assert_eq!(entry(*kind).kind, *kind);
            assert!(!entry(*kind).when.is_empty() && entry(*kind).look_for.len() > 80, "{kind:?}");
        }
        assert_eq!(LIBRARY.len(), ALL.len());
    }

    #[test]
    fn the_kind_comes_from_the_app_and_the_window() {
        use ScreenKind::*;
        for (app, title, expected) in [
            (Some("Keynote"), Some("Membranes — Lecture 7"), Slide),
            (Some("Microsoft PowerPoint"), None, Slide),
            (Some("Google Chrome"), Some("Lecture 7 - Google Slides"), Slide),
            (Some("Safari"), Some("Budget 2027 - Google Sheets"), Table),
            (Some("Google Chrome"), Some("Syllabus - Google Docs"), Document),
            (Some("Preview"), Some("handout.pdf"), Document),
            (Some("Arc"), Some("Retro board | Miro"), Whiteboard),
            (Some("Freeform"), None, Whiteboard),
            (Some("zoom.us"), Some("Zoom Meeting"), Call),
            (Some("Microsoft Teams"), None, Call),
            (Some("Xcode"), Some("panel.rs"), Code),
            (Some("Microsoft Excel"), None, Table),
            (Some("Numbers"), None, Table),
            (Some("Safari"), Some("Cell membrane - Wikipedia"), WebPage),
            (Some("Finder"), None, Other),
            // A short name is a whole word, never part of another app's name
            (Some("Archive Utility"), None, Other),
            (Some("Brave Browser"), Some("New Tab"), WebPage),
            (Some("Code"), Some("main.rs"), Code),
            (None, None, Other),
            // A whole display: no app, the monitor's name as the title
            (None, Some("Built-in Retina Display"), Other),
            // The app decides before the title: a file name in a title
            // never turns an editor or a call into something else
            (Some("Xcode"), Some("report.pdf.swift"), Code),
            (Some("Visual Studio Code"), Some("budget.xlsx — notes"), Code),
            (Some("zoom.us"), Some("Sharing: deck.pptx"), Call),
            (Some("Keynote"), Some("talk.key"), Slide),
            // An extension is a whole extension
            (Some("Finder"), Some("notes.mdx"), Other),
            (Some("Safari"), Some("quarterly.pdf"), Document),
        ] {
            assert_eq!(kind_for(app, title), expected, "{app:?} {title:?}");
        }
    }

    #[test]
    fn private_screens_are_left_out_on_the_device() {
        for (app, title) in [
            // noFriction's own window
            (Some("noFriction"), None),
            // Password managers and keychains
            (Some("1Password 8"), None),
            (Some("Keychain Access"), None),
            (Some("Passwords"), None),
            (Some("Bitwarden"), Some("My vault")),
            // Mail and messages, whatever the window is called
            (Some("Mail"), Some("Re: budget.xlsx")),
            (Some("Messages"), Some("report.pdf")),
            (Some("Microsoft Outlook"), None),
            (Some("Slack"), Some("#general")),
            (Some("WhatsApp"), None),
            // Terminals
            (Some("Terminal"), Some("zsh")),
            (Some("iTerm2"), None),
            // Private browsing
            (Some("Safari"), Some("Private Browsing — Start Page")),
            (Some("Google Chrome"), Some("New Incognito Tab")),
            // A browser tab on mail, a password page or a bank
            (Some("Google Chrome"), Some("Inbox (3) - someone@example.com - Gmail")),
            (Some("Safari"), Some("Sign in — Example")),
            (Some("Arc"), Some("First Bank — Accounts")),
            // Files that hold keys, in any app
            (Some("Visual Studio Code"), Some("server.key")),
            (Some("TextEdit"), Some("deploy.pem")),
            (Some("Xcode"), Some(".env — api")),
            (Some("Finder"), Some("login.keychain")),
            (None, Some("id_rsa")),
        ] {
            assert!(never_sent(app, title), "{app:?} {title:?}");
        }
        for (app, title) in [
            (Some("Keynote"), Some("talk.key")),
            (Some("Keynote"), Some("Membranes — Lecture 7")),
            (Some("Google Chrome"), Some("Lecture 7 - Google Slides")),
            (Some("Preview"), Some("handout.pdf")),
            (Some("Xcode"), Some("panel.rs")),
            (Some("zoom.us"), Some("Zoom Meeting")),
            // Whole words only: not every title with "bank" or "line" in it
            (Some("Preview"), Some("Riverbank ecology.pdf")),
            (Some("Safari"), Some("Timeline of the cell - Wikipedia")),
            (Some("Safari"), Some("Environment — Wikipedia")),
            // A whole display can't be recognised here
            (None, Some("Built-in Retina Display")),
            (None, None),
        ] {
            assert!(!never_sent(app, title), "{app:?} {title:?}");
        }
    }

    #[test]
    fn every_prompt_carries_the_shared_rules_and_the_one_answer_shape() {
        for kind in ALL {
            for recording in [RecordingKind::Class, RecordingKind::Meeting, RecordingKind::Personal] {
                let p = prompt_for(*kind, recording);
                assert!(p.contains(entry(*kind).look_for));
                assert!(p.contains("Use only what is visible in the picture"));
                assert!(p.contains("Never describe a person's appearance"));
                // The rules that protect someone are in every prompt, because a
                // whole-display capture gets the `Other` prompt whatever it shows
                assert!(p.contains("Never describe or name the people in video tiles, and never read the chat."), "{kind:?}");
                assert!(p.contains("write [secret] in its place"), "{kind:?}");
                assert!(p.contains("{\"skip\": true}"));
                assert!(p.contains("\"points\": ["));
            }
        }
        assert!(prompt_for(ScreenKind::Slide, RecordingKind::Class).contains("one screen that was shown during a student's recording of a class"));
        assert!(prompt_for(ScreenKind::Photo, RecordingKind::Class).contains("one photo that was taken during"));
        // The prompt is the same for every recording of a type: nothing of
        // the recording is in it
        assert_eq!(prompt_for(ScreenKind::Slide, RecordingKind::Meeting), prompt_for(ScreenKind::Slide, RecordingKind::Meeting));
    }

    #[test]
    fn the_kind_an_answer_names_is_read_back() {
        assert_eq!(ScreenKind::from_answer("slide"), Some(ScreenKind::Slide));
        assert_eq!(ScreenKind::from_answer(" Web page "), Some(ScreenKind::WebPage));
        assert_eq!(ScreenKind::from_answer("webpage"), Some(ScreenKind::WebPage));
        assert_eq!(ScreenKind::from_answer("web_page"), Some(ScreenKind::WebPage));
        assert_eq!(ScreenKind::from_answer("screen"), Some(ScreenKind::Other));
        assert_eq!(ScreenKind::from_answer("Whiteboard"), Some(ScreenKind::Whiteboard));
        assert!(ScreenKind::Photo.ai_decides() && ScreenKind::Other.ai_decides() && !ScreenKind::Slide.ai_decides());
        assert_eq!(ScreenKind::from_answer("a photo of a cat"), None);
    }
}
