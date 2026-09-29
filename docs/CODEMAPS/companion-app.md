# companion-app code map

> **Scope**: PC GUI companion app modules. For the system-level role, see `ARCHITECTURE.md`.

Single-binary Windows GUI (egui / eframe) that sits alongside the SteamVR driver
and talks to the streaming engine through `%APPDATA%/FocusVisionPCVR/status.json`
(read-only) and `%APPDATA%/FocusVisionPCVR/config/local.toml` (write; the
engine reads it as its top config layer). Does not link against
`streaming-engine` — the engine runs in the driver DLL, not here.

---

## Files

| Path | Purpose | LoC |
|---|---|---|
| `src/main.rs` | `CompanionApp` struct, `eframe::App` impl, 3-tab UI (Home / Deploy / Settings) | ~1150 |
| `src/file_dialog.rs` | Open / save dialogs through PowerShell's Windows Forms, each on its own thread (`DialogTask`); the path comes back as hex of its UTF-8 bytes, since PowerShell 5.1's stdout is in the OEM code page | ~190 |
| `src/config.rs` | `LocalConfig` (video / sleep_mode / face_tracking / recording overrides). Persists to `%APPDATA%/FocusVisionPCVR/config/local.toml` | 194 |
| `src/driver.rs` | SteamVR driver install / uninstall. Detects SteamVR via registry lookup; SteamVR's log directory from `openvrpaths.vrpath` | ~310 |
| `src/adb.rs` | `AdbDevice`, `list_devices` / `install_apk` / `dump_logcat` / `launch_app` (blocking `Command::new("adb")`) | 209 |
| `src/headset_link.rs` | "Send PIN to headset": reads the headset's Wi-Fi address over adb, picks this PC's address toward it (routing table via a connected UDP socket), starts the client with `--es fvp_server/fvp_pin/fvp_udp_port`. Ports come from status.json (`tcp_port`/`udp_port`) | ~150 |
| `src/export.rs` | `export_logs()` — zip the engine log (`engine.log`, `engine.prev.log`), status.json, `config/local.toml`, SteamVR's vrserver / vrcompositor logs, ADB logcat, system info (Windows version, GPUs + driver versions via CIM), PII masked | ~740 |
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

Persistence: checkbox / slider changes are saved to
`%APPDATA%/FocusVisionPCVR/config/local.toml` after a 500 ms debounce (flushed
on exit), atomically, merging only the companion's own keys. The engine picks
up changes on the next SteamVR start (no hot-reload currently).

---

## Key types

### `LocalConfig` (config.rs)
- `VideoOverride { codec: String }` — "h264" or "h265"
- `SleepModeOverride { enabled, timeout_seconds }`
- `FaceTrackingOverride { enabled, smoothing }`
- `RecordingOverride { enabled, output_dir }` — Session Recording
- Parse failure → `log::warn!` + defaults (see `load()`)
- Path: `%APPDATA%/FocusVisionPCVR/config/local.toml`; a legacy
  `exe_dir/../../config/local.toml` or CWD `config/local.toml` is migrated on
  first load. An unparsable file is backed up to `local.toml.bak`.

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
| `driver.rs` | 5 | registered driver, SteamVR log dir from the vrpath, install |
| `stats_history.rs` | ~3 | ring buffer eviction |
| `file_dialog.rs` | 5 | Japanese paths through real PowerShell, hex decoding, dialog thread answer |
| `export.rs` | 24 | PII masking (IP / MAC / SSID / e-mail / user path / PIN), log added masked, system info without wmic |

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
- No runtime CONFIG_UPDATE hot-reload — changes require engine restart
