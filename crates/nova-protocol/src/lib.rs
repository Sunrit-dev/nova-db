//! NOVA Protocol (NVP) binary wire codec and frames.

pub mod codec;
pub mod frame;

pub use codec::NvpCodec;
pub use frame::{
    event_from_bytes, event_to_bytes, FrameType, NvpFrame, RequestPayload, ResponsePayload,
    MAX_FRAME_PAYLOAD_SIZE, NVP_CHECKSUM_SIZE, NVP_HEADER_SIZE, NVP_MAGIC, NVP_VERSION,
};

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use tokio_util::codec::{Decoder, Encoder};

    #[test]
    fn test_nvp_frame_roundtrip() {
        let payload = b"FIND users WHERE age > 21".to_vec();
        let frame = NvpFrame::new(FrameType::Request, 42, payload.clone());

        let encoded = frame.encode().unwrap();
        let (decoded, consumed) = NvpFrame::decode_from_slice(&encoded).unwrap().unwrap();

        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.frame_type, FrameType::Request);
        assert_eq!(decoded.request_id, 42);
        assert_eq!(decoded.payload, payload);
    }

    #[test]
    fn test_tokio_codec_incremental_buffering() {
        let mut codec = NvpCodec::new();
        let mut buf = BytesMut::new();

        let payload = b"WATCH metrics".to_vec();
        let frame = NvpFrame::new(FrameType::Request, 99, payload.clone());

        let mut encoded = BytesMut::new();
        codec.encode(frame, &mut encoded).unwrap();

        // Feed only first 10 bytes -> should return Ok(None)
        buf.extend_from_slice(&encoded[..10]);
        assert_eq!(codec.decode(&mut buf).unwrap(), None);

        // Feed remaining bytes -> should return decoded frame
        buf.extend_from_slice(&encoded[10..]);
        let decoded = codec
            .decode(&mut buf)
            .unwrap()
            .expect("frame should be ready");
        assert_eq!(decoded.request_id, 99);
        assert_eq!(decoded.payload, payload);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_invalid_magic_rejected() {
        let bad_magic = [0x58, 0x58, 0x58, 0x58]; // "XXXX"
        let mut data = Vec::new();
        data.extend_from_slice(&bad_magic);
        data.extend_from_slice(&[0; 30]);

        let res = NvpFrame::decode_from_slice(&data);
        assert!(res.is_err());
    }

    #[test]
    fn test_corrupt_checksum_rejected() {
        let frame = NvpFrame::new(FrameType::Ping, 1, vec![1, 2, 3]);
        let mut encoded = frame.encode().unwrap();

        // Corrupt one byte of payload
        let last_byte = encoded.len() - 5;
        encoded[last_byte] ^= 0xFF;

        let res = NvpFrame::decode_from_slice(&encoded);
        assert!(res.is_err());
    }

    #[test]
    fn test_request_response_payload_roundtrip() {
        let req = RequestPayload::Query {
            nql: "FIND users LIMIT 5".to_string(),
        };
        let b = req.to_bytes().unwrap();
        let parsed = RequestPayload::from_bytes(&b).unwrap();
        assert_eq!(req, parsed);

        let resp = ResponsePayload::Success {
            message: "Created".to_string(),
            affected: 1,
        };
        let rb = resp.to_bytes().unwrap();
        let parsed_resp = ResponsePayload::from_bytes(&rb).unwrap();
        assert_eq!(resp, parsed_resp);
    }

    #[test]
    fn test_concurrent_ping_pong_frames() {
        let mut codec = NvpCodec::new();
        let mut stream_buf = BytesMut::new();

        // Encode 5 consecutive Ping frames
        for req_id in 1000..1005 {
            let frame = NvpFrame::new(FrameType::Ping, req_id, Vec::new());
            codec.encode(frame, &mut stream_buf).unwrap();
        }

        // Decode them sequentially and verify request_id integrity
        for expected_id in 1000..1005 {
            let decoded = codec
                .decode(&mut stream_buf)
                .unwrap()
                .expect("frame should decode successfully");
            assert_eq!(decoded.frame_type, FrameType::Ping);
            assert_eq!(decoded.request_id, expected_id);
            assert!(decoded.payload.is_empty());
        }

        assert!(stream_buf.is_empty());
    }
}
