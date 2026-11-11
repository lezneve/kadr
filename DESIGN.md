---
name: kadr
description: a small windows screenshot utility; graphite frames, ink on ground, crop marks on the frame in use.
colors:
  ground-dark: "#1c1c1e"
  raised-dark: "#242426"
  hover-dark: "#2a2a2c"
  cap-dark: "#2c2c2e"
  seg-on-dark: "#3a3a3c"
  ink-dark: "#f2f2f2"
  ink2-dark: "#a1a1a6"
  ink3-dark: "#98989d"
  hair-dark: "rgba(242, 242, 242, 0.08)"
  alert-dark: "#ff7b72"
  ground-light: "#f5f5f6"
  raised-light: "#ffffff"
  hover-light: "#f2f2f4"
  cap-light: "#f0f0f2"
  seg-on-light: "#ffffff"
  ink-light: "#1d1d1f"
  ink2-light: "#5b5b60"
  ink3-light: "#6e6e73"
  hair-light: "rgba(28, 28, 31, 0.10)"
  alert-light: "#c4302b"
  overlay-mark: "#ffffff"
  tray-ink-light-taskbar: "#1c1c1c"
typography:
  title:
    fontFamily: "Segoe UI Variable Display, Segoe UI, sans-serif"
    fontSize: "20px"
    fontWeight: 500
  head:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 500
  body:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "13px"
    fontWeight: 400
  small:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
  mono:
    fontFamily: "Cascadia Mono, Consolas, monospace"
    fontSize: "12px"
    fontWeight: 400
  overlay:
    fontFamily: "Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
rounded:
  group: "12px"
  segment-track: "8px"
  control: "6px"
  overlay-pill: "3px"
  pill: "9999px"
spacing:
  margin: "24px"
  row: "44px"
  note: "24px"
  head: "24px"
  gap: "16px"
  pad: "16px"
  keycap-gap: "4px"
components:
  group:
    backgroundColor: "{colors.raised-dark}"
    rounded: "{rounded.group}"
  row:
    textColor: "{colors.ink-dark}"
    typography: "{typography.body}"
    height: "{spacing.row}"
    padding: "0 16px"
  row-hover:
    backgroundColor: "{colors.hover-dark}"
  row-pressed:
    backgroundColor: "{colors.cap-dark}"
  keycap:
    backgroundColor: "{colors.cap-dark}"
    textColor: "{colors.ink-dark}"
    typography: "{typography.small}"
    rounded: "{rounded.control}"
    padding: "0 8px"
    height: "24px"
  segment-track:
    backgroundColor: "{colors.cap-dark}"
    rounded: "{rounded.segment-track}"
    padding: "2px"
  segment:
    textColor: "{colors.ink2-dark}"
    typography: "{typography.small}"
    padding: "0 12px"
    height: "28px"
  segment-selected:
    backgroundColor: "{colors.seg-on-dark}"
    textColor: "{colors.ink-dark}"
    rounded: "{rounded.control}"
  switch-on:
    backgroundColor: "{colors.ink-dark}"
    rounded: "{rounded.pill}"
    width: "36px"
    height: "20px"
  switch-off:
    rounded: "{rounded.pill}"
    width: "36px"
    height: "20px"
  field:
    backgroundColor: "{colors.cap-dark}"
    textColor: "{colors.ink-dark}"
    typography: "{typography.mono}"
    rounded: "{rounded.control}"
    padding: "0 8px"
    width: "240px"
    height: "28px"
  note:
    textColor: "{colors.ink3-dark}"
    typography: "{typography.small}"
    height: "{spacing.note}"
  note-alert:
    textColor: "{colors.alert-dark}"
    typography: "{typography.small}"
  overlay-pill:
    backgroundColor: "{colors.ground-dark}"
    textColor: "{colors.ink-dark}"
    typography: "{typography.overlay}"
    rounded: "{rounded.overlay-pill}"
    padding: "4px 8px"
---

# Design System: kadr

## Overview

**Creative North Star: "The Set of Frames"**

the settings window is a set of frames, and the frame you are editing is marked by four crop marks: the same corners the region overlay draws around a selection and the tray icon draws around its centre dot. one mark, three surfaces. everything else is quiet graphite that follows the windows app theme, so the marks are the only emphasis on screen.

density is calm and native: one 520 px column, four groups (hotkeys, after capture, saving, general), each a raised rounded surface of 44 px rows with the label left and the control right. there is no accent hue, no shadow and no drawn motion. state is shown by ink weight, tone steps and the crop marks, never by colour, except for problems the user must act on.

the system refuses the windows-settings card stack where every row weighs the same and an accent colour decorates toggles.

**Key Characteristics:**
- graphite neutral ramp, dark and light, chosen from the windows `AppsUseLightTheme` setting; the title bar is painted in the same ramp.
- ink on ground: selected and on states use ink or a lighter tone, never a hue.
- crop marks on the focused row only; nothing else is emphasised.
- keycaps for hotkeys, cascadia mono for file names.
- flat: depth is two tones plus a hairline.
- nothing kadr draws animates; the overlay also disables dwm transitions.

## Colors

a graphite ramp in two themes, three text roles, one hairline and one red held back for problems.

### Primary
- **Ink** (`ink-dark` / `ink-light`): primary text, keycap labels, the crop marks, the switch track when on. it is the system's only "accent": the mark is ink on ground.

### Tertiary
- **Problem Red** (`alert-dark` / `alert-light`): the only hue, used for text only (never fills or strokes).

### Neutral
- **Graphite Ground** (`ground-dark` / `ground-light`): window background, and the title bar caption colour (dwm attribute 35, with ink as caption text via attribute 36). the overlay pill reuses `ground-dark` in both themes.
- **Raised Frame** (`raised-dark` / `raised-light`): group surfaces; also the switch knob when on.
- **Hover Tint** (`hover-dark` / `hover-light`): a row under the pointer, clipped to the group shape.
- **Keycap Grey** (`cap-dark` / `cap-light`): keycaps, the segment track, the file name field, the pressed row.
- **Lifted Segment** (`seg-on-dark` / `seg-on-light`): the selected segment.
- **Quiet Ink** (`ink2-dark` / `ink2-light`): group heads, the folder path and its chevron, unselected segment labels, the recording keycap edge, the switch knob when off.
- **Hint Ink** (`ink3-dark` / `ink3-light`): hint and note lines ("enter to change", recording instructions), "off" for an unset hotkey, the next file name preview, the switch outline when off.
- **Hairline** (`hair-dark` / `hair-light`): group outline, row separators inset by the row padding, keycap and field outlines. built at 8 % in dark and 10 % in light; the direction contract said 8 % for both, and the build wins.

### Named Rules
**The Ink-On-Ground Rule.** no accent hue. on, selected and focused states use ink, `seg-on` or the crop marks. the system accent is kept out too: the file name field moves the caret to the end on focus so tab never paints a system-accent selection.

**The Red-For-Action Rule.** confirmed by the user on 2026-11-11. red is held for conflicts and every error that needs the user's attention, and as built it appears in exactly these places: a hotkey taken by snipping tool (held under that row until resolved), a key that cannot be a hotkey, a combination already used by another kadr capture, a combination used by another app, an invalid file name (it replaces the next-file preview), and a failed start-with-windows change. instructions, hints and other informational text never use red; they stay in hint ink.

## Typography

**Display Font:** Segoe UI Variable Display (with Segoe UI)
**Body Font:** Segoe UI Variable Text (with Segoe UI)
**Label/Mono Font:** Cascadia Mono (with Consolas)

**Character:** the platform's own type, used plainly. a single 20 px title, then everything at 13 or 12 px; mono appears only where the text is a literal file name.

### Hierarchy
- **Title** (500, 20 px): the word "settings" at the top of the window. one per window.
- **Head** (500, 12 px, quiet ink): group names, lowercase, 8 px in from the group edge.
- **Body** (400, 13 px): row labels ("capture region", "folder", "start with windows").
- **Small** (400, 12 px): keycap labels, segment labels, the folder path, hints, notes, errors.
- **Mono** (400, 12 px): the file name pattern inside the field and the "next file" preview beneath it.
- **Overlay** (400, 12 px, plain Segoe UI through gdi): the size label and the "drag to select · esc to cancel" hint on the region overlay.

no line-height or letter-spacing is set; text is centred vertically in its box and never wraps.

### Named Rules
**The Two Sizes Rule.** everything inside the groups is 13 px or 12 px. hierarchy comes from ink role and weight, not from new sizes.

**The Literal Mono Rule.** cascadia mono is for text the machine will write verbatim (the file name pattern and its preview). nothing else is set in mono.

## Layout

one fixed column, 520 px wide (logical px at 96 dpi, scaled by the window's dpi at runtime). content sits inside a 24 px margin. the title starts 16 px from the top; the first group head starts 56 px down. each group is a 24 px head followed by its surface; groups are 16 px apart; the window ends 24 px below the last group.

rows are 44 px with 16 px inner padding: label left, control right-aligned against the padding. a row that carries a note (hint, recording instruction, conflict, error) grows by 24 px and the note sits under the label line; the file name row always carries its 24 px preview. the window height is computed from content and refit when a note appears, kept inside the monitor's work area. it opens centred horizontally on the monitor under the cursor, a third of the way down.

the focused hotkey row adds "enter to change" in hint ink, 12 px left of its keycaps, on the same line.

## Elevation & Depth

flat. there are no shadows anywhere in the build. depth is two tones (ground and raised) plus a hairline outline; hover and pressed are tone steps inside the group shape. the overlay is flat as well: the screen is dimmed to 140/256 brightness, the selection shows at full brightness, and dwm transitions and rounded corners are disabled so it appears instantly as the screen itself.

### Named Rules
**The Flat Frame Rule.** a frame is a tone and a hairline, never a shadow.

## Shapes

softly rounded containers, small rounded controls, square crop marks.

- groups: 12 px corners, one hairline outline; rows inside are separated by an inset hairline, not by gaps.
- keycaps, the file name field, the selected segment: 6 px.
- segment track: 8 px, 2 px larger than its segments on every side.
- switch track and knob: full pill.
- overlay pill: about 3 px (gdi `RoundRect` with a 6 px ellipse).
- the overlay window itself has square corners.

the crop mark is the recurring geometry: two square-ended strokes meeting at each corner, drawn in three sizes.
- settings row: 2 px stroke, 10 px arm, 4 px inside the row edge, in ink.
- region overlay: 2 px stroke, 14 px arm, 2 px gap outside the 1 px white selection border, in white.
- tray icon: proportional to the small-icon size (stroke 1/10 of it, minimum 2; arm 3/8; margin 1/16) with a centre dot of 1/8 (minimum 2); white on a dark taskbar, `tray-ink-light-taskbar` on a light one. the same viewfinder is the window icon, redrawn when the theme changes.

## Components

quiet, native and exact: controls are drawn, not themed stock controls, and each one states its value without decoration.

### Groups and rows
- **Corner Style:** group radius (12 px).
- **Background:** raised frame on graphite ground.
- **Shadow Strategy:** none; see Elevation & Depth.
- **Border:** one hairline around the group; inset hairline between rows.
- **Internal Padding:** 16 px left and right; rows 44 px tall.
- **States:** hover tint under the pointer, keycap grey while pressed, crop marks when focused.

### Keycaps
- **Style:** one cap per key, 24 px tall, label plus 8 px each side, 4 px apart, right-aligned. keycap grey fill, hairline edge, small ink label. names are lowercase ("ctrl", "shift", "printscreen").
- **Recording:** the caps get a quiet-ink edge and the row note reads "press a new combination · esc keeps the old one · backspace turns it off" in hint ink.
- **Unset:** the word "off" in small hint ink where the caps would be.

### Segmented control (send to)
- **Style:** three segments (clipboard, file, both), 28 px tall, label plus 12 px each side, on an 8 px keycap-grey track inset 2 px.
- **State:** the selected segment lifts to `seg-on` at 6 px with ink text; others are quiet ink and turn ink on hover.

### Switch (start with windows)
- **On:** ink track, raised-frame knob (12 px) 4 px from the right end.
- **Off:** hint-ink outline, quiet-ink knob 4 px from the left end.
- 36 by 20 px, no animation between states.

### File name field
- **Style:** 240 px by 28 px, keycap grey fill, hairline edge, 6 px corners, mono text in ink with 8 px inner padding.
- **Preview:** under the line, "next file  kadr_2026-11-11_18-34-31.png" in mono hint ink, updated as you type; changes save after a 400 ms pause.
- **Error:** an invalid pattern replaces the preview with the reason in small problem red.
- **Focus:** crop marks on the row; caret at the end, no selection highlight.

### Folder row
- the path in small quiet ink, shortened to "…\last\two" when it would exceed 260 px, followed by a drawn chevron (two 1.25 px strokes in quiet ink, not a text glyph). click opens the system folder picker.

### Crop marks (signature)
- drawn on the focused row only, inside its rect, so every control can carry them completely. they mark keyboard focus and replace a focus ring. see Shapes for sizes.

### Region overlay
- dimmed screen, full-brightness selection inside a 1 px white border (scaled by dpi), white crop marks outside it.
- size label "w × h" in an overlay pill: graphite ground fill, ink-dark text, 8 by 4 px padding, 10 px above the selection (inside its top edge when there is no room).
- before dragging, the hint pill "drag to select · esc to cancel" is centred 24 px below the top of the monitor under the cursor.

### Tray
- the viewfinder icon at the exact small-icon size, recoloured for the taskbar theme. the menu is the stock win32 popup.

## Do's and Don'ts

### Do:
- **Do** take every settings-window colour from the active theme's ramp, and paint the title bar with ground and ink so the graphite world reaches the window edge.
- **Do** mark the focused frame with four crop marks (2 px stroke, 10 px arm, 4 px inset) plus at most one hint line, and emphasise nothing else.
- **Do** show hotkeys as keycaps, one cap per key, lowercase names.
- **Do** keep notes and hints to one 12 px line under the row, in hint ink; reserve problem red for the cases in The Red-For-Action Rule.
- **Do** draw icons and indicators as geometry (the chevron, the viewfinder), sized to the dpi.
- **Do** scale every logical value by the window's dpi; the overlay scales per monitor.

### Don't:
- **Don't** introduce an accent hue, including the windows system accent, for selected, on or focused states.
- **Don't** add shadows; depth is ground, raised and hairline.
- **Don't** animate state changes in anything kadr draws; no fades, slides or transitions.
- **Don't** put crop marks on more than one row at a time, or use them as decoration.
- **Don't** use a text glyph as a stand-in for an icon.
- **Don't** round the overlay window or fade it in; it must read as the screen itself.

## Icon and Installer

added after the first documentation pass, from the shipped build.

- **app icon** (`assets/kadr.ico`, drawn by `tools/make-icon.ps1`, embedded by `build.rs`): the four crop marks and the centre dot in ink `#f2f2f2` on a rounded graphite tile `#1c1c1e` (radius 22 % of the size) with a `#48484a` hairline from 24 px up, so it holds on light and dark taskbars. marks sit on whole pixels at every size (16–256); their arms are a share of the gap between them, so the marks never close into a square at 16 px. used for the exe, the settings window, the installer, alt+tab and the taskbar.
- **tray icon**: the same mark without the tile, monochrome, white on a dark taskbar and `#1c1c1c` on a light one, as windows tray icons are.
- **installer window**: the settings window's system at 460 px: title, one paragraph in `ink2` (wrapping body text), one raised group with two switch rows (start with windows, start kadr now), and a button row: primary filled with `ink` and ground-coloured text, secondary on `cap` with a hairline, both 8 px radius. focus wears the crop marks, on rows and on buttons. a second state (`kadr is installed`) keeps only the title, the paragraph and one `close` button.
