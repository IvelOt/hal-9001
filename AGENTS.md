# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

- Add durable project-specific notes here as they are discovered through real work.

## i18n (pt-BR / en-US / es-ES)

All user-facing text lives in `src/i18n.rs` (`Messages` struct + `MESSAGES_PT_BR` / `MESSAGES_EN_US` / `MESSAGES_ES_ES`). Never hardcode literal UI/toast strings elsewhere.
- Sync UI code (has `App`): use `app.lang.messages()` (or a `let m = ...` alias) and reference `m.<field>`.
- Async backend tasks (`src/backend/*.rs`) don't have `App`; they receive a `SharedLang` (cheap `Arc<AtomicU8>` clone, see `spawn_all` in `backend/mod.rs`) and call `.get()` / `.messages()` per-message so a live language switch in the config modal is picked up without restarting the task.
- Dynamic messages with a placeholder (e.g. counts) are stored as literal `{n}`/`{name}` tokens in the message string and filled with `.replace("{n}", &n.to_string())` rather than `format!` on a runtime string.
- `tests/i18n.rs` asserts new fields are non-empty and meaningfully different across languages (guards against a language silently falling back to pt-BR) — add new keys there when extending coverage.

## Maintaining this file

Keep this file for knowledge useful to almost every future agent session in this project.
Do not repeat what the codebase already shows; point to the authoritative file or command instead.
Prefer rewriting or pruning existing entries over appending new ones.
When updating this file, preserve this bar for all agents and keep entries concise.
