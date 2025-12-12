//! Message handling for server-side protocol

#![allow(dead_code)]

use bytes::{Bytes, BytesMut, Buf, BufMut};
use polyglot_common::{ClientMessage, ServerMessage, encode_message, decode_message, MAX_MESSAGE_SIZE};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("Message too large: {0} bytes (max: {1})")]
    MessageTooLarge(usize, usize),
    #[error("Invalid message format")]
    InvalidFormat,
    #[error("Incomplete message")]
    Incomplete,
    #[error("Encoding error: {0}")]
    EncodingError(#[from] rmp_serde::encode::Error),
    #[error("Decoding error: {0}")]
    DecodingError(#[from] rmp_serde::decode::Error),
}

pub struct MessageCodec;

impl MessageCodec {
    pub fn encode_server_message(msg: &ServerMessage) -> Result<Bytes, ProtocolError> {
        let data = encode_message(msg)?;

        if data.len() > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge(data.len(), MAX_MESSAGE_SIZE));
        }

        let mut buf = BytesMut::with_capacity(4 + data.len());
        buf.put_u32(data.len() as u32);
        buf.put_slice(&data);

        Ok(buf.freeze())
    }

    pub fn decode_client_message(buf: &mut BytesMut) -> Result<Option<ClientMessage>, ProtocolError> {
        if buf.len() < 4 {
            return Ok(None);
        }

        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;

        if len > MAX_MESSAGE_SIZE {
            return Err(ProtocolError::MessageTooLarge(len, MAX_MESSAGE_SIZE));
        }

        if buf.len() < 4 + len {
            return Ok(None);
        }

        buf.advance(4);
        let data = buf.split_to(len);
        let msg = decode_message(&data)?;

        Ok(Some(msg))
    }

    pub fn has_complete_message(buf: &[u8]) -> bool {
        if buf.len() < 4 {
            return false;
        }

        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
        buf.len() >= 4 + len
    }
}

pub struct StreamReader {
    buffer: BytesMut,
}

impl StreamReader {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(8192),
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    pub fn try_read(&mut self) -> Result<Option<ClientMessage>, ProtocolError> {
        MessageCodec::decode_client_message(&mut self.buffer)
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl Default for StreamReader {
    fn default() -> Self {
        Self::new()
    }
}

pub struct StreamWriter {
    buffer: BytesMut,
}

impl StreamWriter {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(8192),
        }
    }

    pub fn queue(&mut self, msg: &ServerMessage) -> Result<(), ProtocolError> {
        let encoded = MessageCodec::encode_server_message(msg)?;
        self.buffer.extend_from_slice(&encoded);
        Ok(())
    }

    pub fn pending(&self) -> &[u8] {
        &self.buffer
    }

    pub fn advance(&mut self, count: usize) {
        self.buffer.advance(count);
    }

    pub fn has_pending(&self) -> bool {
        !self.buffer.is_empty()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn take(&mut self) -> Bytes {
        self.buffer.split().freeze()
    }
}

impl Default for StreamWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polyglot_common::Tool;

    #[test]
    fn test_encode_decode_roundtrip() {
        let msg = ServerMessage::ToolResponse {
            tool: Tool::Claude,
            content: "Hello, world!".to_string(),
            done: false,
            tokens: Some(10),
        };

        let encoded = MessageCodec::encode_server_message(&msg).unwrap();

        let mut buf = BytesMut::from(&encoded[..]);

        assert!(MessageCodec::has_complete_message(&buf));
    }

    #[test]
    fn test_stream_reader_writer() {
        let mut writer = StreamWriter::new();
        let msg = ServerMessage::Pong {
            timestamp: 12345,
            server_time: 12346,
        };

        writer.queue(&msg).unwrap();
        assert!(writer.has_pending());

        let data = writer.take();
        assert!(!data.is_empty());
        assert!(!writer.has_pending());
    }
}
