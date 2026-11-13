//! The retired background screenshot analyzer.
//!
//! It ran on a timer and sent queued screenshots to the user's AI. It was
//! retired on 2026-11-13: `start` does nothing and the loop is gone. What is
//! left is the status the old settings commands report.

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::database::DatabaseManager;
use crate::settings::SettingsManager;

/// VLM Scheduler status
#[derive(Debug, Clone, serde::Serialize)]
pub struct VLMSchedulerStatus {
    pub running: bool,
    pub enabled: bool,
    pub interval_secs: u32,
    pub frames_processed: u64,
    pub last_run: Option<String>,
    pub next_run: Option<String>,
    pub pending_frames: i64,
}

use crate::prompt_manager::PromptManager;

/// VLM Scheduler - manages background processing
pub struct VLMScheduler {
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    interval_secs: Arc<RwLock<u32>>,
    frames_processed: Arc<AtomicU64>,
    last_run: Arc<RwLock<Option<DateTime<Utc>>>>,
    database: Arc<DatabaseManager>,
    #[allow(dead_code)]
    settings: Arc<SettingsManager>,
    #[allow(dead_code)]
    prompt_manager: Arc<PromptManager>,
}

impl VLMScheduler {
    pub fn new(
        database: Arc<DatabaseManager>,
        settings: Arc<SettingsManager>,
        prompt_manager: Arc<PromptManager>,
    ) -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            enabled: Arc::new(AtomicBool::new(false)),
            interval_secs: Arc::new(RwLock::new(120)),
            frames_processed: Arc::new(AtomicU64::new(0)),
            last_run: Arc::new(RwLock::new(None)),
            database,
            settings,
            prompt_manager,
        }
    }

    /// Set enabled state
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
        log::info!("VLM Scheduler enabled: {}", enabled);
    }

    /// Set processing interval
    pub fn set_interval(&self, secs: u32) {
        let clamped = secs.clamp(30, 600);
        *self.interval_secs.write() = clamped;
        log::info!("VLM Scheduler interval: {}s", clamped);
    }

    /// Get current status
    pub async fn get_status(&self) -> VLMSchedulerStatus {
        let pending = self.database.count_unsynced_frames().await.unwrap_or(0);
        let interval = *self.interval_secs.read();
        let last = self.last_run.read().clone();

        let next = if self.enabled.load(Ordering::SeqCst) && self.running.load(Ordering::SeqCst) {
            last.map(|l| l + chrono::Duration::seconds(interval as i64))
        } else {
            None
        };

        VLMSchedulerStatus {
            running: self.running.load(Ordering::SeqCst),
            enabled: self.enabled.load(Ordering::SeqCst),
            interval_secs: interval,
            frames_processed: self.frames_processed.load(Ordering::SeqCst),
            last_run: last.map(|l| l.to_rfc3339()),
            next_run: next.map(|n| n.to_rfc3339()),
            pending_frames: pending,
        }
    }

    /// Retired 2026-11-13. This used to start a timer that sent queued
    /// screenshots to the user's AI every couple of minutes, recording or
    /// not, if an old admin switch (`vlm_auto_process`) had been left on.
    /// No screen could turn that switch on any more, but a database from an
    /// old version could still carry it. A screenshot now goes to an AI only
    /// when the user clicks something that says so (docs/AI_PROVIDERS.md),
    /// so this does nothing and there is no loop to start.
    pub fn start(&self) {
        log::warn!("The background screenshot analyzer is retired; nothing was started");
    }

    /// Stop the scheduler loop
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        log::info!("VLM Scheduler stopped");
    }
}
