# Security

## Threat Model

Focus Vision PCVR communicates over Wi-Fi between a Windows PC and an Android HMD. The primary threat is an attacker on the same local network.

### Mitigated Threats

| Threat | Mitigation |
|--------|-----------|
| PIN brute-force | 6-digit PIN (1M combinations), 5 attempts then 300s lockout, cryptographic RNG. A PIN also expires 300 s after it is issued (`PIN_LIFETIME_SECONDS`) and is replaced, so a leaked PIN does not stay valid while the engine waits |
| Man-in-the-middle | TLS 1.3 on TCP control channel (rustls server, MbedTLS client) + TOFU pinning (below) |
| PIN eavesdropping | PIN sent only over TLS-encrypted channel; client refuses any plaintext fallback |
| Server impersonation | TOFU certificate pinning: client computes SHA-256 of the server's leaf cert after the TLS handshake and persists it to `<app internal storage>/server_fingerprint.hex`. Subsequent connections refuse any cert that does not match the pinned hash. To re-pair with a different server, delete the file. The server's certificate + key are persisted to `%APPDATA%/FocusVisionPCVR/tls_identity.bin` and reused across reconnects and engine restarts, so the pinned fingerprint stays valid. |
| PIN prediction | `rand::random()` (cryptographic CSPRNG) replaces `subsec_nanos()` |
| CONFIG_UPDATE injection | TLS authentication required + input validation (range checks on bitrate 10-200, codec enum) |
| Handshake stall (accept-loop DoS) | Every handshake phase is time-bounded: TLS 10 s, HELLO / STREAM_START 10 s each, PIN_RESPONSE 30 s. A client that connects and goes silent (or trickles bytes) is dropped instead of blocking the sequential accept loop for the real HMD. |
| Tracking injection from other LAN hosts | The tracking receiver accepts UDP only from the IP of the HMD that completed TLS + PIN pairing, and only while that session is up; everything else is dropped before parsing. |

### Known Limitations

| Threat | Status | Notes |
|--------|--------|-------|
| UDP stream encryption | Not implemented | Video/audio/tracking sent as plaintext UDP. SRTP planned for future. |
| UDP source spoofing | Partially mitigated | Tracking is bound to the paired HMD's IP, but UDP carries no per-packet authentication: an attacker on the LAN who forges that source IP can still inject poses. Per-packet MAC / DTLS planned for future. |
| Replay attacks | Partially mitigated | TLS prevents replay on control channel. UDP streams have no replay protection. |
| Session hijacking | Low risk | Once paired, no re-authentication. Session bound to TCP connection lifetime. |
| TCP reconnect window | Low risk | For 5 s after a connection is lost (not after a clean DISCONNECT), the engine accepts the **same PIN** the session paired with, so the HMD can reconnect without the user entering a new one. There is no PIN skip and no TLS session resumption: the reconnecting client still runs the full TLS + PIN handshake, the 5-attempt limit and lockout still apply, and after the window a new PIN is issued. The risk is someone who already knows that PIN (it was on the companion's screen) getting in during the window; the client-side TOFU pin still protects the HMD from a fake server. A per-session reconnect token sent inside TLS would remove the PIN reuse but needs a protocol change. |
| Certificate rotation | Manual | One persisted identity (`tls_identity.bin`), no automated rotation. To rotate, delete the file; paired headsets then have to clear their pinned fingerprint. A corrupt file is moved to `tls_identity.bin.bak` and replaced (same re-pin requirement). |
| TLS private key at rest | Low risk | `tls_identity.bin` holds the private key unencrypted, protected by the per-user ACL of `%APPDATA%` (0600 on Unix). Anyone with access to the user profile can impersonate the server. |
| Session recording file disclosure | Low risk / user-controlled | Disabled by default. When `[recording] enabled = true`, raw Annex B video (.h265/.h264) and 16-bit PCM WAV audio are written to `%APPDATA%/FocusVisionPCVR/recordings/` (or user-specified dir). Protected only by OS filesystem permissions on the user's profile. Files may contain gaze coordinates, controller inputs, and desktop screen contents — **review before sharing externally**. No in-product PII scrubbing. |

### Architecture

```
Control Channel (port 9944):
  TCP → TLS 1.3 → Message framing → PIN pairing → Streaming
  CONFIG_UPDATE (0x55): HMD → PC config change (bitrate, codec)
    - Only accepted from TLS-authenticated clients
    - Values validated: bitrate [10-200] Mbps, codec enum [0,1]
    - ACK (0x56) sent back with accept/reject status

Data Channels (UDP):
  Video (9946): RTP + FEC, plaintext
  Tracking (9947): Head pose + eye gaze, plaintext
  Audio (9948): Opus encoded, plaintext

Recording (optional, user-enabled):
  Local filesystem write to %APPDATA%/FocusVisionPCVR/recordings/
  Disabled by default; no network exposure
```

### Responsible Disclosure

Report security issues via GitHub Issues (private if sensitive) or email.

### Dependencies

| Library | Purpose | Version |
|---------|---------|---------|
| rustls | TLS server | 0.23 |
| tokio-rustls | Async TLS | 0.26 |
| rcgen | Self-signed cert generation | 0.13 |
| sha2 | Certificate fingerprint | 0.10 |
| rand | Cryptographic PIN generation | 0.8 |
| MbedTLS | TLS client (Android NDK) | 3.6.2 |
