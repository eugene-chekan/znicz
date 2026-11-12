# Settings screen (#6)

**Date:** 2026-11-12
**Status:** Approved in chat (display + audio scope; `S`; `toml_edit`; folder
removal; cover size; per-row explanations; default config written on first
start with active values). Fades are #90.
**Crates:** `znicz-tui`, `znicz` (binary), `znicz-core` (library op),
`znicz-library` (folder removal)
**Version:** 0.16.0

## Opening and layout

- `S` opens **Settings** wherever `?` works (not while typing in a prompt or
  search). It is `Modal::Settings`, drawn in the list region like Help and
  Devices; transport and message row stay visible. Help lists `S  settings`;
  the Browse footer's `? more` stays as it is.
- One scrollable list of rows in groups. Group headings are not selectable;
  the cursor skips them. Each row is `label … value` (value right-aligned,
  dim hint after it where shown below).
- Bottom of the modal: an **explanation strip** for the highlighted row, two
  lines, dim, word-wrapped to the modal's inner width; when the modal's inner
  height is under 8 rows it is one line cut with `…`. A one-row rule separates
  it from the list.
- Bottom border: the config file path (`~` for the home dir), cut from the left
  with `…` if too long.
- Keys: `j`/`k`/`↓`/`↑`, `g`/`G`, `Ctrl-d`/`Ctrl-u`, mouse click and wheel move
  the cursor; **Enter** / **Space** toggle a switch or step a choice forward,
  wrapping from the last value to the first (or run an action row); **`h`/`l`** (`←`/`→`) step a choice back/forward, stopping at
  the ends (on a switch: `h` off, `l` on); **`d`** on a folder row asks to remove it; **Esc** closes.
  Footer hint while open: `Enter change · h/l choose · Esc close`.

```
  Display
    Show cover                 on
    Cover size                 75%
    Cover protocol             auto
    Cover search for streams   on
    Library layout             columns
    Scroll long titles         on
    Icons                      unicode
  Audio
    Default output device      System default   ↵ choose
    Bit-perfect                on   on restart
  Library
    ~/Music                                     d remove
    + Add a folder…                             ↵
    Rescan all folders                          ↵
  Updates
    Check for updates daily    on
 ────────────────────────────────────────────────
  How big the album art is on the Stage, as a share
  of the largest size that fits.
```

## Rows

| Row | Kind | Values | Config key | Applies | Explanation |
|---|---|---|---|---|---|
| Show cover | switch | on/off | `[tui] show_cover` | now | Show album art on the Stage. Off gives the details the whole width. |
| Cover size | choice | 100% · 75% · 50% · 25% | `[tui] cover_size` (int) | now | How big the album art is on the Stage, as a share of the largest size that fits. |
| Cover protocol | choice | auto · kitty · sixel · halfblocks · off | `[tui] cover_protocol` | now | How the art is drawn. Auto asks the terminal; halfblocks works everywhere but looks coarse; off shows the logo. |
| Cover search for streams | switch | on/off | `[covers] search` | now | Look up radio cover art by artist and title on Deezer and MusicBrainz. |
| Library layout | choice | columns · tree | `[tui] library_layout` | now | Columns shows artists, albums and tracks side by side (when wide enough); tree nests them in one list. |
| Scroll long titles | switch | on/off | `[tui] marquee` | now | Scroll now-playing titles that don't fit. Off cuts them with "…". |
| Icons | choice | unicode · emoji · nerd | `[tui] icons` | now | Symbols used for icons such as the volume marker. Nerd needs a Nerd Font set as the terminal font (README: Nerd Font). |
| Default output device | action | device name / System default | `[audio] device` | now | The output Znicz starts with. Choosing one also switches to it now. |
| Bit-perfect | switch | on/off | `[audio] bit_perfect` | on player restart (hint `on restart`) | Open the device at each file's own sample rate and channels, so samples reach it unchanged. |
| *folder* (one per folder) | folder | path | library database | now | A folder Znicz indexes. d removes it and its tracks from the library (files are not touched). |
| + Add a folder… | action | | library database | now | Index another folder. |
| Rescan all folders | action | | | now | Look for new, changed and deleted files in every folder. |
| Check for updates daily | switch | on/off | `[updates] check` | next start | Ask GitHub once a day whether a new Znicz is out. |

"now" means the TUI's `TuiConfig` (or app state) changes immediately and the
next frame shows it.

### Details

- **Cover size**: `TuiConfig.cover_size: CoverSize` (`P100`, `P75`, `P50`,
  `P25`; parsed from the integer, anything else → `P100`). `stage_layout` gets
  the percentage: the cover rect it computes today is scaled (width and height,
  rounded down, at least 4×2 cells). In the wide layout it is vertically
  centred in the original slot, and the details start right after the scaled
  cover (plus the gap) and take the rest of the width. In the stacked layout it
  is top-aligned in the original slot (no empty band above it) and centred
  horizontally; the details start right below it and take the rest of the
  height. A change clears `cover_draw_key` so the cover is redrawn.
- **Cover protocol**: the terminal can only be queried before the event loop
  starts (a query mid-run would race with key input). At start the TUI keeps
  the queried picker's protocol type when it ran a query (config `auto`,
  `kitty`, `sixel`). A change then calls `picker.set_protocol_type` (auto →
  the queried type; halfblocks/off → Halfblocks; kitty/sixel → that type) and
  clears `cover_draw_key`. When no query ran at start (config `halfblocks` or
  `off`), switching to auto/kitty/sixel is saved and the row shows `on
  restart`.
- **Library layout**: same effect as the existing layout preference (the frame
  already syncs `app.library` from `app.tui.library_layout`).
- **Default output device**: Enter opens the existing Devices modal in a
  "default device" mode (title `Default output device`; first row `System
  default`). Choosing a device writes `[audio] device = "<id>"` (or removes the
  key for System default) and sends `Command::SetDevice` as the normal Devices
  modal does. Esc returns to Settings.
- **Bit-perfect**: only written; the player reads it at start.
- **Folders**: from the library (`Library::folders()` via the TUI's read
  connection), refreshed when the modal opens and when a library job finishes.
  `+ Add a folder…` opens the existing folder prompt (`F`); when it closes,
  Settings is shown again. `Rescan all folders` sends `LibraryOp::RescanAll`
  (as `U`). `d` on a folder opens a confirm line in the explanation strip:
  `Remove ~/Music and its 2,437 tracks from the library? Enter yes · Esc no`.
  Enter sends the new `LibraryOp::RemoveFolder { path }`.
- **`LibraryOp::RemoveFolder { path }`** (znicz-core `library_host.rs`, handled
  by the player's library host like the other ops): `Library::remove_folder`
  deletes the folder row and every track whose path is under it (path prefix
  with a separator, so `/music` does not match `/music2`), in one transaction;
  returns the number of tracks removed. Success toast `Removed ~/Music (2,437
  tracks)`. Older players reject the unknown op; the TUI shows the error toast
  with the existing "restart the player" hint.

## Writing the config

- `znicz-tui/src/config_file.rs`: `ConfigFile { path: PathBuf }` with
  `set_bool(section, key, bool)`, `set_int(section, key, i64)`,
  `set_str(section, key, &str)`, `remove(section, key)`, each `-> Result<(),
  String>`. Each call reads the file (missing → empty document), edits with
  `toml_edit::DocumentMut` (creates the table if missing; keeps comments,
  order, and other keys), writes to `config.toml.tmp` in the same directory,
  then renames over the file. Creates the parent directory if needed.
- The binary passes the effective config path (the `--config` path, else the
  default `~/.config/znicz/config.toml` from `default_config_path()`) to the TUI
  (`App::with_config_file(ConfigFile)`); `None` when no path is known.
- A change: apply in memory, then write. On a write error: restore the old
  value, error toast `Could not save settings: <error>`. With no config file:
  values still change for this run, and the first change shows the toast
  `Settings apply to this run only (no config file path)`.
- Dependency: `toml_edit` (workspace), licence regenerated.

## Default config file

The dist installers cannot run a post-install step, so Znicz creates the file
on its **first start**:

- At every start (any subcommand), when `--config` is not given and the default
  path (`dirs::config_dir()/znicz/config.toml`: `~/.config/znicz/config.toml`,
  `%APPDATA%\znicz\config.toml`) does not exist, write the default config there
  (create the directory; temp file + rename). Log at `info`: `Created <path>
  with the default settings`. A write failure is logged at `warn` and Znicz runs
  on the built-in defaults. An existing file is never touched.
- The file holds **active values**: every setting with its default, plus a
  short comment per key, grouped by section like the README's config block.
  Keys with no fixed default stay commented with an example: `# device =
  "default"` (system default output) and `# path =
  "~/.local/share/znicz/library.db"` (user data directory). It starts with:
  `# Znicz settings. Edit here or in the TUI (S). Reference:
  https://github.com/eugene-chekan/znicz#configuration`.
- The text is one template, `znicz/src/default-config.toml`, loaded with
  `include_str!`. A test parses it and checks every effective value equals the
  built-in default (`tui_config(&parsed) == tui_config(&Config::default())`,
  audio volume and bit-perfect, idle seconds, cover search, update check,
  skills dirs), so the file and the code cannot drift.
- `znicz config init` writes the same file on demand; it refuses when the file
  exists unless `--force`, which first copies the old file to
  `config.toml.bak`. `znicz config path` prints the effective path.
- README's configuration section shows the template's content (or says the
  file is created on first start and lists it).

## Broken config files

- `load_config` today silently falls back to defaults on a parse error. It now
  keeps the error: on a parse error Znicz still runs on defaults, logs `warn`,
  and the TUI shows an error toast at start: `config.toml has an error (line
  12, column 5): <message>. Using the defaults.` (`toml` error span → line and
  column).
- Settings does not write into a file that does not parse: the change applies
  for this run and the toast says `Not saved: config.toml has an error (line
  12, column 5)`.

## Testing

- `ConfigFile`: keeps comments and unrelated keys; creates the file, the
  directory and missing tables; `remove` drops the key and leaves the table;
  replaces a value of a different type; the write goes through the temp file
  (no `.tmp` left behind).
- `CoverSize` parse; `stage_layout` with 50% in wide and stacked layouts (size,
  centring, details position).
- Settings modal (TUI tests with a temp config file): `S` opens it; the cursor
  skips headings; Enter on Show cover flips `app.tui.show_cover` and writes
  `show_cover = false`; `l`/`h` step Cover size and stop at the ends, Enter
  steps forward and wraps; the explanation strip shows the highlighted row's text; a
  failed write (read-only path) restores the value and toasts; no config path
  → run-only toast once.
- Default device: choosing a device in the default mode sends `SetDevice` and
  writes `[audio] device`; System default removes the key.
- `Library::remove_folder`: removes only tracks under the folder (not a
  sibling with the same prefix), forgets the folder, returns the count.
  `RemoveFolder` through the library host.
- Help lists `S`.
- Default config: first start writes the template when missing, leaves an
  existing file alone, skips with `--config`; the template parses to the
  defaults; `config init` refuses without `--force`, and `--force` keeps a
  `.bak`.
- A config with a syntax error gives defaults plus the error with line and
  column; Settings refuses to write into it.

## Docs

### Nerd Font guide

A README section **Nerd Font icons** (linked from the `icons` line in the
config block, the default config template's `icons` comment, and the TUI wiki
page's icon table):

- What it is: a font patched with extra icon glyphs; `icons = "nerd"` needs the
  terminal to use one, otherwise the marker shows as boxes or `?`.
- Pick one font (example: JetBrainsMono Nerd Font) from
  https://www.nerdfonts.com/font-downloads or the `ryanoasis/nerd-fonts`
  GitHub releases.
- Install, per platform:
  - Linux: unzip the release archive into `~/.local/share/fonts/`, then
    `fc-cache -f`; or the distro package where one exists (e.g. Arch
    `ttf-jetbrains-mono-nerd`).
  - macOS: `brew install --cask font-jetbrains-mono-nerd-font`.
  - Windows: unzip, select the `.ttf` files, right-click → Install (or
    `winget` / `scoop` package where available).
- Set it as the terminal's font: Windows Terminal (Settings → profile →
  Appearance → Font face), GNOME Terminal / Konsole (profile → font), Kitty
  (`font_family` in `kitty.conf`), WezTerm (`config.font`), Alacritty
  (`[font.normal] family`). Restart the terminal.
- Check: run `echo -e "\uf028"` (or open Settings and pick Icons → nerd);
  a speaker glyph means it works.
- Package names and commands are checked against the Nerd Fonts docs and the
  package managers when the section is written; names that cannot be verified
  are left out rather than guessed.

### Other docs

`wiki/Architecture/TUI.md` (Settings modal, keys), config reference in the wiki
and README (`[tui] cover_size`), `wiki/Architecture/Library.md` (remove folder),
CHANGELOG 0.16.0, version 0.16.0.
