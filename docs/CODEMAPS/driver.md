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
| `src/direct_mode.cpp` / `.h` | `CDirectModeComponent` (`IVRDriverDirectModeComponent`) — D3D11 device, swap sets, SubmitLayer / Present → blit → encode | 240 + 100 |
| `src/display_component.h` | `CDisplayComponent` (`IVRDisplayComponent`) — render size, projection, eye viewports, no distortion | 80 |
| `src/display_geometry.h` | FOV → raw projection, eye viewports — pure, tested | 60 |
| `src/gpu_adapter.cpp` / `.h` | Pick the GPU (NVIDIA first), create the D3D11 device, LUID for SteamVR | 70 + 70 |
| `src/swap_textures.cpp` / `.h` | `SwapTextureSets` — shareable textures + DXGI shared handles for the compositor | 80 + 55 |
| `src/eye_blit.cpp` / `.h` | `EyeBlit` — draws an eye's region of a layer into NVENC's B8G8R8A8 input (scale, flip, sRGB) | 170 + 80 |
| `src/sync_texture.cpp` / `.h` | `SyncTexture` — the compositor's keyed mutex, held while Present reads the frame | 30 + 30 |
| `src/nvenc_encoder.cpp` / `.h` | NVENC session on EyeBlit's output, QP delta map, `encode()` | 330 + 110 |
| `src/driver_log.h` | `driverLog()` → SteamVR's vrserver.txt | 25 |
| `src/frame_pacer.h` | `FramePacer` — PostPresent waits out each frame's slot at the refresh rate — pure, tested | 45 |
| `src/touch_profile.h` | Oculus Touch identity, input paths and button mapping for the controllers — pure, tested | 110 |
| `src/qp_map.h` | `computeQpDeltaMap()` (foveated QP offsets) — testable pure function | ~110 |

Note: NVENC types come from the official `nvEncodeAPI.h` in
`third_party/nvenc` (SDK 12.2, MIT). The hand-written copies it replaced had
wrong struct versions, layouts and constants. `src/nvenc_config.h` holds the
settings applied on top of NVIDIA's preset, as pure functions the gtests
cover.

---

## Lifecycle

```
SteamVR loads DLL
  → HmdDriverFactory()
    → returns CServerDriver singleton (s_instance)

CServerDriver::Init()
  → fvp_init() (Rust engine start)
  → fvp_set_idr_callback / fvp_set_gaze_callback / fvp_set_bitrate_callback
    (a new bitrate is applied before the next encode: nvEncReconfigureEncoder)
  → create CHmdDevice + 2x CControllerDevice
  → TrackedDeviceAdded() for each

CHmdDevice::Activate()
  → CDirectModeComponent::init(): D3D11 device on the chosen GPU,
    EyeBlit output (encoded size), NvencEncoder on it (failure logged:
    SteamVR runs, nothing streams)
  → properties: display, Prop_GraphicsAdapterLuid_Uint64,
    Prop_DriverDirectModeSendsVsyncEvents_Bool = false (SteamVR times vsync)

per-frame (driven by SteamVR compositor):
  → CServerDriver::RunFrame → CHmdDevice / CControllerDevice pull
    fvp_get_tracking_data() / fvp_get_controller_state()
  → CDirectModeComponent::SubmitLayer() per layer (the first, the scene, is kept)
  → CDirectModeComponent::Present(syncTexture)
    → SyncTexture::acquire (the compositor's keyed mutex)
    → EyeBlit::draw(each eye of the layer → its half of the frame) → release
    → NvencEncoder::encode() → fvp_submit_encoded_nal()
  → CDirectModeComponent::PostPresent() — FramePacer: wait out the frame's
    slot at the refresh rate (as ALVR does)

CServerDriver::Cleanup()
  → fvp_shutdown()          (no more IDR / gaze callbacks)
  → s_instance = nullptr
  → destroy devices
```

---

## Key classes

### `CServerDriver` (server_driver.h)
- `vr::IServerTrackedDeviceProvider` implementation
- Owns: `m_hmd`, `m_leftController`, `m_rightController`, pose thread
- Static `s_instance` for IDR / gaze callbacks from Rust; `Cleanup()` stops
  the engine before clearing it and destroying the devices

### `CHmdDevice` (hmd_device.h)
- Sets `Prop_DisplayFrequency_Float` from `FvpConfig::refresh_rate`
- Sets `Prop_UserIpdMeters_Float` from `FvpConfig::ipd`
- Sets `Prop_GraphicsAdapterLuid_Uint64` so the compositor renders on the
  driver's GPU
- Provides `GetPose()` that returns the latest tracking data
- `GetComponent()` returns `CDirectModeComponent` and `CDisplayComponent`

### `CControllerDevice` (controller_device.h)
- Two instances (left / right) distinguished by `m_isLeft`
- Presented as Oculus Touch (Quest 2) so games' Touch bindings apply —
  identity, input paths and button mapping in `touch_profile.h` (tested)
- Updates SteamVR inputs via `VRDriverInput()->UpdateBooleanComponent` etc.;
  releases them once when the controller stops reporting
- `TriggerHaptic()` → `fvp_haptic_event(id, duration, frequency, amplitude)`

### `CDirectModeComponent` (direct_mode.h)
- `init()`: D3D11 device (`fvp_gpu::createDevice`), `EyeBlit`, `NvencEncoder`
- `CreateSwapTextureSet()` → `SwapTextureSets` (real DXGI shared handles)
- `SubmitLayer()` keeps the frame's first layer; `Present(syncTexture)`
  blits both eyes side by side under the sync texture's mutex, encodes, submits
- Not yet: compositing overlay layers (TODOS)

### `SwapTextureSets` / `EyeBlit` / `SyncTexture`
- D3D11 only, no OpenVR calls — tested on WARP (`tests/test_d3d_pipeline.cpp`)
  with a second device playing the compositor
- `EyeBlit` draws rather than copies: `CopyResource` is skipped between
  format groups (R8G8B8A8 → B8G8R8A8), can't scale (SteamVR supersampling)
  or pick one eye; sRGB / float sources are encoded to sRGB in the shader

### `NvencEncoder` (nvenc_encoder.h)
- Loads `nvEncodeAPI64.dll` + function pointer table
- Configures preset (low-latency HQ) + RC mode (CBR)
- Registers `EyeBlit`'s output as its input (`NV_ENC_BUFFER_FORMAT_ARGB`)
- `init()` returns false when NVENC is unavailable (logged to vrserver.txt);
  there is no fake test-pattern stream
- `setGaze(x, y, valid)` → triggers `computeQpDeltaMap()` for foveated
- `encode()` → bitstream buffer → NAL bytes for `fvp_submit_encoded_nal()`
- IDR trigger: atomic `s_idrRequested` flipped by Rust callback

---

## Tests (78 GoogleTest cases)

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

`driver/tests/test_d3d_pipeline.cpp` (WARP, no GPU needed): shared handles
the compositor can open, whole-set destroy, left/right eye from a
double-wide sRGB texture, scaling, flipped bounds, BGRA and float sources,
the sync texture's keyed mutex. `test_display_geometry.cpp`: projection,
eye viewports, GPU choice, LUID packing.

**Not tested**: `NvencEncoder` encode path (needs NVIDIA GPU),
`CDirectModeComponent` / `CHmdDevice` wiring (needs SteamVR).

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

- Nothing of this pipeline has run under SteamVR or on an NVIDIA GPU yet
- The FOV is a fixed default
  (`display_geometry.h`) until the headset reports its own
- Overlay layers (SteamVR dashboard) are not composited
- NVENC runs only on a driver that supports NVENC API 12.2+ (official header in `third_party/nvenc`)

---

## Extension surface

To add a new device (e.g., third controller, full-body tracker):
1. Subclass `ITrackedDeviceServerDriver`
2. Add `TrackedDeviceAdded` in `CServerDriver::Init`
3. Add the polling loop branch in the pose thread (read from Rust via `fvp_get_*`)
4. Extend `fvp-common::protocol` with new `TrackingData` variants or new msg types
