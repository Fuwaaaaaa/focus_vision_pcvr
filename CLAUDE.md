# Focus Vision PCVR

VIVE Focus Vision向けPCVRストリーミングツール。

## Architecture
- `rust/streaming-engine/` — Rust static library (C ABI via cbindgen)
- `driver/` — C++ OpenVR driver DLL (loaded by SteamVR)
- `client/` — Android OpenXR client (Kotlin + C++ NDK)
- `rust/common/` — Shared types and constants

## Build
```bash
./build.bat   # Windows
cargo build --release -p streaming-engine  # Rust only
cargo test --workspace  # Run tests
```

## Config
`config/default.toml` — override with `config/local.toml` (gitignored).

## Design System
Always read DESIGN.md before making any visual or UI decisions.
All font choices, colors, spacing, and aesthetic direction are defined there.
Do not deviate without explicit user approval.
In QA mode, flag any code that doesn't match DESIGN.md.
