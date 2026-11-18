use alloc::{string::String, vec::Vec};

use serde::Deserialize;

use super::{
    CaptureName, Entry, PatternToken,
    hook::{Hook, HookKind, InlineCopy, PatternHook, PatternHookKind},
    patch::{Flavor, PatchSpec, PatternPatchSpec},
};

const BINARY_PATCHES_TOML: &str = include_str!("../../../data/binary_patches.toml");

pub fn binary_patches() -> Vec<Entry> {
    let doc: RawDoc = toml::from_str(BINARY_PATCHES_TOML).expect("parse data/binary_patches.toml");
    doc.entry.into_iter().map(RawEntry::into_entry).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDoc {
    entry: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    hash: Option<String>,
    name: String,
    #[serde(default)]
    hook: Vec<RawHook>,
    #[serde(default)]
    patch: Vec<RawPatch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHook {
    kind: KindTag,
    pc: Option<u32>,
    pattern: Option<String>,
    dst_offset: Option<i32>,
    src_offset: Option<i32>,
    len_offset: Option<i32>,
    exit_pc: Option<u32>,
    spill_back: Option<bool>,
    count_offset: Option<i32>,
    /// `blit_lut565_lgt` only: the three per-build lookup-table addresses. `blend_a` is
    /// shared with `tint_rect565_lgt`, which indexes the same byte table.
    alpha_map: Option<u32>,
    blend_t: Option<u32>,
    blend_a: Option<u32>,
    /// `orb_name_lgt` only: the `.bss` word holding the string-table base, and a
    /// guest-writable scratch buffer for the composed prefix.
    string_table: Option<u32>,
    scratch: Option<u32>,
    /// `reaver_drop_lgt` only: the second resume point (the function tail taken when
    /// nothing is to be dropped); `exit_pc` is the spawn path.
    skip_pc: Option<u32>,
    /// ★ 163차: `"main"` | `"vanilla"` — install on that APK flavour only, the same key a
    /// patch takes. pc-only.
    flavor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPatch {
    pc: Option<u32>,
    pattern: Option<String>,
    bytes: String,
    expect: Option<String>,
    offset: Option<u32>,
    /// `"main"` | `"vanilla"` — install on that APK flavour only (114차). pc-only.
    flavor: Option<String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum KindTag {
    Memcpy,
    Memset,
    Strcpy,
    Strlen,
    InlineCopy,
    RegInlineCopy,
    Blit565,
    Blit565Lgt,
    Blend565Lgt,
    BlitLut565Lgt,
    TintRect565Lgt,
    OrbSumLgt,
    OrbNameLgt,
    ReaverDropLgt,
    TowerSpeciesLgt,
    TowerSpawnCapLgt,
    BagboxGiveLgt,
    BagboxAddLgt,
    BagboxTakeLgt,
    BagboxCountLgt,
    BagboxCountOfLgt,
    BagboxSpendLgt,
    BagboxRestageLgt,
    BagboxRoomCheckLgt,
    BagboxCashGateLgt,
    EndingTransitionLgt,
    EndingLabelLgt,
    DmgApplyLgt,
    DmgDispLgt,
    SpCostLevelSpendLgt,
    SpCostLevelDispLgt,
    SpCostDispFloorLgt,
    HitSumClampLgt,
    RespawnRateLgt,
    ExpRateLgt,
    CooldownCapLgt,
    DefPctCapLgt,
    StatWideLgt,
    DefReadWideLgt,
    AtkReadWideLgt,
    LootSpawnCapLgt,
    AtkRecoverLgt,
    AtkSpeedLgt,
    AtkEventLgt,
    SlotLabelBoxLgt,
    HpMaxLgt,
    HpAddLgt,
    HpDispCurLgt,
    HpDispMaxLgt,
    AbsorbLgt,
    AccThirdStatLgt,
    AccSummaryLgt,
    AccSummaryOnlineLgt,
    AccSummaryWarehouseLgt,
    AvatarLayerSetLgt,
    AvatarLayerLoadLgt,
    AvatarHelmetPaintLgt,
    AvatarHeadPaintLgt,
}

impl RawEntry {
    fn into_entry(self) -> Entry {
        let name = self.name;
        let hash = self.hash.as_deref().map(|s| parse_hash(s, &name));
        let mut hooks = Vec::new();
        let mut hook_patterns = Vec::new();
        for raw in self.hook {
            match (raw.pc, raw.pattern.as_deref()) {
                (Some(pc), None) => hooks.push(Hook {
                    pc,
                    kind: pc_kind(&raw, &name),
                    flavor: hook_flavor(&raw, &name),
                }),
                (None, Some(pat)) => {
                    if raw.flavor.is_some() {
                        panic!(
                            "entry {name}: hook `flavor` is only valid with `pc` (a generic pattern belongs to the toolchain, not to an APK flavour)"
                        );
                    }
                    let tokens = parse_pattern(pat, &name);
                    let kind_template = pattern_template(&raw, &tokens, &name);
                    hook_patterns.push(PatternHook { tokens, kind_template });
                }
                (Some(_), Some(_)) => panic!("entry {name}: hook cannot specify both `pc` and `pattern`"),
                (None, None) => panic!("entry {name}: hook must specify either `pc` or `pattern`"),
            }
        }
        if hash.is_none() && !hooks.is_empty() {
            panic!("entry {name}: hash is required when pc-based hooks are present (a pc only makes sense for a specific binary)");
        }
        let mut patches = Vec::new();
        let mut patch_patterns = Vec::new();
        for raw in self.patch {
            match into_patch_kind(raw, hash.is_some(), &name) {
                ParsedPatch::Pc(spec) => patches.push(spec),
                ParsedPatch::Pattern(spec) => patch_patterns.push(spec),
            }
        }
        Entry {
            hash,
            name,
            hooks,
            hook_patterns,
            patches,
            patch_patterns,
        }
    }
}

/// A hook's `flavor` key, parsed with the patch rule: anything but the two names is a table
/// bug and fails loudly instead of installing on every flavour.
fn hook_flavor(raw: &RawHook, entry_name: &str) -> Option<Flavor> {
    raw.flavor
        .as_deref()
        .map(|s| Flavor::parse(s).unwrap_or_else(|| panic!("entry {entry_name}: hook `flavor` must be \"main\" or \"vanilla\" (got {s:?})")))
}

enum ParsedPatch {
    Pc(PatchSpec),
    Pattern(PatternPatchSpec),
}

fn into_patch_kind(raw: RawPatch, has_hash: bool, entry_name: &str) -> ParsedPatch {
    let bytes = parse_hex_bytes(&raw.bytes, entry_name, "bytes");
    if bytes.is_empty() {
        panic!("entry {entry_name}: patch `bytes` must be at least 1 byte");
    }
    reject_svc_pattern(&bytes, entry_name);
    let flavor = raw
        .flavor
        .as_deref()
        .map(|s| Flavor::parse(s).unwrap_or_else(|| panic!("entry {entry_name}: patch `flavor` must be \"main\" or \"vanilla\" (got {s:?})")));
    let expect = raw.expect.as_deref().map(|s| {
        let v = parse_hex_bytes(s, entry_name, "expect");
        if v.len() != bytes.len() {
            panic!(
                "entry {entry_name}: patch `expect` length ({}) must equal `bytes` length ({})",
                v.len(),
                bytes.len()
            );
        }
        v
    });

    match (raw.pc, raw.pattern.as_deref()) {
        (Some(_), Some(_)) => panic!("entry {entry_name}: patch cannot specify both `pc` and `pattern`"),
        (None, None) => panic!("entry {entry_name}: patch must specify either `pc` or `pattern`"),
        (Some(pc), None) => {
            if !has_hash {
                panic!("entry {entry_name}: patch `pc` requires entry `hash` (a pc only makes sense for a specific binary)");
            }
            if raw.offset.is_some() {
                panic!("entry {entry_name}: patch `offset` is meaningless with `pc` (only valid for `pattern`)");
            }
            ParsedPatch::Pc(PatchSpec { pc, bytes, expect, flavor })
        }
        (None, Some(pat)) => {
            if flavor.is_some() {
                panic!(
                    "entry {entry_name}: patch `flavor` is only valid with `pc` (a generic pattern belongs to the toolchain, not to an APK flavour)"
                );
            }
            let tokens = parse_pattern(pat, entry_name);
            reject_patch_capture_tokens(&tokens, entry_name);
            let offset = raw.offset.unwrap_or(0);
            let pat_len = tokens.len() as u32;
            let bytes_len = bytes.len() as u32;
            let end = offset
                .checked_add(bytes_len)
                .unwrap_or_else(|| panic!("entry {entry_name}: patch `offset` ({offset}) + `bytes` length ({bytes_len}) overflows u32"));
            if end > pat_len {
                panic!("entry {entry_name}: patch `offset` ({offset}) + `bytes` length ({bytes_len}) exceeds pattern length ({pat_len})");
            }
            ParsedPatch::Pattern(PatternPatchSpec {
                tokens,
                bytes,
                expect,
                offset,
            })
        }
    }
}

fn parse_hex_bytes(s: &str, entry_name: &str, field: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        if tok.len() != 2 || !tok.chars().all(|c| c.is_ascii_hexdigit()) {
            panic!("entry {entry_name}: patch `{field}` token `{tok}` is not a 2-char hex byte");
        }
        out.push(u8::from_str_radix(tok, 16).unwrap());
    }
    out
}

/// Reject `bytes` that, written as a 16-bit little-endian halfword, would
/// produce `SVC #0x80` — the same opcode the hook dispatcher patches in.
/// Allowing it would let a patch silently install an unregistered hook PC and
/// crash with a misleading "fired at unregistered PC" fatal at runtime.
fn reject_svc_pattern(bytes: &[u8], entry_name: &str) {
    for w in bytes.windows(2) {
        if w == [0x80, 0xdf] {
            panic!(
                "entry {entry_name}: patch `bytes` may not contain the SVC #0x80 instruction (`80 df` LE) — that would shadow the hook dispatcher"
            );
        }
    }
}

/// Patches do not get to consume capture results, so capture tokens in a patch
/// pattern have no place to land. Reject them at parse time to avoid a dead
/// match pattern that silently behaves like `??`.
fn reject_patch_capture_tokens(tokens: &[PatternToken], entry_name: &str) {
    for t in tokens {
        match t {
            PatternToken::Capture(_) => {
                panic!("entry {entry_name}: patch pattern may not contain `{{...}}` capture tokens (use `??` for wildcards)");
            }
            PatternToken::BitMatch { capture: Some(_), .. } => {
                panic!("entry {entry_name}: patch pattern may not contain register-capture bits in BitMatch (use `?` for wildcards)");
            }
            _ => {}
        }
    }
}

fn pc_kind(raw: &RawHook, entry_name: &str) -> HookKind {
    match raw.kind {
        KindTag::Memcpy => HookKind::Memcpy,
        KindTag::Memset => HookKind::Memset,
        KindTag::Strcpy => HookKind::Strcpy,
        KindTag::Strlen => HookKind::Strlen,
        KindTag::Blit565 => HookKind::Blit565,
        KindTag::Blit565Lgt => HookKind::Blit565Lgt,
        KindTag::Blend565Lgt => HookKind::Blend565Lgt {
            // The function's shared tail. Required rather than defaulted: it is the one
            // thing the hook cannot derive, and guessing it would jump into the middle of
            // the loop it just replaced.
            exit_pc: raw.exit_pc.unwrap_or_else(|| panic!("entry {entry_name}: blend565_lgt requires exit_pc")),
        },
        // Required, not defaulted, for the same reason as `blend565_lgt`'s `exit_pc`: the
        // hook cannot derive these, and a wrong one would blend through whatever bytes
        // happen to live at that address.
        KindTag::BlitLut565Lgt => HookKind::BlitLut565Lgt(super::blitlut565::Tables {
            alpha_map: raw
                .alpha_map
                .unwrap_or_else(|| panic!("entry {entry_name}: blit_lut565_lgt requires alpha_map")),
            blend_t: raw
                .blend_t
                .unwrap_or_else(|| panic!("entry {entry_name}: blit_lut565_lgt requires blend_t")),
            blend_a: raw
                .blend_a
                .unwrap_or_else(|| panic!("entry {entry_name}: blit_lut565_lgt requires blend_a")),
        }),
        // Both required, both for reasons already stated above: `exit_pc` is the shared
        // tail the hook cannot derive, and `blend_a` is the byte table it indexes — this
        // hook takes the very same table `blit_lut565_lgt` does, and naming the key the
        // same is the point (one address, one place to fix per revision).
        KindTag::TintRect565Lgt => HookKind::TintRect565Lgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: tint_rect565_lgt requires exit_pc")),
            lut: raw
                .blend_a
                .unwrap_or_else(|| panic!("entry {entry_name}: tint_rect565_lgt requires blend_a")),
        },
        // Required, not defaulted: the hook resumes PAST the store it replaces, and a
        // wrong target would land in the middle of the code it just skipped.
        KindTag::OrbSumLgt => HookKind::OrbSumLgt {
            exit_pc: raw.exit_pc.unwrap_or_else(|| panic!("entry {entry_name}: orb_sum_lgt requires exit_pc")),
        },
        KindTag::OrbNameLgt => HookKind::OrbNameLgt {
            tables: super::orbsum::NameTables {
                string_table: raw
                    .string_table
                    .unwrap_or_else(|| panic!("entry {entry_name}: orb_name_lgt requires string_table")),
                scratch: raw.scratch.unwrap_or_else(|| panic!("entry {entry_name}: orb_name_lgt requires scratch")),
            },
            exit_pc: raw.exit_pc.unwrap_or_else(|| panic!("entry {entry_name}: orb_name_lgt requires exit_pc")),
        },
        // Both required: the hook replaces a compare-and-branch, so it needs BOTH of the
        // branch's destinations, and a guessed one would resume inside the loot spawn with
        // the arrays half-built.
        KindTag::ReaverDropLgt => HookKind::ReaverDropLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: reaver_drop_lgt requires exit_pc")),
            skip_pc: raw
                .skip_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: reaver_drop_lgt requires skip_pc")),
        },
        KindTag::TowerSpeciesLgt => HookKind::TowerSpeciesLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: tower_species_lgt requires exit_pc")),
        },
        // Both required, for the reaver hook's reason: it replaces a compare-and-branch,
        // and a guessed destination would resume inside the spawn loop's append.
        KindTag::TowerSpawnCapLgt => HookKind::TowerSpawnCapLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: tower_spawn_cap_lgt requires exit_pc")),
            skip_pc: raw
                .skip_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: tower_spawn_cap_lgt requires skip_pc")),
        },
        KindTag::BagboxGiveLgt => HookKind::BagboxGiveLgt,
        KindTag::BagboxAddLgt => HookKind::BagboxAddLgt,
        KindTag::BagboxTakeLgt => HookKind::BagboxTakeLgt,
        KindTag::BagboxCountLgt => HookKind::BagboxCountLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: bagbox_count_lgt requires exit_pc")),
        },
        KindTag::BagboxCountOfLgt => HookKind::BagboxCountOfLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: bagbox_count_of_lgt requires exit_pc")),
        },
        KindTag::BagboxSpendLgt => HookKind::BagboxSpendLgt,
        KindTag::BagboxRestageLgt => HookKind::BagboxRestageLgt,
        KindTag::BagboxRoomCheckLgt => HookKind::BagboxRoomCheckLgt,
        KindTag::BagboxCashGateLgt => HookKind::BagboxCashGateLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: bagbox_cash_gate_lgt requires exit_pc")),
        },
        KindTag::EndingTransitionLgt => HookKind::EndingTransitionLgt,
        KindTag::EndingLabelLgt => HookKind::EndingLabelLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: ending_label_lgt requires exit_pc")),
        },
        KindTag::DmgApplyLgt => HookKind::DmgApplyLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: dmg_apply_lgt requires exit_pc")),
        },
        KindTag::DmgDispLgt => HookKind::DmgDispLgt,
        KindTag::SpCostLevelSpendLgt => HookKind::SpCostLevelSpendLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: sp_cost_level_spend_lgt requires exit_pc")),
        },
        KindTag::SpCostLevelDispLgt => HookKind::SpCostLevelDispLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: sp_cost_level_disp_lgt requires exit_pc")),
        },
        KindTag::SpCostDispFloorLgt => HookKind::SpCostDispFloorLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: sp_cost_disp_floor_lgt requires exit_pc")),
        },
        KindTag::RespawnRateLgt => HookKind::RespawnRateLgt { exit_pc: raw.exit_pc.expect("respawn_rate_lgt requires exit_pc") },
        KindTag::ExpRateLgt => HookKind::ExpRateLgt { exit_pc: raw.exit_pc.expect("exp_rate_lgt requires exit_pc") },
        KindTag::HitSumClampLgt => HookKind::HitSumClampLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: hit_sum_clamp_lgt requires exit_pc")),
        },
        KindTag::CooldownCapLgt => HookKind::CooldownCapLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: cooldown_cap_lgt requires exit_pc")),
        },
        KindTag::DefPctCapLgt => HookKind::DefPctCapLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: def_pct_cap_lgt requires exit_pc")),
        },
        KindTag::StatWideLgt => HookKind::StatWideLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: stat_wide_lgt requires exit_pc")),
        },
        KindTag::DefReadWideLgt => HookKind::DefReadWideLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: def_read_wide_lgt requires exit_pc")),
        },
        KindTag::AtkReadWideLgt => HookKind::AtkReadWideLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: atk_read_wide_lgt requires exit_pc")),
        },
        KindTag::LootSpawnCapLgt => HookKind::LootSpawnCapLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: loot_spawn_cap_lgt requires exit_pc")),
        },
        KindTag::AtkRecoverLgt => HookKind::AtkRecoverLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: atk_recover_lgt requires exit_pc")),
        },
        KindTag::AtkSpeedLgt => HookKind::AtkSpeedLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: atk_speed_lgt requires exit_pc")),
        },
        KindTag::AtkEventLgt => HookKind::AtkEventLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: atk_event_lgt requires exit_pc")),
        },
        KindTag::SlotLabelBoxLgt => HookKind::SlotLabelBoxLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: slot_label_box_lgt requires exit_pc")),
        },
        KindTag::HpMaxLgt => HookKind::HpMaxLgt {
            exit_pc: raw.exit_pc.unwrap_or_else(|| panic!("entry {entry_name}: hp_max_lgt requires exit_pc")),
        },
        KindTag::HpAddLgt => HookKind::HpAddLgt,
        KindTag::AbsorbLgt => HookKind::AbsorbLgt,
        KindTag::HpDispCurLgt => HookKind::HpDispCurLgt {
            exit_pc: raw.exit_pc.unwrap_or_else(|| panic!("entry {entry_name}: hp_disp_cur_lgt requires exit_pc")),
        },
        KindTag::HpDispMaxLgt => HookKind::HpDispMaxLgt {
            exit_pc: raw.exit_pc.unwrap_or_else(|| panic!("entry {entry_name}: hp_disp_max_lgt requires exit_pc")),
        },
        // 150차: every accessory hook performs the one instruction it displaces and resumes
        // right after it, so each needs its `exit_pc`.
        KindTag::AccThirdStatLgt => HookKind::AccThirdStatLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: acc_third_stat_lgt requires exit_pc")),
        },
        KindTag::AccSummaryLgt => HookKind::AccSummaryLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: acc_summary_lgt requires exit_pc")),
        },
        KindTag::AccSummaryOnlineLgt => HookKind::AccSummaryOnlineLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: acc_summary_online_lgt requires exit_pc")),
        },
        KindTag::AccSummaryWarehouseLgt => HookKind::AccSummaryWarehouseLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: acc_summary_warehouse_lgt requires exit_pc")),
        },
        // 167차: both joins perform the instruction they displace, so both need the resume point.
        KindTag::AvatarLayerSetLgt => HookKind::AvatarLayerSetLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: avatar_layer_set_lgt requires exit_pc")),
        },
        KindTag::AvatarLayerLoadLgt => HookKind::AvatarLayerLoadLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: avatar_layer_load_lgt requires exit_pc")),
        },
        // 170차: the painter's 투구 read re-does the `ldrsh` it displaces.
        KindTag::AvatarHelmetPaintLgt => HookKind::AvatarHelmetPaintLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: avatar_helmet_paint_lgt requires exit_pc")),
        },
        // 171차: the painter's per-piece `cmp r4,#6; bne` — re-done, and the head can be skipped.
        KindTag::AvatarHeadPaintLgt => HookKind::AvatarHeadPaintLgt {
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: avatar_head_paint_lgt requires exit_pc")),
        },
        KindTag::InlineCopy => HookKind::InlineCopy(InlineCopy {
            dst_offset: raw
                .dst_offset
                .unwrap_or_else(|| panic!("entry {entry_name}: pc-based inline_copy requires dst_offset")),
            src_offset: raw
                .src_offset
                .unwrap_or_else(|| panic!("entry {entry_name}: pc-based inline_copy requires src_offset")),
            len_offset: raw
                .len_offset
                .unwrap_or_else(|| panic!("entry {entry_name}: pc-based inline_copy requires len_offset")),
            exit_pc: raw
                .exit_pc
                .unwrap_or_else(|| panic!("entry {entry_name}: pc-based inline_copy requires exit_pc")),
            spill_back: raw
                .spill_back
                .unwrap_or_else(|| panic!("entry {entry_name}: inline_copy requires spill_back")),
        }),
        KindTag::RegInlineCopy => panic!("entry {entry_name}: reg_inline_copy must be pattern-based, not pc-based"),
    }
}

fn pattern_template(raw: &RawHook, tokens: &[PatternToken], entry_name: &str) -> PatternHookKind {
    match raw.kind {
        KindTag::Memcpy => PatternHookKind::Memcpy,
        KindTag::Memset => PatternHookKind::Memset,
        KindTag::Strcpy => PatternHookKind::Strcpy,
        KindTag::Strlen => PatternHookKind::Strlen,
        // Address-specific by construction: the contract (13 arguments at a known SP
        // layout, a displaced 2-byte prologue) is tied to this one function, so there is
        // nothing a byte pattern could safely match.
        KindTag::Blit565 | KindTag::Blit565Lgt | KindTag::BlitLut565Lgt => {
            panic!("entry {entry_name}: blit565 must be pc-based, not pattern-based")
        }
        // Same reason: the contract is a specific SP frame layout inside one function.
        KindTag::Blend565Lgt => panic!("entry {entry_name}: blend565_lgt must be pc-based, not pattern-based"),
        KindTag::TintRect565Lgt => panic!("entry {entry_name}: tint_rect565_lgt must be pc-based, not pattern-based"),
        // Sits at one instruction inside `addItemToInventory` and resumes at a second
        // fixed address; a byte pattern could match neither safely.
        KindTag::OrbSumLgt => panic!("entry {entry_name}: orb_sum_lgt must be pc-based, not pattern-based"),
        KindTag::OrbNameLgt => panic!("entry {entry_name}: orb_name_lgt must be pc-based, not pattern-based"),
        KindTag::ReaverDropLgt => panic!("entry {entry_name}: reaver_drop_lgt must be pc-based, not pattern-based"),
        KindTag::TowerSpeciesLgt => panic!("entry {entry_name}: tower_species_lgt must be pc-based, not pattern-based"),
        KindTag::TowerSpawnCapLgt => panic!("entry {entry_name}: tower_spawn_cap_lgt must be pc-based, not pattern-based"),
        KindTag::BagboxGiveLgt
        | KindTag::BagboxAddLgt
        | KindTag::BagboxTakeLgt
        | KindTag::BagboxCountLgt
        | KindTag::BagboxCountOfLgt
        | KindTag::BagboxSpendLgt
        | KindTag::BagboxRestageLgt
        | KindTag::BagboxRoomCheckLgt
        | KindTag::BagboxCashGateLgt => {
            panic!("entry {entry_name}: bagbox hooks must be pc-based, not pattern-based")
        }
        KindTag::EndingTransitionLgt => {
            panic!("entry {entry_name}: ending_transition_lgt must be pc-based, not pattern-based")
        }
        KindTag::EndingLabelLgt => {
            panic!("entry {entry_name}: ending_label_lgt must be pc-based, not pattern-based")
        }
        KindTag::DmgApplyLgt | KindTag::DmgDispLgt => {
            panic!("entry {entry_name}: dmg hooks must be pc-based, not pattern-based")
        }
        KindTag::SpCostLevelSpendLgt | KindTag::SpCostLevelDispLgt | KindTag::SpCostDispFloorLgt => {
            panic!("entry {entry_name}: sp_cost hooks must be pc-based, not pattern-based")
        }
        KindTag::RespawnRateLgt | KindTag::ExpRateLgt | KindTag::HitSumClampLgt | KindTag::CooldownCapLgt => {
            panic!("entry {entry_name}: stat cap hooks must be pc-based, not pattern-based")
        }
        KindTag::DefPctCapLgt | KindTag::StatWideLgt | KindTag::DefReadWideLgt | KindTag::AtkReadWideLgt => {
            panic!("entry {entry_name}: stat wide hooks must be pc-based, not pattern-based")
        }
        KindTag::LootSpawnCapLgt => {
            panic!("entry {entry_name}: loot_spawn_cap_lgt must be pc-based, not pattern-based")
        }
        KindTag::AtkRecoverLgt => {
            panic!("entry {entry_name}: atk_recover_lgt must be pc-based, not pattern-based")
        }
        KindTag::AtkSpeedLgt => {
            panic!("entry {entry_name}: atk_speed_lgt must be pc-based, not pattern-based")
        }
        KindTag::AtkEventLgt => {
            panic!("entry {entry_name}: atk_event_lgt must be pc-based, not pattern-based")
        }
        KindTag::SlotLabelBoxLgt => {
            panic!("entry {entry_name}: slot_label_box_lgt must be pc-based, not pattern-based")
        }
        KindTag::HpMaxLgt | KindTag::HpAddLgt | KindTag::HpDispCurLgt | KindTag::HpDispMaxLgt => {
            panic!("entry {entry_name}: the player-hp hooks must be pc-based, not pattern-based")
        }
        KindTag::AbsorbLgt => {
            panic!("entry {entry_name}: absorb_lgt must be pc-based, not pattern-based")
        }
        KindTag::AccThirdStatLgt | KindTag::AccSummaryLgt | KindTag::AccSummaryOnlineLgt | KindTag::AccSummaryWarehouseLgt => {
            panic!("entry {entry_name}: the accessory hooks must be pc-based, not pattern-based")
        }
        KindTag::AvatarLayerSetLgt | KindTag::AvatarLayerLoadLgt | KindTag::AvatarHelmetPaintLgt | KindTag::AvatarHeadPaintLgt => {
            panic!("entry {entry_name}: the avatar hooks must be pc-based, not pattern-based")
        }
        KindTag::RegInlineCopy => {
            let need = |cap: CaptureName, label: &str| {
                if !tokens.iter().any(|t| match t {
                    PatternToken::BitMatch { capture: Some((c, _)), .. } => *c == cap,
                    _ => false,
                }) {
                    panic!("entry {entry_name}: reg_inline_copy pattern must capture {label} register");
                }
            };
            need(CaptureName::SrcReg, "src");
            need(CaptureName::DstReg, "dst");
            need(CaptureName::CountReg, "count");
            PatternHookKind::RegInlineCopy {
                count_offset: raw
                    .count_offset
                    .unwrap_or_else(|| panic!("entry {entry_name}: reg_inline_copy requires count_offset")),
            }
        }
        KindTag::InlineCopy => {
            let exit_cap = tokens.iter().any(|t| matches!(t, PatternToken::Capture(CaptureName::ExitB)));
            if !exit_cap && raw.exit_pc.is_none() {
                panic!("entry {entry_name}: inline_copy pattern needs either {{exit_b}} capture or exit_pc");
            }
            if exit_cap && raw.exit_pc.is_some() {
                panic!("entry {entry_name}: inline_copy pattern cannot specify both {{exit_b}} and exit_pc");
            }
            PatternHookKind::InlineCopy {
                dst_offset: resolve_offset("dst_offset", tokens, CaptureName::Dst, raw.dst_offset, entry_name),
                src_offset: resolve_offset("src_offset", tokens, CaptureName::Src, raw.src_offset, entry_name),
                len_offset: resolve_offset("len_offset", tokens, CaptureName::Len, raw.len_offset, entry_name),
                exit_pc: raw.exit_pc,
                spill_back: raw
                    .spill_back
                    .unwrap_or_else(|| panic!("entry {entry_name}: inline_copy requires spill_back")),
            }
        }
    }
}

fn resolve_offset(field: &str, tokens: &[PatternToken], cap: CaptureName, fixed: Option<i32>, entry_name: &str) -> Option<i32> {
    let has_cap = tokens.iter().any(|t| matches!(t, PatternToken::Capture(c) if *c == cap));
    match (has_cap, fixed) {
        (true, None) => None,
        (false, Some(_)) => fixed,
        (true, Some(_)) => panic!("entry {entry_name}: {field} cannot be set when a corresponding capture is in the pattern"),
        (false, None) => panic!("entry {entry_name}: {field} required when no matching capture is in the pattern"),
    }
}

fn parse_hash(s: &str, entry_name: &str) -> [u8; 16] {
    if s.len() != 32 {
        panic!("entry {entry_name}: hash must be 32 hex chars (got {} chars: `{s}`)", s.len());
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .unwrap_or_else(|_| panic!("entry {entry_name}: hash contains non-hex byte at offset {i}: `{s}`"));
    }
    out
}

fn parse_pattern(pattern: &str, entry_name: &str) -> Vec<PatternToken> {
    let mut tokens = Vec::new();
    for raw in pattern.split_whitespace() {
        let token = if raw == "??" {
            PatternToken::AnyByte
        } else if raw.len() == 10
            && let Some(bits) = raw.strip_prefix("0b")
        {
            parse_bit_match(bits, entry_name)
        } else if let Some(rest) = raw.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let cap = match rest {
                "dst" => CaptureName::Dst,
                "src" => CaptureName::Src,
                "len" => CaptureName::Len,
                "exit_b" => CaptureName::ExitB,
                _ => panic!("entry {entry_name}: unknown capture name {{{rest}}} (allowed: dst, src, len, exit_b)"),
            };
            PatternToken::Capture(cap)
        } else if raw.len() == 2 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            PatternToken::Literal(u8::from_str_radix(raw, 16).unwrap())
        } else {
            panic!("entry {entry_name}: invalid pattern token `{raw}`");
        };
        tokens.push(token);
    }
    validate_exit_b(&tokens, entry_name);
    tokens
}

fn validate_exit_b(tokens: &[PatternToken], entry_name: &str) {
    let mut pair_seen = false;
    let mut i = 0;
    while i < tokens.len() {
        if matches!(tokens[i], PatternToken::Capture(CaptureName::ExitB)) {
            let next_is_exit = matches!(tokens.get(i + 1), Some(PatternToken::Capture(CaptureName::ExitB)));
            if !next_is_exit {
                panic!("entry {entry_name}: {{exit_b}} must appear as two consecutive tokens");
            }
            if matches!(tokens.get(i + 2), Some(PatternToken::Capture(CaptureName::ExitB))) {
                panic!("entry {entry_name}: {{exit_b}} appears more than twice consecutively");
            }
            if pair_seen {
                panic!("entry {entry_name}: pattern may contain at most one {{exit_b}} pair");
            }
            pair_seen = true;
            i += 2;
        } else {
            i += 1;
        }
    }
}

/// Parse an 8-character byte specification of `0`/`1` literals, `?` wildcards,
/// and `s`/`d`/`c` register placeholders (3 consecutive of the same letter).
/// e.g. `00sss011` → mask=0b11000111, fixed=0b00000011, capture (src @ shift 3).
fn parse_bit_match(bits: &str, entry_name: &str) -> PatternToken {
    if bits.len() != 8 {
        panic!("entry {entry_name}: bit pattern `0b{bits}` must be 8 characters");
    }
    let mut mask: u8 = 0;
    let mut fixed: u8 = 0;
    let mut capture: Option<(CaptureName, u8, u8)> = None; // (name, lowest bit, count seen)
    for (i, ch) in bits.chars().enumerate() {
        let bit = 7 - i as u8;
        match ch {
            '0' => {
                mask |= 1 << bit;
            }
            '1' => {
                mask |= 1 << bit;
                fixed |= 1 << bit;
            }
            '?' => {}
            's' | 'd' | 'c' => {
                let name = match ch {
                    's' => CaptureName::SrcReg,
                    'd' => CaptureName::DstReg,
                    _ => CaptureName::CountReg,
                };
                match &mut capture {
                    Some((n, lowest, count)) if *n == name => {
                        *lowest = bit; // iterating high→low, so the latest write is the lowest bit
                        *count += 1;
                    }
                    Some(_) => panic!("entry {entry_name}: bit pattern `0b{bits}` mixes multiple register placeholders"),
                    None => capture = Some((name, bit, 1)),
                }
            }
            _ => panic!("entry {entry_name}: invalid char {ch:?} in bit pattern `0b{bits}` (allowed: 0,1,?,s,d,c)"),
        }
    }
    let capture = capture.map(|(name, lowest, count)| {
        if count != 3 {
            panic!("entry {entry_name}: bit pattern `0b{bits}` register placeholder must span exactly 3 bits");
        }
        (name, lowest)
    });
    PatternToken::BitMatch { mask, fixed, capture }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn parse_doc(toml_text: &str) -> Vec<Entry> {
        let doc: RawDoc = toml::from_str(toml_text).expect("parse test toml");
        doc.entry.into_iter().map(RawEntry::into_entry).collect()
    }

    // ── 114차: `flavor` ─────────────────────────────────────────────────────
    #[test]
    fn pc_patch_flavor_parses_and_defaults_to_every_flavour() {
        let entries = parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "flavoured"

            [[entry.patch]]
            pc = 0x4d6a8
            bytes = "63 2b"
            expect = "46 2b"
            flavor = "main"

            [[entry.patch]]
            pc = 0x4d6ac
            bytes = "63 23"
            expect = "46 23"
            flavor = "vanilla"

            [[entry.patch]]
            pc = 0x4d628
            bytes = "c4 23"
            "#,
        );
        let p = &entries[0].patches;
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].flavor, Some(Flavor::Main));
        assert_eq!(p[1].flavor, Some(Flavor::Vanilla));
        assert_eq!(p[2].flavor, None, "no `flavor` key = every flavour, exactly as before 114차");
    }

    #[test]
    #[should_panic(expected = "must be \"main\" or \"vanilla\"")]
    fn pc_patch_with_an_unknown_flavor_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "typo"

            [[entry.patch]]
            pc = 0x4d6a8
            bytes = "63 2b"
            flavor = "Main"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "only valid with `pc`")]
    fn pattern_patch_with_a_flavor_panics() {
        parse_doc(
            r#"
            [[entry]]
            name = "generic"

            [[entry.patch]]
            pattern = "aa bb"
            bytes = "11 22"
            flavor = "main"
            "#,
        );
    }

    #[test]
    fn pc_patch_with_expect_and_bytes() {
        let entries = parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "pc-patch"

            [[entry.patch]]
            pc = 0x12345
            bytes = "00 20 70 47"
            expect = "78 47 01 22"
            "#,
        );
        let e = &entries[0];
        assert_eq!(e.patches.len(), 1);
        let p = &e.patches[0];
        assert_eq!(p.pc, 0x12345);
        assert_eq!(p.bytes, vec![0x00, 0x20, 0x70, 0x47]);
        assert_eq!(p.expect.as_deref(), Some([0x78, 0x47, 0x01, 0x22].as_slice()));
        assert!(e.patch_patterns.is_empty());
    }

    #[test]
    fn pattern_patch_with_offset_and_expect() {
        let entries = parse_doc(
            r#"
            [[entry]]
            name = "pattern-patch"

            [[entry.patch]]
            pattern = "aa bb cc dd"
            bytes = "11 22"
            expect = "bb cc"
            offset = 1
            "#,
        );
        let e = &entries[0];
        assert!(e.patches.is_empty());
        assert_eq!(e.patch_patterns.len(), 1);
        let pp = &e.patch_patterns[0];
        assert_eq!(pp.tokens.len(), 4);
        assert_eq!(pp.bytes, vec![0x11, 0x22]);
        assert_eq!(pp.expect.as_deref(), Some([0xbb, 0xcc].as_slice()));
        assert_eq!(pp.offset, 1);
    }

    #[test]
    fn pattern_patch_default_offset_is_zero() {
        let entries = parse_doc(
            r#"
            [[entry]]
            name = "no-offset"

            [[entry.patch]]
            pattern = "aa bb"
            bytes = "11 22"
            "#,
        );
        assert_eq!(entries[0].patch_patterns[0].offset, 0);
    }

    #[test]
    fn legacy_hook_only_entry_parses_with_empty_patches() {
        let entries = parse_doc(
            r#"
            [[entry]]
            name = "legacy"

            [[entry.hook]]
            kind = "memcpy"
            pattern = "78 47 00 00 01 40 2d e9 03 00 52 e3"
            "#,
        );
        let e = &entries[0];
        assert_eq!(e.hook_patterns.len(), 1);
        assert!(e.hooks.is_empty());
        assert!(e.patches.is_empty());
        assert!(e.patch_patterns.is_empty());
    }

    #[test]
    #[should_panic(expected = "cannot specify both")]
    fn pc_and_pattern_both_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "x"
            [[entry.patch]]
            pc = 0x100
            pattern = "aa bb"
            bytes = "11 22"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "must specify either")]
    fn pc_and_pattern_neither_panics() {
        parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            bytes = "11 22"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "at least 1 byte")]
    fn empty_bytes_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "x"
            [[entry.patch]]
            pc = 0x100
            bytes = ""
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "must equal `bytes` length")]
    fn bytes_expect_length_mismatch_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "x"
            [[entry.patch]]
            pc = 0x100
            bytes = "11 22"
            expect = "33"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "requires entry `hash`")]
    fn pc_patch_without_hash_panics() {
        parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            pc = 0x100
            bytes = "11 22"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "overflows u32")]
    fn offset_plus_bytes_u32_overflow_panics() {
        // offset 0xffff_ffff + bytes_len 2 wraps before the > pat_len check.
        parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            pattern = "aa bb"
            bytes = "11 22"
            offset = 4294967295
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "exceeds pattern length")]
    fn offset_plus_bytes_overflow_panics() {
        parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            pattern = "aa bb"
            bytes = "11 22"
            offset = 1
            "#,
        );
    }

    #[test]
    fn offset_plus_bytes_exactly_at_pattern_end_is_ok() {
        let entries = parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            pattern = "aa bb cc dd"
            bytes = "11 22"
            offset = 2
            "#,
        );
        assert_eq!(entries[0].patch_patterns[0].offset, 2);
    }

    #[test]
    #[should_panic(expected = "meaningless with `pc`")]
    fn pc_with_offset_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "x"
            [[entry.patch]]
            pc = 0x100
            bytes = "11 22"
            offset = 1
            "#,
        );
    }

    // ── 127차: the restage hook is a pc-based entry hook like the intake pair ──
    #[test]
    fn the_restage_hook_parses_as_a_pc_entry_hook() {
        let entries = parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "restage"

            [[entry.hook]]
            kind = "bagbox_restage_lgt"
            pc = 0x5340d
            "#,
        );
        let e = &entries[0];
        assert_eq!(e.hooks.len(), 1);
        assert_eq!(e.hooks[0].pc, 0x5340d);
        assert!(matches!(e.hooks[0].kind, HookKind::BagboxRestageLgt));
        assert!(e.hook_patterns.is_empty());
    }

    // ── 163차: a pc hook takes `flavor` like a patch; a pattern hook may not ──
    #[test]
    fn a_pc_hook_carries_its_flavour() {
        let entries = parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "flavoured hooks"

            [[entry.hook]]
            kind = "cooldown_cap_lgt"
            pc = 0x4bbe1
            exit_pc = 0x4bbe3
            flavor = "main"

            [[entry.hook]]
            kind = "absorb_lgt"
            pc = 0x4b979
            "#,
        );
        let h = &entries[0].hooks;
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].flavor, Some(Flavor::Main));
        assert!(matches!(h[1].kind, HookKind::AbsorbLgt));
        assert_eq!(h[1].flavor, None, "untagged = every flavour");
    }

    #[test]
    #[should_panic(expected = "hook `flavor` must be")]
    fn a_misspelt_hook_flavour_is_a_table_bug() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "bad"

            [[entry.hook]]
            kind = "absorb_lgt"
            pc = 0x4b979
            flavor = "Main"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "hook `flavor` is only valid with `pc`")]
    fn a_pattern_hook_cannot_be_flavoured() {
        parse_doc(
            r#"
            [[entry]]
            name = "generic"

            [[entry.hook]]
            kind = "strlen"
            pattern = "01 30 03 78 00 2b fb d1"
            flavor = "main"
            "#,
        );
    }

    // ── 150차: the accessory hooks are pc-based hooks that each need an exit_pc ──
    #[test]
    fn the_accessory_hooks_parse_as_pc_hooks() {
        let entries = parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "accessories"

            [[entry.hook]]
            kind = "acc_third_stat_lgt"
            pc = 0x4f131
            exit_pc = 0x4f133

            [[entry.hook]]
            kind = "acc_summary_lgt"
            pc = 0x41e87
            exit_pc = 0x41e89

            [[entry.hook]]
            kind = "acc_summary_online_lgt"
            pc = 0x124e7
            exit_pc = 0x124e9

            [[entry.hook]]
            kind = "acc_summary_warehouse_lgt"
            pc = 0x113e7
            exit_pc = 0x113e9
            "#,
        );
        let h = &entries[0].hooks;
        assert_eq!(h.len(), 4);
        assert!(matches!(h[0].kind, HookKind::AccThirdStatLgt { exit_pc: 0x4f133 }));
        assert!(matches!(h[1].kind, HookKind::AccSummaryLgt { exit_pc: 0x41e89 }));
        assert!(matches!(h[2].kind, HookKind::AccSummaryOnlineLgt { exit_pc: 0x124e9 }));
        assert!(matches!(h[3].kind, HookKind::AccSummaryWarehouseLgt { exit_pc: 0x113e9 }));
    }

    #[test]
    #[should_panic(expected = "acc_third_stat_lgt requires exit_pc")]
    fn the_third_option_hook_needs_its_exit() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "no-exit"

            [[entry.hook]]
            kind = "acc_third_stat_lgt"
            pc = 0x4f131
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "bagbox hooks must be pc-based")]
    fn the_restage_hook_refuses_a_pattern() {
        parse_doc(
            r#"
            [[entry]]
            name = "restage-pattern"

            [[entry.hook]]
            kind = "bagbox_restage_lgt"
            pattern = "f0 b5 00 06 09 04"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "may not contain the SVC #0x80")]
    fn bytes_containing_svc_80_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "x"
            [[entry.patch]]
            pc = 0x100
            bytes = "80 df"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "may not contain `{...}` capture")]
    fn patch_pattern_with_capture_token_panics() {
        parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            pattern = "aa {dst} cc dd"
            bytes = "11 22"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "may not contain register-capture bits")]
    fn patch_pattern_with_bitmatch_capture_panics() {
        parse_doc(
            r#"
            [[entry]]
            name = "x"
            [[entry.patch]]
            pattern = "0b00sss011 22 33 44"
            bytes = "11 22"
            "#,
        );
    }

    #[test]
    #[should_panic(expected = "unknown field")]
    fn patch_with_unknown_field_panics() {
        parse_doc(
            r#"
            [[entry]]
            hash = "00000000000000000000000000000000"
            name = "x"
            [[entry.patch]]
            pc = 0x100
            bytes = "11 22"
            kind = "memcpy"
            "#,
        );
    }
}
