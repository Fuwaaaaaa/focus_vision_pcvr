# fvp-common (rust/common/) code map

> **Scope**: Cross-crate shared types, protocol structs, and constants. No
> runtime logic. Consumed by `streaming-engine`, `companion-app`, and the
> C++ driver via cbindgen-generated `streaming_engine.h`.

Small pure-data crate with 3 files. No I/O, no threads. All types are
`#[repr(C)]` where they cross FFI boundaries.

---

## Files

| Path | Purpose | LoC |
|---|---|---|
| `src/lib.rs` | Module re-exports (`protocol`, `constants`) | 4 |
| `src/protocol.rs` | Wire-format structs + version negotiation + enums | 430 |
| `src/constants.rs` | Port offsets, MTU, timeouts, blendshape counts | 31 |

---

## protocol.rs — key items

### Wire structs (`#[repr(C)]`)
| Type | Used for |
|---|---|
| `RtpHeader` | 12-byte RTP header fields (V/P/X/CC/M/PT/seq/ts/SSRC) |
| `FvpHeader` | 10-byte FVP header (frame_index, shard_index, shard_count, flags) |
| `TrackingData` | HMD pose + eye gaze (produced on HMD, consumed by C++ driver) |
| `ControllerState` | Per-controller input state (pose / buttons / thumbstick / battery) |

### Protocol versioning
- `PROTOCOL_VERSION: u16 = 3`
- `parse_hello_version(payload) -> u16` — parses HELLO/HELLO_ACK version header (defaults to 1 on missing)
- `encode_version(v) -> [u8; 2]` — LE encoding for HELLO payload

### Transport feedback (v3 addition)
- `TRANSPORT_FEEDBACK_MAX_ENTRIES = 256`
- `TransportFeedbackEntry { seq: u16, recv_ts_us: u32 }`
- `parse_transport_feedback(payload) -> Option<Vec<Entry>>`
- `encode_transport_feedback(&[Entry]) -> Vec<u8>`

### Flags helper module (`fvp_flags`)
- `encode_simple(is_keyframe: bool) -> u16` — v1/v2 compat
- `encode(is_keyframe, slice_index, slice_count, stream_id) -> u16` — v3 full
- `encode_compat(is_keyframe, slice_index, slice_count, stream_id, negotiated_version) -> u16` — version-aware gate
- `decode_slice_index(flags) -> u8`, `decode_slice_count(flags) -> u8`, etc.

### Enums
- `VideoCodec { H264, H265 }` (`#[repr(u8)]`, exported via cbindgen)

### Message type namespace (`msg_type` module)
`HELLO = 0x01`, `HELLO_ACK = 0x02`, `PIN_REQUEST = 0x10`, `PIN_RESPONSE = 0x11`,
`PIN_RESULT = 0x12`, `STREAM_CONFIG = 0x20`, `STREAM_START = 0x21`,
`HEARTBEAT = 0x30`, `HEARTBEAT_ACK = 0x31`, `IDR_REQUEST = 0x32`,
`TRANSPORT_FEEDBACK = 0x12` (collision note: numeric reuse intentional,
different direction), `FACE_DATA = 0x35`, `HAPTIC_EVENT = 0x38`,
`SLEEP_ENTER = 0x40`, `SLEEP_EXIT = 0x41`, `CONFIG_UPDATE = 0x55`,
`CONFIG_UPDATE_ACK = 0x56`, `CALIBRATE_START = 0x60`, `CALIBRATE_STATUS = 0x61`,
`DISCONNECT = 0x70`.

---

## constants.rs — key items

- `MTU_SIZE = 1400` — conservative IPv4 payload limit for Wi-Fi
- `DEFAULT_TCP_PORT = 9944`, `DEFAULT_UDP_PORT = 9945`
- Port offsets: `VIDEO_PORT_OFFSET = 1`, `TRACKING_PORT_OFFSET = 2`, `AUDIO_PORT_OFFSET = 3`
- `RTP_PT_H265 = 96`, `RTP_CLOCK_RATE = 90_000`
- `DEFAULT_FEC_REDUNDANCY = 0.2`
- `MAX_MSG_LEN = 256 * 1024` — TCP frame limit (reject oversized messages)
- `TOTAL_BLENDSHAPES = 51`
- `MAX_PIN_ATTEMPTS = 5`, `PIN_LOCKOUT_SECONDS = 300`

---

## Tests (23 total)

All in `#[cfg(test)] mod tests` at end of `protocol.rs`:
- 4 round-trip tests (RtpHeader / FvpHeader / TrackingData / ControllerState)
- 4 flags tests (simple / full / max / overlap)
- 3 versioning tests (encode / decode / empty payload)
- 5 transport feedback tests (roundtrip / empty / oversized / truncated / too_short)
- 3 fvp_flags compat gate (v1 / v2 / v3)
- 4 misc (codec enum, const sanity)

No integration tests — the crate is used only as a dependency; integration
is tested in `streaming-engine` + C++ side.

---

## External dependencies (Cargo.toml)

- `serde` (optional, for test TOML round-trips only)
- No tokio / runtime

---

## FFI boundary

Types exported to C via cbindgen in `streaming-engine/include/streaming_engine.h`:
- `TrackingData` → driver reads via `fvp_get_tracking_data()`
- `ControllerState` → driver reads via `fvp_get_controller_state()`
- `VideoCodec` enum → implicit via `FvpConfig::codec` field in `streaming-engine`

The driver side includes the header directly; the Android client uses the
same wire format but parses bytes manually (no cbindgen consumption).
