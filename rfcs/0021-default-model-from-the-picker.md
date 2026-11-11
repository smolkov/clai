# Default model from the model picker

- RFC: 0021
- Status: draft
- Milestone: none; it extends the model picker of RFC 0003
- Spec: [`spec/tui.md`](../spec/tui.md), "Model picker";
  [`spec/config.md`](../spec/config.md), "Providers and models",
  "Reading and writing `[tui]`", "Environment variables";
  [`spec/command.md`](../spec/command.md), "clai models", "clai chat"

## Summary

The model picker gets a second choice under the list: `[ ] Also make it
the default for new sessions`. `Space` ticks it; `Enter` then switches the
session (or creates the new one) and writes `default_model` into the
user config file, the same safe way `/settings` writes `[tui]`. The row
marks show the new default as soon as the server reloads. When something
stronger than the user file decides the default (`CLAI_MODEL`, or a
project file's `default_model`), the picker says so, before and after
saving. The same is possible without the picker: `/model NAME --default`
in the TUI and `clai chat`, and `clai models --set-default NAME`.

## Motivation

- The picker marks `default`, but cannot change it. To make a model the
  default the user leaves the TUI, finds the user file, and edits
  `default_model` by hand with the exact `provider/model` spelling the
  picker just showed.
- New sessions (`/new`, `n` in the session list, `clai`) open the picker
  on `default_model`. A user who keeps choosing another model there has
  to choose it every time.
- The spec says a model change "applies to this session only … and
  `default_model` is not affected" (`spec/tui.md`). That stays true for
  `Enter`; the user needs a separate, deliberate way to change the
  default.
- `default_model` can come from three places (environment, project file,
  user file). Today nothing tells the user which one is in force, so
  editing the user file can seem to do nothing.

## Scope

In:

- The picker: the "default" choice, its key, the footer, the notes.
- Writing `default_model` into the user file: placement, checks, errors.
- Which source decides the default, and how the picker shows it.
- `/model NAME --default` in the TUI and `clai chat`;
  `clai models --set-default NAME`.
- The `models` answer: where the default comes from.

Out:

- Removing `default_model`. It is required (`spec/config.md`,
  "Validation"), so the user always changes it to another model.
- Writing the project file. It comes with the repository; `clai` writes
  only the user's own file, as for `[tui]`.
- `[commit] model` and other per-purpose models.
- A default per workspace kept by `clai` itself (outside the config
  files).

## Design

### The picker

```
┌ Model for "project" ──────────────────────────────────────────────────────┐
│ Filter: _                                                                 │
│                                                                           │
│ anthropic                                                                 │
│   sonnet      claude-sonnet-5-5         200k   ✓ current, default         │
│ ▶ opus        claude-opus-5-5           200k                              │
│ local                                                                     │
│   qwen        qwen3-coder:30b            32k   ⚠ history 41k > 32k        │
│                                                                           │
│ [x] Also make it the default for new sessions                             │
│     saved as default_model in ~/.config/clai/config.toml                  │
│                                                                           │
│ Enter select ─ Space default ─ Esc cancel                                 │
└───────────────────────────────────────────────────────────────────────────┘
```

| Key | Action |
|-----|--------|
| `Space` | Tick or untick "Also make it the default". |
| `Enter` | As now: select and close. With the tick, also save the selected model as `default_model`. |
| `Esc` | Close; nothing is saved, the tick is dropped. |

- `Space` no longer goes into the filter. The filter matches parts of
  `provider/name` and of the model id, and neither has spaces in a
  normal configuration, so a typed space only ever hid every model. Text
  pasted into the filter is kept as it is, spaces included. `Space`
  ticks in the session list too ("mark"), so the key means the same in
  both boxes.
- The tick is not a list row, so `Up` / `Down` and the filter keep
  working as before.
- The tick starts unticked every time the picker opens. **(proposal)**
- On the row that already is the default, the tick line reads
  `already the default` (dim) and `Space` does nothing.
- With the tick and `Enter` on the session's current model, the session
  does not change (as now) and only the default is saved. This is how to
  make the current model the default.
- For a new session (`Model for a new session`), the tick works the same:
  the session is created with the model, and the model becomes the
  default. The picker opens on `default_model` there, so the tick is
  useful only after moving to another model.
- The line under the tick names the file that will be written: the user
  file the server reads (the `config` of the `models` answer), with `~`
  for the home directory.
- In ASCII mode the tick is `[x]` / `[ ]` as well; the footer separators
  follow `tui.symbols`.

### Where the default comes from

The default of a new session is, in order (`spec/config.md`):
`CLAI_MODEL` in the server's environment, then the project file's
`default_model` for that workspace, then the user file. Saving into the
user file changes only the last.

The `models` answer gains `default_from` **(proposal)**:

| Value | Meaning |
|-------|---------|
| `user` | The user file's `default_model`. |
| `env` | `CLAI_MODEL` of the server. |
| `project` | The project file of the workspace asked about. |

The `models` request gains an optional `workspace`, the attached
session's (or, for a new session, the chosen workspace), so the server can
read that workspace's project file. Protocol version: the next one.

When the default does not come from the user file, the tick line says
what wins, in the warning colour, and the tick still saves:

```
│ [x] Also make it the default for new sessions                             │
│     ⚠ CLAI_MODEL=local/qwen overrides the saved default                   │
```

```
│     ⚠ .clai/config.toml of this workspace sets default_model = local/qwen │
```

The save still happens, because the user file is what other workspaces
and a later server without `CLAI_MODEL` use.

### Saving

- `default_model = "provider/model"` is set at the top level of the user
  file, with the full reference (never the short form), so it stays right
  when another provider adds a model of the same name. An existing key
  keeps its place and its comment on the line; a new key goes before the
  first table, where TOML requires top-level keys. **(proposal)**
- The same rules as for `[tui]` (`spec/config.md`, "Reading and writing
  `[tui]`"): the rest of the file is kept byte for byte; the new text is
  checked as loading would check it, and nothing is written if that
  fails; nothing is written if the file changed since the picker read
  its stamp; the file is replaced atomically, keeping its mode, through a
  link to its target.
- Without a user file the server read (a `--config` file that was removed
  since), saving fails with `no user config file to save
  the default in: <path>`; the model is still selected.
- The TUI writes the file, as it does for `/settings`. The server sees the
  change on its next poll (every 2 s) and sends `config_reloaded`; the
  TUI then asks for `models` again, and the `default` mark moves.
- After a save the status line shows
  `default model: anthropic/opus (saved in ~/.config/clai/config.toml)`,
  or the error. A failed save does not undo the model selection.
- Sessions that run keep their model; only new sessions use the new
  default.

### What uses the saved default

The saved model is the default of every later start, because it is in the
file and not in the TUI's or the server's memory:

| Start | Model |
|-------|-------|
| `clai`, `clai new` without `--model` | The saved default. |
| `/new`, `n` in the session list | The picker opens on the saved default. |
| `clai run TASK` without `--model` | The saved default (it reads the file itself). |
| `clai commit` without `--model` and `[commit] model` | The saved default. |
| A server started later, or after a reboot | Reads the file; the saved default. |
| Saved sessions, resumed | Their own model, as now. |

`CLAI_MODEL` and a project file's `default_model` still win for the
starts they apply to; the picker warns about them (above).

**No 2-second gap.** The server reloads the file on its poll, every 2 s.
A `clai` started right after the save (quit the TUI, start it again)
would reach the running server before the poll and get the old default.
So before the server creates a session without a model, and before it
answers `models`, it compares the user and project files' stamps with
the ones it loaded, and reloads first if they changed. **(proposal)** The
same check covers a default changed by hand in an editor.

`src/config/edit.rs` gains `set_top(text, key, value)` next to
`set_tui`, and `save` takes the change as a closure over the text, so
both share the stamp check, validation and atomic write.

### Without the picker

- TUI and `clai chat`: `/model NAME --default` switches the session to
  `NAME` and saves it as the default; `/model --default` saves the
  session's current model. The command list (`/` in the input) and
  argument completion know the flag.
- `clai models --set-default NAME` saves the default without a session,
  through the same `config::edit` code, and prints
  `default model: anthropic/opus (saved in …)` and the override warning
  if there is one. It does not need the server. **(proposal)**
- `clai models` (and `--json`) marks where the default comes from:
  `default (CLAI_MODEL)`, `default (project)`.

## Spec changes

- `spec/tui.md`, "Model picker": the tick line, `Space` (and that it
  no longer edits the filter), the save, the
  override warnings, the status line; the sentence "`default_model` is not
  affected" gains "unless the default tick is set".
- `spec/config.md`: "Reading and writing `[tui]`" becomes "Writing the
  user file", covering `[tui]` and `default_model`.
- `spec/command.md`: `clai models --set-default`; the source marks;
  `/model NAME --default` in `clai chat`.
- `spec/architecture.md`, "Protocol": the next version; `models` with
  `workspace`, the answer with `default_from`.

## Testing

- `src/config/edit.rs`: `set_top` on a file with the key (value and
  trailing comment kept), without it (inserted before the first table),
  with only tables, empty; `save` with a changed stamp, with an invalid
  result, through a link, keeping the mode.
- `src/config/tests.rs`: `default_from` for env, project and user.
- `src/tui/tests.rs`: `Space` ticks and unticks and leaves the filter
  as it is; a pasted space still reaches the filter; `Enter` with the tick sends
  the model change and writes the file; on the current model only writes;
  `Esc` writes nothing; `already the default`; the override warnings;
  a failed save shows the error and keeps the selection; the mark moves
  after `config_reloaded`. Snapshots of the picker with the tick, with a
  warning, and in ASCII.
- `src/protocol/tests.rs`: `models` with `workspace`, `default_from`.
- `src/server/tests.rs`: after the user file's `default_model` changes,
  a `new` request without a model, sent before the next poll, gets the
  new default; so does `models`.
- `tests/server.rs`: save the default, stop the TUI, run `clai new -d`
  at once: the session has the saved model; `clai server stop`, start
  again: still the saved model.
- `tests/models.rs`: `clai models --set-default` writes the file, refuses
  an unknown model with the list, warns about `CLAI_MODEL`.

## Implementation plan

1. **Writing `default_model`**
   - Status: todo
   - Files: `src/config/edit.rs`, `src/config/tests.rs`
   - Result: `set_top` and the shared `save`; `/settings` unchanged.
   - Tests: the edit tests.

2. **Where the default comes from**
   - Status: todo
   - Files: `src/config/mod.rs`, `src/protocol/mod.rs`,
     `src/server/requests.rs`, `src/server/mod.rs`,
     `src/protocol/tests.rs`, `src/server/tests.rs`
   - Result: `default_from`; `models` takes a workspace; protocol bumped;
     `new` and `models` reload a changed config first.
   - Tests: config and protocol tests.

3. **The picker**
   - Status: todo
   - Files: `src/tui/picker.rs`, `src/tui/view/picker.rs`,
     `src/tui/app.rs`, `src/tui/tests.rs`, snapshots
   - Result: the tick, `Space`, saving on `Enter`, warnings, status line.
   - Tests: the TUI tests and snapshots.

4. **Commands**
   - Status: todo
   - Files: `src/tui/commands.rs`, `src/command/chat/`,
     `src/command/models.rs`, `tests/models.rs`
   - Result: `/model NAME --default`, `/model --default`,
     `clai models --set-default`, source marks in `clai models`.
   - Tests: command tests, `tests/models.rs`.

5. **Spec and readme**
   - Status: todo
   - Files: the spec files above, `readme.md`, `todo.md`
   - Result: the spec and readme describe the default choice.
   - Tests: `ci/check.sh`, `ci/test.sh`.

## Alternatives

- **A second key that saves at once** (`Ctrl-D` "make default" on the
  selected row, without closing). Faster, but `Ctrl-D` detaches elsewhere,
  and a save on a single key is easy to hit by mistake. The tick is
  visible before anything is written.
- **`Tab` for the tick.** Free in the picker and leaves the filter
  whole, but `Tab` switches tabs or focus in the other boxes, and
  `Space` already means "mark" in the session list. Chosen by the user:
  `Space`.
- **`Shift-Enter` / `Alt-Enter` = select and make default.** No extra
  line, but invisible, and `Shift-Enter` is not reported by every
  terminal (`spec/tui.md`, "Input editor").
- **Ask "Make it the default too? [y/n]" after each `Enter`.** One more
  question on every model change, for a rare wish.
- **Remember the last chosen model as the default automatically.** What
  some tools do, but it changes the default behind the user's back, and a
  quick switch to a small model for one question would stick.
- **Put the default into `/settings`.** `/settings` is about `[tui]`; the
  picker already lists the models with their marks.

## Open questions

1. ~~The key for the tick~~: answered, `Space` (see "The picker").
2. Should the tick stay set between openings of the picker in one TUI
   run, or start unticked each time (proposal)?
3. `clai models --set-default NAME` (proposal), or a general
   `clai config set KEY VALUE` that also covers `[tui]` keys?
4. When a project file sets `default_model`, should the tick offer to
   write the project file instead? (Proposal: no, it belongs to the
   repository.)
5. Should a saved default also be offered for `[commit] model` (a second
   tick, or a choice of which default)?
