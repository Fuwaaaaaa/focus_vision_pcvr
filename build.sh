#!/bin/bash
set -e

echo "============================================"
echo " Focus Vision PCVR - Build Script"
echo "============================================"

# Check dependencies
command -v cargo >/dev/null 2>&1 || { echo "ERROR: Rust/Cargo not found."; exit 1; }
command -v cmake >/dev/null 2>&1 || { echo "ERROR: CMake not found."; exit 1; }

# Step 1: Build Rust streaming engine
echo ""
echo "[1/3] Building Rust streaming engine..."
cargo build --release -p streaming-engine
echo "Rust build OK."

# Step 2: Build OpenVR driver DLL
echo ""
echo "[2/3] Building OpenVR driver..."
if [ -f driver/src/driver_main.cpp ]; then
    cmake -B driver/build -S driver -DCMAKE_BUILD_TYPE=Release
    cmake --build driver/build --config Release
    echo "Driver build OK."
else
    echo "SKIP: No driver sources yet (Step 2 pending)."
fi

# Step 3: Copy artifacts
echo ""
echo "[3/3] Copying artifacts..."
mkdir -p out
[ -f target/release/libstreaming_engine.a ] && cp target/release/libstreaming_engine.a out/
[ -f target/release/streaming_engine.lib ] && cp target/release/streaming_engine.lib out/

echo ""
echo "============================================"
echo " Build complete!"
echo "============================================"
echo "Artifacts in: out/"
