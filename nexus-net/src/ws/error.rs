/// Protocol error from WebSocket frame decoding.
///
/// Each variant is a specific RFC 6455 violation. No catch-all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Frame header contains an unrecognized opcode.
    InvalidOpcode(u8),
    /// Reserved bits (RSV1-3) are set without a negotiated extension.
    ReservedBitsSet {
        /// The RSV bits that were set (bits 4-6 of byte 0).
        bits: u8,
    },
    /// Server sent a masked frame (RFC 6455 §5.1: server MUST NOT mask).
    MaskedFrameFromServer,
    /// Client sent an unmasked frame (RFC 6455 §5.1: client MUST mask).
    UnmaskedFrameFromClient,
    /// Frame payload exceeds the configured maximum frame size.
    PayloadTooLarge {
        /// Declared payload size.
        size: u64,
        /// Configured maximum.
        max: u64,
    },
    /// Control frame payload exceeds 125 bytes (RFC 6455 §5.5).
    ControlFrameTooLarge {
        /// Declared payload size.
        size: u64,
    },
    /// Control frame is fragmented (RFC 6455 §5.5: MUST NOT be fragmented).
    FragmentedControlFrame,
    /// Close frame has invalid status code.
    InvalidCloseCode(u16),
    /// Close frame reason is not valid UTF-8.
    InvalidUtf8InCloseReason,
    /// Close frame payload is 1 byte (must be 0 or >= 2).
    CloseFrameTooShort,
    /// Received a continuation frame with no preceding start frame.
    ContinuationWithoutStart,
    /// Received a new data frame (Text/Binary) while assembling fragments.
    NewMessageDuringAssembly,
    /// Text message payload is not valid UTF-8.
    InvalidUtf8,
    /// Assembled message exceeds the configured maximum message size.
    MessageTooLarge {
        /// Accumulated size so far.
        accumulated: usize,
        /// Configured maximum.
        max: usize,
    },
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidOpcode(op) => write!(f, "invalid opcode: 0x{op:X}"),
            Self::ReservedBitsSet { bits } => {
                write!(f, "reserved bits set: 0b{bits:03b}")
            }
            Self::MaskedFrameFromServer => write!(f, "server sent masked frame"),
            Self::UnmaskedFrameFromClient => write!(f, "client sent unmasked frame"),
            Self::PayloadTooLarge { size, max } => {
                write!(f, "payload too large: {size} bytes (max {max})")
            }
            Self::ControlFrameTooLarge { size } => {
                write!(f, "control frame too large: {size} bytes (max 125)")
            }
            Self::FragmentedControlFrame => write!(f, "fragmented control frame"),
            Self::InvalidCloseCode(code) => write!(f, "invalid close code: {code}"),
            Self::InvalidUtf8InCloseReason => write!(f, "invalid UTF-8 in close reason"),
            Self::CloseFrameTooShort => {
                write!(f, "close frame too short (1 byte, must be 0 or >= 2)")
            }
            Self::ContinuationWithoutStart => {
                write!(f, "continuation frame without preceding start frame")
            }
            Self::NewMessageDuringAssembly => {
                write!(f, "new data frame received during fragment assembly")
            }
            Self::InvalidUtf8 => write!(f, "text message contains invalid UTF-8"),
            Self::MessageTooLarge { accumulated, max } => {
                write!(f, "assembled message too large: {accumulated} bytes (max {max})")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
