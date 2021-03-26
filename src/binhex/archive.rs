use std::convert::{TryFrom, TryInto};
use std::hash::Hasher;
use std::io::{self, Error, ErrorKind, Read, Write};
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

#[derive(Clone, Debug)]
pub struct BinHexHeader {
    name: String,
    file_type: [u8; 4],
    creator: [u8; 4],
    flag: u16,
    data_fork_length: usize,
    resource_fork_length: usize,
}

pub struct BinHexArchive<R: Read> {
    source: BinHexExpander<DecodeReader<&'static CustomConfig, EncodedBinHexReader<R>>>,

    header: Option<BinHexHeader>,
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

impl<R: Read> BinHexArchive<R> {
    pub fn new(source: R) -> Self {
        let reader = EncodedBinHexReader::new(source);
        let decoder = DecodeReader::new(BINHEX_CONFIG.deref(), reader);
        let expander = BinHexExpander::new(decoder);

        BinHexArchive {
            source: expander,
            header: None,
        }
    }

    pub fn header(&mut self) -> io::Result<BinHexHeader> {
        match &self.header {
            Some(header) => Ok(header.clone()),
            None => self.read_header(),
        }
    }

    fn read_header(&mut self) -> io::Result<BinHexHeader> {
        debug_assert!(self.header.is_none());

        // Headers have a minimum size of 22 bytes (assuming a zero-length name) and a maximum size
        // of 277 bytes (assuming a 255-byte name); to avoid overshooting and eating into the data
        // fork, we read the minimum, check the name length, and extend as needed.
        let mut header_bytes = Vec::with_capacity(277);
        header_bytes.resize(22, 0);

        self.source.read_exact(header_bytes.as_mut_slice())?;

        let name_length = header_bytes[0] as usize;

        header_bytes.resize(header_bytes.len() + name_length, 0);
        self.source
            .read_exact(&mut header_bytes.as_mut_slice()[22..])?;

        let header = BinHexHeader::try_from(header_bytes)?;
        self.header = Some(header.clone());

        Ok(header)
    }

    pub fn extract(
        mut self,
        data_writer: &mut impl Write,
        resource_writer: &mut impl Write,
    ) -> io::Result<()> {
        let header = self.header()?;

        self.copy_fork(data_writer, header.data_fork_length)?;
        self.copy_fork(resource_writer, header.resource_fork_length)?;

        Ok(())
    }

    fn copy_fork(&mut self, dest: &mut impl Write, len: usize) -> io::Result<()> {
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
            Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Data fork checksum failed; expected {:04x}, calculated {:04x}",
                    provided_checksum, calculated_checksum
                ),
            ))
        }
    }
}

impl TryFrom<Vec<u8>> for BinHexHeader {
    type Error = io::Error;

    fn try_from(header_bytes: Vec<u8>) -> Result<Self, Self::Error> {
        let (name_length_bytes, remaining_bytes) = header_bytes.split_at(1);
        let name_length = name_length_bytes[0] as usize;

        if header_bytes.len() != name_length + 22 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "Expected at least {} header bytes, but only found {}",
                    name_length + 22,
                    header_bytes.len()
                ),
            ));
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

        let hash =
            crc16::State::<crc16::XMODEM>::calculate(&header_bytes[..header_bytes.len() - 2]);
        let checksum = u16::from_be_bytes(checksum_bytes.try_into().unwrap());

        if checksum != hash {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!(
                    "Header checksum failed; expected {:04x}, calculated {:04x}",
                    checksum, hash
                ),
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

    #[test]
    fn read_header() {
        let mut archive = BinHexArchive::new(Cursor::new(BINHEX_DATA));

        let header = archive.read_header().unwrap();

        assert_eq!("binhex-test.txt", header.name);
        assert_eq!([0; 4], header.file_type);
        assert_eq!([0; 4], header.creator);
        assert_eq!(0, header.flag);
        assert_eq!(DATA_FORK.len(), header.data_fork_length);
        assert_eq!(RESOURCE_FORK.len(), header.resource_fork_length);
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
