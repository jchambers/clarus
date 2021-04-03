use crate::snd::sampled::SampledSound;
use crate::snd::Frequency;
use fixed::types::U16F16;

/// A Sound Manager command that might be found in an `'snd '` resource.
///
/// TODO Explain that this is a subset of all Sound Manager commands
#[derive(Debug)]
pub enum SoundCommand {
    /// Do nothing.
    Null,

    /// Stop the sound that is currently playing.
    Quiet,

    /// Remove all commands currently queued in the specified sound channel; does not affect any
    /// sound that is currently in progress.
    Flush,

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

    WaveTable {
        len: u16,
    },

    Sound {
        sound: SampledSound,
    },

    Buffer {
        sound: SampledSound,
    },

    Rate {
        multiplier: U16F16,
    },
}
