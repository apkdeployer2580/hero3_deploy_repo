use alloc::{collections::BTreeMap, format, string::ToString, sync::Arc, vec::Vec};

use wie_util::{ByteRead, ByteWrite, Result, WieError, read_generic};

use super::{
    Entry, PatternToken,
    patch::{Flavor, flavor_keeps},
    scan_pattern,
};
use crate::{ArmCore, engine::ArmRegister, function::JumpTo, stdlib};

const BINARY_PATCH_SVC: u32 = 0x80;

#[derive(Debug, Clone, Copy)]
pub struct Hook {
    /// LSB=1 (Thumb) is required; the engine doesn't service ARM-mode SVCs.
    pub pc: u32,
    pub kind: HookKind,
    /// ★ 163차: `flavor = "main" | "vanilla"` in the TOML — install this hook on that APK
    /// flavour only, the same rule as a patch's (`super::patch::Flavor`). `None` = every
    /// flavour. Pattern hooks are toolchain-generic and always `None`.
    pub flavor: Option<Flavor>,
}

#[derive(Debug, Clone, Copy)]
pub enum HookKind {
    RespawnRateLgt { exit_pc: u32 },
    ExpRateLgt { exit_pc: u32 },
    /// ABI: dst=r0, src=r1, len=r2; returns via LR.
    Memcpy,
    /// ABI: dst=r0, val=r1 (low byte), len=r2; returns via LR.
    Memset,
    /// ABI: dst=r0, src=r1; copies NUL-inclusive; returns via LR (R0 unchanged).
    Strcpy,
    /// ABI: str=r0; returns length in R0 via LR.
    Strlen,
    /// Replaces an inline byte-copy loop. Requires a down-counter `len` on the
    /// stack — zeroing it has to terminate the loop, so up-counter (`for i = 0;
    /// i < N`) shapes are not compatible.
    InlineCopy(InlineCopy),
    /// The game's palettised sprite blitter at guest `0x110fe4` — 26% of all guest
    /// instructions in a field frame. See `super::blit565` for the contract; anything
    /// the host cannot reproduce exactly falls back to the guest code.
    Blit565,
    /// The SAME blitter in the LGT `binary.mod` build (`0x2dd7c`): identical arguments
    /// and pixel arithmetic, but the sprite descriptor's pixel/palette words are direct
    /// pointers instead of KTF's allocator handles. See `blit565::SpriteLayout`.
    Blit565Lgt,
    /// The game's semi-transparent rectangle fill (`blendRect`, LGT `0x2b700`), hooked on
    /// its OUTER row loop at `0x2b786` so one call finishes the whole rectangle.
    /// ~21 % of guest samples in a field frame and far more with a menu or fade on
    /// screen — see `super::blend565`. Carries the function's shared tail as `exit_pc`
    /// rather than hardcoding it, so a second revision only needs a new table row.
    Blend565Lgt { exit_pc: u32 },
    /// The game's LUT-blended palettised sprite blitter (LGT `0x2e8c4`, immediately after
    /// [`HookKind::Blit565Lgt`]'s function) — the path a CRITICAL-HIT flash and the other
    /// translucent effects take, which is why neither of the two hooks above touched the
    /// frame drop the player sees during one. `blit565`'s 13 arguments plus two blend
    /// levels; see `super::blitlut565`. The three lookup-table addresses ride on the hook
    /// because they are `.data`/`.bss` addresses of one revision.
    BlitLut565Lgt(super::blitlut565::Tables),
    /// The game's per-pixel LUT COLOUR TRANSFORM of a rectangle already on screen
    /// (`tintRect`, LGT `0x2b8f8` — the function immediately after `blendRect`), hooked on
    /// its OUTER row loop at `0x2b982` exactly like [`HookKind::Blend565Lgt`].
    ///
    /// This is what a CRITICAL HIT actually costs, measured on the device in 49차: 16.0 M
    /// guest instructions in the hitching tick against 0.65 M in its neighbours, with the
    /// blit count, pixel count and SVC count unchanged — i.e. not a blitter at all. See
    /// `super::tint565`. `lut` is the byte-table base, which is the very same table
    /// [`HookKind::BlitLut565Lgt`] takes as `blend_a`.
    TintRect565Lgt { exit_pc: u32, lut: u32 },
    /// The socket-bonus sum (LGT `0x133e4`, inside `addItemToInventory` `0x131d0`).
    /// Not a performance hook: it makes an item able to carry orbs of DIFFERENT kinds,
    /// which the guest's one-orb-id record cannot express. Replaces the bonus formula
    /// and resumes past the `strh` it performs itself, so `exit_pc` is required for the
    /// same reason [`HookKind::Blend565Lgt`] needs one. See `super::orbsum`.
    OrbSumLgt { exit_pc: u32 },
    /// The orb NAME prefix (LGT `0x6c0a` and `0x6c4c`, inside `formatItemName` `0x6b90`).
    /// Swaps the single orb name the game just looked up for a chained one
    /// (`직격흡혈정령의`) when more than one kind is socketed. Declines below two orbs.
    /// Carries the string-table word and the scratch buffer, both `.bss` addresses of one
    /// revision, for the same reason [`HookKind::BlitLut565Lgt`] carries its tables.
    OrbNameLgt { tables: super::orbsum::NameTables, exit_pc: u32 },
    /// One extra species-gated loot roll at the END of the game's loot routine (LGT
    /// `0x38e70`, hooked on its `cmp r7,#0` at `0x38ffc`). Appends a `{리버…` row to the
    /// compacted loot for the eight hard bosses at `reaverdrop::RATE_PCT`, drawing the
    /// game's own `random()`. Performs the displaced compare by choosing between
    /// `exit_pc` (spawn the loot) and `skip_pc` (the function tail). See `super::reaverdrop`.
    ReaverDropLgt { exit_pc: u32, skip_pc: u32 },
    /// The 하늘성 species override (LGT `parseScnHeader` `0x14284`, hooked on its last
    /// load `0x142da ldr r0,[r6]`). Writes the host's armed species triple over the
    /// three words the function just stored, for the one scene the host named, and
    /// performs the displaced load. Ships on BOTH flavours — the tower is not a drop.
    /// See `super::towerspecies`.
    TowerSpeciesLgt { exit_pc: u32 },
    /// The 하늘성 spawn cap (LGT spawn loop, hooked on its empty-slot compare `0x158b0
    /// cmp r0,#0xff`, 145차). Past the armed cap a spawn point takes the game's own "no
    /// monster here" path, so a floor spawns exactly its count. Performs the displaced
    /// compare by choosing between `exit_pc` (append) and `skip_pc` (the `beq` target).
    /// Ships on BOTH flavours, like the species hook. See `super::towerspawn`.
    TowerSpawnCapLgt { exit_pc: u32, skip_pc: u32 },
    /// The bag boxes' intake on `giveItem` (LGT `0x52b14`, function entry): a
    /// container-category unit goes to the host's box instead of a bag cell, and the
    /// caller sees the success the game would have returned. See `super::bagbox`.
    BagboxGiveLgt,
    /// The same intake on `addItemToInventory` (LGT `0x131d0`, function entry) — the
    /// warehouse `05 04` and cash `05 40` deliveries.
    BagboxAddLgt,
    /// The quest VM's item helper (LGT `0x3bcf0`, function entry): in TAKE mode a
    /// hand-in the host's box can cover is spent there and the bag walk skipped.
    BagboxTakeLgt,
    /// That helper's one exit (`0x3bdd2 add sp,#0x18`): in COUNT mode the box is added
    /// to what the bag walk counted. Performs the displaced `add sp` itself, so it needs
    /// an `exit_pc` for the same reason [`HookKind::Blend565Lgt`] does.
    BagboxCountLgt { exit_pc: u32 },
    /// `countOf 0x55534`'s exit (`0x555d0 add sp,#0xc`): bag total in r0 plus the box
    /// mirror, then the displaced `add sp` and resume at `exit_pc` (the `pop`).
    BagboxCountOfLgt { exit_pc: u32 },
    /// `0x555e0(cat, idx, n)` entry: debit the box first (125차-C), skip the bag walk
    /// when it covers all `n`, otherwise resume with the remainder in r2.
    BagboxSpendLgt,
    /// `removeItem 0x5340c(tab, slot, n)` entry (127차): for the ONE cell the host has
    /// staged a boxed unit into — the box's own bag-item cell — write the bag-item record
    /// back instead of clearing the cell, so no compaction and no in-tail save ever sees
    /// the hole. Every other call resumes into the game's own removal.
    BagboxRestageLgt,
    /// The game's bag-room check `0x3c0e4(cat, idx, n)` entry (166차): `n` is reduced by
    /// what the host's box will take before the game's own check runs, so a full bag no
    /// longer refuses a shop purchase or a script give the box has room for.
    BagboxRoomCheckLgt,
    /// The cash shop's full-bag gate (166차, LGT `0x72e0 lsls r0,r0,#0x18` right after
    /// `isBagFull`): "full" is re-answered "not full" when the host's box takes the whole
    /// row, then the displaced shift is re-done and the arm resumes at `exit_pc`.
    BagboxCashGateLgt { exit_pc: u32 },
    /// The 2회차 transition's entry (LGT `0x3af2c`, `push {r4-r7,lr}`, called only by ending
    /// window 18's KEY handler `0x3b420`): latches "a transition ran" + the difficulty byte
    /// it is about to overwrite into `wie_util::stats`, then runs the game's own code
    /// (`resume_past_push`). Changes nothing. See `super::ending`.
    EndingTransitionLgt,
    /// The ending window's row pick in the common-widget installer (LGT `0x46cec`,
    /// `movs r3,#0xaa` = 「하드코어 모드로;재시작합니다」): re-done with the row the
    /// difficulty byte calls for — the 헬 row after a hardcore (or hell) clear. Main only.
    /// See `super::endinglabel`.
    EndingLabelLgt { exit_pc: u32 },
    /// `hitMonster`'s `ldr r2,[sp,#4]` (LGT `0x386cc`): clamps the now-32-bit hit for the
    /// `i16` HP writer and runs the record's virtual HP pool (`rec+0x100`). Resumes past
    /// the displaced load, so it needs an `exit_pc`. See `super::dmgwide`.
    DmgApplyLgt { exit_pc: u32 },
    /// The floating-number drawer's entry (LGT `0x1ff8`): for the PLAYER's number the
    /// halfword slot is copied into a 32-bit scratch word so the `ldr` patches inside the
    /// drawer read a clean value. Displaces the shared `push {r4-r7,lr}`.
    DmgDispLgt,
    /// The skill SP-cost formula's `level` read, clamped to 70 (the original single-play
    /// cap) so the raised Lv99 cap does not drive `100 - level - reduction` negative.
    /// Spend site (LGT `0x4b8b0`, `ldrsh r3,[r6,r0]`). See `super::spcost`.
    SpCostLevelSpendLgt { exit_pc: u32 },
    /// The same clamp on the skill-window "SP소모:%d" display site (LGT `0x43e36`,
    /// `ldrsh r4,[r0,r5]`).
    SpCostLevelDispLgt { exit_pc: u32 },
    /// The skill-window display value floored to 5 (LGT `0x43e5c`, `adds r2,r0,#0`), the
    /// same floor the spend path applies, so an underflowing build shows the amount it
    /// actually spends.
    SpCostDispFloorLgt { exit_pc: u32 },
    /// The wear recompute's 명중 byte store (LGT `0x4f7ca`, `strb r3,[r0]` → `stat+0x2c`):
    /// re-done with the halfword sum clamped at 255 instead of wrapping at 256 (158차 P1).
    /// See `super::statcap`.
    HitSumClampLgt { exit_pc: u32 },
    /// The cast path's 쿨타임 감소 load (LGT `0x4bbe0`, `ldrsh r2,[r0,r3]` = the addStat(0x11)
    /// word): re-done with the sum capped at 60 % (158차 P2). See `super::statcap`.
    CooldownCapLgt { exit_pc: u32 },
    /// The two defence getters' % load (LGT `0x4cf72` / `0x4cfb2`, `ldrsh r3,[r0,r2]`): re-done
    /// with the 물방% / 특방% sum capped at 500 (160차 T2). See `super::statwide`.
    DefPctCapLgt { exit_pc: u32 },
    /// The four stat getters' i16 truncation (LGT `0x4ce4e` · `0x4cefe` · `0x4cf92` · `0x4cfd2`,
    /// `lsls/asrs r0,#0x10`): saturated instead, the true value published when the caller is
    /// the wear recompute's final pass (160차 T1). See `super::statwide`.
    StatWideLgt { exit_pc: u32 },
    /// The monster hit's defence load (LGT `0x38556`, `ldrsh r6,[r3,r2]`): the published true
    /// defence when the halfword is saturated (160차 T1).
    DefReadWideLgt { exit_pc: u32 },
    /// The player hit's attack load (LGT `0x4bdac` / `0x4be12`, `ldrsh r2,[r3,r0]`): the
    /// published true attack when the halfword is saturated (160차 T1).
    AtkReadWideLgt { exit_pc: u32 },
    /// The dropped-item spawn loop's count read (LGT `0x54790`, `movs r2,#0`): when the
    /// 30-slot loot array is full the oldest record is evicted first, so the 60 s lifetime
    /// (162차) can never write past slot 29. See `super::lootlife`.
    LootSpawnCapLgt { exit_pc: u32 },
    /// State 4's recovery countdown (LGT `0x51068` / `0x5108c`, `subs r3,#1`): ticked down
    /// faster by the same stat. The other four ticks of a basic attack.
    AtkRecoverLgt { exit_pc: u32 },
    /// The player's pose length (LGT `0x4af12`, `lsls r0,r0,#0x18` at the tail of
    /// `0x4aee0`). ⛔ The ANSWER is never changed; while the player is attacking or casting
    /// the handler drags the pose CLOCK towards it instead, clamped to the very length this
    /// call is returning, so `counter == frames` can never be stepped over. Off unless the
    /// host arms `wie_util::stats::ATTACK_SPEED_PCT`. See `super::atkspeed`.
    AtkSpeedLgt { exit_pc: u32 },
    /// The state machine's one per-tick frame-event dispatch (LGT `0x50e2c`, `bl 0x4b92c`).
    /// Replays, through the game's own `0x4b92c`, the frames `AtkSpeedLgt` dragged the pose
    /// clock over this tick — a hit lives on a FRAME (`0x57c3c`'s switch), so a skipped
    /// frame was a lost hit — then makes the displaced call. See `super::atkspeed`.
    AtkEventLgt { exit_pc: u32 },
    /// The shell slot list's label box (LGT `0x5494`, `subs r1,#6`): widened to two digits
    /// for `SLOT 10..12` of the save-slot window, keeping the game's right edge. Also
    /// re-does the skipped `movs r3,#0xc`. See `super::slotlabel`.
    SlotLabelBoxLgt { exit_pc: u32 },
    /// ComputeMaxHP's store: the gauge max in, the true max published. See `super::playerhp`.
    HpMaxLgt { exit_pc: u32 },
    /// `addHP` replaced for the player's block (virtual HP + set effects). Returns via LR.
    HpAddLgt,
    /// ★ 163차: `computeAbsorb 0x4b978(block, dmg)` replaced — 흡수 % of a 32-bit hit, capped at
    /// a fifth of the true max HP. Returns via LR. See `super::absorb`.
    AbsorbLgt,
    /// The status screen's current / max HP loads. See `super::playerhp`.
    HpDispCurLgt { exit_pc: u32 },
    HpDispMaxLgt { exit_pc: u32 },
    /// ★ 150차: the wear recompute's slot loop (LGT `0x4f130`, `ldr r0,[sp,#0x48]`) — a
    /// reworked accessory's `Stat` third option goes through the game's own `addStat`, so
    /// wear, 장비장착 preview, unequip and load all see it. See `super::thirdopt`.
    AccThirdStatLgt { exit_pc: u32 },
    /// ★ 150차: the in-game bag summary box's first line (LGT `0x41e86`, `adds r5,r0,#0`
    /// after its `sprintf`) — a reworked accessory shows its third option there. The shop
    /// sell grid shares the painter. See `super::accsummary`.
    AccSummaryLgt { exit_pc: u32 },
    /// The same line in the online single bag (LGT `0x124e6`, buffer in `r6`).
    AccSummaryOnlineLgt { exit_pc: u32 },
    /// The same line in the network warehouse (LGT `0x113e6`, `adds r4,r0,#0`; the item is
    /// the cached wire row, not a bag record).
    AccSummaryWarehouseLgt { exit_pc: u32 },
    /// ★ 167차 아바타 옷장: the paper-doll layer setter's join (LGT `0x5191a`,
    /// `add r5,sp,#0x38`, after `0x517bc` picked a sheet for 장착/해제) — the worn costume's sheet
    /// replaces the game's for the six dressed layers, then the displaced `add`. Ships on BOTH
    /// flavours. See `super::avatar`.
    AvatarLayerSetLgt { exit_pc: u32 },
    /// ★ 167차: the world-init loader's join (LGT `0x498c0`, `ldr r4,[pc,#0x2ec]`, after
    /// `0x4957c` picked a sheet for a loaded character) — the same dressing, then the displaced
    /// literal load. See `super::avatar`.
    AvatarLayerLoadLgt { exit_pc: u32 },
    /// ★ 170차: the figure painter's 투구 read (LGT `0x1bb68` / `0x1bbf0`, `ldrsh r3,[r0,rM]`
    /// of EQUIPPED[1]'s item index) — an empty cell reads as worn while the 투구 slot shows a
    /// costume, so a hairstyle costume is drawn without a worn 투구. See `super::avatar`.
    AvatarHelmetPaintLgt { exit_pc: u32 },
    /// ★ 171차: the figure painter's per-piece layer test (LGT `0x1bb56` / `0x1bbde`,
    /// `cmp r4,#6; bne`) — re-done, except that the player's HEAD piece is skipped while the 투구
    /// slot shows a costume that carries the whole head (its own face, drawn smaller) and a 투구
    /// piece follows in the same record. `exit_pc` = the fall-through (`r4 == 6`). See
    /// `super::avatar`.
    AvatarHeadPaintLgt { exit_pc: u32 },
    /// Replaces an inline byte-copy loop whose src/dst/count live in registers
    /// instead of on the stack. Bytes copied = `read(count) + count_offset`.
    /// After the copy, `src`/`dst` are advanced by that count and `count` is
    /// rewritten so it equals what the loop would have left there (i.e.,
    /// `original - bytes`), then the dispatcher jumps to `exit_pc`.
    RegInlineCopy(RegInlineCopy),
}

#[derive(Debug, Clone, Copy)]
pub struct InlineCopy {
    pub dst_offset: i32,
    pub src_offset: i32,
    pub len_offset: i32,
    pub exit_pc: u32,
    /// Set when the loop's outer code re-reads the stack slots after the body;
    /// the dispatcher then writes back `dst+len`, `src+len`, `len=0`.
    pub spill_back: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RegInlineCopy {
    src: ArmRegister,
    dst: ArmRegister,
    count: ArmRegister,
    count_offset: i32,
    exit_pc: u32,
}

/// Scanned across the install-time memory range; each match becomes a `Hook`.
pub struct PatternHook {
    pub tokens: Vec<PatternToken>,
    pub kind_template: PatternHookKind,
}

pub enum PatternHookKind {
    Memcpy,
    Memset,
    Strcpy,
    Strlen,
    InlineCopy {
        /// `None` => filled from the matching `{dst}` / `{src}` / `{len}`
        /// capture (Thumb1 `SUBS Rn, #imm8` byte, negated to a stack offset).
        /// `Some(v)` => pinned by TOML when the pattern omits the capture.
        dst_offset: Option<i32>,
        src_offset: Option<i32>,
        len_offset: Option<i32>,
        /// `None` => filled from the `{exit_b}` capture's decoded branch target.
        exit_pc: Option<u32>,
        spill_back: bool,
    },
    /// Pattern-matched register-resident copy loop. The `src`/`dst`/`count`
    /// registers are read from the pattern's bit-level captures and `exit_pc`
    /// is derived at install time from `match_addr + pattern.len()` (Thumb bit
    /// set).
    RegInlineCopy {
        count_offset: i32,
    },
}

/// Expand static + pattern hooks into a single `Vec<Hook>` whose PCs are final
/// and Thumb-valid. All pattern matching happens here; downstream consumers
/// (overlap check, `apply_hooks`) only see PC + kind, never raw tokens.
///
/// Static hook PCs come from TOML, so we validate the Thumb bit up front —
/// `install_entry` runs `apply_patches` between resolve and apply, and a
/// fatal-after-write would leave guest memory partially modified.
pub fn resolve_hooks(core: &mut ArmCore, entry: &Entry, scan_ranges: &[(u32, u32)]) -> Result<Vec<Hook>> {
    for hook in &entry.hooks {
        if hook.pc & 1 == 0 {
            return Err(WieError::FatalError(format!(
                "entry {}: hook PC {:#x} targets ARM mode; only Thumb (LSB=1) is supported",
                entry.name, hook.pc
            )));
        }
    }
    // ★ 163차: a flavour-tagged hook is dropped HERE, like a tagged patch in
    //   `resolve_patches` — its site keeps the game's own instruction and the boot log says so.
    let build = Flavor::of_build();
    let mut installed: Vec<Hook> = Vec::with_capacity(entry.hooks.len());
    for hook in &entry.hooks {
        if !flavor_keeps(hook.flavor, build) {
            tracing::warn!(
                "{}: not installing the {}-only hook at {:#x} ({:?}) on this {} build",
                entry.name,
                hook.flavor.map_or("?", Flavor::as_str),
                hook.pc,
                hook.kind,
                build.as_str()
            );
            continue;
        }
        installed.push(*hook);
    }

    for pattern in &entry.hook_patterns {
        let matches = scan_pattern(core, &pattern.tokens, scan_ranges)?;
        for (match_addr, pm) in matches {
            let kind = match &pattern.kind_template {
                PatternHookKind::Memcpy => HookKind::Memcpy,
                PatternHookKind::Memset => HookKind::Memset,
                PatternHookKind::Strcpy => HookKind::Strcpy,
                PatternHookKind::Strlen => HookKind::Strlen,
                PatternHookKind::InlineCopy {
                    dst_offset,
                    src_offset,
                    len_offset,
                    exit_pc,
                    spill_back,
                } => {
                    let dst = dst_offset
                        .or_else(|| pm.dst.map(capture_to_offset))
                        .ok_or_else(|| WieError::FatalError(format!("pattern match at {match_addr:#x} missing dst")))?;
                    let src = src_offset
                        .or_else(|| pm.src.map(capture_to_offset))
                        .ok_or_else(|| WieError::FatalError(format!("pattern match at {match_addr:#x} missing src")))?;
                    let len = len_offset
                        .or_else(|| pm.len.map(capture_to_offset))
                        .ok_or_else(|| WieError::FatalError(format!("pattern match at {match_addr:#x} missing len")))?;
                    let exit = if let Some(v) = exit_pc {
                        *v
                    } else {
                        let site = pm
                            .exit_b_site
                            .ok_or_else(|| WieError::FatalError("pattern missing exit_b site".to_string()))?;
                        let bytes = pm
                            .exit_b_bytes
                            .ok_or_else(|| WieError::FatalError("pattern missing exit_b bytes".to_string()))?;
                        decode_exit_b(site, bytes)
                    };
                    HookKind::InlineCopy(InlineCopy {
                        dst_offset: dst,
                        src_offset: src,
                        len_offset: len,
                        exit_pc: exit,
                        spill_back: *spill_back,
                    })
                }
                PatternHookKind::RegInlineCopy { count_offset } => {
                    let src = arm_register_from_index(
                        pm.src_reg
                            .ok_or_else(|| WieError::FatalError(format!("pattern match at {match_addr:#x} missing src register capture")))?,
                    );
                    let dst = arm_register_from_index(
                        pm.dst_reg
                            .ok_or_else(|| WieError::FatalError(format!("pattern match at {match_addr:#x} missing dst register capture")))?,
                    );
                    let count = arm_register_from_index(
                        pm.count_reg
                            .ok_or_else(|| WieError::FatalError(format!("pattern match at {match_addr:#x} missing count register capture")))?,
                    );
                    let exit_pc = match_addr.wrapping_add(pattern.tokens.len() as u32) | 1;
                    HookKind::RegInlineCopy(RegInlineCopy {
                        src,
                        dst,
                        count,
                        count_offset: *count_offset,
                        exit_pc,
                    })
                }
            };
            let pc = match_addr | 1;
            if installed.iter().any(|h| h.pc == pc) {
                tracing::warn!("Hook at {pc:#x} already registered; skipping duplicate match");
                continue;
            }
            installed.push(Hook { pc, kind, flavor: None });
        }
    }

    Ok(installed)
}

/// Patch the SVC instruction at every hook PC and register the dispatcher.
/// `hooks` must already be the fully expanded list from `resolve_hooks`. The
/// dispatcher is registered even when `hooks` is empty so that any later SVC
/// #0x80 (e.g., from guest code that happens to encode the same bytes) routes
/// to a single, named diagnostic instead of falling out of the engine.
pub fn apply_hooks(core: &mut ArmCore, entry_name: &str, hooks: &[Hook]) -> Result<()> {
    let mut registry = BTreeMap::new();
    for hook in hooks {
        debug_assert!(hook.pc & 1 == 1, "resolve_hooks must reject ARM-mode PCs");
        // ★★ 77차-B: item TABLES are the same on main and vanilla. The only
        // flavour difference is this drop. `cfg` comes from flavor.properties
        // via build_android.ps1 (`--cfg hero3_vanilla`). Skipping here — not
        // omitting the rows / count bytes — is what lets a main save start on
        // a vanilla APK.
        if cfg!(hero3_vanilla) && matches!(hook.kind, HookKind::ReaverDropLgt { .. }) {
            tracing::warn!(
                "vanilla: not installing the reaver drop hook at {:#x} (tables stay; only the drop is off)",
                hook.pc
            );
            continue;
        }
        // ★ 140차: vanilla has no 헬 (`flavor::HELL`), so its hardcore clear really does
        // restart in 하드코어 — the game's own ending line is the true one there. The
        // 헬 row stays in the (shared) text table; only this pick is off.
        if cfg!(hero3_vanilla) && matches!(hook.kind, HookKind::EndingLabelLgt { .. }) {
            tracing::warn!(
                "vanilla: not installing the ending label hook at {:#x} (no 헬 on this flavour; the game's 하드코어 line stays)",
                hook.pc
            );
            continue;
        }
        let patch_addr = hook.pc & !1;
        registry.insert(hook.pc, hook.kind);
        // ★ `Blit565` does not just redirect a call — on the fallback path it EMULATES
        // the instruction it displaces (`push {r4, r5, r6, r7, lr}`) and resumes two
        // bytes on. If the build ever moves that function, the hook would emulate a
        // push that was never there and corrupt the guest stack silently, so refuse to
        // install instead. (Hooks have no TOML `expect` field; this is that check.)
        if matches!(hook.kind, HookKind::Blit565 | HookKind::Blit565Lgt | HookKind::BlitLut565Lgt(_)) {
            let mut prologue = [0u8; 2];
            core.read_bytes(patch_addr, &mut prologue)?;
            if prologue != super::blit565::EXPECTED_PROLOGUE {
                return Err(WieError::FatalError(format!(
                    "blit565 hook at {:#x}: expected prologue {:02x?}, found {prologue:02x?}",
                    hook.pc,
                    super::blit565::EXPECTED_PROLOGUE
                )));
            }
        }
        // Same reasoning for the blend hook, which displaces a `ldr r4,[sp,#0x28]` and
        // has to perform it itself when it declines the job.
        if matches!(hook.kind, HookKind::Blend565Lgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::blend565::check_head(head, hook.pc)?;
        }
        // The tint hook is the same shape one function further on: it displaces a
        // `ldr r1,[sp,#8]` and has to perform it itself when it declines the job.
        if matches!(hook.kind, HookKind::TintRect565Lgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::tint565::check_head(head, hook.pc)?;
        }
        // And the orb-sum hook, which displaces `movs r2,#0x13` and resumes at a second
        // fixed address past the store it performs itself.
        if matches!(hook.kind, HookKind::OrbSumLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::orbsum::check_head(head, hook.pc)?;
        }
        // The reaver hook displaces `cmp r7,#0` and performs it by choosing its resume point.
        if matches!(hook.kind, HookKind::ReaverDropLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::reaverdrop::check_head(head, hook.pc)?;
        }
        // The species hook displaces `ldr r0,[r6]` and performs it itself.
        if matches!(hook.kind, HookKind::TowerSpeciesLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::towerspecies::check_head(head, hook.pc)?;
        }
        // The spawn-cap hook displaces `cmp r0,#0xff` and performs it by choosing its resume point.
        if matches!(hook.kind, HookKind::TowerSpawnCapLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::towerspawn::check_head(head, hook.pc)?;
        }
        // The two bag-box entry hooks displace the shared `push {r4-r7,lr}` and emulate it
        // when they decline (`resume_past_push`) — refuse to install on any other head.
        if matches!(
            hook.kind,
            HookKind::BagboxGiveLgt
                | HookKind::BagboxAddLgt
                | HookKind::BagboxTakeLgt
                | HookKind::BagboxSpendLgt
                | HookKind::BagboxRestageLgt
                | HookKind::BagboxRoomCheckLgt
        ) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::bagbox::check_head(head, hook.pc)?;
        }
        // The cash gate re-does the displaced `lsls r0,r0,#0x18`; pin it at install.
        if matches!(hook.kind, HookKind::BagboxCashGateLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::bagbox::check_cash_gate_head(head, hook.pc)?;
        }
        // The transition hook displaces `push {r4-r7,lr}` and emulates it (`resume_past_push`).
        if matches!(hook.kind, HookKind::EndingTransitionLgt) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::ending::check_head(head, hook.pc)?;
        }
        // The count hook displaces the helper's `add sp,#0x18` and performs it itself.
        if matches!(hook.kind, HookKind::BagboxCountLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::bagbox::check_vm_tail(head, hook.pc)?;
        }
        // countOf's exit displaces `add sp,#0xc` (not the VM helper's `#0x18`).
        if matches!(hook.kind, HookKind::BagboxCountOfLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::bagbox::check_countof_tail(head, hook.pc)?;
        }
        // The apply hook displaces `ldr r2,[sp,#4]` and performs it (clamped) itself.
        if matches!(hook.kind, HookKind::DmgApplyLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::dmgwide::check_apply_head(head, hook.pc)?;
        }
        // The display hook displaces the drawer's `push {r4-r7,lr}` (`resume_past_push`).
        if matches!(hook.kind, HookKind::DmgDispLgt) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::dmgwide::check_disp_head(head, hook.pc)?;
        }
        // The SP-cost hooks each re-do a specific displaced instruction; pin it at install.
        if matches!(hook.kind, HookKind::SpCostLevelSpendLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::spcost::check_spend_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::SpCostLevelDispLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::spcost::check_disp_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::SpCostDispFloorLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::spcost::check_disp_floor_head(head, hook.pc)?;
        }
        // 158차: the two stat-cap hooks each re-do one displaced instruction; pin them at install.
        if let HookKind::RespawnRateLgt { .. } | HookKind::ExpRateLgt { .. } = hook.kind {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            let expected = if matches!(hook.kind, HookKind::RespawnRateLgt { .. }) { [0x33, 0x88] } else { [0x21, 0x6b] };
            if head != expected { return Err(WieError::FatalError(format!("gameplay hook head mismatch at {patch_addr:#x}"))); }
        }
        if matches!(hook.kind, HookKind::HitSumClampLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::statcap::check_hit_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::CooldownCapLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::statcap::check_cd_head(head, hook.pc)?;
        }
        // 160차: the defence % cap, the four getter tails and the two combat readers.
        if matches!(hook.kind, HookKind::DefPctCapLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::statwide::check_pct_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::StatWideLgt { .. }) {
            let mut head = [0u8; 4];
            core.read_bytes(patch_addr, &mut head)?;
            super::statwide::check_tail_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::DefReadWideLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::statwide::check_def_read_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::AtkReadWideLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::statwide::check_atk_read_head(head, hook.pc)?;
        }
        // 162차: the loot spawn cap displaces `movs r2,#0`; pin it at install.
        if matches!(hook.kind, HookKind::LootSpawnCapLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::lootlife::check_head(head, hook.pc)?;
        }
        // The recovery hook re-does `subs r3,#1`; pin it at install.
        if matches!(hook.kind, HookKind::AtkRecoverLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::atkspeed::check_recover_head(head, hook.pc)?;
        }
        // The attack-speed hook re-does `lsls r0,r0,#0x18`; pin it at install.
        if matches!(hook.kind, HookKind::AtkSpeedLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::atkspeed::check_head(head, hook.pc)?;
        }
        // The frame-event hook displaces a whole 32-bit `bl 0x4b92c`; pin both halfwords.
        if matches!(hook.kind, HookKind::AtkEventLgt { .. }) {
            let mut head = [0u8; 4];
            core.read_bytes(patch_addr, &mut head)?;
            super::atkspeed::check_event_head(head, hook.pc)?;
        }
        // The slot-label box hook re-does `subs r1,#6`; pin it at install.
        if matches!(hook.kind, HookKind::SlotLabelBoxLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::slotlabel::check_head(head, hook.pc)?;
        }
        // The player-hp hooks displace one instruction each; pin them at install.
        if matches!(
            hook.kind,
            HookKind::HpMaxLgt { .. } | HookKind::HpAddLgt | HookKind::HpDispCurLgt { .. } | HookKind::HpDispMaxLgt { .. }
        ) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            match hook.kind {
                HookKind::HpMaxLgt { .. } => super::playerhp::check_max_head(head, hook.pc)?,
                HookKind::HpAddLgt => super::playerhp::check_add_head(head, hook.pc)?,
                HookKind::HpDispCurLgt { .. } => super::playerhp::check_disp_cur_head(head, hook.pc)?,
                _ => super::playerhp::check_disp_max_head(head, hook.pc)?,
            }
        }
        // 163차: computeAbsorb is replaced at its `push`; pin it at install.
        if matches!(hook.kind, HookKind::AbsorbLgt) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::absorb::check_head(head, hook.pc)?;
        }
        // 150차: the accessory hooks each re-do one displaced instruction; pin it at install.
        if matches!(hook.kind, HookKind::AccThirdStatLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::thirdopt::check_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::AccSummaryLgt { .. } | HookKind::AccSummaryOnlineLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::accsummary::check_bag_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::AccSummaryWarehouseLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::accsummary::check_warehouse_head(head, hook.pc)?;
        }
        // 167차: the two paper-doll joins re-do `add r5,sp,#0x38` / `ldr r4,[pc,#0x2ec]`.
        if matches!(hook.kind, HookKind::AvatarLayerSetLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::avatar::check_set_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::AvatarLayerLoadLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::avatar::check_load_head(head, hook.pc)?;
        }
        if matches!(hook.kind, HookKind::AvatarHelmetPaintLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::avatar::check_paint_head(head, hook.pc)?;
        }
        // 171차: the head hook re-does `cmp r4,#6; bne` — both instructions pinned.
        if let HookKind::AvatarHeadPaintLgt { exit_pc } = hook.kind {
            let mut head = [0u8; 4];
            core.read_bytes(patch_addr, &mut head)?;
            super::avatar::check_head_paint_head(head, hook.pc, exit_pc)?;
        }
        // The ending-label hook re-does `movs r3,#0xaa`; pin it at install.
        if matches!(hook.kind, HookKind::EndingLabelLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::endinglabel::check_head(head, hook.pc)?;
        }
        // Both name-hook sites displace the same `movs r3,#9`.
        if matches!(hook.kind, HookKind::OrbNameLgt { .. }) {
            let mut head = [0u8; 2];
            core.read_bytes(patch_addr, &mut head)?;
            super::orbsum::check_name_head(head, hook.pc)?;
        }
        let instruction: u16 = 0xdf00 | (BINARY_PATCH_SVC as u16 & 0xff);
        core.write_bytes(patch_addr, &instruction.to_le_bytes())?;
        // warn, not info: the Android build filters the wie crates to warn, and "which hooks
        // actually went in" is the first question asked whenever one of them appears dead.
        tracing::warn!("Hook installed at {:#x}: {:?}", hook.pc, hook.kind);
    }
    if !hooks.is_empty() {
        tracing::warn!("Installed {} hooks for {}", hooks.len(), entry_name);
    }
    core.register_svc_handler(BINARY_PATCH_SVC, handle_binary_patch_svc, &Arc::new(registry))?;
    Ok(())
}

/// Negate the unsigned `SUBS Rn, #imm8` immediate: the captured byte is the
/// distance below R7, so the resulting offset is `-imm8`.
fn capture_to_offset(byte: u8) -> i32 {
    -(byte as i32)
}

fn arm_register_from_index(index: u8) -> ArmRegister {
    match index {
        0 => ArmRegister::R0,
        1 => ArmRegister::R1,
        2 => ArmRegister::R2,
        3 => ArmRegister::R3,
        4 => ArmRegister::R4,
        5 => ArmRegister::R5,
        6 => ArmRegister::R6,
        7 => ArmRegister::R7,
        _ => unreachable!("BitMatch only captures 3 bits, value must be 0..=7"),
    }
}

/// Emulate the `push {r4, r5, r6, r7, lr}` that a function-entry hook overwrote, then resume
/// at the instruction after it. STMDB stores the lowest register at the lowest address.
///
/// For a hook that patches a function's first instruction and can decline the job (`Blit565`):
/// declining has to leave the guest in exactly the state the real prologue would have, or the
/// function's own `pop` unwinds a stack frame that was never pushed.
pub(super) fn resume_past_push(core: &mut ArmCore, hook_pc: u32) -> Result<JumpTo> {
    let (r4, r5, r6, r7, lr, sp) = {
        let inner = core.inner.lock();
        (
            inner.engine.reg_read(ArmRegister::R4),
            inner.engine.reg_read(ArmRegister::R5),
            inner.engine.reg_read(ArmRegister::R6),
            inner.engine.reg_read(ArmRegister::R7),
            inner.engine.reg_read(ArmRegister::LR),
            inner.engine.reg_read(ArmRegister::SP),
        )
    };
    let sp = sp.wrapping_sub(20);
    for (i, value) in [r4, r5, r6, r7, lr].into_iter().enumerate() {
        wie_util::write_generic(core, sp + (i as u32) * 4, value)?;
    }
    core.inner.lock().engine.reg_write(ArmRegister::SP, sp);

    // `hook_pc` carries the Thumb bit and points AT the SVC; step over the two bytes.
    Ok(JumpTo(hook_pc + 2))
}

/// Decode a Thumb `B imm11` (`11100 iiiiiiiiiii`) at `b_site`. Returns the
/// target PC with the Thumb bit set.
fn decode_exit_b(b_site: u32, bytes: [u8; 2]) -> u32 {
    let raw = u16::from_le_bytes(bytes);
    let imm11 = (raw & 0x07ff) as i32;
    let offset = if imm11 & 0x400 != 0 { imm11 - 0x800 } else { imm11 };
    let target = (b_site.wrapping_add(4) as i64).wrapping_add((offset * 2) as i64) as u32;
    target | 1
}

type Registry = Arc<BTreeMap<u32, HookKind>>;

async fn handle_binary_patch_svc(core: &mut ArmCore, registry: &mut Registry) -> Result<JumpTo> {
    let (pc, lr) = core.read_pc_lr()?;
    // PC on entry is the address right after the patched 2-byte SVC. Drop any
    // Thumb bit, step back over the SVC, then re-set the Thumb bit because
    // hook PCs are stored that way.
    let hook_pc = (pc.wrapping_sub(2)) | 1;
    let kind = registry
        .get(&hook_pc)
        .copied()
        .ok_or_else(|| WieError::FatalError(format!("binary-patch hook fired at unregistered PC {hook_pc:#x}")))?;

    match kind {
        HookKind::Memcpy => {
            let (dst, src, len) = {
                let inner = core.inner.lock();
                (
                    inner.engine.reg_read(ArmRegister::R0),
                    inner.engine.reg_read(ArmRegister::R1),
                    inner.engine.reg_read(ArmRegister::R2),
                )
            };
            tracing::trace!("hook memcpy(ptr_dst={dst:#x}, ptr_src={src:#x}, len={len:#x})");
            stdlib::memcpy(core, &mut (), dst, src, len).await?;
            Ok(JumpTo(lr))
        }
        HookKind::Memset => {
            let (dst, val, len) = {
                let inner = core.inner.lock();
                (
                    inner.engine.reg_read(ArmRegister::R0),
                    inner.engine.reg_read(ArmRegister::R1),
                    inner.engine.reg_read(ArmRegister::R2),
                )
            };
            tracing::trace!("hook memset(ptr_dst={dst:#x}, val={:#x}, len={len:#x})", val as u8);
            stdlib::memset(core, &mut (), dst, val, len).await?;
            Ok(JumpTo(lr))
        }
        HookKind::Strcpy => {
            let (dst, src) = {
                let inner = core.inner.lock();
                (inner.engine.reg_read(ArmRegister::R0), inner.engine.reg_read(ArmRegister::R1))
            };
            tracing::trace!("hook strcpy(ptr_dst={dst:#x}, ptr_src={src:#x})");
            stdlib::strcpy(core, &mut (), dst, src).await?;
            Ok(JumpTo(lr))
        }
        HookKind::Strlen => {
            let s = core.inner.lock().engine.reg_read(ArmRegister::R0);
            let len = stdlib::strlen(core, &mut (), s).await?;
            tracing::trace!("hook strlen(ptr_str={s:#x}) -> {len:#x}");
            core.inner.lock().engine.reg_write(ArmRegister::R0, len);
            Ok(JumpTo(lr))
        }
        HookKind::Blit565 => super::blit565::handle(core, hook_pc, lr).await,
        HookKind::Blit565Lgt => super::blit565::handle_lgt(core, hook_pc, lr).await,
        // Not `lr`: this hook is on a loop head inside a function, not on a call, so the
        // resume point is the function's own tail from the table.
        HookKind::Blend565Lgt { exit_pc } => super::blend565::handle_lgt(core, hook_pc, exit_pc).await,
        HookKind::BlitLut565Lgt(tables) => super::blitlut565::handle_lgt(core, hook_pc, tables).await,
        // Same loop-head-not-a-call situation as the blend hook, one function on.
        HookKind::TintRect565Lgt { exit_pc, lut } => super::tint565::handle_lgt(core, hook_pc, exit_pc, lut).await,
        // Like the blend hook, this sits inside a function rather than on a call, so the
        // resume point comes from the table and not from `lr`.
        HookKind::OrbSumLgt { exit_pc } => super::orbsum::handle_lgt(core, hook_pc, exit_pc).await,
        HookKind::OrbNameLgt { tables, exit_pc } => super::orbsum::handle_name_lgt(core, hook_pc, tables, exit_pc).await,
        // Inside a function too; both resume points come from the table.
        HookKind::ReaverDropLgt { exit_pc, skip_pc } => super::reaverdrop::handle_lgt(core, hook_pc, exit_pc, skip_pc).await,
        // Inside a function; the resume point is the instruction after the displaced load.
        HookKind::TowerSpeciesLgt { exit_pc } => super::towerspecies::handle_lgt(core, hook_pc, exit_pc).await,
        // Inside the spawn loop; both resume points come from the table.
        HookKind::TowerSpawnCapLgt { exit_pc, skip_pc } => super::towerspawn::handle_lgt(core, hook_pc, exit_pc, skip_pc).await,
        HookKind::BagboxGiveLgt => super::bagbox::handle_give_lgt(core, hook_pc, lr).await,
        HookKind::BagboxAddLgt => super::bagbox::handle_add_lgt(core, hook_pc, lr).await,
        HookKind::BagboxTakeLgt => super::bagbox::handle_take_lgt(core, hook_pc, lr).await,
        // Inside a function; the resume point is the instruction after the displaced `add sp`.
        HookKind::BagboxCountLgt { exit_pc } => super::bagbox::handle_count_lgt(core, hook_pc, exit_pc).await,
        HookKind::BagboxCountOfLgt { exit_pc } => super::bagbox::handle_countof_lgt(core, hook_pc, exit_pc).await,
        HookKind::BagboxSpendLgt => super::bagbox::handle_spend_lgt(core, hook_pc, lr).await,
        HookKind::BagboxRestageLgt => super::bagbox::handle_restage_lgt(core, hook_pc, lr).await,
        HookKind::BagboxRoomCheckLgt => super::bagbox::handle_room_check_lgt(core, hook_pc, lr).await,
        HookKind::BagboxCashGateLgt { exit_pc } => super::bagbox::handle_cash_gate_lgt(core, hook_pc, exit_pc).await,
        // Observes the 2회차 transition and resumes into it.
        HookKind::EndingTransitionLgt => super::ending::handle_lgt(core, hook_pc).await,
        // The ending window's row pick; resumes at the `b` that stores the row.
        HookKind::EndingLabelLgt { exit_pc } => super::endinglabel::handle_lgt(core, hook_pc, exit_pc).await,
        // Inside `hitMonster`; resumes after the displaced `ldr r2,[sp,#4]`.
        HookKind::DmgApplyLgt { exit_pc } => super::dmgwide::handle_apply_lgt(core, hook_pc, exit_pc).await,
        HookKind::DmgDispLgt => super::dmgwide::handle_disp_lgt(core, hook_pc).await,
        // Skill SP cost: clamp the level fed to the formula to 70, and floor the displayed value.
        HookKind::SpCostLevelSpendLgt { exit_pc } => super::spcost::handle_level_spend_lgt(core, hook_pc, exit_pc).await,
        HookKind::SpCostLevelDispLgt { exit_pc } => super::spcost::handle_level_disp_lgt(core, hook_pc, exit_pc).await,
        HookKind::SpCostDispFloorLgt { exit_pc } => super::spcost::handle_disp_floor_lgt(core, hook_pc, exit_pc).await,
        // The 명중 byte store clamped at 255, the cooldown sum capped at 70 %.
        HookKind::RespawnRateLgt { exit_pc } => super::gameplay::respawn(core, exit_pc).await,
        HookKind::ExpRateLgt { exit_pc } => super::gameplay::exp(core, exit_pc).await,
        HookKind::HitSumClampLgt { exit_pc } => super::statcap::handle_hit_clamp_lgt(core, hook_pc, exit_pc).await,
        HookKind::CooldownCapLgt { exit_pc } => super::statcap::handle_cooldown_cap_lgt(core, hook_pc, exit_pc).await,
        HookKind::DefPctCapLgt { exit_pc } => super::statwide::handle_def_pct_cap_lgt(core, hook_pc, exit_pc).await,
        HookKind::StatWideLgt { exit_pc } => super::statwide::handle_stat_wide_lgt(core, hook_pc, exit_pc).await,
        HookKind::DefReadWideLgt { exit_pc } => super::statwide::handle_def_read_wide_lgt(core, hook_pc, exit_pc).await,
        HookKind::AtkReadWideLgt { exit_pc } => super::statwide::handle_atk_read_wide_lgt(core, hook_pc, exit_pc).await,
        // Inside the spawn loop; the resume point is the `ldrsb` after the displaced `movs`.
        HookKind::LootSpawnCapLgt { exit_pc } => super::lootlife::handle_lgt(core, hook_pc, exit_pc).await,
        // State 4's recovery countdown, ticked down by more than one.
        HookKind::AtkRecoverLgt { exit_pc } => super::atkspeed::handle_recover_lgt(core, hook_pc, exit_pc).await,
        // The player's pose clock, dragged towards this pose's own length (state 2 / 4 only).
        HookKind::AtkSpeedLgt { exit_pc } => super::atkspeed::handle_lgt(core, hook_pc, exit_pc).await,
        // The frame events that clock stepped over, then the displaced `bl 0x4b92c`.
        HookKind::AtkEventLgt { exit_pc } => super::atkspeed::handle_event_lgt(core, hook_pc, exit_pc).await,
        // The slot list's label box, two digits wide when the label has two.
        HookKind::SlotLabelBoxLgt { exit_pc } => super::slotlabel::handle_lgt(core, hook_pc, exit_pc).await,
        HookKind::HpMaxLgt { exit_pc } => super::playerhp::handle_max_lgt(core, hook_pc, exit_pc).await,
        HookKind::HpAddLgt => super::playerhp::handle_add_lgt(core, hook_pc).await,
        HookKind::AbsorbLgt => super::absorb::handle_lgt(core, hook_pc).await,
        HookKind::HpDispCurLgt { exit_pc } => super::playerhp::handle_disp_cur_lgt(core, hook_pc, exit_pc).await,
        HookKind::HpDispMaxLgt { exit_pc } => super::playerhp::handle_disp_max_lgt(core, hook_pc, exit_pc).await,
        // 150차: the wear recompute's slot loop, then the three summary-line painters.
        HookKind::AccThirdStatLgt { exit_pc } => super::thirdopt::handle_lgt(core, hook_pc, exit_pc).await,
        HookKind::AccSummaryLgt { exit_pc } => super::accsummary::handle_bag_lgt(core, hook_pc, exit_pc).await,
        HookKind::AccSummaryOnlineLgt { exit_pc } => super::accsummary::handle_online_lgt(core, hook_pc, exit_pc).await,
        HookKind::AccSummaryWarehouseLgt { exit_pc } => super::accsummary::handle_warehouse_lgt(core, hook_pc, exit_pc).await,
        // 167차: the two paper-doll joins; each resumes after its displaced instruction.
        HookKind::AvatarLayerSetLgt { exit_pc } => super::avatar::handle_set_lgt(core, hook_pc, exit_pc).await,
        HookKind::AvatarLayerLoadLgt { exit_pc } => super::avatar::handle_load_lgt(core, hook_pc, exit_pc).await,
        HookKind::AvatarHelmetPaintLgt { exit_pc } => super::avatar::handle_helmet_paint_lgt(core, hook_pc, exit_pc).await,
        HookKind::AvatarHeadPaintLgt { exit_pc } => super::avatar::handle_head_paint_lgt(core, hook_pc, exit_pc).await,
        HookKind::InlineCopy(spec) => {
            let r7 = core.inner.lock().engine.reg_read(ArmRegister::R7);
            let dst_slot = r7.wrapping_add(spec.dst_offset as u32);
            let src_slot = r7.wrapping_add(spec.src_offset as u32);
            let len_slot = r7.wrapping_add(spec.len_offset as u32);
            let dst: u32 = read_generic(core, dst_slot)?;
            let src: u32 = read_generic(core, src_slot)?;
            let len: u32 = read_generic(core, len_slot)?;
            tracing::trace!(
                "hook inline_copy(ptr_dst={dst:#x}, ptr_src={src:#x}, len={len:#x}, exit={:#x})",
                spec.exit_pc
            );
            stdlib::memcpy(core, &mut (), dst, src, len).await?;
            if spec.spill_back {
                core.write_bytes(dst_slot, &dst.wrapping_add(len).to_le_bytes())?;
                core.write_bytes(src_slot, &src.wrapping_add(len).to_le_bytes())?;
                core.write_bytes(len_slot, &0u32.to_le_bytes())?;
            }
            Ok(JumpTo(spec.exit_pc))
        }
        HookKind::RegInlineCopy(spec) => {
            let (src, dst, count_initial) = {
                let inner = core.inner.lock();
                (
                    inner.engine.reg_read(spec.src),
                    inner.engine.reg_read(spec.dst),
                    inner.engine.reg_read(spec.count),
                )
            };
            let count = count_initial.wrapping_add(spec.count_offset as u32);
            tracing::trace!(
                "hook reg_inline_copy(src={src:#x}, dst={dst:#x}, count={count:#x}, exit={:#x})",
                spec.exit_pc
            );
            stdlib::memcpy(core, &mut (), dst, src, count).await?;
            let mut inner = core.inner.lock();
            inner.engine.reg_write(spec.src, src.wrapping_add(count));
            inner.engine.reg_write(spec.dst, dst.wrapping_add(count));
            inner.engine.reg_write(spec.count, count_initial.wrapping_sub(count));
            Ok(JumpTo(spec.exit_pc))
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};

    use wie_util::ByteRead;

    use super::*;
    use crate::function::{RegisteredFunction, RegisteredFunctionHolder};

    fn registry_with(pc: u32, kind: HookKind) -> Registry {
        let mut map = BTreeMap::new();
        map.insert(pc, kind);
        Arc::new(map)
    }

    /// Set PC to where it would be on entry to the SVC handler for a hook at
    /// `hook_pc` (svc_addr + 2, i.e. just past the patched 2-byte SVC).
    fn set_post_svc_pc(core: &mut ArmCore, hook_pc: u32) {
        let mut inner = core.inner.lock();
        inner.engine.reg_write(ArmRegister::PC, (hook_pc & !1).wrapping_add(2));
    }

    fn entry_with_static(name: &str, hooks: Vec<Hook>) -> Entry {
        Entry {
            hash: Some([0u8; 16]),
            name: name.into(),
            hooks,
            hook_patterns: vec![],
            patches: vec![],
            patch_patterns: vec![],
        }
    }

    #[test]
    fn resolve_hooks_rejects_arm_mode_pc() -> Result<()> {
        let entry = entry_with_static(
            "arm-mode",
            vec![Hook {
                pc: 0x2000, // LSB=0 => ARM mode
                kind: HookKind::Memcpy,
                flavor: None,
            }],
        );
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x1000)?;

        let err = resolve_hooks(&mut core, &entry, &[]).unwrap_err();
        let msg = alloc::format!("{err}");
        assert!(msg.contains("ARM mode"), "unexpected error: {msg}");
        Ok(())
    }

    /// ★ 163차: a hook tagged for the other flavour is dropped before it is installed or
    /// counted — the site keeps the game's own instruction.
    #[test]
    fn resolve_hooks_drops_the_other_flavours_hook_and_keeps_its_own() -> Result<()> {
        let other = match Flavor::of_build() {
            Flavor::Main => Flavor::Vanilla,
            Flavor::Vanilla => Flavor::Main,
        };
        let entry = entry_with_static(
            "flavoured",
            vec![
                Hook { pc: 0x2001, kind: HookKind::Memcpy, flavor: None },
                Hook { pc: 0x2011, kind: HookKind::Memcpy, flavor: Some(other) },
                Hook { pc: 0x2021, kind: HookKind::Memcpy, flavor: Some(Flavor::of_build()) },
            ],
        );
        let mut core = ArmCore::new(false, None)?;
        let pcs: Vec<u32> = resolve_hooks(&mut core, &entry, &[])?.iter().map(|h| h.pc).collect();
        assert_eq!(pcs, [0x2001, 0x2021]);
        Ok(())
    }

    #[test]
    fn apply_hooks_writes_thumb_svc_instruction() -> Result<()> {
        let entry = entry_with_static(
            "patch",
            vec![Hook {
                pc: 0x2001, // Thumb
                kind: HookKind::Memcpy,
                flavor: None,
            }],
        );
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x1000)?;
        core.write_bytes(0x2000, &[0xaa, 0xbb])?;

        let hooks = resolve_hooks(&mut core, &entry, &[])?;
        apply_hooks(&mut core, &entry.name, &hooks)?;

        let mut buf = [0u8; 2];
        core.read_bytes(0x2000, &mut buf)?;
        assert_eq!(buf, [BINARY_PATCH_SVC as u8, 0xdf]);
        Ok(())
    }

    #[futures_test::test]
    async fn memcpy_dispatch_copies_bytes_and_returns_via_lr() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10000, 0x1000)?;

        let src = 0x10000u32;
        let dst = 0x10400u32;
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        core.write_bytes(src, &data)?;

        let hook_pc = 0x10001u32;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R0, dst);
            inner.engine.reg_write(ArmRegister::R1, src);
            inner.engine.reg_write(ArmRegister::R2, data.len() as u32);
            inner.engine.reg_write(ArmRegister::LR, 0xdead_beef);
        }
        set_post_svc_pc(&mut core, hook_pc);

        let registry = registry_with(hook_pc, HookKind::Memcpy);
        RegisteredFunctionHolder::new(handle_binary_patch_svc, &registry).call(&mut core).await?;

        let mut out = [0u8; 8];
        core.read_bytes(dst, &mut out)?;
        assert_eq!(out, data);
        assert_eq!(core.inner.lock().engine.reg_read(ArmRegister::PC), 0xdead_beef & !1);
        Ok(())
    }

    #[futures_test::test]
    async fn memset_dispatch_fills_bytes() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10000, 0x1000)?;

        let dst = 0x10000u32;
        let len = 16u32;
        let hook_pc = 0x10401u32;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R0, dst);
            inner.engine.reg_write(ArmRegister::R1, 0xab);
            inner.engine.reg_write(ArmRegister::R2, len);
            inner.engine.reg_write(ArmRegister::LR, 0x3000);
        }
        set_post_svc_pc(&mut core, hook_pc);

        let registry = registry_with(hook_pc, HookKind::Memset);
        RegisteredFunctionHolder::new(handle_binary_patch_svc, &registry).call(&mut core).await?;

        let mut out = [0u8; 16];
        core.read_bytes(dst, &mut out)?;
        assert_eq!(out, [0xab; 16]);
        Ok(())
    }

    #[futures_test::test]
    async fn strcpy_dispatch_copies_null_terminated_string_and_returns_via_lr() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10000, 0x1000)?;

        let src = 0x10000u32;
        let dst = 0x10400u32;
        let s = b"hello, world!\0";
        core.write_bytes(src, s)?;
        core.write_bytes(dst, &[0xff; 32])?;

        let hook_pc = 0x10801u32;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R0, dst);
            inner.engine.reg_write(ArmRegister::R1, src);
            inner.engine.reg_write(ArmRegister::LR, 0xcafe_babe);
        }
        set_post_svc_pc(&mut core, hook_pc);

        let registry = registry_with(hook_pc, HookKind::Strcpy);
        RegisteredFunctionHolder::new(handle_binary_patch_svc, &registry).call(&mut core).await?;

        let mut out = [0u8; 14];
        core.read_bytes(dst, &mut out)?;
        assert_eq!(&out, s);
        let mut sentinel = [0u8; 1];
        core.read_bytes(dst + s.len() as u32, &mut sentinel)?;
        assert_eq!(sentinel, [0xff]);

        let inner = core.inner.lock();
        assert_eq!(inner.engine.reg_read(ArmRegister::R0), dst);
        assert_eq!(inner.engine.reg_read(ArmRegister::PC), 0xcafe_babe & !1);
        Ok(())
    }

    #[futures_test::test]
    async fn strlen_dispatch_returns_length_in_r0() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10000, 0x1000)?;

        let str_ptr = 0x10100u32;
        let s = b"abcdef\0";
        core.write_bytes(str_ptr, s)?;

        let hook_pc = 0x10c01u32;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R0, str_ptr);
            inner.engine.reg_write(ArmRegister::LR, 0x1234_5678);
        }
        set_post_svc_pc(&mut core, hook_pc);

        let registry = registry_with(hook_pc, HookKind::Strlen);
        RegisteredFunctionHolder::new(handle_binary_patch_svc, &registry).call(&mut core).await?;

        let inner = core.inner.lock();
        assert_eq!(inner.engine.reg_read(ArmRegister::R0), 6);
        assert_eq!(inner.engine.reg_read(ArmRegister::PC), 0x1234_5678 & !1);
        Ok(())
    }

    #[futures_test::test]
    async fn inline_copy_dispatch_reads_frame_copies_and_jumps_to_exit() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x10000, 0x2000)?;

        let src = 0x10000u32;
        let dst = 0x10800u32;
        let payload = [0xaa, 0xbb, 0xcc, 0xdd];
        core.write_bytes(src, &payload)?;

        let frame = 0x11000u32;
        core.write_bytes(frame, &dst.to_le_bytes())?;
        core.write_bytes(frame + 4, &src.to_le_bytes())?;
        core.write_bytes(frame + 8, &(payload.len() as u32).to_le_bytes())?;

        let hook_pc = 0x10201u32;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R7, frame);
        }
        set_post_svc_pc(&mut core, hook_pc);

        let spec = InlineCopy {
            dst_offset: 0,
            src_offset: 4,
            len_offset: 8,
            exit_pc: 0x10401,
            spill_back: true,
        };
        let registry = registry_with(hook_pc, HookKind::InlineCopy(spec));
        RegisteredFunctionHolder::new(handle_binary_patch_svc, &registry).call(&mut core).await?;

        let mut out = [0u8; 4];
        core.read_bytes(dst, &mut out)?;
        assert_eq!(out, payload);

        let mut slot = [0u8; 4];
        core.read_bytes(frame, &mut slot)?;
        assert_eq!(u32::from_le_bytes(slot), dst + payload.len() as u32);
        core.read_bytes(frame + 4, &mut slot)?;
        assert_eq!(u32::from_le_bytes(slot), src + payload.len() as u32);
        core.read_bytes(frame + 8, &mut slot)?;
        assert_eq!(u32::from_le_bytes(slot), 0);

        assert_eq!(core.inner.lock().engine.reg_read(ArmRegister::PC), 0x10400);
        Ok(())
    }

    #[futures_test::test]
    async fn install_then_execute_hits_dispatcher_end_to_end() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x20000, 0x2000)?;
        core.map(0x30000, 0x1000)?;

        let src = 0x30000u32;
        let dst = 0x30200u32;
        let payload = [9u8, 8, 7, 6];
        core.write_bytes(src, &payload)?;

        let hook_pc = 0x20001u32;
        let entry = entry_with_static(
            "e2e",
            vec![Hook {
                pc: hook_pc,
                kind: HookKind::Memcpy,
                flavor: None,
            }],
        );
        let hooks = resolve_hooks(&mut core, &entry, &[])?;
        apply_hooks(&mut core, &entry.name, &hooks)?;

        let mut opcode = [0u8; 2];
        core.read_bytes(0x20000, &mut opcode)?;
        assert_eq!(opcode[1], 0xdf);
        assert_eq!(opcode[0] as u32, BINARY_PATCH_SVC);

        let return_addr = 0x40000u32 | 1;
        {
            let mut inner = core.inner.lock();
            inner.engine.reg_write(ArmRegister::R0, dst);
            inner.engine.reg_write(ArmRegister::R1, src);
            inner.engine.reg_write(ArmRegister::R2, payload.len() as u32);
            inner.engine.reg_write(ArmRegister::LR, return_addr);
            inner.engine.reg_write(ArmRegister::PC, hook_pc);

            let cpsr = inner.engine.reg_read(ArmRegister::Cpsr);
            inner.engine.reg_write(ArmRegister::Cpsr, (cpsr & !0x3f) | 0x1f | 0x20);
            inner.engine.reg_write(ArmRegister::SP, 0x20f00);
        }

        let result = {
            let mut inner = core.inner.lock();
            inner.engine.run(0, 10)?
        };
        let category = match result {
            crate::engine::EngineRunResult::Svc { category, lr, spsr } => {
                let mut inner = core.inner.lock();
                inner.engine.reg_write(ArmRegister::Cpsr, spsr);
                inner.engine.reg_write(ArmRegister::PC, lr);
                category
            }
            _ => panic!("expected Svc"),
        };
        assert_eq!(category, BINARY_PATCH_SVC);

        let registry = registry_with(hook_pc, HookKind::Memcpy);
        let mut core_clone = core.clone();
        RegisteredFunctionHolder::new(handle_binary_patch_svc, &registry)
            .call(&mut core_clone)
            .await?;

        let mut out = [0u8; 4];
        core.read_bytes(dst, &mut out)?;
        assert_eq!(out, payload);

        let inner = core.inner.lock();
        assert_eq!(inner.engine.reg_read(ArmRegister::PC), return_addr & !1);
        Ok(())
    }

    #[test]
    fn pattern_scan_matches_single_hit() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x50000, 0x200)?;

        let pat_bytes = [0xaa, 0xbb, 0xcc, 0xdd];
        let match_addr = 0x50020u32;
        core.write_bytes(match_addr, &pat_bytes)?;

        let entry = Entry {
            hash: None,
            name: "scan-single".into(),
            hooks: vec![],
            hook_patterns: vec![PatternHook {
                tokens: vec![
                    PatternToken::Literal(0xaa),
                    PatternToken::Literal(0xbb),
                    PatternToken::Literal(0xcc),
                    PatternToken::Literal(0xdd),
                ],
                kind_template: PatternHookKind::Memcpy,
            }],
            patches: vec![],
            patch_patterns: vec![],
        };

        let hooks = resolve_hooks(&mut core, &entry, &[(0x50000, 0x200)])?;
        apply_hooks(&mut core, &entry.name, &hooks)?;

        let mut out = [0u8; 2];
        core.read_bytes(match_addr, &mut out)?;
        assert_eq!(out[1], 0xdf);
        assert_eq!(out[0] as u32, BINARY_PATCH_SVC);
        Ok(())
    }

    #[test]
    fn pattern_scan_capture_to_offset_negates_imm8() {
        assert_eq!(capture_to_offset(0x08), -8);
        assert_eq!(capture_to_offset(0x0c), -12);
        // imm8 in `SUBS Rn, #imm8` is unsigned 0..=255, so high values must
        // negate to large negative offsets, not wrap through `i8`.
        assert_eq!(capture_to_offset(0x80), -128);
        assert_eq!(capture_to_offset(0xfc), -252);
    }

    #[test]
    fn pattern_scan_exit_b_computes_forward_branch_target() {
        let bytes = [0x02, 0xe0];
        let site = 0x100u32;
        let exit = decode_exit_b(site, bytes);
        assert_eq!(exit, (site + 4 + 4) | 1);

        let neg = decode_exit_b(site, [0xfe, 0xe7]);
        assert_eq!(neg, site | 1);
    }

    #[test]
    fn pattern_duplicate_pc_warns_once_and_skips() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x60000, 0x100)?;
        core.write_bytes(0x60010, &[0x11, 0x22, 0x33, 0x44])?;

        let entry = Entry {
            hash: None,
            name: "dup".into(),
            hooks: vec![],
            hook_patterns: vec![
                PatternHook {
                    tokens: vec![PatternToken::Literal(0x11), PatternToken::Literal(0x22)],
                    kind_template: PatternHookKind::Memcpy,
                },
                PatternHook {
                    tokens: vec![PatternToken::Literal(0x11), PatternToken::Literal(0x22)],
                    kind_template: PatternHookKind::Memcpy,
                },
            ],
            patches: vec![],
            patch_patterns: vec![],
        };
        let hooks = resolve_hooks(&mut core, &entry, &[(0x60000, 0x100)])?;
        assert_eq!(hooks.len(), 1, "duplicate PC should be skipped");
        Ok(())
    }
}
