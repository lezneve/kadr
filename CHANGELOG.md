# changelog

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
