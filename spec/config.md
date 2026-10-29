# Configuration

All providers and models `clai` can use are defined in the configuration.
`clai` has no built-in list of providers or models: a model that is not in
the configuration cannot be chosen, neither in the TUI nor on the command
line.

Statements marked **(assumption)** need a decision from the user.

## Files

| File                          | Scope   | May define |
|-------------------------------|---------|------------|
| `~/.config/clai/config.toml`  | User    | Everything: providers, models, defaults, UI settings. |
| `<workspace>/.clai/config.toml` | Project | Defaults and limits only: `default_model`, limits, permission rules. Not providers, not models. |

`$XDG_CONFIG_HOME` replaces `~/.config` when it is set.

Providers and models are only allowed in the user file. A project file comes
with a repository the user may not trust; if it could define a provider, it
could send the user's API key or code to any address. A project file that
contains `[providers]` is rejected with an error naming the file.

Precedence, later wins: user file, project file, environment variables,
command-line options.

Format: TOML. Unknown keys are an error, not ignored, so typos are found.
**(assumption)**

## Commit

Settings for commit messages, used by `clai commit` and the TUI command
`/commit` (see [`commit.md`](commit.md)).
Allowed in the user and the project file.

```toml
[commit]
model   = "local/qwen"   # model for commit messages; default: session model or default_model
trailer = "Co-Authored-By: clai <clai@localhost>"   # added after the body; default: none
```

## Providers and models

```toml
# Model used for new sessions unless another one is chosen.
# Written as "<provider>/<model>", using the names defined below.
default_model = "anthropic/sonnet"

[providers.anthropic]
api         = "anthropic"                 # API type, see below
api_key_env = "ANTHROPIC_API_KEY"         # environment variable with the key

[providers.anthropic.models.sonnet]
id                = "claude-sonnet-5-5"   # model id sent to the API
context_window    = 200000                # tokens
max_output_tokens = 64000

[providers.anthropic.models.opus]
id                = "claude-opus-5-5"
context_window    = 200000
max_output_tokens = 64000

[providers.local]
api      = "openai-compatible"
base_url = "http://localhost:11434/v1"    # e.g. Ollama; no key needed

[providers.local.models.qwen]
id             = "qwen3-coder:30b"
context_window = 32768
```

### Provider keys

| Key            | Required | Meaning |
|----------------|----------|---------|
| `api`          | yes      | API type: `anthropic` or `openai-compatible`. Decides how requests are built. **(assumption: these two first)** |
| `base_url`     | no       | API address. Default: the official address of the API type. Required for `openai-compatible`. |
| `api_key_env`  | no       | Name of the environment variable that holds the API key. |
| `api_key_cmd`  | no       | Command whose output is the API key, e.g. `pass show anthropic`. Run once when the provider is first used. **(assumption)** |
| `timeout_s`    | no       | Request timeout in seconds. Default 300. |
| `max_retries`  | no       | Retries for network errors and rate limits. Default 3. |

At most one of `api_key_env` and `api_key_cmd` is set. With neither, requests
are sent without a key. The key itself is never stored in a config file and
never written to a log or shown in the TUI.

### Model keys

| Key                 | Required | Meaning |
|---------------------|----------|---------|
| `id`                | yes      | Model id the provider's API expects. |
| `display_name`      | no       | Name shown in the TUI. Default: the model's config name. |
| `context_window`    | yes      | Maximum context size in tokens; used for the header gauge and history truncation. |
| `max_output_tokens` | no       | Maximum tokens per answer. Default: provider default. |
| `temperature`       | no       | Sampling temperature. Default: provider default. |

### Model references

A model is referred to as `<provider>/<model>`, using the names of the config
tables, e.g. `anthropic/opus` or `local/qwen`. This is the form used by
`default_model`, `--model`, `/model`, `clai ls` and the TUI. If only one
provider defines a model with that name, the provider part may be left out
(`--model opus`). **(assumption)**

## Validation

The configuration is checked when it is loaded. Errors name the file, line and
key, e.g. `config.toml:14: providers.local.models.qwen: missing context_window`.

It is an error when:

- no provider or no model is defined;
- `default_model` is missing or does not name a defined model;
- a provider has an unknown `api`, or both `api_key_env` and `api_key_cmd`;
- an `openai-compatible` provider has no `base_url`.

A missing API key (variable not set, command fails) is not a load error; it
is reported when the provider is first used, in the session that uses it, so
other providers still work.

If there is no user config file at all, `clai` explains where to create it
and prints an example, then exits with code `2`.

## Reloading

The server reads the configuration when it starts and again when the user
file changes. **(assumption)**

- A valid new configuration applies to new sessions and to the model pickers
  at once.
- An invalid new configuration is rejected; the server keeps the previous one
  and shows the errors in all attached TUIs.
- A running session keeps its provider and model even if they were removed or
  changed in the configuration; its next model change must pick a model that
  exists. The TUI marks such a model as "not in config".
- The project file is read when a session is created in that workspace.
