use std::sync::Arc;
use rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

/// Generate a self-signed TLS certificate and build a TLS acceptor.
/// Returns (acceptor, certificate SHA-256 hash hex string).
pub fn create_tls_acceptor() -> Result<(TlsAcceptor, String), Box<dyn std::error::Error>> {
    let key_pair = rcgen::KeyPair::generate()?;
    let cert_params = rcgen::CertificateParams::new(vec!["localhost".to_string()])?;
    let cert = cert_params.self_signed(&key_pair)?;

    let cert_der = cert.der().clone();
    let key_der = key_pair.serialize_der();

    // Compute SHA-256 fingerprint for TOFU pinning
    let fingerprint = sha256_hex(&cert_der);
    log::info!("TLS certificate fingerprint: {}", fingerprint);

    let certs = vec![cert_der];
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(key_der);

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key.into())
        .map_err(|e| format!("TLS config error: {e}"))?;

    Ok((TlsAcceptor::from(Arc::new(config)), fingerprint))
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Sha256, Digest};
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
}
