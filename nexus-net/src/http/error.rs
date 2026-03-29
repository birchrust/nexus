/// HTTP parsing error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// Request or response head is malformed.
    Malformed,
    /// Too many headers (exceeds configured limit).
    TooManyHeaders,
    /// Head section exceeds size limit.
    HeadTooLarge { max: usize },
    /// Read buffer full.
    BufferFull { needed: usize, available: usize },
    /// Write buffer too small for the HTTP message.
    BufferTooSmall { needed: usize, available: usize },
    /// Header name or value contains invalid characters (CR/LF).
    InvalidHeaderValue,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed => write!(f, "malformed HTTP message"),
            Self::TooManyHeaders => write!(f, "too many HTTP headers"),
            Self::HeadTooLarge { max } => write!(f, "HTTP head exceeds {max} bytes"),
            Self::BufferFull { needed, available } => {
                write!(f, "buffer full: need {needed}, {available} available")
            }
            Self::BufferTooSmall { needed, available } => {
                write!(f, "write buffer too small: need {needed} bytes, have {available}")
            }
            Self::InvalidHeaderValue => {
                write!(f, "header name or value contains CR/LF")
            }
        }
    }
}

impl std::error::Error for HttpError {}
