use std::convert::{TryFrom, TryInto};
use std::error;
use std::fmt::{self, Display, Formatter};
use std::hash::Hasher;
use std::io::{self, Read, Write};
use std::ops::Deref;

use super::expand::BinHexExpander;
use super::read::EncodedBinHexReader;

use crc16::{State, XMODEM};
use radix64::io::DecodeReader;
use radix64::CustomConfig;

lazy_static::lazy_static! {
    static ref BINHEX_CONFIG: CustomConfig = CustomConfig::with_alphabet(
        r##"!"#$%&'()*+,-012345689@ABCDEFGHIJKLMNPQRSTUVXYZ[`abcdefhijklmpqr"##)
    .no_padding()
    .build()
    .expect("Failed to build BinHex base64 config");
}

/// A BinHex-encoded archive.
///
/// BinHex archives encode the data fork, resource fork, and metadata associated with a "classic"
/// Macintosh file.
pub struct BinHexArchive<R: Read> {
    source: BinHexExpander<DecodeReader<&'static CustomConfig, EncodedBinHexReader<R>>>,
    header: BinHexHeader,
}

impl<R: Read> BinHexArchive<R> {
    /// Creates a new BinHex archive that will extract data from the given reader.
    ///
    /// # Errors
    ///
    /// This function will return an error if a valid BinHex header could not be read from the given
    /// source.
    pub fn new(source: R) -> Result<Self, BinHexError> {
        let reader = EncodedBinHexReader::new(source);
        let decoder = DecodeReader::new(BINHEX_CONFIG.deref(), reader);
        let mut expander = BinHexExpander::new(decoder);

        let header = {
            // Headers have a minimum size of 22 bytes (assuming a zero-length name) and a maximum
            // size of 277 bytes (assuming a 255-byte name); to avoid overshooting and eating into
            // the data fork, we read the minimum, check the name length, and extend as needed.
            let mut header_bytes = Vec::with_capacity(277);
            header_bytes.resize(22, 0);

            expander.read_exact(header_bytes.as_mut_slice())?;

            let name_length = header_bytes[0] as usize;

            header_bytes.resize(header_bytes.len() + name_length, 0);
            expander.read_exact(&mut header_bytes.as_mut_slice()[22..])?;

            BinHexHeader::try_from(header_bytes)?
        };

        Ok(BinHexArchive {
            source: expander,
            header: header,
        })
    }

    /// Returns the original filename of the file contained in this archive.
    pub fn filename(&mut self) -> &String {
        &self.header.name
    }

    /// Returns the file type identifier for the file contained in this archive.
    ///
    /// For a detailed description of file signatures (including file type identifiers), please see
    /// the ["Giving a Signature to Your Application and a Creator and a File Type to Your
    /// Documents" section of "Inside Macintosh: Macintosh Toolbox
    /// Essentials"](https://developer.apple.com/library/archive/documentation/mac/pdf/MacintoshToolboxEssentials.pdf#page=806).
    pub fn file_type(&mut self) -> [u8; 4] {
        self.header.file_type
    }

    /// Returns the creator identifier for the file contained in this archive.
    ///
    /// For a detailed description of file signatures (including creator identifiers), please see
    /// the ["Giving a Signature to Your Application and a Creator and a File Type to Your
    /// Documents" section of "Inside Macintosh: Macintosh Toolbox
    /// Essentials"](https://developer.apple.com/library/archive/documentation/mac/pdf/MacintoshToolboxEssentials.pdf#page=806).
    pub fn creator(&mut self) -> [u8; 4] {
        self.header.creator
    }

    /// Returns the Finder flags for the file contained in this archive.
    ///
    /// For a detailed description of the Finder flags, please see [the "File Information Record"
    /// section of "Inside Macintosh: Macintosh Toolbox
    /// Essentials"](https://developer.apple.com/library/archive/documentation/mac/pdf/MacintoshToolboxEssentials.pdf#page=845).
    pub fn flags(&mut self) -> u16 {
        self.header.flag
    }

    /// Returns the length, in bytes after decoding, of the data fork contained in this archive.
    pub fn data_fork_len(&mut self) -> usize {
        self.header.data_fork_length
    }

    /// Returns the length, in bytes after decoding, of the resource fork contained in this archive.
    pub fn resource_fork_len(&mut self) -> usize {
        self.header.resource_fork_length
    }

    /// Extracts this archive's content to the given writers, verifying checksums in the process.
    ///
    /// This method may return an error after some or all of the archive's content has been written
    /// to the given writers.
    ///
    /// # Errors
    ///
    /// This method returns an error immediately if it encounters an IO error while extracting data,
    /// if it encounters malformed data in the archive, or if a checksum fails.
    ///
    /// # Examples
    ///
    /// ## Read archive content into memory
    ///
    /// The most common use case for BinHex archives is extracting the content of the archive for
    /// further processing or use. As an example of loading an archive's content into memory:
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use clarus::binhex::{BinHexArchive, BinHexError};
    ///
    /// fn main() -> Result<(), BinHexError> {
    ///     let binhex_file = File::open("example.hqx")?;
    ///     let mut archive = BinHexArchive::new(binhex_file)?;
    ///
    ///     let mut data_fork_content = Vec::with_capacity(archive.data_fork_len());
    ///     let mut rsrc_fork_content = Vec::with_capacity(archive.resource_fork_len());
    ///
    ///     archive.extract(&mut data_fork_content, &mut rsrc_fork_content)
    /// }
    /// ```
    ///
    /// ## Write the data fork of an archive to a file
    ///
    /// It's not always necessary to use both forks of a file. Callers may want to extract only the
    /// data fork of a file, for example. In that case, we can discard the content of the resource
    /// fork using a [`std::io::Sink`]:
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use std::io;
    /// use clarus::binhex::{BinHexArchive, BinHexError};
    ///
    /// fn main() -> Result<(), BinHexError> {
    ///     let binhex_file = File::open("example.hqx")?;
    ///     let mut archive = BinHexArchive::new(binhex_file)?;
    ///
    ///     let mut data_file = File::open("binhex-data.txt")?;
    ///     let mut sink = io::sink();
    ///
    ///     archive.extract(&mut data_file, &mut sink)
    /// }
    /// ```
    ///
    /// ## Verify checksums without writing data
    ///
    /// In some cases, callers may not be immediately interested in the content of an archive at
    /// all, and may just want to verify its integrity. To verify the integrity of an archive—
    /// especially its checksums—we still need to read and inspect all of the data. We don't need
    /// to do anything with it, though, and can simply discard it:
    ///
    /// ```no_run
    /// use std::fs::File;
    /// use std::io;
    /// use clarus::binhex::{BinHexArchive, BinHexError};
    ///
    /// fn main() -> Result<(), BinHexError> {
    ///     let binhex_file = File::open("example.hqx")?;
    ///     let mut archive = BinHexArchive::new(binhex_file)?;
    ///
    ///     let mut data_sink = io::sink();
    ///     let mut rsrc_sink = io::sink();
    ///
    ///     match archive.extract(&mut data_sink, &mut rsrc_sink) {
    ///         Ok(_) => println!("Looks good!"),
    ///         Err(BinHexError::InvalidChecksum(section, provided, calculated)) => {
    ///             println!("Bad checksum in {:?}; expected {:04x}, but calculated {:04x}",
    ///                      section, provided, calculated)
    ///         },
    ///         Err(error) => println!("Something else went wrong: {:?}", error),
    ///     };
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn extract(
        mut self,
        data_writer: &mut impl Write,
        resource_writer: &mut impl Write,
    ) -> Result<(), BinHexError> {
        self.copy_fork(
            ChecksumSection::DataFork,
            data_writer,
            self.header.data_fork_length,
        )?;

        self.copy_fork(
            ChecksumSection::ResourceFork,
            resource_writer,
            self.header.resource_fork_length,
        )
    }

    /// Copies one of an archive's two forks to a destination writer and verifies the checksum at
    /// the end of the fork's content.
    ///
    /// The length of the fork must be known, and the source `Read` must be positioned at the start
    /// of the fork.
    ///
    /// # Errors
    ///
    /// This function will return an error immediately if an IO operation (i.e. [`std::io::copy`])
    /// returns an error. It will also return an error if the checksum at the end of the fork's
    /// content does not match the checksum calculated from the fork's content.
    fn copy_fork(
        &mut self,
        section: ChecksumSection,
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

/// The error type for operations on BinHex-encoded files.
///
/// Errors may occur while attempting to read the data (an `IoError`) or when processing the
/// loaded content.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BinHexError {
    /// An [`std::io::Error`] occurred while reading some part of the archive.
    ///
    /// The kind of IO error is included.
    IoError(io::ErrorKind),

    /// The BinHex archive's header was malformed and could not be read.
    InvalidHeader,

    /// Data in some part of the BinHex archive was malformed and could not be read.
    InvalidData,

    /// The checksum included in a section of a BinHex archive did not match the checksum calculated
    /// from its content.
    ///
    /// The section in which the checksum did not match, the checksum provided in the BinHex file,
    /// and the checksum calculated from the section's content are included.
    InvalidChecksum(ChecksumSection, u16, u16),
}

impl Display for BinHexError {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> fmt::Result {
        match self {
            BinHexError::IoError(kind) => write!(fmt, "IO error: {:?}", kind),
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
        BinHexError::IoError(error.kind())
    }
}

impl error::Error for BinHexError {}

/// A section of a BinHex archive.
///
/// BinHex archives are divided into a header, a data fork, and a resource fork, each of which has
/// its own checksum.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChecksumSection {
    Header,
    DataFork,
    ResourceFork,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BinHexHeader {
    name: String,
    file_type: [u8; 4],
    creator: [u8; 4],
    flag: u16,
    data_fork_length: usize,
    resource_fork_length: usize,
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
                ChecksumSection::Header,
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

struct ForkReader<'a, R: Read> {
    source: &'a mut R,
    len: usize,
    bytes_read: usize,
    crc: State<XMODEM>,
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
    fn filename() -> Result<(), BinHexError> {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT))?;

        assert_eq!(&String::from("SimpleText™ Document"), archive.filename());

        Ok(())
    }

    #[test]
    fn file_type() -> Result<(), BinHexError> {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT))?;
        assert_eq!(b"TEXT", &archive.file_type());

        Ok(())
    }

    #[test]
    fn creator() -> Result<(), BinHexError> {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT))?;
        assert_eq!(b"ttxt", &archive.creator());

        Ok(())
    }

    #[test]
    fn flags() -> Result<(), BinHexError> {
        let mut archive = BinHexArchive::new(Cursor::new(SIMPLE_TEXT_DOCUMENT))?;
        assert_eq!(0x0000, archive.flags());

        Ok(())
    }

    #[test]
    fn data_fork_len() -> Result<(), BinHexError> {
        let mut archive = BinHexArchive::new(Cursor::new(BINHEX_DATA))?;
        assert_eq!(DATA_FORK.len(), archive.data_fork_len());

        Ok(())
    }

    #[test]
    fn resource_fork_len() -> Result<(), BinHexError> {
        let mut archive = BinHexArchive::new(Cursor::new(BINHEX_DATA))?;
        assert_eq!(RESOURCE_FORK.len(), archive.resource_fork_len());

        Ok(())
    }

    #[test]
    fn extract() -> Result<(), BinHexError> {
        let cursor = Cursor::new(BINHEX_DATA);

        let archive = BinHexArchive::new(cursor)?;

        let mut data_fork = vec![];
        let mut resource_fork = vec![];

        archive.extract(&mut data_fork, &mut resource_fork)?;

        assert_eq!(DATA_FORK, data_fork.as_slice());
        assert_eq!(RESOURCE_FORK, resource_fork.as_slice());

        Ok(())
    }
}
