# clai

Command line AI assistant.

## Features

- Chat with an AI model directly from the terminal
- Generate a git commit message from `git diff`
- Correct and improve your English
- Translate text into English
- Multi-provider support: Gemini (default), OpenAI, and Claude
- File tools (read, list) usable by the model
- Session history persisted to disk

## Installation

```sh
cargo install --path .
```

## Configuration

On first run `clai` creates a default config at `~/.config/clai/config.toml`.

```toml
model = "gemini"

[[models]]
name = "gemini"
provider = "gemini"
api_key = ""            # or GEMINI_API_KEY env var
model = "gemini-2.5-flash"
```

You can define several models and switch between them with the top-level
`model` field. Supported providers: `gemini`, `openai`, `claude`.

The default Gemini model config falls back to the `GEMINI_API_KEY` and
`GEMINI_MODEL` environment variables if no key is set in the file.

## Usage

```sh
# Ask a question
clai chat "What is the best way to learn Rust?"

# Generate a git commit message from the current diff
clai commit

# Correct your English
clai correction "i wants to going to the store"

# Translate into English
clai translate "Je voudrais un café."
```

## Commands

| Command      | Description                              |
| ------------ | ---------------------------------------- |
| `chat`       | Chat with an AI model                    |
| `commit`     | Summarize `git diff` into a commit message |
| `correction` | Correct and improve written English      |
| `translate`  | Translate text into English              |

## License

MIT OR Apache-2.0
