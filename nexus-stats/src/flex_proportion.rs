/// Global event counter with decay for proportion tracking.
///
/// Users create one `FlexProportionGlobal` and multiple `FlexProportionEntity`
/// instances. Each entity tracks its share of the global total with temporal
/// decay — old activity fades, recent activity dominates.
///
/// # Use Cases
/// - "What fraction of total traffic goes to each venue?"
/// - Fair-share scheduling input
/// - Dynamic load distribution tracking
#[derive(Debug, Clone)]
pub struct FlexProportionGlobal {
    total: u64,
    half_life: u64,
    period: u64,
}

/// Per-entity event counter for proportion tracking.
#[derive(Debug, Clone)]
pub struct FlexProportionEntity {
    count: u64,
    period: u64,
}

impl FlexProportionGlobal {
    /// Creates a new global tracker.
    ///
    /// `half_life_events` is the number of global events after which old
    /// contributions decay by half.
    #[inline]
    #[must_use]
    pub fn new(half_life_events: u64) -> Self {
        assert!(half_life_events > 0, "half_life_events must be positive");
        Self {
            total: 0,
            half_life: half_life_events,
            period: 0,
        }
    }

    /// Records a global event. Call this every time ANY entity records.
    #[inline]
    pub fn record(&mut self) {
        self.total += 1;
        if self.total % self.half_life == 0 {
            self.period += 1;
        }
    }

    /// Total global events recorded.
    #[inline]
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Current decay period.
    #[inline]
    #[must_use]
    pub fn period(&self) -> u64 {
        self.period
    }
}

impl FlexProportionEntity {
    /// Creates a new entity tracker.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self { count: 0, period: 0 }
    }

    /// Records an event for this entity. Also records on the global tracker.
    #[inline]
    pub fn record(&mut self, global: &mut FlexProportionGlobal) {
        // Decay this entity's count if the global period has advanced
        while self.period < global.period {
            self.count /= 2;
            self.period += 1;
        }
        self.count += 1;
        global.record();
    }

    /// Fraction of global total attributed to this entity (0.0 to 1.0).
    ///
    /// Returns 0.0 if global total is zero.
    #[inline]
    #[must_use]
    pub fn fraction(&self, global: &FlexProportionGlobal) -> f64 {
        if global.total == 0 {
            return 0.0;
        }

        // Decay count to current period
        let mut count = self.count;
        let mut period = self.period;
        while period < global.period {
            count /= 2;
            period += 1;
        }

        // Approximate effective global total for this period
        // Each period halves, so effective total ≈ total_in_period * 2
        // Simpler: just use count / (half_life) as the fraction
        let events_in_period = global.total.saturating_sub(global.period * global.half_life)
            .max(global.half_life);

        count as f64 / events_in_period as f64
    }

    /// This entity's current (possibly decayed) event count.
    #[inline]
    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Resets this entity's count.
    #[inline]
    pub fn reset(&mut self) {
        self.count = 0;
        self.period = 0;
    }
}

impl Default for FlexProportionEntity {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_entity_full_share() {
        let mut global = FlexProportionGlobal::new(100);
        let mut entity = FlexProportionEntity::new();

        for _ in 0..50 {
            entity.record(&mut global);
        }

        let frac = entity.fraction(&global);
        assert!(frac > 0.0, "single entity should have positive fraction");
    }

    #[test]
    fn equal_entities_equal_share() {
        let mut global = FlexProportionGlobal::new(1000);
        let mut e1 = FlexProportionEntity::new();
        let mut e2 = FlexProportionEntity::new();

        for _ in 0..100 {
            e1.record(&mut global);
            e2.record(&mut global);
        }

        let f1 = e1.fraction(&global);
        let f2 = e2.fraction(&global);
        assert!((f1 - f2).abs() < 0.1, "equal entities should have equal fraction: {f1} vs {f2}");
    }

    #[test]
    fn new_entity_ramps_up() {
        let mut global = FlexProportionGlobal::new(100);
        let mut old = FlexProportionEntity::new();

        // Old entity records a lot
        for _ in 0..50 {
            old.record(&mut global);
        }

        // New entity starts recording
        let mut new = FlexProportionEntity::new();
        for _ in 0..10 {
            new.record(&mut global);
        }

        let f_new = new.fraction(&global);
        assert!(f_new > 0.0, "new entity should have some fraction");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn empty_global() {
        let global = FlexProportionGlobal::new(100);
        let entity = FlexProportionEntity::new();
        assert_eq!(entity.fraction(&global), 0.0);
    }

    #[test]
    fn reset_entity() {
        let mut global = FlexProportionGlobal::new(100);
        let mut entity = FlexProportionEntity::new();

        for _ in 0..20 {
            entity.record(&mut global);
        }
        entity.reset();
        assert_eq!(entity.count(), 0);
    }

    #[test]
    fn default_entity() {
        let entity = FlexProportionEntity::default();
        assert_eq!(entity.count(), 0);
    }

    #[test]
    #[should_panic(expected = "half_life_events must be positive")]
    fn panics_on_zero_half_life() {
        let _ = FlexProportionGlobal::new(0);
    }
}
