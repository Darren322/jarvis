//! Cuts `rpicam-vid --codec mjpeg -o -` output into single JPEG frames.
//!
//! The program writes JPEGs back to back with nothing between them. Each starts
//! with the start-of-image marker `FF D8` and ends with the end-of-image marker
//! `FF D9`. Inside a JPEG's compressed data every `FF` byte is followed by `00`,
//! so `FF D9` marks the end of the frame (the program's frames carry no
//! thumbnail, which could hold a second one).

use std::io;

use tokio_util::{
    bytes::{Buf, Bytes, BytesMut},
    codec::Decoder,
};

/// The largest frame kept, in bytes, both markers included. An 800x450 MJPEG
/// frame is far smaller (tens of kilobytes); a longer run of bytes is not a
/// frame worth showing, and the limit bounds what the reader buffers when the
/// end marker never comes.
pub(crate) const MAX_FRAME_BYTES: usize = 1024 * 1024;

const START_OF_IMAGE: [u8; 2] = [0xFF, 0xD8];
const END_OF_IMAGE: [u8; 2] = [0xFF, 0xD9];

/// Where the decoder is in the byte stream.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Scan {
    /// Looking for the next start marker; anything before it is skipped.
    #[default]
    Seeking,
    /// The buffer starts with a start marker. No end marker begins before
    /// `searched`, so the next search starts there.
    InFrame { searched: usize },
    /// The frame grew past [`MAX_FRAME_BYTES`]: dropping bytes up to its end marker.
    Skipping,
}

/// A [`Decoder`] of whole JPEG frames. It never fails: bytes outside a frame are
/// skipped and a frame over [`MAX_FRAME_BYTES`] is dropped, so one bad frame
/// never stops the stream.
#[derive(Debug, Default)]
pub(crate) struct JpegFrames {
    scan: Scan,
}

impl Decoder for JpegFrames {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<Bytes>, io::Error> {
        loop {
            match self.scan {
                Scan::Seeking => match find(buffer, START_OF_IMAGE, 0) {
                    Some(start) => {
                        buffer.advance(start);
                        self.scan = Scan::InFrame {
                            searched: START_OF_IMAGE.len(),
                        };
                    }
                    None => {
                        keep_last_byte(buffer);
                        return Ok(None);
                    }
                },
                Scan::InFrame { searched } => match find(buffer, END_OF_IMAGE, searched) {
                    Some(end) => {
                        let len = end + END_OF_IMAGE.len();
                        self.scan = Scan::Seeking;
                        if len > MAX_FRAME_BYTES {
                            buffer.advance(len);
                            continue;
                        }
                        return Ok(Some(buffer.split_to(len).freeze()));
                    }
                    None if buffer.len() > MAX_FRAME_BYTES => {
                        keep_last_byte(buffer);
                        self.scan = Scan::Skipping;
                        return Ok(None);
                    }
                    None => {
                        // The last byte may be the first half of the end marker.
                        self.scan = Scan::InFrame {
                            searched: buffer.len().saturating_sub(1).max(START_OF_IMAGE.len()),
                        };
                        return Ok(None);
                    }
                },
                Scan::Skipping => match find(buffer, END_OF_IMAGE, 0) {
                    Some(end) => {
                        buffer.advance(end + END_OF_IMAGE.len());
                        self.scan = Scan::Seeking;
                    }
                    None => {
                        keep_last_byte(buffer);
                        return Ok(None);
                    }
                },
            }
        }
    }

    /// At the end of the stream, half a frame or trailing bytes are dropped
    /// rather than reported: the program stopped, and that is all it means.
    fn decode_eof(&mut self, buffer: &mut BytesMut) -> Result<Option<Bytes>, io::Error> {
        let frame = self.decode(buffer)?;
        if frame.is_none() {
            buffer.clear();
            self.scan = Scan::Seeking;
        }
        Ok(frame)
    }
}

/// Where `marker` first begins at or after `from`.
fn find(buffer: &[u8], marker: [u8; 2], from: usize) -> Option<usize> {
    buffer
        .get(from..)?
        .windows(marker.len())
        .position(|pair| pair == marker)
        .map(|at| at + from)
}

/// Drops all but the last byte, which may be the first half of a marker.
fn keep_last_byte(buffer: &mut BytesMut) {
    buffer.advance(buffer.len().saturating_sub(1));
}

#[cfg(test)]
#[path = "../../../tests/unit/features/camera/frames_tests.rs"]
mod tests;
