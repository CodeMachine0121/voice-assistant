# Voice Assistant

Voice dictation for macOS. Tap a shortcut, speak, and tap it again; cleaned-up text appears at your cursor in any app.

- Speech recognition runs on OpenAI (`gpt-4o-transcribe` by default; `gpt-4o-mini-transcribe` and `whisper-1` can be chosen in Settings). **Recordings are sent to OpenAI** and billed to your OpenAI account.
- Cleanup, translation and Edit selection (removing filler words, applying self-corrections, adding punctuation and lists, using your personal dictionary) are done by **your own** model served by [LM Studio](https://lmstudio.ai). Selected text and heard words stay on your Mac.

## Requirements

- macOS 13+ on Apple silicon
- Rust (stable), Xcode Command Line Tools
- An OpenAI API key
- LM Studio with a chat/instruct model loaded and the local server started (default `http://localhost:1234/v1`)

## Build & run

```bash
make build      # -> target/release/Voice Assistant.app
make install    # copy it to /Applications
make open       # or build and launch it in place
```

Run `make help` to list every target.

On first launch:

1. Open **Settings…** from the menu-bar icon and enter your **OpenAI key** under *AI models* (it is kept in your login Keychain, never in the settings file). If no key is saved there, the `OPENAI_API_KEY` environment variable is used — but only when the app is started from a terminal (for example `make run`); an app opened from Finder or the Dock does not see your shell's variables, so saving the key in Settings is the way to go. macOS may ask to allow Keychain access, again after rebuilds that are signed ad hoc. A speech model downloaded by an earlier version (`~/Library/Application Support/VoiceAssistant/models/`) is no longer used and can be deleted.
2. Grant **Microphone**, **Accessibility** and **Input Monitoring** access when asked. You can also use the buttons in Settings. Builds are signed ad hoc by default, which means permissions must be granted again after every rebuild. To keep them, create a self-signed code-signing certificate in Keychain Access and put `CODESIGN_IDENTITY := <certificate name>` in a git-ignored `.make.local` file (or pass `CODESIGN_IDENTITY="<certificate name>"` to every `make build` / `make install`).
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
