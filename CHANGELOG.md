# Changelog

All notable changes to Focus Vision PCVR will be documented in this file.

## [Unreleased]

### Protocol (breaking: v5 — stereo)
- **Video frames carry both eyes side by side.** The driver streamed the
  left eye only and the headset showed it to both eyes. A frame is now twice
  the per-eye `encoded_w` wide, left eye on the left; STREAM_CONFIG grows
  25 → 26 bytes with a stereo-layout byte (`stereo_layout`: 0 = mono,
  1 = side by side). `PROTOCOL_VERSION` is 5 on both sides; a 25-byte
  payload (a server before v5) is read as mono. The pixel count doubles, so
  the same `bitrate_mbps` gives each eye about half the bits.
- **VIEW_CONFIG (0x22, HMD → PC): the headset's field of view and IPD.**
  36 bytes, f32 LE: each eye's `XrFovf` angles (left, right, up, down) and
  the IPD in metres. The client sends it once per session and whenever it
  changes (the IPD is rounded to 0.1 mm so tracking noise isn't a change);
  the engine rejects implausible values (non-finite, past 85°, reversed
  edges, an IPD outside 40–90 mm). An older engine logs it as unknown.

### Protocol (breaking: v4)
- **FVP header carries `data_shard_count` (FEC fix).** The HMD derived the
  data/parity split as `total_shards / 1.2`, which is only right at exactly
  20 % redundancy — adaptive FEC moves between 5 % and 40 %, so frames were
  mis-split (too few data shards → corrupt frame; too many → a recoverable
  frame discarded). The FVP header grows 10 → 12 bytes with
  `data_shard_count: u16 LE` appended after `flags` (bytes 22..24; older
  fields keep their offsets, payload now starts at byte 24).
  `PROTOCOL_VERSION` is 4 on both sides; the server logs a warning for older
  clients. Receivers validate `0 < data <= total <= 4096` and
  `shard_index < total` before sizing or indexing buffers (a forged
  `data > total` previously read past the end of the C++ decoder's arrays).
- New `pipeline::FecFrameReassembler` (Rust receive side, bulk + sliced,
  header-driven RS recovery) now used by the simulator's mock client, and a
  dependency-free `parseFvpHeader()` in the Android client.
- Client receive-path fixes found on the way: the sliced decoder reported
  "complete" before its first frame (pushing an empty frame — IDR request +
  decoder flush — every render loop), completed frames were re-submitted
  every loop until the next frame arrived, frame index 0 never started, and a
  timed-out slice re-requested an IDR on every loop.
- Tests: redundancy 0.05 / 0.2 / 0.4 / 1.0 recovery at maximum loss,
  per-frame redundancy changes, regression tests for the old `/1.2` guess,
  cross-language golden RS vectors shared by `fec.rs` and
  `client/tests/test_fec_decoder.cpp` (host-built via `client/tests/shim/`),
  forged-header fuzz cases.

### Security
- **Any PIN paired after an unfinished handshake.** `PairingState::verify`
  returned Ok for any PIN once the state was paired, and a handshake that
  failed after the PIN step (the client dropped or timed out before
  STREAM_START) left it paired. The accept loop kept the same state, so the
  next LAN client paired with any PIN — ignoring lockout and PIN rotation —
  and became the authorized peer (video, pose and controller input).
  `verify` now always checks the PIN.
- **Dependency advisories.** rustls 0.23.37 → 0.23.45 (RUSTSEC-2026-0285,
  TLS 1.3 handshake messages accepted across encryption levels),
  rustls-webpki → 0.103.15, crossbeam-epoch → 0.9.21, quick-xml → 0.41.0
  (build-time only, via wayland-scanner), webbrowser → 1.2.4, anyhow →
  1.0.104, memmap2 → 0.9.11. `cargo audit` now reports no vulnerabilities;
  the remaining warnings are unmaintained crates and the `lru` / `spin`
  notices from reed-solomon-erasure 6.0.0 (its latest release).
- **CI security audit actually runs.** The step ran in pwsh, where
  `2>/dev/null` fails, and `continue-on-error` hid it, so `cargo audit` never
  ran. It now runs in bash and fails the job on a known vulnerability.
- **Persistent TLS identity (fixes TOFU re-connect failure).** The engine
  minted a new self-signed certificate on every `TcpControlServer::new()` —
  i.e. on every accept-loop iteration and hold period — while the HMD pins
  the first certificate's SHA-256 and refuses any other. Once paired, every
  reconnect and engine restart failed. The certificate + key now live in
  `%APPDATA%/FocusVisionPCVR/tls_identity.bin` (atomic write, SHA-256
  checksum, cached per process); a corrupt file is moved to `.bak` and
  regenerated with a loud re-pair warning.
- **Handshake timeouts.** TLS handshake 10 s, HELLO / STREAM_START 10 s each,
  PIN_RESPONSE 30 s. Previously one client that connected and stayed silent
  blocked the sequential accept loop indefinitely, locking the real HMD out.
  Clients advertising a protocol version older than the server's now log a
  compatibility warning.
- **Tracking UDP source check.** The tracking receiver only accepts datagrams
  from the IP of the HMD that completed TLS + PIN pairing, and only while
  that session is up (`AuthorizedPeerGuard` revokes it on every session exit
  path). Any LAN host could previously inject head/controller poses and the
  foveation gaze point. A transient `recv_from` error no longer ends the
  receiver loop for the rest of the engine's life.
- **Pairing PINs expire.** `PIN_LIFETIME_SECONDS` (300 s) was only shown as
  the companion's "Expires in" countdown; the engine never enforced it, so a
  PIN stayed valid until the next session or restart. The engine now
  replaces the PIN when it expires while waiting for the HMD (a handshake in
  progress keeps the PIN it started with), and status.json carries the real
  seconds left instead of a constant 300. Each heartbeat re-reads the PIN,
  which also fixes the companion showing a dead PIN after a lockout had
  replaced it.

### Android client
- **A connection layer, tested against the real engine.** The client had
  the pieces (TLS client, FEC decoder, a session state machine), but
  nothing ran them, and they had never talked to the engine. New portable
  C++, built for Android and for the host tests:
  - `StreamSession` does connect → TLS → PIN → STREAM_CONFIG →
    STREAM_START, then streams and reconnects after a drop. It uses the
    `ClientSession` policy: exponential backoff, and no retry after a
    rejected PIN, which would only walk into the lockout.
    - It runs on its own thread, the only one touching the TLS
      connection. Other threads queue messages, and the server's
      messages (HEARTBEAT_ACK, HAPTIC_EVENT, SLEEP_ENTER/EXIT,
      CONFIG_UPDATE_ACK) come back as events.
    - Every 500 ms it sends a HEARTBEAT with real statistics, and it
      treats the link as dead after 3 s of silence.
  - `VideoReceiver` receives video on its own thread and accepts packets
    only from the server. The render loop used to read at most 64 packets
    per frame, below what 80 Mbps needs.
  - `FrameAssembler` hands each frame over as soon as it can be rebuilt,
    bulk or sliced, so frames leave in send order.
    - A lost frame (missing shards, a frame index that never came, or a
      100 ms stall) triggers an IDR request, and non-key frames are
      skipped until a keyframe arrives, also at session start.
    - A late packet of a finished frame is ignored.
    - RTP sequence gaps are counted as lost packets. HEARTBEAT reported
      zero loss before, which walked adaptive FEC down to 5 %.
  - `TcpControlClient`:
    - Portable sockets (`net_compat.h`: POSIX or Winsock).
    - `psa_crypto_init()`.
    - Timeouts on connect, the TLS handshake, every pairing step and
      writes, via a non-blocking socket and `mbedtls_net_poll`.
    - Framing that survives split reads, and one TLS record per message.
    - The server certificate is pinned only after pairing succeeds.
    - `disconnect()` no longer frees uninitialised MbedTLS contexts or
      closes the socket twice.
  - `client/tests/test_session_e2e.cpp` starts the headless engine and
    runs these classes against it. Checked: TLS 1.3, pairing, 60 fps of
    sliced and bulk frames in order from a keyframe, HEARTBEAT_ACK, a
    wrong PIN rejected and not retried, reconnecting within the engine's
    5 s hold with the same PIN, and a server that does not match the
    pinned certificate is refused before the PIN is sent. New CI job
    `client-e2e` (Windows, against the simulator binary from rust-build).
- **The app uses it.** `openxr_app` now runs `StreamSession` and
  `VideoReceiver` instead of the unused `TcpControlClient`,
  `NetworkReceiver` and FEC decoders it held.
  - **Address and PIN at launch.** `am start … --es fvp_server <ip[:port]>
    --es fvp_pin <6 digits> [--es fvp_udp_port <base>]`, the form the
    companion will send over adb.
    - MainActivity writes the extras to a file in app-private storage.
      The native loop reads and deletes it (`launch_request.h`, tested),
      and checks again every second for a new launch (`onNewIntent`).
    - The PIN is handed over once. A recreated activity does not replay
      it, since a stale PIN costs an attempt against the lockout.
    - To pair with another PC, clear the app's data (`adb shell pm clear
      com.focusvision.pcvr`); there is no re-pair extra an app on the
      headset could send.
  - **Video.** Frames from `VideoReceiver` go to MediaCodec in order. The
    decoder is recreated when STREAM_CONFIG's codec (H.264 / HEVC) or
    encoded size changes, and flushed for a new session.
    - A frame the decoder cannot take (no input buffer), a failed NAL
      check, or a pause in rendering makes the receiver drop what is
      queued and wait for a keyframe (an IDR is requested).
    - The HEVC NAL check is no longer applied to H.264 streams, where it
      rejected about one frame in eight.
    - Decoded frames feed the HEARTBEAT's fps and decode latency.
  - **Per session.** The tracking sender is started toward the server,
    and Opus audio is received on base+3 (RTP header stripped) and played
    through `AudioPlayer`.
  - **Server messages.** SLEEP_ENTER / EXIT dim the view. HAPTIC_EVENT
    goes to `ControllerPoller::applyHaptic`, which does nothing until
    controller input is initialised (not yet). FACE_DATA is queued on the
    session.
  - **On exit** the app sends DISCONNECT, so the engine ends the session
    instead of holding it.
  - Checked: the APK builds (NDK 26.1). Not run on the headset.
- **The companion hands the address and PIN to the headset.** Home shows a
  **Send PIN to headset** button while the engine waits for a PIN (the
  headset app has no PIN entry screen yet). Deploy's post-install launch
  does the same when a PIN is waiting.
  - It reads the headset's Wi-Fi address over adb (`ip -f inet addr show
    wlan0`).
  - It picks this PC's address toward that one from the routing table,
    by connecting a UDP socket (which sends nothing).
  - It starts the client with `fvp_server` / `fvp_pin` / `fvp_udp_port`
    (`headset_link.rs`).
  - The engine now publishes its ports in status.json (`tcp_port`,
    `udp_port`), so a non-default `[network]` config reaches the headset.
    Older engines fall back to 9944 / 9945.
  - USER_GUIDE now describes this flow. It had described a PIN entry
    screen on the headset, APK drag and drop, and a "Deploy" button,
    none of which exist.
- **Each eye sees its own image, the right way up.** Found by reading the
  code; the shaders are validated with glslang but not yet run on the
  headset.
  - The decoder is sized for the side-by-side frame (v5), and each eye
    shows its half.
  - The newest decoded frame is taken once per loop and shown to both
    eyes. It was taken per eye, so the eyes could show different frames —
    one new, one reprojected.
  - The renderer and the timewarp are one shader. The timewarp path drew
    the image upside down relative to the new-frame path, uploaded its
    matrix untransposed, assumed +Z forward (OpenXR looks down -Z), and
    divided per vertex, which bends straight lines. Reprojection is now
    per pixel; its math (`video_view.h`) is host-tested.

### SteamVR driver
- **The video path is wired.** Before, nothing called the encoder's init, the
  driver had no D3D11 device, and the swap textures' "shared handles" were a
  counter, so SteamVR's compositor could not render for the headset and
  `Present()` returned before encoding. Not yet run under SteamVR or on an
  NVIDIA GPU.
  - The driver creates its D3D11 device on the NVIDIA GPU (else the largest
    hardware GPU) and names it in `Prop_GraphicsAdapterLuid_Uint64`, so the
    compositor renders on the same adapter.
  - Swap textures are shareable, and their handles are the DXGI shared
    handles the compositor opens. Destroying a set destroys all three
    textures (it removed one).
  - The HMD has an `IVRDisplayComponent`: render size, projection (a fixed
    100° per eye until the headset reports its own), eye viewports, no
    distortion. SteamVR times vsync; `PostPresent` waits out each frame's
    slot at the refresh rate (`FramePacer`), as ALVR does. Swap-set indices
    are tracked per set rather than read back from SteamVR's arguments.
  - `Present` reads the frame only while holding the compositor's sync
    texture (keyed mutex), then draws both eyes of the scene layer side by
    side into the encoder's input (`EyeBlit`), and the foveated QP map
    gets a fovea in each eye. A draw instead of `CopyResource`, which
    D3D11 skipped between the compositor's R8G8B8A8 and NVENC's B8G8R8A8,
    and which could not scale (SteamVR supersampling), take one eye of a
    double-wide texture, or keep sRGB. The encoder registers that texture;
    the full-range and foveation settings now reach it.
  - When NVENC is unavailable the driver logs why in vrserver.txt and
    streams nothing. It used to stream a made-up NAL pattern (IDR header +
    `0xAB`) and report the failure only to `OutputDebugString`.
  - `Cleanup` stops the engine before destroying the devices, so a late IDR
    or gaze callback can't reach a destroyed HMD.
  - The D3D11 side is tested on WARP (no GPU needed): a second device plays
    the compositor, opens the swap textures by handle, fills them under the
    keyed mutex, and the driver's output is read back (left/right eye,
    scaling, flipped bounds, sRGB, BGRA and float sources, both eyes side
    by side). Driver gtests: 78.
  - Runtime bitrate changes reach NVENC. The driver registers the engine's
    bitrate callback (adaptive bitrate, sleep mode, the headset's
    CONFIG_UPDATE); the new target is applied before the next encode with
    `nvEncReconfigureEncoder`, without an IDR or a rate-control reset.
  - The controllers present themselves as Oculus Touch (Quest 2), as ALVR
    and Virtual Desktop do, so games' Touch bindings apply: controller type
    `oculus_touch`, SteamVR's own `{oculus}/input/touch_profile.json` and
    render models, X/Y on the left and A/B on the right. Before, the input
    profile pointed at a file that did not exist and SteamVR had no
    bindings for them.
  - SteamVR renders with the headset's own field of view and IPD: the
    driver takes VIEW_CONFIG from the engine (`fvp_set_view_config_callback`)
    and calls `SetDisplayProjectionRaw` / `SetDisplayEyeToHead`. The fixed
    100° per eye is used only until the first connection. The raw
    projection's `top` / `bottom` now follow OpenVR's convention (`top` is
    the lower edge's tangent, as `ComposeProjection` and ALVR use it); they
    were swapped, which turns an asymmetric field of view upside down.
  - **The dashboard and overlays are streamed.** `SubmitLayer` kept only the
    first layer, the scene. Every layer is now drawn, bottom first: the
    scene opaque (some apps submit it with zero alpha), each layer above it
    blended by its alpha (as ALVR does). A layer rendered at another head
    orientation than the scene's (the compositor draws its overlays at a
    newer pose when an app falls behind) is turned to the scene's, per
    pixel, with the eye's field of view (`layer_compose.h`, host-tested;
    the shader is checked on WARP).

### Fixes
- **The engine notices a dead link and lets the headset back in** (#18).
  - A control connection silent for 3 s (six missed heartbeats) is dropped.
    A Wi-Fi drop that lost the FIN/RST left the session streaming to nobody
    and refusing the headset's reconnects.
  - The headset's input is released when it goes away: the session end
    clears the head pose and controllers, a controller silent for 250 ms
    reads as gone, and the driver releases its buttons and sticks, which
    otherwise stayed pressed in SteamVR.
  - `accept()` errors no longer stop the engine. Anyone on the LAN could
    stop it until SteamVR restarted by connecting and resetting six times.
  - Control messages are read without losing half a message to a haptic
    send, and go out as one TLS record each, with `TCP_NODELAY`.
  - The mock client sends real HEARTBEATs (loss, received count, fps), so
    the E2E tests cover the adaptive path and the engine's ACKs.
- **PC audio from any output device.** Capture asked WASAPI for 48 kHz,
  which its shared mode refuses unless it is the device's own rate (cpal
  asks for no conversion), so a 44.1 or 96 kHz device gave no audio at
  all. A 5.1 / 7.1 output's samples went out as if they were stereo.
  - The loopback opens in the device's own format, and `audio/convert.rs`
    turns it into the 48 kHz stereo Opus takes: quad / 5.1 / 7.1 fold to
    stereo (centre and surrounds at −3 dB, no LFE), then a windowed-sinc
    resampler (Blackman, 16 zero crossings; tones within 0.1 % of ideal,
    −70 dB of what 48 kHz can't carry when going down).
  - Checked with unit tests; not yet on a 44.1 kHz or surround device.
- **The companion draws Japanese and fits its window** (#19).
  - Japanese text was boxes (no loaded font had kana or kanji). The OS's
    Japanese font (Yu Gothic Medium, else Meiryo or MS Gothic) follows Geist,
    moved onto Geist's baseline.
  - Every tab scrolls; at 480×640 Settings' codec, Export Logs and Reset
    were below the window.
  - No console window in the release exe (closing it lost the last
    settings change); adb, PowerShell and wmic start without one.
  - A window closed while minimized reopened as an 80×103 sliver (eframe
    saved it as 0×0); eframe's window persistence is off.
- **File dialogs keep Japanese paths, and the window keeps drawing.** The
  APK picker and the stats SVG save dialog read PowerShell 5.1's output as
  UTF-8, but it is written in the console's code page (932 on Japanese
  Windows): a path under a Japanese user name came back mangled. The
  dialog script now prints the path as hex of its UTF-8 bytes
  (`file_dialog.rs`; a test runs the real PowerShell). The dialogs ran on
  the UI thread, which froze the window until they closed; each now runs
  on its own thread, and its button waits for it. PowerShell starts with
  `-NoProfile` (a profile's output could have mixed into the path) and
  `-STA`.
- **The Android client starts.** Found by reading the code; not yet run on
  the headset.
  - The manifest had no `android.app.lib_name`, so `NativeActivity` looked
    for `libmain.so` and threw at launch.
  - The manifest also lacked the `<queries>` that the Khronos OpenXR
    loader needs, with targetSdk 34, to find the runtime.
  - `xrGetOpenGLESGraphicsRequirementsKHR` was never called, and the spec
    makes `xrCreateSession` fail without it. It is now called before
    session creation, and the GLES context version is checked against the
    requirement.
  - The instance asked for OpenXR 1.1 (`XR_CURRENT_API_VERSION`), which a
    1.0-only runtime refuses. It now asks for 1.0, since nothing from 1.1
    is used.
  - The frame loop never returned and never read Android lifecycle
    commands, so pause/resume blocked (ANR) and a destroy was never seen.
    It now handles them every iteration.
  - `shutdown()` can safely run more than once.
- **Installed packages work.** The installer and the release zips had
  two problems, each enough to break an install.
  - The driver DLL was built into `bin/win64/Release/`, because the
    Visual Studio generator appends a per-config folder. SteamVR loads
    `<driver>/bin/win64/`, so a registered driver had no DLL. The output
    path now uses a generator expression, and CI checks the layout.
  - The companion crashed at every launch. Both Geist font URLs return
    404, and `curl -sL … || true` saved GitHub's error page as `.ttf`,
    which egui panics on. The URLs are now pinned to the v1.7.2 tag.
    `curl -f` and a TrueType signature check fail the build instead of
    shipping a broken file. The companion also skips any font file it
    cannot parse, and looks in `fonts/` next to the exe before the
    working directory.
- **The companion sees the driver the installer registered.**
  - The companion only looked in SteamVR's own `drivers` folder. The
    installer registers the driver in place with `vrpathreg`, so Home
    said "Not installed" after every install.
  - It now also reads `external_drivers` from
    `%LOCALAPPDATA%\openvr\openvrpaths.vrpath`. For a registered driver,
    Settings shows the path in place of an Uninstall button that could
    not remove it.
  - "Install Driver" reads the DLL from the build layout (`bin/win64/`).
- **NSIS stack use.** Driver registration used `nsExec::ExecToStack`, which
  pushes the output as well as the exit code, and returned early without
  restoring the registers. It now uses `ExecToLog`, which also puts the
  vrpathreg output in the install log, and always restores them.
- **The driver uses the official NVENC header.** `nvenc_encoder.h` mirrored
  `nvEncodeAPI.h` by hand, and got it wrong in ways that would make every
  NVENC call fail or read the wrong fields:
  - Struct versions were built as `sizeof | ver<<16 | API<<24` instead of
    `API | ver<<16 | 7<<28`, so `NvEncodeAPICreateInstance` would reject
    the call.
  - The function table started with `nvEncOpenEncodeSessionEx`, which
    shifted every slot after it.
  - The OPEN_ENCODE_SESSION_EX, INITIALIZE_PARAMS, LOCK_BITSTREAM,
    RC_PARAMS and PIC_PARAMS layouts were off.
  - `NV_ENC_BUFFER_FORMAT_ARGB` was `0x20` (officially `0x01000000`).
  - `NV_ENC_PIC_FLAG_FORCEIDR` was `4`, which is OUTPUT_SPSPPS, so IDR
    requests never produced an IDR.

  The header now comes from `third_party/nvenc` (nv-codec-headers
  n12.2.72.0, MIT). The encoder starts from NVIDIA's preset config
  (`nvEncGetEncodePresetConfigEx`); a zeroed config is not valid. On top
  of it, `driver/src/nvenc_config.h` applies low-latency CBR with a
  one-frame VBV, IPP only, and forced IDRs that carry the parameter sets.
  It also refuses an NVIDIA driver whose NVENC API is older than 12.2.
  The HEVC QP delta map now uses 32x32 CTBs, the only size NVENC
  supports; it used 64, so the map was a quarter of the expected size.
  The map is passed only when the session was created with
  `NV_ENC_QP_MAP_DELTA`. Driver gtests: 44. Not yet run on an NVIDIA GPU;
  the driver still never calls `initEncoder` (see Known issues).
- **The encoder bitrate is `bitrate_mbps`.** The driver set NVENC's target
  to `encoded_w * encoded_h * bitrate_pixel_factor` — about 7 Mbps at the
  native 1832×1920 — while `bitrate_mbps = 80` was sent to the HMD in
  STREAM_CONFIG and seeded the adaptive bitrate controller (and the
  fallback path used 80 Mbps). `FvpConfig` now carries `bitrate_bps` from
  `[video] bitrate_mbps` and the driver uses it. `bitrate_pixel_factor` is
  deprecated and ignored; config files that set it still load, with a
  warning. (FFI layout change: `FvpConfig.bitrate_pixel_factor` is replaced
  by `bitrate_bps` in the same slot; driver and engine ship together.)
- **Reconnecting within the 5 s hold works.** When a session dropped
  without DISCONNECT (Wi-Fi blip), the engine listened for 5 s with a new
  server and a **new** PIN — which the HMD cannot know — and if a client did
  get through, it closed that authenticated connection and started over with
  yet another PIN. The hold now reuses the session's server and accepts the
  PIN the HMD paired with (full TLS + PIN handshake, attempt limit intact)
  for just those 5 s, and streaming resumes on the reconnected connection.
  SECURITY.md's "5 s PIN skip via TLS session resumption" never existed; the
  threat model now describes this behaviour. The simulator's mock client
  gained `abrupt_close` to exercise it end to end.
- **A session ended on the PC side closes the HMD's connection.** When a
  session ended for a reason other than the TCP connection (the UDP sender
  could not be created, the frame source closed, engine shutdown), the
  control task kept serving the HMD's connection until the HMD hung up, and
  on shutdown the session's audio capture kept running. Every way out of a
  session now cancels it: the control task closes the connection, and audio
  stops. `queue_haptic` is routed to the session only while it runs; before,
  events after a session went to its closed channel and were logged as
  drops. A closed frame source now stops the engine instead of being counted
  as a lost connection and starting a 5 s hold.
- **Large IDR frames are no longer lost to slice FEC.** A slice whose data
  shards did not fit one Reed-Solomon code word (a literal cap of 200 data
  shards, or RS's 256 data + parity total, which at 40 % redundancy is only
  182 data shards) was sent as an empty batch while the other slices went
  out, so the receiver could never complete the frame. With the default 4
  slices that hit IDRs from ~0.87 MB (40 %) / ~0.96 MB (20 %) — the size the
  simulator itself calls realistic. The engine now picks the slice count per
  frame: the configured `slice_count`, raised up to 15 (the 4-bit header
  limit) until every slice fits. Frames are encoded all-or-nothing, never
  with an empty slice. With `slice_fec_enabled = false`, a frame too big for
  one bulk code word (> ~255 KB) is sliced too instead of losing its parity;
  only a frame too big for 15 slices (> ~3.3 MB) goes out unprotected, with a
  rate-limited warning, and one over `MAX_FRAME_SHARDS` is dropped rather
  than sent for every receiver to reject. No protocol change: both receivers
  already read the slice count per frame (new client test for 2 → 15 → 3).
  Dev/test builds now optimize `reed-solomon-erasure` so full-size-IDR tests
  stay fast.
- **The engine reads its config in a real install, and applies `local.toml`.**
  `fvp_init` loaded `config/default.toml` relative to the working directory —
  inside SteamVR that is `vrserver.exe`'s folder, so the installed file was
  never found and the engine silently ran on built-in defaults. No override
  file was read at all, so settings saved in the companion (and the
  `local.toml` that USER_GUIDE tells users to create) had no effect. The
  engine now finds `config/default.toml` by walking up from the driver DLL
  (falling back to the working directory) and deep-merges, key by key and
  lowest precedence first: `local.toml` next to that `default.toml` (dev
  checkouts), then `%APPDATA%\FocusVisionPCVR\config\local.toml` (written by
  the companion). An unparsable layer is skipped with a warning; overrides
  with a wrong value type fall back to `default.toml` alone. The engine log
  lists the layers it applied. See `docs/CONFIG.md`.
- **status.json heartbeat.** The engine now rewrites status.json every second
  while waiting for the HMD, during reconnect backoff, during the 5 s hold
  period (with the session's PIN) and while streaming (wall-clock tick
  instead of every Nth frame). Before, the file went untouched while waiting
  or when frames stalled, so the companion's 5 s mtime check showed the red
  "engine stopped" banner for a healthy engine.
- **Companion: stale status is no longer shown as live.** A missing or stale
  status.json (engine crashed) used to keep the last payload on screen —
  Connected, frozen stats, an old PIN. It now shows Disconnected and clears
  the PIN and its countdown. A slightly-future mtime (coarse FS timestamps,
  clock step) no longer reads as "engine stopped", and a rotated PIN restarts
  the "Expires in" countdown.
- **Companion: settings persistence.** `local.toml` moves to
  `%APPDATA%/FocusVisionPCVR/config/local.toml` (the path USER_GUIDE already
  documented; the old exe/CWD-relative path under Program Files was not
  user-writable and was wiped by the installer on reinstall). The legacy file
  is migrated on first load. Writes are atomic and merge only the keys the
  companion manages, so hand-written keys survive; an unparsable file is
  backed up to `local.toml.bak` before being replaced. Saves are debounced
  (500 ms, flushed on exit) instead of written every frame of a slider drag.
- **Companion: diagnostics PII masking.** The sanitizer converted input byte
  by byte, garbling all non-ASCII text (Japanese log lines) in exported logs,
  and let a sentence-ending IP (`192.168.1.5.`) through. It is now UTF-8 safe
  and masks SSIDs, the pairing PIN, user names in profile paths, e-mail, MAC,
  IPv4 (octets 0–255) and IPv6 addresses, without touching version strings,
  C++ `Class::method` scopes or clock times.

### Internal
- **`run_streaming` split, adaptive state without locks.** The ~440-line
  loop is now `StreamingLoop::{run, accept, run_session, hold}`. The TCP
  control task no longer shares `Arc<Mutex<…>>` state with the frame loop:
  HEARTBEAT stats and TRANSPORT_FEEDBACK reach it as events over an mpsc
  channel (feedback used to be dropped whenever `try_lock` found the GCC
  estimator busy), and the bandwidth/bitrate/burst/GCC/adaptive-FEC/sleep
  state lives in one frame-loop-owned `AdaptiveState`. `handle_tcp_control`
  takes a `ControlChannel` instead of nine arguments. Two small behaviour
  changes: status.json's packet-loss figure now shows the latest heartbeat
  (it read 0 % most of the time because the bitrate tick had consumed the
  stats), and after a session ends the engine waits up to 1 s for the
  control task's disconnect reason, so a clean DISCONNECT that races the
  frame loop is no longer mistaken for a dropped link (which started a
  needless 5 s hold).

### Docs
- **The project status is stated honestly.** The v3.0.0 release notes
  called the release "General Availability", and README and USER_GUIDE
  described a working headset product. Neither P0 below was disclosed.
  - The release notes now say "development preview", with a dated
    correction listing what does not work.
  - README and USER_GUIDE open with the same notice.
  - USER_GUIDE lists the NVIDIA driver this build needs (551.76+, NVENC
    API 12.2).
  - TODOS.md records the open findings of the 2026-09-28 code audit.

### Known issues
- **Nothing has run on the real hardware yet.** The Android client now
  connects and receives (tested against the engine on the host), and the
  SteamVR driver's video path is wired (its D3D11 side tested on WARP), but
  neither has run on the headset, under SteamVR, or on an NVIDIA GPU. The
  hardware-free simulator path is unaffected.

## [3.0.0] - 2026-06-01

General availability. Promotes rc3 to the stable 3.0.0 release and turns the
hardware-free path into a shipped, end-user-runnable experience rather than a
dev-only tool.

### Hardware-free general availability
- **In-process simulation bundled in the installer.** The companion app is
  now built with `--features simulator` in CI (`companion-build`) and
  `build.bat`, so the distributed `focus-vision.exe` carries a real
  `StreamingEngine` + mock HMD client. End users can run the full pipeline
  (TCP+TLS+PIN+RTP+FEC+UDP+Opus) with no VR hardware via the Home tab's
  "▶ Start Simulation" button / `--simulate`, beyond the pre-existing
  UI-only `--demo`. The SteamVR driver DLL still links `streaming-engine`
  without the `simulator` feature, so the real VR path and its binary are
  unaffected.

### Fixes
- **Engine now publishes a live `"streaming"` status.** During an active
  session the engine writes status.json as `"streaming"` with live
  latency/fps/bitrate/subsystems on connect and ~1×/sec, reverting to
  `"waiting"` on disconnect. Previously nothing ever wrote `"streaming"`, so
  the companion's Connected view and all live stats never activated — for the
  simulation OR real hardware. Pinned by a regression assertion in
  `headless_e2e_basic_video_flow` and the companion's `sim_smoke_round_trip`.
- **Deterministic `WSAEADDRINUSE` in the simulation removed.** `pick_free_ports`
  reserves a contiguous, non-ephemeral port block; previously the mock
  client's fixed video/audio receiver ports were derived from an ephemeral
  base that the engine's own ephemeral sender sockets could (on Windows)
  recycle first.
- Removed a leftover `TEMP-AUDIO-SEND` info log from the audio send loop.

### Docs
- `docs/USER_GUIDE.md`, `README.md`, `CLAUDE.md`, and the release notes
  document `--simulate` / "Start Simulation" alongside `--demo`.

### Known limitations
- The real-hardware verification items carried from rc3 remain validated in
  simulation only (real MediaCodec decode time, real face-tracking camera
  input, NVENC full-range color accuracy, NVML GPU thermal thresholds,
  multi-hour soak). See `docs/RELEASE_NOTES_v3.0.0.md` for the full list.

## [3.0.0-rc3] - 2026-05-22

Quality-focused release-candidate on top of rc2. No new product features
or breaking changes — the release tightens the configurations and evidence
that the engine ships with: third-party license disclosure, prerequisite
checks in the installer, R8 minification on the Android client,
workspace-wide clippy cleanliness, and a substantially extended scenario
harness so the headless simulator now covers H.264/H.265 codec comparison,
face-tracking waveform presets, frame-latency injection, and a 15-minute
long-run stability scenario (nightly CI).

### Distribution polish
- **Workspace `[workspace.package]` block** introduced; `streaming-engine`,
  `focus-vision-companion`, `fvp-common` now inherit `version`, `authors`,
  `license = "MIT OR LicenseRef-Commercial"`, `repository` from the root
  `Cargo.toml`. The companion crate retains its `description`. (A1)
- **`THIRD_PARTY_LICENSES.md` / `.html`** generated by `cargo-about` against
  an `about.toml` allow-list (Apache-2.0, MIT, ISC, BSD, Zlib, OFL,
  Ubuntu-font, MPL-2.0, Unicode, Unlicense). 433 crates disclosed in full.
  The NSIS installer now bundles `THIRD_PARTY_LICENSES.html` so end users
  see it on disk after install. (A2)
- **NSIS installer prerequisite check**: `.onInit` reads
  `HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64`. Missing
  VC++ 2015-2022 Redistributable now shows a clear message + opens the
  download URL + aborts, instead of letting the user finish a broken
  install that fails at SteamVR driver load time with cryptic
  `MSVCP140.dll` errors. (B1)
- **Android R8 / resource shrinking enabled** on `release`. New
  `client/app/proguard-rules.pro` keeps `MainActivity`, `NativeActivity`
  surface, JNI native-method names, MediaCodec callback shapes, and
  Surface — everything the C++ NDK code or HTC VR launcher reach by
  reflection. APK size and code load time improve; no functional change. (B2)
- **`scripts/check-versions.ps1`** now accepts semver pre-release suffixes
  (`-rcN`) and inherits version from `[workspace.package]` for sub-crates.

### Code quality
- **Clippy clean across `--workspace --all-features --all-targets
  -- -D warnings`** and across the separate `rust/streaming-engine/fuzz`
  workspace. Pre-existing toolchain regressions (the `(0u * tick)` pedagogical
  asserts, manual `div_ceil`, `clone_on_copy` on `Copy` types under
  `MutexGuard`, `useless_vec` in lib-test buffers, `is_multiple_of` and
  `clamp` modernizations) all fixed with surgical edits. The CLAUDE.md
  caveat about pre-existing warnings is removed. (B3)
- **CI clippy gate** widened to `--all-targets` so test-file regressions
  are caught before merge.

### Real-hardware-free verification (scenario harness extensions)
- **`MockClientStats::depacketize_latency_us_{p50,p95,p99}`** plus a
  `MockClientConfig::measure_decode_latency` opt-in flag. The video
  receiver now records first-RTP-packet → frame-completion wall time and
  the run computes nearest-rank percentiles from up to 200k samples
  (sufficient for the 15-minute long-run scenario). (C1)
- **`scenario_codec_comparison`** test runs the two new scenarios
  `codec_comparison_h264.json` and `codec_comparison_h265.json`
  back-to-back and prints a side-by-side latency table to the test log so
  CI logs carry a numeric diff between codecs. Synthetic-NAL depacketize
  path only — real MediaCodec decode latency is still GA-with-hardware. (C2)
- **`face_tracking_patterns.json` + `_talk.json` + `_smile.json`** drive
  four new `FaceMode` presets (`Blink`, `Talk`, `Smile`, `Frown`) so the
  OSC bridge surfaces the right blendshape on the right address without
  needing the HMD camera. Each scenario asserts via the existing OSC
  loopback receiver. (C4)
- **`Stimulus::InjectFrameLatency { latency_us }` / `ClearFrameLatency`**
  add per-frame producer delay to model OpenXR `xrWaitFrame` jitter.
  `frame_jitter.json` exercises 5 ms and 10 ms windows mid-run. (C5)
- **`long_run_stability.json` (15 min, `#[ignore]`)** runs intermittent
  5–15 % packet loss over the duration and asserts ≥ 30 k frames decoded,
  ≥ 1500 heartbeats, ≥ 100 k packets, ≤ 200 ms p99 depacketize latency.
  New nightly CI job `long-run-stability` (windows-latest, 30-minute
  timeout) runs it on schedule; PRs and pushes are unaffected. (C3)

### Documentation
- `CLAUDE.md`: status tag bumped to "rc3 candidate"; test count updated
  to "500+ Rust tests"; CI gate description corrected to "fully clean".
- `README.md`: badge `version-3.0.0-rc3`; new "Third-party dependencies"
  subsection in the License block linking the new disclosure files and
  documenting the regeneration commands.
- `docs/USER_GUIDE.md`: VC++ 2015-2022 Redistributable added to the
  Prerequisites table; download filename updated to
  `FocusVision-3.0.0-rc3-Setup.exe`.
- `docs/RELEASE_NOTES_v3.0.0-rc3.md` published.

### Out-of-Scope (carried over from rc2)
- 実機 (NVIDIA GPU + VIVE Focus Vision) での 30 分以上の連続セッション耐久試験
- NVENC VUI フルレンジでの色彩確認 (コード・テストは完了、目視は実機)
- HTC Face Tracking 実カメラ入力 → OSC ブリッジの end-to-end
- DRS (Dynamic Resolution Scaling)
- TLS TOFU の Android 実機 / mitmproxy 経由確認

## [3.0.0-rc2] - 2026-05-15

Maintenance release-candidate on top of rc1. Three follow-up changes
that were carried over from the pre-rc1 backlog after the original PRs
(#51 / #52 / #53) had drifted too far from main to rebase. Each change
was re-applied to a fresh branch off main and merged independently —
provenance is documented in each commit's PR body. No hardware
prerequisites; verification is the existing test suites plus the
real-device TLS check listed under Out-of-Scope.

### Security
- **TOFU 証明書ピン留めをクライアント側で実装** (#61): TLS ハンドシェイク
  後にサーバ leaf cert の SHA-256 を
  `<app internal storage>/server_fingerprint.hex` に保存し、以降の接続で
  fingerprint 不一致なら接続を拒否する。これまで `MBEDTLS_SSL_VERIFY_NONE`
  で**証明書を一切検証していなかった**ため、同一 LAN 上の攻撃者が任意
  TLS 証明書で MITM し PIN を盗聴可能だった。SECURITY.md が宣言していた
  TOFU 緩和策がここで初めて実態として動作する。
- **TLS 失敗時の平文フォールバック削除** (#61): `TcpControlClient::connect()`
  は TLS / pinning に失敗した場合、平文に降格せず接続を拒否する。
  これまではハンドシェイクを破壊するだけで暗号化を剥がせる構造だった。

### Fixed
- **NVENC セッションリーク 2 経路を塞ぐ** (#59): `NvencEncoder::init()` を
  idempotent 化（再 init 時は先に `shutdown()`）、partial-init fallback
  パスを `shutdown()` 経由に統一、`shutdown()` の `m_initialized` ガード
  を撤去。GeForce は同時 NVENC セッションを 2 本までに制限しており、
  pair / unpair を繰り返すと leak が積み上がって次のユーザーで
  `NV_ENC_ERR_OUT_OF_MEMORY` が出る経路を閉じる。

### Changed
- **長寿命 spawn を `spawn_named` 経由に集約** (#60): streaming /
  tracking-receiver / audio-encoder / tcp-control の 4 spawn を新 helper
  `spawn_named(handle, name, fut)` 経由にし、debug ログで spawn 時にタスク
  名を出力。サブシステムがサイレントに停止したときに、どれが停止したか
  ログから特定可能になる。`panic = "abort"` プロファイル下なので
  `catch_unwind` は不要（doc コメントに `panic = "unwind"` 切替時の拡張
  方法を明記）。

### Provenance / なぜ rc1 に入らなかったか
3 修正とも 2026-04-24 のコードレビューで提起されていたが、当時の PR ブランチ
（#51 / #52 / #53）は v3.0 Phase E/F の高速変化で main から大きく diverge
し、GitHub UI の rebase ボタンでは取り込めない状態に。PR #52 はさらに古い
base のため、そのまま cherry-pick すると rc1 で生きている
`RECORDING_ENABLED` / `AUDIO_RECORDING_ENABLED` / rustls
`install_default()` を消す破壊的差分を含んでいた。今回は各修正を main 起点
で再起票し（#59 / #60 / #61）、CHANGELOG / SECURITY.md の文章コンフリクト
だけ手動で解決して merge。

### Verification
- `cargo test -p streaming-engine --lib` — 347/347 pass
- `ctest --build-config Release --output-on-failure` — 36/36 pass
- main 上で全 CI ジョブ（Rust Streaming Engine / Companion App /
  Android OpenXR Client / OpenVR Driver DLL / Version Consistency）
  green
- TLS TOFU の Android 実機検証は GA 前の Out-of-Scope（PR #61 の test plan
  チェックリスト参照）

### Out-of-Scope (Carried over from rc1)
- 実機（NVIDIA GPU + VIVE Focus Vision）での 30 分以上の連続セッション
  耐久試験
- NVENC VUI フルレンジでの色彩確認（コード・テストは完了、目視は実機）
- HTC Face Tracking 実カメラ入力 → OSC ブリッジの end-to-end
- DRS（Dynamic Resolution Scaling）

## [3.0.0-rc1] - 2026-05-14

First release candidate for v3.0.0. Focuses on completion-grade UX,
test coverage, and release infrastructure. Every change in this entry
ships without requiring real hardware to validate; items that need a
NVIDIA GPU or Focus Vision headset are explicitly tagged as
"hardware-pending" in the Out-of-Scope section at the bottom.

### Security
- **TOFU 証明書ピン留めをクライアント側で実装:** TLS ハンドシェイク後にサーバ leaf cert の SHA-256 を `<app internal storage>/server_fingerprint.hex` に保存し、以降の接続で fingerprint 不一致なら接続を拒否する。これまで `MBEDTLS_SSL_VERIFY_NONE` で**証明書を一切検証していなかった**ため、同一 LAN 上の攻撃者が自前 TLS 証明書で MITM し PIN を盗聴可能だった。SECURITY.md が宣言していた緩和策が実際に動作するようになる
- **TLS 失敗時の平文フォールバック削除:** `TcpControlClient::connect()` は TLS / pinning に失敗した場合、平文に降格せず接続を拒否する。これまではハンドシェイクを破壊するだけで暗号化を剥がせる構造だった

### Companion App (Demo + UX)
- **`--demo` flag:** new `demo` module synthesizes a 60 s scripted
  state cycle (Disconnected → WaitingForPin "847251" → Connected with
  sine-animated latency / bitrate). Bypasses `status.json`, disables
  ADB scans, and shows a yellow "DEMO MODE — シミュレーション中"
  banner above the tab bar so reviewers / onboarding can exercise the
  whole UI without a VR rig. Title bar also flips to "Focus Vision
  PCVR — DEMO MODE" for visual unambiguity.
- **Home tab — recent activity tail:** collapsing "Recent activity"
  panel at the bottom of Home renders the last 10 lines of
  `status_log` in monospace. Default-collapsed so the tab stays calm;
  pops open when something needs explaining.
- **PIN expires-in countdown:** when the engine emits the new
  `pin_expires_in_seconds` field (and on demo's PIN-waiting phase),
  the Home tab renders `Expires in 4:58` beside the PIN, with yellow
  ≤60 s and red ≤30 s. Countdown is derived locally from the moment
  of receipt so the engine doesn't need to rewrite `status.json`
  every second to keep the timer alive.
- **Stats SVG export:** Settings tab gains an "Export Stats Graph
  (.svg)" button next to "Export Logs (zip)". Renders the last 30 s
  of latency / FPS / packet loss as a self-contained SVG via a pure
  function (egui-independent), with NaN/inf guards and BT.709-style
  axis labeling. SaveFileDialog via PowerShell.

### Companion App (UX, from prior commits)
- **Audio + APK + Window persistence:** the audio enable/bitrate
  toggle, last-used APK path, and (via eframe default) window position
  now survive a relaunch — `[audio]` and `[deploy]` sections joined
  `[video]` / `[sleep_mode]` / `[face_tracking]` / `[recording]` in
  `local.toml`. Sliding the audio bitrate slider previously did
  nothing; it now writes to disk and the engine reads it on next
  start.
- **Reset to defaults:** Settings tab gains a Maintenance group with a
  two-stage confirm button (`Reset to defaults` → `Confirm reset`
  for 3 s). Wipes every companion-side override and re-syncs the
  in-memory shadow state so the visible sliders match what was saved.
- **Recording dir validation:** inline diagnostic next to the output-
  dir input. Blank stays a no-op (engine falls back to
  `%APPDATA%/FocusVisionPCVR/recordings`); missing paths warn yellow
  (engine creates them at startup); a file-instead-of-dir errors red.
- **Engine-stopped banner:** Home tab shows a red banner when
  `status.json` is missing or its mtime is older than 5 s. Surfaces
  engine crashes or SteamVR not running. The banner can't restart the
  engine (it lives inside `vrserver.exe`); it directs the user to
  SteamVR's process-lifecycle instead.
- **SteamVR drivers dir via registry:** resolves the SteamVR install
  via the Windows registry (HKLM\SOFTWARE\Valve\Steam +
  WOW6432Node fallback), so the driver Install/Uninstall buttons
  work even when SteamVR isn't on the standard library path.
- **`CompanionApp` split into `ui/{home, deploy, settings}.rs`:**
  `main.rs` shrinks from 1209 LoC to 410, each tab is its own file.
  The render methods stay attached to `CompanionApp` via per-file
  `impl` blocks. Public/binary API unchanged.

### Streaming Engine
- **`status.json` schema +`pin_expires_in_seconds`:** new optional
  field carries `PIN_LIFETIME_SECONDS` (300 s, added to
  `fvp-common::constants`) so the companion can render a live
  countdown beside the PIN. Engine emits the value on PIN issue;
  the schema is forward-compatible (parser treats `None` as "old
  engine, no countdown").
- **`OscBridge::set_target`:** OSC destination is now configurable
  rather than const "127.0.0.1:9000". Production still defaults to
  the VRChat listener; the new integration test routes to a
  127.0.0.1:0 loopback receiver so the wire format is verified end-
  to-end without poking the user's real VRChat session.
- **NVENC VUI `applyVuiFromConfig` helper extracted:** the H.264 and
  HEVC VUI setup blocks in `nvenc_encoder.cpp::init` now share a
  template helper in the header. Same bits go out — but the helper
  is gtest-callable without spinning up a real encoder, so the
  full_range / BT.709 wiring is now covered by 6 unit tests.
- **`status.json` parser extracted:** `companion-app::status_parser`
  now owns the JSON → struct mapping with typed `ParsedStatus` /
  `ConnectionStatus` / `Subsystems`. `apply_parsed_status` is the
  new write-side seam — tests don't need egui to validate the state
  machine. Fixed a latent bug where the engine's 6-dash sentinel
  PIN briefly displayed as if it were a real code.
- **Audio recording runtime toggle (CONFIG_UPDATE 0x05):** mirrors
  the existing video toggle (0x03). Audio gating is independent of
  video, so an operator can pause audio recording mid-session while
  video keeps writing. The boot config (`recording.enabled`) still
  seeds both gates.
- **Thermal governor wired to engine:** `ThermalGovernor` +
  `NvmlThermalSource` were complete since Phase 1 but sat idle. The
  engine now constructs the governor once before the reconnect loop
  and calls `tick()` on each bitrate-adjust tick, applying the
  multiplier to `bitrate_ctrl.set_thermal_ceiling_bps`. On non-NVIDIA
  hosts or without the `nvml` cargo feature the governor is `None`
  and the block is a no-op. Threshold tuning remains a hardware
  follow-up.

### Thermal Governor (soft-skeleton, merged from earlier work)
- **`thermal::ThermalGovernor`:** GPU-temperature-driven cap on
  adaptive-bitrate ceiling. 75/85/90 °C (warn/limit/emergency),
  70 % at limit, 50 % at emergency, linear ramp back over
  `recovery_seconds` (default 30 s). `[thermal]` config section.
- **Bitrate arbitration explicit:** `arbitrate(ArbitrationInputs)
  -> ArbitrationOutcome` pure function exposes the multiplier
  priority order (Sustained > each reduction > Increase >
  Floor / Thermal clamp) and a winner-signal enum. Behaviour
  unchanged.
- **Reconnect state machine extracted:** `control::reconnect::ReconnectState`
  owns the accept-failure / reconnect-attempt counters and
  exponential backoff (1 → 16 s cap). Splits the two counters so a
  long Wi-Fi drop can't stop the engine just because reconnect
  attempts pile up.

### Tests
- **Workspace:** 313 → 450 (`+137`). Companion: 25 → 60 (`+35`).
  Driver C++ (gtest): 13 → 36 (`+23`).
- **Demo synthesizer:** 6 cases — phase transitions, PIN format,
  schema version, stats bounds, cycle wraparound.
- **PIN expires-in:** 4 cases — parser presence/absence,
  build_status_json emission/omission, demo countdown shape.
- **SVG export:** 5 cases — empty / partial / full history, NaN/inf
  guards, brand header.
- **OSC loopback (integration):** 4 cases — full UDP roundtrip with
  blendshapes → OSC wire bytes, separate lip/eye name tables, EMA
  attenuation on first frame, sub-threshold drop.
- **NVENC VUI:** 6 gtest cases — full-range true/false flag bits,
  videoSignalTypePresentFlag always-on, BT.709 metadata,
  videoFormat=Unspecified, H.264 alias parity.
- **status.json parser:** 12 cases — idle, waiting/real PIN,
  sentinel PIN, streaming with/without subsystems, pre-v3 payload,
  future schema_version, unknown status, malformed JSON, partial
  writes, latency-µs → ms, engine-format round-trip.
- **Audio recording toggle:** 4 cases — disable, enable, invalid
  value rejected, independence between audio and video gates.
- **Recording dir diagnostic:** 4 cases — blank accepted, existing
  dir, nonexistent (warning), file-not-dir (error).
- **Audio / APK config roundtrip:** 4 cases for the new
  `[audio]` and `[deploy]` sections (defaults, roundtrip, missing-
  section fallback, APK-path roundtrip).
- **Driver QP map + NVENC ABI:** 17 cases — off-screen gaze, mid-
  zone CTUs, zero-fovea-radius, huge-mid-radius, 1×1 / 0-sized /
  uneven CTU grids, all three foveated presets, case-sensitive
  preset lookup, NV_ENC_CODEC_CONFIG union sizing,
  `NVENCAPI_STRUCT_VERSION` non-zero.

### CI
- **Coverage (nightly):** `cargo-llvm-cov` via taiki-e/install-action
  produces Cobertura XML on `windows-latest`, uploaded as
  `coverage-cobertura` artifact. `continue-on-error: true` while
  baselines establish; threshold can be set once a few nights of
  data are in.
- **Installer build + Authenticode signing (merged from Phase D):**
  `installer-build` job assembles the NSIS installer from
  companion-app + driver artifacts; an Authenticode signing step
  fires when `WINDOWS_PFX_BASE64` / `WINDOWS_PFX_PASSWORD` repo
  secrets are present (PR builds skip it). Same pattern on the
  companion exe. `Get-AuthenticodeSignature` reports
  Valid / NotSigned in the action log.
- **Android release keystore:** `client/app/build.gradle.kts`
  wires `ANDROID_KEYSTORE_BASE64` + alias / password env vars,
  with an ephemeral-keystore fallback so `assembleRelease`
  succeeds even on forks / PRs.

### Docs
- **`docs/SIGNING.md`:** operator-side guide — cert acquisition,
  base-64 encoding for GitHub secrets, signtool wrapper invocation,
  timestamping URL choice, verification.
- **`docs/USER_GUIDE.md` / `TROUBLESHOOTING.md` / `FAQ.md`:**
  Japanese user-facing manuals (cherry-picked from
  `feat/v3-phase-f-docs`).

### Deferred to post-RC1
- DRS (Dynamic Resolution Scaling) and Hand-tracking — both need
  NVIDIA GPU + Focus Vision headset to verify and are out of scope
  for an "implement & validate" pass. (VUI full-range wiring was
  previously listed here; the bits land via the new
  `applyVuiFromConfig` helper, now covered by gtest. Real-encoder
  bitstream verification remains hardware-pending.)
- `engine.rs::run_streaming` split into session/frame/reconnection
  modules and FFI type unification (`TrackingData` /
  `ControllerState` into `fvp-common`). Both refactors are
  test-net-ready on this branch; deferred to keep the rc1 window
  short.
- Clippy regressions from Rust 1.94+ — newer lint rules
  (`manual_div_ceil`, `useless_vec` on test-only constants,
  `manual_is_multiple_of`) fire on pre-existing test code in
  `streaming-engine` lib tests, fuzz_tests, and video_pipeline_test.
  All new code added in this RC is clippy-clean; the existing
  warnings will be cleaned up in a follow-up.

## [Unreleased]

(All entries previously listed here moved into [3.0.0-rc1] above on the
release cut. Add new post-rc1 work below this header.)

## [2.2.1] - 2026-04-15

### Added
- **GccEstimator:** 独立した遅延ベース帯域推定モジュール。DelayTrend状態判定(Normal/Increasing/Overuse)、bitrate_multiplier、プロービング準備
- **BurstDetector:** Wi-Fi干渉(burst) vs 持続的混雑(sustained)の分類。LossPattern enum、500ms閾値でburst→sustained遷移
- **sent_packet_log:** engine.rsにRTP送信タイムスタンプ記録（HashMap<u16, u64>、5000エントリ上限）。GCC推定器の入力
- **congestion_controlトグル:** config.tomlで`congestion_control = "gcc" | "loss"`を選択可能。"loss"モードでは既存ロスベースのみ使用
- **AdaptiveFEC boost:** BurstDetector連携のboost機能（activate/deactivate）、1秒レート制限、effective_redundancy()
- **スライスFEC:** NALフレームを4分割し独立RSエンコード。IDRフレーム(>=16KB)で送信開始遅延を3-5ms→1-2msに短縮。`slice_fec_enabled`/`slice_count`設定
- **SlicedFecFrameDecoder (Client C++):** 4独立RSコンテキスト、u32 length prefix、100ms timeout、fvp_flags解析
- **IDR_REQUESTレート制限:** max 2/sec (500ms debounce)。スライスタイムアウトからのIDRストーム防止
- **fvp_flags統合:** pipeline.rsのflags hardcode → `fvp_flags::encode_simple()`に修正

### Changed
- **BitrateController:** adjust()がGccEstimatorとBurstDetectorの3引数に拡張。burst時はFEC吸収、sustained時は積極減速
- **BandwidthEstimator:** 遅延計算をGccEstimatorに分離。ロス率EWMAとRTT追跡のみに専念（単一責務）
- **engine.rs:** TRANSPORT_FEEDBACK受信時にGccEstimator.process_feedback()を即時実行（バッチ処理→リアルタイム処理）

### Fixed
- **max reductionバグ:** delay overuse(-10%)とloss(-20%)が累積して-28%になるバグを修正。候補の大きい方のみ採用するmax reduction方式に変更

### Tests
- **テスト313件に増加**（277→313、+36件）
- GccEstimator 7テスト（初期状態、安定リンク、overuse検出、underuse、単一/空feedback、multiplier範囲）
- BurstDetector 6テスト（初期状態、ロスなし、burst検出、sustained検出、閾値以下、回復）
- BitrateController +5テスト（burst抑制、sustained減速、UNDERUSE増速、天井clamp、データなし）
- congestion_control 3テスト（デフォルト、無効値、"loss"モード）
- AdaptiveFEC +3テスト（boost、レート制限、bandwidth_delta）
- SliceSplitter 8テスト（等分割、不均等、小フレーム、1バイト、空、データ整合性、count=2/8、count=0）
- スライスFECパイプライン 6テスト（4スライスencode、backward compat、payload len、空スライス、パケット数、flags統合）
- slice_count設定 3テスト（デフォルト、範囲外、正常値）

## [2.2.0] - 2026-04-10

### Added
- **適応FEC:** パケットロス率に応じてFEC冗長度を5-40%で自動調整。`AdaptiveFecController`がBandwidthEstimatorと連携
- **TCP再接続強化:** `DisconnectReason` enum（ClientRequested/ConnectionLost/ProtocolError）で切断理由を識別。ConnectionLost時は5秒間再接続待機
- **セッションログ:** JSONL形式のストリーミング統計記録（10秒間隔、60秒フラッシュ、7日ローテーション）
- **Protocol v3:** TRANSPORT_FEEDBACK (0x12) メッセージタイプ、FVPヘッダにslice_index/slice_count/stream_idフィールド追加
- **Protocol v3互換ゲート:** `fvp_flags::encode_compat()`でv2クライアントにはkeyframeビットのみ送信（後方互換性保証）
- **Adaptive FEC無効化オプション:** `adaptive_fec_enabled = false`で固定冗長度モード（デバッグ用）
- **メモリ監視:** `metrics/memory.rs` — GetProcessMemoryInfo (Win) / /proc/self/status (Linux) でプロセスRSS監視、1時間50MB超過で警告
- **SECURITY.md更新:** TCP再接続5秒PINスキップウィンドウの脅威モデル・緩和策を追記

### Changed
- **chronoクレート導入:** session_log.rsのカスタムISO 8601タイムスタンプをchrono::Utcに置換（カレンダー計算バグ根絶）
- **AdaptiveFecController:** ハードコード初期値20%を廃止、config.fec_redundancyを初期値として使用
- **engine.rs リファクタ:** ストリーミングループからupdate_adaptive_bitrate/check_sleep_mode/update_latency_atomics/log_periodic_statsを関数抽出

### Fixed
- **FEC config検証:** fec_redundancyが[min, max]範囲外の場合にクランプ + 警告ログ
- **FECテストコメント:** boundary_5_percent テストが>=5%ブラケットに入ることを正確に明記

### Tests
- **テスト263件に増加**（180→263、+83件）
- 適応FEC 12テスト（低/中/高ロス、ステップ制限、NaN、境界値、初期値クランプ）
- DisconnectReason 5テスト（ClientRequested、ProtocolError、enum一意性、TransportFeedback正常/異常）
- セッションログ 7テスト（ディレクトリ作成、書込、Drop、空フラッシュ、タイムスタンプ、ローテーション）
- メモリ監視 4テスト（ベースライン、ポーリング間隔、RSS取得、閾値ロジック）
- Protocol v3互換ゲート 3テスト（v1/v2/v3）
- FEC config検証 2テスト（範囲外クランプ、NaN）
- TransportFeedback 5テスト（ラウンドトリップ、空、oversized、truncated、too_short）
- FVP flags 4テスト（simple、full、max、overlap）

## [2.1.0] - 2026-04-07

### Added
- **FT表情プロファイル:** アバターごとの51ブレンドシェイプ感度調整（JSON保存/読込/削除）。OscBridgeがEMAスムージング後にweight適用
- **FT自動キャリブレーション:** 2ステップガイド式（リラックス→誇張）でmin/max収集、自動weight計算。CALIBRATE_START (0x60) / CALIBRATE_STATUS (0x61) プロトコル
- **フォベアテッドプリセット:** subtle (+3/+8)、balanced (+5/+15)、aggressive (+8/+25)、custom。`foveated.preset` config
- **GoogleTest基盤:** driver/CMakeLists.txtにGoogleTest v1.15.2追加。QPマップ計算テスト7件
- **QPマップ純粋関数化:** `computeQpDeltaMap()` を `qp_map.h` に抽出（テスト可能、NVENC非依存）

### Changed
- **FoveatedConfig:** preset enum追加、`effective_qp_offsets()` でプリセットから実効値を解決
- **OscBridge:** プロファイルweight適用対応、`set_profile()` メソッド追加

### Tests
- **テスト180件に増加**（168→180、+12件）
- FT表情プロファイル6テスト（デフォルト、weight、正規化、serialize、roundtrip）
- FTキャリブレーション6テスト（ステップ遷移、フレーム収集、full flow、定数値、index）
- C++ QPマップテスト7件（CTUグリッド、中心/角gaze、プリセット、サイズ検証）

## [2.0.0] - 2026-04-07

### Strategy
- **差別化先行戦略:** VIVE Hubが既に提供するDP/ハンドトラッキング/パススルーより、独自価値（レイテンシー最適化、FT強化、オープンソース）を優先
- **フェーズ再編成:** Phase 1=レイテンシー基盤、Phase 2=Foveated+FT Suite、Phase 3=ハードウェアパリティ

### Added
- **96fpsサポート:** RTPタイムスタンプをconfigフレームレートから動的計算。30-120fps対応
- **プロトコルバージョニング:** HELLO/HELLO_ACKにu16 protocol_version追加。未知メッセージは警告+スキップ（後方互換性維持）
- **UDPトランスポート最適化:** SO_RCVBUF/SO_SNDBUF 2MB + DSCP EF marking（非致命的フォールバック）
- **フルRGBカラーレンジ:** `video.full_range` config + FvpConfig FFI。NVENC VUIパラメータは実機検証待ち
- **レイテンシーウォーターフォール:** HMD内でencode/network/decode/renderの内訳を色分けバーで表示
- **HEARTBEAT_ACK:** PC側エンコード/トータルレイテンシーをHMDに送信

### Fixed
- **RTPタイムスタンプバグ:** `engine.rs:682`の`/90`ハードコードを修正。96fps/120fpsで正しいタイムスタンプを生成
- **フレームレート依存定数:** ビットレート調整間隔、ログ間隔、LatencyTrackerウィンドウをconfigから動的計算

### Changed
- **Config validate():** `Vec<String>` → `Vec<ConfigError>` に変更。構造化されたフィールド名付きエラー（graceful migration維持）

### Tests
- **テスト168件に増加**（156→168、+12件）
- RTPタイムスタンプ回帰テスト3件（90/96/120fps）
- ビットレート調整間隔スケーリングテスト1件
- プロトコルバージョニングテスト3件（encode/decode、空ペイロード、部分ペイロード）
- Config validation構造化エラーテスト更新

## [1.3.0] - 2026-04-07

### Added
- **コンフィグバリデーション:** bitrate/ports/framerate/smoothing/timeout の範囲チェック。不正値はデフォルトにフォールバック+ログ警告
- **コンパニオンアプリ設定UI:** 睡眠モード（enable/timeout）とFace Tracking（enable/smoothing）をGUIから設定可能に
- **サブシステムステータス表示:** Home画面にFT Active/Idle、Awake/Sleep、Audio OK/Off、Packet Loss%をリアルタイム表示
- **エラー通知改善:** ハプティクスドロップカウンター（AtomicU64）、オーディオ状態フラグ（AtomicBool）
- **HMDダッシュボードオーバーレイ:** VR内からビットレート調整・codec確認が可能な設定パネル
- **CONFIG_UPDATEプロトコル:** HMD→PC設定変更メッセージ（0x55）+ ACK（0x56）、値バリデーション付き
- **Atomic status.json:** temp+rename による部分読み取り防止

### Fixed
- **バージョン文字列:** "v1.0.0" 固定 → `CARGO_PKG_VERSION` から自動取得

### Tests
- **テスト156件に増加**（144→156、+12件）
- ハプティクスパイプライン5テスト（シリアライズ、チャネル満杯、roundtrip）
- コンフィグバリデーション7テスト（範囲外、NaN、ポート競合、エッジ値）

## [1.2.0] - 2026-04-07

### Fixed
- **Face Tracking接続修正:** FACE_DATA (0x35)のTCPハンドラが未実装でFTが完全に動作していなかった問題を修正。OscBridgeへのデータパスを接続
- **バッテリーレベル:** コントローラー状態のバッテリー値が100%固定だった問題を修正。Android sysfsから実値を読み取り

### Added
- **Face Tracking EMAスムージング:** blendshape値に指数移動平均フィルタを適用しジッター低減。係数はconfig設定可能（デフォルト0.6）
- **ハプティクスフィードバック:** SteamVR→PCドライバ→TCP→HMDの完全な振動パイプライン。`HAPTIC_EVENT (0x38)`プロトコルメッセージ、OpenXR `xrApplyHapticFeedback`
- **タッチセンサー:** trigger_touch、grip_touch、thumbstick_touch、thumbstick_clickをポーリング・SteamVRに送信
- **HTC VIVE Focus 3コントローラープロファイル:** フル入力バインディング（トリガー/グリップ/スティック/A/B/X/Y/タッチ）+ simple_controllerフォールバック
- **サムスティックデッドゾーン:** 0.1マグニチュード以下をゼロにクランプしドリフト防止
- **VR睡眠モード:** ヘッドポーズの動き検知で非活動検出。タイムアウト後にビットレート低下（80→8Mbps）+ 画面暗転。動き検知で自動復帰
- **[face_tracking]設定セクション:** enabled、smoothing、osc_port
- **[sleep_mode]設定セクション:** enabled、timeout_seconds、motion_threshold、sleep_bitrate_mbps

### Tests
- **テスト144件に増加**（134→144、+10件）
- Face Dataパーステスト、EMAスムージングテスト、SleepDetectorテスト5件

## [1.1.1] - 2026-04-07

### Fixed
- **TLS制御チャネル修正:** ハンドシェイク後にダミー平文ストリームを返していたバグを修正。制御メッセージが実際のTLS接続上で送受信されるように
- **direct_mode use-after-free修正:** `m_pendingTexture`を生ポインタからComPtrに変更し参照カウント安全性を確保
- **JNI参照リーク修正:** VideoDecoder::init()のエラーパスで`m_javaSurfaceTexture`のグローバル参照を解放
- **FFI unsafe修正:** `fvp_submit_encoded_nal`等のFFI関数に`unsafe`マーキング追加
- **unwrapパニック修正:** trackingポートパース、exportのfile_name()でパニックの可能性を除去
- **Clippy全警告解消:** 33件のclippy警告を修正（map_or→is_some_and、Default derive等）

### Performance
- **NALバッファ clone除去:** `std::mem::take()`で所有権移転。フレーム毎のmemcpy削減（1-5ms/frame）
- **FEC encoder clone除去:** `encode()`が所有権を受け取るように変更。データシャードのコピー削減（2-5ms/frame）
- **レイテンシートラッカー最適化:** `collect()`→`fold()`でVec allocationを除去

### Tests
- **テスト134件に増加**（119→134、+15件）
- TLS handshakeの実際のtokio_rustls接続テスト追加
- tracking パケットパース（gaze拡張、controller）テスト追加
- face tracking OSC全blendshape検証テスト追加
- audio encoder エッジケーステスト追加

## [1.1.0] - 2026-04-06

### Added
- **Codec切替UI:** コンパニオンアプリでH.264/H.265をワンクリック切替。config/local.tomlに保存
- **レイテンシーグラフ:** Homeタブにsparkline形式の30秒レイテンシー/FPSグラフ（egui_plot）
- **ログエクスポート:** PC/HMDログ+システム情報をzip化するワンクリックボタン。PII自動サニタイズ
- **HMD接続品質オーバーレイ:** VR視野にWi-Fi信号強度風の3バーアイコン。パケットロスに応じて緑/黄/赤
- **自動Codec選択:** 初回接続時にH.265/H.264の両方で5秒ベンチマーク→低レイテンシーなcodecを自動選択

## [1.0.0] - 2026-04-06

### Added
- **オーディオストリーミング:** WASAPI loopback → Opus → AAudio。PC音声をHMDで低遅延再生
- **FECクライアント復元:** GF(2^8) Vandermonde行列ベースReed-Solomon。パケットロス耐性
- **Timewarp:** Quaternionベース回転補正。デコード遅延時の頭部追従を維持
- **HeartbeatClient:** 500ms毎にHMD統計（パケットロス、デコードレイテンシー）をTCPで送信
- **適応ビットレート:** HMD実パケットロスをBandwidthEstimatorに接続
- **自動再接続:** 指数バックオフ（1s→16s、max 5回）。TCP切断時にセッション停止→再リッスン
- **エンジン状態IPC:** status.json経由でコンパニオンアプリとPIN/接続状態/統計を共有
- **JNI SurfaceTexture:** zero-copy MediaCodec→GLテクスチャ。ASurfaceTexture_fromSurfaceTexture
- **デコードレイテンシー計測:** submit-to-output wall time。logcatに90フレーム毎の平均出力
- **Android CI:** NDK r26b + Gradle 8.5 + OpenXR SDK FetchContent。APK自動ビルド

### Fixed
- **FecFrameDecoder uint16_t化:** >255シャードのIDRフレームのサイレント破損を防止
- **Timewarpシェーダー型修正:** sampler2D → samplerExternalOES + GL_OES_EGL_image_external_essl3
- **ADB deploy非同期化:** UIフリーズ防止

## [0.1.0.0] - 2026-03-27

### Added
- **PCコンパニオンアプリ:** ドライバーインストール、PIN表示、ADB経由HMDデプロイをGUIで操作。`cargo run -p focus-vision-companion`で起動
- **Real NVENCエンコード:** nvEncodeAPI64.dllをランタイムロード。SDK不要でビルド可能。NVIDIA非搭載環境はテストパターンに自動フォールバック
- **Video pipeline Phase 1 (PC):** NVENCエンコーダー、D3D11テクスチャ入力、DirectMode統合
- **Video pipeline Phase 2 (Android):** MediaCodecデコード (ASurfaceTexture zero-copy出力)、OpenGL ESレンダリング (external OESシェーダー)、UDP受信パイプライン
- **NALバリデーション:** H.265 NALヘッダー検証。不正パケットをドロップしデコーダークラッシュを防止
- **IDRキーフレーム制御:** TCP制御チャンネル経由のIDR_REQUESTメッセージ。E2E: Client→Rust→C++→NvencEncoder
- **新C ABI:** `fvp_submit_encoded_nal()` — C++側でエンコード済みNALデータをRustに渡す
- **`fvp_set_idr_callback()`** — Rust→C++ IDR通知用コールバック登録
- **デザインシステム:** DESIGN.md。Brutally Minimal美学、エメラルドグリーンアクセント、Instrument Serif + Geist + Geist Mono
- **テスト:** IDRフラグ伝搬、NAL→RTPラウンドトリップ、FECリカバリ、TCP制御メッセージ

### Changed
- **SubmittedFrame → EncodedFrame:** Rust側の型をリネーム。nal_data, is_idrフィールド
- **FecEncoder最適化:** ReedSolomonインスタンスをキャッシュ。shard数が同じなら再利用
- **NVENCをC++側に移動:** GPU バッファの跨言語共有を回避 (eng review決定)

### Fixed
- **TCPメッセージ長制限:** 64KB上限追加。悪意あるクライアントのOOM攻撃を防止
- **TCP切断検知:** CancellationToken連携。HMD切断時にストリーミングを停止
- **コールバック安全性:** Cleanup()の順序修正。fvp_shutdown()をs_instance=nullptr前に呼び出し
- **FVPヘッダーエンディアン:** Android側のframe_index/flags読み取りをLEに修正
- **Adversarial review修正 (7件):** FEC shard count計算、最終フレームデコード、整数プロモーション、TCP mid-message cancel、タイムスタンプオーバーフロー、3byte Annex B対応、デッドコード除去
