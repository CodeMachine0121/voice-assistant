# OpenAI Speech Provider — Requirements Brief

## Goal
The app uses AI in two ways: a **speech model** that turns what the user said into words, and a **writing assistant model** that cleans up, translates or edits those words. This change moves speech recognition to the OpenAI service. In Settings the speech model picker is replaced by a **Provider** choice whose only option (and default) is OpenAI, plus a choice of OpenAI speech model and a place to enter the user's OpenAI key. The speech model built into the app and the speech model served by the local AI app are removed. The writing assistant keeps running in the user's local AI app exactly as today.

## Requirements
- Speech is always turned into words by OpenAI. There is no other speech provider.
- Settings shows a **Provider** choice for speech. It offers exactly one option, OpenAI, which is selected by default.
- Settings shows the **OpenAI speech model** to use, chosen from a short list: "gpt-4o-transcribe" (default), "gpt-4o-mini-transcribe" and "whisper-1". A model the user saved earlier that is not on the list stays on offer.
- Settings has a field for the user's **OpenAI key**. What is typed is hidden. The key is kept in the Mac's secure password storage, never in the app's settings, and never appears in logs or messages. Settings says whether a key is saved, and the user can remove it.
- If no key is saved in Settings, the app uses an OpenAI key the user has already set up on their Mac for other tools, if there is one. A key saved in Settings wins over that one.
- Settings tells the user that recordings are sent to OpenAI to be turned into words.
- The speech model built into the app is gone: no model file location, no model download, no "Built-in" choice.
- Speech models served by the local AI app are gone from the speech choice. The local AI app address and the writing assistant model stay, and still drive cleanup, translation and Edit selection.
- Settings saved by earlier versions still load. Whatever speech choice was saved (built-in, or a local AI app speech model) becomes OpenAI with the default speech model; everything else the user saved is kept.
- Everything after the words are heard keeps working unchanged in all three actions: the personal dictionary is sent along to help spelling, Chinese is written in Traditional characters, AI cleanup, translation and Edit selection behave as before.
- Silence is still detected on the Mac, so a silent recording is never sent to OpenAI.
- When speech cannot be turned into words, nothing is written, nothing is added to History, and the user sees a clear message:
  - no key available → told to add their OpenAI key in Settings;
  - OpenAI rejects the key → told the key was rejected and to check it in Settings;
  - OpenAI says too many requests or the account is out of credit → told to try again later or check their OpenAI account;
  - no internet, OpenAI not answering, OpenAI having trouble, or no answer within one minute → told OpenAI could not be reached and to check the internet connection;
  - any other refusal or unusable answer → told OpenAI could not turn the recording into words.
- The app never crashes because a key is missing or OpenAI fails.

## Examples (Specification by Example)
Each example lists **only** the data that affects the behavior — nothing more.

### Rule: Speech is turned into words by OpenAI with the chosen speech model
| # | Given (only relevant data) | When | Then |
|---|---|---|---|
| 1 (happy) | Key saved; speech model "gpt-4o-transcribe" | User dictates "我們明天下午三點開會" | OpenAI hears it; "我們明天下午三點開會。" appears at the cursor |
| 2 (happy) | Speech model changed to "gpt-4o-mini-transcribe" and saved | User dictates | The next recording is heard by "gpt-4o-mini-transcribe" |
| 3 (boundary) | OpenAI returns the words in Simplified characters "我们开会" | Dictate finishes | "我們開會。" — still Traditional |
| 4 (boundary) | Dictionary contains "Cafler" | User says "I work at Cafler" | The dictionary words are sent to help OpenAI spell "Cafler" |
| 5 (boundary) | Recording is silent | User stops | "No speech heard"; nothing is sent to OpenAI |
| 6 (boundary) | Recording lasts the full 5 minutes | Limit reached | The whole recording is sent and transcribed normally |
| 7 (boundary) | Speech model saved earlier as "my-custom-model" (not on the list) | User opens Settings | "my-custom-model" is still shown as the choice and kept on save |

### Rule: Settings offers only OpenAI as the speech provider
| # | Given | When | Then |
|---|---|---|---|
| 1 (happy) | First launch | User opens Settings | Provider shows "OpenAI"; speech model shows "gpt-4o-transcribe" |
| 2 (boundary) | — | User opens the Provider choice | "OpenAI" is the only option |
| 3 (boundary) | — | User looks for the built-in model location or download | They no longer exist |
| 4 (happy) | Local AI app address and writing assistant model set | User dictates with AI cleanup on | Cleanup still runs in the local AI app |

### Rule: The OpenAI key is kept secret and has a fallback
| # | Given | When | Then |
|---|---|---|---|
| 1 (happy) | No key yet | User types a key and saves | Key is kept in the Mac's secure password storage; Settings says a key is saved; the app's saved settings do not contain it |
| 2 (happy) | Key saved | User opens Settings again | The key itself is not shown, only that one is saved |
| 3 (boundary) | No key in Settings; a key already set up on the Mac for other tools | User dictates | That key is used |
| 4 (boundary) | Key in Settings and a different key set up on the Mac for other tools | User dictates | The Settings key is used |
| 5 (boundary) | Key saved | User removes it in Settings | Settings says no key is saved; the key set up for other tools (if any) is used from then on |
| 6 (exception) | No key in Settings, none set up on the Mac | User dictates | Nothing written; message tells them to add their OpenAI key in Settings |
| 7 (boundary) | Key field left empty | User saves other changes | The saved key is left as it was |

### Rule: OpenAI problems become clear messages, never crashes
| # | Given | When | Then |
|---|---|---|---|
| 1 (exception) | Key is wrong or revoked | User dictates | Nothing written; message: key rejected, check it in Settings |
| 2 (exception) | OpenAI says too many requests / out of credit | User dictates | Nothing written; message: try again later or check the OpenAI account |
| 3 (exception) | No internet | User dictates | Nothing written; message: OpenAI not reachable, check the internet connection |
| 4 (boundary) | OpenAI answers after 59 seconds | User dictates | Words are written normally |
| 5 (exception) | OpenAI has not answered after one minute | User dictates | Nothing written; message: OpenAI not reachable |
| 6 (exception) | OpenAI reports it is having trouble | User dictates | Nothing written; message: OpenAI not reachable |
| 7 (exception) | OpenAI refuses the recording for another reason (e.g. unknown speech model) | User dictates | Nothing written; message: OpenAI could not turn the recording into words |

### Rule: Settings from earlier versions move to OpenAI
| # | Given | When | Then |
|---|---|---|---|
| 1 (happy) | Saved speech choice: built-in, with a model file location | App starts | Speech provider is OpenAI with "gpt-4o-transcribe"; shortcuts, microphone, dictionary, writing assistant kept |
| 2 (happy) | Saved speech choice: local AI app model "breeze-asr-25" | App starts | Speech provider is OpenAI with "gpt-4o-transcribe"; writing assistant model kept |
| 3 (boundary) | No speech choice saved at all | App starts | OpenAI with "gpt-4o-transcribe" |

## Out of Scope
- Any change to the writing assistant (cleanup, translation, Edit selection) or to the local AI app address and model choice.
- Other speech providers, and turning speech into words without internet.
- Deleting a speech model file the user downloaded earlier (it is left where it is; the user may delete it).
- A "test my key" button, usage or cost display, and spoken-language selection (OpenAI detects the language itself, which keeps mixed Chinese/English working).
- Demo mode keeps its simulated speech.

## Open Decisions
Items the PRD author should resolve:
- Exact wording of each user-facing message.
- Whether saving an empty key field should ever remove the saved key (proposed: no — removal is its own button).

## Context / Background
- Decisions from the user: remove the built-in speech model entirely; OpenAI is the only provider; default speech model "gpt-4o-transcribe", user-changeable; key in the Mac's secure password storage, falling back to a key already set up on the Mac; missing key gives a message, not a crash.
- Remaining questions were answered by the implementing agent using best practice: short model list rather than free text (avoids typos, keeps a custom saved value); Settings key wins over a key set up for other tools; no spoken-language hint, so mixed-language dictation keeps working; Settings states that recordings leave the Mac.
- Jev sized this as large (1.00). Chosen stages: clarify → prd → architecture → implement (code-first). Improvement and contract checks are run separately by other agents.
- The project description ("nothing leaves your Mac") no longer holds for speech and must be updated.
