#![allow(clippy::suboptimal_flops, clippy::float_cmp)]

macro_rules! impl_kalman2d {
    ($name:ident, $builder:ident, $ty:ty) => {
        /// 2-state Kalman filter with configurable observation model.
        ///
        /// Tracks a 2-element state vector from noisy scalar measurements.
        /// The observation model H is provided at each update, allowing
        /// the measurement to be any linear combination of the state.
        ///
        /// The predict step adds process noise Q to the covariance.
        /// The update step applies the standard Kalman correction.
        ///
        /// # Use Cases
        /// - Position + velocity tracking from noisy position measurements
        /// - Level + trend estimation
        /// - Any 2-state linear system
        #[derive(Debug, Clone)]
        pub struct $name {
            state: [$ty; 2],
            p: [$ty; 4],
            q: [$ty; 4],
            r: $ty,
            last_innovation: Option<$ty>,
            last_innovation_var: Option<$ty>,
            count: u64,
            initial_state: [$ty; 2],
            initial_p: [$ty; 4],
        }

        /// Builder for [`
        #[doc = stringify!($name)]
        /// `].
        #[derive(Debug, Clone)]
        pub struct $builder {
            process_noise: Option<[[$ty; 2]; 2]>,
            measurement_noise: Option<$ty>,
            initial_state: [$ty; 2],
            initial_covariance: [[$ty; 2]; 2],
        }

        impl $name {
            /// Creates a builder.
            #[inline]
            #[must_use]
            pub fn builder() -> $builder {
                $builder {
                    process_noise: Option::None,
                    measurement_noise: Option::None,
                    initial_state: [0.0 as $ty; 2],
                    initial_covariance: [
                        [1.0 as $ty, 0.0 as $ty],
                        [0.0 as $ty, 1.0 as $ty],
                    ],
                }
            }

            /// Adds process noise to the covariance: P = P + Q.
            ///
            /// Call this before `update` to model the passage of time
            /// and the associated uncertainty growth.
            #[inline]
            pub fn predict(&mut self) {
                self.p[0] += self.q[0];
                self.p[1] += self.q[1];
                self.p[2] += self.q[2];
                self.p[3] += self.q[3];
            }

            /// Incorporates a scalar observation with observation model H.
            ///
            /// The observation is modeled as: z = h[0]*state[0] + h[1]*state[1] + noise.
            ///
            /// # Arguments
            /// - `observation` — the measured scalar value
            /// - `h` — the 2-element observation vector [h0, h1]
            #[inline]
            pub fn update(&mut self, observation: $ty, h: [$ty; 2]) {
                // Innovation: y = obs - H*x
                let y = observation - h[0] * self.state[0] - h[1] * self.state[1];

                // Innovation covariance: S = H*P*H' + R
                // Epsilon floor prevents NaN if P degrades numerically.
                let s = (h[0] * h[0] * self.p[0]
                    + h[0] * h[1] * self.p[1]
                    + h[1] * h[0] * self.p[2]
                    + h[1] * h[1] * self.p[3]
                    + self.r)
                    .max(<$ty>::EPSILON);

                // Kalman gain: K = P*H' / S
                let k0 = (self.p[0] * h[0] + self.p[1] * h[1]) / s;
                let k1 = (self.p[2] * h[0] + self.p[3] * h[1]) / s;

                // State update: x = x + K*y
                self.state[0] += k0 * y;
                self.state[1] += k1 * y;

                // Covariance update: P = (I - K*H) * P
                let old_p = self.p;
                self.p[0] = (1.0 as $ty - k0 * h[0]) * old_p[0] + (-k0 * h[1]) * old_p[2];
                self.p[1] = (1.0 as $ty - k0 * h[0]) * old_p[1] + (-k0 * h[1]) * old_p[3];
                self.p[2] = (-k1 * h[0]) * old_p[0] + (1.0 as $ty - k1 * h[1]) * old_p[2];
                self.p[3] = (-k1 * h[0]) * old_p[1] + (1.0 as $ty - k1 * h[1]) * old_p[3];

                self.last_innovation = Option::Some(y);
                self.last_innovation_var = Option::Some(s);
                self.count += 1;
            }

            /// Returns the current state estimate [x0, x1].
            #[inline]
            #[must_use]
            pub fn state(&self) -> [$ty; 2] {
                self.state
            }

            /// Returns the current covariance as a 2x2 matrix.
            #[inline]
            #[must_use]
            pub fn covariance(&self) -> [[$ty; 2]; 2] {
                [
                    [self.p[0], self.p[1]],
                    [self.p[2], self.p[3]],
                ]
            }

            /// Returns the last innovation (measurement residual), or `None`
            /// if no updates have been performed.
            #[inline]
            #[must_use]
            pub fn innovation(&self) -> Option<$ty> {
                self.last_innovation
            }

            /// Returns the last innovation variance (S), or `None`
            /// if no updates have been performed.
            #[inline]
            #[must_use]
            pub fn innovation_variance(&self) -> Option<$ty> {
                self.last_innovation_var
            }

            /// Number of updates performed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 {
                self.count
            }

            /// Resets to initial state and covariance.
            #[inline]
            pub fn reset(&mut self) {
                self.state = self.initial_state;
                self.p = self.initial_p;
                self.last_innovation = Option::None;
                self.last_innovation_var = Option::None;
                self.count = 0;
            }
        }

        impl $builder {
            /// Sets the 2x2 process noise matrix Q (required).
            #[inline]
            #[must_use]
            pub fn process_noise(mut self, q: [[$ty; 2]; 2]) -> Self {
                self.process_noise = Option::Some(q);
                self
            }

            /// Sets the scalar measurement noise variance R (required, must be > 0).
            #[inline]
            #[must_use]
            pub fn measurement_noise(mut self, r: $ty) -> Self {
                self.measurement_noise = Option::Some(r);
                self
            }

            /// Sets the initial state estimate. Default: [0, 0].
            #[inline]
            #[must_use]
            pub fn initial_state(mut self, state: [$ty; 2]) -> Self {
                self.initial_state = state;
                self
            }

            /// Sets the initial covariance matrix. Default: identity.
            #[inline]
            #[must_use]
            pub fn initial_covariance(mut self, p: [[$ty; 2]; 2]) -> Self {
                self.initial_covariance = p;
                self
            }

            /// Builds the filter.
            ///
            /// # Errors
            ///
            /// - `process_noise` and `measurement_noise` must be set.
            /// - `measurement_noise` must be positive.
            #[inline]
            pub fn build(self) -> Result<$name, crate::ConfigError> {
                let q_mat = self
                    .process_noise
                    .ok_or(crate::ConfigError::Missing("process_noise"))?;
                let r = self
                    .measurement_noise
                    .ok_or(crate::ConfigError::Missing("measurement_noise"))?;

                if r <= 0.0 as $ty {
                    return Err(crate::ConfigError::Invalid(
                        "measurement_noise must be positive",
                    ));
                }

                let q = [q_mat[0][0], q_mat[0][1], q_mat[1][0], q_mat[1][1]];
                let p0 = self.initial_covariance;
                let p = [p0[0][0], p0[0][1], p0[1][0], p0[1][1]];

                Ok($name {
                    state: self.initial_state,
                    p,
                    q,
                    r,
                    last_innovation: Option::None,
                    last_innovation_var: Option::None,
                    count: 0,
                    initial_state: self.initial_state,
                    initial_p: p,
                })
            }
        }
    };
}

impl_kalman2d!(Kalman2dF64, Kalman2dF64Builder, f64);
impl_kalman2d!(Kalman2dF32, Kalman2dF32Builder, f32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_signal_converges() {
        let mut kf = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(1.0)
            .build()
            .unwrap();

        for _ in 0..100 {
            kf.predict();
            kf.update(50.0, [1.0, 0.0]);
        }

        let s = kf.state();
        assert!(
            (s[0] - 50.0).abs() < 1.0,
            "state[0] = {}, expected ~50.0",
            s[0]
        );
    }

    #[test]
    fn covariance_shrinks() {
        let mut kf = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(1.0)
            .initial_covariance([[100.0, 0.0], [0.0, 100.0]])
            .build()
            .unwrap();

        kf.predict();
        kf.update(50.0, [1.0, 0.0]);
        let cov1 = kf.covariance();
        let trace1 = cov1[0][0] + cov1[1][1];

        for _ in 0..50 {
            kf.predict();
            kf.update(50.0, [1.0, 0.0]);
        }

        let cov2 = kf.covariance();
        let trace2 = cov2[0][0] + cov2[1][1];

        assert!(
            trace2 < trace1,
            "covariance trace should decrease: {trace1} -> {trace2}"
        );
    }

    #[test]
    fn innovation_stored() {
        let mut kf = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(1.0)
            .build()
            .unwrap();

        assert!(kf.innovation().is_none());
        assert!(kf.innovation_variance().is_none());

        kf.predict();
        kf.update(10.0, [1.0, 0.0]);

        assert!(kf.innovation().is_some());
        assert!(kf.innovation_variance().is_some());
        assert!(kf.innovation_variance().unwrap() > 0.0);
    }

    #[test]
    fn reset_restores_initial() {
        let mut kf = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(1.0)
            .initial_state([5.0, 3.0])
            .build()
            .unwrap();

        for _ in 0..50 {
            kf.predict();
            kf.update(100.0, [1.0, 0.0]);
        }

        kf.reset();
        assert_eq!(kf.count(), 0);
        assert_eq!(kf.state(), [5.0, 3.0]);
        assert!(kf.innovation().is_none());
    }

    #[test]
    fn f32_basic() {
        let mut kf = Kalman2dF32::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(1.0)
            .build()
            .unwrap();

        for _ in 0..100 {
            kf.predict();
            kf.update(50.0, [1.0, 0.0]);
        }

        let s = kf.state();
        assert!(
            (s[0] - 50.0).abs() < 2.0,
            "state[0] = {}, expected ~50.0",
            s[0]
        );
    }

    #[test]
    fn builder_missing_process_noise() {
        let result = Kalman2dF64::builder()
            .measurement_noise(1.0)
            .build();
        assert!(matches!(
            result,
            Err(crate::ConfigError::Missing("process_noise"))
        ));
    }

    #[test]
    fn builder_missing_measurement_noise() {
        let result = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .build();
        assert!(matches!(
            result,
            Err(crate::ConfigError::Missing("measurement_noise"))
        ));
    }

    #[test]
    fn builder_invalid_measurement_noise() {
        let result = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(0.0)
            .build();
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));

        let result = Kalman2dF64::builder()
            .process_noise([[0.01, 0.0], [0.0, 0.01]])
            .measurement_noise(-1.0)
            .build();
        assert!(matches!(result, Err(crate::ConfigError::Invalid(_))));
    }

    #[test]
    fn two_state_tracking() {
        // Track position and velocity. Observe position only.
        // True velocity = 1.0 per step.
        let mut kf = Kalman2dF64::builder()
            .process_noise([[0.1, 0.0], [0.0, 0.1]])
            .measurement_noise(1.0)
            .build()
            .unwrap();

        for i in 0..200 {
            kf.predict();
            // Simulate constant velocity: position = i
            kf.update(i as f64, [1.0, 0.0]);
        }

        let s = kf.state();
        assert!(
            (s[0] - 199.0).abs() < 5.0,
            "position = {}, expected ~199",
            s[0]
        );
    }
}
