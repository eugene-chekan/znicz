# Update notification and in-app update (#83)

**Date:** 2026-11-11
**Status:** Approved in chat (own installer runner instead of axoupdater, see "Installer runner"; restart the player if idle, else ask; notes in the prompt)
**Crates:** `znicz` (binary), `znicz-tui`, `znicz-core` (IPC advert)
**Version:** 0.14.0

## Checking

- The TUI checks at start, in a background thread, when `[updates] check`
  is not `false` (default `true`) and the last check is older than 24 h.
- Request: `GET https://api.github.com/repos/eugene-chekan/znicz/releases/latest`
  with `ureq` (already a dependency), `User-Agent: znicz/<version>`, 10 s
  timeout. Fields used: `tag_name` (`v0.14.0` → `0.14.0`), `body`, `html_url`.
- `latest` is newer when it parses as semver, has no pre-release part and is
  greater than `CARGO_PKG_VERSION`.
- State in `update-check.toml` next to `session.toml` (same data dir):
  `checked_at` (unix seconds), `latest`, `notes`, `url`, `notified`
  (the last version a toast was shown for). A start within 24 h uses the stored
  `latest` without a request. Any error (network, rate limit, parse) is logged
  at `debug` and leaves the state untouched; the next start tries again.
- Release notes: the body up to the first line starting with
  `## Install znicz` (dist's install section), trimmed. Empty → "No release
  notes." A new `CHANGELOG.md` (`## 0.14.0 - 2026-11-11` headings, dist's
  format) makes dist put real notes on the release page from 0.14.0 on.

## Notification

- When a newer version is known: once per version (`notified`), toast
  `Znicz 0.14.0 is available · ? then u to update`.
- Help's bottom-left version title becomes ` znicz 0.13.4 → 0.14.0 `, and in
  Help (not while filtering) `u` opens the **Update** modal. The Help key list
  gets a row `u  update to the new version` only while an update is known.

## Update modal (`Modal::Update`)

States:

1. **Offer**: title `Update to 0.14.0`; the notes, scrollable (`j`/`k`,
   arrows, PgUp/PgDn); bottom hint `Enter update · Esc cancel`.
2. **Not installable** (this binary is not the installer's copy:
   `check_receipt_is_for_this_executable` false, or no receipt): the notes,
   then `This copy was not installed by the Znicz installer.` and a hint:
   for a path containing `/target/` (`\target\`) `Update the checkout and
   rebuild: git pull && cargo build --release`; otherwise the installer
   command for the platform (the `curl … | sh` or `irm … | iex` line from the
   release). Hint `Esc close`; no Enter.
3. **Installing**: `Updating to 0.14.0…`; keys ignored except Ctrl-C. The
   install runs on a thread (axoupdater `run_sync`, installer output disabled,
   `SpecificVersion("0.14.0")`).
4. **Failed**: the error text; `Esc close`.
5. **Done**: `Updated to 0.14.0. Restart Znicz to use it.` Hint
   `Enter restart now · Esc later`.
6. **Player playing** (after Enter in Done, when the player's state is
   Playing or Paused): `The player is playing. Restart it too? Playback
   resumes in a moment.` Hint `Enter restart player · Esc keep it playing`.

## Restart

`App::run` returns a `RunOutcome`: `Quit` or `Restart { resume:
Option<Resume> }`.

- Done → Enter: if the player is Stopped (or the queue is empty), the TUI asks
  the player to shut down (`IpcClient::shutdown`) and returns
  `Restart { resume: None }`. If Playing / Paused, state 6.
- State 6 → Enter: record `Resume { index: queue_position, seconds:
  elapsed (local files only, else 0) }`, shut the player down, return
  `Restart { resume: Some(..) }`. Esc: return `Restart { resume: None }`
  without shutting the player down (it moves to the new version at its next
  idle exit).
- `main` after the TUI restored the terminal: re-run the current executable
  path (`std::env::current_exe`, which axoupdater replaced in place; on
  Windows the old file was renamed aside and the new one sits at the original
  path) with the original arguments minus any old `--resume`, plus
  `--resume <index>:<seconds>` when given. Unix: replace the process (`std::os::unix::process::CommandExt`). Windows:
  spawn with inherited stdio, wait, exit with its code.
- Hidden CLI flag `--resume <index>:<seconds>` (`#[arg(long, hide = true)]`):
  after connecting (the player autostarts and restores the session queue),
  wait up to 5 s for `queue.len() > index`, then `QueuePlayIndex(index)` and,
  if `seconds > 0`, `Seek(seconds)`. Parse errors are ignored with a warning.

## Player version

`ipc.toml` (`Advertise`) gains `version: Option<String>` (`#[serde(default)]`),
written by the player as `CARGO_PKG_VERSION`. `IpcClient::player_version()`
returns it. At TUI start, when it is `Some` and differs from the TUI's
version: toast `Player runs 0.13.4 · it updates when it next restarts`.

## CLI

- `znicz update --check`: print `znicz 0.13.4, latest 0.14.0` (or `… is up to
  date`) and the notes; ignores the 24 h throttle, saves the state.
- `znicz update`: if not the installer's copy, print the hint and exit 1.
  Otherwise install (installer output shown), then print `Updated to 0.14.0.
  A running player keeps the old version until it restarts (znicz player
  stop).`

## Code layout

- `znicz-tui/src/update.rs`: `Release { version, notes, url }`,
  `trait Updates: Send + Sync { fn latest(&self) -> Option<Release>; fn
  can_install(&self) -> bool; fn install(&self, version: &str) ->
  Result<(), String>; fn hint(&self) -> String; }` — `latest` returns the
  cached-or-fetched newer release (None when up to date or unknown).
  `App` holds `Option<Arc<dyn Updates>>` (`App::with_updates`).
- `znicz/src/update.rs`: `GithubUpdates` implementing it; pure helpers
  (`release_notes`, `is_newer`, `CheckState` load/save/`due`) with tests;
  `fetch_latest_at(api_url)` testable with a local `TcpListener` server.
- Dependencies: `axoupdater = { version = "0.10", default-features = false,
  features = ["github_releases", "blocking"] }` and `semver = "1"` in
  `znicz`; `semver` in `znicz-tui` if needed. Licences regenerated; add any
  new licence to `about.toml` `accepted` only if it is permissive.

## Testing

Pure: `is_newer`, `release_notes`, `CheckState::due`, `--resume` parsing,
`dev_build_hint` path detection. HTTP: `fetch_latest_at` against a local
server (200 with body, 404, garbage). TUI with a fake `Updates`: toast once
per version; Help title shows the arrow; `u` opens Update; Enter → Installing
→ Done; failed install → Failed; not installable → hint and Enter inert;
Done + Enter with a stopped player → `RunOutcome::Restart { resume: None }`.
IPC: an advert without `version` still parses; the player writes it.

## Docs

Wiki page `wiki/Guides/Updating.md` (or the existing guides folder),
`[updates] check` in the config reference, README install section mentions
`znicz update`, `CHANGELOG.md`.

## Installer runner (replaces axoupdater)

axoupdater added 4.6 MB (+31 %) to the release binary and ~91 crates,
including `aws-lc-sys` (a C build). Znicz does the same steps itself with
`ureq`:

- **Receipt** (written by the dist installers): `$XDG_CONFIG_HOME/znicz/znicz-receipt.json`
  when `XDG_CONFIG_HOME` is set; otherwise `~/.config/znicz/…` on Unix and
  `%LOCALAPPDATA%\znicz\…` on Windows. JSON fields used: `install_prefix`,
  `version`.
- **Installable** when the receipt parses and the canonical directory of
  `current_exe()` equals the canonical `install_prefix` (or
  `install_prefix/bin`).
- **Install `v`**: download
  `https://github.com/eugene-chekan/znicz/releases/download/v{v}/znicz-installer.{sh,ps1}`
  (cap 2 MB) into a temp dir; run `sh <file>` on Unix, or
  `powershell -NoProfile -ExecutionPolicy Bypass -File <file>` on Windows, with
  `ZNICZ_INSTALL_DIR=<install_prefix>` and `ZNICZ_NO_MODIFY_PATH=1`. TUI: output
  captured; on failure the last lines of stderr/stdout form the error. CLI:
  output shown.
- **Windows**: before running, rename the running `znicz.exe` to
  `znicz.exe.old` in the same folder (a running exe can be renamed but not
  overwritten); if the installer fails, rename it back. At every start,
  delete a leftover `znicz.exe.old` next to the exe (best effort; it fails
  while an old player still runs it, which is fine).
