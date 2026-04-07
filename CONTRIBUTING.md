# Contributing

## Prerequisites

- Rust stable (cargo, rustc)
- CMake 3.18+
- Windows SDK (for driver DLL)
- Android SDK + NDK 26.1 (for client APK)
- NVIDIA GPU with NVENC support (for real encoding; test pattern fallback available)

## Build

```bash
# Full build (Windows)
./build.bat

# Individual components
cargo build --release -p streaming-engine
cargo build --release -p focus-vision-companion
cmake -B driver/build -S driver -DCMAKE_BUILD_TYPE=Release
cmake --build driver/build --config Release
```

## Test

```bash
cargo test --workspace    # All 156+ tests
```

Tests cover: FEC encode/decode, RTP packetization, TCP control handshake (plain + TLS), PIN pairing, TLS certificate generation, adaptive bitrate, audio encoding, config parsing + validation (range checks, NaN, port conflicts), ADB output parsing, PII sanitization, protocol struct layout, haptic event serialization + channel overflow, face tracking EMA smoothing + blendshape parsing, sleep mode motion detection + state transitions, tracking packet parsing (head pose, gaze, controller).

## Project Structure

See `ARCHITECTURE.md` for detailed system diagrams.

```
rust/streaming-engine/  — Core streaming library (Rust, C ABI)
rust/companion-app/     — PC GUI app (egui)
rust/common/            — Shared types
driver/                 — C++ OpenVR driver DLL
client/                 — Android OpenXR client (Kotlin + C++ NDK)
config/                 — TOML configuration
```

## Code Style

- Rust: standard `cargo fmt` / `cargo clippy`
- C++: C++17, consistent with existing codebase style
- Commit messages: `[type] Description` where type is `add`, `fix`, `test`, `docs`, `ci`, `security`, `release`

## Pull Requests

1. Create a feature branch from `main`
2. Ensure `cargo test --workspace` passes
3. Ensure CI passes (Rust + Companion + Driver + Android)
4. Keep commits focused — one logical change per commit
