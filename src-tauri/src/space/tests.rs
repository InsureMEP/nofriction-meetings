//! Storage: sizes, smaller pictures and what a prune removes
//! (docs/STORAGE.md). Files are real, in a temp folder; no picture of a
//! real screen is used.

use super::*;
use crate::database::DatabaseManager;
use std::sync::Arc;

struct Fx {
    db: Arc<DatabaseManager>,
    dir: PathBuf,
    env: RedactionEnv,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn fx() -> Fx {
    let dir = std::env::temp_dir().join(format!("nf-space-{}", uuid::Uuid::new_v4()));
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    let db = DatabaseManager::new(&data.join("nofriction_meetings.db")).await.unwrap();
    db.run_migrations().await.unwrap();
    db.create_meeting("m1", "Kickoff").await.unwrap();
    db.create_meeting("m2", "Biology 101").await.unwrap();
    Fx {
        env: RedactionEnv { app_data_dir: data, cache_dir: dir.join("cache"), video_enabled: false, recording_meetings: Vec::new() },
        db: Arc::new(db),
        dir,
    }
}

/// A picture with detail in it (so it doesn't compress to nothing).
fn picture(w: u32, h: u32) -> image::DynamicImage {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
        let v = ((x * 7 + y * 13) % 251) as u8;
        image::Rgb([v, v.wrapping_mul(3), ((x ^ y) % 255) as u8])
    }))
}

/// A screen of `m` saved at full size, with its row. Returns its path.
async fn screen(f: &Fx, m: &str, name: &str, w: u32, h: u32, source: Option<&str>) -> PathBuf {
    let dir = f.env.app_data_dir.join("frames").join(m);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.jpg", name));
    save_screen(&picture(w, h), &path, false).unwrap();
    f.db.add_screen_state(name, m, chrono::Utc::now(), None, "", 0.0, Some(path.to_str().unwrap()), "other", "{}").await.unwrap();
    if let Some(s) = source {
        sqlx::query("UPDATE screen_states SET source_key = ? WHERE state_id = ?").bind(s).bind(name).execute(f.db.pool()).await.unwrap();
    }
    path
}

fn dims(p: &Path) -> (u32, u32) {
    image::image_dimensions(p).unwrap()
}

async fn rows(f: &Fx, m: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM screen_states WHERE meeting_id = ?").bind(m).fetch_one(f.db.pool()).await.unwrap()
}

#[test]
fn sizes_read_the_way_finder_says_them() {
    assert_eq!(format_bytes(0), "0 KB");
    assert_eq!(format_bytes(412_300), "412 KB");
    assert_eq!(format_bytes(1_000_000), "1.0 MB");
    assert_eq!(format_bytes(8_640_000), "8.6 MB");
    assert_eq!(format_bytes(86_900_000), "87 MB");
    assert_eq!(format_bytes(999_400_000), "999 MB");
    assert_eq!(format_bytes(1_600_000_000), "1.6 GB");
    assert_eq!(format_bytes(250_000_000_000), "250 GB");
}

#[test]
fn a_picture_is_fitted_to_its_limit_and_never_made_larger() {
    assert_eq!(fit(3024, 1964, 1600), (1600, 1039));
    assert_eq!(fit(1964, 3024, 1600), (1039, 1600));
    assert_eq!(fit(1440, 900, 1600), (1440, 900));
    assert_eq!(fit(1600, 1600, 1600), (1600, 1600));
    assert_eq!(fit(0, 0, 1600), (0, 0));
    assert_eq!(fit(5000, 1, 1600), (1600, 1));
    // A photo from the iPhone keeps more than a screen
    assert_eq!(max_side_for("sync_abc.jpg"), PHOTO_MAX_SIDE);
    assert_eq!(max_side_for("state_abc.jpg"), SCREEN_MAX_SIDE);
    assert!(is_shrinkable_name("state_1.jpg") && is_shrinkable_name("sync_1.JPEG"));
    assert!(!is_shrinkable_name("sync_1.png") && !is_shrinkable_name(".shrink-1.part") && !is_shrinkable_name(".hidden.jpg"));
    assert_eq!(estimated_saving(400_000, 3200, 1600), 300_000);
    assert_eq!(estimated_saving(400_000, 1600, 1600), 0);
    assert_eq!(estimated_saving(400_000, 0, 1600), 0);
}

#[test]
fn settings_fall_back_to_smaller_and_forever() {
    assert!(smaller_from_setting(None) && smaller_from_setting(Some("smaller")) && smaller_from_setting(Some("anything")));
    assert!(!smaller_from_setting(Some("full")) && !smaller_from_setting(Some(" full ")));
    assert_eq!(keep_days_from_setting(None), 0);
    assert_eq!(keep_days_from_setting(Some("90")), 90);
    assert_eq!(keep_days_from_setting(Some("7")), 0, "not one of the choices");
    assert_eq!(keep_days_from_setting(Some("soon")), 0);
    let now = chrono::Utc::now();
    let list = vec![
        ("old".to_string(), now - chrono::Duration::days(100)),
        ("new".to_string(), now - chrono::Duration::days(10)),
        ("edge".to_string(), now - chrono::Duration::days(90) + chrono::Duration::minutes(1)),
    ];
    assert_eq!(screens_due(&list, now, 90), vec!["old"]);
    assert_eq!(screens_due(&list, now, 30), vec!["old", "edge"]);
    assert!(screens_due(&list, now, 0).is_empty(), "forever keeps everything");
}

#[tokio::test]
async fn a_new_screen_is_saved_smaller_unless_the_setting_says_full() {
    let f = fx().await;
    let dir = f.env.app_data_dir.join("frames").join("m1");
    std::fs::create_dir_all(&dir).unwrap();
    let img = picture(3024, 1964);
    let small = save_screen(&img, &dir.join("a.jpg"), true).unwrap();
    let full = save_screen(&img, &dir.join("b.jpg"), false).unwrap();
    assert_eq!(dims(&dir.join("a.jpg")), (1600, 1039));
    assert_eq!(dims(&dir.join("b.jpg")), (3024, 1964));
    assert!(small * 2 < full, "{} vs {}", small, full);
    assert_eq!(small, std::fs::metadata(dir.join("a.jpg")).unwrap().len());
    // One that is already small keeps its size
    save_screen(&picture(1280, 800), &dir.join("c.jpg"), true).unwrap();
    assert_eq!(dims(&dir.join("c.jpg")), (1280, 800));
    // A colour profile is carried into the new encoding, and nothing else is
    let profile = vec![7u8; 600];
    let with = encode_jpeg(&picture(64, 64), 32, JPEG_QUALITY, Some(profile.clone())).unwrap();
    let without = encode_jpeg(&picture(64, 64), 32, JPEG_QUALITY, None).unwrap();
    assert!(with.windows(profile.len()).any(|w| w == profile.as_slice()));
    assert!(with.len() > without.len() && !without.windows(11).any(|w| w == b"ICC_PROFILE"));
}

#[tokio::test]
async fn existing_pictures_are_made_smaller_in_place_and_nothing_else_changes() {
    let f = fx().await;
    let big = screen(&f, "m1", "state_big", 3024, 1964, None).await;
    let small = screen(&f, "m1", "state_small", 1440, 900, None).await;
    let photo = screen(&f, "m1", "sync_photo", 4032, 3024, Some("sync")).await;
    let other = screen(&f, "m2", "state_other", 2560, 1440, None).await;
    // A PNG from the iPhone and a hidden file are left alone
    let png = f.env.app_data_dir.join("frames/m1/sync_slide.png");
    picture(3000, 2000).save(&png).unwrap();
    let before_small = std::fs::read(&small).unwrap();
    let before_png = std::fs::metadata(&png).unwrap().len();
    let before_total = dir_usage(&f.env.app_data_dir.join("frames")).0;

    let o = overview(f.db.pool(), &f.env, None, None).await.unwrap();
    assert_eq!(o.shrink.pictures, 3, "three JPEGs are over their limit");
    assert!(o.shrink.saves_bytes > 0);
    assert_eq!(o.screens_count, 5);
    assert_eq!(o.screens_bytes, before_total);
    assert_eq!((o.screen_size, o.keep_screens_days), ("smaller", 0));

    // One recording only
    let seen = std::sync::Mutex::new(Vec::new());
    let freed = make_smaller(f.db.pool(), &f.env, Some("m1"), &|done, total, bytes| seen.lock().unwrap().push((done, total, bytes))).await.unwrap();
    assert_eq!(freed.items, 2);
    assert_eq!(dims(&big), (1600, 1039));
    assert_eq!(dims(&photo), (2048, 1536), "a photo keeps more");
    assert_eq!(std::fs::read(&small).unwrap(), before_small, "already small: not re-encoded");
    assert_eq!(std::fs::metadata(&png).unwrap().len(), before_png);
    assert_eq!(dims(&other), (2560, 1440), "another recording is untouched");
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.last().map(|s| (s.0, s.1)), Some((3, 3)), "{:?}", seen);
    assert_eq!(seen.last().unwrap().2, freed.bytes);
    // What was freed is what the folder lost, and no temp file is left
    let after_total = dir_usage(&f.env.app_data_dir.join("frames")).0;
    assert_eq!(before_total - after_total, freed.bytes);
    assert!(std::fs::read_dir(f.env.app_data_dir.join("frames/m1")).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with('.')));
    // Rows and paths are as they were
    assert_eq!((rows(&f, "m1").await, rows(&f, "m2").await), (3, 1));
    let path: String = sqlx::query_scalar("SELECT keyframe_path FROM screen_states WHERE state_id = 'state_big'").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(PathBuf::from(path), big);

    // Everything, a second time: only what is left to do
    let again = make_smaller(f.db.pool(), &f.env, None, &|_, _, _| {}).await.unwrap();
    assert_eq!(again.items, 1);
    assert_eq!(dims(&other), (1600, 900));
    let o = overview(f.db.pool(), &f.env, Some("full".into()), Some("90".into())).await.unwrap();
    assert_eq!(o.shrink, ShrinkEstimate::default(), "nothing more to save");
    assert_eq!((o.screen_size, o.keep_screens_days), ("full", 90));
}

#[tokio::test]
async fn a_picture_deleted_meanwhile_is_never_put_back() {
    let f = fx().await;
    let path = screen(&f, "m1", "state_gone", 3024, 1964, None).await;
    let (part, before, after) = prepare_smaller(&path).unwrap().expect("a smaller encoding");
    assert!(part.exists() && after < before);
    // A Delete or Strike removed it before the swap
    std::fs::remove_file(&path).unwrap();
    assert_eq!(swap_in(&part, &path, before, after), Shrunk::Gone);
    assert!(!path.exists(), "nothing came back");
    assert!(!part.exists(), "and the new encoding is gone too");
    // The whole recording was deleted (its folder with it): the same
    let doomed = screen(&f, "m1", "state_doomed", 3024, 1964, None).await;
    let (part, before, after) = prepare_smaller(&doomed).unwrap().expect("a smaller encoding");
    std::fs::remove_dir_all(f.env.app_data_dir.join("frames/m1")).unwrap();
    assert_eq!(swap_in(&part, &doomed, before, after), Shrunk::Gone);
    assert!(!f.env.app_data_dir.join("frames/m1").exists(), "the folder is not made again");

    // The recording being recorded is skipped
    let live = screen(&f, "m2", "state_live", 3024, 1964, None).await;
    let mut env = RedactionEnv { recording_meetings: vec!["m2".into()], ..RedactionEnv::for_app() };
    env.app_data_dir = f.env.app_data_dir.clone();
    env.cache_dir = f.env.cache_dir.clone();
    let freed = make_smaller(f.db.pool(), &env, None, &|_, _, _| {}).await.unwrap();
    assert_eq!(freed, Freed::default());
    assert_eq!(dims(&live), (3024, 1964));
    assert!(remove_screens(f.db.pool(), &env, "m2").await.is_err());

    // What a cut-short run left is cleared at launch
    let stray = f.env.app_data_dir.join("frames/m2/.shrink-abc.part");
    std::fs::write(&stray, b"x").unwrap();
    remove_leftover_parts(&f.env.app_data_dir);
    assert!(!stray.exists() && live.exists());
}

#[tokio::test]
async fn the_screen_video_and_leftovers_are_measured_and_removed() {
    let f = fx().await;
    let data = &f.env.app_data_dir;
    screen(&f, "m1", "state_a", 1440, 900, None).await;
    std::fs::create_dir_all(data.join("m1/video")).unwrap();
    std::fs::write(data.join("m1/video/chunk_001.mov"), vec![0u8; 50_000]).unwrap();
    std::fs::write(data.join("m1/video/chunk_times.json"), b"{}").unwrap();
    std::fs::create_dir_all(data.join("m2/video")).unwrap();
    std::fs::write(data.join("m2/video/chunk_001.mov"), vec![0u8; 20_000]).unwrap();
    std::fs::create_dir_all(data.join("models")).unwrap();
    std::fs::write(data.join("models/ggml-base.bin"), vec![0u8; 30_000]).unwrap();
    std::fs::create_dir_all(data.join("snapshots")).unwrap();
    std::fs::write(data.join("snapshots/snap_1.jpg"), vec![0u8; 4_000]).unwrap();
    std::fs::create_dir_all(f.env.cache_dir.join("m1/frames")).unwrap();
    std::fs::write(f.env.cache_dir.join("m1/frames/frame_1.jpg"), vec![0u8; 6_000]).unwrap();
    std::fs::create_dir_all(data.join("logs")).unwrap();
    std::fs::write(data.join("logs/app.log"), vec![0u8; 1_000]).unwrap();
    // The web view's own cache lives in the same cache folder: never ours
    std::fs::create_dir_all(f.env.cache_dir.join("WebKit/NetworkCache")).unwrap();
    std::fs::write(f.env.cache_dir.join("WebKit/NetworkCache/blob"), vec![0u8; 8_000]).unwrap();
    // Folders a deleted recording left behind
    std::fs::create_dir_all(data.join("frames/gone-recording")).unwrap();
    std::fs::write(data.join("frames/gone-recording/state_x.jpg"), vec![0u8; 2_000]).unwrap();
    std::fs::create_dir_all(data.join("gone-recording/video")).unwrap();
    std::fs::write(data.join("gone-recording/video/chunk_001.mov"), vec![0u8; 3_000]).unwrap();
    std::fs::create_dir_all(data.join("lessons/set1")).unwrap();
    std::fs::write(data.join("lessons/set1/lesson.json"), vec![0u8; 500]).unwrap();

    let o = overview(f.db.pool(), &f.env, None, None).await.unwrap();
    assert_eq!(o.video_bytes, 70_002);
    assert_eq!(o.models_bytes, 30_000);
    assert_eq!(o.leftover_bytes, 15_000, "snapshots, cut pictures, and what a deleted recording left");
    assert!(o.database_bytes > 0);
    assert!(o.other_bytes >= 1_500);
    assert_eq!(o.total_bytes, dir_usage(data).0 + 6_000, "the data folder and the cut pictures; the web view's cache is not counted");
    assert_eq!(o.total_bytes, o.screens_bytes + o.video_bytes + o.models_bytes + o.database_bytes + o.leftover_bytes + o.other_bytes);
    assert!(o.free_bytes.is_some_and(|b| b > 0));
    assert_eq!(o.recordings.iter().map(|r| r.meeting_id.as_str()).collect::<Vec<_>>(), vec!["m1", "m2"], "largest first");
    assert_eq!(o.recordings[0].total_bytes, o.recordings[0].screens_bytes + 50_002);
    let list = sizes(f.db.pool(), &f.env).await.unwrap();
    assert_eq!(list.iter().find(|s| s.0 == "m2").map(|s| s.1), Some(20_000));

    // A Strike is still being cut out of m2's video: it is left for later
    sqlx::query("INSERT INTO video_blank_jobs (id, meeting_id, start_at, end_at, status, created_at, updated_at) VALUES ('j1', 'm2', '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z', 'running', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')")
        .execute(f.db.pool())
        .await
        .unwrap();
    assert_eq!(remove_video(f.db.pool(), &f.env, Some("m2")).await.unwrap(), Freed::default());
    assert!(data.join("m2/video/chunk_001.mov").exists());
    sqlx::query("UPDATE video_blank_jobs SET status = 'pending' WHERE id = 'j1'").execute(f.db.pool()).await.unwrap();

    // One recording's video, then all of it
    let one = remove_video(f.db.pool(), &f.env, Some("m2")).await.unwrap();
    let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM video_blank_jobs").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(jobs, 0, "with the video gone there is nothing left to cut out of it");
    assert_eq!(one.bytes, 20_000);
    assert!(!data.join("m2").exists() && data.join("m1/video/chunk_001.mov").exists());
    let all = remove_video(f.db.pool(), &f.env, None).await.unwrap();
    assert_eq!(all.bytes, 50_002);
    assert!(!data.join("m1").exists());
    assert!(!f.env.cache_dir.join("m1").exists(), "pictures cut from it go too");
    assert_eq!(rows(&f, "m1").await, 1, "the screens stay");
    assert!(data.join("frames/m1/state_a.jpg").exists());
    // Only a recording's id ever reaches the file system
    for not_one in ["../models", "lessons", "models", "gone-recording"] {
        assert_eq!(remove_video(f.db.pool(), &f.env, Some(not_one)).await.unwrap(), Freed::default(), "{}", not_one);
    }
    assert!(data.join("models/ggml-base.bin").exists() && data.join("lessons/set1/lesson.json").exists());
    assert!(data.join("gone-recording/video/chunk_001.mov").exists());

    let left = clear_leftovers(f.db.pool(), &f.env).await.unwrap();
    assert_eq!(left.bytes, 9_000, "the cut pictures went with the video above");
    assert!(!data.join("snapshots/snap_1.jpg").exists() && data.join("snapshots").exists());
    assert!(!data.join("frames/gone-recording").exists() && !data.join("gone-recording").exists());
    assert!(data.join("models/ggml-base.bin").exists() && data.join("logs/app.log").exists() && data.join("lessons/set1/lesson.json").exists());
    assert!(data.join("frames/m1/state_a.jpg").exists(), "a recording's own screens are not leftovers");
    assert!(f.env.cache_dir.join("WebKit/NetworkCache/blob").exists(), "the web view's cache is untouched");
    assert_eq!(overview(f.db.pool(), &f.env, None, None).await.unwrap().leftover_bytes, 0);
}

#[tokio::test]
async fn removing_screens_keeps_the_iphones_photos_the_transcript_and_other_recordings() {
    let f = fx().await;
    let mine = screen(&f, "m1", "state_mine", 1440, 900, None).await;
    let mine2 = screen(&f, "m1", "state_mine2", 1440, 900, None).await;
    let photo = screen(&f, "m1", "sync_photo", 1440, 900, Some("sync")).await;
    let other = screen(&f, "m2", "state_other", 1440, 900, None).await;
    f.db.add_transcript_at("m1", "we decided to ship on Friday", None, true, 0.9, chrono::Utc::now()).await.unwrap();
    std::fs::create_dir_all(f.env.app_data_dir.join("m1/video")).unwrap();
    std::fs::write(f.env.app_data_dir.join("m1/video/chunk_001.mov"), vec![0u8; 9_000]).unwrap();

    let freed = remove_screens(f.db.pool(), &f.env, "m1").await.unwrap();
    assert_eq!(freed.items, 2);
    assert!(freed.bytes > 9_000);
    assert!(!mine.exists() && !mine2.exists(), "this Mac's screens are gone");
    assert!(photo.exists(), "a photo from the iPhone stays");
    assert!(other.exists());
    assert!(!f.env.app_data_dir.join("m1").exists(), "and so is the video");
    assert_eq!((rows(&f, "m1").await, rows(&f, "m2").await), (1, 1));
    let lines: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transcripts WHERE meeting_id = 'm1'").fetch_one(f.db.pool()).await.unwrap();
    assert_eq!(lines, 1, "the transcript stays");
    // Nothing left to remove is not an error
    assert_eq!(remove_screens(f.db.pool(), &f.env, "m1").await.unwrap(), Freed::default());
    assert!(remove_screens(f.db.pool(), &f.env, "../frames").await.is_err());
}

#[tokio::test]
async fn keeping_screens_for_a_time_removes_only_older_recordings_and_never_by_default() {
    let f = fx().await;
    let old = screen(&f, "m1", "state_old", 1440, 900, None).await;
    let new = screen(&f, "m2", "state_new", 1440, 900, None).await;
    let long_ago = (chrono::Utc::now() - chrono::Duration::days(120)).to_rfc3339();
    sqlx::query("UPDATE meetings SET started_at = ? WHERE id = 'm1'").bind(&long_ago).execute(f.db.pool()).await.unwrap();
    let now = chrono::Utc::now();

    assert_eq!(apply_retention(f.db.pool(), &f.env, 0, now).await.unwrap(), Freed::default());
    assert!(old.exists() && new.exists(), "forever is the default");
    // Nothing is removed while something is being recorded
    let mut busy = RedactionEnv { recording_meetings: vec!["m2".into()], ..RedactionEnv::for_app() };
    busy.app_data_dir = f.env.app_data_dir.clone();
    busy.cache_dir = f.env.cache_dir.clone();
    assert_eq!(apply_retention(f.db.pool(), &busy, 90, now).await.unwrap(), Freed::default());
    assert!(old.exists());

    let freed = apply_retention(f.db.pool(), &f.env, 90, now).await.unwrap();
    assert_eq!(freed.items, 1);
    assert!(!old.exists() && new.exists());
    assert_eq!(apply_retention(f.db.pool(), &f.env, 90, now).await.unwrap(), Freed::default(), "nothing twice");
}

/// The real thing on a COPY of a library: `NF_SPACE_COPY=<folder holding a
/// copy of the data folder> cargo test --lib space::tests::a_copy -- --ignored --nocapture`.
/// Prints sizes and counts only. Never point it at the live data folder.
#[tokio::test]
#[ignore]
async fn a_copy_of_a_real_library_is_measured_and_made_smaller() {
    let Ok(copy) = std::env::var("NF_SPACE_COPY") else { return };
    let data = PathBuf::from(copy);
    assert!(!data.starts_with(crate::paths::app_data_dir()), "a copy, never the live library");
    let db = DatabaseManager::new(&data.join("nofriction_meetings.db")).await.unwrap();
    let env = RedactionEnv { app_data_dir: data.clone(), cache_dir: data.join("no-cache"), video_enabled: false, recording_meetings: Vec::new() };
    let before = overview(db.pool(), &env, None, None).await.unwrap();
    println!(
        "before: total {} · screens {} in {} pictures · video {} · models {} · library {} · leftover {} · other {}",
        format_bytes(before.total_bytes), format_bytes(before.screens_bytes), before.screens_count, format_bytes(before.video_bytes),
        format_bytes(before.models_bytes), format_bytes(before.database_bytes), format_bytes(before.leftover_bytes), format_bytes(before.other_bytes)
    );
    println!("estimate: {} pictures over their limit, saves about {}", before.shrink.pictures, format_bytes(before.shrink.saves_bytes));
    let started = std::time::Instant::now();
    let freed = make_smaller(db.pool(), &env, None, &|_, _, _| {}).await.unwrap();
    println!("made smaller: {} pictures, {} freed, in {:.1}s", freed.items, format_bytes(freed.bytes), started.elapsed().as_secs_f32());
    let video = remove_video(db.pool(), &env, None).await.unwrap();
    println!("screen video removed: {}", format_bytes(video.bytes));
    let after = overview(db.pool(), &env, None, None).await.unwrap();
    println!("after: total {} · screens {} · video {}", format_bytes(after.total_bytes), format_bytes(after.screens_bytes), format_bytes(after.video_bytes));
    assert_eq!(after.screens_count, before.screens_count, "no picture was removed");
    assert_eq!(after.shrink, ShrinkEstimate::default());
    assert_eq!(after.video_bytes, 0);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM screen_states").fetch_one(db.pool()).await.unwrap();
    println!("screen rows still in the library: {}", rows);
}
