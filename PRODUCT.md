# Product

<!-- impeccable:product-schema 1 -->

## Platform

windows

(native desktop utility for windows 10 1703+ / 11. the core, tray, overlay and menus are win32. the settings window is the only surface where a webview (tauri) is still a candidate; see docs/architecture.md.)

## Users

people on windows who take screenshots often and want it to be instant: press a hotkey, select, paste or find the file. they do not want to configure a workflow tool first. the app should be forgettable until the moment it is needed.

## Product Purpose

kadr takes a screenshot quickly and lets the user use it right away: copy to the clipboard and/or save a png to a predictable folder. three capture modes: region, active window, monitor under the cursor. success means the main action needs no thought, the app costs almost nothing while idle, and it never surprises the user with data it kept or sent.

## Positioning

a deliberately small alternative to feature-heavy capture suites (sharex is the reference point for what users already know). the difference is not a feature but its absence: no uploaders, editors, workflows, accounts or background services. measured idle cost (about 1.6 mb private memory, zero cpu, one process) is part of the product, not an implementation detail.

## Operating Context

- lives in the notification area; used through global hotkeys (default ctrl+printscreen region, alt+printscreen window, ctrl+shift+printscreen screen) or a tray click.
- the region overlay is on screen for a few seconds per capture; everything else is invisible.
- settings are opened rarely: change a hotkey, the folder, the file name pattern, copy/save behaviour, start with windows.
- windows 11 gives bare printscreen to snipping tool by default; kadr's defaults avoid it.
- multi-monitor and mixed-dpi setups are expected, high-dpi laptops common.

## Capabilities and Constraints

- capture: region (with overlay), active window, monitor under cursor. png only.
- after capture: copy, save, or both (default both). save folder default `pictures\kadr`, file name pattern with date/time tokens, never overwrites.
- settings content (complete list): three hotkeys, after-capture action, save folder, file name pattern, start with windows (off by default, per-user `HKCU\...\Run` entry, no service).
- interface language: english, lowercase where natural.
- fully local: no network, no telemetry, no update checks.
- non-goals (need explicit approval): video, gif, ocr, ai, cloud, accounts, plugins, image editor, upload services, scripting, themes store.
- resource rules: no webview kept in memory when no window is open; no new dependency without a measured reason.

## Brand Commitments

- maker: lezneve, "small software. carefully made."
- voice: lowercase, short, calm, no marketing superlatives, no cute phrasing, no corporate pathos.
- name "kadr" is provisional (single constant in code).

## Evidence on Hand

- measurements: docs/measurements.md (idle, latency, memory, spike tauri vs native).
- verified vs unverified behaviour: docs/testing.md.
- no logo, no screenshots for marketing, no testimonials or user numbers exist; none may be invented.

## Product Principles

1. the main action takes one hotkey and zero decisions.
2. nothing runs, renders or listens unless the user is using it.
3. fewer settings, each obviously useful; no setting for a decision the app can make well.
4. privacy by default: nothing leaves the machine, nothing is kept without an obvious reason.
5. claims are measured; unknowns are stated.

## Accessibility & Inclusion

keyboard access for every action, visible focus, readable contrast in light and dark themes, correct scaling from 100 % to 200 %+, screen-reader labels on settings controls, respect for "reduce motion" (no animation is needed to understand any state).
