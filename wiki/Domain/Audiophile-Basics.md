# Audiophile ideas (simple)

“Audiophile” here means: **do not change the music unless we must**.

## Bit-perfect

**Bit-perfect** means the samples that leave the player are the same samples that were in the file (after lossless decode). No extra volume math, no sample-rate conversion, no mixer “helpful” resampling.

In practice this is hard:

- Linux often routes audio through **PipeWire** or **PulseAudio**. Those layers may resample to 48 kHz.
- Windows **WASAPI shared** mode lets the OS mixer run. Exclusive / ASIO is closer to bit-perfect (planned later).
- Opening the **hardware device** (`hw:CARD=…` on ALSA) skips some mixing.

Znicz Phase 1 tries to **open the output at the file’s sample rate**. That is the main speed and quality rule.

## Why resampling is a big deal

Resampling = “this file is 44.1 kHz, the device wants 48 kHz, so invent extra samples.”

A good resampler is OK for casual listening. A bad one smears transients. For “quality first”, we prefer:

1. Ask the device to run at the file rate
2. Only resample if the device cannot do that (not built yet)

## DAC

A **DAC** (digital-to-analog converter) turns numbers into voltage. USB DACs often accept several rates (44.1, 48, 96…). If the OS already converted to 48 kHz, the DAC never sees the original 44.1 kHz stream.

## Gapless

Some albums have tracks that should flow with **no silence** between them. The encoder may add a little padding. Symphonia can strip that when gapless mode is on. Znicz enables that in the decoder.

## Volume

Software volume multiplies samples by a number between 0 and 1. That is **not** bit-perfect. For critical listening, set player volume to 1.0 and use a hardware knob if you can.

## Next

- [Playback pipeline](Playback-Pipeline.md)

## Extra reading

- [What is a DAC?](https://en.wikipedia.org/wiki/Digital-to-analog_converter)
- [PipeWire](https://pipewire.org/)
- [WASAPI](https://learn.microsoft.com/en-us/windows/win32/coreaudio/wasapi)
