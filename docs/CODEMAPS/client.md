# client (Android OpenXR app) code map

> **Scope**: On-HMD Android native app that runs on VIVE Focus Vision. Pairs
> with the PC driver, receives video/audio/control over Wi-Fi, renders via
> OpenXR with timewarp.

Native activity (C++/NDK) with a trivial Kotlin shell. All app logic is
C++; Kotlin only inherits `NativeActivity` and loads the `.so`.

---

## Entry point

| Path | Purpose |
|---|---|
| `kotlin/com/focusvision/pcvr/MainActivity.kt` | `class MainActivity : NativeActivity()` — 9 LoC, loads `libfvp_client.so` |
| `cpp/main.cpp` | `android_main()` entry, global `static OpenXRApp* g_app`, `handleAppCmd` dispatcher | 55 |

---

## Main orchestrator

### `cpp/openxr_app.cpp` / `.h` (577 + 146 LoC)
`class OpenXRApp` — the "god object" that owns every sub-component and
runs the main loop.

Lifecycle: `initialize()` → `mainLoop()` → `shutdown()`

Initialization steps (`initialize()`):
1. `createInstance()` — `xrCreateInstance` with Android loader + required extensions
2. `getSystem()` — `xrGetSystem` + `xrEnumerateViewConfigurationViews`
3. `initEGL()` — EGL display / context / pbuffer surface (GLES 3.0)
4. `createSession()` — `xrCreateSession` with OpenGL ES Android binding
5. `createReferenceSpace()` — `XR_REFERENCE_SPACE_TYPE_STAGE`
6. `createSwapchains()` — per-eye via `XrSwapchainWrapper`
7. Init sub-components (Renderer / Timewarp / Overlay / Heartbeat / FacialTracker / VideoDecoder)

Main loop per-frame:
- `pollEvents()` / `pollAndroidEvents()`
- `receiveAndDecodeVideo()` (UDP → FEC → NAL → MediaCodec)
- `renderFrame()` (OpenXR frame begin → timewarp on decoded texture → submit)
- Heartbeat 500 ms interval
- Battery level poll every 30 s

Owns ~30+ members: OpenXR handles, EGL state, per-eye swapchains, renderer,
timewarp, overlay, network receiver, FEC decoders (bulk + slice),
video decoder, audio player, TCP client, tracking sender, controller
poller, eye tracker, HMD profile, heartbeat, stats reporter, facial
tracker, pose history, pairing state, dashboard state.

---

## Sub-components

### Networking
| File | Class | Role |
|---|---|---|
| `tcp_client.h/.cpp` | `TcpControlClient` | MbedTLS TCP control channel, HELLO/PIN/STREAM_START/IDR_REQUEST/CONFIG_UPDATE |
| `network_receiver.h/.cpp` | `NetworkReceiver` | Non-blocking UDP recv (used for both video and audio ports) |
| `heartbeat_client.h` | `HeartbeatClient` | 500 ms heartbeat → PC, reads StatsReporter |
| `stats_reporter.h` | `StatsReporter` | Packet loss / RTT / frame count tracking |
| `tracking_sender.h/.cpp` | `TrackingSender` | UDP head pose + eye gaze to PC, 90 Hz |

### Video pipeline
| File | Class | Role |
|---|---|---|
| `fec_decoder.h/.cpp` | `FecFrameDecoder` / `SlicedFecFrameDecoder` | Reed-Solomon recovery. Sliced version has 4 independent RS contexts, u32 length prefix, 100 ms timeout |
| `nal_validator.h/.cpp` | `NalValidator` | Sanity-check NAL unit header before feeding MediaCodec |
| `video_decoder.h/.cpp` | `VideoDecoder` | MediaCodec via JNI + SurfaceTexture zero-copy path |
| `timewarp.h/.cpp` | `Timewarp` | GL_TEXTURE_EXTERNAL_OES rotation correction shader |
| `renderer.h/.cpp` | `Renderer` | Final composition to OpenXR swapchain images |

### Audio
| File | Class | Role |
|---|---|---|
| `audio_player.h/.cpp` | `AudioPlayer` | Opus decode (libopus) + AAudio low-latency output |

### HMD I/O
| File | Class | Role |
|---|---|---|
| `controller_poller.h/.cpp` | `ControllerPoller` | OpenXR action set poll for trigger / grip / thumbstick / touch / battery |
| `eye_tracker.h/.cpp` | `EyeTracker` | `XR_EXT_eye_gaze_interaction` gaze pose |
| `facial_tracker.h/.cpp` | `FacialTracker` | HTC OpenXR facial tracking extension (lip + eye blendshapes → 51 floats) |
| `pose_history.h` | `PoseHistory` | Ring buffer of recent head poses for timewarp blend |
| `hmd_profile.h/.cpp` | `HmdProfile` / `DisplayProfile` / `CodecProfile` | Per-HMD static data (IPD, refresh, supported codecs) |

### Rendering
| File | Class | Role |
|---|---|---|
| `xr_swapchain.h/.cpp` | `XrSwapchainWrapper` | Per-eye OpenXR swapchain create / acquire / release / wait |
| `overlay_renderer.h/.cpp` | `OverlayRenderer` | Pairing overlay, connection quality indicator, latency waterfall |

### Utilities
| File | Purpose |
|---|---|
| `platform_defines.h` | `LOGI/LOGW/LOGE` android_log wrappers |
| `xr_utils.h` | `XR_CHECK` error helper |

---

## Pairing flow (PairingState enum)

```
Idle → Searching (TCP connect)
     → PinEntry (HELLO_ACK received)
     → Verifying (PIN_RESPONSE sent)
     → [Failed (wrong PIN, decrement attempts) → PinEntry]
     → [LockedOut (5 wrong attempts, 300s countdown)]
     → Connected (STREAM_CONFIG received, stream starts)
     → Disconnected (TCP drop or timeout)
```

UI renders via `OverlayRenderer` as a quad overlay layer on the OpenXR
composition stack.

---

## Tests

**Currently 0** — all 17 .cpp files are untested.

Highest-value test targets (audit item #11):
- `fec_decoder.cpp` — RS decode correctness for synthetic lost shards
- `nal_validator.cpp` — rejection of malformed NAL headers
- `sliced_fec` timeout + length-prefix validation
- `PoseHistory` ring buffer behavior (deterministic, no external deps)

Blocking factor: test framework setup on Android NDK build. GoogleTest
works but needs separate CMake target + `cargo ndk` integration.

---

## Build

- `CMakeLists.txt` (in cpp/): configures NDK build, links OpenXR loader,
  MbedTLS, libopus (via NDK dep), Android log/EGL/GLESv3/AAudio
- `.gradle` glue runs `cargo ndk` to cross-compile `streaming-engine`'s
  companion types (though most protocol parsing is redone here manually in C++)

---

## Known issues (from audit)

- `openxr_app.cpp:42` — `AttachCurrentThread` without `DetachCurrentThread` (audit #1 — fix PR #42)
- `openxr_app.cpp:110,127,142,147` — EGL init errors don't propagate (audit #2 — fix PR #43)
- `main.cpp:5` — `static OpenXRApp* g_app` race during shutdown callback (audit #3)
- `video_decoder.h:40-46` + `audio_player.h:44,47` — type-unsafe `void*` members (audit #4)
- `openxr_app.cpp` + `.h` — 32+ members, split candidate (audit #18)
- `MainActivity.kt` — 9 LoC, all logic delegated to C++ (audit #25)
- `video_decoder.cpp:78` — `ExceptionCheck / ExceptionClear` without logging (audit #29)

---

## Extension surface

To consume a new message type from PC:
1. Add constant in `fvp-common::protocol::msg_type`
2. Add parse branch in `TcpControlClient::processMessages` (tcp_client.cpp)
3. Forward to the right sub-component
4. Wire version gate if v3-only
