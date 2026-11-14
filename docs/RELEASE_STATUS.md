# Release status and agent coordination

Read this before starting work. Several agents work in this repo at once.
Last updated 2026-11-14 by the release/distribution session. Update it when you change any of the facts below.

## In the stores now (updated 2026-11-14)

| What | Version (build) | State | Notes |
|---|---|---|---|
| App Store, iPhone/iPad (+ Watch) | 1.0.1 (10) | **Released 2026-10-14** (Ready for Sale) | First public release |
| App Store, iPhone | 1.0.2 (11) | **Released** (Ready for Sale) | Search jumps to the line; new iPhone preview video |
| App Store, iPhone | 1.1.0 (12) | **Released 2026-10-23** (Ready for Sale) | Export… (#81), one-step AI connect with Sign in with OpenRouter (#82), new description and promo text (#85) |
| Mac App Store | 3.9.0 (48) | **Released** (Ready for Sale). First Mac release | Make lesson, Export…, one-step AI connect, ⌘E, and the Stoichiometry course inside the app (#92). The website's Mac App Store badge is on (`macLive: true`, #101) |
| App Store review ("v2") | iPhone/iPad 2.0.0 (14) · Mac 4.0.0 (49) | **Waiting for review**, both submitted 2026-11-14 (iOS submission 13b2df21, Mac ffd65ae6), release automatically on approval | The owner said to submit without the device pass ("just submit it"). Screenshots and previews are the 1.1.0 / 3.9.0 ones. Neither build has Read the screens, the capture rule or the Export fix (see "Mac 4.0.1" below) |
| Subscriptions (Pro Monthly/Yearly) | — | **Approved** | |
| TestFlight external (https://testflight.apple.com/join/4JnJD3Fk) | iOS 10, 11 · Mac 43, 44, 45 | In beta testing | |
| TestFlight internal | iOS 2.0.0 (14) · Mac 4.0.0 (49) | Uploaded 2026-11-13 | The internal group's only tester never accepted the invite, so nobody receives these through TestFlight. The owner's own devices are on the public group below (builds up to iOS 11 / Mac 48). To put 14 and 49 there they must be added to "noFriction Beta" and pass Beta App Review |

Website: App Store badges go live when the public listing URL resolves (PR #72).
The Mac badge has been on since Mac 3.9.0 was released (`macLive: true` in
`site/js/store.js`, #101).

## Version plan

| Train | Contents | Owner |
|---|---|---|
| iPhone 1.0.2 / Mac 3.7.1 | **Merged (#60, #63, #64)**, building for TestFlight: macOS 13 minimum, Pro label only for non-Pro, full screen names, SHA-256 on the speech model, 12 unused crates removed, iPhone search jumps to the line | release session |
| Mac 3.8.0 | **Merged (#75, #76)**, build 45 on TestFlight: Lessons tab, stoichiometry course, import limits. Make lesson is a "coming soon" stub. | release session (took over from Kiro 2026-10-14) |
| iOS 1.1.0 | Export… and one-step AI connect (#81, #82). **Released 2026-10-23** | release session |
| Mac 3.9.0 | Make lesson (#87), Export…, one-step AI connect, File → Export Recording… (⌘E) | release session |
| iOS 1.2.0 | Lessons on iPhone (not started) | unassigned |
| Mac 4.0.1 (next Mac build; not built) | On `main` since 2026-11-13/14, **not in Mac 4.0.0 (49)**: Read the screens with the prompt library for pictures (#104); no screenshot while a password app or a private browser window is in front (#107); Fill in the blanks carried into Export, the Obsidian export and Chat, and the system's mono typeface (#108). Before it is built and submitted: (1) the App Review notes and What's New prepared in `docs/APP_STORE_LISTING.md` §8 ("for any build made from `main` after 2026-11-13"); (2) the privacy page: pictures go to the user's AI only on Read the screens, and what is not captured, each with its limits; (3) a pass in the real app: Read the screens with a real image model (never done: no picture has been sent to a model), a recording with 1Password or an incognito window in front (the capture rule), an exported package and an Obsidian note with filled sections. **If the v2 device pass fails and 4.0.0 is rebuilt from `main`, that rebuild is this build**: all of the above applies to it | release session |
| Mac 4.0.0 / iOS 2.0.0 ("v2") | Hands-free start (Up next, reminders, opt-in Record by itself), typed notes the AI finishes, notes that read the whole recording, the notebook's page (#97 Mac, #98 iPhone/iPad); the system typeface (#99); the quick-note panel, ⌃⌥⌘N over any app (#100, Mac); **Fill in the blanks** (#103 Mac, #105 iPhone/iPad); nothing automatic asks the AI while a recording runs, and no picture is sent by any feature (#102). Plan and what wasn't verified: `docs/design/FADELL_V2.md` (§9 iPhone, §10 the panel, §11 Fill in the blanks). **Built and uploaded 2026-11-13: iPhone/iPad 2.0.0 (14) and Mac 4.0.0 (49), attached to their App Store versions with the store text from `docs/APP_STORE_LISTING.md`. Not submitted for review**: that waits for the device pass under "Owner to-dos". What has run in the real Mac app (a sandboxed local build with an empty data folder): the migration, both shortcuts registering, the panel's notice over another app, a mark from the shortcut, a recording with transcription. What has not: a typed note saved from the app or the panel, Stop opening Notes with What you wrote, Fill in the blanks with a real AI, anything on a real iPhone, iPad or Watch. The Mac migration rebuilds `meeting_markers` once (tested against a 3.9.0 table); the iPhone adds one model (`NotebookItemDone`) with no schema version. Screenshots and previews are not updated for v2. **Not in these builds: Read the screens** (#104, the prompt library for pictures). It is on `main` for the next Mac build. Before that build is submitted: the App Review notes and store text must stop saying no image is sent to an AI (`docs/APP_STORE_LISTING.md` §8), the website's privacy page must say that pictures go to the user's AI when they press Read the screens, and it needs a pass in the real app with a real image model (FADELL_V2 §12, "Not verified"). The same build also stops capturing a screen while a password app or a private browser window is in front (#107): the privacy page's sentence on Mac screenshots should say so once that build is live, with its two limits (Safari's private windows, and windows behind another). **Any Mac build made from `main` after 2026-11-13 has both**, including a rebuilt 4.0.0: use the review notes prepared for it in `docs/APP_STORE_LISTING.md` §8 | v2 session (2026-11-12/13); distribution as usual |

## Rules for every agent

- **Distribution.** Only the release session uploads builds, bumps
  `ios/build_number.txt` or `src-tauri/build_number.txt`, changes version
  fields, or touches App Store Connect and TestFlight. Hand it a merged PR.
  Never run `release-*.sh --upload`.
- **Checks.** Never run `npm run build`, because its prebuild bumps the build
  number. Use `npx tsc --noEmit && npx vite build`.
- **Branches.** Work in a worktree on your own branch. Rebase on `origin/main`
  before opening a PR and again before merge. Ask before touching another
  agent's files.
- **Pro.** `docs/PRO.md` is the single source of truth. New Pro features need a
  key in both `src/lib/pro.ts` and `ios/NoFriction/Store/ProFeature.swift`; a
  test checks that they match. AI goes through `ai::client::complete`. Gate
  non-AI Mac features with `entitlement::require_pro_feature`.
- **Navigation.** On the Mac (3.8.0+) it is Record · Recordings · Lessons ·
  Chat, ⌘1–⌘4, with a **Make lesson** button (owner decision 2026-10-10).
  DESIGN.md has the four-view rule.
- **Invariants.** Follow the CLAUDE.md noFriction section: client-only, sync
  device to device, Delete/Strike purge everywhere, no logging of transcript
  text, and both Mac flavors build.
- **Shared machine resources.**
  - The headed gstack browser on port 34567 belongs to the release session and
    is signed in to Apple. Don't stop or reconnect it.
  - The simulators `NF iPhone 17 Pro Max`, `NF-Audit iPhone 17 Pro` and
    `NF iPad Pro 13` are in use. Create your own.

## Owner to-dos (Casey)

- **Run in the real Mac app on 2026-11-14** (the direct-install build made
  from `main` that day, installed on the owner's Mac over 3.6.0, library
  backed up first): the migration of a real library (6 recordings, same
  counts after), both shortcuts registering, a recording with microphone
  and screen, a typed note saved from the note field, the ⌃⌥⌘N panel
  opening over another app (not captured, the other app staying in front),
  Stop ending the recording and opening Notes with **What you wrote**, and
  Read the screens refusing with no image model set up. **Still not run
  anywhere real:** a note saved through the panel, Fill in the blanks and
  Read the screens with a real AI, the capture rule with a password app in
  front, and everything on iPhone, iPad and Watch.
- **v2 was submitted on 2026-11-14 without a device pass** (owner's call).
  Still worth five minutes on a real iPhone once 2.0.0 is on your phone
  (after approval, or through TestFlight if 14 is put on the public group):
  type a note while recording and press Stop (Notes opens with **What you
  wrote**); **Up next** with a calendar event in the next 15 minutes; a
  notebook's page; **Fill in the blanks** with your AI; a recording from
  the Apple Watch; sync with the Mac. Tell the release session what fails.
- **The Mac installer certificate expires 2026-12-27** ("3rd Party Mac Developer
  Installer", the one that signs the Mac App Store package; seen while building
  49). After that date `scripts/release-mas.sh` can't package a Mac build.
  Create a new **Mac Installer Distribution** certificate at
  https://developer.apple.com/account/resources/certificates/add and install it
  in the login keychain (`docs/APPLE_SETUP.md`). Apple Distribution is good
  until 2027-09-22 and Developer ID Application until 2030-12-28.
- Test on real devices via TestFlight: screen capture, iPhone↔Mac sync, and the
  Watch.
- Sign in to GoDaddy in the gstack browser so the release session can revoke
  the token pasted in chat. DNS no longer depends on it.

## Launch-day runbook (after Apple approves)

1. **Owner:** press Release for iPhone 1.0.1 and Mac 3.7.0 at
   https://appstoreconnect.apple.com/apps/6818838861/distribution. Release is
   manual.
2. **Release session, website:** set `live: true` in `site/js/store.js`. The
   "Get notified" blocks become App Store and Mac App Store badges. The links
   are already in that file (id 6818838861). Then:
   - Merge it; CI deploys.
   - Check that both badges open the store pages.
3. **Release session, first update** (iPhone 1.0.2 build 11, Mac 3.7.1 build 44):
   - Create the 1.0.2 and 3.7.1 App Store versions and attach builds 11 and 44.
   - Copy the text from `docs/APP_STORE_LISTING.md`. Fill What's New, which
     applies now that these aren't first versions.
   - Upload the new previews from `marketing/out/previews/`. The Mac one is
     rendered; the iPhone one is in progress.
   - Replace each subscription's review screenshot with
     `ios/AppStore/review/subscription-paywall.png`.
   - Submit. The subscriptions are approved by then, so the API submit path
     works (`docs/APP_STORE_RELEASE.md` §5.9–5.10).
4. **Lessons (Mac 3.8.0):** when the Lessons PR merges, review it against the
   owner decision above, build 3.8.0, run TestFlight, then submit.

Homepage film: assets on the `site-media` release, fetched by the deploy
workflow (`infra/site/README.md`).
