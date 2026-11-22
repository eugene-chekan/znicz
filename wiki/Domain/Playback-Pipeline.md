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
- multiply by volume and by the fade gain (both atomic, each ramped)
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

## Fades

Play and resume fade in, and pause, stop, a manual track change, a removed
playing row and a device switch fade out first. The sound is scaled by the
fade gain in the same callback step, so a fade is a short ramp, not a cut. The
times are `[audio.fade]` (150 ms in, 150 ms pause, 300 ms stop, 150 ms track,
500 ms mute by default; 0 turns one off). Automatic track changes do not fade:
they crossfade (below) or join gaplessly. See
[Core engine](../Architecture/Core-Engine.md#fade-outs) for the waiting commands
and [Audio threading](../Architecture/Audio-Threading.md) for the two gains.

Steady playback is bit-perfect. With no fade and volume 1.0 both gains are
exactly 1.0, and multiplying by 1.0 leaves every sample unchanged. A fade in
progress scales the samples on purpose, so the audio is not bit-identical until
the ramp has landed.

## Radio (HTTP)

A radio station uses the same line with a different first step: a blocking
HTTP GET on the **player thread** instead of opening a file. The body is a
continuous `Read`. Probe, decode, and the ring stay the same. The audio
callback still only pops samples. Starting a station (or any new item) **stops**
the current track first, so a dead URL cannot leave the previous file playing.
If the stream body drops mid-play, the engine stops rather than staying in
Playing with no decoder. Every stream GET sends `Icy-MetaData: 1`. If the
reply has `icy-metaint`, those metadata blocks are stripped so decode only
sees audio. A non-empty Icecast `StreamTitle` updates now-playing; an empty
title restores the station name. After a quarter second of audio, now-playing
shows the **coded bitrate** (bytes of compressed packets versus PCM duration).
That number updates as more of the stream is decoded. Icecast `icy-br` headers
are not used.

## Gapless playback

When a local track finishes decoding, the engine opens the next queue row
straight away. If it is a local file with the same sample rate and channel
count, its samples go into the same ring buffer right after the last sample of
the previous track: the output stream is not reopened, so nothing is lost at
the joint. Bit-perfect output stays bit-perfect; samples are not changed.

Encoder delay and padding are trimmed where the file says how much: MP3
(LAME/Xing header) and Ogg Vorbis by symphonia, AAC in M4A by Znicz from the
`iTunSMPB` tag. FLAC, WAV, AIFF and ALAC have none.

Repeat-one joins the track to itself the same way: the file is opened again
and its first sample follows the last one.

Old behaviour (drain the buffer, reopen the stream) is kept when the next row
is radio, has another sample rate or channel count, or fails to open.

## Crossfade

With `[audio.fade] crossfade_ms` above zero (Settings → Playback, up to 12 s;
off by default) an automatic track change overlaps the two tracks: the outgoing
track fades out while the incoming one fades in. The curve is equal power,
`cos` and `sin` of `t·π/2` over the crossfade, so the loudness stays even.
Repeat-one crossfades into itself: the same file is opened again from its
start.

A crossfade happens only when all of these hold:

- the crossfade time is not zero;
- the current track and the next row are both local files (not radio);
- they have the same sample rate and channel count;
- the current track's length is known.

The crossfade is cut to half of the current track's length. A next track whose
length is known and shorter than twice the crossfade is not crossfaded into:
it gets the gapless join, so short interludes do not blend away. Albums are not
special; turn the crossfade off for albums that run on without a break.

When a rule fails, the change happens as before: a gapless join (same format),
or drain and reopen. Manual changes (Next, Previous, choosing a row, replacing
the queue) keep the fade-out and fade-in from [Fades](#fades).

The mix is done before the samples enter the ring buffer, so the audio callback
is unchanged. The engine starts the crossfade on the frame where it should begin
(`crossfade_from`, from the packet timestamps), opens the next row, and mixes:
each frame is the outgoing sample times `cos` plus the incoming sample times
`sin`. If the outgoing track ends before the crossfade does, its side is
silence and the incoming track keeps rising. The title, cover and seek bar
switch when the crossfade starts to be heard. Samples change only while a
crossfade runs; steady playback stays bit-perfect.

**Shuffle:** when a crossfade is declined (the next track is too short, or it is
another format), the end-of-track join picks the next row again. With shuffle
that can be a different random row from the one the crossfade looked at.

See [Core engine](../Architecture/Core-Engine.md#crossfades) for the commands
that interrupt a crossfade.

## Next

- [Architecture overview](../Architecture/Overview.md)
