# changelog

## 0.1.2 — 2026-11-18

- the editor's frame can be moved and resized after the capture: in the select tool (v) drag an edge or corner (also just outside the image) to resize, drag empty space inside the image to move it. the screen is frozen at the moment of the capture and the frame moves over those pixels, so nothing that appears on the screen later gets into the picture. moving the frame is an edit: undo and redo bring it back, and ctrl+s saves the new frame. the rest of the screen is the frozen capture, darkened
- win+arrow, win+shift+arrow and snap no longer move or resize the editor: it is anchored to the frozen screen; the frame moves with select instead
- highlighter has two modes: the stroke as before, or a translucent filled rectangle. press h again (or the button again) to switch; the choice is remembered

## 0.1.1 — 2026-11-13

- the editor keeps the selection's frame: the same 1 px white border and crop marks as the region overlay stay around the image, so its edges are clear over a dark screen ([#1](https://github.com/lezneve/kadr/issues/1)). the frame is outside the image and never in the copied or saved picture
- ctrl+s saves and closes the editor, like ctrl+c copies and closes; a short graphite note ("saved · file name", "copied") shows for a moment where the capture was. `close_after_save = no` keeps the editor open
- arrow tool (a): a line with a filled head, in the stroke colour and width; shift keeps 45°
- mouse pointer tool (m): puts the pointer from the moment of the capture on the image, where it was; a click or select moves it. the standard arrow when the pointer was hidden; the text i-beam is drawn black with a white edge
- region overlay: hold ctrl while dragging to move the selection as it is
- `%P` in the file name: the program you captured (`claude_2026-11-13_…`, `chrome_…`); `kadr` when unknown

## 0.1.0 — 2026-11-12

the first release.

- every capture opens in a small editor, in place and 1:1, over a darkened screen; nothing is copied or saved until you say so
- ctrl+c copies and closes, ctrl+s saves (a repeated ctrl+s without changes writes nothing new), esc closes
- tools: select (move, resize, delete), pen, line, rectangle, ellipse, highlighter, blur, redact; shift for squares, circles and 45° lines; undo and redo
- colour and width: seven presets, the windows colour dialog, an eyedropper from the capture, 1–32 px with a real-size preview; strokes, highlighter and redact keep their own; remembered between captures
- region overlay: ctrl+a takes the whole monitor
- bare printscreen by default; if windows gives it to snipping tool, kadr asks once and turns that setting off only after "yes" (and back on at uninstall)
- saves into windows' screenshots folder by default, optionally in a subfolder per month or day
- the quick modes stay: copy, save or both right away, without the editor
- the editor is its own short-lived process: the resident kadr stays at ~2 mb
- not code-signed yet (smartscreen warns); sha-256 next to each file

## 0.1.0-rc.1 — 2026-11-11

first release candidate. not stable yet: manual input checks and multi-monitor, per-monitor dpi and hdr checks are still open (README, limitations).

- region, active window and screen capture via global hotkeys and the tray
- copy to the clipboard and/or save a png; nothing is overwritten
- region overlay over the frozen screen, with crop marks and a size label
- settings window: hotkeys with conflict detection, send to, folder, file name pattern, start with windows; light and dark system theme
- settings window runs in its own short-lived process, so the resident app stays at ~2 mb
- per-monitor-v2 dpi awareness, single instance, clean shutdown of every process, also at session end
- installer for the current user (no admin rights): start with windows and start now switches, start menu entry, uninstall from windows settings › apps
- app icon (crop marks on a graphite tile) for the exe, the settings window and the installer; version information in file properties
- uninstall keeps your settings unless you confirm removing them in a separate question
- fully offline; no telemetry
