use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

const MAX_INPUT_BYTES: usize = 2_048;
const MAX_FRAMED_INPUT_BYTES: usize = MAX_INPUT_BYTES + 2;

pub(crate) enum InputLine {
    Eof,
    Prompt(String),
    Rejected(InputRejection),
}

#[derive(Clone, Copy)]
pub(crate) enum InputRejection {
    TooLong,
    InvalidUtf8,
    Blank,
}

impl InputRejection {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::TooLong => "the line exceeds the 2,048-byte limit.",
            Self::InvalidUtf8 => "the line is not valid UTF-8.",
            Self::Blank => "blank messages are not sent to the assistant.",
        }
    }
}

pub(crate) struct InputReader<R> {
    reader: R,
    partial: Vec<u8>,
    discarding_overflow: bool,
}

impl<R> InputReader<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            partial: Vec::with_capacity(MAX_FRAMED_INPUT_BYTES),
            discarding_overflow: false,
        }
    }
}

pub(crate) async fn read_input_line<R>(reader: &mut InputReader<R>) -> io::Result<InputLine>
where
    R: AsyncBufRead + Unpin,
{
    // LEARNING: `R` may be any reader with these capabilities, like a Java
    // generic method bounded by interfaces. `Unpin` means its address need not
    // stay fixed while async helpers poll it; Tokio requires this bound here.
    loop {
        let (consumed, terminated, copied, eof) = {
            let available = reader.reader.fill_buf().await?;
            if available.is_empty() {
                (0, false, Vec::new(), true)
            } else {
                let newline = available.iter().position(|byte| *byte == b'\n');
                let consumed = newline.map_or(available.len(), |index| index + 1);
                let remaining = MAX_FRAMED_INPUT_BYTES.saturating_sub(reader.partial.len());
                let copy_len = if reader.discarding_overflow {
                    0
                } else {
                    consumed.min(remaining.saturating_add(1))
                };
                (
                    consumed,
                    newline.is_some(),
                    available[..copy_len].to_vec(),
                    false,
                )
            }
        };

        if eof {
            if reader.discarding_overflow {
                reader.discarding_overflow = false;
                reader.partial.clear();
                return Ok(InputLine::Rejected(InputRejection::TooLong));
            }
            if reader.partial.is_empty() {
                return Ok(InputLine::Eof);
            }
            let bytes = std::mem::take(&mut reader.partial);
            return decode_input(bytes, false);
        }

        if !reader.discarding_overflow {
            if reader.partial.len().saturating_add(copied.len()) > MAX_FRAMED_INPUT_BYTES {
                reader.partial.clear();
                reader.discarding_overflow = true;
            } else {
                reader.partial.extend_from_slice(&copied);
                if !terminated && reader.partial.len() > MAX_INPUT_BYTES {
                    reader.partial.clear();
                    reader.discarding_overflow = true;
                }
            }
        }
        reader.reader.consume(consumed);

        if terminated {
            if reader.discarding_overflow {
                reader.discarding_overflow = false;
                reader.partial.clear();
                return Ok(InputLine::Rejected(InputRejection::TooLong));
            }
            let bytes = std::mem::take(&mut reader.partial);
            return decode_input(bytes, true);
        }
    }
}

fn decode_input(mut bytes: Vec<u8>, terminated: bool) -> io::Result<InputLine> {
    if terminated {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Ok(InputLine::Rejected(InputRejection::TooLong));
    }
    let prompt = match String::from_utf8(bytes) {
        Ok(prompt) => prompt,
        Err(_) => return Ok(InputLine::Rejected(InputRejection::InvalidUtf8)),
    };
    if prompt.trim().is_empty() {
        return Ok(InputLine::Rejected(InputRejection::Blank));
    }
    Ok(InputLine::Prompt(prompt))
}

#[cfg(test)]
#[path = "../../../tests/unit/interfaces/cli/input_tests.rs"]
mod tests;
