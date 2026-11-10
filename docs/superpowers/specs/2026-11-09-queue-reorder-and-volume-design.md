# Queue reordering, finer volume, and mute fades

**Date:** 2026-11-09
**Status:** Approved
**Crates:** `znicz-core`, `znicz-tui`, `znicz-mcp`
**Issues:** [#36](https://github.com/eugene-chekan/znicz/issues/36) (reorder rows), [#47](https://github.com/eugene-chekan/znicz/issues/47) (finer volume)

Part 5, the last part, of the five-part TUI rework (see the
[roadmap](../../../wiki/Plans/Roadmap.md)). Parts 1–4 shipped in 0.6.0–0.9.0.

## Problem

- The queue can only be reshaped by removing and re-adding rows; there is no
  way to move a row, in the TUI or over MCP.
- `+` / `-` step the volume by 5 %, and the volume bar has four cells — too
  coarse for level matching.
- Mute cuts the sound instantly, and every gain change is an instant jump that
  can click.

## Goals

1. Move the highlighted queue row with `Alt-↑` / `Alt-↓`; an MCP `queue_move`
   tool does the same.
2. 1 % volume steps and a volume bar with eighth-cell resolution.
3. Mute and unmute fade over 500 ms; other gain changes get a 20 ms ramp.

## Non-goals

- Fades on pause, resume, or stop (later, with a settings screen).
- Configurable fade times.
- Reordering saved playlists (#18), the library, or by mouse drag (#8).
- A second reorder key pair for terminals that do not send Alt-arrows.

## Queue reordering (#36)

### Engine

`Command::QueueMove { from: usize, to: usize }`:

- Removes the row at `from` and inserts it at `to` (both 0-based, positions in
  the queue as it is before the move). Out of range → error
  `queue index out of range`. `from == to` does nothing.
- The playing row stays the playing row:
  - `from == queue_position` → `queue_position = to`;
  - `from < queue_position && to >= queue_position` → `queue_position -= 1`;
  - `from > queue_position && to <= queue_position` → `queue_position += 1`;
  - otherwise unchanged.
- Playback never restarts or jumps. Shuffle is unaffected.
- Emits `QueueChanged`, so the session persister saves the new order.

### TUI

With the queue drawer focused (Browse or Stage):

- `Alt-↑` moves the highlighted row up one, `Alt-↓` down one; the cursor stays on
  the moved row. No-op at the top / bottom; no wrap.
- `QUEUE` key table: `Alt-↑ / Alt-↓` — "move the row up / down". The footer is
  unchanged.
- README notes that some terminals (e.g. macOS Terminal without "Use Option as
  Meta") do not send Alt-arrows.

### MCP

`queue_move { from, to }` (0-based) sends `QueueMove` and returns the new queue,
in the same shape as `queue_remove`.

## Finer volume (#47)

- `VOLUME_STEP` becomes `0.01`: `+` / `=` and `-` / `_` move 1 %. Holding the key
  repeats. Mute and MCP `set_volume` (0.0–1.0) keep their ranges.
- `format::volume_bar(volume, cells)` draws `cells` characters with eighth-cell
  resolution: full blocks `█`, then at most one partial block from
  `▏▎▍▌▋▊▉` (1/8 … 7/8), then `▁` for the rest. Levels shown: `8 × cells`.
  Rounding is to the nearest eighth.
- Widths: Browse transport 6 cells (48 levels), Stage volume line 10 cells
  (80 levels). The transport still drops the volume segment first on narrow
  lines.

## Mute fade and gain ramps

### Behaviour

- Mute ramps the output gain to 0 over **500 ms**; unmute ramps from the current
  gain back to the volume over 500 ms. `PlayerState.muted` changes at once, so
  the UI shows `muted` immediately while the sound fades.
- Muting or unmuting during a fade reverses from the gain reached so far, over a
  fresh 500 ms ramp.
- Every other gain change (volume keys, MCP `set_volume`) ramps over **20 ms**,
  which removes clicks from instant jumps.
- A ramp always ends exactly on its target, so at 100 % volume, unmuted, the gain
  is exactly `1.0` and samples pass through unchanged (bit-perfect output is kept).
- A newly opened output stream starts at the current target gain with no ramp.

### Design

- `AudioOutput` replaces `volume_bits` with a shared `GainControl`:
  `target_bits: AtomicU32` (f32 gain), `ramp_ms: AtomicU32`, and
  `generation: AtomicU64` (bumped on every change).
- `AudioOutput::set_gain(target: f32, ramp: Duration)` stores target and ramp,
  then bumps the generation. `set_volume(v)` keeps working as
  `set_gain(v, 20 ms)`.
- The audio callback keeps a `GainRamp { current, target, step, frames_left,
  generation }`. At the start of each callback, if the generation changed, it
  starts a new linear ramp from `current` to the new target over
  `ramp_ms × sample_rate / 1000` frames (at least 1). Each **frame** (all
  channels) is scaled by the same gain; the gain advances once per frame and is
  set exactly to `target` when `frames_left` reaches 0.
- The engine calls `set_gain(effective_volume, 500 ms)` for `SetMuted`, and
  `set_gain(effective_volume, 20 ms)` for `SetVolume`.
- `GainRamp` is a plain struct with pure methods (`retarget`, `next_frame_gain`)
  so it can be unit-tested without a device.

## Testing

- Engine: `QueueMove` reorders and keeps the playing index on the playing item
  (moving the playing row; moving a row across it in both directions; moving
  rows that do not cross it); out-of-range error; `from == to` no-op;
  `QueueChanged` emitted.
- TUI keys: `Alt-↓` / `Alt-↑` move the row and the cursor; no-op at the ends; do
  nothing with library focus.
- MCP: `queue_move` returns the reordered queue; out-of-range is an error.
- `volume_bar`: 0 % all `▁`; 100 % all `█`; 50 % of 6 cells is `███▁▁▁`;
  1/8 of a 1-cell bar is `▏` and 3/8 is `▍`; output is always `cells`
  characters.
- Volume keys: one press moves 0.01.
- `GainRamp`: a 500 ms ramp at 48 kHz takes 24 000 frames and ends exactly on the
  target; reversing mid-ramp starts from the current gain; a 1.0 target ends at
  exactly `1.0`; no change keeps the gain constant.

## Delivery

- Bump `[workspace.package] version` to **0.10.0**.
- README: key table (1 % volume, `Alt-↑` / `Alt-↓`, Alt-arrow terminal note),
  mute fades.
- Wiki: `Architecture/TUI.md` (queue keys, volume bar), `Architecture/MCP.md`
  (`queue_move`), `Architecture/Core-Engine.md` and/or
  `Architecture/Audio-Threading.md` (`QueueMove`, gain ramps and the mute
  fade), roadmap (part 5 done — the rework is complete), `Issues.md` (#36, #47
  closed).
- The MCP playback skill (if it lists queue tools) mentions `queue_move`.
- PR closes #36 and #47.
