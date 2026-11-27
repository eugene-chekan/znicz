# Spotify control, part 5: one Stage, Spotify mode and the browser — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `O` puts the TUI in Spotify mode. The Stage and the transport render a source-neutral model, so the Spotify Stage looks like the Znicz one. The keys drive Spotify: browse and search playlists, liked songs, albums, artists and tracks; play in context; transport, volume, mute, repeat, shuffle; a Spotify Connect device list; the Spotify queue. Browsing and playback run on their own connections, so a slow search never holds up Pause.

**Architecture:** Group A makes the Stage and the transport read a `StageModel` and a `TransportModel` built from `PlayerState`, with characterization tests that pin today's rendering first. Group B adds the mode as a flag beside `View`, a pure browser state machine (`spotify_browse.rs`), pure mode rules (`spotify_mode.rs`), a third IPC connection for transport (`SpotifyWorker` lanes, with `main.rs` opening it at all three `run_tui` call sites), and the App wiring in two new `impl App` files beside `spotify_ui.rs`. The Spotify cover goes through `CoverKey::Url` and `znicz_core::fetch_cover`.

**Tech Stack:** Rust 2024 (MSRV 1.85: no `let` chains), `ratatui` 0.30, `crossterm` 0.28, `ratatui-image` for covers, `image` (already a `znicz-tui` dependency, used in tests to encode a PNG), `znicz-core` (`SpotifyOp`, `SpotifyReply`, `fetch_cover`), `znicz-spotify` (through `znicz_spotify::browser`, already a dependency). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-11-26-spotify-control-design.md` (sections "Spotify mode (`O`)", "One Stage, one transport", "Development Mode limits", "Errors", "Polling", "Notes for part 5 (from the part 4 review)" and "Notes for parts 4–5 (from the part 3 review)"). **Part 4:** `docs/superpowers/plans/2026-11-26-spotify-4-tui-settings.md` (done; the model for format). This plan is written against the code as it is on `feat/spotify-control` at `c57a24f`.

## Global Constraints

- Spotify is called only through `SpotifyHost` (`apply`, `try_state`). The TUI never calls the Web API itself.
- No frame waits for Spotify. Only `SpotifyHost::try_state` and `apply(SpotifyOp::Touch)` run on the UI thread, on the main connection. Everything else goes to a worker lane.
- Transport ops (`Play`, `Pause`, `Next`, `Previous`, `Seek`, `SetVolume`, `SetRepeat`, `SetShuffle`, `QueueAdd`, `Transfer`) never queue behind a browse or search call.
- Development Mode limits (Feb 2026): playlist tracks only for playlists the user owns or collaborates on; no artist top tracks; search returns at most 10 per type.
- Znicz's own rendering must not change. Task 1 pins it before anything moves.
- Tests never touch the real terminal, the clipboard, a browser, the network or the real audio device. Cover tests use a local listener or no cover at all.
- Keys: `O` is free (no existing binding). `O` acts only from Browse or Stage with no modal open.
- Every task ends with a commit. Each task compiles and its tests pass at its own commit.

## Review Focus

The five input classes most likely to bite a person using Spotify mode, most likely first. Each has a test in its owning task.

1. **An empty or whitespace-only search.** Enter with nothing typed sends no request and keeps the prompt open. Test: `an_empty_search_sends_nothing` (Task 12).
2. **Seek and volume at the ends.** Seek before 0 or past the track end clamps; `+` at 100 % and `-` at 0 % stay in range; a track with no duration seeks from 0. Tests: `seek_target_clamps_to_zero_and_duration`, `volume_steps_one_percent_and_clamps` (Task 6).
3. **A reply for a list the user has already left.** A page or error with an old generation is dropped and does not add rows to the list on screen. Test: `a_page_from_an_old_generation_is_dropped` (Task 5), `a_late_browse_reply_after_leaving_is_dropped_in_the_app` (Task 11).
4. **Mute when Spotify does not report a volume, or unmute after the device changed.** Mute with an unknown volume is a toast and changes nothing; unmute restores the saved percent or 50 when none was saved. Tests: `mute_with_unknown_volume_does_nothing`, `mute_remembers_the_volume_and_restores_it` (Task 6).
5. **`O` before the first status read, or while logged out.** It toasts the login line and does not enter. The first read happens on launch, so after about 3 s it works. Tests: `o_without_login_toasts_and_stays_out`, `the_poll_runs_every_3s_in_znicz_mode_when_configured` (Tasks 8, 9).

Also covered: an old player in the mode (`an_outdated_player_stops_polling_in_the_mode`, Task 18), a network error on a change (`a_network_error_on_a_change_says_it_may_still_happen`, Task 18).

---

## Spec refinements and decisions made in this part

- **Browse lane and transport lane.** The existing `spotify_worker` (Part 4) stays the browse lane: Settings' `Configure`, login ops, `Devices`, browse, `Queue`. A new `spotify_transport` worker has the third IPC connection. `main.rs` opens it with `connect_ui` at the three `run_tui` call sites. `ask_spotify` picks the lane by the op. Both lanes use the same tag type, `SpotifyAsk`.
- **`SpotifyAsk` loses `Copy`.** The browse tag carries a `BrowseList` with strings, so it is `Clone` only. Part 4's uses still compile after the `.clone()` calls the tasks name.
- **One device list.** The player's service stores the device list it reads, both on its 15 s device poll and on every `Devices` op (`service.rs` sets `state.devices`). So `SpotifyState.devices` is the one list. The Settings tab and Spotify mode both read it. The `Devices` op is still sent when the tab opens and when `,` opens the device list, to refresh the player's copy. The App's `spotify_devices` field goes away (Task 9).
- **Search groups show at most 10 rows, one request per kind, no further paging.** The brief asks to page within a group by offset. Spotify returns at most 10 per type in Development Mode, so the 11th row is not reachable. The `offset` field stays in `SpotifyOp::Search` for MCP. The browser sends offset 0 only and never requests a second page for a group. This follows the spec's "at most 10 per group" and makes the paging code path for search unnecessary (YAGNI). If the real account returns more, Task 19's manual check says so.
- **Liked songs have no context URI.** `Enter` on a liked track plays the loaded liked tracks as `uris`, with `offset` at the cursor. This is the same shape as search results.
- **Ownership is not known in the TUI.** `SpotifyItem` carries no owner or collaborative flag. The "unavailable" notice comes from the `Tracks` reply (the service already turns a 403/404 on a playlist's items into `SpotifyError::Unavailable` with the spec's text). `Enter` on that playlist row in its parent list still plays it, as the spec says.
- **Stop has no Spotify meaning.** In Spotify mode `s` is a toast, `Spotify has no stop: Space pauses`. It does not stop the Znicz engine either.
- **Mute.** Spotify has no mute. `m` remembers the device's percent in `spotify_mute_restore` and sets volume 0. `m` again restores the remembered percent, or 50 when none was saved. A device with no volume reading does nothing and says `Spotify does not report this device's volume`.
- **The slow poll runs from launch.** When a client ID is set, the status is read every 3 s in Znicz mode too, with `try_state` only (no `Touch`). So `O` knows the login state within 3 s of starting. Until the first read, `O` toasts the login line.
- **Cover fetch failure shows the logo.** `fetch_cover` returns `None` on any failure, with no retry. The key then resolves to `CoverReady::Logo`. The previous cover is kept only while the next one is `Pending`, as for streams.
- **Modal for Spotify devices.** `,` in Spotify mode opens a new `Modal::SpotifyDevices`. It is a list over `SpotifyState.devices`. The Znicz `Modal::Devices` (cpal outputs) is unchanged in Znicz mode.
- **Spotify queue drawer.** `]` in Spotify mode toggles the existing `queue_open` drawer. The drawer draws the Spotify queue (read-only) instead of Znicz's queue.
- **Keys in Spotify mode.** `,`, `]`, `Space`, `n`, `p`, `+`, `-`, `=`, `_`, `e`, `z`, `m`, `a`, `/`, arrows, `Enter`, `h`, `l`, `r` and `s` are Spotify's. `P`, `R`, `S`, `M`, `?`, `q`, `v`, `i`, `O` stay global. `A`, `L`, `F`, `U`, `J` toast `not in Spotify mode`. `Tab` and `Shift-Tab` do nothing in the browser (one column).
- **Keys in the browser keep their Znicz meaning where one fits.** `j`/`k` and arrows move, `g`/`G` first and last, `Ctrl-d`/`Ctrl-u` and PageUp/PageDown page.
- **`Esc`.** Closes the search prompt first; otherwise goes back one browser level; at the top level in Browse it does nothing; in Stage it goes to Browse.

---

## File structure

| File | Responsibility |
| --- | --- |
| `znicz-tui/src/views/stage.rs` | Group A: test goldens (Task 1), then renders `StageModel` (Task 2). Group B: Spotify branch of the Stage (Task 17) |
| `znicz-tui/src/views/now_playing.rs` | Group A: test goldens (Task 1), then renders `TransportModel` (Task 3). Group B: Spotify signal line and message row (Tasks 10, 17) |
| `znicz-tui/src/stage_model.rs` (new) | Pure: `StageModel`, `TransportModel`, `StyledText`, `Tone`, `SeekModel`, `VolumeModel`, `SignalRow`, `EmptyState`; `from_player`, `from_spotify` |
| `znicz-tui/src/cover.rs` | `CoverKey::Url(String)` and its resolution through `znicz_core::fetch_cover` (Task 4) |
| `znicz-tui/src/spotify_browse.rs` (new) | Pure browser state machine: `SpotifyBrowser`, `Level`, `Row`, `BrowseList`, `BrowseAction`, paging trigger, generation |
| `znicz-tui/src/spotify_mode.rs` (new) | Pure mode rules: `SpotifyKey` and `key_action`, `seek_target`, `volume_after`, `MuteAction` and `mute_action`, `repeat_mode`, `now_playing_line`, `error_text`, `stops_polling`, `lane`, `Lane`, texts |
| `znicz-tui/src/spotify_worker.rs` | `SpotifyAsk` payloads (Task 7) |
| `znicz-tui/src/app.rs` | `mod spotify_mode; mod spotify_browse_ui;`; new App fields; `Modal::SpotifyDevices`; `handle_key` and `on_esc` branches; `context()`; `with_spotify_transport`; tick |
| `znicz-tui/src/app/spotify_mode.rs` (new) | `impl App`: `O` toggle, slow and fast poll rules, transport actions, queue drawer, devices modal, mode keys |
| `znicz-tui/src/app/spotify_browse_ui.rs` (new) | `impl App`: browser keys, loading, replies, search prompt, mouse rows |
| `znicz-tui/src/app/spotify_ui.rs` | `ask_spotify` picks the lane; `on_spotify_done` routes browse and transport tags; `SpotifyView.devices` from `state.devices`; `spotify_devices` removed |
| `znicz-tui/src/app/settings_ui.rs` | Tests that set `spotify_devices` now set `spotify_state.devices` |
| `znicz-tui/src/views/spotify.rs` (new) | Drawing: browser column, search prompt row, queue drawer, devices modal |
| `znicz-tui/src/views/mod.rs` | Branches: list region in Spotify Browse, drawer in Spotify mode, the devices modal |
| `znicz-tui/src/hit.rs` | `HitMap.spotify_rows: Option<ListHit>` |
| `znicz-tui/src/keys.rs` | `Context` variants `SpotifyBrowse`, `SpotifySearch`, `SpotifyStage`, `SpotifyQueue`, `SpotifyDevices`; `SPOTIFY` binding table; the `O znicz` hint |
| `znicz-tui/src/lib.rs` | `pub mod spotify_browse; pub mod spotify_mode; pub mod stage_model;` |
| `znicz/src/main.rs` | Three `connect_ui` calls at each `run_tui` site; `run_tui_with_client` takes the transport client |
| `wiki/Architecture/TUI.md` | Views, Stage, keys, the Spotify mode section, Modules |
| `wiki/Architecture/Spotify.md` | The TUI lanes and the mode |
| `README.md` | The features line only |

---

## Group A: one Stage and one transport (no behaviour change)

### Task 1: Pin the Znicz Stage and transport with goldens

Adds buffer tests that draw the current Stage and transport at fixed sizes, against the code as it is now. They are the baseline for Tasks 2 and 3. No production code changes.

**Files:**
- Modify: `znicz-tui/src/views/stage.rs` (tests module)
- Modify: `znicz-tui/src/views/now_playing.rs` (tests module)
- Modify: `znicz-tui/src/app.rs` (a `#[cfg(test)] pub(crate) fn test_app()` helper, used by the view tests)

**Interfaces:**
- Consumes: `stage::render(frame, area, app, state)` (`pub fn render` in `views/stage.rs`), `now_playing::render_transport(frame, area, app, state, show_signal)`, the fixtures `playing(bit_perfect)` and `stream()` already in `views/stage.rs` tests.
- Produces: `pub(crate) fn test_app() -> App` in `app.rs` (test-only): `App::with_library(znicz_core::spawn_player(AudioConfig::default()).0, None)`. Used by Tasks 2, 3, 16, 17.

- [x] **Step 1: Add the test helper**

In `znicz-tui/src/app.rs`, beside the existing `mod tests`, add a `#[cfg(test)] pub(crate) fn test_app() -> App` that does what `test_player()` in `mod tests` does (`znicz_core::spawn_player(znicz_core::AudioConfig::default())`), then `App::with_library(player, None)`. Keep `test_player` as it is.

- [x] **Step 2: Write the golden tests for the Stage**

In `views/stage.rs` tests, add a helper that draws the Stage and returns the rows as strings:

```rust
fn stage_rows(state: &PlayerState, width: u16, height: u16, show_cover: bool) -> Vec<String> {
    let mut app = crate::app::test_app();
    app.tui.show_cover = show_cover;
    app.tui.cover_protocol = if show_cover {
        CoverProtocol::Auto
    } else {
        CoverProtocol::Off
    };
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, frame.area(), &mut app, state))
        .unwrap();
    let buffer = terminal.backend().buffer();
    buffer
        .content()
        .chunks(usize::from(width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect())
        .collect()
}
```

Add one test per variant. Each asserts the full rows against a golden:

- `golden_wide_file_stage` — `playing(false)`, 100 × 24, cover on.
- `golden_wide_bit_perfect_stage` — `playing(true)`, 100 × 24, cover on.
- `golden_narrow_file_stage` — `playing(false)`, 48 × 24, cover on.
- `golden_no_cover_stage` — `playing(false)`, 100 × 24, `show_cover` false.
- `golden_stream_stage` — `stream()`, 100 × 24, `show_cover` **false**. A stream with a cover would resolve through the network cover sources (Deezer, MusicBrainz), so it is not drawn in a test.
- `golden_nothing_playing_stage` — `PlayerState::default()`, 100 × 24, `show_cover` false.

The golden is produced by the code as it is now. Procedure:
1. Write each test as `assert_eq!(stage_rows(...), GOLDEN_X)` with `GOLDEN_X` as `vec![]` first.
2. Run `cargo test -p znicz-tui -- golden_ --nocapture` with a `println!("{:#?}", stage_rows(...))` in each test (temporary).
3. Check each printed frame against the rules in the existing tests (`a_file_shows_artist_album_seek_and_signal`, `resampled_explains_both_rates_and_a_missing_format_is_a_dash`, `a_stream_shows_station_url_and_live`, `nothing_playing_says_so_without_a_signal_panel`). Paste the rows as the golden. Remove the `println!`.

Expected: all six pass on the untouched code.

- [x] **Step 3: Write the golden tests for the transport**

In `views/now_playing.rs` tests add `transport_rows(state, width, height, show_signal)` using `render_transport`, and these goldens, using the same fixtures (copy `playing` and `stream` from `views/stage.rs` as `pub(crate) fn` in the stage tests if needed, or rebuild them in this module; do not duplicate more than these two):

- `golden_wide_transport` — 100 × 4 (title row, signal row, message row, plus one spare), `show_signal` true, `playing(false)`.
- `golden_narrow_transport` — 46 × 3, `show_signal` true.
- `golden_transport_without_signal` — 100 × 3, `show_signal` false.
- `golden_stream_transport` — `stream()`, 100 × 3, `show_signal` true.
- `golden_empty_transport` — `PlayerState::default()`, 100 × 3, `show_signal` true.

The message row needs a fixed message state: the `app.toasts` is empty in `test_app()`, so the row is blank. Add one golden with a toast (`app.toasts.info("hello")` before drawing) named `golden_transport_with_a_message`.

Procedure as in Step 2.

- [x] **Step 4: Run the goldens**

Run: `cargo test -p znicz-tui -- golden_`
Expected: 12 passed. Also run `cargo test -p znicz-tui` and expect no other failures.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/views/stage.rs znicz-tui/src/views/now_playing.rs
git commit -m "test(tui): pin the Znicz Stage and transport before the split

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 2: `StageModel` built by `from_player`, rendered by `stage.rs`

**Files:**
- Create: `znicz-tui/src/stage_model.rs`
- Modify: `znicz-tui/src/lib.rs` (`pub mod stage_model;`, beside `pub mod settings;`)
- Modify: `znicz-tui/src/views/stage.rs` (`render` and `detail_lines` read the model)

**Interfaces:**
- Consumes: `PlayerState`, `RepeatMode`, `PlaybackStatus`, `TrackInfo`, `Station`, `CoverKey`, `stream_query` (stage.rs), `format::*`.
- Produces (in `stage_model.rs`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone { Plain, Strong, Dim, Subtitle, Title, Good, Warn, Bad }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledText { pub text: String, pub tone: Tone }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeekModel {
    /// A position within a known or unknown length.
    Bar { position: Duration, total: Option<Duration> },
    /// A live stream: the position only.
    Live { position: Duration },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeModel { pub volume: f32, pub muted: bool }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalRow { pub label: &'static str, pub value: String, pub tone: Tone }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyState { pub text: String, pub hint: String }

#[derive(Debug, Clone, PartialEq)]
pub struct StageModel {
    /// The title line, not yet fitted to the width.
    pub title: String,
    /// Artist and album lines (file) or station and URL (stream), drawn under the title.
    pub head: Vec<StyledText>,
    /// `None` when nothing plays (no seek line is drawn).
    pub seek: Option<SeekModel>,
    pub status: PlaybackStatus,
    /// `None` when nothing plays (no volume line is drawn).
    pub volume: Option<VolumeModel>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    /// Empty when nothing plays. Otherwise the rows under the "Signal" heading.
    pub signal: Vec<SignalRow>,
    /// The path explanation, dropped first when rows run short.
    pub explanation: Option<String>,
    /// `None` when there is nothing to cover (nothing plays, or covers are off).
    pub cover: Option<CoverKey>,
    /// Shown instead of the signal when nothing plays.
    pub empty: Option<EmptyState>,
}

impl StageModel {
    pub fn from_player(state: &PlayerState, stations: &[Station], cover_search: bool, cover_on: bool) -> Self;
}
```

  `from_player` builds `cover` with `CoverKey::File(path)` for files and `CoverKey::Stream(stream_query(state, track, stations, cover_search))` for streams, only when `cover_on`. Stage's existing `render_cover` keeps the choice of `Logo` when covers are off, so `cover_on` is `shown.cover_protocol != CoverProtocol::Off`.

- [x] **Step 1: Write the mapping tests**

In `stage_model.rs` tests:

```rust
#[test]
fn a_file_model_has_the_artist_album_seek_and_signal() {
    let state = crate::views::stage::tests::playing(false); // make `playing` pub(crate) in stage tests
    let model = StageModel::from_player(&state, &[], true, true);
    assert_eq!(model.title, "Blue in Green"); // use the fixture title
    assert!(matches!(model.seek, Some(SeekModel::Bar { .. })));
    assert_eq!(model.signal[0].label, "file");
    assert!(matches!(model.cover, Some(CoverKey::File(_))));
}

#[test]
fn a_stream_model_is_live_and_its_cover_is_a_stream_query() {
    let model = StageModel::from_player(&crate::views::stage::tests::stream(), &[], true, true);
    assert!(matches!(model.seek, Some(SeekModel::Live { .. })));
    assert!(matches!(model.cover, Some(CoverKey::Stream(_))));
    assert_eq!(model.signal[0].label, "stream");
}

#[test]
fn nothing_playing_has_an_empty_state_and_no_cover_or_seek() {
    let model = StageModel::from_player(&PlayerState::default(), &[], true, true);
    assert_eq!(model.title, "Nothing playing");
    assert!(model.seek.is_none() && model.volume.is_none() && model.cover.is_none());
    assert_eq!(model.empty.unwrap().hint, "Enter a track in Browse (v) to start");
}

#[test]
fn covers_off_gives_no_cover_key() {
    let model = StageModel::from_player(&crate::views::stage::tests::playing(false), &[], true, false);
    assert!(model.cover.is_none());
}
```

Make the fixture functions `pub(crate)` in `views/stage.rs` tests (`pub(crate) mod tests`), so the model tests can use them; the title in the assertion is whatever the fixture says (read it from the fixture, do not guess).

- [x] **Step 2: Run the mapping tests to see them fail**

Run: `cargo test -p znicz-tui -- stage_model`
Expected: FAIL (module missing).

- [x] **Step 3: Implement the model**

Create `stage_model.rs` with the types above. `from_player` moves the logic of `title_text`, `file_lines`, `stream_lines`, `signal_lines` and the seek and volume choices out of `stage.rs`, with the same strings. Keep the functions in `stage.rs` as thin wrappers until Step 4.

Head rules, copied from the current code:
- File: the artist (`tags.artist`, else `album_artist`, non-empty) as `Subtitle`; then `album · year · track N` joined with ` · ` as `Dim`, skipping missing parts.
- Stream: the station name from `state.queue[queue_position]` when it differs from the track title, as `Subtitle`; the URL as `Dim`.

Signal rules, copied from `signal_lines`: `Signal` heading (`Strong`), then `source` (`stream` or `file`, `Plain`), `device` (`name · stream` or `name · no stream open`), `path` row (`● bit perfect` `Good`, `▲ resampled` `Warn`, or the dim "nothing to compare…" line when no output). `explanation` is the explanation string; it is `None` when there is no output.

- [x] **Step 4: Render from the model in `stage.rs`**

`stage::render` calls `StageModel::from_player(state, &app.stations, shown.cover_search, shown.cover_protocol != CoverProtocol::Off)`, and `detail_lines` takes the model instead of `state`. `detail_lines` keeps its drop order (explanation, then signal, then volume). Convert `StyledText`, `SignalRow` and the seek model to ratatui `Line`s with the same `theme` calls as before. `render_cover` takes the model's `cover` (`Option<CoverKey>`), keeps `keep_previous` and the frozen rule exactly as they are.

- [x] **Step 5: Run the goldens and the Stage tests**

Run: `cargo test -p znicz-tui -- golden_ stage`
Expected: all pass, including the Task 1 goldens unchanged.

- [x] **Step 6: Commit**

```bash
git add znicz-tui/src/stage_model.rs znicz-tui/src/lib.rs znicz-tui/src/views/stage.rs
git commit -m "refactor(tui): the Stage renders a StageModel built from the player

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 3: `TransportModel` built by `from_player`, rendered by `now_playing.rs`

**Files:**
- Modify: `znicz-tui/src/stage_model.rs` (add `TransportModel`)
- Modify: `znicz-tui/src/views/now_playing.rs` (`render_transport`, `chrome_line`, `signal_line` read the model; `message_line` unchanged in signature)

**Interfaces:**
- Consumes: `Tone`, `StyledText`, `SeekModel`, `VolumeModel` from Task 2.
- Produces:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct TransportModel {
    pub status: PlaybackStatus,
    /// Track title, or `Nothing playing` (Znicz) / `Nothing playing on Spotify` (Task 17).
    pub title: String,
    /// `artist · album` for the middle of the row; `None` when there is none.
    pub subtitle: Option<String>,
    pub seek: Option<SeekModel>,
    pub volume: Option<VolumeModel>,
    pub repeat: RepeatMode,
    pub shuffle: bool,
    /// The one-line signal: `source → device`, with the path badge in `path`.
    pub signal: Option<(String, String)>,
    pub path_badge: Option<(String, Tone)>,
}

impl TransportModel {
    pub fn from_player(state: &PlayerState) -> Self;
}
```

  `signal` is `(source, device)` where `source` is `format_description()` (or `Nothing` when no track: the current `—` line) and `device` is `format!("{} {}", khz, channel_name)` from the output. When no output the current code draws only the source; keep that: `signal` holds `(source, "")` in that case and `path_badge` is `None`. `path_badge` is `● bit perfect` `Good` or `▲ resampled` `Warn`, drawn only when `status != Stopped` and it fits.

- [x] **Step 1: Write the mapping tests**

```rust
#[test]
fn the_transport_model_of_a_file_has_title_subtitle_and_seek() {
    let model = TransportModel::from_player(&crate::views::stage::tests::playing(false));
    assert_eq!(model.title, /* the fixture's title */ "Blue in Green");
    assert!(model.subtitle.is_some());
    assert!(matches!(model.seek, Some(SeekModel::Bar { .. })));
}

#[test]
fn a_stopped_player_has_no_path_badge() {
    let mut state = crate::views::stage::tests::playing(true);
    state.status = PlaybackStatus::Stopped;
    assert!(TransportModel::from_player(&state).path_badge.is_none());
}
```

- [x] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- transport_model` (FAIL: missing).

- [x] **Step 3: Implement** `TransportModel` and `from_player` by moving the field choices out of `chrome_line` (title, subtitle from `artist_album()`, seek, volume, toggles) and `signal_line`. `chrome_line` and `signal_line` keep their width-fitting code (the drop order is unchanged) and read the model.

- [x] **Step 4: Run the goldens** — `cargo test -p znicz-tui -- golden_ now_playing transport` — all pass unchanged.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/stage_model.rs znicz-tui/src/views/now_playing.rs
git commit -m "refactor(tui): the Browse transport renders a TransportModel

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

## Group B: Spotify mode on `O`

### Task 4: The Spotify cover key

**Files:**
- Modify: `znicz-tui/src/cover.rs` (`CoverKey::Url(String)`, `resolve_cover` arm, a test)

**Interfaces:**
- Consumes: `znicz_core::fetch_cover(url: &str) -> Option<CoverArt>`, `CoverReady`, `decode_capped`.
- Produces: `CoverKey::Url(String)`. A URL resolves to `CoverReady::Embedded` when the bytes decode, `CoverReady::Logo` otherwise (including a failed fetch). It never retries.

- [x] **Step 1: Write the failing test**

In `cover.rs` tests, serve one PNG from a local listener and resolve the key:

```rust
#[test]
fn a_url_key_fetches_and_decodes_its_image() {
    use std::io::{Read, Write};
    let mut png = Vec::new();
    image::DynamicImage::new_rgb8(4, 4)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/cover.png", listener.local_addr().unwrap());
    let served = png.clone();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            served.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&served).unwrap();
    });
    let cache = CoverCache::new();
    let ready = wait_ready(&cache, &CoverKey::Url(url));
    assert!(matches!(ready, CoverReady::Embedded(_)));
}
```

Also `a_url_that_does_not_answer_is_the_logo` with a URL to a port nobody listens on: `http://127.0.0.1:1/x.png` → `CoverReady::Logo`. (Port 1 is refused locally; no external network.)

- [x] **Step 2: Run to see it fail** — `cargo test -p znicz-tui -- url_key` (FAIL: no variant).

- [x] **Step 3: Implement**

Add `Url(String)` to `CoverKey` with its doc comment ("A Spotify album image, fetched by URL."). In `resolve_cover`:

```rust
CoverKey::Url(url) => ResolveResult::Update {
    ready: znicz_core::fetch_cover(url).map_or(CoverReady::Logo, |art| ready(&art.bytes)),
},
```

`CoverKey` already derives `Hash`, `Eq` and `Clone`. The `match` in `CoverCache::get` on `CoverKey` needs no change, because only `Stream` has the untitled-refresh rule.

- [x] **Step 4: Run** — `cargo test -p znicz-tui -- cover` — all pass.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/cover.rs
git commit -m "feat(tui): a cover key that fetches a Spotify image URL

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 5: The browser state machine (pure)

**Files:**
- Create: `znicz-tui/src/spotify_browse.rs`
- Modify: `znicz-tui/src/lib.rs` (`pub mod spotify_browse;`)

**Interfaces:**
- Consumes: `znicz_core::{SpotifyItem, SpotifyKind, SpotifyOp, SpotifyPage, SpotifyTrack}`.
- Produces:

```rust
pub const PAGE: u32 = 50;          // rows per list request
pub const NEAR_END: usize = 5;     // fetch the next page when the cursor is this close to the end
pub const SEARCH_LIMIT: u32 = 10;  // rows per search kind (Spotify's Development Mode limit)
pub const TOP: [&str; 4] = ["Search", "Playlists", "Liked songs", "Albums"];
pub const SEARCH_GROUPS: [(SpotifyKind, &str); 4] = [
    (SpotifyKind::Track, "Tracks"),
    (SpotifyKind::Album, "Albums"),
    (SpotifyKind::Artist, "Artists"),
    (SpotifyKind::Playlist, "Playlists"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseList {
    Playlists,
    LikedSongs,
    Albums,
    /// An album's or playlist's tracks.
    Tracks { uri: String },
    /// An artist's albums.
    ArtistAlbums { uri: String },
    /// One kind of one search, at most `SEARCH_LIMIT` rows.
    Search { query: String, kind: SpotifyKind },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A search group heading; the cursor skips it.
    Header(&'static str),
    Item(SpotifyItem),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    /// `None` for the top level.
    pub list: Option<BrowseList>,
    /// The breadcrumb part: `Playlists`, the album name, `Search: query`.
    pub title: String,
    /// Items of a list, or the groups of a search (headers are derived).
    pub items: Vec<SpotifyItem>,
    /// Search only: the four groups, in `SEARCH_GROUPS` order.
    pub groups: Vec<Vec<SpotifyItem>>,
    pub total: u32,
    /// Requests still on the way for this level.
    pub pending: u32,
    pub cursor: usize,
    /// Shown under the list (a playlist the user may not list, an error).
    pub notice: Option<String>,
}

impl Level {
    /// The rows as drawn: headers and items, in order. Search groups with no
    /// results show no header.
    pub fn rows(&self) -> Vec<Row>;
    /// The cursor can land here (an item, not a header).
    pub fn selectable(&self, index: usize) -> bool;
}

/// What Enter or `l` does at the cursor, decided without IO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseAction {
    /// Start playback.
    Play(SpotifyOp),
    /// Push a level; the request is `op` at generation `generation`.
    Load { generation: u64, list: BrowseList, title: String, op: SpotifyOp },
    /// Open the search prompt (the Search row of the top level).
    OpenSearch,
    /// Nothing to do (a header, an empty list).
    Nothing,
}

#[derive(Debug, Default)]
pub struct SpotifyBrowser {
    levels: Vec<Level>,
    generation: u64,
}

impl SpotifyBrowser {
    pub fn new() -> Self;                           // one top level
    pub fn generation(&self) -> u64;
    /// Back to the top level; bumps the generation so replies in flight are dropped.
    pub fn reset(&mut self);
    pub fn depth(&self) -> usize;
    pub fn level(&self) -> &Level;
    /// Breadcrumb parts, top first.
    pub fn crumbs(&self) -> Vec<&str>;
    pub fn step(&mut self, delta: isize);           // skips headers
    pub fn first(&mut self);
    pub fn last(&mut self);
    /// Enter or `l` at the cursor.
    pub fn enter(&mut self) -> BrowseAction;
    /// `h`: pop one level. False at the top.
    pub fn back(&mut self) -> bool;
    /// Start a search for `query` (trimmed). Pushes the Search level and returns
    /// one `(generation, op)` per group. Empty query: returns nothing and changes nothing.
    pub fn start_search(&mut self, query: &str) -> Vec<(u64, BrowseList, SpotifyOp)>;
    /// A page reply. False (and nothing changes) when the generation is not the current one.
    pub fn apply_page(&mut self, generation: u64, list: &BrowseList, offset: u32, page: &SpotifyPage) -> bool;
    /// An error reply for a list. Same generation rule.
    pub fn apply_error(&mut self, generation: u64, list: &BrowseList, message: String) -> bool;
    /// The next page to request when the cursor is near the end, the level is
    /// not loading and more rows exist. Search never pages.
    pub fn next_page(&mut self) -> Option<(u64, BrowseList, SpotifyOp)>;
}

/// The op for one page of a list.
pub fn op_for(list: &BrowseList, offset: u32) -> SpotifyOp;
```

  Notes for the implementer:
  - `enter()` on a top row: `TOP[0]` returns `OpenSearch`; `TOP[1]` pushes `Playlists`; `TOP[2]` pushes `LikedSongs`; `TOP[3]` pushes `Albums`. Each push bumps the generation and sets `pending = 1`.
  - `enter()` on a list level: an `Item` of kind `Playlist` or `Album` plays it from the top (`SpotifyOp::Play { context_uri: Some(uri), uris: vec![], offset: None }`). An `Item` of kind `Artist` pushes `ArtistAlbums { uri }`. An `Item` of kind `Track` plays it in context: for `Tracks { uri }` the context is that `uri` with `offset` = the track's index among the level's track items; for `LikedSongs` the `uris` are all loaded liked tracks, `offset` = the track's index; for a search group, the `uris` are that group's tracks, `offset` = its index in the group. `Enter` on a Playlist or Album item inside a `Tracks` level or a `Search` group plays that item from the top, as above. `l` (the same method; the App calls `enter()` for both) pushes `Tracks { uri }` for a playlist or album when the level is not a search, so `l` on a playlist row in the Playlists list pushes its tracks and `Enter` plays it. The App decides `l` versus `Enter` with a separate method `open()`, see below.
  - Add `pub fn open(&mut self) -> BrowseAction`: the same as `enter()`, except an `Album` or `Playlist` item pushes `Tracks`, an `Artist` pushes `ArtistAlbums`, a track plays. `enter()` plays albums and playlists and pushes artists and tracks-in-context.
  - `apply_page` appends `page.items` to `items` (or, for a search, to `groups[index of kind]`, where the kind is from `list`), sets `total = page.total`, and sets `pending` to `pending - 1` (saturating). Returns true.
  - `apply_error` sets `notice = Some(message)` when the level's `list` is `Tracks`, `ArtistAlbums` or `Search` (a search error notice is shown for that group); for the other lists it also sets the notice. `pending` decrements. Returns true when the generation matches.
  - `next_page`: `Some` when the top level's list is not Search, `items.len() < total`, `pending == 0`, and `cursor + NEAR_END >= rows().len()`. It increments `pending` to 1 and returns `op_for(list, items.len() as u32)`.
  - `start_search` pushes a `Search` level with `title = format!("Search: {query}")`, `groups = vec![vec![]; 4]`, `pending = 4`, and one `(generation, BrowseList::Search { query, kind }, SpotifyOp::Search { query, kinds: vec![kind], offset: 0, limit: SEARCH_LIMIT })` per group.

- [x] **Step 1: Write the failing tests**

Add to `spotify_browse.rs` tests. A helper builds items and pages:

```rust
fn track(n: u32) -> SpotifyItem {
    SpotifyItem {
        kind: SpotifyKind::Track,
        uri: format!("spotify:track:{n}"),
        name: format!("Track {n}"),
        subtitle: "Artist".into(),
        image_url: None,
        track: None,
    }
}

fn page_of(items: Vec<SpotifyItem>, offset: u32, total: u32) -> SpotifyPage {
    SpotifyPage { items, offset, total }
}

#[test]
fn the_top_level_lists_four_entries_and_search_opens_the_prompt() {
    let mut b = SpotifyBrowser::new();
    assert_eq!(b.level().rows().len(), 0); // top rows are drawn from TOP, not Row::Item
    assert_eq!(b.enter(), BrowseAction::OpenSearch);
    b.step(1);
    assert!(matches!(b.enter(), BrowseAction::Load { list: BrowseList::Playlists, .. }));
}
```

Note for the implementer: the top level keeps `items` empty and the App draws `TOP`. The test above replaces the first assert with `assert_eq!(b.level().cursor, 0)`.

Then the tests, each with the assertion shown:

```rust
#[test]
fn entering_a_list_asks_for_page_zero_at_the_new_generation() {
    let mut b = SpotifyBrowser::new();
    b.step(1); // Playlists
    let before = b.generation();
    match b.enter() {
        BrowseAction::Load { generation, op, .. } => {
            assert!(generation > before);
            assert_eq!(op, SpotifyOp::Playlists { offset: 0, limit: PAGE });
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_page_from_an_old_generation_is_dropped() {
    let mut b = SpotifyBrowser::new();
    b.step(1);
    let BrowseAction::Load { generation: old, list, .. } = b.enter() else { panic!() };
    b.reset();
    let applied = b.apply_page(old, &list, 0, &page_of(vec![track(1)], 0, 1));
    assert!(!applied);
    assert!(b.level().items.is_empty());
}

#[test]
fn paging_is_asked_for_only_near_the_end() {
    let mut b = SpotifyBrowser::new();
    b.step(1);
    let BrowseAction::Load { generation, list, .. } = b.enter() else { panic!() };
    let items: Vec<_> = (0..50).map(track).collect();
    b.apply_page(generation, &list, 0, &page_of(items, 0, 120));
    b.first();
    assert!(b.next_page().is_none(), "far from the end");
    b.last(); // cursor 49, rows 50: within NEAR_END
    let (_, _, op) = b.next_page().expect("near the end");
    assert_eq!(op, SpotifyOp::Playlists { offset: 50, limit: PAGE });
    assert!(b.next_page().is_none(), "one request at a time");
}

#[test]
fn paging_stops_when_the_total_is_loaded() {
    // total 3, loaded 3: no next page
}

#[test]
fn a_track_in_an_album_plays_with_the_album_as_context_and_its_index() {
    // Level Tracks { uri: "spotify:album:a" } with tracks 1..3; cursor on track 2.
    // enter() == BrowseAction::Play(SpotifyOp::Play { context_uri: Some("spotify:album:a".into()), uris: vec![], offset: Some(1) })
}

#[test]
fn a_search_track_plays_with_its_group_as_uris() {
    // start_search("x") then apply_page for the Track group with 2 tracks;
    // cursor on the second track; enter() == Play { context_uri: None, uris: [t1, t2], offset: Some(1) }
}

#[test]
fn liked_songs_play_the_loaded_tracks_from_the_cursor() {
    // LikedSongs with 3 tracks, cursor on index 2: uris = all 3, offset = Some(2)
}

#[test]
fn an_album_from_the_list_plays_from_the_top_but_open_shows_its_tracks() {
    // Albums level, item Album: enter() == Play{context_uri: Some(album uri), uris: vec![], offset: None}
    // open() == Load { list: BrowseList::Tracks { uri } , op: SpotifyOp::Tracks { uri, offset: 0, limit: PAGE }, .. }
}

#[test]
fn an_unavailable_playlist_shows_a_notice_and_the_level_keeps_its_rows_empty() {
    // open a Playlist item (Tracks level pushed), then apply_error(gen, Tracks, "Spotify only lists ...")
    // level().notice == Some("Spotify only lists ...") and level().items is empty
}

#[test]
fn search_results_are_grouped_in_order_with_headers() {
    // start_search, then replies for Playlist, Track, Artist, Album (out of order)
    // rows(): Header("Tracks"), tracks…, Header("Albums"), albums…, Header("Artists"), … Header("Playlists"), …
    // a group with no results has no header
}

#[test]
fn the_cursor_skips_headers() {
    // search with a track group; step(1) from the first track goes to the next track, not to a header
}

#[test]
fn an_empty_search_sends_nothing() {
    let mut b = SpotifyBrowser::new();
    assert!(b.start_search("   ").is_empty());
    assert_eq!(b.depth(), 1);
}

#[test]
fn back_pops_one_level_and_bumps_the_generation() {
    // enter Playlists, record generation g, back(): depth 1, generation > g
}

#[test]
fn seek_and_step_clamp_to_the_list() {
    // step(-5) on an empty top level stays at 0; last() on 3 rows is 2
}
```

Write the full bodies for the tests with a comment placeholder above (the first three in full, the rest with the assertions named in the comments). Each comment line states the assertion; the implementer writes the setup from it.

- [x] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- spotify_browse` (FAIL: missing module).

- [x] **Step 3: Implement** the module to the interfaces above. Use `Vec` indexes; no `unsafe`. Keep `SpotifyBrowser` free of IO.

- [x] **Step 4: Run** — `cargo test -p znicz-tui -- spotify_browse` — all pass.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/spotify_browse.rs znicz-tui/src/lib.rs
git commit -m "feat(tui): the Spotify browser as a pure drill-down state machine

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 6: The mode rules (pure)

**Files:**
- Create: `znicz-tui/src/spotify_mode.rs`
- Modify: `znicz-tui/src/lib.rs` (`pub mod spotify_mode;`)

**Interfaces:**
- Consumes: `KeyEvent`, `SpotifyRepeat`, `SpotifyState`, `SpotifyErrorKind`, `RepeatMode`.
- Produces:

```rust
pub const SEEK_SMALL_SECS: i64 = 5;
pub const SEEK_LARGE_SECS: i64 = 30;
pub const VOLUME_STEP_PERCENT: i16 = 1;
/// The percent `m` restores to when no volume was remembered.
pub const UNMUTE_DEFAULT_PERCENT: u8 = 50;
pub const NOT_IN_MODE: &str = "not in Spotify mode";
pub const NOT_LOGGED_IN: &str = "Spotify: log in under Settings → Spotify (S, 6)";
pub const NO_STOP: &str = "Spotify has no stop: Space pauses";
pub const NO_VOLUME: &str = "Spotify does not report this device's volume";
/// Keys the library owns and Spotify mode refuses.
pub const LIBRARY_ONLY_KEYS: [char; 5] = ['A', 'L', 'F', 'U', 'J'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpotifyKey {
    PlayPause,
    Next,
    Previous,
    /// Seconds, signed.
    Seek(i64),
    /// Percent points, signed.
    Volume(i16),
    Repeat,
    Shuffle,
    Mute,
    Devices,
    QueueAdd,
    QueueDrawer,
    Search,
    Stop,
    Reload,
    LibraryOnly,
}

/// The transport keys of Spotify mode, or `None` for a key that is not Spotify's.
/// Navigation (`j`, `k`, arrows without seek, `Enter`, `h`, `l`, `g`, `G`, paging)
/// is the browser's and returns `None` here.
pub fn key_action(key: KeyEvent) -> Option<SpotifyKey>;

/// The seek target, clamped to `[0, duration]`. A zero duration seeks from 0.
pub fn seek_target(progress_ms: u64, delta_secs: i64, duration_ms: u64) -> u64;

/// A volume in 0.0..=1.0 for `percent + delta`, clamped to 0..=100.
pub fn volume_after(percent: u8, delta: i16) -> f32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteAction {
    /// Remember `remember` and set volume 0.
    Mute { remember: u8 },
    /// Set volume to this percent and forget the remembered one.
    Unmute(u8),
    /// Nothing to do (a device with no reading, or already 0 and nothing remembered).
    Nothing,
}

/// Decide `m`. `volume` is the device's percent (`None` when unknown); `restore` is the remembered percent.
pub fn mute_action(volume: Option<u8>, restore: Option<u8>) -> MuteAction;

pub fn repeat_mode(repeat: SpotifyRepeat) -> RepeatMode; // Off->Off, Context->All, Track->One

/// `♫ Spotify: <title> — <artist> · O to switch`, when something plays with an item.
pub fn now_playing_line(state: &SpotifyState) -> Option<String>;

/// The message row text for an error of a change or a read, and whether it stops polling.
pub struct ErrorPlan { pub text: String, pub stops_polling: bool, pub reread: bool }
pub fn error_plan(kind: SpotifyErrorKind, message: &str) -> ErrorPlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane { Transport, Browse }
/// Transport ops go to the transport lane; everything else to the browse lane.
pub fn lane(op: &SpotifyOp) -> Lane;
```

`key_action` mapping: Space→PlayPause; `n`→Next; `p`→Previous; Right/Left without ALT→Seek(±5), with SHIFT→Seek(±30); `+`/`=`→Volume(+1); `-`/`_`→Volume(-1); `e`→Repeat; `z`→Shuffle; `m`→Mute; `,`→Devices; `a`→QueueAdd; `]`→QueueDrawer; `/`→Search; `s`→Stop; `r`→Reload; an uppercase letter in `LIBRARY_ONLY_KEYS`→LibraryOnly. Any ALT modifier returns None. Ctrl returns None.

`error_plan`: `Network` → text "Spotify did not answer: the change may still happen; the next status read shows the truth", `reread: true`, `stops_polling: false`. `OutdatedPlayer` and `NoService` → text is the message, `stops_polling: true`. `NoActiveDevice`, `PremiumRequired`, `Unavailable`, `RateLimited` → text is the message, all false. Others → message. Note: the message for `NoActiveDevice` comes from the core error already and mentions `,`; `error_plan` does not rewrite it.

`now_playing_line`: `Some(format!("♫ Spotify: {title} — {artist} · O to switch"))` when `playback.is_playing` and the item is present; `artist` is `artist_line()`; an empty artist gives `♫ Spotify: {title} · O to switch`. Otherwise `None`.

- [x] **Step 1: Write the failing tests**

```rust
#[test]
fn space_and_arrows_are_spotifys_and_alt_arrows_are_not() {
    assert_eq!(key_action(key(KeyCode::Char(' '))), Some(SpotifyKey::PlayPause));
    assert_eq!(key_action(key(KeyCode::Right)), Some(SpotifyKey::Seek(5)));
    assert_eq!(key_action(shift(KeyCode::Left)), Some(SpotifyKey::Seek(-30)));
    assert_eq!(key_action(alt(KeyCode::Right)), None);
    assert_eq!(key_action(key(KeyCode::Down)), None, "navigation is the browser's");
    assert_eq!(key_action(key(KeyCode::Enter)), None);
}

#[test]
fn library_only_keys_are_named_and_the_rest_stay_global() {
    assert_eq!(key_action(key(KeyCode::Char('A'))), Some(SpotifyKey::LibraryOnly));
    assert_eq!(key_action(key(KeyCode::Char('P'))), None, "P stays the playlists modal");
}

#[test]
fn seek_target_clamps_to_zero_and_duration() {
    assert_eq!(seek_target(2_000, -5, 180_000), 0);
    assert_eq!(seek_target(178_000, 30, 180_000), 180_000);
    assert_eq!(seek_target(10_000, 5, 0), 0, "no duration: seek from zero");
    assert_eq!(seek_target(10_000, 5, 180_000), 15_000);
}

#[test]
fn volume_steps_one_percent_and_clamps() {
    assert_eq!(volume_after(100, 1), 1.0);
    assert_eq!(volume_after(0, -1), 0.0);
    assert!((volume_after(40, 1) - 0.41).abs() < 1e-6);
}

#[test]
fn mute_remembers_the_volume_and_restores_it() {
    assert_eq!(mute_action(Some(40), None), MuteAction::Mute { remember: 40 });
    assert_eq!(mute_action(Some(0), Some(40)), MuteAction::Unmute(40));
    assert_eq!(mute_action(Some(0), None), MuteAction::Nothing);
    assert_eq!(mute_action(Some(0), Some(0)), MuteAction::Unmute(0));
}

#[test]
fn mute_with_unknown_volume_does_nothing() {
    assert_eq!(mute_action(None, None), MuteAction::Nothing);
    assert_eq!(mute_action(None, Some(30)), MuteAction::Unmute(30));
}
```

Hmm: `mute_action(None, Some(30))` returns `Unmute(30)` — the unknown volume case with a remembered value is an unmute (the device was muted by us and its reading is gone). Keep that test.

```rust
#[test]
fn repeat_maps_to_znicz_labels() {
    assert_eq!(repeat_mode(SpotifyRepeat::Off), RepeatMode::Off);
    assert_eq!(repeat_mode(SpotifyRepeat::Context), RepeatMode::All);
    assert_eq!(repeat_mode(SpotifyRepeat::Track), RepeatMode::One);
}

#[test]
fn the_now_playing_line_needs_a_playing_track() {
    // playback with item "So What" by "Miles Davis", is_playing true:
    // == "♫ Spotify: So What — Miles Davis · O to switch"
    // is_playing false -> None; no item -> None
}

#[test]
fn a_network_error_says_it_may_still_happen_and_rereads() {
    let plan = error_plan(SpotifyErrorKind::Network, "timed out");
    assert!(plan.reread && !plan.stops_polling);
    assert!(plan.text.contains("may still happen"));
}

#[test]
fn an_old_player_stops_polling_and_says_why() {
    assert!(error_plan(SpotifyErrorKind::OutdatedPlayer, "old").stops_polling);
    assert!(error_plan(SpotifyErrorKind::NoService, "none").stops_polling);
}

#[test]
fn transport_ops_are_on_the_transport_lane() {
    assert_eq!(lane(&SpotifyOp::Pause), Lane::Transport);
    assert_eq!(lane(&SpotifyOp::Transfer { device_id: "d".into(), play: true }), Lane::Transport);
    assert_eq!(lane(&SpotifyOp::Search { query: "x".into(), kinds: vec![], offset: 0, limit: 10 }), Lane::Browse);
    assert_eq!(lane(&SpotifyOp::Devices), Lane::Browse);
}
```

`key`, `shift`, `alt` helpers: `KeyEvent::new(code, KeyModifiers::...)`.

- [x] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- spotify_mode`.

- [x] **Step 3: Implement** to the interfaces. `volume_after` returns `f32` as `(percent + delta).clamp(0, 100) as f32 / 100.0`, so `volume_after(40, 1)` is `0.41`.

- [x] **Step 4: Run** — `cargo test -p znicz-tui -- spotify_mode` — all pass.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/spotify_mode.rs znicz-tui/src/lib.rs
git commit -m "feat(tui): the Spotify mode rules as pure functions

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 7: Two lanes, payload tags, and the third connection

**Files:**
- Modify: `znicz-tui/src/spotify_worker.rs` (`SpotifyAsk` payloads; drop `Copy`)
- Modify: `znicz-tui/src/app.rs` (`spotify_transport` field; `with_spotify_transport`; the field's default in `with_engine`)
- Modify: `znicz-tui/src/app/spotify_ui.rs` (`ask_spotify` picks the lane; `on_spotify_done` arms for the new tags)
- Modify: `znicz/src/main.rs` (three `connect_ui` calls; `run_tui_with_client` takes `transport: IpcClient`; calls `with_spotify_transport`)

**Interfaces:**
- Consumes: `spotify_mode::{lane, Lane}`, `SpotifyWorker<T>`.
- Produces:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpotifyAsk {
    Configure,
    BeginLogin,
    CompleteLogin,
    CancelLogin,
    Logout,
    Devices,
    /// Spotify mode's browse request. `generation` is the browser's; a reply with an old one is dropped.
    Browse { generation: u64, list: BrowseList, offset: u32 },
    /// The queue, for the drawer.
    Queue,
    /// Any transport op: play, pause, next, previous, seek, volume, repeat, shuffle, queue add, transfer.
    Transport,
}
```

`App` gets `pub(crate) spotify_transport: Option<SpotifyWorker<SpotifyAsk>>`. `with_spotify_transport(mut self, host: Arc<dyn SpotifyHost>) -> io::Result<Self>`. `ask_spotify(&self, tag, op) -> bool` picks `self.spotify_transport` for `Lane::Transport` and `self.spotify_worker` for `Lane::Browse`, and returns false when that lane is missing.

- [ ] **Step 1: Write the failing lane tests** (in `spotify_ui.rs` tests, using the existing `FakeHost` and `app_with`)

```rust
#[test]
fn a_transport_op_is_sent_on_the_transport_lane_not_behind_a_browse_call() {
    // Build the app with a gated browse host: its Search blocks until released.
    // Give the App a second worker (transport) over a second FakeHost.
    // Send Search on the browse lane (ask_spotify(Browse{..}, Search{..})) and Pause on the transport lane.
    // wait_for_op(&transport_host, |op| *op == SpotifyOp::Pause) is true while the browse Search is still blocked.
}

#[test]
fn a_browse_reply_keeps_its_generation_and_offset() {
    // ask_spotify(Browse { generation: 7, list: BrowseList::Playlists, offset: 0 }, Playlists{..});
    // the reply arrives as Done { tag: Browse { generation: 7, .. }, .. }
    // (assert on the tag by driving on_spotify_done directly with a constructed Done)
}
```

For the gated host, use the existing pattern in `spotify_worker.rs` tests (a `Mutex<Receiver<()>>` gate). Put the gated host in the test module of `spotify_ui.rs`.

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- transport_op browse_reply`.

- [ ] **Step 3: Implement**

- `SpotifyAsk` as above. Fix the Settings arms in `on_spotify_done` to match the new variants (they match by name, so only the `Copy` removal needs `.clone()` where a tag is used twice).
- `on_spotify_done` arms:
  - `Browse { .. }` → `self.on_browse_done(done)` (defined in Task 11; in this task, `on_browse_done` ignores the reply: `let _ = done;`, with a `// Task 11` comment).
  - `Queue` → Task 14 (ignored here).
  - `Transport` → `Err(e)` toasts `e.to_string()` as an error; `Ok` nothing.
- Main: `connect_ui` three times at each of the three `run_tui` call sites (lines 898–899, 961–962 and 1004–1005 at `c57a24f`). Name the third `transport`. Pass `run_tui_with_client(player, spotify, transport, library_path, ...)`. Add the parameter `transport: IpcClient` to `run_tui_with_client`, then `.with_spotify_transport(Arc::new(transport))` after `with_spotify_worker`.
- `spotify_mode::lane` decides the lane.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- spotify_ui` and `cargo build -p znicz` — pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/spotify_worker.rs znicz-tui/src/app.rs znicz-tui/src/app/spotify_ui.rs znicz/src/main.rs
git commit -m "feat(tui): a transport lane with its own connection, and tags with payloads

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 8: The mode flag, `O`, contexts, and `Esc`

**Files:**
- Create: `znicz-tui/src/app/spotify_mode.rs`
- Modify: `znicz-tui/src/app.rs` (`mod spotify_mode;`; fields `spotify_mode: bool`, `spotify_browser: SpotifyBrowser`, `spotify_search: Option<LineEdit>`, `spotify_queue: Option<(Option<SpotifyTrack>, Vec<SpotifyTrack>)>`, `spotify_mute_restore: Option<u8>`, `spotify_devices_cursor: Cursor`; `Modal::SpotifyDevices` (variant only here); `handle_key` branch for the flag; `on_esc` branch; `context()` branch; `blocks_list_focus` includes `SpotifyDevices`)
- Modify: `znicz-tui/src/keys.rs` (`Context` variants `SpotifyBrowse`, `SpotifySearch`, `SpotifyStage`, `SpotifyQueue`, `SpotifyDevices`; their `context_hints` and `context_title`; the `SPOTIFY` table starting with `O`)

**Interfaces:**
- Consumes: `spotify_login_shown`, `spotify_state`, `spotify_unavailable`, `spotify_polled_at`.
- Produces (in `app/spotify_mode.rs`, `impl App`):

```rust
pub(crate) fn toggle_spotify_mode(&mut self);   // O
pub(crate) fn in_spotify_mode(&self) -> bool { self.spotify_mode }
```

- [ ] **Step 1: Write the failing tests** (in `app/spotify_mode.rs` tests, using `spotify_ui::tests::{FakeHost, sent_ops, wait_for_op}` and an `app_with`-style helper that sets `spotify_state.login = LoggedIn` for the logged-in case)

```rust
#[test]
fn o_toggles_spotify_mode_from_browse_and_from_stage() {
    // logged in, no modal: press 'O' -> spotify_mode true; press 'v' -> view Stage, still mode; press 'O' -> false
}

#[test]
fn o_without_login_toasts_the_login_line_and_stays_out() {
    // not logged in: press 'O' -> spotify_mode false; toast_texts contains NOT_LOGGED_IN
}

#[test]
fn v_and_i_still_switch_views_inside_the_mode() {
    // logged in, mode on: 'v' -> View::Stage; 'i' -> View::Browse; mode stays true
}

#[test]
fn entering_the_mode_clears_the_unavailable_stop() {
    // spotify_unavailable = Some("old"); logged in; press 'O' -> spotify_unavailable is None and spotify_polled_at is None
}

#[test]
fn o_does_nothing_while_settings_is_open() {
    // open Settings (press 'S'), press 'O' -> spotify_mode stays false, modal still Settings
}

#[test]
fn the_context_is_spotify_browse_or_stage_in_the_mode() {
    // mode on, Browse -> Context::SpotifyBrowse; mode on, Stage -> Context::SpotifyStage;
    // mode off, Browse -> Context::Library
}

#[test]
fn the_footer_ends_in_o_znicz_in_the_mode() {
    // keys::context_hints(Context::SpotifyStage).last().unwrap().label == "O znicz"
    // and the same for SpotifyBrowse
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- spotify_mode` (note: `app::spotify_mode` tests live under the same name as the pure module; run with `cargo test -p znicz-tui -- app::spotify_mode`).

- [ ] **Step 3: Implement**

- `handle_key`: before `on_global_key`, when `self.spotify_search.is_some()`, call `self.on_spotify_search_key(key)` (Task 12; here a stub that returns without doing anything) and return. Put the `O` key in `on_global_key` as `KeyCode::Char('O') if self.modal == Modal::None => self.toggle_spotify_mode()`.
- `toggle_spotify_mode`:
  - When turning on: if `self.spotify_login_shown() != SpotifyLogin::LoggedIn` (use `spotify_state.login` directly: `!= LoggedIn`), toast `spotify_mode::NOT_LOGGED_IN` and return. Otherwise `spotify_mode = true`, `spotify_unavailable = None`, `spotify_polled_at = None` (forces a read on the next tick), `spotify_browser` stays as it is.
  - When turning off: `spotify_mode = false`, `spotify_browser.reset()`, `spotify_search = None`, `spotify_devices_open`... (the devices modal is `Modal::SpotifyDevices`; if open, close it), `queue_open = false` if it was the Spotify drawer. Keep `spotify_mute_restore` (volume is still on the account).
- `on_esc` branch, first: `if self.spotify_mode && self.modal == Modal::None && self.view == View::Browse { if self.spotify_search.is_some() { self.spotify_search = None; } else { self.spotify_browser.back(); } return; }`. Add `Modal::SpotifyDevices` to the `matches!` list that closes modals.
- `context()`: `Modal::None if self.spotify_mode && self.view == View::Stage => Context::SpotifyStage`; `Modal::None if self.spotify_mode && self.spotify_search.is_some() => Context::SpotifySearch`; `Modal::None if self.spotify_mode && self.queue_open => Context::SpotifyQueue`; `Modal::None if self.spotify_mode => Context::SpotifyBrowse`; `Modal::SpotifyDevices => Context::SpotifyDevices`. Place these before the Znicz `Modal::None` arms.
- `keys.rs`: `SPOTIFY` table (first rows only here, the rest in Tasks 12–15):

```rust
pub const SPOTIFY: &[Binding] = &[
    b("O", "leave Spotify mode"),
    b("v / i", "now playing (stage)"),
    b("Esc", "back one browser level"),
];
```

  Context hints (footer, most important first):
  - `SpotifyBrowse`: `SPACE_PAUSE`, `SEARCH`, `ENTER_PLAY`, `ADD`, `QUEUE_OPEN`, `SPOTIFY_LEAVE` (`hint("O znicz", KeyCode::Char('O'))`).
  - `SpotifyStage`: `SPACE_PAUSE`, `NEXT`, `BROWSE`, `SPOTIFY_LEAVE`.
  - `SpotifySearch`: `ENTER_SEARCH`, `CANCEL`.
  - `SpotifyQueue`: `QUEUE_CLOSE`, `SPOTIFY_LEAVE`.
  - `SpotifyDevices`: `SELECT`, `CLOSE`.
  `context_title`: the Spotify variants give `Some("Spotify")`; add `("Spotify", SPOTIFY)` to `CONTEXT_SECTIONS`.
  `more_hint`: `SpotifySearch` is typed text, so it goes in the list of `None` cases.
  Every `match` on `Context` must cover the new variants (the compiler lists them).
- Test in `keys.rs`: the existing check that every footer hint names a documented key still passes (`O` is in `SPOTIFY`).

- [ ] **Step 4: Run** — `cargo test -p znicz-tui` — all pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_mode.rs znicz-tui/src/keys.rs
git commit -m "feat(tui): O enters Spotify mode, separate from the view

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 9: Polling in the mode, the slow poll, and one device list

**Files:**
- Modify: `znicz-tui/src/app/spotify_ui.rs` (`poll_spotify` rules, `spotify_tab_open` renamed to `spotify_fast_poll`, `SpotifyView.devices` from state, the `Devices` reply sets only `spotify_devices_known`)
- Modify: `znicz-tui/src/app.rs` (remove the `spotify_devices` field)
- Modify: `znicz-tui/src/app/settings_ui.rs` (tests set `app.spotify_state.devices` instead of `app.spotify_devices`; at lines 2203, 2237, 2491, 2554 at `c57a24f`)

**Interfaces:**
- Consumes: `SPOTIFY_POLL_EVERY` (500 ms), `SPOTIFY_SLOW_EVERY` (new, 3 s).
- Produces:

```rust
/// How often the status is read: 500 ms while the Spotify tab, a login panel or Spotify mode shows it;
/// 3 s otherwise when a client ID is set; `None` when there is nothing to read.
pub(crate) fn spotify_poll_every(&self) -> Option<Duration>;
pub(crate) fn spotify_fast_poll(&self) -> bool;   // mode, Spotify tab or login panel
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_poll_runs_every_3s_in_znicz_mode_when_configured() {
    // configured, no mode, no Settings: spotify_poll_every() == Some(Duration::from_secs(3))
}

#[test]
fn the_poll_is_500ms_in_spotify_mode() {
    // mode on (set spotify_mode = true directly): Some(Duration::from_millis(500))
}

#[test]
fn no_poll_without_a_client_id_and_none_once_the_player_is_outdated() {
    // unconfigured -> None; configured with spotify_unavailable Some -> None
}

#[test]
fn the_slow_poll_reads_the_status_but_sends_no_touch() {
    // configured, not in mode, tick poll_spotify twice with the due time forced: host.ops has no Touch
}

#[test]
fn settings_shows_the_players_device_list() {
    // spotify_state.devices = one_device("Kitchen"); spotify_view().devices[0].name == "Kitchen"
}
```

Also update the existing test `a_device_list_arrives_from_the_worker` (spotify_ui.rs): it now asserts `app.spotify_devices_known` and that the state's list is read after one poll (the `FakeHost.state` carries `devices`).

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- spotify_ui poll`.

- [ ] **Step 3: Implement**

- `poll_spotify`: drains replies as today; then `let Some(every) = self.spotify_poll_every() else { return };` and checks `spotify_polled_at` against it. `Touch` is sent only when `spotify_fast_poll()` is true. Errors: `NoService`/`OutdatedPlayer` set `spotify_unavailable`, as today.
- `spotify_poll_every`: `None` when not configured or `spotify_unavailable.is_some()`; `Some(500 ms)` when `spotify_fast_poll()`; else `Some(3 s)`.
- `spotify_fast_poll`: `self.spotify_mode || (self.modal == Modal::Settings && (self.settings_tab == Tab::Spotify || self.spotify_login.is_some())) || self.spotify_login.is_some()`.
- Remove `spotify_tab_open`; its callers (`poll_spotify`) use the new rules.
- `Devices` reply: `(SpotifyAsk::Devices, Ok(SpotifyReply::Devices { .. }))` sets `spotify_devices_known = true` when the client ID matches; the list itself is not copied (`state.devices` already has it, because the player stores what it read).
- `spotify_view()`: `devices: self.spotify_state.devices.clone()`.
- Remove `spotify_devices` from `App` and its defaults.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui` — all pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_ui.rs znicz-tui/src/app/settings_ui.rs
git commit -m "feat(tui): poll the status in Znicz mode too, and read one device list

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 10: The message row shows Spotify playing in Znicz mode

**Files:**
- Modify: `znicz-tui/src/views/now_playing.rs` (`message_line` takes `spotify: Option<&str>`)
- Modify: `znicz-tui/src/views/mod.rs` (both `message_line` call sites pass `spotify_mode::now_playing_line(&app.spotify_state)` only when `!app.spotify_mode`)

**Interfaces:**
- Consumes: `spotify_mode::now_playing_line`.
- Produces: `message_line(toasts, scanning, spotify: Option<&str>, width)`. Order: newest toast, then scan progress, then the Spotify line, then unread alerts, then blank. The Spotify line is drawn in the style `theme::good()` for the note mark and `theme::text()` for the text, cut with `format::truncate`.

- [ ] **Step 1: Write the failing tests** (in `now_playing.rs` tests)

```rust
#[test]
fn the_message_row_shows_spotify_playing_when_nothing_newer_is_on_it() {
    // toasts empty, spotify Some("♫ Spotify: So What — Miles Davis · O to switch"), width 60
    // the row's text starts with "♫ Spotify: So What"
}

#[test]
fn a_newer_toast_beats_the_spotify_line() {
    // toasts.info("saved"), spotify Some(..): the row shows "saved"
}

#[test]
fn unread_alerts_come_after_the_spotify_line() {
    // a Spotify line beats the unread marker
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- message_row`.

- [ ] **Step 3: Implement** the parameter and the branch. Update the existing `message_line` tests to pass `None`.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- now_playing` and the Task 1 goldens (`golden_`) — all pass (the goldens pass `None` in their path, so the rows are unchanged).

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/views/now_playing.rs znicz-tui/src/views/mod.rs
git commit -m "feat(tui): the message row names what Spotify plays in Znicz mode

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 11: Browser wiring: loading, replies, Enter, `l`, `h`

**Files:**
- Create: `znicz-tui/src/app/spotify_browse_ui.rs`
- Modify: `znicz-tui/src/app.rs` (`mod spotify_browse_ui;`; `handle_key` routes the browser keys in the mode; `step`, `page`, `go_first`, `go_last` in mode use the browser; `r` reloads the top or the current level)
- Modify: `znicz-tui/src/app/spotify_ui.rs` (`on_spotify_done` arm `Browse` calls `on_browse_done`; `Queue` and `Transport` as in Task 7)

**Interfaces:**
- Consumes: `SpotifyBrowser`, `BrowseAction`, `BrowseList`, `SpotifyAsk::Browse`, `ask_spotify`.
- Produces (`impl App`):

```rust
pub(crate) fn spotify_browser_key(&mut self, key: KeyEvent) -> bool;   // true when handled
pub(crate) fn spotify_enter(&mut self, open: bool);                     // Enter (open=false) or l (open=true)
pub(crate) fn spotify_back(&mut self) -> bool;                          // h
pub(crate) fn spotify_load(&mut self, generation: u64, list: BrowseList, op: SpotifyOp);
pub(crate) fn spotify_send_play(&mut self, op: SpotifyOp);              // Transport tag
pub(crate) fn on_browse_done(&mut self, done: Done<SpotifyAsk>);
pub(crate) fn request_more_rows(&mut self);                             // called from the tick
```

- [ ] **Step 1: Write the failing tests** (in `spotify_browse_ui.rs` tests, using `FakeHost` and `wait_for_op`; the FakeHost `reply` is set to a `SpotifyReply::Page { page }` for list ops)

```rust
#[test]
fn entering_playlists_sends_the_first_page_and_fills_the_list_when_it_arrives() {
    // spotify_mode on, logged in; press 'j' (to Playlists), press 'l'; wait_for_op(Playlists { offset 0, limit 50 })
    // wait_until(app.spotify_browser.level().items.len() == 2) with a page of 2 items
}

#[test]
fn a_late_browse_reply_after_leaving_is_dropped_in_the_app() {
    // enter Playlists; leave the mode (O); enter again; apply a Done with the old generation
    // the browser's level has no items
}

#[test]
fn enter_plays_a_playlist_from_the_top_and_sends_play_on_the_transport_lane() {
    // at the Playlists list with a playlist item, press Enter -> wait_for_op(Play { context_uri: Some(uri), uris: vec![], offset: None })
    // on the transport host (second FakeHost)
}

#[test]
fn l_on_a_playlist_opens_its_tracks_and_an_unavailable_reply_shows_a_notice() {
    // press 'l' on a playlist item -> wait_for_op(Tracks { uri, offset: 0, limit: 50 })
    // reply Err(Spotify { kind: Unavailable, message: "Spotify only lists the tracks of playlists you own or collaborate on; Enter still plays it" })
    // level().notice == Some(that text)
}

#[test]
fn h_goes_back_one_level_and_never_past_the_top() {
    // enter Playlists, press 'h': depth 1; press 'h' again: depth stays 1
}

#[test]
fn a_browse_error_other_than_unavailable_is_a_toast_and_the_list_stays() {
    // Err(Network) on a page: toast_texts has the message; level().pending == 0
}
```

Use `wait_until(app, pred)` from `spotify_ui` tests (`fn wait_until` is private there; make it `pub(crate)`).

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- spotify_browse_ui`.

- [ ] **Step 3: Implement**

- `spotify_browser_key(key)`: returns false for keys it does not own. Owns: `j`/Down (`step(1)`), `k`/Up (`step(-1)`), `g`/Home (`first`), `G`/End (`last`), Ctrl-`d`/`u` and PageDown/PageUp (step by 10), `Enter` (`spotify_enter(false)`), `l`/Right? (`l` only; Right is seek), `h` (`spotify_back`), `r` (reload: `reset()` and push the top again is not needed; for a list level re-request page 0 by `spotify_load` with the current generation, for the top do nothing), `/` (open the prompt: `self.spotify_search = Some(LineEdit::new())`), `Tab`/`BackTab` (handled, do nothing).
- `spotify_enter(open)`: `let action = if open { browser.open() } else { browser.enter() };` then match: `Play(op)` → `spotify_send_play(op)` on the transport lane; `Load{generation, list, op, ..}` → `spotify_load`; `OpenSearch` → open the prompt; `Nothing` → nothing. `open()` is the `pub fn open` added in Task 5 (see its note).
- `spotify_load(generation, list, op)`: `ask_spotify(SpotifyAsk::Browse { generation, list: list.clone(), offset: offset_of(&op) }, op)`. Offset is 0 for a load.
- `on_browse_done(done)`: `match done.result { Ok(SpotifyReply::Page { page }) => browser.apply_page(gen, &list, offset, &page), Err(ZniczError::Spotify { kind, message }) => browser.apply_error(gen, &list, message) (Unavailable on Tracks sets the notice; other errors also toast when the generation matches), Ok(_) | Err(_) => ... }`. A reply whose generation is stale: `apply_*` returns false; nothing else happens. Non-matching `Ok` variants are ignored.
- `request_more_rows()` (from the tick, only in the mode): `if let Some((generation, list, op)) = browser.next_page() { spotify_load(...) }`.
- `spotify_send_play(op)`: `ask_spotify(SpotifyAsk::Transport, op)`. If `false` (no lane), toast the no-player line.
- `handle_key`: in the mode with `modal == Modal::None` and `view == View::Browse` and `focus == Library`: first `spotify_browser_key(key)`, then `spotify_mode::key_action(key)` for transport keys (Task 13 wires these; until then the transport keys fall through to the Znicz handlers, which is wrong — so in this task add the branch: any key the browser does not own is dropped while the mode is on, `return;`, so no Znicz action runs. Task 13 replaces the drop with transport keys.).
- `step`, `page`, `go_first`, `go_last` in mode: they call the browser instead of `self.library` (Task 11 adds `if self.spotify_mode { ... }` at the top of each).
- The tick (`App::run` loop) calls `request_more_rows()` after `poll_spotify`.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- spotify_browse_ui spotify_ui` — all pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_browse_ui.rs znicz-tui/src/app/spotify_ui.rs znicz-tui/src/spotify_browse.rs
git commit -m "feat(tui): browse Spotify playlists, liked songs, albums and tracks

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 12: The search prompt and search results

**Files:**
- Modify: `znicz-tui/src/app/spotify_browse_ui.rs` (`on_spotify_search_key`, `submit_search`)
- Modify: `znicz-tui/src/app.rs` (the stub from Task 8 is replaced by the real routing: the prompt takes every key)

**Interfaces:**
- Consumes: `LineEdit` (`on_key`, `as_str`), `SpotifyBrowser::start_search`.
- Produces: `pub(crate) fn on_spotify_search_key(&mut self, key: KeyEvent)`; `fn submit_search(&mut self)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn slash_opens_the_prompt_and_typing_fills_it_without_searching() {
    // mode on: '/' then type "kind"; no ops sent (sent_ops is empty)
}

#[test]
fn enter_searches_four_kinds_at_ten_each() {
    // type "kind of blue", Enter: wait_for_op for Search { kinds: [Track], offset 0, limit 10 },
    // and for each of Album, Artist, Playlist
}

#[test]
fn an_empty_search_sends_nothing() {
    // '/' then '   ' then Enter: sent_ops is empty, the prompt stays open
}

#[test]
fn esc_closes_the_prompt_and_keeps_the_browser() {
    // '/', type "x", Esc: spotify_search is None, depth unchanged
}

#[test]
fn search_results_arrive_grouped() {
    // after the four replies (driven through the FakeHost reply), rows() has Header("Tracks") first
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- search`.

- [ ] **Step 3: Implement**

- `on_spotify_search_key(key)`: `Enter` → `submit_search()`; `Esc` → `spotify_search = None`; else `spotify_search.as_mut().unwrap().on_key(key)`. The prompt takes every other key (including `O`, `q`, `?`) — the prompt is typed text.
- `submit_search()`: `let query = prompt.as_str().trim().to_string();` if empty, return (the prompt stays open). Otherwise `spotify_search = None`, and `for (generation, list, op) in browser.start_search(&query)` → `spotify_load(generation, list, op)`.
- The prompt is drawn by the view (Task 16); the footer context is `SpotifySearch`.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- spotify_browse_ui` — all pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_browse_ui.rs
git commit -m "feat(tui): search Spotify on Enter, with four grouped requests

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 13: Transport keys in the mode

**Files:**
- Modify: `znicz-tui/src/app/spotify_mode.rs` (`spotify_transport_key(key: SpotifyKey)`)
- Modify: `znicz-tui/src/app.rs` (`handle_key`: in the mode, the `spotify_mode::key_action` result is handled before the Znicz global keys; the drop from Task 11 is replaced)

**Interfaces:**
- Consumes: `spotify_mode::{key_action, SpotifyKey, seek_target, volume_after, mute_action, MuteAction, NO_STOP, NO_VOLUME, UNMUTE_DEFAULT_PERCENT}`, `spotify_state.playback`, `SpotifyPlayback::progress_at`, `SpotifyRepeat::next`.
- Produces: `pub(crate) fn spotify_transport_key(&mut self, key: SpotifyKey)`; `fn now_unix_ms() -> u64` (`SystemTime::now()` since `UNIX_EPOCH`, in ms; the only clock read).

Behaviour:
- `PlayPause`: `Playing` (playback `is_playing`) → `Pause`; no playback or paused → `Play { context_uri: None, uris: vec![], offset: None }` (resume). Sent on the transport lane.
- `Next`/`Previous` → `SpotifyOp::Next`/`Previous`.
- `Seek(secs)` → `Seek { position_ms: seek_target(progress_at(now), secs, item.duration_ms) }`. With no playback: a toast `Nothing is playing on Spotify`.
- `Volume(delta)` → `SetVolume { volume: volume_after(percent, delta) }` where `percent` is `device.volume_percent`; unknown → `NO_VOLUME` toast.
- `Mute` → `mute_action(volume, spotify_mute_restore)`: `Mute{remember}` → set `spotify_mute_restore = Some(remember)` and send `SetVolume{0.0}`; `Unmute(p)` → send `SetVolume{p/100}` and clear the restore; `Nothing` → toast `NO_VOLUME` when the volume is unknown, else nothing. The restore is set when the op is accepted by the lane (not before); the reply for `Transport` does not touch it, so the restore is set at send time. Tests check the send.
- `Repeat` → `SetRepeat { mode: current.repeat.next() }` (current from playback; `Off` when none).
- `Shuffle` → `SetShuffle { on: !shuffle }`.
- `Stop` → toast `NO_STOP`.
- `Reload` → handled by the browser (Task 11).
- `LibraryOnly` → toast `NOT_IN_MODE`.
- `QueueAdd`, `QueueDrawer`, `Devices`, `Search` → Tasks 14, 15, 12 (here: `QueueAdd` on the highlighted browser track: nothing yet; stub with a comment).

- [ ] **Step 1: Write the failing tests** (in `app/spotify_mode.rs` tests; logged-in state with a playback fixture)

```rust
#[test]
fn space_pauses_a_playing_track_and_resumes_a_paused_one() {
    // playback is_playing true -> wait_for_op(Pause); is_playing false -> wait_for_op(Play { context_uri: None, uris: vec![], offset: None })
}

#[test]
fn the_seek_arrows_send_seek_from_the_clock_position() {
    // playback progress 4_000 read at 1_000, is_playing true, item duration 180_000; press Right (5 s)
    // the op is Seek { position_ms } with position_ms = progress_at(now) + 5_000 (within a second of now);
    // assert position_ms is between 9_000 and 15_000 (the clock moved by a few ms)
}

#[test]
fn plus_sends_one_percent_more_and_minus_one_less() {
    // device volume 40: '+' -> wait_for_op(SetVolume { volume: 0.41 }); '-' -> SetVolume { 0.39 }
}

#[test]
fn m_mutes_to_zero_and_m_again_restores_the_remembered_percent() {
    // volume 40: 'm' -> SetVolume 0.0 and spotify_mute_restore == Some(40);
    // then state volume 0: 'm' -> SetVolume 0.40 and restore None
}

#[test]
fn e_sends_the_next_repeat_mode_and_z_flips_shuffle() {
    // repeat Off: 'e' -> SetRepeat { mode: Context }; shuffle false: 'z' -> SetShuffle { on: true }
}

#[test]
fn s_is_a_toast_and_sends_nothing() {
    // 's' -> toast NO_STOP, sent_ops empty
}

#[test]
fn a_library_only_key_toasts_not_in_spotify_mode() {
    // 'A' -> toast NOT_IN_MODE; sent_ops empty
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- app::spotify_mode`.

- [ ] **Step 3: Implement** the behaviour above. Use `self.ask_spotify(SpotifyAsk::Transport, op)`; if it returns false, toast `Spotify is not connected`. Read the playback from `self.spotify_state` (the poll keeps it fresh; a 500 ms lag is accepted).

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- app::spotify_mode` — all pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_mode.rs
git commit -m "feat(tui): the transport keys drive Spotify in the mode

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 14: Queue add and the read-only Spotify queue drawer

**Files:**
- Modify: `znicz-tui/src/app/spotify_mode.rs` (`a` → `QueueAdd`, `]` → drawer)
- Modify: `znicz-tui/src/app/spotify_ui.rs` (`Queue` reply stores `spotify_queue`)
- Modify: `znicz-tui/src/app.rs` (`]` in the mode toggles `queue_open`; opening sends `SpotifyOp::Queue` on the browse lane; the `]` global arm defers to the mode)

**Interfaces:**
- Consumes: `SpotifyOp::QueueAdd { uri }`, `SpotifyOp::Queue`, `SpotifyReply::Queue { current, next }`, `SpotifyAsk::Queue`.
- Produces: `pub(crate) fn spotify_queue_add(&mut self)` (adds the highlighted track's `uri` from the browser level's `Item` of kind `Track`, or `Album`/`Playlist` uri is refused with the toast `Spotify's queue takes one track at a time`); `pub(crate) fn toggle_spotify_queue(&mut self)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_sends_queue_add_for_the_highlighted_track() {
    // a Tracks level with one track, cursor on it, 'a' -> wait_for_op(QueueAdd { uri })
}

#[test]
fn a_on_an_album_refuses_with_a_toast_and_sends_nothing() {
    // Albums row highlighted, 'a' -> toast "Spotify's queue takes one track at a time"; sent_ops empty
}

#[test]
fn bracket_opens_the_spotify_queue_and_asks_for_it() {
    // ']' -> queue_open true, wait_for_op(Queue); reply Queue { current, next } stores spotify_queue
}

#[test]
fn bracket_in_the_mode_never_opens_the_znicz_queue_drawer_without_the_spotify_read() {
    // mode on: ']' -> the drawer context is SpotifyQueue (not Znicz's queue)
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- queue`.

- [ ] **Step 3: Implement.** `Queue` reply arm in `on_spotify_done`: `Ok(SpotifyReply::Queue { current, next }) => self.spotify_queue = Some((current, next))`. `Err` → toast error. The queue is re-read every time the drawer opens; while it is open, the tick re-requests it every 3 s (`spotify_queue_requested` flag, like Devices, to keep one pending).

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- queue`.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_mode.rs znicz-tui/src/app/spotify_ui.rs
git commit -m "feat(tui): queue a Spotify track and show Spotify's queue read-only

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 15: The Spotify Connect device list and transfer

**Files:**
- Modify: `znicz-tui/src/app/spotify_mode.rs` (`,` opens `Modal::SpotifyDevices`; Enter sends `Transfer`)
- Modify: `znicz-tui/src/app.rs` (`on_global_key`: `,` in the mode opens the Spotify list; the Znicz devices modal keeps `,` out of the mode; `on_spotify_devices_key` routed in `handle_key` for `Modal::SpotifyDevices`)

**Interfaces:**
- Consumes: `spotify_state.devices`, `Cursor` (`step`, `first`, `last`), `SpotifyOp::Transfer { device_id, play: true }`.
- Produces: `pub(crate) fn toggle_spotify_devices(&mut self)`; `pub(crate) fn on_spotify_devices_key(&mut self, key: KeyEvent)`.

Behaviour: opening sends `SpotifyOp::Devices` (browse lane, so the player's list is refreshed) and sets the cursor to the active device. Enter sends `Transfer { device_id, play: true }` on the transport lane and closes the list. A device that does not take over comes back as `unavailable` and is shown by the `Transport` arm of `on_spotify_done` (toast). Esc closes.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn comma_in_the_mode_opens_the_spotify_list_not_the_output_list() {
    // mode on, ',' -> modal == Modal::SpotifyDevices (not Modal::Devices); wait_for_op(Devices)
}

#[test]
fn enter_transfers_playback_to_the_chosen_device() {
    // devices: Kitchen (id "k1") and Desk ("d1"); cursor on Desk; Enter -> wait_for_op(Transfer { device_id: "d1", play: true })
    // modal == None afterwards
}

#[test]
fn a_transfer_that_does_not_take_over_shows_the_error() {
    // FakeHost returns Err(Spotify { kind: Unavailable, message: "device did not take over" }) for Transfer
    // toast_texts contains that message
}

#[test]
fn an_empty_device_list_says_so() {
    // no devices: the modal's rows() is empty; the view test in Task 16 checks the text
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- devices`.

- [ ] **Step 3: Implement.** `FakeHost` needs `reply` support for `Transfer` failing: use `configure_refusal`-style: add `pub transfer_refusal: Mutex<Option<ZniczError>>` in the test-only `FakeHost` (spotify_ui tests) — the test adds the field. `on_spotify_devices_key`: `j`/`k`/arrows move `spotify_devices_cursor` over `spotify_state.devices`; `Enter` transfers; `Esc` closes (the global Esc path closes modals in the list, so add the variant there).

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- devices`.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_mode.rs znicz-tui/src/app/spotify_ui.rs
git commit -m "feat(tui): choose a Spotify Connect device with , in the mode

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 16: Drawing the browser, the search prompt, the queue drawer and the device list

**Files:**
- Create: `znicz-tui/src/views/spotify.rs`
- Modify: `znicz-tui/src/views/mod.rs` (`mod spotify;`; the list region draws `spotify::render_browser` when the mode is on and the view is Browse; the drawer draws `spotify::render_queue` in the mode; the modal `Modal::SpotifyDevices` draws `spotify::render_devices`)
- Modify: `znicz-tui/src/hit.rs` (`pub spotify_rows: Option<ListHit>`)
- Modify: `znicz-tui/src/app/spotify_browse_ui.rs` (`on_left_click` for the browser rows: select only; the wheel moves the cursor)

**Interfaces:**
- Consumes: `pane_block`, `highlight`, `breadcrumb`, `prompt_line`, `placeholder`, `visible_window` (`views/window.rs`), `Level::rows`, `SpotifyBrowser::crumbs`, `spotify_queue`, `spotify_state.devices`.
- Produces:

```rust
pub fn render_browser(frame: &mut Frame, area: Rect, app: &mut App);
pub fn render_queue(frame: &mut Frame, area: Rect, app: &mut App);
pub fn render_devices(frame: &mut Frame, list: Rect, app: &mut App);
```

Drawing rules:
- Browser: `pane_block(title, focused=true, right)` where the title is `Spotify · <user> · <device>` (user from `account`, device from `playback.device` or `Nothing playing`). The breadcrumb `crumbs().join(" › ")` is the bottom right. Rows: the top level draws `TOP` labels (`▸` marks the cursor row through `highlight`), a list level draws `Row::Item`s as `name` then `subtitle` dim after it, a `Row::Header` is a `theme::strong()` line with no selection. A `notice` is drawn as the last line of the pane in `theme::warn()`. When the search prompt is open the last row of the pane is `prompt_line("/ ", edit)`. Empty level with `pending == 0`: `placeholder("Nothing here")`; with `pending > 0`: `placeholder("Loading…")`.
- `app.hits.spotify_rows = Some(ListHit { inner, offset, len })` for the rows (`inner` is the pane's inner area without the title and the bottom notice row).
- Drawing the window uses `visible_window(len, Some(cursor), offset, height)`; the offset is kept in a `ListState`-like `usize` field `spotify_row_offset` on App (add it, default 0).
- Queue: `pane_block("Spotify queue", …)`; `Now` row with the current track as `title — artist` and then `Next` rows; empty: `placeholder("Spotify's queue is empty")`; no reply yet: `Loading…`. No cursor and no click.
- Devices: `pane_block("Spotify Connect", …)` over `list_modal_rect(list)`; rows are the device names with `kind` dim after them, the active device marked `● `; empty: `placeholder("No Spotify devices: open Spotify on a device")`. `app.hits.overlay` and `app.hits.overlay_list` are set as the Znicz devices modal does.

- [ ] **Step 1: Write the failing tests** (buffer tests on `TestBackend`, 80 × 24, using `screen_rows`-style helper)

```rust
#[test]
fn the_browser_shows_the_four_top_entries_with_the_cursor_on_the_first() {
    // mode on, logged in, Browse: rows contain "Search", "Playlists", "Liked songs", "Albums"
}

#[test]
fn an_empty_list_while_loading_says_loading() { /* pending > 0 -> "Loading…" */ }

#[test]
fn an_unavailable_playlist_notice_is_drawn_under_the_rows() { /* notice text present in a row */ }

#[test]
fn the_search_prompt_is_drawn_when_open() { /* "/ kind" appears */ }

#[test]
fn a_click_on_a_row_selects_it_and_does_not_play() {
    // hits.spotify_rows row 2 -> cursor 2; sent_ops has no Play
}

#[test]
fn the_queue_drawer_says_empty_when_spotify_has_nothing_queued() { /* "Spotify's queue is empty" */ }

#[test]
fn the_device_list_says_so_when_there_are_no_devices() { /* placeholder text */ }
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- views::spotify`.

- [ ] **Step 3: Implement** to the rules above. Keep the drawing functions free of state changes except `hits` and the offset.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- views::spotify` and `cargo test -p znicz-tui -- golden_` — pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/views/spotify.rs znicz-tui/src/views/mod.rs znicz-tui/src/hit.rs znicz-tui/src/app/spotify_browse_ui.rs
git commit -m "feat(tui): draw the Spotify browser, its search, queue and device list

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 17: The Spotify Stage and transport

**Files:**
- Modify: `znicz-tui/src/stage_model.rs` (`StageModel::from_spotify`, `TransportModel::from_spotify`)
- Modify: `znicz-tui/src/views/stage.rs` (`render` takes the model from the source; `render_cover` takes `Option<CoverKey>`; the `keep_previous` slot is keyed by the URL for Spotify)
- Modify: `znicz-tui/src/views/now_playing.rs` (`render_transport` takes a `TransportModel` from the source)
- Modify: `znicz-tui/src/views/mod.rs` (the Stage footer message row is unchanged; the Browse transport is drawn by the model)

**Interfaces:**
- Produces:

```rust
impl StageModel {
    /// The Spotify Stage. `now_ms` is Unix milliseconds (for `progress_at`).
    pub fn from_spotify(state: &SpotifyState, now_ms: u64) -> Self;
}
impl TransportModel {
    pub fn from_spotify(state: &SpotifyState, now_ms: u64) -> Self;
}
```

Rules:
- `title`: the item's `title` when playing, else `Nothing playing on Spotify`.
- `head`: the artist line (`artist_line()`, `Subtitle`) and `album · year · track N` (`Dim`, skipping missing parts; the same join as Znicz). With no item: empty.
- `seek`: `SeekModel::Bar { position: progress_at(now) as Duration, total: Some(duration_ms) }`; with no playback, `None`.
- `status`: `Playing` when `is_playing`; `Paused` when playback exists and is not playing; `Stopped` otherwise.
- `volume`: `Some(VolumeModel { volume: percent/100, muted: restore.is_some() })` when the device reports a percent; `None` otherwise or with no playback. (The mute restore is passed by the caller: `from_spotify(state, now_ms, muted: bool)`. Use this signature: `from_spotify(state: &SpotifyState, now_ms: u64, muted: bool)`.)
- `repeat`: `repeat_mode(playback.repeat)`; `shuffle`: `playback.shuffle`.
- `signal` (with a playback): `source` → `spotify` (`Plain`), `device` → `<name> (<kind>)` (`Plain`; `unknown device` when there is none), `path` → `Spotify Connect` (`Good`). Labels `source`, `device`, `path`.
- `explanation`: `decoded and played by the Spotify app on that device; Znicz does not touch the audio`.
- `cover`: `Some(CoverKey::Url(url))` when the item has an `image_url`; else `None`.
- `empty`: `Some(EmptyState { text: "Nothing playing on Spotify", hint: "/ search · O znicz" })` when there is no item.

`TransportModel::from_spotify` uses the same title, subtitle (`artist · album`), seek, volume, repeat, shuffle, and `signal`: `("spotify", "<device name> (<kind>)")`, with `path_badge` `("Spotify Connect", Tone::Good)` when status is not `Stopped`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_spotify_model_has_the_path_signal_rows_and_the_explanation() {
    // a fixture SpotifyState with playback (item "So What", device "Kitchen" kind "Speaker"):
    // signal labels ["source", "device", "path"], values ["spotify", "Kitchen (Speaker)", "Spotify Connect"]
    // explanation == "decoded and played by the Spotify app on that device; Znicz does not touch the audio"
}

#[test]
fn nothing_playing_on_spotify_has_the_logo_text_and_hint() {
    // SpotifyState::default(): title "Nothing playing on Spotify", empty.hint "/ search · O znicz", cover None
}

#[test]
fn the_cover_is_the_track_image_url() {
    // item.image_url Some("https://i.scdn.co/image/a"): cover == Some(CoverKey::Url("https://i.scdn.co/image/a".into()))
}

#[test]
fn a_paused_track_shows_a_paused_marker_and_no_path_badge_when_stopped() { /* status Paused */ }
```

Plus golden tests in `views/stage.rs`, mirroring the Task 1 set: `golden_spotify_wide_stage` (100 × 24, cover off so no network), `golden_spotify_narrow_stage` (48 × 24), `golden_spotify_nothing_playing` (100 × 24). Build the golden the same way as Task 1 Step 2.

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- from_spotify golden_spotify`.

- [ ] **Step 3: Implement.** In `render`: `let model = if app.spotify_mode { StageModel::from_spotify(&app.spotify_state, now_unix_ms(), app.spotify_mute_restore.is_some()) } else { StageModel::from_player(state, &app.stations, shown.cover_search, shown.cover_protocol != CoverProtocol::Off) };`. For Spotify, `render_cover` receives `model.cover` and keys `keep_previous` by the URL. `now_unix_ms()` is defined once in `app/spotify_mode.rs` as `pub(crate) fn now_unix_ms() -> u64`.

Footer for the Stage in the mode: `Context::SpotifyStage` (Task 8).

- [ ] **Step 4: Run** — `cargo test -p znicz-tui` — all pass, including the Task 1 goldens.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/stage_model.rs znicz-tui/src/views/stage.rs znicz-tui/src/views/now_playing.rs znicz-tui/src/views/mod.rs znicz-tui/src/app/spotify_mode.rs
git commit -m "feat(tui): the Stage and transport show what Spotify plays

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 18: Library-only keys, leaving the mode, and the error paths

**Files:**
- Modify: `znicz-tui/src/app/spotify_mode.rs` (leaving the mode; the errors of the Transport and Browse lanes)
- Modify: `znicz-tui/src/app/spotify_ui.rs` (`on_spotify_done` `Transport` arm uses `error_plan`; a `Network` error on a change forces a read)
- Modify: `znicz-tui/src/app.rs` (the `LIBRARY_ONLY_KEYS` toast from `key_action`)

**Interfaces:**
- Consumes: `spotify_mode::{error_plan, ErrorPlan, LIBRARY_ONLY_KEYS, NOT_IN_MODE}`.
- Produces: `fn spotify_change_failed(&mut self, error: ZniczError)` → toast `plan.text` as error; if `plan.reread`, `spotify_polled_at = None`; if `plan.stops_polling`, `spotify_unavailable = Some(plan.text)`.

Behaviour:
- Leaving the mode keeps Spotify playing: no Spotify op is sent by the toggle. Test checks `sent_ops` is empty on leaving. The Znicz engine's pause rule runs in the player, so nothing is duplicated here.
- The errors through the Transport arm: `NoActiveDevice` toasts the core text (mentions `,`); `PremiumRequired` toasts `needs Spotify Premium`; `Unavailable` toasts the device message; `Network` toasts the "may still happen" text and forces a read; `OutdatedPlayer`/`NoService` stop polling and toast why.
- A `Browse` error for an old generation is ignored (Task 11).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn leaving_the_mode_sends_nothing_to_spotify() {
    // mode on, leave with 'O': sent_ops(&host) is empty and the transport host has no ops
}

#[test]
fn library_only_keys_in_the_mode_toast_not_in_spotify_mode() {
    // 'A', 'L', 'F', 'U', 'J' each: toast_texts contains "not in Spotify mode"
}

#[test]
fn a_network_error_on_a_change_says_it_may_still_happen_and_rereads() {
    // Transport reply Err(Network) -> toast contains "may still happen"; spotify_polled_at is None
}

#[test]
fn no_active_device_says_how_to_pick_one() {
    // Transport reply Err(NoActiveDevice, "Spotify has no active device: open Spotify somewhere, or pick one with ,") -> toast contains "pick one with ,"
}

#[test]
fn an_outdated_player_stops_polling_in_the_mode() {
    // spotify_mode on; status_error = OutdatedPlayer; poll_spotify; spotify_unavailable is Some; a second poll sends nothing
}
```

- [ ] **Step 2: Run to see them fail** — `cargo test -p znicz-tui -- leaving library_only network no_active outdated`.

- [ ] **Step 3: Implement** as described. The `Transport` arm: `Err(ZniczError::Spotify { kind, message }) => self.spotify_change_failed(kind, message)`; other `ZniczError` values use `to_string()`.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui` — all pass.

- [ ] **Step 5: Commit**

```bash
git add znicz-tui/src/app.rs znicz-tui/src/app/spotify_mode.rs znicz-tui/src/app/spotify_ui.rs
git commit -m "feat(tui): Spotify mode errors, library-only keys, and leaving the mode

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 19: Docs: wiki, help, README

**Files:**
- Modify: `wiki/Architecture/TUI.md` (Views: the browser and the Spotify Stage; Stage: the shared model; Keys: the `SPOTIFY` table; a "Spotify mode (`O`)" section; Modules: `stage_model`, `spotify_browse`, `spotify_mode`, `views/spotify`, the two `app` files, the transport lane)
- Modify: `wiki/Architecture/Spotify.md` (the TUI lanes: browse and transport, and the mode's polling)
- Modify: `znicz-tui/src/keys.rs` (the `SPOTIFY` table completed with every key of Tasks 8–15: `Space`, `n`/`p`, `→`/`←`, `Shift-→`/`Shift-←`, `+`/`-`, `e`, `z`, `m`, `,`, `]`, `a`, `/`, `Enter`, `h`/`l`, `r`, `s`, `A`/`L`/`F`/`U`/`J` as `not in Spotify mode`)
- Modify: `README.md` (the features line for Spotify only)

**Interfaces:** none.

- [ ] **Step 1: Write the failing test** (in `keys.rs`): every key listed in the `SPOTIFY` table appears in `help_sections(Context::SpotifyBrowse)`, and `context_hints(SpotifyBrowse)` labels are documented. The existing footer test covers the hints; add `spotify_table_lists_the_mode_keys` asserting the exact `keys` values `"O"`, `"Space"`, `"e"`, `","`, `"]"`, `"/"`, `"m"`.

- [ ] **Step 2: Run to see it fail** — `cargo test -p znicz-tui -- keys`.

- [ ] **Step 3: Write the docs.** Follow `.cursor/rules/wiki-sync.mdc`: the wiki says what the code does, nothing invented. The Spotify mode section lists the keys from the `SPOTIFY` table, the browser's levels, the 50-row pages, the search (10 per group, one request per kind), the unavailable notice, and the polling rates (500 ms in the mode, 3 s otherwise). `TUI.md` Stage: the shared `StageModel`/`TransportModel`.

- [ ] **Step 4: Run** — `cargo test -p znicz-tui -- keys` — pass. Also `grep -n "O znicz\|Spotify mode" wiki/Architecture/TUI.md` shows the new section.

- [ ] **Step 5: Commit**

```bash
git add wiki/Architecture/TUI.md wiki/Architecture/Spotify.md znicz-tui/src/keys.rs README.md
git commit -m "docs: the Spotify mode in the wiki, help and README

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 20: Full check

No code change expected. If a step fails, fix it in the task that owns the code with a `fix(tui):` commit, then run this task again.

**Files:** none unless a fix is needed.

- [ ] **Step 1: Format** — `cargo fmt --all -- --check`. Expected: no output. If not, `cargo fmt --all` and commit `style(tui): format the Spotify mode code`.

- [ ] **Step 2: Clippy with warnings denied** — `cargo clippy --workspace --all-targets -- -D warnings`. Expected: no errors. Likely findings: `too_many_arguments` on `push_tail`-style helpers, `needless_borrow` on `&*host`, `manual_let_else`. Fix each in place.

- [ ] **Step 3: Tests** — `cargo test --workspace`. Expected: all pass.

- [ ] **Step 4: Release build of the binary** — `cargo build --workspace --release` (the `main.rs` three-connection wiring is only compiled in the binary).

- [ ] **Step 5: Clean the build output** — `cargo clean` (the project rule in `MEMORY.md`: target/ filled the disk).

- [ ] **Step 6: Commit the ticked plan**

Tick each step in this file and commit only the plan:

```bash
git add docs/superpowers/plans/2026-11-26-spotify-5-mode.md
git commit -m "docs(plan): tick Spotify part 5 full check

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

### Task 21: Manual check

Run with a real Spotify Premium account and the Development Mode app from Part 4 (logged in already). No code is written in this task. Note any surprise and fix it in its own commit (`fix(tui): <what the check found>`), then run Task 20 again.

- [ ] **Step 1: Enter the mode.** Press `O` from Browse. Expected: the browser shows Search, Playlists, Liked songs, Albums. Press `O` while logged out (log out in Settings first, then re-login after the check) and expect `Spotify: log in under Settings → Spotify (S, 6)`.
- [ ] **Step 2: Browse.** Enter Playlists, Liked songs and Albums. Expected: rows load in pages; scroll past the 45th row and the list grows. Open a playlist you do not own (`l`) and expect the notice `Spotify only lists the tracks of playlists you own or collaborate on; Enter still plays it`; `Enter` on it still plays.
- [ ] **Step 3: Search and play.** `/`, type an artist, Enter. Expected: Tracks, Albums, Artists, Playlists headers, at most 10 rows each. Enter on a track plays it; Enter on an album plays it from the top. Record whether any group returns more than 10 (the limit decision in this plan).
- [ ] **Step 4: Transport.** Space pauses and resumes; `n`/`p` skip; Right/Left seek 5 s and Shift-Right/Shift-Left 30 s; `+`/`-` move the volume 1 %; `m` mutes and `m` again restores; `e` cycles off → all → one; `z` shuffles. The message row says nothing for each success.
- [ ] **Step 5: Devices.** `,` lists the Spotify devices. Choose another device: playback moves and a device that does not take over shows its error.
- [ ] **Step 6: Queue.** `a` on a track adds it; `]` shows Spotify's queue read-only with the added track after the current one.
- [ ] **Step 7: The Stage with a cover.** `v` shows the Spotify Stage with the album cover (the previous cover stays while the next loads), the signal rows `source spotify`, `device <name> (<kind>)`, `path Spotify Connect`, and the explanation. Footer ends in `O znicz`.
- [ ] **Step 8: Leave while Spotify plays.** Start a track, press `O`. Expected: Spotify keeps playing; the message row shows `♫ Spotify: <title> — <artist> · O to switch` once no toast is live.
- [ ] **Step 9: Local music pauses Spotify.** Play a local track from Znicz (Library, Enter). Expected: Spotify pauses (the player's rule), and the message row shows nothing about Spotify once paused.
- [ ] **Step 10: Commit.** Tick the checks that passed and commit:

```bash
git add docs/superpowers/plans/2026-11-26-spotify-5-mode.md
git commit -m "docs(plan): Spotify part 5 manual check

Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_012jyZm3UyFwgZfBtgburioV"
```

---

## Self-review against the spec and the brief

| Brief item | Where |
| --- | --- |
| A1 `StageModel` with title, artist, album line, seek, volume/mute/repeat/shuffle, Signal rows, explanation, cover key, empty state; `from_player` | Task 2 |
| A2 `TransportModel` | Task 3 |
| A3 goldens (wide, narrow, no-cover, stream) before the refactor | Task 1 |
| B4 `O` toggles from Browse or Stage; flag beside the view; `v`/`i` still switch; not-logged-in toast; `O` free; resets the unavailable stop | Task 8 (flag, toast, contexts, `Esc`), Task 9 (reset) |
| B5 status poll in mode every 500 ms; slow poll every 3 s in Znicz mode; one device list | Task 9 (rules, `state.devices`), Task 10 (message row) |
| B6 transport lane with its own `IpcClient`, three connections, three `run_tui` sites | Task 7 |
| B6 generation counter; stale `Page` dropped | Task 5 (`apply_page`), Task 11 (test) |
| B6 `SpotifyAsk` payloads: list, offset, generation, search query and kinds | Task 7 (`Browse { generation, list, offset }`, `BrowseList::Search { query, kind }`) |
| B6 replies routed by mode, not Settings' catch-all | Task 7 (`on_spotify_done` arms), Task 11 (`on_browse_done`), Task 18 (`spotify_change_failed`) |
| B7 single drill-down column; top level four entries; `l`/`Enter` in, `h` back | Task 5, Task 11, Task 16 (draw) |
| B7 50 rows, next page within 5 rows of the end, `has_more`/offset | Task 5 (`PAGE`, `NEAR_END`, `next_page`) |
| B7 `/` search prompt with `line_edit`, Enter searches, groups, 10 per group | Task 12 (prompt), Task 5 (`start_search`, groups), Spec refinements (no further paging) |
| B7 artist → albums; no top tracks | Task 5 (`ArtistAlbums`) |
| B7 `Enter` on track plays in context with offset; search tracks as `uris` | Task 5 (`enter`, tests) |
| B7 `Enter` on album/playlist plays from top; `l` on unowned playlist shows unavailable; `Enter` still plays | Task 5 (`open`, notice), Task 11 (test) |
| B7 `a` queue add; `]` read-only Spotify queue | Task 14 |
| B7 mouse rows select, `Enter` plays | Task 16 (hit rows, select only), Task 11 (`Enter`) |
| B8 Space, `n`/`p`, seek 5/30 from `progress_at`, `+`/`-`, `e`, `z`, `m` with restore | Task 6 (rules), Task 13 (behaviour) |
| B8 `,` Spotify Connect list; Transfer; error shown | Task 15 |
| B8 library-only keys toast `not in Spotify mode` | Task 6 (`LIBRARY_ONLY_KEYS`), Task 18 (test) |
| B8 footer and help Spotify contexts | Task 8 (contexts, footer), Task 19 (help table) |
| B9 leaving keeps Spotify playing; no duplicate pause rule | Task 18 (test: no ops sent) |
| B10 Spotify Stage and transport; signal rows; explanation; cover via `CoverKey::Url`; previous cover stays; nothing-playing text; footer `O znicz` | Task 4 (cover), Task 17 (models, render), Task 8 (footer hint) |
| B11 errors: `no_active_device` mentions `,`; `premium_required`; `unavailable`; `network` may still happen; `outdated_player`/`no_service` stop | Task 6 (`error_plan`), Task 18 (tests) |
| B11 errors to message row and Messages list | Task 18 (toasts; toasts feed the message row and the Messages list) |
| B12 wiki, help, README features line | Task 19 |
| B13 manual check | Task 21 |
| B14 out of scope: version bump, CHANGELOG, README beyond features, Spotify tracks in Znicz's queue | Not planned here (Global Constraints and the file table) |
| Keep logic pure and unit-tested: key → op, browser state machine, `StageModel`/`TransportModel`, mute restore, repeat cycle, seek clamp, message-row text | Tasks 2, 3, 5, 6, 10 |
| Tests use `FakeHost`, `wait_for_op`, `sent_ops` | Tasks 7, 8, 9, 11–15, 18 |

**Placeholder scan:** the test bodies in Tasks 5, 8, 9, 12, 14 and 15 name the assertion in a comment where the setup is routine (a level pushed, a page applied). Each comment states the exact op or text to assert, so the implementer writes only the setup. No `TBD`.

**Type consistency:**
- `SpotifyAsk` (Task 7) is used in Tasks 8–18 with the variants `Browse { generation, list, offset }`, `Queue`, `Transport`, plus the Part 4 variants.
- `BrowseList` and `SpotifyBrowser` (Task 5) are used in Tasks 7, 11, 12, 16.
- `SpotifyKey` and `key_action` (Task 6) are used in Tasks 11, 13, 18.
- `now_unix_ms()` is defined in Task 13 (`app/spotify_mode.rs`) and used in Task 17.
- `spotify_mute_restore: Option<u8>` is an App field (Task 8 names it; Tasks 13 and 17 use it).
- `spotify_queue`, `spotify_devices_cursor`, `spotify_search`, `spotify_mode`, `spotify_browser`, `spotify_row_offset` are App fields, added in the task that first uses them.
- `StageModel::from_spotify(state, now_ms, muted)` (Task 17) is the one signature; Task 17 says `muted` is passed by the caller.
- `message_line` takes `spotify: Option<&str>` (Task 10); its callers pass `None` in Spotify mode's Stage and the Spotify line otherwise.
- `error_plan` (Task 6) returns `ErrorPlan { text, stops_polling, reread }` and is used in Task 18.

**Proportion:** the plan describes the code by its signatures, the rules each function must follow, and the tests that pin it. Full bodies appear only for the pure test cases that pin the decisions (seek clamp, mute, volume, key mapping, browser paging and generation). The rest is named by its rule and test, so the implementer writes the body.

**Task order:** Group A (Tasks 1–3) changes no behaviour and ends with the goldens green. Group B's first task (Task 4) is independent. Tasks 5 and 6 are pure and come before any App wiring. Task 7 adds the lanes that Tasks 8–18 use. Task 8 adds the mode flag that Tasks 9–18 check. Each later task only uses names defined in an earlier one.

## Uncertain points for the reviewer

- **Search paging.** The brief asks to page within a search group by offset; this plan does not, because Spotify returns at most 10 per type in Development Mode. Task 21 Step 3 checks it on the real account.
- **Liked songs playback.** No context URI exists; the plan plays the loaded liked tracks as `uris`. The real behaviour on a long list is unverified (`uris` may need a cap).
- **Playlist ownership.** The TUI cannot know ownership without the user's id, so it learns "unavailable" from the `Tracks` reply. The spec's wording is kept.
- **Lane and change dedupe.** Transport ops now come from a second connection. The player's Spotify change dedupe (one record for all Spotify changes) was reviewed in Part 3 for one connection. Two connections changing playback at once is not tested; Task 21 Step 4 exercises it lightly.
- **`s` in the mode.** The plan treats `s` as a toast (`Spotify has no stop`). The brief does not say; this is a judgment call.
- **The 3 s slow poll** adds one IPC call every 3 s while Znicz runs with a client ID set. It is cheap (a mutex read in the player) but it is new load on the player; Task 9's test pins the rate.
- **Cover fetch on the cover worker.** `fetch_cover` blocks up to the agent timeout (8 s connect). It runs on the cover worker thread, as the stream covers do, so the frame is not held.
- **Golden tests** (Tasks 1, 17) pin the current text rendering of cells, not colours. A colour regression would not be caught; the manual check covers it.

---

## Review Focus (checked)

The five input classes are in the Review Focus section above, each with its test. No further uncovered input class was found that would bite a user before Task 21 catches it.

---

**Plan complete and saved to `docs/superpowers/plans/2026-11-26-spotify-5-mode.md`.**
