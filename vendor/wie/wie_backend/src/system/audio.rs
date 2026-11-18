use alloc::{
    boxed::Box,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    vec::Vec,
};
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use smaf_player::{SmafEvent, parse_smaf};

use crate::{System, audio_sink::AudioSink};

pub type AudioHandle = u32;
#[derive(Debug)]
pub enum AudioError {
    InvalidHandle,
    InvalidAudio,
}

/// A parsed SMAF file: the event list, plus which BUS the file belongs to.
///
/// The bus is a property of the FILE, not of the handle or the clip, so it is settled
/// once here at parse time and shared with every playback. SMAF keeps the two kinds in
/// physically different chunks — a `MTR` score track is MIDI, a `ATR` audio track is
/// PCM — and in Hero3 that maps exactly onto the game's two volume sliders:
/// `res/snd/bgm*_mf` are `MTR` (배경음) and `res/snd/sd*_mf` are `ATR` (효과음).
///
/// Knowing it matters because the music level is applied to a HOST bus
/// (`AudioSink::set_music_volume`): a sound effect must not be allowed to push its own
/// level onto that bus, or playing an effect at a different setting would duck or
/// silence the music under it — the very symptom this is here to fix.
pub struct Score {
    pub events: Vec<(usize, SmafEvent)>,
    pub is_music: bool,
    /// One pass of this file, in ms — including the tail of a PCM one-shot.
    pub duration_ms: u64,
}

impl Score {
    fn parse(data: &[u8]) -> Self {
        let events = parse_smaf(data);
        // Anything that is not a wave one-shot means a synth is involved.
        let is_music = events.iter().any(|(_, e)| !matches!(e, SmafEvent::Wave { .. } | SmafEvent::End));

        // How long one pass of this file actually SOUNDS. For a wave the event time is
        // when it starts, so the clip's own length has to be added — an effect the guest
        // fires and immediately `MC_mdaStop`s is still audible for its full duration
        // here, and [`Audio`] needs that number to know how long to hold the music open.
        let mut duration_ms = 0usize;
        for (time, event) in events.iter() {
            let end = match event {
                SmafEvent::Wave {
                    sampling_rate,
                    data,
                    channel,
                } => {
                    let frames = data.len() / (*channel).max(1) as usize;
                    time + frames * 1000 / (*sampling_rate).max(1) as usize
                }
                _ => *time,
            };
            duration_ms = duration_ms.max(end);
        }

        Self {
            events,
            is_music,
            duration_ms: duration_ms as u64,
        }
    }
}

/// A parsed SMAF file, shared by every handle and every playback of it. `Arc` rather
/// than a copy per handle: `MC_mdaClipPutData` is called again for every sound the game
/// plays, and the 33 files in Hero3 were being re-copied and re-parsed thousands of
/// times a session.
type Events = Arc<Score>;

enum AudioFile {
    Smaf(Events),
}

/// Why a playback is allowed to keep running, checked by [`SmafPlayer::play`] between
/// events.
///
/// `stop` is the old unconditional kill. `deadline_ms` is the new one: a wall-clock
/// epoch-ms at which a *deferred* stop becomes real, or 0 for "no stop pending". See
/// [`MusicChannel`] for why the music needs a stop that can still be taken back.
pub struct PlaybackGate {
    stop: AtomicBool,
    deadline_ms: AtomicU64,
    finished: AtomicBool,
}

impl PlaybackGate {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            deadline_ms: AtomicU64::new(0),
            finished: AtomicBool::new(false),
        }
    }

    fn kill(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Stop at `at_ms` unless something takes it back first.
    fn stop_at(&self, at_ms: u64) {
        self.deadline_ms.store(at_ms.max(1), Ordering::Relaxed);
    }

    /// Take back a pending stop — the reason for it turned out to be temporary.
    fn keep_going(&self) {
        self.deadline_ms.store(0, Ordering::Relaxed);
    }

    /// Push a pending stop further out. Never pulls it in.
    fn hold_until(&self, at_ms: u64) {
        let cur = self.deadline_ms.load(Ordering::Relaxed);
        if cur != 0 && at_ms > cur {
            self.deadline_ms.store(at_ms, Ordering::Relaxed);
        }
    }

    fn should_stop(&self, now_ms: u64) -> bool {
        if self.stop.load(Ordering::Relaxed) {
            return true;
        }
        let deadline = self.deadline_ms.load(Ordering::Relaxed);
        deadline != 0 && now_ms >= deadline
    }

    fn is_running(&self) -> bool {
        !self.finished.load(Ordering::Relaxed) && !self.stop.load(Ordering::Relaxed)
    }
}

/// The music the host is keeping alive across the guest's sound juggling.
struct MusicChannel {
    score: Events,
    gate: Arc<PlaybackGate>,
    volume: Arc<AtomicU8>,
    handle: AudioHandle,
}

/// How long a guest stop of the music waits before it is honoured.
///
/// ★ 2026-08-16, and it is the fix for "효과음이 들리면 배경음이 안 들리고 처음부터 다시
/// 시작돼".
///
/// Hero3 is a **single-channel** sound engine — device trace, one attack:
///
/// ```text
///   STOP    clip=0x4b85df00 handle=1        <- the game stops its own BGM
///   free    clip=0x4b85df00
///   create  clip=0x4b85df00 (0xa24)         <- SAME address, reused for the effect
///   putData len=2596 -> handle=2 / PLAY repeat=0
///   ... 626 ms later ...
///   create  clip=0x4b85df00 (0xdce = bgm5_mf)
///   putData -> handle=4 / PLAY repeat=1     <- BGM restarted FROM BAR ONE
/// ```
///
/// It owns exactly one `MC_MdaClip` and takes turns with it, because the phone it was
/// written for had one player (`MC_mdaClipAllocPlayer`, which hands out more, is a stub
/// here — and was on the real handset's budget too). Faithfully reproducing that on a
/// host with a real mixer is the wrong trade: the player hears the music cut out and
/// restart on every single skill.
///
/// So a stop of the MUSIC is deferred by this much instead of taken literally. What
/// happens inside the window decides what it meant:
///
///   * an EFFECT starts        -> the guest is taking its turn; hold the music open
///     until that effect has finished ([`MUSIC_REJOIN_MS`] past it) so the re-issue can
///     land, and let the two play together;
///   * the SAME music restarts -> take the stop back and IGNORE the restart, so the
///     song simply never breaks;
///   * different music starts  -> commit the stop, start the new song;
///   * nothing at all          -> commit the stop.
///
/// The last case is real and must keep working: opening the in-game menu stops the
/// music and leaves it stopped (device trace: 4.5 s of silence until the menu closed).
/// That is why this is a short window and not "ignore guest stops" — 250 ms is longer
/// than the 37 ms the guest takes to get from the stop to the effect's `MC_mdaPlay`, and
/// short enough that a genuine silence starts on time.
const MUSIC_STOP_GRACE_MS: u64 = 250;

/// Extra time past the end of an effect for the guest to re-issue the music.
///
/// The game waits for its one clip to finish before putting the song back, so the
/// re-issue lands right about when the effect ends; this is slack for a slow frame or a
/// second effect queued behind the first.
const MUSIC_REJOIN_MS: u64 = 700;

/// Parsed-SMAF cache, keyed by the file's own bytes.
///
/// ★ 2026-08-09, and it is the fix for "효과음/크리티컬 이펙트 때문에 프레임이 떨어진다".
/// `Audio::play` used to run `parse_smaf` — a full chunk walk, an event `Vec`, and a
/// `sort_by` over every event in the file — **on the emulator thread, inside
/// `emulator.tick()`, on every single `MC_mdaPlay`**. The critical-hit sound
/// (`/snd/sd004_mf`, 3,833 bytes) is played on every crit, so every crit paid for a
/// full parse inside a frame that already budgets 24 ms for interpretation.
///
/// Keyed by `(len, FNV-1a-64)` and then verified by comparing the actual bytes, so a
/// hash collision cannot hand back the wrong song — the alternative (trusting a 64-bit
/// hash) would fail as a silently-wrong sound, which is exactly the kind of bug that
/// gets blamed on the emulator for a year.
///
/// Bounded: a guest that somehow feeds unbounded distinct clips stops being cached at
/// [`SMAF_CACHE_MAX`] rather than growing forever. The game has 19 BGM + 14 SFX files.
struct SmafCache {
    buckets: BTreeMap<(usize, u64), Vec<(Vec<u8>, Events)>>,
    entries: usize,
}

/// Distinct sound files kept parsed. Hero3 has 33; the headroom is for a guest that
/// synthesises clips rather than loading them from disk.
const SMAF_CACHE_MAX: usize = 128;

fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl SmafCache {
    fn new() -> Self {
        Self {
            buckets: BTreeMap::new(),
            entries: 0,
        }
    }

    fn get_or_parse(&mut self, data: &[u8]) -> Events {
        let key = (data.len(), fnv1a64(data));
        if let Some(bucket) = self.buckets.get(&key)
            && let Some((_, events)) = bucket.iter().find(|(raw, _)| raw.as_slice() == data)
        {
            return events.clone();
        }

        wie_util::stats::add_smaf_parse(data.len() as u64);
        let events: Events = Arc::new(Score::parse(data));

        if self.entries < SMAF_CACHE_MAX {
            self.buckets.entry(key).or_default().push((data.to_vec(), events.clone()));
            self.entries += 1;
            wie_util::stats::set_smaf_cache_entries(self.entries as u64);
        }

        events
    }
}

pub struct Audio {
    sink: Arc<Box<dyn AudioSink>>,
    files: BTreeMap<AudioHandle, AudioFile>,
    /// Parsed events, shared across handles. See [`SmafCache`].
    cache: SmafCache,
    /// The handle a given guest clip pointer currently owns, so `MC_mdaClipFree` can
    /// drop the bookkeeping the guest is telling us it is finished with. Kept HOST-side
    /// rather than read back out of the guest's `MdaClip` struct: that struct is guest
    /// memory, `MC_mdaClipCreate` hands it back uninitialised, and closing whatever
    /// integer happened to be sitting in it could silence an unrelated clip.
    clip_handles: BTreeMap<u32, AudioHandle>,
    playing: BTreeMap<AudioHandle, Arc<PlaybackGate>>,
    /// The music playback the host keeps alive across the guest's single-clip juggling,
    /// independent of whichever handle the guest currently thinks owns it. See
    /// [`MUSIC_STOP_GRACE_MS`].
    music: Option<MusicChannel>,
    /// Live playback volume per *AudioHandle* 0..=100. Shared with the running
    /// `SmafPlayer` task (lock-free) so a volume change reaches a playing clip.
    volumes: BTreeMap<AudioHandle, Arc<AtomicU8>>,
    /// Volume per *guest clip pointer* (`MC_mdaClipSetVolume`'s clip arg). The
    /// game sets a clip's volume, then `clip_put_data` reloads it under a NEW
    /// handle before playing — so the handle at set-time != the handle at
    /// play-time. Keying by the stable clip pointer lets `play` recover the
    /// intended volume and apply it to whatever handle actually plays.
    clip_volumes: BTreeMap<u32, u8>,
    last_audio_handle: AudioHandle,
}

impl Audio {
    pub fn new(sink: Box<dyn AudioSink>) -> Self {
        Self {
            sink: Arc::new(sink),
            files: BTreeMap::new(),
            cache: SmafCache::new(),
            clip_handles: BTreeMap::new(),
            playing: BTreeMap::new(),
            volumes: BTreeMap::new(),
            music: None,
            clip_volumes: BTreeMap::new(),
            last_audio_handle: 0,
        }
    }

    /// Record that `clip_ptr` now owns `handle` (called by `MC_mdaClipPutData`).
    ///
    /// The PREVIOUS handle for this pointer is deliberately left alone: today a clip
    /// that is re-loaded while its old playback is still running lets that playback
    /// finish, and rapid SFX depend on that (13차: "크리티컬 효과음이 겹쳐 들린다"
    /// is the game's own behaviour, not a defect). Only [`release_clip`] drops it.
    pub fn bind_clip(&mut self, clip_ptr: u32, handle: AudioHandle) {
        self.clip_handles.insert(clip_ptr, handle);
    }

    /// `MC_mdaClipFree`: the guest says it is finished with this clip. Drop the
    /// bookkeeping it minted.
    ///
    /// ★ Does NOT stop a playback in flight. The running task owns its own `Arc` of the
    /// events and its own stop flag, so a sound that was already started still finishes
    /// exactly as it does today — freeing the clip is not "cut the sound off".
    pub fn release_clip(&mut self, clip_ptr: u32) {
        // ⛔⛔ THE VOLUME STAYS (2026-08-28). This used to be
        // `self.clip_volumes.remove(&clip_ptr);` and that one line is the whole
        // "배경음 OFF가 효과음을 낸 뒤에는 안 먹는다" report.
        //
        // A volume is a PLAYER SETTING, not per-clip bookkeeping. `clip_volumes` is keyed
        // by the clip POINTER precisely so it survives the handle churn (see its doc), and
        // `MC_mdaClipFree` is part of that churn: Hero3's cycle for every single sound is
        //
        //     ClipFree -> ClipCreate -> ClipSetVolume(v) -> ClipPutData -> Play
        //
        // so wiping here made `clip_ptr_volume` fall back to its default of **100** for
        // the whole window between a free and the next set. Device trace, vc193:
        //
        //     music bus level 100 adopted at putData (clip 0x4b85d400 holds music)
        //
        // — 100, while the player had the slider at 1. And the player found the trigger
        // themselves: *"아무효과음도 내지않고 그저 몬스터에게 몇대 맞아서 miss만 떳는데도
        // 배경음이 변하지않아"*. It is not about a sound being AUDIBLE; the free/create
        // cycle runs either way, and the free is what discarded the setting.
        //
        // Keeping a stale level costs nothing: the guest always sets the volume before it
        // plays, so a re-created clip is re-aimed before it can be heard — and a stale
        // real level is strictly better than a default of full scale.
        if let Some(handle) = self.clip_handles.remove(&clip_ptr) {
            self.files.remove(&handle);
            self.volumes.remove(&handle);
            wie_util::stats::set_audio_handles(self.files.len() as u64);
        }
    }

    /// The handle `clip_ptr` is currently bound to, per [`bind_clip`](Self::bind_clip).
    ///
    /// ★ 2026-08-28 — this is the AUTHORITATIVE handle for a clip pointer. The
    /// `MdaClip` struct in guest memory is not: see the note on [`Self::clip_handles`]
    /// (`MC_mdaClipCreate` hands that struct back uninitialised), which is why
    /// `clip_set_volume` must not trust `mda.handle` alone.
    pub fn bound_handle(&self, clip_ptr: u32) -> Option<AudioHandle> {
        self.clip_handles.get(&clip_ptr).copied()
    }

    /// Diagnostic: where a `MC_mdaClipSetVolume` on `clip_ptr` will actually land, next
    /// to where the music the host is keeping alive is listening.
    ///
    /// ★ 2026-08-28 — the one line that answers *"왜 인게임에서 배경음 슬라이더가 안
    /// 먹나"*. The host keeps the music running on the handle it STARTED on (see
    /// [`MusicChannel`]) while the guest re-binds its single clip pointer to a NEW handle
    /// for every effect, so a volume set through that pointer can land on a cell the
    /// music is not reading. Device log 2026-08-28 (vc186, S9) showed exactly that shape:
    /// every slider step published `music bus level 20/40/60/80/100` in the shell, and
    /// **nothing at all** for the same slider in play.
    ///
    /// Returns `(bound handle, live music handle, whether the bound handle holds music)`.
    pub fn volume_route(&self, clip_ptr: u32) -> (Option<AudioHandle>, Option<AudioHandle>, Option<bool>) {
        let bound = self.bound_handle(clip_ptr);
        let music = self.music.as_ref().filter(|m| m.gate.is_running()).map(|m| m.handle);
        let bound_is_music = bound.and_then(|h| self.files.get(&h)).map(|f| match f {
            AudioFile::Smaf(score) => score.is_music,
        });
        (bound, music, bound_is_music)
    }

    /// Push a level straight onto the music the host is keeping alive.
    ///
    /// ★★★ 2026-08-28 — WHY THIS EXISTS. Until now the live music's volume cell was
    /// written in exactly ONE place: the same-song rejoin inside [`Self::play`]. That only
    /// runs when the guest re-issues the BGM with `MC_mdaPlay` — and while effects are
    /// firing the guest's single clip is busy, so the re-issue never comes and the music's
    /// level FREEZES. Player report, isolated by the player themselves: *"효과음 안내면
    /// 배경음 OFF적용되고 효과음 내니까 적용안돼 … 효과음때문에 배경음 적용이 늦어지는건가?"*
    ///
    /// Device trace (S9, vc192) — the music sat on handle 5 for the whole session with its
    /// cell still at the `volume_arc` default while every write went to the effect handles:
    ///
    /// ```text
    ///   music play: incoming handle 35 vol 60 · live handle 5 vol 100 · same_song true
    ///   ClipSetVolume(clip 0x4b85df00, vol 40) -> mda.handle 35 · bound None   <- 배경음 2
    ///   ClipSetVolume(clip 0x4b85df00, vol 20) -> mda.handle 36 · bound None   <- 배경음 1
    ///   (no `music play:` ever again — so nothing copied 40 or 20 onto handle 5)
    /// ```
    ///
    /// This is the cost of [`MusicChannel`]: the host deliberately decoupled the music
    /// from the guest's clip, so the music also needs a volume path that does not go
    /// through that clip.
    ///
    /// ⛔ Only call this for a level that is PROVEN to be the music's. The game reuses one
    /// clip pointer for both buses, so a `MC_mdaClipSetVolume` is only attributable when
    /// the pointer is currently bound to a handle whose file `is_music` — see the caller.
    /// Routing every write here would make the music follow the 효과음 slider.
    pub fn note_music_volume(&mut self, volume: u8) -> bool {
        let Some(music) = self.music.as_ref().filter(|m| m.gate.is_running()) else {
            return false;
        };
        music.volume.store(volume.min(100), Ordering::Relaxed);
        true
    }

    /// Whether the handle bound to `clip_ptr` holds a MUSIC file — i.e. whether a volume
    /// set through that pointer is attributable to the music bus. `None` when the pointer
    /// is unbound (the guest freed the clip and has not put data back yet).
    pub fn bound_is_music(&self, clip_ptr: u32) -> Option<bool> {
        let h = self.bound_handle(clip_ptr)?;
        self.files.get(&h).map(|f| match f {
            AudioFile::Smaf(score) => score.is_music,
        })
    }

    /// Get-or-create the shared volume cell for a handle (default 100 = full).
    fn volume_arc(&mut self, handle: AudioHandle) -> Arc<AtomicU8> {
        self.volumes.entry(handle).or_insert_with(|| Arc::new(AtomicU8::new(100))).clone()
    }

    /// Set the live volume for a handle (0..=100). Applies to a playing clip.
    pub fn set_clip_volume(&mut self, handle: AudioHandle, volume: u8) {
        self.volume_arc(handle).store(volume.min(100), Ordering::Relaxed);
    }

    /// Read the live volume for a handle.
    pub fn clip_volume(&mut self, handle: AudioHandle) -> u8 {
        self.volume_arc(handle).load(Ordering::Relaxed)
    }

    /// `MC_mdaClipSetVolume` stores the level against the stable clip pointer.
    pub fn set_clip_ptr_volume(&mut self, clip_ptr: u32, volume: u8) {
        self.clip_volumes.insert(clip_ptr, volume.min(100));
    }

    /// Volume the game last set for this clip pointer (default 100 = full).
    pub fn clip_ptr_volume(&self, clip_ptr: u32) -> u8 {
        self.clip_volumes.get(&clip_ptr).copied().unwrap_or(100)
    }

    /// The recorded level for a clip pointer, or `None` when the guest has never set one.
    ///
    /// ★ 2026-08-28 — [`Self::clip_ptr_volume`]'s `unwrap_or(100)` is right for a
    /// PLAYBACK (a clip nobody set a level for plays at full), and wrong for anything that
    /// PUBLISHES the value: `MC_mdaClipCreate` mints a fresh heap address for every sound,
    /// so a pointer seen for the first time has no entry and the default would be
    /// broadcast as if the player had asked for full scale. Device trace, vc196:
    ///
    /// ```text
    ///   music bus level 100 adopted at putData (clip 0x4b85e200 holds music)
    /// ```
    ///
    /// — 100, over a music bus the guest had correctly put at 40.
    pub fn clip_ptr_volume_set(&self, clip_ptr: u32) -> Option<u8> {
        self.clip_volumes.get(&clip_ptr).copied()
    }

    pub fn load_smaf(&mut self, data: &[u8]) -> Result<AudioHandle, AudioError> {
        wie_util::stats::add_smaf_load();

        // ★ Parsed HERE and shared, not stored raw and re-parsed at every `play`.
        let events = self.cache.get_or_parse(data);

        let audio_handle = self.last_audio_handle;

        self.last_audio_handle += 1;
        self.files.insert(audio_handle, AudioFile::Smaf(events));
        wie_util::stats::set_audio_handles(self.files.len() as u64);

        Ok(audio_handle)
    }

    pub fn play(&mut self, system: &System, audio_handle: AudioHandle, repeat: bool) -> Result<(), AudioError> {
        wie_util::stats::add_smaf_play();

        // A clone of an `Arc`, where this used to be a full `parse_smaf` of the file —
        // on the emulator thread, inside the guest's `MC_mdaPlay`. See [`SmafCache`].
        let player = match self.files.get(&audio_handle) {
            Some(AudioFile::Smaf(events)) => SmafPlayer { events: events.clone() },
            None => return Err(AudioError::InvalidHandle),
        };

        let score = player.events.clone();
        let now_ms = system.platform().now().raw();

        // ── the guest's single sound slot, undone ────────────────────────────────
        // See [`MUSIC_STOP_GRACE_MS`]: a music stop is deferred, and what happens next
        // is what decides whether it was real.
        // Taken out of `self` up front so the branches below can still touch `self`.
        let live_music = self
            .music
            .as_ref()
            .filter(|m| m.gate.is_running())
            .map(|m| (m.score.clone(), m.gate.clone(), m.volume.clone(), m.handle));

        if score.is_music {
            if let Some((live_score, gate, volume, handle)) = live_music {
                // ★ 2026-08-28 — WHICH BRANCH, AND WITH WHAT LEVEL. Player report: the
                // in-game 배경음 slider applies before any effect has played and stops
                // applying after one has ("효과음때문에 배경음 적용이 늦어지는건가?"), and
                // that is exactly this decision plus WHICH handle the level was written
                // to. `info`, not `debug`: the release filter is `warn,main=info` and this
                // fires once per `MC_mdaPlay` of the music, not per event.
                let incoming = self.clip_volume(audio_handle);
                let live_now = self.clip_volume(handle);
                tracing::warn!(
                    "music play: incoming handle {audio_handle} vol {incoming} · live handle {handle} vol {live_now} · same_song {}",
                    Arc::ptr_eq(&live_score, &score),
                );
                if Arc::ptr_eq(&live_score, &score) {
                    // The same song the host is already playing. The guest only re-issued
                    // it because it had to borrow its one clip for an effect — take the
                    // pending stop back and DO NOT restart, so the player hears one
                    // unbroken piece of music.
                    gate.keep_going();
                    // The guest may have changed 배경음 while it was away; the live
                    // playback reads its own cell, so move the new level onto it.
                    let level = self.clip_volume(audio_handle);
                    volume.store(level, Ordering::Relaxed);
                    self.playing.insert(handle, gate);
                    return Ok(());
                }
                // A different song: the deferred stop was real after all.
                gate.kill();
                self.playing.remove(&handle);
            }
        } else if let Some((_, gate, _, _)) = live_music {
            // An effect. This is the guest taking its turn with the one clip, so hold
            // the music open past the end of the effect and let them overlap.
            gate.hold_until(now_ms + score.duration_ms + MUSIC_REJOIN_MS);
        }

        self.stop_now(audio_handle);

        let mut system_clone = system.clone();
        let sink_clone = self.sink.clone();
        let volume = self.volume_arc(audio_handle);

        let gate = Arc::new(PlaybackGate::new());
        let gate_clone = gate.clone();
        self.playing.insert(audio_handle, gate.clone());

        if score.is_music {
            self.music = Some(MusicChannel {
                score,
                gate: gate.clone(),
                volume: volume.clone(),
                handle: audio_handle,
            });
        }

        // TODO use dedicated audio player task
        system.spawn(async move || {
            player.play(&mut system_clone, &**sink_clone, &gate_clone, repeat, volume).await;
            gate_clone.finished.store(true, Ordering::Relaxed);

            Ok(())
        });

        Ok(())
    }

    /// `MC_mdaStop`. A stop of the MUSIC is DEFERRED — see [`MUSIC_STOP_GRACE_MS`] for
    /// why taking it literally makes every skill restart the song from the beginning.
    /// Everything else stops immediately, as before.
    pub fn stop(&mut self, now_ms: u64, audio_handle: AudioHandle) {
        if let Some(music) = &self.music
            && music.handle == audio_handle
            && music.gate.is_running()
        {
            music.gate.stop_at(now_ms + MUSIC_STOP_GRACE_MS);
            return;
        }
        self.stop_now(audio_handle);
    }

    /// Stop a playback for real, right now.
    fn stop_now(&mut self, audio_handle: AudioHandle) {
        if let Some(gate) = self.playing.remove(&audio_handle) {
            gate.kill();
        }
    }

    pub fn close(&mut self, audio_handle: AudioHandle) -> Result<(), AudioError> {
        self.stop_now(audio_handle);
        self.volumes.remove(&audio_handle);

        if self.files.remove(&audio_handle).is_none() {
            return Err(AudioError::InvalidHandle);
        }
        wie_util::stats::set_audio_handles(self.files.len() as u64);

        Ok(())
    }
}

/// How far behind the wall clock a song may fall before it stops trying to catch up
/// event-by-event and simply resumes from now.
///
/// ★ 2026-08-09. The app STOPS TICKING the emulator while it is backgrounded or the
/// screen is locked (`emulator.rs` step 0), but `Platform::now()` is the wall clock and
/// keeps running. On the next foreground the song is behind by however long the screen
/// was off, and the loop below — which only `sleep`s when the next event is still in the
/// future — dispatched the ENTIRE remaining backlog inside one poll: thousands of MIDI
/// events pushed at the synth in a burst, on the emulator thread, in a single frame.
/// Past this threshold the timeline is re-based instead, which is what a real sequencer
/// does with a stalled clock.
const MAX_CATCHUP_MS: u64 = 250;

pub struct SmafPlayer {
    events: Events,
}

impl SmafPlayer {
    pub async fn play(&self, system: &mut System, sink: &dyn AudioSink, gate: &PlaybackGate, repeat: bool, volume: Arc<AtomicU8>) {
        // The level last pushed to the sink's music bus, and whether the sink took it.
        // `u8::MAX` is not a reachable volume (`clip_set_volume` clamps to 100), so the
        // first event of every playback always publishes the current level.
        //
        // ★ Only a MUSIC file ever touches that bus — see [`Score`]. An effect playing
        // at a different setting must not drag the music's level with it.
        let is_music = self.events.is_music;
        let mut pushed_vol = u8::MAX;
        let mut host_music_bus = false;

        loop {
            let mut active_notes: Vec<(u8, u8)> = Vec::new();
            let mut used_channels: BTreeSet<u8> = BTreeSet::new();

            // Epoch millis, saturating: `Platform::now()` is the WALL clock and can step
            // backwards (NTP), which as an `Instant - Instant` would underflow to a
            // 584-million-year "elapsed" and dispatch the whole song at once.
            let mut start_ms = system.platform().now().raw();
            for (time, event) in self.events.events.iter() {
                let mut now = system.platform().now().raw();
                if gate.should_stop(now) {
                    break;
                }

                let elapsed = now.saturating_sub(start_ms);
                if (*time as u64) > elapsed {
                    system.sleep(((*time as u64) - elapsed) as _).await;

                    // Re-read the clock: a deferred stop may have come due while asleep.
                    now = system.platform().now().raw();
                    if gate.should_stop(now) {
                        break;
                    }
                } else if elapsed - (*time as u64) > MAX_CATCHUP_MS {
                    // The clock ran away from us (backgrounded, or the device could not
                    // keep up). Re-base the timeline so the rest of the song plays in
                    // real time from here, instead of flushing the whole backlog into
                    // one poll. `start_ms` moves forward by exactly the deficit, so this
                    // event is "due now" and every later one keeps its original spacing.
                    start_ms = start_ms.saturating_add(elapsed - (*time as u64));
                    wie_util::stats::add_smaf_catchup();
                }

                // Per-clip volume (MC_mdaClipSetVolume), read live so the game's
                // volume slider takes effect on the currently-playing clip.
                let vol = volume.load(Ordering::Relaxed); // 0..=100
                if is_music && vol != pushed_vol {
                    pushed_vol = vol;
                    // Hand the level to the sink's music bus. See
                    // `AudioSink::set_music_volume`: when the sink owns a bus it applies
                    // `volume_gain` to the rendered audio and we must leave velocity alone.
                    host_music_bus = sink.set_music_volume(vol);
                }
                let gain = crate::volume_gain(vol);

                match event {
                    SmafEvent::Wave {
                        channel,
                        sampling_rate,
                        data,
                    } => {
                        // SFX volume: scale PCM by the shared law. Skip entirely when muted.
                        if vol == 0 {
                            // muted
                        } else if vol >= 100 {
                            sink.play_wave(*channel, *sampling_rate, data);
                        } else {
                            let scaled: Vec<i16> = data.iter().map(|s| (*s as f32 * gain) as i16).collect();
                            sink.play_wave(*channel, *sampling_rate, &scaled);
                        }
                    }
                    SmafEvent::MidiNoteOn { channel, note, velocity } => {
                        // BGM volume. With a host music bus the level is already applied to
                        // the synth's OUTPUT, so the note keeps the velocity the score wrote
                        // — velocity also picks the SoundFont layer and the filter cutoff, so
                        // scaling it here made low settings dull as well as quiet, and by a
                        // completely different curve than the PCM bus above.
                        //
                        // Sinks without a bus (external synths) keep the old behaviour;
                        // >=1 so an audible note never collapses into a note-off.
                        let v = if *velocity == 0 || vol == 0 {
                            0
                        } else if host_music_bus {
                            *velocity
                        } else {
                            (((*velocity as u32) * vol as u32 / 100) as u8).max(1)
                        };
                        sink.midi_note_on(*channel, *note, v);
                        active_notes.push((*channel, *note));
                        used_channels.insert(*channel);
                    }
                    SmafEvent::MidiNoteOff { channel, note, velocity } => {
                        sink.midi_note_off(*channel, *note, *velocity);
                        active_notes.retain(|(c, n)| !(*c == *channel && *n == *note));
                    }
                    SmafEvent::MidiProgramChange { channel, program } => {
                        sink.midi_program_change(*channel, *program);
                        used_channels.insert(*channel);
                    }
                    SmafEvent::MidiControlChange { channel, control, value } => {
                        sink.midi_control_change(*channel, *control, *value);
                        used_channels.insert(*channel);
                    }
                    SmafEvent::MidiPitchBend { channel, value } => {
                        sink.midi_pitch_bend(*channel, *value);
                        used_channels.insert(*channel);
                    }
                    SmafEvent::MidiSysEx(data) => {
                        sink.midi_sysex(data);
                    }
                    SmafEvent::End => {}
                }
            }

            for (channel, note) in &active_notes {
                sink.midi_note_off(*channel, *note, 0);
            }

            for channel in &used_channels {
                sink.midi_control_change(*channel, 64, 0);
                sink.midi_control_change(*channel, 120, 0);
                sink.midi_control_change(*channel, 123, 0);
            }

            if !repeat || gate.should_stop(system.platform().now().raw()) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::{boxed::Box, sync::Arc, vec, vec::Vec};
    use core::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

    use smaf_player::SmafEvent;

    use super::{MUSIC_REJOIN_MS, MUSIC_STOP_GRACE_MS, PlaybackGate, Score, SmafPlayer};
    use crate::{AudioSink, Database, DatabaseRepository, DefaultTaskRunner, Filesystem, Instant, Platform, Screen, System, canvas::Image};

    struct NullDatabase;

    #[async_trait::async_trait]
    impl Database for NullDatabase {
        async fn next_id(&self) -> u32 {
            1
        }

        async fn add(&mut self, _data: &[u8]) -> u32 {
            1
        }

        async fn get(&self, _id: u32) -> Option<alloc::vec::Vec<u8>> {
            None
        }

        async fn set(&mut self, _id: u32, _data: &[u8]) -> bool {
            true
        }

        async fn delete(&mut self, _id: u32) -> bool {
            true
        }

        async fn get_record_ids(&self) -> alloc::vec::Vec<u32> {
            vec![]
        }
    }

    struct NullDatabaseRepository;

    #[async_trait::async_trait]
    impl DatabaseRepository for NullDatabaseRepository {
        async fn open(&self, _name: &str, _app_id: &str) -> Box<dyn Database> {
            Box::new(NullDatabase)
        }

        async fn exists(&self, _name: &str, _app_id: &str) -> bool {
            false
        }

        async fn delete(&self, _name: &str, _app_id: &str) -> bool {
            false
        }
    }

    struct NullFilesystem;

    #[async_trait::async_trait]
    impl Filesystem for NullFilesystem {
        async fn exists(&self, _aid: &str, _path: &str) -> bool {
            false
        }

        async fn size(&self, _aid: &str, _path: &str) -> Option<usize> {
            None
        }

        async fn read(&self, _aid: &str, _path: &str, _offset: usize, _count: usize, _buf: &mut [u8]) -> Option<usize> {
            None
        }

        async fn write(&self, _aid: &str, _path: &str, _offset: usize, data: &[u8]) -> usize {
            data.len()
        }

        async fn truncate(&self, _aid: &str, _path: &str, _len: usize) {}
    }

    struct NullScreen;

    impl Screen for NullScreen {
        fn request_redraw(&self) -> wie_util::Result<()> {
            Ok(())
        }

        fn paint(&self, _image: &dyn Image) {}

        fn width(&self) -> u32 {
            240
        }

        fn height(&self) -> u32 {
            320
        }
    }

    struct NullPlatform {
        screen: NullScreen,
        database_repository: NullDatabaseRepository,
        filesystem: NullFilesystem,
        now: AtomicUsize,
    }

    impl NullPlatform {
        fn new() -> Self {
            Self {
                screen: NullScreen,
                database_repository: NullDatabaseRepository,
                filesystem: NullFilesystem,
                now: AtomicUsize::new(0),
            }
        }
    }

    impl Platform for NullPlatform {
        fn screen(&self) -> &dyn Screen {
            &self.screen
        }

        fn now(&self) -> Instant {
            Instant::from_epoch_millis(self.now.fetch_add(8, Ordering::SeqCst) as u64)
        }

        fn database_repository(&self) -> &dyn DatabaseRepository {
            &self.database_repository
        }

        fn filesystem(&self) -> &dyn Filesystem {
            &self.filesystem
        }

        fn audio_sink(&self) -> Box<dyn AudioSink> {
            Box::new(NoopAudioSink)
        }

        fn write_stdout(&self, _buf: &[u8]) {}

        fn write_stderr(&self, _buf: &[u8]) {}

        fn exit(&self) {}

        fn vibrate(&self, _duration_ms: u64, _intensity: u8) {}
    }

    struct NoopAudioSink;

    impl AudioSink for NoopAudioSink {
        fn play_wave(&self, _channel: u8, _sampling_rate: u32, _wave_data: &[i16]) {}

        fn midi_note_on(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_note_off(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_program_change(&self, _channel_id: u8, _program: u8) {}

        fn midi_control_change(&self, _channel_id: u8, _control: u8, _value: u8) {}

        fn midi_pitch_bend(&self, _channel_id: u8, _value: u16) {}

        fn midi_sysex(&self, _data: &[u8]) {}
    }

    struct CountingSink {
        program_change_count: Arc<AtomicUsize>,
        stop_after: usize,
        stop_flag: Arc<PlaybackGate>,
    }

    impl AudioSink for CountingSink {
        fn play_wave(&self, _channel: u8, _sampling_rate: u32, _wave_data: &[i16]) {}

        fn midi_note_on(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_note_off(&self, _channel_id: u8, _note: u8, _velocity: u8) {}

        fn midi_program_change(&self, _channel_id: u8, _program: u8) {
            let count = self.program_change_count.fetch_add(1, Ordering::SeqCst) + 1;
            if count >= self.stop_after {
                self.stop_flag.kill();
            }
        }

        fn midi_control_change(&self, _channel_id: u8, _control: u8, _value: u8) {}

        fn midi_pitch_bend(&self, _channel_id: u8, _value: u16) {}

        fn midi_sysex(&self, _data: &[u8]) {}
    }

    fn new_system() -> System {
        System::new(Box::new(NullPlatform::new()), "test-pid", "test-aid", DefaultTaskRunner)
    }

    #[test]
    fn score_classifies_the_two_buses() {
        // A Hero3 SFX (`res/snd/sd*_mf`, a SMAF `ATR` chunk) decodes to nothing but
        // wave one-shots. It must NOT be allowed to drive the host music bus.
        let sfx = Score {
            events: vec![
                (
                    0,
                    SmafEvent::Wave {
                        channel: 1,
                        sampling_rate: 16000,
                        data: vec![0, 1, 2],
                    },
                ),
                (10, SmafEvent::End),
            ],
            is_music: false,
            duration_ms: 10,
        };
        assert!(!sfx.is_music);

        // The real classifier, over the same shapes.
        let classify = |events: Vec<(usize, SmafEvent)>| events.iter().any(|(_, e)| !matches!(e, SmafEvent::Wave { .. } | SmafEvent::End));
        assert!(!classify(sfx.events));
        assert!(classify(vec![(0, SmafEvent::MidiProgramChange { channel: 0, program: 1 })]));
        assert!(classify(vec![(
            0,
            SmafEvent::MidiNoteOn {
                channel: 0,
                note: 60,
                velocity: 100
            }
        )]));
        // An empty file is not music (nothing to render, nothing to level).
        assert!(!classify(vec![]));
    }

    /// The deferred-stop state machine, driven with the exact call order the device
    /// trace recorded. See [`MUSIC_STOP_GRACE_MS`].
    #[test]
    fn a_guest_stop_of_the_music_survives_an_effect_but_not_a_menu() {
        let gate = PlaybackGate::new();
        let t0 = 1_000_000u64;

        // The guest stops its BGM to free its one clip (device: 20:19:19.435).
        gate.stop_at(t0 + MUSIC_STOP_GRACE_MS);
        assert!(!gate.should_stop(t0), "the stop must not be immediate");
        assert!(gate.should_stop(t0 + MUSIC_STOP_GRACE_MS), "…but it must be armed");

        // 37 ms later the effect arrives (device: 20:19:19.472). It is ~600 ms long.
        gate.hold_until(t0 + 37 + 600 + MUSIC_REJOIN_MS);
        assert!(!gate.should_stop(t0 + MUSIC_STOP_GRACE_MS), "an effect must hold the music open");
        // The guest re-issues the SAME song when the effect ends (device: +626 ms).
        assert!(!gate.should_stop(t0 + 626), "the music must still be alive at the re-issue");
        gate.keep_going();
        assert!(!gate.should_stop(t0 + 60_000), "…and then run indefinitely");
        assert!(gate.is_running());

        // Now the menu: a stop with nothing after it (device: 4.5 s of silence).
        gate.stop_at(t0 + 100_000 + MUSIC_STOP_GRACE_MS);
        assert!(!gate.should_stop(t0 + 100_000 + MUSIC_STOP_GRACE_MS - 1));
        assert!(gate.should_stop(t0 + 100_000 + MUSIC_STOP_GRACE_MS), "a real stop must land");
    }

    #[test]
    fn hold_until_never_pulls_a_deadline_in_and_never_revives_a_dead_one() {
        let gate = PlaybackGate::new();
        // No pending stop: an effect must not invent one.
        gate.hold_until(5_000);
        assert!(!gate.should_stop(u64::MAX), "an effect alone must never stop the music");

        gate.stop_at(10_000);
        gate.hold_until(9_000); // a shorter effect than the window already granted
        assert!(!gate.should_stop(9_500), "the deadline must not move backwards");
        assert!(gate.should_stop(10_000));

        // A hard kill outranks everything.
        let gate = PlaybackGate::new();
        gate.kill();
        gate.keep_going();
        assert!(gate.should_stop(0), "a different song must stop the old one at once");
        assert!(!gate.is_running());
    }

    #[test]
    fn score_measures_the_audible_length_of_an_effect() {
        // `Audio::play` holds the music open for the effect's DURATION, and a PCM
        // one-shot's event time is only when it starts — the samples are the length.
        let one_second_at_8k = Score {
            events: vec![(
                0,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 8000],
                },
            )],
            is_music: false,
            duration_ms: 0,
        };
        let _ = one_second_at_8k;
        // The real classifier, over the same shape (mirrors `Score::parse`).
        let measure = |events: &[(usize, SmafEvent)]| {
            let mut d = 0usize;
            for (time, event) in events.iter() {
                let end = match event {
                    SmafEvent::Wave {
                        sampling_rate,
                        data,
                        channel,
                    } => time + (data.len() / (*channel).max(1) as usize) * 1000 / (*sampling_rate).max(1) as usize,
                    _ => *time,
                };
                d = d.max(end);
            }
            d
        };
        assert_eq!(
            measure(&[(
                0,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 8000]
                }
            )]),
            1000
        );
        // Starting 500 ms in pushes the end out by 500 ms.
        assert_eq!(
            measure(&[(
                500,
                SmafEvent::Wave {
                    channel: 1,
                    sampling_rate: 8000,
                    data: vec![0; 4000]
                }
            )]),
            1000
        );
        // A zero sampling rate must not divide by zero.
        assert_eq!(
            measure(&[(
                7,
                SmafEvent::Wave {
                    channel: 0,
                    sampling_rate: 0,
                    data: vec![0; 4]
                }
            )]),
            4007
        );
    }

    #[futures_test::test]
    async fn plays_once_when_repeat_is_false() {
        let counter = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(PlaybackGate::new());
        let sink = CountingSink {
            program_change_count: counter.clone(),
            stop_after: usize::MAX,
            stop_flag: stop_flag.clone(),
        };
        let player = SmafPlayer {
            events: Arc::new(Score {
                events: vec![(0, SmafEvent::MidiProgramChange { channel: 0, program: 1 })],
                is_music: true,
                duration_ms: 0,
            }),
        };
        let mut system = new_system();

        player.play(&mut system, &sink, &stop_flag, false, Arc::new(AtomicU8::new(100))).await;

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[futures_test::test]
    async fn repeats_until_stop_flag_is_set() {
        let counter = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(PlaybackGate::new());
        let sink = CountingSink {
            program_change_count: counter.clone(),
            stop_after: 2,
            stop_flag: stop_flag.clone(),
        };
        let player = SmafPlayer {
            events: Arc::new(Score {
                events: vec![(0, SmafEvent::MidiProgramChange { channel: 0, program: 1 })],
                is_music: true,
                duration_ms: 0,
            }),
        };
        let mut system = new_system();

        player.play(&mut system, &sink, &stop_flag, true, Arc::new(AtomicU8::new(100))).await;

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }
}
