# Syntax highlighting

- RFC: 0017
- Status: done
- Spec: [`spec/tui.md`](../spec/tui.md), "Conversation entries", "Permission
  prompts", "Questions from the model", "Details view", "Appearance";
  [`spec/config.md`](../spec/config.md), "TUI", "Themes"

## Summary

The TUI colours code by its language everywhere it shows code:

- **Code blocks** in answers of the model and in questions from the model.
- **Diffs** of `edit_file` and `write_file`: in previews, in expanded calls,
  while a permission question is open, in the details view and in `/diff`.
- **New files** of `write_file`, and the **command** of `run_command`.

The language comes from the fence of a code block (```` ```rust ````) or from
the file name of an edit. Colours come from a syntax theme: the theme's `code`
key, or a default that fits the built-in theme. `tui.syntax_highlight`
(already in the configuration, today "later version") turns it on and off.

## Motivation

- Code is a large part of what the TUI shows: answers with code blocks, and
  diffs the user has to judge before answering `y`. In one colour, a wrong
  brace or a string that does not end is hard to see.
- `syntax_highlight` and the theme key `code` are in the configuration since
  RFC 0006, but do nothing; the settings box says "later version".
- Since RFC 0016, finished edits show their diff by default. Many more lines
  of code are on the screen now.

## Scope

In:

- Highlighting in the TUI: code blocks (answers, questions from the model,
  the ask box), diffs and new file contents (previews, expanded calls,
  permission questions, details view, `/diff`), the command line of
  `run_command`.
- Detection of the language: fence info string, file extension, first line.
- Syntax themes: the theme's `code` key, defaults per built-in theme, a theme
  on the terminal's 16 colours.
- `tui.syntax_highlight`, and its row in the settings box.
- Limits and a cache, so that long sessions and streaming stay fast.

Out:

- `clai chat` and `clai run`: they print plain text and ANSI colours for diffs
  only. They can use the same highlighter later **(proposal)**.
- Inline code (`` `parse` ``): it keeps the `code` style; a single word has no
  context to highlight.
- Command output and `read_file` results: they are not code in one language.
- Semantic highlighting (types, references); only what a grammar sees.
- Themes of other editors beyond what the crate bundles; see the open
  questions.

## Design

### Where it applies

```
▾ edit_file src/parse.rs                                  ✓ edited src/parse.rs
  @@ -12,3 +12,3 @@                                     ← accent, as today
       let lines: Vec<&str> = text.lines().collect();   ← context: highlighted
  -    for i in 0..lines.len() - 1 {                    ← `-` diff_remove, code highlighted
  +    for i in 0..lines.len() {                        ← `+` diff_add, code highlighted
```

| Place | Language from | Notes |
|-------|---------------|-------|
| Code block in an answer | The fence: ```` ```rust ````, ```` ```py ````; an indented block has none | Highlighted as it streams. |
| Code block in a question from the model (prompt, ask box) | The fence | Same renderer as answers. |
| Diff of `edit_file`, `write_file` | The file's extension, then its first line (`#!/bin/sh`) | Each line is highlighted without its `+`, `-` or space. |
| New file of `write_file` | As for a diff | |
| `$ command` of `run_command` | Always shell | Only the command line, not the output. |

- **Diff lines:** the marker (`+`, `-`) keeps the `diff_add` /
  `diff_remove` colour; the code after it gets the syntax colours. To keep
  added and removed lines apart at a glance, the line also gets a background
  tint of its diff colour when the terminal shows true colour **(proposal)**;
  on 256 and 16 colours only the marker shows it.
- **State across lines:** a diff hunk is highlighted as one piece of code, so
  a string or comment that spans lines is coloured right. Removed and added
  lines are highlighted as two texts (the old and the new side), context lines
  in both **(proposal)**, so that a `-` line that opens a comment does not
  colour the `+` lines after it.
- **Unknown language:** the text keeps the style it has today (`code` for
  blocks, diff colours for diffs).

### Syntax themes

- A syntax theme maps a grammar's scopes (`keyword`, `string`, `comment`,
  `entity.name.function`, …) to colours.
- **Choice:** the `code` key of the theme in use (see `config.md`, "Themes")
  names it. Without one, the built-in theme decides **(proposal)**:

  | Theme | Syntax theme |
  |-------|--------------|
  | `terminal` | `ansi`: built in to `clai`, on the terminal's 16 colours, so it follows the terminal's palette like the rest of the theme. |
  | `dark` | `base16-ocean.dark` |
  | `light` | `InspiredGitHub` |
  | bundled `nord`, `gruvbox-dark`, `atom-one-dark`, `solarized-light` | A `code` key in each file, one of the names below. |

- **Names:** the themes the crate bundles (`base16-ocean.dark`,
  `base16-eighties.dark`, `base16-mocha.dark`, `base16-ocean.light`,
  `InspiredGitHub`, `Solarized (dark)`, `Solarized (light)`) and `ansi`. An
  unknown name is an error of the configuration with the list of names, as an
  unknown theme is.
- **Colours** go through the same colouring as the theme: exact with true
  colour, the nearest of the 256 colours otherwise, none with `NO_COLOR`.
  Only foregrounds, bold and italic are used; a syntax theme's background is
  not, so the theme's `background` stays.
- **The `ansi` theme:** a short table of clai's own, e.g. keywords `magenta`,
  strings `green`, comments the `dim` style, numbers and constants `yellow`,
  functions `blue`, types `cyan`. It keeps a `terminal` TUI in the terminal's
  own colours.

### Highlighter

- **`tui/highlight.rs` (new):** `Highlighter` holds the syntaxes and the
  syntax theme. `highlight(language, text) -> Vec<Vec<Piece>>`: one list of
  styled pieces per line. The renderers of Markdown and of tool calls call it
  where they now push one style.
- **Loading:** the syntaxes are loaded on first use, in a background thread
  started with the TUI, so that start-up does not wait **(proposal)**. Until
  they are there, code is shown as today and the entries are laid out again
  when they arrive.
- **Cache:** results are kept by language and text, for the last 512 texts
  **(proposal)**. The entry cache of the conversation already avoids most
  work; this cache covers what is built again: a resize, a theme change, a
  call toggled.
- **Streaming:** while an answer streams, its open code block grows with each
  delta. Complete lines are highlighted once and kept with the parse state
  after them; only the new lines are highlighted. The last line, without its
  newline yet, is shown in the `code` style until it is complete.
- **Limits:** a block or diff of more than 5000 lines, or a line longer than
  2000 characters, is not highlighted, as editors do; grammars get slow there.
  A line is never highlighted for more than a few milliseconds' work; on a
  timeout the rest of the block stays plain **(proposal)**.
- **Copy:** unchanged. Rows keep their sources (RFC 0016); colours are not
  copied.

### Settings

- `tui.syntax_highlight = true` (default) highlights; `false` shows code as
  today. Changing it in the settings box lays out the conversation again, as
  other settings do. The row loses "later version".
- The theme list shows each theme as today; the preview of a theme also
  previews its syntax theme.

## Dependencies

- **`syntect`** with `default-fancy`: grammars and themes of Sublime Text,
  regexes by `fancy-regex` (pure Rust, no `onig` C library). It bundles the
  syntaxes most code uses (Rust, Python, JavaScript, Go, C, Shell, JSON, YAML,
  Markdown, diff, …) and seven themes. Measured: the release binary grows
  from 14.6 MB to 15.3 MB (0.7 MB), with only the features `parsing`,
  `default-syntaxes`, `default-themes` and `regex-fancy`.
- Missing grammars that matter (TOML, TypeScript, Dockerfile) can come from
  `two-face`, which packs the syntaxes of `bat` for `syntect`, at more binary
  size; see the open questions.

## Spec changes

- `spec/tui.md`:
  - "Conversation entries": code blocks and diffs highlighted; how the
    language is found; diff lines keep their markers' colours.
  - "Permission prompts", "Questions from the model", "Details view": the
    same highlighting.
  - "Appearance": syntax themes, the defaults per theme, `NO_COLOR`.
  - "Settings": `syntax_highlight` built.
- `spec/config.md`:
  - "TUI": `syntax_highlight` says what it covers.
  - "Themes": `code`: the names it takes, the default per built-in theme, an
    unknown name is an error.

## Testing

- **Highlighter:** a Rust snippet gets keyword, string and comment styles;
  a multi-line string keeps its style on the next line; an unknown language
  and `syntax_highlight = false` give the old pieces; the limits leave long
  texts plain.
- **Detection:** fence names and aliases (`rs`, `py`, `sh`), extensions,
  shebang lines, none.
- **Diffs:** markers keep the diff colours; old and new sides are highlighted
  apart; the hunk header stays `accent`.
- **Themes:** `code` names, an unknown name as a configuration error, the
  defaults, `ansi` on 16 colours, the 256-colour mapping, `NO_COLOR`.
- **Streaming:** a block that grows by deltas gives the same result as the
  whole block at once; already highlighted lines are not highlighted again
  (counted).
- **Snapshots:** today's snapshots are text only. New tests check the styles
  of cells in the `TestBackend` buffer: a code block, a diff preview, a
  permission question, the details view.
- **Performance:** a test with a 5000-line diff and a session with 1000 code
  blocks lays out within a budget (as the existing long-session test does).

## Implementation plan

1. **Highlighter**
   - Status: done
   - Files: `Cargo.toml`, `tui/highlight.rs` (new)
   - Result: `highlight(language, text)` with detection, limits and cache
   - Tests: highlighter and detection tests
2. **Syntax themes**
   - Status: done
   - Files: `config/tui.rs`, `config/themes.rs`, `tui/style.rs`,
     `themes/*.toml`, `spec/config.md`
   - Result: `code` checked, defaults per theme, `ansi`, colour mapping
   - Tests: theme tests
3. **Code blocks**
   - Status: done
   - Files: `tui/markdown.rs`, `tui/conversation.rs`, `spec/tui.md`
   - Result: answers and questions from the model highlighted, also while
     streaming
   - Tests: streaming tests, style snapshots
4. **Diffs and commands**
   - Status: done
   - Files: `tui/conversation.rs`, `tui/details.rs`
   - Result: previews, expanded calls, permission questions, details view,
     `/diff`
   - Tests: diff tests, style snapshots
5. **Loading and settings**
   - Status: done
   - Files: `tui/mod.rs`, `tui/app.rs`, `tui/settings.rs`
   - Result: background loading; `syntax_highlight` in the settings box; the
     theme preview shows the syntax theme
   - Tests: App tests; performance test
6. **Docs**
   - Status: done
   - Files: `readme.md`, `todo.md`, this RFC
   - Result: status `done`

## Alternatives

- **`tree-sitter` with `tree-sitter-highlight`:** exact parse trees and
  incremental parsing, but one C grammar crate per language, a C compiler
  for the build, and query files to keep up to date. More work for each
  language than `syntect`'s bundled set.
- **`syntect` with `onig`:** the reference regex engine, slightly faster, but
  a C library in the build; `fancy-regex` keeps `clai` pure Rust.
- **Only diff colours, no syntax:** what the TUI has today; the motivation
  above is that it is not enough.
- **Highlight in the server:** one place for all clients, but every event
  would carry styles, and the server does not know the client's theme.
- **Colour whole diff lines only (`+` green, `-` red), syntax only for
  context:** simpler, but the changed lines are the ones the user reads.

## Decisions

Built with the proposals of the open questions; the user asked for the RFC to
be built as it stands. Where the build differs from the design above:

- **No time limit per line:** `syntect` cannot stop a line halfway. The
  limits by size (5000 lines, 2000 characters per line) are the only guard.
- **Diff tint:** only with true colour *and* a theme that sets `background`
  (the bundled dark themes), since the tint is a mix of the diff colour into
  that background; without one the terminal's background is unknown.
- **`ts` fences** use JavaScript, as `syntect` has no TypeScript; TOML has no
  grammar.
- **`ansi` details:** `let` and other storage words are cyan (`storage.type`),
  `fn` magenta; string quotes take the string colour.
- **Bundled themes** use the syntax colours of their own scheme, as the user
  asked: `atom-one-dark`, `nord` and `gruvbox-dark` are syntax themes of
  `clai`'s own, built in code from each scheme's palette (no `.tmTheme` files,
  no `plist` loading), and `solarized-light` uses `syntect`'s
  `Solarized (light)`. `code` takes their names too (open question 5,
  changed).
- **Tests:** the test look has highlighting off, because the grammars load
  once per test process; tests of highlighting turn it on. A 5000-line diff
  lays out in about 3.5 s in a debug build.
- **Edits without details** (logs of an older `clai`) are highlighted too:
  their old and new text are drawn as one hunk.

## Open questions

Answered with the proposals when it was built (see "Decisions").

1. **Crate:** `syntect` with `fancy-regex` (proposal), or `tree-sitter`?
2. **More grammars:** `syntect`'s set (proposal), or `two-face` for TOML,
   TypeScript and others, at a bigger binary?
3. **The `terminal` theme:** `clai`'s own `ansi` table on the 16 colours
   (proposal), or a `syntect` theme mapped to 256 colours?
4. **Diff backgrounds:** a tint of the diff colour behind changed lines with
   true colour (proposal), or markers only everywhere?
5. **Bundled themes:** give `nord` and the others a `code` of the bundled
   syntax themes (proposal), or ship `.tmTheme` files that match them (Nord,
   Gruvbox, One Dark), and allow `code` to name such a file?
6. **`clai chat` and `clai run`:** highlight their diffs and code blocks with
   ANSI colours too, in this RFC or later (proposal: later)?
