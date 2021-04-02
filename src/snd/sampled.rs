use crate::snd::{Frequency, SoundError};
use std::convert::{TryFrom, TryInto};
use std::ops::Range;

#[derive(Debug)]
pub struct SampledSound {
    sample_rate: Frequency,
    loop_range: Option<Range<u32>>,
    base_frequency: u8,
    samples: Vec<u8>,
}

impl TryFrom<&[u8]> for SampledSound {
    type Error = SoundError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        const MIN_HEADER_LENGTH: usize = 22;

        if bytes.len() < MIN_HEADER_LENGTH {
            return Err(SoundError::CorruptResource);
        }

        let sample_ptr = u32::from_be_bytes(bytes[0..4].try_into().unwrap());
        let len = u32::from_be_bytes(bytes[4..8].try_into().unwrap());
        let sample_rate = Frequency::from_be_bytes(bytes[8..12].try_into().unwrap());
        let loop_start = u32::from_be_bytes(bytes[12..16].try_into().unwrap());
        let loop_end = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
        let encoding = bytes[20];
        let base_frequency = bytes[21];

        if sample_ptr != 0 {
            return Err(SoundError::UnresolveablePointer);
        }

        if encoding != 0 {
            return Err(SoundError::UnsupportedSoundFormat(encoding));
        }

        let loop_range = if loop_start == 0 && loop_end == 0 {
            None
        } else {
            Some(Range {
                start: loop_start,
                end: loop_end,
            })
        };

        if bytes.len() < MIN_HEADER_LENGTH + len as usize {
            return Err(SoundError::CorruptResource);
        }

        let samples: Vec<u8> =
            Vec::from(&bytes[MIN_HEADER_LENGTH..MIN_HEADER_LENGTH + len as usize]);

        Ok(SampledSound {
            sample_rate,
            loop_range,
            base_frequency,
            samples,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::snd::RATE_22_KHZ;
    use std::convert::TryFrom;

    #[test]
    fn load() {
        let bytes = include_bytes!("test.snd");
        let sound = SampledSound::try_from(&bytes[20..]).unwrap();

        assert_eq!(RATE_22_KHZ, sound.sample_rate);
        assert_eq!(60, sound.base_frequency);
    }
}
