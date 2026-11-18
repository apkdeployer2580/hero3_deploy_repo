//! Standalone test for the per-build guest anchor table.
//!
//!     rustc --edition 2021 --test anchors_selftest.rs -o t.exe && ./t.exe
//!
//! ★ `include!`s the REAL source (`wie_android/src/guest_anchors.rs`), which is the
//! same file `wie_android::guest_build` and `wie_probe` splice in. That is the point:
//! the table exists so the app and the probe cannot disagree about where CHAR is, and
//! a test that exercised a copy would not defend that.
//!
//! What is actually worth asserting here is not the numbers for their own sake but the
//! DIFFERENCES between the builds — every one of them was a bug at some point:
//!   * KTF reaches CHAR through a pointer global, LGT's record IS a static. Treating
//!     the LGT static as a pointer reads a sprite (that mistake is written up in
//!     MIGRATION_LGT.md §2.7).
//!   * the state byte moved `+0x7c` → `+0x78`.
//!   * containers are `*(handle) + 8` on KTF and direct pointers on LGT. Getting this
//!     one wrong is silent: the chain still resolves, on the wrong bytes.
//! `cargo test -p wie_android` cannot run on this host (linked winit/android deps die
//! at startup), which is why this is a standalone file — see CLAUDE.md.

include!("wie_android/src/guest_anchors.rs");

#[cfg(test)]
mod tests {
    use super::*;

    /// KTF: `*(sl+0x444)` with the observed static fallback. Both are pointers to
    /// follow, never bases to read fields off.
    #[test]
    fn ktf_reaches_char_through_a_pointer() {
        match GuestBuild::Ktf.char_anchor() {
            Some(CharAnchor::Pointer { global, fallback }) => {
                assert_eq!(global, 0x1b_3084);
                assert_eq!(fallback, 0x1b_7b6c);
            }
            _ => panic!("KTF must resolve CHAR through a pointer global"),
        }
    }

    /// LGT: the record itself is a `.bss` static — there is no pointer to null-check,
    /// which is why `items::resolve_char` has to judge it by its CONTENTS.
    #[test]
    fn lgt_char_is_a_static() {
        match GuestBuild::Lgt.char_anchor() {
            Some(CharAnchor::Static(base)) => assert_eq!(base, 0x150_3a88),
            _ => panic!("LGT must resolve CHAR as a static"),
        }
    }

    /// The `.bss` base has to be consistent with the other LGT anchor measured the same
    /// day: the camera/scroll pair at `.bss 0x150d914` is also `CHAR+0x9e8c`
    /// (MIGRATION_LGT.md §2.7). If either number is ever retyped wrong this fails.
    #[test]
    fn lgt_char_base_agrees_with_the_camera_anchor() {
        let Some(CharAnchor::Static(base)) = GuestBuild::Lgt.char_anchor() else {
            panic!("LGT anchor shape changed");
        };
        assert_eq!(base + 0x9e8c, 0x150_d914);
    }

    /// SKT has never been measured; nothing may claim to know where its CHAR is.
    #[test]
    fn skt_has_no_char_anchor() {
        assert!(GuestBuild::Skt.char_anchor().is_none());
    }

    #[test]
    fn the_state_byte_moved_between_builds() {
        assert_eq!(GuestBuild::Ktf.state_off(), 0x7c);
        assert_eq!(GuestBuild::Lgt.state_off(), 0x78);
        assert_ne!(GuestBuild::Ktf.state_off(), GuestBuild::Lgt.state_off());
    }

    /// The container convention. KTF's carrier runtime hands the game handles, LGT's
    /// hands it direct pointers; swapping these is the silent failure mode, because a
    /// wrong-but-in-heap word still dereferences.
    /// ★★★ 76차: the online module's screen word, `*(getBase() + 0xa3d4)`, as a CHAR offset —
    /// the pin source for a CREATED character. `getBase()` (0x1068) returns CHAR + 0x28, and
    /// the word must sit where `net_disconnect_modal` already measured its neighbours
    /// (`showModal`'s clear word is ctx+0x40 = base+0xa3dc, two words above it).
    #[test]
    fn the_shell_screen_word_is_the_online_modules_base_plus_0xa3d4() {
        let off = GuestBuild::Lgt.shell_screen_off().expect("located on LGT");
        assert_eq!(off, 0xa3d4 + 0x28);
        let Some(CharAnchor::Static(char_base)) = GuestBuild::Lgt.char_anchor() else {
            panic!("LGT anchor shape changed");
        };
        let m = GuestBuild::Lgt.net_disconnect_modal().expect("located on LGT");
        assert_eq!(char_base + off, m.clear_word - 8, "screen word = ctx+0x38, clear word = ctx+0x40");
        assert_eq!(char_base + off, 0x150_de84);
        assert!((0x150_0000..0x150_0000 + 0x1_2958).contains(&(char_base + off)), "must be in .bss");
        assert!(GuestBuild::Ktf.shell_screen_off().is_none(), "not located on KTF (the APK ships LGT)");
        assert!(GuestBuild::Skt.shell_screen_off().is_none());
    }

    #[test]
    fn containers_are_handles_on_ktf_and_direct_on_lgt() {
        assert_eq!(GuestBuild::Ktf.container_data_off(), Some(8));
        assert_eq!(GuestBuild::Lgt.container_data_off(), None);
    }

    /// The blend-suppression global is a KTF address; on LGT it has not been located,
    /// and claiming an address there would read an unrelated word of a different
    /// compile. `None` = never suppress, which is what KTF does in practice anyway.
    #[test]
    fn noblend_global_is_only_claimed_where_it_was_measured() {
        assert_eq!(GuestBuild::Ktf.minimap_noblend_global(), Some(0x001a_4bc4));
        assert_eq!(GuestBuild::Lgt.minimap_noblend_global(), None);
        assert_eq!(GuestBuild::Skt.minimap_noblend_global(), None);
    }

    /// The bottom-HUD skip: both builds have the block, and each one's branch has to
    /// encode as a Thumb-1 `b` that actually lands on that build's epilogue. Recomputing
    /// the immediate here is the point — a mistyped branch would jump into the middle of
    /// the renderer, and the byte-check in front of the write cannot catch that.
    #[test]
    fn hud_bottom_branches_reach_their_epilogues() {
        // (build, epilogue the branch must land on)
        for (build, epilogue) in [(GuestBuild::Ktf, 0x14_a324u32), (GuestBuild::Lgt, 0x00_304au32)] {
            let (site, _orig, branch) = build.hud_bottom_patch().expect("both builds are located");
            assert_eq!(branch & 0xf800, 0xe000, "{} patch must be an unconditional Thumb b", build.name());
            let imm11 = i32::from(branch & 0x7ff);
            assert!(imm11 <= 0x3ff, "{}: imm11 {imm11:#x} would be a negative branch", build.name());
            assert_eq!(site as i32 + 4 + imm11 * 2, epilogue as i32, "{} branch target", build.name());
        }
    }

    /// The instruction being overwritten, as read back from the live guest in a probe
    /// run: KTF `ldr r3,[sp,#0x100]` = 0x9b40, LGT `ldr r5,[sp,#0x94]` = 0x9d25. Both
    /// are `ldr rX,[sp,#imm]`, which is what the block starts with on both builds.
    #[test]
    fn hud_bottom_original_is_an_sp_relative_load() {
        for build in [GuestBuild::Ktf, GuestBuild::Lgt] {
            let (_site, orig, _branch) = build.hud_bottom_patch().unwrap();
            assert_eq!(orig & 0xf800, 0x9800, "{}: original must be ldr rX,[sp,#imm]", build.name());
        }
        assert_eq!(GuestBuild::Ktf.hud_bottom_patch().unwrap().1, 0x9b40);
        assert_eq!(GuestBuild::Lgt.hud_bottom_patch().unwrap().1, 0x9d25);
    }

    #[test]
    fn skt_has_no_hud_patch() {
        assert!(GuestBuild::Skt.hud_bottom_patch().is_none());
    }

    /// The EXP divisor is located on both builds, at addresses that must not be
    /// confused with each other (the KTF one is 0x14654e, the LGT one 0x4d3cc — and the
    /// LGT `.text` is only 0x5db85 long, so a KTF address there is not even in range).
    #[test]
    fn exp_divisor_sites_are_per_build_and_in_range() {
        let ktf = GuestBuild::Ktf.exp_divisor_site().unwrap();
        let lgt = GuestBuild::Lgt.exp_divisor_site().unwrap();
        assert_eq!(ktf, 0x14_654e);
        assert_eq!(lgt, 0x04_d3cc);
        assert_ne!(ktf, lgt);
        // LGT is an ELF with `.text` at vaddr 0x1000, size 0x5db85 on Ver 01.00.07.
        assert!((0x1000..0x1000 + 0x5db85).contains(&lgt), "LGT site must be inside .text");
        assert!(!(0x1000..0x1000 + 0x5db85).contains(&ktf), "the KTF site is not an LGT address");
        assert!(GuestBuild::Skt.exp_divisor_site().is_none());
    }

    /// The 천상의 영약 target: the game's own `checkLevelUp`, THUMB bit set, inside LGT
    /// `.text`. It is the kill path's own routine (named alongside `exp_divisor_site`),
    /// so `level_up_fn() - 1` and `exp_divisor_site` must live in the same image.
    #[test]
    fn level_up_fn_is_a_thumb_lgt_text_address() {
        let f = GuestBuild::Lgt.level_up_fn().expect("located on LGT");
        assert_eq!(f, 0x4_d715);
        assert_eq!(f & 1, 1, "must carry the Thumb bit for call_guest_function");
        assert!((0x1000..0x1000 + 0x5db85).contains(&(f & !1)), "inside LGT .text");
        assert!(GuestBuild::Ktf.level_up_fn().is_none(), "not ported to KTF (APK ships LGT)");
        assert!(GuestBuild::Skt.level_up_fn().is_none());
    }

    /// The network-warehouse deposit grade gate: one halfword, `cmp r3,#3` -> `cmp r3,#7`,
    /// inside LGT `.text`. The patch must be size-neutral (both are `cmp rN,#imm8`,
    /// encoding `0x2800 | imm`) and change ONLY the immediate, or it would corrupt the
    /// instruction stream rather than widen the grade bound.
    #[test]
    fn warehouse_grade_gate_is_a_size_neutral_immediate_bump() {
        let (site, orig, patched) = GuestBuild::Lgt.warehouse_grade_gate().expect("located on LGT");
        assert_eq!(site, 0x6f9a);
        assert!((0x1000..0x1000 + 0x5db85).contains(&site), "inside LGT .text");
        // cmp r3,#imm8 = 0x2b00 | imm. Same register, only the immediate moves.
        assert_eq!(orig & 0xff00, 0x2b00, "orig is cmp r3,#imm8");
        assert_eq!(patched & 0xff00, 0x2b00, "patched stays cmp r3,#imm8");
        assert_eq!(orig & 0xff, 3, "orig admits grade <= 3 (하늘)");
        assert_eq!(patched & 0xff, 7, "patched admits grade <= 7 (all real grades top out at 5)");
        assert!(GuestBuild::Ktf.warehouse_grade_gate().is_none());
        assert!(GuestBuild::Skt.warehouse_grade_gate().is_none());
    }

    /// The equip-level relief, one shape per build. KTF rewrites the one-halfword
    /// register move at 0x13d0aa; LGT has no such move, so it biases the PLAYER LEVEL
    /// load at 0x477ce instead of the requirement load at 0x477d2 (MIGRATION_LGT.md
    /// §2.12: the requirement-side rewrite proposed in §2.8 would turn an `ldrsb` into
    /// an `ldrb` and flip any item whose requirement byte is >= 0x80 from allowed to
    /// blocked).
    ///
    /// ★ The `assert_ne!` below is the point of this test: 0x477d2 is four bytes further
    /// on and is the address the older plan named, so an edit that "restores" it would
    /// look like a fix and be the regression.
    #[test]
    fn equip_relief_shapes_are_per_build_and_lgt_biases_the_level_side() {
        match GuestBuild::Ktf.equip_relief_site() {
            Some(EquipReliefSite::MoveHalfword { site }) => assert_eq!(site, 0x13_d0aa),
            _ => panic!("KTF relieves the equip check by rewriting the move halfword"),
        }
        match GuestBuild::Lgt.equip_relief_site() {
            Some(EquipReliefSite::LevelBias { site }) => {
                assert_eq!(site, 0x4_77ce, "LGT biases the player-level load");
                assert_ne!(site, 0x4_77d2, "that is the requirement load — it must keep its ldrsb");
                // Inside LGT `.text` (vaddr 0x1000, size 0x5db85 on Ver 01.00.07),
                // where the KTF move site is not.
                assert!((0x1000..0x1000 + 0x5db85).contains(&site));
                assert!(!(0x1000..0x1000 + 0x5db85).contains(&0x13_d0aau32));
            }
            _ => panic!("LGT relieves the equip check by biasing the level load"),
        }
        assert!(GuestBuild::Skt.equip_relief_site().is_none());
    }

    /// `popup_handle[8]` — the array whose row-0 label is the ONLY thing that can start
    /// a cash buff (`items::cash_use_pending`), hence the only thing that can make the
    /// EXP-divisor and relief patches fire at all. Same Pointer/Static disagreement as
    /// the CHAR record, for the same reason.
    #[test]
    fn popup_handle_anchors_are_per_build() {
        match GuestBuild::Ktf.popup_handles() {
            Some(PopupHandles::Pointer { global }) => assert_eq!(global, 0x1b_2df4),
            _ => panic!("KTF reaches the popup labels through an sl-relative data slot"),
        }
        match GuestBuild::Lgt.popup_handles() {
            Some(PopupHandles::Static(base)) => {
                assert_eq!(base, 0x151_2934);
                // `.bss` is 0x1500000 + 0x12958 on Ver 01.00.07, so the array is inside
                // it — and nowhere near the KTF slot, which is `sl`-relative.
                assert!((0x150_0000..0x150_0000 + 0x1_2958).contains(&base), "must be in .bss");
                assert!(!(0x150_0000..0x150_0000 + 0x1_2958).contains(&0x1b_2df4u32));
            }
            _ => panic!("LGT's popup handle array is a .bss static"),
        }
        assert!(GuestBuild::Skt.popup_handles().is_none());
    }

    /// The commit half of the 퀵슬롯등록 rewrite. The two builds use DIFFERENT mechanisms
    /// and that is the point: KTF calls the game's own `popup_key`, LGT synthesises the
    /// `OK` a finger would produce (probe-verified, MIGRATION_LGT.md §2.16). A build with
    /// `None` here must never OFFER the row — `items::popup_relabel` and
    /// `items::popup_sel_addr` both gate on this, not on `guest_code_addresses_valid`.
    #[test]
    fn popup_commit_is_per_build_and_lgt_needs_no_guest_address() {
        match GuestBuild::Ktf.popup_commit() {
            Some(PopupCommit::GuestCall { fn_addr, select_key }) => {
                assert_eq!(fn_addr, 0x0013_a86c | 1, "Thumb bit must be set for a guest call");
                assert_eq!(select_key, (-5i32) as u32);
            }
            _ => panic!("KTF commits a popup row by calling its own popup_key"),
        }
        assert_eq!(
            GuestBuild::Lgt.popup_commit(),
            Some(PopupCommit::SyntheticOk),
            "LGT must not carry a guest address here — a keypress cannot be stale"
        );
        assert!(GuestBuild::Skt.popup_commit().is_none());
        // A KTF address must never leak into the LGT arm: 0x13a86c is outside LGT `.text`
        // (vaddr 0x1000, size 0x5db85 on Ver 01.00.07) anyway, and this pins that the LGT
        // arm carries no address at all.
        assert!(!matches!(GuestBuild::Lgt.popup_commit(), Some(PopupCommit::GuestCall { .. })));
    }

    /// The inventory page's pre-emptor bytes (51차). The invariant is a COUPLING, not a
    /// pair of numbers: only a build whose commit BYPASSES the game's own key router
    /// (KTF's direct `popup_key` call) may carry any, because that is the only commit
    /// shape the game cannot refuse itself. LGT's synthetic OK rides the router, so its
    /// list must stay empty — the KTF offsets (`0x9c89`, notably `0xa220`, which sits in
    /// the region where the LGT record is documented to shift: `0xa384`→`0xa370`,
    /// `0xa3f8`→`0xa410`) read unrelated LGT fields, and checking them is what kept
    /// every CONSUMABLE popup un-relabelled on the device while skills worked.
    #[test]
    fn popup_preemptors_only_gate_the_build_whose_commit_bypasses_the_router() {
        assert_eq!(GuestBuild::Ktf.popup_preempt_offsets(), &[0x9c89, 0xa220]);
        assert!(GuestBuild::Lgt.popup_preempt_offsets().is_empty(), "LGT's OK rides the game's own router");
        assert!(GuestBuild::Skt.popup_preempt_offsets().is_empty());
        for b in [GuestBuild::Ktf, GuestBuild::Lgt, GuestBuild::Skt] {
            let bypasses_router = matches!(b.popup_commit(), Some(PopupCommit::GuestCall { .. }));
            assert_eq!(
                !b.popup_preempt_offsets().is_empty(),
                bypasses_router,
                "{}: pre-emptor checks belong to exactly the commit that skips the game's own gate",
                b.name()
            );
        }
    }

    #[test]
    fn names_are_the_ones_the_logs_print() {
        assert_eq!(GuestBuild::Ktf.name(), "KTF");
        assert_eq!(GuestBuild::Lgt.name(), "LGT");
        assert_eq!(GuestBuild::Skt.name(), "SKT");
    }

    /// MIGRATION_LGT.md §4-3: the save chain became a per-build table on 2026-08-11
    /// (9차). The LGT addresses come from the save routine at 0x2515e..0x25186, whose
    /// literal pool names all three functions and both record strings in one place.
    #[test]
    fn save_chain_is_per_build_and_carries_each_builds_proof() {
        let ktf = GuestBuild::Ktf.save_chain().unwrap();
        assert_eq!(ktf.fn_serialise, 0x19_60bc | 1);
        assert_eq!(ktf.fn_slot_summary, 0x19_6b90 | 1);
        assert_eq!(ktf.fn_write_record, 0x19_51b0 | 1);
        // KTF scratch: inside the reachability-proven dead range [0x14a32e, 0x14a6aa).
        assert!((0x14_a32e..0x14_a6aa - 3).contains(&ktf.scratch));
        assert_eq!(ktf.slot_off, 0xa3f8);
        assert!(matches!(ktf.names, SaveNames::SlPtrs { game: 0x1b_2d78, slot: 0x1b_2d7c }));

        let lgt = GuestBuild::Lgt.save_chain().unwrap();
        assert_eq!(lgt.fn_serialise, 0x1_ad88 | 1);
        assert_eq!(lgt.fn_slot_summary, 0x1_a6d0 | 1);
        assert_eq!(lgt.fn_write_record, 0x1_a264 | 1);
        // All three must be .text addresses on LGT.
        for f in [lgt.fn_serialise, lgt.fn_slot_summary, lgt.fn_write_record] {
            assert!((0x1000..0x1000 + 0x5db85).contains(&(f & !1)), "{f:#x} must be inside LGT .text");
        }
        // LGT scratch: the page-rounding tail past the declared .bss end — past the
        // maximum literal+imm5 reach (0x15129d0), inside the mapped page (0x1513000).
        assert!((0x151_29d0..0x151_3000 - 3).contains(&lgt.scratch));
        assert_eq!(lgt.scratch % 4, 0);
        // ⚠ The slot offset is the third documented offset that did NOT port
        // (0x7c→0x78, 0xa384→0xa370, 0xa3f8→0xa410) — keep the builds apart.
        assert_eq!(lgt.slot_off, 0xa410);
        assert_ne!(ktf.slot_off, lgt.slot_off);
        // LGT record names are the .text strings themselves.
        assert!(matches!(lgt.names, SaveNames::Direct { game: 0x5_bbb4, slot: 0x5_bbc0 }));

        assert!(GuestBuild::Skt.save_chain().is_none());
    }

    /// LGT `.text` is `0x1000..0x1000+0x5db85` (01.00.07). Every LGT code site in the
    /// table has to land in it — a stray KTF address copied into an LGT arm would
    /// otherwise be written into whatever LGT has at that offset.
    fn in_lgt_text(a: u32) -> bool {
        (0x1000..0x1000 + 0x5db85).contains(&a)
    }

    /// The buff band's three x immediates: all `adds rN,#imm8` (low byte = the imm, so
    /// the table stores the opcode with a ZERO low byte), and the icon is the one that
    /// carries the +2 — patching it with the frame's constant is what tore the band
    /// apart the first time.
    #[test]
    fn band_x_sites_are_add_imm8_with_the_icon_two_across() {
        for b in [GuestBuild::Ktf, GuestBuild::Lgt] {
            let sites = b.band_x_sites();
            assert_eq!(sites.len(), 3, "{} must have frame + icon + number", b.name());
            for &(addr, opcode, delta) in sites {
                assert_eq!(opcode & 0x00ff, 0, "{addr:#x} opcode must carry no immediate");
                assert_eq!(opcode & 0xf800, 0x3000, "{addr:#x} must be a Thumb ADDS/SUBS imm8");
                assert_eq!(opcode & 0x0800, 0, "{addr:#x} must be ADDS, not SUBS");
                assert!(delta <= 2);
            }
            assert_eq!(sites.iter().map(|s| s.2).collect::<Vec<_>>(), vec![0, 2, 0]);
        }
        for &(addr, ..) in GuestBuild::Lgt.band_x_sites() {
            assert!(in_lgt_text(addr), "{addr:#x}");
        }
        assert!(GuestBuild::Skt.band_x_sites().is_empty());
    }

    /// The target box's immediates. The invariant that catches a mistyped constant is
    /// that the PRISTINE halfword and the stated signed base must encode each other:
    /// `items::target_box_x_patches` rewrites the site as `base - shift`, so a base that
    /// does not match the byte actually in the binary would silently move the box by the
    /// wrong amount on every site that disagreed.
    #[test]
    fn target_box_sites_encode_their_own_constants() {
        let enc = |v: i32| -> u16 {
            if v >= 0 {
                0x3100 | v as u16
            } else {
                0x3900 | (-v) as u16
            }
        };
        for b in [GuestBuild::Ktf, GuestBuild::Lgt] {
            let sites = b.target_box_x_sites();
            for &(addr, orig, base) in sites {
                assert_eq!(orig, enc(base), "{} {addr:#x}: halfword and base disagree", b.name());
                // Every site must still encode the maximum shift the app may ask for.
                assert!(
                    (-255..=255).contains(&(base - 211)),
                    "{} {addr:#x}: base {base} cannot encode a 211 px shift",
                    b.name()
                );
            }
            // No address may appear twice: the poke pass writes each one once and a
            // duplicate would mean a site is missing from the list instead.
            let mut addrs: Vec<u32> = sites.iter().map(|s| s.0).collect();
            addrs.sort_unstable();
            let n = addrs.len();
            addrs.dedup();
            assert_eq!(addrs.len(), n, "{} has a duplicated site", b.name());
        }
        // KTF = two byte-identical copies of six. LGT = two copies of four plus the two
        // HP-bar-fill draws its two paths SHARE (both reach 0x27e4).
        assert_eq!(GuestBuild::Ktf.target_box_x_sites().len(), 12);
        assert_eq!(GuestBuild::Lgt.target_box_x_sites().len(), 10);
        // ...and both builds' constants are the same multiset per copy, because it is the
        // same C: -2 frame, +1 bar bg, -44 name, +15 second run, +31/+31 bar fill.
        for b in [GuestBuild::Ktf, GuestBuild::Lgt] {
            let mut c: Vec<i32> = b.target_box_x_sites().iter().map(|s| s.2).collect();
            c.sort_unstable();
            c.dedup();
            assert_eq!(c, vec![-44, -2, 1, 15, 31], "{}", b.name());
        }
        for &(addr, ..) in GuestBuild::Lgt.target_box_x_sites() {
            assert!(in_lgt_text(addr), "{addr:#x}");
        }
        assert!(GuestBuild::Skt.target_box_x_sites().is_empty());
    }

    /// The quickbar page flip: the DATA writes are open on both measured builds, and only
    /// the guest CALL is per-build. Asserting them together is the point — the bug this
    /// replaced had one flag answering both questions, which turned "we have not located
    /// LGT's `skill_bind`" into "the page button does nothing".
    #[test]
    fn quickbar_writes_are_open_where_the_offsets_were_measured() {
        assert!(GuestBuild::Ktf.quickbar_writes_ok());
        assert!(GuestBuild::Lgt.quickbar_writes_ok());
        assert!(!GuestBuild::Skt.quickbar_writes_ok(), "nothing has been measured on SKT");

        assert_eq!(GuestBuild::Ktf.skill_bind_fn(), Some(0x14_5c48 | 1));
        assert_eq!(GuestBuild::Ktf.skill_bind_fn().unwrap() & 1, 1, "Thumb bit or the core enters in ARM state");
        assert_eq!(GuestBuild::Lgt.skill_bind_fn(), None, "unlocated: the raw-write path covers LGT");
        assert_eq!(GuestBuild::Skt.skill_bind_fn(), None);
    }

    /// The game's own "connection lost" modal. Everything asserted here is a relationship
    /// that a mistyped constant would break, because the numbers themselves are only as
    /// good as the disassembly they came out of — and this table's whole reason for
    /// existing is that 47차 shipped three theories the device refuted.
    ///
    /// The load-bearing one is the LAST assert: `state_word` and the four fields around
    /// it are `base + {0xa438, 0xa43c, 0xa7fc, 0xa7fd, 0xa3dc}` off `getBase()` =
    /// `0x1503ab0`, which is also `CHAR + 0x28`. So the modal addresses and the character
    /// anchor have to agree, and a digit dropped from any of them stops agreeing.
    #[test]
    fn net_disconnect_modal_addresses_agree_with_the_char_anchor() {
        let m = GuestBuild::Lgt.net_disconnect_modal().expect("located on LGT");
        let Some(CharAnchor::Static(char_base)) = GuestBuild::Lgt.char_anchor() else {
            panic!("LGT anchor shape changed");
        };
        // `getBase()` (guest 0x1068) returns CHAR + 0x28; every field is base + offset.
        let base = char_base + 0x28;
        assert_eq!(base, 0x150_3ab0, "the game's own accessor returns this");
        assert_eq!(m.state_word, base + 0xa438);
        assert_eq!(m.text_buf, base + 0xa43c);
        assert_eq!(m.panel_flag, base + 0xa7fc);
        assert_eq!(m.sub_flag, base + 0xa7fd);
        assert_eq!(m.clear_word, base + 0xa3dc);
        // The text buffer starts one word past the state word and is the memset width
        // `showModal` uses; a larger cap would run into `0xa53c`+ and a smaller one would
        // truncate messages the game itself fits (the 56-byte idle-timeout string).
        assert_eq!(m.text_buf, m.state_word + 4);
        assert_eq!(m.text_cap, 0x100);
        // Modal 5 is the one whose OK key runs netTeardown and leaves the screen. 7 is
        // the plain dismissible box 47차-C used, and the device showed it goes nowhere;
        // 6 swallows every key forever (`0xbe64` is `movs r0,#1; bx lr`).
        assert_eq!(m.disconnect_state, 5);
        assert_ne!(m.disconnect_state, 7, "type 7 only closes itself — that was 47차-C");
        assert_ne!(m.disconnect_state, 6, "modal 6 eats every key = soft lock");
        assert_ne!(m.disconnect_state, 0, "0 means no modal at all");
        // Every field is inside `.bss` (0x1500000 + 0x12958 on Ver 01.00.07), including
        // the far end of the text buffer.
        let bss = 0x150_0000..0x150_0000 + 0x1_2958;
        for a in [m.state_word, m.text_buf, m.panel_flag, m.sub_flag, m.clear_word] {
            assert!(bss.contains(&a), "{a:#x} must be in .bss");
        }
        assert!(bss.contains(&(m.text_buf + m.text_cap as u32 - 1)), "the buffer must fit in .bss");

        assert!(GuestBuild::Ktf.net_disconnect_modal().is_none(), "not located on KTF (the APK ships LGT)");
        assert!(GuestBuild::Skt.net_disconnect_modal().is_none());
    }

    // ⛔ `net_disconnect_writes_are_show_modal_with_the_state_last` WAS HERE (48차: the
    // host writing modal 5 after a backgrounded socket died). 99차 removed the writes and
    // the text with the online features — 100 % offline, no socket to die. The address
    // test above stays: `state_word` is still the modal-id anchor `items.rs` reads.

    // (…and its two companions, `net_disconnect_text_is_always_nul_terminated` and
    // `net_disconnect_text_is_euckr_the_game_itself_uses`, went with the text constant.)

    /// Auto-loot. The regression this defends is not a wrong number, it is a number that
    /// was only ever right for ONE build while the code asked a blanket question: the
    /// forcing path used to be gated on `guest_code_addresses_valid()` (KTF only) with a
    /// hardcoded `0x2d7`, so on the shipped LGT build it silently did nothing — and the
    /// server had delisted the `오토루팅` cash item *because* the host was supposed to be
    /// doing it. Two features, one flag, and the player got neither.
    ///
    /// ★ 50차 rewrote the answer: it is an ARRAY at `CHAR+0x2d7` indexed by the SAVE SLOT
    /// (`CHAR+0xa410`, the word `SaveChain::slot_off` already names), and the three
    /// "refuted candidates" were elements 0, 1 and 6 of it. The LGT `useItem` arm
    /// `0x5523c` computes `base + *(CHAR+0xa410) + 0x2af` and the reader `0x3ac02`
    /// recomputes the identical address, so both halves are pinned from both sides.
    ///
    /// These asserts encode the three things that each cost a session when they were
    /// wrong: the base is shared with KTF (not one-off per build), the index exists on
    /// LGT and IS the save slot, and the value must have bit 7 because the reader
    /// sign-extends the byte and branches on `< 0`.
    #[test]
    fn autoloot_flag_is_a_slot_indexed_array_with_a_bit7_value() {
        let ktf = GuestBuild::Ktf.autoloot_flag().expect("KTF located");
        let lgt = GuestBuild::Lgt.autoloot_flag().expect("LGT located");
        assert_eq!(GuestBuild::Skt.autoloot_flag(), None, "nothing measured on SKT");

        // Element 0 is the same place on both builds — KTF's documented `CHAR+0x2d7` is
        // simply the slot-0 case, which is why the two ever looked like different answers.
        assert_eq!(ktf.base_off, 0x2d7);
        assert_eq!(lgt.base_off, 0x2d7);
        assert_eq!(lgt.slot_off, Some(0xa410), "the save slot is what picks the element");
        assert_eq!(ktf.slot_off, None, "KTF has only ever been seen on slot 0");

        // ★ The value is a BIT, not a boolean. The LGT reader does
        // `ldrb; lsls #24; asrs #24; cmp #0; bge skip`, so `1` can never arm it.
        assert_eq!(lgt.value, 0x81);
        assert_ne!(lgt.value & 0x80, 0, "bit 7 or the reader's signed test never fires");
        assert_eq!(ktf.value, 0x01, "KTF's own arm stores a plain 1");

        // The slot index must agree with the save chain's — they are the same word, and
        // letting them drift would put the flag in another character's element.
        assert_eq!(lgt.slot_off, Some(GuestBuild::Lgt.save_chain().unwrap().slot_off));

        // Indexing: no slot word yet must NOT silently mean element 0, and an out-of-range
        // word must be refused rather than written somewhere arbitrary in the record.
        assert_eq!(lgt.offset_for(Some(0)), Some(0x2d7));
        assert_eq!(lgt.offset_for(Some(1)), Some(0x2d8), "what 12차 measured on device");
        assert_eq!(lgt.offset_for(Some(6)), Some(0x2dd), "one of the three 'refuted' bytes");
        assert_eq!(lgt.offset_for(Some(AutolootFlag::MAX_SLOT)), Some(0x2d7 + 15));
        assert_eq!(lgt.offset_for(Some(AutolootFlag::MAX_SLOT + 1)), None, "past the array");
        assert_eq!(lgt.offset_for(None), None, "unreadable slot must refuse, not default to 0");
        // KTF is unindexed, so it answers the same however the slot reads.
        assert_eq!(ktf.offset_for(None), Some(0x2d7));
        assert_eq!(ktf.offset_for(Some(9)), Some(0x2d7));

        for b in [GuestBuild::Ktf, GuestBuild::Lgt, GuestBuild::Skt] {
            if let Some(f) = b.autoloot_flag() {
                assert!(b.char_anchor().is_some(), "{b:?}: a flag with no record to apply it to");
                let last = f.offset_for(Some(AutolootFlag::MAX_SLOT)).unwrap_or(f.base_off);
                assert!(last < 0xa800, "{b:?}: {last:#x} is past the end of the record");
                assert_ne!(f.value, 0, "{b:?}: writing 0 would be the OFF state");
            }
        }
    }

    /// 메뉴 터치화 (66차). The table adds exactly FOUR addresses — one cursor byte per
    /// overlay screen whose hit grid is measured — and what matters about them is (a) that
    /// they agree with the record base measured the same day, (b) that they are distinct
    /// (a shared byte would mean one screen's tap moved another's focus), and (c) that a
    /// build nobody measured answers `None` instead of inheriting LGT's numbers.
    ///
    /// ⛔ The other bytes the feature reads (overlay flag, overlay page, popup-open flag,
    /// per-bag INVENTORY cursor) are deliberately NOT here — `items.rs` has owned them
    /// since 17차/44차 and a second copy is how the watcher and the writer drift apart.
    /// If you are about to add them, read the block comment above `items::menu_state`.
    #[test]
    fn the_menu_cursors_are_measured_only_where_they_were_measured() {
        let m = GuestBuild::Lgt.menu_cursors().expect("LGT is the measured build");
        assert_eq!((m.list, m.skill, m.quest, m.system), (0x9c6d, 0x9c7d, 0x9c81, 0x9c82));

        // The probe/device measured these as absolute `.bss` addresses; they have to still
        // agree with the LGT record base, or every read and write lands somewhere else.
        let Some(CharAnchor::Static(base)) = GuestBuild::Lgt.char_anchor() else {
            panic!("LGT's record is a static");
        };
        assert_eq!(base + m.list, 0x150_d6f5, "the byte 66차 poked to move the highlight");
        assert_eq!(base + m.skill, 0x150_d705, "the byte one arrow press moved on 스킬");
        assert_eq!(base + m.quest, 0x150_d709, "…on 퀘스트");
        assert_eq!(base + m.system, 0x150_d70a, "…on SYSTEM");

        // Distinct, and all inside the record like every other anchor here.
        let all = [m.list, m.skill, m.quest, m.system];
        for (i, a) in all.iter().enumerate() {
            assert!(*a < 0xa800, "{a:#x} is past the end of the record");
            for b in &all[i + 1..] {
                assert_ne!(a, b, "two screens cannot share one cursor byte");
            }
        }

        // ★ The INVENTORY cursor array (`items::INV_CURSOR_BASE` = 0x9c72, one byte per
        // bag) runs 0x9c72..0x9c79, so it does NOT collide with any of these four — the
        // bytes it DOES overlap from bag 6 on are 장비's (0x9c77/78/79), which is the
        // game's own layout and is written up in `menutouch::PAGES`.
        for a in all {
            assert!(!(0x9c72..=0x9c79).contains(&a), "{a:#x} is inside the inventory array");
        }

        // ★★ The two cursors 79차-C / 79차-D added. `status` sits BELOW the array; `equip`
        // sits INSIDE it — bags 5·6·7 are 0x9c77·78·79, which are 장비's index/column/row.
        // Asserted as a FACT rather than forbidden, because it is the game's own layout:
        // each screen initialises its own bytes on entry, and both tap paths write only
        // the byte the game writes there, after cross-checking it against the live value.
        // ⛔ Do not "fix" this by adding them to the loop above — that would assert the
        // opposite of what the game does, and the write bound that actually matters is
        // `menutouch::PAGES` against the player's real bag count.
        assert_eq!((m.status, m.equip), (0x9c70, 0x9c77));
        assert!(m.status < 0x9c72, "상태보기's cursor is below the inventory array");
        assert_eq!(
            m.equip,
            0x9c72 + 5,
            "장비's three bytes are bags 5..7 of the inventory cursor array"
        );
        for a in [m.status, m.equip] {
            for b in all {
                assert_ne!(a, b, "{a:#x} collides with a menu-screen cursor");
            }
        }

        // Builds nobody measured must answer None, not LGT's numbers.
        assert_eq!(GuestBuild::Ktf.menu_cursors(), None);
        assert_eq!(GuestBuild::Skt.menu_cursors(), None);
    }

    /// 인게임 오브결합 (2026-08-25): the two guest helpers that rebuild a bag record's
    /// DERIVED fields after the host writes its socket bytes.
    ///
    /// Both come from ONE site and it is the best possible one — the save loader restores
    /// exactly the bytes an in-game combine writes and then calls these two with
    /// `(tab, slot, cat, idx)`. So this test pins the pair against that site.
    ///
    /// ⛔ These are guest CODE addresses, so they are per-build and per-item, and
    /// `guest_code_addresses_valid()` stays false on LGT. A build with no entry turns the
    /// whole feature off — including the popup row that offers it, which is the
    /// fail-closed direction: a build that cannot refresh the cache must not write the
    /// bytes either (`items::orb_popup` asks for this anchor before anything else).
    #[test]
    fn the_item_refresh_helpers_are_per_build_and_thumb() {
        let r = GuestBuild::Lgt.item_refresh().expect("LGT is the measured build");
        // The literal pool entries the save loader reads them from: 0x1a458 = 0x528f1 and
        // 0x1a45c = 0x5283d — i.e. the addresses WITH the Thumb bit, exactly as stored.
        assert_eq!(r.fn_recalc, 0x0005_28f1, "recalcItemStats, from literal 0x1a458");
        // Thumb, like every other guest entry point in this table — `save::LGT_CHAIN`
        // sets the same bit for the same reason.
        assert_eq!(r.fn_recalc & 1, 1, "must carry the Thumb bit");
        // Inside .text (vaddr 0x1000 .. the 01.00.07 image's 0x5db85 end).
        assert!((0x1000..0x5_db85).contains(&(r.fn_recalc & !1)), "outside .text");
        // ⛔ Not the name rebuild at 0x5283c, which the save loader calls next and which
        // this feature deliberately does NOT call (it cannot change a combine's result and
        // its sprintf source is unset for cat > 0x12).
        assert_ne!(r.fn_recalc, 0x0005_283d, "0x5283c is the NAME helper, not the recalc");
        // ★★★ 71차-L — the equip-screen snapshot rebuild, from the literal `0x55058`
        // that `0x55048` loads (`ldr r3,[pc,#8]` then `bl 0x5a6f4`, no arguments).
        assert_eq!(r.fn_equip_snapshot, 0x0004_d859, "rebuildEquipSnapshots, from literal 0x55058");
        assert_eq!(r.fn_equip_snapshot & 1, 1, "must carry the Thumb bit");
        assert!((0x1000..0x5_db85).contains(&(r.fn_equip_snapshot & !1)), "outside .text");
        assert_ne!(r.fn_equip_snapshot, r.fn_recalc, "two different functions");
        // ⛔ NOT the wrapper: `0x55048` compacts the bag first (`0x5380c`), which moves
        // records out from under the host's (tab, slot) + u64-key re-check.
        assert_ne!(r.fn_equip_snapshot, 0x0005_5049, "0x55048 is compact-then-rebuild");
        // ⛔ NOT the character stat rebuild: six args, and it zeroes the derived block.
        assert_ne!(r.fn_equip_snapshot, 0x0004_f015, "0x4f014 must never be called by the host");
        // …and not one of the save chain's entry points: a mixed-up literal would call
        // serialiseSave with (tab, slot, cat, idx).
        let (ser, sum, wr) = crate_save_lgt_chain_addrs();
        for a in [ser, sum, wr] {
            assert_ne!(r.fn_recalc, a);
        }
        // Builds nobody measured answer None, not LGT's numbers.
        assert_eq!(GuestBuild::Ktf.item_refresh(), None, "the KTF equivalents were never located");
        assert_eq!(GuestBuild::Skt.item_refresh(), None);
    }

    /// ★ 150차 — the two guest addresses the host's HP writers call: `ComputeMaxHP` (the timed
    /// max-HP buffs' re-run, 142차-C) and `addHP` (레아의목걸이's 5-second heal). Thumb entry
    /// points inside `.text`, two different functions — swapping them would hand
    /// `ComputeMaxHP` a heal amount as its block pointer — and neither is the wear recompute
    /// `0x4f014`, which the host must never call. The string-table word (the 상세보기 name
    /// box's orb label) is a `.bss` DATA address — no Thumb bit, never a call target — in the
    /// page the save chain's scratch proof already covers.
    #[test]
    fn the_hp_writers_and_the_string_table_are_lgt_only() {
        let max = GuestBuild::Lgt.compute_max_hp_fn().expect("LGT has ComputeMaxHP");
        let add = GuestBuild::Lgt.add_hp_fn().expect("LGT has addHP");
        assert_eq!(max, 0x0004_9001, "ComputeMaxHP 0x49000, Thumb");
        assert_eq!(add, 0x0004_b9c1, "addHP 0x4b9c0 — the `hp_add_lgt` hook's own pc");
        for f in [max, add] {
            assert_eq!(f & 1, 1, "must carry the Thumb bit");
            assert!((0x1000..0x5_db85).contains(&(f & !1)), "outside .text");
            assert_ne!(f, 0x0004_f015, "0x4f014 must never be called by the host");
        }
        assert_ne!(max, add, "two different functions");
        let tbl = GuestBuild::Lgt.string_table_ptr().expect("LGT's string table word");
        assert_eq!(tbl, 0x0151_2928, "the word `getString 0x1bdcc`'s callers load");
        assert_eq!(tbl & 3, 0, "a word-aligned .bss data address");
        assert!((0x0150_0000..0x0160_0000).contains(&tbl), "in .bss, not .text");
        for b in [GuestBuild::Ktf, GuestBuild::Skt] {
            assert_eq!(b.add_hp_fn(), None, "unlocated on {b:?}");
            assert_eq!(b.string_table_ptr(), None, "unlocated on {b:?}");
            assert_eq!(b.compute_max_hp_fn(), None, "unlocated on {b:?}");
        }
    }

    /// ★★★ 72차 — THE «오브 0개» PATCH, AND THE SHAPE THAT vc199 GOT WRONG.
    ///
    /// vc199 shipped a one-halfword version (nop out the `beq` at `0x52fb4`) on the premise
    /// that an empty bag cell holds a count of 0. It does not — nothing clears `+0x3ba` on
    /// removal — so a reused cell became `stale 1 + 1 = 2` and the player saw every first
    /// orb as «2개». The operation is a SELECT, not an increment, and a select does not fit
    /// in two bytes here. These assertions pin the shape so it cannot shrink back.
    #[test]
    fn the_give_item_count_patch_is_a_select_that_rejoins_the_original_code() {
        assert_eq!(GuestBuild::Ktf.give_item_count_patch(), None, "KTF's give-item routine is unlocated");
        assert_eq!(GuestBuild::Skt.give_item_count_patch(), None);
        let (site, orig, patched) = GuestBuild::Lgt.give_item_count_patch().expect("LGT is located");

        // A patch that changes the length of the region it overwrites corrupts everything
        // after it.
        assert_eq!(orig.len(), patched.len(), "the replacement must be the same size");
        assert_eq!(orig.len(), 14, "0x52fb0..0x52fbe — the whole merged-only block's head");
        assert!((0x1000..0x5_db85).contains(&site), "inside .text");
        assert_eq!(site % 2, 0, "Thumb instructions are halfword-aligned");

        // The bytes being replaced really are `ldrb r3,[r4] / cmp r3,#0 / beq …`. If the
        // image ever drifts, the applier refuses rather than writing into the middle of
        // some other instruction — this is the value it compares against.
        assert_eq!(&orig[..6], &[0x23, 0x78, 0x00, 0x2b, 0x0b, 0xd0], "ldrb / cmp / beq");

        // `count = merged ? old + 1 : 1`, and then a branch so control rejoins the code the
        // patch made unreachable. ⛔ Without that branch the fall-through re-enters the old
        // increment and the count goes up twice — which is the vc199 bug, again.
        assert_eq!(
            patched,
            &[0x22, 0x78, 0x59, 0x89, 0x52, 0x42, 0x11, 0x40, 0x01, 0x31, 0x59, 0x81, 0x07, 0xe0],
            "ldrb r2,[r4] / ldrh r1,[r3,#0xa] / rsbs r2,r2,#0 / ands r1,r2 / adds r1,#1 / strh r1,[r3,#0xa] / b"
        );
        let last = u16::from_le_bytes([patched[12], patched[13]]);
        assert_eq!(last & 0xf800, 0xe000, "the last halfword must be an unconditional B");
        // B T2: target = pc + 4 + imm11*2, and it must land on 0x52fce — the instruction
        // after the block this replaces.
        let pc = site as u32 + 12 + 4;
        let imm11 = (last & 0x07ff) as u32;
        assert_eq!(pc + imm11 * 2, 0x0005_2fce, "the branch must rejoin at 0x52fce");
    }

    /// The LGT save chain's three entry points, spelled out here so the collision check
    /// above does not depend on `save.rs` being in scope (this file `include!`s only the
    /// anchor table).
    fn crate_save_lgt_chain_addrs() -> (u32, u32, u32) {
        (0x0001_ad88 | 1, 0x0001_a6d0 | 1, 0x0001_a264 | 1)
    }
}

#[cfg(test)]
mod scene_world_tests {
    use super::*;

    /// The whole table is all-or-nothing, and on KTF it is nothing. Data offsets with
    /// no way to make the game act on them would be a trap: a caller that saw
    /// `map_id_off` would happily read a map id it can never change.
    #[test]
    fn only_lgt_has_a_scene_table() {
        assert!(GuestBuild::Lgt.scene_world().is_some());
        assert!(GuestBuild::Ktf.scene_world().is_none(), "port fn_load_scene first");
        assert!(GuestBuild::Skt.scene_world().is_none());
    }

    /// `loadScene` is called as `ldr r3,=addr|1; bl 0x5a6f4` like every other
    /// cross-module call in this image, so the Thumb bit has to be in the table —
    /// an even address branches to ARM and takes the process down.
    #[test]
    fn load_scene_is_a_thumb_entry_point() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        assert_eq!(s.fn_load_scene & 1, 1, "Thumb bit");
        assert_eq!(s.fn_load_scene & !1, 0x0001_5f28);
    }

    /// ⛔ The literals in the image are base-relative and `getBase()` returns
    /// **CHAR + 0x28**. This is the arithmetic 78차-C got wrong for a whole release,
    /// so it is spelled out rather than trusted: every offset here must be exactly
    /// 0x28 above the literal the disassembly shows.
    #[test]
    fn the_offsets_are_char_relative_not_base_relative() {
        const BASE_IS_CHAR_PLUS: u32 = 0x28;
        let s = GuestBuild::Lgt.scene_world().unwrap();
        // loadScene 0x15f9c: strh of the scene id.
        assert_eq!(s.scene_id_off, 0x9e08 + BASE_IS_CHAR_PLUS);
        // The warp handler 0x3f1d4: r7 = base + 0x9e08, then r7[2] and r7[0xd].
        assert_eq!(s.warp_request_off, 0x9e08 + 2 + BASE_IS_CHAR_PLUS);
        assert_eq!(s.warp_dest_off, 0x9e08 + 0xd + BASE_IS_CHAR_PLUS);
        assert_eq!(s.warp_entry_off, 0x9e08 + 0xc + BASE_IS_CHAR_PLUS);
        // parseScnHeader 0x14284: the literals it adds to getBase().
        assert_eq!(s.map_id_off, 0x9e1c + BASE_IS_CHAR_PLUS);
        assert_eq!(s.spawn_species_off, 0x9e3c + BASE_IS_CHAR_PLUS);
        assert_eq!(s.region_off, 0x9e48 + BASE_IS_CHAR_PLUS);
        // The map loader's creature array, literal 0x9c94 at 0x1590a.
        assert_eq!(s.creatures_off, 0x9c94 + BASE_IS_CHAR_PLUS);
    }

    /// The independent check that the +0x28 above is real: this array is the one KTF
    /// was already documented to keep at CHAR+0x9cbc. The two builds share no code
    /// address at all, so agreeing here is evidence about the offset, not a copy.
    #[test]
    fn the_creature_array_lands_where_ktf_keeps_it() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        assert_eq!(s.creatures_off, 0x9cbc);
        assert_eq!(s.creature_stride, 0x158, "0xac << 1, computed at 0x15912");
    }

    /// Three species, not one — the map's spawn points carry a type of 0/1/2 that
    /// indexes this triple. A caller that treated `spawn_species_off` as a single
    /// word would rewrite one third of a floor's monsters.
    #[test]
    fn the_spawn_table_is_three_words() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        let slots: Vec<u32> = (0..3).map(|i| s.spawn_species_off + i * 4).collect();
        assert_eq!(slots, vec![0x9e64, 0x9e68, 0x9e6c]);
        // …and they must not collide with the map id or the region word, which the
        // same nine-byte header writes.
        assert!(!slots.contains(&s.map_id_off));
        assert!(!slots.contains(&s.region_off));
        assert!(!slots.contains(&s.scene_id_off));
    }

    /// The warp request and its destination live in the SAME small block as the
    /// scene id, at fixed offsets from it — the handler takes `r7 = base + 0x9e08`
    /// once and indexes `r7[2]` / `r7[0xd]`. Asserting the relationship rather than
    /// three loose constants is what keeps a future edit from moving one of them.
    #[test]
    fn the_warp_request_is_a_field_of_the_scene_block() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        assert_eq!(s.warp_request_off, s.scene_id_off + 2);
        assert_eq!(s.warp_dest_off, s.scene_id_off + 0xd);
        // The id is a halfword, so the request byte must sit clear of it.
        assert!(s.warp_request_off >= s.scene_id_off + 2);
        // And neither may collide with the header fields parseScnHeader writes.
        for other in [s.map_id_off, s.region_off, s.spawn_species_off] {
            assert_ne!(s.warp_request_off, other);
            assert_ne!(s.warp_dest_off, other);
        }
    }

    /// The destination is ONE byte. A caller that built a u16 request would write
    /// over `+0xe`, which the handler does not own.
    #[test]
    fn the_warp_destination_is_a_single_byte() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        // 244 scenes ship, so every real destination fits — but assert the bound
        // rather than the count, because the bound is what the byte imposes.
        assert!(u8::try_from(243u32).is_ok());
        assert_eq!(s.warp_dest_off + 1, s.scene_id_off + 0xe);
    }

    /// The two host-owned words in a creature record (2026-09-18) sit in the block the
    /// sentinel A/B proved spare (`+0xf8..+0x107`: survived combat, kills and a respawn
    /// re-init on 80/80 records, frames pixel-identical), are word-aligned, and are the
    /// SAME numbers the guest side uses: `hitMonster`'s patched store is `adds r3,#0xfc`
    /// (`binary_patches.toml`, `0x386fc`) and the `dmg_apply_lgt` hook reads `+0x100`
    /// (`dmgwide::VHP_OFF`). Neither may collide with a field the fork writes.
    #[test]
    fn the_virtual_hp_and_display_words_sit_in_the_measured_spare_block() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        assert_eq!(r.dmg32, 0xfc);
        assert_eq!(r.vhp, 0x100);
        assert_eq!(r.vhp_max, 0x104);
        for w in [r.dmg32, r.vhp, r.vhp_max] {
            assert_eq!(w % 4, 0, "a `ldr`/`str` word");
            assert!((0xf8..=0x104).contains(&w), "inside the sentinel-proven block");
            assert!(w + 4 <= 0x158, "inside the record");
            for used in [r.level, r.hp_max, r.hp_cur, r.hp_spawn, r.respawn_timer, r.hp_regen_permille,
                         r.atk_base, r.atk_eff, r.def_phys_base, r.def_phys_eff, r.def_spec_base,
                         r.def_spec_eff, r.pct_atk_a, r.pct_atk_b, r.pct_def_phys, r.pct_def_spec, r.actor] {
                assert!(used + 4 <= w || used >= w + 4, "{used:#x} overlaps the host word at {w:#x}");
            }
        }
        // `adds r3,#imm8` reaches the display word from the record base; the hook needs no such limit.
        assert!(r.dmg32 <= 0xff);
    }

    /// The scene id and the map id are DIFFERENT words, because 244 scenes share
    /// 135 maps: several scenes name the same map with different monsters. Code that
    /// conflated them would call `loadScene(mapId)` and land somewhere else.
    #[test]
    fn the_scene_id_and_the_map_id_are_separate_words() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        assert_ne!(s.scene_id_off, s.map_id_off);
        assert_eq!(s.scene_id_off, 0x9e30);
        assert_eq!(s.map_id_off, 0x9e44);
    }

    /// `map_spawn_count_off` is inside the MAP OBJECT, not inside CHAR. Mixing the two
    /// spaces is the mistake that made `orbcombine` write into a neighbouring record
    /// (CLAUDE.md's `0x394` warning), so keep it small enough that no reader can
    /// mistake it for a CHAR offset.
    #[test]
    fn the_spawn_count_is_a_map_object_field() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        assert_eq!(s.map_spawn_count_off, 0x34);
        assert!(s.map_spawn_count_off < 0x100, "a MAP field, not a CHAR offset");
        assert!(s.map_spawn_count_off < s.map_id_off);
    }

    /// The entry point is the block field the placement routine reads, so it has to
    /// sit BETWEEN the request and the destination the way the handler indexes them
    /// (`r7[0xc]` and `r7[0xd]`). A table that swapped the two would make every warp
    /// land in the wrong place AND pick the wrong scene.
    #[test]
    fn the_entry_point_is_the_byte_below_the_destination() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        assert_eq!(s.warp_entry_off + 1, s.warp_dest_off);
        assert_eq!(s.warp_entry_off, s.scene_id_off + 0xc);
        assert_ne!(s.warp_entry_off, s.warp_request_off);
    }
}

/// The live monster record - [`CreatureRecord`]. Every offset here was read twice
/// (the painter / combat disassembly, and a probe dump that matched `enemy_dat` for
/// three species at once), and these tests hold the RELATIONS that make the table
/// self-checking, because a wrong offset in this struct writes into a neighbouring
/// monster rather than failing.
mod creature_record_tests {
    use super::*;

    /// The record only exists where the array that holds it does.
    #[test]
    fn the_record_table_travels_with_the_scene_table() {
        assert!(GuestBuild::Lgt.scene_world().is_some());
        assert!(GuestBuild::Ktf.scene_world().is_none());
    }

    /// Everything is inside one stride. An offset past it is a write into the NEXT
    /// monster - the same class of bug as `orbcombine`'s doubled `0x394`.
    #[test]
    fn every_field_is_inside_one_record() {
        let s = GuestBuild::Lgt.scene_world().unwrap();
        let r = s.rec;
        for (name, off) in [
            ("level", r.level),
            ("hp_max", r.hp_max),
            ("hp_cur", r.hp_cur),
            ("hp_spawn", r.hp_spawn),
            ("respawn_timer", r.respawn_timer),
            ("hp_regen", r.hp_regen_permille),
            ("atk_base", r.atk_base),
            ("atk_eff", r.atk_eff),
            ("atk_skill1", r.atk_skill1),
            ("atk_skill2", r.atk_skill2),
            ("def_phys_base", r.def_phys_base),
            ("def_phys_eff", r.def_phys_eff),
            ("def_spec_base", r.def_spec_base),
            ("def_spec_eff", r.def_spec_eff),
            ("pct_atk_a", r.pct_atk_a),
            ("pct_atk_b", r.pct_atk_b),
            ("pct_def_phys", r.pct_def_phys),
            ("pct_def_spec", r.pct_def_spec),
            ("actor", r.actor),
            ("hit", r.hit),
            ("dodge", r.dodge),
            ("crit_res", r.crit_res),
        ] {
            // Two bytes wide at most, so the field plus its width must fit.
            assert!(off + 2 <= s.creature_stride, "{name} at {off:#x} leaves the record");
        }
    }

    /// No two fields may share an address, and the two-byte ones may not overlap.
    /// `hp_max`/`hp_cur` are adjacent halfwords by construction (the HP bar reads
    /// them as one pair), so this is the check that keeps that from becoming three.
    #[test]
    fn the_halfword_fields_do_not_overlap() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        let halfwords = [
            r.hp_max,
            r.hp_cur,
            r.hp_spawn,
            r.respawn_timer,
            r.hp_regen_permille,
            r.atk_base,
            r.atk_eff,
            r.atk_skill1,
            r.atk_skill2,
            r.def_phys_base,
            r.def_phys_eff,
            r.def_spec_base,
            r.def_spec_eff,
            r.pct_atk_a,
            r.pct_atk_b,
            r.pct_def_phys,
            r.pct_def_spec,
            r.hit,
            r.dodge,
            r.crit_res,
        ];
        for (i, a) in halfwords.iter().enumerate() {
            assert_eq!(a % 2, 0, "{a:#x} is a halfword, so it must be aligned");
            for b in &halfwords[i + 1..] {
                assert!(a.abs_diff(*b) >= 2, "{a:#x} and {b:#x} overlap");
            }
        }
        assert_eq!(r.hp_max + 2, r.hp_cur, "the HP bar reads them as one pair");
    }

    /// The three base/effective pairs. The game's own modifier setters recompute the
    /// effective one from the base (`0x38888`, `0x388c4`, `0x38900`, `0x3893c`), so a
    /// host fork that wrote a base and expected the fight to change would do nothing:
    /// the resolver at `0x4bdd4` / `0x4be4c` subtracts the EFFECTIVE field.
    #[test]
    fn each_base_stat_has_a_separate_effective_copy() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        for (base, eff) in [
            (r.atk_base, r.atk_eff),
            (r.def_phys_base, r.def_phys_eff),
            (r.def_spec_base, r.def_spec_eff),
        ] {
            assert_ne!(base, eff);
        }
        // The two groups are laid out DIFFERENTLY, and the difference is measured
        // rather than tidy: attack is `[base][eff]` adjacent (`0x8c`, `0x8e`), while
        // the defences are one four-halfword block whose bases come first —
        // `[phys_base][spec_base][phys_eff][spec_eff]` at 0x68..0x6e. A reader who
        // assumed "eff = base + 2" everywhere would write the special base when it
        // meant the physical effective value.
        assert_eq!(r.atk_base + 2, r.atk_eff);
        assert_eq!(r.def_phys_base + 4, r.def_phys_eff);
        assert_eq!(r.def_spec_base + 4, r.def_spec_eff);
        assert_eq!(r.def_phys_base + 2, r.def_spec_base);
    }

    /// The exact offsets, so a refactor cannot quietly move one. These are the
    /// numbers the probe dump matched against `enemy_dat`.
    #[test]
    fn the_measured_offsets() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        assert_eq!(r.actor, 0x00);
        assert_eq!(r.pct_atk_a, 0x40);
        assert_eq!(r.pct_atk_b, 0x42);
        assert_eq!(r.pct_def_phys, 0x44);
        assert_eq!(r.pct_def_spec, 0x46);
        assert_eq!(r.def_phys_base, 0x68);
        assert_eq!(r.def_spec_base, 0x6a);
        assert_eq!(r.def_phys_eff, 0x6c);
        assert_eq!(r.def_spec_eff, 0x6e);
        assert_eq!(r.level, 0x70);
        assert_eq!(r.hp_max, 0x7c);
        assert_eq!(r.hp_cur, 0x7e);
        assert_eq!(r.hp_spawn, 0x82);
        assert_eq!(r.hp_regen_permille, 0x84);
        assert_eq!(r.atk_base, 0x8c);
        assert_eq!(r.atk_eff, 0x8e);
        // 160차 T4: a boss's three attacks are consecutive halfwords (probe, scene 231:
        // `+0x8c 00 00 | 41 04 | 41 04 | 20 02` = base 0, then 1089 / 1089 / 544).
        assert_eq!(r.atk_skill1, 0x90);
        assert_eq!(r.atk_skill2, 0x92);
    }

    /// The physical and the special defence are read by two arms of the SAME
    /// subtraction, so they must be a pair one halfword apart - and they must not be
    /// the same field, which is the mistake that would make gun damage ignore the
    /// weapon split entirely.
    #[test]
    fn the_two_defences_are_a_pair() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        assert_ne!(r.def_phys_eff, r.def_spec_eff);
        assert_eq!(r.def_phys_eff + 2, r.def_spec_eff);
        assert_eq!(r.def_phys_base + 2, r.def_spec_base);
    }

    /// The level is ONE signed byte, so it must not be treated as a halfword: the
    /// painter reads it with `ldrsb` and the combat code subtracts it from the
    /// player's, so a 2-byte write would clobber `+0x71` for no reason.
    #[test]
    fn the_level_is_a_single_signed_byte() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        // The tower's own range: 1..=70 is `guardian::LEVEL_MAX` (the game's clamp
        // before 111차 raised the PLAYER to 99 — 99 fits i8 too), and the palette tops
        // out at 67, so every level a floor can ask for fits in i8.
        for lv in [1i32, 60, 67, 70, 80, 127] {
            assert!(i8::try_from(lv).is_ok(), "level {lv} must fit one signed byte");
        }
        assert!(i8::try_from(128i32).is_err(), "so 128 is NOT a level the host may write");
        // And it is not adjacent to a halfword field, so a stray 2-byte store here
        // would not corrupt one.
        assert!(r.level + 2 <= r.hp_max);
    }

    /// ⛔ There is no "dead" flag: liveness IS `(i16)hp_cur > 0`. The only writer
    /// that lowers HP clamps a negative result to 0 (`0x38620`), and the AI tick and
    /// the targeting both branch on `<= 0` - so a cleared floor is "no record among
    /// `MAP + map_spawn_count_off` has a positive `hp_cur`".
    ///
    /// ⛔⛔ AND: writing 0 into `hp_cur` does NOT kill the monster. Measured
    /// (`--crea-set 1730:all:7e:0000` on scene 205): all 17 records held 0 for the
    /// next 70 frames, the spawn count stayed 17, and every monster kept moving and
    /// being drawn. The death state (AI state 7, `0x3a58c`) is entered by the damage
    /// path, not by an HP poll. So HP is the right thing to READ for clear detection
    /// and the wrong thing to write for an instant clear.
    #[test]
    fn liveness_is_the_current_hp_and_nothing_else() {
        let r = GuestBuild::Lgt.scene_world().unwrap().rec;
        // A pure restatement of the predicate, so the rule has a home in code.
        let alive = |hp_cur: i16| hp_cur > 0;
        assert!(!alive(0), "0 is dead - the writer clamps there");
        assert!(!alive(-1), "and negative cannot happen, but is not alive either");
        assert!(alive(1));
        // hp_cur is what the predicate reads, not hp_max: a monster at full max HP
        // and zero current HP is dead, and the two offsets are different fields.
        assert_ne!(r.hp_cur, r.hp_max);
    }
}

/// The respawn timer is what makes a tower floor clearable, so its address is pinned
/// on its own rather than only by the "inside one record" sweep.
///
/// ⛔ It sits BELOW the HP block (`0x74` against `0x7c`/`0x7e`/`0x82`) — the game keeps
/// it with the AI state, not with the stats — so a reader who assumes "the HP fields
/// are contiguous and everything else is above them" gets this one wrong.
#[test]
fn the_respawn_timer_sits_just_below_the_hp_block() {
    let r = GuestBuild::Lgt.scene_world().unwrap().rec;
    assert_eq!(r.respawn_timer, 0x74);
    assert!(r.respawn_timer < r.hp_max, "measured at 0x74, below hp_max 0x7c");
    // Two bytes, and they must not run into hp_max.
    assert!(r.respawn_timer + 2 <= r.hp_max);
}
