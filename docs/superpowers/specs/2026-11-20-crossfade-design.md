# Crossfade between tracks (#90, part 2)

**Version:** 0.19.0. Closes #90.

## Goal

When one track ends and the next starts on its own, the two overlap for a set
time: the old one fades out while the new one fades in. Off by default; up to
12 s. The Playback tab's rows become sliders with a typed value.

## When it crossfades

At an automatic track change only, when all of these hold:

- the crossfade time is not zero;
- the current track and the next row are both local files (not radio);
- they have the same sample rate and channel count;
- the current track's length is known (`TrackInfo.duration`).

Repeat-one crossfades too: the next track is the same file opened again from
its start.

Albums are not special: every automatic change crossfades. Turn crossfade off
for albums that run continuously.

The crossfade is cut to half of the current track's length. A next track
whose length is known and shorter than twice the crossfade is not crossfaded
into: it gets the gapless join, so short interludes do not blend away.

When a rule fails, the change happens as today: a gapless join (same format),
or drain and reopen. Manual changes (Next, Previous, choosing a row, replacing
the queue) keep the v0.18.0 fade-out and fade-in.

Gapless joins now also loop a repeat-one track (`may_chain` no longer excludes
repeat-one): a repeat-one track restarts without reopening the stream.

## Design

### Mixing (`znicz-core/src/audio/crossfade.rs`, new)

Mixing happens in the engine, before samples enter the ring buffer. The mix is
decided by sample position, not by time, so the audio callback does not
change.

- `Crossfade` holds the incoming decoder, its own rate converter (a second
  `RateConverter` when the output resamples), a buffer of its decoded but not
  yet mixed samples, `length` (frames) and `done` (frames mixed so far).
- `mix(&mut self, outgoing: &[f32], channels) -> Vec<f32>`: for each frame of
  the outgoing packet, take one frame of the incoming track (decoding more as
  needed), and write `a·cos(t·π/2) + b·sin(t·π/2)` with `t = done / length`
  (equal power: the loudness stays even).
- If the outgoing track ends before `length` (its length was an estimate),
  the incoming track keeps rising on the same curve alone, until `t = 1`.
- If the outgoing track has frames left at `t = 1` (it was longer than its
  stated length), they are dropped.
- At `t = 1` the crossfade is over: the incoming decoder and converter become
  the engine's current ones, and the outgoing decoder is dropped.
- The incoming track's samples go through `adapt_channels` and its converter
  exactly as the main decoder's do.

The curve and the frame bookkeeping are pure and unit-tested:
`crossfade_gains(t) -> (f32, f32)`, and `CrossfadeMix`, which takes the
outgoing packet and a closure that yields incoming packets, so tests feed it
synthetic samples. Incoming samples decoded but not mixed when the crossfade
ends are pushed before the incoming decoder takes over, so nothing is lost.

### Engine (`player/engine.rs`)

- `AudioDecoder::position_frames()`: the frame index of the next decoded
  frame, from the packet timestamps (`ts_to_frames`), so it is right after a
  seek.
- When a decoder becomes current (play, gapless join, end of a crossfade) or
  the crossfade time changes, the engine works out `crossfade_from`: the frame
  where the crossfade should start (`total − min(crossfade, total / 2)`), or
  none when a rule fails. The main decode step yields (a new
  `DecodeStep::Yield`, `PumpOutcome::Yielded`) as soon as the decoder's
  position reaches it, so the crossfade starts on time and not up to a pump
  pass (32 packets) late. On a yield the engine starts the crossfade:
  - pick the next row with `pick_next_after` (as gapless; for repeat-one this
    is the playing row);
  - open it; check the format and the incoming length; on a failure,
    clear `crossfade_from` for this track and carry on (the end of the track
    then joins as today);
  - the length is `min(crossfade, frames left)`, so a seek past
    `crossfade_from` still gets a shorter crossfade;
  - queue a `PendingTrack` with `starts_at = feeder.pushed_samples()`, so the
    title, cover and seek bar switch when the crossfade starts to be heard and
    the new track's position counts from its first sample.
- While a crossfade runs, `self.crossfade` owns both decoders and
  `self.decoder` is empty; checks that ask "is a decoder open" (audible,
  decoding, the device switch's format, the idle wait) also count a running
  crossfade. The pump's decode step mixes: an outgoing packet goes through the
  mixer; when the outgoing decoder ends, the mix continues with silence for
  the outgoing side until `t = 1`.
- When the crossfade ends, its decoder becomes `self.decoder` and no drain or
  gapless chain runs for the old track.

Commands while a crossfade runs:

| Command | Effect |
|---|---|
| Seek while the crossfade is not yet heard (a pending track exists) | Drop the crossfade and its pending track; seek the outgoing track. |
| Seek after it is heard | End the crossfade at once (the incoming decoder becomes current, the outgoing one is dropped), then seek. |
| Next, Previous, Play, Stop, replacing the queue | They go through `stop()`, which drops the crossfade; their v0.18.0 fades apply. |
| Pause / Resume | Decoding waits; the mix continues on resume. |
| Device switch | The crossfade ends at once (the incoming track becomes current, the outgoing one is dropped), then the stream reopens for the new device as for a normal track. The mix was built for the old device's channels and rate. |
| Queue edits | The pending row is found again by item, as gapless. |
| `SetFades` | A new crossfade time applies to the next crossfade, not the running one. |

### Config

`[audio.fade] crossfade_ms = 0` (off). `FadeTimes` gets `crossfade:
Duration`; `SetFades` carries it. `default-config.toml` and the README
template gain the key.

### Settings: slider rows

Every row on the Playback tab becomes a slider:

| Row | Range | Step | Shown as |
|---|---|---|---|
| Fade in (play, resume), Fade on pause, Fade on stop, Fade on track change, Mute fade | 0–2000 ms | 50 ms | `off`, `150 ms` |
| Crossfade | 0–12 s | 0.5 s | `off`, `3.5 s` |

- The row shows a bar (`format::progress_bar`, half-cell steps) about 16
  cells wide, then the value.
- `h` / `l` and `←` / `→` move one step and stop at the ends. A value off the
  step grid (typed, or from the file) moves to the next grid value in that
  direction.
- Enter or Space opens a prompt in the explanation strip with the current
  value: `Crossfade (0–12 s): 3.5`. The prompt uses `line_edit.rs` (caret,
  Left / Right, Home / End, Backspace).
  - Accepted: a number in the row's unit (seconds for crossfade,
    milliseconds for the fades), or with a unit: `3.5s`, `3500ms`, `150 ms`.
  - Enter saves. Esc cancels. An invalid or out-of-range value keeps the
    prompt open and the strip says why (`0–12 s`).
  - While the prompt is open, other Settings keys (tabs, rows) are text input
    or ignored; the footer shows `Enter save · Esc cancel`.
- `Kind::Slider { min, max, step, unit }` replaces the fade ladder
  (`FADE_LADDER_MS`, `step_fade` go). Parsing (`parse_slider_value`) and
  stepping (`step_slider`) are pure functions in `settings.rs`.
- A change saves `[audio.fade] <key>` in milliseconds and sends `SetFades`, as
  today.

## Bit-perfect

Samples change only while a crossfade runs. Steady playback is untouched.

## Testing

- `crossfade.rs`: the gains at t = 0, ½, 1 (equal power: a² + b² = 1); a mix of
  two constant sources; packet sizes that differ; the outgoing source ending
  early; the hand-over at the end.
- Engine rules: a pure `should_crossfade(...)` for each rule (time zero,
  stream, format, unknown length, repeat-one allowed) and the length cap.
- `gapless.rs`: `may_chain` allows repeat-one.
- `tests/playback.rs` (hardware, skipped on CI): two 3 s WAVs with a 1 s
  crossfade take about 5 s, and the second `TrackStarted` comes about 1 s
  before the first would end; repeat-one with a crossfade keeps playing past
  the track's length.
- Settings: slider stepping, snapping, the prompt (valid, with units,
  invalid, out of range, Esc), save and `SetFades`; the slider row draws.

## Docs and release

- Wiki: `Domain/Playback-Pipeline.md` (crossfade), `Architecture/Core-Engine.md`
  (crossfade in the pump, commands), `Architecture/TUI.md` (slider rows and
  the prompt).
- README config template, CHANGELOG, version 0.19.0, third-party licences.
- Close #90.
