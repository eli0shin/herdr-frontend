//! Framed mode: `[channel u32 LE][kind u8][len u32 LE][payload]` (`core:crates/herdr-core/src/wire.rs`).
//!
//! Channel 0 carries one JSON-RPC message per DATA frame. Every other channel is a stream: an attach
//! stream's terminal bytes, or an events stream's notification lines.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::types::{Display, Reason};

/// Bytes in a frame header.
pub const HEADER: usize = 9;
/// The largest payload the core accepts or sends: 16 MiB.
pub const MAX_PAYLOAD: usize = 16 << 20;
/// The JSON-RPC channel.
pub const CONTROL: u32 = 0;
/// Credit every stream starts with before the client returns any: 256 KiB.
pub const WINDOW: usize = 256 * 1024;
/// How far a viewer may fall behind before the core repaints it instead: 4 MiB.
pub const MAX_PENDING: usize = 4 << 20;
/// Unread replies the core keeps before it drops the connection: 64 MiB.
pub const MAX_CONTROL: usize = 64 << 20;

/// The kind byte of each frame (`herdr-core-protocol.md` §1 "Framing").
pub mod kind {
    /// Channel 0: one JSON-RPC message. Streams: their bytes. Viewer → core: human input.
    pub const DATA: u8 = 0;
    /// Both ways: u32 LE count of bytes the receiver consumed.
    pub const CREDIT: u8 = 1;
    /// Client → core: empty (detach). Core → client: a JSON reason.
    pub const CLOSE: u8 = 2;
    /// u16 LE cols, u16 LE rows. Viewer → core; core → observers that asked for it.
    pub const RESIZE: u8 = 3;
    /// Viewer → core: 1 gained focus, 0 lost it.
    pub const FOCUS: u8 = 4;
    /// Viewer → core: `Display` JSON.
    pub const FACTS: u8 = 5;
    /// Viewer → core: replies the terminal engine generated; not human activity.
    pub const AUTOMATIC_INPUT: u8 = 6;
}

/// A frame's kind. Kinds this crate doesn't know decode as `Unknown` and are skipped by callers,
/// as the core skips them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Data,
    Credit,
    Close,
    Resize,
    Focus,
    Facts,
    AutomaticInput,
    Unknown(u8),
}

impl From<u8> for Kind {
    fn from(byte: u8) -> Self {
        match byte {
            kind::DATA => Kind::Data,
            kind::CREDIT => Kind::Credit,
            kind::CLOSE => Kind::Close,
            kind::RESIZE => Kind::Resize,
            kind::FOCUS => Kind::Focus,
            kind::FACTS => Kind::Facts,
            kind::AUTOMATIC_INPUT => Kind::AutomaticInput,
            other => Kind::Unknown(other),
        }
    }
}

impl From<Kind> for u8 {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Data => kind::DATA,
            Kind::Credit => kind::CREDIT,
            Kind::Close => kind::CLOSE,
            Kind::Resize => kind::RESIZE,
            Kind::Focus => kind::FOCUS,
            Kind::Facts => kind::FACTS,
            Kind::AutomaticInput => kind::AUTOMATIC_INPUT,
            Kind::Unknown(byte) => byte,
        }
    }
}

/// What went wrong reading or writing frames. Either way the connection can't continue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    /// A payload above `MAX_PAYLOAD`.
    TooLarge { len: usize },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::TooLarge { len } => write!(f, "frame of {len} bytes exceeds {MAX_PAYLOAD}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// One frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: Kind,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(channel: u32, kind: Kind, payload: impl Into<Vec<u8>>) -> Self {
        Frame {
            channel,
            kind,
            payload: payload.into(),
        }
    }

    /// DATA: a JSON-RPC message on channel 0, or human input on an attach channel.
    pub fn data(channel: u32, bytes: impl Into<Vec<u8>>) -> Self {
        Frame::new(channel, Kind::Data, bytes)
    }

    /// CREDIT: the client consumed `bytes` bytes of this stream.
    pub fn credit(channel: u32, bytes: u32) -> Self {
        Frame::new(channel, Kind::Credit, bytes.to_le_bytes())
    }

    /// CLOSE from the client: detach or unsubscribe.
    pub fn close(channel: u32) -> Self {
        Frame::new(channel, Kind::Close, Vec::new())
    }

    /// RESIZE: the viewer's grid. Sending it takes the size (§4).
    pub fn resize(channel: u32, cols: u16, rows: u16) -> Self {
        let mut payload = Vec::with_capacity(4);
        payload.extend_from_slice(&cols.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        Frame::new(channel, Kind::Resize, payload)
    }

    /// FOCUS: gaining focus counts as activity and takes the size; losing it doesn't.
    pub fn focus(channel: u32, gained: bool) -> Self {
        Frame::new(channel, Kind::Focus, vec![u8::from(gained)])
    }

    /// FACTS: the viewer's colors and cell size.
    pub fn facts(channel: u32, display: &Display) -> Self {
        let payload = serde_json::to_vec(display).expect("Display always serializes");
        Frame::new(channel, Kind::Facts, payload)
    }

    /// AUTOMATIC_INPUT: replies the terminal engine generated. Send it only when both
    /// `hello.features` and the attach reply allow it; otherwise send DATA.
    pub fn automatic_input(channel: u32, bytes: impl Into<Vec<u8>>) -> Self {
        Frame::new(channel, Kind::AutomaticInput, bytes)
    }

    /// A CREDIT frame's count. `None` for other kinds and malformed payloads, which the core
    /// ignores too.
    pub fn credit_bytes(&self) -> Option<u32> {
        match (self.kind, <[u8; 4]>::try_from(self.payload.as_slice())) {
            (Kind::Credit, Ok(bytes)) => Some(u32::from_le_bytes(bytes)),
            _ => None,
        }
    }

    /// A RESIZE frame's `(cols, rows)`. `None` for other kinds and malformed payloads.
    pub fn resize_grid(&self) -> Option<(u16, u16)> {
        match (self.kind, self.payload.as_slice()) {
            (Kind::Resize, &[c0, c1, r0, r1]) => {
                Some((u16::from_le_bytes([c0, c1]), u16::from_le_bytes([r0, r1])))
            }
            _ => None,
        }
    }

    /// A FOCUS frame's state. `None` for other kinds and malformed payloads.
    pub fn focus_gained(&self) -> Option<bool> {
        match (self.kind, self.payload.as_slice()) {
            (Kind::Focus, &[byte]) => Some(byte != 0),
            _ => None,
        }
    }

    /// Why the core closed a stream. `None` for other kinds; an empty or malformed reason gives
    /// an empty `CloseReason`, since the stream is over either way.
    pub fn close_reason(&self) -> Option<CloseReason> {
        (self.kind == Kind::Close).then(|| CloseReason::from_payload(&self.payload))
    }

    /// Appends the encoded frame to `out`.
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), FrameError> {
        let len = self.payload.len();
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge { len });
        }
        out.reserve(HEADER + len);
        out.extend_from_slice(&self.channel.to_le_bytes());
        out.push(self.kind.into());
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&self.payload);
        Ok(())
    }

    /// The encoded frame.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let mut out = Vec::with_capacity(HEADER + self.payload.len());
        self.encode_into(&mut out)?;
        Ok(out)
    }
}

/// The JSON reason on a CLOSE from the core, such as
/// `{"reason":"exited","exit_code":0,"signal":null}`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloseReason {
    pub reason: Option<Reason>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub finished_ms: Option<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl CloseReason {
    /// Parses a CLOSE payload. Empty or malformed payloads give an empty reason.
    pub fn from_payload(payload: &[u8]) -> Self {
        serde_json::from_slice(payload).unwrap_or_default()
    }
}

/// Splits a byte stream into frames, however the reads split it.
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// Where the next undecoded byte is in `buf`.
    start: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes read from the connection.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        } else if self.start > self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole frame, or `None` until more bytes arrive. A header announcing a payload
    /// above `MAX_PAYLOAD` is an error that repeats: the connection must end.
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        let pending = &self.buf[self.start..];
        let Some(header) = pending.get(..HEADER) else {
            return Ok(None);
        };
        let channel = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let kind = Kind::from(header[4]);
        let len = u32::from_le_bytes([header[5], header[6], header[7], header[8]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(FrameError::TooLarge { len });
        }
        let Some(payload) = pending.get(HEADER..HEADER + len) else {
            return Ok(None);
        };
        let frame = Frame {
            channel,
            kind,
            payload: payload.to_vec(),
        };
        self.start += HEADER + len;
        Ok(Some(frame))
    }

    /// Bytes pushed but not yet returned as frames.
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.start
    }

    /// Whether the stream sits between frames, so an end of stream here is clean.
    pub fn is_between_frames(&self) -> bool {
        self.buffered() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(decoder: &mut Decoder) -> Vec<Frame> {
        std::iter::from_fn(|| decoder.next_frame().unwrap()).collect()
    }

    #[test]
    fn kinds_round_trip_through_their_bytes() {
        for byte in 0..=u8::MAX {
            assert_eq!(u8::from(Kind::from(byte)), byte);
        }
        assert_eq!(Kind::from(6), Kind::AutomaticInput);
        assert_eq!(Kind::from(7), Kind::Unknown(7));
    }

    #[test]
    fn encodes_the_wire_layout() {
        let bytes = Frame::data(0x0102_0304, b"hi".to_vec()).encode().unwrap();
        assert_eq!(bytes, [4, 3, 2, 1, 0, 2, 0, 0, 0, b'h', b'i']);
        assert_eq!(
            Frame::close(5).encode().unwrap(),
            [5, 0, 0, 0, 2, 0, 0, 0, 0]
        );
    }

    #[test]
    fn control_frames_carry_their_payloads() {
        let credit = Frame::credit(3, 1 << 20);
        assert_eq!(credit.payload, (1u32 << 20).to_le_bytes());
        assert_eq!(credit.credit_bytes(), Some(1 << 20));

        let resize = Frame::resize(3, 120, 40);
        assert_eq!(resize.payload, [120, 0, 40, 0]);
        assert_eq!(resize.resize_grid(), Some((120, 40)));

        assert_eq!(Frame::focus(3, true).payload, [1]);
        assert_eq!(Frame::focus(3, false).payload, [0]);
        assert_eq!(Frame::focus(3, true).focus_gained(), Some(true));
        assert_eq!(Frame::focus(3, false).focus_gained(), Some(false));

        let facts = Frame::facts(
            3,
            &Display {
                fg: Some("#ffffff".into()),
                bg: None,
                cursor: None,
                cell: Some([8, 17]),
                ..Default::default()
            },
        );
        assert_eq!(facts.kind, Kind::Facts);
        assert_eq!(
            serde_json::from_slice::<Value>(&facts.payload).unwrap(),
            serde_json::json!({"fg": "#ffffff", "cell": [8, 17]})
        );

        let automatic = Frame::automatic_input(3, b"\x1b[?1;2c".to_vec());
        assert_eq!(automatic.kind, Kind::AutomaticInput);
        assert_eq!(u8::from(automatic.kind), kind::AUTOMATIC_INPUT);
    }

    #[test]
    fn malformed_control_payloads_read_as_nothing() {
        assert_eq!(
            Frame::new(1, Kind::Credit, vec![1, 2, 3]).credit_bytes(),
            None
        );
        assert_eq!(
            Frame::new(1, Kind::Data, vec![1, 2, 3, 4]).credit_bytes(),
            None
        );
        assert_eq!(
            Frame::new(1, Kind::Resize, vec![1, 2, 3]).resize_grid(),
            None
        );
        assert_eq!(
            Frame::new(1, Kind::Resize, vec![1, 2, 3, 4, 5]).resize_grid(),
            None
        );
        assert_eq!(Frame::new(1, Kind::Focus, vec![]).focus_gained(), None);
        assert_eq!(Frame::data(1, b"x".to_vec()).close_reason(), None);
    }

    #[test]
    fn close_reasons_parse_with_unknown_fields_kept() {
        let frame = Frame::new(
            2,
            Kind::Close,
            br#"{"reason":"exited","exit_code":0,"signal":null,"later":1}"#.to_vec(),
        );
        let reason = frame.close_reason().unwrap();
        assert_eq!(reason.reason, Some(Reason::Exited));
        assert_eq!(reason.exit_code, Some(0));
        assert_eq!(reason.signal, None);
        assert_eq!(reason.extra["later"], 1);
        assert_eq!(Frame::close(2).close_reason(), Some(CloseReason::default()));
        assert_eq!(
            Frame::new(2, Kind::Close, b"not json".to_vec()).close_reason(),
            Some(CloseReason::default())
        );
    }

    #[test]
    fn decodes_frames_split_at_every_byte() {
        let frames = [
            Frame::data(0, br#"{"jsonrpc":"2.0","id":1,"result":{}}"#.to_vec()),
            Frame::credit(4, 99),
            Frame::new(7, Kind::Unknown(42), b"later".to_vec()),
            Frame::close(4),
            Frame::data(4, vec![0xff; 300]),
        ];
        let mut bytes = Vec::new();
        for frame in &frames {
            frame.encode_into(&mut bytes).unwrap();
        }
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::new();
            decoder.push(&bytes[..split]);
            let mut got = decode_all(&mut decoder);
            decoder.push(&bytes[split..]);
            got.extend(decode_all(&mut decoder));
            assert_eq!(got, frames, "split at {split}");
            assert!(decoder.is_between_frames());
        }
    }

    #[test]
    fn decodes_one_byte_at_a_time() {
        let frames = [
            Frame::resize(9, 80, 24),
            Frame::data(0, Vec::new()),
            Frame::focus(9, true),
        ];
        let mut bytes = Vec::new();
        for frame in &frames {
            frame.encode_into(&mut bytes).unwrap();
        }
        let mut decoder = Decoder::new();
        let mut got = Vec::new();
        for byte in &bytes {
            decoder.push(std::slice::from_ref(byte));
            got.extend(decode_all(&mut decoder));
        }
        assert_eq!(got, frames);
    }

    #[test]
    fn a_partial_frame_waits_and_is_not_clean() {
        let bytes = Frame::data(1, b"hello".to_vec()).encode().unwrap();
        let mut decoder = Decoder::new();
        decoder.push(&bytes[..HEADER + 2]);
        assert_eq!(decoder.next_frame().unwrap(), None);
        assert!(!decoder.is_between_frames());
        assert_eq!(decoder.buffered(), HEADER + 2);
        decoder.push(&bytes[HEADER + 2..]);
        assert_eq!(
            decoder.next_frame().unwrap(),
            Some(Frame::data(1, b"hello".to_vec()))
        );
        assert_eq!(decoder.next_frame().unwrap(), None);
    }

    #[test]
    fn oversized_frames_are_errors() {
        let mut header = Vec::new();
        header.extend_from_slice(&0u32.to_le_bytes());
        header.push(kind::DATA);
        header.extend_from_slice(&(MAX_PAYLOAD as u32 + 1).to_le_bytes());
        let mut decoder = Decoder::new();
        decoder.push(&header);
        let err = decoder.next_frame().unwrap_err();
        assert_eq!(
            err,
            FrameError::TooLarge {
                len: MAX_PAYLOAD + 1
            }
        );
        assert_eq!(decoder.next_frame().unwrap_err(), err, "the error repeats");

        let too_big = Frame::data(0, vec![0; MAX_PAYLOAD + 1]);
        assert_eq!(
            too_big.encode().unwrap_err(),
            FrameError::TooLarge {
                len: MAX_PAYLOAD + 1
            }
        );
        assert!(Frame::data(0, vec![0; MAX_PAYLOAD]).encode().is_ok());
    }

    #[test]
    fn the_buffer_compacts_as_frames_are_read() {
        let frame = Frame::data(1, vec![7; 1000]).encode().unwrap();
        let mut decoder = Decoder::new();
        for _ in 0..100 {
            decoder.push(&frame);
            assert!(decoder.next_frame().unwrap().is_some());
        }
        assert!(
            decoder.buf.capacity() < 4 * frame.len(),
            "{}",
            decoder.buf.capacity()
        );
    }
}
