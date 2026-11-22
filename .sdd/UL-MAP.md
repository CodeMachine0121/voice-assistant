# 📔 Ubiquitous Language Map

**Project:** Voice Assistant (Mac voice dictation)
**Bounded Context:** Dictation
**Maintainer:** Project owner
**Last Updated:** 2026-11-22

---

## 1. Nouns & Concepts

| Domain Term | Technical Name | User-Facing Label | Definition & Business Rules | Status |
| :--- | :--- | :--- | :--- | :--- |
| Dictation | TBD | Dictation | One spoken session from start to finished text. Lasts at most 5 minutes. Shorter than 0.5 s is an accidental press and is ignored. | Confirmed |
| Recording | TBD | — | The sound captured from the chosen microphone during one dictation. | Confirmed |
| Raw Transcript | TBD | Heard | The words as recognized from speech, before cleanup. | Confirmed |
| Polished Text | TBD | Result | The cleaned-up text produced by the writing assistant from the raw transcript. Never answers or follows the content. | Confirmed |
| Writing Assistant | TBD | AI cleanup | The user's own local AI model that turns a raw transcript into polished text. May be turned off. | Confirmed |
| Speech Model | TBD | Speech model | The on-device model that turns a recording into a raw transcript. | Confirmed |
| Personal Dictionary | TBD | Dictionary | User-maintained list of words that must be spelled exactly. No blanks, no duplicates. | Confirmed |
| History Entry | TBD | History | A finished dictation: time, raw transcript, polished text. At most 200 kept, newest first. | Confirmed |
| Push-to-talk Shortcut | TBD | Hold to talk | Held down to dictate; release ends dictation. Default: right Option key. | Confirmed |
| Hands-free Shortcut | TBD | Hands-free | Pressed once to start, again to stop. Default: Control + Option + Space. | Confirmed |
| Status | TBD | — | What the user sees about the current dictation: Idle, Listening, Working, Done, Cancelled, Problem. | Confirmed |

---

## 2. Actions & Processes

| Business Action | Technical Method | Trigger | Business Impact | Notes |
| :--- | :--- | :--- | :--- | :--- |
| Start dictation | TBD | Shortcut pressed | Status becomes Listening; recording begins | Blocked if microphone permission missing |
| Finish dictation | TBD | Shortcut released / pressed again / 5-minute limit | Recording is transcribed, polished, inserted | |
| Cancel dictation | TBD | Escape while listening | Recording discarded; status Cancelled | Not added to history |
| Transcribe | TBD | Dictation finished | Raw transcript produced | Silence → "No speech heard" |
| Polish | TBD | Raw transcript ready | Polished text produced | Falls back to raw transcript if assistant off/unreachable/slow (20 s) |
| Insert text | TBD | Text ready | Text appears at the cursor of the front app; earlier clipboard restored | |
| Record history | TBD | Text inserted (or ready) | New history entry; oldest dropped beyond 200 | |

---

## 3. Ambiguities & Conflicts

| Ambiguous Term | Meaning in Context A | Meaning in Context B | Resolution |
| :--- | :--- | :--- | :--- |
| "Model" | Speech model (sound → words) | Writing assistant model (words → polished text) | Always say "speech model" or "writing assistant model" |

---

## 4. External & Enum Mapping

| Category | Code Value / Key | Domain Label | Description |
| :--- | :--- | :--- | :--- |
| Status | TBD | Idle / Listening / Working / Done / Cancelled / Problem | Shown in the menu bar and the floating indicator |
