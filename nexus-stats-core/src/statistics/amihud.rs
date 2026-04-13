use crate::smoothing::EmaF64;

/// Streaming Amihud ILLIQ ratio — illiquidity measure.
///
/// Computes the exponentially weighted average of |return| / dollar_volume.
/// Higher values indicate less liquid instruments (larger price impact per
/// unit of volume). This is the standard Amihud (2002) illiquidity measure.
///
/// # Examples
///
/// ```
/// use nexus_stats_core::statistics::AmihudF64;
///
/// let mut amihud = AmihudF64::builder()
///     .alpha(0.05)
///     .build()
///     .unwrap();
///
/// amihud.update(0.01, 1_000_000.0).unwrap(); // 1% return, $1M volume
/// let illiq = amihud.illiq();
/// assert!(illiq > 0.0);
/// ```
#[derive(Debug, Clone)]
pub struct AmihudF64 {
    ema: EmaF64,
    count: u64,
}

/// Builder for [`AmihudF64`].
#[derive(Debug, Clone)]
pub struct AmihudF64Builder {
    alpha: Option<f64>,
}

impl AmihudF64 {
    /// Creates a builder.
    #[inline]
    #[must_use]
    pub fn builder() -> AmihudF64Builder {
        AmihudF64Builder { alpha: None }
    }

    /// Feed an absolute return and dollar volume for one period.
    ///
    /// `abs_return` should be the absolute value of the return (e.g., `|r|`).
    /// `dollar_volume` must be positive.
    ///
    /// # Errors
    ///
    /// Returns `DataError` if either value is NaN or infinite.
    #[inline]
    pub fn update(&mut self, abs_return: f64, dollar_volume: f64) -> Result<(), crate::DataError> {
        check_finite!(abs_return);
        check_finite!(dollar_volume);

        if dollar_volume <= 0.0 {
            // Zero or negative volume is not a data error — it means no
            // trading happened. Skip the observation silently.
            return Ok(());
        }

        self.count += 1;
        let ratio = abs_return / dollar_volume;
        let _ = self.ema.update(ratio);
        Ok(())
    }

    /// Current ILLIQ ratio (EMA of |return| / volume).
    #[inline]
    #[must_use]
    pub fn illiq(&self) -> f64 {
        self.ema.value().unwrap_or(0.0)
    }

    /// Number of valid observations (positive-volume periods).
    #[inline]
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Whether enough observations have been fed.
    #[inline]
    #[must_use]
    pub fn is_primed(&self) -> bool {
        self.ema.is_primed()
    }

    /// Reset all state.
    pub fn reset(&mut self) {
        self.ema.reset();
        self.count = 0;
    }
}

impl AmihudF64Builder {
    /// Direct smoothing factor. Must be in (0, 1).
    #[inline]
    #[must_use]
    pub fn alpha(mut self, alpha: f64) -> Self {
        self.alpha = Some(alpha);
        self
    }

    /// Samples for weight to decay by half.
    #[inline]
    #[must_use]
    #[cfg(any(feature = "std", feature = "libm"))]
    pub fn halflife(mut self, halflife: f64) -> Self {
        let ln2 = core::f64::consts::LN_2;
        self.alpha = Some(1.0 - crate::math::exp(-ln2 / halflife));
        self
    }

    /// Build the Amihud ILLIQ tracker.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if alpha is missing or invalid.
    pub fn build(self) -> Result<AmihudF64, crate::ConfigError> {
        let alpha = self.alpha.ok_or(crate::ConfigError::Missing("alpha"))?;

        let ema = EmaF64::builder().alpha(alpha).build()?;

        Ok(AmihudF64 { ema, count: 0 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_illiq() {
        let mut a = AmihudF64::builder().alpha(0.1).build().unwrap();
        a.update(0.01, 1_000_000.0).unwrap();
        let illiq = a.illiq();
        assert!(illiq > 0.0);
        assert!((illiq - 1e-8).abs() < 1e-10);
    }

    #[test]
    fn higher_impact_higher_illiq() {
        let mut liquid = AmihudF64::builder().alpha(0.5).build().unwrap();
        let mut illiquid = AmihudF64::builder().alpha(0.5).build().unwrap();

        for _ in 0..50 {
            liquid.update(0.001, 10_000_000.0).unwrap();
            illiquid.update(0.01, 100_000.0).unwrap();
        }

        assert!(
            illiquid.illiq() > liquid.illiq(),
            "illiquid ({}) should have higher ILLIQ than liquid ({})",
            illiquid.illiq(),
            liquid.illiq()
        );
    }

    #[test]
    fn zero_volume_skipped() {
        let mut a = AmihudF64::builder().alpha(0.1).build().unwrap();
        a.update(0.01, 0.0).unwrap();
        assert_eq!(a.count(), 0);
        assert!(!a.is_primed());
    }

    #[test]
    fn reset_clears_state() {
        let mut a = AmihudF64::builder().alpha(0.1).build().unwrap();
        a.update(0.01, 1_000_000.0).unwrap();
        a.reset();
        assert_eq!(a.count(), 0);
        assert!(!a.is_primed());
    }

    #[test]
    fn nan_rejected() {
        let mut a = AmihudF64::builder().alpha(0.1).build().unwrap();
        assert!(a.update(f64::NAN, 1.0).is_err());
        assert!(a.update(1.0, f64::NAN).is_err());
    }

    #[test]
    fn inf_rejected() {
        let mut a = AmihudF64::builder().alpha(0.1).build().unwrap();
        assert!(a.update(f64::INFINITY, 1.0).is_err());
    }
}
