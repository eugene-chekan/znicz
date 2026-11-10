# Stream covers (`znicz-core/src/cover/`)

A radio stream has no file tags, so its cover comes from a **chain of
sources**. The chain runs in order and the first image that downloads wins.
The TUI asks for it through `CoverKey::Stream` (see [TUI](TUI.md#stage)).

## The chain

`default_sources()` in `cover/mod.rs` returns, in order:

| # | Source | Name | What it uses |
|---|--------|------|--------------|
| 1 | `IcyUrlSource` | `icy_url` | The ICY `StreamUrl` when it is an image URL |
| 2 | `AudioAddictSource` | `audioaddict` | Song `art_url` for RadioTunes, DI.FM, RockRadio, JazzRadio, ClassicalRadio, ZenRadio |
| 3 | `DeezerSource` | `deezer` | Artist / title search on Deezer |
| 4 | `MusicBrainzSource` | `musicbrainz` | Artist / title search on MusicBrainz, then the Cover Art Archive |
| 5 | `StationArtSource` | `station_art` | The station's local `art` file |

When every source is empty the TUI draws the Znicz logo.

Notes on each source:

- **ICY URL.** A `StreamUrl` that is a web page (HTML) is not an image and is
  skipped. Only a URL whose download is an image counts.
- **AudioAddict.** Uses the channel's own API cache (about 15 s). Its answers
  are not remembered by the chain.
- **Deezer.** Queried with a plain `"{artist} {title}"` string. The advanced
  `artist:"…" track:"…"` syntax returns nothing now. Hits are kept only when the
  artist name matches. A `200` body with an `error` field, HTTP 429 and 5xx are
  *unavailable*. Other 4xx are *not found*.
- **MusicBrainz.** Lucene query `artist:"…" AND recording:"…"`. A hit needs
  score 90 or more and a matching artist. The first 3 releases are probed at the
  Cover Art Archive `front-500` size.
- **Station art.** A readable local file. A missing file is *not found*.

## Types

In `cover/mod.rs`:

- `StreamQuery`: `stream_url`, `station_name`, `icy_title`, `icy_url`,
  `station_art`, and `search` (artist / title search allowed).
- `Found`: what one source answered. `Url(String)` (to download), `Art(CoverArt)`
  (bytes in hand), `NotFound`, or `Unavailable`.
- `CoverSource` trait: `name()`, `key()` (what the answer depends on, or `None`
  when the source does not apply), `find()` (runs on the cover worker thread,
  may block), and `remembers()` (default `true`).
- `Resolution`: `Found(CoverArt)`, `Nothing` (every applicable source answered
  without an image), or `Unsure` (no image, and at least one source was
  unavailable). `Unsure` keeps the cover already on screen.
- `StreamCovers`: runs the chain. `resolve(&StreamQuery) -> Resolution`.

A downloaded image counts only if it is `image/*` by `Content-Type`, or it has
PNG or JPEG magic bytes. A download that fails is remembered as *not found* for
that source, and the chain moves on.

## Remembered answers

- Answers are keyed by `(source name, source key)`. Queries that share a key
  share an answer.
- At most **256** answers are kept (`ANSWER_CAP`), oldest dropped first. They
  last for the session only; nothing is written to disk.
- `Unavailable` is never remembered, so the next request asks again.
- A source with `remembers() == false` (AudioAddict) is asked every time.

## TUI behaviour

- A stream with an ICY title is resolved once per title. A pending lookup is
  never sent twice.
- A stream without a title is refreshed every **15 s** (`STREAM_REFRESH`), so
  the cover follows the track.
- An `Unsure` answer is retried once per interval.
- Requests that were queued for a track that has since changed are dropped.
- While the next cover loads, Stage keeps the previous cover of the same stream
  on screen (`views/stage.rs`).

## Switches

- `config.toml`: `[covers] search = true` (default on). Turns off the artist /
  title search (Deezer, MusicBrainz) for every stream.
- `stations.toml`: `cover_search = false` on one station. Same effect for that
  station only. The ICY URL, AudioAddict and station art still apply.

Both values are wired through `TuiConfig.cover_search` in `znicz/src/main.rs`.

## Privacy

With search on, the playing stream's ICY title (artist and track) is sent to
Deezer and, when Deezer has no match, to MusicBrainz. Set `search = false` or
`cover_search = false` to keep titles on the device.

## MusicBrainz etiquette

MusicBrainz asks clients to be polite. The source:

- sends the User-Agent `znicz/<version> ( https://github.com/eugene-chekan/znicz )`
  (`cover/http.rs`),
- makes at most one request per second (`MIN_GAP`).

## Adding a source

1. Implement `CoverSource` in a new file `znicz-core/src/cover/<name>.rs`. Keep
   `key()` cheap and return `None` when the source does not apply.
2. Add a constructor `at(origin)` that takes the service origin, so tests can
   point it at a local server.
3. Test it against `cover::http::test_server::serve` (a local HTTP server used by
   the existing sources). Cover the found, not-found, and unavailable paths.
4. List it in `default_sources()` in `cover/mod.rs`, in the place the order
   requires. Update the order test in the same file and this page.
5. Run the live check: `cargo test -p znicz-core --test cover_live -- --ignored --nocapture`.
