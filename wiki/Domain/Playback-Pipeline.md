# How playback works

Think of a factory line.

```
File on disk
    → open & probe (what codec? what rate?)
    → decode packets to PCM samples
    → push samples into a ring buffer
    → sound-card callback pops samples
    → speakers / DAC
```

## 1. Probe

We open the file and ask Symphonia: “what is this?” We learn sample rate, channels, duration, codec name. That becomes **now playing** text.

## 2. Decode

Compressed formats arrive in **packets** (chunks). The decoder turns each packet into a block of PCM samples (we use `f32` in memory for a simple path).

This runs on the **player thread**, not on the audio callback. Decode can allocate and read disk. That is OK here. It is **not** OK inside the sound-card callback.

## 3. Ring buffer

A **ring buffer** is a fixed-size circle of slots.

- The decoder **produces** (writes) samples
- The sound card **consumes** (reads) samples
- If the circle is empty, you hear silence (an **underrun**)
- If the circle is full, the decoder waits

Znicz uses `rtrb`: one producer, one consumer, **no mutex**. That is required so the sound-card thread never waits on a lock.

## 4. Sound-card callback

The OS calls us many times per second: “fill this small buffer **now**.” We may only:

- pop from the ring
- multiply by volume (atomic)
- write silence if paused

No file I/O. No `Mutex`. No `Vec` allocation.

## 5. Why speed went wrong (and how we fixed it)

Bug you heard: **normal for 3–5 seconds, then 2×–3× speed**. Seek made it normal again for a few seconds.

### The real cause: thrown-away samples

The ring buffer holds about two seconds. Decoding is far faster than playback, so
after a few seconds the ring is **full**. At that moment a decoded packet no
longer fits completely. The old code wrote what fitted and then **discarded the
rest of the packet**:

```rust
let written = output.push_samples(&samples);
if written < samples.len() {
    break; // the remaining samples are lost forever
}
```

A FLAC packet is about 4096 frames, which is 8192 samples in stereo. The player
only checked that ~1024 slots were free before decoding, so most of every packet
was dropped once the ring filled. The samples that survive are played at the
correct rate, but the *music* jumps forward — that is what you hear as
fast-forward.

The timing explains the rest of the symptom:

- the first seconds are fine, because the ring still has room
- seeking empties the ring, so it sounds correct again until it refills

Measured on a 15 second file: **2.019×** before the fix, **1.002×** after.

Note that the position display looked correct the whole time, because it counted
samples *sent* to the device, not the music actually contained in them.

### The fix

Keep the leftover. `Feeder` (`znicz-core/src/audio/feeder.rs`) stores the tail of
a partly written packet and writes it first on the next round. Nothing is ever
dropped, so playback cannot run ahead.

### Two other ways speed can break

Both are now handled in `znicz-core/src/audio/convert.rs`:

1. **Wrong channel count.** Mono samples sent to a stereo stream play twice as
   fast, because the device takes two samples per frame. We remap channels.
2. **Wrong sample rate.** 44.1 kHz audio in a 96 kHz stream plays about twice as
   fast. We first ask the device for the file's own rate (bit perfect). If it
   refuses, we resample instead of letting the speed drift.

### Checking it yourself

```bash
cargo run --release -p znicz-core --example timing -- yourfile.flac
```

It prints a speed factor. Anything other than about 1.000× is a bug.

See [Audio threading](../Architecture/Audio-Threading.md).

## Radio (HTTP)

A radio station uses the same line with a different first step: a blocking
HTTP GET on the **player thread** instead of opening a file. The body is a
continuous `Read`. Probe, decode, and the ring stay the same. The audio
callback still only pops samples.

## Next

- [Architecture overview](../Architecture/Overview.md)
