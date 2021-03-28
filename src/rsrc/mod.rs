//! Extract resource data from resource forks of "classic" Macintosh files.
//!
//! Classic Mac files were divided into two "forks:" the data fork and the resource fork. The data
//! fork contained arbitrary, application-defined data (like single-stream files familiar to most
//! of us today). The resource fork contained a structured map of well-defined data structures that
//! represented things like UI elements, sounds, and images.
//!
//! For a complete overview of resources, please see the ["Resource Manager" chapter of "Inside
//! Macintosh: More Macintosh
//! Toolbox"](https://developer.apple.com/library/archive/documentation/mac/pdf/MoreMacintoshToolbox.pdf)

use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::io::{Error, Read, Seek, SeekFrom};

const NO_NAME: u16 = 0xffff;

/// A resource type identifier.
///
/// Resource type identifiers are commonly represented as four-character ASCII strings in
/// user-facing contexts. For details, please see the ["The Resource Type" section of "Inside
/// Macintosh: More Macintosh
/// Toolbox."](https://developer.apple.com/library/archive/documentation/mac/pdf/MoreMacintoshToolbox.pdf#page=72)
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceType {
    bytes: [u8; 4],
}

impl From<[u8; 4]> for ResourceType {
    fn from(bytes: [u8; 4]) -> Self {
        ResourceType { bytes }
    }
}

impl TryFrom<&[u8]> for ResourceType {
    type Error = ResourceTypeError;

    fn try_from(slice: &[u8]) -> Result<Self, Self::Error> {
        if slice.len() == 4 {
            Ok(ResourceType::from(
                TryInto::<[u8; 4]>::try_into(slice).unwrap(),
            ))
        } else {
            Err(ResourceTypeError::BadEncodedLength(slice.len()))
        }
    }
}

impl TryFrom<&str> for ResourceType {
    type Error = ResourceTypeError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() != 4 {
            return Err(ResourceTypeError::BadInitialLength(value.len()));
        }

        let (cow, _, had_errors) = encoding_rs::MACINTOSH.encode(value);

        if had_errors {
            Err(ResourceTypeError::IllegalCharacters)
        } else if cow.len() == 4 {
            Ok(ResourceType::from(
                TryInto::<[u8; 4]>::try_into(cow.as_ref()).unwrap(),
            ))
        } else {
            Err(ResourceTypeError::BadEncodedLength(cow.len()))
        }
    }
}

impl From<ResourceType> for String {
    fn from(resource_type: ResourceType) -> Self {
        encoding_rs::MACINTOSH
            .decode(&resource_type.bytes)
            .0
            .to_string()
    }
}

impl PartialEq<ResourceType> for &str {
    fn eq(&self, other: &ResourceType) -> bool {
        let (cow, _, _) = encoding_rs::MACINTOSH.encode(self);

        return cow.as_ref() == other.bytes;
    }
}

impl PartialEq<ResourceType> for String {
    fn eq(&self, other: &ResourceType) -> bool {
        &self.as_str() == other
    }
}

impl PartialEq<ResourceType> for [u8] {
    fn eq(&self, other: &ResourceType) -> bool {
        self == other.bytes
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ResourceTypeError {
    IllegalCharacters,
    BadInitialLength(usize),
    BadEncodedLength(usize),
}

/// Provides access to resources stored in the resource fork of a "classic" Mac file.
pub struct ResourceFork<R: Read + Seek> {
    source: R,
    header: Header,
    attributes: u16,
    ids_by_name: HashMap<(ResourceType, String), u16>,
    resources_by_id: HashMap<(ResourceType, u16), ResourceMetadata>,
}

impl<R: Read + Seek> ResourceFork<R> {
    /// Creates a new `ResourceFork` that loads resources from the given source.
    pub fn new(mut source: R) -> Result<Self, ResourceError> {
        let header = {
            let mut header_buf = [0; 16];
            source.read_exact(&mut header_buf)?;

            Header::from(header_buf)
        };

        source.seek(SeekFrom::Start((header.map_offset) as u64))?;

        // A resource fork with no resources should still have a resource map that's at least 30
        // bytes long
        if header.map_len < 30 {
            return Err(ResourceError::CorruptResourceMap);
        }

        let mut map_bytes = vec![0; header.map_len as usize];
        source.read_exact(&mut map_bytes)?;

        // The resource map header includes 16 reserved bytes for a copy of the fork header, four
        // bytes for a handle to the next resource map, two bytes for a file reference number; we're
        // not using any of that and can just skip over the reserved bytes.
        let (_reserved, remaining_bytes) = map_bytes.split_at(22);
        let (attribute_bytes, remaining_bytes) =
            remaining_bytes.split_at(std::mem::size_of::<u16>());
        let (type_list_offset_bytes, remaining_bytes) =
            remaining_bytes.split_at(std::mem::size_of::<u16>());
        let (name_list_offset_bytes, remaining_bytes) =
            remaining_bytes.split_at(std::mem::size_of::<u16>());
        let (type_count_bytes, _) = remaining_bytes.split_at(std::mem::size_of::<u16>());

        let attributes = u16::from_be_bytes(attribute_bytes.try_into().unwrap());
        let type_list_offset = u16::from_be_bytes(type_list_offset_bytes.try_into().unwrap());
        let name_list_offset = u16::from_be_bytes(name_list_offset_bytes.try_into().unwrap());

        // The type count in the resource fork is "number of types in the map minus 1"
        let type_count = u16::from_be_bytes(type_count_bytes.try_into().unwrap()) + 1;

        // The map length must be at least 30 bytes (for the header, including the type count), then
        // 8 bytes for each item in the type list
        if header.map_len < 30 + (8 * type_count as u32) {
            return Err(ResourceError::CorruptResourceMap);
        }

        let mut ids_by_name = HashMap::new();
        let mut resources_by_id = HashMap::new();

        for t in 0..type_count {
            // Plus 2 because the type count is technically part of the type list
            let type_offset = (type_list_offset + 2 + (t * 8)) as usize;

            let type_entry_bytes: [u8; 8] =
                map_bytes[type_offset..type_offset + 8].try_into().unwrap();
            let type_entry = TypeListEntry::from(type_entry_bytes);

            for r in 0..type_entry.count {
                let reference_offset =
                    (type_list_offset + type_entry.reference_list_offset + (r * 12)) as usize;

                // The reference list entry is 12 bytes; we expect to have at least that many still
                // to read at the given offset
                if header.map_len < reference_offset as u32 + 12 {
                    return Err(ResourceError::CorruptResourceMap);
                }

                let reference_entry_bytes: [u8; 12] = map_bytes
                    [reference_offset..reference_offset + 12]
                    .try_into()
                    .unwrap();

                let reference_entry = ReferenceListEntry::from(reference_entry_bytes);

                let maybe_name = if reference_entry.name_list_offset == NO_NAME {
                    None
                } else {
                    // Make sure we have at least one byte to read at the start of the name entry
                    if header.map_len < (name_list_offset + reference_entry.name_list_offset) as u32
                    {
                        return Err(ResourceError::CorruptResourceMap);
                    }

                    let name_len =
                        map_bytes[(name_list_offset + reference_entry.name_list_offset) as usize];

                    // ...and that we have enough bytes left to read the whole name
                    if header.map_len
                        < (name_list_offset + reference_entry.name_list_offset + 1) as u32
                    {
                        return Err(ResourceError::CorruptResourceMap);
                    }

                    let name_start =
                        (name_list_offset + reference_entry.name_list_offset + 1) as usize;
                    let name_bytes = &map_bytes[name_start..name_start + name_len as usize];

                    Some(encoding_rs::MACINTOSH.decode(&name_bytes).0.to_string())
                };

                if let Some(ref name) = maybe_name {
                    ids_by_name
                        .insert((type_entry.resource_type, name.clone()), reference_entry.id);
                }

                resources_by_id.insert(
                    (type_entry.resource_type, reference_entry.id),
                    ResourceMetadata {
                        resource_type: type_entry.resource_type,
                        id: reference_entry.id,
                        name: maybe_name,
                        attributes: reference_entry.attributes,
                        data_offset: reference_entry.data_offset,
                    },
                );
            }
        }

        Ok(ResourceFork {
            source,
            header,
            attributes,
            ids_by_name,
            resources_by_id,
        })
    }

    /// Returns an iterator over the metadata of all of the resources contained in this resource
    /// fork.
    pub fn resources(&self) -> impl Iterator<Item = &ResourceMetadata> {
        self.resources_by_id.values()
    }

    /// Loads data and metadata for the resource with the given type and ID.
    ///
    /// Resources are uniquely identified by their type and ID (or name, if present; see
    /// [`ResourceFork::load_by_name`]). Multiple resources may have the same ID as long as they are
    /// not of the same type.
    ///
    /// # Errors
    ///
    /// This method returns an error if no resource could be found for the given type/ID or if the
    /// underlying reader returns an error while reading resource data.
    pub fn load_by_id(
        &mut self,
        resource_type: ResourceType,
        id: u16,
        dest: &mut Vec<u8>,
    ) -> Result<&ResourceMetadata, ResourceError> {
        if let Some(entry) = self.resources_by_id.get(&(resource_type, id)) {
            // Make sure we can at least load the data length bytes
            if self.header._data_len < entry.data_offset + 4 {
                return Err(ResourceError::CorruptResourceData);
            }

            let mut len_bytes = [0; std::mem::size_of::<u32>()];

            self.source.seek(SeekFrom::Start(
                (self.header.data_offset + entry.data_offset) as u64,
            ))?;

            self.source.read_exact(&mut len_bytes)?;
            let resource_len = u32::from_be_bytes(len_bytes);

            // ...and make sure we can load the rest of the resource data, too
            if self.header._data_len < entry.data_offset + 4 + resource_len {
                return Err(ResourceError::CorruptResourceData);
            }

            dest.resize(resource_len as usize, 0);
            self.source.read_exact(dest)?;

            Ok(entry)
        } else {
            Err(ResourceError::NotFound)
        }
    }

    pub fn load_by_name(
        &mut self,
        resource_type: ResourceType,
        name: String,
        dest: &mut Vec<u8>,
    ) -> Result<&ResourceMetadata, ResourceError> {
        if let Some(&id) = self.ids_by_name.get(&(resource_type, name)) {
            self.load_by_id(resource_type, id, dest)
        } else {
            Err(ResourceError::NotFound)
        }
    }

    // Getting and Setting Resource Fork Attributes, 146
    pub fn attributes(&self) -> u16 {
        self.attributes
    }
}

#[derive(Clone, Debug)]
pub struct ResourceMetadata {
    resource_type: ResourceType,
    id: u16,
    name: Option<String>,
    attributes: u8,
    data_offset: u32,
}

impl ResourceMetadata {
    pub fn resource_type(&self) -> ResourceType {
        self.resource_type
    }

    pub fn id(&self) -> u16 {
        self.id
    }

    pub fn name(&self) -> Option<&String> {
        self.name.as_ref()
    }

    // The Resource Map, page 38
    pub fn attributes(&self) -> u8 {
        self.attributes
    }
}

#[derive(Debug)]
pub enum ResourceError {
    IoError,
    NotFound,
    CorruptResourceMap,
    CorruptResourceData,
}

impl From<std::io::Error> for ResourceError {
    fn from(_: Error) -> Self {
        // TODO
        ResourceError::IoError
    }
}

struct Header {
    data_offset: u32,
    map_offset: u32,
    _data_len: u32,
    map_len: u32,
}

impl From<[u8; 16]> for Header {
    fn from(bytes: [u8; 16]) -> Self {
        let (data_offset_bytes, bytes) = bytes.split_at(std::mem::size_of::<u32>());
        let (map_offset_bytes, bytes) = bytes.split_at(std::mem::size_of::<u32>());
        let (data_len_bytes, map_len_bytes) = bytes.split_at(std::mem::size_of::<u32>());

        Header {
            data_offset: u32::from_be_bytes(data_offset_bytes.try_into().unwrap()),
            map_offset: u32::from_be_bytes(map_offset_bytes.try_into().unwrap()),
            _data_len: u32::from_be_bytes(data_len_bytes.try_into().unwrap()),
            map_len: u32::from_be_bytes(map_len_bytes.try_into().unwrap()),
        }
    }
}

struct TypeListEntry {
    resource_type: ResourceType,
    count: u16,
    reference_list_offset: u16,
}

impl From<[u8; 8]> for TypeListEntry {
    fn from(bytes: [u8; 8]) -> Self {
        TypeListEntry {
            resource_type: bytes[0..4].try_into().unwrap(),
            count: u16::from_be_bytes(bytes[4..6].try_into().unwrap()) + 1,
            reference_list_offset: u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
        }
    }
}

struct ReferenceListEntry {
    id: u16,
    name_list_offset: u16,
    attributes: u8,
    data_offset: u32,
}

impl From<[u8; 12]> for ReferenceListEntry {
    fn from(bytes: [u8; 12]) -> Self {
        let mut offset_bytes = vec![0; 4];
        offset_bytes[1..4].copy_from_slice(&bytes[5..8]);

        ReferenceListEntry {
            id: u16::from_be_bytes(bytes[0..2].try_into().unwrap()),
            name_list_offset: u16::from_be_bytes(bytes[2..4].try_into().unwrap()),
            attributes: bytes[4],
            data_offset: u32::from_be_bytes(offset_bytes.try_into().unwrap()) & 0x00ffffff,
            // Last four bytes of entry are reserved/unused
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Cursor;

    const RSRC_DATA: &[u8] = include_bytes!("string-table.rsrc");

    #[test]
    fn load_resource() -> Result<(), ResourceError> {
        let mut resource_fork = ResourceFork::new(Cursor::new(RSRC_DATA))?;

        assert_eq!(1, resource_fork.resources().count());

        let mut resource_data = vec![];

        let metadata = resource_fork.load_by_id(
            ResourceType::try_from("STR#").unwrap(),
            777,
            &mut resource_data,
        )?;

        assert!(resource_data.len() > 0);
        assert_eq!("STR#", metadata.resource_type);
        assert_eq!(777, metadata.id);
        assert_eq!(Some(String::from("Example")), metadata.name);

        resource_data.clear();

        let metadata = resource_fork.load_by_name(
            ResourceType::try_from("STR#").unwrap(),
            String::from("Example"),
            &mut resource_data,
        )?;
        assert!(resource_data.len() > 0);
        assert_eq!("STR#", metadata.resource_type);
        assert_eq!(777, metadata.id);
        assert_eq!(Some(String::from("Example")), metadata.name);

        Ok(())
    }

    #[test]
    fn resource_type_from_slice() {
        let bytes = b"__snd __";

        assert_eq!(
            ResourceType::from(*b"snd "),
            ResourceType::try_from(&bytes[2..6]).unwrap()
        );

        assert_eq!(
            ResourceTypeError::BadEncodedLength(3),
            ResourceType::try_from(&bytes[2..5]).unwrap_err()
        );
    }

    #[test]
    fn resource_type_from_str() {
        assert_eq!(
            ResourceType::from(*b"snd "),
            ResourceType::try_from("snd ").unwrap()
        );

        assert_eq!(
            ResourceTypeError::BadInitialLength(5),
            ResourceType::try_from("OH NO").unwrap_err()
        );

        assert_eq!(
            ResourceTypeError::IllegalCharacters,
            ResourceType::try_from("🤘").unwrap_err()
        );
    }

    #[test]
    fn resource_type_to_string() {
        assert_eq!(
            String::from("snd "),
            String::from(ResourceType::from(*b"snd "))
        );
    }

    #[test]
    fn resource_partial_equal_string() {
        assert_eq!(String::from("snd "), ResourceType::from(*b"snd "));
        assert_ne!(String::from("nope"), ResourceType::from(*b"snd "));
    }

    #[test]
    fn resource_partial_equal_str() {
        assert_eq!("snd ", ResourceType::from(*b"snd "));
        assert_ne!("nope", ResourceType::from(*b"snd "));
    }

    #[test]
    fn resource_partial_equal_slice() {
        let bytes = b"__snd __";

        assert_eq!(bytes[2..6], ResourceType::from(*b"snd "));
        assert_ne!(bytes[2..8], ResourceType::from(*b"snd "));
    }
}
