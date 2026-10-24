# F1 Island (Rust)

A rewrite of [F1 Island](https://github.com/AymanBAHHOU/f1-island), the Dynamic Island for Formula 1 on your desktop, in **Rust** with [Tauri](https://tauri.app). Work in progress; the Electron app stays the reference until this one matches it.

Why: learn Rust on a real project, and end up with a smaller (about 10 MB, not about 110 MB) and faster app.

## How it works

- The **screen** is a web page (`ui/`), shown by the system's web view. It's the same island as the Electron app, so its HTML, CSS and JavaScript come across from `renderer/` over time.
- The **logic** is Rust (`src-tauri/`): the window, the data, live timing, the maths. The page calls Rust functions by name (`invoke('island_status')`), and Rust sends events to the page (`hover`).

## Stages

| # | Stage | What it teaches |
|---|---|---|
| 1 | **Hello island**: a transparent, always-on-top, click-through window; the island opens when the mouse is over it; data comes from a Rust function | project layout, `cargo`, commands, shared state, threads |
| 2 | **Pure logic with tests**: version compare, dock rule, status log, alert rules, session clock | structs, enums, pattern matching, `#[test]` |
| 3 | **Fetching data**: the schedule, standings, circuit history | async Rust, `reqwest`, `serde` (JSON to Rust types), errors with `Result` |
| 4 | **The live feed**: F1 live timing over a WebSocket | long-running async tasks, shared state, sending updates to the page |
| 5 | **The heavy maths**: title chances (Monte Carlo), session analysis | performance, iterators |
| 6 | **The shell**: dragging, docking, tray, hotkey, auto-update, installer | Tauri plugins, packaging |

macOS notch support comes last (it needs native Mac code).

## Run it

You need [Rust](https://rustup.rs), Node.js, and on Windows the Visual Studio C++ build tools (WebView2 is built into Windows 11).

```bash
npm install
npm run dev
```

## Disclaimer

F1 Island is an unofficial fan project. It is not associated in any way with the Formula 1 companies, the FIA or any Formula 1 team. F1, FORMULA ONE, FORMULA 1 and related marks are trademarks of Formula One Licensing B.V.

## License

MIT
