use alloc::{format, string::String, vec::Vec};

use wie_util::{ByteRead, ByteWrite, Result, WieError};

use super::{Entry, PatternToken, hook::Hook, scan_pattern};
use crate::ArmCore;

pub struct PatchSpec {
    pub pc: u32,
    pub bytes: Vec<u8>,
    pub expect: Option<Vec<u8>>,
    /// `flavor = "main" | "vanilla"` in the TOML: install this patch on that APK flavour
    /// only. `None` = every flavour. See [`Flavor`].
    pub flavor: Option<Flavor>,
}

/// The APK flavour this library was compiled for (`BRANCHES.md`). `flavor.properties`
/// `flavor=vanilla` → `build_android.ps1` → rustc `--cfg hero3_vanilla`; nothing else
/// sets it, and nothing at runtime can flip it.
///
/// ★★ 114차 (2026-09-19): until now the only flavour difference was ONE hook skipped by
/// name in `hook.rs` (`ReaverDropLgt`). The vanilla rule grew — the level cap stays 70
/// there while main went to 99 (111차) — and that is five raw-byte patches, so the choice
/// moved into the data: a patch tagged `flavor = "main"` is simply not installed on a
/// vanilla build (`resolve_patches`), and the boot log says so per patch. The item TABLES
/// stay shared by design (77차-B): a flavour tag must never sit on a count-table byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Main,
    Vanilla,
}

impl Flavor {
    /// The flavour of THIS build.
    pub fn of_build() -> Flavor {
        if cfg!(hero3_vanilla) { Flavor::Vanilla } else { Flavor::Main }
    }

    /// The TOML spelling. Anything but the two names is a table bug and is rejected
    /// (`None`) so the parser fails loudly instead of installing on every flavour.
    pub fn parse(s: &str) -> Option<Flavor> {
        match s {
            "main" => Some(Flavor::Main),
            "vanilla" => Some(Flavor::Vanilla),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Flavor::Main => "main",
            Flavor::Vanilla => "vanilla",
        }
    }
}

/// Does a patch tagged `spec` install on a build of flavour `build`? Untagged = always.
pub fn flavor_keeps(spec: Option<Flavor>, build: Flavor) -> bool {
    match spec {
        None => true,
        Some(f) => f == build,
    }
}

pub struct PatternPatchSpec {
    pub tokens: Vec<PatternToken>,
    pub bytes: Vec<u8>,
    pub expect: Option<Vec<u8>>,
    pub offset: u32,
}

struct Patch {
    addr: u32,
    bytes: Vec<u8>,
    expect: Option<Vec<u8>>,
}

/// Resolve every patch site, reject any overlap with `hooks` (their SVC
/// regions), and apply. Returns the count actually applied. Two-phase apply
/// (verify-all-then-write-all) keeps multi-patch atomicity: if any expect
/// fails, no patch is written.
pub fn install_patches(core: &mut ArmCore, entry: &Entry, scan_ranges: &[(u32, u32)], hooks: &[Hook]) -> Result<usize> {
    let patches = resolve_patches(core, entry, scan_ranges)?;
    validate_overlap(&patches, hooks, &entry.name)?;
    apply_patches(core, &entry.name, &patches)?;
    Ok(patches.len())
}

fn resolve_patches(core: &mut ArmCore, entry: &Entry, scan_ranges: &[(u32, u32)]) -> Result<Vec<Patch>> {
    let build = Flavor::of_build();
    let mut out: Vec<Patch> = Vec::with_capacity(entry.patches.len());
    for p in &entry.patches {
        // ★ 114차: a flavour-tagged patch is dropped HERE, before `expect` is checked and
        //   before the count is taken — so a vanilla boot reports `Applied 46 patches`
        //   against main's 51 and the skipped bytes stay original in the guest.
        if !flavor_keeps(p.flavor, build) {
            tracing::warn!(
                "{}: not installing the {}-only patch at {:#x} ({} bytes) on this {} build",
                entry.name,
                p.flavor.map_or("?", Flavor::as_str),
                p.pc,
                p.bytes.len(),
                build.as_str()
            );
            continue;
        }
        out.push(Patch {
            addr: p.pc,
            bytes: p.bytes.clone(),
            expect: p.expect.clone(),
        });
    }

    for (idx, pp) in entry.patch_patterns.iter().enumerate() {
        let matches = scan_pattern(core, &pp.tokens, scan_ranges)?;
        if matches.is_empty() {
            tracing::warn!("Patch pattern #{idx} in entry {}: no matches", entry.name);
            continue;
        }
        for (match_addr, _) in matches {
            let addr = match_addr.checked_add(pp.offset).ok_or_else(|| {
                WieError::FatalError(format!(
                    "entry {}: patch pattern #{idx} address overflow: match {match_addr:#x} + offset {:#x}",
                    entry.name, pp.offset
                ))
            })?;
            out.push(Patch {
                addr,
                bytes: pp.bytes.clone(),
                expect: pp.expect.clone(),
            });
        }
    }

    Ok(out)
}

/// Two-phase apply: verify every patch's `expect` and simulate the post-patch
/// neighborhood for SVC #0x80 emergence first, then write all bytes. Both
/// pre-write phases observe guest memory but never mutate it, so any failure
/// leaves memory untouched even when other patches would have succeeded.
fn apply_patches(core: &mut ArmCore, entry_name: &str, patches: &[Patch]) -> Result<()> {
    for patch in patches {
        let mut current = alloc::vec![0u8; patch.bytes.len()];
        core.read_bytes(patch.addr, &mut current)?;
        match &patch.expect {
            Some(expect) => {
                if current.as_slice() != expect.as_slice() {
                    return Err(WieError::FatalError(format!(
                        "entry {entry_name}: patch at {:#x} expected {} but found {}",
                        patch.addr,
                        hex_dump(expect),
                        hex_dump(&current),
                    )));
                }
            }
            None => {
                tracing::warn!(
                    "entry {entry_name}: patch at {:#x} applied without `expect` verification (length {})",
                    patch.addr,
                    patch.bytes.len()
                );
            }
        }
        reject_emergent_svc(core, entry_name, patch)?;
    }
    for patch in patches {
        core.write_bytes(patch.addr, &patch.bytes)?;
        // warn, not info: the Android build filters the wie crates to warn, and "did this
        // raw-byte rewrite of guest code actually go in" is the first question asked when a
        // patched behaviour does not appear. The hook installer logs at warn for the same
        // reason; a patch is strictly more invasive than a hook and was staying silent.
        tracing::warn!("Patch applied at {:#x} ({} bytes)", patch.addr, patch.bytes.len());
    }
    if !patches.is_empty() {
        tracing::warn!("Applied {} patches for {entry_name}", patches.len());
    }
    Ok(())
}

/// Read the `[addr - 1, addr + bytes.len() + 1)` window, splice in the patch
/// bytes, and reject if the resulting halfwords would form `SVC #0x80`. This
/// catches cases where the patch boundary plus an unmodified neighbor synthesizes
/// the dispatcher opcode, which `reject_svc_pattern` (parser-side, payload-only)
/// can't see.
fn reject_emergent_svc(core: &mut ArmCore, entry_name: &str, patch: &Patch) -> Result<()> {
    let pad_start = patch.addr.saturating_sub(1);
    let body_offset = (patch.addr - pad_start) as usize;
    let window_len = body_offset + patch.bytes.len() + 1;
    let mut window = alloc::vec![0u8; window_len];
    core.read_bytes(pad_start, &mut window)?;
    window[body_offset..body_offset + patch.bytes.len()].copy_from_slice(&patch.bytes);
    for w in window.windows(2) {
        if w == [0x80, 0xdf] {
            return Err(WieError::FatalError(format!(
                "entry {entry_name}: patch at {:#x} would synthesize SVC #0x80 with a neighboring byte",
                patch.addr
            )));
        }
    }
    Ok(())
}

enum Region {
    Patch(u32),
    Hook(u32),
}

impl Region {
    fn label(&self) -> String {
        match self {
            Region::Patch(addr) => format!("patch@{addr:#x}"),
            Region::Hook(pc) => format!("hook@{pc:#x}"),
        }
    }
}

/// Reject any byte-region collision between patches and hook SVC sites before
/// we write anything. Patches are `[addr, addr + bytes.len())`; hook SVC sites
/// are the 2 bytes at `pc & !1`.
fn validate_overlap(patches: &[Patch], hooks: &[Hook], entry_name: &str) -> Result<()> {
    let mut regions: Vec<(u32, u32, Region)> = Vec::new();
    for p in patches {
        debug_assert!(!p.bytes.is_empty(), "parser must reject empty `bytes`");
        regions.push((p.addr, p.addr.saturating_add(p.bytes.len() as u32), Region::Patch(p.addr)));
    }
    for h in hooks {
        let base = h.pc & !1;
        regions.push((base, base.saturating_add(2), Region::Hook(h.pc)));
    }
    regions.sort_by_key(|r| r.0);
    for w in regions.windows(2) {
        let (start_a, end_a, ref kind_a) = w[0];
        let (start_b, _, ref kind_b) = w[1];
        if start_b < end_a {
            return Err(WieError::FatalError(format!(
                "entry {entry_name}: binary_patches overlap at {start_b:#x} between {} ([{start_a:#x}, {end_a:#x})) and {} (start {start_b:#x})",
                kind_a.label(),
                kind_b.label()
            )));
        }
    }
    Ok(())
}

fn hex_dump(bytes: &[u8]) -> String {
    let mut s = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};

    use super::*;
    use crate::binary_patches::{
        PatternToken,
        hook::{Hook, HookKind},
    };

    fn empty_entry(name: &str) -> Entry {
        Entry {
            hash: Some([0u8; 16]),
            name: name.into(),
            hooks: Vec::new(),
            hook_patterns: Vec::new(),
            patches: Vec::new(),
            patch_patterns: Vec::new(),
        }
    }

    fn pc_patch(pc: u32, flavor: Option<Flavor>) -> PatchSpec {
        PatchSpec {
            pc,
            bytes: vec![0x11, 0x22],
            expect: None,
            flavor,
        }
    }

    /// The other flavour, whichever this test binary was built as — so the test is
    /// meaningful under both `cargo test` and `RUSTFLAGS=--cfg hero3_vanilla cargo test`.
    fn other_flavor() -> Flavor {
        match Flavor::of_build() {
            Flavor::Main => Flavor::Vanilla,
            Flavor::Vanilla => Flavor::Main,
        }
    }

    // ── 114차: flavour-tagged patches ───────────────────────────────────────
    #[test]
    fn flavor_keeps_untagged_everywhere_and_tagged_only_on_its_own_flavour() {
        for build in [Flavor::Main, Flavor::Vanilla] {
            assert!(flavor_keeps(None, build), "untagged must install on {build:?}");
            assert!(flavor_keeps(Some(build), build), "own tag must install on {build:?}");
        }
        assert!(!flavor_keeps(Some(Flavor::Main), Flavor::Vanilla), "main-only must not install on vanilla");
        assert!(!flavor_keeps(Some(Flavor::Vanilla), Flavor::Main), "vanilla-only must not install on main");
    }

    #[test]
    fn flavor_parse_accepts_exactly_the_two_names() {
        assert_eq!(Flavor::parse("main"), Some(Flavor::Main));
        assert_eq!(Flavor::parse("vanilla"), Some(Flavor::Vanilla));
        for bad in ["Main", "VANILLA", "original", "", " main"] {
            assert_eq!(Flavor::parse(bad), None, "{bad:?} must be rejected");
        }
        assert_eq!(Flavor::parse(Flavor::Main.as_str()), Some(Flavor::Main));
        assert_eq!(Flavor::parse(Flavor::Vanilla.as_str()), Some(Flavor::Vanilla));
    }

    #[test]
    fn resolve_patches_drops_the_other_flavours_patch_and_keeps_its_own() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        let mut entry = empty_entry("flavoured");
        entry.patches.push(pc_patch(0x1000, None));
        entry.patches.push(pc_patch(0x2000, Some(other_flavor())));
        entry.patches.push(pc_patch(0x3000, Some(Flavor::of_build())));
        let resolved = resolve_patches(&mut core, &entry, &[])?;
        let addrs: Vec<u32> = resolved.iter().map(|p| p.addr).collect();
        assert_eq!(addrs, vec![0x1000, 0x3000], "the other flavour's patch is not resolved, so it is neither verified nor counted");
        Ok(())
    }

    #[test]
    fn install_patches_counts_only_what_it_wrote() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        // Inside the mapping with a margin: the SVC-formation check reads each patch's
        // neighbouring bytes, and at a mapping's very first byte that read faults.
        core.map(0x50000, 0x100)?;
        core.write_bytes(0x50010, &[0xaa, 0xbb, 0xaa, 0xbb])?;
        let mut entry = empty_entry("flavour-count");
        entry.patches.push(PatchSpec {
            pc: 0x50010,
            bytes: vec![0x11, 0x22],
            expect: Some(vec![0xaa, 0xbb]),
            flavor: None,
        });
        entry.patches.push(PatchSpec {
            pc: 0x50012,
            bytes: vec![0x33, 0x44],
            // ⛔ An `expect` that would FAIL on this memory: proves the skipped patch is
            //    not even verified (a stale main-only expect must not break a vanilla boot).
            expect: Some(vec![0x99, 0x99]),
            flavor: Some(other_flavor()),
        });
        let n = install_patches(&mut core, &entry, &[], &[])?;
        assert_eq!(n, 1, "the count the boot log prints is the number actually written");
        let mut after = [0u8; 4];
        core.read_bytes(0x50010, &mut after)?;
        assert_eq!(after, [0x11, 0x22, 0xaa, 0xbb], "the other flavour's bytes stay original");
        Ok(())
    }

    #[test]
    fn resolve_patches_pc_passes_through() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        let mut entry = empty_entry("pc-pass");
        entry.patches.push(PatchSpec {
            pc: 0x1000,
            bytes: vec![0xaa, 0xbb],
            expect: Some(vec![0xcc, 0xdd]),
            flavor: None,
        });
        let resolved = resolve_patches(&mut core, &entry, &[])?;
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].addr, 0x1000);
        assert_eq!(resolved[0].bytes, vec![0xaa, 0xbb]);
        assert_eq!(resolved[0].expect.as_deref(), Some([0xcc, 0xdd].as_slice()));
        Ok(())
    }

    #[test]
    fn resolve_patches_pattern_multi_match_applies_offset() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x40000, 0x100)?;
        // Lay the same 4-byte pattern at two locations
        core.write_bytes(0x40000, &[0xaa, 0xbb, 0xcc, 0xdd])?;
        core.write_bytes(0x40020, &[0xaa, 0xbb, 0xcc, 0xdd])?;

        let mut entry = empty_entry("multi");
        entry.patch_patterns.push(PatternPatchSpec {
            tokens: vec![
                PatternToken::Literal(0xaa),
                PatternToken::Literal(0xbb),
                PatternToken::Literal(0xcc),
                PatternToken::Literal(0xdd),
            ],
            bytes: vec![0x11, 0x22],
            expect: None,
            offset: 1,
        });
        let resolved = resolve_patches(&mut core, &entry, &[(0x40000, 0x100)])?;
        let addrs: Vec<u32> = resolved.iter().map(|p| p.addr).collect();
        assert_eq!(addrs, vec![0x40001, 0x40021]);
        Ok(())
    }

    #[test]
    fn resolve_patches_pattern_offset_overflow_is_fatal() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        // match_addr (0x1000_0000) + offset (0xffff_ffff) overflows u32.
        core.map(0x1000_0000, 0x10)?;
        core.write_bytes(0x1000_0000, &[0xaa, 0xbb])?;

        let mut entry = empty_entry("overflow");
        entry.patch_patterns.push(PatternPatchSpec {
            tokens: vec![PatternToken::Literal(0xaa), PatternToken::Literal(0xbb)],
            bytes: vec![0x11, 0x22],
            expect: None,
            offset: 0xffff_ffff,
        });
        match resolve_patches(&mut core, &entry, &[(0x1000_0000, 0x10)]) {
            Ok(_) => panic!("expected overflow fatal"),
            Err(e) => assert!(format!("{e}").contains("overflow"), "{e}"),
        }
        Ok(())
    }

    #[test]
    fn resolve_patches_pattern_zero_match_warns_and_skips() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x40000, 0x40)?;
        // Memory is zeros; nothing matches.
        let mut entry = empty_entry("zero-match");
        entry.patch_patterns.push(PatternPatchSpec {
            tokens: vec![PatternToken::Literal(0xaa), PatternToken::Literal(0xbb)],
            bytes: vec![0x11, 0x22],
            expect: None,
            offset: 0,
        });
        let resolved = resolve_patches(&mut core, &entry, &[(0x40000, 0x40)])?;
        assert!(resolved.is_empty());
        Ok(())
    }

    #[test]
    fn apply_patches_happy_path_pc() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x100)?;
        core.write_bytes(0x2000, &[0xaa, 0xbb, 0xcc, 0xdd])?;

        let patches = vec![Patch {
            addr: 0x2000,
            bytes: vec![0x00, 0x20, 0x70, 0x47],
            expect: Some(vec![0xaa, 0xbb, 0xcc, 0xdd]),
        }];
        apply_patches(&mut core, "happy", &patches)?;

        let mut out = [0u8; 4];
        core.read_bytes(0x2000, &mut out)?;
        assert_eq!(out, [0x00, 0x20, 0x70, 0x47]);
        Ok(())
    }

    #[test]
    fn apply_patches_expect_mismatch_does_not_write() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x100)?;
        core.write_bytes(0x2000, &[0xaa, 0xbb, 0xcc, 0x00])?;

        let patches = vec![Patch {
            addr: 0x2000,
            bytes: vec![0x00, 0x20, 0x70, 0x47],
            expect: Some(vec![0xaa, 0xbb, 0xcc, 0xdd]),
        }];
        let err = apply_patches(&mut core, "mm", &patches).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("expected"), "unexpected: {msg}");
        let mut out = [0u8; 4];
        core.read_bytes(0x2000, &mut out)?;
        assert_eq!(out, [0xaa, 0xbb, 0xcc, 0x00], "memory must not be modified on mismatch");
        Ok(())
    }

    #[test]
    fn apply_patches_verifies_all_before_writing_any() -> Result<()> {
        // Two patches: the first would succeed, the second's expect mismatches.
        // Two-phase apply must leave both regions untouched.
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x100)?;
        core.write_bytes(0x2000, &[0xaa, 0xbb])?;
        core.write_bytes(0x2010, &[0x99, 0x88])?; // mismatches second patch's expect

        let patches = vec![
            Patch {
                addr: 0x2000,
                bytes: vec![0x11, 0x22],
                expect: Some(vec![0xaa, 0xbb]),
            },
            Patch {
                addr: 0x2010,
                bytes: vec![0x33, 0x44],
                expect: Some(vec![0xcc, 0xdd]), // doesn't match guest memory
            },
        ];
        let err = apply_patches(&mut core, "verify-all", &patches).unwrap_err();
        assert!(format!("{err}").contains("expected"));

        // Both patch regions must be unchanged — first patch must NOT have
        // been applied even though its own expect matched.
        let mut buf = [0u8; 2];
        core.read_bytes(0x2000, &mut buf)?;
        assert_eq!(buf, [0xaa, 0xbb], "first patch must not be applied if any later patch fails");
        core.read_bytes(0x2010, &mut buf)?;
        assert_eq!(buf, [0x99, 0x88]);
        Ok(())
    }

    #[test]
    fn apply_patches_rejects_emergent_svc_after_patch() -> Result<()> {
        // Guest byte at addr+1 is 0xdf; a 1-byte patch writing 0x80 at addr
        // would synthesize SVC #0x80 (`80 df` LE) with the unmodified neighbor.
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x100)?;
        core.write_bytes(0x2000, &[0x00, 0xdf])?;

        let patches = vec![Patch {
            addr: 0x2000,
            bytes: vec![0x80],
            expect: None,
        }];
        let err = apply_patches(&mut core, "emergent", &patches).unwrap_err();
        assert!(format!("{err}").contains("synthesize SVC"), "{err}");

        // Memory must not have been written.
        let mut buf = [0u8; 2];
        core.read_bytes(0x2000, &mut buf)?;
        assert_eq!(buf, [0x00, 0xdf]);
        Ok(())
    }

    #[test]
    fn apply_patches_rejects_emergent_svc_before_patch() -> Result<()> {
        // Guest byte at addr-1 is 0x80; a patch writing 0xdf as its first byte
        // synthesizes SVC #0x80 with the unmodified preceding byte.
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x100)?;
        core.write_bytes(0x2000, &[0x80, 0x00, 0x00])?;

        let patches = vec![Patch {
            addr: 0x2001,
            bytes: vec![0xdf, 0xff],
            expect: None,
        }];
        let err = apply_patches(&mut core, "emergent-pre", &patches).unwrap_err();
        assert!(format!("{err}").contains("synthesize SVC"), "{err}");
        Ok(())
    }

    #[test]
    fn apply_patches_without_expect_writes_with_warn() -> Result<()> {
        let mut core = ArmCore::new(false, None)?;
        core.map(0x2000, 0x100)?;
        core.write_bytes(0x2000, &[0x11, 0x22])?;

        let patches = vec![Patch {
            addr: 0x2000,
            bytes: vec![0x33, 0x44],
            expect: None,
        }];
        apply_patches(&mut core, "no-expect", &patches)?;
        let mut out = [0u8; 2];
        core.read_bytes(0x2000, &mut out)?;
        assert_eq!(out, [0x33, 0x44]);
        Ok(())
    }

    fn hook_at(pc: u32) -> Hook {
        Hook { pc, kind: HookKind::Memcpy, flavor: None }
    }

    #[test]
    fn validate_overlap_patch_patch_overlap_is_fatal() {
        let patches = vec![
            Patch {
                addr: 0x2000,
                bytes: vec![1, 2, 3, 4],
                expect: None,
            },
            Patch {
                addr: 0x2002,
                bytes: vec![5, 6],
                expect: None,
            },
        ];
        let err = validate_overlap(&patches, &[], "pp").unwrap_err();
        assert!(format!("{err}").contains("overlap"));
    }

    #[test]
    fn validate_overlap_patch_hook_overlap_is_fatal() {
        let patches = vec![Patch {
            addr: 0x2000,
            bytes: vec![1, 2],
            expect: None,
        }];
        let err = validate_overlap(&patches, &[hook_at(0x2001)], "ph").unwrap_err();
        assert!(format!("{err}").contains("overlap"));
    }

    #[test]
    fn validate_overlap_patch_pattern_hook_overlap_is_fatal() {
        // Simulates resolve_hooks having expanded a pattern hook to PC 0x3001.
        let patches = vec![Patch {
            addr: 0x3000,
            bytes: vec![1, 2],
            expect: None,
        }];
        let err = validate_overlap(&patches, &[hook_at(0x3001)], "pat-hook").unwrap_err();
        assert!(format!("{err}").contains("overlap"));
    }

    #[test]
    fn validate_overlap_adjacent_regions_ok() {
        // patch [0x2000, 0x2002), hook [0x2002, 0x2004) — touching but not overlapping.
        let patches = vec![Patch {
            addr: 0x2000,
            bytes: vec![1, 2],
            expect: None,
        }];
        validate_overlap(&patches, &[hook_at(0x2003)], "adjacent").expect("adjacent should be ok");
    }
}
