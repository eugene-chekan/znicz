# Audio engine (`znicz-core`)

The engine is a loop on a background thread named `znicz-player`.

Each turn it:

1. Drains `Command`s from the channel
2. Decodes some samples into the ring (`pump_decode`)
3. Updates the position clock
4. Sleeps a few milliseconds so it does not burn a CPU core

## Commands

See `znicz-core/src/player/commands.rs`. Examples:

- `Play(QueueItem)` — stop whatever is playing, then open a local file or an HTTP stream, open output, start decode. A failed open leaves the player Stopped.
- `Pause` / `Resume` — atomic flag; callback writes silence while paused. Pause fades out first (see [Fade-outs](#fade-outs)). Pause also skips `pump_decode`, so a radio stream stops pulling bytes.
- `Seek(duration)` — seek the decoder, flush the ring. On a radio row this returns `radio cannot seek`.
- `QueueMove { from, to }` — move one queue row. The playing row stays playing (the playing index follows it); an index out of range is an error, `from == to` is a no-op. An older running player rejects it, and the client suggests `znicz player stop`.
- `SetVolume` ramps over 20 ms; `SetMuted` fades over `[audio.fade] mute_ms` (500 ms by default). `SetFades` replaces the fade times at once; a new crossfade time applies to the next crossfade, not a running one.
- `NextTrack`, `QueueAdd`, …

## Output device

`AudioOutput` uses [cpal](https://github.com/RustAudio/cpal).

On open it:

1. Finds the device (default or `--device`)
2. Lists supported sample rates and channel counts
3. **Picks a config that can run at the file’s sample rate** when possible
4. Builds a stream and moves the ring **consumer** into the callback
5. Keeps the ring **producer** on the player thread

Linux uses ALSA (often via PipeWire). Windows uses WASAPI.

## Decoder

`AudioDecoder` in `audio/source.rs` wraps Symphonia:

- `probe` / `format` to find a track
- `decode` packets to interleaved `f32`
- gapless option enabled

A local file may treat a read error as end of track (`Ok(None)`). A radio stream is opened with `source.url()` set; the same Symphonia `IoError` is a decode failure. The engine then takes the Failed pump path, emits `PlayerEvent::Error`, and **stops** (status Stopped, output dropped). It does not restore the decoder.

## HTTP radio

`HttpStreamSource` (`audio/http.rs`) does a blocking GET on the player thread
and wraps the response body as a Symphonia `MediaSource` that cannot seek.
Decode keeps reading that body on the same thread. Pause stops the pump, so
bytes are not pulled while paused.

## Fade-outs

`player/fade.rs` decides which commands fade out first. A fade-out runs before
`Pause`, `Stop`, `NextTrack`, `PreviousTrack`, `Play`, `QueuePlayIndex`,
`ReplaceQueue`, `SetDevice`, and `QueueRemove` of the playing row. Their times
come from `[audio.fade]`: `pause_ms`, `stop_ms`, and `track_ms` for the track
commands; a zero time turns the fade off. Nothing fades when nothing is audible
(not Playing, or no stream open), and every other command runs at once.

The engine's `apply` holds commands that arrive during a fade-out in a waiting
list (`FadeOut`), in arrival order. `SetVolume`, `SetMuted` and `SetFades` run
at once. A `Resume` cancels a waiting `Pause` when that pause is the only
command waiting; the sound comes back up from where the fade got to. Any other
command waits. Once the callback reports the fade landed, or the deadline passes
(the fade time plus 500 ms), the waiting commands run in order. A command that
starts another fade-out takes the rest of the list along.

A track that ends during a fade-out waits for it, so a waiting `Pause`, `Stop`
or `NextTrack` applies to the track that was playing, not the next one.
Automatic track changes do not fade: they crossfade or join (see [Gapless joins](#gapless-joins) and [Crossfades](#crossfades)).
`SetFades` changes the times the next fade uses.

## Gapless joins

`PumpOutcome::Finished` calls `on_decoder_finished`, which calls `chain_next`. It
picks the next row (`pick_next_after`, starting after the last pending track,
found again in the queue by item; if it was removed, no join) and opens it. On a
match the new decoder replaces the old one and a `PendingTrack { index, item,
info, starts_at }` is queued, `starts_at` being the feeder's pushed count at
the joint (`player/gapless.rs`).

Each loop pass `advance_pending` compares the played count (pushed − queued)
with the first pending `starts_at`. When it is reached the engine emits
`TrackEnded` then `TrackStarted`, sets `current_track` and `queue_position`
(found again by item if the queue was edited), and rebases the feeder count so
the position starts at 0:00.

`stop` clears the pending tracks; a seek while one is pending reopens the
playing row first, and is refused (`nothing to seek in`) when the queue was
cleared under the chained audio; `SetDevice` and `finish_track` show any left
at once.

`may_chain` allows any local file, repeat-one included. With repeat-one the
pick is the playing row itself, so the same file follows its own end in the
same stream and nothing is reopened.

## Crossfades

`player/engine/crossfade.rs` drives a crossfade; the mix itself is in
`audio/crossfade.rs` (`Crossfade`, `CrossfadeMix`, `crossfade_gains`). The rules
and the curve are in [Playback pipeline](../Domain/Playback-Pipeline.md#crossfade).

- `arm_crossfade` runs whenever a decoder becomes current (play, a gapless join,
  the end of a crossfade) and on `SetFades`. It sets `crossfade_from` (in
  `player/fade.rs`): the frame where the crossfade starts, `length` before the
  end and at most half the track, or none when a rule fails.
- A track whose crossfade failed (the next row did not open or does not match,
  or it is too short) is marked `crossfade_declined`. `SetFades` does not re-arm
  it, and the flag is cleared when the next track starts.
- The main decode step yields (`DecodeStep::Yield`) at `crossfade_from`, judged by
  `AudioDecoder::position_frames`. `start_crossfade` picks the next row with
  `open_next_row` (the same pick as gapless), queues it as a pending track whose
  `starts_at` is the pushed count, and builds the `Crossfade`. The length is
  `min(crossfade, frames left)`, so a seek past `crossfade_from` still gets a
  shorter crossfade.
- While it runs, `self.crossfade` owns both decoders and `self.decoder` is empty.
  `has_source` and `source_format` look at the crossfade too, so the device
  switch and the idle check see it. `pump_decode` pumps the crossfade in place of
  the main decoder; pause stops the pump, so the mix waits and resumes with the
  track.
- When the mix is done (after its leftover is sent) the step yields and `hand_over`
  makes the incoming decoder current and arms its own crossfade.

Commands while a crossfade runs:

| Command | Effect |
|---|---|
| Seek before the incoming track is heard (pending track exists) | `replay_current` reopens the playing row; `play_item` stops first, which drops the crossfade and its pending track; then the seek. |
| Seek after it is heard | `end_heard_crossfade` ends it at once (the incoming track becomes current), then the seek. |
| Next, Previous, Play, Stop, replacing the queue | They go through `stop()`, which drops the crossfade; their fades apply. |
| Pause / Resume | Decoding waits; the mix continues on resume. |
| `SetDevice` | `end_running_crossfade` first, so the incoming track is current; then the stream reopens for the new device. The mix was built for the old device's channels and rate. |
| Queue edits | The pending row is found again by item (`resolve_position`). |
| `SetFades` | The new time applies to the next crossfade; a running one keeps its length. |

## Files to read

| File | Role |
|------|------|
| `player/engine.rs` | Loop, commands, queue |
| `audio/output.rs` | cpal stream, config pick |
| `audio/buffer.rs` | ring size and pair |
| `audio/source.rs` | Symphonia, `AudioSource` |
| `audio/crossfade.rs` | Equal-power curve, the crossfade mix |
| `player/engine/crossfade.rs` | Starting, handing over and ending a crossfade |
| `player/fade.rs` | Fade-out commands, crossfade start frame and length |
| `audio/http.rs` | HTTP GET, unseekable body |
| `station.rs` | `stations.toml`, `play_station` |
| `player/state.rs` | snapshots for UI and MCP (`QueueItem`, `Queue`) |

## Queue revision

`PlayerState.queue` is a `Queue`: the items behind an `Arc` and a `revision`
that goes up on every change (`items_mut`, `replace` are the only writers).
Cloning a state snapshot copies the `Arc`, not the items. On the wire the queue
is still a plain JSON array.

Over IPC, a `State` request carries the revision the client already holds
(`queue_revision`). When it matches, the player replies with an empty queue and
`queue_unchanged: true`, and the client puts back the queue it cached. The cache
belongs to one connection, so a reconnect (a new player process, whose revisions
start again) drops it. Older clients send no revision and get the full queue;
older players never set `queue_unchanged`.
