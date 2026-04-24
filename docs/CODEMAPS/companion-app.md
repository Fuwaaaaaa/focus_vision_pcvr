# companion-app code map

> **Scope**: PC GUI companion app modules. For the system-level role, see `ARCHITECTURE.md`.

Single-binary Windows GUI (egui / eframe) that sits alongside the SteamVR driver
and talks to the streaming engine through `%APPDATA%/FocusVisionPCVR/status.json`
(read-only) and `config/local.toml` (write). Does not link against
`streaming-engine` — the engine runs in the driver DLL, not here.

---

## Files

| Path | Purpose | LoC |
|---|---|---|
| `src/main.rs` | `CompanionApp` struct, `eframe::App` impl, 3-tab UI (Home / Deploy / Settings) | 921 |
| `src/config.rs` | `LocalConfig` (video / sleep_mode / face_tracking / recording overrides). Persists to `config/local.toml` | 194 |
| `src/driver.rs` | SteamVR driver install / uninstall. Detects SteamVR via registry lookup | 115 |
| `src/adb.rs` | `AdbDevice`, `list_devices` / `install_apk` / `dump_logcat` / `launch_app` (blocking `Command::new("adb")`) | 209 |
| `src/export.rs` | `export_logs()` — zip PC log + ADB logcat + system info, sanitize IP/PII | 178 |
| `src/stats_history.rs` | 30-second ring buffer for latency / FPS / packet-loss sparklines | 102 |

---

## UI structure (main.rs)

```
CompanionApp (25+ fields)
├── Home tab       → render_home()       ~200 LoC
│   ├── Pairing PIN display
│   ├── Connection status (disconnected / waiting / connected)
│   ├── Subsystem badges (FT / sleep / audio / packet loss)
│   └── Sparkline graphs (egui_plot)
├── Deploy tab     → render_deploy()     ~125 LoC
│   ├── SteamVR driver install toggle
│   ├── ADB device picker + apk_path
│   └── Deploy button (async, Arc<Mutex<Option<String>>> result)
└── Settings tab   → render_settings()   ~175 LoC
    ├── Codec toggle (h264 / h265)
    ├── Sleep mode (enabled + timeout)
    ├── Face tracking (enabled + smoothing)
    └── Session Recording (enabled + output_dir)
```

Persistence: every checkbox / slider change writes `LocalConfig` immediately
to `config/local.toml`. Engine picks up changes on next restart (no hot-reload
currently).

---

## Key types

### `LocalConfig` (config.rs)
- `VideoOverride { codec: String }` — "h264" or "h265"
- `SleepModeOverride { enabled, timeout_seconds }`
- `FaceTrackingOverride { enabled, smoothing }`
- `RecordingOverride { enabled, output_dir }` — Session Recording
- Parse failure → `log::warn!` + defaults (see `load()`)
- Path: `exe_dir/../../config/local.toml` (dev layout) fallback to `config/local.toml` (CWD)

### `AdbDevice` (adb.rs)
- `serial: String`, `status: String`
- `find_adb()` searches PATH + %LOCALAPPDATA%/Android/Sdk + %ProgramFiles%

### `StatsHistory` (stats_history.rs)
- Ring buffer capacity ~2700 samples (30s at 90fps)
- Feeds `Plot` rendering via `PlotPoints::from_iter`

---

## Tests (27 total)

| File | Tests | Focus |
|---|---|---|
| `config.rs` | 9 | round-trip, recording override, parse failure fallback |
| `adb.rs` | 6 | device list parsing, timeout handling |
| `driver.rs` | ~3 | SteamVR dir detection |
| `stats_history.rs` | ~3 | ring buffer eviction |
| `export.rs` | 0 | **no tests yet** — next PR candidate |

---

## External dependencies (Cargo.toml)

- `eframe` / `egui` — GUI
- `egui_plot` — sparklines
- `serde` / `toml` — config persistence
- `dirs_next` — %APPDATA% resolution
- `log` / `env_logger` — logging
- `chrono` — timestamps for exports
- `zip` — export bundle

---

## Known issues (from audit)

- `main.rs` 921 LoC; `render_home` alone is 207 LoC — split candidate
- `export.rs` 0 tests — zip / PII paths unverified
- No runtime CONFIG_UPDATE hot-reload — changes require engine restart
