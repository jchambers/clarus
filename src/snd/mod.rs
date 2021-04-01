mod command;

use crate::snd::command::SoundCommand;
use crate::snd::DataFormat::SquareWave;
use std::convert::{TryFrom, TryInto};

pub type Frequency = fixed::types::U16F16;

#[derive(Debug)]
pub struct SndResource {
    resource_format: ResourceFormat,
    data_formats: Vec<DataFormat>,
    commands: Vec<SoundCommand>,
    sound_data: Vec<u8>,
}

impl TryFrom<&[u8]> for SndResource {
    type Error = SoundError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        // Even if everything else is empty, we need at least six bytes (2 for the resource format,
        // 2 for the number of data formats, and 2 for the number of commands).
        const MIN_RESOURCE_LEN: usize = 6;

        if bytes.len() < MIN_RESOURCE_LEN {
            return Err(SoundError::CorruptResource);
        }

        let resource_format =
            ResourceFormat::try_from(u16::from_be_bytes(bytes[0..2].try_into().unwrap())).unwrap();

        let (data_formats, command_length_offset) = match resource_format {
            ResourceFormat::Snd1 => {
                let data_format_count =
                    u16::from_be_bytes(bytes[2..4].try_into().unwrap()) as usize;

                // Do we have enough data to plausibly contain all the given number of data formats?
                if bytes.len() < MIN_RESOURCE_LEN + (data_format_count * 6) as usize {
                    return Err(SoundError::CorruptResource);
                }

                let mut data_formats = vec![];

                for offset in (4..4 + (data_format_count * 6)).step_by(6) {
                    data_formats.push(DataFormat::try_from(
                        TryInto::<&[u8; 6]>::try_into(&bytes[offset..offset + 6]).unwrap(),
                    )?);
                }

                (data_formats, 4 + (6 * data_format_count))
            }
            ResourceFormat::Snd2 => (vec![], 6),
        };

        let command_count = u16::from_be_bytes(
            bytes[command_length_offset..command_length_offset + 2]
                .try_into()
                .unwrap(),
        ) as usize;

        // Do we have enough data to plausibly contain the specified number of commands?
        if bytes.len() < command_length_offset + (command_count * 8) {
            return Err(SoundError::CorruptResource);
        }

        let commands_offset = command_length_offset + 2;
        let mut commands = vec![];

        for offset in (commands_offset..commands_offset + (command_count * 8)).step_by(8) {
            let command_bytes: &[u8; 8] = bytes[offset..offset + 8].try_into().unwrap();
            commands.push(SoundCommand::try_from(command_bytes)?);
        }

        Ok(SndResource {
            resource_format,
            data_formats,
            commands,
            sound_data: vec![],
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum ResourceFormat {
    Snd1,
    Snd2,
}

impl TryFrom<u16> for ResourceFormat {
    type Error = SoundError;

    fn try_from(format_id: u16) -> Result<Self, Self::Error> {
        match format_id {
            1 => Ok(ResourceFormat::Snd1),
            2 => Ok(ResourceFormat::Snd2),
            id => Err(SoundError::IllegalResourceFormat(id)),
        }
    }
}

#[derive(Debug)]
pub enum DataFormat {
    SquareWave(u32),
    WaveTable(u32),
    SampledSound(u32),
}

impl TryFrom<&[u8; 6]> for DataFormat {
    type Error = SoundError;

    fn try_from(bytes: &[u8; 6]) -> Result<Self, Self::Error> {
        let format_id = u16::from_be_bytes(bytes[0..2].try_into().unwrap());
        let init_params = u32::from_be_bytes(bytes[2..6].try_into().unwrap());

        let format = match format_id {
            1 => DataFormat::SquareWave(init_params),
            3 => DataFormat::WaveTable(init_params),
            5 => DataFormat::SampledSound(init_params),
            id => return Err(SoundError::IllegalDataFormat(id)),
        };

        Ok(format)
    }
}

struct SoundHeader {
    len: u32,
    sample_rate: Frequency,
    loop_start: u32,
    loop_end: u32,
    encode: u8,
    base_frequency: u8,
    samples: Vec<u8>,
}

#[derive(Debug)]
pub enum SoundError {
    IllegalResourceFormat(u16),
    IllegalDataFormat(u16),
    IllegalCommand(u16),
    IllegalParameter {
        command: u16,
        param1: u16,
        param2: u32,
    },
    CorruptResource,
}

#[cfg(test)]
mod test {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn test() {
        let bytes = include_bytes!("test.snd");

        let snd = SndResource::try_from(&bytes[..]).unwrap();

        assert_eq!(ResourceFormat::Snd1, snd.resource_format);
    }
}
