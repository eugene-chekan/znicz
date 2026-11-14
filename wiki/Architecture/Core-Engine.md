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
- `Pause` / `Resume` — atomic flag; callback writes silence while paused. Pause also skips `pump_decode`, so a radio stream stops pulling bytes.
- `Seek(duration)` — seek the decoder, flush the ring. On a radio row this returns `radio cannot seek`.
- `QueueMove { from, to }` — move one queue row. The playing row stays playing (the playing index follows it); an index out of range is an error, `from == to` is a no-op. An older running player rejects it, and the client suggests `znicz player stop`.
- `SetVolume` ramps over 20 ms; `SetMuted` fades over 500 ms.
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

## Files to read

| File | Role |
|------|------|
| `player/engine.rs` | Loop, commands, queue |
| `audio/output.rs` | cpal stream, config pick |
| `audio/buffer.rs` | ring size and pair |
| `audio/source.rs` | Symphonia, `AudioSource` |
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
