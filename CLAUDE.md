## gstack (REQUIRED — global install)

**Before doing ANY work, verify gstack is installed:**

```bash
test -d ~/.claude/skills/gstack/bin && echo "GSTACK_OK" || echo "GSTACK_MISSING"
```

If GSTACK_MISSING: STOP. Do not proceed. Tell the user:

> gstack is required for all AI-assisted work in this repo.
> Install it:
> ```bash
> git clone --depth 1 https://github.com/Cpotenzone/gstack.git ~/.claude/skills/gstack
> cd ~/.claude/skills/gstack && ./setup --team
> ```
> Then restart your AI coding tool.

Do not skip skills, ignore gstack errors, or work around missing gstack.

Using gstack skills: After install, skills like /qa, /ship, /review, /investigate,
and /browse are available. Use /browse for all web browsing.
Use ~/.claude/skills/gstack/... for gstack file paths (the global path).

## Secrets, deploys, and what NOT to commit

The Targon persistent-volume deploy is **decommissioned**. Production ships
via GitHub Actions → SSH → GCE (`.github/workflows/production_deploy.yml`).
Any reference to `/pie/criticalasset/*` paths, runtime file-writes that
"need persistent storage," or Targon container assumptions is stale — delete
it, don't work around it.

### API key handling (non-negotiable)

- **Never hardcode an API key as a fallback.** Not `process.env.X || 'AIza...'`,
  not `apiKey = 'fc-...'`. If the env var is missing, fail loud. A prior leak
  required a history rewrite — don't reintroduce the pattern.
- **Local dev**: keys live in `criticalasset/secrets.json` (gitignored).
  Callsites read `path.join(process.cwd(), 'secrets.json')` with
  `process.env.X` as the fallback. Match the pattern in
  `criticalasset/app/api/admin/scrapers/firecrawl/route.js::getFirecrawlKey()`.
- **Production**: keys come from GitHub Actions secrets, forwarded to the
  GCE box by the deploy workflow into `/opt/criticalasset/criticalasset/.env.local`.
  Add a new key by: (1) `gh secret set KEY_NAME`, (2) extend the `env:` block
  and `envs:` whitelist and the heredoc in `production_deploy.yml`.
- **Never** restore a UI-based write endpoint for secrets. The POST to
  `/api/settings` was removed because stateless deploys lose the file on
  every redeploy. `/api/settings` is GET-only (status display).

### What must stay out of the repo

The root `.gitignore` already covers these; don't re-add them:

- `node_modules/`, `.next/`, `dist/`, `build/`, `out/`
- `.env`, `.env.*` (except `.env.example` if you create one), `secrets.json`, `*.pem`, `*.key`
- `*.tar.gz`, `*.tgz` — historically the repo collected 20+ `app_*_update.tar.gz`
  "snapshot" archives. Don't commit new ones; use git tags or releases instead.
- `.DS_Store`

If something belongs tracked but is caught by the above (rare), use a targeted
`!pattern` override rather than weakening the base rule.

### If you find a leaked secret

1. Revoke / rotate the key **first** (dashboard for the provider).
2. Remove the hardcoded value and any callers that assume it.
3. For history scrub, use `git filter-repo --replace-text` with a
   `old==>REDACTED_NAME` list. Back up with a local `backup-pre-scrub`
   branch and tag before running. Never run filter-repo without confirming
   force-push is acceptable to the user.

## Cross-subsystem schema ownership

Some tables are shared across subsystems. The convention: **schema ownership lives
in `criticalasset/app/api/admin/migrate/route.js`** (the centralized migration
endpoint), and consuming subsystems read/write but NEVER ALTER.

| Table | Schema owned by | Consumed by |
|-------|-----------------|-------------|
| `findings` | `app/api/admin/migrate/route.js` | engine-v2 (osint findings), water-control, water-containment (audit→twin feedback loop, `provenance JSONB`), inspections, AI insights |
| `obligations` | `app/api/admin/migrate/route.js` | findings → obligations chain, water-containment v1.1+ |
| `insurance_signals` | `app/api/admin/migrate/route.js` | reports, scoring engine |
| `contractors`, `properties`, `inspection_events`, `normalized_permits`, `jurisdictions`, `systems`, `assets` | `app/api/admin/migrate/route.js` | many |

**Subsystem-owned tables** (each subsystem ALTERs its own via its own bootstrap):
- `water_containment_*` — owned by `app/lib/waterContainment/schemaBootstrap.js`
- `osint_extractions`, `reviewer_feedback`, `research_runs` — owned by `app/lib/engineV2/schema.js`
- `render_buildings`, `render_pins`, `render_topology_edges`, etc. — owned by `app/lib/digitalTwin/schema.js`
- `water_control_*` — owned by `migrations/water_control_schema.sql` (run via admin route)

**Adding a column to a shared table** (e.g., extending `findings` for a new feature):
1. Add the `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` to the `migrations` array in
   `app/api/admin/migrate/route.js`.
2. Document which subsystem reads/writes it (comment above the ALTER).
3. NEVER add the ALTER to a subsystem's own bootstrap (e.g.,
   `ensureWaterContainmentSchema()`) — it would run on every WC request and
   cross-subsystem mutations in the request path are fragile.
4. Run the migration via `POST /api/admin/migrate` (admin-only).

**Why this matters:** If two subsystems both try to ALTER the same shared table
with conflicting shapes, the loser silently corrupts the consumer that ran second.
Centralizing ownership prevents this. The `findings.provenance JSONB` column for
the WC audit→twin loop landed via this pattern in PR #13.

## noFriction (the app): project rules and hard-won lessons

The two sections above ("Secrets, deploys…" and "Cross-subsystem schema
ownership") describe the CriticalAsset web platform, not this repo. For
noFriction (Tauri Mac app in `src-tauri/` + `src/`, SwiftUI iOS app in `ios/`),
these rules apply.

**Product invariants (don't regress):**
- **Client-only.** No noFriction servers. Never add Supabase, Pinecone, an
  ingest server or any owner-hosted endpoint. AI requests go straight from the
  client to the endpoint the user entered. Spec: `docs/AI_PROVIDERS.md`.
- **AI is Apple on-device or one user-entered endpoint; presets are only a
  shortcut for filling that endpoint in.** The choices are Apple Foundation
  Models (iOS/macOS 26+ with Apple Intelligence) or one OpenAI-compatible
  endpoint with a base URL, model and optional user-supplied key. Provider
  preset cards (OpenRouter, ChatGPT/OpenAI, Anthropic/Claude, Muse/Meta, Grok/xAI,
  Mistral; owner decisions 2026-10-09 and 2026-10-22) are static data in exactly two tables
  (`src-tauri/src/ai/providers.rs::ENDPOINT_PRESETS`,
  `ios/NoFriction/AI/AIProvider.swift::AIPreset.all`, docs cited beside each
  entry) that pre-fill the URL and model; the saved connection stays the
  `custom` provider with the same Keychain binding and consent rule, and the
  preset match is derived from the URL, never stored. Never add a default
  remote URL, a provider active at first run, a preset selected without a
  click, key-prefix detection, a startup provider probe, bundled keys or
  hosted AI; provider hosts (and OpenRouter's sign-in and key-exchange URLs)
  may appear nowhere but the two tables. Pre-consent requests happen only on
  their own explicit click: Connect / Paste key and connect / Test connection
  (the word "Hi", one token), Sign in with OpenRouter (OAuth PKCE; loopback
  callback on 127.0.0.1, the key it issues goes to the Keychain) and Find AI
  on this Mac (loopback model lists). Connecting is one step on purpose
  (owner: "stupid easy"); docs/AI_PROVIDERS.md "Connecting". Custom URL, model and key start
  empty; switching the endpoint deletes the old key and clears consent. A
  saved legacy named provider id fails closed (no fallback to another network
  service) and meetings are kept. `python3 scripts/check-ai-provider-policy.py`
  must pass; both release scripts run it and then scan the signed artifact for
  credentials and retired service hosts (the four preset hosts are
  inventoried, not failed). Spec: `docs/AI_PROVIDERS.md`.
- **Sync is device to device** (`docs/SYNC.md`): iPhone ↔ Mac directly on
  the local network, TLS pinned to the Mac's certificate, Pro only. Never
  add a relay, iCloud or any server to it, never send removed (deleted or
  struck) text in a sync message, and keep removals crossing before content.
  The policy guard checks the sync code has no URL, HTTP client or AI call.
- **Transcription is on-device only:** local Whisper on the Mac, Apple speech
  on iOS. The cloud transcription modules (Deepgram, Gladia, Google, Gemini)
  were removed from the source; don't bring them back.
- **Never ship or hardcode an API key**, and never hardcode a private host (the
  old Castle/GX10 tailnet URL was removed). User-entered keys live only in the
  Keychain (`secrets.rs` on the Mac, `KeychainStore` on iOS), bound to the
  normalized endpoint: changing the endpoint deletes the old key and clears
  its consent and model, and a key is never reused at another destination.
  Never return a key to the UI (only `last4`), and never log keys or
  transcript text.
- **Consent before sending meeting content to a public endpoint.** Whether an
  endpoint is local depends on its URL (loopback, private ranges, `.local`,
  Tailscale), never on a name. Public endpoints need HTTPS; plain HTTP only
  for private hosts. The consent dialog shows the actual destination.
- **Contact and legal links:** `https://nofriction.io/privacy`,
  `https://nofriction.io/contact`, `casey@nofriction.io`; Terms are Apple's
  standard EULA (`src/lib/build.ts`, `ios/NoFriction/App/AppLinks.swift`).
- **Subscriptions:** noFriction Pro, `com.nofriction.meetings.pro.monthly`
  ($0.99/month) and `.pro.yearly` ($5.99/year), 1-week free trial each, USA
  only, Family Sharing off. Recording and microphone transcription stay free; Pro list in docs/PRO.md (owner decision 2026-10-10).
- **Delete and Strike must purge everywhere.** The checklist is in
  `docs/REDACTION.md`. Any new place that stores transcript or screen text must
  be added to that purge.
- **Apple Watch app** (`ios/NoFrictionWatch`, bundle
  `com.nofriction.meetings.watchkitapp`, embedded in the iOS app):
  - The watch only records. The iPhone transcribes on-device from the
    transferred file.
  - The watch deletes a part only after the iPhone app acknowledges it, not on
    `didFinish` success.
  - The policy guard rejects network or speech APIs in the watch target.
  - Simulators can't deliver WatchConnectivity files, so delivery must be
    verified on real devices. See `docs/WATCH_APP.md`.
- **v2: hands-free, typed notes, the notebook's page** (plan and shared
  rules: `docs/design/FADELL_V2.md`; owner request 2026-11-12):
  - **A recording never starts by itself unless the user turned that on.**
    Up next only offers (a card, the tray's first item, a notification);
    **Record by itself** (`auto_start_recording`) is off by default and its
    setting says "Tell people you're recording." A call that was on when a
    recording stopped must not start another (`up_next::CallTracker`).
  - **No AI runs while recording.** The note field keeps what is typed; the
    AI works after Stop. Don't bring back live AI cards. Automatic notes
    wait until no recording is running (`meeting_notes::run::after_stop`;
    iPhone `AutoNotes`, which checks before every request); only the
    user's own **Make notes** may run during another recording.
  - **A picture reaches an AI only on a click that says so.** One feature
    sends images: **Read the screens** (`screen_reading.rs`; FADELL_V2.md
    §12). Its confirmation shows every picture that would go, with a tick
    box each, and names the destination; the ticked ids and that host and
    model are passed back, and `read_screens` sends those ids there and
    nothing else. It never works the list out again, re-validates each
    screen, and makes each request with `ai::complete_vision_if`, which
    resolves the target once, compares it to the confirmed host and model
    and sends on that same target (never check with one config snapshot
    and send with another). At
    most 12 a click, one picture per request, never by itself, never on a
    timer, never for the recording that is running, never while a Delete
    is in its undo window. The prompt is fixed text: no transcript, title,
    app or window name goes with a picture. Private screens are never
    offered (`screen_prompts::never_sent`: mail, messages, password apps,
    terminals, private browsing when the title says so (Safari's doesn't),
    key files, noFriction's own window);
    that is decided on the Mac, not left to the model. It works from the
    window a screen was captured from; a whole-display capture is judged
    by the window that was in front on that display; one of noFriction's
    own is passed over unless it fills the display, which is then left
    out (`privacy_filter::front_window_on`, stored in
    `screen_states.front_app` / `front_title` at capture, never sent).
    Screens recorded before that have no such note. A private window that was
    only visible behind another can't be recognised, so the confirmation
    shows every picture whole and enlargeable: don't crop those tiles and
    don't claim more than this in user-facing text. The run and its
    sender are private to `screen_reading.rs`; only the confirmation's
    command reaches them. Only an answer marks a screen read. An endpoint
    that refuses or fails stops the run and leaves the screen unread (once
    it has read a picture in that run, one rejection is skipped as the
    picture's own problem; a second in a row stops it).
    Captions and picture picks still send text about a picture, never the
    picture. The policy guard lists every file allowed to build an image
    message; add to that list on purpose, with the click that leads there.
    `vlm_scheduler.rs` (a timer that sent queued screenshots) is retired;
    don't start it or queue screens for it.
  - **What was read from a screen is a text snapshot on that screen**
    (`source = 'screen_reading'`), so the screen's purge is its purge
    (docs/REDACTION.md: no purge filters `text_snapshots` by source).
    Every other reader of `text_snapshots` that shows or uses the text
    leaves these rows out (Links, chat search, the timeline, the Obsidian
    export, the data editor's list): they are an AI's words, not text
    captured from the screen. A new reader of that table must leave them
    out too; the policy guard lists the files that may read it.
    Don't give readings to Make notes, Fill in the blanks or any other AI
    prompt until a screen's purge also cleans what was made from them.
    The prompts are a library (`screen_prompts.rs`): one entry per kind
    of screen (a photo from the iPhone has its own), the kind decided on
    the device from the app first and the window title only for a browser
    or an unknown app, and rules every prompt shares (only what is
    visible, never a person's appearance, a call read for what is shared
    and never its people or chat, `[secret]` for anything like a
    password, `skip` for private or empty screens). A rule that protects
    someone goes in the shared rules: a whole-display capture has no app
    name and gets the `Other` prompt.
  - **Nothing automatic asks the AI while a recording runs.** Work nobody
    clicked for (automatic notes, the topics that follow notes) goes
    through `study::BackgroundCompleter`, which waits for the recording
    to stop before every request. A Meeting's notes use
    `settings.meeting_report_prompt` however they are made; the old
    prompt library's themes are not read.
  - **Notes are saved in one transaction with their check**
    (`meeting_notes::save_checked`: `BEGIN IMMEDIATE`, then "no pending
    delete and every line read is still there", then the insert). A check
    followed by a separate write lets a Delete or Strike land in between.
    iPhone: `NotesStore.save` is one synchronous step on the model context;
    never put an `await` between its check and its write, and send every
    notes write through it.
  - **A typed note's detail never outlives the note.** Any new way to
    delete or change a `note`-kind marker must call
    `meeting_notes::typed_notes_changed` (it prunes `note_details` and
    touches the notes row so sync resends them). iPhone: go through
    `TypedNoteStore` (edit, delete, and the two sync paths), and keep
    details paired with notes by identity
    (`WhatYouWrote.finalize(_:asked:now:)`), never by position alone.
  - **Up next and the notebook's page call no AI and send nothing.** They
    are computed on the device from the calendar (only if access was
    already granted: never prompt for it there), the recordings, and which
    call apps hold the microphone. The policy guard's rules still apply.
  - **A typed note is a mark of kind `note`**, never a fourth mark kind:
    not in the Marks list, its filter or its count, and not a kind a mark
    can be changed to. It syncs as a mark; a version that doesn't know the
    kind skips it without an error (keep that true for any new kind). On
    iPhone use `MarkerKind.markKinds` and `Meeting.marks` for anything that
    lists marks (`allCases` and `markers` include notes); the watch decodes
    marks only.
  - **The quick-note panel is the note field with another way in**
    (`quick_note.rs`, ⌃⌥⌘N on the Mac; FADELL_V2.md §10). A line goes
    through `markers::commands::keep_note`, like the in-app field's: don't
    give the panel its own storage, AI or setting, and never let it start
    a recording. The panel must be created non-activating
    (`NSWindowStyleMaskNonactivatingPanel` at init; set later, the window
    server is out of step) and must keep ⌘ keys to itself while it has the
    keyboard (⌘Q would reach the app's menu and quit mid-recording). Its
    callbacks must not panic: the shipped build aborts on one. A save can
    land after the panel was closed or reopened, so it goes through
    `quick_note::settle`. Check a change with
    `cargo run --example quick_note_panel` in `src-tauri/`: it runs the
    panel without the app or its data. Don't take a screenshot to look at
    it (`screencapture` puts a Screen Recording dialog on the owner's
    screen); the example's `RENDER=` saves the panel's own drawing.
  - **Fill in the blanks is a second pass kept with the notes**
    (`meeting_notes/fill.rs`; FADELL_V2.md §11). It runs only on the
    user's click, through the same gates as Make notes. The user's typed
    lines are never rewritten. Background (what a textbook says) is
    allowed only for a Class and is always marked; a Meeting or Personal
    recording gets only what was said. The result is saved under the same
    transcript check as notes and is removed outright by
    `redact_ai_outputs`, by a typed note it was made beside changing
    (`typed_notes_changed`) and by Make again. The two headings and their
    bullet forms are the same on both platforms (`fill::markdown`),
    because they cross in the notes' Markdown. iPhone: the request is in
    `FillInTheBlanks.swift`, the save is the synchronous
    `NotesStore.saveFill` (no `await` between its check and its write),
    and every removal calls `FillInTheBlanks.removed(from:)`. Neither
    Make notes nor Fill in the blanks sends anything while a Delete is in
    its undo window, on either platform.
  - **Notes read the whole recording.** Never truncate a transcript for a
    prompt; condense it in parts (`study::condense_to_budget`,
    `MeetingAI`'s condense step). Prompts carry marks and typed notes by
    time, never by id.
  - **The notebook's page stores no content.** Ticks are a hash of the
    item's text per recording (`notebook_item_done`); anything else it shows
    is read from notes, marks and guides, so their purge is its purge.
  - **Yellow is for actions, focus and the ★ mark.** Typed notes, counts
    and chips are neutral (design review 2026-11-12).
  - **iPhone reminders are local notifications planned from the routine**
    (`RoutineReminders`, at most 24 pending). Permission is asked only from
    the Record tab's card, whose button says **Continue**; never at launch
    and never from a background path. The iPhone never starts a recording
    by itself: the **Start Recording** shortcut opens the app first, and a
    request made during first-run setup is dropped. What the iPhone does
    differently and what was never run on a device: FADELL_V2.md §9.
- Two Mac flavors must always build: the default (Developer ID DMG) and
  `--features mas` (sandboxed Mac App Store build). In `mas`: no ffmpeg,
  Accessibility, osascript, shell plugin or `.env`; Pro gating goes through
  `entitlement::require_pro()` in `ai::client::complete`.

**Lessons that cost real time:**
- **A hook's cleanup must not depend on a caller's inline callback.**
  `useUndoDelete(onError)` built `commit` with `[onError]`, callers passed
  an arrow function, so the "leaving the view applies the delete" cleanup
  ran on the next render and every such delete skipped its Undo window.
  Read caller callbacks through a ref when an effect's cleanup has a side
  effect.
- **Never log an AI error's text.** A provider's error body can repeat the
  transcript it was sent. Log a kind (`meeting_notes::run::failure_kind`).
- **Migrations run on ONE connection** (`run_migrations` acquires a single
  connection). Running them back-to-back on `&pool` opened extra connections
  mid-migration, and one could keep a stale schema ("no such table" on fresh
  installs, about 30% under load). To reproduce flaky DB bugs, run the lib test
  binary with `--test-threads=32` 10+ times.
- **Private frameworks:** App Review 2.5.1 rejects a binary with any
  `/System/Library/PrivateFrameworks/` load command, even if nothing calls it.
  The cpal ScreenCaptureKit fork pulled in `cidre` with default features, whose
  `am` module links `MobileDevice.framework`. cpal is vendored at
  `src-tauri/vendor/cpal` with cidre limited to the ScreenCaptureKit feature set
  (no `am`, no `private`); the release scripts check `otool -L`. Check that before adding any
  crate that binds Apple frameworks.
- **Permission pre-prompts** (5.1.1): a button shown before a system
  permission dialog must not say "Allow". Use "Continue" (Setup screen on the
  Mac, Permissions page on iPhone).
- **Signing:** a custom "Always Trust" on the Developer ID cert makes codesign
  emit a non-Apple-anchored requirement. The app then fails its own signature
  check, macOS can't attach permission grants to it, and the mic and screen
  prompts repeat forever. `release-macos.sh` now refuses such a build. Check
  with `codesign -dr - <app>` (it must contain `anchor apple generic`).
- **Screen capture** without Screen Recording permission shows the macOS prompt
  on every capture call. Always check permission (`CGPreflightScreenCaptureAccess`)
  before capturing.
- **Whisper invents text on silence** ("Bye-bye. Bye-bye."). Keep
  `transcription/filter.rs` in the path of every provider, and never feed
  filtered text back as prompt context.
- **Stop must call `end_meeting`.** It once didn't, so every meeting had zero
  duration and reports never ran.
- **Before reinstalling the Mac app,** check that no recording is running
  (latest transcript timestamp). Quitting the app mid-recording loses the stop.
- **The Mac app's DB uses WAL.** When the app has no open connections,
  `sqlite3 -readonly` fails to open it. For the pre-reinstall recording check,
  use a normal connection that only SELECTs:
  `sqlite3 -cmd ".timeout 10000" "$DB" "select max(timestamp) from transcripts"`
  (never select transcript text).
- The bundle ID is `com.nofriction.meetings` everywhere (it can't change; the user-facing name is just "noFriction", product name set in `tauri.conf.json`). The Mac data folder
  migrated from `ai.nofriction.meetings`. `paths.rs` never deletes old data.
- **Controls don't inherit a font.** `button`, `input`, `select` and
  `textarea` keep the browser's control font unless told to inherit (the
  whole Record sheet rendered in it until 2026-11-12). The rule is in
  `src/App.css`. The typeface is the system face (owner decision
  2026-11-13): no font is bundled, so don't name one the app doesn't ship.
- **Check the UI in the film harness before claiming a Mac UI change works:**
  `HARNESS_PORT=5194 npx vite --config marketing/film/mac-harness/vite.config.mjs`,
  then `http://127.0.0.1:5194/?cursor=1` (`&upnext=routine|calendar|call`).
  A new Tauri command needs a case in `mock/backend.ts`. A blank page with
  no console error is a missing module export.
- **Shell gotchas:** `wc -l` pads with spaces, so compare with `-gt`/`-eq` or
  `tr -d ' '`. macOS `sed` doesn't support `0,/re/`. Don't run `npm run build`
  for checks, because its prebuild bumps `build_number.txt`; use
  `npx tsc && npx vite build`.

**Verify before claiming done:**
- `cargo test --lib` and `cargo test --lib --features mas` (in `src-tauri/`)
- `npx tsc --noEmit`
- the iOS command: `xcodebuild test -project ios/NoFriction.xcodeproj -scheme NoFriction -only-testing:NoFrictionTests` on a simulator

**Release docs:** `docs/APPLE_SETUP.md` (portal steps) and
`docs/APPLE_SETUP_CLOSEOUT_20261003.md` (what is configured and what remains),
`docs/LAUNCH_CHECKLIST.md`, `docs/APP_STORE_RELEASE.md` (status + owner tasks),
`docs/MAC_APP_STORE_BUILD.md`, `scripts/release-macos.sh` (DMG),
`scripts/release-mas.sh` (Mac App Store), `site/` (privacy/support pages).

## Skill routing

When the user's request matches an available skill, ALWAYS invoke it using the Skill
tool as your FIRST action. Do NOT answer directly, do NOT use other tools first.
The skill has specialized workflows that produce better results than ad-hoc answers.

Key routing rules:
- Product ideas, "is this worth building", brainstorming → invoke office-hours
- Bugs, errors, "why is this broken", 500 errors → invoke investigate
- Ship, deploy, push, create PR → invoke ship
- QA, test the site, find bugs → invoke qa
- Code review, check my diff → invoke review
- Update docs after shipping → invoke document-release
- Weekly retro → invoke retro
- Design system, brand → invoke design-consultation
- Visual audit, design polish → invoke design-review
- Architecture review → invoke plan-eng-review
- Save progress, checkpoint, resume → invoke checkpoint
- Code quality, health check → invoke health
