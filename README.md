<p style="text-align: center; font-weight: bold; font-size: 100pt"><img src="./docs/gothic.webp" alt="Gothic" width="600"><p>

CLI automation for your favorite agentic IDE (Cursor, Trae, Kiro, Codex .etc).

Gothic launches IDE and connects through CDP, submits tasks, tracks task state, and handles human-in-the-loop (HITL) actions automatically.

## Build

```powershell
cargo build --release
```

## Configuration

Generate a config file in the working directory:

```powershell
cargo run -- config init --path .\config.jsonc
```

Validate it:

```powershell
cargo run -- config check --path .\config.jsonc
```

for example:

```json
{
  "$schema": "docs/config-schema.json",
  "trae_executable_path": "C:\\Program Files\\Trae\\Trae.exe",
  "command_strategy": "allow",
  "question_strategy": "auto",
  "max_concurrent_task": 5,
  "max_task_action_retry": 3,
  "task_poll_interval_ms": 2000,
  "logging": {
    "directory": "logs",
    "level": "info"
  },
  "model": {
    "api_key": "",
    "base_url": "",
    "model_name": "gpt-5-mini"
  }
}
```

## Usage

Run tasks and listen for coming events:

```powershell
gothic run trae --task "Task A" --task "Task B"
```

## Config Fields

| Field | Description |
| --- | --- |
| `trae_executable_path` | absolute path to `Trae.exe` |
| `command_strategy` | `allow`, `deny`, or `llm` |
| `question_strategy` | `skip`, `auto`, or `llm` |
| `max_concurrent_task` | reserved for future scheduling |
| `max_task_action_retry` | retry limit for automated task actions |
| `task_poll_interval_ms` | task polling interval |
| `logging.directory` | log directory |
| `logging.level` | default tracing filter |
| `model.api_key` | API key for OpenAI-compatible endpoints |
| `model.base_url` | base URL for OpenAI-compatible endpoints |
| `model.model_name` | model used for `question_strategy = "llm"` |

## License

MIT
