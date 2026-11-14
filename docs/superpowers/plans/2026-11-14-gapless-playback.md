# Gapless Playback Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consecutive local tracks with the same sample rate and channel count play into the same output stream with no gap, and AAC encoder delay and padding are trimmed (#92).

**Architecture:** When the current decoder ends, the engine opens the next queue row at once and keeps pumping into the same ring buffer, remembering the sample count where the new track starts (`PendingTrack`). When the speakers reach that sample, the engine switches the track metadata and resets the position. Pure logic lives in two new modules (`player/gapless.rs`, `audio/trim.rs`) so it is unit-tested without a sound card.

**Tech Stack:** Rust 2024, symphonia 0.5.5, cpal, rtrb, crossbeam-channel.

Spec: `docs/superpowers/specs/2026-11-14-gapless-playback-design.md`.

Branch: `feat/gapless-92`. **Every task starts with `git branch --show-current`; it must print `feat/gapless-92`. Stop if it does not.**

Commit trailer for every commit:

```
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

Checks before each commit: `cargo fmt --all`, `cargo clippy -p znicz-core --all-targets -- -D warnings`, `cargo test -p znicz-core`.

---

### Task 1: AAC trim helpers (`audio/trim.rs`)

**Files:**
- Create: `znicz-core/src/audio/trim.rs`
- Modify: `znicz-core/src/audio/mod.rs` (add `pub mod trim;` in alphabetical order, after `pub mod source;`)

- [ ] **Step 1: Write the module with its tests, functions as `todo!()`**

```rust
//! Encoder delay and padding for AAC in MP4 (the iTunes `iTunSMPB` tag).
//!
//! AAC encoders add priming frames at the start and padding at the end. The
//! tag says how many, so the player can drop them and the tracks of a
//! continuous album join without a gap. symphonia trims MP3 and Vorbis itself;
//! AAC is left to us.

use std::ops::Range;

use symphonia::core::units::TimeBase;

/// How much of a decoded AAC stream is real audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderGap {
    /// Priming frames at the start.
    pub delay: u64,
    /// Real audio frames after the priming.
    pub total: u64,
}

/// Parse an `iTunSMPB` value: space-separated hex fields
/// `reserved delay padding total …`.
pub fn parse_itunsmpb(value: &str) -> Option<EncoderGap> {
    todo!()
}

/// The frames of one decoded packet to keep. `first_frame` is the stream
/// frame index of the packet's first frame, priming included.
pub fn trim_window(first_frame: u64, frames: usize, gap: EncoderGap) -> Range<usize> {
    todo!()
}

/// Drop the priming and padding frames of one interleaved packet.
pub fn trim_packet(
    samples: Vec<f32>,
    channels: usize,
    first_frame: u64,
    gap: EncoderGap,
) -> Vec<f32> {
    todo!()
}

/// A packet timestamp in frames. Without a time base the timestamp already
/// counts frames.
pub fn ts_to_frames(ts: u64, time_base: Option<TimeBase>, sample_rate: u32) -> u64 {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: EncoderGap = EncoderGap {
        delay: 2112,
        total: 10_000,
    };

    #[test]
    fn parses_an_itunes_value() {
        let value = " 00000000 00000840 000001CA 00000000001A8DF6 00000000 00000000";
        assert_eq!(
            parse_itunsmpb(value),
            Some(EncoderGap {
                delay: 0x840,
                total: 0x1A8DF6,
            })
        );
    }

    #[test]
    fn rejects_malformed_values() {
        assert_eq!(parse_itunsmpb(""), None);
        assert_eq!(parse_itunsmpb(" 00000000 00000840"), None);
        assert_eq!(parse_itunsmpb(" 00000000 zz 000001CA 1A8DF6"), None);
        // A zero total says nothing about the real length.
        assert_eq!(parse_itunsmpb(" 00000000 00000840 000001CA 0"), None);
    }

    #[test]
    fn drops_the_priming_frames() {
        assert_eq!(trim_window(0, 1024, GAP), 1024..1024);
        assert_eq!(trim_window(2048, 1024, GAP), 64..1024);
    }

    #[test]
    fn keeps_packets_in_the_middle() {
        assert_eq!(trim_window(4096, 1024, GAP), 0..1024);
    }

    #[test]
    fn drops_the_padding_frames() {
        // Real audio ends at frame 2112 + 10000 = 12112.
        assert_eq!(trim_window(11_264, 1024, GAP), 0..848);
        assert_eq!(trim_window(12_288, 1024, GAP), 0..0);
    }

    #[test]
    fn trims_interleaved_samples_by_frame() {
        let gap = EncoderGap { delay: 1, total: 2 };
        let samples = vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5];
        assert_eq!(trim_packet(samples, 2, 0, gap), vec![2.0, 2.5, 3.0, 3.5]);
    }

    #[test]
    fn leaves_untouched_packets_alone() {
        let samples = vec![0.25; 2048];
        assert_eq!(trim_packet(samples.clone(), 2, 4096, GAP), samples);
    }

    #[test]
    fn converts_timestamps_to_frames() {
        assert_eq!(ts_to_frames(44_100, Some(TimeBase::new(1, 44_100)), 44_100), 44_100);
        assert_eq!(ts_to_frames(2, Some(TimeBase::new(1, 2)), 48_000), 48_000);
        assert_eq!(ts_to_frames(1234, None, 44_100), 1234);
    }
}
```

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p znicz-core audio::trim`
Expected: the tests FAIL, panicking at `not yet implemented`.

- [ ] **Step 3: Implement**

```rust
pub fn parse_itunsmpb(value: &str) -> Option<EncoderGap> {
    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }
    let delay = u64::from_str_radix(fields[1], 16).ok()?;
    let total = u64::from_str_radix(fields[3], 16).ok()?;
    if total == 0 {
        return None;
    }
    Some(EncoderGap { delay, total })
}

pub fn trim_window(first_frame: u64, frames: usize, gap: EncoderGap) -> Range<usize> {
    let frames = frames as u64;
    let audio_end = gap.delay + gap.total;
    let start = gap.delay.saturating_sub(first_frame).min(frames);
    let end = audio_end.saturating_sub(first_frame).min(frames).max(start);
    start as usize..end as usize
}

pub fn trim_packet(
    samples: Vec<f32>,
    channels: usize,
    first_frame: u64,
    gap: EncoderGap,
) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = samples.len() / channels;
    let keep = trim_window(first_frame, frames, gap);
    if keep == (0..frames) {
        return samples;
    }
    samples[keep.start * channels..keep.end * channels].to_vec()
}

pub fn ts_to_frames(ts: u64, time_base: Option<TimeBase>, sample_rate: u32) -> u64 {
    match time_base {
        Some(base) => {
            let frames = ts as u128 * base.numer as u128 * sample_rate as u128 / base.denom as u128;
            frames as u64
        }
        None => ts,
    }
}
```

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo test -p znicz-core audio::trim`
Expected: 8 passed.

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/audio/trim.rs znicz-core/src/audio/mod.rs
git commit -m "feat(core): iTunSMPB parsing and AAC trim helpers (#92)"
```

---

### Task 2: Trim AAC in the decoder (`audio/source.rs`)

**Files:**
- Modify: `znicz-core/src/audio/source.rs` (imports at the top, `AudioDecoder` struct ~line 303, `AudioDecoder::open` ~line 318, `decode_next` ~line 428)

No new automated test: there is no AAC fixture in the repo and the arithmetic is covered by Task 1. The check is that every existing test still passes.

- [ ] **Step 1: Imports**

Change the codecs import and add:

```rust
use symphonia::core::codecs::{
    CODEC_TYPE_AAC, CODEC_TYPE_NULL, CodecParameters, CodecType, DecoderOptions,
};
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::units::{Time, TimeBase};

use crate::audio::trim::{EncoderGap, parse_itunsmpb, trim_packet, ts_to_frames};
```

(Keep the other imports. `Time` was already imported; merge it into the `units` line.)

- [ ] **Step 2: Helper above `pub struct AudioDecoder`**

```rust
/// Encoder delay and padding for AAC, from the `iTunSMPB` tag. Other codecs
/// either have none or are trimmed by symphonia (MP3, Vorbis).
fn encoder_gap(format: &mut dyn FormatReader, codec: CodecType) -> Option<EncoderGap> {
    if codec != CODEC_TYPE_AAC {
        return None;
    }
    let metadata = format.metadata();
    let revision = metadata.current()?;
    let tag = revision
        .tags()
        .iter()
        .find(|tag| tag.key.ends_with("iTunSMPB"))?;
    parse_itunsmpb(&tag.value.to_string())
}
```

- [ ] **Step 3: Struct fields**

Add to `AudioDecoder` after `channels: u16,`:

```rust
    /// AAC priming and padding to drop, when the file says.
    gap: Option<EncoderGap>,
    /// Converts packet timestamps to frames for the trim.
    time_base: Option<TimeBase>,
```

- [ ] **Step 4: Fill them in `open`**

After `let codec_params = track.codec_params.clone();` the `track` borrow of `probed.format` must end before the format is used mutably. Replace the tail of `open` so it reads:

```rust
        let track_id = track.id;
        let codec_params = track.codec_params.clone();
        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params.channels.map(|c| c.count() as u16).unwrap_or(2);

        let mut format = probed.format;
        let gap = encoder_gap(format.as_mut(), codec_params.codec);

        let decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|e| ZniczError::Decode(e.to_string()))?;

        let track_info = track_info_from_params(&codec_params, source);
        let is_stream = source.url().is_some();

        Ok((
            Self {
                format,
                decoder,
                track_id,
                sample_rate,
                channels,
                gap,
                time_base: codec_params.time_base,
                is_stream,
                coded_bytes: 0,
                pcm_frames: 0,
                icy_title,
                icy_url,
            },
            track_info,
        ))
```

- [ ] **Step 5: Trim in `decode_next`**

Right after `let coded = packet.buf().len() as u64;` add `let packet_ts = packet.ts();`. In the `Ok(decoded)` arm, replace `let samples = sample_buf.samples().to_vec();` with:

```rust
                    let mut samples = sample_buf.samples().to_vec();
                    if let Some(gap) = self.gap {
                        let first_frame = ts_to_frames(packet_ts, self.time_base, self.sample_rate);
                        samples = trim_packet(samples, self.channels as usize, first_frame, gap);
                    }
```

The feeder already skips empty packets, so a fully trimmed packet needs no special case.

- [ ] **Step 6: Verify**

Run: `cargo test -p znicz-core` and `cargo clippy -p znicz-core --all-targets -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 7: Commit**

```bash
git add znicz-core/src/audio/source.rs
git commit -m "feat(core): trim AAC encoder delay and padding (#92)"
```

---

### Task 3: Pending-track bookkeeping (`player/gapless.rs`)

**Files:**
- Create: `znicz-core/src/player/gapless.rs`
- Modify: `znicz-core/src/player/mod.rs` (add `pub mod gapless;` after `pub mod engine;`)

- [ ] **Step 1: Write the module with tests, bodies as `todo!()`**

```rust
//! Gapless playback: tracks already decoded into the output buffer but not
//! yet heard.
//!
//! When a local track finishes decoding, the engine opens the next one and
//! keeps feeding the same stream. The buffer then holds the end of one track
//! and the start of the next. A `PendingTrack` remembers where the next one
//! begins, so the shown track switches when the speakers get there.

use std::collections::VecDeque;

use crate::player::state::{QueueItem, RepeatMode, TrackInfo};

/// A chained track waiting for its first sample to be played.
#[derive(Debug, Clone)]
pub struct PendingTrack {
    /// Queue row it was picked from.
    pub index: usize,
    pub item: QueueItem,
    pub info: TrackInfo,
    /// Feeder sample count (all channels) at the track's first sample,
    /// counted from the start of the track before it.
    pub starts_at: u64,
}

/// Chained tracks in play order.
#[derive(Debug, Default)]
pub struct PendingTracks {
    tracks: VecDeque<PendingTrack>,
}

impl PendingTracks {
    pub fn push(&mut self, track: PendingTrack) {
        todo!()
    }

    pub fn clear(&mut self) {
        todo!()
    }

    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// Queue row of the last chained track: the next pick follows it.
    pub fn last_index(&self) -> Option<usize> {
        todo!()
    }

    /// The first pending track once `played` has reached its first sample.
    /// The remaining tracks are shifted to count from the returned one, as the
    /// caller does with the feeder.
    pub fn take_started(&mut self, played: u64) -> Option<PendingTrack> {
        todo!()
    }
}

/// Whether the track after the one that just finished decoding may join it
/// in the same stream. Repeat-one and radio keep the old stop-and-reopen.
pub fn may_chain(repeat: RepeatMode, current_is_stream: bool) -> bool {
    todo!()
}

/// Where a chained track sits in the queue now. The queue may have been
/// edited since it was picked.
pub fn resolve_position(queue: &[QueueItem], index: usize, item: &QueueItem) -> Option<usize> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> QueueItem {
        QueueItem::file(format!("/music/{name}.flac"))
    }

    fn pending(index: usize, name: &str, starts_at: u64) -> PendingTrack {
        PendingTrack {
            index,
            item: file(name),
            info: TrackInfo {
                path: None,
                url: None,
                icy_stream_url: None,
                title: name.to_string(),
                codec: "FLAC".into(),
                sample_rate: 44_100,
                channels: 2,
                bits_per_sample: Some(16),
                bitrate_kbps: None,
                duration: None,
                tags: Default::default(),
            },
            starts_at,
        }
    }

    /// The parts of a taken track the tests compare.
    fn key(track: Option<PendingTrack>) -> Option<(usize, u64)> {
        track.map(|track| (track.index, track.starts_at))
    }

    #[test]
    fn waits_until_the_first_sample_is_played() {
        let mut tracks = PendingTracks::default();
        tracks.push(pending(1, "b", 1000));

        assert_eq!(key(tracks.take_started(999)), None);
        assert_eq!(key(tracks.take_started(1000)), Some((1, 1000)));
        assert!(tracks.is_empty());
    }

    #[test]
    fn later_tracks_count_from_the_one_taken() {
        let mut tracks = PendingTracks::default();
        tracks.push(pending(1, "b", 1000));
        tracks.push(pending(2, "c", 1500));

        assert_eq!(key(tracks.take_started(1200)), Some((1, 1000)));
        // Played is now counted from b: 200. c starts 500 into b.
        assert_eq!(key(tracks.take_started(200)), None);
        assert_eq!(key(tracks.take_started(500)), Some((2, 500)));
    }

    #[test]
    fn last_index_follows_the_latest_chain() {
        let mut tracks = PendingTracks::default();
        assert_eq!(tracks.last_index(), None);
        tracks.push(pending(3, "d", 10));
        tracks.push(pending(4, "e", 20));
        assert_eq!(tracks.last_index(), Some(4));
        tracks.clear();
        assert!(tracks.is_empty());
        assert_eq!(tracks.last_index(), None);
    }

    #[test]
    fn repeat_one_and_radio_do_not_chain() {
        assert!(may_chain(RepeatMode::Off, false));
        assert!(may_chain(RepeatMode::All, false));
        assert!(!may_chain(RepeatMode::One, false));
        assert!(!may_chain(RepeatMode::Off, true));
    }

    #[test]
    fn an_unchanged_row_keeps_its_index() {
        let queue = vec![file("a"), file("b")];
        assert_eq!(resolve_position(&queue, 1, &file("b")), Some(1));
    }

    #[test]
    fn a_moved_row_is_found_again() {
        let queue = vec![file("b"), file("a")];
        assert_eq!(resolve_position(&queue, 1, &file("b")), Some(0));
    }

    #[test]
    fn a_removed_row_has_no_position() {
        let queue = vec![file("a")];
        assert_eq!(resolve_position(&queue, 1, &file("b")), None);
    }
}
```

`RepeatMode` is `Off | One | All`. `TrackTags` (in `metadata.rs`) derives `Default`.

- [ ] **Step 2: Run the tests and check that they fail**

Run: `cargo test -p znicz-core player::gapless`
Expected: FAIL with `not yet implemented`.

- [ ] **Step 3: Implement**

```rust
impl PendingTracks {
    pub fn push(&mut self, track: PendingTrack) {
        self.tracks.push_back(track);
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn last_index(&self) -> Option<usize> {
        self.tracks.back().map(|track| track.index)
    }

    pub fn take_started(&mut self, played: u64) -> Option<PendingTrack> {
        if self.tracks.front()?.starts_at > played {
            return None;
        }
        let started = self.tracks.pop_front()?;
        for track in &mut self.tracks {
            track.starts_at -= started.starts_at;
        }
        Some(started)
    }
}

pub fn may_chain(repeat: RepeatMode, current_is_stream: bool) -> bool {
    repeat != RepeatMode::One && !current_is_stream
}

pub fn resolve_position(queue: &[QueueItem], index: usize, item: &QueueItem) -> Option<usize> {
    if queue.get(index) == Some(item) {
        return Some(index);
    }
    queue.iter().position(|row| row == item)
}
```

- [ ] **Step 4: Run the tests and check that they pass**

Run: `cargo test -p znicz-core player::gapless`
Expected: 7 passed. (Until Task 5 uses the module, clippy may warn about dead code. If it does, add `#![allow(dead_code)]` at the top of `gapless.rs` now; Task 5 removes it.)

- [ ] **Step 5: Commit**

```bash
git add znicz-core/src/player/gapless.rs znicz-core/src/player/mod.rs
git commit -m "feat(core): pending-track bookkeeping for gapless playback (#92)"
```

---

### Task 4: Feeder guard test across decoders

The engine relies on this: after `PumpOutcome::Finished`, the feeder holds no leftover. So a new decoder's samples follow the last sample of the old one, and `pushed_samples` keeps counting across the joint. The test documents this and guards it. It passes against today's code, which is expected.

**Files:**
- Modify: `znicz-core/src/audio/feeder.rs` (tests module, after `reports_decoder_failure`)

- [ ] **Step 1: Add the test**

```rust
    #[test]
    fn a_new_decoder_continues_right_after_the_last_sample() {
        let first = packets(5, 1000);
        let first_len: f32 = 5000.0;
        let second: Vec<Vec<f32>> = packets(8, 1000)
            .into_iter()
            .map(|packet| packet.into_iter().map(|v| v + first_len).collect())
            .collect();
        let expected: Vec<f32> = first.iter().chain(second.iter()).flatten().copied().collect();

        let mut sink = FakeSink::new(4096);
        let mut feeder = Feeder::new();

        for source in [first, second] {
            let mut queue = source.into_iter();
            loop {
                let mut decode = || match queue.next() {
                    Some(samples) => DecodeStep::Samples(samples),
                    None => DecodeStep::End,
                };
                let outcome = feeder.pump(&mut sink, 256, 4, &mut decode);
                sink.play(1500);
                if outcome == PumpOutcome::Finished {
                    assert_eq!(feeder.pending_samples(), 0, "leftover at the joint");
                    break;
                }
            }
        }

        sink.finish();
        assert_eq!(sink.played, expected, "samples lost or reordered at the joint");
        assert_eq!(feeder.pushed_samples(), expected.len() as u64);
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p znicz-core a_new_decoder_continues`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add znicz-core/src/audio/feeder.rs
git commit -m "test(core): feeder keeps samples contiguous across decoders (#92)"
```

---

### Task 5: Chain tracks in the engine (`player/engine.rs`)

**Files:**
- Modify: `znicz-core/src/player/engine.rs`
- Modify: `znicz-core/src/player/gapless.rs` (remove `#![allow(dead_code)]` if Task 3 added it)

The engine needs a real sound card to play, so this task has no new unit test. The pure parts are covered by Task 3, and Task 6 adds a hardware test. Keep the existing engine tests green.

- [ ] **Step 1: Imports and field**

Add the import:

```rust
use crate::player::gapless::{PendingTrack, PendingTracks, may_chain, resolve_position};
```

Add a field to `PlayerEngine` after `converter`:

```rust
    /// Chained tracks already in the buffer but not yet heard (gapless).
    pending: PendingTracks,
```

Initialise it in `PlayerEngine::new` with `pending: PendingTracks::default(),`.

- [ ] **Step 2: Split `pick_next` so chaining can pick after a pending track**

Replace `pick_next` with:

```rust
    /// Which queue entry to play after the playing one, or `None` to stop.
    fn pick_next(&mut self, auto: bool) -> Option<usize> {
        let current = self.state.read().unwrap().queue_position;
        self.pick_next_after(current, auto)
    }

    /// Which queue entry follows `current`, or `None` to stop.
    ///
    /// `auto` marks the end of a track, where "repeat one" replays it. Pressing
    /// next always moves on instead.
    fn pick_next_after(&mut self, current: usize, auto: bool) -> Option<usize> {
        let (len, repeat, shuffle) = {
            let state = self.state.read().unwrap();
            (state.queue.len(), state.repeat, state.shuffle)
        };

        if len == 0 {
            return None;
        }
        if auto && repeat == RepeatMode::One {
            return Some(current);
        }
        if shuffle {
            return Some(self.random_index(len, current));
        }
        if current + 1 < len {
            return Some(current + 1);
        }
        if repeat == RepeatMode::All {
            return Some(0);
        }
        None
    }
```

- [ ] **Step 3: `chain_next`**

Add after `pick_next_after`:

```rust
    /// Open the track after the one that just finished decoding, if it can
    /// follow it in the same stream: a local file with the same sample rate
    /// and channel count. `None` means drain and reopen as before; a file that
    /// fails to open is reported by that path, not here.
    fn chain_next(&mut self, finished: &AudioDecoder) -> Option<AudioDecoder> {
        let (repeat, playing) = {
            let state = self.state.read().unwrap();
            (state.repeat, state.queue_position)
        };
        if !may_chain(repeat, finished.is_stream()) {
            return None;
        }

        let after = self.pending.last_index().unwrap_or(playing);
        let index = self.pick_next_after(after, true)?;
        let item = self.state.read().unwrap().queue.get(index).cloned()?;
        let QueueItem::File { path } = &item else {
            return None;
        };

        let (decoder, info) = match AudioDecoder::open(&LocalFileSource::new(path.clone())) {
            Ok(opened) => opened,
            Err(e) => {
                tracing::debug!(error = %e, "next track did not open, no gapless join");
                return None;
            }
        };
        let same_format = decoder.sample_rate() == finished.sample_rate()
            && decoder.channels() == finished.channels();
        if !same_format {
            return None;
        }

        self.pending.push(PendingTrack {
            index,
            item,
            info,
            starts_at: self.feeder.pushed_samples(),
        });
        Some(decoder)
    }
```

- [ ] **Step 4: Use it when a decoder finishes**

In `pump_decode`, replace the `PumpOutcome::Finished` arm with:

```rust
            PumpOutcome::Finished => {
                self.publish_stream_bitrate(&decoder);
                self.publish_stream_title(&decoder);
                match self.chain_next(&decoder) {
                    Some(next) => {
                        // Same stream, buffer and converter: the next track's
                        // first sample follows this one's last.
                        self.decoder = Some(next);
                        self.converter = converter;
                    }
                    None => {
                        // The buffer still holds audio. Let it play out,
                        // otherwise the last seconds of the track are cut off.
                        self.converter = None;
                        self.draining_since = Some(Instant::now());
                    }
                }
            }
```

- [ ] **Step 5: Switch at the boundary**

Add these methods (next to `tick_position`):

```rust
    /// Samples (all channels) the speakers have played of the current track.
    fn played_samples(&self) -> u64 {
        self.feeder
            .pushed_samples()
            .saturating_sub(self.output.queued_samples() as u64)
    }

    /// Show each chained track whose first sample has been played.
    fn advance_pending(&mut self) {
        while let Some(track) = self.pending.take_started(self.played_samples()) {
            self.start_chained(track);
        }
    }

    /// Show every chained track now: the buffer holding them is gone or done.
    fn start_all_pending(&mut self) {
        while let Some(track) = self.pending.take_started(u64::MAX) {
            self.start_chained(track);
        }
    }

    fn start_chained(&mut self, track: PendingTrack) {
        // Count the position from the new track's first sample.
        let pushed = self.feeder.pushed_samples();
        self.feeder
            .set_pushed_samples(pushed.saturating_sub(track.starts_at));

        self.event_tx.send(PlayerEvent::TrackEnded).ok();
        {
            let mut state = self.state.write().unwrap();
            if let Some(position) = resolve_position(&state.queue, track.index, &track.item) {
                state.queue_position = position;
            }
            state.current_track = Some(track.info.clone());
            state.position = Duration::ZERO;
        }
        self.event_tx
            .send(PlayerEvent::TrackStarted(Box::new(track.info)))
            .ok();
        self.emit_state_changed();
    }
```

In `run`, call it between pumping and the drain check:

```rust
            self.pump_decode();
            self.advance_pending();

            if self.drain_complete() {
                self.finish_track();
            }
```

At the top of `finish_track`, before `self.draining_since = None;`, add:

```rust
        // A chain whose last track was tiny may end before its boundary
        // check ran; show those tracks so none is skipped.
        self.start_all_pending();
```

- [ ] **Step 6: Commands**

1. `stop`: add `self.pending.clear();` after `self.draining_since = None;`. Play, Previous, QueuePlayIndex, ReplaceQueue and removing the playing row all go through `stop`, so this covers them.

2. `seek`: after the stream check, before `if let Some(decoder) = &mut self.decoder {`, add:

```rust
        if !self.pending.is_empty() {
            // The decoder already reads the next track; reopen this one.
            self.replay_current()?;
        }
```

and add the method:

```rust
    /// Reopen the playing row from the start, keeping it paused if it was.
    fn replay_current(&mut self) -> Result<()> {
        let (index, paused) = {
            let state = self.state.read().unwrap();
            (state.queue_position, state.status == PlaybackStatus::Paused)
        };
        self.play_queue_index(index)?;
        if paused {
            self.output.set_paused(true);
            self.update_status(PlaybackStatus::Paused);
        }
        Ok(())
    }
```

3. `Command::SetDevice`: after the `open_stream` call inside `if let Some(decoder) = &self.decoder { … }`, the old buffer is gone. Change the arm to:

```rust
            Command::SetDevice(id) => {
                self.config.device_id = Some(id);
                if let Some(decoder) = &self.decoder {
                    let rate = decoder.sample_rate();
                    let ch = decoder.channels();
                    self.output
                        .open_stream(rate, ch, self.config.device_id.as_deref())?;
                    // The reopened stream starts empty: the chained tracks in
                    // the old buffer are gone, so show the decoder's track.
                    self.start_all_pending();
                }
                self.emit_state_changed();
            }
```

- [ ] **Step 7: Verify**

Run: `cargo fmt --all && cargo clippy -p znicz-core --all-targets -- -D warnings && cargo test -p znicz-core`
Expected: all pass. Then build the workspace to check that nothing else broke: `cargo build --workspace`.

- [ ] **Step 8: Commit**

```bash
git add znicz-core/src/player/engine.rs znicz-core/src/player/gapless.rs
git commit -m "feat(core): gapless joins between same-format local tracks (#92)"
```

---

### Task 6: Hardware playback tests

**Files:**
- Modify: `znicz-core/tests/playback.rs` (add after `track_plays_for_its_real_duration`; reuse `skip_hardware_playback` and `write_silent_wav(path, sample_rate, channels, seconds)`)

These run only on a machine with a sound card and are skipped on CI, like the other tests in this file.

- [ ] **Step 1: Add a helper and two tests**

```rust
/// Play `files` as a queue from the first row and collect when each track
/// started, until the last one ends or `limit` passes.
fn play_queue(files: Vec<std::path::PathBuf>, limit: Duration) -> (Vec<Duration>, usize, Duration) {
    let count = files.len();
    let (player, _thread) = spawn_player(AudioConfig::default());
    player
        .send_blocking(Command::ReplaceQueue {
            items: files.into_iter().map(znicz_core::QueueItem::file).collect(),
            position: 0,
        })
        .expect("queue");

    let started = Instant::now();
    player
        .send_blocking(Command::QueuePlayIndex(0))
        .expect("play");

    let mut starts = Vec::new();
    let mut ended = 0;
    while started.elapsed() < limit && ended < count {
        for event in player.drain_events() {
            match event {
                PlayerEvent::TrackStarted(_) => starts.push(started.elapsed()),
                PlayerEvent::TrackEnded => ended += 1,
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    (starts, ended, started.elapsed())
}

#[test]
fn same_format_tracks_play_back_to_back() {
    if skip_hardware_playback() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.wav");
    let b = dir.path().join("b.wav");
    write_silent_wav(&a, 44_100, 2, 2);
    write_silent_wav(&b, 44_100, 2, 2);

    let (starts, ended, total) = play_queue(vec![a, b], Duration::from_secs(10));

    assert_eq!(starts.len(), 2, "each track starts once: {starts:?}");
    assert_eq!(ended, 2, "each track ends once");
    let second = starts[1];
    assert!(
        second >= Duration::from_millis(1700) && second <= Duration::from_millis(2500),
        "the second track must start at the boundary, started at {second:?}"
    );
    assert!(total < Duration::from_millis(5000), "took {total:?}");
}

#[test]
fn a_format_change_falls_back_and_plays_both() {
    if skip_hardware_playback() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a44.wav");
    let b = dir.path().join("b48.wav");
    write_silent_wav(&a, 44_100, 2, 1);
    write_silent_wav(&b, 48_000, 2, 1);

    let (starts, ended, _) = play_queue(vec![a, b], Duration::from_secs(10));

    assert_eq!(starts.len(), 2, "both tracks start: {starts:?}");
    assert_eq!(ended, 2, "both tracks end");
}
```

If `write_silent_wav` takes `seconds` as another integer type, match it. If `ReplaceQueue` or `QueuePlayIndex` are not re-exported through `znicz_core::Command`, use the path the other tests use.

- [ ] **Step 2: Run**

Run: `cargo test -p znicz-core --test playback -- --test-threads=1`
Expected on a machine with audio: all pass. Without a device or with `CI` set: the tests print a skip message and pass.

- [ ] **Step 3: Commit**

```bash
git add znicz-core/tests/playback.rs
git commit -m "test(core): gapless join and format-change fallback on real output (#92)"
```

---

### Task 7: Docs, changelog, version

**Files:**
- Modify: `Cargo.toml` (`version = "0.16.0"` → `"0.17.0"`), `Cargo.lock` (updated by the build)
- Modify: `CHANGELOG.md`
- Modify: `wiki/Domain/Playback-Pipeline.md`, `wiki/Architecture/Core-Engine.md`

- [ ] **Step 1: Version**

Set `version = "0.17.0"` in the root `Cargo.toml`, then run `cargo build --workspace` so `Cargo.lock` follows.

- [ ] **Step 2: CHANGELOG**

Add at the top, under `# Changelog`:

```markdown
## 0.17.0 - 2026-11-14

- Gapless playback: consecutive local tracks with the same sample rate and
  channel count play into the same output stream, with no silence or click
  between them. The title, cover and seek bar switch when the next track is
  heard. A format change, radio and repeat-one keep the old stop-and-reopen.
- AAC (M4A) files are trimmed of encoder delay and padding (`iTunSMPB`), as
  MP3 and Ogg Vorbis already were.
```

- [ ] **Step 3: Wiki**

Read both pages first and put the text where it fits the existing structure (match their heading levels and tone).

`wiki/Domain/Playback-Pipeline.md`, a section "Gapless playback":

```markdown
## Gapless playback

When a local track finishes decoding, the engine opens the next queue row
straight away. If it is a local file with the same sample rate and channel
count, its samples go into the same ring buffer right after the last sample of
the previous track: the output stream is not reopened, so nothing is lost at
the joint. Bit-perfect output stays bit-perfect; samples are not changed.

Encoder delay and padding are trimmed where the file says how much: MP3
(LAME/Xing header) and Ogg Vorbis by symphonia, AAC in M4A by Znicz from the
`iTunSMPB` tag. FLAC, WAV, AIFF and ALAC have none.

Old behaviour (drain the buffer, reopen the stream) is kept when the next row
is radio, has another sample rate or channel count, or fails to open, and with
repeat-one.
```

`wiki/Architecture/Core-Engine.md`, a section "Gapless joins":

```markdown
## Gapless joins

`PumpOutcome::Finished` calls `chain_next`, which picks the next row
(`pick_next_after`, starting after the last pending track) and opens it. On a
match the new decoder replaces the old one and a `PendingTrack { index, item,
info, starts_at }` is queued, `starts_at` being the feeder's pushed count at
the joint (`player/gapless.rs`).

Each loop pass `advance_pending` compares the played count (pushed − queued)
with the first pending `starts_at`. When it is reached the engine emits
`TrackEnded` then `TrackStarted`, sets `current_track` and `queue_position`
(found again by item if the queue was edited), and rebases the feeder count so
the position starts at 0:00.

`stop` clears the pending tracks; a seek while one is pending reopens the
playing row first; `SetDevice` and `finish_track` show any left at once.
```

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: all pass.

```bash
git add Cargo.toml Cargo.lock CHANGELOG.md wiki/Domain/Playback-Pipeline.md wiki/Architecture/Core-Engine.md docs/superpowers/specs/2026-11-14-gapless-playback-design.md docs/superpowers/plans/2026-11-14-gapless-playback.md
git commit -m "chore: release 0.17.0 (gapless playback)"
```
