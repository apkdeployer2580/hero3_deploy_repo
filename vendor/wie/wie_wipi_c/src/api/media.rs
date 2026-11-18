use alloc::vec;

use bytemuck::{Pod, Zeroable};

use wipi_types::wipic::WIPICWord;

use wie_util::{Result, read_generic, write_generic};

use crate::context::WIPICContext;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MdaClip {
    clip_id: i32,
    h_proc: i32,
    r#type: u8,
    in_use: u8, // bool
    _padding1: [u8; 2],
    dev_id: i32,

    x: i32,
    y: i32,
    w: i32,
    h: i32,
    mute: u8, // bool
    _padding2: [u8; 3],
    watermark: i32,
    position: i32,
    quality: i32,
    mode: i32,
    state: i32,
    penpot: i32,
    num_slave: i32,

    clip_save: WIPICWord, // MC_MdaClip**

    audio_tone_saved_len: i32,
    audio_tone_len: i32,
    audio_tone: WIPICWord,          // MC_MdaToneType*
    audio_tone_duration: WIPICWord, // M_Int32 *

    audio_freq_saved_len: i32,
    audio_freq_len: i32,
    audio_hi_freq: WIPICWord,       // M_Int32 *
    audio_low_freq: WIPICWord,      // M_Int32 *
    audio_freq_duration: WIPICWord, // M_Int32 *

    sound_data_saved_len: i32,
    sound_data_len: i32,
    sound_data: WIPICWord, // M_Byte *

    original_volume: i32,

    pos: i8,
    _padding3: [u8; 3],
    codec_config_data_size: i32,
    codec_config_data: WIPICWord, // M_Byte *
    tick_duration: i32,

    b_control: u8, // bool
    _padding4: [u8; 3],

    movie_record_size_width: i32,
    movie_record_size_height: i32,
    max_record_length: i32,

    temp_record_space: WIPICWord, // M_Byte *
    temp_record_space_size: i32,
    temp_record_size: i32,

    next_ptr: WIPICWord, // MC_MdaClip*

    mda_id: i32,
    device_info: i32,

    // not in sdk, for internal usage
    handle: u32,
}

pub async fn clip_create(context: &mut dyn WIPICContext, ptr_type: WIPICWord, buf_size: WIPICWord, callback: WIPICWord) -> Result<WIPICWord> {
    tracing::debug!("MC_mdaClipCreate({ptr_type:#x}, {buf_size:#x}, {callback:#x})");

    let clip = context.alloc_raw(size_of::<MdaClip>() as u32)?;

    Ok(clip)
}

pub async fn clip_free(context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::debug!("MC_mdaClipFree({clip:#x})");

    // some app call clip free with null clip...
    if clip == 0 {
        return Ok(0);
    }

    // ★ 2026-08-09: drop the host-side bookkeeping this clip minted. Every
    // `MC_mdaClipPutData` used to mint an audio handle that NOTHING ever removed, each
    // one holding its own copy of the sound file — an unbounded leak on a device that
    // Android already kills first for its RSS. Keyed by the clip POINTER on the host
    // side, so no uninitialised guest field is ever trusted, and a playback already in
    // flight is left to finish.
    context.system().audio().release_clip(clip);

    context.free_raw(clip, size_of::<MdaClip>() as u32)?;

    Ok(0)
}

pub async fn clip_get_type(_context: &mut dyn WIPICContext, clip: WIPICWord, buf: WIPICWord, buf_size: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaClipGetType({clip:#x}, {buf:#x}, {buf_size:#x})");

    Ok(0)
}

pub async fn get_mute_state(_context: &mut dyn WIPICContext, source: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaGetMuteState({source:#x})");

    Ok(0)
}

pub async fn clip_get_info(
    _context: &mut dyn WIPICContext,
    clip: WIPICWord,
    command: WIPICWord,
    buf: WIPICWord,
    buf_size: WIPICWord,
) -> Result<WIPICWord> {
    tracing::warn!("stub OEMC_mdaClipGetInfo({clip:#x}, {command:#x}, {buf:#x}, {buf_size:#x})");

    Ok(0)
}

pub async fn clip_put_data(context: &mut dyn WIPICContext, ptr_clip: WIPICWord, buf: WIPICWord, buf_size: WIPICWord) -> Result<i32> {
    tracing::debug!("MC_mdaClipPutData({ptr_clip:#x}, {buf:#x}, {buf_size:#x})");

    if ptr_clip == 0 {
        return Ok(-1);
    }

    let mut data = vec![0; buf_size as _];
    context.read_bytes(buf, &mut data)?;

    let handle = context.system().audio().load_smaf(&data);
    if let Err(x) = handle {
        tracing::error!("Failed to load audio: {x:?}");
        return Ok(0);
    }

    let handle = handle.unwrap();
    // Remember which handle this clip pointer owns, so `MC_mdaClipFree` can release it.
    context.system().audio().bind_clip(ptr_clip, handle);
    // ★★★ 2026-08-28 — WHERE A DEFERRED 배경음 CHANGE FINALLY BECOMES ATTRIBUTABLE.
    //
    // Hero3's cycle for every sound is
    //     ClipFree -> ClipCreate -> ClipSetVolume(v) -> ClipPutData -> Play
    // so at `ClipSetVolume` the pointer is UNBOUND and `v` cannot be assigned to a bus.
    // Device trace, vc194 — the player's slider write lands in exactly that hole:
    //
    //     ClipSetVolume(clip 0x4b85d400, vol 40) -> bound None · bound holds music None
    //     ClipSetVolume(clip 0x4b85d400, vol 20) -> bound None · bound holds music None
    //
    // HERE the data has just been parsed, so `is_music` is known, and `v` is still in
    // `clip_volumes[ptr]`. If this clip is the music, that `v` IS the 배경음 level and the
    // live music channel takes it now — it must not wait for the same-song rejoin in
    // `Audio::play`, which reads a handle cell the player's write never reached.
    //
    // ⛔⛔ THIS ONLY WORKS BECAUSE `release_clip` NO LONGER WIPES `clip_volumes`. vc193 had
    // this block WITHOUT that fix and it published the map's default instead:
    //     "music bus level 100 adopted at putData" — while the slider was at 1.
    // If that line ever comes back reading 100, the wipe has been reintroduced; fix it
    // there, not here.
    {
        let mut audio = context.system().audio();
        // ⛔ `clip_ptr_volume_set`, not `clip_ptr_volume`: a pointer the guest has never
        // set a level for must publish NOTHING. `MC_mdaClipCreate` mints a new heap
        // address per sound, so the `unwrap_or(100)` default was being broadcast over a
        // correct level (vc196: "music bus level 100 adopted at putData" while the bus was
        // correctly at 40).
        if audio.bound_is_music(ptr_clip) == Some(true)
            && let Some(v) = audio.clip_ptr_volume_set(ptr_clip)
        {
            if audio.note_music_volume(v) {
                tracing::warn!("music bus level {v} adopted at putData (clip {ptr_clip:#x} holds music)");
            }
        }
    }
    let mut clip: MdaClip = read_generic(context, ptr_clip)?;
    clip.handle = handle;
    write_generic(context, ptr_clip, clip)?;

    Ok(buf_size as _)
}

pub async fn clip_get_data(_context: &mut dyn WIPICContext, clip: WIPICWord, buf: WIPICWord, buf_size: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaClipGetData({clip:#x}, {buf:#x}, {buf_size:#x})");

    Ok(0)
}

pub async fn clip_set_position(_context: &mut dyn WIPICContext, clip: WIPICWord, ms: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaClipSetPosition({clip:#x}, {ms:#x})");

    Ok(0)
}

pub async fn clip_get_volume(context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    if clip == 0 {
        return Ok(0);
    }
    let v = context.system().audio().clip_ptr_volume(clip);
    Ok(v as WIPICWord)
}

pub async fn clip_set_volume(context: &mut dyn WIPICContext, clip: WIPICWord, volume: WIPICWord) -> Result<WIPICWord> {
    tracing::debug!("MC_mdaClipSetVolume({clip:#x}, {volume})");
    if clip == 0 {
        return Ok(0);
    }
    // Volume level (0..=100, level*20 from Hero3's settings). Store it against the
    // stable clip pointer — clip_put_data mints a NEW handle between here and play,
    // so a handle-keyed volume would land on the wrong (previous) clip. `play`
    // recovers it by pointer. Also push it to the current handle so a mid-play
    // change (BGM slider) is heard live. Was a no-op stub -> sliders did nothing.
    let v = volume.min(100) as u8;
    let mda: MdaClip = read_generic(context, clip)?;
    let mut audio = context.system().audio();
    // ★★★ 2026-08-28 — THE LINE THAT SAYS WHERE A SLIDER CHANGE LANDS. Player report:
    // *"인게임 내에서 환경설정으로 배경음 OFF로 돌렸는데도 소리 재생되고있어"*, while the
    // very same slider worked in the shell. `Audio::volume_route` explains that shape —
    // the host keeps the music on the handle it started on, the guest re-binds its one
    // clip pointer per effect.
    //
    // ⛔ `warn`, NOT `info` — and that is not a taste. The release filter is
    // `FilterBuilder::parse("warn,main=info")` (`wie_android/src/lib.rs`), and `main` is
    // the **wie_android** crate alone: every vendor crate logs under `wie_wipi_c::…` /
    // `wie_backend::…` / `wie_lgt::…` and is therefore held at `warn`. An `info!` here is
    // INVISIBLE on a device, and 2026-08-28 that cost a wrong conclusion — the absence of
    // these lines was read as "the game never calls MC_mdaClipSetVolume" when they were
    // simply filtered.
    let (bound, music, bound_is_music) = audio.volume_route(clip);
    tracing::warn!(
        "MC_mdaClipSetVolume(clip {clip:#x}, vol {v}) -> mda.handle {} · bound {bound:?} · live music {music:?} · bound holds music {bound_is_music:?}",
        mda.handle
    );
    audio.set_clip_ptr_volume(clip, v);
    audio.set_clip_volume(mda.handle, v);
    // ★★★ 2026-08-28 — AND ONTO THE LIVE MUSIC, when this write is provably the music's.
    // The live music does not read any handle the guest can name (the host keeps it on the
    // handle it started on), so without this the level only ever arrives via the same-song
    // rejoin in `Audio::play` — which never happens while effects hold the guest's clip.
    // That is the "효과음 내니까 적용안돼" report.
    //
    // ⛔ `bound_is_music == Some(true)` is the whole gate, and it is not optional: the game
    // reuses ONE clip pointer for both buses, so an unattributed write routed here would
    // make the music follow the 효과음 slider. `None` (the pointer is freed, which is the
    // common case right after `MC_mdaClipFree`) is NOT good enough.
    if bound_is_music == Some(true) && audio.note_music_volume(v) {
        tracing::warn!("music bus level {v} adopted from clip {clip:#x} (bound handle holds music)");
    }
    // …and to the handle the pointer is ACTUALLY bound to. `mda.handle` is guest memory
    // that `MC_mdaClipCreate` never initialises (see `Audio::clip_handles`), so it can
    // name a stale handle — or none. Writing both cells is free: an extra volume cell
    // nobody reads costs nothing, and a missed one silently ignores the player's slider.
    if let Some(h) = bound
        && h != mda.handle
    {
        audio.set_clip_volume(h, v);
    }

    Ok(0)
}

pub async fn get_volume(_context: &mut dyn WIPICContext) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaGetVolume");

    Ok(0)
}

pub async fn play(context: &mut dyn WIPICContext, ptr_clip: WIPICWord, repeat: WIPICWord) -> Result<i32> {
    tracing::debug!("MC_mdaPlay({ptr_clip:#x}, {repeat})");

    if ptr_clip == 0 {
        return Ok(0);
    }

    let clip: MdaClip = read_generic(context, ptr_clip)?;
    tracing::debug!("MC_mdaPlay clip={ptr_clip:#x} handle={} repeat={repeat}", clip.handle);

    // Recover the volume the game set for this clip pointer and bind it to the
    // handle that actually plays (the handle changed since clip_set_volume).
    {
        let mut audio = context.system().audio();
        let vol = audio.clip_ptr_volume(ptr_clip);
        let bound = audio.bound_handle(ptr_clip);
        // ★ 2026-08-28 — the other half of the "배경음 슬라이더가 효과음 뒤에는 안 먹는다"
        // trace: this says which handle the intended level is being bound to, next to the
        // handle the guest's own struct names. When they disagree the level lands on a cell
        // the running music is not reading.
        tracing::warn!("MC_mdaPlay(clip {ptr_clip:#x}) vol {vol} -> mda.handle {} · bound {bound:?}", clip.handle);
        audio.set_clip_volume(clip.handle, vol);
        if let Some(h) = bound
            && h != clip.handle
        {
            audio.set_clip_volume(h, vol);
        }
    }

    let system = context.system();

    let result = system.audio().play(system, clip.handle, repeat != 0);

    if let Err(x) = result {
        tracing::error!("Failed to load audio: {x:?}");
    }

    Ok(0)
}

pub async fn clip_alloc_player(_context: &mut dyn WIPICContext, clip: WIPICWord, param: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaClipAllocPlayer({clip:#x}, {param:#x})");

    Ok(0)
}

pub async fn clip_free_player(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaClipFreePlayer({clip:#x})");

    Ok(0)
}

pub async fn vibrator(context: &mut dyn WIPICContext, level: i32, timeout: i32) -> Result<WIPICWord> {
    tracing::debug!("MC_mdaVibrator({level}, {timeout})");

    let duration_ms = timeout.max(0) as u64;
    let intensity = (level.clamp(0, 10) * 10) as u8;
    context.system().platform().vibrate(duration_ms, intensity);

    Ok(0)
}

pub async fn set_mute_state(_context: &mut dyn WIPICContext, source: i32, b_mute: i32) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaSetMuteState({source:#x}, {b_mute})");

    Ok(0)
}

pub async fn pause(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaPause({clip:#x})");

    Ok(0)
}

pub async fn resume(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaResume({clip:#x})");

    Ok(0)
}

pub async fn stop(context: &mut dyn WIPICContext, ptr_clip: WIPICWord) -> Result<WIPICWord> {
    tracing::debug!("MC_mdaStop({ptr_clip:#x})");

    if ptr_clip == 0 {
        return Ok(0);
    }

    let clip: MdaClip = read_generic(context, ptr_clip)?;

    let system = context.system();
    let now_ms = system.platform().now().raw();

    system.audio().stop(now_ms, clip.handle);

    Ok(0)
}

pub async fn record(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaRecord({clip:#x})");

    Ok(0)
}

pub async fn unk7(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaUnk7({clip:#x})");

    Ok(0)
}

pub async fn unk17(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaUnk17({clip:#x})");

    Ok(0)
}

pub async fn unk18(_context: &mut dyn WIPICContext, clip: WIPICWord) -> Result<WIPICWord> {
    tracing::warn!("stub MC_mdaUnk18({clip:#x})");

    Ok(0)
}
