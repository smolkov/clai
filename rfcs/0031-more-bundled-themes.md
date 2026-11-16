# More bundled themes

- RFC: 0031
- Status: done
- Spec: [`spec/config.md`](../spec/config.md), "Themes", "Theme files";
  [`spec/tui.md`](../spec/tui.md), "Theme list", "Appearance"

## Summary

`codeagent` bundles ten more themes, each with its interface colours and
matching syntax colours: `emacs`, `flexoki-dark`, `flexoki-light`,
`material-oceanic`, `monokai`, `nightfox`, `one-light`, `solarized-dark`,
`textual-dark` and `tokyo-night`. `solarized-light` is already bundled; it
stays and gets its dark half. The `code` key gets the new syntax themes by
the same names, so a custom theme can use them too.

## Motivation

- Four bundled themes (`atom-one-dark`, `nord`, `gruvbox-dark`,
  `solarized-light`) cover few tastes. Monokai, Solarized, Tokyo Night,
  Nightfox, Material, One and Flexoki are palettes people already use in
  their editor and terminal, and want to see in the TUI too; Emacs users
  know its default colours by heart.
- Only one bundled theme is light. With `emacs`, `flexoki-light` and
  `one-light` there are four.
- Today a user who wants one of these writes a theme file by hand, and
  cannot get matching syntax colours at all: `code` only names the syntax
  themes `syntect` brings, and of these palettes only Solarized is among
  them.

## Scope

In:

- Ten theme files in `themes/`, bundled into the binary as today.
- Syntax palettes for the new themes that `syntect` does not bring.
- The new names in the `code` key, the theme list and the docs.

Out:

- More light or dark variants (Tokyo Night Day, Dayfox, Material Lighter,
  One Dark is already `atom-one-dark`): a later change if wanted.
- Reading `.tmTheme` or other theme formats from disk.
- Detecting the terminal's background to pick dark or light by itself.

## Design

### Names

Theme names use `-` like the bundled ones (`atom-one-dark`), so
`material_oceanic` and `one_light` become `material-oceanic` and
`one-light`. **(proposal)** `emacs` is the look of GNU Emacs without a
custom theme: its default light faces.

### Interface colours

Each theme is a file in `themes/`, in the format of the existing ones (see
`spec/config.md`, "Theme files"), with every colour key set, `background`
included. `success` is also `diff_add`, `error` also `diff_remove`.
**(proposal)**

| Theme | `base` | `background` | `text` | `dim` | `user` | `accent` | `border` | `selection` | `success` | `error` | `warning` |
|-------|--------|--------------|--------|-------|--------|----------|----------|-------------|-----------|---------|-----------|
| `emacs` | light | `#ffffff` | `#000000` | `#7f7f7f` | `#0000ff` | `#483d8b` | `#bfbfbf` | `#eedc82` | `#228b22` | `#ff0000` | `#cd6600` |
| `flexoki-dark` | dark | `#100f0f` | `#cecdc3` | `#878580` | `#3aa99f` | `#4385be` | `#403e3c` | `#282726` | `#879a39` | `#d14d41` | `#d0a215` |
| `flexoki-light` | light | `#fffcf0` | `#100f0f` | `#6f6e69` | `#24837b` | `#205ea6` | `#dad8ce` | `#e6e4d9` | `#66800b` | `#af3029` | `#ad8301` |
| `material-oceanic` | dark | `#263238` | `#b0bec5` | `#78909c` | `#89ddff` | `#82aaff` | `#37474f` | `#314549` | `#c3e88d` | `#f07178` | `#ffcb6b` |
| `monokai` | dark | `#272822` | `#f8f8f2` | `#90908a` | `#a6e22e` | `#66d9ef` | `#49483e` | `#3e3d32` | `#a6e22e` | `#f92672` | `#fd971f` |
| `nightfox` | dark | `#192330` | `#cdcecf` | `#738091` | `#63cdcf` | `#719cd6` | `#39506d` | `#2b3b51` | `#81b29a` | `#c94f6d` | `#dbc074` |
| `one-light` | light | `#fafafa` | `#383a42` | `#696c77` | `#0184bc` | `#4078f2` | `#d3d3d4` | `#e5e5e6` | `#50a14f` | `#e45649` | `#986801` |
| `solarized-dark` | dark | `#002b36` | `#839496` | `#657b83` | `#2aa198` | `#268bd2` | `#073642` | `#073642` | `#859900` | `#dc322f` | `#b58900` |
| `textual-dark` | dark | `#121212` | `#e0e0e0` | `#808080` | `#ffa62b` | `#0178d4` | `#2c2c2c` | `#1e1e1e` | `#4ebf71` | `#ba3c5b` | `#ffa62b` |
| `tokyo-night` | dark | `#1a1b26` | `#c0caf5` | `#737aa2` | `#7dcfff` | `#7aa2f7` | `#3b4261` | `#283457` | `#9ece6a` | `#f7768e` | `#e0af68` |

`dim` is the palette's comment grey where that is readable on the
background, else the next lighter grey of the same palette (`monokai`,
`material-oceanic`, `one-light`, `solarized-dark`, `tokyo-night`), so
notices and hints stay legible; see "Testing", contrast. **(proposal)**

### Syntax colours

`solarized-dark` takes `syntect`'s `Solarized (dark)`, as `solarized-light`
takes `Solarized (light)`. The other nine get a `Palette` of their own in
`OWN_PALETTES` (`src/tui/highlight.rs`), the ten roles the bundled themes
already use; the `code` key of each theme file names it. **(proposal)**

| Theme | `text` | `comment` | `keyword` | `string` | `number` | `function` | `kind` | `variable` | `operator` | `tag` |
|-------|--------|-----------|-----------|----------|----------|------------|--------|------------|------------|-------|
| `emacs` | `#000000` | `#b22222` | `#a020f0` | `#8b2252` | `#008b8b` | `#0000ff` | `#228b22` | `#a0522d` | `#000000` | `#0000ff` |
| `flexoki-dark` | `#cecdc3` | `#878580` | `#879a39` | `#3aa99f` | `#8b7ec8` | `#da702c` | `#d0a215` | `#4385be` | `#878580` | `#d14d41` |
| `flexoki-light` | `#100f0f` | `#6f6e69` | `#66800b` | `#24837b` | `#5e409d` | `#bc5215` | `#ad8301` | `#205ea6` | `#6f6e69` | `#af3029` |
| `material-oceanic` | `#b0bec5` | `#546e7a` | `#c792ea` | `#c3e88d` | `#f78c6c` | `#82aaff` | `#ffcb6b` | `#eeffff` | `#89ddff` | `#f07178` |
| `monokai` | `#f8f8f2` | `#75715e` | `#f92672` | `#e6db74` | `#ae81ff` | `#a6e22e` | `#66d9ef` | `#fd971f` | `#f92672` | `#f92672` |
| `nightfox` | `#cdcecf` | `#738091` | `#9d79d6` | `#81b29a` | `#f4a261` | `#86abdc` | `#dbc074` | `#cdcecf` | `#aeafb0` | `#c94f6d` |
| `one-light` | `#383a42` | `#a0a1a7` | `#a626a4` | `#50a14f` | `#986801` | `#4078f2` | `#c18401` | `#e45649` | `#0184bc` | `#e45649` |
| `textual-dark` | `#d4d4d4` | `#6a9955` | `#569cd6` | `#ce9178` | `#b5cea8` | `#dcdcaa` | `#4ec9b0` | `#9cdcfe` | `#d4d4d4` | `#569cd6` |
| `tokyo-night` | `#c0caf5` | `#565f89` | `#bb9af7` | `#9ece6a` | `#ff9e64` | `#7aa2f7` | `#2ac3de` | `#e0af68` | `#89ddff` | `#f7768e` |

Where they come from:

- **Emacs**: the default `font-lock` faces on a light background
  (`font-lock-comment-face` Firebrick, `-keyword-face` Purple,
  `-string-face` VioletRed4, `-function-name-face` Blue1, `-type-face`
  ForestGreen, `-variable-name-face` Sienna, `-constant-face` DarkCyan).
- **Flexoki**: the mapping its author publishes: keywords green, strings
  cyan, numbers purple, functions orange, constants yellow, variables blue.
  Comments use `tx-2`, not `tx-3`, so they stay readable.
- **Material Oceanic**: the Material Theme palette on its Oceanic
  background.
- **Monokai**: the classic Sublime Text palette; types `#66d9ef`, parameters
  `#fd971f`.
- **Nightfox**: the `nightfox` style of `nightfox.nvim`; functions in its
  bright blue.
- **One Light**: Atom's One Light, the partner of `atom-one-dark`.
- **Textual Dark**: the VS Code Dark+ colours of Textual's `vscode_dark`
  text area theme.
- **Tokyo Night**: the `night` style of the published palette.

`Palette` stays as it is; no theme needs more roles.

### Lists

- `CODE_THEMES` (`src/config/tui.rs`) gains the nine own palettes after
  `gruvbox-dark`, so the `code` key of any theme can name them. Names are
  lower case like the bundled themes; `Monokai` stays unknown.
- `BUNDLED` (`src/config/themes.rs`) gains the ten files. Order in the theme
  list: the four of today, then the new ones by name. **(proposal)**
- A user theme file of one of these names replaces the bundled one, as for
  the others.

### Terminals without true colour

Nothing new: `#rrggbb` is shown as the nearest of the 256 colours, as for
the existing bundled themes.

## Spec changes

- `spec/config.md`, "Themes": the list of bundled themes; the `code` values;
  "Theme files", "Bundled".
- `spec/tui.md`: the theme list snapshot text, and "Appearance" where it
  names the syntax theme of each bundled theme.
- `readme.md`: the list of bundled themes and syntax themes.
- `CHANGELOG.md`, `todo.md`.

## Testing

- All bundled themes load without warnings, in the new order
  (`bundled_themes_load_without_errors`).
- Each new `code` name is accepted; `Monokai` is still refused, and the
  error lists the new names.
- Each own palette builds a `syntect` theme whose `foreground` is its
  `text` colour, and colours a Rust keyword, string and comment with the
  palette's colours (the loop over `OWN_PALETTES` names in `highlight.rs`
  grows).
- The theme list snapshot shows the new themes; one frame drawn with
  `tokyo-night` and one with `one-light` (light background).
- Contrast, in a test over every bundled theme: `text` on `background` at
  least 4.5:1, `dim`, `accent`, `success`, `error` and `warning` at least
  3:1. A value above that fails is changed to the nearest shade of its
  palette that passes, and noted under "Changes while building".

## Implementation plan

1. **Syntax palettes**
   - Status: done
   - Files: `src/tui/highlight.rs`, `src/config/tui.rs`
   - Result: `code` takes the nine new names; code is coloured with them.
   - Tests: palette themes, `code` names.

2. **Theme files**
   - Status: done
   - Files: `themes/*.toml`, `src/config/themes.rs`, `src/config/tests.rs`
   - Result: `/theme tokyo-night` and the others work.
   - Tests: bundled themes load, contrast.

3. **TUI and docs**
   - Status: done
   - Files: `src/tui/tests.rs`, snapshots, `spec/config.md`, `spec/tui.md`,
     `readme.md`, `CHANGELOG.md`, `todo.md`
   - Result: the theme list shows them; the docs list them.
   - Tests: snapshots.

## Alternatives

- **`.tmTheme` files from the projects, loaded by `syntect`.** Exact syntax
  colours, but ten more files of a few hundred lines each, licences to
  check per file, and a second way to define syntax colours next to
  `Palette`. The ten roles of `Palette` are enough for what the TUI shows.
- **Only the interface colours, with an existing syntax theme.** Monokai
  code in `base16-ocean.dark` colours does not look like Monokai; the point
  of these themes is that the whole screen matches.
- **One `flexoki` theme.** Flexoki is designed as a pair; with
  `flexoki-dark` and `flexoki-light` the light half costs one file and one
  palette.
- **Names with `_` as asked (`material_oceanic`, `one_light`).** Valid names,
  but next to `atom-one-dark` and `gruvbox-dark` the list would mix both
  styles.

## Changes while building

- `emacs` `warning`: `#ff8c00` (DarkOrange, 2.3:1 on white) became
  `#cd6600` (DarkOrange3, 3.8:1), the nearest shade that passes.
- The contrast test covers the ten new themes only. The four older ones keep
  their published colours, though some are below the ratios:
  `atom-one-dark` and `nord` `dim`, and `solarized-light` `text`, `dim`,
  `success` and `warning`; Solarized has no darker green or yellow.
- The theme list snapshots are 40 rows high, so the theme files group still
  shows below the longer bundled group.

## Open questions

Answered with the proposals: `flexoki-dark` and `flexoki-light`; names with
`-`; `emacs` is the default light faces; `textual-dark` takes the VS Code
Dark+ colours; `solarized-light` stays on `syntect`'s theme; each theme file
names the source and licence of its colours.


1. Names: `flexoki-dark` and `flexoki-light` (proposed), or `flexoki` for the
   dark one alone? `material-oceanic` and `one-light` with `-` (proposed),
   or with `_` as asked?
2. `emacs`: the default light faces (proposed), or the dark variant Emacs
   picks on a dark background (`--reverse-video`, `frame-background-mode
   dark`)?
3. `textual-dark` syntax colours: VS Code Dark+ as in Textual's
   `vscode_dark` (proposed), or Monokai, which Textual's Markdown code
   blocks show by default? To be checked against Textual's source before
   building.
4. Should `solarized-light` get an own palette instead of `syntect`'s
   `Solarized (light)`, so both Solarized themes are built the same way?
   Proposed: no; `syntect`'s Solarized themes match the published palette.
5. Licences: the palettes are colour values (Flexoki, Tokyo Night,
   Nightfox, Material, One and Solarized are MIT; Monokai's and Emacs'
   colours are used widely). Proposed: name the source and licence in the
   comment at the top of each theme file, as the existing ones name theirs.
