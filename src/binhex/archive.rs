use std::convert::{TryFrom, TryInto};
use std::error;
use std::fmt::{self, Display, Formatter};
use std::hash::Hasher;
use std::io::{self, Read, Write};
use std::ops::Deref;

use super::expand::BinHexExpander;
use super::read::EncodedBinHexReader;

use crc16::{State, XMODEM};
use lazycell::LazyCell;
use radix64::io::DecodeReader;
use radix64::CustomConfig;

lazy_static::lazy_static! {
    static ref BINHEX_CONFIG: CustomConfig = CustomConfig::with_alphabet(
        r##"!"#$%&'()*+,-012345689@ABCDEFGHIJKLMNPQRSTUVXYZ[`abcdefhijklmpqr"##)
    .no_padding()
    .build()
    .expect("Failed to build BinHex base64 config");
}

pub struct BinHexArchive<R: Read> {
    source: BinHexExpander<DecodeReader<&'static CustomConfig, EncodedBinHexReader<R>>>,
    header: LazyCell<BinHexHeader>,
}

#[derive(Debug)]
pub enum BinHexError {
    IoError(io::Error),
    InvalidHeader,
    InvalidData,
    InvalidChecksum(ArchiveSection, u16, u16),
}

#[derive(Debug)]
pub enum ArchiveSection {
    Header,
    DataFork,
    ResourceFork,
}

#[derive(Clone, Debug)]
struct BinHexHeader {
    name: String,
    file_type: [u8; 4],
    creator: [u8; 4],
    flag: u16,
    data_fork_length: usize,
    resource_fork_length: usize,
}

struct ForkReader<'a, R: Read> {
    source: &'a mut R,
    len: usize,
    bytes_read: usize,
    crc: State<XMODEM>,
}

impl Display for BinHexError {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
        match self {
            BinHexError::IoError(source) => write!(fmt, "IO error: {}", source),
            BinHexError::InvalidHeader => write!(fmt, "Malformed BinHex header"),
            BinHexError::InvalidData => write!(fmt, "Malformed BinHex data"),
            BinHexError::InvalidChecksum(section, provided, calculated) => write!(
                fmt,
                "Invalid checksum; section={:?}, expected={:04x}, calculated={:04x}",
                section, provided, calculated
            ),
        }
    }
}

impl From<io::Error> for BinHexError {
    fn from(error: io::Error) -> Self {
        BinHexError::IoError(error)
    }
}

impl error::Error for BinHexError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            BinHexError::IoError(error) => Some(error),
            _ => None,
        }
    }
}

impl<'a, R: Read> ForkReader<'a, R> {
    fn new(source: &'a mut R, len: usize) -> Self {
        ForkReader {
            source,
            len,
            bytes_read: 0,
            crc: State::<XMODEM>::new(),
        }
    }

    fn checksum(&self) -> u16 {
        debug_assert!(self.bytes_read == self.len);
        self.crc.get()
    }
}

impl<'a, R: Read> Read for ForkReader<'a, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() || self.bytes_read == self.len {
            Ok(0)
        } else {
            let remaining_bytes = self.len - self.bytes_read;

            let bytes_copied = if buf.len() <= remaining_bytes {
                self.source.read(buf)?
            } else {
                self.source.read_exact(&mut buf[..remaining_bytes])?;
                remaining_bytes
            };

            self.crc.write(&buf[..bytes_copied]);
            self.bytes_read += bytes_copied;

            Ok(bytes_copied)
        }
    }
}

impl<R: Read> BinHexArchive<R> {
    pub fn new(source: R) -> Self {
        let reader = EncodedBinHexReader::new(source);
        let decoder = DecodeReader::new(BINHEX_CONFIG.deref(), reader);
        let expander = BinHexExpander::new(decoder);

        BinHexArchive {
            source: expander,
            header: LazyCell::new(),
        }
    }

    fn header(&mut self) -> Result<&BinHexHeader, BinHexError> {
        let source = &mut self.source;

        self.header.try_borrow_with(|| {
            // Headers have a minimum size of 22 bytes (assuming a zero-length name) and a maximum size
            // of 277 bytes (assuming a 255-byte name); to avoid overshooting and eating into the data
            // fork, we read the minimum, check the name length, and extend as needed.
            let mut header_bytes = Vec::with_capacity(277);
            header_bytes.resize(22, 0);

            source.read_exact(header_bytes.as_mut_slice())?;

            let name_length = header_bytes[0] as usize;

            header_bytes.resize(header_bytes.len() + name_length, 0);
            source.read_exact(&mut header_bytes.as_mut_slice()[22..])?;

            BinHexHeader::try_from(header_bytes)
        })
    }

    pub fn filename(&mut self) -> Result<String, BinHexError> {
        self.header().map(|header| header.name.clone())
    }

    pub fn file_type(&mut self) -> Result<[u8; 4], BinHexError> {
        self.header().map(|header| header.file_type)
    }

    pub fn creator(&mut self) -> Result<[u8; 4], BinHexError> {
        self.header().map(|header| header.creator)
    }

    pub fn flags(&mut self) -> Result<u16, BinHexError> {
        self.header().map(|header| header.flag)
    }

    pub fn data_fork_len(&mut self) -> Result<usize, BinHexError> {
        self.header().map(|header| header.data_fork_length)
    }

    pub fn resource_fork_len(&mut self) -> Result<usize, BinHexError> {
        self.header().map(|header| header.resource_fork_length)
    }

    pub fn extract(
        mut self,
        data_writer: &mut impl Write,
        resource_writer: &mut impl Write,
    ) -> Result<(), BinHexError> {
        let (data_fork_length, resource_fork_length) = {
            let header = self.header()?;
            (header.data_fork_length, header.resource_fork_length)
        };

        self.copy_fork(ArchiveSection::DataFork, data_writer, data_fork_length)?;
        self.copy_fork(
            ArchiveSection::ResourceFork,
            resource_writer,
            resource_fork_length,
        )
    }

    fn copy_fork(
        &mut self,
        section: ArchiveSection,
        dest: &mut impl Write,
        len: usize,
    ) -> Result<(), BinHexError> {
        let (bytes_copied, calculated_checksum) = {
            let mut fork_reader = ForkReader::new(&mut self.source, len);
            (io::copy(&mut fork_reader, dest)?, fork_reader.checksum())
        };

        debug_assert!(bytes_copied == len as u64);

        let provided_checksum = {
            let mut checksum_bytes = [0; 2];
            self.source.read_exact(&mut checksum_bytes)?;

            u16::from_be_bytes(checksum_bytes)
        };

        if provided_checksum == calculated_checksum {
            Ok(())
        } else {
            Err(BinHexError::InvalidChecksum(
                section,
                provided_checksum,
                calculated_checksum,
            ))
        }
    }
}

impl TryFrom<Vec<u8>> for BinHexHeader {
    type Error = BinHexError;

    fn try_from(header_bytes: Vec<u8>) -> Result<Self, Self::Error> {
        let (name_length_bytes, remaining_bytes) = header_bytes.split_at(1);
        let name_length = name_length_bytes[0] as usize;

        if header_bytes.len() != name_length + 22 {
            return Err(BinHexError::InvalidHeader);
        }

        let (name_bytes, remaining_bytes) = remaining_bytes.split_at(name_length);
        let (_version_byte, remaining_bytes) = remaining_bytes.split_at(1);
        let (file_type_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (creator_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (flag_bytes, remaining_bytes) = remaining_bytes.split_at(2);
        let (data_fork_length_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (resource_fork_length_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (checksum_bytes, remaining_bytes) = remaining_bytes.split_at(2);

        debug_assert!(remaining_bytes.is_empty());

        let calculated_checksum =
            crc16::State::<crc16::XMODEM>::calculate(&header_bytes[..header_bytes.len() - 2]);
        let provided_checksum = u16::from_be_bytes(checksum_bytes.try_into().unwrap());

        if provided_checksum != calculated_checksum {
            return Err(BinHexError::InvalidChecksum(
                ArchiveSection::Header,
                provided_checksum,
                calculated_checksum,
            ));
        }

        let (name_cow, _, _) = encoding_rs::MACINTOSH.decode(name_bytes);
        let name = name_cow.to_string();
        let file_type: [u8; 4] = TryInto::<[u8; 4]>::try_into(file_type_bytes).unwrap();
        let creator: [u8; 4] = TryInto::<[u8; 4]>::try_into(creator_bytes).unwrap();
        let flag: u16 = u16::from_be_bytes(flag_bytes.try_into().unwrap());
        let data_fork_length: usize =
            u32::from_be_bytes(data_fork_length_bytes.try_into().unwrap()) as usize;
        let resource_fork_length: usize =
            u32::from_be_bytes(resource_fork_length_bytes.try_into().unwrap()) as usize;

        Ok(BinHexHeader {
            name,
            file_type,
            creator,
            flag,
            data_fork_length,
            resource_fork_length,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;
    use std::io::Cursor;

    const BINHEX_DATA: &[u8] = indoc! {br#"
            (This file must be converted with BinHex 4.0)
            :$f*TEQKPH#edCA0d,R4iG!#3$L8!N!-TR@dpN!8J5'9XE'mJCR*[E5"dD'8JC'&
            dB5"QEh*V)5!pN!9Bm5f3"5")C@aXEb"QFQpY)(4SC5"bCA0[GA*MC5"QEh*V)5!
            YN!8SI!:"#
    };

    const DATA_FORK: &[u8] = b"===== Hello from the data fork! =====";
    const RESOURCE_FORK: &[u8] = b"----- Hello from the resource fork! -----";

    const SIMPLE_TEXT_DOCUMENT: &[u8] = indoc! {br##"
            (This file must be converted with BinHex 4.0)

            :&&0TEA"XC94PH(5U)%4[Bh9YC@jd!&4&@&4dG(Kd!*!&&J!!!8c328KPE'a[)'C
            bEfdJ8fPYF'aP9'9iG#'`*3!!!3#3!`%D!*!$'J#3!c*J6qm!%'!',`a1Z[@161i
            B`2r`6PiJAdr[!"41d#)[!"46D@e`E'98CAKdUL"%Ef0eE@9ZG'CcFfPd)(*dFf&
            X!'T849K8G(4iG!#3%)!!N!IFK-H8!*!'!4iF0J"#3%K!C`5!`63!5%)`!i$"0!!
            L!N*!5%"J)L3!3N")3%K#N!-Q!A)!H!r8JY'!dS'`Jf8%N!#$8J&4c2r`60m!(%j
            e)PmJAk!P,S"U!N+A6Y%!N!1U!!)!!!%!!!J!!!%)!!J!!!%8!!%!!!&F!!)!!!*
            i!!%!!!,i!!3!!!-@!*!%&J!"!*!&%!!-!!-!N!--!*!)!3#3!`%D!*!$'J#3!c)
            2`&')%J#3""`!-J!!Fh4jE!#3!`S!J2rr!*!%$m#lc&2h!:"##
    };

    #[test]
    fn read_header() {
        let mut archive = BinHexArchive::new(Cursor::new(BINHEX_DATA));

        let header = archive.header().unwrap();

        assert_eq!("binhex-test.txt", header.name);
        assert_eq!([0; 4], header.file_type);
        assert_eq!([0; 4], header.creator);
        assert_eq!(0, header.flag);
        assert_eq!(DATA_FORK.len(), header.data_fork_length);
        assert_eq!(RESOURCE_FORK.len(), header.resource_fork_length);
    }

    #[test]
    fn filename() {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT));

        assert_eq!(
            String::from("SimpleText™ Document"),
            archive.filename().unwrap()
        );
    }

    #[test]
    fn file_type() {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT));
        assert_eq!(b"TEXT", &archive.file_type().unwrap());
    }

    #[test]
    fn creator() {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT));
        assert_eq!(b"ttxt", &archive.creator().unwrap());
    }

    #[test]
    fn flags() {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT));
        assert_eq!(0x0000, archive.flags().unwrap());
    }

    #[test]
    fn data_fork_len() {
        let mut archive = BinHexArchive::new(Cursor::new(BINHEX_DATA));
        assert_eq!(DATA_FORK.len(), archive.data_fork_len().unwrap());
    }

    #[test]
    fn resource_fork_len() {
        let mut archive = BinHexArchive::new(Cursor::new(BINHEX_DATA));
        assert_eq!(RESOURCE_FORK.len(), archive.resource_fork_len().unwrap());
    }

    #[test]
    fn extract() {
        let cursor = Cursor::new(BINHEX_DATA);

        let archive = BinHexArchive::new(cursor);

        let mut data_fork = vec![];
        let mut resource_fork = vec![];

        archive.extract(&mut data_fork, &mut resource_fork).unwrap();

        assert_eq!(DATA_FORK, data_fork.as_slice());
        assert_eq!(RESOURCE_FORK, resource_fork.as_slice());
    }
}
