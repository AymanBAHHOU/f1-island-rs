# Porting checklist (Electron app → this app)

The page (`ui/`) is the Electron app's renderer, unchanged except for `tauri-shim.js`,
which provides `window.f1` on top of Tauri commands and events. A feature is done when its
`window.f1` call works against real Rust code.

## Done
- [x] Weekend, Standings, Circuit pages (Jolpica)
- [x] News (RSS, open in browser)
- [x] Title fight (same chances as Electron, checked) · fresher standings (OpenF1) · snapshot assembled in Rust
- [x] OpenF1 client (queue, disk cache) · weekend results
- [x] Live timing connection, Live tab, qualifying gaps, status log
- [x] Hover / click-through

## To do
- [x] Track map
- [x] Analysis backend (identical output to Electron on real sessions): sessions, runs, compare data, tyres & speed, replay, telemetry, location
- [x] Alerts (flags, SC/VSC, penalties, rate limits; preview)
- [ ] Demo mode · settings
- [ ] Shell: drag, dock + slide, pin, auto-hide, summon hotkey, big-mode geometry, menu, tray, single instance, login item, position memory, mac notch
- [ ] Release: updater + signing, installers, release workflow, README
