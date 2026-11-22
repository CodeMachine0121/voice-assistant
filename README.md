# Voice Assistant

Private, on-device voice dictation for macOS. Hold a shortcut, speak, and release; cleaned-up text appears at your cursor in any app.

- Speech recognition runs locally with whisper.cpp (Metal).
- Cleanup (removing filler words, applying self-corrections, adding punctuation and lists, using your personal dictionary) is done by **your own** model served by [LM Studio](https://lmstudio.ai).
- Nothing leaves your Mac.

## Requirements

- macOS 13+ on Apple silicon
- Rust (stable), Xcode Command Line Tools, CMake (`brew install cmake`)
- LM Studio with a chat/instruct model loaded and the local server started (default `http://localhost:1234/v1`)

## Build & run

```bash
scripts/bundle.sh release
open "target/release/Voice Assistant.app"
```

On first launch:

1. Open **Settings…** from the menu-bar icon and click **Download default model** (whisper `ggml-small`, about 470 MB, stored in `~/Library/Application Support/VoiceAssistant/models/`).
2. Grant **Microphone**, **Accessibility** and **Input Monitoring** access when asked. You can also use the buttons in Settings. Ad-hoc signed builds must be granted again after every rebuild.
3. Pick your LM Studio model (or leave it empty to use whichever model is loaded).

## Using it

| Action | Default |
| --- | --- |
| Push-to-talk | hold **right Option** |
| Hands-free start/stop | **Control + Shift + Space** |
| Cancel | **Escape** |

Presses shorter than 0.5 s are ignored, and a dictation stops on its own after 5 minutes. If LM Studio is not reachable (or takes longer than 20 s), the raw transcript is inserted. The last 200 dictations are kept under **History…**.

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
VOICE_ASSISTANT_DEMO=1 cargo run -p voice-assistant   # UI with simulated dictation
```

Layout: `crates/voice-core` holds the business rules and ports. Each `crates/voice-*` crate is one adapter, and `app` is the egui menu-bar app and composition root.
