# AudioAddict covers follow the song: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** RadioTunes / DI.FM covers change with the song (about 1–2 s late) instead of showing the previous song's cover.

**Architecture:** A new "try again shortly" answer (`Found::Soon` → `Resolution::Soon`) lets a source ask for a quick retry without letting later sources win. The TUI retries `Soon` after 1.5 s. The AudioAddict lookup asks the per-channel `currently_playing` endpoint (cache-busted), checks that its track is the stream title, and takes the cover from `tracks/{id}`.

**Tech Stack:** Rust, ureq 3, serde_json; tests use local `TcpListener` servers.

Spec: `docs/superpowers/specs/2026-11-13-audioaddict-cover-timing-design.md`.
Branch: `fix/audioaddict-cover-lag`. Every task starts with `git branch --show-current` and stops if it is not that branch.

Checks before each commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

---

### Task 1: `Found::Soon` and `Resolution::Soon` in the cover chain

**Files:** `znicz-core/src/cover/mod.rs` (enums ~line 38 and ~68, `StreamCovers::resolve` ~138, tests in the same file), any exhaustive `match` on `Found` / `Resolution` in the workspace (`grep -rn "Resolution::\|Found::Unavailable" --include=*.rs`).

- [ ] **Step 1: Tests** in `cover/mod.rs` tests (reuse the file's existing fake-source helpers; read them first):
  - a chain `[Soon-source, source-that-finds-art]` resolves to `Resolution::Soon`, and the second source's `find` is never called (count calls).
  - a source answering `Soon` is asked again on the next `resolve` (not remembered), even though `remembers()` is true.
- [ ] **Step 2:** run; fails to compile.
- [ ] **Step 3: Implement.**

```rust
/// One source's answer.
pub enum Found {
    // … existing variants …
    /// The source expects an answer shortly (its data is catching up): stop
    /// here, show what is on screen, and ask again soon. Not remembered.
    Soon,
}

pub enum Resolution {
    // … existing variants …
    /// A source asked to be asked again shortly; keep what is shown.
    Soon,
}
```

In `resolve`, inside the `match source.find(query)`:

```rust
Found::Soon => {
    tracing::debug!(source = source.name(), "cover source asks to retry soon");
    return Resolution::Soon;
}
```

Fix every other exhaustive match so it compiles (the TUI's `resolve_cover` maps `Resolution::Soon` to `ResolveResult::KeepPrevious` for now; Task 2 refines it).
- [ ] **Step 4:** checks pass.
- [ ] **Step 5: Commit** `"Let a cover source ask for a quick retry"`.

---

### Task 2: The TUI retries `Soon` after 1.5 s

**Files:** `znicz-tui/src/cover.rs` (`Slot` ~line 60, worker ~88–140, `get` ~170, `ResolveResult` / `resolve_cover` ~205, tests at the bottom).

- [ ] **Step 1: Test** (next to `an_unsure_answer_is_retried_after_the_interval`; copy its style and its fake source pattern): a source that answers `Found::Soon` on its first call and `Found::Art(PNG_1X1…)` afterwards. With `with_stream_refresh(Duration::from_secs(60))` and `with_soon_retry(Duration::from_millis(100))`, `wait_ready` on a titled stream key returns `Embedded` within the 5 s wait, and the source was called exactly twice. (With only the 60 s refresh it would never be retried in time, so this proves the short interval is used.)
- [ ] **Step 2:** run; fails.
- [ ] **Step 3: Implement.**
  - `const SOON_RETRY: Duration = Duration::from_millis(1500);`
  - `Slot.retry: bool` becomes `retry_after: Option<Duration>` (doc: "Retry the lookup this long after the last answer: unsure answers wait the refresh interval, `Soon` answers a moment.").
  - `ResolveResult::KeepPrevious` gets `{ retry_after: Duration }`. `resolve_cover` takes the two intervals (pass `stream_refresh` and `soon_retry` into the worker closure by value): `Unsure` → `KeepPrevious { retry_after: stream_refresh }`, `Soon` → `KeepPrevious { retry_after: soon_retry }`.
  - Worker: `KeepPrevious { retry_after }` sets `slot.retry_after = Some(retry_after)`; `Update` sets `None`.
  - `CoverCache` stores `soon_retry`; add `pub fn with_soon_retry(mut self, every: Duration) -> Self` like `with_stream_refresh`. Because the worker is spawned in `with_streams`, read both intervals in the worker from shared state (e.g. `Arc<Mutex<(Duration, Duration)>>` or two `Arc<AtomicU64>` millis) so the builder methods still take effect after spawning. Keep it simple and documented.
  - `get`: refresh when `!slot.refreshing && match slot.retry_after { Some(after) => elapsed >= after, None => untitled_stream && elapsed >= self.stream_refresh }`.
- [ ] **Step 4:** checks pass (the existing unsure / pending / untitled tests must stay green).
- [ ] **Step 5: Commit** `"Retry a 'soon' cover answer after 1.5 s"`.

---

### Task 3: AudioAddict lookup by the current track

**Files:** `znicz-core/src/audioaddict.rs` (rewrite the lookup; keep `AudioAddictNetwork`, `parse_audioaddict_channel`, `normalize_art_url`, the test server helpers), `znicz-core/src/cover/audioaddict.rs`, `znicz-core/src/lib.rs` re-exports (`audioaddict_cache_fresh`, `audioaddict_cover_url`, `join_audioaddict_art_url` are not used outside `znicz-core`; delete what becomes dead, with its tests).

**Lookup API:**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioAddictLookup {
    Found(String),
    NoArt,
    RefreshFailed,
    /// The channel is still on another track than the stream title.
    NotYet,
}

pub fn audioaddict_cover_lookup(network: AudioAddictNetwork, channel_key: &str, title: Option<&str>) -> AudioAddictLookup {
    audioaddict_cover_lookup_at(network, channel_key, title, DEFAULT_ORIGIN)
}
pub fn audioaddict_cover_lookup_at(network, channel_key, title, origin: &str) -> AudioAddictLookup
```

**Steps of `audioaddict_cover_lookup_at`:**
1. Channel id: a static cache `(origin, network) → (Instant, HashMap<String key, u64 id>)` built from `GET {origin}/v1/{slug}/currently_playing` (array of `{"channel_id":407,"channel_key":"smoothbeats",…}`; `channel_id` may be a number or a string). Refresh when missing, older than 1 hour, or the key is not in it. Fetch failure → `RefreshFailed`; key still unknown after a refresh → `NoArt`.
2. `GET {origin}/v1/{slug}/currently_playing/channel/{id}?t={unique}` where `unique` is nanoseconds since the epoch plus an atomic counter (two calls must differ). Body: `{"channel_id":407,"channel_key":"smoothbeats","track":{"id":1272704,"display_artist":"Else Born","display_title":"This Time/Remember","start_time":"…","duration":…}}`. Failure → `RefreshFailed`; no `track` / `track.id` → `NoArt`.
3. If `title` is `Some`: `current = format!("{} - {}", display_artist, display_title)`; if `fold(current.trim()) != fold(title.trim())` (use `crate::cover::title::fold`) → `NotYet`.
4. Track cover: static cache `(origin, network, track_id) → Option<String>` (clear when over 256 entries). Miss → `GET {origin}/v1/{slug}/tracks/{track_id}`; art = `asset_url`, else `images.default`, through `normalize_art_url`. Request failure → `RefreshFailed` (not cached). Cached `None` or no art → `NoArt`; else `Found(url)`.

Reuse the module's `fetch_json` (1 MB cap) and `agent()`.

**Source (`cover/audioaddict.rs`):**

```rust
pub struct AudioAddictSource {
    /// When each channel + title was first `NotYet`, to stop waiting after a while.
    waiting: Mutex<HashMap<String, Instant>>,
}
const GIVE_UP_AFTER: Duration = Duration::from_secs(30);
```

`find`: parse the channel (none → `NotFound`); `lookup = audioaddict_cover_lookup(network, &channel, query.icy_title.as_deref())`. Map: `Found(url)` → `Found::Url(url)` (and forget the waiting entry); `NoArt` → `NotFound`; `RefreshFailed` → `Unavailable`; `NotYet` → record the first-seen instant for the source key (`self.key(query)`); if waited ≥ `GIVE_UP_AFTER` → `NotFound`, else `Found::Soon`. Keep the map small (remove entries older than a few minutes when inserting). For testability add `fn find_at(&self, query, origin: &str, give_up: Duration) -> Found` used by `find` with `DEFAULT_ORIGIN` / `GIVE_UP_AFTER` (make `DEFAULT_ORIGIN` `pub(crate)` if needed). `remembers()` stays `false`.

- [ ] **Step 1: Tests** in `audioaddict.rs` (extend `serve_audioaddict` handlers to route `/currently_playing/channel/`, `/currently_playing`, `/tracks/`; count hits per route with separate counters or by recording paths). Each test uses its own server (own origin), so the static caches do not collide.
  - channel on the previous track → `NotYet`.
  - channel on the stream title (case and spacing differ) → `Found(tracks asset_url)`.
  - `tracks/{id}` has only `images.default` with `{?size,height,width,quality,pad}` → template stripped, `https:` prefixed.
  - two lookups of the same track hit `/tracks/` once.
  - `title = None` → current track's cover.
  - `currently_playing/channel` returns 500 → `RefreshFailed`; `tracks` returns 500 → `RefreshFailed`, and a later lookup retries it.
  - unknown channel key → `NoArt`.
  - two channel requests carry different `t=` values (record the request paths).
- [ ] **Step 2: Tests** in `cover/audioaddict.rs`: with a local server whose channel is on another track, `find_at(…, give_up = 100 ms)` → `Found::Soon`, then after sleeping past 100 ms → `Found::NotFound`; a matching track → `Found::Url`. Keep the existing key test.
- [ ] **Step 3:** run; fail.
- [ ] **Step 4:** implement; update `lib.rs` re-exports and any callers.
- [ ] **Step 5:** checks pass.
- [ ] **Step 6: Commit** `"AudioAddict covers come from the current track, not the lagging history"`.

---

### Task 4: Live test, docs, version

**Files:** `znicz-core/tests/cover_live.rs` (read its style; tests are `#[ignore]`), `wiki/Architecture/Cover-Sources.md`, `wiki/Architecture/TUI.md` (retry intervals), `wiki/Domain/Formats-and-Metadata.md` (AudioAddict `art_url` sentence), root `Cargo.toml` version → `0.13.2`.

- [ ] **Step 1:** add an ignored live test: `audioaddict_cover_lookup(AudioAddictNetwork::RadioTunes, "smoothbeats", None)` returns `Found(url)` with an `https://` URL. Run it once with `cargo test -p znicz-core --test cover_live -- --ignored audioaddict` and note the result in the report.
- [ ] **Step 2:** docs. Cover-Sources: AudioAddict now asks the channel's current track (`currently_playing/channel/{id}`, cache-busted because Cloudflare caches it for 4 s), checks it against the stream title, takes the cover from `tracks/{id}`; until the API catches up it answers *soon* (the chain stops, the TUI retries after 1.5 s, the previous cover stays); after 30 s it gives up and the search sources run. Describe `Found::Soon` / `Resolution::Soon` where the page lists answers. TUI.md: unsure covers retry every 15 s, *soon* answers after 1.5 s. Formats-and-Metadata: replace "AudioAddict `art_url`" wording with the current track's cover.
- [ ] **Step 3:** version `0.13.2`; `cargo build -q`; checks pass.
- [ ] **Step 4: Commit** `"Document AudioAddict cover timing; bump to 0.13.2"`.
