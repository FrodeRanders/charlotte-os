//! Bounded HTTP/1.1 response-head and chunk framing parser.

use alloc::{
    string::{
        String,
        ToString,
    },
    vec::Vec,
};

use zeroize::{
    Zeroize,
    Zeroizing,
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
    RequestTooLarge,
}

/// Borrowed fields for one HTTP/1.1 request. The caller retains authorization
/// through serialization/send. This encoder owns Connection and Content-Length.
pub struct RequestHead<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub headers: &'a [crate::sigv4::Header<'a>],
    pub content_length: Option<u64>,
}

/// Validate wire fields and reserve the complete header before copying. The
/// returned owner erases on normal send success, rejection or abandonment.
pub fn request_head(request: &RequestHead<'_>) -> Result<Zeroizing<Vec<u8>>, Error> {
    if request.method.is_empty()
        || !request.method.bytes().all(token_byte)
        || !request.path.starts_with('/')
        || request.path.bytes().any(|byte| !byte.is_ascii() || byte <= b' ' || byte == 0x7f)
    {
        return Err(Error::InvalidHeader);
    }
    let mut host = false;
    for (index, header) in request.headers.iter().enumerate() {
        if header.name.is_empty()
            || !header.name.bytes().all(token_byte)
            || header.value.bytes().any(|byte| (byte < b' ' && byte != b'\t') || byte == 0x7f)
            || ["content-length", "connection", "transfer-encoding"]
                .iter()
                .any(|name| header.name.eq_ignore_ascii_case(name))
            || request.headers[..index]
                .iter()
                .any(|earlier| earlier.name.eq_ignore_ascii_case(header.name))
        {
            return Err(Error::InvalidHeader);
        }
        if header.name.eq_ignore_ascii_case("host") && header.value.trim().is_empty() {
            return Err(Error::InvalidHeader);
        }
        host |= header.name.eq_ignore_ascii_case("host");
    }
    if !host {
        return Err(Error::InvalidHeader);
    }
    // Decimal length formatting uses initialized local storage with no heap copy.
    let mut digits = [0u8; 20];
    let mut start = digits.len();
    if let Some(mut length) = request.content_length {
        loop {
            start -= 1;
            digits[start] = b'0' + (length % 10) as u8;
            length /= 10;
            if length == 0 {
                break;
            }
        }
    }
    let mut len = request.method.len();
    let mut add = |count: usize| -> Result<(), Error> {
        len = len.checked_add(count).ok_or(Error::RequestTooLarge)?;
        Ok(())
    };
    add(1)?;
    add(request.path.len())?;
    add(b" HTTP/1.1\r\n".len())?;
    for header in request.headers {
        add(header.name.len())?;
        add(b": ".len())?;
        add(header.value.len())?;
        add(b"\r\n".len())?;
    }
    if request.content_length.is_some() {
        add(b"Content-Length: ".len())?;
        add(digits.len() - start)?;
        add(b"\r\n".len())?;
    }
    add(b"Connection: close\r\n\r\n".len())?;
    let mut output = Zeroizing::new(Vec::with_capacity(len));
    output.extend_from_slice(request.method.as_bytes());
    output.push(b' ');
    output.extend_from_slice(request.path.as_bytes());
    output.extend_from_slice(b" HTTP/1.1\r\n");
    for header in request.headers {
        output.extend_from_slice(header.name.as_bytes());
        output.extend_from_slice(b": ");
        output.extend_from_slice(header.value.as_bytes());
        output.extend_from_slice(b"\r\n");
    }
    if request.content_length.is_some() {
        output.extend_from_slice(b"Content-Length: ");
        output.extend_from_slice(&digits[start..]);
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(output)
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
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

/// Parsed response metadata and the first owned body fragment.
pub type ResponseStart = (ResponseHead, Zeroizing<Vec<u8>>);

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

/// One bounded response head, without accumulation reallocations. The caller
/// supplies bounded transport chunks and retains its existing read/deadline
/// policy. A completed head or rejection consumes this parsing episode.
pub struct ResponseHeadBuffer {
    bytes: Zeroizing<Vec<u8>>,
    finished: bool,
}

impl Default for ResponseHeadBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ResponseHeadBuffer {
    pub fn new() -> Self {
        Self {
            bytes: Zeroizing::new(Vec::with_capacity(MAX_RESPONSE_HEAD)),
            finished: false,
        }
    }

    /// Copy only the header prefix into the reserved buffer. On completion,
    /// return the current chunk's body suffix in its own erasing allocation.
    pub fn push(&mut self, input: &[u8]) -> Result<Option<ResponseStart>, Error> {
        if self.finished {
            return Err(Error::InvalidHeader);
        }
        self.finished = true;
        for (offset, byte) in input.iter().enumerate() {
            if self.bytes.len() == MAX_RESPONSE_HEAD {
                return Err(Error::HeadTooLarge);
            }
            self.bytes.push(*byte);
            if self.bytes.ends_with(b"\r\n\r\n") {
                let (head, _) = ResponseHead::parse(&self.bytes)?;
                let suffix = &input[offset + 1..];
                let mut body = Zeroizing::new(Vec::with_capacity(suffix.len()));
                body.extend_from_slice(suffix);
                return Ok(Some((head, body)));
            }
        }
        if self.bytes.len() == MAX_RESPONSE_HEAD {
            return Err(Error::HeadTooLarge);
        }
        self.finished = false;
        Ok(None)
    }
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
/// each call returns its own erasing output, reserved before decoding. Framing
/// scratch erases on consumption/Drop. Rejection fences further decoding.
pub struct ChunkedDecoder {
    state: ChunkState,
    line: Zeroizing<[u8; 128]>,
    line_len: usize,
    failed: bool,
}

impl Default for ChunkedDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ChunkedDecoder {
    pub fn new() -> Self {
        Self {
            state: ChunkState::Size,
            line: Zeroizing::new([0; 128]),
            line_len: 0,
            failed: false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.state == ChunkState::Complete
    }

    pub fn decode(&mut self, input: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        if self.failed {
            return Err(Error::InvalidChunk);
        }
        self.failed = true;
        // Framing removal cannot produce more bytes than this input chunk.
        let mut output = Zeroizing::new(Vec::with_capacity(input.len()));
        self.decode_inner(input, &mut output)?;
        self.failed = false;
        Ok(output)
    }

    fn decode_inner(&mut self, mut input: &[u8], output: &mut Vec<u8>) -> Result<(), Error> {
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
                            self.line.zeroize();
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
                        self.line.zeroize();
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
                        self.line.zeroize();
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
            output.extend_from_slice(&decoder.decode(part).unwrap());
        }
        assert!(decoder.is_complete());
        assert_eq!(output, b"Wikipedia");
    }

    #[test]
    fn chunk_framing_scratch_erases_after_consumption() {
        let mut decoder = ChunkedDecoder::new();
        assert!(decoder.decode(b"8;ZZZZZZZZ\r\n").unwrap().is_empty());
        assert!(decoder.line.iter().all(|byte| *byte == 0));
        assert_eq!(decoder.decode(b"ZZZZZZZZ\r\n").unwrap().as_slice(), b"ZZZZZZZZ");
        assert!(decoder.line.iter().all(|byte| *byte == 0));
        assert!(decoder.decode(b"0\r\nX-Public: ZZZZZZZZ\r\n\r\n").unwrap().is_empty());
        assert!(decoder.is_complete());
        assert!(decoder.line.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn rejected_chunk_scratch_erases_before_retained_storage_release() {
        use core::{
            mem::MaybeUninit,
            ptr,
        };

        let mut storage = MaybeUninit::new(ChunkedDecoder::new());
        let decoder = storage.as_mut_ptr();
        // SAFETY: the slot contains one initialized owner. Capture only its
        // initialized line bytes, destroy it once, then inspect those bytes
        // while the MaybeUninit slot still retains the backing storage.
        unsafe {
            assert_eq!((*decoder).decode(b"ZZZZZZZZ\r\n").err(), Some(Error::InvalidChunk));
            let line = (*decoder).line.as_ptr();
            assert!((0..8).all(|i| ptr::read_volatile(line.add(i)) == b'Z'));
            ptr::drop_in_place(decoder);
            assert!((0..8).all(|i| ptr::read_volatile(line.add(i)) == 0));
        }
    }
}
