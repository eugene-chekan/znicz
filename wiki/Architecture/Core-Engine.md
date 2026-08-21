# Audio engine (`znicz-core`)

The engine is a loop on a background thread named `znicz-player`.

Each turn it:

1. Drains `Command`s from the channel
2. Decodes some samples into the ring (`pump_decode`)
3. Updates the position clock
4. Sleeps a few milliseconds so it does not burn a CPU core

## Commands

See `znicz-core/src/player/commands.rs`. Examples:

- `Play(path)` — open file, open output stream, start decode
- `Pause` / `Resume` — atomic flag; callback writes silence while paused
- `Seek(duration)` — seek the decoder, flush the ring
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

## Files to read

| File | Role |
|------|------|
| `player/engine.rs` | Loop, commands, queue |
| `audio/output.rs` | cpal stream, config pick |
| `audio/buffer.rs` | ring size and pair |
| `audio/source.rs` | Symphonia |
| `player/state.rs` | snapshots for UI and MCP |
