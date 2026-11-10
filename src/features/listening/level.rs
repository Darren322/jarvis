//! Mic loudness from the recorder's audio tap.
//!
//! `rms_level` is the pure scale; `run_level_meter` turns a stream of tapped
//! chunks into a steady `LEVEL_INTERVAL` stream of values for an interface to
//! read. Nothing here knows about the orb or the terminal.

use tokio::{
    sync::{broadcast, watch},
    time::{Instant, MissedTickBehavior},
};

use super::AudioChunk;
use std::time::Duration;

/// Time between published levels: about 30 per second (30.3 Hz).
pub(crate) const LEVEL_INTERVAL: Duration = Duration::from_millis(33);
/// How long the last chunk's level is held while no new chunk arrives. It must
/// exceed the gap between chunks (a 4096-byte read is 128 ms of audio); after it
/// passes the level falls to 0.0, so a finished recording does not leave the
/// meter stuck at its last value.
pub(crate) const LEVEL_HOLD: Duration = Duration::from_millis(250);
/// The magnitude of `i16::MIN`, the largest a sample can be: full scale.
const FULL_SCALE: f64 = 32_768.0;

/// Loudness of S16_LE mono PCM as the root mean square of its samples, from
/// 0.0 to 1.0.
///
/// The scale is linear, not decibels: 0.0 is digital silence and 1.0 is a signal
/// whose every sample has the magnitude of `i16::MIN` (a full-scale square wave
/// or a clipped, held-down rail). A full-scale sine is about 0.71, and speech
/// into a USB mic is typically 0.01 to 0.1, so a display that wants speech to
/// look lively applies its own gain and clamp. An empty chunk is 0.0, and an odd
/// trailing byte (half a sample) is ignored.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired by visual-jarvis ticket 07")
)]
pub(crate) fn rms_level(pcm: &[u8]) -> f32 {
    // The remainder is the odd trailing byte, dropped here.
    let (samples, _odd_trailing_byte) = pcm.as_chunks::<2>();
    let count = samples.len();
    if count == 0 {
        return 0.0;
    }
    // Each square is at most 2^30, so the sum fits u64 for any slice that fits memory.
    let sum_of_squares: u64 = samples
        .iter()
        .map(|bytes| u64::from(i16::from_le_bytes(*bytes).unsigned_abs()).pow(2))
        .sum();
    // `as f64` can only lose low bits of a large count or sum, far below the
    // precision a display needs.
    let mean_square = sum_of_squares as f64 / count as f64;
    // The ratio is in 0.0..=1.0 by construction; the clamp makes that explicit
    // before the narrowing cast, which then cannot overflow.
    (mean_square.sqrt() / FULL_SCALE).clamp(0.0, 1.0) as f32
}

/// Publishes the loudness of the tapped audio on `levels` every `LEVEL_INTERVAL`
/// until the tap closes (its `Recorder` is dropped) or nobody reads `levels` any
/// more.
///
/// Each tick carries the level of the newest chunk for `LEVEL_HOLD`, then 0.0, so
/// the meter can stay alive across recordings. A lagged tap (this task was
/// starved) skips the missed chunks. Drop-safe at any `.await`; the caller owns
/// the task (`tokio::spawn` or `select!`).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "wired by visual-jarvis ticket 07")
)]
pub(crate) async fn run_level_meter(
    mut chunks: broadcast::Receiver<AudioChunk>,
    levels: watch::Sender<f32>,
) {
    let mut ticks = tokio::time::interval(LEVEL_INTERVAL);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut newest: Option<(Instant, f32)> = None;
    loop {
        tokio::select! {
            // A chunk is processed before the tick that should report it.
            biased;
            chunk = chunks.recv() => match chunk {
                Ok(chunk) => newest = Some((Instant::now(), rms_level(&chunk))),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => {
                    levels.send_replace(0.0);
                    return;
                }
            },
            _ = ticks.tick() => {
                let level = match newest {
                    Some((at, level)) if at.elapsed() <= LEVEL_HOLD => level,
                    _ => 0.0,
                };
                if levels.send(level).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/features/listening/level_tests.rs"]
mod tests;
