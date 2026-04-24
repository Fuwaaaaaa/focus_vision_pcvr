# driver (C++ OpenVR driver) code map

> **Scope**: Windows SteamVR OpenVR driver DLL. Linked against `streaming-engine`
> via cbindgen-generated `include/streaming_engine.h`. See `ARCHITECTURE.md`
> for how this fits in the compositor → encode → network pipeline.

The DLL exports `HmdDriverFactory` (via `driver_main.cpp`) which SteamVR
loads on boot. The driver opens the Rust streaming engine, registers HMD
+ 2 controllers as tracked devices, and serves video via
`IVRDriverDirectModeComponent`.

---

## Files

| Path | Purpose | LoC |
|---|---|---|
| `src/driver_main.cpp` | `HmdDriverFactory` entry point, returns `CServerDriver` | ~30 |
| `src/server_driver.cpp` / `.h` | `CServerDriver` (`IServerTrackedDeviceProvider`) — Init/Cleanup lifecycle + SteamVR interface glue | 240 + 33 |
| `src/hmd_device.cpp` / `.h` | `CHmdDevice` (`ITrackedDeviceServerDriver`) — pose, IPD, refresh rate, component activation | 180 |
| `src/controller_device.cpp` / `.h` | `CControllerDevice` (`ITrackedDeviceServerDriver`) — input component, haptic via `fvp_haptic_event` | 180 |
| `src/direct_mode.cpp` / `.h` | `CDirectModeComponent` (`IVRDriverDirectModeComponent`) — CreateSwapTextureSet, Present, frame lifecycle | 150 + 40 |
| `src/frame_copy.cpp` / `.h` | D3D11 texture → pinned CPU buffer / CUDA buffer for NVENC input | 130 + 35 |
| `src/nvenc_encoder.cpp` / `.h` | NVENC session, QP delta map, `EncodeFrame()` → `fvp_submit_encoded_nal` | 470 + 320 |
| `src/qp_map.h` | `computeQpDeltaMap()` (foveated QP offsets) — testable pure function | ~110 |

Note: `nvenc_encoder.h` includes many `#[repr]` equivalents — inlined copies
of NVENC SDK structs (to avoid pulling the full NVIDIA SDK into the build).
This is a known fragility: field offsets must be kept in sync with the SDK.

---

## Lifecycle

```
SteamVR loads DLL
  → HmdDriverFactory()
    → returns CServerDriver singleton (s_instance)

CServerDriver::Init()
  → fvp_init() (Rust engine start)
  → fvp_set_idr_callback / fvp_set_gaze_callback / fvp_set_bitrate_callback
  → create CHmdDevice + 2x CControllerDevice
  → TrackedDeviceAdded() for each
  → spawn pose polling thread

per-frame (driven by SteamVR compositor):
  → CHmdDevice::PoseUpdated via pose thread pulling fvp_get_tracking_data()
  → CControllerDevice::InputUpdated via pose thread pulling fvp_get_controller_state()
  → CDirectModeComponent::Present()
    → FrameCopy::copy(texture)
    → NvencEncoder::encode() → fvp_submit_encoded_nal()

CServerDriver::Cleanup()
  → fvp_shutdown()
  → stop pose thread
  → s_instance = nullptr
```

---

## Key classes

### `CServerDriver` (server_driver.h)
- `vr::IServerTrackedDeviceProvider` implementation
- Owns: `m_hmd`, `m_leftController`, `m_rightController`, pose thread
- Static `s_instance` for IDR / gaze / bitrate callbacks from Rust
- Known race: callback can fire during `Cleanup()` while `s_instance` goes null — audit flagged

### `CHmdDevice` (hmd_device.h)
- Sets `Prop_DisplayFrequency_Float` from `FvpConfig::refresh_rate`
- Sets `Prop_UserIpdMeters_Float` from `FvpConfig::ipd`
- Provides `GetPose()` that returns the latest tracking data
- Activates `CDirectModeComponent` via `GetComponent()`

### `CControllerDevice` (controller_device.h)
- Two instances (left / right) distinguished by `m_role`
- Updates SteamVR inputs via `VRDriverInput()->UpdateBooleanComponent` etc.
- Battery % via `Prop_DeviceBatteryPercentage_Float`
- `TriggerHapticPulse()` → `fvp_haptic_event(role, 10ms, 200Hz, 1.0)` (amplitude hardcoded for now)

### `CDirectModeComponent` (direct_mode.h)
- `CreateSwapTextureSet()` allocates D3D11 textures
- `Present(PresentInfo*)` is the main per-frame hook
- Calls `FrameCopy` → `NvencEncoder::encode()`
- Returns present timing info to SteamVR

### `FrameCopy` (frame_copy.h)
- Takes D3D11 texture from `m_pendingTexture`
- Copies to staging buffer (pinned for NVENC) via `ID3D11DeviceContext::CopyResource`
- Thread safety: `ComPtr` based, no raw pointer escape

### `NvencEncoder` (nvenc_encoder.h)
- Loads `nvEncodeAPI64.dll` + function pointer table
- Configures preset (low-latency HQ) + RC mode (CBR)
- Supports H.264 and H.265 (selected via `FvpConfig::codec`)
- `setGaze(x, y, valid)` → triggers `computeQpDeltaMap()` for foveated
- `EncodeFrame(texture)` → bitstream buffer → `fvp_submit_encoded_nal()`
- IDR trigger: atomic `s_idrRequested` flipped by Rust callback

---

## Tests (7 GoogleTest)

`driver/tests/test_qp_map.cpp`:
- `ComputeQpDeltaMap_centerGaze_fovealZero` — gaze at (0,0) produces zero QP offset in fovea
- `ComputeQpDeltaMap_cornerGaze` — gaze at (1,1) produces expected offsets
- `ComputeQpDeltaMap_presetSubtle` — subtle preset applies +3/+8
- `ComputeQpDeltaMap_presetBalanced` — balanced +5/+15
- `ComputeQpDeltaMap_presetAggressive` — aggressive +8/+25
- `ComputeQpDeltaMap_gridSize` — CTU grid dimensions
- `ComputeQpDeltaMap_customValues` — preset=Custom uses config values directly

Build via `cd driver/build && cmake --build . && ctest`. Run on Windows only
(NVENC SDK / D3D11 dependencies).

**Not tested**: `NvencEncoder` encode path (needs NVIDIA GPU), `FrameCopy`
(needs D3D11 device), `CDirectModeComponent` (needs SteamVR). audit item
#12.

---

## External dependencies (CMakeLists.txt)

- `openvr_api.lib` (SteamVR SDK)
- `d3d11.lib` / `dxgi.lib` (Windows graphics)
- `nvEncodeAPI64.dll` (loaded at runtime via `LoadLibrary`, not linked)
- GoogleTest v1.15.2 via `FetchContent` for tests

Header-only consumption of `streaming_engine.h` (cbindgen output), linked
against `streaming_engine.lib` (cdylib import lib).

---

## Known issues (from audit)

- `server_driver.cpp:11,72,93` — `s_instance` nullptr race during shutdown callbacks (audit #10)
- `nvenc_encoder.cpp:17-18,102,341` — ComPtr `.Get()` for threading surface (audit #9)
- `direct_mode.cpp:38,48` — terse error handling, missing log detail (audit #30)
- NVENC SDK struct offsets — inlined copies must match NVIDIA SDK version (documented in TODOS.md)

---

## Extension surface

To add a new device (e.g., third controller, full-body tracker):
1. Subclass `ITrackedDeviceServerDriver`
2. Add `TrackedDeviceAdded` in `CServerDriver::Init`
3. Add the polling loop branch in the pose thread (read from Rust via `fvp_get_*`)
4. Extend `fvp-common::protocol` with new `TrackingData` variants or new msg types
