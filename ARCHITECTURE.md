# Architecture

Focus Vision PCVR — VIVE Focus Vision向けワイヤレスPCVRストリーミング。

## System Overview

```
PC (Windows)                           Wi-Fi               HMD (Focus Vision)
========================              ========             =======================
                                         |
  SteamVR                                |          OpenXR Runtime
    |                                    |              |
  OpenVR Driver DLL                      |          Android App
    |                                    |              |
  ┌─────────────────────┐               |          ┌──────────────────┐
  │  Direct Mode        │               |          │  OpenXR App      │
  │  ├─ FrameCopy       │               |          │  ├─ Renderer     │
  │  └─ NvencEncoder    │               |          │  ├─ Timewarp     │
  │     ├─ QP delta map │               |          │  ├─ VideoDecoder │
  │     └─ Foveated     │               |          │  ├─ AudioPlayer  │
  └────────┬────────────┘               |          │  ├─ FecDecoder   │
           │                             |          │  ├─ EyeTracker   │
  ┌────────▼────────────┐               |          │  └─ Overlay      │
  │  Streaming Engine   │               |          └──────┬───────────┘
  │  (Rust static lib)  │               |                 │
  │  ├─ Pipeline        │──── UDP ──────┼──── UDP ────────┤
  │  │  ├─ RTP          │  (video+FEC)  |  (video+FEC)    │
  │  │  ├─ FEC          │               |                 │
  │  │  └─ Transport    │──── UDP ──────┼──── UDP ────────┤
  │  ├─ Audio           │   (Opus)      |    (Opus)       │
  │  │  ├─ Capture      │               |                 │
  │  │  └─ Encoder      │               |                 │
  │  ├─ Control         │──── TLS ──────┼──── TLS ────────┤
  │  │  ├─ TCP Server   │  (port 9944)  |  (port 9944)    │
  │  │  ├─ Pairing      │               |                 │
  │  │  └─ TLS          │               |                 │
  │  ├─ Adaptive        │               |                 │
  │  │  └─ Bitrate      │◄── UDP ──────┼──── UDP ────────┤
  │  ├─ Tracking        │  (head pose)  |  (head pose)    │
  │  │  └─ Receiver     │               |                 │
  │  └─ Codec Benchmark │               |                 │
  └─────────────────────┘               |                 │
                                         |                 │
  ┌─────────────────────┐               |                 │
  │  Companion App      │               |                 │
  │  (egui single .exe) │               |                 │
  │  ├─ Driver Install  │               |                 │
  │  ├─ PIN Display     │               |                 │
  │  ├─ ADB Deploy      │── USB ────────┼─────────────────┘
  │  ├─ Codec Toggle    │               |
  │  ├─ Latency Graph   │               |
  │  └─ Log Export      │               |
  └─────────────────────┘
```

## Crate / Module Map

```
focus_vision_psvr/
├── rust/
│   ├── streaming-engine/     Rust static library (C ABI via cbindgen)
│   │   ├── config.rs         TOML config loading
│   │   ├── engine.rs         Core engine state, FFI callbacks
│   │   ├── lib.rs            C FFI exports (fvp_*)
│   │   ├── pipeline.rs       Frame → RTP+FEC packets
│   │   ├── codec_benchmark.rs  Auto H.264/H.265 selection
│   │   ├── control/
│   │   │   ├── tcp_server.rs   TLS TCP control channel
│   │   │   ├── pairing.rs     6-digit PIN + lockout
│   │   │   └── tls.rs         Self-signed cert + TlsAcceptor
│   │   ├── transport/
│   │   │   ├── rtp.rs         RTP packetization
│   │   │   ├── fec.rs         Reed-Solomon FEC
│   │   │   └── udp.rs         UDP send/recv
│   │   ├── audio/
│   │   │   ├── capture.rs     WASAPI loopback
│   │   │   └── encoder.rs     Opus encoding
│   │   ├── adaptive/
│   │   │   ├── bandwidth_estimator.rs  EWMA loss tracking
│   │   │   └── bitrate_controller.rs  CBR adjustment
│   │   ├── tracking/
│   │   │   └── receiver.rs    UDP head pose + gaze
│   │   └── metrics/
│   │       └── latency.rs     Frame timestamp tracking
│   │
│   ├── companion-app/        PC GUI app (egui, single .exe)
│   │   ├── main.rs           3-tab UI: Home/Deploy/Settings
│   │   ├── config.rs         local.toml read/write
│   │   ├── adb.rs            ADB device management
│   │   ├── driver.rs         SteamVR driver install
│   │   ├── stats_history.rs  30s ring buffer for graphs
│   │   └── export.rs         Log zip + PII sanitization
│   │
│   └── common/               Shared types
│       ├── protocol.rs       RTP/FVP headers, msg types
│       └── constants.rs      Ports, MTU, timeouts
│
├── driver/                   C++ OpenVR driver DLL
│   └── src/
│       ├── direct_mode.cpp   SteamVR DirectMode component
│       ├── nvenc_encoder.cpp NVENC H.265/H.264 + foveated QP
│       └── nvenc_encoder.h   Inline NVENC API structs
│
├── client/                   Android OpenXR client
│   └── app/src/main/
│       ├── cpp/
│       │   ├── openxr_app.cpp    Main app loop
│       │   ├── video_decoder.cpp JNI SurfaceTexture zero-copy
│       │   ├── audio_player.cpp  Opus + AAudio
│       │   ├── fec_decoder.cpp   RS recovery (GF(2^8))
│       │   ├── tcp_client.cpp    TLS TCP (MbedTLS)
│       │   ├── tracking_sender.cpp  UDP head+gaze
│       │   ├── timewarp.cpp      Rotation correction
│       │   ├── overlay_renderer.cpp Signal bar overlay
│       │   └── eye_tracker.cpp   XR_EXT_eye_gaze
│       └── kotlin/
│           └── MainActivity.kt   NativeActivity loader
│
└── config/
    ├── default.toml          Shipping defaults
    └── local.toml            User overrides (gitignored)
```

## Video Pipeline

```
                         PC                                              HMD
  ┌──────────────────────────────────┐          ┌──────────────────────────────────┐
  │                                  │          │                                  │
  │  SteamVR compositor              │          │  UDP recv (port 9946)            │
  │       │                          │          │       │                          │
  │  D3D11 texture (BGRA)            │          │  RTP depacketize                 │
  │       │                          │          │       │                          │
  │  FrameCopy (GPU copy)            │          │  FEC decode (Reed-Solomon)       │
  │       │                          │          │       │                          │
  │  NVENC encode (H.265/H.264)      │          │  NAL validate                    │
  │       │  ┌── foveated? ──┐       │          │       │                          │
  │       │  │ QP delta map  │       │          │  MediaCodec decode               │
  │       │  │ (eye gaze)    │       │          │       │  ┌── SurfaceTexture ──┐  │
  │       │  └───────────────┘       │          │       │  │ JNI zero-copy      │  │
  │       │                          │          │       │  └────────────────────┘  │
  │  NAL data                        │          │  GL_TEXTURE_EXTERNAL_OES         │
  │       │                          │          │       │                          │
  │  RTP packetize + FVP header      │          │  Timewarp (rotation correction)  │
  │       │                          │          │       │                          │
  │  FEC encode (20% redundancy)     │          │  OpenXR swapchain render         │
  │       │                          │          │       │                          │
  │  UDP send (port 9946)     ───────┼── Wi-Fi──┼───────┘                          │
  │                                  │          │                                  │
  └──────────────────────────────────┘          └──────────────────────────────────┘

  Latency budget: 50ms target
  ├─ Encode:    3-5ms (NVENC hardware)
  ├─ Network:   2-5ms (Wi-Fi 6)
  ├─ Decode:    3-8ms (MediaCodec hardware)
  ├─ Timewarp:  <1ms (GPU shader)
  └─ Buffer:    remaining (~30ms)
```

## Audio Pipeline

```
  PC                                    HMD
  ┌──────────────────┐                 ┌──────────────────┐
  │ WASAPI loopback  │                 │ UDP recv (9948)   │
  │ (system audio)   │                 │      │            │
  │      │           │                 │ Opus decode       │
  │ 48kHz stereo PCM │                 │      │            │
  │      │           │                 │ AAudio write      │
  │ Opus encode      │                 │ (low-latency)     │
  │ (128kbps, 10ms)  │                 │      │            │
  │      │           │                 │ HMD speakers      │
  │ UDP send (9948)  │── Wi-Fi ────────│                   │
  └──────────────────┘                 └──────────────────┘
```

## Control Channel (TLS)

```
  Handshake flow (port 9944, TLS 1.3):

  Client (HMD)                    Server (PC)
      │                               │
      │◄──── TLS ClientHello ─────────│
      │───── TLS ServerHello ────────►│
      │      [ephemeral self-signed]  │
      │                               │
      │───── HELLO (v1.0) ───────────►│
      │◄──── HELLO_ACK ──────────────│
      │◄──── PIN_REQUEST ────────────│
      │                               │
      │  [user enters 6-digit PIN]    │
      │                               │
      │───── PIN_RESPONSE (u32 LE) ──►│
      │◄──── PIN_RESULT (OK/NG) ─────│
      │◄──── STREAM_CONFIG ──────────│
      │───── STREAM_START ───────────►│
      │                               │
      │◄───► HEARTBEAT (500ms) ──────│
      │      [latency, loss, fps]     │
      │                               │

  Security:
  - TLS 1.3 (rustls server, MbedTLS client)
  - 6-digit PIN (cryptographic RNG, 1M combinations)
  - 5 attempts then 300s lockout
  - TOFU certificate pinning (SHA-256 fingerprint)
```

## Foveated Encoding

```
  Eye gaze → QP delta map → NVENC

  HMD EyeTracker                          PC NvencEncoder
  ┌────────────┐                          ┌────────────────┐
  │ XR_EXT_    │  UDP tracking packet     │ setGaze(x,y)   │
  │ eye_gaze   │─── (46 bytes) ──────────►│      │          │
  │            │  [pose + gaze_x,y,valid] │ computeQpMap() │
  └────────────┘                          │      │          │
                                          │  ┌───▼────────┐ │
                                          │  │ CTU grid   │ │
   Quality zones:                         │  │            │ │
   ● Fovea (r=15%): QP+0  (full)        │  │  ●···      │ │
   ◐ Mid   (r=35%): QP+5  (soft blur)   │  │ ◐◐●◐◐     │ │
   ○ Periph (>35%): QP+15 (compress)    │  │ ○◐◐◐○     │ │
                                          │  │ ○○○○○     │ │
                                          │  └────────────┘ │
                                          │ picParams.qpMap │
                                          └────────────────┘
```

## CI / Distribution

```
  GitHub Actions (on push to main + tags):

  ┌─────────────────┐  ┌──────────────────┐  ┌─────────────────┐
  │ Rust Build       │  │ Companion Build   │  │ Android Build    │
  │ (windows-latest) │  │ (windows-latest)  │  │ (ubuntu-latest)  │
  │                  │  │                   │  │                  │
  │ cargo test       │  │ cargo build       │  │ gradle           │
  │ cargo build      │  │ + fonts download  │  │ assembleDebug    │
  │                  │  │ + config bundle   │  │                  │
  │ streaming_engine │  │ focus-vision.exe  │  │ *.apk            │
  │ .lib             │  │ + fonts/ + config/│  │                  │
  └────────┬─────────┘  └────────┬──────────┘  └────────┬─────────┘
           │                     │                       │
  ┌────────▼─────────┐          │                       │
  │ Driver Build      │          │                       │
  │ (windows-latest)  │          │                       │
  │                   │          │                       │
  │ cmake + link .lib │          │                       │
  │                   │          │                       │
  │ driver DLL        │          │                       │
  └────────┬──────────┘          │                       │
           │                     │                       │
           ▼                     ▼                       ▼
  ┌──────────────────────────────────────────────────────────┐
  │                   GitHub Release (on v* tag)              │
  │                                                          │
  │  FocusVision-Companion-v1.1.0.zip                        │
  │  FocusVision-Driver-v1.1.0.zip                           │
  │  FocusVision-Client-v1.1.0.apk                           │
  └──────────────────────────────────────────────────────────┘
```

## Test Coverage

104 tests (all passing):
- **streaming-engine**: 66 (unit + integration)
- **companion-app**: 25 (config, ADB, stats, export, PII)
- **common**: 6 (protocol structs, flags, msg types)
- **integration**: 7 (full video pipeline RTP/FEC roundtrip)
