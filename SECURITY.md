# Security

## Threat Model

Focus Vision PCVR communicates over Wi-Fi between a Windows PC and an Android HMD. The primary threat is an attacker on the same local network.

### Mitigated Threats

| Threat | Mitigation |
|--------|-----------|
| PIN brute-force | 6-digit PIN (1M combinations), 5 attempts then 300s lockout, cryptographic RNG |
| Man-in-the-middle | TLS 1.3 on TCP control channel (rustls server, MbedTLS client) + TOFU pinning (below) |
| PIN eavesdropping | PIN sent only over TLS-encrypted channel; client refuses any plaintext fallback |
| Server impersonation | TOFU certificate pinning: client computes SHA-256 of the server's leaf cert after the TLS handshake and persists it to `<app internal storage>/server_fingerprint.hex`. Subsequent connections refuse any cert that does not match the pinned hash. To re-pair with a different server, delete the file. |
| PIN prediction | `rand::random()` (cryptographic CSPRNG) replaces `subsec_nanos()` |
| CONFIG_UPDATE injection | TLS authentication required + input validation (range checks on bitrate 10-200, codec enum) |

### Known Limitations

| Threat | Status | Notes |
|--------|--------|-------|
| UDP stream encryption | Not implemented | Video/audio/tracking sent as plaintext UDP. SRTP planned for future. |
| Replay attacks | Partially mitigated | TLS prevents replay on control channel. UDP streams have no replay protection. |
| Session hijacking | Low risk | Once paired, no re-authentication. Session bound to TCP connection lifetime. |
| TCP reconnect PIN skip | Low risk | 5s window after connection loss where client can reconnect without PIN. Mitigated by TLS session resumption (session ticket verifies same client cryptographically) + TOFU certificate pinning (SHA-256 fingerprint). New clients always require PIN. Window is short (5s) and attacker would need to impersonate the TLS session. |
| Certificate rotation | Manual | New cert generated each server restart. No automated rotation. |
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
