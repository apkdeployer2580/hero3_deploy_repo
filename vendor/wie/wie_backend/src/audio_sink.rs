pub trait AudioSink: Sync + Send {
    fn play_wave(&self, channel: u8, sampling_rate: u32, wave_data: &[i16]);
    fn midi_note_on(&self, channel_id: u8, note: u8, velocity: u8);
    fn midi_note_off(&self, channel_id: u8, note: u8, velocity: u8);
    fn midi_program_change(&self, channel_id: u8, program: u8);
    fn midi_control_change(&self, channel_id: u8, control: u8, value: u8);
    fn midi_pitch_bend(&self, channel_id: u8, value: u16);
    fn midi_sysex(&self, data: &[u8]);

    /// Set the level of the MUSIC (MIDI) bus, `0..=100`, as the guest's
    /// `MC_mdaClipSetVolume` gave it. Returns `true` if the sink applied it as an
    /// **output gain on rendered audio** — in which case [`SmafPlayer`] must NOT also
    /// scale note velocity, because that would apply the level twice.
    ///
    /// ★ 2026-08-16, and it is the fix for "배경음이 최대 볼륨에서도 작다" +
    /// "배경음 3단계와 효과음 3단계의 음량이 너무 차이난다".
    ///
    /// Hero3's two buses are physically different SMAF chunk types — BGM is `MTR`
    /// (a MIDI score, rendered by the host synth) and SFX is `ATR` (Yamaha ADPCM,
    /// played as PCM). The player used to apply the game's volume to each in its own
    /// domain: PCM was scaled LINEARLY on the samples, but MIDI was applied by
    /// multiplying NOTE VELOCITY. Those are not the same law and not even close:
    ///
    ///   * velocity -> loudness runs through the SoundFont's concave
    ///     velocity-to-attenuation modulator, so a 0.6x velocity is roughly a 0.36x
    ///     amplitude — twice the dB drop that 0.6x on PCM gives; and
    ///   * velocity also SELECTS the sample layer and moves the filter cutoff, so a
    ///     quieter setting made the music duller, not just softer.
    ///
    /// Measured on this game's own assets (`GeneralUser.sf3`, 20 s renders, both
    /// sliders at the default level 3): BGM landed at **-42 to -48 dBFS RMS** while
    /// SFX sat at **-8 to -24 dBFS RMS**. A 27-39 dB gap from one setting.
    ///
    /// With a host music bus the level is instead applied to the synth's OUTPUT with
    /// [`volume_gain`] — the same curve the PCM bus uses — so both buses answer to
    /// their slider identically and the notes keep their intended timbre.
    ///
    /// Sinks that hand MIDI to an external synth (`wie_cli`'s midir, `wie_web`'s
    /// WebMIDI) have no bus to scale, so they keep the default `false` and the old
    /// velocity path, unchanged.
    fn set_music_volume(&self, _volume: u8) -> bool {
        false
    }
}

/// The one definition of "what does volume level N sound like", shared by both buses.
///
/// The guest sends `0..=100` (Hero3's settings are 0-5 steps, `level * 20`). A linear
/// map spends almost all of its range in the top step (-14/-8/-4.4/-1.9/0 dB), which is
/// why "3단계" of the PCM bus was still nearly full volume. Squaring it gives the taper
/// a 5-step slider wants — -28/-16/-8.9/-3.9/0 dB — while keeping 0 silent and 100 unity.
pub fn volume_gain(volume: u8) -> f32 {
    let v = volume.min(100) as f32 / 100.0;
    v * v
}

#[cfg(test)]
mod tests {
    use super::volume_gain;

    #[test]
    fn silent_at_zero_and_unity_at_full() {
        assert_eq!(volume_gain(0), 0.0);
        assert_eq!(volume_gain(100), 1.0);
        // The guest is clamped by `clip_set_volume`, but never trust that here.
        assert_eq!(volume_gain(255), 1.0);
    }

    #[test]
    fn hero3_five_steps_are_monotonic_and_evenly_tapered() {
        // level*20, i.e. the values `MC_mdaClipSetVolume` actually receives.
        let steps: [f32; 6] = [0, 20, 40, 60, 80, 100].map(volume_gain);
        for w in steps.windows(2) {
            assert!(w[1] > w[0], "levels must increase: {w:?}");
        }
        // Default setting (level 3) is a real, audible cut, not the ~-4 dB the old
        // linear PCM law gave.
        let db = 20.0 * steps[3].log10();
        assert!((db + 8.87).abs() < 0.01, "level 3 should be ~-8.9 dB, got {db}");
    }
}
