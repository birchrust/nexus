use crate::buf::ReadBuf;
use super::error::ProtocolError;
use super::frame::{RawOpcode, Role};
use super::mask::apply_mask;
use super::message::{CloseCode, CloseFrame, Message};

/// Error from [`FrameReader::read`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// ReadBuf cannot accept the incoming bytes.
    BufferFull {
        /// Bytes the caller tried to write.
        needed: usize,
        /// Bytes available in spare region.
        available: usize,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferFull { needed, available } => {
                write!(f, "buffer full: need {needed} bytes, {available} available")
            }
        }
    }
}

impl std::error::Error for ReadError {}

/// WebSocket frame reader — parses wire bytes into [`Message`]s.
///
/// Handles wire frame parsing, fragment assembly, control frame
/// interleaving, masking, and UTF-8 validation. The user sees complete
/// `Message` values — never raw frames or continuations.
///
/// # Usage
///
/// ```
/// use nexus_net::ws::{FrameReader, Role, Message};
///
/// let mut reader = FrameReader::builder()
///     .role(Role::Client)
///     .buffer_capacity(65_536)
///     .build();
///
/// // Feed wire bytes
/// reader.read(&[0x81, 0x05, 0x48, 0x65, 0x6C, 0x6C, 0x6F]).unwrap();
///
/// // Parse messages
/// match reader.next().unwrap().unwrap() {
///     Message::Text(s) => assert_eq!(s, "Hello"),
///     _ => panic!("expected text"),
/// }
/// ```
pub struct FrameReader {
    buf: ReadBuf,
    msg_buf: Vec<u8>,
    ctrl_buf: Vec<u8>,  // control frame payload (max 125 bytes)
    prealloc_capacity: usize,
    compact_threshold: usize,

    state: ParseState,
    remaining_payload: usize,
    mask_key: Option<[u8; 4]>,
    mask_offset: u8,

    assembling: bool,
    assembly_opcode: Option<RawOpcode>,

    role: Role,
    max_frame_size: u64,
    max_message_size: usize,

    // Flag: msg_buf contains data from a previously returned Message.
    // Cleared at the start of next().
    needs_clear: bool,
    // Flag: last completed message used ctrl_buf (control frame during assembly).
    used_ctrl: bool,
}

#[derive(Clone, Copy, Default)]
enum ParseState {
    #[default]
    Head,
    Payload {
        opcode: RawOpcode,
        fin: bool,
    },
}

/// Builder for [`FrameReader`].
pub struct FrameReaderBuilder {
    buffer_capacity: usize,
    pre_padding: usize,
    post_padding: usize,
    prealloc_capacity: usize,
    compact_threshold: usize,
    max_frame_size: u64,
    max_message_size: usize,
    role: Role,
}

impl FrameReader {
    /// Create a builder.
    #[must_use]
    pub fn builder() -> FrameReaderBuilder {
        FrameReaderBuilder {
            buffer_capacity: 65_536,
            pre_padding: 16,
            post_padding: 4,
            prealloc_capacity: 4096,
            compact_threshold: 256 * 1024,
            max_frame_size: 16 * 1024 * 1024,
            max_message_size: 16 * 1024 * 1024,
            role: Role::Server,
        }
    }

    /// Buffer wire bytes from a source.
    ///
    /// Copies `src` into the internal ReadBuf. For zero-copy I/O, use
    /// [`spare()`](Self::spare) + [`filled()`](Self::filled) instead.
    pub fn read(&mut self, src: &[u8]) -> Result<(), ReadError> {
        let spare = self.buf.spare();
        if src.len() > spare.len() {
            return Err(ReadError::BufferFull {
                needed: src.len(),
                available: spare.len(),
            });
        }
        spare[..src.len()].copy_from_slice(src);
        self.buf.filled(src.len());
        Ok(())
    }

    /// Writable region for direct socket reads.
    #[inline]
    pub fn spare(&mut self) -> &mut [u8] {
        self.buf.spare()
    }

    /// Commit bytes written into [`spare()`](Self::spare).
    #[inline]
    pub fn filled(&mut self, n: usize) {
        self.buf.filled(n);
    }

    /// Parse the next complete message.
    ///
    /// Returns validated `Message` (UTF-8 for Text, parsed CloseFrame for
    /// Close). Never returns Continuation.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Message<'_>>, ProtocolError> {
        // Clear buffers from previous message
        if self.needs_clear {
            if self.used_ctrl {
                // Control frame used ctrl_buf — don't touch msg_buf (assembly in progress)
                self.ctrl_buf.clear();
            } else {
                // Data message used msg_buf — clear it
                self.msg_buf.clear();
                if self.msg_buf.capacity() > self.compact_threshold {
                    self.msg_buf = Vec::with_capacity(self.prealloc_capacity);
                }
            }
            self.needs_clear = false;
        }

        // Run the state machine. Returns the opcode of a completed message
        // or None if more bytes needed. msg_buf is populated by the time
        // this returns Some.
        let completed = self.pump()?;

        match completed {
            None => Ok(None),
            Some(opcode) => {
                self.needs_clear = true;
                self.make_message(opcode)
            }
        }
    }

    /// State machine: consume bytes from ReadBuf, populate msg_buf.
    /// Returns the opcode of a completed message, or None if more bytes needed.
    fn pump(&mut self) -> Result<Option<RawOpcode>, ProtocolError> {
        loop {
            let state = self.state;
            match state {
                ParseState::Payload { opcode, fin } => {
                    let available = self.buf.len();
                    if available == 0 {
                        return Ok(None);
                    }

                    let take = available.min(self.remaining_payload);
                    self.consume_partial_payload(take);

                    if self.remaining_payload == 0 {
                        self.state = ParseState::Head;
                        if let Some(completed) = self.route_opcode(opcode, fin)? {
                            return Ok(Some(completed));
                        }
                        continue;
                    }
                    return Ok(None);
                }

                ParseState::Head => {
                    let data_len = self.buf.len();
                    if data_len < 2 {
                        return Ok(None);
                    }

                    let byte1 = self.buf.data()[1];
                    let header_size = Self::header_size(byte1);
                    if data_len < header_size {
                        return Ok(None);
                    }

                    let parsed = {
                        let data = self.buf.data();
                        self.parse_header(&data[..header_size])?
                    };

                    if !parsed.opcode.is_control() {
                        let total = self.msg_buf.len() + parsed.payload_len;
                        if total > self.max_message_size {
                            return Err(ProtocolError::MessageTooLarge {
                                accumulated: total,
                                max: self.max_message_size,
                            });
                        }
                    }

                    self.buf.advance(header_size);

                    // Control frames during assembly: route to ctrl_buf
                    let use_ctrl = parsed.opcode.is_control() && self.assembling;

                    let available = self.buf.len();
                    if available >= parsed.payload_len {
                        if use_ctrl {
                            self.consume_payload_into_ctrl(parsed.mask_key, parsed.payload_len);
                        } else {
                            self.consume_payload(parsed.mask_key, parsed.payload_len);
                        }
                        if let Some(completed) = self.route_opcode(parsed.opcode, parsed.fin)? {
                            return Ok(Some(completed));
                        }
                        continue;
                    }

                    // Partial payload
                    self.remaining_payload = parsed.payload_len;
                    self.mask_key = parsed.mask_key;
                    self.mask_offset = 0;

                        if available > 0 {
                            self.consume_partial_payload(available);
                        }

                        self.state = ParseState::Payload {
                            opcode: parsed.opcode,
                            fin: parsed.fin,
                        };
                    return Ok(None);
                }
            }
        }
    }

    /// Route a completed frame. Returns the final opcode to surface as a
    /// Message, or None if the frame was consumed internally (assembly).
    fn route_opcode(
        &mut self,
        opcode: RawOpcode,
        fin: bool,
    ) -> Result<Option<RawOpcode>, ProtocolError> {
        // Control frames: always immediate
        if opcode.is_control() {
            self.used_ctrl = self.assembling; // ctrl_buf used during assembly
            return Ok(Some(opcode));
        }
        self.used_ctrl = false;

        match opcode {
            RawOpcode::Text | RawOpcode::Binary => {
                if self.assembling {
                    return Err(ProtocolError::NewMessageDuringAssembly);
                }
                if fin {
                    return Ok(Some(opcode));
                }
                self.assembling = true;
                self.assembly_opcode = Some(opcode);
                Ok(None)
            }
            RawOpcode::Continuation => {
                if !self.assembling {
                    return Err(ProtocolError::ContinuationWithoutStart);
                }
                if fin {
                    self.assembling = false;
                    let opcode = self.assembly_opcode.take().unwrap();
                    return Ok(Some(opcode));
                }
                Ok(None)
            }
            _ => unreachable!(),
        }
    }

    /// Bytes of buffer space remaining.
    #[inline]
    pub fn remaining(&self) -> usize {
        self.buf.remaining()
    }

    /// Bytes of unconsumed data in the buffer.
    #[inline]
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Reset all state.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.msg_buf.clear();
        self.ctrl_buf.clear();
        self.state = ParseState::Head;
        self.remaining_payload = 0;
        self.mask_key = None;
        self.mask_offset = 0;
        self.assembling = false;
        self.assembly_opcode = None;
        self.needs_clear = false;
        self.used_ctrl = false;
    }

    // =========================================================================
    // Internals
    // =========================================================================

    fn header_size(byte1: u8) -> usize {
        let masked = byte1 & 0x80 != 0;
        let len_code = byte1 & 0x7F;
        let base = match len_code {
            0..=125 => 2,
            126 => 4,
            _ => 10,
        };
        if masked { base + 4 } else { base }
    }

    fn parse_header(&self, header: &[u8]) -> Result<ParsedHeader, ProtocolError> {
        let byte0 = header[0];
        let byte1 = header[1];

        let fin = byte0 & 0x80 != 0;
        let rsv = (byte0 >> 4) & 0x07;
        let opcode_raw = byte0 & 0x0F;
        let masked = byte1 & 0x80 != 0;
        let len_code = byte1 & 0x7F;

        if rsv != 0 {
            return Err(ProtocolError::ReservedBitsSet { bits: rsv });
        }

        let opcode = RawOpcode::from_u8(opcode_raw)
            .ok_or(ProtocolError::InvalidOpcode(opcode_raw))?;

        match self.role {
            Role::Server if !masked => return Err(ProtocolError::UnmaskedFrameFromClient),
            Role::Client if masked => return Err(ProtocolError::MaskedFrameFromServer),
            _ => {}
        }

        let (payload_len, mask_offset) = match len_code {
            0..=125 => (u64::from(len_code), 2),
            126 => {
                let len = u16::from_be_bytes([header[2], header[3]]);
                (u64::from(len), 4)
            }
            _ => {
                let len = u64::from_be_bytes(header[2..10].try_into().unwrap());
                (len, 10)
            }
        };

        if opcode.is_control() {
            if payload_len > 125 {
                return Err(ProtocolError::ControlFrameTooLarge { size: payload_len });
            }
            if !fin {
                return Err(ProtocolError::FragmentedControlFrame);
            }
        }

        if payload_len > self.max_frame_size {
            return Err(ProtocolError::PayloadTooLarge {
                size: payload_len,
                max: self.max_frame_size,
            });
        }

        let mask_key = if masked {
            Some([
                header[mask_offset],
                header[mask_offset + 1],
                header[mask_offset + 2],
                header[mask_offset + 3],
            ])
        } else {
            None
        };

        Ok(ParsedHeader {
            fin,
            opcode,
            mask_key,
            payload_len: payload_len as usize,
        })
    }

    /// Consume a full control frame payload from ReadBuf → ctrl_buf.
    fn consume_payload_into_ctrl(&mut self, mask_key: Option<[u8; 4]>, payload_len: usize) {
        self.ctrl_buf.clear();
        if payload_len == 0 {
            return;
        }
        if let Some(mask) = mask_key {
            let data = &mut self.buf.data_mut()[..payload_len];
            apply_mask(data, mask);
        }
        let data = &self.buf.data()[..payload_len];
        self.ctrl_buf.extend_from_slice(data);
        self.buf.advance(payload_len);
    }

    /// Consume a full payload from ReadBuf → msg_buf.
    fn consume_payload(&mut self, mask_key: Option<[u8; 4]>, payload_len: usize) {
        if payload_len == 0 {
            return;
        }

        if let Some(mask) = mask_key {
            // Unmask in-place in ReadBuf then copy
            let data = &mut self.buf.data_mut()[..payload_len];
            apply_mask(data, mask);
        }

        let data = &self.buf.data()[..payload_len];
        self.msg_buf.extend_from_slice(data);
        self.buf.advance(payload_len);
    }

    /// Consume partial payload (for frames spanning reads).
    fn consume_partial_payload(&mut self, n: usize) {
        if n == 0 {
            return;
        }

        // Unmask with rotated key
        if let Some(key) = self.mask_key {
            let data = &mut self.buf.data_mut()[..n];
            let offset = self.mask_offset as usize;
            let rotated = [
                key[(offset) % 4],
                key[(offset + 1) % 4],
                key[(offset + 2) % 4],
                key[(offset + 3) % 4],
            ];
            apply_mask(data, rotated);
            self.mask_offset = ((offset + n) % 4) as u8;
        }

        let data = &self.buf.data()[..n];
        self.msg_buf.extend_from_slice(data);
        self.buf.advance(n);
        self.remaining_payload -= n;
    }

    /// Construct a Message from the completed msg_buf contents.
    fn make_message(&self, opcode: RawOpcode) -> Result<Option<Message<'_>>, ProtocolError> {
        // Control frames may use ctrl_buf if they arrived during assembly
        let payload_buf = if self.used_ctrl { &self.ctrl_buf } else { &self.msg_buf };

        match opcode {
            RawOpcode::Ping => Ok(Some(Message::Ping(payload_buf))),
            RawOpcode::Pong => Ok(Some(Message::Pong(payload_buf))),
            RawOpcode::Close => Self::parse_close_from(payload_buf),
            RawOpcode::Text => {
                let s = std::str::from_utf8(&self.msg_buf)
                    .map_err(|_| ProtocolError::InvalidUtf8)?;
                Ok(Some(Message::Text(s)))
            }
            RawOpcode::Binary => Ok(Some(Message::Binary(&self.msg_buf))),
            RawOpcode::Continuation => unreachable!("pump never returns Continuation"),
        }
    }

    fn parse_close_from(buf: &[u8]) -> Result<Option<Message<'_>>, ProtocolError> {
        if buf.is_empty() {
            return Ok(Some(Message::Close(CloseFrame {
                code: CloseCode::NoStatus,
                reason: "",
            })));
        }

        if buf.len() == 1 {
            return Err(ProtocolError::CloseFrameTooShort);
        }

        let raw_code = u16::from_be_bytes([buf[0], buf[1]]);
        let code = CloseCode::from_u16(raw_code)?;
        let reason_bytes = &buf[2..];
        let reason = std::str::from_utf8(reason_bytes)
            .map_err(|_| ProtocolError::InvalidUtf8InCloseReason)?;

        Ok(Some(Message::Close(CloseFrame { code, reason })))
    }
}

struct ParsedHeader {
    fin: bool,
    opcode: RawOpcode,
    mask_key: Option<[u8; 4]>,
    payload_len: usize,
}

impl std::fmt::Debug for FrameReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameReader")
            .field("buffered", &self.buf.len())
            .field("remaining", &self.buf.remaining())
            .field("msg_buf_len", &self.msg_buf.len())
            .field("assembling", &self.assembling)
            .field("role", &self.role)
            .finish()
    }
}

impl FrameReaderBuilder {
    /// ReadBuf capacity. Default: 64KB.
    #[must_use]
    pub fn buffer_capacity(mut self, n: usize) -> Self {
        self.buffer_capacity = n;
        self
    }

    /// ReadBuf pre-padding. Default: 16.
    #[must_use]
    pub fn pre_padding(mut self, n: usize) -> Self {
        self.pre_padding = n;
        self
    }

    /// ReadBuf post-padding. Default: 4.
    #[must_use]
    pub fn post_padding(mut self, n: usize) -> Self {
        self.post_padding = n;
        self
    }

    /// Pre-allocate message assembly buffer. Default: 4KB.
    #[must_use]
    pub fn message_capacity(mut self, n: usize) -> Self {
        self.prealloc_capacity = n;
        self
    }

    /// Shrink msg_buf when capacity exceeds this. Default: 256KB.
    #[must_use]
    pub fn compact_threshold(mut self, n: usize) -> Self {
        self.compact_threshold = n;
        self
    }

    /// Maximum single frame payload. Default: 16MB.
    #[must_use]
    pub fn max_frame_size(mut self, n: u64) -> Self {
        self.max_frame_size = n;
        self
    }

    /// Maximum assembled message size. Default: 16MB.
    #[must_use]
    pub fn max_message_size(mut self, n: usize) -> Self {
        self.max_message_size = n;
        self
    }

    /// Connection role. Default: Server.
    #[must_use]
    pub fn role(mut self, r: Role) -> Self {
        self.role = r;
        self
    }

    /// Build the reader.
    #[must_use]
    pub fn build(self) -> FrameReader {
        FrameReader {
            buf: ReadBuf::new(self.buffer_capacity, self.pre_padding, self.post_padding),
            msg_buf: Vec::with_capacity(self.prealloc_capacity),
            ctrl_buf: Vec::with_capacity(125),
            prealloc_capacity: self.prealloc_capacity,
            compact_threshold: self.compact_threshold,
            state: ParseState::Head,
            remaining_payload: 0,
            mask_key: None,
            mask_offset: 0,
            assembling: false,
            assembly_opcode: None,
            role: self.role,
            max_frame_size: self.max_frame_size,
            max_message_size: self.max_message_size,
            needs_clear: false,
            used_ctrl: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = Vec::new();
        let byte0 = if fin { 0x80 } else { 0x00 } | opcode;
        frame.push(byte0);
        if payload.len() <= 125 {
            frame.push(payload.len() as u8);
        } else if payload.len() <= 65535 {
            frame.push(126);
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else {
            frame.push(127);
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        frame.extend_from_slice(payload);
        frame
    }

    fn make_masked_frame(fin: bool, opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
        let mut frame = Vec::new();
        let byte0 = if fin { 0x80 } else { 0x00 } | opcode;
        frame.push(byte0);
        let len_byte = if payload.len() <= 125 {
            payload.len() as u8
        } else if payload.len() <= 65535 {
            126
        } else {
            127
        };
        frame.push(0x80 | len_byte);
        if payload.len() > 125 && payload.len() <= 65535 {
            frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        } else if payload.len() > 65535 {
            frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        let mut masked = payload.to_vec();
        apply_mask(&mut masked, mask);
        frame.extend_from_slice(&masked);
        frame
    }

    fn client_reader() -> FrameReader {
        FrameReader::builder().role(Role::Client).build()
    }

    fn server_reader() -> FrameReader {
        FrameReader::builder().role(Role::Server).build()
    }

    // === Single frame ===

    #[test]
    fn text_message() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x1, b"Hello")).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn binary_message() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x2, &[0xDE, 0xAD])).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Binary(b) => assert_eq!(b, &[0xDE, 0xAD]),
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    #[test]
    fn empty_text() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x1, b"")).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, ""),
            other => panic!("expected empty Text, got {other:?}"),
        }
    }

    #[test]
    fn masked_text() {
        let mut r = server_reader();
        let mask = [0x37, 0xFA, 0x21, 0x3D];
        r.read(&make_masked_frame(true, 0x1, b"Hello", mask)).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // === Fragment assembly ===

    #[test]
    fn two_fragments() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x1, b"Hel")).unwrap();
        r.read(&make_frame(true, 0x0, b"lo")).unwrap();
        // Both frames buffered — pump assembles in one next() call
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn three_binary_fragments() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x2, b"AB")).unwrap();
        r.read(&make_frame(false, 0x0, b"CD")).unwrap();
        r.read(&make_frame(true, 0x0, b"EF")).unwrap();
        // All three frames buffered — assembles in one next()
        match r.next().unwrap().unwrap() {
            Message::Binary(b) => assert_eq!(b, b"ABCDEF"),
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    // === Control frames during assembly ===

    #[test]
    fn ping_during_assembly() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x1, b"Hel")).unwrap();
        r.read(&make_frame(true, 0x9, b"ping")).unwrap();
        r.read(&make_frame(true, 0x0, b"lo")).unwrap();

        // Ping is interleaved — returned first
        match r.next().unwrap().unwrap() {
            Message::Ping(p) => assert_eq!(p, b"ping"),
            other => panic!("expected Ping, got {other:?}"),
        }
        // Then the assembled text
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // === Close frames ===

    #[test]
    fn close_with_code_and_reason() {
        let mut r = client_reader();
        let mut payload = vec![];
        payload.extend_from_slice(&1000u16.to_be_bytes());
        payload.extend_from_slice(b"goodbye");
        r.read(&make_frame(true, 0x8, &payload)).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Close(cf) => {
                assert_eq!(cf.code, CloseCode::Normal);
                assert_eq!(cf.reason, "goodbye");
            }
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn close_no_body() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x8, b"")).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Close(cf) => {
                assert_eq!(cf.code, CloseCode::NoStatus);
                assert_eq!(cf.reason, "");
            }
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn close_code_only() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x8, &1001u16.to_be_bytes())).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Close(cf) => {
                assert_eq!(cf.code, CloseCode::GoingAway);
                assert_eq!(cf.reason, "");
            }
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    fn close_invalid_code() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x8, &999u16.to_be_bytes())).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::InvalidCloseCode(999))));
    }

    #[test]
    fn close_invalid_utf8_reason() {
        let mut r = client_reader();
        let mut payload = vec![];
        payload.extend_from_slice(&1000u16.to_be_bytes());
        payload.extend_from_slice(&[0xFF, 0xFE]); // invalid UTF-8
        r.read(&make_frame(true, 0x8, &payload)).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::InvalidUtf8InCloseReason)));
    }

    #[test]
    fn close_too_short() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x8, &[0x03])).unwrap(); // 1 byte
        assert!(matches!(r.next(), Err(ProtocolError::CloseFrameTooShort)));
    }

    // === UTF-8 validation ===

    #[test]
    fn invalid_utf8_text() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x1, &[0xFF, 0xFE])).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::InvalidUtf8)));
    }

    #[test]
    fn multibyte_utf8_across_fragments() {
        let mut r = client_reader();
        // "é" is [0xC3, 0xA9] — split across two fragments
        r.read(&make_frame(false, 0x1, &[0xC3])).unwrap();
        r.read(&make_frame(true, 0x0, &[0xA9])).unwrap();
        // Both buffered — assembles in one next()
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "é"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // === Partial delivery ===

    #[test]
    fn partial_header() {
        let mut r = client_reader();
        let frame = make_frame(true, 0x1, b"Hello");
        r.read(&frame[..1]).unwrap();
        assert!(r.next().unwrap().is_none());
        r.read(&frame[1..]).unwrap();
        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("Hello")));
    }

    #[test]
    fn payload_spans_reads() {
        let mut r = client_reader();
        let frame = make_frame(true, 0x1, b"Hello, World!");
        r.read(&frame[..7]).unwrap();
        assert!(r.next().unwrap().is_none());
        r.read(&frame[7..]).unwrap();
        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("Hello, World!")));
    }

    // === Multiple messages ===

    #[test]
    fn two_messages_one_read() {
        let mut r = client_reader();
        let mut data = make_frame(true, 0x1, b"one");
        data.extend_from_slice(&make_frame(true, 0x1, b"two"));
        r.read(&data).unwrap();

        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("one")));
        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("two")));
    }

    // === Protocol errors ===

    #[test]
    fn masked_from_server() {
        let mut r = client_reader();
        r.read(&make_masked_frame(true, 0x1, b"x", [1, 2, 3, 4])).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::MaskedFrameFromServer)));
    }

    #[test]
    fn unmasked_from_client() {
        let mut r = server_reader();
        r.read(&make_frame(true, 0x1, b"x")).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::UnmaskedFrameFromClient)));
    }

    #[test]
    fn reserved_bits() {
        let mut r = client_reader();
        let mut frame = make_frame(true, 0x1, b"x");
        frame[0] |= 0x40;
        r.read(&frame).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::ReservedBitsSet { .. })));
    }

    #[test]
    fn continuation_without_start() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x0, b"orphan")).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::ContinuationWithoutStart)));
    }

    #[test]
    fn new_message_during_assembly() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x1, b"start")).unwrap();
        r.read(&make_frame(true, 0x1, b"new")).unwrap();
        // pump() encounters the error during assembly
        assert!(matches!(r.next(), Err(ProtocolError::NewMessageDuringAssembly)));
    }

    #[test]
    fn message_too_large() {
        let mut r = FrameReader::builder()
            .role(Role::Client)
            .max_message_size(10)
            .build();
        r.read(&make_frame(true, 0x1, b"way too long!!")).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::MessageTooLarge { .. })));
    }

    #[test]
    fn control_frame_too_large() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x9, &[0; 126])).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::ControlFrameTooLarge { .. })));
    }

    #[test]
    fn fragmented_control() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x9, b"ping")).unwrap();
        assert!(matches!(r.next(), Err(ProtocolError::FragmentedControlFrame)));
    }

    // === into_owned ===

    #[test]
    fn message_into_owned() {
        let mut r = client_reader();
        r.read(&make_frame(true, 0x1, b"owned")).unwrap();
        let msg = r.next().unwrap().unwrap();
        let owned = msg.into_owned();
        assert!(matches!(owned, super::super::message::OwnedMessage::Text(s) if s == "owned"));
    }

    // === Buffer full ===

    #[test]
    fn buffer_full() {
        let mut r = FrameReader::builder()
            .role(Role::Client)
            .buffer_capacity(16)
            .build();
        assert!(matches!(r.read(&[0; 32]), Err(ReadError::BufferFull { .. })));
    }

    // === Reset ===

    #[test]
    fn reset_then_new_message() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x1, b"partial")).unwrap();
        let _ = r.next();
        r.reset();
        assert_eq!(r.buffered(), 0);
        // After reset, accepts new messages cleanly
        r.read(&make_frame(true, 0x1, b"fresh")).unwrap();
        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("fresh")));
    }

    // === spare/filled direct I/O ===

    #[test]
    fn spare_filled_path() {
        let mut r = client_reader();
        let frame = make_frame(true, 0x1, b"direct");
        let spare = r.spare();
        spare[..frame.len()].copy_from_slice(&frame);
        r.filled(frame.len());
        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("direct")));
    }

    // === Masked payload spanning reads (#8) ===

    #[test]
    fn masked_payload_spans_reads() {
        let mut r = server_reader();
        let mask = [0x37, 0xFA, 0x21, 0x3D];
        let frame = make_masked_frame(true, 0x1, b"Hello, World!", mask);
        // Split mid-payload: 2 header + 4 mask + 4 payload bytes
        let split = 10;
        r.read(&frame[..split]).unwrap();
        assert!(r.next().unwrap().is_none());
        r.read(&frame[split..]).unwrap();
        assert!(matches!(r.next().unwrap().unwrap(), Message::Text("Hello, World!")));
    }

    // === Multiple control frames during assembly (#9) ===

    #[test]
    fn multiple_controls_during_assembly() {
        let mut r = client_reader();
        r.read(&make_frame(false, 0x1, b"Hel")).unwrap();
        r.read(&make_frame(true, 0x9, b"ping1")).unwrap();
        r.read(&make_frame(true, 0xA, b"pong1")).unwrap();
        r.read(&make_frame(true, 0x0, b"lo")).unwrap();

        match r.next().unwrap().unwrap() {
            Message::Ping(p) => assert_eq!(p, b"ping1"),
            other => panic!("expected Ping, got {other:?}"),
        }
        match r.next().unwrap().unwrap() {
            Message::Pong(p) => assert_eq!(p, b"pong1"),
            other => panic!("expected Pong, got {other:?}"),
        }
        match r.next().unwrap().unwrap() {
            Message::Text(s) => assert_eq!(s, "Hello"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // === Compact threshold (#10) ===

    #[test]
    fn msg_buf_compaction() {
        let mut r = FrameReader::builder()
            .role(Role::Client)
            .message_capacity(64)
            .compact_threshold(256)
            .buffer_capacity(128 * 1024)
            .max_frame_size(128 * 1024)
            .max_message_size(128 * 1024)
            .build();

        let big_payload = vec![0x42; 512];
        r.read(&make_frame(false, 0x2, &big_payload[..256])).unwrap();
        r.read(&make_frame(true, 0x0, &big_payload[256..])).unwrap();

        let msg = r.next().unwrap().unwrap();
        assert!(matches!(&msg, Message::Binary(b) if b.len() == 512));
        drop(msg);

        // Next call triggers cleanup — msg_buf should compact
        assert!(r.next().unwrap().is_none());
        assert!(r.msg_buf.capacity() <= 64);
    }

    // === 64-bit payload length (#11) ===

    #[test]
    fn extended_64bit_length() {
        let mut r = FrameReader::builder()
            .role(Role::Client)
            .buffer_capacity(128 * 1024)
            .max_frame_size(128 * 1024)
            .max_message_size(128 * 1024)
            .build();

        let payload = vec![0x42; 70_000];
        let frame = make_frame(true, 0x2, &payload);
        r.read(&frame).unwrap();
        match r.next().unwrap().unwrap() {
            Message::Binary(b) => assert_eq!(b.len(), 70_000),
            other => panic!("expected Binary, got {other:?}"),
        }
    }

    // === Buffer full with diagnostics (#5) ===

    #[test]
    fn buffer_full_diagnostics() {
        let mut r = FrameReader::builder()
            .role(Role::Client)
            .buffer_capacity(16)
            .build();
        match r.read(&[0; 32]) {
            Err(ReadError::BufferFull { needed, available }) => {
                assert_eq!(needed, 32);
                assert_eq!(available, 16);
            }
            other => panic!("expected BufferFull, got {other:?}"),
        }
    }
}
