// noFriction - the prompt library for reading a screen
// (docs/design/FADELL_V2.md §12, docs/AI_PROVIDERS.md "Read the screens").
//
// When the user presses Read the screens, each chosen screen goes to their
// AI as one picture with one prompt. Which prompt depends on what kind of
// screen it is, and that is decided here, on the device, from the app and
// window the screen came from: no extra request is made to classify it.
//
// Every prompt asks for the same JSON, so one parser reads every answer:
//   {"skip": false, "kind": "slide", "title": "...", "points": ["..."],
//    "terms": [{"term": "...", "meaning": "..."}], "text": "..."}
//
// The library is data. To add a kind: add a `ScreenKind`, its entry in
// `LIBRARY`, and the apps that show it in `kind_for`. The shared rules are
// added to every prompt by `prompt_for`; an entry only says what to look
// for on that kind of screen.

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
            ScreenKind::Other => "Screen",
        }
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
        when: "Xcode, VS Code, a JetBrains editor, Terminal, iTerm",
        look_for: "This is source code or a terminal. Give the file or command being shown as the title. As points, say what the \
visible code does, function by function, and any error or output message. Put the few lines that matter most, copied exactly, \
in \"text\". Never include anything that looks like a password, key or token: write [secret] in its place.",
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
title and its content as points. Ignore the participants: never describe or name the people in video tiles, and never read \
the chat. If nothing is being shared, answer with \"skip\": true.",
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

/// An app's name as words between single spaces (" visual studio code "),
/// so "arc" matches Arc and not Search, and "word" not 1Password.
fn words(name: &str) -> String {
    let spaced: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    format!(" {} ", spaced.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn is_app(app_words: &str, names: &[&str]) -> bool {
    names.iter().any(|n| app_words.contains(&format!(" {} ", n)))
}

/// What kind of screen this is, from the app and window it came from.
/// Decided on the device; nothing is sent to find out.
pub fn kind_for(app_name: Option<&str>, window_title: Option<&str>) -> ScreenKind {
    let app = app_name.unwrap_or("").trim().to_lowercase();
    let title = window_title.unwrap_or("").trim().to_lowercase();
    // What a browser tab or a shared window is showing wins over the app
    if has(&title, &["google slides", "- slides", "powerpoint", ".pptx", ".key"]) {
        return ScreenKind::Slide;
    }
    if has(&title, &["google sheets", "- sheets", ".xlsx", ".csv", ".numbers"]) {
        return ScreenKind::Table;
    }
    if has(&title, &["google docs", "- docs", ".docx", ".pdf", ".md"]) {
        return ScreenKind::Document;
    }
    if has(&title, &["miro", "figjam", "excalidraw", "whiteboard", "jamboard"]) {
        return ScreenKind::Whiteboard;
    }
    let app = words(&app);
    if is_app(&app, &["keynote", "powerpoint"]) {
        ScreenKind::Slide
    } else if is_app(&app, &["miro", "figjam", "freeform", "excalidraw", "whiteboard"]) {
        ScreenKind::Whiteboard
    } else if is_app(&app, &["zoom", "teams", "meet", "facetime", "webex"]) {
        ScreenKind::Call
    } else if is_app(&app, &["xcode", "visual studio code", "code", "terminal", "iterm", "iterm2", "intellij", "pycharm", "webstorm", "rustrover", "cursor", "zed"]) {
        ScreenKind::Code
    } else if is_app(&app, &["excel", "numbers", "airtable"]) {
        ScreenKind::Table
    } else if is_app(&app, &["word", "pages", "preview", "acrobat", "notion", "obsidian", "notes", "textedit", "books"]) {
        ScreenKind::Document
    } else if is_app(&app, &["safari", "chrome", "firefox", "arc", "brave", "edge", "opera", "vivaldi"]) {
        ScreenKind::WebPage
    } else {
        ScreenKind::Other
    }
}

/// What every prompt ends with: the rules that hold for any screen.
const SHARED_RULES: &str = "Rules:\n\
- Use only what is visible in the picture. Never guess at text that is too small, cut off or blurred.\n\
- Copy formulas, numbers, dates, names and quoted wording exactly as shown.\n\
- Never describe a person's appearance, and never name a person from their face.\n\
- If the picture shows something private that is not the subject of the recording (passwords, keys, messages, email, \
banking, medical records), answer only {\"skip\": true}.\n\
- If there is nothing worth keeping (a blank screen, a desktop, a loading page), answer only {\"skip\": true}.\n\
- At most 8 points, each one sentence. At most 6 terms.\n\
\n\
Answer with JSON only, in this shape:\n\
{\"skip\": false, \"kind\": \"slide\", \"title\": \"the title, or null\", \"points\": [\"one statement from the screen\"], \
\"terms\": [{\"term\": \"a term the screen defines\", \"meaning\": \"its meaning as the screen gives it\"}], \
\"text\": \"wording that must be kept exactly (a formula, a few lines of code), or null\"}";

/// The whole prompt sent with one picture. `said` is what was being said
/// while the screen was shown (it helps the AI tell what matters; the
/// answer must still come from the picture).
pub fn prompt_for(kind: ScreenKind, recording: RecordingKind, said: &str) -> String {
    let who = match recording {
        RecordingKind::Class => "a student's recording of a class",
        RecordingKind::Meeting => "a recording of a meeting",
        RecordingKind::Personal => "a personal recording",
    };
    let mut s = format!(
        "You read one screen that was shown during {who}, so its content can be kept with the notes.\n\n{look}\n\n",
        who = who,
        look = entry(kind).look_for
    );
    let said = said.trim();
    if !said.is_empty() {
        s.push_str(&format!(
            "What was being said while it was shown (for context only; report what the picture shows, not this):\n{}\n\n",
            said
        ));
    }
    s.push_str(SHARED_RULES);
    s
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
            (Some("Terminal"), Some("zsh"), Code),
            (Some("Microsoft Excel"), None, Table),
            (Some("Numbers"), None, Table),
            (Some("Safari"), Some("Cell membrane - Wikipedia"), WebPage),
            (Some("Finder"), None, Other),
            // A short name is a whole word, never part of another app's name
            (Some("Archive Utility"), None, Other),
            (Some("1Password"), None, Other),
            (Some("Brave Browser"), Some("New Tab"), WebPage),
            (Some("Code"), Some("main.rs"), Code),
            (None, None, Other),
        ] {
            assert_eq!(kind_for(app, title), expected, "{app:?} {title:?}");
        }
    }

    #[test]
    fn every_prompt_carries_the_shared_rules_and_the_one_answer_shape() {
        for kind in ALL {
            for recording in [RecordingKind::Class, RecordingKind::Meeting, RecordingKind::Personal] {
                let p = prompt_for(*kind, recording, "The pump moves three sodium out.");
                assert!(p.contains(entry(*kind).look_for));
                assert!(p.contains("Use only what is visible in the picture"));
                assert!(p.contains("Never describe a person's appearance"));
                assert!(p.contains("{\"skip\": true}"));
                assert!(p.contains("\"points\": ["));
                assert!(p.contains("for context only"));
                assert!(p.contains("The pump moves three sodium out."));
            }
        }
        // Nothing was said: no empty context block
        let p = prompt_for(ScreenKind::Slide, RecordingKind::Class, "  ");
        assert!(!p.contains("for context only"));
        assert!(p.contains("a student's recording of a class"));
        // A call is read for what is shared, never for who is in it
        let call = prompt_for(ScreenKind::Call, RecordingKind::Meeting, "");
        assert!(call.contains("never describe or name the people in video tiles"));
    }

    #[test]
    fn the_kind_an_answer_names_is_read_back() {
        assert_eq!(ScreenKind::from_answer("slide"), Some(ScreenKind::Slide));
        assert_eq!(ScreenKind::from_answer(" Web page "), Some(ScreenKind::WebPage));
        assert_eq!(ScreenKind::from_answer("webpage"), Some(ScreenKind::WebPage));
        assert_eq!(ScreenKind::from_answer("web_page"), Some(ScreenKind::WebPage));
        assert_eq!(ScreenKind::from_answer("screen"), Some(ScreenKind::Other));
        assert_eq!(ScreenKind::from_answer("a photo of a cat"), None);
    }
}
