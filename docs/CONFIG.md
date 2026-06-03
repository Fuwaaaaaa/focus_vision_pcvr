# Focus Vision PCVR — Configuration Reference

Config file: `config/default.toml` (override with `config/local.toml`, gitignored).
All values are validated on startup. Invalid values are clamped to defaults with a warning.

## `[network]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `tcp_port` | u16 | 9944 | >= 1024 | TCP control channel port (TLS handshake, PIN pairing, heartbeat, face tracking) |
| `udp_port` | u16 | 9945 | >= 1024, != tcp_port | Base UDP port. Video = udp_port + VIDEO_PORT_OFFSET, Audio = udp_port + AUDIO_PORT_OFFSET |
| `fec_redundancy` | f32 | 0.2 | 0.0-1.0 | FEC parity ratio. 0.2 = 20% parity shards added to each frame |

**Validation:** tcp_port and udp_port must be >= 1024. If they're equal, udp_port is auto-incremented. Below 1024 is clamped to default.

## `[video]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `codec` | string | "h265" | "h264", "h265" | Video codec. H.265 = better compression, H.264 = faster decode on some devices |
| `bitrate_mbps` | u32 | 80 | 10-200 | Target bitrate in Mbps. Adaptive bitrate may adjust this at runtime |
| `resolution_per_eye` | [u32; 2] | [1832, 1920] | — | Per-eye render resolution [width, height]. Must match SteamVR render target |
| `framerate` | u32 | 90 | 30-120 | Target framerate. Supported: 72, 90, 96, 120 |
| `full_range` | bool | true | — | Full RGB (0-255) vs limited range (16-235). Affects NVENC VUI parameters |
| `resolution_scale` | f32 | 1.0 | 0.5-1.0 | Encode resolution scale. 1.0 = native (no change). Below 1.0 encodes at a smaller resolution to cut bandwidth; the HMD restores it. **Fixed at session start.** See AI Super Resolution below |
| `bitrate_pixel_factor` | f32 | 2.0 | 1.0-4.0 | Bits-per-pixel multiplier for the encoder bitrate (`bitrate = encoded_w * encoded_h * factor`). Raise it to spend more bits on a downscaled stream |

### AI Super Resolution — `resolution_scale` (Phase 0)

`resolution_scale < 1.0` makes the PC encode each eye at a smaller resolution
(e.g. `0.5` → 916×960 instead of 1832×1920), roughly quartering the encoded
pixel count and the bandwidth. The HMD is told the real encoded size in
STREAM_CONFIG and decodes at that size.

**Phase 0 is a bandwidth ↔ softness trade-off only.** The current build stretches
the decoded frame back to native with plain bilinear filtering — there is *no*
sharpening yet, so a downscaled stream looks softer. The GLSL bicubic + sharpen
upscaler (Phase 1) is held until the target hardware is available (see
`TODOS.md`). A client that does not advertise upscaler support is always sent the
native resolution, so older clients are never silently degraded.

`bitrate_pixel_factor` lets you tune perceived quality at a given
`resolution_scale` without a rebuild: at `resolution_scale = 0.5`, a factor of
`2.0` gives ~20 Mbps and `3.0` gives ~30 Mbps for the same frame.

## `[display]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `ipd` | f32 | 0.063 | Inter-pupillary distance in meters |
| `seconds_from_vsync_to_photons` | f32 | 0.011 | Display latency compensation |

## `[audio]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | true | — | Enable audio streaming (WASAPI loopback → Opus → UDP) |
| `bitrate_kbps` | u32 | 128 | 32-512 | Opus encoder bitrate in kbps |
| `frame_size_ms` | u32 | 10 | — | Opus frame size in milliseconds |
| `sample_rate` | u32 | 48000 | 48000 only | Must be 48000 (Opus requirement) |
| `channels` | u16 | 2 | — | Audio channels (stereo) |

**Validation:** sample_rate must be 48000 (Opus compatibility). bitrate_kbps clamped to [32-512].

## `[foveated]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | false | — | Enable foveated encoding (requires eye tracker) |
| `preset` | string | "balanced" | subtle/balanced/aggressive/custom | Preset QP offset profiles |
| `fovea_radius` | f32 | 0.15 | (0.0, 0.5] | Inner fovea zone radius (fraction of frame) |
| `mid_radius` | f32 | 0.35 | (fovea_radius, 1.0] | Mid zone radius. Must be > fovea_radius |
| `mid_qp_offset` | i32 | 5 | — | QP delta for mid zone (only with preset="custom") |
| `peripheral_qp_offset` | i32 | 15 | — | QP delta for periphery (only with preset="custom") |

**Presets:** subtle (+3/+8), balanced (+5/+15), aggressive (+8/+25), custom (uses mid/peripheral_qp_offset values).

## `[face_tracking]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | true | — | Enable face tracking OSC bridge (HTC blendshapes → VRChat) |
| `smoothing` | f32 | 0.6 | [0.0, 0.99] | EMA smoothing factor. 0.0 = raw, 0.99 = very smooth |
| `osc_port` | u16 | 9000 | — | VRChat OSC listener port (localhost) |
| `active_profile` | string | "" | — | Expression profile name. Empty = no profile (all weights 1.0) |

**Validation:** smoothing is checked for NaN/Infinity and clamped to [0.0, 0.99].

## `[sleep_mode]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | true | — | Enable automatic sleep mode on user inactivity |
| `timeout_seconds` | u32 | 300 | 30-3600 | Seconds of no head movement before entering sleep |
| `motion_threshold` | f32 | 0.002 | (0.0, 0.1] | Minimum head movement (meters/frame) to count as active |
| `sleep_bitrate_mbps` | u32 | 8 | — | Reduced bitrate during sleep mode |

## `[pairing]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `max_attempts` | u8 | 5 | PIN entry attempts before lockout |
| `lockout_seconds` | u64 | 300 | Lockout duration after max attempts (5 minutes) |
