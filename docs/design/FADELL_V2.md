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
- **No floating window** (yet). A quick-note panel over other apps needs a
  non-activating panel the app doesn't have; ⌃⌥⌘M still marks from anywhere.
  It is the next thing to build once the note line has earned it.

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
  earned it. **Answer: build it.**

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
