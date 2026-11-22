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
make build      # -> target/release/Voice Assistant.app
make install    # copy it to /Applications
make open       # or build and launch it in place
```

Run `make help` to list every target.

On first launch:

1. Open **Settings…** from the menu-bar icon and click **Download default model** (whisper `ggml-small`, about 470 MB, stored in `~/Library/Application Support/VoiceAssistant/models/`).
2. Grant **Microphone**, **Accessibility** and **Input Monitoring** access when asked. You can also use the buttons in Settings. Builds are signed ad hoc by default, which means permissions must be granted again after every rebuild. To keep them, create a self-signed code-signing certificate in Keychain Access and build with `make build CODESIGN_IDENTITY="<certificate name>"`.
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
make check   # fmt check + clippy -D warnings + all tests
make demo    # UI with simulated dictation, no permissions needed
make run     # run from source with logs
```

Layout: `crates/voice-core` holds the business rules and ports. Each `crates/voice-*` crate is one adapter, and `app` is the egui menu-bar app and composition root.
