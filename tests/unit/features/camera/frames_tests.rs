use tokio_util::{bytes::BytesMut, codec::Decoder};

use super::{JpegFrames, MAX_FRAME_BYTES};

/// A JPEG-shaped frame of exactly `len` bytes: start marker, `fill`, end marker.
fn frame(len: usize, fill: u8) -> Vec<u8> {
    assert!(len >= 4, "a frame holds both markers");
    let mut bytes = vec![0xFF, 0xD8];
    bytes.resize(len - 2, fill);
    bytes.extend_from_slice(&[0xFF, 0xD9]);
    bytes
}

/// Feeds `reads` to one decoder, one read at a time, as `FramedRead` would, and
/// returns every frame it cut out, then what `decode_eof` adds at the end.
fn decode_all(reads: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut decoder = JpegFrames::default();
    let mut buffer = BytesMut::new();
    let mut frames = Vec::new();
    for read in reads {
        buffer.extend_from_slice(read);
        while let Some(frame) = decoder.decode(&mut buffer).expect("decoding never fails") {
            frames.push(frame.to_vec());
        }
    }
    while let Some(frame) = decoder
        .decode_eof(&mut buffer)
        .expect("leftover bytes at the end are not an error")
    {
        frames.push(frame.to_vec());
    }
    frames
}

/// One table row: what happens, the reads in order, and the frames expected.
type Case = (&'static str, Vec<Vec<u8>>, Vec<Vec<u8>>);

fn joined(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn frames_are_cut_at_their_markers_however_the_reads_fall() {
    let one = frame(10, 0x11);
    let two = frame(12, 0x22);
    let cases: Vec<Case> = vec![
        (
            "one frame in one read",
            vec![one.clone()],
            vec![one.clone()],
        ),
        (
            "two frames in one read",
            vec![joined(&[&one, &two])],
            vec![one.clone(), two.clone()],
        ),
        (
            "a frame split across reads",
            vec![one[..3].to_vec(), one[3..7].to_vec(), one[7..].to_vec()],
            vec![one.clone()],
        ),
        (
            "the end marker split between two reads",
            vec![one[..one.len() - 1].to_vec(), one[one.len() - 1..].to_vec()],
            vec![one.clone()],
        ),
        (
            "the start marker split between two reads",
            vec![one[..1].to_vec(), one[1..].to_vec()],
            vec![one.clone()],
        ),
        (
            "bytes before the first frame are skipped",
            vec![joined(&[b"rpicam says hi \xFF\x00", &one])],
            vec![one.clone()],
        ),
        (
            "bytes between frames are skipped",
            vec![joined(&[&one, b"junk", &two])],
            vec![one.clone(), two.clone()],
        ),
        (
            "half a frame at the end is dropped",
            vec![joined(&[&one, &two[..5]])],
            vec![one.clone()],
        ),
        (
            "nothing but junk gives nothing",
            vec![b"no frames here".to_vec()],
            vec![],
        ),
        ("no bytes gives nothing", vec![], vec![]),
    ];
    for (name, reads, expected) in cases {
        assert_eq!(decode_all(&reads), expected, "{name}");
    }
}

#[test]
fn a_frame_of_the_limit_is_kept_and_one_byte_more_is_dropped() {
    let at_limit = frame(MAX_FRAME_BYTES, 0x33);
    let over_limit = frame(MAX_FRAME_BYTES + 1, 0x44);
    let after = frame(16, 0x55);
    let cases: Vec<Case> = vec![
        (
            "the limit, in one read",
            vec![at_limit.clone()],
            vec![at_limit.clone()],
        ),
        (
            "the limit, in small reads",
            at_limit.chunks(8192).map(<[u8]>::to_vec).collect(),
            vec![at_limit.clone()],
        ),
        (
            "one byte more, in one read, and the next frame still comes",
            vec![joined(&[&over_limit, &after])],
            vec![after.clone()],
        ),
        (
            "one byte more, in small reads, and the next frame still comes",
            joined(&[&over_limit, &after])
                .chunks(8192)
                .map(<[u8]>::to_vec)
                .collect(),
            vec![after.clone()],
        ),
    ];
    for (name, reads, expected) in cases {
        let frames = decode_all(&reads);
        assert_eq!(frames.len(), expected.len(), "{name}");
        assert!(frames == expected, "{name}");
    }
}

#[test]
fn a_stream_that_never_ends_a_frame_keeps_a_bounded_buffer() {
    let mut decoder = JpegFrames::default();
    let mut buffer = BytesMut::new();
    buffer.extend_from_slice(&[0xFF, 0xD8]);
    let read = vec![0x66; 8192];
    for _ in 0..(3 * MAX_FRAME_BYTES / read.len()) {
        buffer.extend_from_slice(&read);
        assert_eq!(decoder.decode(&mut buffer).expect("never fails"), None);
        assert!(
            buffer.len() <= MAX_FRAME_BYTES + read.len(),
            "{}",
            buffer.len()
        );
    }
    // Still in step: the next whole frame comes through.
    let next = frame(20, 0x77);
    buffer.extend_from_slice(&[0xFF, 0xD9]);
    buffer.extend_from_slice(&next);
    assert_eq!(
        decoder
            .decode(&mut buffer)
            .expect("never fails")
            .map(|frame| frame.to_vec()),
        Some(next)
    );
}

#[test]
fn the_frame_limit_is_one_mebibyte() {
    assert_eq!(MAX_FRAME_BYTES, 1024 * 1024);
}
