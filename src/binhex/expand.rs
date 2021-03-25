use std::io::{BufRead, BufReader};
use std::{cmp, io};

const RLE_ESCAPE: u8 = 0x90;
const CANCEL_ESCAPE: u8 = 0x00;

#[derive(Debug)]
enum State {
    // Looking for new bytes to copy, possibly with an RLE-expandable byte
    Scan(Option<u8>),

    // Reached an escape, possibly with an RLE-expandable byte
    Escape(Option<u8>),

    // Performing RLE expansion of the given byte for the given remaining run length
    Expand(u8, usize),

    // We've successfully drained the source buffer
    Done,
}

#[derive(Debug)]
enum Event {
    CopiedBytes(usize, u8),
    FoundEscape,
    FoundRunLength(usize),
    SourceEmpty,
}

impl State {
    fn advance(&self, event: Event) -> io::Result<Self> {
        match (self, event) {
            (State::Scan(_), Event::CopiedBytes(_, last_byte)) => Ok(State::Scan(Some(last_byte))),
            (State::Scan(expandable_byte), Event::FoundEscape) => {
                Ok(State::Escape(*expandable_byte))
            }
            (State::Scan(_), Event::SourceEmpty) => Ok(State::Done),
            (State::Escape(_), Event::CopiedBytes(_, last_byte)) => {
                Ok(State::Scan(Some(last_byte)))
            }
            (State::Escape(Some(expandable_byte)), Event::FoundRunLength(run_length)) => {
                Ok(State::Expand(*expandable_byte, run_length))
            }
            (State::Expand(byte, run_length), Event::CopiedBytes(bytes_copied, _)) => {
                if bytes_copied < *run_length {
                    Ok(State::Expand(*byte, run_length - bytes_copied))
                } else {
                    Ok(State::Scan(None))
                }
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Illegal state transition",
            )),
        }
    }
}

pub struct BinHexExpander<R: io::Read> {
    source: BufReader<R>,
    state: State,
}

impl<R: io::Read> BinHexExpander<R> {
    pub fn new(source: R) -> Self {
        BinHexExpander {
            source: BufReader::new(source),
            state: State::Scan(None),
        }
    }
}

impl<R: io::Read> io::Read for BinHexExpander<R> {
    fn read(&mut self, dest: &mut [u8]) -> io::Result<usize> {
        let mut bytes_copied = 0;

        loop {
            let buf = match self.source.fill_buf() {
                Ok(buf) => buf,
                Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };

            let event = match self.state {
                State::Scan(_) => {
                    if buf.is_empty() {
                        Event::SourceEmpty
                    } else {
                        let capacity = cmp::min(buf.len(), dest.len() - bytes_copied);

                        debug_assert!(capacity > 0);

                        match memchr::memchr(RLE_ESCAPE, &buf[..capacity]) {
                            Some(0) => {
                                self.source.consume(1);

                                Event::FoundEscape
                            }
                            Some(pos) => {
                                dest[bytes_copied..bytes_copied + pos].copy_from_slice(&buf[..pos]);

                                let last_byte = buf[pos - 1];

                                bytes_copied += pos;
                                self.source.consume(pos);

                                Event::CopiedBytes(pos, last_byte)
                            }
                            None => {
                                dest[bytes_copied..bytes_copied + capacity]
                                    .copy_from_slice(&buf[..capacity]);

                                let last_byte = buf[capacity - 1];

                                bytes_copied += capacity;
                                self.source.consume(capacity);

                                Event::CopiedBytes(capacity, last_byte)
                            }
                        }
                    }
                }
                State::Escape(_) => {
                    if buf.is_empty() {
                        Event::SourceEmpty
                    } else {
                        match buf[0] {
                            CANCEL_ESCAPE => {
                                dest[bytes_copied] = RLE_ESCAPE;

                                bytes_copied += 1;
                                self.source.consume(1);

                                Event::CopiedBytes(1, RLE_ESCAPE)
                            }
                            _ => {
                                let run_length = buf[0] as usize;
                                self.source.consume(1);

                                // We subtract one because we've already copied one instance of the
                                // byte to be expanded
                                Event::FoundRunLength(run_length - 1)
                            }
                        }
                    }
                }
                State::Expand(byte, run_length) => {
                    let capacity = cmp::min(run_length, dest.len() - bytes_copied);

                    dest[bytes_copied..bytes_copied + capacity].fill(byte);
                    bytes_copied += capacity;

                    Event::CopiedBytes(capacity, byte)
                }
                State::Done => {
                    return Ok(bytes_copied);
                }
            };

            self.state = self.state.advance(event)?;

            if bytes_copied == dest.len() {
                return Ok(bytes_copied);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io;
    use std::io::Read;

    #[test]
    fn expand_no_escapes() {
        let mut cursor =
            io::Cursor::new([0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 4];

        assert_eq!(4, expander.read(&mut buf).unwrap());
        assert_eq!(buf, [0, 1, 2, 3]);

        assert_eq!(4, expander.read(&mut buf).unwrap());
        assert_eq!(buf, [4, 5, 6, 7]);

        assert_eq!(2, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..2], [8, 9]);
    }

    #[test]
    fn expand_cancelled_escape_at_end() {
        let mut cursor = io::Cursor::new([0x2b, 0x90, 0x00]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 4];

        assert_eq!(2, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..2], [0x2b, 0x90]);
    }

    #[test]
    fn expand_cancelled_escape_in_stream() {
        let mut cursor = io::Cursor::new([0x2b, 0x90, 0x00, 0x14]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 4];

        assert_eq!(3, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..3], [0x2b, 0x90, 0x14]);
    }

    #[test]
    fn expand_rle_at_end() {
        let mut cursor = io::Cursor::new([0xff, 0x90, 0x04]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 8];

        assert_eq!(4, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..4], [0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn expand_rle_multiple_reads() {
        let mut cursor = io::Cursor::new([0xff, 0x90, 0x04]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 2];

        assert_eq!(2, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..2], [0xff, 0xff]);

        assert_eq!(2, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..2], [0xff, 0xff]);

        assert_eq!(0, expander.read(&mut buf).unwrap());
    }

    #[test]
    fn expand_rle_in_stream() {
        let mut cursor = io::Cursor::new([0xff, 0x90, 0x04, 0x2b]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 8];

        assert_eq!(5, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..5], [0xff, 0xff, 0xff, 0xff, 0x2b]);
    }

    #[test]
    fn expand_cancelled_escape_rle() {
        let mut cursor = io::Cursor::new([0x2b, 0x90, 0x00, 0x90, 0x05]);
        let mut expander = BinHexExpander::new(&mut cursor);

        let mut buf = [0; 8];

        assert_eq!(6, expander.read(&mut buf).unwrap());
        assert_eq!(buf[0..6], [0x2b, 0x90, 0x90, 0x90, 0x90, 0x90]);
    }
}
