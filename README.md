# sui

Coding agent — 粋・推・遂. A lightweight coding agent with a workflow engine and memory features.

## Crates

| Crate | Role |
| --- | --- |
| `sui` | Ratatui TUI binary |
| `sui-app` | Event loop, prompt, slash commands |
| `sui-agent` | Tool-calling turn loop |
| `sui-llm` | OpenAI-compatible Chat Completions / Responses client |
| `sui-tools` | `code_search`, `edit`, `bash` |
| `sui-widget` | Prompt widget |
| `sui-theme` | Colour palettes from `config.toml` |
| `sui-workflow` | Deterministic Rhai workflows |

## LLM connection

Write `~/.config/sui/config.toml` (or set `$SUI_CONFIG`):

```toml
[llm]
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = "sk-..."
api_mode = "chat_completions"
```

Alternatively set `SUI_LLM_BASE_URL`, `SUI_LLM_MODEL`, and optional `SUI_LLM_API_KEY`.

Named models use `[[model."name"]]` sections and `/model <name>` in the TUI.

## Run

```bash
cargo run -p sui
```

Default mode is prompt chat. `!` on an empty prompt enters one-shot shell. `/` opens slash commands.

When tools are attached, the agent prefers `code_search` to inspect the workspace and `edit` (Git unified diffs) to change files.
