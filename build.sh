#!/bin/bash
set -e
echo "=== Focus Vision PCVR Build ==="
command -v cargo >/dev/null 2>&1 || { echo "ERROR: Cargo not found."; exit 1; }
echo "[1/2] Building Rust streaming engine..."
cargo build --release -p streaming-engine
echo "Rust build OK."
echo "[2/2] Building OpenVR driver..."
if command -v cmake >/dev/null 2>&1 && [ -f driver/src/driver_main.cpp ]; then
    cmake -B driver/build -S driver -DCMAKE_BUILD_TYPE=Release
    cmake --build driver/build --config Release
    echo "Driver build OK."
else
    echo "SKIP: CMake not found or no driver sources."
fi
mkdir -p out
[ -f target/release/streaming_engine.lib ] && cp target/release/streaming_engine.lib out/
[ -f target/release/libstreaming_engine.a ] && cp target/release/libstreaming_engine.a out/
echo "Build complete!"
