# Cargo workspaces

This repo is a **workspace**. One `Cargo.toml` at the root lists members:

```toml
[workspace]
members = ["znicz-core", "znicz-library", "znicz-tui", "znicz-mcp", "znicz"]
```

Shared crate versions live in `[workspace.dependencies]`. Each member says `serde.workspace = true` instead of repeating version numbers. The app version is `[workspace.package] version` (currently **0.10.0**); every crate inherits it.

## Useful commands

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p znicz -- --list-devices
cargo run -p znicz -- path/to/file.flac
```

`target/` holds build output. Do not commit it (see `.gitignore`).

Cargo never deletes old build output. Each version bump and each change in
dependencies or features builds a fresh set of test binaries beside the old
ones, so `target/` keeps growing. Run `cargo clean` now and then; everything in
it rebuilds. To keep that growth down, the dev profile (used by `cargo build`
and `cargo test`) keeps only file / line debug info for workspace crates and
none for dependencies (`[profile.dev]` in the root `Cargo.toml`). Backtraces
still show files and lines; stepping through code in a debugger shows less.
That cut a full test build from 5.8 GB to 2.3 GB and its time from about
280 s to 145 s.

## CI

GitHub Actions (`.github/workflows/ci.yml`) runs on pushes to `main` and on
pull requests. A **lint** job on Ubuntu checks rustfmt and Clippy (`-D warnings`).
A **test** job runs `cargo test --workspace` on Ubuntu and Windows. Windows
uses `--test-threads=1` because WASAPI is not safe from cargo's parallel test
threads (`App` lists devices as it starts, except on CI). Linux installs `pkg-config` and
`libasound2-dev` so cpal can compile. Hardware playback tests skip on CI (Windows
runners still expose WASAPI) and when there is no sound device.

The compiler comes from `rust-toolchain.toml` (`stable`, minimum 1.85, plus
clippy and rustfmt).

## Edition

We use **edition 2021** so we stay compatible with current cpal/rmcp crates. Edition is not the same as compiler version. The compiler is pinned in `rust-toolchain.toml`.

## Extra reading

- [Cargo book: workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Edition guide](https://doc.rust-lang.org/edition-guide/)
