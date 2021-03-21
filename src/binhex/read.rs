use std::io::{BufRead, Error, ErrorKind, Read, Result};
use std::cmp::min;

const BANNER: &[u8] = b"(This file must be converted with BinHex";
const DATA_DELIMITER: u8 = b':';

/// A `Read` implementation that extracts BinHex-encoded data from an underlying reader.
///
/// The data produced by an `EncodedBinHexReader` is the still-encoded data contained within a
/// BinHex source (usually a file) stripped of extraneous banners, delimiters, and whitespace.
/// Callers will almost certainly need to pass the data through a BinHex decoder.
struct EncodedBinHexReader<'a, R: 'a + BufRead> {
    source: &'a mut R,

    found_data_start: bool,
    found_data_end: bool,
}

impl<'a, R: BufRead> EncodedBinHexReader<'a, R> {

    pub fn new(source: &'a mut R) -> Self {
        EncodedBinHexReader {
            source,

            found_data_start: false,
            found_data_end: false,
        }
    }

    fn seek_to_banner_end(&mut self) -> Result<()> {
        let mut matched_len = 0;

        while matched_len < BANNER.len() {
            let buf = match self.source.fill_buf() {
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
                Ok(buf) if buf.len() == 0 => {
                    return Err(Error::new(ErrorKind::InvalidData,
                                          "Stream did not contain a BinHex banner"));
                }
                Ok(buf) => buf,
            };

            // Check for a match for however many bytes we still need right at the start of the
            // buffer (facilitates partial match carryover and probably just works 99% of the time
            // for the full banner anyhow).
            let compare_len = min(BANNER.len() - matched_len, buf.len());

            if buf[0..compare_len] == BANNER[matched_len..matched_len + compare_len] {
                matched_len += compare_len;

                // We've either consumed enough of the buffer to complete the match or we've reached
                // the end of the buffer; either way, consume the bytes read up to that point.
                self.source.consume(compare_len);
            } else {
                // We didn't find a match at the start of the buffer for either the full banner or
                // whatever part we were looking for. Search for other possible banner starts
                // instead. This will find EITHER a match that's completely contained within the
                // buffer XOR a partial match at the end of the buffer.
                let match_start = memchr::memchr_iter(BANNER[0], buf)
                    .find(|&start| {
                        if buf.len() - start >= BANNER.len() {
                            buf[start..start + BANNER.len()] == *BANNER
                        } else {
                            buf[start..] == BANNER[0..buf.len() - start]
                        }
                    });

                match match_start {
                    None => {
                        matched_len = 0;

                        let consumed = buf.len();
                        self.source.consume(consumed);
                    }
                    Some(start) => {
                        matched_len = min(BANNER.len(), buf.len() - start);
                        self.source.consume(start + matched_len);
                    }
                }
            }
        }

        Ok(())
    }

    fn seek_to_data_start(&mut self) -> Result<()> {
        while !self.found_data_start {
            let buf = match self.source.fill_buf() {
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
                Ok(buf) if buf.len() == 0 => {
                    return Err(Error::new(ErrorKind::InvalidData,
                                          "Stream did not contain an opening ':' delimiter"));
                }
                Ok(buf) => buf,
            };

            match memchr::memchr(DATA_DELIMITER, buf) {
                None => {
                    let len = buf.len();
                    self.source.consume(len);
                }
                Some(i) => {
                    self.source.consume(i + 1);
                    self.found_data_start = true;
                }
            }
        }

        Ok(())
    }
}

impl<'a, R: BufRead> Read for EncodedBinHexReader<'a, R> {
    fn read(&mut self, dest: &mut [u8]) -> Result<usize> {
        if dest.len() == 0 {
            return Ok(0);
        }

        if !self.found_data_start {
            self.seek_to_banner_end()?;
            self.seek_to_data_start()?;
        }

        let mut bytes_copied = 0;

        loop {
            let buf = match self.source.fill_buf() {
                Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
                Ok(buf) if buf.len() == 0 => {
                    return Err(Error::new(ErrorKind::InvalidData,
                                          "Stream did not contain a closing ':' delimiter"));
                }
                Ok(buf) => buf,
            };

            {
                let leading_whitespace_bytes = buf.iter()
                    .take_while(|b| b" \t\r\n".contains(b))
                    .count();

                if leading_whitespace_bytes > 0 {
                    self.source.consume(leading_whitespace_bytes);
                    continue;
                }
            }

            // We know for sure that the buffer begins with a non-whitespace character; copy a
            // contiguous chain of bytes until the next whitespace character, the end of the source
            // buffer, the end of the BinHex data, or the end of the destination buffer, whichever
            // comes first.
            assert!(buf.len() > 0);

            let capacity = min(buf.len(), dest.len() - bytes_copied);

            let next_whitespace = match (memchr::memchr(b' ', &buf[..capacity]),
                                         memchr::memchr3(b'\t', b'\r', b'\n', &buf[..capacity])) {
                (Some(a), Some(b)) => Some(min(a, b)),
                (a, b) => a.or(b),
            };

            let src_end = match next_whitespace {
                Some(i) => min(i, capacity),
                None => capacity
            };

            let end = match memchr::memchr(DATA_DELIMITER, buf) {
                Some(data_end) if data_end < src_end => {
                    // We'll reach the end of the entire BinHex stream in this iteration
                    self.found_data_end = true;
                    data_end
                }
                _ => src_end
            };

            dest[bytes_copied..bytes_copied + end].copy_from_slice(&buf[..end]);
            self.source.consume(end);

            bytes_copied += end;

            if bytes_copied == dest.len() || self.found_data_end {
                return Ok(bytes_copied);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EncodedBinHexReader;
    use indoc::indoc;
    use std::io::{Cursor, BufReader, ErrorKind, Read};

    #[test]
    fn read() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut buf_reader = BufReader::new(cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        let mut binhex_data = vec![];

        assert_eq!(binhex_reader.read_to_end(&mut binhex_data).unwrap(), 134);
        assert_eq!(binhex_data.as_slice(), br#"$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!YN!8SI!"#);
    }

    #[test]
    fn read_no_banner() {
        let cursor = Cursor::new(indoc! {br#"
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
        });

        let mut buf_reader = BufReader::new(cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        let mut binhex_data = vec![];

        assert_eq!(binhex_reader.read_to_end(&mut binhex_data).map_err(|e| e.kind()),
                   Err(ErrorKind::InvalidData));
    }

    #[test]
    fn read_no_data_end() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!"#
        });

        let mut buf_reader = BufReader::new(cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        let mut binhex_data = vec![];

        assert_eq!(binhex_reader.read_to_end(&mut binhex_data).map_err(|e| e.kind()),
                   Err(ErrorKind::InvalidData));
    }

    #[test]
    fn seek_to_banner_end() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :great success!"#
        });

        let mut buf_reader = BufReader::new(cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        binhex_reader.seek_to_banner_end().unwrap();

        let mut remaining_bytes = vec![];
        buf_reader.read_to_end(&mut remaining_bytes).unwrap();

        assert_eq!(remaining_bytes, b" 4.0)\n:great success!");
    }

    #[test]
    fn seek_to_banner_end_leading_whitespace() {
        // Start with some leading whitespace
        let cursor = Cursor::new(indoc! {br#"
                  (This file must be converted with BinHex 4.0)
            :great success!"#
        });

        let mut buf_reader = BufReader::new(cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        binhex_reader.seek_to_banner_end().unwrap();

        let mut remaining_bytes = vec![];
        buf_reader.read_to_end(&mut remaining_bytes).unwrap();

        assert_eq!(remaining_bytes, b" 4.0)\n:great success!");
    }

    #[test]
    fn seek_to_banner_end_partial_match() {
        // Start with leading whitespace and a really small BufReader capacity to ensure that
        // we're dealing with partial matches across reads
        let cursor = Cursor::new(indoc! {br#"
                  (This file must be converted with BinHex 4.0)
            :great success!"#
        });

        let mut buf_reader = BufReader::with_capacity(7, cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        binhex_reader.seek_to_banner_end().unwrap();

        let mut remaining_bytes = vec![];
        buf_reader.read_to_end(&mut remaining_bytes).unwrap();

        assert_eq!(remaining_bytes, b" 4.0)\n:great success!");
    }

    #[test]
    fn seek_to_banner_end_no_banner() {
        // Start with leading whitespace and a really small BufReader capacity to ensure that
        // we're dealing with partial matches across reads
        let cursor = Cursor::new(br#"This is not a legit BinHex file:"#);

        let mut buf_reader = BufReader::with_capacity(7, cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        assert_eq!(binhex_reader.seek_to_banner_end().map_err(|e| e.kind()),
                   Err(ErrorKind::InvalidData));
    }

    #[test]
    fn seek_to_data_start() {
        let cursor = Cursor::new(indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :great success!"#
        });

        let mut buf_reader = BufReader::new(cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        binhex_reader.seek_to_data_start().unwrap();

        let mut remaining_bytes = vec![];
        buf_reader.read_to_end(&mut remaining_bytes).unwrap();

        assert_eq!(remaining_bytes, b"great success!");
    }

    #[test]
    fn seek_to_data_start_no_delimiter() {
        // Start with leading whitespace and a really small BufReader capacity to ensure that
        // we're dealing with partial matches across reads
        let cursor = Cursor::new(br#"This is not a legit BinHex file"#);

        let mut buf_reader = BufReader::with_capacity(7, cursor);
        let mut binhex_reader = EncodedBinHexReader::new(&mut buf_reader);

        assert_eq!(binhex_reader.seek_to_data_start().map_err(|e| e.kind()),
                   Err(ErrorKind::InvalidData));
    }
}
