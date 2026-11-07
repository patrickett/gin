//! LSP base protocol framing: `Content-Length: n\r\n\r\n` and then `n` bytes of JSON.

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// A message larger than this is refused (and skipped), not buffered.
pub const MESSAGE_BYTES_MAX: usize = 64 * 1024 * 1024;
/// A header block longer than this many lines is not a header block.
const HEADER_LINES_MAX: usize = 64;
const HEADER_LINE_BYTES_MAX: usize = 8 * 1024;

#[derive(Debug)]
pub enum ReadError {
    /// The peer closed the stream between messages.
    Eof,
    Io(io::Error),
    /// The framing is wrong (missing or invalid `Content-Length`, truncated body). The reader
    /// is positioned where the next message may start; the caller answers with a ParseError.
    Frame(String),
    /// The frame was fine and its body is not JSON.
    Json(String),
}

fn read_line_limited<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> io::Result<bool> {
    line.clear();
    loop {
        let (done, used) = {
            let buffer = reader.fill_buf()?;
            if buffer.is_empty() {
                return Ok(!line.is_empty());
            }
            match buffer.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    line.extend_from_slice(&buffer[..=i]);
                    (true, i + 1)
                }
                None => {
                    line.extend_from_slice(buffer);
                    (false, buffer.len())
                }
            }
        };
        reader.consume(used);
        if line.len() > HEADER_LINE_BYTES_MAX {
            return Ok(true);
        }
        if done {
            return Ok(true);
        }
    }
}

/// Read one framed JSON message.
pub fn read_message<R: BufRead>(reader: &mut R) -> Result<Value, ReadError> {
    let mut length: Option<Result<usize, String>> = None;
    let mut line = Vec::new();
    let mut seen_any = false;
    for _ in 0..HEADER_LINES_MAX {
        if !read_line_limited(reader, &mut line).map_err(ReadError::Io)? {
            return Err(if seen_any { ReadError::Frame("stream ended inside the headers".into()) } else { ReadError::Eof });
        }
        seen_any = true;
        if line.len() > HEADER_LINE_BYTES_MAX {
            return Err(ReadError::Frame("header line too long".into()));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            let length = match length {
                Some(Ok(length)) => length,
                Some(Err(message)) => return Err(ReadError::Frame(message)),
                None => return Err(ReadError::Frame("missing Content-Length header".into())),
            };
            if length > MESSAGE_BYTES_MAX {
                skip(reader, length).map_err(ReadError::Io)?;
                return Err(ReadError::Frame(format!("message of {length} bytes exceeds the {MESSAGE_BYTES_MAX} byte limit")));
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).map_err(|error| match error.kind() {
                io::ErrorKind::UnexpectedEof => ReadError::Frame("stream ended inside a message body".into()),
                _ => ReadError::Io(error),
            })?;
            return serde_json::from_slice(&body).map_err(|error| ReadError::Json(error.to_string()));
        }
        if let Some((name, value)) = text.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = Some(value.trim().parse::<usize>().map_err(|_| format!("invalid Content-Length {:?}", value.trim())));
            }
        }
    }
    Err(ReadError::Frame("too many header lines".into()))
}

fn skip<R: BufRead>(reader: &mut R, mut count: usize) -> io::Result<()> {
    while count > 0 {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            break;
        }
        let used = buffer.len().min(count);
        reader.consume(used);
        count -= used;
    }
    Ok(())
}

pub fn write_message<W: Write>(writer: &mut W, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}
