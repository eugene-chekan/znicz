# GitHub Actions CI

**Date:** 2026-08-31
**Status:** Approved
**Crates:** workspace (no crate API change)

## Problem

Pushes and pull requests are not checked automatically. A Windows or Clippy
break can land on `main` unnoticed.

## Goals

1. On every push to `main` and every pull request: format, Clippy, and tests.
2. Tests run on **Linux and Windows**, matching the README.
3. No GitHub Releases, no crates.io publish.

## Non-goals

- Tag-based binary uploads
- crates.io
- macOS
- A dummy sound device so playback tests run on the runner
- Version bump (this is not player behaviour)

## Workflow

One file: `.github/workflows/ci.yml`.

| Job | Runner | Commands |
| --- | --- | --- |
| `lint` | `ubuntu-latest` | `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` |
| `test` | `ubuntu-latest` and `windows-latest` | `cargo test --workspace` (Windows: `--test-threads=1`) |

Linux jobs install `pkg-config` and `libasound2-dev` so cpal can compile.
Windows tests use one thread: WASAPI is not safe from cargo's parallel test
pool (`App` lists devices as it starts). Hardware playback tests skip when
`CI` is set. Playback tests already skip when there is no output device.

`permissions: contents: read`. A new commit on the same branch cancels the
older run. Cargo is cached with `Swatinem/rust-cache`.

## Toolchain

`rust-toolchain.toml` stays the source of truth (`stable`, min 1.85) and lists
`clippy` and `rustfmt` so local and CI match.
