// noFriction - which side a line was said on.
//
// The Mac hears two things while it records: its microphone (the user, and
// anyone in the room) and its own audio output (the other people on a call,
// or a video). They are mixed into one stream for transcription
// (audio_mixer.rs), so a transcript line doesn't say which it was. This
// module keeps how loud each source was, moment by moment, and answers for
// a line's time span: microphone, or computer audio.
//
// It is the honest part of "who is speaking": the side, not the person.
// Nothing here identifies a voice. Only loudness per source is kept (a
// number per audio buffer, for the last few minutes, in memory); no audio.
//
// The answer is stored on the line (`transcripts.source`) and given to the
// assistant in Ask (chat/live.rs) with the people of the recording, so it
// can say "the other side said…" and never has to guess a name.

use crate::capture_engine::AudioSource;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::VecDeque;

/// `transcripts.source`: picked up by this Mac's microphone
pub const MIC: &str = "mic";
/// `transcripts.source`: played by this Mac (a call's other side, a video)
pub const SYSTEM: &str = "system";

/// How long loudness is remembered. A line is saved a few seconds after it
/// was said; a long utterance on a slow Mac takes longer.
const KEEP_MS: i64 = 4 * 60 * 1000;

/// Computer audio is silence (zeros, or nearly) unless something plays.
const SYSTEM_ACTIVE_RMS: f32 = 0.003;
/// A microphone always hears the room; speech close to it is well above this.
const MIC_ACTIVE_RMS: f32 = 0.012;
/// Share of the line's span a source must be active in to count.
const ACTIVE_SHARE: f32 = 0.3;

#[derive(Clone, Copy)]
struct Bin {
    /// When the buffer ended (wall clock, ms)
    end_ms: i64,
    /// How long it was
    ms: i64,
    rms: f32,
}

#[derive(Default)]
struct Ledger {
    mic: VecDeque<Bin>,
    system: VecDeque<Bin>,
}

static LEDGER: Lazy<Mutex<Ledger>> = Lazy::new(|| Mutex::new(Ledger::default()));

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}

/// One buffer of audio arrived from a source (called by the mixer for every
/// buffer; cheap, and it never blocks on anything but its own short lock).
pub fn note(source: AudioSource, samples: &[f32], rate: u32, channels: u16) {
    let frames = samples.len() as i64 / channels.max(1) as i64;
    let ms = if rate > 0 { frames * 1000 / rate as i64 } else { 0 };
    if ms <= 0 {
        return;
    }
    note_at(source, rms(samples), chrono::Utc::now().timestamp_millis(), ms);
}

fn note_at(source: AudioSource, level: f32, end_ms: i64, ms: i64) {
    let mut ledger = LEDGER.lock();
    let queue = match source {
        AudioSource::Microphone => &mut ledger.mic,
        AudioSource::System => &mut ledger.system,
    };
    queue.push_back(Bin { end_ms, ms, rms: level });
    while queue.front().is_some_and(|b| b.end_ms < end_ms - KEEP_MS) {
        queue.pop_front();
    }
}

/// Share of the span [from, to] in which the source was louder than `floor`,
/// 0 to 1. None when nothing was heard from it in that span at all.
fn active_share(bins: &VecDeque<Bin>, from_ms: i64, to_ms: i64, floor: f32) -> Option<f32> {
    let (mut heard, mut active) = (0i64, 0i64);
    for b in bins.iter().rev() {
        if b.end_ms < from_ms {
            break;
        }
        let start = b.end_ms - b.ms;
        if start > to_ms {
            continue;
        }
        let overlap = b.end_ms.min(to_ms) - start.max(from_ms);
        if overlap <= 0 {
            continue;
        }
        heard += overlap;
        if b.rms >= floor {
            active += overlap;
        }
    }
    (heard > 0).then(|| active as f32 / heard as f32)
}

/// Which side the span belongs to, from how much of it each source was
/// active. A tag is stored only when it is clear:
/// - the computer's audio was playing and the microphone was quiet: the
///   other side (headphones, or the user silent);
/// - the microphone was active and the computer's audio was being captured
///   and was quiet: the user's side.
/// Both active is not told apart (with speakers on, the microphone hears
/// the call; with headphones, the user may be talking over it), and when
/// the computer's audio isn't captured at all a microphone line could be
/// either side: no tag.
fn decide(mic: Option<f32>, system: Option<f32>) -> Option<&'static str> {
    let mic_active = mic.is_some_and(|m| m >= ACTIVE_SHARE);
    match system {
        Some(s) if s >= ACTIVE_SHARE => (!mic_active).then_some(SYSTEM),
        Some(_) if mic_active => Some(MIC),
        _ => None,
    }
}

/// Which side a line said from `start` for `seconds` was on: `MIC`,
/// `SYSTEM`, or None when it can't be told.
pub fn side_of(start: chrono::DateTime<chrono::Utc>, seconds: f64) -> Option<&'static str> {
    let from = start.timestamp_millis();
    // A line of no length still gets a window to be judged by
    let to = from + ((seconds * 1000.0) as i64).max(400);
    let ledger = LEDGER.lock();
    decide(
        active_share(&ledger.mic, from, to, MIC_ACTIVE_RMS),
        active_share(&ledger.system, from, to, SYSTEM_ACTIVE_RMS),
    )
}

/// How the assistant is told about a tagged line.
pub fn label(source: Option<&str>) -> Option<&'static str> {
    match source {
        Some(MIC) => Some("mic"),
        Some(SYSTEM) => Some("computer audio"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bins(levels: &[(i64, f32)]) -> VecDeque<Bin> {
        // (end_ms, rms), each 100 ms long
        levels.iter().map(|(end_ms, rms)| Bin { end_ms: *end_ms, ms: 100, rms: *rms }).collect()
    }

    #[test]
    fn a_span_is_judged_by_how_much_of_it_each_source_was_active() {
        // The microphone: quiet room, then speech from 1.0 s to 2.0 s
        let mic: Vec<(i64, f32)> = (1..=30).map(|n| (n * 100, if (11..=20).contains(&n) { 0.05 } else { 0.004 })).collect();
        let mic = bins(&mic);
        assert_eq!(active_share(&mic, 1_000, 2_000, MIC_ACTIVE_RMS), Some(1.0));
        assert_eq!(active_share(&mic, 2_000, 3_000, MIC_ACTIVE_RMS), Some(0.0));
        let half = active_share(&mic, 1_500, 2_500, MIC_ACTIVE_RMS).unwrap();
        assert!((half - 0.5).abs() < 0.01, "{half}");
        // Nothing was heard in a span outside what is remembered
        assert_eq!(active_share(&mic, 10_000, 11_000, MIC_ACTIVE_RMS), None);
        assert_eq!(active_share(&VecDeque::new(), 0, 1_000, MIC_ACTIVE_RMS), None);
    }

    #[test]
    fn a_side_is_stored_only_when_it_is_clear() {
        // The microphone, with the computer's audio captured and quiet: the user's side
        assert_eq!(decide(Some(0.9), Some(0.0)), Some(MIC));
        // The computer's audio isn't captured: a call on the speakers would
        // look the same, so no tag
        assert_eq!(decide(Some(0.9), None), None);
        // The computer's audio with a quiet microphone: the other side
        assert_eq!(decide(Some(0.0), Some(0.6)), Some(SYSTEM));
        assert_eq!(decide(None, Some(0.6)), Some(SYSTEM));
        // Both active (speakers on, or talking over the call): not told apart
        assert_eq!(decide(Some(0.8), Some(0.9)), None);
        // Neither clearly active: no guess
        assert_eq!(decide(Some(0.1), Some(0.1)), None);
        assert_eq!(decide(None, None), None);
    }

    #[test]
    fn loudness_is_kept_for_a_few_minutes_and_no_audio_is() {
        assert_eq!(rms(&[]), 0.0);
        assert!((rms(&[0.5, -0.5, 0.5, -0.5]) - 0.5).abs() < 1e-6);
        // Old bins are dropped as new ones arrive
        let base = 2_000_000_000_000i64;
        note_at(AudioSource::System, 0.2, base, 100);
        note_at(AudioSource::System, 0.2, base + KEEP_MS + 1_000, 100);
        let kept = LEDGER.lock().system.iter().filter(|b| b.end_ms >= base).count();
        assert_eq!(kept, 1, "the first bin aged out");
        assert_eq!(label(Some(MIC)), Some("mic"));
        assert_eq!(label(Some(SYSTEM)), Some("computer audio"));
        assert_eq!(label(Some("else")), None);
        assert_eq!(label(None), None);
    }
}
