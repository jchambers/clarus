use std::cmp;
use std::io::{Error, ErrorKind, Read, Result};

const BANNER: &[u8] = b"(This file must be converted with BinHex";
const DATA_DELIMITER: u8 = b':';

/// A `Read` implementation that extracts BinHex-encoded data from an underlying reader.
///
/// The data produced by an `EncodedBinHexReader` is the still-encoded data contained within a
/// BinHex source (usually a file) stripped of extraneous banners, delimiters, and whitespace.
/// Callers will almost certainly need to pass the data through a BinHex decoder.
pub struct EncodedBinHexReader<R: Read> {
    source: R,
    state: State,
}

impl<R: Read> EncodedBinHexReader<R> {
    pub fn new(source: R) -> Self {
        EncodedBinHexReader {
            source,
            state: State::FindBannerStart,
        }
    }
}

impl<R: Read> Read for EncodedBinHexReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        let mut bytes_copied = 0;

        while bytes_copied == 0 && self.state != State::Done {
            let bytes_read = match self.source.read(buf) {
                Err(e) => return Err(e),
                Ok(0) => return Err(Error::from(ErrorKind::UnexpectedEof)),
                Ok(bytes_read) => bytes_read,
            };

            let mut bytes_consumed = 0;

            while bytes_consumed < bytes_read && self.state != State::Done {
                debug_assert!(!buf[bytes_consumed..bytes_read].is_empty());

                let event = match self.state {
                    State::FindBannerStart => {
                        match memchr::memchr(BANNER[0], &buf[bytes_consumed..bytes_read]) {
                            Some(start) => {
                                bytes_consumed += start + 1;
                                Event::FoundBannerStart
                            }
                            None => {
                                bytes_consumed = bytes_read;
                                Event::ConsumedBytes
                            }
                        }
                    }
                    State::PartialBannerMatch(matched) => {
                        let check_len =
                            cmp::min(bytes_read - bytes_consumed, BANNER.len() - matched);

                        if buf[bytes_consumed..].starts_with(&BANNER[matched..matched + check_len])
                        {
                            bytes_consumed += check_len;
                            Event::MatchedBannerBytes(check_len)
                        } else {
                            Event::ConsumedBytes
                        }
                    }
                    State::FindDataStart => {
                        match memchr::memchr(DATA_DELIMITER, &buf[bytes_consumed..bytes_read]) {
                            Some(pos) => {
                                bytes_consumed += pos + 1;
                                Event::FoundDataStart
                            }
                            None => {
                                bytes_consumed = bytes_read;
                                Event::ConsumedBytes
                            }
                        }
                    }
                    State::ReadData => match next_data_byte(&buf[bytes_consumed..bytes_read]) {
                        Some(start) => {
                            let data_bytes = compact(&mut buf[bytes_consumed + start..bytes_read]);

                            if bytes_consumed + start > 0 {
                                buf.copy_within(
                                    bytes_consumed + start..bytes_consumed + start + data_bytes,
                                    0,
                                );
                            }

                            match memchr::memchr(DATA_DELIMITER, &buf[..data_bytes]) {
                                Some(data_end) => {
                                    bytes_copied += data_end;

                                    Event::FoundDataEnd
                                }
                                None => {
                                    bytes_consumed = bytes_read;
                                    bytes_copied += data_bytes;

                                    Event::ConsumedBytes
                                }
                            }
                        }
                        None => {
                            bytes_consumed = bytes_read;
                            Event::ConsumedBytes
                        }
                    },
                    State::Done => {
                        return Ok(bytes_copied);
                    }
                };

                self.state = self.state.advance(event)?;
            }
        }

        Ok(bytes_copied)
    }
}

/// The internal state of a BinHex reader.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum State {
    /// Looking for the BinHex banner, which may come after an arbitrary amount of human-readable
    /// descriptive text.
    FindBannerStart,

    /// Found a partial match for the BinHex banner at the end of a read buffer; the rest (or some
    /// of the rest) of the banner may be at the beginning of the next read buffer.
    PartialBannerMatch(usize),

    /// Looking for the "data start" byte after a complete BinHex banner.
    FindDataStart,

    /// Copying encoded BinHex data to a destination buffer.
    ReadData,

    /// Found the "data end" byte after a stream of encoded BinHex data.
    Done,
}

impl State {
    fn advance(&self, event: Event) -> Result<Self> {
        match (self, event) {
            (State::FindBannerStart, Event::ConsumedBytes) => Ok(State::FindBannerStart),
            (State::FindBannerStart, Event::FoundBannerStart) => Ok(State::PartialBannerMatch(1)),
            (State::PartialBannerMatch(_), Event::ConsumedBytes) => Ok(State::FindBannerStart),
            (State::PartialBannerMatch(len), Event::MatchedBannerBytes(matched)) => {
                if len + matched == BANNER.len() {
                    Ok(State::FindDataStart)
                } else {
                    Ok(State::PartialBannerMatch(len + matched))
                }
            }
            (State::FindDataStart, Event::ConsumedBytes) => Ok(State::FindDataStart),
            (State::FindDataStart, Event::FoundDataStart) => Ok(State::ReadData),
            (State::ReadData, Event::ConsumedBytes) => Ok(State::ReadData),
            (State::ReadData, Event::FoundDataEnd) => Ok(State::Done),
            _ => panic!("Illegal state transition from {:?} with {:?}", self, event),
        }
    }
}

/// An event produced when reading BinHex data that may alter that reader's state.
#[derive(Copy, Clone, Debug)]
enum Event {
    /// The reader consumed one or more bytes that didn't lead to any state-changing events.
    ConsumedBytes,

    /// The reader encountered the start of a sequence that may be the opening BinHex banner.
    FoundBannerStart,

    /// The reader found a contiguous sequence of bytes that match the next expected bytes in a
    /// BinHex banner.
    MatchedBannerBytes(usize),

    /// The reader located the "data start" byte after the BinHex banner.
    FoundDataStart,

    /// The reader located the "data end" byte at the end of the encoded data stream.
    FoundDataEnd,
}

/// Returns the index of the next whitespace byte in the given slice.
fn next_whitespace(bytes: &[u8]) -> Option<usize> {
    match (
        memchr::memchr(b' ', bytes),
        memchr::memchr3(b'\t', b'\r', b'\n', bytes),
    ) {
        (Some(a), Some(b)) => Some(cmp::min(a, b)),
        (a, b) => a.or(b),
    }
}

/// Returns the index of the next non-whitespace byte in the given slice.
fn next_data_byte(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        None
    } else {
        let leading_whitespace_bytes = bytes.iter().take_while(|b| b" \t\r\n".contains(b)).count();

        if leading_whitespace_bytes == bytes.len() {
            None
        } else {
            Some(leading_whitespace_bytes)
        }
    }
}

/// Removes whitespace from a slice by shifting non-whitespace bytes toward the beginning of the
/// slice and returning the number of non-whitespace bytes in the buffer.
///
/// When this function returns, the given slice _may_ contain "junk" data after the end of the
/// compacted non-whitespace bytes. Callers should ignore any bytes beyond the returned number of
/// non-whitespace bytes.
fn compact(bytes: &mut [u8]) -> usize {
    let mut scan = 0;
    let mut bytes_written = 0;

    while let Some(start_offset) = next_data_byte(&bytes[scan..]) {
        let chunk_start = scan + start_offset;

        let chunk_end = next_whitespace(&bytes[chunk_start..])
            .map(|whitespace_offset| chunk_start + whitespace_offset)
            .unwrap_or_else(|| bytes.len());

        bytes.copy_within(chunk_start..chunk_end, bytes_written);

        let len = chunk_end - chunk_start;

        bytes_written += len;
        scan += start_offset + len;
    }

    bytes_written
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use std::io::{Cursor, ErrorKind, Read};

    #[test]
    fn read() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut binhex_reader = EncodedBinHexReader::new(cursor);
        let mut binhex_data = vec![];

        assert_eq!(binhex_reader.read_to_end(&mut binhex_data).unwrap(), 134);
        assert_eq!(binhex_data.as_slice(), br#"$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!YN!8SI!"#);
    }

    #[test]
    fn read_large_buffer() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut binhex_reader = EncodedBinHexReader::new(cursor);
        let mut buf = [0; 512];

        let expected = br#"$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!YN!8SI!"#;

        assert_eq!(binhex_reader.read(&mut buf).unwrap(), 134);
        assert_eq!(buf[0..134], expected[0..]);
    }

    #[test]
    fn read_tiny_buffer() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut binhex_reader = EncodedBinHexReader::new(cursor);
        let mut buf = [0; 1];
        let mut accumulated_data = vec![];

        while let Ok(1) = binhex_reader.read(&mut buf) {
            accumulated_data.extend_from_slice(&buf);
        }

        assert_eq!(accumulated_data.len(), 134);
        assert_eq!(accumulated_data.as_slice(), br#"$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!YN!8SI!"#);
    }

    #[test]
    fn read_no_banner() {
        let cursor = Cursor::new(indoc! {br#"
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut binhex_reader = EncodedBinHexReader::new(cursor);
        let mut binhex_data = vec![];

        assert_eq!(
            binhex_reader
                .read_to_end(&mut binhex_data)
                .map_err(|e| e.kind()),
            Err(ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn read_no_data_end() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!"#
        });

        let mut binhex_reader = EncodedBinHexReader::new(cursor);
        let mut binhex_data = vec![];

        assert_eq!(
            binhex_reader
                .read_to_end(&mut binhex_data)
                .map_err(|e| e.kind()),
            Err(ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn read_junk_before_banner() {
        let cursor = Cursor::new(indoc! {br#"
            (((((((((This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut binhex_reader = EncodedBinHexReader::new(cursor);
        let mut binhex_data = vec![];

        assert_eq!(binhex_reader.read_to_end(&mut binhex_data).unwrap(), 134);
        assert_eq!(binhex_data.as_slice(), br#"$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!YN!8SI!"#);
    }

    #[test]
    fn next_whitespace() {
        assert_eq!(None, super::next_whitespace(b""));
        assert_eq!(None, super::next_whitespace(b"No_whitespace_here"));
        assert_eq!(Some(1), super::next_whitespace(b"A string with spaces"));
        assert_eq!(Some(4), super::next_whitespace(b"Some\ttabs"));
        assert_eq!(Some(4), super::next_whitespace(b"Some\rcarriage\rreturns"));
        assert_eq!(Some(8), super::next_whitespace(b"Newlines\neverywhere!"));
        assert_eq!(
            Some(5),
            super::next_whitespace(b"Check out\tthis\rmix\nof whitespace")
        );
    }

    #[test]
    fn next_data_byte() {
        assert_eq!(None, super::next_data_byte(b""));
        assert_eq!(None, super::next_data_byte(b" \r\n\t"));
        assert_eq!(
            Some(4),
            super::next_data_byte(b"    This isn't all whitespace\r\n\t")
        );
    }

    #[test]
    fn compact() {
        let test_strings = [
            "compaction!",
            "    compaction!",
            "compaction!    ",
            "    compaction!    ",
            "c  omp a       ction !",
            "  c  omp a       ction !",
            "c  omp a       ction !    ",
            "  c  omp a   ction !      ",
        ];

        test_strings.iter().for_each(|&string| {
            let mut bytes = Vec::from(string);

            assert_eq!(11, super::compact(bytes.as_mut_slice()));
            assert_eq!(b"compaction!"[..], bytes.as_slice()[..11]);
        });
    }
}
