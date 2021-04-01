use crate::snd::{Frequency, SoundError};
use fixed::types::U16F16;
use std::convert::{TryFrom, TryInto};

/// A Sound Manager command.
///
/// Sound Manager used the same set of commands and data structures in several contexts (resources,
/// imperative state change commands, and imperative query commands). Many commands relied on
/// 32-bit pointers to locations in memory; in other cases, Sound Manager would write return values
/// into parts of command data structures.
///
/// In general, the purpose of this crate is to provide tools to read `'snd '` resources rather than
/// to actually replicate the functionality of Sound Manager. In that spirit, placeholders for
/// pointers and return values have been omitted from commands. Most of the commands that had
/// pointers or return value "slots" wouldn't make sense in the context of sound resources in any
/// case.
#[derive(Debug)]
pub enum SoundCommand {
    /// Do nothing.
    Null,

    /// Stop the sound that is currently playing.
    Quiet,

    /// Remove all commands currently queued in the specified sound channel; does not affect any
    /// sound that is currently in progress.
    Flush,

    /// Reset the initialization parameters for the specified channel.
    ReInit {
        params: u32,
    },

    /// Suspend further command processing in a channel until the specified duration has elapsed.
    /// The duration is given in units of half-milliseconds.
    Wait {
        duration: u16,
    },

    /// Pause any further command processing in a channel until a [`SoundCommand::Resume`] is
    /// received.
    Pause,

    /// Resume command processing in a channel that was previously paused by
    /// [`SoundCommand::Pause`].
    Resume,

    /// Execute the callback function specified as a parameter when creating a new channel. Both
    /// parameters are application-specific and will be passed as arguments to the callback
    /// function.
    Callback(u16, u32),

    /// Synchronize multiple channels of sound. The `identifier` is an arbitrary,
    /// application-selected value. Every time a `Sync` command is executed, the `count` for all
    /// channels with that identifier is decremented. Channels resume processing commands when
    /// `count` reaches zero.
    Sync {
        count: u16,
        identifier: u32,
    },

    /// Tests whether Sound Manager supports the given initialization options. Sound Manager would
    /// write the result to a part of the command data structure.
    Available {
        params: u32,
    },

    /// Determines which version of a sound data format is available. Sound Manager would write the
    /// result to a part of the command data structure.
    Version,

    /// Determine the total CPU load factor for all existing sound activity and for a new sound
    /// channel with the given initialization parameters. Sound Manager would write the result to a
    /// part of the command data structure.
    TotalLoad {
        params: u32,
    },

    /// Determine the CPU load factor for a new sound channel with the given initialization
    /// parameters. Sound Manager would write the result to a part of the command data structure.
    Load {
        params: u32,
    },

    /// Play the specified note for the specified duration in units of half-milliseconds.
    FreqDuration {
        note: Frequency,
        duration: u16,
    },

    // Rest a channel for the specified duration in units of half-milliseconds.
    Rest {
        duration: u16,
    },
    Freq {
        frequency: Frequency,
    },
    Amp {
        amplitude: u8,
    },
    Timbre {
        timbre: u8,
    },
    GetAmp,
    Volume {
        left: u16,
        right: u16,
    },
    GetVolume,
    WaveTable {
        len: u16,
    },
    Sound {
        offset: Option<u32>,
    },
    Buffer {
        offset: Option<u32>,
    },
    Rate {
        multiplier: U16F16,
    },
    GetRate,
}

impl TryFrom<&[u8; 8]> for SoundCommand {
    type Error = SoundError;

    fn try_from(bytes: &[u8; 8]) -> Result<Self, Self::Error> {
        let command_id = u16::from_be_bytes(bytes[0..2].try_into().unwrap());
        let param1 = u16::from_be_bytes(bytes[2..4].try_into().unwrap());
        let param2 = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
        let offset_bit_set = command_id & 0x8000 != 0;

        let command = match command_id & 0x7fff {
            0 => SoundCommand::Null,
            3 => SoundCommand::Quiet,
            4 => SoundCommand::Flush,
            5 => SoundCommand::ReInit { params: param2 },
            10 => SoundCommand::Wait { duration: param1 },
            11 => SoundCommand::Pause,
            12 => SoundCommand::Resume,
            13 => SoundCommand::Callback(param1, param2),
            14 => SoundCommand::Sync {
                identifier: param2,
                count: param1,
            },
            24 => SoundCommand::Available { params: param2 },
            25 => SoundCommand::Version,
            26 => SoundCommand::TotalLoad { params: param2 },
            27 => SoundCommand::Load { params: param2 },
            40 => SoundCommand::FreqDuration {
                note: Frequency::from_bits(param2),
                duration: param1,
            },
            41 => SoundCommand::Rest { duration: param1 },
            42 => SoundCommand::Freq {
                frequency: Frequency::from_bits(param2),
            },
            43 => {
                if param1 <= 255 {
                    SoundCommand::Amp {
                        amplitude: (param1 & 0x00ff) as u8,
                    }
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
                    SoundCommand::Timbre {
                        timbre: (param1 & 0x00ff) as u8,
                    }
                } else {
                    return Err(SoundError::IllegalParameter {
                        command: 44,
                        param1,
                        param2,
                    });
                }
            }
            45 => SoundCommand::GetAmp,
            46 => SoundCommand::Volume {
                left: (param2 & 0x0000ffff) as u16,
                right: (param2 & 0xffff0000 >> 16) as u16,
            },
            47 => SoundCommand::GetVolume,
            60 => SoundCommand::WaveTable { len: param1 },
            80 => SoundCommand::Sound {
                offset: if offset_bit_set { Some(param2) } else { None },
            },
            81 => SoundCommand::Buffer {
                offset: if offset_bit_set { Some(param2) } else { None },
            },
            82 => SoundCommand::Rate {
                multiplier: U16F16::from_bits(param2),
            },
            85 => SoundCommand::GetRate,
            id => return Err(SoundError::IllegalCommand(id)),
        };

        Ok(command)
    }
}
