# Focus Vision PCVR

VIVE Focus Vision向けPCVRストリーミングツール（v1.3）。

## Architecture
- `rust/streaming-engine/` — Rust static library (C ABI via cbindgen)
- `rust/companion-app/` — PC companion GUI app (egui, single .exe)
- `rust/common/` — Shared types and constants
- `driver/` — C++ OpenVR driver DLL (loaded by SteamVR)
- `client/` — Android OpenXR client (Kotlin + C++ NDK)

Key modules in streaming-engine:
- `engine.rs` — Main streaming loop, TCP control, haptic events
- `sleep_mode.rs` — User inactivity detection and sleep/wake transitions
- `face_tracking/osc_bridge.rs` — HTC blendshapes → VRChat OSC with EMA smoothing
- `config.rs` — TOML config with validation (range checks, NaN rejection)
- `transport/` — RTP packetization, FEC, UDP with buffer pool
- `adaptive/` — Bandwidth estimation, bitrate controller
- `control/` — TCP server with TLS, PIN pairing, CONFIG_UPDATE protocol

See `ARCHITECTURE.md` for detailed system diagrams and data flow.

## Build
```bash
./build.bat   # Windows full build
cargo build --release -p streaming-engine    # Rust streaming engine
cargo build --release -p focus-vision-companion  # PC companion app
cargo test --workspace  # Run 156+ tests
```

## Testing
```bash
cargo test --workspace              # All tests (156+)
cargo test -p streaming-engine      # Engine: 118 tests (FEC, RTP, pairing, TLS, haptics, sleep, FT, config validation)
cargo test -p focus-vision-companion # Companion: 25 tests (config, ADB, export)
cargo test -p fvp-common            # Common: 6 tests (protocol structs, flags)
```

## Companion App
```bash
cargo run -p focus-vision-companion  # Run the PC companion app
```
Features: SteamVR driver install, 6-digit PIN display, ADB deploy, codec toggle (H.264/H.265), latency graphs, log export, sleep mode settings, face tracking settings, subsystem status display.

## Config
`config/default.toml` — override with `config/local.toml` (gitignored).
Config values are validated on startup (range checks, NaN rejection, port conflict detection).

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
