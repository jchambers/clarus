mod command;
mod sampled;

pub use crate::snd::command::SoundCommand;
pub use crate::snd::sampled::SampledSound;
use fixed::types::U16F16;
use std::convert::{TryFrom, TryInto};

pub type Frequency = fixed::types::U16F16;

pub const RATE_44_KHZ: Frequency = Frequency::from_bits(0xac440000);
pub const RATE_22_KHZ: Frequency = Frequency::from_bits(0x56ee8ba3);
pub const RATE_11_KHZ: Frequency = Frequency::from_bits(0x2b7745d1);

/// A `'snd '` resource represents a sound, and `SndResource` instances describe the contents of a
/// single `'snd '` resource.
///
/// Sounds are constructed from one or more commands, and may be produced procedurally or using
/// sampled data.
#[derive(Debug)]
pub struct SndResource {
    resource_format: ResourceFormat,
    data_formats: Vec<DataFormat>,
    commands: Vec<SoundCommand>,
}

impl SndResource {
    /// Returns the format of this sound resource. [`ResourceFormat::Snd2`] was used primarily for
    /// HyperCard sounds and was considered obsolete/deprecated by [`ResourceFormat::Snd1`] by 1994
    /// at the latest.
    pub fn resource_format(&self) -> ResourceFormat {
        self.resource_format
    }

    /// Returns the "data formats" described by this sound resource. May be empty for `Snd1`
    /// resources and is always empty for `Snd2` resources.
    pub fn data_formats(&self) -> &Vec<DataFormat> {
        &self.data_formats
    }

    /// Returns the list of commands contained in this sound resource.
    pub fn commands(&self) -> &Vec<SoundCommand> {
        &self.commands
    }
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

            let command_id = u16::from_be_bytes(command_bytes[0..2].try_into().unwrap());
            let param1 = u16::from_be_bytes(command_bytes[2..4].try_into().unwrap());
            let param2 = u32::from_be_bytes(command_bytes[4..8].try_into().unwrap());
            let offset_bit_set = command_id & 0x8000 != 0;

            let command = match command_id & 0x7fff {
                0 => SoundCommand::Null,
                3 => SoundCommand::Quiet,
                4 => SoundCommand::Flush,
                10 => SoundCommand::Wait(param1),
                11 => SoundCommand::Pause,
                12 => SoundCommand::Resume,
                13 => SoundCommand::Callback(param1, param2),
                14 => SoundCommand::Sync {
                    identifier: param2,
                    count: param1,
                },
                40 => {
                    if param2 > 127 {
                        return Err(SoundError::IllegalParameter {
                            command: 40,
                            param1,
                            param2,
                        });
                    }

                    SoundCommand::FreqDuration {
                        note: param2 as u8,
                        duration: param1,
                    }
                }
                41 => SoundCommand::Rest(param1),
                42 => {
                    if param2 > 127 {
                        return Err(SoundError::IllegalParameter {
                            command: 42,
                            param1,
                            param2,
                        });
                    }

                    SoundCommand::Freq(param2 as u8)
                }
                43 => {
                    if param1 <= 255 {
                        SoundCommand::Amp(param1 as u8)
                    } else {
                        return Err(SoundError::IllegalParameter {
                            command: 43,
                            param1,
                            param2,
                        });
                    }
                }
                44 => {
                    // Yes, less than. For whatever reason, timbre is bounded between 0 and 254,
                    // inclusive.
                    if param1 < 255 {
                        SoundCommand::Timbre(param1 as u8)
                    } else {
                        return Err(SoundError::IllegalParameter {
                            command: 44,
                            param1,
                            param2,
                        });
                    }
                }
                60 => {
                    if !offset_bit_set {
                        return Err(SoundError::UnresolveablePointer);
                    }

                    let len = param1 as usize;
                    let offset = param2 as usize;

                    if bytes.len() < offset + len {
                        return Err(SoundError::CorruptResource);
                    }

                    SoundCommand::WaveTable(Vec::from(&bytes[offset..offset + len]))
                }
                80 => {
                    if !offset_bit_set {
                        return Err(SoundError::UnresolveablePointer);
                    }

                    SoundCommand::Sound(SampledSound::try_from(&bytes[param2 as usize..])?)
                }
                81 => {
                    if !offset_bit_set {
                        return Err(SoundError::UnresolveablePointer);
                    }

                    SoundCommand::Buffer(SampledSound::try_from(&bytes[param2 as usize..])?)
                }
                82 => SoundCommand::Rate(U16F16::from_bits(param2)),
                id => return Err(SoundError::IllegalCommand(id)),
            };

            commands.push(command);
        }

        Ok(SndResource {
            resource_format,
            data_formats,
            commands,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    UnresolveablePointer,
    UnsupportedSoundFormat(u8),
    CorruptResource,
}

#[cfg(test)]
mod test {
    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn load_sound() {
        let bytes = include_bytes!("test.snd");

        let snd = SndResource::try_from(&bytes[..]).unwrap();

        assert_eq!(ResourceFormat::Snd1, snd.resource_format());

        assert_eq!(1, snd.data_formats().len());
        assert!(matches!(
            &snd.data_formats()[0],
            DataFormat::SampledSound(_)
        ));

        assert_eq!(1, snd.commands().len());

        if let SoundCommand::Buffer(ref sampled_sound) = snd.commands()[0] {
            assert_eq!(RATE_22_KHZ, sampled_sound.sample_rate());
            assert_eq!(60, sampled_sound.base_frequency());
            assert!(!sampled_sound.samples().is_empty());
        } else {
            panic!("Unexpected sound command");
        }
    }
}
