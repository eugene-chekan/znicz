# Remove the playing queue row

**Date:** 2026-08-31
**Status:** Approved
**Crates:** `znicz-core`, `znicz-tui`
**Issues:** [#14](https://github.com/eugene-chekan/znicz/issues/14) (this slice),
[#15](https://github.com/eugene-chekan/znicz/issues/15) (MCP `queue_remove`, later)

Deleting the row that is currently playing leaves the list and the decoder
disagreeing: a different row can look like it is playing, while the deleted
file still makes sound. Pause and resume stay on that deleted file.

## Problem

`Command::QueueRemove` drops the row and may move `queue_position`. It does
not reliably stop or replace the decoder. After `d` on the playing row, the
playing marker, the now-playing line, and the sound card can name three
different things.

## Goals

1. After any remove, the playing marker, the now-playing line, and
   pause/resume all refer to the row the decoder has open. The deleted file
   must not keep making sound.
2. Delete the **playing** row: play the row that **slides into that index**,
   or **stop** if nothing remains there.
3. Delete any **other** row: keep playing the same file or station. If the
   removed row was before the playing one, the playing index moves down by
   one.
4. Same rule for a file and for a station.

## This slice does not include

- MCP `queue_remove` ([#15](https://github.com/eugene-chekan/znicz/issues/15))
- Changing Next, Previous, shuffle, or repeat
- Auto-skip past a dead replacement (error, stay stopped)
- Retuning `C` / `QueueClear` (whether clear should stop)
- Parked issues `#5`–`#9`, Phase 5, Phase 6

## Daily motion

Three tracks in the queue, the middle one playing. `]`, cursor on that row,
`d`. The last track moves up into that slot and **starts**. The middle file
is silent.

If it was the last row that was playing, `d` **stops**. The list keeps the
earlier rows. Nothing is decoded.

If you `d` a row that is not playing, the gap closes and the current track
keeps going.

## Queue behaviour

`QueueItem` and `Command::QueueRemove(usize)` do not change. The engine
function `remove_from_queue` is the one place that applies the rule. TUI
`d` / Delete already send that command.

| You delete… | Playback |
| --- | --- |
| A row **before** the playing one | Keep going. Playing index `-= 1`. |
| A row **after** the playing one | Keep going. Index unchanged. |
| The **playing** row, and a row remains at that index | **Stop** the old decoder, then play **by index** the row that slid in. Shuffle and repeat do not pick a different row. |
| The **playing** row, and nothing remains at that index (last row, or the only row) | **Stop**. Playing index is the last remaining row, or `0` if the queue is empty. |
| While **paused** on the playing row, and a replacement exists | Same as playing: the replacement **starts**. Pause does not stick. |

A failed open of the replacement: already stopped, surface the error, do not
skip ahead, do not revive the deleted file.

## TUI

After `QueueRemove` returns, clamp the queue cursor using the **length
after** the command (not a snapshot from before the remove).

- Replacement exists: cursor stays on that same index (the row that slid in).
- Last / only playing row deleted: cursor sits on the last remaining row, or
  nowhere if the queue is empty.

`o` still jumps to the playing index.

## Tests

`znicz-core` (and a TUI cursor test) pin:

- Remove the playing row when another row follows: shorter queue, playing
  index in range, pause/resume would control **that** row, not the deleted
  one.
- Remove the last (or only) playing row: **Stopped**, playing index in range
  or queue empty.
- Remove a row before / after the playing one: decoder unchanged; list
  closes the gap (existing tests cover the list).
- A dead replacement file: error, stopped, deleted file not playing.

Tests that open a stream use **loopback only**. Skip hardware when `CI` is
set.

## Docs

- GitHub [#14](https://github.com/eugene-chekan/znicz/issues/14) is this bug.
- GitHub [#15](https://github.com/eugene-chekan/znicz/issues/15) is MCP
  `queue_remove` after this fix.
- `wiki/Issues.md` lists both.
- When this ships: `wiki/Architecture/TUI.md` queue paragraph notes that `d`
  on the playing row starts the next remaining one (or stops). Workspace
  patch version in the same cycle.

## Out of scope (again)

No MCP tool in this slice. Agents still use `queue_clear` or TUI `d` until
[#15](https://github.com/eugene-chekan/znicz/issues/15).
