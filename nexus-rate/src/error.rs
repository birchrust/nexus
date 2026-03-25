/// Configuration error from building a rate limiter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A required parameter was not set.
    Missing(&'static str),
    /// A parameter value is invalid.
    Invalid(&'static str),
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing(param) => write!(f, "configuration error: {param} must be set"),
            Self::Invalid(msg) => write!(f, "configuration error: {msg}"),
        }
    }
}

impl std::error::Error for ConfigError {}
