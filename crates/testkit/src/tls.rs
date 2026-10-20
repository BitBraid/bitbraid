//! Certificates for the servers tests start: an authority of the test's
//! own, made in memory, that signs a certificate for the names a server
//! answers to. A client is told to trust the authority (git through
//! `http.sslCAInfo`, the application's client with the certificate in
//! DER, a program through `SSL_CERT_FILE`) and nothing else; nothing is
//! added to the system's store.
//!
//! Sources: the documentation of `rcgen` (`CertificateParams`, `Issuer`,
//! `KeyPair`, `IsCa`, `KeyUsagePurpose`) and of `rustls`
//! (`ServerConfig`, `crypto::ring`); RFC 7468 (the PEM text form); RFC
//! 5280 §4.2.1.9 (basic constraints).

use std::{path::Path, sync::Arc};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// What a TLS server is set up with, as `rustls` has it: what
/// [`Authority::server_config`] makes, for a crate that does not depend
/// on `rustls` itself.
pub use rustls::ServerConfig;

/// An authority that signs certificates for test servers.
pub struct Authority {
    issuer: Issuer<'static, KeyPair>,
    certificate: Vec<u8>,
}

impl std::fmt::Debug for Authority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Authority").finish_non_exhaustive()
    }
}

/// DER as PEM text, with this label (`CERTIFICATE`).
fn pem(label: &str, der: &[u8]) -> String {
    let text = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in text.as_bytes().chunks(64) {
        out.push_str(&String::from_utf8_lossy(line));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

impl Authority {
    /// A new authority, with a key made for it.
    pub fn new() -> Result<Self> {
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, "BitBraid test authority");
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let key = KeyPair::generate()?;
        let certificate = params.self_signed(&key)?.der().to_vec();
        Ok(Self {
            issuer: Issuer::new(params, key),
            certificate,
        })
    }

    /// The authority's certificate, in DER.
    pub fn certificate_der(&self) -> Vec<u8> {
        self.certificate.clone()
    }

    /// The authority's certificate, as PEM text.
    pub fn certificate_pem(&self) -> String {
        pem("CERTIFICATE", &self.certificate)
    }

    /// Write the authority's certificate as PEM text to `path`, for a
    /// client that reads the authorities it trusts from a file.
    pub fn write_pem(&self, path: &Path) -> Result<()> {
        std::fs::write(path, self.certificate_pem())
            .with_context(|| format!("writing {}", path.display()))
    }

    /// A server configuration that presents a certificate for `names`
    /// (host names or IP addresses), signed by this authority.
    pub fn server_config(&self, names: &[&str]) -> Result<Arc<ServerConfig>> {
        let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        let mut params = CertificateParams::new(names.clone())?;
        if let Some(first) = names.first() {
            params
                .distinguished_name
                .push(DnType::CommonName, first.as_str());
        }
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        let key = KeyPair::generate()?;
        let certificate: CertificateDer<'static> =
            params.signed_by(&key, &self.issuer)?.der().clone();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)?;
        Ok(Arc::new(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pem_text_is_base64_in_lines_between_its_markers() {
        let text = pem("CERTIFICATE", &[0u8; 100]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&"-----BEGIN CERTIFICATE-----"));
        assert_eq!(lines.last(), Some(&"-----END CERTIFICATE-----"));
        assert_eq!(lines[1].len(), 64);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn an_authority_signs_a_server_certificate() {
        let authority = Authority::new().unwrap();
        assert!(authority.certificate_pem().starts_with("-----BEGIN"));
        assert!(!authority.certificate_der().is_empty());
        authority
            .server_config(&["git.example.test", "127.0.0.1"])
            .unwrap();
    }
}
