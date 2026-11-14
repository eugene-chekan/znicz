# Settings Screen Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Settings modal (`S`) in the TUI that shows and changes display and audio settings, manages music folders, and writes `config.toml` without losing the user's comments. A default `config.toml` is written on first start, and a broken config is reported instead of silently ignored (#6).

**Architecture:**
- `toml_edit` edits the config file in place (`ConfigFile`).
- A pure `settings` model lists the rows, their values and their explanations. The modal (`views/settings.rs` plus key handling in `app.rs`) applies each change to `App` state live, then saves it.
- The binary writes a default config from one template and passes the config path and any parse error into the TUI.
- Folder removal is a new `LibraryOp` that the player's library service runs.

**Tech Stack:** Rust 2024, ratatui, toml / toml_edit, rusqlite, clap.

Spec: `docs/superpowers/specs/2026-11-12-settings-screen-design.md` (read it — it holds the row table with exact labels and explanation texts).
Branch: `settings-screen-0.16.0`. Check `git branch --show-current` before every task.
Verification for every task: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

---

### Task 1: `ConfigFile` (toml_edit writer)

**Files:** Create `znicz-tui/src/config_file.rs` (register in `znicz-tui/src/lib.rs`, export `ConfigFile`); modify root `Cargo.toml` (`toml_edit = "0.22"` in `[workspace.dependencies]`, or the version matching the `toml` 0.8 family already in the lock — check `cargo tree -i toml_edit`), `znicz-tui/Cargo.toml` (`toml_edit.workspace = true`; `tempfile` dev-dependency if missing).

- [x] Tests first (`#[cfg(test)]`, `tempfile::TempDir`):
  - `set_keeps_comments_and_other_keys`: file `"# mine\n[tui]\nmarquee = true # scroll\nicons = \"emoji\"\n"`; `set_bool("tui","marquee",false)` → text contains `# mine`, `icons = "emoji"`, and `marquee = false`, and the inline comment `# scroll` stays.
  - `set_creates_file_dir_and_table`: path `dir/sub/config.toml` missing → `set_int("tui","cover_size",75)` → file parses (`toml::from_str::<toml::Value>`) with `tui.cover_size == 75`.
  - `remove_drops_only_the_key`: `[audio]\ndevice = "x"\nvolume = 1.0\n` → `remove("audio","device")` → no `device`, `volume` kept, `[audio]` kept. Removing a missing key is `Ok`.
  - `set_replaces_a_value_of_another_type`: `marquee = "yes"` → `set_bool` gives `marquee = false`.
  - `no_temp_file_left_behind`: after a set, the directory has only `config.toml`.
  - `broken_file_is_not_written`: file `"[tui\n"` → `set_bool` returns `Err` and the file text is unchanged.
- [x] Implement:

```rust
//! Edit `config.toml` in place: one key at a time, keeping the user's
//! comments, order and other keys.

use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, value};

#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: PathBuf,
}

impl ConfigFile {
    pub fn new(path: impl Into<PathBuf>) -> Self { Self { path: path.into() } }
    pub fn path(&self) -> &Path { &self.path }

    pub fn set_bool(&self, section: &str, key: &str, v: bool) -> Result<(), String> {
        self.edit(|t| { t[key] = value(v); })
    }
    pub fn set_int(&self, section: &str, key: &str, v: i64) -> Result<(), String> { /* same with value(v) */ }
    pub fn set_str(&self, section: &str, key: &str, v: &str) -> Result<(), String> { /* same */ }
    pub fn remove(&self, section: &str, key: &str) -> Result<(), String> { /* t.remove(key) */ }

    fn edit(&self, section: &str, change: impl FnOnce(&mut Table)) -> Result<(), String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.to_string()),
        };
        let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| e.to_string())?;
        let table = doc
            .entry(section)
            .or_insert(Item::Table(Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("[{section}] is not a table"))?;
        change(table);
        write_atomically(&self.path, &doc.to_string())
    }
}

/// Write next to the target, then rename over it.
pub fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| { let _ = std::fs::remove_file(&tmp); e.to_string() })
}
```

  (Fix the signature sketch: `edit` takes `section`; the setters pass it.)
- [x] Verify, commit `feat(tui): ConfigFile edits config.toml in place`.

### Task 2: Default config file, `znicz config`, parse errors

**Files:** Create `znicz/src/default-config.toml`; modify `znicz/src/main.rs` (`load_config`, startup, `Commands`), `znicz-tui/src/app.rs` (accept a startup error toast — see step 5).

- [x] Template `znicz/src/default-config.toml` (exact defaults from `Config`/`tui_config`/`AudioSection::default`/`idle_secs.unwrap_or(900)` — verify each in `main.rs` and use what the code uses):

```toml
# Znicz settings. Edit here or in the TUI (S).
# Reference: https://github.com/eugene-chekan/znicz#configuration

[audio]
# Output device id; leave unset for the system default (see `znicz devices`).
# device = "default"
volume = 1.0
# Open the device at each file's own sample rate and channels.
bit_perfect = true

[player]
# Seconds the player stays up while stopped with no UI; 0 never exits.
idle_secs = 900

[library]
# Where the library database lives; unset uses the user data directory.
# path = "~/.local/share/znicz/library.db"

[tui]
show_cover = true          # album art on the Stage
cover_size = 100           # Stage art size, % of the largest fit: 100 | 75 | 50 | 25
cover_protocol = "auto"    # auto | kitty | sixel | halfblocks | off
library_layout = "columns" # columns | tree
marquee = true             # scroll long now-playing titles; false cuts with …
icons = "unicode"          # unicode | emoji | nerd (needs a Nerd Font: see README "Nerd Font icons")

[covers]
search = true              # look radio covers up by artist / title (Deezer, iTunes, MusicBrainz)

[updates]
check = true               # ask GitHub for a new release once a day

[mcp]
skills_dirs = []
```

  `cover_size` is added to `TuiSection` in Task 4; until then parsing ignores it (serde ignores unknown fields unless `deny_unknown_fields` — check; if denied, add the field here instead).
- [x] Test `default_template_parses_to_the_defaults`: `toml::from_str::<Config>(include_str!("default-config.toml"))` → compare with `Config::default()` the effective values: `tui_config(..)` equal (derive/compare `TuiConfig`, it is `PartialEq`), `audio.volume`, `audio.bit_perfect`, `audio.device` (None), `player.idle_secs.unwrap_or(900)`, `covers.search.unwrap_or(true)`, `updates.check.unwrap_or(true)`, `mcp.skills_dirs` empty, `library.path` None.
- [x] `load_config` returns `(Config, Option<ConfigProblem>)` where `ConfigProblem { path: PathBuf, message: String }` and `message` is `"line L, column C: <toml message>"` (use `toml::de::Error::span()` and count lines/columns in the text; 1-based). Unreadable file → problem with the io error. Missing file → no problem. Log `tracing::warn!` on a problem. Tests: broken text gives line/column; good text gives none.
- [x] First start: `fn ensure_default_config(cli_config: Option<&Path>) -> Option<PathBuf>`: if `cli_config` is `Some` → do nothing; else default path; if it does not exist → `write_atomically(path, TEMPLATE)` (reuse a small local copy of the helper or depend on `znicz_tui::config_file::write_atomically`), `tracing::info!("Created {} with the default settings", path.display())`; on error `warn`. Call it at the top of `main` before `load_config`. Tests with a temp dir (make the function take the target path explicitly for testability: `ensure_default_config_at(path: &Path) -> std::io::Result<bool>`): writes when missing, leaves an existing file byte-identical.
- [x] `Commands::Config { #[command(subcommand)] action: ConfigAction }` with `ConfigAction::Init { #[arg(long)] force: bool }` and `ConfigAction::Path`. `init`: target = `--config` path or default; exists and no `--force` → print `<path> already exists (use --force to overwrite)` and exit 1; with `--force` copy to `<path>.bak` (`config.toml.bak`) then write; print `Wrote <path>`. `path` prints the effective path. Tests for the pure part (`init_config(path, force) -> Result<InitOutcome, String>`).
- [x] Pass into the TUI: the effective config path (`--config` or default) as `ConfigFile` and the `ConfigProblem` (as a message string) — add to `TuiExtras` and on to `App` via builder methods `App::with_config_file(ConfigFile)` and `App::with_config_problem(String)`. At TUI start a problem shows `toasts.error(format!("config.toml has an error ({msg}). Using the defaults."))`. Store `config_broken: bool` on App for Task 6.
- [x] Verify, commit `feat: default config.toml on first start, znicz config, report config errors`.

### Task 3: Remove a music folder

**Files:** `znicz-library/src/store.rs`, `znicz-core/src/library_host.rs`, `znicz-library/src/service.rs` (the job runner that handles `LibraryOp`), any `match` on `LibraryOp` the compiler finds (MCP, CLI).

- [x] Tests in store.rs: `remove_folder_drops_its_tracks_and_forgets_it`: folders `/music` and `/music2` remembered, tracks `/music/a.flac`, `/music/sub/b.flac`, `/music2/c.flac` → `remove_folder(Path::new("/music"))` returns 2; `folders()` is `[/music2]`; only `/music2/c.flac` left. Also a trailing-slash input `/music/` behaves the same. On Windows-style paths the separator is `std::path::MAIN_SEPARATOR` — build the prefix as `format!("{}{}", folder.display(), MAIN_SEPARATOR)` after trimming trailing separators, and match `path = folder OR path LIKE prefix || '%' ESCAPE '\'` with `%`, `_` and `\` in the prefix escaped.
- [x] Implement `pub fn remove_folder(&mut self, folder: &Path) -> Result<usize>` in one transaction (delete tracks, delete the `folders` row).
- [x] `LibraryOp::RemoveFolder { path: PathBuf }` (+ `failure_text` arm: `format!("could not remove the folder: {error}")`). The service runs it as a job like `Prune` (enqueue `Job::RemoveFolder(path)`; on completion its `ScanSummary`/status reports the removed count — follow how Prune reports its count; the TUI's `poll_library` turns finished jobs into toasts: make the toast `Removed <path> (<n> tracks)`).
- [x] Test through the service (follow the existing service tests for Prune).
- [x] Verify, commit `feat(library): remove a music folder and its tracks`.

### Task 4: Cover size

**Files:** `znicz-tui/src/tui_config.rs`, `znicz-tui/src/layout.rs` (`stage_layout`), its callers (`views/stage.rs`), `znicz/src/main.rs` (`TuiSection.cover_size: Option<i64>`, `tui_config`).

- [x] `CoverSize { P100, P75, P50, P25 }` with `from_percent(i64) -> Self` (other → P100), `percent(self) -> u16`, `next`/`prev` (stop at ends) and `ALL`. `TuiConfig.cover_size` default P100. Tests for parse.
- [x] `stage_layout(area, font_w, font_h, show_cover, size: CoverSize)`: compute the cover rect as today, then scale width and height by the percentage (integer, rounded down, at least 4 wide and 2 high), centre it vertically in the original cover slot (and horizontally in the stacked layout where the cover is centred today — keep today's horizontal rule). Wide layout: details `x` = scaled cover right edge + gap, width = rest. Stacked: details `y` right below the scaled cover, height = rest. P100 must give exactly today's rects (test: compare against the current function's outputs for a few areas before changing it — write that test first).
- [x] Tests: 50% wide (cover width ≈ half, vertically centred, details start after it); 50% stacked.
- [x] Verify, commit `feat(tui): cover size setting`.

### Task 5: Settings model

**Files:** Create `znicz-tui/src/settings.rs`.

Pure data, no rendering. Rows are an enum so code can match on them.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Heading(&'static str),
    ShowCover, CoverSize, CoverProtocol, CoverSearch, LibraryLayout, Marquee, Icons,
    DefaultDevice, BitPerfect,
    Folder(PathBuf), AddFolder, RescanAll,
    UpdateCheck,
}

pub fn rows(folders: &[PathBuf]) -> Vec<Row>; // headings "Display", "Audio", "Library", "Updates" in spec order
impl Row {
    pub fn selectable(&self) -> bool;          // false for Heading
    pub fn label(&self) -> String;             // spec labels; Folder → path with ~ for home
    pub fn explanation(&self) -> &'static str; // spec texts verbatim
}
```

  Values come from a small view struct so the model stays testable:

```rust
pub struct Values<'a> {
    pub tui: &'a TuiConfig,
    pub default_device: Option<&'a str>,   // display name, None = "System default"
    pub bit_perfect: bool,
    pub update_check: bool,
    pub protocol_needs_restart: bool,
    pub bit_perfect_changed: bool,
}
pub fn value_text(row: &Row, v: &Values) -> String;   // "on"/"off", "75%", "auto", ...
pub fn hint_text(row: &Row, v: &Values) -> Option<&'static str>; // "↵ choose", "on restart", "d remove", "↵"
```

  Choice stepping: `fn step_choice(row: &Row, tui: &mut TuiConfig, forward: bool, wrap: bool) -> bool` for CoverSize/CoverProtocol/LibraryLayout/Icons (returns whether it changed). Enter/Space uses `wrap = true`, `h`/`l` uses `wrap = false`.
- [x] Tests: rows order and headings; headings not selectable; every selectable row has a non-empty explanation equal to the spec text (copy the table texts into the test for 3–4 rows); `step_choice` wraps with `wrap` and stops at ends without; value texts.
- [x] Verify, commit `feat(tui): settings rows model`.

### Task 6: Settings modal

**Files:** Create `znicz-tui/src/views/settings.rs`; modify `znicz-tui/src/app.rs` (Modal::Settings, `S`, keys, apply+save), `znicz-tui/src/views/mod.rs` (draw it like Help/Devices), `znicz-tui/src/keys.rs` (Context::Settings with bindings `Enter / Space change`, `h / l choose`, `d remove folder`, `Esc close`; add `b("S", "settings")` to GLOBAL), `znicz-tui/src/hit.rs` if list hits are per-modal.

- [x] Tests first (app.rs tests; use a temp config file through `with_config_file`):
  - `s_opens_settings_and_esc_closes`.
  - `cursor_skips_headings` (first selectable row is Show cover; `k` at the top stays; moving down over "Audio" lands on Default output device).
  - `enter_on_show_cover_flips_it_and_saves` → `app.tui.show_cover == false`, file has `show_cover = false`.
  - `l_and_h_step_cover_size_and_stop_at_the_ends`; `enter_steps_and_wraps`.
  - `the_strip_shows_the_highlighted_rows_explanation` (render to TestBackend, find the text).
  - `a_failed_save_restores_the_value_and_toasts` (config path inside a non-existent directory whose parent is a file, so create_dir_all fails).
  - `without_a_config_file_the_change_applies_and_toasts_once`.
  - `a_broken_config_is_not_written` (`with_config_problem` set → change applies, toast `Not saved: config.toml has an error (...)`, file unchanged).
  - `help_lists_s`.
- [x] App state: `settings_cursor: Cursor` (existing cursor type, stepping over non-selectable rows — add a helper that moves to the next selectable index), `settings_rows: Vec<Row>` rebuilt on open and after library jobs, `config_file: Option<ConfigFile>`, `config_problem: Option<String>`, `run_only_warned: bool`, `settings_confirm: Option<PathBuf>` (pending folder removal), `bit_perfect` / `update_check` / `default_device` values (pass from the binary through `TuiExtras` → `App::with_settings_values(..)`; they are config values, not live player state).
- [x] Apply + save, one function: `fn change_setting(&mut self, row: &Row, action: Change)` where `Change::{Next{wrap}, Prev, Toggle, Set(bool)}`. Each row: mutate state; then `save(section, key, value)`:
  - show_cover → `[tui] show_cover`; also `cover_draw_key = None`.
  - cover_size → `[tui] cover_size` int; `cover_draw_key = None`.
  - cover_protocol → `[tui] cover_protocol` lowercase string; if the start-up query ran (store `queried_protocol: Option<ProtocolType>` in `run()` from `make_picker` when protocol was Auto/Kitty/Sixel), `picker.set_protocol_type(..)` per spec and `cover_draw_key = None`; otherwise mark `protocol_needs_restart = true`.
  - cover_search → `[covers] search`.
  - library_layout → `[tui] library_layout` (`"columns"`/`"tree"`).
  - marquee → `[tui] marquee`. icons → `[tui] icons`.
  - bit_perfect → `[audio] bit_perfect`; set `bit_perfect_changed = true` (hint `on restart`).
  - update_check → `[updates] check`.
  - `save` rules: no config file → toast once `Settings apply to this run only (no config file path)`; config problem → toast `Not saved: config.toml has an error (<msg>)`; write error → restore the previous value (keep a clone of the old state before mutating) and `toasts.error(format!("Could not save settings: {e}"))`.
- [x] Keys in Settings: j/k/↓/↑/g/G/Ctrl-d/Ctrl-u move (skip headings); Enter/Space: switch → toggle, choice → next with wrap, action rows → run (Task 7); h/← and l/→: choice prev/next without wrap, switch → off/on; `d` on Folder → confirm (Task 7); Esc → close (or cancel a pending confirm). Mouse: click selects a row, wheel moves (follow how Help/Devices use `ListHit`).
- [x] Render `views/settings.rs`: modal block titled `Settings`, bottom border the config path (home → `~`, cut from the left with `…`), list area = inner minus the strip (2 lines + 1 rule; 1 line when inner height < 8), rows `  label … value  hint` (headings bold, not highlighted), highlight like other modals, `visible_window` for the list (views/window.rs), strip = explanation (or the confirm text) wrapped/dimmed. Footer hint `Enter change · h/l choose · Esc close`.
- [x] Verify, commit `feat(tui): settings modal`.

### Task 7: Default device and folder actions

**Files:** `znicz-tui/src/app.rs`, `znicz-tui/src/views/devices.rs`, `znicz-tui/src/views/settings.rs`.

- [x] Tests: `choosing_a_default_device_switches_and_saves` (FakePlayer/whatever existing devices tests use: Enter on Default output device opens Devices in default mode with title `Default output device` and first row `System default`; Enter on a device sends `SetDevice(id)` and writes `[audio] device = "<id>"`; back in Settings the value shows the device name); `system_default_removes_the_key`; `esc_in_default_mode_returns_to_settings`; `add_folder_opens_the_prompt_and_returns_to_settings`; `rescan_sends_rescan_all`; `d_asks_then_enter_sends_remove_folder` (strip shows `Remove ~/x and its N tracks from the library? Enter yes · Esc no` — N from the library read connection: `SELECT COUNT(*)` via a new `Library::count_under(folder)` sharing the prefix logic with Task 3; Enter sends `LibraryOp::RemoveFolder`); `esc_cancels_the_removal`.
- [x] Implement a `devices_mode: DevicesMode { Switch, Default }` flag; in Default mode the list has a virtual first row; Enter → as spec; after choosing or Esc, reopen Settings with the cursor on the device row. Folder prompt: a `return_to_settings: bool` so closing the prompt reopens Settings. Folders list refreshes on open and when `poll_library` sees a finished job while Settings is open.
- [x] Verify, commit `feat(tui): default device and folder actions in settings`.

### Task 8: Docs, version

- [x] README:
  - Configuration: say the file is created with the defaults on first start, show the template (copy `znicz/src/default-config.toml`), mention `znicz config init [--force]`, `znicz config path`, and the Settings screen (`S`).
  - New section **Nerd Font icons** per the spec's "Nerd Font guide" (verify package names/commands: Nerd Fonts site/README, Homebrew cask name, Arch package name; leave out what cannot be verified). Link it from the `icons` line.
- [x] Wiki: `wiki/Architecture/TUI.md` (Settings modal, `S`, keys, explanation strip; link the Nerd Font section from the icon table), `wiki/Architecture/Library.md` (remove folder), `wiki/Architecture/Overview.md` config block (`cover_size`, first-start file), `_Sidebar.md` if a page is added.
- [x] `Cargo.toml` version `0.16.0`; `cargo update -w`; `CHANGELOG.md` `## 0.16.0 - <date>`; `scripts/third-party-licenses.sh` (toml_edit is MIT/Apache).
- [x] Verify, commit `chore: release 0.16.0`.
