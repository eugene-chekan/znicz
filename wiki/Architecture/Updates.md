# Updates

How Znicz finds a new release and replaces itself
([#83](https://github.com/eugene-chekan/znicz/issues/83)).

## Checking

The TUI starts a background thread that asks GitHub for the latest release
(`api.github.com/repos/eugene-chekan/znicz/releases/latest`, plain `ureq`) at
most once every 24 hours. The answer is kept in `update-check.toml` next to
`session.toml`: when it checked, the latest version, its notes and page, and
the last version a toast was shown for. Starts in between read that file and
make no request. Network errors, rate limits and odd answers go to the debug
log and change nothing; the next start tries again.

A release counts when its tag (`v0.14.0`) is a plain semver version, without a
pre-release part, newer than the running build.

The notes are the release body up to dist's `## Install znicz …` section.
dist fills that body from `CHANGELOG.md`.

`[updates] check = false` in `config.toml` turns the check off. `znicz update`
and `znicz update --check` always ask.

## Telling the user

- One toast per new version: `Znicz 0.14.0 is available · ? then u to update`.
- The footer always shows the running version (`? more · v0.16.0`). While an
  update is known it reads `? more · v0.16.0 → 0.17.0 available`, highlighted,
  so the news outlasts the toast; with less room `v0.16.0 → 0.17.0`, and on
  narrow terminals hints give way to it.
- `u` in Help opens the **Update** modal. The `u` row is always listed in Help as
  `check for updates`.
- `u` in Help with no update known checks right away, ignoring the daily limit
  and `[updates] check = false`. `Checking for updates…` is shown meanwhile,
  and a second `u` does not start another check. The outcomes:
  - a newer release: the Update modal opens (or, when another modal is open,
    the `is available` toast);
  - up to date: `Znicz 0.14.1 is up to date` (success toast);
  - an error: `Update check failed: …` (error toast).

## The Update modal

| Step | Shows | Keys |
|------|-------|------|
| Offer | the notes | `Enter` update · `Esc` cancel · `j` `k` / wheel scroll |
| Not installable | the notes, why, and how to update by hand | `Esc` close |
| Installing | `Updating to 0.14.0…` | none (the close button and Ctrl-C are ignored) |
| Failed | the installer's last lines | `Esc` close |
| Done | `Updated to 0.14.0. Restart Znicz to use it.` | `Enter` restart now · `Esc` later |
| Player playing | `Restart it too? Playback resumes in a moment.` | `Enter` restart player · `Esc` keep it playing |

An install that ends while the modal is closed is announced in a toast;
reopening the modal shows where it got to.

## Installing

Only the copy the release installer made can replace itself. The installers
write a receipt (`$XDG_CONFIG_HOME/znicz/znicz-receipt.json`, otherwise
`~/.config/znicz/` or `%LOCALAPPDATA%\znicz\`) naming the install folder. The
running binary must sit in that folder. A cargo build (`target/debug`,
`target/release`) is told to `git pull && cargo build --release`; any other
copy is shown the installer command.

To install version `v`, Znicz downloads that release's `znicz-installer.sh`
(`.ps1` on Windows) and runs it with `sh` (`powershell -ExecutionPolicy
Bypass -File`), with `ZNICZ_INSTALL_DIR` set to the receipt's folder and
`ZNICZ_NO_MODIFY_PATH=1`. On Windows `PSModulePath` is removed from the
installer's environment: a PowerShell 7 terminal sets it to its own modules,
which Windows PowerShell 5.1 cannot load
([PowerShell#18530](https://github.com/PowerShell/PowerShell/issues/18530)). That is the same script as the README's
`curl … | sh`, fetched over HTTPS from GitHub; there is no separate signature
check. dist's own updater library (axoupdater) does the same steps but added
4.6 MB and a C crypto build, so Znicz does them itself.

**Windows** cannot overwrite a running `.exe`, but it can rename one. Before
the installer runs, `znicz.exe` is renamed to `znicz.exe.old-<millis>`; if the
installer fails it is renamed back. Every start deletes the `znicz.exe.old-*`
files it can (one an old player still runs stays until that player exits).

## Restarting

`App::run` returns `RunOutcome::Quit` or `RunOutcome::Restart { resume }`.

- **Done → Enter** with a stopped player (or an empty queue): the TUI asks the
  player to exit and restarts.
- **Player playing → Enter**: the TUI records the queue row and, for a local
  file, the position; asks the player to exit; restarts with the hidden
  `--resume <row>:<seconds>`. The new TUI autostarts the new player (which
  restores the session queue), plays that row and seeks.
- **Player playing → Esc**: restarts the TUI only; the player moves to the new
  version at its next idle exit.

The restart runs the binary at its install path (on Linux `current_exe()`
names the replaced file `… (deleted)`; that suffix is dropped) with only the
global `--config` / `--device` options and `--resume`, so the first start's
files, playlist or station are not queued again. Unix replaces the process;
Windows runs the new one in the same console and exits with its status.
Before that, it waits up to 5 s for a player that was just told to exit to
release `player.lock`.

## Player version

The player writes its version into `ipc.toml`. When a TUI finds a player
older than itself it shows `Player runs 0.13.4 · it updates when it next
restarts`. MCP servers that an agent started keep their version until the
agent restarts them.

## Code

- `znicz-tui/src/update.rs`: `Release`, the `Updates` trait.
- `znicz-tui/src/app.rs`: the check and install threads, `UpdateStep`,
  `RunOutcome`, `Resume`; `views/update.rs` draws the modal.
- `znicz/src/update.rs`: `GithubUpdates`, `CheckState`, `znicz update`;
  `znicz/src/update/installer.rs`: receipt, download, running the installer,
  the Windows rename.
- `znicz/src/main.rs`: `--resume`, `restart`; `daemon.rs`:
  `wait_for_player_exit`, `live_exe_path`.
