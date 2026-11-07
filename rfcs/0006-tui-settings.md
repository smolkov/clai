# TUI settings

- RFC: 0006
- Status: draft
- Milestone: the `[tui]` settings of "TUI sessions and settings" (see
  [`implementation.md`](../implementation.md))
- Spec: [`spec/tui.md`](../spec/tui.md), "Input editor", "Appearance";
  [`spec/config.md`](../spec/config.md), "TUI", "Themes", "Reloading"

## Summary

`/settings` opens a box with the `[tui]` settings of the user file, one row per
key with its value and what it does. The user changes a value in place, sees it
at once in the running TUI, and saves it with `s` into the user file, keeping
the file's comments and order. The TUI also starts to apply `[tui]` changes
while it runs, from the box and from an edited file, and draws the themes of
`spec/config.md` (`terminal`, `dark`, `light`, `[themes.NAME]`), which it
validates today but does not use. Themes can also be files in a `themes/`
directory next to the user file, one per theme (`themes/nord.toml`), and
`clai` brings four of them: `atom-one-dark`, `nord`, `gruvbox-dark` and
`solarized-light`. The theme also has a
command of its own: `/theme` opens a list of all themes, previews the selected
one, and `Enter` takes it; `/theme NAME` switches at once.

## Motivation

- Changing how the TUI looks or behaves means leaving it, finding the user
  file, knowing the key names and their ranges, and starting the TUI again:
  `[tui]` is read only at start today, although `spec/config.md` says "a change
  applies at once to the running TUI".
- Several settings are only useful when tried: `tool_output_lines`,
  `expand_tool_calls`, `timestamps`, `mouse` (which decides whether the
  terminal's own text selection works).
- Every box has a command (`spec/tui.md`, "Command list"); a settings box fits
  that rule and the command list makes it easy to find.

## Scope

In:

- `/settings` and the settings box: browse, change, preview, save, discard.
- Saving to the user file without losing its comments, with the same
  validation as loading.
- Applying `[tui]` live in the TUI: from the box, and when the user file
  changes (`config_reloaded`).
- Marks for values that something else overrides: `CLAI_THEME`, `NO_COLOR`,
  a locale that is not UTF-8.
- Themes: `terminal`, `dark`, `light` and custom `[themes.NAME]` drawn, with
  `CLAI_THEME`, `NO_COLOR` and the 256-colour fallback for `#rrggbb`.
- `/theme [NAME]`: the theme list, and switching by name, with argument
  completion of theme names.
- Theme files: a `themes/` directory next to the user file, one `NAME.toml`
  per theme; the bundled themes `atom-one-dark`, `nord`, `gruvbox-dark` and
  `solarized-light`; an optional
  `background` colour key.

Out:

- The `code` key of a theme (the syntax highlighting theme): checked as today,
  used once syntax highlighting exists.
- Syntax highlighting (`syntax_highlight`): not built; its row is dim and says
  `later version`.
- Editing `[themes.NAME]` tables, providers, models, permissions and limits.
  `/permissions` stays its own command.
- Configurable keys (`spec/tui.md`, open question 3).

## Design

### The box

```
┌ Settings ─ ~/.config/clai/config.toml ────────────────────────────────────┐
│ ▶ theme              terminal   Colour theme                              │
│   symbols            auto       Unicode or ASCII symbols                  │
│   mouse            * off        Capture the mouse for scrolling           │
│   tool_output_lines  20         Lines of tool output before "… N more"    │
│   expand_tool_calls  off        Keep finished tool calls expanded         │
│   syntax_highlight   on         later version                             │
│   timestamps         off        Time of each entry                        │
│   scroll_lines       3          Lines per mouse-wheel step                │
│   max_fps            30         Redraw limit                              │
│                                                                           │
│ mouse: off keeps the terminal's text selection; on scrolls with the wheel │
│ Enter change ─ ←/→ value ─ d default ─ s save ─ Esc close                 │
└───────────────────────────────────────────────────────────────────────────┘
```

- Rows in the order of `spec/config.md`, "TUI". Each shows the key, the value in
  effect and a one-line description. The line under the list explains the
  selected key, with its values or range.
- `*` marks a value changed and not saved yet. A value not set in the file shows
  the default, dim.
- A value something else decides is marked and its row explains it:
  `theme` with `CLAI_THEME` set (`⚠ CLAI_THEME=dark`), any colour with `NO_COLOR`,
  `symbols = unicode` in a locale that is not UTF-8 (still applied, as the
  setting asks). **(proposal)**
- The title names the file that `s` writes: the TUI's own user file (`--config`
  or the default path).

### Keys

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select a row. |
| `Left` / `Right` | Previous / next value of an on/off or list setting; one step for a number. |
| `Enter`, `Space` | On/off: toggle. List: next value. Number: edit it in the row; `Enter` takes it, `Esc` goes back, a value out of range says so and stays. |
| `d` | Back to the default; the key is then removed from the file on save. |
| `s` | Save all changes, see below. |
| `Esc` | Close. With unsaved changes it asks `save changes? [s] save  [n] discard  [Esc] back`. |

`theme` cycles through `terminal`, `dark`, `light` and the `[themes.NAME]` of the
file, in file order. `symbols` cycles `auto`, `unicode`, `ascii`.

### Preview

A change applies to the running TUI at once, before it is saved, so its effect
can be seen behind the box. Discarding puts the values of the file back. Other
TUIs see nothing until the file is saved.

What applying means per key:

| Key | Applied by |
|-----|------------|
| `theme`, `symbols` | A new `Look`; every line cache of the conversation is dropped and laid out again. |
| `mouse` | A new effect `Effect::Mouse(bool)`: the shell turns mouse capture on or off. |
| `tool_output_lines`, `expand_tool_calls` | `Conversation` settings; the line caches are dropped. Calls the user expanded or collapsed keep that. |
| `timestamps` | The offset of local time is read once; entries keep the stamp they got, new ones follow the setting. **(proposal)** |
| `scroll_lines` | `Settings`. |
| `max_fps` | The shell reads the frame time from `App` before each frame. |
| `syntax_highlight` | Nothing yet. |

### Saving

- `s` edits the user file with `toml_edit`: it sets or removes the changed keys
  of `[tui]`, creates the table if it is missing, and leaves everything else as
  it is, comments and order included. **(proposal)**
- Before writing, the new text is loaded like at start (`config::load` on the
  text). If that fails, nothing is written and the box shows the first error,
  e.g. a `theme` that names a theme that is gone.
- The file is written atomically: a temporary file in the same directory with
  the old file's mode, then a rename. A file that is a link is written through
  the link, to its target. **(proposal)**
- If the file changed on disk since the box opened (time and size, as the
  server polls it), `s` says `the file changed; Esc and open /settings again`
  and writes nothing. Changes are never merged silently.
- After a save the server notices the file within 2 s and sends
  `config_reloaded` to every client; other TUIs then apply it as below.
- `CLAI_THEME` is not touched by saving: it still wins for this run, and the
  row says so.

### Live reload of `[tui]`

On `config_reloaded`, the TUI loads its own user file again (the same
`LoadOptions` as at start) and applies `[tui]` as for a preview. A file that
does not load keeps the previous settings and shows the error in the status
line, as `spec/config.md` says. With the settings box open, values the user has
not changed follow the file, and changed ones stay, marked `*`.

`[tui]` is read by the TUI, not by the server, because `--config` may differ
between them and `[tui]` belongs to the client. The server's notice is only the
trigger. **(proposal)**

### Themes

The TUI has one fixed set of styles on the 16 colours today
(`src/tui/style.rs`); it becomes the `terminal` theme, unchanged, so the frames
of all tests stay as they are.

- `Styles::new(theme: &Palette, colour: Colouring)` builds the styles from a
  palette of the twelve colour keys of `spec/config.md`, "Themes". `heading` is
  `accent` in bold, as today. `selection` is a background colour; in `terminal`
  it stays reversed video.
- A **palette** is one per built-in theme. A custom theme is its `base` palette
  with its own keys over it, from `config::Theme`, which loading already builds.
- `dark` and `light` use 256-colour indexes **(proposal)**:

  | Key | `dark` | `light` |
  |-----|--------|---------|
  | `text` | 252 | 235 |
  | `dim` | 244 | 245 |
  | `user` | 117 bold | 25 bold |
  | `accent` | 75 | 26 |
  | `border` | 240 | 250 |
  | `selection` | bg 238 | bg 254 |
  | `success` | 114 | 28 |
  | `error` | 203 bold | 160 bold |
  | `warning` | 221 | 136 |
  | `diff_add` | 114 | 28 |
  | `diff_remove` | 203 | 160 |

  The background stays the terminal's: a theme does not paint it.
- `Colouring` is `None` (`NO_COLOR`), `Ansi256` or `TrueColour`, from `NO_COLOR`
  and `COLORTERM` (`truecolor` or `24bit`), detected once at start. Without true
  colour, `#rrggbb` becomes the nearest of the 6×6×6 cube and the grey ramp of
  the 256 colours.
- `CLAI_THEME` names the theme for this run; an unknown name is a usage error at
  start, as for `tui.theme`. In the settings box the `theme` row then says
  `⚠ CLAI_THEME=NAME`, and changing it there previews, but does not outlast
  the run.
- Changing the theme lays out the conversation again, since cached lines carry
  their styles.

### Theme files

Besides `[themes.NAME]` tables, a theme can be a file of its own, so themes can
be shared and copied without editing the configuration.

- **Directory:** `themes/` next to the user file, so
  `$XDG_CONFIG_HOME/clai/themes/` by default, or next to the `--config` file.
  Each `NAME.toml` there is the theme `NAME`, e.g. `themes/nord.toml` is
  `nord`. Other files are ignored; so is a missing directory.
- **Content:** the keys of a `[themes.NAME]` table, at the top level of the file:

  ```toml
  # ~/.config/clai/themes/nord.toml
  base        = "dark"
  background  = "#2e3440"
  text        = "#d8dee9"
  dim         = "#616e88"
  user        = "#88c0d0"
  accent      = "#81a1c1"
  border      = "#4c566a"
  selection   = "#434c5e"
  success     = "#a3be8c"
  error       = "#bf616a"
  warning     = "#ebcb8b"
  diff_add    = "#a3be8c"
  diff_remove = "#bf616a"
  ```

- **Bundled themes:** `clai` brings `atom-one-dark`, `nord`, `gruvbox-dark` and
  `solarized-light`, built into the binary from `themes/*.toml` in the
  repository, with the colours of the published palettes (Atom One Dark, Nord,
  Gruvbox and Solarized, all MIT). They are listed like any other theme and are
  written in the format above, so a user can copy one into the directory and
  change it. More can follow as files only, without code.

  | Key | `atom-one-dark` | `nord` | `gruvbox-dark` | `solarized-light` |
  |-----|-----------------|--------|----------------|-------------------|
  | `base` | `dark` | `dark` | `dark` | `light` |
  | `background` | `#282c34` | `#2e3440` | `#282828` | `#fdf6e3` |
  | `text` | `#abb2bf` | `#d8dee9` | `#ebdbb2` | `#657b83` |
  | `dim` | `#5c6370` | `#616e88` | `#928374` | `#93a1a1` |
  | `user` | `#56b6c2` | `#88c0d0` | `#8ec07c` | `#2aa198` |
  | `accent` | `#61afef` | `#81a1c1` | `#83a598` | `#268bd2` |
  | `border` | `#4b5263` | `#4c566a` | `#504945` | `#93a1a1` |
  | `selection` | `#3e4451` | `#434c5e` | `#3c3836` | `#eee8d5` |
  | `success`, `diff_add` | `#98c379` | `#a3be8c` | `#b8bb26` | `#859900` |
  | `error`, `diff_remove` | `#e06c75` | `#bf616a` | `#fb4934` | `#dc322f` |
  | `warning` | `#e5c07b` | `#ebcb8b` | `#fabd2f` | `#b58900` |

- **`background`** (new colour key, for every theme): when set, the TUI paints
  the whole screen with it, so a theme made for a dark background also reads
  in a light terminal. Without it, as for the built-in three, the terminal's
  background stays.
- **Names and order of precedence:** a `[themes.NAME]` table, then a file in
  the directory, then a bundled theme. A file `nord.toml` thus replaces the
  bundled `nord`. A table and a file with the same name are an error, since
  one would hide the other without a sign. `terminal`, `dark` and `light`
  cannot be replaced, as for tables today. A name is the file name without
  `.toml` and follows the rules of theme names.
- **`base`** stays one of `terminal`, `dark`, `light`, as for tables: a theme
  file cannot build on another file.
- **Errors** name the theme file with its line, like errors of the user file.
  A broken theme file is a load error only if it is the theme in use
  (`tui.theme` or `CLAI_THEME`); otherwise it is left out of the list and the
  settings box shows `⚠ themes/x.toml: …` under the `theme` row. One bad file in a shared directory then does not stop `clai`.
- **Reloading:** the TUI reads the directory at start, on `config_reloaded`, and
  when the settings box opens. A theme file edited on its own is seen then; the
  server does not watch the directory. **(proposal)**
- In the settings box, `theme` steps through `terminal`, `dark`, `light`, the
  bundled themes, the files of the directory (by name), then the tables of the
  user file (in file order). A theme from a file shows its file under the list.
- `clai` never writes into the theme directory; saving only sets `tui.theme`.

### Theme list

The theme can be changed without the settings box (this answers open question 2
of `spec/tui.md`):

```
┌ Theme ────────────────────────────────────────────────────────────────────┐
│ Filter: _                                                                 │
│                                                                           │
│ built in                                                                  │
│   terminal                                     ✓ current                  │
│   dark                                                                    │
│   light                                                                   │
│ bundled                                                                   │
│ ▶ atom-one-dark                                                           │
│   nord                                         ~/.config/clai/themes/     │
│ user file                                                                 │
│   solar                                        base dark                  │
│                                                                           │
│ Enter take ─ Esc back to terminal                                         │
└───────────────────────────────────────────────────────────────────────────┘
```

- `/theme` opens it; it is a box with a command, as the rule of the command list
  asks. The settings box opens the same list with `Enter` on its `theme` row.
- Groups and order as the `theme` row steps through them: built in, bundled,
  theme files, tables of the user file. A file that replaces a bundled theme
  shows its directory; a broken file is listed dim with `⚠` and its error, and
  cannot be chosen. Typing filters by name, like the model picker.
- **Preview:** moving the selection applies that theme at once, behind the box,
  so the user sees it on the real conversation. `Esc` puts the theme back that
  was in use when the list opened.
- `Enter` takes the selected theme and saves it into the user file as
  `tui.theme`, the default theme: every TUI started from then on uses it, and
  open TUIs switch to it on `config_reloaded`. It is written as `s` writes in
  the settings box, with the same checks. Moving the selection only previews;
  nothing is saved before `Enter`, and `Esc` saves nothing. Opened from
  the settings box, `Enter` only sets the row, and the box's `s` saves it with
  the rest.
- `/theme NAME` switches to `NAME` and saves it the same way, without the list;
  an unknown name is a notice that names `/theme` for the list, and the input
  stays. The argument list of RFC 0005 offers the theme names after `/theme `.
- With `CLAI_THEME` set, the list says so in its title
  (`Theme ─ ⚠ CLAI_THEME=nord for this run`), and `Enter` saves the choice for
  later runs while this run keeps the chosen one too.
- With `NO_COLOR` the list still opens, but says that colours are off, since the
  preview would show nothing.

`src/tui/themes.rs` holds the list state, like `Picker`; `view/themes.rs` draws
it with the picker's box code.

### State

`src/tui/settings.rs` holds the box state, pure like the picker:

```rust
pub struct SettingsBox {
    /// The values of the file when the box opened or last reloaded.
    saved: config::Tui,
    /// The values shown and applied.
    current: config::Tui,
    /// Keys set to their default with `d`, removed on save.
    reset: BTreeSet<&'static str>,
    selected: usize,
    editing: Option<Editor>,
    confirm_close: bool,
    message: Option<String>,
    file: PathBuf,
    /// Time and size of the file when it was read.
    stamp: Option<(SystemTime, u64)>,
    themes: Vec<String>,
}
```

`App` asks it for the `config::Tui` to apply after each key and turns that into
`Settings`, `Look` and effects. Saving is an effect, `Effect::SaveSettings`, so
the file write stays in the shell and `App` stays free of I/O; its result comes
back as an input.

## Dependencies

- `toml_edit`: edits a TOML file and keeps comments, spacing and order. It comes
  from the same project as `toml`, which already uses it through `toml`'s
  parser, so it adds little to the build. **(proposal)**

## Spec changes

Written when the RFC is accepted.

- `spec/tui.md`: `/settings` and `/theme [NAME]` in the command table; a "Theme
  list" section; open question 2 answered; a "Settings" section with the
  box, keys, preview, save and close.
- `spec/config.md`, "Themes": theme files, their directory, precedence and
  errors; the four bundled themes; the `background` key; the
  palettes of `dark` and `light`; that a theme
  leaves the background to the terminal; the `(assumption)` on true colour
  confirmed.
- `spec/config.md`, "TUI": the TUI writes the user file only from `/settings`,
  and how (kept comments, atomic, refused when invalid or changed meanwhile);
  live reload of `[tui]` from the TUI's own file.
- `readme.md`: `/settings`; `/help` in the TUI.

## Testing

- `SettingsBox`: every key in each kind of row; ranges; `d`; close with and
  without changes; values following a reload while some are changed.
- Saving: a user file with comments, a missing `[tui]`, keys removed with `d`,
  keys of other tables untouched (snapshot of the written text); an invalid
  result writes nothing; a changed file is refused; the file mode is kept.
- `App`: a preview changes the `Look`, `tool_output_lines` and `expand_tool_calls`
  of the conversation; `Effect::Mouse` on a change of `mouse`; discard restores;
  `config_reloaded` applies a new `[tui]`; an invalid file keeps the old one.
- Theme files: as in the plan step above; a frame of each bundled theme.
- Themes: the styles of each palette; a custom theme keeps its base for keys it
  does not set; `#rrggbb` to the nearest 256 colour; `NO_COLOR` leaves only
  attributes; snapshots of one frame per built-in theme, with colours.
- `insta` snapshots: the box in Unicode and ASCII, with a changed value, a
  number being edited, the close question, an override mark, and a narrow
  screen.

## Implementation plan

1. **Themes**
   - Status: todo
   - Files: `src/tui/style.rs`, `src/tui/mod.rs`, `src/config/tui.rs`
   - Result: `dark`, `light` and custom themes drawn; `CLAI_THEME`; the
     256-colour fallback; the `background` key. `terminal` looks as before.
   - Tests: palettes per theme, a custom theme over its base, `NO_COLOR`, nearest
     colour, a painted background; frames of each built-in theme; existing
     snapshots unchanged.
2. **Theme files**
   - Status: todo
   - Files: `themes/atom-one-dark.toml`, `themes/nord.toml`,
     `themes/gruvbox-dark.toml`, `themes/solarized-light.toml`,
     `src/config/themes.rs`, `src/config/mod.rs`
   - Result: the theme directory, the bundled themes, precedence, errors with
     the file and line; a broken unused file left out.
   - Tests: a directory with two files; a file over a bundled theme; a table and
     a file with one name; a missing directory; a broken file in use and not in
     use; all bundled files load without errors.
3. **Settings applied live**
   - Status: todo
   - Files: `src/tui/app.rs`, `src/tui/mod.rs`, `src/tui/conversation.rs`
   - Result: `App::apply(config::Tui)`, `Effect::Mouse`, the frame time from
     `App`; `config_reloaded` reloads `[tui]`.
   - Tests: preview and reload tests of `App`.
4. **Writing the user file**
   - Status: todo
   - Files: `Cargo.toml`, `src/config/edit.rs`
   - Result: set and remove `[tui]` keys, validate, write atomically, refuse a
     changed file.
   - Tests: saving tests above.
5. **The theme list**
   - Status: todo
   - Files: `src/tui/themes.rs`, `src/tui/view/themes.rs`, `src/tui/app.rs`,
     `src/tui/commands.rs`
   - Result: `/theme` opens the list with preview, `Enter` takes and saves,
     `Esc` restores; `/theme NAME`; theme names in the argument list.
   - Tests: preview while moving, `Esc` restores, `Enter` saves, a broken file
     cannot be chosen, filter; `/theme NAME` and an unknown name; snapshots of
     the list in Unicode and ASCII.
6. **The settings box**
   - Status: todo
   - Files: `src/tui/settings.rs`, `src/tui/view/settings.rs`,
     `src/tui/themes.rs`,
     `src/tui/view/popup.rs`, `src/tui/app.rs`, `src/tui/commands.rs`
   - Result: `/settings` opens it; change, preview, save, discard.
   - Tests: box and `App` tests; snapshots.
7. **Docs**
   - Status: todo
   - Files: `spec/`, `readme.md`, `implementation.md`, `todo.md`,
     `rfcs/readme.md`
   - Result: spec changes above; status `done`.
   - Tests: `cargo fmt`, `clippy`, `test`.

## Alternatives

- **Settings for this run only, never saved.** No file writes and no new
  crate, but every change is lost on the next start, and the user still has to
  edit the file by hand. Could be the first step if saving is not wanted now.
- **The server writes the file** (a `save_settings` request). One writer and no
  race with the server's own reload, but `[tui]` is the client's, the TUI may
  use another `--config`, and the protocol would need version 4 for it.
- **Open `$EDITOR` on the user file.** Little code, but no preview, no
  explanation per key, and the TUI has to give up the terminal (`spec/tui.md`,
  open question 1).
- **Rewrite the file with `toml` serialization.** No new crate, but it drops
  comments and reorders keys, which a hand-written configuration file must not
  suffer.

## Open questions

Answered by the user:

1. **Save to the file?** Yes, with `s`, into the user file.
2. **Themes in this RFC?** Yes: `dark`, `light` and custom themes are built
   here (see "Themes").
3. **More than `[tui]`?** No, only the `[tui]` keys.
4. **A key besides `/settings`?** No; the command list finds `/settings`.

Answered later:

5. **Theme files.** A `themes/` directory next to the user file, `NAME.toml`
   per theme, and a `/theme` command with its own list.
6. **Which themes to bundle.** `atom-one-dark`, `nord`, `gruvbox-dark`,
   `solarized-light`.
7. **`background`.** Painted when a theme sets it.
8. **A broken theme file that is not in use.** Left out with a warning; an
   error only for the theme in use.
9. **`/theme` saves.** `Enter` in the theme list and `/theme NAME` save the
   theme into the user file as the default, `tui.theme`.

Still open:

10. **The palettes of `dark` and `light`** in "Themes" are a proposal; they can
    be changed after a look at them in a real terminal.
