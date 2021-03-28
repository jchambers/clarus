//! Tools for extracting data from BinHex 4.0 archives.
//!
//! BinHex is an encoding system for "classic" Mac files that combines the binary data from a file's
//! data and resource forks into a single ASCII-encoded file. BinHex was generally used to transfer
//! files via email or online services that didn't have robust support for binary files.
//!
//! For additional details about Mac file forks, please see [Chapter 1
//! ("Introduction to File
//! Management")](https://developer.apple.com/library/archive/documentation/mac/pdf/Files/Intro_to_Files.pdf)
//! of ["Inside Macintosh:
//! Files."](https://developer.apple.com/library/archive/documentation/mac/pdf/Files/pdf.html)
//!
//! For details about the BinHex 4.0 file format, please see:
//!
//! - [BinHex 4.0 Definition - Peter N Lewis, Aug 1991.](https://files.stairways.com/other/binhex-40-specs-info.txt)
//! - [RFC 1741 - MIME Content Type for BinHex Encoded Files](https://tools.ietf.org/html/rfc1741)

mod archive;
mod expand;
mod read;

pub use archive::{BinHexArchive, BinHexError, ChecksumSection};
