# Changelog

All notable changes to noFriction are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

## [Mac 4.0.0 (49) · iPhone and iPad 2.0.0 (14)] - 2026-11-13

Built and uploaded to TestFlight on 2026-11-13; not yet submitted for
review (docs/RELEASE_STATUS.md). The v2 work (plan and reasons:
docs/design/FADELL_V2.md), on the Mac and on iPhone and iPad. An entry
marked (Mac) or (iPhone) is on that platform only.

### Added
- **Fill in the blanks.** One button in a recording's Notes. It reads the
  whole recording again beside the notes you have and adds **What you
  missed** (things that were said and aren't in the notes, each with the
  time it was said) and **More detail** (the notes' key ideas explained
  further). For a Class it may also add what a textbook says, and marks
  each such entry "Background, not said in the recording". Your own lines
  are never rewritten. It runs only when you press it, and is part of Notes
  (noFriction Pro).
- **Type a note while it listens.** One field under the live transcript;
  Return keeps the line with its moment and it appears in the transcript
  there. Notes opens with **What you wrote**. On the Mac, Rewind shows
  typed notes inline and as pins under the scrubber. A typed note is stored
  beside the marks (kind `note`), so Export, the review guide, Chat and
  sync handle it (on the Mac, time-range Delete too); it is never a fourth
  mark kind.
- **Type a note from any app (Mac).** **⌃⌥⌘N** opens one field over
  whatever is in front; Return keeps the line at that moment and the field
  is gone. The app in front stays in front. **File → Type a Note** and the
  menu-bar icon's **Type a Note** open the same field. With nothing
  recording it says so and takes no keys; it never starts a recording.
- **A notebook's page.** **Due** / **Action items** / **To-dos** from every
  recording's notes with a tick each, **Marked** (every ✎ and ?), **Review
  everything** (one deck and one quiz from every guide in the notebook),
  **Record** and **Ask**. No AI is called; ticks are kept as hashes only.
  Mac: picking a Notebook chip in Recordings opens it. iPhone and iPad:
  pick the chip and tap **Open BIO 101** above the list, or **Open** beside
  the notebook on a recording.
- **Up next.** When a calendar event is starting, a notebook you usually
  record is due, or (Mac) a call app takes the microphone, the Record
  screen shows one card whose button starts it with the answers filled in.
  **Remind me when something starts** (on) sends it as a notification.
  Mac: **Record by itself** (off) starts the recording, and the tray's
  first item starts what is up next. iPhone: the reminders are for the
  notebooks you usually record, planned ahead so they arrive with the app
  closed; the Record tab asks once before iOS asks for notifications.
- **Start Recording shortcut (iPhone).** Siri, the Shortcuts app and the
  Action button open noFriction and start a recording with your last type
  and length.
- **Making notes…** A live state in Notes while automatic notes are
  written. Mac: a **Notes are ready** notification when they finish and the
  app isn't in front.
- **Ticks in Notes.** Each action item, deadline or to-do in a recording's
  Notes has the same tick as on its notebook's page.
- **Still open from last time.** The Up next card lists what the suggested
  notebook's notes still have open.
- **Review everything from one recording on.** **Start from** begins the
  deck and quiz at any recording ("since the midterm").

### Changed
- **Notes read the whole recording.** On the Mac they were written from the
  first 8,000 characters of the transcript (about ten minutes of speech);
  on iPhone a long recording lost its middle. A long recording is now read
  in parts, the way the review guide is, and the prompt lists what you
  marked and typed; each typed note gets a line under it saying what was
  being said then.
- **Stop opens the recording on Notes**, not Rewind (Mac) or the Record tab
  (iPhone).
- **Automatic notes** also run for a recording of a minute or more that has
  a typed note or a mark (six minutes otherwise). They are new on iPhone
  and iPad: **Settings → Make notes automatically** (on), for Pro with an
  AI connection that is set up and already allowed.
- **Which calendar event a recording belongs to (iPhone).** The event that
  is on now, the most recently started when several are, else one starting
  within 15 minutes. It was the event with the most overlap, so a long
  block could hide the meeting inside it.
- Mac: buttons, fields and chips use the app's typeface instead of the
  browser's control font; a recording's row shows the time under its day
  heading; the loading spinner uses the palette and stays still under
  Reduce Motion.
- Deleting a typed note has Undo on the Mac, like every other delete; on
  iPhone it asks first.
- Mac: the typeface is the system face on every Mac. The app named Inter
  but never shipped it, so it showed only where someone had installed it.
  (The design rules now also say the 44px target is for iPhone and iPad;
  the Mac keeps desktop control sizes. No control changed size.)

### Fixed

On the Mac, unless marked.

- **Undo on a delete could be skipped.** Where a screen passed its error
  handler inline, the Undo toast applied the delete on the next redraw
  instead of after five seconds (deleting a recording, a chat or a typed
  note). The toast now keeps its timing whatever the screen does.
- **Notes are saved in one step with their check.** A Delete or Strike that
  landed between "the transcript is unchanged" and the save could leave
  notes made from the removed text. The check and the save are now one
  database transaction.
- **Automatic notes wait for the recording to stop.** If another recording
  (or the next part of a long one) had started, the automatic run could
  call the AI while recording. It now waits.
- **A typed note's detail goes with the note.** Deleting a typed note left
  what the notes wrote under it in the database; it is now removed in the
  same step, and the notes are sent to the other device again.
- A note typed just before Stop is kept on that recording, stamped with the
  moment Return was pressed.
- Two quick clicks on **Record by itself** or **Remind me when something
  starts** can no longer be stored out of order.
- **Ask** on a notebook's page opens Chat on that notebook (it opened on
  the recording that was selected).
- A short recording that starts and stops between two looks at the
  microphone no longer leaves its call to be offered again; "Recording …"
  is posted only once the recording runs.
- Notes made from a transcript that was edited while they were being
  written are no longer saved (Mac and iPhone).
- The notes' "Made" time was the time the view was opened, not when they
  were made.
- A failed notes run no longer puts the model's answer in its error text.
- A recording is no longer named after an all-day calendar event ("Out of
  office", a birthday), and when two events overlap the one that started
  most recently is matched, not the longer block around it.
- A note typed while the previous one was still being kept is never lost.
- **No screenshot can be sent to an AI in the background (Mac).** An old
  analyzer could send one screenshot per request, up to ten every two
  minutes, recording or not, if an admin switch from an early version was
  still set in the database. No screen could turn it on any more. The
  timer, its queue and its three unused commands are removed, and the
  release check now lists every file allowed to send an image.
- **A Meeting's notes are written the same way however they are made
  (Mac).** **Make notes** could pick up an old "prospecting" prompt while
  automatic notes used the normal one.
- **Nothing automatic asks the AI while a recording runs (Mac).** The
  topics that follow notes had no check, and automatic notes for a long
  recording only checked before their first request. Each request now
  waits for the recording to stop.
- The consent dialog said it sends "selected screenshots". No feature sends
  a picture; it now says so.
- **Make notes doesn't send text that is about to be deleted.** A run
  started inside a Delete's five-second undo window read the transcript as
  it still was. It now waits to be asked again (Mac and iPhone).
- iPhone: **Make notes** is refused for a recording that is still running
  (it could be opened from People); a quiz answer number far out of range
  in a model's reply no longer stops the app.
- Up next's usual length no longer creeps up. A recording that stopped at
  its 60-minute limit ends a second or two after the hour; counted as 61
  minutes it was offered as 90 the next time, then with no limit.

## [Mac 3.9.0 (47) · iOS 1.1.0 (12)] - 2026-10-23

Export… and one-step AI connect are on both platforms (iOS 1.1.0 and Mac
3.9.0); Make lesson and ⌘E are Mac only.

### Added
- **File → Export Recording… (⌘E)** on the Mac opens Export… for the open recording.
- **Make lesson (Mac; docs/LESSONS.md §13).** The Lessons "coming soon"
  preview is now the real thing: **Make lesson** on a Class recording in
  Recordings, or in Lessons with a Class-recording picker, turns the
  transcript into a playable course with your own AI (Apple on-device or your
  endpoint): a concept graph, one to four lessons with plain-text
  explanations, worked examples in words, common mistakes and a
  multiple-choice checkpoint, and a final exam that also draws on the
  checkpoints. Each part is asked for, parsed tolerantly, checked by the
  lessons validators and asked for once more if unusable; a cyclic concept
  graph or a reference to a missing concept is repaired, and a part that
  still fails is reported while the rest is kept. No figures, no interactive
  simulators and no AI-computed number as an answer key (multiple-choice keys
  are resolved to an index). Long lectures are condensed to fit Apple's
  4K-token model. The lesson is stored as a `generated` set tied to the
  recording and is deleted by any Delete or Strike in it (live and backups,
  with its progress and certificate); making it again replaces it. Making a
  lesson is noFriction Pro (new `lessons` paywall key on Mac and iOS);
  taking lessons stays free and offline. Lesson bundles gain schema minor 1
  (`text` explainers, `answerText`).
- **Connecting AI is one step (Mac and iPhone; docs/AI_PROVIDERS.md
  "Connecting").** **Sign in with OpenRouter**: the browser opens, you sign
  in, and the key comes back on its own (OAuth PKCE over a loopback callback);
  one account for ChatGPT, Claude, Gemini, Grok, Mistral and Llama. Key
  services (ChatGPT, Claude, Muse, Grok, Mistral): **Get a key**, then **Paste
  key and connect**, which saves, checks with one word and asks for permission
  in one go; the key goes from the clipboard to the Keychain without being
  shown. **Find AI on this Mac** finds Ollama, LM Studio, Jan or llama.cpp.
  Failures say what to do next. OpenRouter is a curated preset again (owner
  decision 2026-10-22).
- **Export… a recording as a package (Mac and iPhone; docs/EXPORT.md).** A
  zip or folder with the recording as Markdown and as a printable web page,
  the photos and screens you pick (with captions), marks, links, the
  follow-up email, the review guide and flashcards, and the transcript. AI
  tools fill it in: **Find highlights** (key moments quoted word for word from
  the transcript; invented quotes are dropped), **Pick with AI** and **Write
  captions** for the pictures, **Make notes**, **Draft email**, and **Fill
  with AI** for whatever is missing. Exporting is free; the AI tools are Pro.
  Nothing the AI writes in the sheet is stored, and stricken text exports as
  the marker. The iPhone's share button becomes a menu: Export… and Share as
  Text.

### Fixed
- Rewind's transcript lines report how long each was spoken (it was always 0).
- Two unused live-intel commands no longer write insight or decision text to the log.

## [Mac 3.8.0 (46) and earlier unreleased work] - 2026-10-21

### Fixed
- **Mac App Review 2.5.1 (private API).** The Mac binary no longer loads
  `MobileDevice.framework`. The ScreenCaptureKit fork of `cpal` pulled in
  `cidre` with its default `full` feature set, whose `am` module links that
  private framework even though nothing calls it. `cpal` is now vendored at
  `src-tauri/vendor/cpal` (Kree0 fork, commit e851e41) with `cidre` limited
  to the ScreenCaptureKit set (`sc`, `av`, `cv`, `cg`, `cat`,
  `dispatch`, `blocks`, `async`). Both release scripts now refuse a
  binary with any `/System/Library/PrivateFrameworks/` load command.
- **Mac App Review 5.1.1(iv) (permission pre-prompts).** The Setup screen's
  Microphone, Screen & System Audio and Calendar buttons say **Continue**
  instead of **Allow**, and the screen's own button is **Done**. The iPhone
  Permissions page gets the same change before Apple asks.

### Added
- **Lessons — interactive education (Mac 3.8.0).** A new fourth top-bar view
  (Record / Recordings / Lessons / Chat, ⌘1–4) that ships a six-lesson
  Stoichiometry course at AP Chem / college level: interactive explainers
  (equation balancer, mole-ratio and dimensional-analysis builders, a
  limiting-reagent simulator, a titration-curve explorer, an ideal-gas PV
  explorer, molecule diagrams), AI-authored figures embedded in the bundle,
  step-reveal worked examples, a common-mistakes explainer, an 8–10 item
  checkpoint per lesson, per-concept mastery with spaced-repetition review and
  adaptive trainers, a cumulative 30-question exam (80% to pass, unlocked after
  all six checkpoints), a mastery dashboard with an exam-readiness meter, and a
  printable offline certificate with a local verification hash. Lessons are
  portable `.lesson` bundles (zip + hashed manifest) you can also import from a
  file, folder, or a user-pasted URL. Taking a lesson is **100% offline and
  free**; grading is authoritative in Rust and no AI/image call is made for a
  present lesson. A visible **Generate** button previews the coming-soon
  recording→lesson pipeline (it makes no AI call this build). The iOS render
  layer is a documented follow-up; the data and correctness layer is shared
  Rust. See `docs/LESSONS.md` and `docs/LESSONS_AUTHORING.md`.
- **Apple Watch app (iOS 1.0.0, next build).** Record a meeting on the watch
  with one tap: elapsed time, live level, Pause/Resume, Stop. It keeps
  recording with the wrist down; a call or Siri pauses it until you tap
  Resume (watchOS doesn't let apps restart recording in the background). The
  audio goes to the iPhone over Apple's watch connection and is deleted from
  the watch once the iPhone has it. The iPhone imports it as a normal
  meeting: transcribed on the iPhone, on the device, matched to your
  calendar, with AI notes, follow-up email, Delete / Strike, export and
  People. Long recordings are transcribed in chunks and continue where they
  stopped if the app is closed. See `docs/WATCH_APP.md`.

### Changed
- **Meetings, classes and everyday life (Mac).** Record asks **What is it?**
  (Meeting · Class · Personal, remembered; Meeting by default) above **How
  long?**. "Class" is now an optional **Notebook** for any type, with
  **Notebooks** filter chips. Notes follow the type: meeting notes, lecture
  notes, or (Personal) a summary, key points and to-dos. The third mark is
  ✎ Follow up / On the test / Remember, the STUDY tab is **REVIEW** (a Review
  guide, or Study guide for a class), and the school-policy notice shows on
  the first Class recording. Untitled recordings are named "BIO 101 — Oct 7"
  or "Personal — Oct 7". Recordings that had a class become Class recordings
  (`meetings.recording_kind`, backfilled once). UI text that meant any
  recording now says "recording". See `docs/TIMED_RECORDING_AND_NOTEBOOKS.md`.

---

## [3.6.0] - 2026-10-01

Prepared for TestFlight and App Store review on iPhone, iPad and Mac. AI runs
on Apple's on-device model or on an endpoint you set up yourself, transcription
stays on the device, and nothing goes through a noFriction server.

### Added
- **Choose where AI runs.** Use Apple's on-device model (iOS/macOS 26 or later
  with Apple Intelligence; no key, nothing leaves the device), or enter your
  own OpenAI-compatible endpoint: its base URL, the model and, only if it needs
  one, your key. Local servers such as Ollama or LM Studio work. noFriction
  supplies no AI service, model or API key, and nothing remote is set up by
  default. A key you enter stays in the Keychain, tied to that endpoint. The
  app asks before sending meeting content to a public endpoint, and shows
  where it will go.
- **Delete and "Strike from the record."** Remove words, whole lines or
  screens from a meeting:
  - **Delete** leaves no trace and has 5 seconds of undo.
  - **Strike** destroys the content permanently and leaves a marker with the
    time, the date and an optional reason.
  - Removed content is purged from the transcript, search, recorded audio
    (iPhone), screenshots and screen video, AI notes, exports, app backups and
    database free space.
- **Recordings stop when the meeting ends.** The app notices when Zoom, Teams,
  Meet or FaceTime lets go of the mic, the call window closes, the calendar
  event is over, or nobody has spoken for a few minutes. A 30-second banner
  lets you keep recording.
- **iPhone and iPad app.**
  - Settings tab.
  - Subscription (noFriction Pro, monthly or yearly) through the App Store.
  - Recording-consent notice.
  - Privacy manifest.
- **Mac App Store build** (sandboxed): `scripts/release-mas.sh`, with StoreKit
  and Apple on-device AI through a small Swift bridge.
- **Privacy policy and support pages** in `site/`.
- **New app icon.**

### Changed
- **Client-only.** Supabase, Pinecone and the ingest server were removed.
  Search and "chat with your meetings" now run on the local full-text index.
- **Transcription is on-device only:** local Whisper on the Mac, Apple speech
  recognition on iPhone and iPad. The cloud transcription options (Deepgram,
  Gladia, Google, Gemini) were removed.
- AI services chosen by name in earlier versions are no longer available; AI
  asks you to choose Apple on-device or enter an endpoint. Meetings are kept.
- The app's identifier is now `com.nofriction.meetings` on every platform. The
  Mac data folder moves on first launch, and nothing is deleted.

### Fixed
- **No more "bye bye bye" loops.** Whisper's invented filler on silence or
  noise is filtered out.
- **Meetings are marked as ended when you stop recording.** Before, they never
  were, so automatic reports never ran. Meetings left open by earlier versions
  are closed on launch.
- Fresh installs could randomly hit "no such table" right after setting up the
  database.
- Screen-recording permission prompts no longer repeat.
- Release builds refuse to ship a broken code signature, the cause of the
  repeated mic and screen permission prompts.
- Transcripts are no longer written to the log file.

---

## [2.7.0] - 2026-02-13

### Added

#### LiveIntelAgent v2 — Smart Real-Time Meeting Intelligence
- **8 event types** (was 6): ActionItem, Decision, RiskSignal, Commitment, QuestionSuggestion, TopicShift, **KeyInsight**, **Deadline**
- **Smart pattern matching**: Strong vs weak patterns — weak signals require 2+ co-occurring matches to fire
- **30-second cooldown** per event type to prevent insight spam
- **4-word minimum** segment guard — skips filler ("yeah", "uh huh") fragments
- **Hash-based deduplication** across rule-based and AI extraction paths
- **Deadline extraction**: 28 temporal patterns ("by Friday", "end of quarter", "ship by", etc.)
- **Sentiment tracking**: Exponential weighted average from 40+ signal words (-1.0 to 1.0)
- **Meeting energy score**: WPM rate + speaker diversity → 0-100 composite score

#### AI Integration Improvements
- AI prompt now includes live conversation state (current topic, sentiment, energy, speakers)
- AI response parsing for `key_insights` → proper `KeyInsight` event type
- AI response parsing for `deadlines` → `Deadline` events with owner and reference
- `MeetingStats` struct tracks aggregate counts of segments, words, speakers, and event types

### Changed
- Default AI model upgraded from `qwen2.5-coder:7b` to `qwen3:8b` (Sage — deep reasoning model)
- Serendipity API key updated
- `MeetingIntelPanel.tsx` icons: 💡 for key_insight, 📅 for deadline, ❓ for question_suggestion
- Meeting export (stop_recording) now includes Key Insights and Deadlines markdown sections

### Fixed
- `KeyInsight` events were incorrectly mapped to `QuestionSuggestion` — now use dedicated type
- Dead code warning on `AiRisk.type` field suppressed with `#[allow(dead_code)]`

---

## [2.6.0] - 2026-02-11

### Added

#### Prompt Studio — Master Prompt Management UI
- **🧠 PROMPTS tab** in Agency view with split-pane editor
- **Category filtering**: All, Intelligence, Meeting, VLM
- **Persona filtering**: Prospecting, Fundraising, Product Dev, Admin, Personal
- **Inline editor**: Edit name, description, system prompt, temperature, active toggle
- **Test Panel**: Collapsible input/response box — test prompts against AI inline
- **Duplicate & Delete**: Clone prompts for variants, delete custom prompts

#### Persona-Aware AI Pipeline
- **20 built-in prompts** seeded across 4 features × 5 personas
- **Prompt resolution**: All AI surfaces now resolve persona-specific prompts from PromptManager
- **Prompt fallbacks**: Graceful degradation when persona-specific prompt not found

#### LiveIntelAgent Hybrid AI Upgrade
- **Rule-based + AI-powered** dual-phase extraction for live meeting insights
- **`ai_analyze` method**: Sends transcript segments to AI with persona-specific system prompt
- **Structured parsing**: AI response parsed into ActionItems, Decisions, Risks, and KeyInsights
- **Graceful degradation**: Falls back to rule-based only if AI fails

#### New/Updated Tauri Commands
- `test_prompt` — Test any prompt with sample input from the UI
- `get_live_insights` — Now includes AI-powered phase for deeper insights
- `generate_catch_up`, `chat_with_data`, `generate_meeting_report` — All wired to PromptManager

### Changed
- HelpView updated with Prompt Studio documentation (navigation, how-to, tech spec)
- Meeting report prompt management moved from Settings to Prompt Studio
- CHANGELOG updated with full v2.6.0 entry

### Fixed
- CSS `background-clip` and `line-clamp` lint warnings in PromptStudio.css

---

## [2.5.0] - 2026-02-03

### Added

#### RAG (Retrieval Augmented Generation) Pipeline
- **Intelligent Data Access**: AI chat now searches your meeting history before responding
- **Context Cards**: Visual display of sources used in AI responses with confidence scores
- **Conversation Storage**: All Q&A pairs stored to Pinecone (vectors) and Supabase (structured)
- **RAG Toggle**: Enable/disable history search via UI toggle
- **History Quick Action**: One-click search across past meetings

#### New Tauri Commands
- `thebrain_rag_chat` - RAG-enhanced chat without storage
- `thebrain_rag_chat_with_memory` - RAG chat with automatic conversation storage
- `store_conversation` - Manual conversation storage
- `get_conversation_history` - Retrieve past conversations

#### Server-Side Updates (nofriction-intel)
- TheBrain OAuth token authentication in `vlm.py` and `llm.py`
- Token caching with automatic refresh on 401
- Environment variable configuration for credentials

### Changed
- `AIChat.tsx` - Integrated RAG toggle and context display
- `CopilotPanel.tsx` - Added RAG features and model selector

### Fixed
- RwLock guard issues across await points in async commands
- Old "Nano Banana" branding in release documentation

---

## [2.5.0-alpha] - 2026-01-25

### Added

#### Always-On Recording (v2.5 Major Feature)
- **Ambient Capture Mode**: Low-power background recording
- **Meeting Detection**: Auto-detect Zoom, Google Meet, Teams
- **Power Manager**: Battery-aware capture throttling
- **Privacy Filter**: Exclude sensitive apps from capture

#### New Modules
- `ambient_capture.rs` - Background capture service
- `meeting_trigger.rs` - Meeting app detection
- `power_manager.rs` - Battery optimization
- `privacy_filter.rs` - App exclusion rules
- `tray_builder.rs` - System tray enhancements
- `continue_prompt.rs` - Session continuation
- `interaction_loop.rs` - User interaction handling

#### New Commands
- `start_ambient_capture` / `pause_capture`
- `start_meeting_capture`
- `get_capture_mode`
- `get_always_on_settings` / `set_always_on_enabled`
- `get_running_meeting_apps`
- `check_audio_usage`
- `dismiss_meeting_detection`

### Changed
- Tray menu now shows capture mode status
- Settings UI includes Always-On configuration

---

## [2.1.0] - 2026-01-20

### Added

#### Admin Console (Management Suite)
- **Storage Management**: Visualize and manage recording storage
- **Recording Deletion**: Batch delete with preview
- **Audit Log**: Track all administrative actions
- **System Health**: Monitor app performance
- **Feature Flags**: Toggle experimental features

#### Native Text Extraction
- **Vision OCR**: macOS-native screen text extraction
- **Accessibility Extractor**: UI element parsing
- **Semantic Classifier**: Content categorization

#### Calendar Integration
- **macOS Calendar**: Read calendar events
- **Meeting Context**: Auto-associate recordings with events
- **Upcoming Meetings**: Show scheduled meetings

#### Prompt Management (Phase 2)
- **Theme-Specific Prompts**: Prompts organized by activity theme
- **Version History**: Track prompt changes
- **A/B Testing**: Compare prompt effectiveness

#### New Modules
- `admin_commands.rs` - Admin operations
- `audit_log.rs` - Action logging
- `data_editor.rs` - Learned data CRUD
- `storage_manager.rs` - Storage statistics
- `calendar_client.rs` - macOS Calendar API
- `semantic_classifier.rs` - Content classification
- `vision_ocr.rs` - Native OCR
- `accessibility_extractor.rs` - UI text extraction

#### New Components
- `AdminConsole.tsx` - System management UI
- `AuditLog.tsx` - Action history viewer
- `LearnedDataEditor.tsx` - Data editing
- `ToolsConsole.tsx` - Developer tools
- `VideoDiagnostics.tsx` - Capture diagnostics

### Changed
- Sidebar reorganized with admin section
- Settings split into multiple tabs

---

## [2.0.0] - 2026-01-10

### Added

#### Video Recording
- **Native Screen Recording**: Full video capture (not just frames)
- **Moment Pinning**: Bookmark important points
- **Frame Extraction**: Pull frames from video
- **Storage Management**: Video retention policies

#### VLM Scheduler
- **Batch Processing**: Queue frames for VLM analysis
- **Auto-Processing**: Configure automatic analysis intervals
- **Rate Limiting**: Prevent API overload

#### Activity Themes
- **Theme Tracking**: Track time spent per activity type
- **Theme-Specific Settings**: Different capture settings per theme
- **Today's Usage**: See theme time breakdown

#### Intelligence Pipeline
- **Ingest Queue**: Managed processing queue
- **Ingest Client**: nofriction-intel integration
- **Topic Clusters**: Group related content

### Changed
- Frame capture replaced with video recording
- VLM processing now batched and scheduled
- UI updated with video controls

### Deprecated
- Legacy frame dump functionality

---

## [1.5.0] - 2026-01-02

### Added

#### Realtime Transcription
- **Deepgram WebSocket**: Streaming speech-to-text
- **Speaker Diarization**: Who said what
- **Multi-Provider Support**: Deepgram, Gladia, Google STT

#### Combined Audio
- **System + Mic**: Capture both simultaneously
- **Audio Buffer**: Smooth audio handling

#### Prompt Library
- **Custom Prompts**: Create and edit AI prompts
- **Prompt Templates**: Variables and formatting
- **Import/Export**: Share prompts

#### Model Configuration
- **Multiple Models**: Select AI model per task
- **Local + Cloud**: Ollama and TheBrain support
- **Model Availability**: Check which models are ready

### Fixed
- Audio sync issues in long recordings
- Memory leak in frame extraction
- Transcript search performance

---

## [1.0.0] - 2025-12-28

### Added

#### Initial Release
- **Recording Engine**: Mic + screen capture
- **Transcription**: Deepgram integration
- **Rewind**: Visual timeline playback
- **AI Chat**: Ollama/TheBrain integration
- **Knowledge Base**: Pinecone vector search
- **Settings**: Comprehensive configuration
- **Setup Wizard**: First-run experience

#### Core Modules
- `capture_engine.rs`
- `database.rs`
- `transcription/`
- `ai_client.rs`
- `vlm_client.rs`
- `pinecone_client.rs`
- `settings.rs`

#### UI Components
- `App.tsx`
- `AIChat.tsx`
- `RewindGallery.tsx`
- `FullSettings.tsx`
- `SetupWizard.tsx`
- Plus 35+ supporting components

### Security
- Local-first data storage
- Encrypted at-rest
- Optional cloud sync

---

## Version History Summary

| Version | Date | Theme |
|---------|------|-------|
| 2.7.0 | 2026-02-13 | Smart Live Intel v2 + Qwen3 |
| 2.6.0 | 2026-02-11 | Prompt Studio + AI Intelligence |
| 2.5.0 | 2026-02-03 | RAG Pipeline + Always-On |
| 2.1.0 | 2026-01-20 | Admin Console + Calendar |
| 2.0.0 | 2026-01-10 | Video Recording + VLM |
| 1.5.0 | 2026-01-02 | Realtime Transcription |
| 1.0.0 | 2025-12-28 | Initial Release |

---

## Migration Notes

### Upgrading to 2.5.0

1. **Supabase Migration Required**
   Run the conversations table migration:
   ```sql
   -- See supabase/migrations/20260203_create_conversations_table.sql
   ```

2. **Environment Variables**
   For nofriction-intel, add:
   ```bash
   VLM_USERNAME=your_thebrain_username
   VLM_PASSWORD=your_thebrain_password
   ```

3. **RAG Toggle**
   RAG is enabled by default. Disable via the 📚 toggle if not using.

### Upgrading to 2.1.0

1. **Permissions**
   Grant Calendar access for meeting detection.

2. **Admin Console**
   Access via sidebar → Admin (requires local authentication).

---

## Roadmap

### Planned Features
- [ ] Conversation threading (group related Q&As)
- [ ] Query suggestions (auto-complete from history)
- [ ] Daily briefing generation  
- [ ] Mobile app companion
- [ ] Team sharing (multi-user knowledge base)
- [ ] Webhook integrations
- [ ] Custom model fine-tuning

### Known Issues
- `objc` crate warnings (cosmetic, does not affect functionality)
- Some unused imports in Rust code (scheduled for cleanup)

---

## Contributors

- noFriction AI Team
- Casey Potenzone

---

[2.7.0]: https://github.com/nofriction/meetings/compare/v2.6.0...v2.7.0
[2.6.0]: https://github.com/nofriction/meetings/compare/v2.5.0...v2.6.0
[2.5.0]: https://github.com/nofriction/meetings/compare/v2.1.0...v2.5.0
[2.1.0]: https://github.com/nofriction/meetings/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/nofriction/meetings/compare/v1.5.0...v2.0.0
[1.5.0]: https://github.com/nofriction/meetings/compare/v1.0.0...v1.5.0
[1.0.0]: https://github.com/nofriction/meetings/releases/tag/v1.0.0
