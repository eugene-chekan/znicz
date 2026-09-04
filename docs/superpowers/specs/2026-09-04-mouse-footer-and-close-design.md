# Mouse: clickable footer and explicit close

**Date:** 2026-09-04
**Status:** Approved
**Depends on:** [2026-09-03-mouse-support-design.md](2026-09-03-mouse-support-design.md) (shipped as #8 / 0.4.4)
**Crates:** `znicz-tui` (hit map, footer draw, `on_mouse`)
**Version:** bump `z` (`0.4.4` → `0.4.5`)

## Problem

Playtesting the first mouse slice showed two mismatches with how the player should feel:

1. Outside-click dismiss for overlays is too easy to trigger by accident and fights select-oriented clicking.
2. The bottom key-hint line is the natural mouse affordance for actions, but clicks there do nothing.

## Goals

1. **No outside-click close** for help, inspector, devices, playlists, radio, or the queue drawer.
2. **Explicit close control:** a clickable **X** in the **top-right chrome** of each of those surfaces.
3. **Clickable footer:** each ` · `-separated hint is its own hit target and runs that action (via a synthesized key into `on_key`).
4. **Queue border column opens only.** Closing the drawer is X, `]`, or a footer `] close` hit — not a click on the library body or the toggle column while open.
5. **Typing prompts** (library search, playlist/station forms) still **cancel on outside click**, same as today.

## This slice does not include

- Double-click to activate
- Seek bar / cover / toast clicks
- Restructuring `keys.rs` into a full action enum (named footer actions)
- Third-party mouse widget crates (`ratatui-interact`, etc.)
- Documenting mouse in the `?` help overlay (keyboard tables stay as they are)

## Why not “Ratatui mouse widgets”?

Ratatui is immediate-mode: widgets paint into a `Rect` and do not keep click handlers. Crossterm supplies raw `Event::Mouse`; `ListState` only stores selection/offset for drawing. Hit-testing last-frame rects (our `HitMap`) is the supported pattern. This slice extends that map; it does not switch frameworks.

## Architecture

Extend the existing hit map and left-click priority. Footer hints synthesize one primary `KeyEvent` and call `App::on_key` so keyboard and mouse share one path.

### Hit map additions

| Region | What we store |
| --- | --- |
| `close` | `Option<Rect>` — the X cell(s) for the open modal, or for the queue drawer when `Modal::None` |
| `footer_hints` | `Vec<FooterHit>` — `{ rect, key: KeyEvent }` for segments that fully fit on the footer row |

Only one `close` rect per frame: a modal wins over the queue drawer.

### Draw path

- Each modal and the queue drawer paint `X` in the top-right border/title area and write `hits.close`.
- `status::render_footer` splits the hint string on ` · `, lays out spans left-to-right, and records a rect + primary key per mapped segment.
- Existing list / overlay body / toggle / search-prompt rects stay. Toggle behaviour changes only in `on_mouse` (open-only).

### Left-click priority

1. Typing prompt outside → cancel (unchanged).
2. `footer_hints` hit → `on_key(that key)` (works while overlays are up).
3. `close` hit → `Modal::None`, or close the queue when no modal.
4. Overlay list row → select only; other overlay chrome (except X) → ignore. **No** outside-close.
5. Else library / queue row select; queue toggle **opens** when closed. Library under an open drawer does **not** close it; toggle while open does **not** close it.

Wheel rules are unchanged.

## Footer token → key

Split on ` · `. Map the key side of each segment to one `KeyEvent`. Unmapped segments draw as text only (no hit).

| Hint fragment (examples) | Primary key |
| --- | --- |
| `Enter play` / `Enter select` | Enter |
| `Esc close` / `Esc cancel` | Esc |
| `i / Esc close` | Esc (close wins) |
| `Space pause` | Space |
| `a add`, `d remove`, `C clear`, `o jump`, `n new`, … | that char (shift as shown) |
| `/ search` | `/` |
| `] queue` / `] close` | `]` |
| `? help` | `?` |
| `, devices` | `,` |
| `P` / `R` | `P` / `R` |
| `Alt-← / Alt-→ pan` | Alt+Right (one pan step; keyboard still has both) |
| `r rescan` | `r` |
| Typing footer `type · ← → · Enter · Esc cancel` | Enter and Esc only; ignore non-action chrome |

If the footer is too narrow, only segments that fully fit get hit rects — same as visible text.

## Testing

Extend `znicz-tui/tests/mouse.rs` (TDD). Cover at least:

- Click outside help / devices / inspector → modal **stays**
- Click `hits.close` → modal closes; queue open + no modal + close → drawer closes
- Library click under open queue → drawer **stays**; toggle column while open → **stays**
- Footer `?` opens help; Devices `Esc close` closes; a library `a add` path goes through `on_key`
- Transport still no-op; overlay chrome except X still no-op
- Prompt outside-click cancel unchanged

Do not integration-test mouse capture against a real terminal.

## Docs

- [wiki/Architecture/TUI.md](../../../wiki/Architecture/TUI.md): replace outside-close language with X + clickable footer; toggle opens only.
- [wiki/Domain/TUI-Players.md](../../../wiki/Domain/TUI-Players.md): pointer bullet aligned.
- No new mouse lines in generated `?` help.

## Error handling

Missing or zero-sized hit map: mouse remains a no-op. Unmapped footer text: no hit. Out-of-range coordinates: no-op. None of this is a toast or player error.
