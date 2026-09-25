# streaming-engine code map

> **Scope**: モジュール内部の構造 (ファイル・主要型・関数) を一覧化。
> システム間のデータフロー・プロトコルは `ARCHITECTURE.md` を参照。

Rust static library built as `cdylib` via cbindgen. Loaded by the C++ OpenVR
driver (`driver/`) and also used by the companion app (`rust/companion-app/`)
through shared types in `rust/common/`.

---

## Entry points

| Path | Symbol | Purpose |
|---|---|---|
| `src/lib.rs` | `fvp_init` / `fvp_shutdown` | C FFI init / teardown from driver |
| `src/lib.rs` | `fvp_submit_encoded_nal` | NVENC → engine NAL submission (+ recording tap) |
| `src/lib.rs` | `fvp_get_tracking_data` / `fvp_get_controller_state` | pose/controller read from C++ thread |
| `src/lib.rs` | `fvp_set_*_callback` | register driver-side callbacks (IDR, gaze, bitrate) |
| `src/lib.rs` | `fvp_haptic_event` | queue haptic from SteamVR → HMD |
| `src/lib.rs` | `fvp_get_config` | driver reads effective config struct |
| `src/lib.rs` | `write_status_file` | JSON IPC to companion app |

Module declarations: `src/lib.rs:1-14` (14 modules).

---

## Core

### `src/engine.rs` (~1800 LoC + tests)
- `StreamingEngine` — owner of tokio runtime, frame channel, tracking state,
  latency tracker, cancel token, optional recording Arc<Mutex<Recorder>>
- `EncodedFrame` / `HmdStats` / `HapticEvent`
- `run_streaming()` → `StreamingLoop` (engine-lifetime state: reconnect
  counters, thermal governor, frame receiver):
  - `run()` — accept → session → hold loop, disconnect-reason bookkeeping
  - `accept()` — backoff + `TcpControlServer::listen_and_accept` with the PIN heartbeat
  - `run_session()` — per-session setup (tracking peer, control task, UDP,
    audio) and the frame loop
  - `hold()` — 5 s reconnect window on the session's server/PIN; a reconnect
    comes back as the next session
- `ControlChannel` + `handle_tcp_control()` — TCP control task (heartbeat,
  face data, transport feedback, CONFIG_UPDATE, disconnect; haptic/sleep out).
  HEARTBEAT stats and TRANSPORT_FEEDBACK go to the frame loop as
  `ControlEvent`s over an mpsc channel — no shared locks
- `AdaptiveState` — frame-loop-owned bandwidth/bitrate/burst/GCC/adaptive-FEC/
  sleep state; `on_event()` and a per-second `tick()`
- `VideoSender` — packetizer + `FrameFecEncoder` + UDP + send-time log
- `check_sleep_mode()` / `update_latency_atomics()` / `log_periodic_stats()`
- `spawn_audio_pipeline()` — audio capture→Opus→RTP UDP
- `init_recorder()` / `recording_output_dir()` — Session Recording setup
- Callback statics: `IDR_CALLBACK` / `GAZE_CALLBACK` / `BITRATE_CALLBACK`

### `src/config.rs` (~820 LoC)
- `AppConfig` (9 sub-configs)
- `ConfigError` / `validate()` with `validate_range` / `validate_f32_range` helpers
- Default impls mirror serde default fns (consistency verified by
  `test_default_matches_empty_toml_parse`)
- Sub-configs: `NetworkConfig`, `VideoConfig`, `AudioConfig`, `PairingConfig`,
  `DisplayConfig`, `FoveatedConfig` (+ `FoveatedPreset` enum),
  `FaceTrackingConfig`, `SleepModeConfig`, `MemoryMonitorConfig`, `RecordingConfig`

### `src/pipeline.rs` (~550 LoC)
- `encode_frame_to_packets` — fallback (no FEC)
- `encode_frame_to_packets_with_fec` — bulk FEC path
- `encode_frame_sliced` — slice-based FEC (all slices or nothing; never an empty slice)
- `choose_fec_layout` / `FecLayout` — per frame: bulk, sliced(n) or unprotected. Slices for
  frames ≥ `MIN_SLICE_SIZE` = 16 KB, count raised from `slice_count` up to 15 until each slice
  fits one RS code word; an over-limit bulk frame is sliced too
- `FrameFecEncoder` — the engine's per-session sender: bulk + 15 slice `FecEncoder`s at one
  redundancy, `encode()` returns one packet batch per code word
- `decode_packets_to_frame` — low-level decode with caller-supplied shard counts (integration tests)
- `FecFrameReassembler` — header-driven receiver (bulk + sliced, RS recovery, emits each frame once); the simulator mock HMD's video path
- All paths write `data_shard_count` into every packet's FVP header (v4) via the shared `shards_to_packets`
- `MIN_SLICE_SIZE` constant

---

## transport/

| File | Key items | Tests |
|---|---|---|
| `rtp.rs` | `write_rtp_header`, `write_fvp_header`, `read_fvp_header`, `FvpHeader` (12 B, `is_valid`), `RtpPacketizer`, `RtpDepacketizer`, `ReassembledFrame` | 14 |
| `fec.rs` | `FecEncoder` (`parity_for`, `max_data_shards` = RS limit at the current redundancy), `FecDecoder`, `AdaptiveFecController` (burst-boost aware, rate-limited), `FecError`, `RS_MAX_TOTAL_SHARDS`; cross-language golden parity vectors shared with `client/tests/test_fec_decoder.cpp` | 20 |
| `slice.rs` | `SliceSplitter` (NAL → N byte-aligned slices) | 8 |
| `udp.rs` | `UdpSender` / `UdpReceiver` (SO_RCVBUF/SNDBUF 2 MB, DSCP EF) | 2 |

---

## adaptive/

| File | Key items | Responsibility |
|---|---|---|
| `bandwidth_estimator.rs` | `BandwidthEstimator` | EWMA loss tracking + RTT (single-responsibility after PR splitting delay into gcc_estimator) |
| `bitrate_controller.rs` | `BitrateController` | CBR adjustment; `adjust(bw, gcc, burst) -> bool`; max-reduction model |
| `gcc_estimator.rs` | `GccEstimator` (`DelayTrend`, `bitrate_multiplier`) | delay-based bandwidth estimation (`process_feedback`) |
| `burst_detector.rs` | `BurstDetector` (`LossPattern` enum) | Wi-Fi burst vs sustained classifier (500 ms threshold by default; `new_with_thresholds` for tests) |

---

## control/

| File | Key items |
|---|---|
| `tcp_server.rs` | `TcpControlServer` (`listen_and_accept` rotates an expired PIN between connections; `pin_status` for status.json; `rearm_for_reconnect` for the hold), step_hello_exchange / step_pin_pairing / step_stream_config / step_stream_start, AsyncStream trait, read_message_generic / send_message_generic |
| `pairing.rs` | `PairingState` (6-digit PIN, 5 attempts, 300 s lockout, CSPRNG, 300 s PIN lifetime via `expires_in` / `rotate_if_expired`, `rearm_for_reconnect`) |
| `tls.rs` | self-signed cert + `TlsAcceptor`, SHA-256 fingerprint |

---

## audio/

| File | Key items |
|---|---|
| `capture.rs` | `AudioCapture` (cpal WASAPI loopback) |
| `encoder.rs` | `AudioEncoder` (libopus, 48 kHz stereo, 10 ms frames) |

---

## face_tracking/

| File | Key items |
|---|---|
| `osc_bridge.rs` | `OscBridge` (EMA smoothing via `apply_smoothing_and_send` helper, VRChat OSC) |
| `profiles.rs` | `FtProfile`, `validate()` (normalize + sanitize), JSON save/load |
| `calibration.rs` | `CalibrationState`, 2-step guided (Relax → ExaggerateAll) |
| `mod.rs` | HTC blendshape indices, `TOTAL_BLENDSHAPES = 51` |

---

## tracking/

| File | Key items |
|---|---|
| `receiver.rs` | `TrackingReceiver` (UDP, head pose + eye gaze + controllers) |

---

## metrics/

| File | Key items |
|---|---|
| `latency.rs` | `FrameTimestamps` / `LatencyTracker` (encode/network/decode/render) |
| `session_log.rs` | `SessionLogger` (JSONL, 60 s flush, 7-day rotation, single write_all) |
| `memory.rs` | `MemoryMonitor` (GetProcessMemoryInfo / /proc/self/status, 1-hour delta threshold) |

---

## recording/

| File | Key items |
|---|---|
| `mod.rs` | `Recorder` (Annex B .h265/.h264, best-effort, poisoned flag) |
| `audio.rs` | `AudioRecorder` (16-bit PCM WAV, seek-patched header on close) |

---

## Other

| File | Key items |
|---|---|
| `src/video/test_pattern.rs` | `generate_nv12_frame` (integration test helper) |
| `src/sleep_mode.rs` | `SleepDetector`, `SleepTransition` |
| `src/codec_benchmark.rs` | auto H.264 vs H.265 selection |

---

## Tests layout

- **In-crate unit tests**: `#[cfg(test)] mod tests` at file end of most modules (272+ tests)
- **Integration tests**: `tests/video_pipeline_test.rs` (7), `tests/recording_test.rs` (5)
- **Fuzz targets**: `fuzz/fuzz_targets/*.rs` (fuzz_rtp, fuzz_fec, fuzz_protocol, fuzz_config, fuzz_slice, fuzz_recording)
- **Benches**: `benches/` via Criterion (RTP, FEC, adaptive FEC, config, memory, slice FEC)

Run:
```bash
cargo test --workspace                           # all unit + integration
cargo bench -p streaming-engine --no-run         # benches compile
cd rust/streaming-engine && cargo fuzz list       # list fuzz targets
```

---

## C FFI exports (for driver/client interoperability)

Generated via `build.rs` (cbindgen) → `include/streaming_engine.h`. Stable ABI
consumed by `driver/src/*.cpp`.

Exported types/consts:
- `TrackingData`, `ControllerState` (from `fvp-common::protocol`)
- `FvpConfig` (in `src/lib.rs`, maps TOML → C)
- `MIN_SLICE_SIZE`, `TOTAL_BLENDSHAPES`

All `fvp_*` functions use `#[no_mangle] pub extern "C"`.

---

## Known large files / refactor candidates

| File | LoC | Notes |
|---|---|---|
| `src/engine.rs` | ~1800 + tests | `run_streaming` は `StreamingLoop::{run, accept, run_session, hold}` に分割済み (2026-09)。`run_session` (~200 LoC) はさらに分けられる |
| `src/config.rs` | ~820 | Default impl boilerplate 残存（意図的：明示的な Rust 値として保持、整合性はテストで担保） |
| `src/pipeline.rs` | ~550 | encode variants ×3 は責務分離済み |
