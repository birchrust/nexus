/// Directional change or anomaly direction.
///
/// Used by algorithms that detect shifts, anomalies, or trends in a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// No directional change detected.
    Neutral,
    /// Upward: mean shifted up, value is high, trend is rising.
    Rising,
    /// Downward: mean shifted down, value is low, trend is falling.
    Falling,
}

/// System condition state.
///
/// Used by algorithms that monitor health, saturation, or pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    /// Within acceptable bounds.
    Normal,
    /// Exceeded threshold — degraded performance.
    Degraded,
}

/// Configuration error from building a stats primitive.
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

#[cfg(feature = "std")]
impl std::error::Error for ConfigError {}

/// Error returned when a streaming update receives invalid data.
///
/// The library distinguishes two failure categories:
/// - **Data errors** (NaN, Inf) → returned as `Result<_, DataError>`
/// - **Programmer errors** (wrong dimensions, out-of-range) → panic
///
/// The library makes no assumptions about the caller's policy.
/// Each system has different implications for bad data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataError {
    /// Input contained NaN.
    NotANumber,
    /// Input contained positive or negative infinity.
    Infinite,
}

impl core::fmt::Display for DataError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotANumber => write!(f, "input contained NaN"),
            Self::Infinite => write!(f, "input contained infinity"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DataError {}
