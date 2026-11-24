# Storage

The owner, 2026-11-24: "you need to add a space management and prune feature
and make it a centric data point in the app. its very easy for this thing to
eat space. perhaps make the images smaller. do this for both app versions."

What follows is what takes the room, what the app now does about it, and
the rules that keep it safe. Mac: `src-tauri/src/space.rs`,
`src/features/settings/StorageSettings.tsx`, `src/lib/space.ts`. iPhone and
iPad: `ios/NoFriction/Space/`.

## What was taking the room

Measured on the owner's Mac that day: 1.6 GB of app data for 10 recordings,
and 11 GB of library copies made before each install of a test build
(`~/Library/Application Support/noFriction-backups`, made by the release
session, not by the app).

| What | Size there | Why |
|---|---|---|
| Screen video | about 900 MB | The direct-download build recorded the main display with ffmpeg at about 22 MB a minute. No screen of the app ever showed it: Rewind uses the screens. The Mac App Store build never recorded it |
| Speech model | 547 MB | The Whisper model that transcribes on the Mac |
| Screens | 266 MB, 672 pictures | Saved at the size the screen was captured, 2560 to 3024 px wide on a Retina display |
| Everything else | about 12 MB | The library file, logs, lessons |

On iPhone and iPad the room goes to the audio kept for every recording
(AAC at 64 kbit/s, about 29 MB an hour) and to camera photos, which were
saved at the camera's full resolution (several MB each).

## Mac

### What changed for new recordings

- **No screen video.** `build_info::VIDEO_RECORDING` is false in every
  build and `start_video_recording` refuses. The recorder, the blanking of
  old chunks on a Strike (`redaction/video_blank.rs`) and the old commands
  stay, because video made by earlier versions is still on disk until the
  user removes it.
- **Screens are saved smaller.** `space::save_screen` saves a captured
  screen with its long side at `SCREEN_MAX_SIDE` (1600 px), JPEG quality
  60. On the owner's library that is about 42% of the bytes of the same
  screens at full size. Read the screens never sent more than 1568 px, so
  what an AI is given does not change. Settings → Storage → **Save
  screens** offers **Full size** (`space_screen_size`); it is read once
  when a recording starts.

### Settings → Storage

Everything is measured from the files (`space::overview`), never
estimated: the total, the free space on the disk (`statvfs`), a bar with
one part each for Screens and photos, Screen video, Speech model,
Transcripts and notes (the library file), Leftover files and Everything
else, and the recordings, largest first.

| Action | What it does | What it doesn't |
|---|---|---|
| **Make smaller** (all, or one recording) | Re-saves JPEGs whose long side is over the limit (1600 px for a screen, 2048 px for a photo from the iPhone) at that limit, in place. The saving shown first is an estimate from the pictures' sizes; the toast says what was freed | The row, the path, what was read from the screen, sync, export. PNGs. Pictures already small. The recording being recorded |
| **Remove** (Screen video) | Deletes `<data>/<recording>/` (the chunks and their sidecar files), the pictures cut from it in the cache, and blanking jobs for it | Screens, transcripts, notes |
| **Clear** (Leftover files) | Empties `<data>/snapshots` (screens captured with **Capture screen** while nothing was recording: no row, no screen lists them), removes the pictures cut from the old video (`<cache>/<recording>/frames`, `thumbnails`) and the folders a deleted recording left behind | Anything a recording uses. The web view's own cache, which lives in the same cache folder |
| **Remove screens** (one recording) | A Delete of the screens this Mac captured for it (`redaction::prune_screens`, the same purge as any Delete of screens) and its screen video | The transcript, notes, guides, and pictures that came from the iPhone (`source_key = 'sync'`) |
| **Keep screens for** Forever / 1 year / 90 days / 30 days | With anything but Forever (the default), Remove screens is applied to recordings older than that: when the setting is chosen (after a confirmation that names how many recordings and how much), 90 seconds after launch and once a day; while a recording runs it waits and looks again every ten minutes | Nothing while a recording runs. Nothing by default |

Anything that removes asks once more in place before it does.

### The size is on the main screen

- Each row of the Recordings list shows what the recording holds on disk
  (from 1 MB up), after its time and length.
- Under the notebook chips: "Your recordings use X on this Mac." with a
  link to Settings → Storage.
- `space_changed` is emitted after every action; the list and the page
  read the sizes again.

### A link that opens it

`nofriction://settings/storage` opens Settings at Storage
(`src-tauri/src/app_link.rs`; the scheme is in `Info.plist`). Any Settings
page works: `nofriction://settings/<recording | transcription | ai | sync |
storage | subscription | about>`. A link only opens a page: it carries
nothing into the app and starts nothing, and anything else is ignored.

### Rules

- **A smaller picture never brings a deleted one back.** The new encoding
  is written beside the picture as `.shrink-<uuid>.part` and the two files
  are exchanged in one step that fails if the picture is gone
  (`renamex_np(RENAME_SWAP)`), under the redaction lock. If a Delete or
  Strike removed the picture first, or the whole recording was deleted,
  the new encoding is deleted and no folder is made again. Parts left by a
  quit are removed at launch (`remove_leftover_parts`).
- **A new encoding carries nothing of the old file but its colour
  profile.** No EXIF. A photo from the iPhone is turned upright first (its
  rotation was in EXIF) and keeps its Display P3 profile.
- **The old video is not pulled from under a Strike.** While a blanking
  job for a recording is running, its video is left for later; its jobs
  are deleted only once its folder is gone. Only the id of a real
  recording ever becomes a path.
- **The Mac still stays awake while it records** in the direct-download
  build. That was held by starting the screen video; `start_recording`
  holds it now.
- **Removing screens is a Delete.** Rows go, so a paired iPhone removes
  its copies of those screens at the next sync, and what Read the screens
  read from them goes too. The page says so before it happens.
- **Photos from the iPhone are the user's own pictures.** A prune never
  removes them; Make smaller keeps more of them (2048 px).
- **Nothing here touches a recording that is being recorded**, and
  nothing here logs a title, a path or text.
- The policy guard pins the retired video, the swap under the lock, the
  iPhone-photo exclusion and that `space.rs` makes no network or AI call.

### Not done

- The speech model can only be changed in Transcription settings; the
  page links there.
- The library file is not compacted (VACUUM) by this page.
- "Remove screens" purges those screens from each app backup too, as any
  Delete does. For "Keep screens for" on a large library that is slow.
- Thinning (keeping one screen a minute) was considered and left out: the
  capture already skips screens that did not change.
- `StorageManager` and `ChunkManager` (older helpers with no caller in
  the UI) still look in folders nothing writes to. They were left as they
  are.

## iPhone and iPad

`ios/NoFriction/Space/`: `SpaceUsage.swift` (sizes, `SpaceIndex`),
`PictureShrinker.swift`, `SpacePrune.swift`, `SpaceView.swift`.

### What changed for new recordings

- **New photos are saved smaller.** A camera or library photo added while
  recording is saved with its long side at 2048 px, JPEG quality 0.75,
  upright, with no location or camera details in the file
  (`PictureShrinker.shrunk`, called from `RecordingSession.addSnapshot`).
  Settings → Storage → **New photos** offers **Original**
  (`space.photoSize`). Screens from screen capture were already 1280 px.

### Settings → Storage

"noFriction uses X" and "Y free on this iPhone", then Audio, Photos and
screens, and Everything else (the store, Apple Watch recordings still
arriving, what Sync remembers), measured from the files.

| Action | What it does | What it doesn't |
|---|---|---|
| **Make photos smaller** (all, or one recording) | Re-saves JPEGs over 2048 px at that size, in place. "Saves about X" is an estimate; "Freed X" is what happened | The `Snapshot` row, so nothing is re-sent by Sync and the Mac keeps the copy it has. PNGs. The recording being recorded |
| **Remove audio** (one recording) | Deletes the audio file and sets `Meeting.audioRemovedAt`. The recording's page says "Audio removed to save space on <date>. The transcript is kept." where playback was | The transcript, notes, photos and screens. Refused while the recording is being recorded or still being transcribed |
| **Keep audio for** Forever / 1 year / 90 days / 30 days | With anything but Forever (the default), Remove audio is applied to saved recordings that ended longer ago than that: when it is chosen (after a confirmation that names how many recordings and how much), at launch and when the Storage screen opens | Nothing by default. Nothing still importing or being recorded |
| **Delete recording** | The existing delete, with its confirmation | |

### The size is on the main screen

- Each row of the Recordings list shows the recording's size from 1 MB up.
- Under the list: "N recordings · X on this iPhone", which opens Storage.

### Rules

- **Space is never saved by deleting a row.** A deleted `Snapshot` or
  `Meeting` row would be deleted on the Mac at the next sync. Only Delete
  recording does that.
- **Audio never leaves the iPhone**, so removing it changes nothing on the
  Mac. It can't be played again afterwards; the confirmation says so.
- **Removed audio is not put back.** A watch recording delivered again is
  treated as already imported when `audioRemovedAt` is set
  (`WatchImporter`). A file that a Strike from the Mac re-created while the
  audio was being removed is swept at launch and when Storage opens
  (`sweepRemovedAudio`), and goes with Delete recording
  (`RecordingDeletion`, by the recording's own file names).
- **A smaller photo never brings a deleted one back.** The new file is
  written beside the original as `.shrinking-<uuid>.jpg` and exchanged with
  it in one step that fails if the original is gone
  (`renamex_np(RENAME_SWAP)`). Files left by a kill are removed at launch.
- The privacy manifest declares the free-space API (DiskSpace, 85F4.1:
  shown to the user).

### Not done, not verified

- Run on the simulator only (unit tests and one UI test): not on a real
  iPhone or iPad, not with real camera photos, not while a sync is running.
- Removing audio ends the undo window of a Delete that is still pending
  (pending edits are finished first).
- A new photo is made smaller on the main actor when it is added.
