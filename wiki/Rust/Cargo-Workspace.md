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

## Releasing

Release builds are made by [dist](https://github.com/axodotdev/cargo-dist)
(`dist-workspace.toml`, generated workflow `.github/workflows/release.yml`).

1. Bump `[workspace.package] version` and merge to `main` as usual.
2. Tag that commit and push the tag: `git tag v0.13.1 && git push origin v0.13.1`.
3. The workflow builds and publishes a GitHub Release with:
   - `znicz-x86_64-unknown-linux-gnu.tar.xz` (built on Ubuntu 22.04, so glibc 2.35+),
   - `znicz-aarch64-unknown-linux-gnu.tar.xz` (native ARM runner),
   - `znicz-x86_64-pc-windows-msvc.zip` (C runtime linked statically, no DLLs),
   - a `.sha256` per archive and `sha256.sum`,
   - `znicz-installer.sh` and `znicz-installer.ps1`, which download the right
     archive, verify it, and install to `~/.local/bin` (added to `PATH`).

Archives hold the binary, `README.md`, both licence files and
`THIRD-PARTY-LICENSES.html`. They are built with
`[profile.dist]`, which inherits `[profile.release]` (thin LTO, one codegen unit,
stripped symbols): about 15 MB unpacked, 5 MB packed. The Linux binary links only
glibc and `libasound.so.2`.

**Third-party licences.** `THIRD-PARTY-LICENSES.html` lists every crate compiled
into the release binary (dev and build dependencies excluded, the three release
targets only) with its licence text. [cargo-about](https://github.com/EmbarkStudios/cargo-about)
builds it from `Cargo.lock` with `about.toml` (accepted licences) and `about.hbs`
(the page). After any dependency change run `scripts/third-party-licenses.sh`
and commit the file; the CI lint job regenerates it and fails when it differs.
A new licence that is not in `accepted` fails the script until it is reviewed
and added. Symphonia is MPL-2.0: fine to ship unmodified in an MIT / Apache
binary, as long as the page points at its source, which it does.

After changing `dist-workspace.toml` or upgrading dist, run `dist generate`
(never edit `release.yml` by hand) and `dist plan` to see what a tag would
publish. `dist build --artifacts=local` builds the archive for this machine
into `target/distrib/`.

## Edition

We use **edition 2021** so we stay compatible with current cpal/rmcp crates. Edition is not the same as compiler version. The compiler is pinned in `rust-toolchain.toml`.

## Extra reading

- [Cargo book: workspaces](https://doc.rust-lang.org/cargo/reference/workspaces.html)
- [Edition guide](https://doc.rust-lang.org/edition-guide/)
