# Images in the input

- RFC: 0034
- Status: done
- Spec: [`spec/tui.md`](../spec/tui.md), "Input editor", "Keys";
  [`spec/providers.md`](../spec/providers.md), "Message", open question 2;
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/config.md`](../spec/config.md), "Model keys"

## Summary

`Ctrl-V` in the TUI input takes an image from the clipboard (a screenshot)
and puts a placeholder `[Image #1]` at the cursor. The user writes text
around it as usual, and can paste more images; `Enter` sends the text and
the images together as one message. Models that take images see them; the
conversation shows the placeholders with the image's type, size and
dimensions. With text in the clipboard and no image, `Ctrl-V` pastes the
text.

## Motivation

- A screenshot says what text cannot: a broken layout, an error dialog, a
  diagram, code in a tool that does not let one copy it. Today the user has
  to describe it, or save it and ask the model to read a file that
  `read_file` cannot show (see `spec/tools.md`, open question 1).
- The models `codeagent` is used with (Claude, GPT, Gemini) all read images.
- `spec/providers.md` has the question open since the first version.

## Scope

In:

- `Ctrl-V` in the input: an image from the clipboard, else its text.
- A pasted path of an image file (drag and drop into the terminal) becomes
  an image too.
- `/image PATH` for the cases where the clipboard cannot be read (SSH).
- Image placeholders in the input editor; text written before, between and
  after them.
- An `Image` block in the shared types, the protocol, the saved session and
  export/import.
- Encoding in the `anthropic` and `openai-compatible` adapters (and
  `gemini`, RFC 0032, when it is built).
- A model key `images` that says whether a model takes them.

Out:

- Showing the image itself in the terminal (sixel, kitty graphics).
- Resizing or converting images (would need an image crate).
- Images from tools: `read_file` on a picture, screenshots of a browser.
- Images in `codeagent chat` and `codeagent run` (the protocol allows them;
  `run --image PATH` can come later).
- PDFs and other documents.

## Design

### Reading the clipboard

The terminal does not pass images: bracketed paste carries text only, and
OSC 52 reads (where allowed at all) are text only. So on `Ctrl-V` the TUI
asks the desktop's clipboard itself, with the same programs `tui.clipboard`
already uses for copying (`src/tui/clipboard.rs`), no new crate:

| Desktop | Types | Image |
|---------|-------|-------|
| Wayland (`WAYLAND_DISPLAY`) | `wl-paste --list-types` | `wl-paste --type image/png` |
| X11 (`DISPLAY`) | `xclip -selection clipboard -t TARGETS -o` | `xclip -selection clipboard -t image/png -o` |
| macOS | `osascript -e 'clipboard info'` | `osascript` writing `«class PNGf»` to a temporary file in the session's scratch directory, read and deleted **(proposal)** |

- The first of `image/png`, `image/jpeg`, `image/webp`, `image/gif` in the
  list is taken. Without an image type, `Ctrl-V` reads the clipboard's text
  (`wl-paste`, `xclip -o`, `pbpaste`) and pastes it as bracketed paste
  does.
- The program runs off the TUI's main loop with a timeout of 2 s; the input
  shows `pasting…` meanwhile, and keys typed meanwhile are kept.
- Over SSH (`SSH_TTY`, `SSH_CONNECTION`) and with `tui.clipboard = "off"` or
  `"osc52"`, the clipboard is not read; `Ctrl-V` says so in the status line:
  `Ctrl-V: no access to the local clipboard over SSH; use /image PATH`.
- Without the programs: `Ctrl-V: install wl-clipboard or xclip to paste
  images`.
- Terminals that take `Ctrl-V` for their own paste (Windows Terminal, some
  configurations of others) never send it; their paste arrives as
  bracketed paste of text, as today. `Alt-V` does the same as `Ctrl-V`.
  **(proposal)**

### Image files

- Pasted text that is exactly one path (after trimming, quotes and `file://`
  removed, `~` expanded) of an existing file with an image type becomes an
  image instead of text. Terminals paste dropped files this way.
  **(proposal)**
- `/image PATH` adds the image at `PATH` (relative to the workspace) to the
  input and keeps the text typed so far. Tab completes paths as for other
  file arguments.
- The type comes from the file's first bytes, not its name: PNG, JPEG, GIF
  and WebP. Anything else is refused: `not an image (PNG, JPEG, GIF or
  WebP)`.
- Reading a file outside the workspace is the user's own action and asks no
  permission; the file is read by the TUI, not by a tool.

### Limits

- One image: at most 5 MiB (Anthropic's limit per image; the others are
  higher). A larger one is refused with its size and a hint to crop it.
  **(proposal)**
- One message: at most 10 images and 12 MiB of images, so that the
  base64-encoded `send` stays below the protocol's 16 MiB per message.
- Width and height are read from the header (PNG `IHDR`, JPEG `SOF`, GIF
  and WebP headers) for the display and the token estimate; an image whose
  header cannot be read is refused.

### Input editor

- An image is one placeholder in the text: `[Image #N]`, numbered from 1 in
  the order of pasting, per message. It is drawn in the accent colour and
  moves and deletes as one character: `Backspace` behind it, or `Delete`
  before it, removes it and its image. **(proposal)**
- Text is written around it as usual; newlines, cursor keys and `Ctrl-W`
  work as before. Numbers are not reused within a message, so `[Image #2]`
  stays `#2` when `#1` is deleted; they start at 1 again after sending.
- Typing the text `[Image #1]` by hand is plain text, not an image.
- Above the input, while images are attached:
  `images: #1 PNG 1366×306 48 KB · #2 JPEG 800×600 91 KB`.
- History (`Up` / `Down`) brings back the text and its images.
- `Ctrl-C` with text clears images too.
- `Enter` with only images and no text sends them; with a model that does
  not take images (below), `Enter` is refused and the status line says
  which model and how to change it.

### Sending

- `send` (`src/protocol/mod.rs`) gains `images: [{media_type, data}]`, data
  in base64, in placeholder order. The protocol version goes up.
- The user message becomes blocks in text order: the text before
  `[Image #1]`, the image, the text after, and so on. The placeholder stays
  in the text block before the image, so the model can refer to "image 1".
  **(proposal)**
- A queued message (sent while a turn runs) keeps its images.
- The `user_message` event carries, per image, `media_type`, size, width,
  height and its hash, not the data; other attached clients show the
  placeholders and the line above the text.

### Shared types

`Block::Image { media_type, data }` in user messages (`data`: the bytes,
shared with `Arc<[u8]>` so the history is not copied per request).

| Adapter | Wire format |
|---------|-------------|
| `anthropic` | `{"type": "image", "source": {"type": "base64", "media_type", "data"}}` |
| `openai-compatible` | user `content` as an array: `{"type": "text", "text"}` and `{"type": "image_url", "image_url": {"url": "data:<type>;base64,<data>"}}` |
| `gemini` (RFC 0032) | `{"inlineData": {"mimeType", "data"}}` part |

The prompt cache (RFC 0028) works as before; the image is part of the
cached prefix.

### Models without images

- Model key `images = true | false`. Default: `true` for `anthropic` and
  `gemini`, `false` for `openai-compatible`, where many local models are
  text-only. **(proposal)**
- The TUI refuses to send images to a model with `images = false`.
- When a session with images in its history switches to such a model, the
  adapter sends each image as the text `[image omitted: this model does not
  read images]`, so the history stays valid.

### Context and tokens

- Images count in the context gauge with an estimate of
  `width × height / 750` tokens each (Anthropic's formula; models scale
  large images down, so the estimate is an upper bound). **(proposal)**
- Trimming old tool results (RFC 0008) does not touch images: they are in
  user messages, not tool results. **(proposal)**

### Saving

- Images are saved once per session as
  `sessions/<id>/images/<sha256>.<ext>` (mode `0600`); `history.jsonl` and
  `events.jsonl` hold the hash and the metadata. The same screenshot pasted
  twice is one file. **(proposal)**
- On resume the images of the history are read into memory; a missing file
  becomes the "omitted" text, with a warning in the log.
- `codeagent export` writes the images into its JSON file, base64 under
  their hash; `import` writes them back as files. The session format stays `1`: an older
  `codeagent` that meets an image block refuses the session as unknown
  content, which is the rule for newer formats. **(to check)**

### Secrets

Secret hiding (RFC 0030) works on text and cannot look into an image. The
first image of a session shows a one-time note in the conversation:
`Images are sent as they are; secrets in them are not hidden.`
**(proposal)**

## Spec changes

- `spec/tui.md`: "Input editor" (placeholders, `Ctrl-V`, `Alt-V`, dropped
  files, `/image`), "Keys", the conversation's display of user messages
  with images.
- `spec/providers.md`: `Image` in "Message"; the three wire formats; open
  question 2 answered.
- `spec/config.md`: model key `images`; `tui.clipboard` also governs
  reading.
- `spec/sessions.md`: `images/` in the session directory; export.
- `spec/agent-loop.md`: the image token estimate.
- `readme.md`, `CHANGELOG.md`, `todo.md`.

## Testing

- Clipboard: the program chosen for each desktop, with a fake `PATH` of
  scripts that print a type list and image bytes; text fallback; timeout;
  SSH; `clipboard = off`.
- Image sniffing and dimensions for PNG, JPEG, GIF and WebP fixtures;
  refusal of other files, of too large ones and of broken headers.
- Editor: placeholders insert at the cursor, delete as one, keep numbers,
  survive history; text around them.
- Message blocks in text order for text, image, text, image.
- Adapters: the expected JSON for each format; the "omitted" text for
  `images = false`.
- Saving: images written once per hash, `0600`; resume; a missing file;
  export and import.
- Protocol: a `send` with images; the size limits.
- TUI snapshots: input with two placeholders and the image line; the
  conversation showing a user message with an image.
- By hand: a screenshot pasted on Wayland, X11 and macOS, and a dropped
  file, sent to a Claude model.

## Implementation plan

1. **Image block**
   - Status: done
   - Files: `src/provider/types.rs`, `src/provider/anthropic.rs`,
     `src/provider/openai.rs`, `src/config/`
   - Result: a message can hold images and the adapters send them; model
     key `images`.
   - Tests: encoding, `images = false`.

2. **Protocol and saving**
   - Status: done
   - Files: `src/protocol/`, `src/session/store.rs`, `src/agent/`,
     export and import
   - Result: images travel from a client to the model and are kept.
   - Tests: send, limits, resume, export.

3. **Images from files**
   - Status: done
   - Files: `src/tui/image.rs` (new: sniffing, dimensions),
     `src/tui/editor.rs`, `src/tui/app.rs`, `src/tui/commands.rs`
   - Result: `/image PATH` and dropped files work, with placeholders.
   - Tests: sniffing, editor, snapshots.

4. **Clipboard**
   - Status: done
   - Files: `src/tui/clipboard.rs`, `src/tui/keys.rs`
   - Result: `Ctrl-V` and `Alt-V` paste images and text.
   - Tests: clipboard with fake programs.

5. **Docs**
   - Status: done
   - Files: `spec/`, `readme.md`, `CHANGELOG.md`, `todo.md`
   - Result: the docs describe it.

## Alternatives

- **A clipboard crate (`arboard`).** One API for all desktops, but a larger
  dependency with X11 and Wayland libraries linked in, while the programs
  are already used for copying.
- **Only `/image PATH`.** Simple and works over SSH, but a screenshot then
  has to be saved first; the point is the paste.
- **Images as a separate list, not placeholders in the text.** Easier in
  the editor, but the user cannot say "compare the first with the second"
  by position, and the model gets the images apart from the text they
  belong to.
- **The kitty clipboard protocol** (OSC 5522), which can read images over
  SSH in kitty. Only one terminal; a later addition if wanted.
- **Saving images inline in `history.jsonl` as base64.** One file less,
  but every history line with an image becomes megabytes and the same
  screenshot is stored again per paste.

## Changes while building

- Per message at most 11 MiB of images, not 12: in base64 12 MiB is the
  protocol's whole 16 MiB.
- No token estimate for images: the context gauge shows the tokens the
  provider counted, so there is nothing to estimate.
- A dropped path is taken as an image only with an image extension, so
  pasting a path of another file never reads it; the type still comes from
  the bytes.
- The images live in the input editor, so they move with session switches.
  The history brings them back only for the last 8 messages with images
  sent from this TUI, and not when a number is taken by another image.
- The server, not the TUI, checks the model's `images`, so `codeagent chat`
  is covered too; a refused message comes back to the input with its
  images.
- The note about secrets comes once per session, judged by the history.
- `/image` has no Tab completion of paths: the TUI completes no paths yet.
- Format stays 1. An older `codeagent` refuses a history with an image
  block as unreadable.

## Open questions

Answered with the proposals: `Ctrl-V` also pastes text; model key `images`;
5 MiB per image and 10 per message, no resizing; `[Image #1]`; old images
are kept. `codeagent run --image` is left for later.

