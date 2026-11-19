# Topics and Chat: shared spec (Mac + iOS)

Two AI features on top of recordings, with the same words on both platforms:

- **Topics**: what a recording was about, as 1–4 short noun phrases
  ("Q4 roadmap", "Mitosis"). Named by the AI when notes are made (and on
  demand with **Find topics**); the user can rename, remove and add. On the
  Mac a **Topics** chip row sits beside **Notebooks** in the recordings list
  and **Group by: Date · Notebook · Topic** regroups the list; on iOS topics
  are a search facet and live on the recording (Notebooks are the one
  filter, the list is by day).
- **Chat**: ask about your recordings. A scope at the top, **All recordings
  · this Notebook · this Topic · this recording**, decides which recordings
  are searched. Answers are Markdown with `[n]` citations; each citation
  opens that recording's transcript at the moment it quotes. The scope is
  shown with every answer.

Both use the user's AI exactly as notes do (Apple on-device or the one
user-entered endpoint, [AI_PROVIDERS.md](AI_PROVIDERS.md)): the same Pro
gate and the same consent dialog, no separate gate. Retrieval for Chat is
local: no network, no embedding service, no index outside the app's store.
Nothing here stores transcript or screen text anywhere the purge
([REDACTION.md](REDACTION.md)) doesn't reach.

The Mac section is written by the Mac implementation, the iOS section by
the iOS side. Rules both follow: 1–4 topics per recording, labels of at
most 40 characters and 6 words, generic words ("meeting") refused,
near-duplicate keys merged (equal, equal without spaces, one typo apart);
AI topics the user removed stay removed on a re-run; scope labels **All
recordings**, **Notebook · X**, **Topic · X**, **Recording · X**; chat
answers that drew on an edited or deleted recording are deleted and the
thread flagged, the user's questions kept.

## Mac

Code: `src-tauri/src/topics.rs` (schema, label and key rules, prompt,
validation, storage, commands; tests in `topics/tests.rs`),
`src-tauri/src/chat.rs` (threads, messages, the ask flow, purge, commands),
`chat/retrieval.rs` (scope → SQL filter, passages, budget packing),
`chat/prompt.rs` (system prompt, history, user message; tests in
`chat/tests.rs`). Front end: `src/lib/topicsLogic.ts` + `topics.ts`,
`src/lib/chatLogic.ts` + `chat.ts` (tests `*.test.ts`, `npm test`),
`src/components/TopicChips.tsx` (filter chips, Group by, row chips, the
Notes-tab editor), `src/components/chat/` (`RecordingsChat.tsx`,
`ScopePicker.tsx`, `ChatMarkdown.tsx`).

### Topics

- **Model.** Table `meeting_topics (id, meeting_id → meetings ON DELETE
  CASCADE, topic, topic_key, confidence, source 'ai'|'user', created_at,
  UNIQUE (meeting_id, topic_key))` and `meeting_topics_removed (meeting_id,
  topic_key)` for AI topics the user removed. Created in
  `topics::ensure_schema`, called from `DatabaseManager::run_migrations`
  on its one connection. `topic_key(label)`: lowercase, anything but
  letters and digits becomes a space (apostrophes dropped), simple plurals
  folded ("Cell Membranes" → `cell membrane`). `display_label`: cleaned
  with the study parser (`clean_text`: one line, no control characters, no
  list markers), quotes and trailing punctuation trimmed, ≤ 40 characters,
  ≤ 6 words, generic words refused, an all-lowercase label capitalized
  ("q4 roadmap" → "Q4 roadmap"; "BIO 101" stays).
- **Near-duplicates.** `near(a, b)`: equal keys, equal without spaces, or
  one edit apart for keys of 6+ characters that hold no digit ("q4
  roadmap" ≠ "q3 roadmap"). On save, `canonical` looks the key up among
  every key already stored (most used label first): a near key adopts
  that key *and* its label, so the chip row and the Chat scope see one
  topic. `index()` (`list_topics`) returns each key with its most used
  label and recording count, and each recording's topics, user topics
  first.
- **When.** `meeting_notes.rs` calls `topics::find_after_notes` after
  notes are saved (automatic notes after a recording and **Generate
  notes**/**Regenerate**): a background task with the same AI gates as
  the notes, failures logged, never surfaced. **Find topics** / **Find
  again** on the Notes tab calls `find_topics` directly; a recording
  being recorded is refused, one run per recording at a time (`RUNNING`).
- **Prompt and validation.** `system_prompt()` asks for
  `{"topics":[{"label","confidence"}]}`, 1–4 noun phrases of 1–4 words,
  nothing generic, nothing invented, stricken text never guessed. The
  input is `study::load_input` (the Review guide's: `[m:ss]` lines,
  stricken spans as `[stricken from the record]`, deleted words gone), a
  long transcript chunked with `study::prompt::chunk_lines` to the
  model's window ("Part i of n", at most 8 parts sampled across the
  recording) and the parts merged by `merge_parts` (near keys join, a
  topic named in several parts scores higher). One retry per request on
  an unusable answer; AI access errors (consent, Pro, no provider) stop
  the run and reach the UI as their machine-readable strings. `validate`
  extracts the first JSON value (fences, prose and think blocks
  stripped, trailing commas tolerated), accepts a bare array or an array
  of strings, cleans labels, clamps confidence (missing → 0.5), merges
  near-duplicates, keeps the best four. Candidates under 0.3 are not
  stored. Nothing from the answer is logged.
- **Saving (`save_ai_topics`).** `BEGIN IMMEDIATE`; the transcript
  fingerprint (`StudyInput::fingerprint`) and the pending-Delete check are
  re-run inside the lock, exactly as the Review guide does, so labels made
  from text deleted or stricken meanwhile are never written. Then the
  recording's AI rows are replaced; user rows stay; a candidate near a
  user topic or a removed key is skipped; AI rows fill up to 4 in all.
- **Editing (`set_meeting_topics`).** The Notes tab's editor sends the
  whole list; every label is stored as the user's (≤ 8, cleaned like the
  AI's, generic ones dropped, keys canonicalized so a spelling the store
  already has merges). Any topic missing from the list is remembered in
  `meeting_topics_removed`; a label added back clears that.
- **Recordings list** (`MeetingHistory.tsx`). **Topics** chips (All ·
  label · count) under the Notebooks chips; a chip filters the list
  (client-side, from the index). **Group by: Date · Notebook · Topic**
  (remembered in `localStorage` `nf.recordings.groupBy`): Date groups by
  day ("Today", "Yesterday", the date), Notebook by name (case folded,
  then "No notebook"), Topic lists a recording under each of its topics,
  biggest topic first, then "No topics" (`groupRecordings`). Each row
  shows up to two topic chips and "+n"; clicking one filters by it. The
  Notes tab shows the recording's topics with **edit** (rename, remove,
  add) and **Find topics**; user topics have a dashed outline.
- **Purge.** A Delete (when it commits) or Strike of transcript text
  deletes the recording's AI topics (`topics::purge_ai_for_meeting`, in
  `redaction::redact_ai_outputs`, live database and app backups; the
  preview says "n AI topics on this recording: deleted"); user topics
  stay. Screen-only edits keep them. Deleting the recording deletes every
  topic and its removed-key memory (`DatabaseManager::delete_meeting`,
  explicit, besides the cascade). See [REDACTION.md](REDACTION.md).

### Chat

- **Scope.** `Scope { kind: all|notebook|topic|meeting, value }`
  (`chat/retrieval.rs`), resolved to a `Filter` that becomes one SQL
  fragment (`… IN (SELECT id FROM meetings WHERE class_name = ? COLLATE
  NOCASE)`, `… IN (SELECT meeting_id FROM meeting_topics WHERE topic_key
  = ?)`, `= ?`) and a label. `ScopePicker` offers All recordings, This
  Notebook (recent notebooks), This Topic (the index), This recording
  (every recording, the one open in Recordings first); a new chat opens with
  the recording open in Recordings, else All. `chat_scope_summary` returns
  the label, count, recent titles, topics, notebooks and types of the
  scope for the header and the suggestions. Every answer stores and shows
  the scope it was made in.
- **Threads.** `chat_threads (id, title, scope_kind, scope_value, flag,
  created_at, updated_at)`, `chat_messages (id, thread_id → threads ON
  DELETE CASCADE, role, content, citations JSON, scope_label,
  created_at)`, `chat_message_sources (message_id, meeting_id)`: every
  recording whose passages went into the prompt for that answer, cited
  or not. The title is the first question (≤ 48 characters). **Chats**
  lists past threads (delete with the bin); **New chat** starts one on
  the next question. The last 8 messages of the thread go back as prior
  turns (4 in a window under 8K tokens, each clipped), without the
  passages; the system prompt limits citations to the numbers given now.
- **Retrieval (`retrieval::retrieve`, local only).** Transcript lines by
  FTS5 (`transcripts_fts MATCH` with the OR-ed terms of
  `database::fts_or_query`, bm25-ranked, scoped by the filter on
  `t.meeting_id`, rendered through `redaction::render_plain` so stricken
  spans read `[stricken from the record]` and placeholder-only lines are
  skipped), the newest saved notes per recording (summary, decisions,
  action items, key points, scored by the share of the question's terms
  they contain) and marker notes (same, with the marker's time). Every
  passage carries the recording's title, date, type, Notebook and time.
  `pack` ranks them, caps a recording at 4 passages when the scope spans
  several, drops duplicates, fits a character budget left beside the
  system prompt, scope, question, memory and answer
  (`study::prompt::body_budget`), 12 at most, and numbers them 1…n. Fewer
  than three hits ("Summarize my week") fall back to the newest scoped
  recordings' notes summary or lines sampled across their transcript (12
  for one recording, 3 each for several). No embeddings, no index outside
  SQLite, no network.
- **Prompt and answer.** `prompt::system_prompt` (answer only from the
  sources, cite as `[n]` right after the fact, say what's missing, Markdown
  without HTML or tables, stricken text never guessed) + memory + `SCOPE:
  … SOURCES: [n] Title — date, Type, Notebook X — said at m:ss / marked
  at m:ss / from the saved notes … QUESTION: …`. `max_tokens` ≤ 900,
  scaled down for small windows. One request through `study::Completer`
  (`StreamingCompleter` → `ai::complete_text_stream`: consent, endpoint
  policy, Pro in the Mac App Store build), streamed: the answer appears
  while it is written (see "Streamed answers" below). The finished answer
  is `clean_answer`ed (think blocks, a wrapping fence) and stored with `citations` = every passage sent (`Citation { n,
  meeting_id, title, timestamp_ms, excerpt, source }`). `ChatMarkdown`
  renders a Markdown subset (paragraphs, headings capped at h3, bullet and
  numbered lists, bold, code, fenced code) as React text nodes, never
  HTML; each `[n]` becomes a chip, and **Sources** under the answer lists
  them all. A chip calls `requestRecordingSeek(meeting_id, ms)`
  (`lib/navigation.ts`) and selects the recording; `InsightDeckView` takes
  the request when the recording is selected (or at once) and shows the
  moment through `RewindGallery`'s `seek` prop, like the quiz and Links.
- **Suggested questions.** `suggestedQuestions(scope, summary)`: up to
  four from the scope's titles, types, notebooks and topics ("What did we
  decide about Q4 roadmap?", "What's on the test for BIO 101?", "Summarize
  my week"); no AI call.
- **Purge (`chat::purge_for_meeting`).** Every assistant message with a
  `chat_message_sources` row for the recording is deleted and its thread
  flagged (`FLAG_REMOVED`, shown above the chat and in the list); a thread
  scoped to the recording is flagged too; user questions stay. Called in
  `redaction::redact_ai_outputs` (so a Delete that commits and any Strike
  of transcript text, live and in app backups; the preview says "n CHAT
  answers that drew on this recording: deleted") and in
  `DatabaseManager::delete_meeting`. Questions and answers are never
  logged.

### Streamed answers (2026-11-18)

The owner: "the AI is slow, it needs to be streaming text." An answer used
to appear only when the model had finished it. Measured on the owner's Mac
with Apple's on-device model (`cargo run --example stream_answer`, and the
ignored tests in `store.rs`): the first words come after about half a
second for a small prompt and 2.5 to 4 seconds for a full window; the whole
answer takes two to several times that. The wait before the first words is
the model reading the prompt, which streaming does not shorten.

- **Where.** Chat, the Ask panel, and the Ask card over other apps. Notes,
  the Review guide, topics, Fill in the blanks, lessons and Read the
  screens are not streamed: their answers are JSON the app parses whole.
- **Client (`ai/client.rs`).** `complete_stream` / `run_stream` send the
  same request as `complete` with `stream: true` and read server-sent
  events (`SseText`: OpenAI-compatible chunks and Anthropic Messages
  events; reasoning and thinking deltas are skipped). The same Pro gate,
  target resolution, consent, timeout, 4 MB cap and key redaction. A
  server that answers with plain JSON is read whole; one that refuses
  `stream` (a 400 naming it) is asked again without it. Apple's on-device
  model streams through `nf_apple_generate_stream` in the Swift bridge
  (`LanguageModelSession.streamResponse`).
- **The sink.** The caller's `Sink` gets the whole answer so far each time
  it grows, never a fragment, with think blocks hidden
  (`visible_so_far`). `chat::stream_sink` cleans it like the finished
  answer (`prompt::clean_partial`), lets it through a few times a second
  (40 ms for the window, 90 ms for the card) and stops at a purge (below).
- **To the window.** `chat_ask` and `chat_ask_recording` take a
  `requestId` from the UI and emit `chat_stream { requestId, text }`.
  `lib/chat.ts::withStream` listens for its own id; `StreamingAnswer`
  renders the text so far with the same Markdown renderer
  (`partialAnswer` closes a `**` or `` ` `` opened on the last line and
  holds back a `[1` that hasn't finished), a caret after the last word,
  and citation numbers that become chips when the stored turn replaces
  it. The command's result is still the stored turn; the event carries
  nothing that isn't in it.
- **To the card.** `quick_note.rs::partial_on_main` puts the answer so far
  on the card under the same conditions as the finished one (that
  question still with the AI, the card up for that recording, no purge).
- **A purge while it is written.** `purge_for_meeting` counts every purge
  of its library (`PURGES`, by database file). A sink made before it shows
  one empty answer (which takes the text off the window and the card) and
  nothing after, and `ask_as` does not keep or return the finished answer
  ("Text was removed from a recording while this was being answered…").
  Any purge in the library counts, not only one of a recording the answer
  drew on. The answer so far is in memory only and is never logged.
- **A stream that breaks.** A body that ends without a finish reason or an
  end marker is an error ("Couldn't reach your AI"), never half an answer
  kept as a whole one.
- **iPhone and iPad.** Chat streams the same way (see "Streamed answers"
  under iOS below).

### Ask (2026-11-17)

The owner: "make a chatbot available within the app that has the context
of the conversation. i want to be able to immediately ask about things
that are being talked about, the context of how they are being spoken of.
so share the needed info like who is speaking, the context the company the
notes".

- **Where.** A panel beside the live transcript while a recording runs
  (`AskPanel.tsx` in `RecordView`), with ⌘J to put the keyboard in it. The
  Chat tab's "This recording" scope answers the same way (`chat_ask` hands
  a single-recording scope to `ask_recording`), so the conversation
  started while recording continues there after Stop. In Chat it draws on
  that recording alone (`Mode.related = false`): the user narrowed it. An
  answer from the panel that used earlier recordings says so in its label
  ("Recording · X and earlier recordings").
- **Only when asked.** One request per question the user sends
  (`chat_ask_recording`). Nothing asks by itself, on a timer or on what is
  said. This is the one AI call the user can make about the recording that
  is running; the rule in CLAUDE.md says so.
- **What it is given** (`chat/live.rs`, put together on the Mac for each
  question):
  - the recording: title, type, Notebook, start time, and that it is still
    running and how far in;
  - its calendar event: the title, the place and the notes as they are in
    the calendar (a meeting link written there goes with them), and the
    people: name, company (never a personal mail domain), organiser, which
    one is the user, and how many of the user's recordings they are in. No
    attendee's email address is sent. (`people.notes` exists in the schema
    but nothing writes it, so it is not sent or promised.)
  - which side spoke a line, when that could be told: `(mic)` or
    `(computer audio)` (below). Never a name;
  - the user's typed notes and marks in this recording, with their times;
  - the last minutes of transcript, in order, the newest last, lines of
    one side joined into chunks: up to six minutes, or as much of them as
    half the model's room holds (about two minutes of fast speech on a 4K
    model). Always given, because "what did they just say?" has no words
    to search for;
  - earlier in this recording (everything before the first line given as
    recent): lines matched to the question (or spread over what came
    before, when nothing matches), and the notes already made from it:
    the summary, and the decisions, action items, points and filled-in
    lines that match;
  - from the panel only, earlier recordings in the same Notebook, or with
    the same people when one attendee is known to be the user (an address
    that is in most recordings doesn't count): the saved notes of the
    first three of them, and lines and notes matched to the question.
  Half of the room goes to the last minutes; the rest is shared by the
  user's notes, earlier lines and earlier recordings. It fits Apple's 4K
  on-device model (a test checks the newest line is still there).
- **The prompt.** Answer first, short. What was said comes only from the
  numbered sources, cited `[n]`. Asked what a term, name or company
  means: how it is being talked about here first (cited), then what it is
  in general, marked "(background, not said in the recording)", the same
  marking Fill in the blanks uses. Never guess who said something.
- **Which side spoke** (`audio_source.rs`). The microphone and the
  computer's audio are mixed for transcription, so a line doesn't say
  where it came from. The mixer notes how loud each source was (a number
  per audio buffer, a few minutes, in memory; no audio), and when a line
  is saved it is tagged `mic` or `system` in `transcripts.source` only
  when that is clear: one source was active and the other was being
  captured and was quiet. Both active (speakers on, or talking over the
  call) and a recording without the computer's audio get no tag. So tags
  are common with headphones and rare on speakers. It is a side, not a
  person: in a room, `mic` is everyone in the room. The column is shared
  with sync, whose `screen` value (audio of an iPhone app) is left as it
  is and not shown as a tag.
- **What it says about itself.** The panel's first line says what it
  knows right now (`chat_recording_knows`, no AI call), and **What it's
  given** lists everything above in plain words (`askLogic.ts`). Keep
  that list true.
- **Stored and purged like chat.** A thread scoped to the recording; each
  answer's sources include this recording (always, even when it was made
  from the people and the event alone) and any other it drew on, so
  deleting or striking any of them removes the answer. Stricken text is
  never in a source.
- **Not on iPhone or iPad yet.**
- **Not verified:** the side tags on a real call (the thresholds are set
  from reasoning, not from recordings), and answer quality with a real
  model during a real meeting.

#### Ask from any app (2026-11-18)

The owner: "enhance these capabilities". On a call the user is in Zoom,
Meet or their slides, not in the noFriction window, so the panel beside
the transcript is the one place they are not looking.

- **⌃⌥⌘A** (also **File → Ask About This Recording** and the menu-bar
  icon's item) opens the quick-note panel for a question
  (`quick_note.rs::toggle_ask`). It takes the keyboard without bringing
  noFriction forward, as the note panel does. Return sends the question;
  Return with nothing typed asks "What did I miss in the last few
  minutes?". If nothing is recording it says so and asks nothing.
- **Return gives the keyboard back.** The moment a question is sent the
  card stays on screen and the keyboard returns to the app in front
  (`panel::release_keyboard`, `PINNED`), so a Return or a ⌘ key meant
  for the call never reaches the card. The field shows what was asked
  (also for the default question).
- **The answer is a card** under the field (`panel::show_answer`): the
  panel grows downward to fit it, its top edge stays put, up to 300 pt
  (17 lines; longer ends in an ellipsis). ⌃⌥⌘A while it is up takes the
  keyboard for the next question and leaves the answer where it is;
  ⌃⌥⌘A again (or Esc, now that it has the keyboard) closes it. Left
  alone it closes two minutes after the answer; if the user is in it at
  that moment (copying a line) it closes when they leave it. An answer
  that lands after the card was closed and opened again for the same
  recording still shows; one that lands on a closed card, or on the note
  panel, is only in the app.
- **Only the recording that is running.** Return re-reads the recorder:
  if the card's recording has stopped, it says so and asks nothing, and
  the Ask key on an old card opens a fresh one for what is recording
  now.
- **A shorter answer** (`Mode.glance`): the same sources as the panel in
  the app, plus one rule in the prompt (at most three short lines or
  bullets, the most useful first) and 320 answer tokens instead of 700.
  The card shows it as plain text (`plain_answer`: no emphasis marks,
  bullets as "•", no source numbers because they can't be opened there,
  at most 900 characters). The stored message is the whole Markdown
  answer with its sources.
- **The same conversation.** The question goes to the recording's latest
  thread (`ask_recording`), so it is in the panel and in Chat with its
  sources; the panel refreshes on the `ask_answered` event. Purged like
  any chat message. The card itself keeps nothing: closing it empties it.
- **Consent, Pro and errors.** The card can't show the consent dialog or
  the paywall over another app, so `ask_error` says where to go
  ("Open noFriction and ask there once") and nothing is sent. An
  endpoint's own error text is never on the card or in the log.
- **One field, two uses.** The note key and the Ask key open the same
  panel. Pressing the other key changes it over (`press`). A note being
  typed is put aside when the panel opens for a question and comes back
  with the note key (`Session.note_draft`); a question is never left in
  the field as a note. While a note is being kept the Ask key never
  takes the field for a question (it closes an open panel), and while a
  question is with the AI neither key changes the panel over. A question
  typed and not sent is gone when the card closes.
- **Naming the other side on a two-person call** (`live::other_side`).
  When the event has exactly the user and one other person, and lines
  are tagged with a side, the context says the computer's audio is most
  likely that person and that an answer may name them, written
  "Name (likely)" the first time. It is worked out from the calendar,
  not heard, and "What it's given" says so (`askLogic.ts::otherSide`).
  With three people, nobody known to be the user, or no side tags, the
  rule is as before: a side, never a name. Applies to the panel and Chat
  as well as the card.
- **Checked:** `cargo run --example ask_card` puts the real card on
  screen without taking the keyboard and checks that it grows to fit,
  keeps its top edge, holds a long answer to the limit, and is empty once
  closed. An independent review of the state handling found the card
  could stay on "Asking your AI…" forever when reopened mid-question,
  stay pinned, keep the keyboard after answering, and ask about a
  recording that had stopped; all fixed before merge. With `KEYBOARD=1`
  the example also checks that the card takes the keyboard for the
  question, gives it up on Return while staying on screen, and that the
  answer lands without taking it back (17 checks). **Not verified:** the
  shortcut over a real call, and a question answered by a real model
  from the card. On a calendar that lists a room or drops a
  participant, a three-person call can look like two: that is why the
  name is always marked "(likely)".

### Tests (`topics/tests.rs`, `chat/tests.rs`, `*.test.ts`)

Rust (20): topic JSON shapes and the cap of four, malformed input never
panics, key folding and `near`, display labels, part merging, one request
for a short recording, chunked reading of a long one within a 4K window,
retry once and AI errors stop, cross-recording merge with user edits and
removed keys kept, no save after an edit (fingerprint) and no stricken
words in the prompt, purge on Strike (user topics kept) and meeting
delete with backups and old schemas tolerated; chat citation numbers,
answer cleaning and titles, scope parsing and filters, packing (rank,
per-recording cap, budget), history clipping, retrieval within scope and
budget (notes, markers, fallback, summary), an answer's scope and
citations with thread memory, stricken words never in the prompt, purge
on Strike and recording delete (questions kept, threads flagged, backups,
old schemas). TypeScript (12 of 67): topic filter, row chips, grouping by
date, notebook and topic, Group by parsing; default scope and labels,
suggested questions per scope, inline and block Markdown parsing,
citation numbers, clock. No test calls a real endpoint.

## iOS

Code: `ios/NoFriction/Topics/` (`TopicModels.swift`: the `MeetingTopic`
model, the `Topic` label/key rules, `TopicIndex`, `TopicStore`;
`TopicAI.swift`: prompt, `TopicParse`, `MeetingAI.findTopics`;
`TopicViews.swift`: chips, filter row, editor) and `ios/NoFriction/Chat/`
(`ChatModels.swift`: `ChatScope`, `ChatThread`, `ChatThreadMessage`,
`ChatCitation`, `ChatStore`; `ChatRetrieval.swift`; `ChatAI.swift`: prompt,
`MeetingAI.chat`, citation mapping, `ChatSuggestions`; `ChatView.swift`).
Tests: `ios/NoFrictionTests/TopicsAndChatTests.swift`.

### Topics

- **Model.** `MeetingTopic { id, label, key, confidence, source (ai|user),
  createdAt, meeting }`, cascade with `Meeting` (`Meeting.topics`). `key` is
  `Topic.key(label)`: lowercased, diacritics folded, anything but letters
  and digits becomes a space, articles and connectives dropped, simple
  plurals singularized ("The Q4 Roadmaps" → `q4 roadmap`). Labels are
  trimmed, one line, ≤ 40 characters, ≤ 6 words. `Meeting.removedTopicKeysJSON`
  (keys only) remembers AI topics the user removed. Everything added is an
  optional column or a new entity, so stores from older builds open without
  a schema version (a test round-trips an on-disk store).
- **When.** The same hook as notes: **Make notes** / **Make again** names the
  topics after the notes are saved (a failure shows under the topics, not
  under the notes). **Find topics** / **Find again** in the recording's
  Notes section runs it alone. Both go through `MeetingDetailView.requestAI`,
  so Pro, provider setup and consent are checked the same way.
- **Prompt and validation.** `MeetingAI.topicsSystem` asks for
  `{"topics":[{"label":…,"confidence":0–1}]}`, 1–4 noun phrases of 1–4
  words, no generic words, nothing invented, stricken text never guessed.
  The transcript goes as `[m:ss]` lines (`StudyInput.transcriptLines`); a
  transcript that doesn't fit the model's context is sent in chunks
  (`MeetingAI.chunkLines`, "Part i of n") and the chunks' topics are merged
  (`mergeTopicCandidates`: near-duplicate keys join, the commonest label
  leads, a topic several chunks named scores higher). Each request is retried
  once when the answer can't be used; AI access errors stop the run. The
  answer is untrusted: `TopicParse.validate` extracts the first JSON value
  (fences, prose and `<think>` stripped, trailing commas tolerated), cleans
  every label (`StudyParse.clean`), clamps confidence (`0.8`, `"80%"` and
  `80` all read as 0.8; missing → 0.5), drops generic and over-long labels,
  merges near-duplicates and keeps at most 4. Nothing from the answer is
  logged.
- **Near-duplicates.** `Topic.similar(a, b)`: equal keys, equal without
  spaces ("off site" ~ "offsite"), the same words in another order
  ("roadmap q4"), or one typo apart for keys of 8+ characters. `TopicIndex`
  merges topics across recordings with that rule, so the chip row stays
  small: one group per topic with its recordings, the user's spelling (or
  the commonest) as the label, biggest group first. The Topic filter, the
  Topic grouping and the Chat scope all use the index, so "Q4 roadmap" and
  "the q4 roadmaps" are one topic everywhere.
- **Saving (`TopicStore.applyAI`).** A run replaces the recording's AI
  topics. User topics stay; a candidate matching a user topic (similar key)
  or a removed key is skipped; candidates under confidence 0.3 are dropped;
  at most 4 AI topics and 8 topics in all.
- **Editing.** In Notes → Topics: tap a chip for **Rename…** / **Remove**,
  **Add** for a new one. A rename makes the topic the user's; adding a label
  that matches an AI topic makes that topic the user's; removing an AI topic
  records its key so a re-run doesn't bring it back; adding it again clears
  that. Generic words ("meeting") are refused. User topics show a small
  person mark.
- **Recordings list.** No topic chips and no Group by (removed in the
  Fadell-audit pass, F-12: a recording listed under each topic appeared
  several times). The list is by day, **Notebooks** chips are the one
  filter, and the search box matches topic labels. Topics show on the
  recording and as a Chat scope.
- **Purge.** Topics go with the recording (cascade). A Delete (when it
  commits) or Strike of transcript text deletes the recording's AI topics,
  through `RedactionEngine.purgeDerived` (with the study guide and the chat
  answers); the user's own topics stay. Screen-only edits keep topics. See
  [REDACTION.md](REDACTION.md).

### Chat

- **Tab bar.** Record · Recordings · **Chat** · People · Settings. Five tabs
  fit the iPhone tab bar (iOS shows "More" only from six), so People stays
  a tab; on iPad the same tabs are the sidebar.
- **Scope.** The menu at the top: **All recordings**, **This Notebook ▸**
  (recent notebooks), **This Topic ▸** (the merged topic groups), **This
  recording ▸** (saved recordings with a transcript). The scope covers the
  saved recordings with a transcript; the one being recorded is excluded.
  The bar shows how many recordings the scope covers. `ChatScope.filter`
  applies it. Every answer is stored with the scope it was made in and shows
  it ("Scope: Notebook · BIO 101").
- **Threads.** `ChatThread { id, title, createdAt, updatedAt, scopeKind,
  scopeValue, scopeLabel, flagged, flagNote, messages }`, cascade to
  `ChatThreadMessage { id, role (user|assistant), content, createdAt,
  scopeLabel, citationsJSON, thread }`. The title is the first question (≤ 48
  characters). **New chat** starts a thread on the next question; the list
  button shows past chats (swipe to delete). The last 8 turns (16 messages)
  of the thread go back to the model as memory, without the passages.
- **Retrieval (`ChatRetrieval`, local only).** The scoped recordings become
  `ChatSource`s (transcript lines as the prompts read them: stricken spans
  as `[stricken from the record]`, deleted words gone; AI notes; marker
  notes). Passages are windows of consecutive transcript lines (~420
  characters, cited at the first line's time), one per `##` section of the
  notes, and one per marker note. The question's terms (lowercased,
  diacritics folded, stopwords out, plurals singularized) rank passages by
  term frequency weighted by rarity across the passages, with a bonus for a
  passage that has every term or the words in a row, a small one for notes,
  recent recordings breaking ties. The best fit the character budget left
  beside the prompt, the memory and the answer (`MeetingAI.bodyBudget`), at
  most 5 per recording and 12 in all, and go to the model in reading order.
  When no term matches ("summarize everything"), the most recent
  recordings' notes and opening lines stand in. No network and no
  embeddings; nothing is indexed outside the store.
- **Streamed answers (2026-11-18).** The answer appears while the model
  writes it. `AIClient.complete(…, onText:)` sends the same request with
  `stream: true` and reads the events line by line (lines end at a line
  feed only, and every byte counts against the cap;
  `ResponseParser.StreamedText`); Apple's on-device model streams through
  `LanguageModelSession.streamResponse` (`AppleOnDevice.complete`). Same
  consent check, URL policy, timeout, 4 MB cap and redaction; a server
  that answers with plain JSON is read whole, one that refuses `stream` is
  asked again without it, and a body that just stops is an error, never
  half an answer. `ChatView` passes `MeetingAI.liveComplete(endpoint,
  onText:)` and shows `ChatStream.text` (`MeetingAI.chatPartial` hides
  think blocks and closes a `**` or `` ` `` opened on the last line) in
  the answer's bubble; the stored answer with its citation chips replaces
  it. If text is removed from any recording meanwhile (`ChatStore.purges`),
  what was shown goes at once (`ChatStore.purged` notification) and the
  finished answer is not kept (`ChatStore.purgedMeanwhile`). **Chats** and
  **New chat** wait while an answer is on its way. Notes, the Review guide and topics are
  not streamed. Not run on a device or with a real model: the unit tests
  use a stub and the simulator has no on-device model.
- **Prompt.** `MeetingAI.chatSystem` (answer only from the passages, cite as
  `[n]`, say what wasn't found, never guess at stricken text) +
  memory + `SCOPE: … PASSAGES: [n] Title · date · Transcript 12:34 ·
  Notebook: … QUESTION: …`. `maxTokens` ≤ 900.
- **Citations.** `MeetingAI.citations(in:passages:)` reads every `[n]` in
  the answer (first mention order, each once, numbers with no passage
  ignored) and stores `ChatCitation { n, meetingID, title, timestamp, offset,
  kind (Transcript|Notes|Marker), excerpt (≤ 140 characters) }` with the
  message. The chips under an answer show `[n] Title 12:34`; a tap opens
  `MeetingDetailView(meeting:jumpTo:)`, which scrolls the transcript to the
  line being spoken at that moment (the same `jump` the markers and the
  quiz use). A deleted recording's chip shows "This recording was deleted".
- **Suggested questions.** An empty chat offers up to four questions built
  from the scope's titles, types and topics (`ChatSuggestions.questions`),
  with no AI call; a tap asks it.
- **Gating.** `ChatView.ask` runs the same steps as a recording's AI buttons:
  Pro (paywall) → provider configured (setup sheet) → consent for a public
  endpoint (consent sheet, naming the destination) → the request through
  `AIClient`. The question is kept and asked once the step is done.
- **Purge.** `ChatStore.purge(meetingID:title:deleted:context:)`: every
  assistant message citing the recording is deleted (answers quote its
  transcript and notes), the thread is flagged with a note ("was deleted /
  was edited; answers that cited it were removed"), and a thread scoped to
  that recording is flagged too. User questions are the user's own words
  and stay. Called by **Delete Recording** (before the row goes) and by
  `RedactionEngine.purgeDerived` for a Delete that commits (also a Delete
  recovered at launch) and for a Strike of transcript text. Deleting a
  thread cascades its messages. Questions and answers are never logged.

### Tests (`TopicsAndChatTests.swift`)

Topic JSON extraction and validation (fences, thinking, bare arrays,
alternative field names, confidence forms, generic/long/duplicate labels,
the cap of four, malformed input never crashes); label and key
normalization; `similar` and `canonicalKey`; `TopicIndex` merging across
recordings; chunk merging; `findTopics` with a mock (retry once, access
errors throw, chunking of a long transcript); `applyAI` keeping user topics
and removed keys; the per-recording cap; purge on Delete commit (user
topics kept) and on Strike (screen-only strike keeps them); cascade on
recording delete; Group by Notebook / Topic / Date; chat purge on Strike,
Delete commit and recording delete (thread flagged, questions kept,
recording-scoped thread flagged, thread delete cascades); 8-turn memory;
scope filtering and stored scopes; chat end to end with a mock (memory
placement, prompt shape, citations); an on-disk store round trip; passage
building; term ranking; scoping, budget, per-recording cap, reading order
and the no-match fallback; citation mapping and round trip; suggested
questions. No test calls a real endpoint.
