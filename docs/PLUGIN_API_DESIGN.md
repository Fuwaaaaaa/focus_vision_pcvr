# Focus Vision PCVR — Plugin API Design (v3.0 Draft)

## Overview

A plugin system that allows community developers to extend the streaming pipeline
with custom data channels. Plugins can tap into the data flow (tracking, face tracking,
audio) and send/receive custom messages over the TCP control channel.

## Goals

1. Enable community extensions (body tracking relay, custom overlays, OSC routing)
2. No recompilation required — plugins are loaded at runtime
3. Security-first: plugins cannot access raw network sockets or bypass TLS

## Architecture

```
                    ┌─────────────────────┐
                    │   Plugin Manager    │
                    │ (load, unload, IPC) │
                    └──────┬──────────────┘
                           │
          ┌────────────────┼────────────────┐
          │                │                │
    ┌─────▼─────┐   ┌─────▼─────┐   ┌─────▼─────┐
    │  Plugin A  │   │  Plugin B  │   │  Plugin C  │
    │ Body Track │   │ OSC Router │   │  Overlay   │
    └───────────┘   └───────────┘   └───────────┘
```

## Message Type Registration

Plugins register custom message types in the reserved range `0x80-0xBF` (64 slots).
System messages use `0x00-0x7F` (current protocol). Range `0xC0-0xFF` is reserved for future use.

```rust
pub struct PluginMessageType {
    pub id: u8,                    // 0x80-0xBF
    pub name: String,              // Human-readable name for logging
    pub max_payload_bytes: usize,  // Per-message size limit (default: 4096)
    pub direction: Direction,      // ToHMD, FromHMD, Bidirectional
}

pub enum Direction {
    ToHMD,       // PC → HMD only
    FromHMD,     // HMD → PC only
    Bidirectional,
}
```

## Plugin Manifest

Plugins declare themselves via a JSON manifest:

```json
{
    "name": "body-tracking-relay",
    "version": "1.0.0",
    "author": "community-dev",
    "description": "Relays SlimeVR body tracking data to HMD via custom channel",
    "messages": [
        {
            "id": "0x80",
            "name": "BODY_TRACKING_DATA",
            "direction": "to_hmd",
            "max_payload_bytes": 512
        }
    ],
    "data_taps": ["tracking", "face_tracking"],
    "entry_point": "body_relay.dll"
}
```

## Data Pipeline Taps

Plugins can register read-only taps on existing data flows:

| Tap | Data | Frequency | Use Case |
|-----|------|-----------|----------|
| `tracking` | 6DoF head pose + controllers | Per-frame (90Hz) | Body tracking relay |
| `face_tracking` | 51 blendshapes (raw, pre-smoothing) | Per-frame | Custom OSC routing |
| `audio` | Opus frames | 100Hz (10ms frames) | Audio effects, lip sync |
| `latency` | FrameTimestamps | Per-frame | Performance monitoring |
| `config_change` | Key/value updates | On change | Dashboard sync |

Taps are **read-only**. Plugins cannot modify pipeline data.

## Security Boundaries

### Sandboxing

| Capability | Trusted | Standard |
|-----------|---------|----------|
| Custom TCP messages | Yes | Yes (registered types only) |
| Data taps | All | Declared in manifest only |
| File system | Read/write plugin dir | Read-only plugin dir |
| Network | localhost only | None |
| Config changes | Yes | No |

### Validation

- All plugin messages go through the existing `MAX_MSG_LEN` (64KB) check
- Per-message-type size limits enforced by the plugin manager
- Rate limiting: max 100 messages/second per plugin
- Message type conflicts: first-registered wins, late registrations are rejected
- Plugin crash isolation: plugins run in separate threads, panics are caught

### Trust Levels

- **Trusted:** Signed by the project maintainer. Full capabilities.
- **Standard:** Unsigned community plugins. Restricted capabilities.
- Plugins cannot escalate their trust level at runtime.

## Example: Body Tracking Relay

A plugin that receives SlimeVR body tracking data over localhost UDP
and relays it to the HMD via a custom TCP message:

```rust
// Plugin entry point
pub fn init(ctx: &mut PluginContext) {
    ctx.register_message(PluginMessageType {
        id: 0x80,
        name: "BODY_TRACKING_DATA".into(),
        max_payload_bytes: 512,
        direction: Direction::ToHMD,
    });

    ctx.register_tap("tracking", |data: &TrackingData| {
        // Read head tracking for relative body positioning
    });

    // Start SlimeVR UDP listener
    ctx.spawn(async move {
        let socket = UdpSocket::bind("127.0.0.1:6969").await?;
        loop {
            let (data, _) = socket.recv_from(&mut buf).await?;
            ctx.send_message(0x80, &data)?;
        }
    });
}
```

## Implementation Phases

1. **Phase 1 (v3.0):** Message type registration + basic plugin loading (DLL/SO)
2. **Phase 2 (v3.1):** Data pipeline taps + plugin manifest validation
3. **Phase 3 (v3.2):** Sandboxing, trust levels, plugin marketplace

## Open Questions

- Should plugins run in-process (DLL) or out-of-process (IPC)?
  - In-process: Lower latency, simpler data sharing
  - Out-of-process: Better isolation, crash protection
  - Recommendation: In-process for v3.0, migrate to out-of-process in v3.2
- WASM plugins? Would provide excellent sandboxing but add latency.
- Plugin hot-reload during streaming?
