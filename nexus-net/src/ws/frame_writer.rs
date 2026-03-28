use super::frame::Role;

/// WebSocket frame encoder.
///
/// Encodes messages into RFC 6455 wire format. If the role is Client,
/// frames are masked with a random 4-byte key. If Server, no masking.
///
/// # Usage
///
/// ```
/// use nexus_net::ws::{FrameWriter, Role};
///
/// let writer = FrameWriter::new(Role::Server);
/// let mut dst = vec![0u8; writer.max_encoded_len(5)];
/// let n = writer.encode_text(b"Hello", &mut dst);
/// assert_eq!(&dst[..n], &[0x81, 0x05, 0x48, 0x65, 0x6C, 0x6C, 0x6F]);
/// ```
pub struct FrameWriter {
    role: Role,
}

impl FrameWriter {
    /// Create a writer for the given role.
    #[must_use]
    pub fn new(role: Role) -> Self {
        Self { role }
    }

    /// Encode a text message frame. Returns bytes written.
    ///
    /// # Panics
    /// Panics if `dst` is too small. Use [`max_encoded_len`](Self::max_encoded_len).
    pub fn encode_text(&self, payload: &[u8], dst: &mut [u8]) -> usize {
        self.encode(0x81, payload, dst) // FIN + Text
    }

    /// Encode a binary message frame. Returns bytes written.
    pub fn encode_binary(&self, payload: &[u8], dst: &mut [u8]) -> usize {
        self.encode(0x82, payload, dst) // FIN + Binary
    }

    /// Encode a ping control frame. Returns bytes written.
    ///
    /// # Panics
    /// Panics if payload exceeds 125 bytes (RFC 6455 §5.5).
    pub fn encode_ping(&self, payload: &[u8], dst: &mut [u8]) -> usize {
        assert!(payload.len() <= 125, "ping payload must be <= 125 bytes");
        self.encode(0x89, payload, dst) // FIN + Ping
    }

    /// Encode a pong control frame. Returns bytes written.
    ///
    /// # Panics
    /// Panics if payload exceeds 125 bytes.
    pub fn encode_pong(&self, payload: &[u8], dst: &mut [u8]) -> usize {
        assert!(payload.len() <= 125, "pong payload must be <= 125 bytes");
        self.encode(0x8A, payload, dst) // FIN + Pong
    }

    /// Encode a close frame. Returns bytes written.
    ///
    /// # Panics
    /// Panics if code + reason exceeds 125 bytes.
    pub fn encode_close(&self, code: u16, reason: &[u8], dst: &mut [u8]) -> usize {
        let payload_len = 2 + reason.len();
        assert!(payload_len <= 125, "close payload must be <= 125 bytes");

        let mut close_payload = [0u8; 125];
        close_payload[..2].copy_from_slice(&code.to_be_bytes());
        close_payload[2..payload_len].copy_from_slice(reason);

        self.encode(0x88, &close_payload[..payload_len], dst)
    }

    /// Maximum encoded size for a given payload length.
    /// Accounts for header (2-10 bytes) + optional mask (4 bytes).
    #[must_use]
    pub fn max_encoded_len(&self, payload_len: usize) -> usize {
        let header = if payload_len <= 125 {
            2
        } else if payload_len <= 65535 {
            4
        } else {
            10
        };
        let mask = if self.role == Role::Client { 4 } else { 0 };
        header + mask + payload_len
    }

    /// Encode a close frame with structured [`CloseCode`](super::CloseCode) and UTF-8 reason.
    ///
    /// # Panics
    /// Panics if 2 + reason.len() exceeds 125 bytes.
    pub fn encode_close_code(
        &self,
        code: super::message::CloseCode,
        reason: &str,
        dst: &mut [u8],
    ) -> usize {
        self.encode_close(code.as_u16(), reason.as_bytes(), dst)
    }

    // =========================================================================
    // Internal
    // =========================================================================

    fn encode(&self, byte0: u8, payload: &[u8], dst: &mut [u8]) -> usize {
        let mask_bit: u8 = if self.role == Role::Client { 0x80 } else { 0 };
        let payload_len = payload.len();

        let mut offset = 0;

        // Byte 0: FIN + opcode
        dst[offset] = byte0;
        offset += 1;

        // Byte 1: MASK bit + payload length
        if payload_len <= 125 {
            dst[offset] = mask_bit | (payload_len as u8);
            offset += 1;
        } else if payload_len <= 65535 {
            dst[offset] = mask_bit | 0x7E;
            offset += 1;
            dst[offset..offset + 2].copy_from_slice(&(payload_len as u16).to_be_bytes());
            offset += 2;
        } else {
            dst[offset] = mask_bit | 0x7F;
            offset += 1;
            dst[offset..offset + 8].copy_from_slice(&(payload_len as u64).to_be_bytes());
            offset += 8;
        }

        // Mask key (client only)
        if self.role == Role::Client {
            let mask = generate_mask();
            dst[offset..offset + 4].copy_from_slice(&mask);
            offset += 4;

            // Copy and mask payload
            dst[offset..offset + payload_len].copy_from_slice(payload);
            super::mask::apply_mask(&mut dst[offset..offset + payload_len], mask);
        } else {
            dst[offset..offset + payload_len].copy_from_slice(payload);
        }

        offset + payload_len
    }

}

/// Generate a random 4-byte mask key.
/// Uses a simple LCG for speed — mask quality doesn't affect security
/// (it's only to prevent proxy cache poisoning per RFC 6455 §10.3).
fn generate_mask() -> [u8; 4] {
    thread_local! {
        static STATE: std::cell::Cell<u64> = {
            // Non-deterministic seed: mix thread ID + timestamp
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            let tid = {
                let mut h = 0u64;
                for b in format!("{:?}", std::thread::current().id()).bytes() {
                    h = h.wrapping_mul(31).wrapping_add(u64::from(b));
                }
                h
            };
            std::cell::Cell::new(time ^ tid)
        };
    }

    STATE.with(|s| {
        let mut state = s.get();
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        s.set(state);
        let bytes = state.to_ne_bytes();
        [bytes[0], bytes[1], bytes[2], bytes[3]]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_text_server() {
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; writer.max_encoded_len(5)];
        let n = writer.encode_text(b"Hello", &mut dst);
        assert_eq!(n, 7);
        assert_eq!(dst[0], 0x81); // FIN + Text
        assert_eq!(dst[1], 0x05); // no mask, len=5
        assert_eq!(&dst[2..7], b"Hello");
    }

    #[test]
    fn encode_binary_server() {
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; writer.max_encoded_len(4)];
        let n = writer.encode_binary(&[0xDE, 0xAD, 0xBE, 0xEF], &mut dst);
        assert_eq!(n, 6);
        assert_eq!(dst[0], 0x82); // FIN + Binary
        assert_eq!(&dst[2..6], &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn encode_close_server() {
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; writer.max_encoded_len(9)];
        let n = writer.encode_close(1000, b"goodbye", &mut dst);
        assert_eq!(dst[0], 0x88); // FIN + Close
        assert_eq!(&dst[2..4], &1000u16.to_be_bytes());
        assert_eq!(&dst[4..n], b"goodbye");
    }

    #[test]
    fn encode_ping_server() {
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; writer.max_encoded_len(4)];
        let n = writer.encode_ping(b"ping", &mut dst);
        assert_eq!(dst[0], 0x89); // FIN + Ping
        assert_eq!(&dst[2..n], b"ping");
    }

    #[test]
    fn encode_pong_server() {
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; writer.max_encoded_len(4)];
        let n = writer.encode_pong(b"pong", &mut dst);
        assert_eq!(dst[0], 0x8A); // FIN + Pong
        assert_eq!(&dst[2..n], b"pong");
    }

    #[test]
    fn encode_client_is_masked() {
        let writer = FrameWriter::new(Role::Client);
        let mut dst = vec![0u8; writer.max_encoded_len(5)];
        let n = writer.encode_text(b"Hello", &mut dst);
        assert_eq!(n, 11); // 2 header + 4 mask + 5 payload
        assert_eq!(dst[0], 0x81); // FIN + Text
        assert_eq!(dst[1] & 0x80, 0x80); // mask bit set
        assert_eq!(dst[1] & 0x7F, 5); // len=5
        // Payload is masked — shouldn't equal plaintext
        assert_ne!(&dst[6..11], b"Hello");
    }

    #[test]
    fn encode_16bit_length() {
        let writer = FrameWriter::new(Role::Server);
        let payload = vec![0x42; 256];
        let mut dst = vec![0u8; writer.max_encoded_len(256)];
        let n = writer.encode_binary(&payload, &mut dst);
        assert_eq!(n, 4 + 256); // 2 + 2 (16-bit len) + 256
        assert_eq!(dst[1] & 0x7F, 126); // extended 16-bit
        let len = u16::from_be_bytes([dst[2], dst[3]]);
        assert_eq!(len, 256);
    }

    #[test]
    fn max_encoded_len_small() {
        let server = FrameWriter::new(Role::Server);
        assert_eq!(server.max_encoded_len(0), 2);
        assert_eq!(server.max_encoded_len(125), 2 + 125);
        assert_eq!(server.max_encoded_len(126), 4 + 126);

        let client = FrameWriter::new(Role::Client);
        assert_eq!(client.max_encoded_len(0), 2 + 4);
        assert_eq!(client.max_encoded_len(125), 2 + 4 + 125);
    }

    #[test]
    fn round_trip_server() {
        use crate::ws::{FrameReader, Message};
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; writer.max_encoded_len(5)];
        let n = writer.encode_text(b"Hello", &mut dst);

        let mut reader = FrameReader::builder().role(Role::Client).build();
        reader.read(&dst[..n]).unwrap();
        assert!(matches!(reader.next().unwrap().unwrap(), Message::Text("Hello")));
    }

    #[test]
    fn round_trip_client() {
        use crate::ws::{FrameReader, Message};
        let writer = FrameWriter::new(Role::Client);
        let mut dst = vec![0u8; writer.max_encoded_len(5)];
        let n = writer.encode_text(b"Hello", &mut dst);

        let mut reader = FrameReader::builder().role(Role::Server).build();
        reader.read(&dst[..n]).unwrap();
        assert!(matches!(reader.next().unwrap().unwrap(), Message::Text("Hello")));
    }

    #[test]
    fn encode_close_code_round_trip() {
        use crate::ws::{FrameReader, Message, CloseCode};
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; 64];
        let n = writer.encode_close_code(CloseCode::Normal, "goodbye", &mut dst);

        let mut reader = FrameReader::builder().role(Role::Client).build();
        reader.read(&dst[..n]).unwrap();
        match reader.next().unwrap().unwrap() {
            Message::Close(cf) => {
                assert_eq!(cf.code, CloseCode::Normal);
                assert_eq!(cf.reason, "goodbye");
            }
            other => panic!("expected Close, got {other:?}"),
        }
    }

    #[test]
    #[should_panic(expected = "ping payload must be <= 125")]
    fn ping_too_large() {
        let writer = FrameWriter::new(Role::Server);
        let mut dst = vec![0u8; 256];
        writer.encode_ping(&[0; 126], &mut dst);
    }
}
