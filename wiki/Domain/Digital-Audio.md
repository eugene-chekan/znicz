# Digital audio in plain words

A speaker moves air. A computer only stores **numbers**. Digital audio is the trick that turns numbers into motion, and motion back into numbers.

## Sampling

Imagine a wave (the sound). Many times per second, we write down “how high is the wave right now?”. Each write-down is a **sample**.

**Sample rate** = how many samples per second.

| Rate | Meaning |
|------|---------|
| 44 100 Hz | CD quality. 44 100 samples every second, per channel |
| 48 000 Hz | Common for video and many DACs |
| 96 000 Hz | High-res. Twice as many numbers per second as 48 kHz |

If you play 44 100 samples per second through a device that thinks it is getting 96 000 samples per second, the music runs **too fast** (about 2×). Matching these two numbers is the most important speed rule in Znicz.

## Bit depth

Each sample is a number with a certain size:

- **16-bit** — CD. About 65 000 possible values
- **24-bit** — studio / hi-res. Much finer steps
- **float (f32)** — what many computer mixers use inside

More bits = quieter details can still be stored. It does **not** make music “twice as loud”.

## Channels

- **Mono** = 1 stream of samples
- **Stereo** = 2 streams (left, right), usually **interleaved**: L R L R L R …

If you treat stereo as mono, you also get **2× speed** (you play left and right as one stream of “next samples”). Znicz must keep channel count matched too.

## PCM

**PCM** means the numbers *are* the waveform. WAV (uncompressed) and FLAC (compressed but lossless) both become PCM after decode. The player then sends PCM to the sound device.

## Compression (short)

| Kind | Idea | Example |
|------|------|---------|
| Lossless | Smaller file, **same** samples after decode | FLAC, ALAC |
| Lossy | Throws away hard-to-hear parts | MP3, AAC, Opus |
| Uncompressed | Raw PCM on disk | WAV, AIFF |

Audiophiles often prefer lossless so decode can return the original samples.

## Next

- [Audiophile ideas](Audiophile-Basics.md)
- [Playback pipeline](Playback-Pipeline.md)

## Extra reading

- [xiph.org wiki: Digital Audio](https://wiki.xiph.org/index.php/Digital_Audio)
- [Sample rate (Wikipedia)](https://en.wikipedia.org/wiki/Sampling_(signal_processing)#Audio_sampling)
