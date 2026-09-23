use crate::frame::NvpFrame;
use bytes::BytesMut;
use std::io;
use tokio_util::codec::{Decoder, Encoder};

/// Tokio framed codec for asynchronous encoding and decoding of NVP frames.
#[derive(Debug, Default, Clone)]
pub struct NvpCodec;

impl NvpCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Decoder for NvpCodec {
    type Item = NvpFrame;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match NvpFrame::decode_from_slice(src) {
            Ok(Some((frame, bytes_consumed))) => {
                let _ = src.split_to(bytes_consumed);
                Ok(Some(frame))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string())),
        }
    }
}

impl Encoder<NvpFrame> for NvpCodec {
    type Error = io::Error;

    fn encode(&mut self, item: NvpFrame, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let bytes = item
            .encode()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        dst.extend_from_slice(&bytes);
        Ok(())
    }
}
