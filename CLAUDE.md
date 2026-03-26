# Focus Vision PCVR

VIVE Focus Vision向けPCVRストリーミングツール。

## Architecture

- `rust/streaming-engine/` — Rust static library (C ABI via cbindgen). Handles encoding, networking, FEC.
- `driver/` — C++ OpenVR driver DLL. Loaded by SteamVR. Links to Rust static lib.
- `client/` — Android OpenXR client (Kotlin + C++ NDK). Runs on VIVE Focus Vision.
- `rust/common/` — Shared types and constants between Rust crates.

## Build

```bash
# Full build (Rust + C++ driver)
./build.bat      # Windows
./build.sh       # Linux/CI

# Rust only
cargo build --release -p streaming-engine

# Run tests
cargo test --workspace
```

## Config

Settings in `config/default.toml`. Override with `config/local.toml` (gitignored).

## Design Doc

`~/.gstack/projects/focus-vision-psvr/柳田風和-main-design-20260326-133154.md`
