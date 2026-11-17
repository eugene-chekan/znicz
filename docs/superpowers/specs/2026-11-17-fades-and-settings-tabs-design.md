# Fades and Settings tabs (#90, part 1)

**Version:** 0.18.0. Crossfade, the other half of #90, gets its own design
later.

## Goal

1. Play, resume, pause, stop, a manual track change and a device switch fade
   instead of cutting the sound. Mute's fade time becomes configurable.
2. The Settings modal gets tabs, one per group, with a new Playback tab for the
   fade times.

## Part 1: fades

### Today

- `AudioOutput` holds one `GainControl` (target gain, ramp time, generation).
  Each stream callback walks a `GainRamp` toward the target and lands on it
  exactly, so a gain of 1.0 leaves samples untouched.
- Mute ramps over a fixed `MUTE_FADE` (500 ms); every other volume change over
  `VOLUME_RAMP` (20 ms).
- Pause sets a flag; the callback outputs silence at once. Play opens a stream
  at full gain; stop closes it. All three cut.

### What fades

| Action | Behaviour |
|---|---|
| Play from stopped | The new stream starts silent and fades in over `play_ms`. |
| Resume | Unpause and fade in over `play_ms`. |
| Pause | Fade out over `pause_ms`, then pause. |
| Stop | Fade out over `stop_ms`, then stop. |
| Manual track change: Next, Previous, `Play`, `QueuePlayIndex` while playing | Fade out over `track_ms`, change track, fade in over `play_ms`. |
| Device switch while playing | Fade out over `track_ms`, reopen on the new device, fade in over `play_ms`. |
| Mute / unmute | Ramp over `mute_ms`. |
| Automatic track end, gapless join, seek, queue edits, volume change | No fade (volume keeps its 20 ms ramp). |

A time of 0 means no fade: the action runs at once, as today. Starting playback
while stopped or paused has nothing to fade out, so it only fades in.
Replacing the queue or removing the playing row while playing fades out over
`track_ms` first.

### Audio layer (`znicz-core/src/audio`)

- `AudioOutput` gets a second `GainControl`, `fade`, next to the volume gain.
  The callback keeps a second `GainRamp` for it and applies
  `volume_gain * fade_gain` per frame. Both ramps land exactly on their target,
  so at full volume and no fade the samples pass through unchanged.
- `GainControl` gets a `landed` generation (an `AtomicU64`): the callback
  stores the ramp's generation when it reaches its target. `GainControl::set`
  returns the generation it created, and `has_landed(generation)` tells the
  engine that fade has finished.
- `AudioOutput` methods: `fade_to(target, ramp) -> u64` (the generation),
  `fade_landed(generation) -> bool`, and `set_fade_now(gain)` (instant, used to
  make a new stream start silent).
- The paused flag still forces silence. A paused stream's fade ramp does not
  advance; that is fine, because the engine pauses only after the fade-out
  landed and resumes before fading in.
- `fill_f32`, `fill_i16` and `fill_u16` share their argument list; the two
  controls and ramps go into one `Gains` struct (volume and fade) so the
  `too_many_arguments` allowances do not grow.

### Engine (`player/engine.rs`, new `player/fade.rs`)

`player/fade.rs` holds the logic with no audio in it:

- `FadeTimes { play, pause, stop, track, mute: Duration }`, serde, with
  `Default` = 150, 150, 300, 150, 500 ms.
- `fade_out_time(command, &FadeTimes, &Playback) -> Option<Duration>`: how long
  to fade out before `command` runs, or `None` to run it at once. `Playback`
  says whether sound is playing (status Playing with a stream) and which row is
  playing. Only while sound is playing:
  - `Pause` → `pause`; `Stop` → `stop`;
  - `NextTrack`, `PreviousTrack`, `Play`, `QueuePlayIndex`, `ReplaceQueue`,
    `QueueRemove` of the playing row, `SetDevice` → `track`.
  A time of zero is `None`.
- `FadeOut { waiting: VecDeque<CommandEnvelope>, generation, deadline }`: the
  command that started the fade and every command that arrived since, in
  order, each with its ack.
- While a fade-out is pending:
  - `SetVolume`, `SetMuted` and `SetFades` run at once.
  - `Resume`, when the only waiting command is a `Pause`, cancels it: the fade
    ramps back up over `play` from wherever it is, and both acks are answered
    `Ok`.
  - Every other command waits behind the others. This keeps order: playing an
    album sends `ReplaceQueue` then `QueuePlayIndex`, and both must run, in
    that order, once the sound is down.
- `deadline` is the fade time plus 500 ms. If the callback has not reported
  the fade landed by then (a dead stream), the waiting commands run anyway.

The engine:

- keeps `fade_out: Option<FadeOut>` and checks it each loop pass. When
  `output.fade_landed(generation)` or the deadline has passed, it runs the
  waiting commands in order. The first runs without fading (the fade already
  happened). Later ones are judged afresh: after a `Next` the new track is
  playing, so a waiting `Stop` fades out again, and the commands after it wait
  for that fade. Each ack is answered with its command's result once it runs,
  so "after `send_blocking` returns `Ok`, the state reflects the command"
  stays true (the MCP server relies on it). A fade-out adds at most `track_ms`
  or `stop_ms` to the wait; `COMMAND_TIMEOUT` is 20 s;
- fades in when a track starts from a command (`Play`, `QueuePlayIndex`,
  `NextTrack`, `PreviousTrack`, `Resume`, a device switch, removing the playing
  row): `set_fade_now(0.0)` before the stream opens, then `fade_to(1.0, play)`.
  An automatic next track (`finish_track`), gapless chaining and the reopen
  before a pending-seek (`replay_current`) do not fade: the fade gain is set to
  1.0 at once;
- mute uses `fades.mute` instead of `MUTE_FADE`, which is removed.

`Command::SetFades(FadeTimes)` replaces the times at once. It is not deferred.
`AudioConfig` gets `fades: FadeTimes`.

Status during a fade-out: the state keeps saying Playing until the action runs,
then changes as before. The TUI therefore shows the change when the sound has
gone, which matches what is heard.

### Config (`znicz` binary)

```toml
[audio.fade]
# Fade times in milliseconds; 0 turns a fade off.
play_ms = 150    # play and resume (fade in)
pause_ms = 150
stop_ms = 300
track_ms = 150   # manual track change and device switch (fade out)
mute_ms = 500
```

- `AudioSection` gets an optional `fade` table; missing keys take the defaults.
- `default-config.toml` gains the block above.
- The player reads it at start. The TUI sends `SetFades` after a change in
  Settings, so the running player uses the new times at once. An older player
  answers the unknown command with the usual hint to run `znicz player stop`.

### Bit-perfect

Steady playback stays bit-perfect: both gains land exactly on 1.0. Only the
fade itself changes samples, for 150–300 ms by default. The Signal panel does
not show this; a label for that short a time would only flicker.

## Part 2: Settings tabs

### Tabs and rows

| Tab | Key | Rows |
|---|---|---|
| Display | `1` | Show cover, Cover size, Cover protocol, Cover search, Library layout, Marquee, Icons |
| Audio | `2` | Default output device, Bit-perfect |
| Playback | `3` | Fade in (play, resume), Fade on pause, Fade on stop, Fade on track change, Mute fade |
| Library | `4` | the folders, + Add a folder…, Rescan all folders |
| General | `5` | Update check |

Section headings (`Row::Heading`) go away: the tab names replace them.

### Layout

- The first line inside the border is the tab bar:
  ` Display │ Audio │ Playback │ Library │ General `. The active tab is
  highlighted; while the tab bar has focus the highlight uses the cursor
  style.
- A rule under the tab bar, then the active tab's rows, then the explanation
  strip as today. The config path stays in the border.
- The modal's height is the same on every tab: the tallest tab's rows (the
  Library tab grows with the folder count, capped by the screen as today).

### Keys

On open, focus is on the first row of the tab used last in this run (Display
the first time). Coming back from Devices or the folder prompt returns to the
row that opened them, on its tab.

| Where | Key | Action |
|---|---|---|
| Rows | `j` / `k`, `↓` / `↑`, `g` / `G`, `Ctrl-d` / `Ctrl-u` | Move between rows |
| Rows | `↑` / `k` on the first row | Focus the tab bar |
| Rows | `←` / `→`, `h` / `l`, Enter, Space | Change the value, as today |
| Rows | `d` on a folder, Enter on an action | As today |
| Tab bar | `←` / `→`, `h` / `l` | Previous / next tab; stop at the ends |
| Tab bar | `↓`, `j`, Enter | Focus the first row |
| Anywhere | `Tab` / `Shift-Tab` | Next / previous tab, wrapping; focus stays where it is (bar or first row) |
| Anywhere | `1`–`5` | Jump to that tab |
| Anywhere | `Esc` | Cancel a pending folder removal, else close |

Switching tabs puts the row cursor on the new tab's first row and cancels a
pending folder removal. Row keys (`d`, Enter on an action) do nothing on the
tab bar.

Footer hints follow focus: on the tab bar `←/→ tab · ↓ rows · Esc close`
(a new `Context::SettingsTabs`); in the rows today's
`Enter change · h/l choose · Esc close` plus `Tab next tab`. The Help table
for Settings lists `Tab / Shift-Tab`, `1–5` and `↑ on the first row`.

### Fade rows

- `Kind::Choice` on the ladder 0 (shown `off`), 50, 100, 150, 200, 300, 500,
  750, 1000, 1500, 2000 ms; `h` / `l` stop at the ends, Enter wraps, as other
  choices.
- A value from the file that is not on the ladder is shown as it is
  (`240 ms`); a step moves to the next ladder value in that direction.
- Each row has explanation text, e.g. "Fade out over this time before pausing.
  Off pauses at once."
- A change writes `[audio.fade] <key>` with `ConfigFile` (comments kept) and
  sends `SetFades` with all five times. The row needs no `on restart` hint.

### Code

- `config_file.rs`: a dotted section (`audio.fade`) is a nested table, so
  `set_int("audio.fade", "play_ms", 150)` writes `[audio.fade] play_ms = 150`.
- `settings.rs`: `Tab` enum (`label()`, `key()`, `all()`), `rows(tab, folders)`,
  the five fade rows, `fade_ladder` stepping, hint texts. `Row::Heading` and
  `Kind::Heading` are removed.
- `app/settings_ui.rs`: `SettingsFocus { tab, on_tab_bar, row }` replacing the
  plain row index; the key table above; `select_setting` finds the row's tab.
- `views/settings.rs`: the tab bar and fixed height.
- `App` gets the five fade values in `Values` / `SettingsValues`, filled from
  the config at start.

## Testing

- `gain.rs`: `set` returns a generation; the callback marks it landed when the
  ramp reaches its target; a later `set` is not reported as landed by an older
  ramp.
- `output.rs` (`fill_f32` without a device): volume × fade; full volume and
  full fade pass samples through unchanged; a fade-out ends at exactly 0.
- `fade.rs`: `fade_out_time` for every command, playing or not, and with a
  zero time; the waiting order; the Resume-cancels-Pause rule; the deadline.
- Engine (`spawn_player` without a device, as the existing tests do): the
  commands that need no stream (`SetFades`) are accepted.
- `tests/playback.rs` (hardware; skipped on CI): pause takes about `pause_ms`
  before the status says Paused; pause then an immediate resume keeps playing;
  `next` returns with the new track playing.
- `znicz` config: `[audio.fade]` parses, missing keys default, a bad value is a
  config error with line and column.
- Settings: tab keys (Tab, Shift-Tab, 1–5, `↑` to the bar, arrows on the bar),
  `←`/`→` still change values in the rows, row keys inert on the bar, returning
  from Devices lands on the Audio tab, fade ladder stepping and snapping, a
  change writes the config and sends `SetFades`; a render test for the tab bar
  and the fixed height.

## Docs and release

- Wiki: `Architecture/TUI.md` (Settings: tabs, keys, Playback rows),
  `Architecture/Audio-Threading.md` (two gains, landed marker),
  `Architecture/Core-Engine.md` (deferred actions), `Domain/Playback-Pipeline.md`
  (fades and bit-perfect).
- README config template: `[audio.fade]`.
- CHANGELOG and version 0.18.0; third-party licences refreshed.
- #90 stays open for crossfade, with a comment on what shipped.
