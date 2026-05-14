# Focus Vision PCVR

VIVE Focus Vision向けPCVRストリーミングツール（v3.0.0、現在 rc1 candidate）。

## Architecture
- `rust/streaming-engine/` — Rust static library (C ABI via cbindgen)
- `rust/companion-app/` — PC companion GUI app (egui, single .exe)
  - `src/ui/{home,deploy,settings}.rs` — タブごとに分離した render impl
  - `src/status_parser.rs` — `status.json` パース層（egui 非依存、テスト可能）
- `rust/common/` — Shared types and constants
- `driver/` — C++ OpenVR driver DLL (loaded by SteamVR)
- `client/` — Android OpenXR client (Kotlin + C++ NDK)
- `installer/` — NSIS Windows installer (`focus_vision.nsi`)
- `scripts/sign-windows.ps1` — Authenticode signing wrapper (env-driven)

Key modules in streaming-engine:
- `engine.rs` — Main streaming loop, TCP control, haptic events, thermal governor wiring
- `thermal.rs` — `ThermalGovernor` + `NvmlThermalSource` (feature = "nvml"), bitrate ceiling cap
- `sleep_mode.rs` — User inactivity detection and sleep/wake transitions
- `face_tracking/osc_bridge.rs` — HTC blendshapes → VRChat OSC with EMA smoothing + profile weights
- `face_tracking/profiles.rs` — Per-avatar expression profiles (51 blendshape weights, JSON)
- `face_tracking/calibration.rs` — Guided auto-calibration (min/max → weight computation)
- `config.rs` — TOML config with validation (structured ConfigError, range checks, NaN rejection)
- `transport/` — RTP packetization, FEC (adaptive + fixed + slice), UDP with buffer pool
- `transport/slice.rs` — SliceSplitter: NAL → N slices at byte boundaries
- `adaptive/` — Bandwidth estimation, bitrate controller, GCC delay estimator, burst detector
- `control/` — TCP server with TLS, PIN pairing, CONFIG_UPDATE protocol (`0x03` video, `0x05` audio)
- `control/reconnect.rs` — Accept-failure / reconnect-attempt counters + exponential backoff
- `metrics/session_log.rs` — JSONL session logging with rotation
- `metrics/memory.rs` — Process RSS monitoring (GetProcessMemoryInfo / /proc/self/status)
- `recording/` — Video Annex-B + audio WAV recorders, runtime toggles via CONFIG_UPDATE
- `simulator.rs` + `bin/headless.rs` + `bin/mock_client.rs` — In-process E2E harness (`--features simulator`)

See `ARCHITECTURE.md` for detailed system diagrams and data flow.

## Build
```bash
./build.bat   # Windows full build
cargo build --release -p streaming-engine          # Rust streaming engine
cargo build --release -p focus-vision-companion    # PC companion app
cargo build --release -p streaming-engine --features simulator --bins   # E2E binaries
cargo test --workspace                              # 450+ tests
```

## Testing
```bash
cargo test --workspace                              # All tests (450+)
cargo test -p streaming-engine                      # Engine: 340+ tests + integration
cargo test -p focus-vision-companion --bins         # Companion: 60 tests (config, ADB, export, status_parser, demo, svg_export, ui/settings validator)
cargo test -p fvp-common                            # Common: protocol structs / flags / versioning
cargo bench -p streaming-engine                     # Criterion benchmarks
cargo clippy --workspace -- -D warnings             # CI clippy gate (some pre-existing toolchain regressions; new RC code is clean)
# Fuzz targets (Linux CI / cargo-fuzz):
cd rust/streaming-engine && cargo fuzz list         # fuzz_rtp, fuzz_fec, fuzz_protocol, fuzz_config, fuzz_slice, fuzz_recording
# Headless E2E (simulator feature, runs full TCP+TLS+RTP+FEC+UDP loop in-process):
cargo run -p streaming-engine --bin focus-vision-headless --features simulator
cargo test --workspace --features simulator -- --test-threads=1  # includes headless_e2e_test
# C++ tests (requires CMake build):
cd driver/build && cmake --build . --config Release
ctest --test-dir driver/build --build-config Release --output-on-failure  # 36 gtest cases
```

## Companion App
```bash
cargo run -p focus-vision-companion          # Run the PC companion app
cargo run -p focus-vision-companion -- --demo  # Demo mode — synthesizes status without engine
```

`--demo` runs a 60 s scripted cycle (Disconnected → WaitingForPin
"847251" → Connected with animated stats). Bypasses `status.json`,
disables ADB scans, and shows a yellow "DEMO MODE" banner so the UI
can be exercised without a VR rig.

Tabs (file-per-tab under `rust/companion-app/src/ui/`):
- **Home** — engine-stopped banner, driver status, contextual setup hint, PIN display (with `Expires in` countdown when active), live stats sparklines, subsystem indicators, "Recent activity" log tail (collapsing)
- **Deploy** — ADB device list, APK picker (path persists), install button
- **Settings** — driver, audio (persists), sleep mode, face tracking, session recording with inline dir validation, codec radio, diagnostics zip export, **stats SVG export**, reset-to-defaults (2-stage confirm)

## Config
`config/default.toml` — override with `config/local.toml` (gitignored).
Companion app additionally writes a `local.toml` for its UI-side overrides
(`[video] [sleep_mode] [face_tracking] [recording] [audio] [deploy]`).
Config values are validated on startup (range checks, NaN rejection, port conflict detection).

## Release / Signing
- `installer/focus_vision.nsi` — NSIS installer source
- `scripts/sign-windows.ps1` — Authenticode wrapper (reads `WINDOWS_PFX_BASE64` + `WINDOWS_PFX_PASSWORD` env vars)
- `client/app/build.gradle.kts` — Android keystore signing (`ANDROID_KEYSTORE_BASE64` + alias / password env vars, with ephemeral-keystore fallback)
- `.github/workflows/build.yml` — `installer-build` job + Authenticode signing steps (conditional on secrets)
- `docs/SIGNING.md` — operator-side signing manual

## Security
- TCP control channel encrypted with TLS 1.3 (rustls server, MbedTLS client)
- 6-digit PIN with cryptographic RNG (1M combinations, 5 attempts then 300s lockout)
- TOFU certificate pinning (SHA-256 fingerprint)
- CONFIG_UPDATE messages validated (range checks, rate limiting)
- See `SECURITY.md` for threat model.

## Design System
Always read DESIGN.md before making any visual or UI decisions.
All font choices, colors, spacing, and aesthetic direction are defined there.
Do not deviate without explicit user approval.
In QA mode, flag any code that doesn't match DESIGN.md.
