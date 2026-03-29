use std::fmt;

/// TLS operation error.
#[derive(Debug)]
pub enum TlsError {
    /// rustls protocol error (handshake failure, certificate error, etc.)
    Rustls(rustls::Error),
    /// I/O error during TLS operations.
    Io(std::io::Error),
    /// Invalid hostname for SNI.
    InvalidHostname(String),
    /// No system root certificates found.
    NoRootCerts,
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rustls(e) => write!(f, "TLS error: {e}"),
            Self::Io(e) => write!(f, "TLS I/O error: {e}"),
            Self::InvalidHostname(h) => write!(f, "invalid TLS hostname: {h}"),
            Self::NoRootCerts => write!(f, "no system root certificates found"),
        }
    }
}

impl std::error::Error for TlsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rustls(e) => Some(e),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rustls::Error> for TlsError {
    fn from(e: rustls::Error) -> Self {
        Self::Rustls(e)
    }
}

impl From<std::io::Error> for TlsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
