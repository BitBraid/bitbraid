//! Reading one HTTP/1.1 answer off a connection: its head, then its body
//! by whichever of the three ways the head says how long it is.
//!
//! Everything read is counted: a head of more than [`HEAD_LIMIT`] bytes, a
//! body larger than the caller takes, a chunk size that is no number, a
//! body that ends before its length: each is an error, never a guess.
//!
//! A body is kept whole in the answer, or, for a caller that streams it
//! ([`read_response_to`]), written out piece by piece as it comes when the
//! status says it is what was asked for: a package of tens of megabytes is
//! never held in memory.
//!
//! Sources: RFC 9112 §2 (message format), §4 (status line), §6 (message
//! body: `Transfer-Encoding`, `Content-Length`, the order they are looked
//! at in §6.3), §7.1 (chunked transfer coding); RFC 9110 §15 (which
//! statuses have no body); the documentation of `httparse`.

use std::io;

use super::{FetchError, KEPT_HEADERS, Response};

/// Longest head read, in bytes.
pub(crate) const HEAD_LIMIT: usize = 16 * 1024;
/// Most header lines of one head.
const MAX_HEADERS: usize = 64;
/// Most answers that only say "wait" (1xx) before the real one.
const MAX_INTERIM: usize = 4;
/// Longest line of the chunked coding: a size, and whatever follows it.
const CHUNK_LINE: usize = 256;
/// Most bytes of trailer lines after the last chunk.
const TRAILER_LIMIT: usize = 8 * 1024;
/// How much is asked of the connection at a time.
const READ: usize = 8 * 1024;

/// Where the bytes come from: a connection read against a clock, or a
/// slice in a test. Nothing read means the other side is done.
pub(crate) trait Source {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, FetchError>;
}

/// The connection with what was read ahead of what was used.
struct Buffered<'a, S: Source> {
    source: &'a mut S,
    held: Vec<u8>,
    at: usize,
    ended: bool,
}

impl<'a, S: Source> Buffered<'a, S> {
    fn new(source: &'a mut S) -> Self {
        Self {
            source,
            held: Vec::new(),
            at: 0,
            ended: false,
        }
    }

    /// What is read and not yet used.
    fn ahead(&self) -> &[u8] {
        self.held.get(self.at..).unwrap_or_default()
    }

    fn used(&mut self, count: usize) {
        self.at = self.at.saturating_add(count).min(self.held.len());
    }

    /// Read more. False when the other side is done.
    fn more(&mut self) -> Result<bool, FetchError> {
        if self.ended {
            return Ok(false);
        }
        if self.at > 0 {
            self.held.drain(..self.at.min(self.held.len()));
            self.at = 0;
        }
        let before = self.held.len();
        self.held.resize(before.saturating_add(READ), 0);
        let read = match self.held.get_mut(before..) {
            Some(space) => self.source.read(space),
            None => Ok(0),
        };
        let read = match read {
            Ok(read) => read.min(READ),
            Err(err) => {
                self.held.truncate(before);
                return Err(err);
            }
        };
        self.held.truncate(before.saturating_add(read));
        if read == 0 {
            self.ended = true;
        }
        Ok(read > 0)
    }

    /// One line without its end, of at most `limit` bytes.
    fn line(&mut self, limit: usize) -> Result<Vec<u8>, FetchError> {
        loop {
            if let Some(end) = self.ahead().iter().position(|byte| *byte == b'\n') {
                if end > limit {
                    return Err(FetchError::Malformed(say!("http-line-too-long")));
                }
                let mut line = self.ahead().get(..end).unwrap_or_default().to_vec();
                self.used(end.saturating_add(1));
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(line);
            }
            if self.ahead().len() > limit {
                return Err(FetchError::Malformed(say!("http-line-too-long")));
            }
            if !self.more()? {
                return Err(FetchError::Malformed(say!("http-body-ends-early")));
            }
        }
    }

    /// Exactly `count` bytes onto `into`.
    fn take(&mut self, count: usize, into: &mut dyn io::Write) -> Result<(), FetchError> {
        let mut left = count;
        while left > 0 {
            if self.ahead().is_empty() && !self.more()? {
                return Err(FetchError::Malformed(say!("http-body-ends-early")));
            }
            let part = self.ahead().len().min(left);
            into.write_all(self.ahead().get(..part).unwrap_or_default())
                .map_err(|err| FetchError::Stored(err.to_string()))?;
            self.used(part);
            left = left.saturating_sub(part);
        }
        Ok(())
    }
}

/// How the head says the body's length is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    /// There is no body.
    Nothing,
    /// So many bytes.
    Length(usize),
    /// In chunks, each with its size before it.
    Chunked,
    /// Until the connection closes.
    UntilClose,
}

/// What of a head is kept.
struct Head {
    status: u16,
    location: Option<String>,
    content_type: Option<String>,
    /// The lines of [`KEPT_HEADERS`], names in lower case.
    headers: Vec<(String, String)>,
    framing: Framing,
}

/// Keep a header line when it is one of [`KEPT_HEADERS`]; one given again
/// is joined to the first with `, `, as RFC 9110 §5.3 allows.
fn keep(headers: &mut Vec<(String, String)>, name: &str, value: &[u8]) {
    let name = name.to_ascii_lowercase();
    if !KEPT_HEADERS.contains(&name.as_str()) {
        return;
    }
    let Some(value) = text(value) else {
        return;
    };
    match headers.iter_mut().find(|(known, _)| *known == name) {
        Some((_, before)) => {
            before.push_str(", ");
            before.push_str(&value);
        }
        None => headers.push((name, value)),
    }
}

fn text(value: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(value).ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Read the head in `bytes` when it is whole: what it says and how many
/// bytes it took.
fn parse_head(bytes: &[u8], limit: usize) -> Result<Option<(Head, usize)>, FetchError> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut parsed = httparse::Response::new(&mut headers);
    let used = match parsed.parse(bytes) {
        Ok(httparse::Status::Complete(used)) => used,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(_) => return Err(FetchError::Malformed(say!("http-head"))),
    };
    let status = parsed
        .code
        .ok_or(FetchError::Malformed(say!("http-head")))?;
    if !(100..=599).contains(&status) {
        return Err(FetchError::Malformed(say!("http-status")));
    }
    let mut location = None;
    let mut content_type = None;
    let mut length: Option<u64> = None;
    let mut codings: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    for header in parsed.headers.iter() {
        let name = header.name;
        keep(&mut kept, name, header.value);
        if name.eq_ignore_ascii_case("location") {
            location = text(header.value);
        } else if name.eq_ignore_ascii_case("content-type") {
            content_type = text(header.value);
        } else if name.eq_ignore_ascii_case("content-length") {
            // Said twice it has to be the same number both times.
            let value = std::str::from_utf8(header.value)
                .map_err(|_| FetchError::Malformed(say!("http-length")))?;
            for part in value.split(',') {
                let part = part.trim();
                if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(FetchError::Malformed(say!("http-length")));
                }
                let number = part
                    .parse::<u64>()
                    .map_err(|_| FetchError::Malformed(say!("http-length")))?;
                if length.is_some_and(|before| before != number) {
                    return Err(FetchError::Malformed(say!("http-length")));
                }
                length = Some(number);
            }
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            let value = std::str::from_utf8(header.value)
                .map_err(|_| FetchError::Malformed(say!("http-transfer-coding")))?;
            codings.extend(
                value
                    .split(',')
                    .map(|coding| coding.trim().to_ascii_lowercase())
                    .filter(|coding| !coding.is_empty()),
            );
        } else if name.eq_ignore_ascii_case("content-encoding") {
            // Asked for as it is, and nothing here unpacks anything.
            let plain =
                text(header.value).is_none_or(|coding| coding.eq_ignore_ascii_case("identity"));
            if !plain {
                return Err(FetchError::Malformed(say!("http-content-coding")));
            }
        }
    }
    let framing = if (100..200).contains(&status) || status == 204 || status == 304 {
        Framing::Nothing
    } else if !codings.is_empty() {
        if codings.len() != 1 || codings.first().map(String::as_str) != Some("chunked") {
            return Err(FetchError::Malformed(say!("http-transfer-coding")));
        }
        Framing::Chunked
    } else if let Some(length) = length {
        match usize::try_from(length) {
            Ok(length) if length <= limit => Framing::Length(length),
            _ => return Err(FetchError::TooLarge),
        }
    } else {
        Framing::UntilClose
    };
    Ok(Some((
        Head {
            status,
            location,
            content_type,
            headers: kept,
            framing,
        },
        used,
    )))
}

fn read_head<S: Source>(input: &mut Buffered<'_, S>, limit: usize) -> Result<Head, FetchError> {
    // A head can only be whole where a line ends: look again when one did.
    let mut looked = 0usize;
    loop {
        let ahead = input.ahead();
        let news = ahead.get(looked..).unwrap_or_default();
        if news.contains(&b'\n')
            && let Some((head, used)) = parse_head(ahead, limit)?
        {
            input.used(used);
            return Ok(head);
        }
        looked = ahead.len();
        if looked > HEAD_LIMIT {
            return Err(FetchError::Malformed(say!("http-head-too-long")));
        }
        // `more` moves what is ahead to the front: nothing is used yet
        // while a head is being read, so the count stays right.
        if !input.more()? {
            return Err(FetchError::Malformed(if looked == 0 {
                say!("http-nothing-came-back")
            } else {
                say!("http-head-ends-early")
            }));
        }
    }
}

fn read_chunked<S: Source>(
    input: &mut Buffered<'_, S>,
    limit: usize,
    body: &mut dyn io::Write,
) -> Result<(), FetchError> {
    let mut taken = 0usize;
    loop {
        let line = input.line(CHUNK_LINE)?;
        let line = std::str::from_utf8(&line)
            .map_err(|_| FetchError::Malformed(say!("http-chunk-size")))?;
        // What follows a semicolon is an extension nobody here reads.
        let digits = line.split(';').next().unwrap_or_default().trim();
        if digits.is_empty() || digits.len() > 16 || !digits.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(FetchError::Malformed(say!("http-chunk-size")));
        }
        let size = u64::from_str_radix(digits, 16)
            .map_err(|_| FetchError::Malformed(say!("http-chunk-size")))?;
        if size == 0 {
            break;
        }
        let size = usize::try_from(size).map_err(|_| FetchError::TooLarge)?;
        if size > limit.saturating_sub(taken) {
            return Err(FetchError::TooLarge);
        }
        input.take(size, body)?;
        taken = taken.saturating_add(size);
        if !input.line(CHUNK_LINE)?.is_empty() {
            return Err(FetchError::Malformed(say!("http-chunk-end")));
        }
    }
    // Trailer lines, to the empty one; none is of use here.
    let mut trailers = 0usize;
    loop {
        let line = input.line(CHUNK_LINE)?;
        if line.is_empty() {
            return Ok(());
        }
        trailers = trailers.saturating_add(line.len());
        if trailers > TRAILER_LIMIT {
            return Err(FetchError::Malformed(say!("http-trailer-too-long")));
        }
    }
}

fn read_until_close<S: Source>(
    input: &mut Buffered<'_, S>,
    limit: usize,
    body: &mut dyn io::Write,
) -> Result<(), FetchError> {
    let mut taken = 0usize;
    loop {
        let ahead = input.ahead().len();
        if ahead > limit.saturating_sub(taken) {
            return Err(FetchError::TooLarge);
        }
        input.take(ahead, body)?;
        taken = taken.saturating_add(ahead);
        if !input.more()? {
            return Ok(());
        }
    }
}

/// Read a body framed as the head says, of at most `limit` bytes, onto
/// `body`.
fn read_body<S: Source>(
    input: &mut Buffered<'_, S>,
    framing: Framing,
    limit: usize,
    body: &mut dyn io::Write,
) -> Result<(), FetchError> {
    match framing {
        Framing::Nothing => Ok(()),
        Framing::Length(length) if length > limit => Err(FetchError::TooLarge),
        Framing::Length(length) => input.take(length, body),
        Framing::Chunked => read_chunked(input, limit, body),
        Framing::UntilClose => read_until_close(input, limit, body),
    }
}

/// Most bytes of the body of an answer that is not what was asked for (a
/// redirect, an error) that a streaming read keeps.
pub(crate) const KEPT_OTHERWISE: usize = 64 * 1024;

/// Read one answer whole. A body of more than `limit` bytes is refused:
/// as soon as the head says so, or as soon as that much has come.
pub(crate) fn read_response<S: Source>(
    source: &mut S,
    limit: usize,
) -> Result<Response, FetchError> {
    read_answer(source, limit, None)
}

/// Read one answer, writing its body to `into` as it comes when the status
/// says it is what was asked for (200 to 299), up to `limit` bytes; the
/// body of another answer is kept in it, up to [`KEPT_OTHERWISE`] bytes.
/// An answer whose body was written out is handed back without one.
pub(crate) fn read_response_to<S: Source>(
    source: &mut S,
    limit: usize,
    into: &mut dyn io::Write,
) -> Result<Response, FetchError> {
    read_answer(source, limit, Some(into))
}

fn read_answer<S: Source>(
    source: &mut S,
    limit: usize,
    into: Option<&mut dyn io::Write>,
) -> Result<Response, FetchError> {
    let mut input = Buffered::new(source);
    let mut interim = 0usize;
    let head = loop {
        let head = read_head(&mut input, limit)?;
        // "Wait" is not an answer: the one after it is. A change of
        // protocol was not asked for.
        if !(100..200).contains(&head.status) {
            break head;
        }
        interim = interim.saturating_add(1);
        if head.status == 101 || interim > MAX_INTERIM {
            return Err(FetchError::Malformed(say!("http-status")));
        }
    };
    let mut body = Vec::new();
    let wanted = (200..300).contains(&head.status);
    match into {
        Some(into) if wanted => read_body(&mut input, head.framing, limit, into)?,
        Some(_) => read_body(
            &mut input,
            head.framing,
            limit.min(KEPT_OTHERWISE),
            &mut body,
        )?,
        None => {
            if let Framing::Length(length) = head.framing {
                body.reserve(length.min(READ));
            }
            read_body(&mut input, head.framing, limit, &mut body)?;
        }
    }
    Ok(Response {
        status: head.status,
        location: head.location,
        content_type: head.content_type,
        headers: head.headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        FetchError, HEAD_LIMIT, KEPT_OTHERWISE, Response, Source, read_response, read_response_to,
    };

    /// Bytes handed out a few at a time, as a connection does.
    struct Drip<'a> {
        bytes: &'a [u8],
        at: usize,
        step: usize,
        /// What the connection does when the bytes are used up, where it
        /// is not simply closed.
        then: Option<FetchError>,
    }

    impl Source for Drip<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> Result<usize, FetchError> {
            let left = &self.bytes[self.at..];
            if left.is_empty() {
                return match &self.then {
                    Some(err) => Err(err.clone()),
                    None => Ok(0),
                };
            }
            let count = left.len().min(self.step).min(buffer.len());
            buffer[..count].copy_from_slice(&left[..count]);
            self.at += count;
            Ok(count)
        }
    }

    fn read(bytes: &[u8], limit: usize) -> Result<Response, FetchError> {
        // The same whatever the pieces it arrives in.
        let mut answers = Vec::new();
        for step in [1, 2, 3, 7, 4096, usize::MAX] {
            let mut source = Drip {
                bytes,
                at: 0,
                step,
                then: None,
            };
            answers.push(read_response(&mut source, limit));
        }
        let first = answers[0].clone();
        for answer in &answers {
            assert_eq!(*answer, first);
        }
        first
    }

    #[test]
    fn a_body_of_a_given_length_is_read() {
        let answer = read(
            b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5\r\n\r\nhelloEXTRA",
            100,
        )
        .unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.content_type.as_deref(), Some("image/png"));
        assert_eq!(answer.location, None);
        assert_eq!(answer.body, b"hello");
    }

    #[test]
    fn a_body_in_chunks_is_put_together() {
        let answer = read(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              4\r\nWiki\r\n5;ext=1\r\npedia\r\nE\r\n in\r\n\r\nchunks.\r\n0\r\nX-After: 1\r\n\r\n",
            100,
        )
        .unwrap();
        assert_eq!(answer.body, b"Wikipedia in\r\n\r\nchunks.");
    }

    #[test]
    fn a_body_without_a_length_lasts_until_the_connection_closes() {
        let answer = read(b"HTTP/1.0 200 OK\r\n\r\nall of it", 100).unwrap();
        assert_eq!(answer.body, b"all of it");
        let empty = read(b"HTTP/1.1 404 Not Found\r\n\r\n", 100).unwrap();
        assert_eq!((empty.status, empty.body.len()), (404, 0));
    }

    #[test]
    fn the_header_lines_that_matter_are_kept_and_no_others() {
        let answer = read(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nETag: W/\"abc\"\r\n\
              Link: <https://api.example.org/x?page=2>; rel=\"next\"\r\n\
              link: <https://api.example.org/x?page=9>; rel=\"last\"\r\n\
              X-RateLimit-Remaining: 4999\r\nX-RateLimit-Reset: 1700000000\r\n\
              RateLimit-Remaining: 1999\r\nRateLimit-Reset: 1700000001\r\n\
              Retry-After: 60\r\nX-OAuth-Scopes: repo, read:org\r\nX-Next-Page: 3\r\n\
              Set-Cookie: session=secret\r\nX-Other: ignored\r\n\r\nok",
            100,
        )
        .unwrap();
        assert_eq!(answer.header("etag"), Some("W/\"abc\""));
        assert_eq!(
            answer.header("Link"),
            Some(
                "<https://api.example.org/x?page=2>; rel=\"next\", \
                 <https://api.example.org/x?page=9>; rel=\"last\""
            )
        );
        assert_eq!(answer.header("x-ratelimit-remaining"), Some("4999"));
        assert_eq!(answer.header("X-RateLimit-Reset"), Some("1700000000"));
        assert_eq!(answer.header("ratelimit-remaining"), Some("1999"));
        assert_eq!(answer.header("ratelimit-reset"), Some("1700000001"));
        assert_eq!(answer.header("retry-after"), Some("60"));
        assert_eq!(answer.header("x-oauth-scopes"), Some("repo, read:org"));
        assert_eq!(answer.header("x-next-page"), Some("3"));
        assert_eq!(answer.header("set-cookie"), None);
        assert_eq!(answer.header("x-other"), None);
        assert!(!format!("{answer:?}").contains("secret"));
    }

    #[test]
    fn a_redirect_names_where_to_look() {
        let answer = read(
            b"HTTP/1.1 302 Found\r\nLocation:  /elsewhere \r\nContent-Length: 0\r\n\r\n",
            100,
        )
        .unwrap();
        assert_eq!(answer.status, 302);
        assert_eq!(answer.location.as_deref(), Some("/elsewhere"));
    }

    #[test]
    fn statuses_without_a_body_have_none() {
        let answer = read(
            b"HTTP/1.1 304 Not Modified\r\nContent-Length: 10\r\n\r\n",
            100,
        )
        .unwrap();
        assert_eq!((answer.status, answer.body.len()), (304, 0));
        let answer = read(b"HTTP/1.1 204 No Content\r\n\r\n", 100).unwrap();
        assert_eq!(answer.status, 204);
    }

    #[test]
    fn an_answer_that_says_wait_is_passed_over() {
        let answer = read(
            b"HTTP/1.1 103 Early Hints\r\nLink: </x>\r\n\r\n\
              HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            100,
        )
        .unwrap();
        assert_eq!((answer.status, answer.body.as_slice()), (200, &b"ok"[..]));
        let many = b"HTTP/1.1 100 Continue\r\n\r\n".repeat(20);
        assert_eq!(read(&many, 100), Err(FetchError::Malformed("the status")));
        assert_eq!(
            read(b"HTTP/1.1 101 Switching Protocols\r\n\r\n", 100),
            Err(FetchError::Malformed("the status"))
        );
    }

    #[test]
    fn more_than_is_taken_is_refused() {
        // Said in the head: refused before a byte of the body is read.
        assert_eq!(
            read(b"HTTP/1.1 200 OK\r\nContent-Length: 101\r\n\r\n", 100),
            Err(FetchError::TooLarge)
        );
        assert_eq!(
            read(
                b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999\r\n\r\n",
                100
            ),
            Err(FetchError::Malformed("the length"))
        );
        // Not said: refused when that much has come.
        let mut endless = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        endless.extend(std::iter::repeat_n(b'x', 101));
        assert_eq!(read(&endless, 100), Err(FetchError::TooLarge));
        // In chunks: refused at the chunk that goes over.
        let mut chunks = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3c\r\n".to_vec();
        chunks.extend(std::iter::repeat_n(b'x', 0x3c));
        chunks.extend_from_slice(b"\r\n29\r\n");
        assert_eq!(read(&chunks, 100), Err(FetchError::TooLarge));
        // And taken when it is exactly as much.
        let mut fits = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3c\r\n".to_vec();
        fits.extend(std::iter::repeat_n(b'x', 0x3c));
        fits.extend_from_slice(b"\r\n28\r\n");
        fits.extend(std::iter::repeat_n(b'y', 0x28));
        fits.extend_from_slice(b"\r\n0\r\n\r\n");
        assert_eq!(read(&fits, 100).unwrap().body.len(), 100);
        assert_eq!(
            read(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\n",
                100
            ),
            Err(FetchError::TooLarge)
        );
        // Exactly as much as is taken is taken.
        let mut full = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        full.extend(std::iter::repeat_n(b'x', 100));
        assert_eq!(read(&full, 100).unwrap().body.len(), 100);
    }

    #[test]
    fn a_body_that_ends_early_is_an_error() {
        assert_eq!(
            read(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort", 100),
            Err(FetchError::Malformed("the body ends early"))
        );
        assert_eq!(
            read(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nab",
                100
            ),
            Err(FetchError::Malformed("the body ends early"))
        );
        assert_eq!(
            read(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n0\r\n",
                100
            ),
            Err(FetchError::Malformed("the body ends early"))
        );
    }

    #[test]
    fn what_is_not_http_is_refused() {
        for (bytes, why) in [
            (&b""[..], "nothing came back"),
            (b"HTTP/1.1 200", "the head ends early"),
            (b"HTTP/1.1 200 OK\r\nContent-Le", "the head ends early"),
            (b"\x00\x01\x02\x03\n\n\n", "the head"),
            (b"SSH-2.0-OpenSSH_9.9\r\n", "the head"),
            (b"HTTP/1.1 abc OK\r\n\r\n", "the head"),
            (b"HTTP/1.1 099 Odd\r\n\r\n", "the status"),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
                "the length",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 1e3\r\n\r\n",
                "the length",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\nabcd",
                "the length",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Length: 3, 4\r\n\r\nabcd",
                "the length",
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
                "the transfer coding",
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: identity\r\n\r\n",
                "the transfer coding",
            ),
            (
                b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 1\r\n\r\nx",
                "the content coding",
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
                "a chunk size",
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n-1\r\n",
                "a chunk size",
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\r\n",
                "a chunk size",
            ),
            (
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabXX\r\n0\r\n\r\n",
                "the end of a chunk",
            ),
        ] {
            assert_eq!(
                read(bytes, 100),
                Err(FetchError::Malformed(why)),
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    #[test]
    fn the_same_length_said_twice_is_one_length() {
        let answer = read(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\nok",
            100,
        )
        .unwrap();
        assert_eq!(answer.body, b"ok");
        // Chunked wins over a length, as the standard has it.
        let answer = read(
            b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\nTransfer-Encoding: chunked\r\n\r\n\
              2\r\nok\r\n0\r\n\r\n",
            100,
        )
        .unwrap();
        assert_eq!(answer.body, b"ok");
    }

    #[test]
    fn a_head_without_end_is_given_up() {
        let mut head = b"HTTP/1.1 200 OK\r\n".to_vec();
        while head.len() <= HEAD_LIMIT + 9000 {
            head.extend_from_slice(b"X-Filler: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n");
        }
        // Too many lines for the reader of heads, or too many bytes for
        // this one: whichever is met first, it is refused.
        assert!(matches!(read(&head, 100), Err(FetchError::Malformed(_))));
        let mut line = b"HTTP/1.1 200 OK\r\nX-Filler: ".to_vec();
        line.extend(std::iter::repeat_n(b'a', HEAD_LIMIT * 2));
        assert_eq!(
            read(&line, 100),
            Err(FetchError::Malformed("the head is too long"))
        );
        // Chunk lines and trailers have their limits too.
        let mut chunk = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;".to_vec();
        chunk.extend(std::iter::repeat_n(b'e', 5000));
        assert_eq!(
            read(&chunk, 100),
            Err(FetchError::Malformed("a line is too long"))
        );
        let mut trailer = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n".to_vec();
        for _ in 0..200 {
            trailer.extend_from_slice(
                b"X-T: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n",
            );
        }
        trailer.extend_from_slice(b"\r\n");
        assert_eq!(
            read(&trailer, 100),
            Err(FetchError::Malformed("the trailer is too long"))
        );
    }

    /// Read with the body written out, the same whatever the pieces it
    /// arrives in: the answer and what was written.
    fn streamed(bytes: &[u8], limit: usize) -> Result<(Response, Vec<u8>), FetchError> {
        let mut answers = Vec::new();
        for step in [1, 3, 4096, usize::MAX] {
            let mut source = Drip {
                bytes,
                at: 0,
                step,
                then: None,
            };
            let mut written = Vec::new();
            let answer = read_response_to(&mut source, limit, &mut written);
            answers.push(answer.map(|answer| (answer, written)));
        }
        let first = answers[0].clone();
        for answer in &answers {
            assert_eq!(*answer, first);
        }
        first
    }

    #[test]
    fn a_body_that_was_asked_for_is_written_out_as_it_comes() {
        for bytes in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"[..],
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhe\r\n3\r\nllo\r\n0\r\n\r\n",
            b"HTTP/1.0 200 OK\r\n\r\nhello",
        ] {
            let (answer, written) = streamed(bytes, 5).unwrap();
            assert_eq!((answer.status, answer.body.len()), (200, 0));
            assert_eq!(written, b"hello");
            // The limit holds for what is written as for what is kept.
            assert_eq!(streamed(bytes, 4), Err(FetchError::TooLarge));
        }
        // Anything else is kept and not written: a redirect, an error,
        // bounded however much the caller takes of what it asked for.
        let (answer, written) = streamed(
            b"HTTP/1.1 302 Found\r\nLocation: /there\r\nContent-Length: 4\r\n\r\nmove",
            1 << 30,
        )
        .unwrap();
        assert_eq!((answer.status, answer.body.as_slice()), (302, &b"move"[..]));
        assert_eq!(answer.location.as_deref(), Some("/there"));
        assert!(written.is_empty());
        let mut huge = b"HTTP/1.1 404 Not Found\r\n\r\n".to_vec();
        huge.extend(std::iter::repeat_n(b'x', KEPT_OTHERWISE + 1));
        assert_eq!(streamed(&huge, 1 << 30), Err(FetchError::TooLarge));
        let said = format!(
            "HTTP/1.1 500 Oops\r\nContent-Length: {}\r\n\r\n",
            KEPT_OTHERWISE + 1
        );
        assert_eq!(
            streamed(said.as_bytes(), 1 << 30),
            Err(FetchError::TooLarge)
        );
    }

    #[test]
    fn a_body_that_cannot_be_written_out_ends_the_read() {
        struct Full;
        impl std::io::Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("no space"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut source = Drip {
            bytes: b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello",
            at: 0,
            step: 64,
            then: None,
        };
        assert_eq!(
            read_response_to(&mut source, 100, &mut Full),
            Err(FetchError::Stored("no space".to_owned()))
        );
    }

    #[test]
    fn what_the_connection_says_is_passed_on() {
        for then in [
            FetchError::Timeout,
            FetchError::Cancelled,
            FetchError::Broken("reset".to_owned()),
        ] {
            let mut source = Drip {
                bytes: b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc",
                at: 0,
                step: 5,
                then: Some(then.clone()),
            };
            assert_eq!(read_response(&mut source, 100), Err(then));
        }
    }
}
