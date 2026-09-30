# Focus Vision PCVR — Configuration Reference

Config file: `config/default.toml`, overridden by `local.toml` files (see the
load order below).
All values are validated on startup. Invalid values are clamped to defaults with a warning.

## Where the engine reads its config

The engine runs inside SteamVR's `vrserver.exe`, so files are located relative
to the driver DLL, not the working directory. Layers, lowest precedence first:

1. **`config/default.toml`** — found by walking up from the driver DLL's folder
   (installed: `<install dir>\config\default.toml`; dev build: `<repo>\config\default.toml`),
   falling back to `config/default.toml` under the working directory.
2. **`config/local.toml`** next to that `default.toml` — dev checkouts (gitignored).
3. **`%APPDATA%\FocusVisionPCVR\config\local.toml`** — per-user overrides. The
   companion app's Settings tab writes here — only the sections you change —
   and hand-written keys in this file are kept when the companion saves.
   Settings → **Reset to defaults** removes the companion's keys, so
   `default.toml` applies again.

Later layers override earlier ones **key by key** (a `local.toml` only needs the
keys it changes). Unknown keys and sections are ignored. A layer that fails to
parse is skipped with a warning; if the overrides produce an invalid value type,
the engine falls back to `default.toml` alone. Changes apply on the next engine
(SteamVR) start. The engine log lists the layers it applied.

## `[network]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `tcp_port` | u16 | 9944 | >= 1024 | TCP control channel port (TLS handshake, PIN pairing, heartbeat, face tracking) |
| `udp_port` | u16 | 9945 | >= 1024, != tcp_port | Base UDP port. Video = udp_port + VIDEO_PORT_OFFSET, Audio = udp_port + AUDIO_PORT_OFFSET |
| `fec_redundancy` | f32 | 0.2 | 0.0-1.0 | FEC parity ratio. 0.2 = 20% parity shards added to each frame |
| `slice_fec_enabled` | bool | true | — | Split frames ≥ 16 KB into independently FEC-coded slices |
| `slice_count` | u8 | 4 | 2-15 | Slices per frame. Raised for a single frame (up to 15) when a slice would exceed one Reed-Solomon code word (data + parity ≤ 256 shards of 1200 B) — e.g. a ~1 MB IDR goes out in 5 slices |

**Validation:** tcp_port and udp_port must be >= 1024. If they're equal, udp_port is auto-incremented. Below 1024 is clamped to default.

## `[video]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `codec` | string | "h265" | "h264", "h265" | Video codec. H.265 = better compression, H.264 = faster decode on some devices |
| `bitrate_mbps` | u32 | 80 | 10-200 | Target bitrate in Mbps: the NVENC target, the value STREAM_CONFIG sends to the HMD, and the adaptive bitrate controller's starting point; the controller's changes reach NVENC at runtime. Covers both eyes: a frame holds two (side by side) |
| `resolution_per_eye` | [u32; 2] | [1832, 1920] | — | Per-eye render resolution [width, height]. Must match SteamVR render target |
| `framerate` | u32 | 90 | 30-120 | Target framerate. Supported: 72, 90, 96, 120 |
| `full_range` | bool | true | — | Full RGB (0-255) vs limited range (16-235). Affects NVENC VUI parameters |
| `resolution_scale` | f32 | 1.0 | 0.5-1.0 | Encode resolution scale. 1.0 = native (no change). Below 1.0 encodes at a smaller resolution to cut bandwidth; the HMD restores it. **Fixed at session start.** See AI Super Resolution below |
| `bitrate_pixel_factor` | — | — | — | **Deprecated, ignored.** Used to set the encoder bitrate to `encoded_w * encoded_h * factor` (~7 Mbps at native), contradicting `bitrate_mbps`. Old files that set it still load; the engine logs a warning |

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

The encoder bitrate is `bitrate_mbps` at any `resolution_scale`; lower it
together with the scale if the goal is to save bandwidth.

## `[display]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `ipd` | f32 | 0.063 | Inter-pupillary distance in meters |
| `seconds_from_vsync_to_photons` | f32 | 0.011 | Display latency compensation |

## `[audio]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | true | — | Enable audio streaming (WASAPI loopback → Opus → UDP) |
| `bitrate_kbps` | u32 | 128 | 32-512 | Opus encoder bitrate in kbps (the companion's Audio slider) |
| `frame_size_ms`, `sample_rate`, `channels` | — | — | — | **Not settings.** The stream is always 10 ms frames of 48 kHz stereo Opus: the headset decodes nothing else, and capture converts the output device's own rate and channels. Files that list them still load; a value other than 10 / 48000 / 2 is logged as ignored |

**Validation:** bitrate_kbps outside [32-512] is reset to 128.

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

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `max_attempts` | u8 | 5 | 1-10 | Wrong PINs before the lockout. The lockout also issues a new PIN |
| `lockout_seconds` | u64 | 300 | 300-3600 | How long the lockout lasts. It survives an engine restart (`lockout.json`) |

**Validation:** a value outside its range is reset to the default. The bounds
keep a LAN attacker at no more than twice the default's guesses (5 per 300 s);
see `SECURITY.md`.

## `[memory_monitor]`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | true | — | Watch the memory of the process the engine runs in (vrserver.exe) |
| `poll_interval_seconds` | u32 | 60 | 10-3600 | How often it is read |
| `growth_threshold_mb` | u32 | 50 | 1-100000 | Growth within an hour that is logged as a warning |

Every hour `engine.log` gets the figure (`Memory monitor: … MB`), or a
`Memory growth warning` when it grew by the threshold or more — a leak's first
sign in a long session. The figure covers all of vrserver.exe, SteamVR included.

## `[thermal]`

Lowers the bitrate ceiling when the NVIDIA GPU runs hot (`warn_celsius` <
`limit_celsius` < `emergency_celsius`, back up over `recovery_seconds`).
**It works only in a build with the `nvml` feature; the released installer's
is not one**, so there `enabled = true` only logs a warning. Off by default.

## Session logs (no settings)

While streaming, the engine writes a line of stats every 10 s to
`%APPDATA%\FocusVisionPCVR\sessions\session_<UTC start>.jsonl`: the PC side's
latency (`pc_latency_us`, from the frame being ready to encode to its packets
going out), the frames sent in the last second (`pc_fps`), `bitrate_mbps`, the headset's `loss_pct`, `fec_pct`,
the headset's `hmd_fps` and `hmd_decode_us`, `sleeping`, and vrserver.exe's
memory (`rss_mb`). Lines are written a minute at a time and when the session
ends. Files older than 7 days are deleted when the engine starts. The
companion's **Export Logs** puts the newest five in the zip.
