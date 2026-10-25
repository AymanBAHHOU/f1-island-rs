# Porting checklist (Electron app → this app)

The page (`ui/`) is the Electron app's renderer, unchanged except for `tauri-shim.js`,
which provides `window.f1` on top of Tauri commands and events. A feature is done when its
`window.f1` call works against real Rust code.

Where it could be compared, the Rust output was checked **identical** to the Electron app's
on real data (snapshot, title-fight chances, session analysis, telemetry, replay, demo data).

## Done
- [x] Weekend, Standings, Title, Circuit, News pages; weekend results (top three per session)
- [x] Live timing connection, Live tab, qualifying gaps, status log, Live-tab rule
- [x] Alerts (flags, SC/VSC, penalties, rate limits) and "Preview alerts"
- [x] Session analysis: Summary, Short runs, Long runs, Compare, Replay, Tyres & speed
- [x] Track map, telemetry, lap location, deleted laps
- [x] OpenF1 client (queue, disk cache, rate-limit retry); fresher standings with "After <GP>" note
- [x] Settings file (same format as Electron's); preview (demo) mode; update chip
- [x] Window: always-on-top, transparent, click-through with hover polling
- [x] Drag, docking (drop near the top edge, slides to centre), position memory, reset position
- [x] Big window for the analysis view (and dragging it)
- [x] Tray icon + menu, island menu (⋯ / right click), pop-up lists, session picker
- [x] Global hotkey, single instance, start with Windows, multi-monitor / wake-from-sleep re-placement
- [x] Debug flags: `--pinned --big --tab= --page= --start= --demo= --demo-alerts --fake-update= --docked`

## To do
- [ ] macOS: camera-notch attachment and the "Idle appearance" menu are written (AppKit via objc2 in `mac.rs`, plus the tested geometry in `notch.rs`) and compiled by CI, but not yet tried on a real notched Mac
- [x] Release pipeline: signed NSIS installer + portable exe + macOS dmgs, draft release (workflow written; Windows installer builds and signs locally, 4.5 MB)
- [ ] Before the first release: set the repository secret `TAURI_SIGNING_PRIVATE_KEY` (the key is in `~/.f1-island-signing/`), and make the repo (or a releases repo) public so installed apps can reach `latest.json`
- [ ] Not ported on purpose: `--screenshot=` (developer tool for the README)
