# Local Dictation — user help

This is the user-facing help that docs/10 §5–§6 asks for. It describes the current build. Platforms other than macOS are covered in `docs/PLATFORM_SUPPORT.md`.

## Getting started

1. Open **Local Dictation** from Applications. It runs in the menu bar and has no Dock icon.
2. Setup opens on first launch. It walks you through the microphone, a speech model, the shortcut, automatic paste, and a trial.
3. Click into any text field, press the shortcut (default **Ctrl+Option+Space**), speak, and pause. When you stop talking, the text is pasted into the field you started in.

## Choosing a microphone

Go to **Settings → Recording → Microphone**. *System default* follows whichever device macOS uses when you start dictating.

If the microphone is unplugged during dictation, recording stops with an error. The app never switches to another microphone by itself. Reconnect the device or pick another one.

Use **Test microphone** to see the input level. The test turns the microphone off as soon as you leave the page.

## Language and model

- **Settings → Models → Language.** *Detect automatically* works for mixed use. Choosing English, German or Indonesian is faster and more accurate for short phrases.
- **Models.**
  - **Base** (148 MB) is the default.
  - **Small** (488 MB) is more accurate but slower.
  - Models download from Hugging Face only when you ask, and are verified before use.
  - Without internet, use **Import** to pick a `ggml-base.bin` or `ggml-small.bin` you already have. Its checksum must match the approved file.
- **Unload** frees the model's memory. The next dictation fails until you activate a model again.

## Silence and long dictation

- Dictation ends after **1.0 s** of silence. You can change this from 0.5 to 2.5 s under **Settings → Recording**. Raise it if you pause mid-sentence.
- Pressing the shortcut again finishes dictation immediately.
- **Cancel** (default **Ctrl+Option+Escape**) discards the recording without transcribing it.
- One dictation is capped at **120 s**.
- **Continuous dictation** (Settings → Recording) keeps listening after each phrase: every pause pastes what you just said into the same field, and the microphone stays on. Press the shortcut again to stop, or stay silent for the "give up" time. If you switch to another field or app, the current phrase becomes a draft and listening stops. It needs automatic paste, and in this mode the 120 s cap applies to each phrase.

## Permissions (macOS)

| Permission | Why | Where to change it |
|---|---|---|
| Microphone | Recording while you dictate | System Settings → Privacy & Security → Microphone |
| Accessibility | Checking that the same text field is still focused, and sending one ⌘V | System Settings → Privacy & Security → Accessibility |

- Without Accessibility, the app still works in **copy-only** mode: text appears in the review window, and you copy it yourself.
- After changing a permission in System Settings, come back to the app and press **Check again**.
- If you deny a permission and later want it back, enable it in System Settings. macOS does not show the prompt twice.
- **After updating the app, grant Accessibility again.** Releases are not notarized yet, so macOS treats each new version as a different app. The old entry can still look switched on but no longer applies, and dictation falls back to copy-only. In System Settings → Privacy & Security → Accessibility, select Local Dictation, remove it with **−**, then add it again with **+** (or run `tccutil reset Accessibility app.localdictation.m0` and grant it when asked).
- Local Dictation never asks for Screen Recording, Input Monitoring or Automation.

## Shortcut conflicts

The app saves a shortcut only after macOS accepts it. If another app owns the chord, you see "Shortcut is in use". Choose a different chord under **Settings → Shortcuts**.

Plain letters, and Shift with a single letter, are not allowed. They would trigger while you type.

## When the app does not paste

Instead of pasting, the app keeps your text as a **draft** and tells you why:

- **You switched app, window or text field** before transcription finished. The app pastes only into the field where you started.
- **Password fields, terminals, and apps you excluded** never receive automatic paste. Terminals are excluded because a pasted line break can run a command.
- **You were still holding the shortcut keys** 1.5 s after the text was ready. The app never releases keys you are holding.
- **The app could not identify the text field.** Some apps don't expose it. Use **Copy** and paste yourself.

Open the draft from the menu bar with **Review text…**. From there you can edit it, copy it, or paste it into the field that is focused now with the paste-draft shortcut, if you set one. Drafts live only in memory and expire after 10 minutes.

## Troubleshooting

| Symptom | What to do |
|---|---|
| "Paste sent" but no text appeared | The app sent ⌘V once, but it cannot see whether the other app accepted it. Check the target app first. The text is still on the clipboard, so press ⌘V yourself. The app never resends automatically. |
| Transcription seems stuck | Press cancel. The speech engine is stopped and restarted, and the app is ready again in a moment. |
| "Clipboard busy" | Another app held the clipboard. Your text is kept as a draft; use **Copy** again. |
| "Model needs attention" | The model file changed or is damaged. Under **Settings → Models**, choose **Verify**, or remove and reinstall the model. |
| Nothing happens on the shortcut | Check the menu bar tooltip. If it says *Setup needed*, finish setup. Otherwise check for a shortcut conflict. |
| Text appears in the wrong language | Set the language explicitly. Auto-detection on very short phrases is unreliable. |

### Support report

**Settings → About → Support report → Preview report**, then **Save report…**. The report contains versions, settings values, permission and error codes, and timing summaries. It never contains what you said, audio, app or window names, device names or file paths. Nothing is uploaded; you choose where to save it and whom to send it to.

## Privacy

- The microphone is open only while you dictate or run the microphone test.
- Audio stays in memory and is never written to disk.
- Transcription happens on this computer. The only network traffic is a model download you start yourself.
- There is no history, no analytics, and no log of what you said.
- Unsent text (a draft) lives in memory for up to 10 minutes. It is cleared when you quit, put the Mac to sleep, lock the screen, switch user or log out. It is never pasted after wake.
- **The clipboard is not private.** Pasted or copied text stays on the system clipboard. There, clipboard managers, Universal Clipboard and other apps can read it. The app cannot remove it from clipboard history or sync.

## Uninstalling

1. Quit Local Dictation from the menu bar.
2. Drag **Local Dictation.app** from Applications to the Bin.
3. Optional: remove settings and models by deleting `~/Library/Application Support/app.localdictation.m0/`. It holds `settings.json` and the `models/` folder, which can be hundreds of MB.
4. If you turned on *Launch at login*, turn it off before uninstalling, or delete the Local Dictation entry in `~/Library/LaunchAgents/`.
5. Optional: remove the permission entries under System Settings → Privacy & Security → Microphone and Accessibility.

Uninstalling cannot delete copies of dictated text that macOS or other apps keep: clipboard history, sync, system logs or swap. *Clear draft* in the app only affects the app's own in-memory draft.
