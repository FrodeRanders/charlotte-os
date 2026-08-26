//! Bounded HTTP/1.1 response-head and chunk framing parser.

use alloc::string::{
    String,
    ToString,
};

pub const MAX_RESPONSE_HEAD: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Incomplete,
    HeadTooLarge,
    InvalidStatus,
    InvalidHeader,
    ConflictingLength,
    UnsupportedTransferEncoding,
    InvalidChunk,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseHead {
    pub status: u16,
    pub content_length: Option<u64>,
    pub chunked: bool,
    pub connection_close: bool,
    pub etag: Option<String>,
    pub version_id: Option<String>,
    pub request_id: Option<String>,
}

impl ResponseHead {
    /// Parse a response head and return it with the offset of the first body
    /// byte. Callers retain bytes following that offset as the first body
    /// fragment.
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize), Error> {
        if bytes.len() > MAX_RESPONSE_HEAD && find_head_end(bytes).is_none() {
            return Err(Error::HeadTooLarge);
        }
        let end = find_head_end(bytes).ok_or(Error::Incomplete)?;
        if end > MAX_RESPONSE_HEAD {
            return Err(Error::HeadTooLarge);
        }
        let text = core::str::from_utf8(&bytes[..end - 4]).map_err(|_| Error::InvalidHeader)?;
        let mut lines = text.split("\r\n");
        let status_line = lines.next().ok_or(Error::InvalidStatus)?;
        let mut status_parts = status_line.split_ascii_whitespace();
        if status_parts.next() != Some("HTTP/1.1") {
            return Err(Error::InvalidStatus);
        }
        let status = status_parts
            .next()
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|value| (100..=599).contains(value))
            .ok_or(Error::InvalidStatus)?;
        let mut response = Self {
            status,
            content_length: None,
            chunked: false,
            connection_close: false,
            etag: None,
            version_id: None,
            request_id: None,
        };
        for line in lines {
            let (name, value) = line.split_once(':').ok_or(Error::InvalidHeader)?;
            let value = value.trim_matches([' ', '\t']);
            if name.eq_ignore_ascii_case("content-length") {
                let length = value.parse::<u64>().map_err(|_| Error::InvalidHeader)?;
                if response.content_length.replace(length).is_some() {
                    return Err(Error::ConflictingLength);
                }
            } else if name.eq_ignore_ascii_case("transfer-encoding") {
                if !value.eq_ignore_ascii_case("chunked") {
                    return Err(Error::UnsupportedTransferEncoding);
                }
                response.chunked = true;
            } else if name.eq_ignore_ascii_case("connection") {
                response.connection_close = value.eq_ignore_ascii_case("close");
            } else if name.eq_ignore_ascii_case("etag") {
                response.etag = Some(value.trim_matches('"').to_string());
            } else if name.eq_ignore_ascii_case("x-amz-version-id") {
                response.version_id = Some(value.to_string());
            } else if name.eq_ignore_ascii_case("x-amz-request-id")
                || name.eq_ignore_ascii_case("x-emc-request-id")
            {
                response.request_id = Some(value.to_string());
            }
        }
        if response.chunked && response.content_length.is_some() {
            return Err(Error::ConflictingLength);
        }
        Ok((response, end))
    }
}

fn find_head_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n").map(|offset| offset + 4)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChunkState {
    Size,
    Data(usize),
    DataCrLf,
    Trailers,
    Complete,
}

/// Incremental HTTP chunked-transfer decoder. Input may be split at any byte;
/// decoded output is appended to `output` and framing bytes are discarded.
pub struct ChunkedDecoder {
    state: ChunkState,
    line: [u8; 128],
    line_len: usize,
}

impl Default for ChunkedDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkedDecoder {
    pub const fn new() -> Self {
        Self {
            state: ChunkState::Size,
            line: [0; 128],
            line_len: 0,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.state == ChunkState::Complete
    }

    pub fn decode(
        &mut self,
        mut input: &[u8],
        output: &mut alloc::vec::Vec<u8>,
    ) -> Result<(), Error> {
        while !input.is_empty() && !self.is_complete() {
            match self.state {
                ChunkState::Size | ChunkState::Trailers => {
                    let byte = input[0];
                    input = &input[1..];
                    if self.line_len == self.line.len() {
                        return Err(Error::InvalidChunk);
                    }
                    self.line[self.line_len] = byte;
                    self.line_len += 1;
                    if self.line_len >= 2 && &self.line[self.line_len - 2..self.line_len] == b"\r\n"
                    {
                        let line_len = self.line_len - 2;
                        if self.state == ChunkState::Trailers {
                            self.line_len = 0;
                            if line_len == 0 {
                                self.state = ChunkState::Complete;
                            }
                            continue;
                        }
                        let line = core::str::from_utf8(&self.line[..line_len])
                            .map_err(|_| Error::InvalidChunk)?;
                        let size_text = line.split(';').next().ok_or(Error::InvalidChunk)?.trim();
                        let size = usize::from_str_radix(size_text, 16)
                            .map_err(|_| Error::InvalidChunk)?;
                        self.line_len = 0;
                        self.state = if size == 0 {
                            ChunkState::Trailers
                        } else {
                            ChunkState::Data(size)
                        };
                    }
                }
                ChunkState::Data(remaining) => {
                    let take = remaining.min(input.len());
                    output.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    self.state = if take == remaining {
                        ChunkState::DataCrLf
                    } else {
                        ChunkState::Data(remaining - take)
                    };
                }
                ChunkState::DataCrLf => {
                    let needed = 2 - self.line_len;
                    let take = needed.min(input.len());
                    self.line[self.line_len..self.line_len + take].copy_from_slice(&input[..take]);
                    self.line_len += take;
                    input = &input[take..];
                    if self.line_len == 2 {
                        if &self.line[..2] != b"\r\n" {
                            return Err(Error::InvalidChunk);
                        }
                        self.line_len = 0;
                        self.state = ChunkState::Size;
                    }
                }
                ChunkState::Complete => break,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use alloc::vec::Vec;

    use super::*;

    #[test]
    fn parses_s3_response_and_preserves_body_offset() {
        let bytes = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nETag: \"abc\"\r\nX-Amz-Request-Id: req-1\r\n\r\nhello";
        let (head, offset) = ResponseHead::parse(bytes).unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.content_length, Some(5));
        assert_eq!(head.etag.as_deref(), Some("abc"));
        assert_eq!(head.request_id.as_deref(), Some("req-1"));
        assert_eq!(&bytes[offset..], b"hello");
    }

    #[test]
    fn decodes_arbitrarily_split_chunks_and_trailers() {
        let mut decoder = ChunkedDecoder::new();
        let mut output = Vec::new();
        for part in [
            b"4\r".as_slice(),
            b"\nWi".as_slice(),
            b"ki\r\n5\r\np".as_slice(),
            b"edia\r\n0\r\nX-Test: yes\r\n\r\n".as_slice(),
        ] {
            decoder.decode(part, &mut output).unwrap();
        }
        assert!(decoder.is_complete());
        assert_eq!(output, b"Wikipedia");
    }
}
