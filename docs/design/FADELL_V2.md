# noFriction v2: product review and plan (Fadell lens)

Date: 2026-11-12. Scope: Mac 3.9.0 (`src/`, `src-tauri/`), iPhone 1.1.0
(`ios/NoFriction`), Apple Watch. The first audit
([FADELL_AUDIT.md](FADELL_AUDIT.md), 2026-10-09) cut the old "command
center" and left one clean recorder. This one asks what the next version is
for.

How it was looked at: the Mac app ran in the film harness
(`marketing/film/mac-harness`) at 1440×900 and every screen from Record to
Review was walked; the Rust and Swift code behind each step was read; the
iPhone flow was traced in code.

The lens is the same: the product is the whole journey, not the screen. A
journey has a before, a during and an after. v1 designed the during.

---

## 1. Verdict

v1 is a tool you operate. You remember it, open it, answer three questions,
press Record, press Stop, open the recording, switch to Notes, and press Make
notes. Every step is clean, and every step is yours.

v2 is an appliance that works for you. It notices that class is starting,
listens without asking for the screen, takes your few typed words and
finishes them, and when you look up the notes are done and the semester is in
one place.

The single most common failure of any recorder is the recording that was
never made. The second is the recording nobody opened again. v1 does nothing
about the first and little about the second.

---

## 2. What the walk found

Ranked by what it costs the user.

**V-1 · Mac + iPhone · Notes cover only part of a long recording.** The
Mac's notes prompt takes the first 8,000 characters of the transcript
(`meeting_notes.rs`, `chars().take(8000)`), about ten minutes of speech. A
50-minute lecture gets notes on its opening. The iPhone trims the middle out
of a long transcript to fit the model (`ContextFit.fit`), so its notes skip
the body of the lecture. The study guide already reads a long recording in
parts (`study::condense_to_budget`, `MeetingAI.studyGuide`); notes never got
the same treatment. *This is the product's main promise and it is quietly
broken for the people the store copy names first.*

**V-2 · Mac + iPhone · Notes ignore what you marked.** Marks feed the study
guide and Chat, but not the notes. You press ★ at the moment that matters
and the notes are written as if you hadn't.

**V-3 · Mac + iPhone · There is nowhere to write.** The only words a user
can add during a recording are an optional 280-character note on a mark. A
student takes notes. Today they take them in another app, with noFriction
hidden behind it, and the two never meet.

**V-4 · Mac + iPhone · Nothing starts by itself, and nothing reminds you.**
`auto_start_recording` is a setting no code reads. The Mac can already tell
that a call app holds the microphone (it uses that to stop), and both
platforms read the calendar, but only after you press Record. The idle
Record screen says "Ready when you are" on a Wednesday at 9:00 when you have
recorded BIO 101 every Wednesday at 9:00 for six weeks.

**V-5 · Mac · Stop lands on the wrong tab.** After Stop the recording opens
on Rewind: a black player and a timeline. The thing you came for is one tab
over, and automatic notes give no sign they are being made. On the iPhone
Stop returns to an empty Record screen; notes are never made unless you find
the recording and ask.

**V-6 · Mac + iPhone · A notebook is a filter, not a place.** "BIO 101" is
a chip that narrows a list. What the instructor said is due, what will be on
the test and what you wanted to ask are spread over fourteen recordings,
each behind three clicks. The unit of a student's work is the course; the
unit of a manager's is the project. The app's unit is the recording.

**V-7 · Mac + iPhone · The Record sheet asks what the app already knows.**
Three questions every time (the first audit's F-19 stands). When the
calendar or your own habit says what this is, the answer should be one
button with the answer on it.

**V-8 · iPhone · No way in from outside the app.** No Shortcut, Siri phrase,
Action Button or notification starts a recording; the watch has all of
those.

What not to touch is unchanged from the first audit: the red button, Mark,
Delete and Strike, the review guide, Chat's citations, the watch, the
palette. v2 adds no top-level view and no new noun.

---

## 3. The three features

Each is one sentence a student or a project lead would repeat to a friend.

### A. "It was already listening." (Hands-free)

*Before.* noFriction learns when you record and offers it at the right
moment.

- **Up next.** The idle Record screen (Mac) and Record tab (iPhone) show one
  card when something is starting: the calendar event that is on now or
  begins within 15 minutes, a notebook you usually record at this time, or
  (Mac) a call app that has just taken the microphone. One button with the
  answer on it: **Record BIO 101**. No sheet. The plain Record button and
  its three questions stay for everything else.
- **The routine is learned, never set up.** A notebook recorded on the same
  weekday within 25 minutes of the same time, at least twice in the last
  six weeks, is a routine. There is no schedule screen. Stop recording it
  and it fades; **Not now** hides it for the day.
- **A nudge.** With the window closed, the same suggestion arrives as a
  notification (**Settings → Recording → Remind me when something starts**,
  on). On the iPhone the notification is scheduled ahead from the routine,
  so it fires with the app closed.
- **Record by itself** (Mac, off by default). When on, the same three
  signals start the recording and a notification says so. It is opt-in
  because recording people is the user's act; the setting says "Tell people
  you're recording."
- **iPhone, from outside the app.** A **Start recording** App Shortcut
  (Siri, Shortcuts, Action Button), like the watch already has.
- It already stops by itself (v1). Nothing changes there.

### B. "I typed six words and got the whole page." (Your notes, finished)

*During and after.* Type while it listens; the notes are built on what you
wrote.

- **One line to type in.** Under the live transcript, a single field: "Type
  a note". Enter keeps it, stamped with that moment, and it appears in the
  transcript where it was written. No second pane, no mode. iPhone: the same
  field on the Record screen.
- **A note is a mark you typed.** It is stored as a mark of kind `note`
  beside ★ ? ✎, so everything that already handles marks handles notes:
  Rewind shows them at their moment, a time-range Delete removes them, the
  study guide and Chat read them, Export includes them, and sync carries
  them (older versions skip a kind they don't know). The Marks list stays
  the three marks.
- **Notes are built around them.** Make notes (and automatic notes) now
  read the whole recording, in parts when it is long, and are told what you
  typed and marked. The Notes view opens with **What you wrote**: each of
  your lines, the time it was typed (click to hear it), and under it what
  was being said, written out. Then the notes for the type, as before.
- **Free users keep their own words.** Typing, and seeing your lines in the
  transcript and in Notes, is free. The AI that finishes them is Pro, as
  notes already are.

### C. "Everything for the course is on one page." (The notebook page)

*After.* Picking a notebook opens it.

- **What's due.** Every announcement, deadline, action item and to-do the
  notes found, across the notebook, newest first, each with a checkbox and
  the recording it came from. Class: "Due". Meeting: "Action items".
  Personal: "To-dos".
- **On the test and questions** (Follow ups / To remember, by type). Every
  ✎ and every ? Question in the notebook, with what you wrote, one click
  from the moment.
- **Review everything.** The flashcards and practice questions from every
  study guide in the notebook as one deck and one quiz. No new AI call: it
  is the guides you already made, together. The night before the exam this
  is the button.
- **Record** and **Ask** on the page: start the next recording in this
  notebook with one click, or open Chat scoped to it.
- Reading the page is free. What fills it (notes, guides) is made by Pro
  features, as today.

### Also in v2 (fixes, not features)

- **Stop opens Notes**, with your lines there at once and "Making notes…"
  while the AI works. On the iPhone automatic notes are new: they are made
  by themselves when AI is set up (**Settings → Make notes automatically**,
  on).
- **Notes are ready** (Mac). A notification when automatic notes finish
  and the app isn't in front. Not on the iPhone: iOS pauses the app in the
  background, so notes are made while it is open (§9).
- **Automatic notes for short recordings you engaged with.** Six minutes
  stays the rule; a recording with a typed note or a mark gets notes from
  one minute.

---

## 4. What v2 does not do

- **No AI on screen while you record.** The first audit cut live insight
  cards for a reason. The app listens and keeps what you type; the AI works
  after Stop.
- **No schedule editor, no notebook manager, no fifth view.** The routine is
  learned, notebooks stay derived from recordings, and the notebook page
  lives inside Recordings.
- **No server.** Everything above is computed on the device from what is
  already stored. Suggestions never leave it.
- **No floating window.** (Changed 2026-11-13: the owner asked for the
  quick-note panel, §10. It is one field over the app in front, not a
  second window to manage; ⌃⌥⌘M still marks from anywhere.)

---

## 5. Counts, before and after

| Journey | v1 | v2 |
|---|---|---|
| Returning user, class is starting → recording (Mac) | Open app, Record, 3 answers, Start: 3 clicks min | 1 click on **Record BIO 101**, or 0 with Record by itself |
| Stop → reading notes (Mac) | Stop, Notes tab, (Make notes): 2–3 clicks | Stop: 1 click |
| Stop → reading notes (iPhone) | Stop, Recordings, recording, scroll, Make notes: 4 taps | Stop: 1 tap |
| "What's due in BIO 101?" | Open each lecture → Notes → scroll | Click **BIO 101** |
| Study for the exam from 14 lectures | 14 guides, opened one at a time | **Review everything** |
| Write your own note at a moment | Mark, then type ≤ 280 characters in a popover | Type, Enter |
| Share of a 50-minute lecture the notes read | About the first 10 minutes (Mac); head and tail (iPhone) | All of it |
| New top-level views / new nouns | — | 0 / 0 |

---

## 6. Shared rules for both platforms

So the Mac and the iPhone build the same thing.

**Typed notes**

- Stored kind: `note` (Mac `meeting_markers.kind`, iOS `MarkerKind.note`),
  wire value `"note"` in the existing sync `MarkItem`. No protocol change:
  3.9.0 and 1.1.0 already skip a mark kind they don't know.
- One line each: whitespace closed up, at most 1,000 characters (marks keep
  280). Never debounced against a mark.
- Not one of the kinds a mark can be changed to, and not in the Marks list,
  its filter or its count. A note can be edited or deleted.
- Symbol in prompts and exports: `✍`, label "Note".
- A time-range Delete or Strike (Mac) removes the notes inside it, like
  marks. Word and line edits leave them alone (they are the user's words);
  the iPhone edits by selection only, so its edits never remove a typed
  note.

**Notes generation**

- Input: the whole stored transcript with times (`[m:ss] text`, stricken
  spans as `[stricken from the record]`), the marks, and the typed notes
  numbered `1..n` with their times. Ids never go in a prompt.
- A transcript that doesn't fit beside the prompt is condensed in parts
  first (the study guide's map step). Nothing is cut off the end or out of
  the middle.
- The model is told: cover what was marked and typed first; for each typed
  note, write one or two sentences of what was being said then.
- Mac: the stored JSON gains `your_notes: [{"n": 1, "detail": "…"}]`, kept
  in `meeting_notes.note_details` as `[{marker_id, detail}]`. iPhone: the
  Markdown gains a first section `## What you wrote` with `- **your line**:
  detail` bullets. Both are AI output: redaction treats them like the rest
  of the notes.

**Rules the correctness review added (both platforms)**

- **The save and its check are one step.** Notes are written only if every
  line they were made from is still in the transcript and no Delete is
  waiting, and that check and the write happen together (Mac: one
  `BEGIN IMMEDIATE` transaction; iPhone: one synchronous step on the model
  context, with no `await` between the check and the insert).
- **Nothing automatic calls the AI while a recording runs.** Automatic
  notes wait for it to stop, and no automatic request starts while one
  runs. A request already on its way when Record is pressed is left to
  finish (both platforms). The user's own **Make notes** is theirs to
  press at any time.
- **A typed note's detail never outlives the note.** Deleting a typed note
  (directly, by a time-range Delete or Strike, or from the other device)
  removes what the notes wrote under it in the same step. A detail belongs
  to its note by identity (Mac: the marker id; iPhone: the ids the request
  numbered, `WhatYouWrote.finalize(_:asked:now:)`): if the typed notes
  change while notes are being written, an edited note keeps its detail, a
  deleted note's detail is dropped, and it never lands under another line.
- **A typed note belongs to the recording and the moment it was typed in.**
  The moment is taken at Return, and the line is kept on that recording
  even if Stop was pressed before the save arrived.
- **"What you wrote" in synced notes lists every typed note** (`- **line**:
  detail`, or `- **line**` with nothing under it). The receiver merges it
  with the typed notes it holds and never hides a line it can't match.
- **An answer for something that is no longer open is dropped** (switching
  recordings or notebooks while a read is on its way).
- **Logs name the kind of an AI failure, never its text.**

**The routine (pure function, same on both)**

All times are local, as minutes of the day (no wrap past midnight).

- Candidates for a notebook: its recordings that started in the last 42
  days, on the same weekday as today, and not today.
- A candidate that started at `s` matches now when now is in
  `[s − 10 min, s + 25 min]`.
- The notebook is suggested when at least 2 candidates match now and none
  of its recordings started today.
- Tie: the notebook with more matches, then the one recorded most recently.
- The suggestion carries the notebook (its most recent spelling), its usual
  type (the type of its latest recording) and its usual length (the median
  length of the matches in whole minutes, rounded up to 15/30/60/90; longer
  or unknown is no limit). Whole minutes are counted down: a recording that
  stopped at its 60-minute limit ends a second or two after the hour and
  ran 60 minutes, not 61.
- Priority of sources: calendar event now → routine → (Mac) call app.
- **Not now** hides that suggestion until the next local day.
- iPhone reminder: at the median start time of a weekday's cluster (at
  least 2 of the notebook's recordings on that weekday, within 35 minutes
  of each other, last 42 days).

**The notebook page**

- Scope: recordings whose notebook matches, ignoring case.
- Items: from each recording's newest notes (Mac `action_items` JSON; iPhone
  the bullets under the notes' last heading: "Action items", "Announcements
  and deadlines" or "To-dos and reminders").
- Done state is kept by a hash of the item's text, per recording. No item
  text is stored a second time.
- The marks section: marks of kind `test` and `question`, with the note and
  the time, titled for the notebook's type.
- Review everything: the union of the notebook's stored flashcards and quiz
  questions, in recording order.

**Vocabulary** (additions to DESIGN.md)

| Concept | Say | Never |
|---|---|---|
| What the user types while recording | a note; the field says **Type a note** | jot, comment, annotation |
| Their lines in the Notes view | **What you wrote** | My notes, user notes |
| The suggestion | **Up next**; button **Record \<name\>** | Smart start, auto-detect |
| Dismissing it | **Not now** | Dismiss, Snooze |
| The notebook's page | the notebook's name; sections **Due / Action items / To-dos**, **On the test and questions** / **Follow ups and questions** / **To remember and questions**, **Review everything** | Dashboard, Course home, Hub |
| The settings | **Remind me when something starts**, **Record by itself**, **Make notes automatically** | Auto-detect, Ambient, Smart |

---

## 7. What the design review added (2026-11-12)

`/design-review` ran on the built Mac UI in the film harness (the report
and its screenshots are kept outside the repo). Sixteen findings were
fixed; three betterments it proposed were built:

- **Ticks in Notes.** The tick beside an item is the same in a recording's
  Notes and on its notebook's page.
- **Still open from last time.** The Up next card lists what the suggested
  notebook's notes still have open.
- **Start from.** Review everything can begin at any recording.

Decisions it left for the owner, and the answers (2026-11-13):

- **Inter isn't bundled.** It shows only where it is installed; most Macs
  see the system face. Bundle it (about 300 KB, OFL) or name the system
  face in DESIGN.md. **Answer: the system face.** The CSS no longer names
  Inter, so the app looks the same on every Mac (DESIGN.md 1.4.0).
- **44px targets.** DESIGN.md asks for 44px; the app's controls are
  30–40px, as desktop apps are. Either the rule or the controls should
  change. **Answer: the rule.** 44pt is for iPhone and iPad; the Mac keeps
  desktop control sizes (DESIGN.md, Accessibility).
- **A quick-note panel over other apps** (a non-activating floating field
  on a global key) is the next thing to build once the note field has
  earned it. **Answer: build it.** Built 2026-11-13 (§10).

---

## 8. What the correctness review found (2026-11-12)

Before merging, an independent review (Codex) read the Mac branch for
bugs. Fifteen findings; all were checked against the code and fixed:

| | Finding | Fix |
|---|---|---|
| 1 | The notes' "transcript unchanged" check and their save were two steps; a Delete could land between them | One transaction (`meeting_notes::save_checked`) |
| 2 | Two quick clicks on a Starting switch could be stored out of order | Writes per switch go out in order |
| 3 | Automatic notes could call the AI while another recording ran | They wait for it to stop |
| 4 | A failed automatic run logged the start of the error text | The log names a kind only |
| 5 | A deleted typed note left its detail in `note_details` (direct delete, time-range purge, synced removal) | Pruned in the same step |
| 6 | Changing a typed note didn't send the notes again | The notes row is touched |
| 7 | **Older than v2:** the Undo toast applied a delete on the next redraw when its error handler was passed inline (recordings, chats) | The handler is read through a ref |
| 8 | A recording that started and stopped between two microphone samples left its call to be offered again; an unreadable sample was read as "nobody" | The tracker is told at once; an unreadable sample changes nothing |
| 9 | **Ask** on a notebook's page opened Chat on the selected recording | The follow-the-recording effect skips its first run |
| 10 | An answer for the previous recording could land in the next one's Notes or marks | Latest request wins; one Notes panel per recording |
| 11 | Return, then Stop at once: the note was refused | The note carries its recording and moment |
| 12 | Synced notes with "What you wrote" hid the typed-note rows | Merged by line; nothing hidden |
| 13 | The same stale answer on a notebook's page | Latest request wins |
| 14 | A second call app inherited the first one's time on the microphone | The hold is per app |
| 15 | "Recording …" was posted before the recording ran | Posted once it runs; else the reminder |

---

## 9. iPhone and iPad (2026-11-13)

The three features were built for iPhone and iPad to the rules in §6
(`ios/README.md` lists the files and tests).

**Where the iPhone differs, and why**

| | Mac | iPhone and iPad |
|---|---|---|
| What Up next reads | Calendar, routine, call apps | Calendar and routine. An iPhone app can't see which apps use the microphone |
| Starting by itself | **Record by itself**, off by default | Not offered. An iPhone app can't start the microphone from the background |
| From outside the app | The tray's first item | **Start Recording** App Shortcut (Siri, Shortcuts, Action button). It opens the app, then starts with the last type and length; a request made during first-run setup is dropped |
| Reminder | Sent by the running app, for any of the three signals | Local notifications planned from the routine for the next 7 days (at most 24 pending), so they arrive with the app closed. Calendar events send none |
| Notification permission | Asked by macOS | Asked in context: a card on the Record tab once a routine exists, button **Continue**. Never at launch; not on a day **Not now** was tapped |
| Automatic notes | After Stop; **Notes are ready** when done | After Stop while the app is open; no notification |
| Deleting a typed note | Undo toast | Asks first (no Undo) |
| Time-range Delete | Removes the typed notes in its span | None: the iPhone edits by selection (words, lines, screens), which leaves typed lines alone and rewrites the notes |
| Where a detail is kept | `meeting_notes.note_details` by marker id | In the notes' Markdown, `## What you wrote` |
| A typed note in the review guide's prompt | Whole | Its first 280 characters |
| The notebook's page | Opens beside the list when a chip is picked | **Open \<name\>** above the list when a chip is picked, and **Open** beside a recording's notebook |

**What the correctness review found (iPhone branch)**

The same independent review (Codex) read the iPhone branch. Ten findings:

| | Finding | Outcome |
|---|---|---|
| 1 | A topics answer arriving after the recording was deleted could write to it | Fixed: `TopicStore.applyAI` and `AutoNotes` check the recording still exists |
| 2 | A Shortcut start asked for during first-run setup stayed pending and could start later | Fixed: the request is dropped while setup is up |
| 3 | Details were paired with typed notes by position when the count matched, so a delete plus a note arriving by sync during generation could put one note's detail under another | Fixed: paired by identity, as on the Mac (§6) |
| 4 | On a small model window typed notes had a fixed half of their prompt space | Improved: they also take what the marks leave. With a very small window and a very long recording the last-resort fallback still samples lines, and typed notes are clipped and, past the budget, counted ("and N more"); every typed line is still listed in **What you wrote**, some with nothing under it |
| 5 | Deleting transcript lines leaves typed notes in that time | As designed: the iPhone has no time-range edit, and §6 leaves typed lines alone for word and line edits. The notes, with **What you wrote**, are rewritten and flagged |
| 6 | An automatic request already on its way continues when Record is pressed | Accepted, same as the Mac: no new automatic request starts while recording (§6) |
| 7 | The routine window is narrower than "8:50 to 9:30" | Not a defect: §6 defines the window per recording, and both platforms compute it that way |
| 8 | No **Notes are ready** notification | Not built on the iPhone (table above) |
| 9 | The review guide reads the first 280 characters of a typed note | Accepted limit; Chat and the notes read the whole line |
| 10 | A failed save to the store is not reported | Left as it is across the app (`try? context.save()` is the convention); not changed here |

The review found these correct: no `await` between the notes' check and
their save; the Pro, AI-set-up and consent gates; typed notes never among
the marks; an unknown mark kind skipped in sync; the watch unchanged (no
note kind, no network or speech); reminders capped and permission only
after **Continue**; no host, key, URL or content logging added.

One difference found on the way was a Mac bug and was fixed there: the
routine's usual length counted a recording that ran 60:02 as 61 minutes,
so a 60-minute class was offered as 90 the next week, then with no limit.

**Not verified**

The Simulator has no speech recognition, calendar events of its own,
notification delivery from a closed app, Siri or an Action button, and no
Apple on-device model. Recording flows were driven with demo data; the
calendar source, a reminder firing, the Shortcut and the Apple model were
not run. Nothing was run on a device. iPad was looked at in two
screenshots; the UI tests ran on iPhone.

---

## 10. The quick-note panel (Mac, 2026-11-13)

The note field with another way in. With slides, a call or a browser in
front, **⌃⌥⌘N** opens one field over it. Return keeps the line at that
moment and the panel is gone. The app in front stays in front.

**Rules**

- **It is the note field, nothing more.** A line goes through
  `markers::commands::keep_note`, the call the in-app field makes: a mark
  of kind `note` on the recording the panel was opened in, with the
  late-note rule (§6) unchanged. Everything a typed note does afterwards
  (What you wrote, Rewind, Export, sync, purge) follows from that. No AI,
  no network, no setting; free.
- **The app in front stays in front.** The panel is an `NSPanel` created
  non-activating (`quick_note/panel.rs`), so it takes the keyboard without
  bringing noFriction forward, and closing it hands the keyboard back. It
  is native, not a second web view: it costs no web process and is on
  screen before the next key is pressed.
- **A hidden app stays hidden.** Hiding noFriction (⌘H) doesn't hide the
  panel, so opening it doesn't bring the main window back over the slides.
- **While it has the keyboard, ⌘ keys stay in it.** ⌘Q typed into the panel
  would otherwise reach noFriction's own menu and quit it mid-recording.
  The editing keys work on any keyboard layout (⌘V ⌘C ⌘X ⌘A ⌘Z, ⌘← ⌘→ ⌘⌫),
  ⌘W closes it, the rest do nothing.
- **The panel stores nothing until Return.** Esc, a click elsewhere or the
  key again closes it and leaves the line in the field, in memory, while
  the same recording is on. Another recording, or none, starts empty.
- **It never starts a recording.** With nothing recording, the key shows
  "Nothing is recording" for a moment and takes no keys.
- **After Return the field takes no keys until the line is kept or not.**
  Kept: the panel closes. Not kept: it stays, with the line and the
  reason. The save can land after the panel was closed or opened again, so
  it touches only what is still its own (`quick_note::settle`): it never
  empties a field that holds something else, and a failure that lands on
  a closed panel is said the next time it opens in that recording.
- **It asks to be left out of screen captures and shared screens**
  (`NSWindowSharingNone`). macOS decides whether a capture honors that.
- **File → Type a Note** and the tray's **Type a Note** open the same
  panel, for when another app owns the key.
- **Its callbacks must not panic.** The shipped build aborts on a panic,
  which would end the app mid-recording.

**iPhone and iPad:** not offered. An iPhone app can't put a field over
another app.

**Verified** with `cargo run --example quick_note_panel` (in `src-tauri/`),
which runs the panel's window on the Mac without the app or its data
(36 checks):

- The panel is on screen and has the keyboard while the app in front and
  the menu bar's owner are unchanged, and AppKit doesn't announce that the
  app became active (noFriction listens for that to answer a notification
  click).
- A key press sent through the window server arrives in it, and stops
  arriving once it is closed or while only the notice is up.
- Typing, ⌘A, ⌘← ⌘→ ⌘⌫, ⌘A on a layout that isn't Latin, ⌘W, ⌘Q blocked.
- Return reports the line once; while it is being kept, typing and a
  second Return do nothing and Esc still closes; after a failure the field
  takes keys again with the line in it.
- Esc keeps the draft, and typing continues after it.
- With the app hidden, the panel comes up and the app stays hidden.
- The window server lists the panel as not shared.

**What the review found (2026-11-13)**

An independent review read the code before it merged. It found no crash
or undefined behaviour, and these:

| # | Finding | Outcome |
|---|---|---|
| 1 | When a slow save landed, it emptied and closed the panel whatever had happened since Return: text typed after Return was wiped, and a failure that landed on a closed panel was never said | Fixed: the field takes no keys while a line is being kept, and the save touches only what is still its own |
| 2 | With noFriction hidden (⌘H), the panel might not appear | The example showed something else: the panel appeared and the whole app came back with it. Fixed (`canHide` off) |
| 3 | Opening the panel might count as "the app became active" and bring the main window forward after a notification | Checked: AppKit doesn't announce it. No change |
| 4 | ⌘← ⌘→ ⌘⌫ did nothing in the field; ⌘V ⌘C ⌘X ⌘A ⌘Z did nothing on Russian, Greek or Hebrew layouts; the panel's own key, if it ever reached the panel, would close it at once | Fixed: the field's keys go to the field, letters are matched as ⌘ types them, and the panel leaves its own key to the global one |
| 5 | The guard around the callbacks does nothing in the shipped build, which aborts on a panic | No reachable panic was found. The comment now says so, and the rule is above |
| 6 | "Nothing typed is stored until Return" rests on macOS leaving the panel out of the app's own screenshots | The window server lists the panel as not shared. That a capture honors it is not verified (below) |
| 7 | The example could pass falsely: ⌘Q reaching the menu ended it with a success code, and two checks could not fail | Fixed: the Quit item sets a flag, and the checks send a real key and require that it doesn't arrive |

**Not verified**

- In the real app: the key, the menu and tray items, and a line arriving
  in the live transcript. The app was not launched (it has no data-folder
  override, so a development build would open the real database).
- Over a full-screen app, on a second display, with Stage Manager, and
  with an input method (Japanese, Chinese).
- That a screenshot or a shared screen leaves the panel out. If the app's
  own capture didn't, a line being typed would be in that screenshot like
  anything else on the screen, kept and purged with the recording's
  screens.
- How it looks over real content. Its own drawing was checked; the blurred
  background was not.

**Not changed**

- The Mac's screen-text reader (default flavor, `accessibility_extractor.rs`)
  reads the focused window of the app in front, noFriction's own windows
  included. Over another app the panel is never the app in front, so it
  isn't read. With noFriction itself in front it is read like the main
  window's note field always was.

---

## 11. Fill in the blanks (2026-11-13)

The owner, during the release: "add a magic button to fill in the blanks
for class notes or to extend them with key and additional details."

One button in a recording's Notes, once notes exist. It reads the whole
recording again beside the notes that exist and the lines the user typed,
and adds two sections:

- **What you missed.** Things that were said and aren't in the notes, in
  the order said, each with its time (on the Mac, a click opens Rewind
  there). At most 12.
- **More detail.** The notes' key ideas explained further from what was
  said. At most 8.

**Rules**

- **Only on a click.** Never automatic, never while that recording is still
  running. The same gates as Make notes: Pro (Mac App Store build), an AI
  that is set up, consent.
- **The user's lines are theirs.** Typed notes are shown to the AI so it
  knows what is covered; they are never rewritten.
- **What wasn't said is marked.** For a Class the AI may add what a
  standard textbook says about a concept the lecture didn't explain. Every
  such entry is stored and shown as background, "not said in the
  recording", and carries no time. A Meeting or Personal recording gets
  only what was said: an answer that marks something as background there is
  dropped.
- **The answer is untrusted.** Sizes are capped, a time outside the
  recording is dropped, repeats are dropped, and an unusable answer is
  asked for once more, then reported without quoting it.
- **Saved under the notes' check.** One transaction: no Delete pending,
  every line that was read still there, the notes row still the
  recording's latest, and the typed notes it was made beside unchanged.
- **It never outlives what it was made from.** Delete or Strike of
  transcript text removes it outright (it paraphrases the recording). A
  typed note it was made beside being changed or deleted removes it. Notes
  made again start without it.
- **One form on both platforms.** `## What you missed` and `## More detail`
  with fixed bullet forms (docs/SYNC.md), because the result crosses
  between devices inside the notes' Markdown.

**Where it lives.** Mac: `meeting_notes/fill.rs`, `meeting_notes.filled`
(JSON), the `fill_in_notes` command, the button in `MeetingNotesPanel.tsx`.
iPhone and iPad: `AI/FillInTheBlanks.swift` (the same three prompts,
character for character, and the same parsing rules), `NotesStore.fill` and
the synchronous `NotesStore.saveFill`, the two sections at the end of
`aiNotes`, and one helper every removal calls
(`FillInTheBlanks.removed(from:)`).

**Where the iPhone differs.**

- The result lives in the notes' Markdown, so there is no time to click:
  a missed point shows its time as text.
- An empty answer on **Fill again** keeps the sections that were there;
  the Mac replaces them with "Nothing to add".
- The Mac removes sections that came from the iPhone when a typed note is
  added, changed or deleted (it can't tell which notes they were made
  beside); the iPhone removes them when one is changed or deleted.
- A Delete or Strike of only photos or screens keeps the sections on the
  iPhone: no transcript text changed.

**Not in yet.** For notes made on the Mac, Export, Chat and the Obsidian
export read the notes' own columns and don't carry the two sections.

**Verified (Mac).** Unit tests for the prompts by type, the parsing rules,
saving and each way it is removed; the screen in the film harness (the
button, progress, both sections, **Fill again**, and the paywall without
Pro).

**What the review found (2026-11-13).** An independent review read the Mac
half before it merged. The Mac-only design held; the gaps were where the
iPhone is involved:

| # | Finding | Outcome |
|---|---|---|
| 1 | A Strike or Delete made on the iPhone left the two sections on the iPhone | Fixed in the iPhone half: `redactAIOutputs` (and the Delete finished at the next launch) removes them |
| 2 | Sections inside notes that came from the iPhone were outside every removal on the Mac | Fixed: Delete, Strike, a typed-note change and this Mac's own fill all strip them from that text too |
| 3 | A typed note edited on the iPhone didn't remove what the Mac had filled in | Fixed: a typed note arriving by sync goes through `typed_notes_changed` |
| 4 | Background was recognised only by a few spellings, so "textbook" or "outside knowledge" passed as said | Fixed: an entry counts as said only when it says so, or gives its time and says nothing else; everything else is background |
| 5 | On a small model the notes shown to the AI could crowd out the recording | Fixed: they take about a quarter of the model's window, at most 6,000 characters |
| 6 | An empty fill on iPhone-made notes hid the iPhone's sections on screen but still sent them | Fixed: this Mac's result replaces theirs in the stored text |
| 7 | Only the first 40 typed notes were tracked, and a trailing space changed a note's hash | Fixed: every typed note, compared as the notes read it |
| 8 | The Delete and Strike confirmation didn't list it | Fixed |
| 9 | A fill (or Make notes) started inside a Delete's undo window sent the text that was about to be deleted | Fixed for both: nothing is sent while a Delete is pending |
| 10 | The Mac and the screen disagreed on what a heading is | Fixed: a bold line is a heading in both |

**Not verified.** No real AI was asked: the quality of what a model writes
for a real lecture, and how well a small model keeps to "mark background",
are untested. Not run in the real app or on a device. On iPhone the gate
order was driven only as far as the paywall, VoiceOver and iPad were not
looked at. The two devices filling in and syncing with each other is
covered by unit tests on each side, not by a run between a real iPhone and
Mac.

---

## 12. Read the screens, and the prompt library (Mac, 2026-11-13)

The owner, during the release: "you need to make a prompt library for
reviewing the images. how many images are you sending? Are they all the
time?" The answer then was none: no feature sent a picture, and a retired
background analyzer that could have was removed the same day. This is the
feature that does, built so those questions always have an exact answer.

In a recording's Notes, **On the screens** has **Read the screens…**. A
confirmation says how many screens will go and to which host, and lists
each with its time, its kind and its app. Its button says the number
again. Each screen is then sent as one picture and what the AI read from
it is kept as text, with its time.

**Rules**

- **How many: the number on the button, at most 12.** The count the user
  confirmed is passed back, and the Mac refuses to send a different one.
- **Which: the ones that matter.** The screen showing at each mark and
  typed note, then the rest spread evenly over the recording. A screen is
  read once; noFriction's own window never.
- **When: only on that click.** Never by itself, never on a timer, never
  on a key phrase, never while that recording is running.
- **What goes: one picture per request**, a copy made smaller (1568 px on
  its long edge), with the prompt for its kind and up to 420 characters of
  what was being said then. Nothing else.
- **Where: the user's image model.** The gates of every AI call apply (Pro
  in the Mac App Store build, consent for a public endpoint). Apple's
  on-device model can't read images, so nothing is sent with it.
- **The prompt library** (`screen_prompts.rs`). One entry per kind of
  screen: slide, whiteboard, document, code, table, chart, web page, call,
  and anything else. The kind is decided on the device from the app and
  window the screen came from; no request is made to classify it. Every
  prompt shares the same rules (only what is visible, exact copies of
  formulas and numbers, never a person's appearance or name, `skip` for a
  private or empty screen) and the same JSON answer, so one parser reads
  them all. A call is read for what is shared, never for who is in it.
- **What is kept is the screen's.** A reading is a text snapshot on the
  screen's own id, so deleting or striking the screen removes it through
  the purge that already exists. It is saved only if the screen still
  exists, with the same picture, when the answer arrives.
- **Not given to other AI features.** Make notes and Fill in the blanks
  don't read it: notes would then quote a screen, and a screen's purge
  doesn't clean notes.

**Not on iPhone or iPad.** Its AI client only sends text.

**Verified.** Unit tests: the library (one prompt per kind, the kind from
app and window, the shared rules), the choice of screens, the parser, one
click sending the number shown and refusing another, a reading dropped
when its screen is deleted or its picture replaced meanwhile, a refusal
stopping the run, and a screen's Delete and Strike removing its reading.
The screen in the film harness: the confirmation, progress, the list.

**Not verified.** No picture was sent to a real model: what a model reads
from a real slide, whether it honours `skip` on a private screen, and what
twelve requests cost are untested. Not run in the real app.

