# Realtime audio in Rust

“Realtime” here does **not** mean “as fast as possible”. It means: **when the sound card asks for samples, you must answer in a few milliseconds, every time**.

## What is forbidden in the callback

- `malloc` / `Vec::new` / `format!`
- `Mutex::lock` (it can wait)
- File or network I/O
- Logging that allocates (be careful with `tracing` in the callback)

Znicz’s callback pops `f32` from `rtrb` and scales volume from an atomic.

## What belongs on the player thread

- Symphonia decode
- Opening devices
- Seeking
- Talking to the TUI

## Sample format

Files may be 16-bit integer. The device may want `f32`. Converting **on the player thread** (or once per callback sample without allocating) is fine. Allocating a new `Vec` **inside** the i16 callback was a Phase 1 bug we removed.

## Speed = rate match

```
playback_speed ≈ device_rate / file_rate
```

If those rates differ and you do not resample, pitch and tempo both change. Config picking in `select_stream_config` exists to keep this ratio at **1**.

## Extra reading

- [Bencina: Time waits for nothing](https://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing)
- [cpal](https://github.com/RustAudio/cpal)
- [Symphonia getting started](https://github.com/pdeljanov/symphonia/blob/master/GETTING_STARTED.md)
- [rtrb design](https://github.com/mgeier/rtrb)
