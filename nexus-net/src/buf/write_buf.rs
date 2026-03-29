/// Flat byte slab for outbound protocol frames.
///
/// sk_buff headroom model: payload is appended at the tail, protocol
/// headers are prepended into reserved headroom. The result is one
/// contiguous [`data()`](WriteBuf::data) slice for the write syscall.
///
/// Fixed capacity. No growth.
///
/// # Layout
///
/// ```text
/// [ headroom          | payload data         | tailroom    ]
/// ^                   ^                      ^             ^
/// 0                   head                   tail          buf.len()
/// ```
///
/// After [`clear()`](WriteBuf::clear): `head = headroom`, `tail = headroom`.
///
/// # Examples
///
/// ```
/// use nexus_net::buf::WriteBuf;
///
/// let mut wbuf = WriteBuf::new(128, 14);
///
/// // Build message: payload first, then header
/// wbuf.append(b"Hello, world!");
/// wbuf.prepend(&[0x81, 0x0D]); // WS text frame header
///
/// // data() = contiguous [header | payload]
/// assert_eq!(&wbuf.data()[..2], &[0x81, 0x0D]);
/// assert_eq!(&wbuf.data()[2..], b"Hello, world!");
///
/// // For partial writes:
/// // let n = socket.write(wbuf.data())?;
/// // wbuf.advance(n);
/// ```
pub struct WriteBuf {
    buf: Box<[u8]>,
    head: usize,
    tail: usize,
    reset_offset: usize,
}

impl WriteBuf {
    /// Create with total capacity and reserved headroom.
    ///
    /// Usable tailroom = capacity - headroom.
    ///
    /// # Panics
    /// Panics if `headroom >= capacity`.
    #[must_use]
    pub fn new(capacity: usize, headroom: usize) -> Self {
        assert!(
            headroom < capacity,
            "headroom ({headroom}) must be less than capacity ({capacity})"
        );
        Self {
            buf: vec![0u8; capacity].into_boxed_slice(),
            head: headroom,
            tail: headroom,
            reset_offset: headroom,
        }
    }

    // =========================================================================
    // Build outbound data
    // =========================================================================

    /// Prepend bytes into headroom (protocol headers).
    /// Moves head backward.
    ///
    /// # Panics
    /// Panics if `src.len() > self.headroom()`.
    #[inline]
    pub fn prepend(&mut self, src: &[u8]) {
        if src.len() > self.headroom() {
            Self::panic_headroom(src.len(), self.headroom());
        }
        let new_head = self.head - src.len();
        self.buf[new_head..self.head].copy_from_slice(src);
        self.head = new_head;
    }

    /// Append bytes at tail (payload data).
    ///
    /// # Panics
    /// Panics if `src.len() > self.tailroom()`.
    #[inline]
    pub fn append(&mut self, src: &[u8]) {
        if src.len() > self.tailroom() {
            Self::panic_tailroom(src.len(), self.tailroom());
        }
        self.buf[self.tail..self.tail + src.len()].copy_from_slice(src);
        self.tail += src.len();
    }

    // =========================================================================
    // Send side
    // =========================================================================

    /// Complete outbound data (contiguous: headers + payload).
    #[inline]
    pub fn data(&self) -> &[u8] {
        &self.buf[self.head..self.tail]
    }

    /// Mutable access to outbound data.
    /// For in-place operations like XOR masking.
    #[inline]
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.buf[self.head..self.tail]
    }

    /// Consume `n` bytes from front after a partial write.
    ///
    /// # Panics
    /// Panics if `n > self.len()`.
    pub fn advance(&mut self, n: usize) {
        assert!(
            n <= self.len(),
            "advance({n}) exceeds data length ({})",
            self.len()
        );
        self.head += n;
    }

    // =========================================================================
    // Capacity queries
    // =========================================================================

    /// Bytes available for prepend.
    #[inline]
    pub fn headroom(&self) -> usize {
        self.head
    }

    /// Bytes available for append.
    #[inline]
    pub fn tailroom(&self) -> usize {
        self.buf.len() - self.tail
    }

    /// Bytes of outbound data.
    #[inline]
    pub fn len(&self) -> usize {
        self.tail - self.head
    }

    /// Whether the buffer has no outbound data.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    /// Reset for next message. Cursors return to headroom offset.
    pub fn clear(&mut self) {
        self.head = self.reset_offset;
        self.tail = self.reset_offset;
    }

    #[cold]
    #[inline(never)]
    fn panic_headroom(needed: usize, available: usize) -> ! {
        panic!("prepend: {needed} bytes exceeds headroom ({available})")
    }

    #[cold]
    #[inline(never)]
    fn panic_tailroom(needed: usize, available: usize) -> ! {
        panic!("append: {needed} bytes exceeds tailroom ({available})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_layout() {
        let buf = WriteBuf::new(128, 14);
        assert_eq!(buf.buf.len(), 128);
        assert_eq!(buf.headroom(), 14);
        assert_eq!(buf.tailroom(), 114);
        assert!(buf.is_empty());
    }

    #[test]
    fn append_data() {
        let mut buf = WriteBuf::new(128, 14);
        buf.append(b"Hello");
        assert_eq!(buf.data(), b"Hello");
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn prepend_data() {
        let mut buf = WriteBuf::new(128, 14);
        buf.append(b"World");
        buf.prepend(b"Hello");
        assert_eq!(buf.data(), b"HelloWorld");
        assert_eq!(buf.len(), 10);
    }

    #[test]
    fn prepend_then_append() {
        let mut buf = WriteBuf::new(128, 14);
        buf.append(b"payload");
        buf.prepend(&[0x81, 0x07]); // WS-like header
        let d = buf.data();
        assert_eq!(&d[..2], &[0x81, 0x07]);
        assert_eq!(&d[2..], b"payload");
    }

    #[test]
    fn advance_partial_write() {
        let mut buf = WriteBuf::new(128, 14);
        buf.append(b"Hello, world!");
        buf.advance(7);
        assert_eq!(buf.data(), b"world!");
        assert_eq!(buf.len(), 6);
    }

    #[test]
    fn headroom_tailroom_tracking() {
        let mut buf = WriteBuf::new(128, 14);
        assert_eq!(buf.headroom(), 14);
        assert_eq!(buf.tailroom(), 114);

        buf.append(b"12345");
        assert_eq!(buf.headroom(), 14);
        assert_eq!(buf.tailroom(), 109);

        buf.prepend(b"AB");
        assert_eq!(buf.headroom(), 12);
        assert_eq!(buf.tailroom(), 109);
    }

    #[test]
    fn clear_resets() {
        let mut buf = WriteBuf::new(128, 14);
        buf.append(b"data");
        buf.prepend(b"hdr");
        buf.clear();
        assert!(buf.is_empty());
        assert_eq!(buf.headroom(), 14);
        assert_eq!(buf.tailroom(), 114);
    }

    #[test]
    fn multiple_cycles() {
        let mut buf = WriteBuf::new(64, 10);
        for i in 0u8..5 {
            buf.clear();
            buf.append(&[i; 4]);
            buf.prepend(&[0xFF, i]);
            assert_eq!(buf.len(), 6);
            assert_eq!(buf.data()[0], 0xFF);
            assert_eq!(buf.data()[1], i);
            assert_eq!(&buf.data()[2..], &[i; 4]);
        }
    }

    #[test]
    #[should_panic(expected = "headroom")]
    fn prepend_exceeds_headroom() {
        let mut buf = WriteBuf::new(64, 4);
        buf.prepend(&[0; 8]); // 8 > 4 headroom
    }

    #[test]
    #[should_panic(expected = "tailroom")]
    fn append_exceeds_tailroom() {
        let mut buf = WriteBuf::new(16, 4);
        buf.append(&[0; 16]); // only 12 tailroom
    }

    #[test]
    #[should_panic(expected = "headroom")]
    fn headroom_ge_capacity_panics() {
        WriteBuf::new(10, 10);
    }

    #[test]
    #[should_panic(expected = "advance")]
    fn advance_exceeds_data() {
        let mut buf = WriteBuf::new(64, 10);
        buf.append(b"Hi");
        buf.advance(5);
    }

    #[test]
    fn zero_length_operations() {
        let mut buf = WriteBuf::new(32, 8);
        buf.append(b"");
        buf.prepend(b"");
        assert!(buf.is_empty());
        buf.advance(0);
        assert!(buf.is_empty());
    }

    #[test]
    fn advance_full_then_reuse() {
        let mut buf = WriteBuf::new(64, 10);
        buf.append(b"Hello");
        buf.advance(5);
        assert!(buf.is_empty());
        // After full advance, head moved but headroom is consumed
        assert_eq!(buf.headroom(), 15); // original 10 + consumed 5
        // clear() restores headroom
        buf.clear();
        assert_eq!(buf.headroom(), 10);
    }
}
