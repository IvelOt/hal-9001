# Project agent memory

This file is the project's committed home for project-intrinsic agent knowledge: build, test, release, architecture, and sharp-edge notes that should travel with the code.

- Add durable project-specific notes here as they are discovered through real work.

## i18n (pt-BR / en-US / es-ES)

All user-facing text lives in `src/i18n.rs` (`Messages` struct + `MESSAGES_PT_BR` / `MESSAGES_EN_US` / `MESSAGES_ES_ES`). Never hardcode literal UI/toast strings elsewhere.
- Sync UI code (has `App`): use `app.lang.messages()` (or a `let m = ...` alias) and reference `m.<field>`.
- Async backend tasks (`src/backend/*.rs`) don't have `App`; they receive a `SharedLang` (cheap `Arc<AtomicU8>` clone, see `spawn_all` in `backend/mod.rs`) and call `.get()` / `.messages()` per-message so a live language switch in the config modal is picked up without restarting the task.
- Dynamic messages with a placeholder (e.g. counts) are stored as literal `{n}`/`{name}` tokens in the message string and filled with `.replace("{n}", &n.to_string())` rather than `format!` on a runtime string.
- `tests/i18n.rs` asserts new fields are non-empty and meaningfully different across languages (guards against a language silently falling back to pt-BR) — add new keys there when extending coverage.

## Multi-boot image intelligence (pure Rust)

Provisioning smarts live at *add time*, not boot time: `multiboot_add_iso_task` (`storage.rs`) runs `image_probe::inspect`, classifies the image, and writes a per-image `ISOs/<stem>.cfg` GRUB fragment next to the ISO.
- `image_probe.rs` — magic-byte format detection (`probe_format`) + OS classification (`inspect`). ISO9660/UDF signature is read at byte offset `32768` (2048-byte logical sector 16), *not* `512*16`.
- `iso_reader.rs` — pure ISO9660/Joliet reader (`open`/`list_paths`/`find`/`extract`); multi-extent files are merged by consecutive directory records.
- `grub_gen.rs` — deterministic stanzas (`grub_for_linux`/`grub_for_partitioned`/`grub_for_windows`/`generic_fallback`).
- `windows_provision.rs` — native Windows boot-file extraction into a FAT32 mount (UEFI `chainloader` needs a real device handle, never a `(loop)`).
- `assets/multiboot/grub.cfg` is an orchestrator: `ISOs/*.cfg` fragments take precedence; the legacy cascade only runs for images without a generated `.cfg`.
- Synthetic ISO9660 fixtures for tests live in `tests/common/mod.rs` (`build_iso`), plus `build_gpt_fat_disk` for partitioned-image tests.

## Hardware-conditional controls (probe-then-hide pattern)

For a control that only some hardware supports (e.g. `src/backend/power.rs::BatteryBypass`, a battery-bypass/conservation-mode knob): probe real sysfs paths through a `probe_at(root: &Path)` function parameterized over the root directory (defaults to `/` in the public `probe()`), so `tests/*.rs` can point it at a `tempfile::tempdir()` standing in for `/sys/...` without touching the real system. Probe once in `App::new()` and store `Option<T>`; render the key hint/indicator only when `Some`, and thread an availability `bool` through `InputStream::next` / `map_key` (see `src/events/input.rs`) so the keybinding itself doesn't fire on unsupported hosts. Toggling re-probes fresh (cheap) and does the actual write in `tokio::task::spawn_blocking`, falling back from a direct `std::fs::write` to `pkexec tee` / `sudo tee` on `PermissionDenied`.

## Maintaining this file

Keep this file for knowledge useful to almost every future agent session in this project.
Do not repeat what the codebase already shows; point to the authoritative file or command instead.
Prefer rewriting or pruning existing entries over appending new ones.
When updating this file, preserve this bar for all agents and keep entries concise.
