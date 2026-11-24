//! Storage (docs/STORAGE.md): what noFriction keeps on this Mac, how big it
//! is, and making it smaller.
//!
//! The owner, 2026-11-24: "add a space management and prune feature and make
//! it a centric data point in the app. its very easy for this thing to eat
//! space. perhaps make the images smaller."
//!
//! What takes the room, measured on his Mac that day (1.6 GB for 10
//! recordings):
//! - the screen video the direct-download build recorded (about 22 MB a
//!   minute) that no screen of the app ever showed: retired, see
//!   `build_info::VIDEO_RECORDING`, and removable here;
//! - screens saved at full Retina size (2560 to 3024 px wide): new ones are
//!   saved at `SCREEN_MAX_SIDE`, old ones can be made smaller in place;
//! - the speech model, the database and the rest, which are only shown.
//!
//! Rules:
//! - Sizes are counted from the files, never guessed.
//! - Making a picture smaller keeps its row and its path, so nothing changes
//!   for sync, export, Rewind or Read the screens (which never sends more
//!   than 1568 px anyway). It writes a new encoding beside the file and
//!   swaps it in under the redaction lock, only if the picture is still
//!   there: a Delete or Strike that came first wins, and nothing it removed
//!   comes back.
//! - Removing a recording's screens is a Delete of those screens (the same
//!   purge, `redaction::prune_screens`): what was read from them goes too,
//!   and a paired iPhone removes its copies at the next sync. Pictures that
//!   came from the iPhone are never removed by it.
//! - Nothing here runs on a recording that is still being recorded, and no
//!   file name, title or text is logged.

use crate::redaction::RedactionEnv;
use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};
use std::path::{Path, PathBuf};

/// The long side of a screen as it is saved. Read the screens sends at most
/// 1568 px, and a Retina screenshot at this size is still read comfortably.
pub const SCREEN_MAX_SIDE: u32 = 1600;
/// The long side kept for a photo that came from the iPhone (a whiteboard
/// or a slide: more detail is worth keeping than for a screen).
pub const PHOTO_MAX_SIDE: u32 = 2048;
/// JPEG quality of a picture saved or re-saved smaller.
pub const JPEG_QUALITY: u8 = 60;
/// JPEG quality of a screen saved at full size (what it was before Storage).
pub const FULL_SIZE_QUALITY: u8 = 75;

/// `settings` key: "smaller" (default) or "full".
pub const SETTING_SCREEN_SIZE: &str = "space_screen_size";
/// `settings` key: days a recording's screens are kept; "0" (default) = forever.
pub const SETTING_KEEP_SCREENS_DAYS: &str = "space_keep_screens_days";
/// The choices offered for it.
pub const KEEP_DAYS_CHOICES: [u32; 4] = [0, 365, 90, 30];

/// A new encoding waiting to replace its picture (`.shrink-<uuid>.part`).
const PART_PREFIX: &str = ".shrink-";

fn err<E: std::fmt::Display>(ctx: &'static str) -> impl Fn(E) -> String {
    move |e| format!("{}: {}", ctx, e)
}

// ═══════════════════════════════════════════════════════════════════════════
// Pure pieces
// ═══════════════════════════════════════════════════════════════════════════

/// A size the way Finder says it (1 MB = 1,000,000 bytes): "0 KB", "412 KB",
/// "87 MB", "1.6 GB".
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    if b < MB {
        format!("{} KB", (b / KB).round() as u64)
    } else if b < 100.0 * MB {
        let v = b / MB;
        if v < 10.0 { format!("{:.1} MB", v) } else { format!("{} MB", v.round() as u64) }
    } else if b < GB {
        format!("{} MB", (b / MB).round() as u64)
    } else {
        let v = b / GB;
        if v < 100.0 { format!("{:.1} GB", v) } else { format!("{} GB", v.round() as u64) }
    }
}

/// The size a `w` x `h` picture is saved at so that its long side is at most
/// `max_side`. A picture already within it keeps its size.
pub fn fit(w: u32, h: u32, max_side: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= max_side || long == 0 {
        return (w, h);
    }
    let scale = max_side as f64 / long as f64;
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

/// The limit for a picture by its file name: a photo from the iPhone
/// (`sync_…`) keeps more than a screen.
pub fn max_side_for(file_name: &str) -> u32 {
    if file_name.starts_with("sync_") { PHOTO_MAX_SIDE } else { SCREEN_MAX_SIDE }
}

/// A picture file this module may re-save: a JPEG that isn't a hidden or
/// temporary file. PNGs from the iPhone are left as they are.
pub fn is_shrinkable_name(file_name: &str) -> bool {
    if file_name.starts_with('.') {
        return false;
    }
    let lower = file_name.to_ascii_lowercase();
    lower.ends_with(".jpg") || lower.ends_with(".jpeg")
}

/// Roughly what a picture of `bytes` shrinks by when its long side goes from
/// `long` to `max_side`: bytes follow the number of pixels. For "saves about".
pub fn estimated_saving(bytes: u64, long: u32, max_side: u32) -> u64 {
    if long <= max_side || long == 0 {
        return 0;
    }
    let keep = (max_side as f64 / long as f64).powi(2);
    (bytes as f64 * (1.0 - keep)).round() as u64
}

/// Recordings whose screens are due under "Keep screens for `days`": started
/// more than `days` ago. `days == 0` keeps everything. `(id, started_at)`.
pub fn screens_due(recordings: &[(String, chrono::DateTime<chrono::Utc>)], now: chrono::DateTime<chrono::Utc>, days: u32) -> Vec<String> {
    if days == 0 {
        return Vec::new();
    }
    let cutoff = now - chrono::Duration::days(days as i64);
    recordings.iter().filter(|(_, started)| *started < cutoff).map(|(id, _)| id.clone()).collect()
}

/// "smaller" unless the setting says "full".
pub fn smaller_from_setting(value: Option<&str>) -> bool {
    value.map(str::trim) != Some("full")
}

/// Days from the setting; anything that isn't one of the choices keeps forever.
pub fn keep_days_from_setting(value: Option<&str>) -> u32 {
    value.and_then(|v| v.trim().parse::<u32>().ok()).filter(|d| KEEP_DAYS_CHOICES.contains(d)).unwrap_or(0)
}

// ═══════════════════════════════════════════════════════════════════════════
// Encoding
// ═══════════════════════════════════════════════════════════════════════════

/// `img` as a JPEG whose long side is at most `max_side`. Always a new
/// encoding: nothing of a file's metadata is in it but `icc`, the colour
/// profile, when the picture had one (a photo from the iPhone is Display
/// P3; without its profile its colours would shift).
pub fn encode_jpeg(img: &image::DynamicImage, max_side: u32, quality: u8, icc: Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    use image::ImageEncoder;
    let (w, h) = fit(img.width(), img.height(), max_side);
    let rgb = if (w, h) == (img.width(), img.height()) {
        img.to_rgb8()
    } else {
        // CatmullRom keeps small text readable where Triangle blurs it
        img.resize_exact(w, h, image::imageops::FilterType::CatmullRom).to_rgb8()
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    if let Some(icc) = icc {
        let _ = encoder.set_icc_profile(icc);
    }
    encoder
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(err("Couldn't encode the picture"))?;
    Ok(out.into_inner())
}

/// Save a screen that was just captured. `smaller`: at `SCREEN_MAX_SIDE`;
/// otherwise at the size and quality it always had. Returns the bytes written.
pub fn save_screen(img: &image::DynamicImage, path: &Path, smaller: bool) -> Result<u64, String> {
    let bytes = if smaller { encode_jpeg(img, SCREEN_MAX_SIDE, JPEG_QUALITY, None)? } else { encode_jpeg(img, u32::MAX, FULL_SIZE_QUALITY, None)? };
    std::fs::write(path, &bytes).map_err(err("Couldn't save the screen"))?;
    Ok(bytes.len() as u64)
}

/// A picture the way a viewer shows it, with its colour profile: a photo
/// from the iPhone keeps its rotation in EXIF, which a new encoding doesn't
/// carry, so it is turned upright here.
fn open_upright_with_profile(path: &Path) -> image::ImageResult<(image::DynamicImage, Option<Vec<u8>>)> {
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path)?.with_guessed_format()?.into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let icc = decoder.icc_profile().ok().flatten();
    let mut img = image::DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    Ok((img, icc))
}

/// What became of one picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shrunk {
    /// Already small enough, not a JPEG, or the new encoding wasn't smaller
    Kept,
    /// Replaced: bytes before and after
    Smaller { before: u64, after: u64 },
    /// It was deleted or stricken meanwhile: nothing was written
    Gone,
}

/// The new encoding of the picture at `path`, written beside it as a `.part`
/// file, or None when the picture stays as it is. Slow (decodes the image):
/// call it off the async threads. Returns (part path, bytes before, bytes after).
pub fn prepare_smaller(path: &Path) -> Result<Option<(PathBuf, u64, u64)>, String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !is_shrinkable_name(name) {
        return Ok(None);
    }
    let max_side = max_side_for(name);
    let before = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return Ok(None),
    };
    let (w, h) = image::image_dimensions(path).map_err(err("Couldn't read the picture"))?;
    if w.max(h) <= max_side {
        return Ok(None);
    }
    let (img, icc) = open_upright_with_profile(path).map_err(err("Couldn't read the picture"))?;
    let bytes = encode_jpeg(&img, max_side, JPEG_QUALITY, icc)?;
    let after = bytes.len() as u64;
    if after >= before {
        return Ok(None);
    }
    let part = path.with_file_name(format!("{}{}.part", PART_PREFIX, uuid::Uuid::new_v4().simple()));
    std::fs::write(&part, &bytes).map_err(err("Couldn't write the smaller picture"))?;
    Ok(Some((part, before, after)))
}

/// Put a prepared encoding in place of its picture, if the picture is still
/// there. The two files are exchanged in one step that fails when the
/// picture is gone (`renamex_np(RENAME_SWAP)`), so nothing a Delete or
/// Strike removed, and no folder of a recording deleted meanwhile, can be
/// written back, whichever lock the delete held. The caller also holds the
/// redaction lock.
pub fn swap_in(part: &Path, path: &Path, before: u64, after: u64) -> Shrunk {
    let done = exchange(part, path);
    // After the exchange this is the original; either way it goes
    let _ = std::fs::remove_file(part);
    match done {
        Exchange::Done => Shrunk::Smaller { before, after },
        Exchange::Missing => Shrunk::Gone,
        Exchange::Failed => Shrunk::Kept,
    }
}

enum Exchange {
    Done,
    /// The picture isn't there any more
    Missing,
    Failed,
}

#[cfg(target_os = "macos")]
fn exchange(part: &Path, path: &Path) -> Exchange {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(from), Ok(to)) = (std::ffi::CString::new(part.as_os_str().as_bytes()), std::ffi::CString::new(path.as_os_str().as_bytes())) else {
        return Exchange::Failed;
    };
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } == 0 {
        return Exchange::Done;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ENOENT) => Exchange::Missing,
        // A volume that can't exchange files: look, then move over the picture
        Some(libc::ENOTSUP) if path.exists() => {
            if std::fs::rename(part, path).is_ok() { Exchange::Done } else { Exchange::Failed }
        }
        Some(libc::ENOTSUP) => Exchange::Missing,
        _ => Exchange::Failed,
    }
}

#[cfg(not(target_os = "macos"))]
fn exchange(part: &Path, path: &Path) -> Exchange {
    if !path.exists() {
        return Exchange::Missing;
    }
    if std::fs::rename(part, path).is_ok() { Exchange::Done } else { Exchange::Failed }
}

/// Encodings left by a run that was cut short (a quit, a crash): at launch.
pub fn remove_leftover_parts(data: &Path) {
    let Ok(dirs) = std::fs::read_dir(data.join("frames")) else { return };
    for dir in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(PART_PREFIX) && name.ends_with(".part") {
                let _ = std::fs::remove_file(f.path());
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Measuring
// ═══════════════════════════════════════════════════════════════════════════

/// Bytes and files under `dir`, hidden files included. A folder that isn't
/// there is empty.
pub fn dir_usage(dir: &Path) -> (u64, u64) {
    let mut bytes = 0u64;
    let mut files = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            // A link is never followed out of the folder
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else if meta.is_file() {
                bytes += meta.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

fn file_bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Bytes free for the user on the volume that holds `path`.
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// A plain recording id (the same test as `commands::meeting_file_dirs`).
fn safe_id(id: &str) -> bool {
    !crate::commands::meeting_file_dirs(Path::new("/x"), Path::new("/y"), id).is_empty()
}

fn screens_dir(data: &Path, id: &str) -> PathBuf {
    data.join("frames").join(id)
}

fn video_dir(data: &Path, id: &str) -> PathBuf {
    data.join(id).join("video")
}

/// One recording's files.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct RecordingSpace {
    pub meeting_id: String,
    pub title: String,
    pub started_at: String,
    /// Screens and photos: bytes and how many
    pub screens_bytes: u64,
    pub screens_count: u64,
    /// Screen video from earlier versions of the direct-download build
    pub video_bytes: u64,
    pub total_bytes: u64,
    /// Roughly what making its pictures smaller frees (0: nothing to do)
    pub saves_bytes: u64,
    /// It is being recorded now: nothing can be done to it yet
    pub recording: bool,
}

/// What making the pictures smaller would do.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Default)]
pub struct ShrinkEstimate {
    /// Pictures larger than their limit
    pub pictures: u64,
    /// Roughly the bytes it frees
    pub saves_bytes: u64,
}

/// Everything, for the Storage page.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Overview {
    pub total_bytes: u64,
    /// Free on the disk; None when it can't be read
    pub free_bytes: Option<u64>,
    pub screens_bytes: u64,
    pub screens_count: u64,
    pub video_bytes: u64,
    /// Speech models (`models/`)
    pub models_bytes: u64,
    /// The library file and its journal
    pub database_bytes: u64,
    /// Files no recording uses: screens captured with no recording running
    /// (`snapshots/`), pictures cut from the old screen video (in the cache
    /// folder), and folders left behind by recordings that were deleted
    pub leftover_bytes: u64,
    /// Logs, lessons, backups and anything else in the data folder
    pub other_bytes: u64,
    pub shrink: ShrinkEstimate,
    /// Largest first
    pub recordings: Vec<RecordingSpace>,
    pub screen_size: &'static str,
    pub keep_screens_days: u32,
}

fn shrink_estimate_dir(dir: &Path) -> ShrinkEstimate {
    let mut est = ShrinkEstimate::default();
    let Ok(files) = std::fs::read_dir(dir) else { return est };
    for f in files.flatten() {
        let name = f.file_name();
        let name = name.to_string_lossy();
        if !is_shrinkable_name(&name) {
            continue;
        }
        // The header only: no pixels are decoded
        let Ok((w, h)) = image::image_dimensions(f.path()) else { continue };
        let saves = estimated_saving(file_bytes(&f.path()), w.max(h), max_side_for(&name));
        if saves > 0 {
            est.pictures += 1;
            est.saves_bytes += saves;
        }
    }
    est
}

/// Folders of the old screen video's cut pictures in the cache folder
/// (`<cache>/<recording>/frames|thumbnails`). The web view keeps its own
/// cache in the same folder: that is never counted or cleared here.
fn cut_picture_dirs(cache: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(cache) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && (p.join("frames").is_dir() || p.join("thumbnails").is_dir()))
        .collect()
}

/// Folders a deleted recording left behind: `frames/<id>` and `<id>`
/// (its video) for an id that is no recording's any more.
fn orphan_dirs(data: &Path, known: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let is_orphan = |name: &str| safe_id(name) && !known.iter().any(|k| k == name);
    if let Ok(entries) = std::fs::read_dir(data.join("frames")) {
        out.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(is_orphan)));
    }
    if let Ok(entries) = std::fs::read_dir(data) {
        // Only a folder that holds a screen video is a recording's
        out.extend(
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join("video").is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(is_orphan)),
        );
    }
    out
}

fn leftover_dirs(data: &Path, cache: &Path, known: &[String]) -> Vec<PathBuf> {
    let mut dirs = vec![data.join("snapshots")];
    dirs.extend(cut_picture_dirs(cache));
    dirs.extend(orphan_dirs(data, known));
    dirs
}

/// Measure everything. Blocking (it walks the folders): `overview` runs it
/// off the async threads.
fn measure(data: &Path, cache: &Path, recordings: Vec<(String, String, String)>, recording_now: &[String]) -> Overview {
    let mut o = Overview::default();
    let known: Vec<String> = recordings.iter().map(|r| r.0.clone()).collect();
    for (id, title, started_at) in &recordings {
        if !safe_id(id) {
            continue;
        }
        let (screens_bytes, screens_count) = dir_usage(&screens_dir(data, id));
        // The recording's own folder is the video and its sidecar files
        let (video_bytes, _) = dir_usage(&data.join(id));
        let est = shrink_estimate_dir(&screens_dir(data, id));
        o.shrink.pictures += est.pictures;
        o.shrink.saves_bytes += est.saves_bytes;
        o.screens_bytes += screens_bytes;
        o.screens_count += screens_count;
        o.video_bytes += video_bytes;
        o.recordings.push(RecordingSpace {
            meeting_id: id.clone(),
            title: title.clone(),
            started_at: started_at.clone(),
            screens_bytes,
            screens_count,
            video_bytes,
            total_bytes: screens_bytes + video_bytes,
            saves_bytes: est.saves_bytes,
            recording: recording_now.iter().any(|m| m == id),
        });
    }
    o.recordings.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes).then_with(|| b.started_at.cmp(&a.started_at)));
    o.models_bytes = dir_usage(&data.join("models")).0;
    o.database_bytes = ["nofriction_meetings.db", "nofriction_meetings.db-wal", "nofriction_meetings.db-shm"]
        .iter()
        .map(|f| file_bytes(&data.join(f)))
        .sum();
    o.leftover_bytes = leftover_dirs(data, cache, &known).iter().map(|d| dir_usage(d).0).sum();
    // The data folder, and of the cache folder only what is noFriction's own
    let cut: u64 = cut_picture_dirs(cache).iter().map(|d| dir_usage(d).0).sum();
    o.total_bytes = dir_usage(data).0 + cut;
    o.other_bytes = o
        .total_bytes
        .saturating_sub(o.screens_bytes + o.video_bytes + o.models_bytes + o.database_bytes + o.leftover_bytes);
    o.free_bytes = free_bytes(data);
    o
}

async fn recordings(pool: &Pool<Sqlite>) -> Result<Vec<(String, String, String)>, String> {
    let rows = sqlx::query("SELECT id, title, started_at FROM meetings ORDER BY started_at DESC")
        .fetch_all(pool)
        .await
        .map_err(err("Couldn't read the recordings"))?;
    Ok(rows.iter().map(|r| (r.get("id"), r.get("title"), r.get("started_at"))).collect())
}

/// Everything noFriction keeps on this Mac, measured now.
pub async fn overview(pool: &Pool<Sqlite>, env: &RedactionEnv, screen_size: Option<String>, keep_days: Option<String>) -> Result<Overview, String> {
    let list = recordings(pool).await?;
    let (data, cache, now) = (env.app_data_dir.clone(), env.cache_dir.clone(), env.recording_meetings.clone());
    let mut o = tokio::task::spawn_blocking(move || measure(&data, &cache, list, &now))
        .await
        .map_err(err("Couldn't measure the storage"))?;
    o.screen_size = if smaller_from_setting(screen_size.as_deref()) { "smaller" } else { "full" };
    o.keep_screens_days = keep_days_from_setting(keep_days.as_deref());
    Ok(o)
}

/// Bytes per recording, for the list. Cheap: file sizes only.
pub async fn sizes(pool: &Pool<Sqlite>, env: &RedactionEnv) -> Result<Vec<(String, u64)>, String> {
    let list = recordings(pool).await?;
    let data = env.app_data_dir.clone();
    tokio::task::spawn_blocking(move || {
        list.into_iter()
            .filter(|(id, _, _)| safe_id(id))
            .map(|(id, _, _)| {
                let bytes = dir_usage(&screens_dir(&data, &id)).0 + dir_usage(&data.join(&id)).0;
                (id, bytes)
            })
            .collect()
    })
    .await
    .map_err(err("Couldn't measure the recordings"))
}

// ═══════════════════════════════════════════════════════════════════════════
// Making it smaller
// ═══════════════════════════════════════════════════════════════════════════

/// What a run did.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Default)]
pub struct Freed {
    /// Pictures re-saved, or files and screens removed
    pub items: u64,
    pub bytes: u64,
}

fn picture_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(files) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = files
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(is_shrinkable_name))
        .collect();
    out.sort();
    out
}

/// Re-save the pictures of one recording (`Some(id)`) or of every recording
/// at their limit. The recording that is being recorded is skipped.
/// `progress(done, total, bytes freed so far)`.
pub async fn make_smaller(
    pool: &Pool<Sqlite>,
    env: &RedactionEnv,
    meeting_id: Option<&str>,
    progress: &(dyn Fn(u64, u64, u64) + Send + Sync),
) -> Result<Freed, String> {
    let ids: Vec<String> = match meeting_id {
        Some(id) => vec![id.to_string()],
        None => recordings(pool).await?.into_iter().map(|(id, _, _)| id).collect(),
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for id in ids.iter().filter(|id| safe_id(id) && !env.recording_meetings.contains(id)) {
        files.extend(picture_files(&screens_dir(&env.app_data_dir, id)));
    }
    let total = files.len() as u64;
    let mut freed = Freed::default();
    for (i, path) in files.into_iter().enumerate() {
        let for_blocking = path.clone();
        let prepared = tokio::task::spawn_blocking(move || prepare_smaller(&for_blocking))
            .await
            .map_err(err("Couldn't make a picture smaller"))?;
        // One unreadable picture doesn't stop the rest
        if let Ok(Some((part, before, after))) = prepared {
            let _lock = crate::redaction::lock().await;
            if let Shrunk::Smaller { before, after } = swap_in(&part, &path, before, after) {
                freed.items += 1;
                freed.bytes += before - after;
            }
        }
        progress(i as u64 + 1, total, freed.bytes);
    }
    Ok(freed)
}

/// Remove the screen video of one recording (`Some(id)`) or of all of them.
/// Nothing in noFriction shows it; the transcript, screens and notes stay.
/// A recording that is being recorded, or whose video a Strike is still
/// being cut out of, is left for later.
pub async fn remove_video(pool: &Pool<Sqlite>, env: &RedactionEnv, meeting_id: Option<&str>) -> Result<Freed, String> {
    // Only ever a recording's own folder: the id has to be one
    let ids: Vec<String> = recordings(pool)
        .await?
        .into_iter()
        .map(|(id, _, _)| id)
        .filter(|id| meeting_id.is_none_or(|m| m == id))
        .collect();
    let _lock = crate::redaction::lock().await;
    let mut freed = Freed::default();
    for id in ids.iter().filter(|id| safe_id(id) && !env.recording_meetings.contains(id)) {
        let dir = env.app_data_dir.join(id);
        if !dir.exists() {
            continue;
        }
        // The blanking worker runs outside the lock: its chunk isn't pulled
        // from under it
        let running: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM video_blank_jobs WHERE meeting_id = ? AND status = 'running'")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap_or(0);
        if running > 0 {
            continue;
        }
        let (bytes, files) = dir_usage(&dir);
        if std::fs::remove_dir_all(&dir).is_err() {
            continue;
        }
        freed.items += files;
        freed.bytes += bytes;
        // Pictures cut from that video
        let _ = std::fs::remove_dir_all(env.cache_dir.join(id));
        // With no chunk left, nothing remains to be cut out of it
        let _ = sqlx::query("DELETE FROM video_blank_jobs WHERE meeting_id = ?").bind(id).execute(pool).await;
    }
    Ok(freed)
}

/// Files no recording uses (`leftover_dirs`): screens captured with no
/// recording running, pictures cut from the old screen video, and folders
/// left by recordings that were deleted. The web view's cache is not touched.
pub async fn clear_leftovers(pool: &Pool<Sqlite>, env: &RedactionEnv) -> Result<Freed, String> {
    let known: Vec<String> = recordings(pool).await?.into_iter().map(|r| r.0).collect();
    let (data, cache) = (env.app_data_dir.clone(), env.cache_dir.clone());
    // A recording deleted this instant isn't mistaken for a leftover half-way
    let _lock = crate::redaction::lock().await;
    tokio::task::spawn_blocking(move || {
        let mut freed = Freed::default();
        let snapshots = data.join("snapshots");
        for dir in leftover_dirs(&data, &cache, &known) {
            let (bytes, files) = dir_usage(&dir);
            if dir == snapshots {
                // The folder stays; what is in it goes
                for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                    let p = e.path();
                    let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
                }
            } else {
                let _ = std::fs::remove_dir_all(&dir);
            }
            freed.items += files;
            freed.bytes += bytes.saturating_sub(dir_usage(&dir).0);
        }
        freed
    })
    .await
    .map_err(err("Couldn't clear the leftover files"))
}

/// Remove the screens this Mac captured for a recording, and its screen
/// video. A Delete of those screens (`redaction::prune_screens`): what was
/// read from them goes with them and a paired iPhone removes its copies.
/// The transcript, the notes and photos from the iPhone stay.
pub async fn remove_screens(pool: &Pool<Sqlite>, env: &RedactionEnv, meeting_id: &str) -> Result<Freed, String> {
    if !safe_id(meeting_id) {
        return Err("That recording can't be found".into());
    }
    if env.recording_meetings.iter().any(|m| m == meeting_id) {
        return Err("Stop the recording first.".into());
    }
    let before = dir_usage(&screens_dir(&env.app_data_dir, meeting_id)).0;
    // With the video gone first, no chunk has to be blanked for these screens
    let video = remove_video(pool, env, Some(meeting_id)).await?;
    let removed = crate::redaction::prune_screens(pool, env, meeting_id).await?;
    let after = dir_usage(&screens_dir(&env.app_data_dir, meeting_id)).0;
    Ok(Freed { items: removed as u64, bytes: video.bytes + before.saturating_sub(after) })
}

/// Apply "Keep screens for": remove the screens of recordings older than
/// the setting. Does nothing with the default (forever) or while anything
/// is being recorded. Returns what it freed.
pub async fn apply_retention(pool: &Pool<Sqlite>, env: &RedactionEnv, keep_days: u32, now: chrono::DateTime<chrono::Utc>) -> Result<Freed, String> {
    let mut freed = Freed::default();
    if keep_days == 0 || !env.recording_meetings.is_empty() {
        return Ok(freed);
    }
    let list: Vec<(String, chrono::DateTime<chrono::Utc>)> = recordings(pool)
        .await?
        .into_iter()
        .filter_map(|(id, _, started)| {
            chrono::DateTime::parse_from_rfc3339(&started).ok().map(|d| (id, d.with_timezone(&chrono::Utc)))
        })
        .collect();
    for id in screens_due(&list, now, keep_days) {
        // Only recordings that still have something to remove
        if dir_usage(&screens_dir(&env.app_data_dir, &id)).1 == 0 && !video_dir(&env.app_data_dir, &id).exists() {
            continue;
        }
        match remove_screens(pool, env, &id).await {
            Ok(f) => {
                freed.items += f.items;
                freed.bytes += f.bytes;
            }
            // The reason may name a file: it stays out of the log
            Err(_) => log::warn!("Storage: a recording's screens couldn't be removed"),
        }
    }
    if freed.bytes > 0 {
        log::info!("Storage: kept screens for {} days, freed {}", keep_days, format_bytes(freed.bytes));
    }
    Ok(freed)
}

// ═══════════════════════════════════════════════════════════════════════════
// Tauri commands
// ═══════════════════════════════════════════════════════════════════════════

pub mod commands {
    use super::*;
    use crate::AppState;
    use tauri::{AppHandle, Emitter, State};

    /// `{ done, total, bytes }` while pictures are made smaller.
    pub const PROGRESS_EVENT: &str = "space_progress";
    /// Sizes changed: lists that show them read them again.
    pub const CHANGED_EVENT: &str = "space_changed";

    fn env(app: &AppHandle) -> RedactionEnv {
        crate::redaction::commands::env_from(app)
    }

    async fn setting(state: &AppState, key: &str) -> Option<String> {
        state.settings.get(key).await.ok().flatten()
    }

    #[tauri::command]
    pub async fn space_overview(app: AppHandle, state: State<'_, AppState>) -> Result<Overview, String> {
        let (size, keep) = (setting(&state, SETTING_SCREEN_SIZE).await, setting(&state, SETTING_KEEP_SCREENS_DAYS).await);
        overview(state.database.pool(), &env(&app), size, keep).await
    }

    /// `[[recording id, bytes]]` for the Recordings list.
    #[tauri::command]
    pub async fn space_sizes(app: AppHandle, state: State<'_, AppState>) -> Result<Vec<(String, u64)>, String> {
        sizes(state.database.pool(), &env(&app)).await
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn space_make_smaller(app: AppHandle, state: State<'_, AppState>, meeting_id: Option<String>) -> Result<Freed, String> {
        let to = app.clone();
        let freed = make_smaller(state.database.pool(), &env(&app), meeting_id.as_deref(), &move |done, total, bytes| {
            let _ = to.emit(PROGRESS_EVENT, serde_json::json!({ "done": done, "total": total, "bytes": bytes }));
        })
        .await?;
        log::info!("Storage: {} pictures made smaller, {} freed", freed.items, format_bytes(freed.bytes));
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(freed)
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn space_remove_video(app: AppHandle, state: State<'_, AppState>, meeting_id: Option<String>) -> Result<Freed, String> {
        let freed = remove_video(state.database.pool(), &env(&app), meeting_id.as_deref()).await?;
        log::info!("Storage: screen video removed, {} freed", format_bytes(freed.bytes));
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(freed)
    }

    #[tauri::command(rename_all = "camelCase")]
    pub async fn space_remove_screens(app: AppHandle, state: State<'_, AppState>, meeting_id: String) -> Result<Freed, String> {
        let freed = remove_screens(state.database.pool(), &env(&app), &meeting_id).await?;
        log::info!("Storage: {} screens of one recording removed, {} freed", freed.items, format_bytes(freed.bytes));
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(freed)
    }

    #[tauri::command]
    pub async fn space_clear_leftovers(app: AppHandle, state: State<'_, AppState>) -> Result<Freed, String> {
        let freed = clear_leftovers(state.database.pool(), &env(&app)).await?;
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(freed)
    }

    /// "smaller" or "full": how new screens are saved, from the next recording.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn space_set_screen_size(state: State<'_, AppState>, size: String) -> Result<(), String> {
        let value = if size == "full" { "full" } else { "smaller" };
        state.settings.set(SETTING_SCREEN_SIZE, value).await.map_err(err("Couldn't save the setting"))
    }

    /// Days a recording's screens are kept (0 = forever). A shorter time
    /// takes effect at once.
    #[tauri::command(rename_all = "camelCase")]
    pub async fn space_set_keep_screens(app: AppHandle, state: State<'_, AppState>, days: u32) -> Result<Freed, String> {
        if !KEEP_DAYS_CHOICES.contains(&days) {
            return Err("That isn't one of the choices.".into());
        }
        state.settings.set(SETTING_KEEP_SCREENS_DAYS, &days.to_string()).await.map_err(err("Couldn't save the setting"))?;
        let freed = apply_retention(state.database.pool(), &env(&app), days, chrono::Utc::now()).await?;
        let _ = app.emit(CHANGED_EVENT, ());
        Ok(freed)
    }

    /// At launch and once a day: clear what a cut-short run left, then apply
    /// "Keep screens for". Waits out a recording.
    pub fn start_housekeeping(app: AppHandle) {
        remove_leftover_parts(&crate::paths::app_data_dir());
        tauri::async_runtime::spawn(async move {
            use tauri::Manager;
            // Not in the way of the app coming up
            tokio::time::sleep(std::time::Duration::from_secs(90)).await;
            loop {
                let mut again = std::time::Duration::from_secs(24 * 3600);
                if let Some(state) = app.try_state::<AppState>() {
                    let days = keep_days_from_setting(setting(&state, SETTING_KEEP_SCREENS_DAYS).await.as_deref());
                    let env = env(&app);
                    if days > 0 && !env.recording_meetings.is_empty() {
                        // Not while something is recorded: soon after it stops
                        again = std::time::Duration::from_secs(600);
                    } else if days > 0 {
                        match apply_retention(state.database.pool(), &env, days, chrono::Utc::now()).await {
                            Ok(freed) if freed.bytes > 0 => {
                                let _ = app.emit(CHANGED_EVENT, ());
                            }
                            Ok(_) => {}
                            Err(_) => log::warn!("Storage: keeping screens for {} days failed", days),
                        }
                    }
                }
                tokio::time::sleep(again).await;
            }
        });
    }
}

#[cfg(test)]
mod tests;
