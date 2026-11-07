# Keymap and discoverability

**Date:** 2026-11-07
**Status:** Approved
**Crate:** `znicz-tui`

## Context: the TUI reimagining

This is part 1 of a five-part rework of the player experience (variant "A then
B" from brainstorming on 2026-11-07). Each part gets its own spec, plan, and PR.

| # | Part | Scope |
| --- | --- | --- |
| 1 | **Keymap and discoverability** (this spec) | `h` / `l` movement, seek keys, context footer, context-first help with a filter |
| 2 | Space and layering | Modals stay inside the list region; toasts move into a transport slot with a message history; focus / accent styling; breadcrumb in the library title; playing-track marker in the library |
| 3 | Empty-library onboarding | Scan a folder from inside the TUI, with progress |
| 4 | Stage mode ([#46](https://github.com/eugene-chekan/znicz/issues/46)) | Full-screen now playing with a large cover, signal chain and up next; the inspector folds in; marquee for long titles ([#34](https://github.com/eugene-chekan/znicz/issues/34)) |
| 5 | List and volume polish | Reorder rows with Alt-↑ / Alt-↓ ([#36](https://github.com/eugene-chekan/znicz/issues/36)); finer volume ([#47](https://github.com/eugene-chekan/znicz/issues/47)) |

Decided for later parts: **Browse has no cover once Stage exists.** The library
gets the full height above a text transport; the cover lives in Stage only.
Part 2 therefore needs no elaborate cover-sizing rule, only "drop the cover when
space is short" until part 4 removes it from Browse.

## Problem

The keymap works but is hard to learn:

- `h` / `l` seek. Vim users expect them to move left and right, and in column
  view the only way between columns is `Tab`.
- The footer is one fixed string per pane. On narrow terminals it is cut off,
  and the library line shows `P · R` with no labels.
- Help is one two-column wall of about sixty bindings, closed by any key, with
  no way to find a binding by what it does.
- Footer clicks work by parsing hint text back into keys
  (`footer_hits::key_for_hint_segment`), which breaks whenever a label changes
  shape.

## Goals

1. `h` / `l` move horizontally in the library in every layout; arrows seek.
2. The footer shows a short, prioritised hint list for the current context,
   always ending in `? more`.
3. Help opens on the current context, scrolls, and filters by key or action.
4. Footer hints and help come from the same tables in `keys.rs`, and footer
   clicks carry real key events.

## Non-goals

- Command palette ([#5](https://github.com/eugene-chekan/znicz/issues/5)).
- User-configurable key bindings.
- Which-key popups.
- Removing the hints duplicated in modal bottom borders (part 2).
- Any change to `znicz-core`, `znicz-library`, or MCP.

## Key changes

### Seek

| Key | Before | After |
| --- | --- | --- |
| `→` / `←` | seek 5 s (also `l` / `h`) | seek 5 s |
| `Shift-→` / `Shift-←` | — | seek 30 s |
| `H` / `L` | seek 30 s | unbound |
| `Alt-→` / `Alt-←` | pan titles | unchanged |

Terminals that do not report `Shift-←` / `Shift-→` as distinct keys lose 30 s
seek; 5 s seek still works. Accepted.

### `h` / `l`

Only when the library has focus, browse mode is active, no modal is open, and no
prompt is typing:

| Layout | `h` | `l` |
| --- | --- | --- |
| Columns | Previous column; no-op on Artists | Next column; no-op on Tracks |
| Tree | Expanded artist or album: collapse it. Otherwise move to the parent row; no-op on a top-level collapsed artist | Collapsed artist or album: expand it. Otherwise no-op |
| Single-column paging | Back one level (same as `Esc` in the library) | Drill into the selected artist or album. On a track: no-op (does **not** play) |
| Search results | no-op | no-op |

In the queue, in modals, and in help, `h` / `l` do nothing. They never wrap.
`Tab` / `Shift-Tab` keep their current behaviour (wrapping column cycle, then
library ↔ queue).

### Unchanged

`Space` (pause; expand / collapse in the tree), `,` devices, `P` playlists,
`R` radio, `i` inspector, `]` queue, `j` / `k` / `g` / `G` / `Ctrl-d` /
`Ctrl-u`, and all queue, playlist, radio, and device keys.

## Help

A single-column popup, centred, at most 64 columns wide and the full list
height. It shrinks to fit small windows and scrolls. The two-column layout is
removed.

```
┌ Keys · Queue ───────────────────────────────────X┐
│ Queue                                            │
│  Enter            play the selected track        │
│  d / Del          remove from the queue          │
│  C                clear the queue                │
│  o                jump to the playing track      │
│ Playback                                         │
│  Space            play / pause                   │
│  → / ←            seek 5s                        │
│  Shift-→ / ←      seek 30s                       │
│  …                                               │
│ Navigation                                       │
│ Library                       (dimmed)           │
│ Playlists · Radio · Devices   (dimmed)           │
└ znicz 0.6.0 ──────────── / filter · Esc close ─┘
```

### Section order

1. The context help was opened from (Library, Queue, Devices, Inspector,
   Playlists, or Radio). The tree layout and search results open on Library.
   Help cannot open from `Search`, `Prompt`, or `StationPrompt`: there `?` is
   typed into the prompt.
2. Playback.
3. Navigation.
4. Every other context, styled dim.

The title is ` Keys · <context> `. Sections are listed in full; there is no
collapse or expand.

### Keys inside help

| Key | Action |
| --- | --- |
| `j` / `k`, `↓` / `↑`, `Ctrl-d` / `Ctrl-u`, `g` / `G` | Scroll |
| `/` | Start the filter prompt |
| typing (filter open) | Edit the filter (same `LineEdit` as library search) |
| `Enter` (filter open) | Keep the filter, return to scroll keys |
| `Esc` (filter open) | Clear the filter |
| `Esc` (filter closed) | Close help |
| `?` / `q` (filter closed) | Close help; `q` does **not** quit the TUI here |
| X button | Close help |
| any other key | Ignored |

Playback keys are inert while help is open, as today.

### Filter

Case-insensitive substring match against both `Binding::keys` and
`Binding::action`. A section with no matching binding is hidden. An empty filter
shows everything. No match at all shows a single dim line, `no keys match`.

### Footer of the popup

Bottom-left: ` znicz <version> ` (dropped when it does not fit, as today).
Bottom-right: ` / filter · Esc close `, replacing ` any key closes `.

## Footer

The hint line lists the current context's hints in priority order, drawing as
many as fit from the left. ` ? more ` is right-aligned and its width is reserved
first, so it is always visible (except in the Help context, which has no
`? more`). Separators stay ` · `.

| Context | Hints, in priority order |
| --- | --- |
| `Library` (columns, paging) | `Space pause` · `/ search` · `a add` · `] queue` · `P playlists` · `R radio` |
| `LibraryTree` | `/ search` · `a add` · `o expand` · `] queue` · `P playlists` · `R radio` |
| `Search` (prompt typing) | `Enter search` · `Esc cancel` |
| `Queue` | `Enter play` · `d remove` · `o jump` · `C clear` · `] close` |
| `Devices` | `Enter select` · `r rescan` · `Esc close` |
| `Inspector` | `Space pause` · `Esc close` |
| `Playlists`, `Radio` | `Enter play` · `a add` · `n new` · `e edit` · `d delete` · `Esc close` |
| `Prompt` (playlist name) | `Enter save` · `Esc cancel` |
| `StationPrompt` (station form) | `Enter save` · `Esc cancel` · `Tab next field` |
| `Help` | `/ filter` · `Esc close` |

Removed from the footer, kept in help: `Alt-← / Alt-→ pan`, `, devices`,
`i inspect`, `c copy`.

Toasts still never replace the hint line.

## Architecture

### `keys.rs`

The binding tables stay the single source. `Binding` gains an optional hint:

```rust
pub struct Binding {
    pub keys: &'static str,
    pub action: &'static str,
    pub hint: Option<Hint>,
}

pub struct Hint {
    pub label: &'static str,
    pub key: KeyEvent,
}
```

New items:

- `enum Context { Library, LibraryTree, Search, Queue, Devices, Inspector,
  Playlists, Radio, Prompt, StationPrompt, Help }`.
- `fn context_hints(context: Context) -> Vec<Hint>` returns the footer hints in
  the order of the table above. Hints for contexts whose tables do not hold the
  binding (for example `Esc cancel` in `Search`) are declared next to that
  function as consts.
- `fn help_sections(context: Context) -> Vec<Section>` where
  `Section { title, bindings, dim }`, in display order.
- `fn filter_sections(sections, query) -> Vec<Section>` implements the filter as
  a pure function.

`hints(pane: &str)` is removed.

The `GLOBAL` table is updated for the new seek keys; `NAVIGATION` gains
`h / l` — "left / right: column, tree level, back / into".

### `app.rs`

- `App::context() -> keys::Context` works out the context once per frame from
  the modal, focus, prompt state, and effective library layout. The footer and
  help both read it.
- `Modal::Help` becomes `Modal::Help(HelpState)`, with
  `HelpState { context, scroll, filter: Option<LineEdit> }`. The context is
  captured when help opens.
- A help key handler runs before the global keys whenever help is open.
- Seek moves to `KeyCode::Right` / `Left` (`SHIFT` → `SEEK_LARGE`). The `H` and
  `L` arms are deleted.
- `h` / `l` call a new `library_horizontal(forward: bool)`, which dispatches on
  the effective layout to new `LibraryPane` methods: `column_left`,
  `column_right` (non-wrapping), `tree_collapse_or_parent`, `tree_expand`, and
  a non-playing drill-in for paging (`enter` must not play a track from `l`).
  Paging back reuses `back`.

### `views/status.rs` and `footer_hits.rs`

The footer renders from `context_hints(app.context())`. Each drawn segment
registers a `FooterHit` with the hint's own `KeyEvent`.
`key_for_hint_segment` and the text-splitting in `layout_footer_hits` are
deleted; `layout_footer_hits` takes `&[Hint]` and the reserved `? more` hint.

### `views/help.rs`

Renders `filter_sections(help_sections(state.context), filter)` into one
scrolled `Paragraph`. Scroll is clamped to the content height. The filter
prompt, when open, is the last inner row: `/ <text>` with the caret.

## Testing

Unit (`keys.rs`, `help.rs`):

- Every hint's key matches a documented binding, or one of the declared
  context-only hint consts.
- Every context's hints plus ` ? more ` fit at 80 columns.
- Filter: matches on key text, on action text, case-insensitive; empty sections
  hidden; no match yields an empty list.
- `help_sections(Queue)` starts with Queue, then Playback, then Navigation; other
  sections are `dim`.

Keys (`tests/keys.rs`):

- `h` / `l` in columns, including no-op at Artists and Tracks.
- `h` / `l` in the tree: collapse, move to parent, expand, no-op on tracks.
- `h` / `l` in paging: back, drill in, no-op (no play) on a track.
- `h` / `l` do nothing with queue focus and in search results.
- `→` seeks 5 s, `Shift-→` seeks 30 s, `H` / `L` do nothing.
- Help: `/` opens the filter, typing edits it, `Esc` clears then closes, `q`
  closes without quitting, `Space` is ignored.

Render (`tests/render.rs`):

- Footer at 40 and 80 columns keeps ` ? more ` and drops low-priority hints.
- The tree context shows `o expand`, not `Space pause`.
- Help opened from the queue is titled ` Keys · Queue ` with Queue first.
- A filtered help view shows only matching rows.

Mouse (`tests/mouse.rs`):

- Clicking `a add` sends `a`; clicking ` ? more ` opens help.

## Delivery

- Bump `[workspace.package] version` to **0.6.0** (key bindings change).
- Update `wiki/Architecture/TUI.md` (keys, footer, help) and
  `wiki/Plans/Roadmap.md` (the five-part rework).
- Regenerate frames with `cargo run -p znicz-tui --example preview` and check
  them.
