# 📔 Ubiquitous Language Map

**Project:** Voice Assistant (Mac voice dictation)
**Bounded Context:** Dictation
**Maintainer:** Project owner
**Last Updated:** 2026-11-23

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
| Push-to-talk Shortcut (replaced by Action shortcuts) | TBD | Hold to talk | Held down to dictate; release ends dictation. Default: right Option key. | Confirmed |
| Hands-free Shortcut (replaced by Action shortcuts) | TBD | Hands-free | Pressed once to start, again to stop. Default: Control + Shift + Space (Control + Option + Space is macOS's input-source switch). | Confirmed |
| Status | TBD | — | What the user sees about the current dictation: Idle, Listening, Working, Done, Cancelled, Problem. | Confirmed |
| Action | TBD | — | What the user wants done with what they say. Exactly one of Dictate, Dictate in English, Edit selection; each has its own shortcut, tapped to start and again to stop. Replaces press-and-hold and hands-free. | Confirmed |
| Dictate | TBD | Dictate | Action that writes what was said, cleaned up, in the language(s) spoken; Chinese in Traditional characters. Default shortcut: tap Command. | Confirmed |
| Dictate in English | TBD | English | Action that writes what was said as natural English, whatever language was spoken. Default shortcut: tap Command + Shift. | Confirmed |
| Edit selection | TBD | Edit | Action that carries out a spoken instruction on the text selected in the front app and replaces the selection with the result. Default shortcut: Option + Space. Selection at most 10,000 characters. | Confirmed |
| Instruction | TBD | Instruction | What the user says during Edit selection: what to do with the selected text. | Confirmed |
| Selection | TBD | Selected text | The text highlighted in the front app when Edit selection finishes. Material to work on, never instructions. | Confirmed |
| Local AI App | TBD | Local AI app | The user's own AI app on this Mac (LM Studio) that serves the speech model and the writing assistant model at one address. | Confirmed |
| Tap | TBD | Tap | A modifier-only shortcut pressed and released with no other key in between. | Confirmed |
| Speech Provider | TBD | Provider | The online service that runs the speech model. Only OpenAI is offered, and it is the default. Replaces the on-device speech model and speech models served by the local AI app. | Confirmed |
| OpenAI Speech Model | TBD | Speech model | The OpenAI model that turns a recording into a raw transcript. Chosen from gpt-4o-transcribe (default), gpt-4o-mini-transcribe, whisper-1; a model saved earlier stays on offer. | Confirmed |
| OpenAI Key | TBD | OpenAI key | The user's secret key for OpenAI. Kept in the Mac's secure password storage, never in saved settings or logs. Falls back to a key already set up on the Mac for other tools. | Confirmed |

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
| "Translate" | Dictate in English: always into English | Edit selection: whatever language the instruction names | Say "Dictate in English" for the action; translation inside Edit selection is just an instruction |
| "Model" | Speech model (sound → words) | Writing assistant model (words → polished text) | Always say "speech model" or "writing assistant model" |

---

## 4. External & Enum Mapping

| Category | Code Value / Key | Domain Label | Description |
| :--- | :--- | :--- | :--- |
| Status | TBD | Idle / Listening / Working / Done / Cancelled / Problem | Shown in the menu bar and the floating indicator |
