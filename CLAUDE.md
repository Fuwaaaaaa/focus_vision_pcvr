# Focus Vision PCVR

VIVE Focus Vision向けPCVRストリーミングツール。

## Architecture
- `rust/streaming-engine/` — Rust static library (C ABI via cbindgen)
- `rust/companion-app/` — PC companion GUI app (egui, single .exe)
- `rust/common/` — Shared types and constants
- `driver/` — C++ OpenVR driver DLL (loaded by SteamVR)
- `client/` — Android OpenXR client (Kotlin + C++ NDK)

## Build
```bash
./build.bat   # Windows
cargo build --release -p streaming-engine    # Rust streaming engine
cargo build --release -p focus-vision-companion  # PC companion app
cargo test --workspace  # Run tests
```

## Companion App
```bash
cargo run -p focus-vision-companion  # Run the PC companion app
```
Features: SteamVR driver install, PIN display, ADB deploy to HMD, streaming stats.

## Config
`config/default.toml` — override with `config/local.toml` (gitignored).

## Design System
Always read DESIGN.md before making any visual or UI decisions.
All font choices, colors, spacing, and aesthetic direction are defined there.
Do not deviate without explicit user approval.
In QA mode, flag any code that doesn't match DESIGN.md.
