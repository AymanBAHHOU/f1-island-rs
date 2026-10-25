# Porting checklist (Electron app → this app)

The page (`ui/`) is the Electron app's renderer, unchanged except for `tauri-shim.js`,
which provides `window.f1` on top of Tauri commands and events. A feature is done when its
`window.f1` call works against real Rust code.

## Done
- [x] Weekend, Standings, Circuit pages (Jolpica)
- [x] News (RSS, open in browser)
- [x] Live timing connection, Live tab, qualifying gaps, status log
- [x] Hover / click-through

## To do
- [ ] Title fight (Monte Carlo) · fresher standings (OpenF1) · weekend results · track map
- [ ] Analysis: sessions, short/long runs, compare, tyres & speed, replay, telemetry, location
- [ ] Alerts · demo mode · settings
- [ ] Shell: drag, dock + slide, pin, auto-hide, summon hotkey, big-mode geometry, menu, tray, single instance, login item, position memory, mac notch
- [ ] Release: updater + signing, installers, release workflow, README
