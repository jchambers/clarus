use std::convert::{TryFrom, TryInto};
use std::io::{BufRead, BufReader, Read, Error, ErrorKind, Write};
use super::expand::BinHexExpander;
use super::read::EncodedBinHexReader;
use radix64::CustomConfig;
use radix64::io::DecodeReader;
use std::ops::Deref;
use std::cmp;
use crc16::{State, XMODEM};

const COPY_BUFFER_LENGTH: usize = 1024;

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

pub struct BinHexArchive<S: BufRead> {
    source: BinHexExpander<BufReader<DecodeReader<&'static CustomConfig, EncodedBinHexReader<S>>>>,

    header: Option<BinHexHeader>,
}

impl<S: BufRead> BinHexArchive<S> {

    pub fn new(source: S) -> Self {
        let encoded_reader = EncodedBinHexReader::new(source);
        let decoder = DecodeReader::new(BINHEX_CONFIG.deref(), encoded_reader);
        let buf_decoder = BufReader::new(decoder);
        let expander = BinHexExpander::new(buf_decoder);

        BinHexArchive { source: expander, header: None }
    }

    pub fn header(&mut self) -> std::io::Result<BinHexHeader> {
        match &self.header {
            Some(header) => Ok(header.clone()),
            None => self.read_header(),
        }
    }

    fn read_header(&mut self) -> std::io::Result<BinHexHeader> {
        debug_assert!(self.header.is_none());

        // Headers have a minimum size of 22 bytes (assuming a zero-length name) and a maximum size
        // of 277 bytes (assuming a 255-byte name); to avoid overshooting and eating into the data
        // fork, we read the minimum, check the name length, and extend as needed.
        let mut header_bytes = Vec::with_capacity(277);
        header_bytes.resize(22, 0);

        self.source.read_exact(header_bytes.as_mut_slice())?;

        let name_length = header_bytes[0] as usize;

        header_bytes.resize(header_bytes.len() + name_length, 0);
        self.source.read_exact(&mut header_bytes.as_mut_slice()[22..])?;

        let header = BinHexHeader::try_from(header_bytes)?;
        self.header = Some(header.clone());

        Ok(header)
    }

    pub fn extract<D: Write, Z: Write>(mut self, data_writer: &mut D, resource_writer: &mut Z)
        -> std::io::Result<()> {

        let header = self.header()?;

        let data_hash = Self::copy_and_checksum(&mut self.source, data_writer, header.data_fork_length)?;

        let data_checksum = {
            let mut data_checksum_bytes = [0; 2];
            self.source.read_exact(&mut data_checksum_bytes)?;

            u16::from_be_bytes(data_checksum_bytes)
        };

        if data_checksum != data_hash {
            return Err(Error::new(ErrorKind::InvalidData,
                                  format!("Data fork checksum failed; expected {:04x}, calculated {:04x}",
                                          data_checksum, data_hash)));
        }

        let resource_hash = Self::copy_and_checksum(&mut self.source, resource_writer, header.resource_fork_length)?;

        let resource_checksum = {
            let mut resource_checksum_bytes = [0; 2];
            self.source.read_exact(&mut resource_checksum_bytes)?;

            u16::from_be_bytes(resource_checksum_bytes)
        };

        if resource_checksum != resource_hash {
            return Err(Error::new(ErrorKind::InvalidData,
                                  format!("Resource fork checksum failed; expected {:04x}, calculated {:04x}",
                                          resource_checksum, resource_hash)));
        }

        Ok(())
    }

    fn copy_and_checksum<R: Read, W: Write>(src: &mut R, dest: &mut W, len: usize)
        -> std::io::Result<u16> {

        let mut buf = [0; COPY_BUFFER_LENGTH];
        let mut bytes_copied = 0;

        let mut crc = State::<XMODEM>::new();

        loop {
            let capacity = cmp::min(len - bytes_copied, buf.len());
            let bytes_read = src.read(&mut buf[..capacity])?;

            dest.write_all(&buf[..bytes_read])?;
            crc.update(&buf[..bytes_read]);

            bytes_copied += bytes_read;

            if bytes_copied == len {
                break Ok(crc.get());
            }
        }
    }
}

impl TryFrom<Vec<u8>> for BinHexHeader {
    type Error = std::io::Error;

    fn try_from(header_bytes: Vec<u8>) -> Result<Self, Self::Error> {

        let (name_length_bytes, remaining_bytes) = header_bytes.split_at(1);
        let name_length = name_length_bytes[0] as usize;

        if header_bytes.len() != name_length + 22 {
            return Err(Error::new(ErrorKind::InvalidInput,
                                  format!("Expected at least {} header bytes, but only found {}",
                                          name_length + 22, header_bytes.len())));
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

        let hash = crc16::State::<crc16::XMODEM>::calculate(&header_bytes[..header_bytes.len() - 2]);
        let checksum = u16::from_be_bytes(checksum_bytes.try_into().unwrap());

        if checksum != hash {
            return Err(Error::new(ErrorKind::InvalidData,
                                  format!("Header checksum failed; expected {:04x}, calculated {:04x}",
                                          checksum, hash)));
        }

        let (name_cow, _, _) = encoding_rs::MACINTOSH.decode(name_bytes);
        let name = name_cow.to_string();
        let file_type: [u8; 4] = TryInto::<[u8; 4]>::try_into(file_type_bytes).unwrap();
        let creator: [u8; 4] = TryInto::<[u8; 4]>::try_into(creator_bytes).unwrap();
        let flag: u16 = u16::from_be_bytes(flag_bytes.try_into().unwrap());
        let data_fork_length: usize = u32::from_be_bytes(data_fork_length_bytes.try_into().unwrap()) as usize;
        let resource_fork_length: usize = u32::from_be_bytes(resource_fork_length_bytes.try_into().unwrap()) as usize;

        Ok(BinHexHeader { name, file_type, creator, flag, data_fork_length, resource_fork_length })
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;
    use indoc::indoc;
    use super::*;
    use std::fs::File;

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
        let cursor = Cursor::new(BINHEX_DATA);

        let mut archive = BinHexArchive::new(cursor);

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

        let mut archive = BinHexArchive::new(cursor);

        let mut data_fork = vec![];
        let mut resource_fork = vec![];

        archive.extract(&mut data_fork, &mut resource_fork).unwrap();

        assert_eq!(DATA_FORK, data_fork.as_slice());
        assert_eq!(RESOURCE_FORK, resource_fork.as_slice());
    }
}
