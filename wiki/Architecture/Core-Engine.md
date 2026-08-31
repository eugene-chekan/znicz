# Audio engine (`znicz-core`)

The engine is a loop on a background thread named `znicz-player`.

Each turn it:

1. Drains `Command`s from the channel
2. Decodes some samples into the ring (`pump_decode`)
3. Updates the position clock
4. Sleeps a few milliseconds so it does not burn a CPU core

## Commands

See `znicz-core/src/player/commands.rs`. Examples:

- `Play(QueueItem)` — open a local file or an HTTP stream, open output, start decode
- `Pause` / `Resume` — atomic flag; callback writes silence while paused. Pause also skips `pump_decode`, so a radio stream stops pulling bytes.
- `Seek(duration)` — seek the decoder, flush the ring. On a radio row this returns `radio cannot seek`.
- `SetVolume`, `NextTrack`, `QueueAdd`, …

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

A local file may treat a read error as end of track (`Ok(None)`). A radio stream is opened with `source.url()` set; the same Symphonia `IoError` is a decode failure. The engine then takes the Failed pump path and emits `PlayerEvent::Error`, not a silent finish.

## HTTP radio

`HttpStreamSource` (`audio/http.rs`) does a blocking GET on the player thread
and wraps the response body as a Symphonia `MediaSource` that cannot seek.
Decode keeps reading that body on the same thread. Pause stops the pump, so
bytes are not pulled while paused.

## Files to read

| File | Role |
|------|------|
| `player/engine.rs` | Loop, commands, queue |
| `audio/output.rs` | cpal stream, config pick |
| `audio/buffer.rs` | ring size and pair |
| `audio/source.rs` | Symphonia, `AudioSource` |
| `audio/http.rs` | HTTP GET, unseekable body |
| `station.rs` | `stations.toml`, `play_station` |
| `player/state.rs` | snapshots for UI and MCP (`QueueItem`) |
