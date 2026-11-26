# Spotify control, part 4: TUI Settings tab and login panel — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Settings gets a sixth tab, "Spotify", with the client ID, redirect port, browser rule, account and playback device. Logging in runs in a panel that shows the URL, copies it with OSC 52, and accepts a pasted address. Every Spotify call runs on a background worker, so no frame waits for Spotify.

**Architecture:** Three pure modules hold the logic that can be tested without a terminal: `spotify_rows.rs` (account text, device choices, value checks, panel text), `osc52.rs` (the clipboard sequence), and the `Row` and `Values` additions in `settings.rs`. `spotify_worker.rs` is a generic `SpotifyWorker<T>`: a thread owns a `SpotifyHost` (a second `IpcClient` in production), takes `(tag, SpotifyOp)` requests on a channel and sends `Done { tag, result }` back. `app/spotify_ui.rs` is a second `impl App` beside `settings_ui.rs`. It drains the worker's replies each tick, polls `SpotifyHost::try_state` on the main client at most every 500 ms while the Spotify tab is open, and runs the login panel. Saving goes through the existing `save_setting` path, which sends `SpotifyOp::Configure` only after the config file was written.

**Tech Stack:** Rust 2024 (MSRV 1.85: no `let` chains; `Option::is_none_or` is allowed), `ratatui` 0.30, `crossterm` 0.28, `base64` 0.22 (already a workspace dependency, used by `znicz-spotify`), `toml_edit` (through `ConfigFile`), `tempfile` and `toml` for tests. The only new dependency edges are `znicz-tui → znicz-spotify` (for `OpenBrowser`, `should_open`, `open`) and `znicz-tui → base64`. Neither creates a cycle, since `znicz-spotify` depends only on `znicz-core`.

**Spec:** `docs/superpowers/specs/2026-11-26-spotify-control-design.md` (sections "Settings → Spotify (tab 6)", "Opening the browser", "Notes for parts 4–5"). **Part 3:** `docs/superpowers/plans/2026-11-26-spotify-3-mcp.md` (done). This plan is the model for format.

**Part map** (each part is its own plan, written when the previous one is done):

1. Core types plus the `znicz-spotify` crate (done).
2. Player, IPC, the pause rule, `znicz spotify` commands, `[spotify]` config, browser helpers (done).
3. Typed Spotify errors over IPC, the second connection for MCP, MCP tools, resource, skill (done).
4. This plan: TUI Settings → Spotify tab, the login panel, the Spotify worker, wiki, manual check.
5. TUI `StageModel` / `TransportModel` split, Spotify mode (`O`), browsing and playback through the worker.

**Spec refinements and findings made in this part:**
- **Login panel text.** The spec's third line reads "Waiting for the callback, or paste the address you were sent to." The brief says "Waiting for the browser; or paste the address you were sent to." This plan uses the brief's wording, since the brief is the newer decision. The panel text is one pure function (Task 2), so changing it later is a one-line edit.
- **Dimmed rows.** The spec dims "the rows below Client ID". The brief names Account and Playback device. This plan follows the brief. Redirect port and Open browser stay editable without a client ID, because they only configure things.
- **The panel is drawn over the rows, not in the two-line strip.** The URL is about 200 characters and needs three or four wrapped lines, which the strip cannot hold. While the panel is open, the list area shows the panel body and the strip shows the keys or the paste prompt (Task 10).
- **OSC 8 is not used.** Ratatui writes cells, and an OSC 8 hyperlink would need its escape bytes placed inside a cell's symbol, which ratatui does not support cleanly. The URL is printed as plain wrapped text. `c` copies it. Ctrl-click on the URL in the panel is not supported.
- **tmux needs no passthrough wrapper.** The spec says "tmux with `set-clipboard on`". With that option tmux takes the OSC 52 sequence and forwards it to the outer terminal. The TUI sends the plain sequence. No existing code in this repo writes a raw escape sequence or handles tmux: the cover code relies on ratatui-image's own terminal query. The manual check (Task 14) tests `c` inside tmux. If it fails there, the fix is a DCS passthrough wrapper, planned as a follow-up.
- **Client ID change confirms only when logged in.** Changing the ID logs the user out (the player does this in `set_config`). With no token there is nothing to lose, so the confirmation appears only when logged in.
- **Config with an error restores the in-memory value.** The existing rule keeps a changed value for the run when the file is broken. For Spotify rows that would make the screen show a client ID the player does not have. So a Spotify row that cannot be saved is restored (Task 7).
- **Polling also sends `Touch`.** The spec's poll is `SpotifyStatus` plus `Touch`, both on the main connection. Each poll sends `Touch`, which keeps the player's fast poll going while the tab is open.
- **Esc order.** `on_esc` runs before the Settings keys, so the panel's Esc is handled there (Task 9). In the paste prompt, Esc returns to the panel. In the panel, Esc cancels the login.
- **Closing Settings during a login.** The panel is dropped (`S`, `?`, or another modal). The login continues in the player. The next `Enter` on Account sends `BeginLogin` again, which the player answers with the URL for the login it still holds (the service test `logging_in_twice_does_not_report_a_busy_port`).
- **Narrow terminals.** The six labels need 58 columns inside the modal, which is 70 % of the width (at least 40). Below about 86 columns the frame cuts the end of the bar, so "Spotify" is partly hidden; `Tab` and `6` still reach it, and a click on the visible part works (`tab_label_rects` already clips to the bar). Accepted for this part; a two-row bar is not worth the layout change.
- **No `Configure` unless the file was written.** The TUI sends `Configure` only from `save_setting`, after `ConfigFile` wrote the change. With a config problem, with no config file path, or when the write fails, nothing is sent and the Spotify row goes back to its old value (Task 7). The first save, when `config.toml` does not exist yet, is a normal write: the file is created and the whole `[spotify]` copy the player already has is sent. Clearing the client ID is the one change that logs the user out on purpose; while logged in it asks first (Task 8).
- **Panel keys.** While the login panel is open only `c`, `Enter` and `Esc` are the panel's. Every other key falls through to the global keys, so `S` and `?` still leave Settings and drop the panel (the login keeps running in the player). In the paste prompt every key types, as in the other prompts.
- **Tests never touch the real terminal, clipboard or browser.** `App` holds two small closures, `clipboard` and `browser`, whose defaults are `osc52::copy` and `znicz_spotify::browser::open`. Tests replace both with recorders (Task 9). A test that called the defaults would set the developer's clipboard or open a browser.
- **The worker is generic.** `SpotifyWorker<T>` takes any tag. Part 5 reuses it for browsing and playback, so only the tag type changes.
- **Not in this part:** Spotify mode (`O`), the shared Stage, the CLI, the version bump, CHANGELOG, README. The README and the commented `[spotify]` block in `default-config.toml` are final-PR items, as in Part 3.

**Conventions for every task:**
- Run commands from the repo root `/home/eugene/projects/znicz`.
- Commit messages end with a blank line and the line `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`.
- Never chain `git commit … && git push`. This plan does not push.
- `cargo test` takes one filter before `--`. To run several filters, put them all after it: `cargo test -p znicz-tui -- spotify_rows settings::tests`.
- Each task compiles and passes its tests at its own commit. A task never refers to a type, row or function that a later task adds.
- The commit in each task is made after its tests pass. Intermediate commits may warn about unused code until the task that wires it in lands. Task 13 runs clippy with `-D warnings`, and it must pass there.
- Spotify calls are never made on the UI thread except `SpotifyHost::try_state` and then `apply(SpotifyOp::Touch)` on the main connection, which answer at once (the player's `SpotifyStatus` reads a mutex and no client lock). Two small local round trips every 500 ms, and only while the Spotify tab is open, do not stall a frame. The status is read first, so an old player that does not know `Touch` costs one reconnect, not two.
- In every file, `ZniczError` and the Spotify types come from `znicz_core` (`use znicz_core::{ZniczError, SpotifyErrorKind, SpotifyHost, SpotifyOp, SpotifyReply, SpotifyState, SpotifyLogin, SpotifyAccount, SpotifyDevice, SpotifyConfig, valid_client_id, Result}`). Check the re-exports in `znicz-core/src/lib.rs` (lines 45–49) before the first build, and add any that are missing to that `pub use`.
- Test helpers that already exist are used as they are: `settings_app(&dir)`, `press(&mut app, code)`, `screen_rows(&mut app)`, `screen_buffer(&mut app)`, `saved(&dir)` and `toast_texts(&app)` in `znicz-tui/src/app/settings_ui.rs`.

---

## File structure

| File | Responsibility |
| --- | --- |
| `znicz-tui/src/spotify_rows.rs` (new) | Pure Spotify row logic: `SpotifyView`, account text, device choices, value checks, port range, login panel lines, login outcome, the strings for the confirm questions, `sent_to_player` |
| `znicz-tui/src/osc52.rs` (new) | `sequence(text)` builds the OSC 52 bytes; `copy(text)` writes them to stdout between frames (the App calls it through its `clipboard` field, which tests replace) |
| `znicz-tui/src/spotify_worker.rs` (new) | `SpotifyAsk` tags, `Done<T>`, the generic `SpotifyWorker<T>` with `spawn`, `send`, `try_recv` |
| `znicz-tui/src/settings.rs` | `Tab::Spotify` (sixth tab), `Row` variants, `Kind::Text` and `Kind::Number`, `rows`, `tab`, `kind`, `label`, `explanation`, `config_key`, `value_text`, `hint_text`, `dimmed`, `prompt_range`, `Values.spotify`, `SettingsValues.spotify` and `.open_browser`; `step` made generic over `Clone` |
| `znicz-tui/src/app/settings_ui.rs` | Keys for the Spotify rows, the client ID and port prompts, the confirm question, applying and saving the Spotify rows, the Configure send, restore on a broken config |
| `znicz-tui/src/app/spotify_ui.rs` (new) | `impl App`: `poll_spotify`, `on_spotify_done`, `on_spotify_state`, account and device actions, the login panel and its keys, copy, browser notice |
| `znicz-tui/src/app.rs` | `mod spotify_ui;`; the new App fields and their defaults; `with_remote` sets the main status host; `with_spotify_worker`; `with_settings_values` copies the Spotify values; the tick calls `poll_spotify`; `on_esc` handles the panel and the confirm; `Context` for the panel |
| `znicz-tui/src/views/settings.rs` | Tab label strip text "1–6", the panel body in the list area, the paste prompt and confirm strip, dimmed rows, `values()` builds `SpotifyView`, the prompt's range text |
| `znicz-tui/src/keys.rs` | `SETTINGS` says `1–6` and lists the login panel's keys; `Context::SettingsLogin` with its footer hints |
| `znicz-tui/src/lib.rs` | `pub mod osc52;`, `pub mod spotify_rows;`, `pub mod spotify_worker;` (beside `pub mod settings;`) |
| `znicz-tui/Cargo.toml` | `base64.workspace = true`, `znicz-spotify = { path = "../znicz-spotify" }` |
| `znicz/src/main.rs` | `SettingsValues` gets `spotify_config(&config.spotify)` and `open_browser_setting(&config.spotify)`; the three `run_tui_with_client` call sites open a second connection with `connect_ui(device, config)` and pass it in |
| `wiki/Architecture/TUI.md` | Settings section: tab 6, rows, keys, login panel, copying, browser rule; Modules list |
| `wiki/Architecture/Spotify.md` | TUI login route |

---

### Task 1: The sixth tab and every tab-count assumption

Adds `Tab::Spotify` after General, makes `1`–`6` select tabs, and updates the places that assumed five tabs. No rows yet: `rows(Tab::Spotify, _)` returns an empty list until Task 3.

**Files:**
- Modify: `znicz-tui/src/settings.rs` (the `Tab` enum, `ALL`, `label`, `from_key`, `rows`, the test `tabs_have_labels_and_number_keys`)
- Modify: `znicz-tui/src/app/settings_ui.rs` (`on_tab_bar_key`: the right-arrow end is `Tab::Spotify`; the tests `tab_and_shift_tab_switch_tabs_and_wrap`, `the_tab_bar_shows_every_tab`, and any other test that expects `General` to be the last tab)
- Modify: `znicz-tui/src/views/settings.rs` (`TAB_BAR_HELP`)
- Modify: `znicz-tui/src/keys.rs` (`SETTINGS`: `b("1–6", "go to a tab")`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `Tab::Spotify` with `label() == "Spotify"`, `Tab::from_key('6') == Some(Tab::Spotify)`, and `Tab::ALL` of length 6 in the order Display, Audio, Playback, Library, General, Spotify.

- [x] **Step 1: Write the failing tests**

In `znicz-tui/src/settings.rs`, replace the body of `tabs_have_labels_and_number_keys` so it asserts the six tabs:

```rust
    #[test]
    fn tabs_have_labels_and_number_keys() {
        assert_eq!(
            Tab::ALL.map(Tab::label),
            ["Display", "Audio", "Playback", "Library", "General", "Spotify"]
        );
        assert_eq!(Tab::from_key('1'), Some(Tab::Display));
        assert_eq!(Tab::from_key('3'), Some(Tab::Playback));
        assert_eq!(Tab::from_key('6'), Some(Tab::Spotify));
        assert_eq!(Tab::from_key('7'), None);
        assert_eq!(Tab::from_key('0'), None);
        assert_eq!(Tab::Spotify.next(), Tab::Display);
        assert_eq!(Tab::Display.previous(), Tab::Spotify);
    }
```

In `znicz-tui/src/app/settings_ui.rs`, in `tab_and_shift_tab_switch_tabs_and_wrap` the last assertion becomes `assert_eq!(app.settings_tab, Tab::Spotify);` (BackTab from Audio wraps to Display, then to Spotify). In `the_tab_bar_shows_every_tab`, the label list is `["Display", "Audio", "Playback", "Library", "General", "Spotify"]`.

Add a test to the same file's test module:

```rust
    #[test]
    fn the_right_arrow_stops_at_the_last_tab_spotify() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        assert_eq!(app.settings_tab, Tab::Spotify);
        // The tab has no rows yet, so there is no row to press Up on.
        app.settings_on_tab_bar = true;
        press(&mut app, KeyCode::Right);
        assert_eq!(app.settings_tab, Tab::Spotify);
    }
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui -- tabs_have_labels_and_number_keys the_right_arrow_stops_at_the_last_tab_spotify`
Expected: FAIL (no variant `Spotify`).

- [x] **Step 3: Implement**

In `settings.rs`: add `Spotify` after `General` in the `Tab` enum; `ALL: [Tab; 6]` with `Tab::Spotify` last; `label` returns `"Spotify"`; `from_key` doc says `'1'` to `'6'` (its code already uses `ALL.get`); `rows` gets `Tab::Spotify => Vec::new(),` for now. Task 3 fills it in.

In `settings_ui.rs` `on_tab_bar_key`, the right-arrow guard changes from `Tab::General` to `Tab::Spotify`.

In `views/settings.rs`, `TAB_BAR_HELP` says `"←/→ choose a tab, ↓ or Enter go to its settings. Tab and 1–6 switch tabs from anywhere."`

In `keys.rs`, `SETTINGS` has `b("1–6", "go to a tab")`.

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- settings::tests settings_ui::tests`
Expected: PASS. The tests `each_tab_has_its_rows` and the ones that index `Tab::General` as the last tab still pass, because General is unchanged.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/settings.rs znicz-tui/src/app/settings_ui.rs znicz-tui/src/views/settings.rs znicz-tui/src/keys.rs
git commit -m "feat(tui): add the Spotify Settings tab

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 2: The pure Spotify helpers

Everything the rows say and check, as functions with no IO. This file is the place for the strings the spec and the brief fix. It does not use `settings.rs`, so it compiles and passes on its own; Task 3 builds on it.

**Files:**
- Create: `znicz-tui/src/spotify_rows.rs`
- Modify: `znicz-tui/src/lib.rs` (add `pub mod spotify_rows;` after `pub mod settings;`)
- Modify: `znicz-tui/Cargo.toml` (add `znicz-spotify = { path = "../znicz-spotify" }` and `base64.workspace = true` under `[dependencies]`)

**Interfaces:**
- Consumes: `znicz_core::{SpotifyAccount, SpotifyDevice, SpotifyLogin, valid_client_id}`, `znicz_spotify::browser::OpenBrowser`. Nothing from `settings.rs`: Task 3 builds on this module, not the other way round.
- Produces:
  - `pub struct SpotifyView { pub configured: bool, pub client_id_head: String, pub device: String, pub redirect_port: u16, pub open_browser: OpenBrowser, pub login: SpotifyLogin, pub account: Option<SpotifyAccount>, pub devices: Vec<SpotifyDevice>, pub unavailable: Option<String> }` (derives `Debug, Clone, Default, PartialEq`)
  - `pub const OPEN_BROWSERS: [OpenBrowser; 3]`
  - `pub const PORT_MIN: u16 = 1024`, `pub const DEFAULT_PORT` is `SpotifyConfig::DEFAULT_PORT` (use that)
  - `pub const CLIENT_FIRST: &str`, `pub const LOGOUT_QUESTION: &str`, `pub const CLIENT_ID_QUESTION: &str`, `pub const WRONG_ADDRESS: &str`
  - `pub fn account_text(view: &SpotifyView) -> String`
  - `pub fn device_choices(view: &SpotifyView) -> Vec<String>`
  - `pub fn device_value(name: &str) -> String`
  - `pub fn device_online(view: &SpotifyView, name: &str) -> bool`
  - `pub fn client_id_problem(text: &str) -> Option<&'static str>`
  - `pub fn parse_port(text: &str) -> std::result::Result<u16, String>`
  - `pub fn open_browser_name(value: OpenBrowser) -> &'static str`
  - `pub fn redirect_uri(port: u16) -> String`
  - `pub fn login_panel_lines(url: &str, listening: bool) -> Vec<String>`
  - `pub enum LoginOutcome { Success(String), Failure(String) }`
  - `pub fn login_outcome(login: &SpotifyLogin, account: Option<&SpotifyAccount>) -> Option<LoginOutcome>`

- [x] **Step 1: Write the failing tests**

Create `znicz-tui/src/spotify_rows.rs` with the module doc and tests only (the functions come in Step 3). The tests:

```rust
//! The Spotify rows of Settings, as pure functions: what the account row says,
//! which devices the choice offers, what a typed value must be, and the text of
//! the login panel. No IO, no App.

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> SpotifyDevice {
        SpotifyDevice {
            id: format!("id-{name}"),
            name: name.into(),
            kind: "Computer".into(),
            is_active: false,
            volume_percent: None,
        }
    }

    fn account(premium: Option<bool>) -> SpotifyAccount {
        SpotifyAccount { user: "Eugene".into(), premium }
    }

    fn view(login: SpotifyLogin) -> SpotifyView {
        SpotifyView { configured: true, login, ..SpotifyView::default() }
    }

    #[test]
    fn the_account_row_says_each_login_state() {
        assert_eq!(account_text(&view(SpotifyLogin::NotConfigured)), "Not configured");
        assert_eq!(account_text(&view(SpotifyLogin::NotLoggedIn)), "Not logged in");
        assert_eq!(account_text(&view(SpotifyLogin::LoggingIn)), "Logging in…");
        assert_eq!(
            account_text(&view(SpotifyLogin::Failed("bad redirect".into()))),
            "Failed: bad redirect"
        );
    }

    #[test]
    fn a_logged_in_account_says_what_its_plan_is_once_known() {
        let mut v = view(SpotifyLogin::LoggedIn);
        v.account = Some(account(Some(true)));
        assert_eq!(account_text(&v), "Logged in as Eugene · Premium");
        v.account = Some(account(Some(false)));
        assert_eq!(
            account_text(&v),
            "Logged in as Eugene · not Premium (control unavailable)"
        );
        v.account = Some(account(None));
        assert_eq!(account_text(&v), "Logged in as Eugene");
        v.account = None;
        assert_eq!(account_text(&v), "Logged in");
    }

    #[test]
    fn an_unavailable_player_wins_over_the_login_state() {
        let mut v = view(SpotifyLogin::LoggedIn);
        v.unavailable = Some("the running player has no Spotify service".into());
        assert_eq!(
            account_text(&v),
            "Spotify is unavailable: the running player has no Spotify service"
        );
    }

    #[test]
    fn device_choices_start_with_the_active_device_and_keep_an_offline_name() {
        let mut v = view(SpotifyLogin::LoggedIn);
        v.devices = vec![device("Laptop"), device("Kitchen"), device("Laptop")];
        assert_eq!(device_choices(&v), ["", "Laptop", "Kitchen"]);
        v.device = "Garage".into();
        assert_eq!(device_choices(&v), ["", "Laptop", "Kitchen", "Garage"]);
        assert!(!device_online(&v, "Garage"));
        assert!(device_online(&v, "Kitchen"));
        assert!(device_online(&v, ""));
    }

    #[test]
    fn the_active_device_is_named_in_words() {
        assert_eq!(device_value(""), "Active device");
        assert_eq!(device_value("Kitchen"), "Kitchen");
    }

    #[test]
    fn a_client_id_is_32_hex_characters_or_empty() {
        assert_eq!(client_id_problem(""), None);
        assert_eq!(client_id_problem("   "), None);
        assert_eq!(client_id_problem("0123456789abcdef0123456789ABCDEF"), None);
        assert_eq!(client_id_problem("  0123456789abcdef0123456789abcdef \n"), None);
        assert_eq!(client_id_problem("0123456789abcdef0123456789abcde"), Some("32 hex characters"));
        assert_eq!(client_id_problem("0123456789abcdef0123456789abcdefa"), Some("32 hex characters"));
        assert_eq!(client_id_problem("0123456789abcdef0123456789abcdeg"), Some("32 hex characters"));
    }

    #[test]
    fn a_port_is_1024_to_65535() {
        assert_eq!(parse_port("8898"), Ok(8898));
        assert_eq!(parse_port(" 1024 "), Ok(1024));
        assert_eq!(parse_port("65535"), Ok(65535));
        assert_eq!(parse_port("1023"), Err("1024–65535".to_string()));
        assert_eq!(parse_port("0"), Err("1024–65535".to_string()));
        assert_eq!(parse_port("65536"), Err("1024–65535".to_string()));
        assert_eq!(parse_port("http"), Err("1024–65535".to_string()));
        assert_eq!(parse_port(""), Err("1024–65535".to_string()));
    }

    #[test]
    fn the_redirect_uri_is_the_one_the_app_needs() {
        assert_eq!(redirect_uri(8898), "http://127.0.0.1:8898/callback");
    }

    #[test]
    fn open_browser_names_and_order() {
        assert_eq!(OPEN_BROWSERS.map(open_browser_name), ["auto", "always", "never"]);
    }

    #[test]
    fn the_login_panel_says_what_to_do_in_each_case() {
        let url = "https://accounts.spotify.com/authorize?client_id=x";
        let listening = login_panel_lines(url, true);
        assert_eq!(listening.len(), 3);
        assert_eq!(listening[0], "Open this URL in any browser and approve:");
        assert_eq!(listening[1], format!("{url}   (c copy · Enter paste)"));
        assert_eq!(
            listening[2],
            "Waiting for the browser; or paste the address you were sent to."
        );
        let pasting = login_panel_lines(url, false);
        assert_eq!(pasting[2], "Paste the address you were sent to.");
    }

    #[test]
    fn login_outcomes_only_for_finished_logins() {
        assert_eq!(
            login_outcome(&SpotifyLogin::LoggedIn, Some(&account(None))),
            Some(LoginOutcome::Success("Logged in as Eugene".into()))
        );
        assert_eq!(
            login_outcome(&SpotifyLogin::Failed("no".into()), None),
            Some(LoginOutcome::Failure("Spotify login failed: no".into()))
        );
        assert_eq!(login_outcome(&SpotifyLogin::LoggingIn, None), None);
        assert_eq!(login_outcome(&SpotifyLogin::NotLoggedIn, None), None);
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui spotify_rows`
Expected: FAIL to compile (the functions are not defined yet).

- [x] **Step 3: Implement**

Write the functions in `spotify_rows.rs`, above the test module:

```rust
use znicz_core::{SpotifyAccount, SpotifyDevice, SpotifyLogin, valid_client_id};
use znicz_spotify::browser::OpenBrowser;

/// The lowest port the login callback may use; the top is 65535.
pub const PORT_MIN: u16 = 1024;
pub const OPEN_BROWSERS: [OpenBrowser; 3] = [OpenBrowser::Auto, OpenBrowser::Always, OpenBrowser::Never];
/// The strip's text when a row needs a client ID first.
pub const CLIENT_FIRST: &str = "Set a client ID first, in the Client ID row.";
pub const LOGOUT_QUESTION: &str = "Log out of Spotify? Enter logs out · Esc cancels";
pub const CLIENT_ID_QUESTION: &str =
    "Changing the client ID logs you out of Spotify. Enter saves · Esc cancels";
/// Shown under the paste prompt when Spotify rejects the pasted address.
pub const WRONG_ADDRESS: &str = "that address is not from this login";

/// What the Spotify rows show, copied from the App each frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpotifyView {
    /// A client ID is set.
    pub configured: bool,
    /// The first eight characters of the client ID, for the row. The full ID never reaches the row text.
    pub client_id_head: String,
    /// `[spotify] device`; empty is the active device.
    pub device: String,
    pub redirect_port: u16,
    pub open_browser: OpenBrowser,
    /// The login as the App shows it (see `App::spotify_login_shown`).
    pub login: SpotifyLogin,
    pub account: Option<SpotifyAccount>,
    /// The last device list from `SpotifyOp::Devices`.
    pub devices: Vec<SpotifyDevice>,
    /// The player does not answer Spotify requests: the reason.
    pub unavailable: Option<String>,
}

/// The account row's text.
pub fn account_text(view: &SpotifyView) -> String {
    if let Some(reason) = &view.unavailable {
        return format!("Spotify is unavailable: {reason}");
    }
    match &view.login {
        SpotifyLogin::NotConfigured => "Not configured".into(),
        SpotifyLogin::NotLoggedIn => "Not logged in".into(),
        SpotifyLogin::LoggingIn => "Logging in…".into(),
        SpotifyLogin::Failed(reason) => format!("Failed: {reason}"),
        SpotifyLogin::LoggedIn => match &view.account {
            None => "Logged in".into(),
            Some(account) => {
                let plan = match account.premium {
                    Some(true) => " · Premium",
                    Some(false) => " · not Premium (control unavailable)",
                    None => "",
                };
                format!("Logged in as {}{plan}", account.user)
            }
        },
    }
}

/// The Playback device choices: the active device (empty name) first, then
/// each device Spotify lists once, then the configured name when Spotify does
/// not list it (it is shown as offline).
pub fn device_choices(view: &SpotifyView) -> Vec<String> {
    let mut names = vec![String::new()];
    for device in &view.devices {
        if !names.contains(&device.name) {
            names.push(device.name.clone());
        }
    }
    if !view.device.is_empty() && !names.contains(&view.device) {
        names.push(view.device.clone());
    }
    names
}

/// A choice's text: the empty name is the active device.
pub fn device_value(name: &str) -> String {
    if name.is_empty() { "Active device".into() } else { name.into() }
}

/// The active device is always reachable; a named device must be listed.
pub fn device_online(view: &SpotifyView, name: &str) -> bool {
    name.is_empty() || view.devices.iter().any(|d| d.name == name)
}

/// Why a typed client ID is refused, or `None` when it is accepted. The text
/// is trimmed first. Empty is accepted: it clears the ID.
pub fn client_id_problem(text: &str) -> Option<&'static str> {
    let text = text.trim();
    (!text.is_empty() && !valid_client_id(text)).then_some("32 hex characters")
}

/// A typed port, or the range text the strip shows.
pub fn parse_port(text: &str) -> std::result::Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(port) if port >= PORT_MIN => Ok(port),
        _ => Err("1024–65535".into()),
    }
}

pub fn open_browser_name(value: OpenBrowser) -> &'static str {
    match value {
        OpenBrowser::Auto => "auto",
        OpenBrowser::Always => "always",
        OpenBrowser::Never => "never",
    }
}

/// The URI the Spotify app must register.
pub fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

/// The panel's three lines. The URL line carries the keys.
pub fn login_panel_lines(url: &str, listening: bool) -> Vec<String> {
    let waiting = if listening {
        "Waiting for the browser; or paste the address you were sent to."
    } else {
        "Paste the address you were sent to."
    };
    vec![
        "Open this URL in any browser and approve:".into(),
        format!("{url}   (c copy · Enter paste)"),
        waiting.into(),
    ]
}

/// A finished login, for the toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    Success(String),
    Failure(String),
}

/// The toast for a login that has finished. `None` while it runs, and for a
/// login that was cancelled (`NotLoggedIn`).
pub fn login_outcome(login: &SpotifyLogin, account: Option<&SpotifyAccount>) -> Option<LoginOutcome> {
    match login {
        SpotifyLogin::LoggedIn => {
            let user = account.map_or("Spotify", |a| a.user.as_str());
            Some(LoginOutcome::Success(format!("Logged in as {user}")))
        }
        SpotifyLogin::Failed(reason) => Some(LoginOutcome::Failure(format!("Spotify login failed: {reason}"))),
        _ => None,
    }
}

```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui spotify_rows`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/spotify_rows.rs znicz-tui/src/lib.rs znicz-tui/Cargo.toml Cargo.lock
git commit -m "feat(tui): pure helpers for the Spotify Settings rows

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 3: The Spotify rows, their values and kinds

Adds the five rows to `settings.rs` and fills `rows(Tab::Spotify, _)`. Adds `Kind::Text` and `Kind::Number`. Adds `Values.spotify` and the value, hint and dim rules. Makes `step` generic so later tasks can step strings and the browser setting. Adding fields to `Values` and `SettingsValues` breaks their two non-test constructors, so this task also fills them in: the view's `values()` with an empty `SpotifyView` for now (Task 6 reads the real one), and `main.rs` with the config's Spotify values.

**Files:**
- Modify: `znicz-tui/src/settings.rs`
- Modify: `znicz-tui/src/spotify_rows.rs` (`sent_to_player`, which needs `Row`)
- Modify: `znicz-tui/src/views/settings.rs` (`values()`: `spotify: SpotifyView::default()`)
- Modify: `znicz/src/main.rs` (the `SettingsValues` literal)

**Interfaces:**
- Consumes: `crate::spotify_rows::{SpotifyView, account_text, device_choices, device_value, device_online, redirect_uri, open_browser_name}`.
- Produces:
  - `Row::{ClientId, RedirectPort, OpenBrowser, Account, PlaybackDevice}`
  - `Kind::Text`, `Kind::Number`
  - `pub fn dimmed(row: &Row, v: &Values) -> bool`
  - `pub fn prompt_range(row: &Row) -> Option<String>`
  - `pub fn spotify_rows::sent_to_player(row: &Row) -> bool`
  - `pub(crate) fn step<T: Clone + PartialEq>(all: &[T], current: &T, forward: bool, wrap: bool) -> T`
  - `Values` gains `pub spotify: SpotifyView`
  - `SettingsValues` gains `pub spotify: SpotifyConfig` and `pub open_browser: OpenBrowser`

- [x] **Step 1: Write the failing tests**

Add to the `settings.rs` test module (it needs `use znicz_core::SpotifyLogin;`; the `values()` helper gets `spotify: SpotifyView::default()`, and the other `Values { … }` literals in the tests, three of them, get the same field):

```rust
    #[test]
    fn the_spotify_tab_has_five_rows_in_spec_order() {
        assert_eq!(
            rows(Tab::Spotify, &[]),
            vec![
                Row::ClientId,
                Row::RedirectPort,
                Row::OpenBrowser,
                Row::Account,
                Row::PlaybackDevice,
            ]
        );
        for row in rows(Tab::Spotify, &[]) {
            assert_eq!(row.tab(), Tab::Spotify);
        }
    }

    #[test]
    fn the_spotify_kinds_match_the_spec_table() {
        assert_eq!(Row::ClientId.kind(), Kind::Text);
        assert_eq!(Row::RedirectPort.kind(), Kind::Number);
        assert_eq!(Row::OpenBrowser.kind(), Kind::Choice);
        assert_eq!(Row::Account.kind(), Kind::Action);
        assert_eq!(Row::PlaybackDevice.kind(), Kind::Choice);
    }

    #[test]
    fn the_spotify_config_keys_match_the_config_file() {
        assert_eq!(Row::ClientId.config_key(), Some(("spotify", "client_id")));
        assert_eq!(Row::RedirectPort.config_key(), Some(("spotify", "redirect_port")));
        assert_eq!(Row::OpenBrowser.config_key(), Some(("spotify", "open_browser")));
        assert_eq!(Row::PlaybackDevice.config_key(), Some(("spotify", "device")));
        assert_eq!(Row::Account.config_key(), None);
    }

    #[test]
    fn the_redirect_row_shows_the_uri_and_the_account_row_its_state() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        v.spotify = SpotifyView {
            configured: true,
            redirect_port: 9001,
            login: SpotifyLogin::NotLoggedIn,
            ..SpotifyView::default()
        };
        assert_eq!(value_text(&Row::RedirectPort, &v), "http://127.0.0.1:9001/callback");
        assert_eq!(value_text(&Row::Account, &v), "Not logged in");
        assert_eq!(value_text(&Row::OpenBrowser, &v), "auto");
    }

    #[test]
    fn the_client_id_row_says_only_whether_an_id_is_set() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        assert_eq!(value_text(&Row::ClientId, &v), "not set");
        v.spotify.configured = true;
        v.spotify.client_id_head = "0123abcd".into();
        assert_eq!(value_text(&Row::ClientId, &v), "0123abcd…");
    }

    #[test]
    fn the_playback_device_row_names_the_active_device_and_marks_an_offline_one() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        v.spotify.configured = true;
        assert_eq!(value_text(&Row::PlaybackDevice, &v), "Active device");
        assert_eq!(hint_text(&Row::PlaybackDevice, &v), None);
        v.spotify.device = "Garage".into();
        assert_eq!(value_text(&Row::PlaybackDevice, &v), "Garage");
        assert_eq!(hint_text(&Row::PlaybackDevice, &v), Some("offline"));
    }

    #[test]
    fn account_and_device_are_dimmed_until_a_client_id_is_set() {
        let tui = TuiConfig::default();
        let mut v = values(&tui);
        assert!(dimmed(&Row::Account, &v));
        assert!(dimmed(&Row::PlaybackDevice, &v));
        assert!(!dimmed(&Row::ClientId, &v));
        assert!(!dimmed(&Row::RedirectPort, &v));
        assert!(!dimmed(&Row::OpenBrowser, &v));
        v.spotify.configured = true;
        assert!(!dimmed(&Row::Account, &v));
    }

    #[test]
    fn prompt_ranges_for_the_text_rows() {
        assert_eq!(prompt_range(&Row::RedirectPort), Some("1024–65535".into()));
        assert_eq!(prompt_range(&Row::ClientId), Some("32 hex characters, or empty".into()));
        assert_eq!(prompt_range(&Row::Account), None);
    }

    #[test]
    fn step_works_on_owned_values() {
        let all = ["".to_string(), "A".to_string(), "B".to_string()];
        assert_eq!(step(&all, &"A".to_string(), true, false), "B");
        assert_eq!(step(&all, &"B".to_string(), true, false), "B");
        assert_eq!(step(&all, &"B".to_string(), true, true), "");
        assert_eq!(step(&all, &"".to_string(), false, true), "B");
        // A value that is not listed steps from the first one.
        assert_eq!(step(&all, &"Z".to_string(), true, false), "A");
    }
```

And in `spotify_rows.rs`'s test module add (it needs `use crate::settings::Row;` in that module):

```rust
    #[test]
    fn only_the_player_side_rows_are_sent_to_the_player() {
        assert!(sent_to_player(&Row::ClientId));
        assert!(sent_to_player(&Row::RedirectPort));
        assert!(sent_to_player(&Row::PlaybackDevice));
        assert!(!sent_to_player(&Row::OpenBrowser));
        assert!(!sent_to_player(&Row::Account));
        assert!(!sent_to_player(&Row::ShowCover));
    }
```

Update the `every_row_has_an_explanation` test (it iterates `Tab::ALL`, so it already covers the new rows) and the `kinds_match_the_spec_table` test if it lists the kinds exhaustively; leave the existing expectations for the old rows alone.

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui -- settings::tests spotify_rows`
Expected: FAIL to compile (no variants, no `dimmed`, no `prompt_range`, no `Values.spotify`, no generic `step`, no `sent_to_player`).

- [x] **Step 3: Implement**

In `settings.rs`:

1. Add the imports: `use znicz_core::{FadeTimes, SpotifyConfig};`, `use znicz_spotify::browser::OpenBrowser;`, `use crate::spotify_rows::{self, SpotifyView};`.
2. Add the variants to `Row`, in this order at the end: `ClientId`, `RedirectPort`, `OpenBrowser`, `Account`, `PlaybackDevice`.
3. Add `Kind::Text` and `Kind::Number` to `Kind`.
4. `rows`: `Tab::Spotify => vec![Row::ClientId, Row::RedirectPort, Row::OpenBrowser, Row::Account, Row::PlaybackDevice]`.
5. `tab()`: the five rows map to `Tab::Spotify`.
6. `kind()`: `ClientId => Kind::Text`, `RedirectPort => Kind::Number`, `OpenBrowser | PlaybackDevice => Kind::Choice`, `Account => Kind::Action`.
7. `label()`: `"Client ID"`, `"Redirect port"`, `"Open browser"`, `"Account"`, `"Playback device"`.
8. `explanation()`, one static line each:
   - ClientId: `"Your Spotify app's client ID, 32 hex characters. Get one at developer.spotify.com/dashboard: register a free app and add the redirect URI below to it. Empty clears it."`
   - RedirectPort: `"The local port the login callback listens on. Add exactly the URI shown to the Spotify app's Redirect URIs."`
   - OpenBrowser: `"Open the login page in a browser. auto guesses from the terminal and does not open over SSH; always and never override the guess."`
   - Account: `"Log in to Spotify, or log out. Spotify playback needs Premium."`
   - PlaybackDevice: `"The Spotify device to use when none is active. Active device keeps whichever device Spotify is using."`
9. `config_key()`: the four keys from the test, `Account => None`.
10. `value_text()`:
    - ClientId: `if v.spotify.configured { format!("{}…", v.spotify.client_id_head) } else { "not set".into() }`
    - RedirectPort: `spotify_rows::redirect_uri(v.spotify.redirect_port)`
    - OpenBrowser: `spotify_rows::open_browser_name(v.spotify.open_browser).into()`
    - Account: `spotify_rows::account_text(&v.spotify)`
    - PlaybackDevice: `spotify_rows::device_value(&v.spotify.device)`
11. `hint_text()`: `PlaybackDevice` returns `Some("offline")` when `v.spotify.configured && !spotify_rows::device_online(&v.spotify, &v.spotify.device)`, else `None`. Change the other arms only by adding the new one; keep the existing arms.
12. Add:

```rust
/// Rows that cannot be used until a client ID is set: the strip points at
/// Client ID and their Enter does nothing.
pub fn dimmed(row: &Row, v: &Values) -> bool {
    matches!(row, Row::Account | Row::PlaybackDevice) && !v.spotify.configured
}

/// The range a typed value must be in, for the prompt's label and error. `None`
/// for rows that do not take typed values.
pub fn prompt_range(row: &Row) -> Option<String> {
    match row {
        Row::RedirectPort => Some("1024–65535".into()),
        Row::ClientId => Some("32 hex characters, or empty".into()),
        _ => slider(row).map(slider_range_text),
    }
}
```

13. `Values` gets the field `pub spotify: SpotifyView` (after `fades`).
14. `SettingsValues` gets `pub spotify: SpotifyConfig` (default `SpotifyConfig::default()`) and `pub open_browser: OpenBrowser` (default `OpenBrowser::default()`). The manual `Default` impl adds both.
15. Make `step` generic, with the body below, and `pub(crate)`. Update its five call sites (`step_protocol` and the four arms of `step_choice`) to pass `&tui.cover_protocol` and so on. The existing tests `choice_rows_step_and_wrap_when_asked` and `choice_rows_stop_at_the_ends_without_wrap` stay green.

```rust
pub(crate) fn step<T: Clone + PartialEq>(all: &[T], current: &T, forward: bool, wrap: bool) -> T {
    let i = all.iter().position(|v| v == current).unwrap_or(0);
    let last = all.len() - 1;
    let j = match (forward, wrap) {
        (true, _) if i < last => i + 1,
        (true, true) => 0,
        (true, false) => i,
        (false, _) if i > 0 => i - 1,
        (false, true) => last,
        (false, false) => i,
    };
    all[j].clone()
}
```

16. In `spotify_rows.rs` add (with `use crate::settings::Row;`):

```rust
/// Rows whose value the player must hear about. The browser setting is only
/// read by the front end.
pub fn sent_to_player(row: &Row) -> bool {
    matches!(row, Row::ClientId | Row::RedirectPort | Row::PlaybackDevice)
}
```

17. `views/settings.rs` `values()`: add `spotify: SpotifyView::default(),` (import `crate::spotify_rows::SpotifyView`). Task 6 replaces it with `app.spotify_view()`.
18. `znicz/src/main.rs`, the `SettingsValues` literal (about line 403): add `spotify: spotify_config(&config.spotify), open_browser: open_browser_setting(&config.spotify),`. Both functions already exist in that file.

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- settings::tests spotify_rows && cargo build --workspace`
Expected: PASS; the whole workspace builds.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/settings.rs znicz-tui/src/spotify_rows.rs znicz-tui/src/views/settings.rs znicz/src/main.rs
git commit -m "feat(tui): the five Spotify Settings rows and their values

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The OSC 52 sequence

**Files:**
- Create: `znicz-tui/src/osc52.rs`
- Modify: `znicz-tui/src/lib.rs` (add `pub mod osc52;`)

**Interfaces:**
- Consumes: `base64::Engine`, `base64::engine::general_purpose::STANDARD`.
- Produces: `pub fn sequence(text: &str) -> String`, `pub fn copy(text: &str) -> std::io::Result<()>`.

- [x] **Step 1: Write the failing tests**

Create `znicz-tui/src/osc52.rs`:

```rust
//! The OSC 52 clipboard sequence. The terminal sets its clipboard from it; tmux
//! takes it when `set-clipboard` is on.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_is_osc_52_with_base64_and_bel() {
        assert_eq!(sequence("hi"), "\x1b]52;c;aGk=\x07");
        assert_eq!(sequence(""), "\x1b]52;c;\x07");
    }

    #[test]
    fn the_encoding_is_standard_base64_with_padding() {
        assert_eq!(sequence("a"), "\x1b]52;c;YQ==\x07");
        assert_eq!(sequence("ab"), "\x1b]52;c;YWI=\x07");
        assert_eq!(sequence("abc"), "\x1b]52;c;YWJj\x07");
        // The standard alphabet has `+` and `/`, not `-` and `_`.
        assert_eq!(sequence(">>>"), "\x1b]52;c;Pj4+\x07");
        assert_eq!(sequence("???"), "\x1b]52;c;Pz8/\x07");
    }

    #[test]
    fn the_url_survives_the_round_trip() {
        let url = "https://accounts.spotify.com/authorize?client_id=0123&state=abc&x=y%2Fz";
        let body = sequence(url);
        let encoded = body
            .strip_prefix("\x1b]52;c;")
            .and_then(|rest| rest.strip_suffix('\x07'))
            .expect("framed");
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .expect("base64");
        assert_eq!(String::from_utf8(decoded).unwrap(), url);
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui osc52`
Expected: FAIL to compile (`sequence` missing).

- [x] **Step 3: Implement**

Above the tests:

```rust
use std::io::Write;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// The bytes that ask the terminal to set its clipboard to `text`.
pub fn sequence(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text))
}

/// Write the sequence to the terminal. It is called from a key handler, between
/// two frames: ratatui flushes at the end of every draw, so the bytes never land
/// inside a frame, and an escape sequence changes no cell, so the next frame's
/// diff is unaffected. Tests do not call this; `App::clipboard` is replaced.
pub fn copy(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    out.write_all(sequence(text).as_bytes())?;
    out.flush()
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui osc52`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/osc52.rs znicz-tui/src/lib.rs
git commit -m "feat(tui): OSC 52 sequence for copying the login URL

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 5: The generic Spotify worker

**Files:**
- Create: `znicz-tui/src/spotify_worker.rs`
- Modify: `znicz-tui/src/lib.rs` (add `pub mod spotify_worker;`)

**Interfaces:**
- Consumes: `znicz_core::{Result, SpotifyHost, SpotifyOp, SpotifyReply}`.
- Produces:
  - `pub enum SpotifyAsk { Configure, BeginLogin, CompleteLogin, CancelLogin, Logout, Devices }` (derives `Debug, Clone, Copy, PartialEq, Eq`)
  - `pub struct Done<T> { pub tag: T, pub result: Result<SpotifyReply> }`
  - `pub struct SpotifyWorker<T> { … }` with
    - `pub fn spawn(host: Arc<dyn SpotifyHost>) -> std::io::Result<Self>` where `T: Send + 'static`
    - `pub fn send(&self, tag: T, op: SpotifyOp) -> bool`
    - `pub fn try_recv(&self) -> Option<Done<T>>`

- [x] **Step 1: Write the failing tests**

Create `znicz-tui/src/spotify_worker.rs` with this test module (and the implementation in Step 3):

```rust
#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::time::{Duration, Instant};

    use super::*;
    use znicz_core::{SpotifyState, ZniczError};

    /// Records each op; `Devices` blocks until the test sends a release.
    struct Host {
        seen: Mutex<Vec<SpotifyOp>>,
        gate: Mutex<Receiver<()>>,
        entered: Sender<()>,
    }

    impl SpotifyHost for Host {
        fn apply(&self, op: SpotifyOp) -> Result<SpotifyReply> {
            self.seen.lock().unwrap().push(op.clone());
            if op == SpotifyOp::Devices {
                let _ = self.entered.send(());
                self.gate.lock().unwrap().recv().unwrap();
                return Ok(SpotifyReply::Devices { devices: Vec::new() });
            }
            if op == SpotifyOp::Logout {
                return Err(ZniczError::Player("nope".into()));
            }
            Ok(SpotifyReply::Done)
        }
        fn state(&self) -> SpotifyState {
            SpotifyState::default()
        }
    }

    fn host() -> (Arc<Host>, Sender<()>, Receiver<()>) {
        let (release, gate) = channel();
        let (entered, started) = channel();
        let host = Arc::new(Host {
            seen: Mutex::new(Vec::new()),
            gate: Mutex::new(gate),
            entered,
        });
        (host, release, started)
    }

    fn wait_for(worker: &SpotifyWorker<SpotifyAsk>) -> Done<SpotifyAsk> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(done) = worker.try_recv() {
                return done;
            }
            assert!(Instant::now() < deadline, "no reply from the worker");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn replies_come_back_tagged_in_request_order() {
        let (host, release, _started) = host();
        release.send(()).unwrap();
        let worker = SpotifyWorker::spawn(host.clone()).unwrap();
        assert!(worker.send(SpotifyAsk::Configure, SpotifyOp::Touch));
        assert!(worker.send(SpotifyAsk::Logout, SpotifyOp::Logout));
        let first = wait_for(&worker);
        assert_eq!(first.tag, SpotifyAsk::Configure);
        assert!(first.result.is_ok());
        let second = wait_for(&worker);
        assert_eq!(second.tag, SpotifyAsk::Logout);
        assert!(second.result.is_err());
        assert_eq!(host.seen.lock().unwrap().as_slice(), [SpotifyOp::Touch, SpotifyOp::Logout]);
    }

    #[test]
    fn a_slow_op_does_not_block_try_recv() {
        let (host, release, started) = host();
        let worker = SpotifyWorker::spawn(host).unwrap();
        worker.send(SpotifyAsk::Devices, SpotifyOp::Devices);
        started.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = Instant::now();
        assert!(worker.try_recv().is_none());
        assert!(before.elapsed() < Duration::from_millis(50));
        release.send(()).unwrap();
        assert_eq!(wait_for(&worker).tag, SpotifyAsk::Devices);
    }

    #[test]
    fn dropping_the_worker_ends_its_thread() {
        let (host, release, _started) = host();
        release.send(()).unwrap();
        let worker = SpotifyWorker::<SpotifyAsk>::spawn(host.clone()).unwrap();
        assert!(Arc::strong_count(&host) >= 2, "the thread holds the host");
        drop(worker);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&host) > 1 {
            assert!(Instant::now() < deadline, "the thread did not end");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui spotify_worker`
Expected: FAIL to compile.

- [x] **Step 3: Implement**

Above the tests:

```rust
//! A background connection for the Spotify calls that can be slow (up to about
//! 40 s). The UI thread sends a request and drains the replies each tick, so
//! no frame waits on Spotify. Generic over the tag, so Part 5 can reuse it.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use znicz_core::{Result, SpotifyHost, SpotifyOp, SpotifyReply};

/// What the Settings tab asked for. Part 5 adds its own tags for browsing and playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyAsk {
    Configure,
    BeginLogin,
    CompleteLogin,
    CancelLogin,
    Logout,
    Devices,
}

/// The answer to one request, with the tag that asked for it.
#[derive(Debug)]
pub struct Done<T> {
    pub tag: T,
    pub result: Result<SpotifyReply>,
}

/// The worker thread's two channels. Dropping the worker closes the request
/// channel, and the thread ends after the request it is running.
pub struct SpotifyWorker<T> {
    requests: Sender<(T, SpotifyOp)>,
    replies: Receiver<Done<T>>,
}

impl<T: Send + 'static> SpotifyWorker<T> {
    /// Start the thread that owns `host`. Requests run in the order sent.
    pub fn spawn(host: Arc<dyn SpotifyHost>) -> std::io::Result<Self> {
        let (requests, inbox) = channel::<(T, SpotifyOp)>();
        let (outbox, replies) = channel::<Done<T>>();
        std::thread::Builder::new()
            .name("spotify-worker".into())
            .spawn(move || {
                while let Ok((tag, op)) = inbox.recv() {
                    let result = host.apply(op);
                    if outbox.send(Done { tag, result }).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self { requests, replies })
    }

    /// Queue a request. False when the thread has stopped.
    pub fn send(&self, tag: T, op: SpotifyOp) -> bool {
        self.requests.send((tag, op)).is_ok()
    }

    /// The next finished request, without waiting.
    pub fn try_recv(&self) -> Option<Done<T>> {
        match self.replies.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}
```

`Done<T>` is `Send` for the reply channel when `T` is: `SpotifyReply` owns plain data, and every `ZniczError` variant holds a `String`, an `io::Error` or a `SpotifyErrorKind`, all `Send` (checked in `znicz-core/src/error.rs`).

- [x] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui spotify_worker`
Expected: PASS.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/spotify_worker.rs znicz-tui/src/lib.rs
git commit -m "feat(tui): generic background worker for Spotify calls

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 6: App wiring, the status poll and the worker replies

Connects everything to the App without a panel yet. Adds the fields, the constructors, the second connection in `main.rs`, the Settings values, the tick hook, the status poll with the outdated-player stop, the reply handler, and the login-finished toast.

**Files:**
- Modify: `znicz-tui/src/app.rs` (`mod spotify_ui;` and a `pub(crate) use` of its types; fields; defaults in the `with_engine` literal; `with_remote`; `with_settings_values`; `with_spotify_worker`; `run_loop` calls `self.poll_spotify()` after `self.poll_install()`)
- Create: `znicz-tui/src/app/spotify_ui.rs`
- Modify: `znicz-tui/src/views/settings.rs` (`values()` reads `app.spotify_view()`)
- Modify: `znicz-tui/src/app/settings_ui.rs` (test helpers become `pub(crate)`; see Step 1)
- Modify: `znicz/src/main.rs` (see Step 3)

**Interfaces:**
- Consumes: `SpotifyWorker`, `SpotifyAsk`, `Done` (Task 5); `spotify_rows::{LoginOutcome, login_outcome}` (Task 2); `znicz_core::{SpotifyHost, SpotifyState, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyErrorKind, ZniczError}`; `znicz_spotify::browser::OpenBrowser`.
- Produces (App fields, all `pub(crate)`):
  - `spotify: SpotifyConfig` (the TUI's copy of `[spotify]`)
  - `open_browser: OpenBrowser`
  - `spotify_main: Option<Arc<dyn SpotifyHost>>`
  - `spotify_worker: Option<SpotifyWorker<SpotifyAsk>>`
  - `spotify_state: SpotifyState`
  - `spotify_devices: Vec<SpotifyDevice>`
  - `spotify_unavailable: Option<String>`
  - `spotify_polled_at: Option<Instant>`
  - `spotify_login: Option<LoginPanel>` (the panel's keys and drawing come in Tasks 9 and 10; in this task declare `pub(crate) struct LoginPanel { pub url: String, pub listening: bool, pub paste: Option<SpotifyPaste>, pub notice: Option<String> }` and `pub(crate) struct SpotifyPaste { pub text: LineEdit, pub error: Option<String> }` in `spotify_ui.rs`, so the field compiles here)
  - `spotify_confirm: Option<SpotifyConfirm>` (`pub(crate) enum SpotifyConfirm { Logout, ClientId(String) }`, declared in `spotify_ui.rs`)
  - `pub fn with_spotify_worker(self, host: Arc<dyn SpotifyHost>) -> std::io::Result<Self>` on App
  - `pub(crate) fn spotify_login_shown(&self) -> SpotifyLogin`
  - `pub(crate) fn spotify_view(&self) -> SpotifyView`
  - `pub(crate) fn poll_spotify(&mut self)`

- [ ] **Step 1: Write the failing tests**

In `znicz-tui/src/app/spotify_ui.rs`, the test module (the implementation comes in Step 3). The fake host and app helper:

```rust
#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use crossterm::event::KeyCode;
    use znicz_core::{
        SpotifyAccount, SpotifyConfig, SpotifyDevice, SpotifyErrorKind, SpotifyHost,
        SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState, ZniczError,
    };

    use super::*;
    use crate::app::App;
    use crate::app::settings_ui::tests as settings_tests;
    use crate::spotify_worker::{Done, SpotifyAsk, SpotifyWorker};

    /// A host with a scripted status and a log of the ops it was given.
    #[derive(Default)]
    pub(crate) struct FakeHost {
        pub state: Mutex<Option<SpotifyState>>,
        pub status_error: Mutex<Option<ZniczError>>,
        pub ops: Mutex<Vec<SpotifyOp>>,
        pub reply: Mutex<Option<SpotifyReply>>,
    }

    impl SpotifyHost for FakeHost {
        fn apply(&self, op: SpotifyOp) -> znicz_core::Result<SpotifyReply> {
            self.ops.lock().unwrap().push(op);
            Ok(self.reply.lock().unwrap().clone().unwrap_or(SpotifyReply::Done))
        }
        fn state(&self) -> SpotifyState {
            self.state.lock().unwrap().clone().unwrap_or_default()
        }
        fn try_state(&self) -> znicz_core::Result<SpotifyState> {
            if let Some(error) = self.status_error.lock().unwrap().take() {
                return Err(error);
            }
            Ok(self.state())
        }
    }

    fn spotify_error(kind: SpotifyErrorKind) -> ZniczError {
        ZniczError::Spotify { kind, message: "the player has no Spotify service".into() }
    }

    fn logged_in_state() -> SpotifyState {
        SpotifyState {
            login: SpotifyLogin::LoggedIn,
            account: Some(SpotifyAccount { user: "Eugene".into(), premium: Some(true) }),
            ..SpotifyState::default()
        }
    }

    fn app_with(dir: &tempfile::TempDir, host: Arc<FakeHost>) -> App {
        let mut app = settings_tests::settings_app(dir).with_settings_values(
            crate::settings::SettingsValues {
                spotify: SpotifyConfig {
                    client_id: "0123456789abcdef0123456789abcdef".into(),
                    ..SpotifyConfig::default()
                },
                ..crate::settings::SettingsValues::default()
            },
        );
        app.spotify_main = Some(host.clone());
        app.spotify_worker = Some(SpotifyWorker::spawn(host).unwrap());
        app
    }

    fn open_spotify_tab(app: &mut App) {
        app.toggle_settings_modal();
        app.set_settings_tab(crate::settings::Tab::Spotify);
    }

    fn wait_until(app: &mut App, done: impl Fn(&App) -> bool) {
        for _ in 0..500 {
            app.poll_spotify();
            if done(app) { return; }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the condition never held");
    }

    #[test]
    fn the_status_poll_runs_only_while_the_spotify_tab_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.state.lock().unwrap() = Some(logged_in_state());
        let mut app = app_with(&dir, host.clone());
        app.poll_spotify();
        assert!(app.spotify_polled_at.is_none());
        open_spotify_tab(&mut app);
        app.poll_spotify();
        assert_eq!(app.spotify_state.login, SpotifyLogin::LoggedIn);
        assert!(host.ops.lock().unwrap().contains(&SpotifyOp::Touch));
    }

    #[test]
    fn the_poll_waits_half_a_second_between_reads() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host.clone());
        open_spotify_tab(&mut app);
        app.poll_spotify();
        let first = app.spotify_polled_at;
        app.poll_spotify();
        assert_eq!(app.spotify_polled_at, first);
    }

    #[test]
    fn a_player_without_spotify_stops_the_poll_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.status_error.lock().unwrap() = Some(spotify_error(SpotifyErrorKind::NoService));
        let mut app = app_with(&dir, host.clone());
        open_spotify_tab(&mut app);
        app.poll_spotify();
        assert_eq!(
            app.spotify_unavailable.as_deref(),
            Some("the player has no Spotify service")
        );
        // Make the next poll overdue, so only the stop can hold it back.
        let overdue = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(5))
            .expect("the clock is past five seconds");
        app.spotify_polled_at = Some(overdue);
        app.poll_spotify();
        assert_eq!(app.spotify_polled_at, Some(overdue), "no reconnect on each frame");
    }

    #[test]
    fn a_network_error_on_the_poll_is_logged_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.status_error.lock().unwrap() = Some(spotify_error(SpotifyErrorKind::Network));
        let mut app = app_with(&dir, host);
        open_spotify_tab(&mut app);
        app.poll_spotify();
        assert!(app.spotify_unavailable.is_none());
    }

    #[test]
    fn the_login_panel_closes_and_toasts_when_the_login_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host.clone());
        open_spotify_tab(&mut app);
        app.spotify_login = Some(LoginPanel {
            url: "https://accounts.spotify.com/authorize".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        app.on_spotify_state(logged_in_state());
        assert!(app.spotify_login.is_none());
        assert!(settings_tests::toast_texts(&app).iter().any(|t| t == "Logged in as Eugene"));
    }

    #[test]
    fn a_cancelled_login_closes_the_panel_without_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        open_spotify_tab(&mut app);
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: true, paste: None, notice: None });
        app.on_spotify_state(SpotifyState::default());
        assert!(app.spotify_login.is_none());
        assert!(settings_tests::toast_texts(&app).is_empty());
    }

    #[test]
    fn a_device_list_arrives_from_the_worker() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(SpotifyReply::Devices {
            devices: vec![SpotifyDevice {
                id: "d1".into(),
                name: "Kitchen".into(),
                kind: "Speaker".into(),
                is_active: false,
                volume_percent: None,
            }],
        });
        let mut app = app_with(&dir, host);
        open_spotify_tab(&mut app);
        wait_until(&mut app, |app| !app.spotify_devices.is_empty());
        assert_eq!(app.spotify_devices[0].name, "Kitchen");
    }

    #[test]
    fn a_configure_error_is_a_toast() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let mut app = app_with(&dir, host);
        app.on_spotify_done(Done {
            tag: SpotifyAsk::Configure,
            result: Err(ZniczError::Spotify { kind: SpotifyErrorKind::Network, message: "timed out".into() }),
        });
        assert!(settings_tests::toast_texts(&app).iter().any(|t| t == "timed out"));
    }

    #[test]
    fn the_spotify_view_shows_not_logged_in_before_the_first_poll() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_with(&dir, Arc::new(FakeHost::default()));
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::NotLoggedIn);
        let unconfigured = settings_tests::settings_app(&dir);
        assert_eq!(unconfigured.spotify_login_shown(), SpotifyLogin::NotConfigured);
    }

    #[test]
    fn the_shown_login_follows_the_state_once_it_is_known_and_survives_a_tab_switch() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(&dir, Arc::new(FakeHost::default()));
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::LoggedIn);
        // Showing the tab polls again but keeps what was known.
        app.on_spotify_tab_shown();
        assert_eq!(app.spotify_login_shown(), SpotifyLogin::LoggedIn);
    }

    #[test]
    fn the_poll_also_runs_while_a_login_panel_is_open_on_another_tab() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.state.lock().unwrap() = Some(logged_in_state());
        let mut app = app_with(&dir, host);
        open_spotify_tab(&mut app);
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: true, paste: None, notice: None });
        app.set_settings_tab(crate::settings::Tab::Display);
        app.poll_spotify();
        assert!(app.spotify_login.is_none(), "the finished login closed the panel");
    }
}
```

Visibility for the tests: `spotify_ui::tests` is declared `pub(crate) mod tests` (above) because `settings_ui`'s tests use `FakeHost`. In the other direction, `settings_ui`'s test module becomes `pub(crate) mod tests`, and these helpers in it become `pub(crate) fn`: `settings_app`, `press`, `select`, `saved`, `type_text`, `toast_texts`, `screen_rows`, `screen_buffer`. `spotify_ui` is a private module of `app`, but `settings_ui` is its sibling, so `crate::app::spotify_ui::…` and `crate::app::settings_ui::tests::…` resolve from both test modules. The two test modules use each other's helpers, which is fine in Rust.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui spotify_ui`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

(a) `znicz-tui/src/app.rs`:

- Add `mod spotify_ui;` beside `mod settings_ui;` (line 37), and re-export the types the views need by name: `pub(crate) use spotify_ui::{LoginPanel, SpotifyConfirm, SpotifyPaste};`. `views/settings.rs` is outside `app`, so it cannot name `app::spotify_ui::…` (the module is private) but can name `crate::app::SpotifyConfirm`.
- Add the App fields (after `folder_prompt_from_settings`):

```rust
    /// `[spotify]` as the TUI has it. The player's copy is sent with `Configure`.
    pub(crate) spotify: SpotifyConfig,
    /// `[spotify] open_browser`; the TUI reads it, the player does not.
    pub(crate) open_browser: OpenBrowser,
    /// The status host: the main connection, which answers at once.
    pub(crate) spotify_main: Option<Arc<dyn SpotifyHost>>,
    /// The second connection, for every Spotify call that can be slow.
    pub(crate) spotify_worker: Option<SpotifyWorker<SpotifyAsk>>,
    /// The last status poll.
    pub(crate) spotify_state: SpotifyState,
    /// Devices from the last `SpotifyOp::Devices` reply.
    pub(crate) spotify_devices: Vec<SpotifyDevice>,
    /// The player does not answer Spotify requests; polling has stopped.
    pub(crate) spotify_unavailable: Option<String>,
    /// When the status was last polled.
    pub(crate) spotify_polled_at: Option<Instant>,
    /// The login panel, while a login runs.
    pub(crate) spotify_login: Option<LoginPanel>,
    /// A question in the Settings strip that waits for Enter.
    pub(crate) spotify_confirm: Option<SpotifyConfirm>,
```

- Add the imports: `use znicz_core::{SpotifyConfig, SpotifyDevice, SpotifyHost, SpotifyState};`, `use znicz_spotify::browser::OpenBrowser;`, `use crate::spotify_worker::{SpotifyAsk, SpotifyWorker};`, the re-export above brings `LoginPanel` and `SpotifyConfirm` into scope in `app.rs`.
- In the struct literal in `with_engine` (around line 738–770), add the defaults: `spotify: SpotifyConfig::default(), open_browser: OpenBrowser::default(), spotify_main: None, spotify_worker: None, spotify_state: SpotifyState::default(), spotify_devices: Vec::new(), spotify_unavailable: None, spotify_polled_at: None, spotify_login: None, spotify_confirm: None,`.
- `with_remote`:

```rust
    pub fn with_remote(player: IpcClient, library: Option<Library>) -> Self {
        let host: Arc<dyn LibraryHost> = Arc::new(player.clone());
        let status: Arc<dyn SpotifyHost> = Arc::new(player.clone());
        let mut app = Self::with_engine(Engine::Remote(player), library);
        app.library_host = Some(host);
        app.spotify_main = Some(status);
        app
    }
```

- `with_settings_values` copies: `self.spotify = values.spotify; self.open_browser = values.open_browser;`.
- Add:

```rust
    /// Start the second connection's worker. Spotify requests that can be slow
    /// run there, never on the UI thread.
    pub fn with_spotify_worker(mut self, host: Arc<dyn SpotifyHost>) -> std::io::Result<Self> {
        self.spotify_worker = Some(SpotifyWorker::spawn(host)?);
        Ok(self)
    }
```

- In `run_loop`, after `self.poll_install();` add `self.poll_spotify();`.
- In `on_esc` (Task 9 adds the panel branch; here add only the confirm branch): `if self.modal == Modal::Settings && self.spotify_confirm.is_some() { self.spotify_confirm = None; return; }`.
- In `close_modal`, add `self.spotify_confirm = None;` next to `self.settings_confirm = None;`.
- In `drop_removal_outside_settings` (settings_ui.rs), when the modal is not Settings also drop `spotify_confirm` and `spotify_login`.

(b) `znicz-tui/src/app/spotify_ui.rs` (the implementation above the test module):

```rust
//! The Spotify rows of the Settings modal: the status poll, the worker's
//! replies, the account and device actions, and the login panel. A second
//! `impl App` beside `settings_ui.rs`.

use std::time::{Duration, Instant};

use znicz_core::{SpotifyErrorKind, SpotifyLogin, SpotifyOp, SpotifyReply, SpotifyState, ZniczError};

use super::{App, Modal};
use crate::line_edit::LineEdit;
use crate::settings::Tab;
use crate::spotify_rows::{self, LoginOutcome, SpotifyView};
use crate::spotify_worker::{Done, SpotifyAsk};

// `SpotifyHost` is not imported: its methods are called through `Arc<dyn SpotifyHost>`.

/// How often the status is read while the Spotify tab is open.
const SPOTIFY_POLL_EVERY: Duration = Duration::from_millis(500);

/// The paste prompt inside the login panel.
pub(crate) struct SpotifyPaste {
    pub text: LineEdit,
    /// Spotify refused the address: the reason, shown under the prompt.
    pub error: Option<String>,
}

/// The login panel: the URL to open and, when the user pastes, the prompt.
pub(crate) struct LoginPanel {
    pub url: String,
    /// The callback listener is up, so the browser redirect can finish the login.
    pub listening: bool,
    pub paste: Option<SpotifyPaste>,
    /// One line about the browser, when it could not be opened.
    pub notice: Option<String>,
}

/// A change that waits for Enter in the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpotifyConfirm {
    Logout,
    /// The new client ID, which logs the user out.
    ClientId(String),
}

impl App {
    /// The login as the Settings rows show it. Without a client ID there is no
    /// login. The state's default is `NotConfigured`; with a client ID set that
    /// reads as not logged in until the first poll says otherwise. What an
    /// earlier poll learned stays when the tab is shown again.
    pub(crate) fn spotify_login_shown(&self) -> SpotifyLogin {
        if !self.spotify.is_configured() {
            return SpotifyLogin::NotConfigured;
        }
        match &self.spotify_state.login {
            SpotifyLogin::NotConfigured => SpotifyLogin::NotLoggedIn,
            login => login.clone(),
        }
    }

    /// Everything the Spotify rows show, copied for one frame.
    pub(crate) fn spotify_view(&self) -> SpotifyView {
        SpotifyView {
            configured: self.spotify.is_configured(),
            client_id_head: self.spotify.client_id.chars().take(8).collect(),
            device: self.spotify.device.clone(),
            redirect_port: self.spotify.redirect_port,
            open_browser: self.open_browser,
            login: self.spotify_login_shown(),
            account: self.spotify_state.account.clone(),
            devices: self.spotify_devices.clone(),
            unavailable: self.spotify_unavailable.clone(),
        }
    }

    /// Send a request to the worker. False when there is none.
    pub(crate) fn ask_spotify(&self, tag: SpotifyAsk, op: SpotifyOp) -> bool {
        self.spotify_worker.as_ref().is_some_and(|worker| worker.send(tag, op))
    }

    /// The status is worth reading: Settings shows the Spotify tab, or a login
    /// panel waits for the login to finish (the user may have changed tab).
    pub(crate) fn spotify_tab_open(&self) -> bool {
        self.modal == Modal::Settings
            && (self.settings_tab == Tab::Spotify || self.spotify_login.is_some())
    }

    /// The Spotify tab was shown: try the player again and refresh the device list.
    pub(crate) fn on_spotify_tab_shown(&mut self) {
        self.spotify_unavailable = None;
        self.spotify_polled_at = None;
        if self.spotify.is_configured() {
            self.ask_spotify(SpotifyAsk::Devices, SpotifyOp::Devices);
        }
    }

    /// Each tick: take the worker's replies, then read the status when it is due.
    pub(crate) fn poll_spotify(&mut self) {
        while let Some(done) = self.spotify_worker.as_ref().and_then(|w| w.try_recv()) {
            self.on_spotify_done(done);
        }
        if !self.spotify_tab_open() || self.spotify_unavailable.is_some() {
            return;
        }
        let due = self
            .spotify_polled_at
            .is_none_or(|at| at.elapsed() >= SPOTIFY_POLL_EVERY);
        if !due {
            return;
        }
        self.spotify_polled_at = Some(Instant::now());
        let Some(host) = self.spotify_main.clone() else {
            return;
        };
        // Both calls are on the main connection and answer at once. The status
        // goes first: an older player that does not know `Touch` then costs one
        // reconnect, and the poll stops.
        match host.try_state() {
            Ok(state) => {
                self.on_spotify_state(state);
                // Touch keeps the player's fast poll going.
                let _ = host.apply(SpotifyOp::Touch);
            }
            Err(ZniczError::Spotify {
                kind: SpotifyErrorKind::NoService | SpotifyErrorKind::OutdatedPlayer,
                message,
            }) => self.spotify_unavailable = Some(message),
            // The next poll shows the truth; do not log every 500 ms at warn.
            Err(e) => tracing::debug!("spotify status: {e}"),
        }
    }

    /// A status read. A login that left LoggingIn closes the panel and toasts.
    pub(crate) fn on_spotify_state(&mut self, state: SpotifyState) {
        self.spotify_state = state;
        let login = self.spotify_state.login.clone();
        if self.spotify_login.is_some() && login != SpotifyLogin::LoggingIn {
            self.spotify_login = None;
            match spotify_rows::login_outcome(&login, self.spotify_state.account.as_ref()) {
                Some(LoginOutcome::Success(text)) => self.toasts.success(text),
                Some(LoginOutcome::Failure(text)) => self.toasts.error(text),
                None => {}
            }
        }
    }

    /// A worker reply, by what asked for it.
    pub(crate) fn on_spotify_done(&mut self, done: Done<SpotifyAsk>) {
        match (done.tag, done.result) {
            (SpotifyAsk::BeginLogin, Ok(SpotifyReply::AuthorizeUrl { url, listening, .. })) => {
                self.open_login_panel(url, listening);
            }
            (SpotifyAsk::CompleteLogin, Err(ZniczError::Spotify { kind: SpotifyErrorKind::WrongAddress, .. })) => {
                if let Some(paste) = self.spotify_login.as_mut().and_then(|p| p.paste.as_mut()) {
                    paste.error = Some(spotify_rows::WRONG_ADDRESS.into());
                }
            }
            // The panel closes when the status shows the login finished.
            (SpotifyAsk::CompleteLogin, Ok(_)) | (SpotifyAsk::CancelLogin, _) => {}
            (SpotifyAsk::Logout, Ok(_)) => self.toasts.success("Logged out of Spotify"),
            (SpotifyAsk::Devices, Ok(SpotifyReply::Devices { devices })) => {
                self.spotify_devices = devices;
            }
            // Devices is read when the tab opens; without a login it fails, and
            // the account row already says so.
            (SpotifyAsk::Devices, Err(e)) => tracing::warn!("spotify devices: {e}"),
            (_, Err(e)) => self.toasts.error(e.to_string()),
            (_, Ok(_)) => {}
        }
    }
}
```

The panel's key handlers and the browser logic are added in Task 9. For this task, add a stub `fn open_login_panel(&mut self, url: String, listening: bool)` that only sets `self.spotify_login = Some(LoginPanel { url, listening, paste: None, notice: None })` and `self.spotify_state.login = SpotifyLogin::LoggingIn`. It has the signature Task 9 keeps; Task 9 replaces the body. In `on_spotify_done` the call is `self.open_login_panel(url, listening);`.

In `views/settings.rs`, `values()`: replace the Task 3 placeholder with `spotify: app.spotify_view(),`.

Two things in `on_spotify_done`: the `CompleteLogin` `Err(WrongAddress)` arm comes before `(_, Err(e))`, so it is not toasted. Good.

(c) `znicz/src/main.rs`:

- The `extras.settings` literal already carries the two Spotify values (Task 3).
- The three call sites of `run_tui_with_client` (`run_tui`, `play_station_and_run`, `load_playlist_and_run`; about lines 923, 961, 1002): each gets `let spotify = connect_ui(device, config)?;` right after its own `connect_ui` and before the call, and passes it: `run_tui_with_client(player, spotify, library_path, …)`. `device: Option<&str>` and `config: Option<&Path>` are parameters of all three. `connect_ui` is the same helper the main connection uses, so the second connection is a separate `IpcClient` with its own socket and lock, not a clone of the first. It has the `Ui` role, like the main one.
- `run_tui_with_client` takes `spotify: IpcClient` as its second parameter and after `App::with_remote(player, library)` does `let mut app = App::with_remote(player, library).with_spotify_worker(Arc::new(spotify)).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;` (the `Arc::new(spotify)` coerces to `Arc<dyn SpotifyHost>`). Add `use znicz_core::SpotifyHost` only if the coercion needs it in scope (it does not).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- spotify_ui settings_ui && cargo build --workspace`
Expected: PASS; the binary builds.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_ui.rs znicz-tui/src/app/settings_ui.rs znicz-tui/src/views/settings.rs znicz/src/main.rs
git commit -m "feat(tui): Spotify status poll and worker replies in the App

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Open browser, playback device and port saves through `Configure`

Makes Open browser, Playback device and Redirect port work end to end: stepping, saving, sending `Configure` for the player-side ones, and restoring a Spotify row whose save was refused (broken config, no config file path, failed write). The Client ID prompt and the Account row come in Task 8, so no row in this task points at code a later task adds.

**Files:**
- Modify: `znicz-tui/src/app/settings_ui.rs`
- Modify: `znicz-tui/src/app/settings_ui.rs` `set_settings_tab`, `switch_settings_tab`, `toggle_settings_modal`: call `self.on_spotify_tab_shown()` when the Spotify tab is shown.

**Interfaces:**
- Consumes: `spotify_rows::{device_choices, OPEN_BROWSERS, open_browser_name, parse_port, sent_to_player}`; `settings::step`; `SpotifyAsk::Configure`.
- Produces: in `settings_ui.rs`: `fn open_text_prompt(&mut self, row: &Row)`, `fn submit_port_prompt(&mut self)`, `fn send_spotify_config(&self, row: &Row)`, `fn set_redirect_port(&mut self, port: u16) -> bool`, `fn step_playback_device(&mut self, forward: bool, wrap: bool) -> bool`, `pub(super) fn change_with` (made `pub(super)` for `spotify_ui.rs`; `save_setting` stays private).

- [ ] **Step 1: Write the failing tests**

In `settings_ui.rs`'s test module:

```rust
    fn spotify_app(dir: &tempfile::TempDir, host: Arc<crate::app::spotify_ui::tests::FakeHost>) -> App {
        let mut app = settings_app(dir).with_settings_values(SettingsValues {
            spotify: SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                ..SpotifyConfig::default()
            },
            ..SettingsValues::default()
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_main = Some(host);
        app
    }

    fn sent_ops(host: &crate::app::spotify_ui::tests::FakeHost) -> Vec<SpotifyOp> {
        // The worker is asynchronous: wait until it has been quiet for one
        // sample (at least 50 ms, at most 2 s), then read what it was given.
        let mut last = usize::MAX;
        for _ in 0..80 {
            std::thread::sleep(Duration::from_millis(25));
            let now = host.ops.lock().unwrap().len();
            if now == last {
                break;
            }
            last = now;
        }
        host.ops.lock().unwrap().clone()
    }

    #[test]
    fn enter_steps_the_open_browser_setting_and_saves_it_without_sending_it() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::OpenBrowser);
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&dir)["spotify"]["open_browser"].as_str(), Some("always"));
        assert!(sent_ops(&host).iter().all(|op| !matches!(op, SpotifyOp::Configure { .. })));
    }

    #[test]
    fn stepping_the_device_saves_it_and_sends_configure() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_devices = vec![SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }];
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&dir)["spotify"]["device"].as_str(), Some("Kitchen"));
        let ops = sent_ops(&host);
        assert!(ops.iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.device == "Kitchen"
        )));
    }

    #[test]
    fn choosing_the_active_device_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "[spotify]\ndevice = \"Kitchen\"\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir).with_settings_values(SettingsValues {
            spotify: SpotifyConfig {
                client_id: "0123456789abcdef0123456789abcdef".into(),
                device: "Kitchen".into(),
                ..SpotifyConfig::default()
            },
            ..SettingsValues::default()
        });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        app.spotify_devices = vec![SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }];
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Left);
        assert!(saved(&dir)["spotify"].get("device").is_none());
        assert!(app.spotify.device.is_empty());
    }

    #[test]
    fn a_port_typed_in_the_prompt_is_saved_and_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        // Replace the default 8898 with 9001.
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&dir)["spotify"]["redirect_port"].as_integer(), Some(9001));
        assert!(sent_ops(&host).iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.redirect_port == 9001
        )));
    }

    #[test]
    fn a_port_out_of_range_keeps_the_prompt_open() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "80");
        press(&mut app, KeyCode::Enter);
        let prompt = app.settings_prompt.as_ref().expect("still open");
        assert_eq!(prompt.error.as_deref(), Some("1024–65535"));
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn a_broken_config_is_not_written_and_not_sent_and_the_row_goes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\nclient_id = 1\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir)
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse {
                message: "bad".into(),
                line: 1,
                column: 1,
            })
            .with_settings_values(SettingsValues {
                spotify: SpotifyConfig {
                    client_id: "0123456789abcdef0123456789abcdef".into(),
                    ..SpotifyConfig::default()
                },
                ..SettingsValues::default()
            });
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::OpenBrowser);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.open_browser, OpenBrowser::Auto);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[spotify\nclient_id = 1\n");
        assert!(!sent_ops(&host).iter().any(|op| matches!(op, SpotifyOp::Configure { .. })));
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("Not saved")));
    }

    #[test]
    fn the_first_save_creates_the_file_and_sends_what_the_player_already_has_plus_the_change() {
        // No config file yet: the app and the player both run on the defaults.
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        assert!(!dir.path().join("config.toml").exists());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::RedirectPort);
        press(&mut app, KeyCode::Enter);
        for _ in 0..4 {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "9001");
        press(&mut app, KeyCode::Enter);
        assert_eq!(saved(&dir)["spotify"]["redirect_port"].as_integer(), Some(9001));
        let sent = sent_ops(&host);
        assert!(sent.iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config }
                if config.redirect_port == 9001 && config.client_id.is_empty() && config.device.is_empty()
        )));
    }

    #[test]
    fn without_a_config_file_path_a_spotify_row_goes_back_and_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.config_file = None;
        app.spotify_devices = vec![SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }];
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::PlaybackDevice);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify.device.is_empty());
        assert!(!sent_ops(&host).iter().any(|op| matches!(op, SpotifyOp::Configure { .. })));
        assert!(toast_texts(&app).iter().any(|t| t.starts_with("Not saved")));
    }
```

`ConfigProblem::Parse { message, line, column }` is the real shape (`config_file.rs`). `Enter` on `OpenBrowser` steps `auto` to `always`; the broken config refuses the save, so the restore puts `Auto` back. `OpenBrowser` derives `PartialEq`.

Add `use znicz_core::{SpotifyConfig, SpotifyDevice, SpotifyOp}; use znicz_spotify::browser::OpenBrowser; use crate::spotify_worker::SpotifyWorker;` to the test module imports.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui settings_ui`
Expected: FAIL (the new rows do nothing yet).

- [ ] **Step 3: Implement**

In `settings_ui.rs`:

1. `on_settings_key`, the Enter arm: add the rows before the slider arm:

```rust
                Row::RedirectPort => self.open_text_prompt(&row),
```

Task 8 adds `Row::ClientId` to that arm and the `Row::Account` arm. The `Change::Activate` path for `OpenBrowser` and `PlaybackDevice` is already the `_ => self.change_setting(&row, Change::Activate)` arm.

2. `apply_setting`, Choice arm: before the `CoverProtocol` branch add

```rust
                if *row == Row::OpenBrowser {
                    let new = settings::step(&spotify_rows::OPEN_BROWSERS, &self.open_browser, forward, wrap);
                    let changed = new != self.open_browser;
                    self.open_browser = new;
                    return changed;
                }
                if *row == Row::PlaybackDevice {
                    return self.step_playback_device(forward, wrap);
                }
```

and add `Kind::Text | Kind::Number` to the `Kind::Action | Kind::Folder => false` arm.

3. Add:

```rust
    /// Step the Playback device choice through the active device, the listed
    /// devices and the configured one. Without a client ID the row does nothing.
    fn step_playback_device(&mut self, forward: bool, wrap: bool) -> bool {
        if !self.spotify.is_configured() {
            return false;
        }
        let view = self.spotify_view();
        let choices = spotify_rows::device_choices(&view);
        let new = settings::step(&choices, &self.spotify.device, forward, wrap);
        let changed = new != self.spotify.device;
        self.spotify.device = new;
        changed
    }

    /// Set the redirect port. Returns whether it changed.
    fn set_redirect_port(&mut self, port: u16) -> bool {
        let changed = self.spotify.redirect_port != port;
        self.spotify.redirect_port = port;
        changed
    }

    /// Enter on the client ID or redirect port: a prompt with the current value.
    fn open_text_prompt(&mut self, row: &Row) {
        let text = match row {
            Row::ClientId => self.spotify.client_id.clone(),
            Row::RedirectPort => self.spotify.redirect_port.to_string(),
            _ => return,
        };
        self.settings_prompt = Some(SliderPrompt {
            row: row.clone(),
            text: LineEdit::from_text(text),
            error: None,
        });
    }

    /// Enter in the port prompt: a number in range saves; anything else keeps the prompt open.
    fn submit_port_prompt(&mut self) {
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        match spotify_rows::parse_port(prompt.text.as_str()) {
            Err(range) => prompt.error = Some(range),
            Ok(port) => {
                self.settings_prompt = None;
                self.change_with(&Row::RedirectPort, |app| app.set_redirect_port(port));
            }
        }
    }

    /// Send the player its copy of `[spotify]` after a player-side row saved.
    /// Called only from `save_setting`, after the file was written. An empty
    /// client ID in that copy logs the user out and deletes the token, so the
    /// copy is never sent while the config has a problem, even if a caller
    /// forgets the order.
    fn send_spotify_config(&self, row: &Row) {
        if self.config_problem.is_some() || !spotify_rows::sent_to_player(row) {
            return;
        }
        self.ask_spotify(
            SpotifyAsk::Configure,
            SpotifyOp::Configure { config: self.spotify.clone() },
        );
    }
```

4. `submit_slider_prompt`: at the top, dispatch by row:

```rust
        if self.settings_prompt.as_ref().is_some_and(|p| p.row == Row::RedirectPort) {
            return self.submit_port_prompt();
        }
```

Task 8 adds the client ID to this dispatch.

5. `stored_value`: add the four rows (the values are in the design notes above):

```rust
            Row::ClientId if self.spotify.client_id.is_empty() => Stored::Remove,
            Row::ClientId => Stored::Text(self.spotify.client_id.clone()),
            Row::RedirectPort => Stored::Int(i64::from(self.spotify.redirect_port)),
            Row::OpenBrowser => Stored::Str(spotify_rows::open_browser_name(self.open_browser)),
            Row::PlaybackDevice if self.spotify.device.is_empty() => Stored::Remove,
            Row::PlaybackDevice => Stored::Text(self.spotify.device.clone()),
```

(These go inside the `Some(match row {…})` after the existing arms.)

6. `save_setting`: the Spotify rows must never reach the player unless the file was written. Add `let spotify = row.tab() == Tab::Spotify;` after the `config_key` check, then three changes:
   - No config file path: for a Spotify row, `self.restore_settings(before); self.toasts.warn("Not saved: no config file path"); return;` before the existing one-time "run only" toast, which stays for the other rows. Nothing is sent.
   - Config problem: compute `text` as now, then `if spotify { self.restore_settings(before); }` before `self.toasts.warn(text); return;`. Nothing is sent. (`text` is an owned `String`, so the borrow of `self.config_problem` has ended.)
   - After the write: `match written { Err(e) => {…existing…} Ok(()) => self.send_spotify_config(row) }`. A failed write restores through the existing branch and sends nothing.

   So `Configure` leaves the TUI in exactly one place, after `ConfigFile` returned `Ok`, with `config_problem` empty. The first save, when `config.toml` does not exist, is such a write: the file is created and the player gets the copy it already runs on plus the one change.

7. `settings_snapshot` gets `spotify: self.spotify.clone(), open_browser: self.open_browser`; `SettingsSnapshot` gets the two fields; `restore_settings` puts both back. Restoring the config does not send Configure (the player was not told).

8. Show the Spotify tab: in `toggle_settings_modal`, `set_settings_tab` and `switch_settings_tab` (the last one calls `set_settings_tab`, so only `toggle_settings_modal` and `set_settings_tab` need the hook). At the end of `set_settings_tab`: `if tab == Tab::Spotify { self.on_spotify_tab_shown(); }`. At the end of `toggle_settings_modal` after `self.modal = Modal::Settings;`: `if self.settings_tab == Tab::Spotify { self.on_spotify_tab_shown(); }`.

9. Make `change_with` `pub(super)` (Task 8 calls it from `spotify_ui.rs`).

Imports in `settings_ui.rs`: `use znicz_core::{Command, FadeTimes, LibraryOp, SpotifyConfig, SpotifyOp}; use znicz_spotify::browser::OpenBrowser; use crate::spotify_rows; use crate::spotify_worker::SpotifyAsk; use crate::settings::{…}` (add nothing else).

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- settings_ui spotify_ui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app/settings_ui.rs
git commit -m "feat(tui): save Spotify browser, device and port settings

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 8: The client ID prompt, the logout question and the Account action

Clearing or changing the client ID makes the player log the user out (and delete the token), so it never happens without the user's Enter: while logged in, the new ID waits behind a question in the strip. Nothing is saved or sent before that Enter.

**Files:**
- Modify: `znicz-tui/src/app/settings_ui.rs`
- Modify: `znicz-tui/src/app/spotify_ui.rs`

**Interfaces:**
- Consumes: `spotify_rows::{client_id_problem, CLIENT_ID_QUESTION, LOGOUT_QUESTION, CLIENT_FIRST}`, `SpotifyConfirm`.
- Produces: `pub(crate) fn spotify_account_enter(&mut self)` (spotify_ui.rs), `pub(crate) fn confirm_spotify(&mut self)` (spotify_ui.rs), `fn submit_client_id_prompt(&mut self)` and `fn set_client_id(&mut self, id: String) -> bool` (settings_ui.rs).

- [ ] **Step 1: Write the failing tests**

In `settings_ui.rs`'s tests:

```rust
    const ID: &str = "0123456789abcdef0123456789abcdef";
    const NEW_ID: &str = "fedcba9876543210fedcba9876543210";

    #[test]
    fn a_client_id_that_is_not_32_hex_keeps_the_prompt_open_with_why() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "xyz");
        press(&mut app, KeyCode::Enter);
        let prompt = app.settings_prompt.as_ref().expect("still open");
        assert_eq!(prompt.error.as_deref(), Some("32 hex characters"));
        assert_eq!(app.spotify.client_id, ID);
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn clearing_the_client_id_while_logged_in_asks_first_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), format!("[spotify]\nclient_id = \"{ID}\"\n")).unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify_confirm, Some(SpotifyConfirm::ClientId(String::new())));
        assert_eq!(app.spotify.client_id, ID);
        assert!(saved(&dir)["spotify"].get("client_id").is_some(), "the file still has the ID");
        assert!(!sent_ops(&host).iter().any(|op| matches!(op, SpotifyOp::Configure { .. })));
    }

    #[test]
    fn an_empty_client_id_clears_the_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), format!("[spotify]\nclient_id = \"{ID}\"\n")).unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        press(&mut app, KeyCode::Enter);
        assert!(app.settings_prompt.is_none());
        assert!(app.spotify.client_id.is_empty());
        assert!(saved(&dir)["spotify"].get("client_id").is_none());
    }

    #[test]
    fn a_new_client_id_while_logged_in_asks_first_and_says_it_logs_out() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::ClientId);
        press(&mut app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, NEW_ID);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify_confirm, Some(SpotifyConfirm::ClientId(NEW_ID.into())));
        assert_eq!(app.spotify.client_id, ID, "nothing changes before Enter");
        assert!(!dir.path().join("config.toml").exists(), "nothing is written before Enter");
        assert!(!sent_ops(&host).iter().any(|op| matches!(op, SpotifyOp::Configure { .. })));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify.client_id, NEW_ID);
        assert_eq!(saved(&dir)["spotify"]["client_id"].as_str(), Some(NEW_ID));
        assert!(sent_ops(&host).iter().any(|op| matches!(
            op,
            SpotifyOp::Configure { config } if config.client_id == NEW_ID
        )));
    }

    #[test]
    fn esc_cancels_a_pending_client_id_change() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host);
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_confirm = Some(SpotifyConfirm::ClientId(NEW_ID.into()));
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_confirm.is_none());
        assert_eq!(app.spotify.client_id, ID);
        assert!(app.modal == Modal::Settings, "Esc only cancels the question");
    }

    #[test]
    fn enter_on_account_without_a_client_id_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = settings_app(&dir);
        app.spotify_worker = Some(SpotifyWorker::spawn(host.clone()).unwrap());
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_login.is_none());
        assert!(app.spotify_confirm.is_none());
        assert!(sent_ops(&host).iter().all(|op| !matches!(op, SpotifyOp::BeginLogin)));
    }

    #[test]
    fn enter_on_account_when_logged_out_begins_a_login() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::NotLoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert!(sent_ops(&host).contains(&SpotifyOp::BeginLogin));
    }

    #[test]
    fn enter_on_account_when_logged_in_asks_then_logs_out() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_state.login = SpotifyLogin::LoggedIn;
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.spotify_confirm, Some(SpotifyConfirm::Logout));
        assert!(!sent_ops(&host).contains(&SpotifyOp::Logout));
        press(&mut app, KeyCode::Enter);
        assert!(sent_ops(&host).contains(&SpotifyOp::Logout));
    }
```

The `select`, `press`, `saved` and `type_text` helpers are the ones made `pub(crate)` in Task 6; the test module imports `Row` (`crate::settings::Row`), `SpotifyLogin` and `SpotifyConfirm` (`crate::app::SpotifyConfirm`). The screen text of the question is checked in Task 10, where the strip is drawn.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui -- settings_ui`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `settings_ui.rs`:

0. `on_settings_key`'s Enter arm: `Row::ClientId | Row::RedirectPort => self.open_text_prompt(&row),` and `Row::Account => self.spotify_account_enter(),`. `submit_slider_prompt`'s dispatch also sends `Row::ClientId` to `submit_client_id_prompt`: replace its `is_some_and` test with `match self.settings_prompt.as_ref().map(|p| p.row.clone()) { Some(Row::ClientId) => return self.submit_client_id_prompt(), Some(Row::RedirectPort) => return self.submit_port_prompt(), _ => {} }`.

1. Add:

```rust
    /// Enter in the client ID prompt. The value is trimmed and checked; a
    /// refused value keeps the prompt open with the reason. A changed ID while
    /// logged in waits for Enter in the strip, because the player logs out.
    fn submit_client_id_prompt(&mut self) {
        let Some(prompt) = self.settings_prompt.as_mut() else {
            return;
        };
        let text = prompt.text.as_str().trim().to_string();
        if let Some(problem) = spotify_rows::client_id_problem(&text) {
            prompt.error = Some(problem.into());
            return;
        }
        self.settings_prompt = None;
        if text == self.spotify.client_id {
            return;
        }
        if self.spotify_login_shown() == SpotifyLogin::LoggedIn {
            self.spotify_confirm = Some(SpotifyConfirm::ClientId(text));
            return;
        }
        self.change_with(&Row::ClientId, |app| app.set_client_id(text));
    }

    /// Set the client ID. Returns whether it changed.
    pub(super) fn set_client_id(&mut self, id: String) -> bool {
        let changed = self.spotify.client_id != id;
        self.spotify.client_id = id;
        changed
    }
```

2. `on_settings_key`: before the `if self.settings_confirm.is_some() && key.code == KeyCode::Enter` line (after the row lookup, so the question is about the row under the cursor), add

```rust
        if self.spotify_confirm.is_some() && key.code == KeyCode::Enter {
            self.confirm_spotify();
            return true;
        }
```

3. `set_settings_cursor` also clears `spotify_confirm`: add `self.spotify_confirm = None;` beside `self.settings_confirm = None;`. The `Up`-to-the-tab-bar arm in `on_settings_key` clears it too, next to its `self.settings_confirm = None;`.

4. `on_settings_key` already routes Esc through `on_esc` (Task 6 added the confirm branch).

5. `spotify_confirm` lives in the strip: the strip text is handled in Task 10.

In `spotify_ui.rs`, add:

```rust
    /// Enter on Account. Without a client ID nothing happens (the strip says
    /// so). Logged in: ask before logging out. Otherwise start the login.
    pub(crate) fn spotify_account_enter(&mut self) {
        if !self.spotify.is_configured() {
            return;
        }
        if self.spotify_login_shown() == SpotifyLogin::LoggedIn {
            self.spotify_confirm = Some(SpotifyConfirm::Logout);
        } else {
            self.ask_spotify(SpotifyAsk::BeginLogin, SpotifyOp::BeginLogin);
        }
    }

    /// Enter on the strip's question: log out, or save the new client ID.
    pub(crate) fn confirm_spotify(&mut self) {
        match self.spotify_confirm.take() {
            Some(SpotifyConfirm::Logout) => {
                self.ask_spotify(SpotifyAsk::Logout, SpotifyOp::Logout);
            }
            Some(SpotifyConfirm::ClientId(id)) => {
                self.change_with(&Row::ClientId, |app| app.set_client_id(id));
            }
            None => {}
        }
    }
```

Add `use crate::settings::Row;` to `spotify_ui.rs` and `use super::settings_ui`-visible `change_with` (made `pub(super)` in Task 7; `spotify_ui` is a sibling in `app`, so `pub(super)` from `settings_ui` is `app`-visible — check the path: `pub(super)` in `app::settings_ui` means visible in `app`, which includes `app::spotify_ui`).

Note: `change_with` is inside `impl App` in `settings_ui.rs` with `pub(super)` visibility, so `spotify_ui.rs` can call `self.change_with(...)`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- settings_ui spotify_ui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app/settings_ui.rs znicz-tui/src/app/spotify_ui.rs
git commit -m "feat(tui): client ID prompt, logout question and the Account action

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 9: The login panel, its keys, copy and the browser rule

**Files:**
- Modify: `znicz-tui/src/app/spotify_ui.rs`
- Modify: `znicz-tui/src/app/settings_ui.rs` (`on_settings_key` routes panel keys; `drop_removal_outside_settings` drops the panel)
- Modify: `znicz-tui/src/app.rs` (the `clipboard` and `browser` fields; `on_esc` branch for the panel; `context()`)
- Modify: `znicz-tui/src/keys.rs` (`Context::SettingsLogin`, its hints, the help lines)

**Interfaces:**
- Consumes: `znicz_spotify::browser::{should_open, open}`, `crate::osc52::copy`, `LoginPanel`, `SpotifyPaste`, `spotify_rows::{login_panel_lines, WRONG_ADDRESS}`.
- Produces:
  - App fields `clipboard` and `browser`: `Box<dyn Fn(&str) -> std::io::Result<()> + Send>`, defaulting to `osc52::copy` and `znicz_spotify::browser::open`. Tests swap them, so no test writes an escape sequence to the developer's terminal or starts a browser.
  - `fn open_login_panel(&mut self, url: String, listening: bool)` (same signature as Task 6's stub; the body now applies the browser rule), `pub(crate) fn on_spotify_panel_key(&mut self, key: KeyEvent) -> bool`, `pub(crate) fn esc_spotify_panel(&mut self)`, `fn copy_login_url(&mut self)`, `fn submit_paste(&mut self)`.
  - `Context::SettingsLogin` and the hints `c copy URL`, `Enter paste`, `Esc cancel`; the help lines for the panel in `keys::SETTINGS`.

- [ ] **Step 1: Write the failing tests**

In `spotify_ui.rs`'s tests (the panel and key tests belong with the code in this task):

```rust
    /// What the app handed to the clipboard and to the browser opener.
    #[derive(Clone, Default)]
    struct Recorded {
        copied: Arc<Mutex<Vec<String>>>,
        opened: Arc<Mutex<Vec<String>>>,
    }

    /// An app on the Spotify tab whose clipboard and browser are recorders.
    /// The host reports `LoggingIn`, as the player does while a panel is open,
    /// so a status poll during a test does not close the panel.
    fn panel_app(dir: &tempfile::TempDir, host: Arc<FakeHost>) -> (App, Recorded) {
        *host.state.lock().unwrap() = Some(SpotifyState {
            login: SpotifyLogin::LoggingIn,
            ..SpotifyState::default()
        });
        let mut app = app_with(dir, host);
        let recorded = Recorded::default();
        let copied = recorded.copied.clone();
        app.clipboard = Box::new(move |text| {
            copied.lock().unwrap().push(text.to_string());
            Ok(())
        });
        let opened = recorded.opened.clone();
        app.browser = Box::new(move |url| {
            opened.lock().unwrap().push(url.to_string());
            Ok(())
        });
        open_spotify_tab(&mut app);
        (app, recorded)
    }

    #[test]
    fn begin_login_opens_the_panel_with_the_url_and_the_listening_line() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        *host.reply.lock().unwrap() = Some(SpotifyReply::AuthorizeUrl {
            url: "https://accounts.spotify.com/authorize?x=1".into(),
            redirect_uri: "http://127.0.0.1:8898/callback".into(),
            listening: true,
        });
        let (mut app, recorded) = panel_app(&dir, host.clone());
        app.open_browser = OpenBrowser::Always;
        app.ask_spotify(SpotifyAsk::BeginLogin, SpotifyOp::BeginLogin);
        wait_until(&mut app, |app| app.spotify_login.is_some());
        let panel = app.spotify_login.as_ref().unwrap();
        assert_eq!(panel.url, "https://accounts.spotify.com/authorize?x=1");
        assert!(panel.listening);
        assert!(panel.notice.is_none());
        assert_eq!(app.spotify_state.login, SpotifyLogin::LoggingIn);
        assert_eq!(recorded.opened.lock().unwrap().as_slice(), ["https://accounts.spotify.com/authorize?x=1"]);
    }

    #[test]
    fn a_failed_browser_open_is_one_line_and_the_panel_stays() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.open_browser = OpenBrowser::Always;
        app.browser = Box::new(|_| Err(std::io::Error::other("no opener")));
        app.open_login_panel("https://accounts.spotify.com/authorize?x=1".into(), false);
        let panel = app.spotify_login.as_ref().expect("the panel waits");
        assert_eq!(
            panel.notice.as_deref(),
            Some("Could not open a browser: no opener. Use the URL below.")
        );
        assert!(!panel.listening);
    }

    #[test]
    fn the_browser_rule_decides_whether_the_opener_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.open_browser = OpenBrowser::Never;
        app.open_login_panel("u".into(), true);
        assert!(recorded.opened.lock().unwrap().is_empty());
        assert!(app.spotify_login.as_ref().unwrap().notice.is_none());
    }

    #[test]
    fn enter_in_the_panel_opens_the_paste_prompt_and_a_pasted_address_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host.clone());
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: false, paste: None, notice: None });
        press(&mut app, KeyCode::Enter);
        assert!(app.spotify_login.as_ref().unwrap().paste.is_some());
        for c in "code=x&state=y".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        press(&mut app, KeyCode::Enter);
        wait_until(&mut app, |_| {
            host.ops.lock().unwrap().iter().any(|op| matches!(op, SpotifyOp::CompleteLogin { .. }))
        });
        assert!(host.ops.lock().unwrap().contains(&SpotifyOp::CompleteLogin {
            redirect_url: "code=x&state=y".into()
        }));
    }

    #[test]
    fn a_wrong_address_keeps_the_paste_prompt_open_with_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host);
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: false,
            paste: Some(SpotifyPaste { text: LineEdit::from_text("code=x&state=bad"), error: None }),
            notice: None,
        });
        app.on_spotify_done(Done {
            tag: SpotifyAsk::CompleteLogin,
            result: Err(spotify_error(SpotifyErrorKind::WrongAddress)),
        });
        let paste = app.spotify_login.as_ref().unwrap().paste.as_ref().unwrap();
        assert_eq!(paste.error.as_deref(), Some("that address is not from this login"));
    }

    #[test]
    fn esc_in_the_paste_prompt_returns_to_the_panel_and_esc_in_the_panel_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeHost::default());
        let (mut app, _recorded) = panel_app(&dir, host.clone());
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(SpotifyPaste { text: LineEdit::new(), error: None }),
            notice: None,
        });
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_login.is_some());
        assert!(app.spotify_login.as_ref().unwrap().paste.is_none());
        press(&mut app, KeyCode::Esc);
        assert!(app.spotify_login.is_none());
        wait_until(&mut app, |_| host.ops.lock().unwrap().contains(&SpotifyOp::CancelLogin));
        assert_eq!(app.modal, Modal::Settings, "Esc cancels the login, not Settings");
    }

    #[test]
    fn c_hands_the_url_to_the_clipboard_writer_and_says_it_was_sent() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel { url: "https://x/y?z=1".into(), listening: true, paste: None, notice: None });
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(recorded.copied.lock().unwrap().as_slice(), ["https://x/y?z=1"]);
        assert!(settings_tests::toast_texts(&app).iter().any(|t| t.starts_with("Sent the login URL")));
    }

    #[test]
    fn a_failed_copy_is_an_error_toast() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.clipboard = Box::new(|_| Err(std::io::Error::other("closed")));
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: true, paste: None, notice: None });
        press(&mut app, KeyCode::Char('c'));
        assert!(settings_tests::toast_texts(&app).iter().any(|t| t.starts_with("Could not copy the URL")));
    }

    #[test]
    fn keys_the_panel_does_not_own_fall_through_so_s_still_closes_settings() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: true, paste: None, notice: None });
        press(&mut app, KeyCode::Char('S'));
        assert_eq!(app.modal, Modal::None);
        assert!(app.spotify_login.is_none(), "the panel goes with Settings");
    }

    #[test]
    fn in_the_paste_prompt_every_key_types() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(SpotifyPaste { text: LineEdit::new(), error: None }),
            notice: None,
        });
        for c in "S?qc6".chars() {
            press(&mut app, KeyCode::Char(c));
        }
        let paste = app.spotify_login.as_ref().unwrap().paste.as_ref().unwrap();
        assert_eq!(paste.text.as_str(), "S?qc6");
        assert_eq!(app.modal, Modal::Settings);
        assert!(!app.should_quit);
    }

    #[test]
    fn the_footer_and_help_know_the_panel() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: true, paste: None, notice: None });
        assert_eq!(app.context(), crate::keys::Context::SettingsLogin);
        app.spotify_login.as_mut().unwrap().paste = Some(SpotifyPaste { text: LineEdit::new(), error: None });
        assert_eq!(app.context(), crate::keys::Context::SettingsPrompt);
        let lines = crate::keys::SETTINGS;
        assert!(lines.iter().any(|b| b.keys == "1–6"));
        assert!(lines.iter().any(|b| b.keys == "Login: c"));
        assert!(lines.iter().any(|b| b.keys == "Login: Enter"));
        assert!(lines.iter().any(|b| b.keys == "Login: Esc"));
    }

    #[test]
    fn the_panel_is_dropped_when_settings_is_no_longer_on_screen() {
        let dir = tempfile::tempdir().unwrap();
        let (mut app, _recorded) = panel_app(&dir, Arc::new(FakeHost::default()));
        app.spotify_login = Some(LoginPanel { url: "u".into(), listening: true, paste: None, notice: None });
        app.modal = Modal::Help;
        app.drop_removal_outside_settings();
        assert!(app.spotify_login.is_none());
    }
```

The helpers (`press`, `toast_texts`, `settings_app`) were made `pub(crate)` in Task 6. The test module of `spotify_ui.rs` also needs `use znicz_spotify::browser::OpenBrowser; use crate::line_edit::LineEdit; use crate::app::Modal;` (`LoginPanel`, `SpotifyPaste`, `Done` come in through `use super::*` and Task 6's imports). `press` goes through `App::on_key`, so these tests exercise the real routing: Esc through `on_esc`, `c` and Enter through `on_settings_key`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui -- spotify_ui`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `spotify_ui.rs`:

1. Replace the body of the Task 6 stub `open_login_panel` (same signature) with:

```rust
    /// The URL came back from BeginLogin: show the panel, and open the browser
    /// when the setting and this terminal's environment say so (`self.browser`
    /// is `znicz_spotify::browser::open` in production).
    fn open_login_panel(&mut self, url: String, listening: bool) {
        self.spotify_state.login = SpotifyLogin::LoggingIn;
        let mut notice = None;
        let env = |key: &str| std::env::var(key).ok();
        if znicz_spotify::browser::should_open(self.open_browser, &env) {
            if let Err(e) = (self.browser)(&url) {
                notice = Some(format!("Could not open a browser: {e}. Use the URL below."));
            }
        }
        self.spotify_login = Some(LoginPanel { url, listening, paste: None, notice });
    }
```

`on_spotify_done` already calls `self.open_login_panel(url, listening)`.

2. Add the key handling. The panel owns `c`, `Enter` and `Esc` (Esc arrives through `on_esc`). Every other key returns false, so it falls through to the global keys: `S` and `?` still work, and the login keeps running in the player. In the paste prompt every key types.

```rust
    /// Keys while the login panel is open. Returns false for keys the panel
    /// does not own.
    pub(crate) fn on_spotify_panel_key(&mut self, key: KeyEvent) -> bool {
        let pasting = self.spotify_login.as_ref().is_some_and(|p| p.paste.is_some());
        if pasting {
            if key.code == KeyCode::Enter {
                self.submit_paste();
            } else if let Some(paste) = self.spotify_login.as_mut().and_then(|p| p.paste.as_mut()) {
                paste.text.on_key(key);
                paste.error = None;
            }
            return true;
        }
        match key.code {
            KeyCode::Char('c') => self.copy_login_url(),
            KeyCode::Enter => {
                if let Some(panel) = self.spotify_login.as_mut() {
                    panel.paste = Some(SpotifyPaste { text: LineEdit::new(), error: None });
                }
            }
            _ => return false,
        }
        true
    }

    /// Esc with the panel open: a paste prompt closes back to the panel;
    /// otherwise the login is cancelled and the panel closes.
    pub(crate) fn esc_spotify_panel(&mut self) {
        let Some(panel) = self.spotify_login.as_mut() else {
            return;
        };
        if panel.paste.take().is_some() {
            return;
        }
        self.spotify_login = None;
        self.ask_spotify(SpotifyAsk::CancelLogin, SpotifyOp::CancelLogin);
    }

    /// `c`: write the OSC 52 sequence through `self.clipboard`. Tells the user
    /// it was sent, not that it arrived: the terminal does not say.
    fn copy_login_url(&mut self) {
        let Some(url) = self.spotify_login.as_ref().map(|p| p.url.clone()) else {
            return;
        };
        match (self.clipboard)(&url) {
            Ok(()) => self.toasts.info("Sent the login URL to the terminal's clipboard"),
            Err(e) => self.toasts.error(format!("Could not copy the URL: {e}")),
        }
    }

    /// Enter in the paste prompt: send the pasted address. An empty address does nothing.
    fn submit_paste(&mut self) {
        let Some(paste) = self.spotify_login.as_mut().and_then(|p| p.paste.as_mut()) else {
            return;
        };
        let text = paste.text.as_str().trim().to_string();
        if text.is_empty() {
            return;
        }
        paste.error = None;
        self.ask_spotify(SpotifyAsk::CompleteLogin, SpotifyOp::CompleteLogin { redirect_url: text });
    }
```

Add `use crossterm::event::{KeyCode, KeyEvent};` to `spotify_ui.rs`. Pasting into the prompt arrives as one key event per character (the TUI does not turn on bracketed paste), and each is typed by `LineEdit::on_key`; a pasted trailing newline is an Enter and sends the address.

3. `on_settings_key` (settings_ui.rs): right after the `if self.settings_prompt.is_some() { … }` block, add:

```rust
        if self.spotify_login.is_some() {
            return self.on_spotify_panel_key(key);
        }
```

4. `app.rs`:
   - Two fields, after `spotify_confirm`:

```rust
    /// Writes the clipboard sequence to the terminal. Tests replace it.
    pub(crate) clipboard: Box<dyn Fn(&str) -> std::io::Result<()> + Send>,
    /// Opens the login page in a browser. Tests replace it.
    pub(crate) browser: Box<dyn Fn(&str) -> std::io::Result<()> + Send>,
```

     with the defaults `clipboard: Box::new(crate::osc52::copy), browser: Box::new(znicz_spotify::browser::open),` in the `with_engine` literal.
   - `on_esc`: add before the existing `settings_prompt` check. The panel and a slider prompt cannot be open together:

```rust
        if self.modal == Modal::Settings && self.spotify_login.is_some() {
            self.esc_spotify_panel();
            return;
        }
```

   - `context()`: before the `Modal::Settings if self.settings_prompt.is_some()` arm add `Modal::Settings if self.spotify_login.as_ref().is_some_and(|p| p.paste.is_some()) => Context::SettingsPrompt,` and `Modal::Settings if self.spotify_login.is_some() => Context::SettingsLogin,`.

5. `drop_removal_outside_settings` (settings_ui.rs): inside the `if self.modal != Modal::Settings` block add `self.spotify_login = None; self.spotify_confirm = None;` (Task 6 may already have added the `spotify_confirm` half; keep one copy). Update its doc comment: "The login panel goes too. The login itself keeps running in the player."

6. `keys.rs`:
   - `Context::SettingsLogin` in the enum, with a doc line: "Settings with the Spotify login panel open: `c`, Enter and Esc are the panel's."
   - Hints: `const COPY_URL: Hint = hint("c copy URL", KeyCode::Char('c'));` and `const PASTE_ADDRESS: Hint = hint("Enter paste", KeyCode::Enter);`. In `context_hints`: `Context::SettingsLogin => &[COPY_URL, PASTE_ADDRESS, CANCEL],`.
   - `context_title`: add `Context::SettingsLogin` to the arm that returns `Some("Settings")`, so help opens on the Settings section.
   - `more_hint` needs no change (`?` is a global key while the panel waits).
   - `SETTINGS` (the help table) gets three lines after the `1–6` line: `b("Login: c", "copy the login URL")`, `b("Login: Enter", "paste the redirect address")`, `b("Login: Esc", "cancel the login")`. Help is generated from this table, so the panel keys are listed there.
   - In the keys tests, `ALL_CONTEXTS` becomes `[Context; 19]` with `Context::SettingsLogin` after `SettingsPrompt`. `every_context_fits_a_wide_footer_with_more` then covers the new hints.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- spotify_ui settings_ui keys`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app/spotify_ui.rs znicz-tui/src/app/settings_ui.rs znicz-tui/src/app.rs znicz-tui/src/keys.rs
git commit -m "feat(tui): Spotify login panel with copy, paste and cancel

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 10: Drawing the Spotify tab and the login panel

**Files:**
- Modify: `znicz-tui/src/views/settings.rs`
- Modify: `znicz-tui/src/app/settings_ui.rs` (tests only)

**Interfaces:**
- Consumes: `spotify_rows::{CLIENT_FIRST, LOGOUT_QUESTION, CLIENT_ID_QUESTION, login_panel_lines, WRONG_ADDRESS}`, `settings::{dimmed, prompt_range}`, `crate::app::{LoginPanel, SpotifyConfirm}` (the re-export from Task 6).
- Produces: none (drawing only).

- [ ] **Step 1: Write the failing tests**

In `views/settings.rs`, add a test module section (or in `settings_ui.rs`'s tests, which already have `screen_rows` and `screen_buffer`). Put these in `settings_ui.rs`'s test module, since it owns the helpers:

```rust
    #[test]
    fn the_spotify_tab_shows_its_five_rows_and_the_tab_label() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        let screen = screen_rows(&mut app).join("\n");
        for text in ["Client ID", "Redirect port", "Open browser", "Account", "Playback device", "Spotify"] {
            assert!(screen.contains(text), "{text} missing:\n{screen}");
        }
    }

    #[test]
    fn without_a_client_id_the_strip_points_at_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        select(&mut app, Row::Account);
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Set a client ID first"), "{screen}");
    }

    #[test]
    fn the_login_panel_shows_the_url_and_the_keys_in_the_list_area() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "https://accounts.spotify.com/authorize?client_id=0123".into(),
            listening: true,
            paste: None,
            notice: None,
        });
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Open this URL in any browser and approve:"), "{screen}");
        assert!(screen.contains("accounts.spotify.com/authorize"), "{screen}");
        assert!(screen.contains("Waiting for the browser"), "{screen}");
    }

    #[test]
    fn the_panel_draws_in_a_small_terminal_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "https://accounts.spotify.com/authorize?".repeat(8),
            listening: false,
            paste: None,
            notice: Some("Could not open a browser: no opener.".into()),
        });
        let _ = screen_buffer(&mut app);
    }

    #[test]
    fn a_long_url_wraps_in_the_panel_and_none_of_it_is_lost() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        let url = format!("https://accounts.spotify.com/authorize?{}", "a1b2c3d4".repeat(12));
        app.spotify_login = Some(crate::app::LoginPanel {
            url,
            listening: true,
            paste: None,
            notice: None,
        });
        let screen = screen_rows(&mut app);
        // The URL is one word longer than the popup is wide: it must run over
        // several rows (broken at the width), not be cut at the first.
        let rows_with_url = screen
            .iter()
            .filter(|row| row.contains("a1b2c3d4a1b2c3d4"))
            .count();
        assert!(rows_with_url >= 2, "{}", screen.join("\n"));
    }

    #[test]
    fn the_paste_prompt_shows_in_the_strip_with_the_reason_when_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_login = Some(crate::app::LoginPanel {
            url: "u".into(),
            listening: true,
            paste: Some(crate::app::SpotifyPaste {
                text: crate::line_edit::LineEdit::from_text("code=x"),
                error: Some("that address is not from this login".into()),
            }),
            notice: None,
        });
        let screen = screen_rows(&mut app).join("\n");
        assert!(screen.contains("Address: code=x"), "{screen}");
        assert!(screen.contains("that address is not from this login"), "{screen}");
    }

    #[test]
    fn the_logout_question_and_the_client_id_question_show_in_the_strip() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = settings_app(&dir);
        press(&mut app, KeyCode::Char('S'));
        press(&mut app, KeyCode::Char('6'));
        app.spotify_confirm = Some(crate::app::SpotifyConfirm::Logout);
        assert!(screen_rows(&mut app).join("\n").contains("Log out of Spotify?"));
        app.spotify_confirm = Some(crate::app::SpotifyConfirm::ClientId("x".into()));
        assert!(screen_rows(&mut app).join("\n").contains("Changing the client ID logs you out"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p znicz-tui -- settings_ui`
Expected: FAIL.

- [ ] **Step 3: Implement**

In `views/settings.rs`:

1. (`values(app)` already reads `app.spotify_view()` since Task 6.)
2. `render`: after `let list_area`/`rule`/`strip` are computed and before the rows are drawn, branch:
   - If `app.spotify_login.is_some()`: draw the panel body into `list_area` with `Paragraph::new(lines).wrap(Wrap { trim: false })` (ratatui 0.30's `ratatui::widgets::Wrap`; with `trim: false` a word longer than the width, such as the URL, is broken at the width instead of being cut off; the strip's own `wrap()` helper would truncate it, so it is not used here), where `lines` is `spotify_rows::login_panel_lines(&panel.url, panel.listening)` as `Line`s, followed by a `notice` line in `theme::warn()` when present. Skip the row lines.
   - Otherwise draw the rows as now.
3. The strip text, in priority order:
   - `settings_prompt` open: `slider_prompt_line(...)`, generalized (step 4).
   - `spotify_login` with `paste` open: `views::prompt_line("Address: ", &paste.text)`, plus `"  " + WRONG_ADDRESS` in `theme::warn()` when `paste.error` is set.
   - `spotify_login` open without paste: `"c copy · Enter paste · Esc cancel the login"` (dim).
   - `spotify_confirm` is `Logout`: `LOGOUT_QUESTION`; `ClientId(_)`: `CLIENT_ID_QUESTION` (dim, wrapped as the existing removal question is).
   - `settings_confirm` (folder): unchanged.
   - Dimmed row under the cursor (`settings::dimmed(row, &values)`): `CLIENT_FIRST`.
   - Otherwise the existing tab-bar / row explanation.
4. `row_line`: when `settings::dimmed(row, values)`, draw the label and value in `theme::dim()` instead of `theme::text()`. Keep the highlight style for the cursor row.
5. `slider_prompt_line`: label and range come from `settings::prompt_range(row)`:

```rust
fn slider_prompt_line(row: &Row, text: &LineEdit, error: Option<&str>) -> Line<'static> {
    let range = settings::prompt_range(row).unwrap_or_default();
    let mut line = views::prompt_line(&format!("{} ({range}): ", row.label()), text);
    if let Some(range) = error {
        line.spans.push(Span::styled(format!("  must be {range}"), theme::warn()));
    }
    line
}
```

For the port, the strip reads "Redirect port (1024–65535): …" and the error "must be 1024–65535". For the client ID: "Client ID (32 hex characters, or empty): …", error "must be 32 hex characters".

- Import `ratatui::widgets::Wrap` (the file imports `Clear, Paragraph` today) and `crate::spotify_rows` and `crate::app::SpotifyConfirm`.

In `settings.rs` (`value_text` etc.) nothing more.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p znicz-tui -- settings_ui settings::tests`
Expected: PASS. Also run the render test: `cargo test -p znicz-tui --test render`.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/views/settings.rs znicz-tui/src/app/settings_ui.rs
git commit -m "feat(tui): draw the Spotify tab, its strip and the login panel

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 11: The config-error rule, checked end to end

Part 2's lesson: the player deletes the user's Spotify token when a `Configure` carries an empty client ID. So the TUI must never send `Configure` from a config that did not load cleanly, and never send it at all when the change was not written. Tasks 7 and 8 build that; this task proves it across rows with tests that read the worker's host, the only place a `Configure` can show up. The help and footer keys were tested in Task 9.

**Files:**
- Modify: `znicz-tui/src/app/settings_ui.rs` (tests only)

**Interfaces:**
- Consumes: the whole of Tasks 1–10.
- Produces: integration tests for the rules the brief says must hold.

- [ ] **Step 1: Write the tests**

In `settings_ui.rs`'s tests:

```rust
    /// Every Spotify change a user can make, one after the other.
    fn change_every_spotify_row(app: &mut App) {
        press(app, KeyCode::Char('S'));
        press(app, KeyCode::Char('6'));
        select(app, Row::OpenBrowser);
        press(app, KeyCode::Enter);
        select(app, Row::PlaybackDevice);
        press(app, KeyCode::Enter);
        select(app, Row::RedirectPort);
        press(app, KeyCode::Enter);
        for _ in 0..4 {
            press(app, KeyCode::Backspace);
        }
        type_text(app, "9001");
        press(app, KeyCode::Enter);
        select(app, Row::ClientId);
        press(app, KeyCode::Enter);
        for _ in 0..ID.len() {
            press(app, KeyCode::Backspace);
        }
        type_text(app, NEW_ID);
        press(app, KeyCode::Enter);
    }

    fn kitchen() -> Vec<SpotifyDevice> {
        vec![SpotifyDevice {
            id: "d1".into(),
            name: "Kitchen".into(),
            kind: "Speaker".into(),
            is_active: false,
            volume_percent: None,
        }]
    }

    #[test]
    fn a_broken_config_never_changes_the_player_through_any_spotify_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[spotify\n").unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone())
            .with_config_file(ConfigFile::new(&path))
            .with_config_problem(ConfigProblem::Parse { message: "bad".into(), line: 1, column: 1 });
        app.spotify_devices = kitchen();
        change_every_spotify_row(&mut app);
        assert!(
            !sent_ops(&host).iter().any(|op| matches!(op, SpotifyOp::Configure { .. })),
            "nothing reaches the player"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[spotify\n");
        assert_eq!(app.spotify.redirect_port, SpotifyConfig::DEFAULT_PORT);
        assert_eq!(app.spotify.client_id, ID, "the row shows what the player has");
        assert!(app.spotify.device.is_empty());
        assert_eq!(app.open_browser, OpenBrowser::Auto);
    }

    #[test]
    fn a_clean_config_sends_one_configure_per_player_side_change_and_never_an_empty_id_unasked() {
        let dir = tempfile::tempdir().unwrap();
        let host = Arc::new(crate::app::spotify_ui::tests::FakeHost::default());
        let mut app = spotify_app(&dir, host.clone());
        app.spotify_devices = kitchen();
        change_every_spotify_row(&mut app);
        let configures: Vec<_> = sent_ops(&host)
            .into_iter()
            .filter_map(|op| match op {
                SpotifyOp::Configure { config } => Some(config),
                _ => None,
            })
            .collect();
        // Device, port, client ID: three saves. The browser setting is not sent.
        assert_eq!(configures.len(), 3, "{configures:?}");
        assert!(configures.iter().all(|c| !c.client_id.is_empty()), "an ID was never blanked");
        assert_eq!(configures.last().unwrap().client_id, NEW_ID);
        assert_eq!(configures.last().unwrap().redirect_port, 9001);
        assert_eq!(configures.last().unwrap().device, "Kitchen");
    }
```

`ID` and `NEW_ID` are the constants from Task 8; move them to the top of the test module if they are not already visible here. `spotify_app` builds an app with a client ID, so the `ClientId` change here starts from a set ID while logged out (`login_shown` is `NotLoggedIn`), and saves without the question.

- [ ] **Step 2: Run the tests**

Run: `cargo test -p znicz-tui -- settings_ui`
Expected: PASS. These tests check Tasks 7 and 8 together, so they may pass at once. If either fails, fix the behaviour in `save_setting` or `send_spotify_config` (Task 7), not the test.

- [ ] **Step 3: Run the whole crate**

Run: `cargo test -p znicz-tui`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add znicz-tui/src/app/settings_ui.rs
git commit -m "test(tui): the Spotify rules hold end to end

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 12: Wiki

Updates `wiki/Architecture/TUI.md` and `wiki/Architecture/Spotify.md` in the same change, as `wiki-sync.mdc` requires.

**Files:**
- Modify: `wiki/Architecture/TUI.md` (the Settings section at lines 273–355; the Modules list at line 556; the Keys section at line 106 if it lists Settings keys)
- Modify: `wiki/Architecture/Spotify.md` (the Login section at line 159; the Errors section at line 191)

**Interfaces:** none.

- [ ] **Step 1: Read the sections**

Read `wiki/Architecture/TUI.md` lines 106–141 and 273–355, and `wiki/Architecture/Spotify.md` lines 159–203, to match the existing wording.

- [ ] **Step 2: Update TUI.md**

In the Settings section:
- Say Settings has six tabs (Display, Audio, Playback, Library, General, Spotify) and that `1`–`6` select them. The page now says "five tabs" (line 275) and shows `1`–`5` in the key table (line 290) and in the paragraph after it (line 301); change all three.
- Add the Spotify tab's five rows with what each shows and does: Client ID (32 hex characters, empty clears, the strip points at developer.spotify.com/dashboard), Redirect port (1024–65535, default 8898, the row shows `http://127.0.0.1:<port>/callback`), Open browser (`auto` / `always` / `never`), Account (its states, Enter logs in, Enter when logged in asks in the strip, `Enter` logs out, `Esc` cancels), Playback device (Active device or a listed device, offline devices marked, refreshed when the tab opens).
- Say that Account and Playback device are dimmed until a client ID is set.
- Describe the login panel: the URL, `c` copies it with OSC 52 (terminals that support it; in tmux, `set-clipboard on`), `Enter` opens the paste prompt, a wrong address keeps the prompt open, `Esc` in the prompt returns to the panel, `Esc` in the panel cancels the login, and the panel closes by itself when the login ends.
- Describe the browser rule in one paragraph: the front end decides from its own environment (SSH variables mean no; Linux needs `DISPLAY` or `WAYLAND_DISPLAY`); `open_browser` overrides it; a failed open shows one line and the panel keeps waiting.
- State that every Spotify call except the status read and `Touch` runs on a background thread with its own connection to the player (a second `IpcClient`, never a clone of the main one), so no frame waits for Spotify; that the status is read at most every 500 ms, only while the Spotify tab (or a login panel) is open, and stops for good when the player has no Spotify service or is too old; and that the thread ends when the TUI closes.
- State the save rule: a Spotify row is written to `config.toml` in place, and only then is the player told (`Configure`). With a broken config, or no config path, the row goes back and nothing is sent, because an empty client ID would log the user out. Changing the client ID while logged in asks first.
- Say that `c` writes the OSC 52 sequence between frames and that the terminal gives no sign it was accepted, which is why the toast says "Sent".
- In the Modules list, add `spotify_rows.rs`, `spotify_worker.rs`, `osc52.rs`, and `app/spotify_ui.rs`, each with one line.

Use simple English. No invented features.

- [ ] **Step 3: Update Spotify.md**

In the Login section, add the TUI route: Settings → Spotify, Enter on Account, the panel, copying, the paste route, and the same browser rule. In the Errors section, add that the TUI shows `Spotify is unavailable: …` in the Account row when the player has no Spotify service or is an older version, and that polling stops then until the tab is opened again.

- [ ] **Step 4: Verify**

Run: `grep -n '1–5\|`1`–`5`\|five tabs' wiki/Architecture/TUI.md wiki/Architecture/Spotify.md wiki/Home.md`
Expected: no lines that still describe five tabs. (The wiki writes the range as `` `1`–`5` ``, so a search for plain `1–5` alone would miss it.) The README's Settings row (line 219) names the tabs; it is a final-PR item with the rest of the README, as the spec says.

- [ ] **Step 5: Commit**

```bash
git add wiki/Architecture/TUI.md wiki/Architecture/Spotify.md
git commit -m "docs(wiki): Spotify Settings tab and login panel

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

---

### Task 13: Full check

No code change expected. Runs the whole gate. If something fails, fix it in the task that owns it, with a `fix(spotify):` commit, then run this task again.

**Files:** none (unless a fix is needed).

- [ ] **Step 1: Format**

Run: `cargo fmt --all -- --check`
Expected: no output. If it reports differences, run `cargo fmt --all` and commit with `style(tui): format the Spotify Settings code`.

- [ ] **Step 2: Clippy with warnings denied**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no errors. Common findings to expect: `needless_borrow` on `&*host`, `manual_let_else`, `match_like_matches_macro` in `settings.rs` (`hint_text`), `unnecessary_wraps` on `spotify_rows::parse_port`. Fix each in place.

- [ ] **Step 3: Tests**

Run: `cargo test --workspace`
Expected: all pass.

- [ ] **Step 4: Wiki and keys agreement**

Run: `grep -n '1–6\|`1`–`6`' znicz-tui/src/keys.rs wiki/Architecture/TUI.md`
Expected: both name the range.

- [ ] **Step 5: Commit the ticked plan**

Mark each step in this plan as done (`- [x]`) and commit only the plan:

```bash
git add docs/superpowers/plans/2026-11-26-spotify-4-tui-settings.md
git commit -m "docs(plan): tick Spotify part 4 full check

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

If Steps 1–3 needed fixes, those fixes were committed separately before this one.

---

### Task 14: Manual check

Run with a real Spotify Premium account and a Development Mode app. Set the client ID in Settings, not by hand in `config.toml`, to check the whole route. Redirect URI `http://127.0.0.1:8898/callback` is registered and the account is added under User Management. No code is written in this task. Note any surprise and fix it in its own commit.

- [ ] **Step 1: Client ID**

Open Settings (`S`), press `6`, Enter on Client ID. Type a 31-character string and press Enter.
Expected: the prompt stays open with `must be 32 hex characters`. Clear it and type the real 32-character ID, Enter.
Check: `config.toml` has `[spotify] client_id = "…"`, the row shows the first eight characters then `…`, and the Account row shows `Not logged in`.

- [ ] **Step 2: Redirect port**

Enter on Redirect port. Type `80` and Enter: the prompt stays open with `must be 1024–65535`. Type `8898` and Enter.
Expected: the row shows `http://127.0.0.1:8898/callback`.

- [ ] **Step 3: Login by redirect**

Enter on Account. Expected: the panel shows the URL and `Waiting for the browser; or paste the address you were sent to.`, and a browser opens (on a desktop session). Approve in the browser.
Expected: the panel closes and the toast reads `Logged in as <name>`. The Account row shows `Logged in as <name> · Premium`, or `not Premium (control unavailable)` after a refused playback call.

- [ ] **Step 4: Login by paste, and `c` copy**

Log out first (Enter on Account, Enter on the question). Enter on Account. Press `c`.
Expected: the toast says the URL was sent to the clipboard. Paste somewhere to confirm it is the authorize URL (the terminal must support OSC 52).
Press Enter. Paste a wrong address (for example `code=x&state=wrong`) and press Enter.
Expected: `that address is not from this login` under the prompt, and the prompt stays open.
Paste the real address from the browser's "can't connect" page and press Enter.
Expected: the panel closes and the toast reads `Logged in as <name>`.

- [ ] **Step 5: Login over SSH**

From a machine on SSH into the test machine: run the TUI there. Enter on Account.
Expected: no browser opens (SSH variables are set). The panel says `Paste the address you were sent to.` or the listening line if the listener is up over `ssh -L 8898:127.0.0.1:8898`. Approve on the local machine. With the tunnel, the callback finishes the login; without it, paste.

- [ ] **Step 6: `c` in tmux**

Start the TUI inside tmux with `set -g set-clipboard on`. Open the panel and press `c`. Paste into another application.
Expected: the URL is pasted. If it is not, record the tmux version and whether `set-clipboard` is on. This is the open question from Spec refinements. If the copy fails only in tmux, the fix is the DCS passthrough wrapper in `osc52.rs` (`\x1bPtmux;` … `\x1b\\`), gated on the `TMUX` environment variable. Plan that as its own task before closing this check.

- [ ] **Step 7: Logout with confirmation**

Enter on Account. The strip reads `Log out of Spotify? Enter logs out · Esc cancels`. Press Esc: nothing changes. Enter on Account, Enter on the question.
Expected: the toast reads `Logged out of Spotify`, and the Account row shows `Not logged in`.

- [ ] **Step 8: Playback device**

With the tab open, the device list is refreshed. Step Playback device with `l` and `h`.
Expected: `Active device` first, then each device by name. Pick one. `config.toml` has `device = "<name>"`, and a `Configure` reaches the player (a later `spotify_devices` MCP call or `znicz spotify status` shows it as the preferred device). Choose `Active device` again: the `device` key is removed.

- [ ] **Step 9: Typo in config.toml**

Add a deliberate typo to `config.toml` (for example `[spotify` with no closing bracket), open Settings, press `6`, and change the port with Enter and a typed value.
Expected: the toast reads `Not saved: config.toml has an error (…)`, the file is unchanged, the row goes back to its old value, and `znicz spotify status` shows the player's settings as they were before the change. Fix the typo and restart the TUI.

- [ ] **Step 10: The first save, with no config file, and a client ID change**

Move `config.toml` aside and start the TUI (a player that is already running keeps its old settings, so stop it first with `znicz player stop`). Open Settings, press `6`, change Open browser.
Expected: `config.toml` is created with `[spotify] open_browser = "…"`, and nothing else changes: the player is not told, and a login that was valid stays valid.
Put the file back. Log in, then change the client ID to another valid one.
Expected: the strip asks first and says it logs you out; `Esc` leaves everything as it was; `Enter` saves, logs you out, and the Account row reads `Not logged in`.

- [ ] **Step 11: Commit**

Mark the checks that passed, then commit:

```bash
git add docs/superpowers/plans/2026-11-26-spotify-4-tui-settings.md
git commit -m "docs(plan): Spotify part 4 manual check passed

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>"
```

If a check found a bug, fix it in its own commit with `fix(spotify): <what the manual check found>` and the trailer, run Task 13, and then commit the ticked plan.

---

## Review Focus

The five input classes or failure modes most likely to bite a person using this tab, most likely first. Each has a test in its owning task.

1. **A client ID pasted with spaces, a newline or uppercase hex.** Accepted after trimming; uppercase is valid hex. Test: `a_client_id_is_32_hex_characters_or_empty` (Task 2).
2. **A port typed as `80`, `0`, `65536`, `http` or nothing.** Refused with the range shown, the prompt stays open, and nothing is written. Tests: `a_port_is_1024_to_65535` (Task 2), `a_port_out_of_range_keeps_the_prompt_open` (Task 7).
3. **A broken `config.toml` while the user changes a Spotify row.** Nothing is written, nothing reaches the player, and the row goes back. Tests: `a_broken_config_is_not_written_and_not_sent_and_the_row_goes_back` (Task 7), `a_broken_config_never_changes_the_player_through_the_spotify_tab` (Task 11).
4. **A Spotify call that takes 40 seconds, or a player with no Spotify service.** The frame still draws (the worker never blocks `try_recv`), and an old player stops the poll and says why. Tests: `a_slow_op_does_not_block_try_recv` (Task 5), `a_player_without_spotify_stops_the_poll_and_says_why` (Task 6).
5. **A pasted address that is wrong, empty or from an old login.** A wrong address keeps the prompt open with the reason; an empty paste sends nothing. Tests: `a_wrong_address_keeps_the_paste_prompt_open_with_the_reason` (Task 9), and the empty case in `submit_paste` (Task 9, checked by manual step 4).

Also covered: a long URL on a narrow terminal (`the_panel_draws_in_a_small_terminal_without_panicking`, Task 10).

---

## Self-review against the spec and the brief

| Brief item | Where |
| --- | --- |
| 1. Tab 6 "Spotify" after General; `1`–`6`; every tab-count assumption | Task 1 (settings, settings_ui right-arrow end, tab-bar help, keys, tests) |
| 1. Client ID: `line_edit` prompt, `valid_client_id`, empty clears, strip says where to get one | Task 2 (`client_id_problem`), Task 3 (explanation), Task 8 (prompt submit), Task 10 (prompt label) |
| 1. Redirect port: number prompt 1024–65535, default 8898, row shows `http://127.0.0.1:<port>/callback`, strip says add the URI | Task 2 (`parse_port`, `redirect_uri`), Task 3 (value and explanation), Task 7 (prompt) |
| 1. Open browser `auto` / `always` / `never` | Task 2 (`OPEN_BROWSERS`), Task 7 (step and save) |
| 1. Account states including `Logged in as <name>` with plan suffix, `Failed: <reason>`, Enter logs in, logout confirm in strip | Task 2 (`account_text`), Task 8 (`spotify_account_enter`, `confirm_spotify`), Task 10 (question text) |
| 1. Playback device: Active device plus devices from `SpotifyState.devices`; configured-but-offline dim `offline`; refreshed on tab open | Task 2 (`device_choices`, `device_online`), Task 3 (hint), Task 6 (`on_spotify_tab_shown`, `Devices`) |
| 1. Dimmed rows until a client ID is set; strip points at Client ID | Task 3 (`dimmed`), Task 10 (`CLIENT_FIRST`) |
| 2. Save through `ConfigFile`; never write a broken config; same toast | Task 7 (`save_setting`: restore on problem, on no file path, on a failed write) |
| 2. `SpotifyOp::Configure` only from a config that loaded; no send on problem, no file path or failed write; first save with no file | Task 7 (`send_spotify_config` after `Ok` only, and itself refuses while `config_problem` is set), tests `a_broken_config…`, `the_first_save_creates_the_file…`, `without_a_config_file_path…`; Task 11 across all rows |
| 2. Changing or clearing `client_id` logs out; strip says so before confirming; nothing saved or sent before Enter | Task 8 (`SpotifyConfirm::ClientId`, tests for change and clear), Task 10 (the question is drawn) |
| 2. `open_browser` front-end only, never sent | Task 7 (`open_browser` not in `sent_to_player`), test `enter_steps_the_open_browser_setting…` |
| 3. Login panel text with `listening` third line | Task 2 (`login_panel_lines`), Task 10 (drawn) |
| 3. OSC 8 or plain, stated | Spec refinements; Task 10 draws plain text |
| 3. `c` copies with OSC 52 `\x1b]52;c;<base64>\x07`, written between frames; tmux check; tests never write to the real terminal | Task 4 (`sequence`, padding and alphabet tests), Task 9 (`copy_login_url` through `App::clipboard`), Task 14 Step 6 |
| 3. Browser opens by `should_open`; failure is one line and waits | Task 9 (`open_login_panel`, `notice`), test `a_failed_browser_open…` |
| 3. `Enter` opens paste prompt; `CompleteLogin`; `wrong_address` message; `Esc` sends `CancelLogin` | Task 9 |
| 3. Panel closes when status leaves `LoggingIn`; toast `Logged in as` or failure | Task 6 (`on_spotify_state`), test `the_login_panel_closes_and_toasts…` |
| 4. Second `IpcClient`, created like the main one | Task 6 Step 3(c) (`connect_ui` in each `run_tui` path) |
| 4. General `SpotifyWorker` with ops in, replies and errors out, tagged; own `IpcClient`; ends when the App drops | Task 5 (incl. `dropping_the_worker_ends_its_thread`), Task 6 Step 3(c) |
| 4. `try_state` polled on the main client, only with the tab open, at most every 500 ms; `Touch` too | Task 6 (`poll_spotify`) |
| 4. `no_service` / `outdated_player` stops polling and shows `Spotify is unavailable: …` | Task 6 (`spotify_unavailable`), test `a_player_without_spotify…` |
| 4. `network` on a change shows the message; next poll shows the truth | Task 6 (`on_spotify_done` toasts), Task 5 (worker keeps running) |
| 5. `wiki/Architecture/TUI.md` Settings section and `Spotify.md` login route | Task 12 |
| 5. Help screen lists tab keys and login keys; footer hints for the panel | Task 1 (`keys::SETTINGS`, `1–6`), Task 9 (`Context::SettingsLogin`, its hints, the `Login: …` help lines, test `the_footer_and_help_know_the_panel`) |
| 6. Manual check: client ID, redirect, paste, SSH, `c` in terminal and tmux, logout confirm, device, typo | Task 14 Steps 1–9 |
| 7. Out of scope: Spotify mode (Part 5), shared Stage, version bump, CHANGELOG, README | Not planned here (see Spec refinements) |
| Pure logic with unit tests: row values, step choices, Account labels, panel text, OSC 52 encoding, port check, device choice list | Task 2 (tests), Task 3 (values), Task 4 (OSC 52) |
| Commit message `docs(plan): Spotify part 4, TUI Settings and login` | Written when this plan is committed |

**Placeholder scan:** no `TBD`. Tests and signatures are written out. The real shapes were checked against the code: `ConfigProblem::Parse { line, column, message }`, `keys::Binding { keys, action }`, `Context` (matched in `context_hints`, `context_title` and the `ALL_CONTEXTS` test), `ZniczError: Send`, `Paragraph` and `Wrap` in ratatui 0.30.

**Task order:** each task compiles and passes at its own commit. `spotify_rows.rs` (Task 2) does not use `settings.rs`; `settings.rs` (Task 3) uses it. Task 3 also fills the two non-test constructors its new fields break (`views/settings.rs` and `main.rs`). The Account arm, the client ID prompt and the question's drawing wait for Tasks 8 and 10.

**Type consistency:**
- `SpotifyView` is defined in Task 2 (with `client_id_head`) and used in Tasks 3, 6, 10.
- `SpotifyAsk`, `Done<T>` and `SpotifyWorker<T>` are defined in Task 5 and used in Tasks 6–9.
- `LoginPanel`, `SpotifyPaste` and `SpotifyConfirm` are declared in Task 6 Step 3(b), re-exported as `crate::app::{…}`, and used in Tasks 8–10.
- `on_spotify_done`, `on_spotify_state`, `poll_spotify`, `spotify_view`, `spotify_login_shown` are defined in Task 6 and used in Tasks 7–10.
- `open_login_panel(url, listening)` is a stub in Task 6 and gets its body in Task 9 with the same signature; the opener and the clipboard are App fields (`browser`, `clipboard`), not parameters.
- `change_with` becomes `pub(super)` in Task 7 and is used by `spotify_ui.rs` in Task 8.
- `settings::step` is generic over `Clone` (Task 3) and called with `&current` in Task 7's `OPEN_BROWSERS` and device steps.
- `dimmed`, `prompt_range`, `Values.spotify`, `SettingsValues.spotify`, `SettingsValues.open_browser` are defined in Task 3 and used in Tasks 6, 7, 10.

**Proportion:** the plan is shorter than the spec in prose, and its code covers only the pure logic, the worker, and the non-obvious wiring. The rest is named by signature and test, so the implementer writes the body.
