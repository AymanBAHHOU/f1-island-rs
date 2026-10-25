# F1 Island (Rust)

The Dynamic Island for Formula 1 on your desktop, written in **Rust** with [Tauri](https://tauri.app). It is the same island as the Electron app ([F1 Island](https://github.com/AymanBAHHOU/f1-island)), rebuilt to be much smaller (about 10 MB instead of about 110 MB) and lighter on memory.

Weekend schedule and results, standings, title fight chances, circuit history, news, live timing with alerts, and full session analysis (short and long runs, compare, replay, tyres and speed). It floats above your windows, opens when the pointer is over it, and can be dragged anywhere or docked to the top edge. See [PORTING.md](PORTING.md) for the feature list and what has been verified.

## How it works

- The **screen** is a web page (`ui/`), shown by the system's web view. It is the Electron app's page, unchanged; `ui/tauri-shim.js` gives it the `window.f1` interface on top of Tauri.
- The **logic** is Rust (`src-tauri/`). The page calls Rust functions by name (`invoke('snapshot')`) and Rust sends events to the page (`hover`, `live:state`, `live:alert`).
- Code that needs no window lives in the library (`src-tauri/src/lib.rs` and its modules) and is tested without a screen (`cargo test --lib`). The window, menus and tray are in the program itself (`main.rs`, `shell.rs`, `menus.rs`, `live.rs`, `self_update.rs`).

| Area | Modules |
|---|---|
| Data | `jolpica`, `openf1`, `news`, `snapshot`, `standings`, `circuit`, `schedule`, `results`, `season` |
| Title fight | `championship` (Monte Carlo, same random numbers as the Electron app), `title` |
| Live timing | `live_client`, `live_state`, `merge`, `live_tab`, `live_window`, `clock`, `status_log`, `alerts` |
| Analysis | `analysis` (the calculation), `analysis_data` (fetching, telemetry, track map, replay) |
| Window | `geometry` (where the island goes), `dock`, `notch`, `platform`, `settings`, `demo`, `updater` |

## Run it

You need [Rust](https://rustup.rs), Node.js, and on Windows the Visual Studio C++ build tools (WebView2 is built into Windows 11).

```bash
npm install
npm run dev
```

Useful flags (after `--` with `npm run dev --`): `--pinned` (start open), `--big` (open the analysis view), `--page=`, `--tab=`, `--demo=race` (a pretend race), `--demo-alerts`, `--fake-update=9.9.9`, `--docked`.

## Test it

```bash
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

`cargo test --lib -- --ignored` also runs the tests that use the real network.

## Releases

Push a version tag to build the Windows installer, the portable exe and the Mac apps, and create a **draft** GitHub release:

1. Set the same version in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`.
2. `git tag v0.1.0 && git push origin v0.1.0`
3. When the draft looks right, publish it on GitHub.

The installer updates itself: the app checks the release's `latest.json`, downloads the signed update in the background, and installs it when you choose **Restart to update**. The portable exe and the Mac app only announce a new version.

Updates are signed. The public key is in `tauri.conf.json`; the private key is the repository secret `TAURI_SIGNING_PRIVATE_KEY` (its password, if any, is `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`). Keep a copy of the private key somewhere safe: without it, installed apps can never be updated to a new release. To make a new pair: `npx tauri signer generate -w f1-island.key`, then put the public key in `tauri.conf.json`.

The builds are not code-signed or notarized yet (Windows SmartScreen and macOS Gatekeeper ask for confirmation on first run).

## Disclaimer

F1 Island is an unofficial fan project. It is not associated in any way with the Formula 1 companies, the FIA or any Formula 1 team. F1, FORMULA ONE, FORMULA 1 and related marks are trademarks of Formula One Licensing B.V.

## License

MIT
