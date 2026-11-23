# Voice Assistant — working rules

Mac voice dictation (Typeless-like). Rust workspace, egui/eframe UI, OpenAI speech-to-text (key in the Keychain), LM Studio for text polishing.
Design: `.sdd/2026-11-22-voice-dictation/ARCH.md` (ports & adapters). Requirements: `PRD.md` in the same folder.

## Layout
- `crates/voice-core` — business rules + ports (traits). No platform, network or file I/O.
- `crates/voice-{audio,stt,polish,storage,macos}` — one adapter crate per port.
- `app` — eframe binary; wiring only, no business rules.

## Rules
- Never change the public API in `voice-core` from an adapter branch; if a port must change, say so in the PR.
- Every crate: `cargo clippy --all-targets -- -D warnings` and `cargo test` must pass; `cargo fmt`.
- Tests live next to code (`#[cfg(test)]`) or in `tests/`; fake adapters instead of real devices/network.
- No `unwrap()`/`expect()` in non-test code except for documented invariants.
- Commits: Conventional Commits in English, one commit per completed slice or integration.
- Never put spec identifiers (story ids like `US-01`, scenario numbers, document names such as PRD/ARCH/BRIEF) in code, comments, test names or commit messages.
