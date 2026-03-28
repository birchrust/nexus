/// Flat byte slab for inbound protocol parsing.
///
/// The I/O layer reads bytes into the slab via [`spare()`](ReadBuf::spare) +
/// [`filled()`](ReadBuf::filled). The protocol parser walks it via
/// [`data()`](ReadBuf::data) + [`advance()`](ReadBuf::advance). When all
/// data is consumed, cursors reset to the pre-padding offset — no memmove.
///
/// Fixed capacity. No growth.
///
/// # Layout
///
/// ```text
/// [ pre_padding | usable capacity                        | post_padding ]
///               ^                                        ^
///               head/tail start here                     end of usable
/// ```
///
/// Pre-padding: reserved bytes before the data region. Protocol layers
/// can use this for header reassembly (e.g., uWS writes spilled header
/// bytes backward into pre-padding). Accessible via [`pre_padding_mut()`](ReadBuf::pre_padding_mut).
///
/// Post-padding: reserved bytes after the data region. SIMD operations
/// may overrun by up to alignment width. ReadBuf guarantees this space
/// exists but doesn't manage it.
///
/// # Examples
///
/// ```
/// use nexus_net::buf::ReadBuf;
///
/// let mut buf = ReadBuf::with_capacity(4096);
///
/// // I/O: read bytes into spare region
/// let spare = buf.spare();
/// spare[..5].copy_from_slice(b"Hello");
/// buf.filled(5);
///
/// // Parse: consume from data region
/// assert_eq!(buf.data(), b"Hello");
/// buf.advance(5);
/// assert!(buf.is_empty()); // cursors auto-reset
/// ```
pub struct ReadBuf {
    buf: Vec<u8>,
    head: usize,
    tail: usize,
    capacity: usize,
    pre_padding: usize,
}

impl ReadBuf {
    /// Create a ReadBuf with explicit padding.
    ///
    /// Total allocation: `pre_padding + capacity + post_padding`.
    #[must_use]
    pub fn new(capacity: usize, pre_padding: usize, post_padding: usize) -> Self {
        let total = pre_padding + capacity + post_padding;
        Self {
            buf: vec![0u8; total],
            head: pre_padding,
            tail: pre_padding,
            capacity,
            pre_padding,
        }
    }

    /// Convenience: capacity only, zero padding.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(capacity, 0, 0)
    }

    // =========================================================================
    // Read side (parser)
    // =========================================================================

    /// Unconsumed bytes. Always a single contiguous slice.
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.buf[self.head..self.tail]
    }

    /// Mutable access to unconsumed bytes.
    /// For in-place operations like XOR unmasking.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.head..self.tail]
    }

    /// Consume `n` bytes from the front.
    ///
    /// If the buffer becomes empty after advance, resets head and tail
    /// to `pre_padding` offset (free — no memmove, just cursor reset).
    ///
    /// # Panics
    /// Panics if `n > self.len()`.
    #[inline]
    pub fn advance(&mut self, n: usize) {
        assert!(
            n <= self.len(),
            "advance({n}) exceeds buffered data ({})",
            self.len()
        );
        self.head += n;
        if self.head == self.tail {
            self.head = self.pre_padding;
            self.tail = self.pre_padding;
        }
    }

    // =========================================================================
    // Write side (I/O layer)
    // =========================================================================

    /// Writable tail region for direct socket reads.
    ///
    /// ```ignore
    /// let n = socket.read(buf.spare())?;
    /// buf.filled(n);
    /// ```
    ///
    /// Returns `buf[tail .. pre_padding + capacity]`.
    /// May be empty if tail has reached the capacity boundary.
    #[inline]
    pub fn spare(&mut self) -> &mut [u8] {
        let end = self.pre_padding + self.capacity;
        &mut self.buf[self.tail..end]
    }

    /// Commit `n` bytes written into [`spare()`](Self::spare).
    ///
    /// # Panics
    /// Panics if `n` would push tail past capacity boundary.
    #[inline]
    pub fn filled(&mut self, n: usize) {
        let new_tail = self.tail + n;
        let end = self.pre_padding + self.capacity;
        assert!(
            new_tail <= end,
            "filled({n}) would exceed capacity (tail={}, end={end})",
            self.tail
        );
        self.tail = new_tail;
    }

    // =========================================================================
    // Padding access
    // =========================================================================

    /// Mutable access to the pre-padding region (bytes before head).
    ///
    /// Returns `buf[0..head]`. Protocol layers can use this for header
    /// reassembly — e.g., writing spilled header bytes backward so the
    /// parser sees a contiguous header without memmove.
    ///
    /// The returned slice includes both the original pre-padding AND any
    /// consumed-but-not-reset space before head.
    #[inline]
    pub fn pre_padding_mut(&mut self) -> &mut [u8] {
        &mut self.buf[..self.head]
    }

    // =========================================================================
    // Capacity queries
    // =========================================================================

    /// Bytes of unconsumed data.
    #[inline]
    pub fn len(&self) -> usize {
        self.tail - self.head
    }

    /// Whether the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Usable capacity (excluding padding).
    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Writable space at the tail (same as `spare().len()`).
    #[inline]
    pub fn remaining(&self) -> usize {
        self.pre_padding + self.capacity - self.tail
    }

    /// Discard all data. Cursors reset to pre_padding offset.
    pub fn clear(&mut self) {
        self.head = self.pre_padding;
        self.tail = self.pre_padding;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_allocation_size() {
        let buf = ReadBuf::new(100, 16, 4);
        assert_eq!(buf.buf.len(), 120);
        assert_eq!(buf.capacity(), 100);
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.remaining(), 100);
    }

    #[test]
    fn with_capacity_zero_padding() {
        let buf = ReadBuf::with_capacity(4096);
        assert_eq!(buf.capacity(), 4096);
        assert_eq!(buf.buf.len(), 4096); // no padding
        assert_eq!(buf.remaining(), 4096);
        assert_eq!(buf.pre_padding, 0);
    }

    #[test]
    fn spare_filled_data() {
        let mut buf = ReadBuf::with_capacity(64);
        buf.spare()[..5].copy_from_slice(b"Hello");
        buf.filled(5);
        assert_eq!(buf.data(), b"Hello");
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.remaining(), 59);
    }

    #[test]
    fn advance_consumes() {
        let mut buf = ReadBuf::with_capacity(64);
        buf.spare()[..10].copy_from_slice(b"HelloWorld");
        buf.filled(10);
        buf.advance(5);
        assert_eq!(buf.data(), b"World");
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn advance_auto_reset() {
        let mut buf = ReadBuf::with_capacity(64);
        buf.spare()[..5].copy_from_slice(b"Hello");
        buf.filled(5);
        buf.advance(5);
        assert!(buf.is_empty());
        assert_eq!(buf.remaining(), 64);
    }

    #[test]
    fn data_mut_in_place() {
        let mut buf = ReadBuf::with_capacity(64);
        buf.spare()[..4].copy_from_slice(&[0x00, 0x01, 0x02, 0x03]);
        buf.filled(4);
        for b in buf.data_mut().iter_mut() {
            *b ^= 0xFF;
        }
        assert_eq!(buf.data(), &[0xFF, 0xFE, 0xFD, 0xFC]);
    }

    #[test]
    fn pre_padding_mut_accessible() {
        let mut buf = ReadBuf::new(64, 16, 4);
        buf.spare()[..5].copy_from_slice(b"World");
        buf.filled(5);

        // Pre-padding is 16 bytes before head
        let padding = buf.pre_padding_mut();
        assert_eq!(padding.len(), 16);

        // Write header bytes backward
        padding[12..16].copy_from_slice(b"Hdr:");

        // After consuming data, pre-padding grows
        buf.advance(3);
        assert_eq!(buf.pre_padding_mut().len(), 19); // 16 + 3 consumed
    }

    #[test]
    fn pre_padding_mut_after_advance() {
        let mut buf = ReadBuf::new(64, 8, 0);
        buf.spare()[..10].copy_from_slice(b"0123456789");
        buf.filled(10);
        buf.advance(5);

        // Pre-padding should be 8 + 5 consumed = 13
        let padding = buf.pre_padding_mut();
        assert_eq!(padding.len(), 13);
    }

    #[test]
    fn remaining_tracks() {
        let mut buf = ReadBuf::with_capacity(32);
        assert_eq!(buf.remaining(), 32);
        buf.spare()[..10].copy_from_slice(&[0; 10]);
        buf.filled(10);
        assert_eq!(buf.remaining(), 22);
        buf.advance(10);
        assert_eq!(buf.remaining(), 32); // auto-reset
    }

    #[test]
    fn clear_resets() {
        let mut buf = ReadBuf::with_capacity(64);
        buf.spare()[..10].copy_from_slice(&[0; 10]);
        buf.filled(10);
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.remaining(), 64);
    }

    #[test]
    fn spare_when_full() {
        let mut buf = ReadBuf::with_capacity(8);
        buf.spare()[..8].copy_from_slice(&[0; 8]);
        buf.filled(8);
        assert!(buf.spare().is_empty());
        assert_eq!(buf.remaining(), 0);
    }

    #[test]
    fn zero_length_operations() {
        let mut buf = ReadBuf::with_capacity(32);
        buf.filled(0);
        assert!(buf.is_empty());
        buf.advance(0);
        assert!(buf.is_empty());
    }

    #[test]
    fn large_capacity_smoke() {
        let mut buf = ReadBuf::with_capacity(256 * 1024);
        let data: Vec<u8> = (0..1024).map(|i| (i & 0xFF) as u8).collect();
        buf.spare()[..1024].copy_from_slice(&data);
        buf.filled(1024);
        assert_eq!(buf.data(), data.as_slice());
        buf.advance(512);
        assert_eq!(buf.data(), &data[512..]);
        buf.advance(512);
        assert!(buf.is_empty());
        assert_eq!(buf.remaining(), 256 * 1024);
    }

    #[test]
    fn multiple_write_advance_cycles() {
        let mut buf = ReadBuf::with_capacity(32);
        for i in 0u8..10 {
            buf.spare()[..4].copy_from_slice(&[i; 4]);
            buf.filled(4);
            assert_eq!(buf.data(), &[i; 4]);
            buf.advance(4);
            assert!(buf.is_empty());
        }
    }

    #[test]
    #[should_panic(expected = "advance")]
    fn advance_exceeds_data() {
        let mut buf = ReadBuf::with_capacity(32);
        buf.spare()[..5].copy_from_slice(b"Hello");
        buf.filled(5);
        buf.advance(10);
    }

    #[test]
    #[should_panic(expected = "filled")]
    fn filled_exceeds_capacity() {
        let mut buf = ReadBuf::with_capacity(8);
        buf.filled(16);
    }

    #[test]
    fn partial_advance_then_fill() {
        let mut buf = ReadBuf::with_capacity(32);
        buf.spare()[..10].copy_from_slice(b"0123456789");
        buf.filled(10);
        buf.advance(5);
        assert_eq!(buf.data(), b"56789");

        buf.spare()[..3].copy_from_slice(b"ABC");
        buf.filled(3);
        assert_eq!(buf.data(), b"56789ABC");
    }

    #[test]
    fn with_padding_smoke() {
        let mut buf = ReadBuf::new(64, 16, 32);
        assert_eq!(buf.buf.len(), 112); // 16 + 64 + 32
        assert_eq!(buf.capacity(), 64);
        assert_eq!(buf.remaining(), 64);

        buf.spare()[..10].copy_from_slice(b"0123456789");
        buf.filled(10);
        assert_eq!(buf.data(), b"0123456789");
    }
}
