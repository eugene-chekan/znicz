# Update notification and in-app update (#83): Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The TUI tells the user about a new release once a day at most, shows its notes, installs it through dist's updater, and restarts itself (and the player when asked) on the new version.

**Architecture:** `znicz-tui` defines an `Updates` trait and the UI (toast, Help title, `u`, `Modal::Update`, `RunOutcome`). The `znicz` binary implements it (`GithubUpdates`: ureq for the release check, axoupdater for the install), owns the daily check state, the `znicz update` CLI, the restart (Unix process replacement / Windows spawn+wait) and the hidden `--resume`. The IPC advert carries the player's version.

**Tech Stack:** Rust 2024, ratatui 0.30, ureq 3, axoupdater 0.10 (`blocking`), semver 1, toml/serde.

Spec: `docs/superpowers/specs/2026-11-11-update-notification-design.md` — read it; this plan does not repeat every string.
Branch: `feat/update-notification`. Every task starts with `git branch --show-current` and stops if it is not that branch.

Checks before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

---

### Task 1: Player version in the IPC advert; shutdown through `Engine`

**Files:** `znicz-core/src/player/ipc.rs` (`struct Advertise` ~line 31, where it is written ~303 and ~968, `IpcClient` ~685–800), `znicz-tui/src/app.rs` (`Engine` ~line 80).

- [x] **Step 1: Tests** in `ipc.rs` tests:
  - an advert TOML without `version` (`port = 1\ntoken = "x"\n`) deserialises with `version == None`.
  - a server started like the existing tests (find one that uses `advertise_path()` and `IpcServer::start…`) writes an advert whose `version` is `Some(env!("CARGO_PKG_VERSION"))`, and `IpcClient::connect(...)?.player_version()` returns the same.
- [x] **Step 2:** run; fails.
- [x] **Step 3: Implement.** `Advertise { port, token, #[serde(default)] version: Option<String> }`; every place that builds an `Advertise` for writing sets `version: Some(env!("CARGO_PKG_VERSION").to_string())`. `IpcClient` keeps the version read from the advert when it connects / reconnects and exposes `pub fn player_version(&self) -> Option<String>`. In `znicz-tui` `Engine` add:

```rust
/// The player's version, when it advertises one (remote players only).
pub fn player_version(&self) -> Option<String> {
    match self {
        Self::Local(_) => None,
        Self::Remote(player) => player.player_version(),
    }
}

/// Ask a remote player process to exit. The local engine has no process.
pub fn shutdown_player(&self) -> znicz_core::Result<()> {
    match self {
        Self::Local(_) => Ok(()),
        Self::Remote(player) => player.shutdown(),
    }
}
```

- [x] **Step 4:** checks pass.
- [x] **Step 5: Commit** `"The player advertises its version"`.

---

### Task 2: `Updates` trait, toast, Help title and the `u` key

**Files:** new `znicz-tui/src/update.rs`, `znicz-tui/src/lib.rs` (module + `pub use update::{Release, Updates};`), `znicz-tui/src/app.rs`, `znicz-tui/src/views/help.rs` (`version_title` ~line 123), `znicz-tui/src/keys.rs` (help rows).

`update.rs`:

```rust
//! What the TUI needs from the updater; the binary provides the real one.

/// A release newer than this build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    /// Release notes, already trimmed of the installer section.
    pub notes: String,
    pub url: String,
}

pub trait Updates: Send + Sync {
    /// The newer release, from the daily check (may use the network; called
    /// on a background thread). `None` when up to date or unknown.
    fn latest(&self) -> Option<Release>;
    /// Whether this binary is the installer's copy and can replace itself.
    fn can_install(&self) -> bool;
    /// Install `version` (blocking; called on a background thread).
    fn install(&self, version: &str) -> Result<(), String>;
    /// How to update when `can_install` is false.
    fn hint(&self) -> String;
    /// The version shown last in a toast, and recording a new one.
    fn notified(&self) -> Option<String>;
    fn set_notified(&self, version: &str);
}
```

App state (in `app.rs`): `pub updates: Option<Arc<dyn Updates>>`, `pub update: Option<Release>` (the known newer release), a receiver for the background check result. `pub fn with_updates(mut self, updates: Arc<dyn Updates>) -> Self` stores it; `run()` (before the loop) spawns a thread that calls `latest()` and sends the result over an `mpsc` channel. Each loop iteration (`poll_player_events` area) does `try_recv`; on `Some(release)`: set `self.update`, and if `updates.notified() != Some(release.version)`, toast info `Znicz {v} is available · ? then u to update` and `set_notified(v)`.

Help: `version_title(popup_width, hint)` gets the update version: ` znicz 0.13.4 → 0.14.0 ` when `app.update` is `Some` (keep the existing "drop when it does not fit" rule; pass `Option<&str>`). In `on_help_key` (not editing, no modifiers): `KeyCode::Char('u') if self.update.is_some() => self.open_update()` where `open_update` for now just sets `self.modal = Modal::Update` (Task 3 fills it in; add the `Modal::Update` variant now with a placeholder render that draws an empty bordered modal titled `Update`, and `Esc` closes it). Help rows: `keys.rs` gets a row `u` / `update to the new version` in the Help context section, shown only when an update is known — if `help_sections` cannot see the app, add the row unconditionally under a heading that says "when an update is available" (pick the simpler; note it in the report).

- [x] **Step 1: Tests** (use the existing App test helpers in `app.rs` tests / `znicz-tui/tests/keys.rs` — read them first; build an `App` with a local player). A `FakeUpdates` (in the test module) returning a fixed `Release { version: "9.9.9", .. }`, `can_install` configurable, `install` recording calls and returning a configurable result, `notified`/`set_notified` in a `Mutex<Option<String>>`.
  - after `with_updates(fake)` and one pump of the check result (expose a `pub(crate) fn poll_updates(&mut self)` and a test hook that runs the check synchronously, e.g. `pub(crate) fn check_updates_now(&mut self)`), `app.update` is `Some`, a toast with `9.9.9 is available` exists, `fake.notified() == Some("9.9.9")`.
  - a second app with the fake already notified for `9.9.9`: no toast.
  - `version_title(80, hint, Some("9.9.9"))` text is ` znicz {CARGO_PKG_VERSION} → 9.9.9 `.
  - Help open + `u` with an update → `modal == Modal::Update`; without an update → stays Help.
- [x] **Step 2:** run; fails.
- [x] **Step 3:** implement.
- [x] **Step 4:** checks pass.
- [x] **Step 5: Commit** `"Tell the TUI user about a new release"`.

---

### Task 3: The Update modal, install, and `RunOutcome`

**Files:** `znicz-tui/src/app.rs`, new `znicz-tui/src/views/update.rs` (+ `views/mod.rs` dispatch next to the other modals), `znicz-tui/src/lib.rs` (`pub use app::{RunOutcome, Resume};`).

Types:

```rust
/// How the TUI ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    Quit,
    /// Start the (updated) binary again; `resume` replays where the player was.
    Restart { resume: Option<Resume> },
}

/// Where to pick playback up after a restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resume {
    pub index: usize,
    pub seconds: u64,
}

pub(crate) enum UpdateStep {
    Offer,
    NotInstallable,
    Installing,
    Failed(String),
    Done,
    PlayerPlaying,
}
```

App gets `update_step: UpdateStep`, `update_scroll: usize`, an install result receiver, and `outcome: RunOutcome` (default `Quit`). `run()` returns `color_eyre::Result<RunOutcome>`; the loop ends on `should_quit` and returns `self.outcome.clone()`. Update `znicz/src/main.rs` `run_tui_with_client` to compile (`let outcome = app.run()?;` and for now treat any outcome as quit; Task 5 handles restart).

Behaviour (spec "Update modal" and "Restart"):
- `open_update`: `update_step = if can_install { Offer } else { NotInstallable }`, scroll 0, `modal = Modal::Update`.
- Keys in `Modal::Update`:
  - Offer: `Enter` → `Installing`, spawn a thread calling `updates.install(&version)` and sending the result; `Esc`/`q` close; `j`/`k`/arrows/PgUp/PgDn scroll the notes.
  - NotInstallable / Failed: `Esc`/`q`/`Enter` close; scroll keys scroll.
  - Installing: everything ignored except Ctrl-C (quit).
  - Done: `Enter` → if `state.status == Stopped` or the queue is empty: `player.shutdown_player()` (log a warning on error), `outcome = Restart { resume: None }`, `should_quit = true`; else → `PlayerPlaying`. `Esc` closes the modal (the update stays installed; `self.update = None` so Help shows the plain version).
  - PlayerPlaying: `Enter` → `resume = Resume { index: state.queue_position, seconds: if the current queue item is a `QueueItem::File` { state.position.as_secs() } else { 0 } }`, `shutdown_player()`, `outcome = Restart { resume: Some(resume) }`, quit. `Esc` → `outcome = Restart { resume: None }`, quit (player untouched).
- Each loop iteration polls the install receiver: `Ok(())` → `Done`, `Err(e)` → `Failed(e)`.
- Render (`views/update.rs`, modelled on `views/help.rs` / `views/messages.rs` modal code: `crate::layout::modal_rect`, `Clear`, bordered block, `theme::border_active`, close button hit rect): title `Update to {version}`; body by step:
  - Offer: notes (wrapped, scrolled); bottom hint ` Enter update · Esc cancel `.
  - NotInstallable: notes, blank line, `This copy was not installed by the Znicz installer.`, `updates.hint()`; hint ` Esc close `.
  - Installing: `Updating to {version}…`; no hint.
  - Failed: `Update failed: {e}`; hint ` Esc close `.
  - Done: `Updated to {version}. Restart Znicz to use it.`; hint ` Enter restart now · Esc later `.
  - PlayerPlaying: `The player is playing. Restart it too? Playback resumes in a moment.`; hint ` Enter restart player · Esc keep it playing `.

- [ ] **Step 1: Tests** with `FakeUpdates` from Task 2 (prefer unit tests inside `app.rs`; integration tests only see `pub` items):
  - Offer + Enter → Installing; after polling the install result `Ok` → Done; fake recorded `install("9.9.9")`.
  - install `Err("boom")` → `Failed("boom")`, rendered text contains `boom`.
  - not installable: Enter does nothing (step stays NotInstallable), render contains the hint.
  - Done + Enter with a stopped local player → `should_quit`, `outcome == Restart { resume: None }`.
  - The playing branch: unit-test the decision as a pure fn `fn restart_step(status: PlaybackStatus, queue_len: usize) -> bool` (true = ask about the player) and test Stopped/empty/Playing/Paused; and test that PlayerPlaying + Enter gives `Restart { resume: Some(Resume { index: queue_position, .. }) }` and Esc gives `Restart { resume: None }` (set `update_step = PlayerPlaying` directly).
- [ ] **Step 2:** run; fails.
- [ ] **Step 3:** implement.
- [ ] **Step 4:** checks pass.
- [ ] **Step 5: Commit** `"Update modal: install the new release and offer a restart"`.

---

### Task 4: `GithubUpdates`, check state, and `znicz update`

**Files:** new `znicz/src/update.rs`, `znicz/src/main.rs` (mod, `Commands::Update { #[arg(long)] check: bool }`, `[updates]` config section `UpdatesSection { check: Option<bool> }`), `znicz/Cargo.toml` (`axoupdater = { version = "0.10", default-features = false, features = ["github_releases", "blocking"] }`, `semver = "1"`, `ureq` / `serde_json` / `toml` / `serde` as needed — copy versions from other crates' `Cargo.toml` or `[workspace.dependencies]`).

Pure helpers (all `pub(crate)`, unit-tested):

```rust
pub(crate) const API_URL: &str = "https://api.github.com/repos/eugene-chekan/znicz/releases/latest";
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// `latest` is a release (no pre-release part) newer than `current`.
pub(crate) fn is_newer(current: &str, latest: &str) -> bool
/// The body up to dist's `## Install znicz` section, trimmed; empty → "No release notes."
pub(crate) fn release_notes(body: &str) -> String
/// `v0.14.0` → `0.14.0`.
pub(crate) fn version_from_tag(tag: &str) -> &str
/// The hint for a copy the installer did not make.
pub(crate) fn update_hint(exe: &Path) -> String
```

`update_hint`: if any path component is `target` → `Update the checkout and rebuild: git pull && cargo build --release`; else on Windows `Run the installer again: powershell -ExecutionPolicy Bypass -c "irm https://github.com/eugene-chekan/znicz/releases/latest/download/znicz-installer.ps1 | iex"`, elsewhere `Run the installer again: curl --proto '=https' --tlsv1.2 -LsSf https://github.com/eugene-chekan/znicz/releases/latest/download/znicz-installer.sh | sh`.

`CheckState` (`#[derive(Serialize, Deserialize, Default)]`, all fields `#[serde(default)]`): `checked_at: u64`, `latest: Option<String>`, `notes: Option<String>`, `url: Option<String>`, `notified: Option<String>`; `load(path) -> Self` (missing / bad file → default), `save(&self, path) -> io::Result<()>` (create parent dir), `due(&self, now_secs: u64) -> bool` (`now_secs.saturating_sub(checked_at) >= CHECK_EVERY.as_secs()`).

`fetch_latest_at(api_url) -> Result<(String version, String notes, String url), String>`: ureq GET with header `User-Agent: znicz/<version>` and `Accept: application/vnd.github+json`, 10 s timeout; parse `tag_name`, `body` (may be null → ""), `html_url`.

`GithubUpdates { state_path: PathBuf, check: bool, api_url: String, state: Mutex<CheckState> }`:
- `latest()`: if `!check` → None. If `state.due(now)` → `fetch_latest_at`; on Ok store `latest/notes/url/checked_at=now` and save; on Err `tracing::debug!` and keep state. Then return `Some(Release)` when `state.latest` is newer than `CARGO_PKG_VERSION`, else None.
- `can_install()`: `AxoUpdater::new_for("znicz")`, `load_receipt()` ok and `check_receipt_is_for_this_executable()` == Ok(true).
- `install(v)`: `AxoUpdater::new_for("znicz")`, `load_receipt()`, `disable_installer_output()`, `configure_version_specifier(UpdateRequest::SpecificVersion(v.into()))`, `always_update(true)`, `run_sync()`; map errors with `.to_string()`.
- `hint()`: `update_hint(&std::env::current_exe().unwrap_or_default())`.
- `notified()` / `set_notified(v)`: from / into the state, saved.
- State path: `znicz_library::default_session_path()` with the file name replaced by `update-check.toml` (`None` → disable checks).

CLI `znicz update [--check]` (spec "CLI"): `--check` forces a fetch (ignore `due`), saves, prints `znicz {current}, latest {v}` + notes, or `znicz {current} is up to date`. Without `--check`: fetch latest; if not newer print up to date; if not `can_install` print the hint and exit with an error; else install with installer output **enabled** and print the spec's message.

- [ ] **Step 1: Tests** in `update.rs`: `is_newer` (`0.13.4`/`0.14.0` true; equal false; older false; `0.15.0-rc.1` false; garbage false); `release_notes` (body with notes + `## Install znicz 0.14.0` section → notes only; body that starts with the install section → `No release notes.`; empty → same); `version_from_tag`; `update_hint` for `/home/x/znicz/target/release/znicz` vs `/home/x/.local/bin/znicz`; `CheckState::due` (0 → due; now-1 h → not due; now-25 h → due); `CheckState` save/load round trip in a `tempfile::tempdir()`; `fetch_latest_at` against a local `TcpListener` server (copy the tiny HTTP server pattern from `znicz-core/src/audioaddict.rs` tests): 200 with `{"tag_name":"v0.14.0","body":"Fixes\n\n## Install znicz 0.14.0\n…","html_url":"https://x"}` → `("0.14.0","Fixes","https://x")`; 404 → Err; `not json` → Err. `GithubUpdates::latest` with a server, a fresh state file and `check=true` → `Some(Release{version:"99.0.0"..})` when the server says `v99.0.0`, and the state file now has `checked_at > 0`; with `check=false` → None and no request.
- [ ] **Step 2:** run; fails.
- [ ] **Step 3:** implement. Do not wire into the TUI yet (Task 5).
- [ ] **Step 4:** checks pass; also `cargo run -q -- update --check` once by hand and paste its output in the report (needs network; if offline, say so).
- [ ] **Step 5: Commit** `"znicz update: check GitHub releases and install through the dist receipt"`.

---

### Task 5: Wire updates into the TUI; restart and `--resume`; version mismatch toast

**Files:** `znicz/src/main.rs` (`Cli`, `run_tui_with_client` ~line 675 and its callers, `main`).

- `Cli` gains `#[arg(long, hide = true, value_name = "INDEX:SECONDS")] resume: Option<String>`. `pub(crate) fn parse_resume(s: &str) -> Option<Resume>` (`"3:125"` → index 3, seconds 125; anything else → None + `tracing::warn!`).
- `run_tui_with_client` takes the `GithubUpdates` (built in `main` from config `[updates] check`, default true) as `Option<Arc<dyn Updates>>` and the parsed `Option<Resume>`; before `app.run()`: `app = app.with_updates(updates)` when Some; if `app.player.player_version()` is `Some(v)` and `v != env!("CARGO_PKG_VERSION")` → `app.toasts.info(format!("Player runs {v} · it updates when it next restarts"))`; if resume is Some → `apply_resume(&app.player, resume)`:

```rust
/// After a restart: wait for the restored queue, then play where we were.
fn apply_resume(player: &Engine, resume: Resume) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while player.state().queue.len() <= resume.index {
        if Instant::now() >= deadline {
            tracing::warn!("resume: queue not restored in time");
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = player.send_blocking(Command::QueuePlayIndex(resume.index));
    if resume.seconds > 0 {
        let _ = player.send_blocking(Command::Seek(Duration::from_secs(resume.seconds)));
    }
}
```

- After `app.run()` returns (the terminal is already restored by `App::run`, and drop the stderr redirect first), on `RunOutcome::Restart { resume }` call `restart(resume)`:
  - `exe = live_exe_path(std::env::current_exe()?)`; `args = restart_args(std::env::args_os().skip(1), resume)`; build `std::process::Command::new(exe).args(args)`.
  - Unix: replace this process with the new one through `std::os::unix::process::CommandExt` (its process-replacing method takes no shell and only returns on failure; return that error).
  - Windows: `let status = cmd.status()?; std::process::exit(status.code().unwrap_or(0));`.
- `pub(crate) fn restart_args(args, resume) -> Vec<OsString>`: drops any existing `--resume X` / `--resume=X`, appends `--resume {index}:{seconds}` when given.
- `pub(crate) fn live_exe_path(p: PathBuf) -> PathBuf`: on Linux `current_exe()` reads `/proc/self/exe`, which after the installer replaced the file names the old inode as `… (deleted)`; strip a trailing ` (deleted)`.

- [ ] **Step 1: Tests:** `parse_resume` (`3:125`, `0:0`, `x`, `3:`, `:5`), `restart_args` (no resume; with resume; replaces an old `--resume 1:2` and `--resume=1:2`; keeps other args and their order), `live_exe_path` (`/a/znicz (deleted)` → `/a/znicz`; plain path unchanged).
- [ ] **Step 2:** run; fails.
- [ ] **Step 3:** implement.
- [ ] **Step 4:** checks pass.
- [ ] **Step 5: Commit** `"Restart on the updated binary and resume playback"`.

---

### Task 6: Docs, changelog, version (controller)

- [ ] `CHANGELOG.md` with `## 0.14.0 - 2026-11-11` (this feature) and short entries for 0.13.1–0.13.4 from `git log`.
- [ ] Wiki: new `wiki/Architecture/Updates.md` (check, state file, notification, modal steps, restart and `--resume`, player version, CLI, axoupdater and the receipt), link from `_Sidebar.md` and `Architecture/Overview.md`; `TUI.md` (Help `u`, Update modal); README config (`[updates] check = true`) and install section (`znicz update`).
- [ ] Version `0.14.0`; `scripts/third-party-licenses.sh` (new licences: add to `about.toml` `accepted` only if permissive; report them).
- [ ] Checks; commit `"Document updates; changelog; bump to 0.14.0"`.
