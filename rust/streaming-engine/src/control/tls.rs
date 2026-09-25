use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use rustls::ServerConfig;
use sha2::{Digest, Sha256};
use tokio_rustls::TlsAcceptor;

/// File name of the persisted server identity inside the per-user data dir
/// (`%APPDATA%/FocusVisionPCVR/` on Windows).
const IDENTITY_FILE_NAME: &str = "tls_identity.bin";

/// On-disk container: magic | cert_len u32 LE | cert DER | key_len u32 LE |
/// PKCS#8 key DER | SHA-256(cert || key). Cert and key live in one file so a
/// single atomic rename can never leave a cert paired with the wrong key, and
/// the trailing digest rejects a truncated or bit-flipped file instead of
/// serving a certificate whose fingerprint no paired headset recognises.
const IDENTITY_MAGIC: &[u8; 8] = b"FVPTLS01";
/// Real values are a few hundred bytes (ECDSA P-256); anything larger is a
/// corrupt length field, not a certificate.
const MAX_IDENTITY_PART_LEN: usize = 16 * 1024;

/// The server's TLS certificate + private key and the certificate's SHA-256
/// fingerprint (lowercase hex, what the HMD pins on first use).
pub struct TlsIdentity {
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    fingerprint: String,
}

impl TlsIdentity {
    /// Generate a fresh self-signed certificate. rcgen's default validity
    /// (1975 → 4096) means a persisted identity never needs rotation for
    /// expiry; the client authenticates it by fingerprint, not by chain.
    pub fn generate() -> Result<Self, Box<dyn std::error::Error>> {
        let key_pair = rcgen::KeyPair::generate()?;
        let cert_params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
        let cert = cert_params.self_signed(&key_pair)?;
        Ok(Self::from_parts(cert.der().to_vec(), key_pair.serialize_der()))
    }

    fn from_parts(cert_der: Vec<u8>, key_der: Vec<u8>) -> Self {
        let fingerprint = sha256_hex(&cert_der);
        Self { cert_der, key_der, fingerprint }
    }

    /// SHA-256 of the DER certificate, lowercase hex (64 chars).
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Build a TLS acceptor serving this identity.
    pub fn acceptor(&self) -> Result<TlsAcceptor, Box<dyn std::error::Error>> {
        let certs = vec![rustls::pki_types::CertificateDer::from(self.cert_der.clone())];
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(self.key_der.clone());
        let config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key.into())
            .map_err(|e| format!("TLS config error: {e}"))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 4 + self.cert_der.len() + 4 + self.key_der.len() + 32);
        out.extend_from_slice(IDENTITY_MAGIC);
        out.extend_from_slice(&(self.cert_der.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.cert_der);
        out.extend_from_slice(&(self.key_der.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.key_der);
        let mut hasher = Sha256::new();
        hasher.update(&self.cert_der);
        hasher.update(&self.key_der);
        out.extend_from_slice(&hasher.finalize());
        out
    }

    /// Parse the on-disk container. `None` for any structural problem
    /// (bad magic, oversized or overrunning length, trailing bytes, digest
    /// mismatch) — the caller regenerates rather than trusting it.
    fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let rest = bytes.strip_prefix(IDENTITY_MAGIC.as_slice())?;
        let (cert_der, rest) = split_len_prefixed(rest)?;
        let (key_der, rest) = split_len_prefixed(rest)?;
        if rest.len() != 32 {
            return None;
        }
        let mut hasher = Sha256::new();
        hasher.update(cert_der);
        hasher.update(key_der);
        if hasher.finalize().as_slice() != rest {
            return None;
        }
        Some(Self::from_parts(cert_der.to_vec(), key_der.to_vec()))
    }
}

fn split_len_prefixed(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    let len_bytes: [u8; 4] = buf.get(..4)?.try_into().ok()?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len == 0 || len > MAX_IDENTITY_PART_LEN {
        return None;
    }
    let body = buf.get(4..4 + len)?;
    Some((body, &buf[4 + len..]))
}

/// Load the persisted identity at `path`, or generate one and persist it.
///
/// The HMD pins the server certificate on first connect (TOFU) and refuses
/// any other certificate afterwards, so the identity must survive engine
/// restarts and reconnects. A corrupt file is moved aside to `<path>.bak`
/// and replaced — already-paired headsets will then report a fingerprint
/// mismatch and must clear their pin, which is logged loudly here. A failed
/// write is non-fatal: the fresh identity is still served for this process.
pub fn load_or_create_identity(path: &Path) -> Result<TlsIdentity, Box<dyn std::error::Error>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            if let Some(identity) = TlsIdentity::from_bytes(&bytes) {
                if identity.acceptor().is_ok() {
                    log::info!("TLS identity loaded from {}", path.display());
                    return Ok(identity);
                }
            }
            let backup = path.with_extension("bin.bak");
            log::error!(
                "TLS identity at {} is corrupt — moving it to {} and generating a new one. \
                 Headsets paired with the old certificate will refuse to connect until \
                 their pinned fingerprint is cleared.",
                path.display(),
                backup.display(),
            );
            let _ = std::fs::rename(path, &backup);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            log::warn!("Cannot read TLS identity {}: {} — generating a new one", path.display(), e);
        }
    }

    let identity = TlsIdentity::generate()?;
    match persist_identity(path, &identity) {
        Ok(()) => log::info!("TLS identity generated and saved to {}", path.display()),
        Err(e) => log::warn!(
            "Could not save TLS identity to {}: {} — the certificate will change on the \
             next engine start and paired headsets will have to re-pin it",
            path.display(),
            e,
        ),
    }
    Ok(identity)
}

/// Atomic write (temp file + rename) so a crash mid-write never leaves a
/// half-written identity behind.
fn persist_identity(path: &Path, identity: &TlsIdentity) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("bin.tmp");
    write_private_file(&tmp, &identity.to_bytes())?;
    std::fs::rename(&tmp, path)
}

/// Write a file readable only by the current user. On Windows the file
/// inherits the per-user ACL of `%APPDATA%`; on Unix we set 0600 explicitly.
fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

/// Where the engine keeps its identity. Tests get `None` (in-memory only) so
/// `cargo test` never touches the developer's real `%APPDATA%`.
fn identity_path() -> Option<PathBuf> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        dirs_next::data_dir().map(|d| d.join("FocusVisionPCVR").join(IDENTITY_FILE_NAME))
    }
}

static SHARED_ACCEPTOR: Mutex<Option<(TlsAcceptor, String)>> = Mutex::new(None);

/// The engine-wide TLS acceptor, backed by the persisted identity.
///
/// `TcpControlServer::new()` runs on every accept-loop iteration; before
/// this, each call minted a fresh certificate, so the fingerprint the HMD
/// pinned on first pairing was gone by the next reconnect and every later
/// connection failed the client's TOFU check. The identity is loaded (or
/// created) once per process and reused, and persisted so it also survives
/// SteamVR restarts. Returns (acceptor, SHA-256 fingerprint hex).
pub fn shared_tls_acceptor() -> Result<(TlsAcceptor, String), Box<dyn std::error::Error>> {
    let mut guard = SHARED_ACCEPTOR.lock().unwrap_or_else(|p| p.into_inner());
    if let Some((acceptor, fingerprint)) = guard.as_ref() {
        return Ok((acceptor.clone(), fingerprint.clone()));
    }
    let identity = match identity_path() {
        Some(path) => load_or_create_identity(&path)?,
        None => TlsIdentity::generate()?,
    };
    let acceptor = identity.acceptor()?;
    let fingerprint = identity.fingerprint().to_string();
    log::info!("TLS certificate fingerprint: {}", fingerprint);
    *guard = Some((acceptor.clone(), fingerprint.clone()));
    Ok((acceptor, fingerprint))
}

/// Generate a one-off self-signed certificate and build a TLS acceptor.
/// Not persisted — production code uses [`shared_tls_acceptor`].
/// Returns (acceptor, certificate SHA-256 hash hex string).
pub fn create_tls_acceptor() -> Result<(TlsAcceptor, String), Box<dyn std::error::Error>> {
    let identity = TlsIdentity::generate()?;
    Ok((identity.acceptor()?, identity.fingerprint().to_string()))
}

fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(data);
    let bytes: &[u8] = digest.as_slice();
    let mut hex = String::with_capacity(64);
    for byte in bytes {
        write!(hex, "{:02x}", byte).unwrap();
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install_crypto_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn test_create_tls_acceptor_succeeds() {
        install_crypto_provider();
        let result = create_tls_acceptor();
        assert!(result.is_ok(), "Failed: {:?}", result.err());
    }

    #[test]
    fn test_certificate_fingerprint_is_64_hex_chars() {
        install_crypto_provider();
        let (_, fp) = create_tls_acceptor().unwrap();
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_different_calls_produce_different_certs() {
        install_crypto_provider();
        let (_, fp1) = create_tls_acceptor().unwrap();
        let (_, fp2) = create_tls_acceptor().unwrap();
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_shared_acceptor_is_stable_across_calls() {
        // REGRESSION: the control server is rebuilt on every accept-loop
        // iteration. The fingerprint must not change between iterations or
        // the HMD's TOFU pin rejects every reconnect.
        install_crypto_provider();
        let (_, fp1) = shared_tls_acceptor().unwrap();
        let (_, fp2) = shared_tls_acceptor().unwrap();
        assert_eq!(fp1, fp2);
    }

    #[test]
    fn test_identity_persists_and_is_reused() {
        install_crypto_provider();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(IDENTITY_FILE_NAME);

        let first = load_or_create_identity(&path).unwrap();
        assert!(path.exists(), "identity must be written to disk");
        let second = load_or_create_identity(&path).unwrap();
        assert_eq!(first.fingerprint(), second.fingerprint(),
            "reloading must serve the same certificate (engine restart case)");
        assert!(second.acceptor().is_ok());
    }

    #[test]
    fn test_identity_bytes_roundtrip() {
        install_crypto_provider();
        let id = TlsIdentity::generate().unwrap();
        let parsed = TlsIdentity::from_bytes(&id.to_bytes()).expect("roundtrip");
        assert_eq!(parsed.fingerprint(), id.fingerprint());
        assert_eq!(parsed.key_der, id.key_der);
    }

    #[test]
    fn test_identity_from_bytes_rejects_damage() {
        install_crypto_provider();
        let good = TlsIdentity::generate().unwrap().to_bytes();

        assert!(TlsIdentity::from_bytes(&[]).is_none());
        assert!(TlsIdentity::from_bytes(&good[..good.len() - 1]).is_none(), "truncated");

        let mut extra = good.clone();
        extra.push(0);
        assert!(TlsIdentity::from_bytes(&extra).is_none(), "trailing bytes");

        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xFF;
        assert!(TlsIdentity::from_bytes(&bad_magic).is_none(), "bad magic");

        let mut flipped = good.clone();
        flipped[20] ^= 0x01; // inside the certificate
        assert!(TlsIdentity::from_bytes(&flipped).is_none(), "digest must catch bit flips");

        let mut huge_len = good.clone();
        huge_len[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(TlsIdentity::from_bytes(&huge_len).is_none(), "oversized length");
    }

    #[test]
    fn test_corrupt_identity_is_backed_up_and_regenerated() {
        install_crypto_provider();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(IDENTITY_FILE_NAME);
        std::fs::write(&path, b"not an identity").unwrap();

        let id = load_or_create_identity(&path).unwrap();
        assert_eq!(id.fingerprint().len(), 64);
        assert_eq!(
            std::fs::read(path.with_extension("bin.bak")).unwrap(),
            b"not an identity",
            "the corrupt file must be preserved for diagnosis"
        );
        // The replacement is persisted and stable from here on.
        let again = load_or_create_identity(&path).unwrap();
        assert_eq!(again.fingerprint(), id.fingerprint());
    }
}
