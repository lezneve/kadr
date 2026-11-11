---
version: 1
slug: "settings-window"
primary_target: "settings-window"
related_targets: ["src/overlay.rs","src/tray.rs"]
---

# kadr settings window (and tray menu, region overlay)

mode: operate. audience and product truth: PRODUCT.md. the window is opened rarely, holds eight settings in four groups, applies changes immediately, and must not keep anything resident after it closes.

open decisions: native win32 vs tauri for this window, decided by a measured comparison (docs/architecture.md); interface language english.

## Direction contract

THESIS: the window is a set of frames, and the frame you are editing is marked by four crop marks, the same corners as the region overlay and the tray icon. it refuses the windows-settings card stack where every row weighs the same and an accent colour decorates toggles.

OWN-WORLD: a graphite neutral ramp that follows the system theme (dark ground #1c1c1e with raised #242426, light ground #f5f5f6 with raised #ffffff), three text roles, hairlines at 8 % ink. no accent hue: the mark is ink on ground. red is held for problems the user must act on: a hotkey conflict or an input error (a key that cannot be used, an invalid file name, a failed autostart change). conflicts stay until resolved. informational text never uses red. (amended after the finish review; confirmed by the user on 2026-11-11.) segoe ui variable 13 px / 12 px, keycaps for hotkeys, cascadia mono for the file name preview.

STORY: open, read four groups at a glance, click a hotkey's keycaps and press a new combination, see the next file name update as you type, close. nothing to save, nothing to confirm.

FIRST VIEWPORT: a 520 px window. title bar with the four-corner mark and "kadr". "settings" at 20 px. groups hotkeys, after capture, saving, general, each a raised rounded surface of rows: label left, control right. the focused row wears the crop marks plus one hint line; nothing else is emphasised.

FORM: crop marks, rank 1 of 7 on the grounded list (taken as the pick over the assigned index card). seed:3815c3b3eba3754f

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
