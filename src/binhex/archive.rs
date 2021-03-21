use std::convert::{TryFrom, TryInto};
use crc::{crc16, Hasher16};

#[derive(Debug)]
pub struct BinHexHeader {
    name: Vec<u8>,
    file_type: [u8; 4],
    auth: [u8; 4],
    flag: u16,
    data_fork_length: u32,
    resource_fork_length: u32,
}

pub struct BinHexError {
}

impl TryFrom<Vec<u8>> for BinHexHeader {
    type Error = BinHexError;

    fn try_from(header_bytes: Vec<u8>) -> Result<Self, Self::Error> {

        let name_length = header_bytes[0] as usize;

        let name = Vec::from(&header_bytes[1..name_length]);

        let (_, remaining_bytes) = header_bytes.split_at(name_length + 2);
        let (file_type_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (auth_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (flag_bytes, remaining_bytes) = remaining_bytes.split_at(2);
        let (data_fork_length_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (resource_fork_length_bytes, remaining_bytes) = remaining_bytes.split_at(4);
        let (crc_bytes, remaining_bytes) = remaining_bytes.split_at(2);

        debug_assert!(remaining_bytes.is_empty());

        let mut digest = crc16::Digest::new(0x1021);
        digest.write(&header_bytes[..header_bytes.len() - 2]);
        digest.write(&[0; 2]);

        let checksum = u16::from_be_bytes(crc_bytes.try_into().unwrap());

        if checksum != digest.sum16() {
            // TODO Include error kinds/messages
            return Err(BinHexError {});
        }

        let file_type: [u8; 4] = TryInto::<[u8; 4]>::try_into(file_type_bytes).unwrap();
        let auth: [u8; 4] = TryInto::<[u8; 4]>::try_into(auth_bytes).unwrap();
        let flag: u16 = u16::from_be_bytes(flag_bytes.try_into().unwrap());
        let data_fork_length: u32 = u32::from_be_bytes(data_fork_length_bytes.try_into().unwrap());
        let resource_fork_length: u32 = u32::from_be_bytes(resource_fork_length_bytes.try_into().unwrap());

        Ok(BinHexHeader { name, file_type, auth, flag, data_fork_length, resource_fork_length })
    }
}

#[cfg(test)]
mod test {

}