use alloc::{format, vec, vec::Vec};

use wie_util::{ByteRead, Result, WieError};

use crate::ArmCore;

mod absorb;
mod accsummary;
mod atkspeed;
mod avatar;
mod bagbox;
mod blend565;
mod blit565;
mod blitlut565;
mod dmgwide;
mod ending;
mod endinglabel;
mod helldrop;
mod helldrop_table;
mod hook;
mod lootlife;
mod orbsum;
mod parser;
mod patch;
mod playerhp;
mod reaverdrop;
mod slotlabel;
mod spcost;
mod statcap;
mod gameplay;
mod statwide;
mod thirdopt;
mod tint565;
mod towerspawn;
mod towerspecies;

use self::{
    hook::{Hook, PatternHook},
    patch::{PatchSpec, PatternPatchSpec},
};

/// Match the binary against the embedded patch table and install any matching
/// patches/hooks. The on-disk MD5 selects a hash-keyed entry first, falling
/// back to a hash-less generic entry. `scan_ranges` are the `(base, size)` byte
/// ranges searched for pattern matching (typically the guest `.text` region).
/// Returns the number of patches + hooks installed.
pub fn install_binary_patches(core: &mut ArmCore, data: &[u8], scan_ranges: &[(u32, u32)]) -> Result<usize> {
    let hash = md5::compute(data).0;
    let entries = parser::binary_patches();
    // ★★ KNOWN LIMITATION, MEASURED 2026-08-11 — do not "fix" this by calling
    //    `install_entry` twice; that is exactly what was tried and it aborts the boot.
    //
    //    A hash match returns here, so a title with its own entry gets NONE of the
    //    hash-less entry's generic hooks. Hero3 has a hash-keyed entry on both carriers,
    //    which means the six compiler-library hooks in `data/binary_patches.toml`
    //    (memcpy / memset / strcpy / strlen / the two inline-copy loops) have never once
    //    been installed for the game this repo exists for. The device log has been saying
    //    so all along: "Installed 1 hooks".
    //
    //    The two kinds of entry answer different questions and were never in competition:
    //    a hash-keyed entry names addresses inside ONE build of ONE game, while a
    //    hash-less entry matches compiler-library BYTE PATTERNS that belong to the
    //    toolchain, not the title.
    //
    //    Why the obvious change fails: `install_entry` ends by calling
    //    `register_svc_handler(BINARY_PATCH_SVC, …)` with the registry IT built, and a
    //    second call panics with `SVC handler already registered for 128` before the
    //    first frame — `apply_hooks` builds the dispatcher registry and registers it. So
    //    both entries are COLLECTED and applied once, which is what `install_entries`
    //    does. See MIGRATION_LGT.md §2.17.
    let mut selected: Vec<(&Entry, bool)> = Vec::new();
    if let Some(entry) = entries.iter().find(|e| matches!(e.hash, Some(h) if h == hash)) {
        selected.push((entry, true));
    }
    if let Some(entry) = entries.iter().find(|e| e.hash.is_none()) {
        selected.push((entry, false));
    }
    install_entries(core, &selected, scan_ranges)
}

struct Entry {
    hash: Option<[u8; 16]>,
    name: alloc::string::String,
    hooks: Vec<Hook>,
    hook_patterns: Vec<PatternHook>,
    patches: Vec<PatchSpec>,
    patch_patterns: Vec<PatternPatchSpec>,
}

enum PatternToken {
    Literal(u8),
    AnyByte,
    Capture(CaptureName),
    /// Bit-level match for a byte that mixes a fixed opcode with a 3-bit
    /// register field (Thumb1 low-register encoding). `(byte & mask) == fixed`
    /// is the literal check; if `capture` is set, `(byte >> shift) & 0b111`
    /// is read into the named register slot, with cross-byte consistency
    /// enforced at match time.
    BitMatch {
        mask: u8,
        fixed: u8,
        capture: Option<(CaptureName, u8)>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CaptureName {
    Dst,
    Src,
    Len,
    ExitB,
    SrcReg,
    DstReg,
    CountReg,
}

#[derive(Debug, Clone)]
struct PatternMatch {
    addr: u32,
    dst: Option<u8>,
    src: Option<u8>,
    len: Option<u8>,
    src_reg: Option<u8>,
    dst_reg: Option<u8>,
    count_reg: Option<u8>,
    /// Address of the first `{exit_b}` byte — combined with `exit_b_bytes` to
    /// compute the branch target.
    exit_b_site: Option<u32>,
    exit_b_bytes: Option<[u8; 2]>,
}

/// Install several entries as ONE unit: every hook is resolved first, the patches are
/// written against the combined hook list, and the SVC dispatcher is registered exactly
/// once at the end.
///
/// ── WHY THIS IS NOT A LOOP OVER `install_entry` (2026-08-11) ──────────────────
/// Three things only work when the entries are handled together:
///  1. **One dispatcher.** `hook::apply_hooks` ends with
///     `register_svc_handler(BINARY_PATCH_SVC, …)`, which is fatal the second time.
///  2. **Whole-picture overlap.** `patch::install_patches` refuses a raw-byte patch that
///     would land on a hook's SVC. Given one entry's hooks it can only see that entry's
///     share, so a generic patch could silently overwrite a game-specific hook.
///  3. **Collision priority.** A generic library pattern can match the same PC as a
///     hash-keyed hook. `selected` is ordered hash-keyed-first and the FIRST hook at a
///     PC wins, so the game-specific one does.
fn install_entries(core: &mut ArmCore, selected: &[(&Entry, bool)], scan_ranges: &[(u32, u32)]) -> Result<usize> {
    let mut per_entry: Vec<Vec<Hook>> = Vec::with_capacity(selected.len());
    for (entry, _) in selected {
        per_entry.push(hook::resolve_hooks(core, entry, scan_ranges)?);
    }
    let mut all: Vec<Hook> = Vec::new();
    for hooks in &per_entry {
        for h in hooks {
            if all.iter().any(|e| e.pc == h.pc) {
                tracing::warn!("hook at {:#x} already claimed by an earlier entry; keeping the first", h.pc);
                continue;
            }
            all.push(*h);
        }
    }

    let mut total = 0;
    for ((entry, is_specific), hooks) in selected.iter().zip(per_entry.iter()) {
        let n_patches = patch::install_patches(core, entry, scan_ranges, &all)?;

        // A hash-keyed entry that produces zero installations is a strong signal
        // that the binary drifted from what the patch table targets. Generic
        // (hash-less) entries have no such guarantee. We trust the caller's
        // `is_specific` rather than `entry.hash.is_some()` so that a placeholder
        // all-zero hash in a fixture doesn't accidentally trigger this.
        if n_patches == 0 && hooks.is_empty() {
            if *is_specific {
                return Err(WieError::FatalError(format!(
                    "entry {}: hash matched but produced 0 patches/hooks (table likely out of date)",
                    entry.name
                )));
            }
            tracing::warn!("entry {}: generic entry produced 0 patches/hooks", entry.name);
        }
        total += n_patches + hooks.len();
    }

    let names = selected.iter().map(|(e, _)| e.name.as_str()).collect::<Vec<_>>().join(" + ");
    hook::apply_hooks(core, &names, &all)?;
    Ok(total)
}

fn install_entry(core: &mut ArmCore, entry: &Entry, scan_ranges: &[(u32, u32)], is_specific: bool) -> Result<usize> {
    install_entries(core, &[(entry, is_specific)], scan_ranges)
}

/// Scans on 2-byte boundaries since all Thumb instructions are halfword-aligned.
fn scan_pattern(core: &mut ArmCore, tokens: &[PatternToken], scan_ranges: &[(u32, u32)]) -> Result<Vec<(u32, PatternMatch)>> {
    let mut results = Vec::new();
    let pat_len = tokens.len();
    if pat_len == 0 {
        return Ok(results);
    }

    for (base, size) in scan_ranges {
        let mut buf = vec![0u8; *size as usize];
        core.read_bytes(*base, &mut buf)?;

        let mut off = 0usize;
        while off + pat_len <= buf.len() {
            if let Some(mut pm) = try_match(tokens, &buf[off..off + pat_len]) {
                pm.addr = base + off as u32;
                if pm.exit_b_bytes.is_some() {
                    for (ti, t) in tokens.iter().enumerate() {
                        if matches!(t, PatternToken::Capture(CaptureName::ExitB)) {
                            pm.exit_b_site = Some(base + (off + ti) as u32);
                            break;
                        }
                    }
                }
                results.push((pm.addr, pm));
            }
            off += 2;
        }
    }

    Ok(results)
}

/// Returns a `PatternMatch` with captures populated; `addr` and `exit_b_site`
/// are filled in by the caller from the actual scan position.
fn try_match(tokens: &[PatternToken], bytes: &[u8]) -> Option<PatternMatch> {
    if bytes.len() < tokens.len() {
        return None;
    }
    let mut m = PatternMatch {
        addr: 0,
        dst: None,
        src: None,
        len: None,
        src_reg: None,
        dst_reg: None,
        count_reg: None,
        exit_b_site: None,
        exit_b_bytes: None,
    };
    let mut i = 0;
    while i < tokens.len() {
        let b = bytes[i];
        match &tokens[i] {
            PatternToken::Literal(v) => {
                if b != *v {
                    return None;
                }
                i += 1;
            }
            PatternToken::AnyByte => {
                i += 1;
            }
            PatternToken::BitMatch { mask, fixed, capture } => {
                if b & mask != *fixed {
                    return None;
                }
                if let Some((name, shift)) = capture {
                    let value = (b >> shift) & 0b111;
                    let slot = match name {
                        CaptureName::SrcReg => &mut m.src_reg,
                        CaptureName::DstReg => &mut m.dst_reg,
                        CaptureName::CountReg => &mut m.count_reg,
                        _ => return None,
                    };
                    if let Some(prev) = *slot
                        && prev != value
                    {
                        return None;
                    }
                    *slot = Some(value);
                }
                i += 1;
            }
            PatternToken::Capture(name) => match name {
                CaptureName::Dst => {
                    m.dst = Some(b);
                    i += 1;
                }
                CaptureName::Src => {
                    m.src = Some(b);
                    i += 1;
                }
                CaptureName::Len => {
                    m.len = Some(b);
                    i += 1;
                }
                CaptureName::ExitB => {
                    if i + 1 >= bytes.len() || i + 1 >= tokens.len() {
                        return None;
                    }
                    let hi = bytes[i + 1];
                    if !matches!(tokens[i + 1], PatternToken::Capture(CaptureName::ExitB)) {
                        return None;
                    }
                    // Reject matches where the captured bytes aren't a Thumb
                    // unconditional `B imm11` (`11100 iiiiiiiiiii`, encoded as
                    // 0xE0xx-0xE7xx little-endian). Without this, a permissive
                    // pattern can land on arbitrary instructions and decode
                    // them as if they were branches, sending the dispatcher
                    // to a garbage `exit_pc`.
                    if hi & 0xf8 != 0xe0 {
                        return None;
                    }
                    m.exit_b_bytes = Some([b, hi]);
                    i += 2;
                }
                CaptureName::SrcReg | CaptureName::DstReg | CaptureName::CountReg => {
                    // Register captures only land in `BitMatch`, not whole-byte `Capture`.
                    return None;
                }
            },
        }
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn embedded_toml_parses() {
        let entries = parser::binary_patches();
        assert!(!entries.is_empty(), "embedded TOML produced no entries");
        for entry in &entries {
            assert!(!entry.name.is_empty(), "entry missing name");
        }
    }

    /// ★ 163차 — what vanilla does NOT get, pinned. The user's rule (2026-10-05): vanilla is
    /// the game plus the shared conveniences; every balance patch the new items brought is
    /// 개정판 only (「바닐라에서는 쿨감 제한 적용되면안돼. 신규템으로 인한 밸런스 관련 패치했던거
    /// 적용되면 안되는거야」). A site added to — or dropped from — this list is a decision, not
    /// a refactor: change it here and in `wie_android::flavor` together.
    #[test]
    fn the_main_only_sites_are_exactly_the_kaejeongpan_balance() {
        use hook::HookKind as K;
        use patch::Flavor;
        let entries = parser::binary_patches();
        let lgt = entries
            .iter()
            .find(|e| e.hash == Some([0x6a, 0x35, 0x29, 0x44, 0xbb, 0x43, 0xc0, 0x78, 0x9c, 0xa9, 0xf4, 0xb6, 0xf0, 0x70, 0x2f, 0x4b]))
            .expect("the LGT 01.00.07 entry");
        let mut main_hooks: Vec<u32> = lgt.hooks.iter().filter(|h| h.flavor == Some(Flavor::Main)).map(|h| h.pc).collect();
        main_hooks.sort_unstable();
        let mut want: Vec<u32> = [
            0x38557, // def_read_wide (160차 T1)
            0x41e87, 0x113e7, 0x124e7, // the three accessory summary lines (150차)
            0x4af13, // atk_speed (131차)
            0x4bbe1, // cooldown cap 70 %
            0x4bdad, 0x4be13, // atk_read_wide (160차 T1)
            0x4ce4f, 0x4ceff, 0x4cf73, 0x4cf93, 0x4cfb3, 0x4cfd3, // getters: wide (T1) · 방어% 500 (T2)
            0x4f131, // the accessory third option (150차)
            0x4f7cb, // 명중 255 (158차 P1)
            0x50e2d, 0x51069, 0x5108d, // atk_event · atk_recover (132차-C · 131차)
        ]
        .to_vec();
        want.sort_unstable();
        assert_eq!(main_hooks, want);
        for h in lgt.hooks.iter().filter(|h| h.flavor == Some(Flavor::Main)) {
            assert!(
                matches!(
                    h.kind,
                    K::HitSumClampLgt { .. }
                        | K::CooldownCapLgt { .. }
                        | K::DefPctCapLgt { .. }
                        | K::StatWideLgt { .. }
                        | K::DefReadWideLgt { .. }
                        | K::AtkReadWideLgt { .. }
                        | K::AtkSpeedLgt { .. }
                        | K::AtkEventLgt { .. }
                        | K::AtkRecoverLgt { .. }
                        | K::AccThirdStatLgt { .. }
                        | K::AccSummaryLgt { .. }
                        | K::AccSummaryOnlineLgt { .. }
                        | K::AccSummaryWarehouseLgt { .. }
                ),
                "{:#x} {:?}",
                h.pc,
                h.kind
            );
        }
        // Nothing vanilla-only, and the shared machinery stays shared: 흡수 (163차), the HP gauge
        // (142차 — inert on vanilla, and its heal fix keeps a 16-bit wrap from killing), the
        // damage path's folds and hooks, the three bags, the loot lifetime.
        assert!(lgt.hooks.iter().all(|h| h.flavor != Some(Flavor::Vanilla)));
        for kind_is in [
            (|k: &K| matches!(k, K::AbsorbLgt)) as fn(&K) -> bool,
            |k| matches!(k, K::HpAddLgt),
            |k| matches!(k, K::HpMaxLgt { .. }),
            |k| matches!(k, K::DmgApplyLgt { .. }),
            |k| matches!(k, K::DmgDispLgt),
            |k| matches!(k, K::LootSpawnCapLgt { .. }),
            |k| matches!(k, K::BagboxGiveLgt),
            |k| matches!(k, K::BagboxRoomCheckLgt),
            |k| matches!(k, K::BagboxCashGateLgt { .. }),
        ] {
            let h = lgt.hooks.iter().find(|h| kind_is(&h.kind)).expect("a shared hook");
            assert_eq!(h.flavor, None, "{:#x} {:?} ships on both flavours", h.pc, h.kind);
        }
        // Patches: the five 111차 level-cap bytes and the two damage-cap literals (163차).
        let mut main_patches: Vec<u32> = lgt.patches.iter().filter(|p| p.flavor == Some(Flavor::Main)).map(|p| p.pc).collect();
        main_patches.sort_unstable();
        assert_eq!(main_patches, [0x385cc, 0x4beac, 0x4d628, 0x4d664, 0x4d668, 0x4d6a8, 0x4d6ac]);
        let cap = |pc: u32| lgt.patches.iter().find(|p| p.pc == pc).expect("the cap literal");
        assert_eq!(cap(0x385cc).expect.as_deref(), Some([0x0f, 0x27, 0, 0].as_slice()), "vanilla keeps 9,999 (monster -> player)");
        assert_eq!(cap(0x4beac).expect.as_deref(), Some([0x0f, 0x27, 0, 0].as_slice()), "vanilla keeps 9,999 (player -> monster)");
        // 흡수: both callers hand the whole damage to the replaced function, on both flavours.
        for (pc, bytes) in [(0x4e210u32, [0x01, 0x1c, 0xc0, 0x46]), (0x4e936, [0x39, 0x1c, 0xc0, 0x46])] {
            let p = lgt.patches.iter().find(|p| p.pc == pc).expect("an absorb caller patch");
            assert_eq!(p.bytes, bytes, "{pc:#x}");
            assert_eq!(p.flavor, None);
        }
        let absorb = lgt.hooks.iter().find(|h| matches!(h.kind, K::AbsorbLgt)).expect("the absorb hook");
        assert_eq!(absorb.pc, 0x4b979);
    }

    /// ★ 150차 — the '%' rule `0x6d34(buf, id)` widened to `acc_options::REGEN_STAT` (0x78) by a
    /// 4-byte patch: the table entry is there, and on every id a caller can pass (an `ldrsb`,
    /// or a plain byte) the patched rule appends '%' exactly when the original did, plus 0x78.
    #[test]
    fn the_percent_rule_patch_adds_exactly_the_regen_id() {
        let entries = parser::binary_patches();
        let lgt = entries
            .iter()
            .find(|e| e.hash == Some([0x6a, 0x35, 0x29, 0x44, 0xbb, 0x43, 0xc0, 0x78, 0x9c, 0xa9, 0xf4, 0xb6, 0xf0, 0x70, 0x2f, 0x4b]))
            .expect("the LGT 01.00.07 entry");
        let p = lgt.patches.iter().find(|p| p.pc == 0x6d38).expect("the '%' patch");
        assert_eq!(p.bytes, [0x78, 0x29, 0x0c, 0xd0], "cmp r1,#0x78 / beq 0x6d56");
        assert_eq!(p.expect.as_deref(), Some([0x12, 0x29, 0x12, 0xd8].as_slice()), "cmp r1,#0x12 / bhi 0x6d62");
        assert_eq!(p.flavor, None, "both flavours");
        assert_eq!(p.bytes[0], wie_util::acc_options::REGEN_STAT, "the compare's immediate is the regen id");
        // beq at 0x6d3a: target = 0x6d3a + 4 + 2*imm8.
        assert_eq!(0x6d3a + 4 + 2 * u32::from(p.bytes[2]), 0x6d56, "lands on the sprintf block");

        const MASKS: [u32; 3] = [0xf, 0xf0 << 1, 0xf8 << 11];
        // ARMv4T LSL by register: the bottom byte of the shift register, 32+ shifts to 0.
        let lsl = |v: u32, rs: u32| {
            let s = rs & 0xff;
            if s >= 32 { 0 } else { v << s }
        };
        let tested = |r1: u32| MASKS.iter().any(|m| lsl(1, r1) & m != 0);
        let original = |r1: u32| r1 <= 0x12 && tested(r1);
        let patched = |r1: u32| r1 == 0x78 || tested(r1);
        let ids = (-128i32..=127).map(|v| v as u32).chain(0u32..=255);
        for r1 in ids {
            let want = original(r1) || r1 == 0x78;
            assert_eq!(patched(r1), want, "r1 = {r1:#x}");
        }
        let with: std::vec::Vec<u32> = (0u32..=0x7f).filter(|&r| patched(r)).collect();
        assert_eq!(with, [0, 1, 2, 3, 5, 6, 7, 8, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x78]);

        // The shipped binary (when this checkout has it): the original bytes, the three mask
        // builders the fall-through keeps, and the block the new `beq` lands on.
        let path = std::format!("{}/../../../tools_re/_binary.mod", env!("CARGO_MANIFEST_DIR"));
        let Ok(elf) = std::fs::read(&path) else {
            std::eprintln!("skipped the binary half: {path} is not in this checkout");
            return;
        };
        let at = |va: usize, n: usize| &elf[va - 0x1000 + 0x34..va - 0x1000 + 0x34 + n];
        assert_eq!(at(0x6d38, 4), p.expect.as_deref().unwrap());
        assert_eq!(at(0x6d3c, 4), [0x01, 0x20, 0x88, 0x40], "movs r0,#1 / lsls r0,r1");
        assert_eq!(at(0x6d40, 2), [0x0f, 0x23], "movs r3,#0xf");
        assert_eq!(at(0x6d46, 4), [0xf0, 0x23, 0x5b, 0x00], "movs r3,#0xf0 / lsls r3,r3,#1");
        assert_eq!(at(0x6d4e, 4), [0xf8, 0x23, 0xdb, 0x02], "movs r3,#0xf8 / lsls r3,r3,#0xb");
        assert_eq!(at(0x6d56, 4), [0x10, 0x1c, 0x03, 0x49], "adds r0,r2,#0 / ldr r1,=fmt");
    }

    #[test]
    fn pattern_scan_capture_extracts_dst_src() {
        let tokens = [
            PatternToken::Literal(0x00),
            PatternToken::Capture(CaptureName::Dst),
            PatternToken::Literal(0x3d),
            PatternToken::Capture(CaptureName::Src),
        ];
        let bytes = [0x00, 0x08, 0x3d, 0x0c];

        let m = try_match(&tokens, &bytes).expect("match");
        assert_eq!(m.dst, Some(0x08));
        assert_eq!(m.src, Some(0x0c));
    }

    #[test]
    fn pattern_bit_match_captures_register_at_correct_shift() {
        // `0b00sss011` matches `(src << 3) | 0b011`. Cross-byte consistency
        // forces the same `s` capture to agree across positions.
        let tokens = [
            PatternToken::BitMatch {
                mask: 0b1100_0111,
                fixed: 0b0000_0011,
                capture: Some((CaptureName::SrcReg, 3)),
            },
            PatternToken::BitMatch {
                mask: 0b1111_1000,
                fixed: 0b0011_0000,
                capture: Some((CaptureName::SrcReg, 0)),
            },
        ];
        // 0x0B = 0b00001011 → src bits 5..3 = 001 → R1
        // 0x31 = 0b00110001 → src bits 2..0 = 001 → R1 (consistent)
        let m = try_match(&tokens, &[0x0b, 0x31]).expect("match");
        assert_eq!(m.src_reg, Some(1));

        // Inconsistent capture must fail: first byte says R1, second says R2.
        assert!(try_match(&tokens, &[0x0b, 0x32]).is_none());
        // First byte's literal bits don't match (low nibble != 0b011).
        assert!(try_match(&tokens, &[0x0a, 0x31]).is_none());
    }

    #[test]
    fn pattern_scan_rejects_exit_b_capture_when_bytes_are_not_thumb_b() {
        let tokens = [PatternToken::Capture(CaptureName::ExitB), PatternToken::Capture(CaptureName::ExitB)];
        // `0c 3a` is `SUBS R2, #0xC` — high byte 0x3a is not in 0xE0-0xE7.
        assert!(try_match(&tokens, &[0x0c, 0x3a]).is_none());
        // `e8 e7` is a valid back-branch (high byte 0xE7).
        assert!(try_match(&tokens, &[0xe8, 0xe7]).is_some());
    }

    mod install_entry_tests {
        use alloc::{vec, vec::Vec};

        use wie_util::ByteWrite;

        use super::*;
        use crate::binary_patches::{
            hook::{Hook, HookKind},
            patch::{PatchSpec, PatternPatchSpec},
        };

        fn empty_entry(name: &str, hash: Option<[u8; 16]>) -> Entry {
            Entry {
                hash,
                name: name.into(),
                hooks: Vec::new(),
                hook_patterns: Vec::new(),
                patches: Vec::new(),
                patch_patterns: Vec::new(),
            }
        }

        #[test]
        fn install_entry_returns_patch_plus_hook_count() -> Result<()> {
            let mut core = ArmCore::new(false, None)?;
            core.map(0x2000, 0x100)?;
            core.write_bytes(0x2000, &[0xaa, 0xbb])?;

            let mut entry = empty_entry("count", Some([0u8; 16]));
            entry.patches.push(PatchSpec {
                pc: 0x2000,
                bytes: vec![0x11, 0x22],
                expect: Some(vec![0xaa, 0xbb]),
                flavor: None,
            });
            entry.hooks.push(Hook {
                pc: 0x2010 | 1,
                kind: HookKind::Memcpy,
                flavor: None,
            });

            let n = install_entry(&mut core, &entry, &[], true)?;
            assert_eq!(n, 2);
            Ok(())
        }

        #[test]
        fn install_entry_specific_entry_with_zero_results_is_fatal() -> Result<()> {
            let mut core = ArmCore::new(false, None)?;
            // Map a region with bytes that won't match the patch pattern.
            core.map(0x2000, 0x40)?;

            let mut entry = empty_entry("specific-empty", Some([1u8; 16]));
            entry.patch_patterns.push(PatternPatchSpec {
                tokens: vec![PatternToken::Literal(0xaa), PatternToken::Literal(0xbb)],
                bytes: vec![0x11, 0x22],
                expect: None,
                offset: 0,
            });

            let err = install_entry(&mut core, &entry, &[(0x2000, 0x40)], true).unwrap_err();
            assert!(format!("{err}").contains("hash matched but produced 0"));
            Ok(())
        }

        #[test]
        fn install_entry_generic_entry_with_zero_results_is_ok() -> Result<()> {
            let mut core = ArmCore::new(false, None)?;
            core.map(0x2000, 0x40)?;

            let mut entry = empty_entry("generic-empty", None);
            entry.patch_patterns.push(PatternPatchSpec {
                tokens: vec![PatternToken::Literal(0xaa), PatternToken::Literal(0xbb)],
                bytes: vec![0x11, 0x22],
                expect: None,
                offset: 0,
            });

            let n = install_entry(&mut core, &entry, &[(0x2000, 0x40)], false)?;
            assert_eq!(n, 0);
            Ok(())
        }

        #[test]
        fn install_entry_patch_pattern_overlapping_hook_pattern_is_fatal() -> Result<()> {
            // Hook pattern matches at 0x3000 (Thumb hook PC = 0x3001), patch
            // pattern matches at 0x3000 too — the SVC region [0x3000, 0x3002)
            // overlaps the patch [0x3000, 0x3002). Must fatal before any write.
            let mut core = ArmCore::new(false, None)?;
            core.map(0x3000, 0x40)?;
            core.write_bytes(0x3000, &[0x70, 0xb5, 0x00, 0x00])?;

            let mut entry = empty_entry("pat-overlap", None);
            entry.hook_patterns.push(crate::binary_patches::hook::PatternHook {
                tokens: vec![PatternToken::Literal(0x70), PatternToken::Literal(0xb5)],
                kind_template: crate::binary_patches::hook::PatternHookKind::Memcpy,
            });
            entry.patch_patterns.push(PatternPatchSpec {
                tokens: vec![PatternToken::Literal(0x70), PatternToken::Literal(0xb5)],
                bytes: vec![0x00, 0x00],
                expect: None,
                offset: 0,
            });

            let err = install_entry(&mut core, &entry, &[(0x3000, 0x40)], false).unwrap_err();
            assert!(format!("{err}").contains("overlap"), "{err}");
            // Memory unchanged.
            let mut buf = [0u8; 2];
            core.read_bytes(0x3000, &mut buf)?;
            assert_eq!(buf, [0x70, 0xb5]);
            Ok(())
        }

        #[test]
        fn install_entry_applies_patch_before_hook() -> Result<()> {
            // Patch at [0x4000, 0x4002), hook at 0x4003 (Thumb, SVC at
            // [0x4002, 0x4004)). Adjacent regions, no overlap. Both writes
            // must end up in memory.
            let mut core = ArmCore::new(false, None)?;
            core.map(0x4000, 0x40)?;
            core.write_bytes(0x4000, &[0xaa, 0xbb, 0xcc, 0xdd])?;

            let mut entry = empty_entry("ordered", Some([0u8; 16]));
            entry.patches.push(PatchSpec {
                pc: 0x4000,
                bytes: vec![0x11, 0x22],
                expect: Some(vec![0xaa, 0xbb]),
                flavor: None,
            });
            entry.hooks.push(Hook {
                pc: 0x4003,
                kind: HookKind::Memcpy,
                flavor: None,
            });

            let n = install_entry(&mut core, &entry, &[], true)?;
            assert_eq!(n, 2);

            let mut buf = [0u8; 4];
            core.read_bytes(0x4000, &mut buf)?;
            assert_eq!(&buf[0..2], &[0x11, 0x22], "patch applied");
            assert_eq!(buf[3], 0xdf, "hook SVC high byte");
            assert_eq!(buf[2] as u32, 0x80, "hook SVC low byte = SVC #0x80");
            Ok(())
        }

        #[test]
        fn install_entry_pattern_patch_expect_mismatch_is_fatal() -> Result<()> {
            let mut core = ArmCore::new(false, None)?;
            core.map(0x5000, 0x40)?;
            core.write_bytes(0x5000, &[0x10, 0x20, 0x30, 0x40])?;

            let mut entry = empty_entry("pp-mm", None);
            entry.patch_patterns.push(PatternPatchSpec {
                tokens: vec![
                    PatternToken::Literal(0x10),
                    PatternToken::Literal(0x20),
                    PatternToken::Literal(0x30),
                    PatternToken::Literal(0x40),
                ],
                bytes: vec![0xaa, 0xbb],
                expect: Some(vec![0x99, 0x99]), // doesn't match what's there
                offset: 1,
            });

            let err = install_entry(&mut core, &entry, &[(0x5000, 0x40)], false).unwrap_err();
            assert!(format!("{err}").contains("expected"), "{err}");
            // Memory unchanged.
            let mut buf = [0u8; 4];
            core.read_bytes(0x5000, &mut buf)?;
            assert_eq!(buf, [0x10, 0x20, 0x30, 0x40]);
            Ok(())
        }
    }
}
