# Gapless playback (#92)

## Goal

Tracks that run into each other (live albums, DJ mixes, classical works,
concept albums) play with no silence and no click at the boundary.

Gapless is always on. There is no setting: it never alters samples and falls
back to today's behaviour whenever it cannot apply.

## Today

- `AudioDecoder` already opens files with `FormatOptions { enable_gapless: true }`.
  In symphonia 0.5.5 this trims encoder delay and padding for MP3 (LAME/Xing)
  and Ogg Vorbis. FLAC, WAV, AIFF and ALAC need no trimming.
- AAC in M4A is not trimmed by symphonia. Its `iTunSMPB` freeform tag is
  exposed in the metadata (key ending in `iTunSMPB`), so we trim it ourselves.
- Opus is out of scope: symphonia 0.5 has no Opus decoder.
- When the decoder ends (`PumpOutcome::Finished`), the engine waits for the
  ring buffer to drain (`draining_since`), then `finish_track` calls
  `play_queue_index`, which stops and reopens the output stream. That drain and
  reopen is the gap.

## Design

### Chaining the next track

When the current decoder ends, the engine tries to chain the next track
before it starts draining.

`try_chain_next()` chains only when all of these hold:

- repeat is not `One` (repeat-one keeps today's behaviour);
- the current queue row is a local file, not a stream;
- `pick_next(true)` returns a row, and that row is a local file;
- the file opens without error;
- its sample rate and channel count equal the current decoder's.

If any check fails, the engine drains and reopens as today. A file that fails
to open is reported by the normal path (`finish_track` → `play_queue_index`).
The engine does not report it twice.

When chaining:

- the new decoder replaces the old one in `self.decoder`;
- the output stream, the ring buffer, the feeder and the rate converter stay
  as they are. Keeping the converter makes resampled output seamless too, and
  bit-perfect output stays bit-perfect;
- the engine stores
  `PendingTrack { index, item, info, starts_at }`, where `starts_at` is
  `feeder.pushed_samples()` at the moment of the switch: the first sample of
  the new track.

`pick_next(true)` is called once, when chaining, so shuffle picks one track
and does not advance its generator again.

### Switching at the boundary

On every loop pass, the engine compares the played position
(`pushed_samples − queued_samples`) with `pending.starts_at`. Once it is
reached:

- emit `TrackEnded`, then `TrackStarted(info)`;
- set `current_track`. Set `queue_position` to `pending.index` if that row
  still holds `pending.item`, otherwise to the first row equal to `item`,
  otherwise leave it unchanged (the row was removed; the audio is already
  buffered and plays);
- subtract `starts_at` from the feeder's pushed count, so the position count
  starts at 0:00 on the new track's first sample;
- clear `pending` and emit `StateChanged`.

The title, cover and seek bar therefore change within one loop pass (≤ 5 ms)
of the audio.

### Commands while a track is pending

- Play, Next, Previous, Stop, ReplaceQueue, QueuePlayIndex and removing the
  playing row already stop and restart playback. They clear `pending`.
- Seek: the decoder already belongs to the next track, so the engine replays
  the current row (`play_queue_index(queue_position)`) and then seeks it.
- Pause and Resume leave `pending` alone. Paused, the played position does not
  move, so the switch waits.
- SetDevice reopens the stream with the same rate and channels. The buffer is
  lost, so it clears `pending` and switches to the pending track at once
  (`TrackStarted`, position 0).
- Queue edits (add, move, remove another row) leave `pending` alone; the
  boundary resolves the row as above.

### AAC trimming

- A pure function `parse_itunsmpb(&str) -> Option<EncoderGap>` reads the tag
  value (` 00000000 00000840 000001CA 00000000001A8DF6 …`, hex fields:
  reserved, delay, padding, total frames). `EncoderGap { delay, total }`.
- `AudioDecoder::open` reads the tag from the format's metadata and, for AAC
  only, keeps an `EncoderGap`.
- `decode_next` drops the first `delay` frames and stops after `total` frames
  (counting frames after the delay), using a pure helper
  `trim_window(frame_index, frames, gap) -> Range<usize>`.
- `seek` sets the frame index from the actual seek position (`SeekedTo`) so
  the end trim stays right after a seek.

### Code layout

- `znicz-core/src/player/gapless.rs` (new): `PendingTrack`, `can_chain(...)`
  (the checks above, on plain data), `boundary_reached(played, starts_at)`,
  `resolve_position(queue, index, item)`. Pure, unit-tested.
- `znicz-core/src/audio/trim.rs` (new): `EncoderGap`, `parse_itunsmpb`,
  `trim_window`. Pure, unit-tested.
- `player/engine.rs`: `try_chain_next`, the boundary check in the run loop,
  the command handling above. The engine is close to 950 lines; the new logic
  goes into `gapless.rs` so it does not grow much.
- `audio/source.rs`: the tag lookup and the trim in `decode_next` and `seek`.

## Testing

- `gapless.rs`: every `can_chain` rule (repeat-one, stream, rate or channel
  mismatch, no next row); `boundary_reached`; `resolve_position` with an
  unchanged, moved and removed row.
- `trim.rs`: parsing real `iTunSMPB` values, malformed values, and the trim
  window at the start, middle and end of a track.
- `feeder.rs`: two decoders fed one after another through a `FakeSink` give one
  continuous run of samples with nothing lost or added at the joint.
- `tests/playback.rs` (hardware, skipped on CI as the other tests are): two
  short WAVs with the same format play as one: one `TrackStarted` per track,
  the second at the boundary, total time close to the sum; a 44.1 kHz then a
  48 kHz file falls back and still plays both.

## Docs and release

- Wiki: `wiki/Domain/Playback-Pipeline.md` (gapless section: what is trimmed,
  when it falls back) and `wiki/Architecture/Core-Engine.md` (chaining and the
  pending boundary).
- CHANGELOG and version bump to 0.17.0.
- Close #92. Mention in #90 that chaining is where crossfade would hook in.
