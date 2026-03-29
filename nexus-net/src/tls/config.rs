use std::sync::Arc;

use rustls::ClientConfig;
use rustls::pki_types::CertificateDer;

use super::TlsError;

/// Shared TLS configuration. Create once at startup, pass to each connection.
///
/// Wraps `Arc<ClientConfig>` — cloning is cheap.
///
/// # Examples
///
/// ```ignore
/// // Safe defaults: system root certs, TLS 1.2+1.3, AES-GCM preferred.
/// let config = TlsConfig::new()?;
///
/// // TLS 1.3 only, no certificate verification (testing).
/// let config = TlsConfig::builder()
///     .tls13_only()
///     .danger_no_verify()
///     .build()?;
/// ```
#[derive(Clone)]
pub struct TlsConfig {
    pub(crate) inner: Arc<ClientConfig>,
}

impl TlsConfig {
    /// Create with safe defaults.
    ///
    /// - System root certificates via `rustls-native-certs`
    /// - TLS 1.2 + 1.3 (both supported)
    /// - aws-lc-rs crypto backend (AES-GCM with AES-NI)
    pub fn new() -> Result<Self, TlsError> {
        Self::builder().build()
    }

    /// Create a builder for custom configuration.
    #[must_use]
    pub fn builder() -> TlsConfigBuilder {
        TlsConfigBuilder {
            custom_roots: Vec::new(),
            skip_system_certs: false,
            no_verify: false,
            tls13_only: false,
        }
    }
}

/// Builder for [`TlsConfig`].
pub struct TlsConfigBuilder {
    custom_roots: Vec<CertificateDer<'static>>,
    skip_system_certs: bool,
    no_verify: bool,
    tls13_only: bool,
}

impl TlsConfigBuilder {
    /// Add a custom root certificate (DER-encoded).
    ///
    /// Useful for internal CAs or self-signed certificates.
    #[must_use]
    pub fn add_root_cert(mut self, der: impl Into<CertificateDer<'static>>) -> Self {
        self.custom_roots.push(der.into());
        self
    }

    /// Skip loading system root certificates.
    ///
    /// Use when providing all root certificates manually.
    #[must_use]
    pub fn skip_system_certs(mut self) -> Self {
        self.skip_system_certs = true;
        self
    }

    /// Disable certificate verification entirely.
    ///
    /// # Safety
    ///
    /// This disables all server identity checks. Use only for testing
    /// against local servers with self-signed certificates.
    #[must_use]
    pub fn danger_no_verify(mut self) -> Self {
        self.no_verify = true;
        self
    }

    /// Restrict to TLS 1.3 only.
    ///
    /// TLS 1.3 has a simpler handshake (1-RTT vs 2-RTT) and mandatory
    /// forward secrecy. Disable TLS 1.2 if all endpoints support 1.3.
    #[must_use]
    pub fn tls13_only(mut self) -> Self {
        self.tls13_only = true;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> Result<TlsConfig, TlsError> {
        let versions: &[&rustls::SupportedProtocolVersion] = if self.tls13_only {
            &[&rustls::version::TLS13]
        } else {
            rustls::ALL_VERSIONS
        };

        let config = if self.no_verify {
            ClientConfig::builder_with_protocol_versions(versions)
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoVerifier))
                .with_no_client_auth()
        } else {
            let mut root_store = rustls::RootCertStore::empty();

            if !self.skip_system_certs {
                let result = rustls_native_certs::load_native_certs();
                if result.certs.is_empty() {
                    return Err(TlsError::NoRootCerts);
                }
                root_store.add_parsable_certificates(result.certs);
            }

            for cert in self.custom_roots {
                root_store.add(cert).map_err(TlsError::Rustls)?;
            }

            if root_store.is_empty() {
                return Err(TlsError::NoRootCerts);
            }

            ClientConfig::builder_with_protocol_versions(versions)
                .with_root_certificates(root_store)
                .with_no_client_auth()
        };

        Ok(TlsConfig {
            inner: Arc::new(config),
        })
    }
}

/// Certificate verifier that accepts everything. Testing only.
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
