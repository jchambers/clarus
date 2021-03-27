// http://files.stairways.com/other/binhex-40-specs-info.txt
// https://tools.ietf.org/html/rfc1741

mod archive;
mod expand;
mod read;

pub use archive::{BinHexArchive, BinHexError, ChecksumSection};
