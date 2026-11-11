# kadr

screenshots, without the extra stuff.

a small windows utility by lezneve. press a hotkey, select, paste or find the file. nothing else runs, renders or listens while you are not using it.

**version 0.1.0**, the first release. windows 10/11, x64. free and open source (MIT). download: [releases](https://github.com/lezneve/kadr/releases).

## what it does

| hotkey | captures |
|---|---|
| `printscreen` | a region you select |
| `alt+printscreen` | the active window |
| `ctrl+shift+printscreen` | the monitor under the mouse cursor |

each capture opens in the editor, at the place it was taken, 1:1. nothing is copied or saved until you say so:

- `ctrl+c` copies it and closes the editor (no file is written)
- `ctrl+s` saves it as a png in windows' screenshots folder (`pictures\screenshots`; the clipboard is left alone); a second `ctrl+s` without changes writes nothing new
- `esc` closes it without exporting

in the region overlay `ctrl+a` takes the whole monitor. while the editor is open, the rest of the screen stays darkened.

in the editor you can draw on the capture: select (v), pen (p), line (l), rectangle (r), ellipse (e), highlighter (h), blur (b) and redact (x). shift makes squares and circles and keeps lines at 45°. with select you move a shape, drag a corner to resize it, and delete it with delete. `ctrl+z` / `ctrl+y` undo and redo.

the dot on the toolbar opens a small style panel: seven colours (also keys 1–7), any colour through the windows colour dialog, an eyedropper that takes a colour from the capture (key i), and a width from 1 to 32 px with a preview at its real size (also `[` and `]`, or the mouse wheel over the toolbar). strokes (pen, line, rectangle, ellipse), the highlighter and redact each keep their own colour and width; blur has none. with a shape selected, the panel changes that shape. the last colours and widths are remembered in `%APPDATA%\kadr\editor.ini`.

blur only makes text hard to read; to really hide something, use redact, an opaque fill.

the editor stays open after saving (after copying too, with `close_after_copy = no` in settings.ini). the close button and esc close it at once, without a question: what is not copied or saved is gone. its small toolbar sits next to the capture and can be dragged; it stays inside the screen. every capture gets its own editor, and each one is gone, memory and all, when you close it.

the quick modes are still there (settings › after capture): copy, save, or both right away, without the editor. nothing is overwritten: a second capture in the same second gets `_2`.

in the region overlay: drag to select, release to capture. esc, a right click or switching away cancels. a click without dragging does nothing.

the tray icon: a click captures a region; right click for the menu (capture region, capture screen, open folder, settings, quit).

on windows 11 snipping tool may take the printscreen key ("use the print screen key to open screen capture"). kadr then asks once whether to turn that windows setting off for you. only after "yes" it changes that one setting for your account; snipping tool stays installed and win+shift+s still opens it. uninstalling kadr turns the setting back on. if you say no, choose another hotkey in settings, or set printscreen again there to be asked again.

on the test machine (with that windows setting on) a second app could register bare printscreen while kadr held it, so kadr may not be able to tell when another screenshot tool already uses the key. if printscreen opens another tool, close that tool or pick another hotkey.

## settings

tray → settings. changes apply immediately; there is nothing to save.

- hotkeys for the three captures (click a row, press the new combination; esc keeps the old one; backspace turns it off)
- send to: editor (default), clipboard, file, or both
- folder, a subfolder per month or day (`2026-11`, `2026-11-12`), and the file name pattern (`%Y %m %d %H %M %S`)
- start with windows (off by default)

settings live in `%APPDATA%\kadr\settings.ini`, a plain text file you can also edit by hand; kadr picks up changes when you save it.

## install

download `kadr-0.1.0-setup.exe` from [releases](https://github.com/lezneve/kadr/releases) and run it. it installs kadr for you only, in `%LOCALAPPDATA%\Programs\kadr`, with a start menu entry and an entry in windows settings › apps. no administrator rights. two switches: start with windows (off by default) and start kadr now (on). running it again updates kadr and keeps your settings.

portable use works too: copy `kadr.exe` anywhere and run it.

requirements: windows 10 1703 or later, or windows 11, x64. tested on windows 11 only (see limitations).

## uninstall

windows settings › apps › kadr › uninstall. it quits kadr and removes the program, the start menu entry and start with windows. a second question asks whether to remove your settings too (`%APPDATA%\kadr`); the default is no, so a later install finds them again. your captures in `pictures\kadr` always stay.

to remove its own folder after it exits, the uninstaller runs a small copy of itself from `%TEMP%` (`kadr-uninstall-<n>.exe`), which windows' temp clean-up removes later.

portable copy: tray → quit, turn off start with windows first if you turned it on, then delete `kadr.exe` and, if you like, `%APPDATA%\kadr`.

## installer file name

only a file named `kadr-setup.exe` or `kadr-<version>-setup.exe` (also with the ` (2)` a browser adds to a second download) opens the installer. any other name, for example `kadr-portable.exe`, runs kadr itself.

## privacy

kadr works fully offline: no network access, no telemetry, no update checks, no logs of what you capture. captures exist only where you send them (clipboard, file). screenshots on the clipboard are marked not to be uploaded to windows' cloud clipboard; local clipboard history (win+v) treats them like any other image.

## resource use

measured on one machine (windows 11, ryzen 7 8845hs, 2560×1600 at 150 %), release build; details in [docs/measurements.md](docs/measurements.md):

| | |
|---|---|
| idle | 1 process, ~2 mb private memory, 0 % cpu |
| ready after start | ~27 ms |
| hotkey → screen captured | ~32 ms (p95 39 ms) |
| hotkey → region overlay on screen | ~61 ms (p95 69 ms) |
| settings window | own process, ~11 mb while open, nothing left after closing |
| kadr.exe (also the installer) | ~0.4 mb |

these are this machine's numbers, not guarantees.

## limitations

not verified, so not claimed to work:

- **multiple monitors**, monitors left of or above the primary (negative coordinates)
- **different scaling per monitor**, and moving the settings window between them
- **hdr displays** (gdi capture on hdr is reported to give wrong colours)
- 4k and several 4k monitors (memory and overlay speed scale with screen size)
- windows 10
- real keyboard and mouse input in the final build: needs the manual checks in [docs/testing.md](docs/testing.md) and [docs/overlay-check.md](docs/overlay-check.md)

known:

- the files are not code-signed yet: windows smartscreen warns on first start ("more info" → "run anyway"), and smart app control (when on) blocks kadr. check the sha-256 next to each file. see [docs/smartscreen.md](docs/smartscreen.md)
- on some pcs bare printscreen never reaches kadr even after the snipping tool setting is off (seen on one laptop; a vendor key utility is the likely cause). use another region hotkey in settings, e.g. `ctrl+printscreen`; `tools\printscreen-diag.ps1` shows where the key goes

- the tray menu and the file name field's context menu stay light in windows dark mode (windows has no documented way to theme them)
- protected content (drm video) captures as black
- with uac turned off, a kadr started with reduced rights cannot add its tray icon (it says so)

## build from source

```powershell
cargo build --release          # target\release\kadr.exe
cargo test
powershell -NoProfile -ExecutionPolicy Bypass -File tests\smoke.ps1
```

needs rust (stable, msvc toolchain). one dependency: `windows` (microsoft's windows-rs). the test scripts never press keys or move the mouse, except `tests\input.ps1`, which is meant to be run by a person.

## documentation

- [docs/architecture.md](docs/architecture.md) — decisions and why
- [docs/measurements.md](docs/measurements.md) — how and what was measured
- [docs/testing.md](docs/testing.md) — what is verified and what is not
- [docs/overlay-check.md](docs/overlay-check.md) — manual overlay check
- [docs/incidents.md](docs/incidents.md) — what went wrong during development
- [DESIGN.md](DESIGN.md) — the visual system

## license

MIT, see [LICENSE](LICENSE).
