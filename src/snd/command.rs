use crate::snd::sampled::SampledSound;
use fixed::types::U16F16;

/// A Sound Manager command that might be found in an `'snd '` resource.
///
/// Sound Manager supported a range of sound commands. While the majority of Sound Manager commands
/// control and modify sound playback, some were used to query system capabilities/state and others
/// were used to load data from specific parts of memory. The latter two classes of commands
/// did not appear in `'snd '` resources (since there was no way for a static resource to know where
/// to return responses to queries or to know the layout of a running system's memory) and are
/// excluded from this enumeration.
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
    Wait { duration: u16 },

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
    Sync { count: u16, identifier: u32 },

    /// Play the specified note (given as a MIDI note value) for the specified duration in units of
    /// half-milliseconds.
    FreqDuration { note: u8, duration: u16 },

    /// Rest a channel for the specified duration in units of half-milliseconds.
    Rest { duration: u16 },

    /// Change the frequency/pitch of a sound to the given note (specified as a MIDI note value), or
    /// start playing at the given frequency if no sound is currently playing.
    Freq { note: u8 },

    /// Change the amplitude of the currently-playing sound or of the next sound to be played if no
    /// sound is currently playing.
    Amp { amplitude: u8 },

    /// Change the timbre (or tone) of a sound currently being defined using square-wave data. A
    /// timbre value of 0 produces a clear tone; a timbre value of 254 produces a buzzing tone.
    /// Only applicable to square-wave sounds.
    Timbre { timbre: u8 },

    /// Install a wave table as a voice in the configured channel. TODO: Is the "pointer" to a
    /// location in memory, or can it be to an offset in the resource if the "offset bit" is set?
    /// The docs are unclear.
    WaveTable { len: u16 },

    /// Install a sampled sound as a voice in a channel.
    Sound { sound: SampledSound },

    /// Play a buffer of sampled-sound data.
    Buffer { sound: SampledSound },

    /// Set the rate of a sampled sound that is currently playing, effectively altering its pitch
    /// and duration. A rate of 0 to pauses a sampled sound that is playing. The rate is given as a
    /// multiplier of 22 kHz; to set the rate to 44 kHz, for example, use a multiplier of 2.0. Only
    /// applies to sampled sounds.
    Rate { multiplier: U16F16 },
}
