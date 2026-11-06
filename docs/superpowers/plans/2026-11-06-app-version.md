# App Version Visibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Make the running build visible through `znicz --version` and the TUI help footer.

**Architecture:** Clap derives the CLI version from `CARGO_PKG_VERSION`. The help overlay reads the same value at compile time and draws it as a second bottom title, dropping it when the popup is too narrow.

**Tech Stack:** Rust 2021, Clap 4, ratatui 0.30, cargo test.

---

### Task 1: Commit the plan

**Files:**
- Create: `docs/superpowers/plans/2026-11-06-app-version.md` (this file)

- [x] **Step 1: Commit the plan**

```bash
git add docs/superpowers/plans/2026-11-06-app-version.md
git commit -m "Add app version implementation plan"
```

---

### Task 2: Wire `znicz --version`

**Files:**
- Modify: `znicz/src/main.rs` (command attribute, tests module)
- Test: `znicz/src/main.rs`

- [x] **Step 1: Write the failing test**

At the end of `znicz/src/main.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn version_is_wired() {
        let rendered = Cli::command().render_version();
        assert!(
            rendered.contains(env!("CARGO_PKG_VERSION")),
            "expected the workspace version in {rendered:?}"
        );
    }
}
```

- [x] **Step 2: Run it to verify it fails**

Run:

```bash
cargo test -p znicz --bin znicz --offline version_is_wired
```

Expected: FAIL. Clap renders `"znicz \n"` because the command has no version.

- [x] **Step 3: Add the version attribute**

Replace:

```rust
#[command(name = "znicz", about = "Audiophile TUI music player")]
```

with:

```rust
#[command(name = "znicz", version, about = "Audiophile TUI music player")]
```

- [x] **Step 4: Run the test to verify it passes**

Run:

```bash
cargo test -p znicz --bin znicz --offline version_is_wired
```

Expected: `1 passed; 0 failed`.

- [x] **Step 5: Commit**

```bash
git add znicz/src/main.rs
git commit -m "Wire znicz --version"
```

---

### Task 3: Add the help footer version title helper

**Files:**
- Modify: `znicz-tui/src/views/help.rs` (constants, helper, tests)

- [x] **Step 1: Write the failing unit tests**

In `znicz-tui/src/views/help.rs`, inside `mod tests`, add:

```rust
    #[test]
    fn the_version_title_names_the_package_version() {
        let title = version_title(80).expect("a normal popup has room");
        let text: String = title
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert_eq!(text, format!(" znicz {} ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn a_narrow_popup_drops_the_version_and_keeps_the_close_hint() {
        assert!(version_title(30).is_none());
        assert!(version_title(80).is_some());
    }
```

- [x] **Step 2: Run them to verify they fail**

Run:

```bash
cargo test -p znicz-tui --lib --offline help::tests
```

Expected: compile error, `cannot find function version_title`.

- [x] **Step 3: Add the constants and helper**

After the `use` block in `znicz-tui/src/views/help.rs`, add:

```rust
const VERSION: &str = env!("CARGO_PKG_VERSION");
const VERSION_PREFIX: &str = " znicz ";
const CLOSE_HINT: &str = " any key closes ";
```

Add this function before `fn section`:

```rust
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

- [x] **Step 4: Run the tests to verify they pass**

Run:

```bash
cargo test -p znicz-tui --lib --offline help::tests
```

Expected: `2 passed; 0 failed`.

- [x] **Step 5: Commit**

```bash
git add znicz-tui/src/views/help.rs
git commit -m "Add help footer version title"
```

---

### Task 4: Draw the version in the help overlay

**Files:**
- Modify: `znicz-tui/src/views/help.rs` (`render`)
- Test: `znicz-tui/tests/render.rs`

- [x] **Step 1: Write the failing render test**

In `znicz-tui/tests/render.rs`, after `the_help_overlay_draws_at_every_size`, add:

```rust
#[test]
fn the_help_overlay_shows_the_app_version_and_close_hint() {
    let mut app = App::with_library(player(), None);
    app.modal = Modal::Help;
    let screen = draw(&mut app, 120, 40);
    assert!(
        screen.contains(&format!("znicz {}", env!("CARGO_PKG_VERSION"))),
        "version missing from help:\n{screen}"
    );
    assert!(screen.contains("any key closes"), "close hint missing");
}
```

- [x] **Step 2: Run it to verify it fails**

Run:

```bash
cargo test -p znicz-tui --test render --offline the_help_overlay_shows_the_app_version_and_close_hint
```

Expected: FAIL, `version missing from help`.

- [x] **Step 3: Add the second bottom title**

In `render`, replace:

```rust
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(" Keys ", theme::title()))
        .title(crate::views::close_title())
        .title_bottom(Line::from(Span::styled(" any key closes ", theme::dim())).right_aligned());
    let inner = block.inner(popup);
```

with:

```rust
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::border_active())
        .title(Span::styled(" Keys ", theme::title()))
        .title(crate::views::close_title())
        .title_bottom(Line::from(Span::styled(CLOSE_HINT, theme::dim())).right_aligned());
    if let Some(version) = version_title(popup.width) {
        block = block.title_bottom(version);
    }
    let inner = block.inner(popup);
```

- [x] **Step 4: Run the test to verify it passes**

Run:

```bash
cargo test -p znicz-tui --test render --offline the_help_overlay_shows_the_app_version_and_close_hint
```

Expected: `1 passed; 0 failed`.

- [x] **Step 5: Run the help and render suites**

Run:

```bash
cargo test -p znicz-tui --lib --offline
cargo test -p znicz-tui --test render --offline
```

Expected: all pass, including the tiny-size render coverage.

- [x] **Step 6: Commit**

```bash
git add znicz-tui/src/views/help.rs znicz-tui/tests/render.rs
git commit -m "Show the version in the help footer"
```

---

### Task 5: Verify the workspace

**Files:**
- Test: workspace and the real binary

- [x] **Step 1: Run formatting**

```bash
cargo fmt --all -- --check
```

Expected: exit 0, no output.

- [x] **Step 2: Run Clippy**

```bash
cargo clippy --workspace --all-targets --offline -- -D warnings
```

Expected: exit 0, no warnings.

- [x] **Step 3: Run the full test suite**

```bash
cargo test --workspace --offline
```

Expected: every suite reports `test result: ok` with 0 failed.

---

### Task 6: Bump the version and update the docs

**Files:**
- Modify: `Cargo.toml` (`[workspace.package] version`)
- Modify: `Cargo.lock` (regenerated by cargo)
- Modify: `README.md`
- Modify: `wiki/Architecture/TUI.md`
- Modify: `wiki/Issues.md`

- [x] **Step 1: Bump the workspace version**

In `Cargo.toml`, change:

```toml
version = "0.5.1"
```

to:

```toml
version = "0.5.2"
```

- [x] **Step 2: Refresh the lock file**

```bash
cargo check --workspace --offline
grep -A1 'name = "znicz' Cargo.lock
```

Expected: all five workspace crates show `version = "0.5.2"`.

- [x] **Step 3: Add the CLI usage line**

In `README.md`, after the `# Select device` block, add:

```bash
# Print the app version
znicz --version
```

- [x] **Step 4: Mention the help footer**

In `README.md`, replace:

```text
Press `?` in the player for the full keymap. The essentials:
```

with:

```text
Press `?` in the player for the full keymap; the overlay footer shows the
running version. The essentials:
```

- [x] **Step 5: Document the help footer in the wiki**

In `wiki/Architecture/TUI.md`, after:

```text
Repeat is `e`, shuffle is `z`. The full keymap lives in `znicz-tui/src/keys.rs`
and in the `?` overlay.
```

add:

```text
The `?` overlay also shows the build in its bottom-left corner
(` znicz 0.y.z `); `znicz --version` prints the same number.
```

- [x] **Step 6: Move issue #22 to Closed in the wiki**

In `wiki/Issues.md`, remove this line from **Open**:

```text
- [#22 Show the app version](https://github.com/eugene-chekan/znicz/issues/22) (place and shape still open)
```

Add this at the top of **Closed**, before `### [#7 Three-column ...]`:

```markdown
### [#22 Show the app version in the player](https://github.com/eugene-chekan/znicz/issues/22)

- **Fixed:** 2026-11-06
- **Component:** `znicz`, `znicz-tui`
- **Status:** **Fixed** in 0.5.2

`znicz --version` / `-V` prints the workspace version. The `?` help overlay
shows ` znicz 0.5.2 ` in its bottom-left corner, next to `any key closes`, and
omits it when the popup is too narrow. MCP state and always-visible chrome stay
out of scope.
```

- [x] **Step 7: Re-run formatting and the full suite**

```bash
cargo fmt --all -- --check
cargo test --workspace --offline
```

Expected: exit 0 and all tests pass.

- [x] **Step 8: Confirm the real CLI output**

```bash
cargo run -p znicz --offline -- --version
```

Expected:

```text
znicz 0.5.2
```

- [x] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock README.md wiki/Architecture/TUI.md wiki/Issues.md
git commit -m "Bump to 0.5.2 and document the version"
```

---

### Task 7: Push and open the pull request

**Files:** none.

- [x] **Step 1: Confirm the branch state**

```bash
git --no-optional-locks status --short
git --no-pager log --oneline main..HEAD
```

Expected: a clean tree and the planned commits on `feat/app-version`.

- [x] **Step 2: Push**

```bash
git push -u origin HEAD
```

- [x] **Step 3: Open the pull request**

Write the body to a file, then create the PR:

```bash
cat > /tmp/znicz-version-pr.md <<'PR'
## Summary
- Wire `znicz --version` / `-V` through Clap.
- Show ` znicz 0.5.2 ` in the `?` help footer, next to `any key closes`.
- Omit the version when the popup is too narrow; bump to 0.5.2 and update docs.

## Test Plan
- [x] `cargo test --workspace --offline`
- [x] `cargo clippy --workspace --all-targets --offline -- -D warnings`
- [x] `cargo fmt --all -- --check`
- [x] `cargo run -p znicz --offline -- --version`

Closes #22.
PR
gh pr create --title "Show the app version in the CLI and help overlay" --body-file /tmp/znicz-version-pr.md --base main
```

Expected: the PR URL is printed. Do not merge it.
