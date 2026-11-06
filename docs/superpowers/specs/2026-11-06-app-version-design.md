# App Version Visibility Design

## Problem

Nothing in the running app says which build it is. Support, bug reports, and
"which build is this?" all need the `[workspace.package] version` from the root
`Cargo.toml`. Today:

- `znicz --version` / `-V` is not wired (the Clap `Cli` command has no
  `version`).
- The `?` help overlay is titled **Keys** and lists bindings only.
- MCP state has no version, but that is out of scope for this change.

Issue: [#22](https://github.com/eugene-chekan/znicz/issues/22).

## Scope

Two surfaces, chosen in brainstorming:

1. **CLI:** `znicz --version` and `znicz -V`.
2. **TUI help footer:** `?` shows `znicz <version>` in the bottom-left corner
   of the overlay, next to the existing `any key closes` hint.

## Non-goals

- MCP `get_player_state` or server metadata.
- Always-visible transport or status chrome.
- The signal inspector.
- Git hashes, build dates, or a second version file.
- Changing how versions are bumped.

## Design

### CLI

In `znicz/src/main.rs`, add `version` to the existing command attribute:

```rust
#[derive(Debug, Parser)]
#[command(name = "znicz", version, about = "Audiophile TUI music player")]
struct Cli {
```

Clap derives the number from `CARGO_PKG_VERSION` of the `znicz` crate, which
inherits `[workspace.package] version`. Output is the Clap default:

```text
znicz 0.5.2
```

No new constant or file.

### TUI help overlay

`znicz-tui/src/views/help.rs` keeps the top title `Keys` and gains a second
bottom title. The right one stays:

```rust
.title_bottom(Line::from(Span::styled(CLOSE_HINT, theme::dim())).right_aligned());
```

The left one is added only when it fits:

```rust
const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PREFIX: &str = " znicz ";
const CLOSE_HINT: &str = " any key closes ";

/// The bottom-left version title, or `None` when both titles cannot fit.
///
/// The close hint is functional, so it keeps the row when a tiny popup has no
/// room for the version.
fn version_title(popup_width: u16) -> Option<Line<'static>> {
    let label = format!("{VERSION_PREFIX}{VERSION} ");
    let budget = popup_width.saturating_sub(2) as usize;
    if label.chars().count() + CLOSE_HINT.chars().count() > budget {
        return None;
    }
    Some(Line::from(Span::styled(label, theme::dim())).left_aligned())
}
```

`env!("CARGO_PKG_VERSION")` expands to the `znicz-tui` package version. Every
workspace crate inherits the same root version, so this matches the CLI and the
running binary. A test pins that the string contains the package version.

The overlay is 86% of the screen width. At normal sizes both titles fit
(` znicz 0.5.2 ` is 13 columns, ` any key closes ` is 16). Below 31 columns
the version is omitted rather than allowed to collide with the close hint.

## Error handling

None of this can fail at runtime: the version is a compile-time string and the
width check is saturating arithmetic. A tiny terminal must keep drawing the
overlay without panicking; the existing small-size render coverage still runs.

## Testing

- `znicz/src/main.rs` unit test: `Cli::command().render_version()` contains
  `CARGO_PKG_VERSION`. It fails before `version` is added, because Clap renders
  `znicz \n` with an empty version.
- `help.rs` unit tests: `version_title(80)` is `Some`, `version_title(30)` is
  `None`, and the rendered text is exactly ` znicz <version> `.
- `znicz-tui/tests/render.rs`: opening `?` at 120x40 shows both the version and
  `any key closes`; the existing `the_help_overlay_draws_at_every_size` still
  passes.

## Release and docs

- Bump `[workspace.package] version` from 0.5.1 to 0.5.2 (compatible addition).
- `wiki/Architecture/TUI.md`: mention the help-footer version.
- `wiki/Issues.md`: move #22 to Closed with a short write-up.
- `README.md`: add `znicz --version` to the usage block.
- `Cargo.lock` follows the workspace bump.

## Acceptance criteria

- `znicz --version` and `znicz -V` print `znicz 0.5.2` and exit 0.
- `?` shows `znicz 0.5.2` bottom-left and `any key closes` bottom-right at
  normal terminal sizes, with no overlap.
- On a popup too narrow for both, the version is omitted and the close hint
  remains.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, and `cargo test --workspace` pass.
- Issue #22 is closed by the PR, with `wiki/Issues.md` updated.
