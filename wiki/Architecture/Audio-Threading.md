# Threads and the realtime rule

Znicz uses three kinds of work:

| Thread | Job | May allocate / lock? |
|--------|-----|----------------------|
| UI (main) | Draw TUI, read keys | Yes |
| Player | Decode, handle commands | Yes |
| Audio callback | Fill the device buffer | **No** |

The audio callback is **realtime**. If it waits (mutex, disk, malloc), the device runs out of samples. Then you hear clicks, silence, or — on some systems — a **speed jump** while the graph recovers.

## Lock-free ring

```
Player thread                    Audio callback
     |                                  |
  Producer.push(sample)           Consumer.pop()
     |                                  |
     +-------- rtrb ring buffer --------+
```

`rtrb` is SPSC: **s**ingle **p**roducer, **s**ingle **c**onsumer. That matches this design. Do not share one `Mutex` around both ends.

Pause uses an `AtomicBool`. Gain uses a `GainControl` (target, ramp time, generation, and the generation that has landed; all atomics) in `znicz-core/src/audio/gain.rs`, so the callback reads it without a lock. There are two of them: the **volume** gain (the user's volume, and the mute) and the **fade** gain (play, resume, pause, stop, track change and device switch fades). Each stream callback keeps its own `GainRamp` for each and applies `volume × fade` once per **frame** (all channels share it). A volume change ramps over 20 ms to avoid clicks; a mute fades over `[audio.fade] mute_ms` (500 ms by default). The engine drives the fade gain through `AudioOutput::fade_to` and `fade_in` (`audio/output.rs`): a fade-out before a command, a fade-in after play or resume. Ramps keep running while paused: the paused flag writes silence, but the gains still move, so a pause fade-out reaches silence before the pause takes effect.

Each control has a generation. When a ramp reaches its target, the callback marks that generation **landed** (`has_landed`). The engine waits for the landed marker before it runs the command that was waiting for a fade-out, with a grace period in case the callback never reports. A finished ramp lands exactly on the target, so gain 1.0 passes samples through untouched (bit-perfect). A stream opened with a pending fade-in starts from silence; any other stream starts at the target, so an unmute fade that is still running is not replayed on a track change.

## Back-pressure: never drop audio

The ring is small on purpose (about two seconds), so it fills up quickly. A full
ring is normal, not an error. When a decoded packet does not fit, the player must
**keep the leftover** and write it later. Dropping it makes the music skip
forward, which sounds like fast-forward. `Feeder` in
`znicz-core/src/audio/feeder.rs` owns that leftover, and its tests assert that no
sample is ever lost. See [the playback pipeline](../Domain/Playback-Pipeline.md#5-why-speed-went-wrong-and-how-we-fixed-it).

## Seek

Seek cannot “rewind” the ring from the producer side. The player sets a **flush** flag. The callback throws away queued samples, then decode continues from the new position. The leftover held by the feeder is dropped too, because it belongs to the old position. Radio streams cannot seek: the engine returns `radio cannot seek` instead of moving the HTTP cursor.

## Crossfade

A crossfade mixes the two tracks on the player thread, before the samples go into
the ring (`audio/crossfade.rs`, driven from `player/engine/crossfade.rs`). The
callback does not change: it still pops samples, scales them by the gains and
writes. The mix goes through the same feeder as a decoded packet, so its leftover
is kept under the back-pressure rule above.

## End of track

When the decoder reaches the end, up to two seconds of music are still in the
ring. The player waits for the ring to drain before it reports `TrackEnded`,
otherwise the end of every track would be cut off.

## Golden rules

1. Callback: pop, scale by the frame gain, write. Stop.
2. Match **sample rate** and **channels** to the file when the device allows it.
3. Never throw away decoded samples to make them fit. Keep them for the next round.
4. Prefill the ring a little before you need it, but do not hold a lock while decoding.

## Extra reading

- [rtrb](https://docs.rs/rtrb/)
- [Real-time audio programming 101](http://www.rossbencina.com/code/real-time-audio-programming-101-time-waits-for-nothing) (C++, same rules)
- [cpal streams](https://docs.rs/cpal/)
