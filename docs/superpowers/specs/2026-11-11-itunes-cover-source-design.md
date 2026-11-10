# iTunes cover source and lead-artist retry

**Date:** 2026-11-11
**Status:** Approved
**Crate:** `znicz-core`
**Extends:** [Stream cover sources](2026-11-10-stream-cover-sources-design.md)

## Problem

On FluxFM Jazzradio, about 2 in 10 tracks get no cover, while moode finds them.
moode's covers come from the iTunes Search API (`is1-ssl.mzstatic.com/...`).
Separately, jazz credits such as `Anna Maria Jopek & Friends with Pat Metheny`
make poor search text: Deezer and iTunes return nothing for the full credit,
while the lead artist alone finds the track.

## Goals

1. An `itunes` cover source in the chain, between Deezer and MusicBrainz.
2. Every search source (Deezer, iTunes, MusicBrainz) retries with the lead
   artist when the full credit finds nothing.

## Behaviour

- Chain order: `icy_url`, `audioaddict`, `deezer`, **`itunes`**, `musicbrainz`,
  `station_art`.
- **Lead artist:** the credit up to the first joiner, matched case-insensitively:
  ` & `, ` and `, ` with `, ` feat. `, ` feat `, ` ft. `, ` featuring `, `, `,
  ` x `, ` + `. No joiner → the credit itself.
- **Attempts:** each search source searches with the full credit first; if that
  finds no matching hit, it searches again with the lead artist (only when it
  differs). A network failure or a retryable status on any attempt makes the
  source `Unavailable` at once.
- **Matching is unchanged:** a hit counts when `artist_matches(full credit, hit
  artist)`, which already accepts any name contained in the credit
  (`Pat Metheny` for `Anna Maria Jopek & Friends with Pat Metheny`).
- **iTunes:** `GET https://itunes.apple.com/search?term=<artist title>&entity=song&limit=5`.
  First result whose `artistName` matches and that has `artworkUrl100` →
  `Found::Url` with `/100x100bb.` replaced by `/1000x1000bb.` (Apple serves the
  requested size). HTTP 403 and 429 / 5xx → `Unavailable` (iTunes answers 403
  when rate limited); other 4xx → `NotFound`. No results → `NotFound`.
- The cache key stays `artist|title`, shared by all search sources.

## Testing

- `lead_artist`: each joiner; case-insensitive (` With `, ` FEAT. `); first
  joiner wins; no joiner; `artist_attempts` deduplicates.
- Deezer and MusicBrainz: first attempt empty, lead-artist attempt hits →
  found; the second request carries the lead artist; a credit without joiners
  makes one request.
- iTunes against a local server: match → 1000 px URL; wrong artist skipped; no
  results → `NotFound`; 403 → `Unavailable`; 404 → `NotFound`; lead-artist retry.
- Default chain order includes `itunes`.
- Live test (`#[ignore]`): adds a title found only via the lead artist
  (`Anna Maria Jopek & Friends with Pat Metheny - Przypływ, odpływ`, any source).

## Delivery

Version **0.12.0**. README and `wiki/Architecture/Cover-Sources.md`: chain
order, iTunes, lead-artist retry, privacy sentence names Apple too.
`wiki/Domain/Formats-and-Metadata.md` and `wiki/Architecture/TUI.md`: chain order.
