use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use crc32fast::Hasher;
use nova_core::document::Document;
use nova_core::error::{ErrorCode, NovaError, Result};
use nova_core::event::DataEvent;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

/// NVP Protocol Magic Bytes: "NVP1"
pub const NVP_MAGIC: [u8; 4] = [0x4E, 0x56, 0x50, 0x31];
/// Current NVP Protocol Version
pub const NVP_VERSION: u16 = 1;
/// Fixed header size before payload: 4(magic) + 2(version) + 1(type) + 1(flags) + 8(req_id) + 4(payload_len) = 20 bytes
pub const NVP_HEADER_SIZE: usize = 20;
/// Checksum size: 4 bytes CRC32
pub const NVP_CHECKSUM_SIZE: usize = 4;
/// Maximum allowed frame payload size to protect against malicious DOS attacks (16 MB)
pub const MAX_FRAME_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;

/// Frame types supported by NVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum FrameType {
    Request = 0x01,
    Response = 0x02,
    Event = 0x03,
    Error = 0x04,
    Ping = 0x05,
    Pong = 0x06,
}

impl FrameType {
    pub fn from_u8(b: u8) -> Result<Self> {
        match b {
            0x01 => Ok(FrameType::Request),
            0x02 => Ok(FrameType::Response),
            0x03 => Ok(FrameType::Event),
            0x04 => Ok(FrameType::Error),
            0x05 => Ok(FrameType::Ping),
            0x06 => Ok(FrameType::Pong),
            _ => Err(NovaError::protocol(format!(
                "Unknown NVP frame type: 0x{b:02X}"
            ))),
        }
    }
}

/// A complete binary network frame in the NOVA Protocol.
#[derive(Debug, Clone, PartialEq)]
pub struct NvpFrame {
    pub frame_type: FrameType,
    pub flags: u8,
    pub request_id: u64,
    pub payload: Vec<u8>,
}

impl NvpFrame {
    pub fn new(frame_type: FrameType, request_id: u64, payload: Vec<u8>) -> Self {
        Self {
            frame_type,
            flags: 0,
            request_id,
            payload,
        }
    }

    pub fn ping(request_id: u64) -> Self {
        Self::new(FrameType::Ping, request_id, Vec::new())
    }

    pub fn pong(request_id: u64) -> Self {
        Self::new(FrameType::Pong, request_id, Vec::new())
    }

    /// Encode frame into complete byte buffer with header, payload, and CRC32 checksum.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.payload.len() > MAX_FRAME_PAYLOAD_SIZE {
            return Err(NovaError::new(
                ErrorCode::FrameTooLarge,
                format!(
                    "Payload size {} bytes exceeds maximum allowed {} bytes",
                    self.payload.len(),
                    MAX_FRAME_PAYLOAD_SIZE
                ),
            ));
        }

        let mut buf = Vec::with_capacity(NVP_HEADER_SIZE + self.payload.len() + NVP_CHECKSUM_SIZE);

        buf.extend_from_slice(&NVP_MAGIC);
        buf.write_u16::<BigEndian>(NVP_VERSION)
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        buf.write_u8(self.frame_type as u8)
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        buf.write_u8(self.flags)
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        buf.write_u64::<BigEndian>(self.request_id)
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        buf.write_u32::<BigEndian>(self.payload.len() as u32)
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        buf.extend_from_slice(&self.payload);

        let mut hasher = Hasher::new();
        hasher.update(&buf);
        let checksum = hasher.finalize();

        buf.write_u32::<BigEndian>(checksum)
            .map_err(|e| NovaError::protocol(e.to_string()))?;

        Ok(buf)
    }

    /// Decode from raw slice. Returns Ok(Some((frame, total_bytes_consumed))) if complete,
    /// Ok(None) if incomplete buffer, or Err on corruption/oversized frame.
    pub fn decode_from_slice(src: &[u8]) -> Result<Option<(Self, usize)>> {
        if src.len() < NVP_HEADER_SIZE {
            return Ok(None);
        }

        if src[0..4] != NVP_MAGIC {
            return Err(NovaError::with_details(
                ErrorCode::ProtocolViolation,
                "Invalid NVP magic bytes",
                format!("Expected {:?}, found {:?}", NVP_MAGIC, &src[0..4]),
            ));
        }

        let mut r = Cursor::new(&src[4..NVP_HEADER_SIZE]);
        let version = r
            .read_u16::<BigEndian>()
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        if version != NVP_VERSION {
            return Err(NovaError::protocol(format!(
                "Unsupported NVP version: {version}"
            )));
        }

        let frame_type = FrameType::from_u8(
            r.read_u8()
                .map_err(|e| NovaError::protocol(e.to_string()))?,
        )?;
        let flags = r
            .read_u8()
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        let request_id = r
            .read_u64::<BigEndian>()
            .map_err(|e| NovaError::protocol(e.to_string()))?;
        let payload_len = r
            .read_u32::<BigEndian>()
            .map_err(|e| NovaError::protocol(e.to_string()))? as usize;

        if payload_len > MAX_FRAME_PAYLOAD_SIZE {
            return Err(NovaError::new(
                ErrorCode::FrameTooLarge,
                format!("Oversized frame: declared length {payload_len} exceeds limit {MAX_FRAME_PAYLOAD_SIZE}"),
            ));
        }

        let total_frame_len = NVP_HEADER_SIZE + payload_len + NVP_CHECKSUM_SIZE;
        if src.len() < total_frame_len {
            // Incomplete frame, wait for more data
            return Ok(None);
        }

        let payload = src[NVP_HEADER_SIZE..NVP_HEADER_SIZE + payload_len].to_vec();

        let mut r_cs = Cursor::new(&src[NVP_HEADER_SIZE + payload_len..total_frame_len]);
        let expected_checksum = r_cs
            .read_u32::<BigEndian>()
            .map_err(|e| NovaError::protocol(e.to_string()))?;

        let mut hasher = Hasher::new();
        hasher.update(&src[0..NVP_HEADER_SIZE + payload_len]);
        let calculated_checksum = hasher.finalize();

        if expected_checksum != calculated_checksum {
            return Err(NovaError::checksum_mismatch(
                expected_checksum,
                calculated_checksum,
            ));
        }

        let frame = NvpFrame {
            frame_type,
            flags,
            request_id,
            payload,
        };

        Ok(Some((frame, total_frame_len)))
    }
}

/// High-level request payload sent from client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RequestPayload {
    /// Execute an NQL query
    Query { nql: String },
    /// Start a WATCH subscription
    Watch { nql: String },
    /// Cancel an active subscription
    Unwatch { subscription_id: u64 },
    /// Authenticate session
    Auth { token: String },
    /// Ping heartbeat
    Ping,
}

/// High-level response payload sent from server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ResponsePayload {
    /// Successful mutation acknowledgment
    Success { message: String, affected: usize },
    /// Document result set
    Records {
        documents: Vec<Document>,
        count: usize,
    },
    /// Count result
    Count(usize),
    /// Exists result
    Exists(bool),
    /// Acknowledgment of a WATCH subscription
    WatchAck { subscription_id: u64 },
    /// Structured error response
    Error(NovaError),
    /// Pong heartbeat reply
    Pong,
}

/// Helper methods for serializing and deserializing payloads.
impl RequestPayload {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| NovaError::protocol(e.to_string()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes)
            .map_err(|e| NovaError::protocol(format!("Invalid request payload: {e}")))
    }
}

impl ResponsePayload {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| NovaError::protocol(e.to_string()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes)
            .map_err(|e| NovaError::protocol(format!("Invalid response payload: {e}")))
    }
}

pub fn event_to_bytes(event: &DataEvent) -> Result<Vec<u8>> {
    serde_json::to_vec(event).map_err(|e| NovaError::protocol(e.to_string()))
}

pub fn event_from_bytes(bytes: &[u8]) -> Result<DataEvent> {
    serde_json::from_slice(bytes)
        .map_err(|e| NovaError::protocol(format!("Invalid event payload: {e}")))
}
