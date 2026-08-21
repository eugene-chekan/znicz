---
name: audiophile-playback
description: Configure and troubleshoot audiophile-quality playback in znicz — bit-perfect output, sample rate matching, format selection, and DAC setup on Linux and Windows.
---

# Audiophile Playback

## Goals

- Match output sample rate and format to source when possible
- Prefer direct device access for USB DACs
- Avoid unnecessary resampling

## Workflow

1. Call `list_devices` to enumerate outputs
2. Read `znicz://devices` for JSON device list
3. Use `set_device` with the target device id
4. Play lossless files (FLAC, ALAC, WAV) via `play`
5. Read `znicz://now-playing` to verify codec, rate, and bit depth
6. Load this skill when troubleshooting quality issues

## Linux notes

- ALSA `hw:X,Y` devices may offer more direct DAC access than `default`
- PipeWire/PulseAudio bridges may resample; prefer hardware devices when bit-perfect matters

## Windows notes

- WASAPI shared mode is the default via cpal
- ASIO support is planned for exclusive bit-perfect output
