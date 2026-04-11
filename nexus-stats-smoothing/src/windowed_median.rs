#[inline]
fn abs_f64(x: f64) -> f64 {
    x.abs()
}

#[inline]
fn abs_f32(x: f32) -> f32 {
    x.abs()
}

#[inline]
fn abs_i64(x: i64) -> i64 {
    x.abs()
}

#[inline]
fn abs_i32(x: i32) -> i32 {
    x.abs()
}

macro_rules! impl_windowed_median_float {
    ($name:ident, $ty:ty, $abs_fn:expr) => {
        /// Windowed median with runtime-configured window size (requires `alloc` feature).
        ///
        /// Maintains a ring buffer and an insertion-sorted shadow array
        /// for O(N) update with O(1) median/quartile queries.
        ///
        /// # Use Cases
        /// - Robust central tendency (median is outlier-resistant)
        /// - IQR-based anomaly detection
        /// - Modified z-score for non-normal distributions
        pub struct $name {
            ring: *mut $ty,
            sorted: *mut $ty,
            window: usize,
            head: usize,
            count: u64,
        }

        // SAFETY: both buffers are exclusively owned, T is Copy + Send
        unsafe impl Send for $name {}

        impl $name {
            #[inline]
            fn ring(&self) -> &[$ty] {
                // SAFETY: buffer allocated with capacity `window`, all elements initialized
                unsafe { core::slice::from_raw_parts(self.ring, self.window) }
            }

            #[inline]
            fn ring_mut(&mut self) -> &mut [$ty] {
                // SAFETY: buffer exclusively owned, all elements initialized
                unsafe { core::slice::from_raw_parts_mut(self.ring, self.window) }
            }

            #[inline]
            fn sorted(&self) -> &[$ty] {
                // SAFETY: buffer allocated with capacity `window`, all elements initialized
                unsafe { core::slice::from_raw_parts(self.sorted, self.window) }
            }

            #[inline]
            fn sorted_mut(&mut self) -> &mut [$ty] {
                // SAFETY: buffer exclusively owned, all elements initialized
                unsafe { core::slice::from_raw_parts_mut(self.sorted, self.window) }
            }

            /// Creates a new windowed median tracker with the given window size.
            ///
            /// # Panics
            ///
            /// Window size must be > 0.
            #[inline]
            #[must_use]
            pub fn new(window_size: usize) -> Self {
                assert!(window_size > 0, "window size must be > 0");
                let mut ring_vec = core::mem::ManuallyDrop::new(alloc::vec![0.0 as $ty; window_size]);
                let ring = ring_vec.as_mut_ptr();
                let mut sorted_vec = core::mem::ManuallyDrop::new(alloc::vec![0.0 as $ty; window_size]);
                let sorted = sorted_vec.as_mut_ptr();
                Self { ring, sorted, window: window_size, head: 0, count: 0 }
            }

            /// Feeds a sample.
            ///
            /// # Errors
            ///
            /// Returns `DataError::NotANumber` if the sample is NaN, or
            /// `DataError::Infinite` if the sample is infinite.
            #[inline]
            pub fn update(&mut self, sample: $ty) -> Result<(), nexus_stats_core::DataError> {
                check_finite!(sample);
                let len = (self.count as usize).min(self.window);
                let head = self.head;
                let window = self.window;
                // SAFETY: both buffers allocated with capacity `window`, all elements initialized,
                // exclusively owned. We need both slices simultaneously plus scalar fields.
                let ring = unsafe { core::slice::from_raw_parts_mut(self.ring, window) };
                let sorted = unsafe { core::slice::from_raw_parts_mut(self.sorted, window) };

                if self.count >= window as u64 {
                    let evicted = ring[head];
                    ring[head] = sample;

                    // Find removal position (where the evicted value is)
                    let remove_pos = {
                        let mut lo = 0;
                        let mut hi = len;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if sorted[mid] < evicted { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    };

                    // Find insertion position for new sample in the (len-1) array
                    // that would exist after removal. Adjust search range based on
                    // removal position to search the correct virtual array.
                    let insert_pos = if sample <= evicted {
                        // New value goes left of or at remove position
                        let mut lo = 0;
                        let mut hi = remove_pos;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if sorted[mid] <= sample { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    } else {
                        // New value goes right of remove position — search in
                        // the portion after remove_pos (these shift left by 1)
                        let mut lo = remove_pos;
                        let mut hi = len - 1;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            // After removal, sorted[mid] becomes sorted[mid+1]
                            if sorted[mid + 1] <= sample { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    };

                    // Single-pass shift using ptr::copy (memmove)
                    let ptr = sorted.as_mut_ptr();
                    if remove_pos < insert_pos {
                        // Shift left: elements between remove_pos and insert_pos
                        // SAFETY: remove_pos < insert_pos < len, all in bounds
                        unsafe {
                            core::ptr::copy(
                                ptr.add(remove_pos + 1),
                                ptr.add(remove_pos),
                                insert_pos - remove_pos,
                            );
                        }
                        sorted[insert_pos] = sample;
                    } else if remove_pos > insert_pos {
                        // Shift right: elements between insert_pos and remove_pos
                        // SAFETY: insert_pos < remove_pos < len, all in bounds
                        unsafe {
                            core::ptr::copy(
                                ptr.add(insert_pos),
                                ptr.add(insert_pos + 1),
                                remove_pos - insert_pos,
                            );
                        }
                        sorted[insert_pos] = sample;
                    } else {
                        // Same position — just overwrite
                        sorted[remove_pos] = sample;
                    }
                } else {
                    ring[head] = sample;

                    let insert_pos = {
                        let mut lo = 0;
                        let mut hi = len;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if sorted[mid] <= sample { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    };
                    for i in (insert_pos..len).rev() {
                        sorted[i + 1] = sorted[i];
                    }
                    sorted[insert_pos] = sample;
                }

                self.head = (head + 1) % window;
                self.count += 1;
                Ok(())
            }

            #[inline]
            fn current_len(&self) -> usize { (self.count as usize).min(self.window) }

            /// Median value, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn median(&self) -> Option<$ty> {
                let len = self.current_len();
                if len == 0 { return Option::None; }
                let sorted = self.sorted();
                if len % 2 == 1 {
                    Option::Some(sorted[len / 2])
                } else {
                    Option::Some((sorted[len / 2 - 1] + sorted[len / 2]) / (2 as $ty))
                }
            }

            /// Returns the q-th quantile of the current window.
            ///
            /// `q = 0.0` returns the minimum, `q = 1.0` returns the maximum,
            /// `q = 0.5` matches [`median()`](Self::median) for odd window sizes.
            ///
            /// Returns `None` if the window is empty.
            ///
            /// # Panics
            ///
            /// Panics if `q` is not in `[0.0, 1.0]`.
            #[inline]
            #[must_use]
            pub fn quantile(&self, q: $ty) -> Option<$ty> {
                assert!(q >= (0.0 as $ty) && q <= (1.0 as $ty), "quantile must be in [0.0, 1.0]");
                let len = self.current_len();
                if len == 0 { return Option::None; }
                let sorted = &self.sorted()[..len];
                let idx = ((len - 1) as $ty * q) as usize;
                Option::Some(sorted[idx])
            }

            /// Median Absolute Deviation (MAD), or `None` if empty.
            #[inline]
            #[must_use]
            pub fn mad(&self) -> Option<$ty> {
                let median = self.median()?;
                let len = self.current_len();
                let sorted = self.sorted();
                let mut deviations = alloc::vec![0.0 as $ty; self.window];
                for i in 0..len {
                    deviations[i] = $abs_fn(sorted[i] - median);
                }
                deviations[..len].sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                if len % 2 == 1 {
                    Option::Some(deviations[len / 2])
                } else {
                    Option::Some((deviations[len / 2 - 1] + deviations[len / 2]) / (2 as $ty))
                }
            }

            /// First quartile (Q1), or `None` if < 4 samples.
            #[inline]
            #[must_use]
            pub fn q1(&self) -> Option<$ty> {
                let len = self.current_len();
                if len < 4 { Option::None } else { Option::Some(self.sorted()[len / 4]) }
            }

            /// Third quartile (Q3), or `None` if < 4 samples.
            #[inline]
            #[must_use]
            pub fn q3(&self) -> Option<$ty> {
                let len = self.current_len();
                if len < 4 { Option::None } else { Option::Some(self.sorted()[3 * len / 4]) }
            }

            /// Interquartile range (Q3 - Q1), or `None` if < 4 samples.
            #[inline]
            #[must_use]
            pub fn iqr(&self) -> Option<$ty> {
                match (self.q1(), self.q3()) {
                    (Some(q1), Some(q3)) => Option::Some(q3 - q1),
                    _ => Option::None,
                }
            }

            /// Modified z-score: `0.6745 * (x - median) / MAD`.
            #[inline]
            #[must_use]
            #[allow(clippy::float_cmp)]
            pub fn modified_z_score(&self, sample: $ty) -> Option<$ty> {
                let median = self.median()?;
                let mad = self.mad()?;
                if mad == (0.0 as $ty) { return Option::None; }
                Option::Some(0.6745 as $ty * (sample - median) / mad)
            }

            /// Window size.
            #[inline]
            #[must_use]
            pub fn window_size(&self) -> usize { self.window }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Whether the window is full.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool { self.count >= self.window as u64 }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.ring_mut().fill(0.0 as $ty);
                self.sorted_mut().fill(0.0 as $ty);
                self.head = 0;
                self.count = 0;
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: both buffers were allocated by Vec with capacity `window`.
                // T is Copy so no element drops needed. Reclaim the allocations.
                unsafe {
                    let _ = alloc::vec::Vec::from_raw_parts(self.ring, 0, self.window);
                    let _ = alloc::vec::Vec::from_raw_parts(self.sorted, 0, self.window);
                }
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                let mut ring_vec = alloc::vec![0.0 as $ty; self.window];
                ring_vec.copy_from_slice(self.ring());
                let mut ring_md = core::mem::ManuallyDrop::new(ring_vec);
                let ring = ring_md.as_mut_ptr();

                let mut sorted_vec = alloc::vec![0.0 as $ty; self.window];
                sorted_vec.copy_from_slice(self.sorted());
                let mut sorted_md = core::mem::ManuallyDrop::new(sorted_vec);
                let sorted = sorted_md.as_mut_ptr();

                Self {
                    ring,
                    sorted,
                    window: self.window,
                    head: self.head,
                    count: self.count,
                }
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("window", &self.window)
                    .field("count", &self.count)
                    .finish()
            }
        }
    };
}

macro_rules! impl_windowed_median_int {
    ($name:ident, $ty:ty, $zero:expr, $abs_fn:expr) => {
        /// Windowed median with runtime-configured window size (requires `alloc` feature).
        ///
        /// Maintains a ring buffer and an insertion-sorted shadow array
        /// for O(N) update with O(1) median/quartile queries.
        ///
        /// # Use Cases
        /// - Robust central tendency (median is outlier-resistant)
        /// - IQR-based anomaly detection
        /// - Modified z-score for non-normal distributions
        pub struct $name {
            ring: *mut $ty,
            sorted: *mut $ty,
            window: usize,
            head: usize,
            count: u64,
        }

        // SAFETY: both buffers are exclusively owned, T is Copy + Send
        unsafe impl Send for $name {}

        impl $name {
            #[inline]
            fn ring(&self) -> &[$ty] {
                // SAFETY: buffer allocated with capacity `window`, all elements initialized
                unsafe { core::slice::from_raw_parts(self.ring, self.window) }
            }

            #[inline]
            fn ring_mut(&mut self) -> &mut [$ty] {
                // SAFETY: buffer exclusively owned, all elements initialized
                unsafe { core::slice::from_raw_parts_mut(self.ring, self.window) }
            }

            #[inline]
            fn sorted(&self) -> &[$ty] {
                // SAFETY: buffer allocated with capacity `window`, all elements initialized
                unsafe { core::slice::from_raw_parts(self.sorted, self.window) }
            }

            #[inline]
            fn sorted_mut(&mut self) -> &mut [$ty] {
                // SAFETY: buffer exclusively owned, all elements initialized
                unsafe { core::slice::from_raw_parts_mut(self.sorted, self.window) }
            }

            /// Creates a new windowed median tracker with the given window size.
            ///
            /// # Panics
            ///
            /// Window size must be > 0.
            #[inline]
            #[must_use]
            pub fn new(window_size: usize) -> Self {
                assert!(window_size > 0, "window size must be > 0");
                let mut ring_vec = core::mem::ManuallyDrop::new(alloc::vec![$zero; window_size]);
                let ring = ring_vec.as_mut_ptr();
                let mut sorted_vec = core::mem::ManuallyDrop::new(alloc::vec![$zero; window_size]);
                let sorted = sorted_vec.as_mut_ptr();
                Self { ring, sorted, window: window_size, head: 0, count: 0 }
            }

            /// Feeds a sample.
            #[inline]
            pub fn update(&mut self, sample: $ty) {
                let len = (self.count as usize).min(self.window);
                let head = self.head;
                let window = self.window;
                // SAFETY: both buffers allocated with capacity `window`, all elements initialized,
                // exclusively owned. We need both slices simultaneously plus scalar fields.
                let ring = unsafe { core::slice::from_raw_parts_mut(self.ring, window) };
                let sorted = unsafe { core::slice::from_raw_parts_mut(self.sorted, window) };

                if self.count >= window as u64 {
                    let evicted = ring[head];
                    ring[head] = sample;

                    // Find removal position (where the evicted value is)
                    let remove_pos = {
                        let mut lo = 0;
                        let mut hi = len;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if sorted[mid] < evicted { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    };

                    // Find insertion position for new sample in the (len-1) array
                    // that would exist after removal. Adjust search range based on
                    // removal position to search the correct virtual array.
                    let insert_pos = if sample <= evicted {
                        // New value goes left of or at remove position
                        let mut lo = 0;
                        let mut hi = remove_pos;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if sorted[mid] <= sample { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    } else {
                        // New value goes right of remove position — search in
                        // the portion after remove_pos (these shift left by 1)
                        let mut lo = remove_pos;
                        let mut hi = len - 1;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            // After removal, sorted[mid] becomes sorted[mid+1]
                            if sorted[mid + 1] <= sample { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    };

                    // Single-pass shift using ptr::copy (memmove)
                    let ptr = sorted.as_mut_ptr();
                    if remove_pos < insert_pos {
                        // Shift left: elements between remove_pos and insert_pos
                        // SAFETY: remove_pos < insert_pos < len, all in bounds
                        unsafe {
                            core::ptr::copy(
                                ptr.add(remove_pos + 1),
                                ptr.add(remove_pos),
                                insert_pos - remove_pos,
                            );
                        }
                        sorted[insert_pos] = sample;
                    } else if remove_pos > insert_pos {
                        // Shift right: elements between insert_pos and remove_pos
                        // SAFETY: insert_pos < remove_pos < len, all in bounds
                        unsafe {
                            core::ptr::copy(
                                ptr.add(insert_pos),
                                ptr.add(insert_pos + 1),
                                remove_pos - insert_pos,
                            );
                        }
                        sorted[insert_pos] = sample;
                    } else {
                        // Same position — just overwrite
                        sorted[remove_pos] = sample;
                    }
                } else {
                    ring[head] = sample;

                    let insert_pos = {
                        let mut lo = 0;
                        let mut hi = len;
                        while lo < hi {
                            let mid = lo + (hi - lo) / 2;
                            if sorted[mid] <= sample { lo = mid + 1; } else { hi = mid; }
                        }
                        lo
                    };
                    for i in (insert_pos..len).rev() {
                        sorted[i + 1] = sorted[i];
                    }
                    sorted[insert_pos] = sample;
                }

                self.head = (head + 1) % window;
                self.count += 1;
            }

            #[inline]
            fn current_len(&self) -> usize { (self.count as usize).min(self.window) }

            /// Median value, or `None` if empty.
            #[inline]
            #[must_use]
            pub fn median(&self) -> Option<$ty> {
                let len = self.current_len();
                if len == 0 { return Option::None; }
                let sorted = self.sorted();
                if len % 2 == 1 {
                    Option::Some(sorted[len / 2])
                } else {
                    Option::Some((sorted[len / 2 - 1] + sorted[len / 2]) / (2 as $ty))
                }
            }

            /// Median Absolute Deviation (MAD), or `None` if empty.
            #[inline]
            #[must_use]
            pub fn mad(&self) -> Option<$ty> {
                let median = self.median()?;
                let len = self.current_len();
                let sorted = self.sorted();
                let mut deviations = alloc::vec![$zero; self.window];
                for i in 0..len {
                    deviations[i] = $abs_fn(sorted[i] - median);
                }
                deviations[..len].sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                if len % 2 == 1 {
                    Option::Some(deviations[len / 2])
                } else {
                    Option::Some((deviations[len / 2 - 1] + deviations[len / 2]) / (2 as $ty))
                }
            }

            /// First quartile (Q1), or `None` if < 4 samples.
            #[inline]
            #[must_use]
            pub fn q1(&self) -> Option<$ty> {
                let len = self.current_len();
                if len < 4 { Option::None } else { Option::Some(self.sorted()[len / 4]) }
            }

            /// Third quartile (Q3), or `None` if < 4 samples.
            #[inline]
            #[must_use]
            pub fn q3(&self) -> Option<$ty> {
                let len = self.current_len();
                if len < 4 { Option::None } else { Option::Some(self.sorted()[3 * len / 4]) }
            }

            /// Interquartile range (Q3 - Q1), or `None` if < 4 samples.
            #[inline]
            #[must_use]
            pub fn iqr(&self) -> Option<$ty> {
                match (self.q1(), self.q3()) {
                    (Some(q1), Some(q3)) => Option::Some(q3 - q1),
                    _ => Option::None,
                }
            }

            /// Modified z-score: `0.6745 * (x - median) / MAD`.
            #[inline]
            #[must_use]
            #[allow(clippy::float_cmp)]
            pub fn modified_z_score(&self, sample: $ty) -> Option<$ty> {
                let median = self.median()?;
                let mad = self.mad()?;
                if mad == ($zero) { return Option::None; }
                Option::Some(0.6745 as $ty * (sample - median) / mad)
            }

            /// Window size.
            #[inline]
            #[must_use]
            pub fn window_size(&self) -> usize { self.window }

            /// Number of samples processed.
            #[inline]
            #[must_use]
            pub fn count(&self) -> u64 { self.count }

            /// Whether the window is full.
            #[inline]
            #[must_use]
            pub fn is_primed(&self) -> bool { self.count >= self.window as u64 }

            /// Resets to empty state.
            #[inline]
            pub fn reset(&mut self) {
                self.ring_mut().fill($zero);
                self.sorted_mut().fill($zero);
                self.head = 0;
                self.count = 0;
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: both buffers were allocated by Vec with capacity `window`.
                // T is Copy so no element drops needed. Reclaim the allocations.
                unsafe {
                    let _ = alloc::vec::Vec::from_raw_parts(self.ring, 0, self.window);
                    let _ = alloc::vec::Vec::from_raw_parts(self.sorted, 0, self.window);
                }
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                let mut ring_vec = alloc::vec![$zero; self.window];
                ring_vec.copy_from_slice(self.ring());
                let mut ring_md = core::mem::ManuallyDrop::new(ring_vec);
                let ring = ring_md.as_mut_ptr();

                let mut sorted_vec = alloc::vec![$zero; self.window];
                sorted_vec.copy_from_slice(self.sorted());
                let mut sorted_md = core::mem::ManuallyDrop::new(sorted_vec);
                let sorted = sorted_md.as_mut_ptr();

                Self {
                    ring,
                    sorted,
                    window: self.window,
                    head: self.head,
                    count: self.count,
                }
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("window", &self.window)
                    .field("count", &self.count)
                    .finish()
            }
        }
    };
}

impl_windowed_median_float!(WindowedMedianF64, f64, abs_f64);
impl_windowed_median_float!(WindowedMedianF32, f32, abs_f32);
impl_windowed_median_int!(WindowedMedianI64, i64, 0, abs_i64);
impl_windowed_median_int!(WindowedMedianI32, i32, 0, abs_i32);

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[3.0, 1.0, 4.0, 1.0, 5.0] {
            wm.update(v).unwrap();
        }
        assert_eq!(wm.median(), Some(3.0));
    }

    #[test]
    fn reset() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[1.0, 2.0, 3.0] {
            wm.update(v).unwrap();
        }
        wm.reset();
        assert_eq!(wm.count(), 0);
        assert!(wm.median().is_none());
    }

    #[test]
    fn empty() {
        let wm = WindowedMedianF64::new(10);
        assert!(wm.median().is_none());
        assert!(wm.mad().is_none());
    }

    #[test]
    fn single_sample() {
        let mut wm = WindowedMedianF64::new(10);
        wm.update(42.0).unwrap();
        assert_eq!(wm.median(), Some(42.0));
    }

    #[test]
    fn known_median_odd() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[3.0, 1.0, 4.0, 1.0, 5.0] {
            wm.update(v).unwrap();
        }
        // sorted: [1, 1, 3, 4, 5], median = 3
        assert_eq!(wm.median(), Some(3.0));
    }

    #[test]
    fn known_median_even() {
        let mut wm = WindowedMedianF64::new(4);
        for &v in &[1.0, 3.0, 5.0, 7.0] {
            wm.update(v).unwrap();
        }
        // sorted: [1, 3, 5, 7], median = (3+5)/2 = 4
        assert_eq!(wm.median(), Some(4.0));
    }

    #[test]
    fn mad_correctness() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[1.0, 2.0, 3.0, 4.0, 5.0] {
            wm.update(v).unwrap();
        }
        // median = 3, deviations = [2, 1, 0, 1, 2], sorted = [0, 1, 1, 2, 2]
        // MAD = 1.0
        assert_eq!(wm.mad(), Some(1.0));
    }

    #[test]
    fn iqr() {
        let mut wm = WindowedMedianF64::new(8);
        for &v in &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0] {
            wm.update(v).unwrap();
        }
        let q1 = wm.q1().unwrap();
        let q3 = wm.q3().unwrap();
        assert!(q3 > q1, "Q3 ({q3}) should be > Q1 ({q1})");
        assert!(wm.iqr().is_some());
    }

    #[test]
    fn modified_z_score() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[1.0, 2.0, 3.0, 4.0, 5.0] {
            wm.update(v).unwrap();
        }
        let z = wm.modified_z_score(10.0);
        assert!(z.is_some());
        assert!(z.unwrap() > 0.0, "outlier should have positive z-score");
    }

    #[test]
    fn window_rolls() {
        let mut wm = WindowedMedianF64::new(3);
        wm.update(10.0).unwrap();
        wm.update(20.0).unwrap();
        wm.update(30.0).unwrap();
        assert_eq!(wm.median(), Some(20.0));

        // Roll in new values
        wm.update(100.0).unwrap(); // evicts 10
        // sorted: [20, 30, 100], median = 30
        assert_eq!(wm.median(), Some(30.0));
    }

    #[test]
    fn i64_basic() {
        let mut wm = WindowedMedianI64::new(5);
        for v in [3, 1, 4, 1, 5] {
            wm.update(v);
        }
        assert_eq!(wm.median(), Some(3));
    }

    #[test]
    fn i32_basic() {
        let mut wm = WindowedMedianI32::new(3);
        wm.update(10);
        wm.update(20);
        wm.update(30);
        assert_eq!(wm.median(), Some(20));
    }

    #[test]
    fn f32_basic() {
        let mut wm = WindowedMedianF32::new(3);
        wm.update(1.0).unwrap();
        wm.update(3.0).unwrap();
        wm.update(2.0).unwrap();
        assert_eq!(wm.median(), Some(2.0));
    }

    #[test]
    fn priming() {
        let mut wm = WindowedMedianF64::new(5);
        for _ in 0..4 {
            wm.update(1.0).unwrap();
            assert!(!wm.is_primed());
        }
        wm.update(1.0).unwrap();
        assert!(wm.is_primed());
    }

    #[test]
    fn rejects_nan_and_inf() {
        let mut wm = WindowedMedianF64::new(5);
        assert!(matches!(
            wm.update(f64::NAN),
            Err(nexus_stats_core::DataError::NotANumber)
        ));
        assert!(matches!(
            wm.update(f64::INFINITY),
            Err(nexus_stats_core::DataError::Infinite)
        ));
        assert!(matches!(
            wm.update(f64::NEG_INFINITY),
            Err(nexus_stats_core::DataError::Infinite)
        ));
        assert_eq!(wm.count(), 0);
    }

    #[test]
    fn quantile_matches_median() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[3.0, 1.0, 4.0, 1.0, 5.0] {
            wm.update(v).unwrap();
        }
        // sorted: [1.0, 1.0, 3.0, 4.0, 5.0] — median = 3.0
        assert_eq!(wm.quantile(0.5).unwrap(), wm.median().unwrap());
    }

    #[test]
    fn quantile_extremes() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[10.0, 20.0, 30.0, 40.0, 50.0] {
            wm.update(v).unwrap();
        }
        // sorted: [10, 20, 30, 40, 50]
        assert_eq!(wm.quantile(0.0).unwrap(), 10.0);
        assert_eq!(wm.quantile(1.0).unwrap(), 50.0);
    }

    #[test]
    fn quantile_75th() {
        let mut wm = WindowedMedianF64::new(5);
        for &v in &[10.0, 20.0, 30.0, 40.0, 50.0] {
            wm.update(v).unwrap();
        }
        // sorted: [10, 20, 30, 40, 50]
        // idx = (4) * 0.75 = 3.0 → sorted[3] = 40.0
        assert_eq!(wm.quantile(0.75).unwrap(), 40.0);
    }

    #[test]
    fn quantile_empty_returns_none() {
        let wm = WindowedMedianF64::new(5);
        assert!(wm.quantile(0.5).is_none());
    }

    #[test]
    #[should_panic(expected = "quantile must be in [0.0, 1.0]")]
    fn quantile_out_of_range() {
        let mut wm = WindowedMedianF64::new(5);
        wm.update(1.0).unwrap();
        let _ = wm.quantile(1.5);
    }
}
