// Standalone self-test of the bag boxes (오브가방·재료가방·소비가방).
//
//   rustc --edition 2021 --test bagbox_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host, so this compiles the REAL
// sources — not copies. `bagbox.rs` is dependency-free for exactly that reason; the
// names table is the generated file, included beside it and passed in as a slice.

include!("wie_android/src/bagbox.rs");
include!("wie_android/src/bagbox_names.rs");

fn cell(tab: u8, slot: u8, cat: u16, idx: u16, count: u16) -> BagCell {
    BagCell { tab, slot, cat, idx, count }
}

#[test]
fn the_category_assignment_is_the_confirmed_one() {
    assert_eq!(kind_of(0x10), Some(Kind::Orb));
    assert_eq!(kind_of(0x0e), Some(Kind::Material));
    // 125차-B (user): 조합서 0x0f is NOT boxed — it stays in the bag, where the smith's
    // 특수조합 grid reads it. The 재료가방 used to hold it; see `retired_cats`.
    assert_eq!(kind_of(0x0f), None);
    assert_eq!(Kind::Material.retired_cats(), &[0x0f]);
    assert!(Kind::Orb.retired_cats().is_empty() && Kind::Consumable.retired_cats().is_empty());
    assert_eq!(kind_of(0x12), Some(Kind::Consumable));
    assert_eq!(kind_of(0x0d), Some(Kind::Consumable));
    // Equipment and quest items stay in the bag.
    for cat in 0x00..=0x0c {
        assert_eq!(kind_of(cat), None, "cat {cat:#x}");
    }
    assert_eq!(kind_of(0x11), None);
}

#[test]
fn the_masks_name_the_boxes_and_keep_the_three_cells() {
    assert_eq!(accept_mask(&[Kind::Orb]), 1 << 0x10);
    assert_eq!(accept_mask(&Kind::ALL), (1 << 0x10) | (1 << 0x0e) | (1 << 0x12) | (1 << 0x0d));
    // (118차 added 부활의서 0x0b here for a day; 118차-B keeps it boxed and lends one at death)
    assert_eq!(keep_idx_mask(), (1 << 0x16) | (1 << 0x17) | (1 << 0x19));
    assert!(is_bag_item(0x12, 0x16));
    assert!(!is_bag_item(0x12, 0x00));
    assert_eq!(Kind::from_bag_item(0x12, 0x19), Some(Kind::Consumable));
}

// ── 127차: a boxed unit is used through the box's own bag cell ──

#[test]
fn a_staged_record_is_what_give_item_writes_for_one_unit() {
    // 포션 (cat 0x12 idx 0) — the name as the item DB stores it, EUC-KR, NUL-terminated.
    let name = [0xc6u8, 0xf7, 0xbc, 0xc7, 0x00];
    let r = stage_record(&name, 0x12, 0x00);
    assert_eq!(r.len(), STAGE_LEN);
    assert_eq!(&r[..4], &name[..4], "the name is copied");
    assert!(r[4..STAGE_NAME_LEN].iter().all(|b| *b == 0), "into a zeroed buffer — no stale tail");
    assert_eq!(r[0x1e], 0xff, "+0x1e = -1 (`0x52fae strb`)");
    assert_eq!(r[0x1f], 1, "occupied (`0x52f6e`)");
    assert_eq!(r[0x20], 0, "+0x20 = 0 (`0x52f82`)");
    assert_eq!(r[0x21], 0xff, "+0x21 stays the cleared form's -1");
    assert_eq!(u16::from_le_bytes([r[0x22], r[0x23]]), 0x12, "category");
    assert_eq!(u16::from_le_bytes([r[0x24], r[0x25]]), 0x00, "index");
    assert_eq!(u16::from_le_bytes([r[0x26], r[0x27]]), STAGE_COUNT, "ONE unit");
    assert_eq!(STAGE_COUNT, 1, "the tails the restage hook answers are the count-0 ones");
}

#[test]
fn a_staged_name_never_overruns_its_buffer() {
    // A name with no NUL and longer than the buffer keeps its last byte for the NUL.
    let long = [0xb0u8; 0x40];
    let r = stage_record(&long, 0x12, 0x05);
    assert_eq!(r[STAGE_NAME_LEN - 1], 0, "the 30th byte is the terminator");
    assert!(r[..STAGE_NAME_LEN - 1].iter().all(|b| *b == 0xb0));
    assert_eq!(r[0x1f], 1, "the flags behind the name are untouched by the overrun");
    // An empty name is a legal (if nameless) record — the DB name can be missing; the
    // caller refuses that case before it gets here, this only proves nothing overflows.
    let e = stage_record(&[], 0x12, 0x05);
    assert!(e[..STAGE_NAME_LEN].iter().all(|b| *b == 0));
}

#[test]
fn a_snapshot_is_restored_only_when_it_is_the_bag_item_record() {
    let mut snap = [0u8; 0x3c];
    snap[0x1f] = 1;
    snap[0x22..0x24].copy_from_slice(&0x12u16.to_le_bytes());
    snap[0x24..0x26].copy_from_slice(&0x19u16.to_le_bytes());
    assert!(snapshot_is_bag_item(&snap, Kind::Consumable), "the 소비가방 cell");
    assert!(!snapshot_is_bag_item(&snap, Kind::Orb), "…is not the 오브가방's");
    snap[0x24..0x26].copy_from_slice(&0x16u16.to_le_bytes());
    assert!(snapshot_is_bag_item(&snap, Kind::Orb));
    snap[0x1f] = 0;
    assert!(!snapshot_is_bag_item(&snap, Kind::Orb), "an empty record is never written back");
    assert!(!snapshot_is_bag_item(&snap[..0x20], Kind::Orb), "a short read is not a record");
    // A potion staged over the cell must never be mistaken for the cell.
    let potion = stage_record(&[0xc6, 0xf7, 0xbc, 0xc7, 0], 0x12, 0x00);
    assert!(!snapshot_is_bag_item(&potion, Kind::Consumable));
}

#[test]
fn the_resurrection_scroll_is_boxed_and_one_is_lent_at_death() {
    // 118차-B: 부활의서 is an ordinary 소비가방 item — the sweep takes it like any potion…
    assert_eq!(kind_of(REVIVE_SCROLL.0), Some(Kind::Consumable));
    assert!(!is_bag_item(REVIVE_SCROLL.0, REVIVE_SCROLL.1));
    let cells = [BagCell { tab: 0, slot: 3, cat: REVIVE_SCROLL.0, idx: REVIVE_SCROLL.1, count: 2 }];
    let plan = sweep_plan(&Boxes::default(), &cells, &Kind::ALL, false);
    assert_eq!(plan.len(), 1, "swept into the box like every consumable");
    // …and the host lends the game exactly ONE while the player is dying: only when the
    // bag has none and the box has some — never a second one for the plain bag to keep.
    assert_eq!(revive_supply_units(2, 0), 1);
    assert_eq!(revive_supply_units(1, 0), 1);
    assert_eq!(revive_supply_units(0, 0), 0, "nothing to lend");
    assert_eq!(revive_supply_units(2, 1), 0, "the bag already holds one");
    assert_eq!(revive_supply_units(9, 3), 0);
}

#[test]
fn add_merges_below_ninety_nine_first_then_opens_stacks() {
    let mut b = Boxes::default();
    assert_eq!(b.add(Kind::Orb, 0x10, 3, 50), 0);
    assert_eq!(b.add(Kind::Orb, 0x10, 3, 60), 0);
    // 50 + 60: the first stack fills to 99, the rest opens a second one.
    assert_eq!(b.stacks(Kind::Orb), &[Stack { cat: 0x10, idx: 3, count: 99 }, Stack { cat: 0x10, idx: 3, count: 11 }]);
    // A 99 stack is never a merge target (the guest's `<= 98`).
    assert_eq!(b.add(Kind::Orb, 0x10, 3, 1), 0);
    assert_eq!(b.stacks(Kind::Orb)[1].count, 12);
    assert_eq!(b.room(Kind::Orb), (CAP - 2) as u32);
    assert_eq!(b.total(Kind::Orb), 111);
}

/// ★★★ 120차 (user, 2026-09-20: *"인게임 48 도 80 으로 올려야해"*): a box is the size of
/// the whole bag at its largest — 5 tabs x 16 = 80 stacks — counted APART from the bag.
/// 92차..119차 had 48 (three tabs). The warehouse's boxes follow the same rule with their
/// own constant (`offline::WH_BOX_CAP` = 8 bags x 16 = 128), pinned in `offline_selftest`.
#[test]
fn a_box_is_the_size_of_the_whole_bag_at_its_largest() {
    assert_eq!(CAP, 80);
    assert_eq!(CAP, 5 * 16, "five tabs of sixteen cells");
    // And an old 48-stack file loads whole — the cap only ever grew. (Materials, because
    // three consumable idx values are the bag cells themselves and `parse` drops those.)
    let mut b = Boxes::default();
    for i in 0..48u16 {
        assert_eq!(b.add(Kind::Material, 0x0e, i, 99), 0);
    }
    assert_eq!(b.room(Kind::Material), 32);
    assert_eq!(Boxes::parse(&b.to_text()).stacks(Kind::Material).len(), 48);
}

#[test]
fn a_full_box_reports_the_leftover_and_fits_predicts_it() {
    let mut b = Boxes::default();
    for i in 0..CAP as u16 {
        assert_eq!(b.add(Kind::Orb, 0x10, i % 15, 99), 0);
    }
    assert_eq!(b.room(Kind::Orb), 0);
    assert_eq!(b.fits(Kind::Orb, 0x10, 1), 0);
    assert_eq!(b.add(Kind::Orb, 0x10, 1, 5), 5);
    // Spend one stack down and the room comes back exactly there.
    assert!(b.spend(Kind::Orb, 0, 99));
    assert_eq!(b.room(Kind::Orb), 1);
    assert_eq!(b.fits(Kind::Orb, 0x10, 1), 99);
}

/// ★ 165차: the merge room the host publishes per item (`wie_util::stats::BAGBOX_MERGE`) is
/// exactly what `add` fills before it opens a stack — the intake hook spends a free stack
/// only past it, which is how a 99-unit purchase inside one guest call costs one stack
/// instead of one per unit (the shop's 99 `giveItem` calls used to run out of stacks and
/// spill into the bag).
#[test]
fn the_published_merge_room_is_what_add_fills_without_a_new_stack() {
    let mut b = Boxes::default();
    assert_eq!(b.merge_room(Kind::Consumable, 0x12, 0), 0, "a new item has none");
    assert_eq!(b.add(Kind::Consumable, 0x12, 0, 99 + 50), 0);
    assert_eq!(b.merge_room(Kind::Consumable, 0x12, 0), 49, "a full stack takes nothing");
    assert_eq!(b.merge_room(Kind::Consumable, 0x12, 4), 0, "per item");
    // Two partial stacks (a window spend can leave them) both count.
    assert!(b.spend(Kind::Consumable, 0, 1));
    assert_eq!(b.merge_room(Kind::Consumable, 0x12, 0), 1 + 49);
    // Filling exactly that much opens no stack; one unit more does.
    let stacks = b.stacks(Kind::Consumable).len();
    assert_eq!(b.add(Kind::Consumable, 0x12, 0, 50), 0);
    assert_eq!(b.stacks(Kind::Consumable).len(), stacks);
    assert_eq!(b.merge_room(Kind::Consumable, 0x12, 0), 0);
    assert_eq!(b.add(Kind::Consumable, 0x12, 0, 1), 0);
    assert_eq!(b.stacks(Kind::Consumable).len(), stacks + 1);
    assert_eq!(b.merge_room(Kind::Consumable, 0x12, 0), STACK_MAX as u32 - 1);
    // `fits` is the merge room plus a full stack per free cell.
    assert_eq!(b.fits(Kind::Consumable, 0x12, 0), 98 + b.room(Kind::Consumable) * 99);
}

#[test]
fn spend_refuses_what_is_not_there_and_removes_an_emptied_stack() {
    let mut b = Boxes::default();
    b.add(Kind::Orb, 0x10, 2, 3);
    assert!(!b.spend(Kind::Orb, 1, 1), "no such stack");
    assert!(!b.spend(Kind::Orb, 0, 4), "more than it holds");
    assert!(!b.spend(Kind::Orb, 0, 0), "nothing");
    assert!(b.spend(Kind::Orb, 0, 3));
    assert!(b.stacks(Kind::Orb).is_empty());
}

#[test]
fn the_file_round_trips_and_parses_defensively() {
    let mut b = Boxes::default();
    b.add(Kind::Orb, 0x10, 0x0f, 2);
    b.add(Kind::Material, 0x0e, 3, 99);
    b.add(Kind::Material, 0x0e, 3, 1);
    b.add(Kind::Consumable, 0x0d, 1, 7);
    let text = b.to_text();
    assert!(text.starts_with(FILE_HEADER));
    assert_eq!(Boxes::parse(&text), b);
    // Garbage, a wrong-box category, a bag-item cell and a zero count are all dropped;
    // a count over the cap is clamped.
    let junk = "# bagbox v1\nwhat\norb 5 1 1\nmat 16 1 1\nuse 18 22 1\nuse 18 0 0\norb 16 1 500\n";
    let p = Boxes::parse(junk);
    assert_eq!(p.stacks(Kind::Orb), &[Stack { cat: 0x10, idx: 1, count: 99 }]);
    assert!(p.stacks(Kind::Material).is_empty());
    assert!(p.stacks(Kind::Consumable).is_empty());
}

/// ⛔ A file WITHOUT its header is damage, not an empty box (104차-B code review). A
/// publish whose data never reached the flash reads back as zero bytes, and every file this
/// format ever wrote begins with the header — so the host must not arm an empty box over
/// it and then write that emptiness back durably.
#[test]
fn a_file_without_its_header_is_damage_not_an_empty_box() {
    assert_eq!(Boxes::parse_file(""), None, "zero bytes");
    assert_eq!(Boxes::parse_file("orb 16 1 5\n"), None, "stacks without the header");
    assert_eq!(Boxes::parse_file(FILE_HEADER), Some(Boxes::default()), "a legitimately empty box");
    assert_eq!(Boxes::parse_file("# bagbox v1\r\n"), Some(Boxes::default()), "CRLF");
    let mut b = Boxes::default();
    b.add(Kind::Orb, 0x10, 1, 5);
    assert_eq!(Boxes::parse_file(&b.to_text()), Some(b));
}

#[test]
fn the_sweep_takes_whole_cells_only_for_boxes_that_are_on() {
    let mut b = Boxes::default();
    for i in 0..(CAP - 1) as u16 {
        b.add(Kind::Orb, 0x10, 0, 99); // 47 full stacks: one free cell left
        let _ = i;
    }
    let cells = [
        cell(0, 0, 0x10, 5, 99),  // fits the last free cell
        cell(0, 1, 0x10, 6, 2),   // no room after that — left alone
        cell(0, 2, 0x10, 0, 4),   // would need a cell (all 99s) — left alone
        cell(1, 3, 0x0e, 3, 10),  // materials: box not enabled this phase
        cell(1, 4, 0x12, 0x16, 1), // a bag-item cell: never
        cell(1, 5, 0x05, 2, 1),   // equipment: never
        cell(2, 0, 0x10, 9, 0),   // a zero-count orb: taken (and dropped)
    ];
    let plan = sweep_plan(&b, &cells, &[Kind::Orb], true);
    let taken: Vec<(u8, u8)> = plan.iter().map(|s| (s.cell.tab, s.cell.slot)).collect();
    assert_eq!(taken, vec![(0, 0), (2, 0)]);
    assert!(plan.iter().all(|s| s.kind == Kind::Orb));
    // With every box on, the material cell is taken too.
    let plan = sweep_plan(&Boxes::default(), &cells, &Kind::ALL, true);
    let taken: Vec<(u8, u8)> = plan.iter().map(|s| (s.cell.tab, s.cell.slot)).collect();
    assert_eq!(taken, vec![(0, 0), (0, 1), (0, 2), (1, 3), (2, 0)]);
}

#[test]
fn names_come_from_the_generated_table_with_the_games_orb_suffix() {
    let (mark, name, effect) = name_of(NAMES, 0x10, 0x00);
    assert_eq!((mark, name.as_str()), ("", "투신의 오브"));
    assert!(effect.contains("HP"), "{effect}");
    let (_, name, _) = name_of(NAMES, 0x10, 0x0f);
    assert_eq!(name, "소켓확장");
    // The tower material 85차 appended is in the table (cat 0x0e idx 0x2e).
    let (mark, name, _) = name_of(NAMES, 0x0e, 0x2e);
    assert_eq!((mark, name.as_str()), ("{", "닉스코어파편"));
    // An unknown row is labelled, never blank.
    let (_, name, _) = name_of(NAMES, 0x10, 0x3f);
    assert_eq!(name, "0x10/0x3f");
}

#[test]
fn the_open_label_is_euc_kr_and_fits_the_allocation() {
    assert_eq!(LABEL_OPEN, [0xbf, 0xad, 0xb1, 0xe2, 0x00]);
    assert!(LABEL_OPEN.len() <= 10);
    assert_eq!(POPUP_ROWS_OPEN, 1);
    assert_eq!(Kind::Orb.title(), "오브가방");
}

// ── the three cells are pinned to bag 1, cells 1–3 (2026-09-14) ─────────────

/// Apply a plan the way the worker does — one swap at a time, against a live layout —
/// and hand back where everything ended up. This is the only honest check: the worker
/// makes ONE swap per poll and then recomputes, so a plan that is only correct when
/// applied atomically would be wrong in the game.
fn apply(cells: &[BagCell], plan: &[PinSwap]) -> Vec<BagCell> {
    let mut out = cells.to_vec();
    for sw in plan {
        let a = out.iter().position(|c| (c.tab, c.slot) == sw.from);
        let b = out.iter().position(|c| (c.tab, c.slot) == sw.to);
        if let Some(a) = a {
            out[a].tab = sw.to.0;
            out[a].slot = sw.to.1;
        }
        if let Some(b) = b {
            out[b].tab = sw.from.0;
            out[b].slot = sw.from.1;
        }
    }
    out
}

fn seated(cells: &[BagCell]) -> bool {
    Kind::ALL.into_iter().all(|k| {
        let (cat, idx) = k.bag_item();
        match cells.iter().find(|c| c.cat == cat && c.idx == idx) {
            // A cell the bag does not hold is the grant's problem, not the pin's.
            None => true,
            Some(c) => (c.tab, c.slot) == pinned_seat(k),
        }
    })
}

#[test]
fn the_three_cells_are_seated_in_bag_one_cells_one_to_three() {
    assert_eq!(pinned_seat(Kind::Orb), (0, 0));
    assert_eq!(pinned_seat(Kind::Material), (0, 1));
    assert_eq!(pinned_seat(Kind::Consumable), (0, 2));
    // Already seated: nothing to do. ⛔ An idle poll that keeps writing guest records
    // would fight the game's own compaction every 30 frames.
    let done = vec![
        cell(0, 0, 0x12, 0x16, 1),
        cell(0, 1, 0x12, 0x17, 1),
        cell(0, 2, 0x12, 0x19, 1),
        cell(0, 3, 0x04, 0x07, 1),
    ];
    assert!(pin_plan(&done).is_empty());
    assert!(seated(&done));
}

#[test]
fn the_pin_swaps_the_players_gear_out_of_the_seats() {
    // The usual shape on an existing character: equipment first, the three behind it.
    let cells = vec![
        cell(0, 0, 0x00, 0x01, 1),
        cell(0, 1, 0x04, 0x07, 1),
        cell(0, 2, 0x0b, 0x02, 1),
        cell(0, 5, 0x12, 0x16, 1),
        cell(0, 9, 0x12, 0x17, 1),
        cell(1, 3, 0x12, 0x19, 1),
    ];
    let plan = pin_plan(&cells);
    assert_eq!(plan.len(), 3);
    let after = apply(&cells, &plan);
    assert!(seated(&after), "{after:?}");
    // ⛔ Nothing was lost or duplicated: the plan is a permutation of the occupied cells.
    assert_eq!(after.len(), cells.len());
    let mut seats: Vec<(u8, u8)> = after.iter().map(|c| (c.tab, c.slot)).collect();
    seats.sort_unstable();
    seats.dedup();
    assert_eq!(seats.len(), cells.len(), "two cells ended up in one seat");
}

#[test]
fn the_pin_survives_the_cells_being_in_each_others_seats() {
    // The case a one-shot "move into the hole" gets wrong: each cell is sitting where
    // another one belongs, so every swap has to be planned against the layout the
    // previous swap leaves behind.
    let cells = vec![
        cell(0, 0, 0x12, 0x19, 1),
        cell(0, 1, 0x12, 0x16, 1),
        cell(0, 2, 0x12, 0x17, 1),
    ];
    let after = apply(&cells, &pin_plan(&cells));
    assert!(seated(&after), "{after:?}");
}

#[test]
fn the_pin_is_reached_one_swap_at_a_time() {
    // The worker applies `plan[0]` and recomputes next poll. That has to CONVERGE.
    let start = vec![
        cell(0, 4, 0x12, 0x19, 1),
        cell(0, 0, 0x0c, 0x01, 1),
        cell(0, 1, 0x12, 0x16, 1),
        cell(0, 7, 0x12, 0x17, 1),
        cell(0, 2, 0x04, 0x03, 1),
    ];
    let mut cells = start.clone();
    for poll in 0..8 {
        let plan = pin_plan(&cells);
        if plan.is_empty() {
            assert!(seated(&cells), "poll {poll}: the plan is empty but nothing is seated");
            assert!(poll <= Kind::ALL.len(), "took {poll} polls for three cells");
            return;
        }
        cells = apply(&cells, &plan[..1]);
    }
    panic!("the pin never settled: {cells:?}");
}

#[test]
fn a_bag_without_the_cells_asks_for_no_swaps() {
    // A brand-new character has none of the three yet — the GRANT adds them, and it must
    // not be pre-empted by a plan naming a cell that is not there.
    let cells = vec![cell(0, 0, 0x04, 0x07, 1), cell(0, 1, 0x00, 0x02, 1)];
    assert!(pin_plan(&cells).is_empty());
    // One of the three present, the other two not: exactly one swap, for the one present.
    let some = vec![cell(0, 0, 0x04, 0x07, 1), cell(0, 3, 0x12, 0x17, 1)];
    let plan = pin_plan(&some);
    assert_eq!(plan, vec![PinSwap { from: (0, 3), to: pinned_seat(Kind::Material) }]);
}

// ── the absorbed ledger: a stale save must not fill the box twice (100차) ──────

#[test]
fn a_cell_on_the_ledger_is_cleared_without_being_added_again() {
    // The migration sweep moved these two into the box and the game never saved; the
    // player reloads the same record, so the very same cells are back in the bag.
    let stale = [cell(1, 2, 0x10, 5, 1), cell(2, 0, 0x12, 0, 40)];
    let mut b = Boxes::default();
    b.add(Kind::Orb, 0x10, 5, 1);
    b.add(Kind::Consumable, 0x12, 0, 40);
    b.extend_absorbed(stale);
    // A new stack beside them (a hook fallback, say) is genuinely new.
    let fresh = cell(3, 4, 0x0e, 3, 7);
    let cells = [stale[0], stale[1], fresh];
    let plan = sweep_plan(&b, &cells, &Kind::ALL, true);
    let flags: Vec<((u8, u8), bool)> = plan.iter().map(|s| ((s.cell.tab, s.cell.slot), s.already_in_box)).collect();
    assert_eq!(flags, vec![((1, 2), true), ((2, 0), true), ((3, 4), false)]);
    // ⛔ Position AND content: the same orb one cell over, or one more potion, is a
    // different stack and goes in.
    let moved = cell(1, 3, 0x10, 5, 1);
    let grown = cell(2, 0, 0x12, 0, 41);
    let plan = sweep_plan(&b, &[moved, grown], &Kind::ALL, true);
    assert!(plan.iter().all(|s| !s.already_in_box));
    // A ledger cell needs no room: the box can be full and it is still cleared. ★ 106차:
    // it must still HOLD the stack the entry claims, so the filler is that same orb.
    let mut full = Boxes::default();
    for _ in 0..CAP {
        full.add(Kind::Orb, 0x10, 5, 99);
    }
    full.extend_absorbed([stale[0]]);
    let plan = sweep_plan(&full, &[stale[0], cell(0, 5, 0x10, 6, 1)], &[Kind::Orb], true);
    assert_eq!(plan.len(), 1, "the new orb does not fit, the ledger cell is still cleared");
    assert!(plan[0].already_in_box);
}

#[test]
fn folding_a_loan_into_the_box_leaves_the_file_unchanged() {
    // ★★★ 108차 — 제보 2026-09-19: *"특수조합중에 메인메뉴를 몇번갔다오니까 재료가방에
    // 재료가 없어졌어"*.
    //
    // A record LOAD replaces the bag, so an outstanding loan's units are not in it: they
    // belong to the BOX now. The host folds them there on the load edge, and the FILE —
    // which was already `boxes + loan` — must come out identical, or the fold would be a
    // duplicate (or a loss) rather than a bookkeeping move.
    let mut b = Boxes::default();
    b.add(Kind::Material, 0x0e, 3, 40);
    b.add(Kind::Material, 0x0e, 7, 1);
    b.add(Kind::Orb, 0x10, 5, 3);
    let mut loan = Loan::default();
    // A craft window materialised both materials into the bag.
    for (cat, idx, n) in [(0x0e, 3, 40), (0x0e, 7, 1)] {
        let took = b.spend_units(Kind::Material, cat, idx, n);
        assert_eq!(took, n);
        loan.add(Kind::Material, cat, idx, took);
    }
    // ⛔ THE BOX ITSELF now reads empty of materials — this is what the WINDOW shows, and
    // what the next materialise plans from.
    assert!(b.stacks(Kind::Material).is_empty());
    let on_disk = loan.overlay(&b).to_text();

    // The load edge folds the loan back in.
    let folded = loan.overlay(&b);
    let mut after = loan.clone();
    after.clear();
    assert!(after.is_empty());
    assert_eq!(after.overlay(&folded).to_text(), on_disk, "the file content must not move");
    // …and now the window sees them again.
    assert_eq!(folded.units_of(Kind::Material, 0x0e, 3), 40);
    assert_eq!(folded.units_of(Kind::Material, 0x0e, 7), 1);
    assert_eq!(folded.units_of(Kind::Orb, 0x10, 5), 3, "an untouched box is untouched");

    // ⛔ And it is IDEMPOTENT: a second load edge with an empty loan changes nothing.
    assert_eq!(after.overlay(&folded).to_text(), folded.to_text());
}

#[test]
fn what_the_guest_takes_out_of_a_box_is_loaned_until_a_save() {
    // ★★★ 107차 — 제보 2026-09-19: *"특수조합으로 A아이템을 재료로 써서 B아이템을 만들고
    // 저장안하고 나갔다오면 레시피는 사라져있고 A아이템이 남아있어"*.
    //
    // A box item a quest script hands in (a 재료가방 material here — 125차-B took recipes
    // out of the boxes, so the 2026-09-19 recipe was the last of its kind) is spent
    // through the count/take hook, which debits the box directly. What the guest got
    // for it — the crafted item, the quest step — is in UNSAVED memory, so a reload rolls
    // the guest back. The debit must roll back with it, and a loan is what does that.
    let mut b = Boxes::default();
    b.add(Kind::Material, 0x0e, 5, 1); // one material a quest hands in
    b.add(Kind::Material, 0x0e, 3, 40); // and the material it consumes
    let mut loan = Loan::default();

    // The hook already told the guest the recipe is gone; the host applies that.
    let took = b.spend_units(Kind::Material, 0x0e, 5, 1);
    assert_eq!(took, 1);
    loan.add(Kind::Material, 0x0e, 5, took);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 5), 0, "the box itself is debited");

    // ⛔ …but the FILE still shows it, so a reload without a save gets the recipe back.
    let on_disk = loan.overlay(&b);
    assert_eq!(on_disk.units_of(Kind::Material, 0x0e, 5), 1, "no save yet: the material survives the reload");
    assert_eq!(on_disk.units_of(Kind::Material, 0x0e, 3), 40, "the untouched material is unaffected");

    // Once a GameSave commits, the guest's half is real and the debit promotes.
    loan.clear();
    assert_eq!(loan.overlay(&b).units_of(Kind::Material, 0x0e, 5), 0, "saved: the material is really spent");

    // Only what the box actually gave up is owed back — a take of more than it holds
    // loans the shortfall to nobody.
    let mut short = Boxes::default();
    short.add(Kind::Material, 0x0e, 7, 2);
    let mut l2 = Loan::default();
    let got = short.spend_units(Kind::Material, 0x0e, 7, 5);
    assert_eq!(got, 2);
    l2.add(Kind::Material, 0x0e, 7, got);
    assert_eq!(l2.overlay(&short).units_of(Kind::Material, 0x0e, 7), 2, "⛔ never more than was there");
}

#[test]
fn a_ledger_entry_the_box_cannot_back_is_a_new_stack_not_a_deletion() {
    // ★★★ 106차 — 제보 2026-09-19: *"2번째 세이브 슬롯으로 일반조합 한뒤에 실패해서 다시
    // 메인메뉴로 나갔다가 들어왔더니 갑자기 모든 재료가방 내의 아이템이 사라졌어"*.
    //
    // The sweep absorbs a material cell and writes the ledger. A crafting window then
    // materialises it BACK, and `findSlot` returns the cell the sweep just emptied — so
    // the bag holds a cell that matches the ledger entry byte for byte, while the box has
    // been DEBITED for it. The old rule deleted such a cell and added nothing.
    let stale = cell(1, 4, 0x0e, 3, 99);
    let mut b = Boxes::default();
    b.add(Kind::Material, 0x0e, 3, 99);
    b.extend_absorbed([stale]);
    // While the box still holds them the claim is true and the cell is cleared, not added.
    let plan = sweep_plan(&b, &[stale], &Kind::ALL, true);
    assert_eq!(plan.len(), 1);
    assert!(plan[0].already_in_box, "the box holds it, so this is the 100차 stale-save case");

    // Now materialise: the units leave the box for the bag. The ledger entry is unchanged
    // and still matches the cell — but the box can no longer back it.
    let mut debited = b.clone();
    assert_eq!(debited.spend_units(Kind::Material, 0x0e, 3, 99), 99);
    assert!(debited.was_absorbed(&stale), "the entry is still there — that is the trap");
    let plan = sweep_plan(&debited, &[stale], &Kind::ALL, true);
    assert_eq!(plan.len(), 1);
    assert!(!plan[0].already_in_box, "⛔ the box does not hold it: ADD it, never delete it");
    // …and the sweep really puts the materials back.
    let mut after = debited.clone();
    after.add(Kind::Material, 0x0e, 3, 99);
    assert_eq!(after.units_of(Kind::Material, 0x0e, 3), 99);

    // A PARTIAL backing is not enough for a whole-cell claim either — the shortfall would
    // be the part that vanishes.
    let mut half = b.clone();
    half.spend_units(Kind::Material, 0x0e, 3, 1);
    let plan = sweep_plan(&half, &[stale], &Kind::ALL, true);
    assert!(!plan[0].already_in_box, "98 in the box cannot cover a claim of 99");

    // ⛔ And one entry backs ONE cell: two identical cells need two stacks' worth, or the
    // second is a new stack. (The bag cannot really hold two identical cells at the same
    // position, but the ledger is a list and a stale duplicate must not delete twice.)
    let mut two = Boxes::default();
    two.add(Kind::Material, 0x0e, 3, 99);
    two.extend_absorbed([stale]);
    let plan = sweep_plan(&two, &[stale, stale], &Kind::ALL, true);
    assert!(plan[0].already_in_box);
    assert!(!plan[1].already_in_box, "the box only covers the claim once");
}

#[test]
fn the_ledger_rides_in_the_slot_file_and_is_a_union_until_a_commit() {
    let mut b = Boxes::default();
    b.add(Kind::Orb, 0x10, 3, 12);
    b.extend_absorbed([cell(0, 4, 0x10, 3, 12), cell(4, 15, 0x12, 1, 99)]);
    // ⛔ It has to survive a restart — the stale reload that needs it IS a restart.
    let back = Boxes::parse(&b.to_text());
    assert_eq!(back, b);
    assert_eq!(back.absorbed().len(), 2);
    assert!(back.was_absorbed(&cell(4, 15, 0x12, 1, 99)));
    // ★★★ 100차-B: a UNION, not a rewrite. The first 100차 replaced the ledger with "the
    // cells this sweep cleared", so the migration's ten entries vanished the moment a
    // 재료가방 reclaim swept one cell in a session that never saved — and a kill + reload
    // put the whole bag into the box a second time. Every sweep adds; nothing removes.
    b.extend_absorbed([cell(2, 2, 0x0e, 3, 5)]);
    assert_eq!(
        Boxes::parse(&b.to_text()).absorbed(),
        &[cell(0, 4, 0x10, 3, 12), cell(4, 15, 0x12, 1, 99), cell(2, 2, 0x0e, 3, 5)]
    );
    // The same cell swept twice (a reload that re-absorbed it) is on it once.
    b.extend_absorbed([cell(2, 2, 0x0e, 3, 5), cell(0, 4, 0x10, 3, 12)]);
    assert_eq!(b.absorbed().len(), 3);
    // Only a GameSave commit empties it: the disk now holds the swept bag.
    b.clear_absorbed();
    assert!(b.absorbed().is_empty());
    assert!(Boxes::parse(&b.to_text()).absorbed().is_empty());
}

#[test]
fn the_ledger_is_consulted_only_at_the_load_edge() {
    // ★★★ 100차-B (code review): the 재료가방 pushed 제련석 x20 back into the very cell the
    // migration had emptied — same (tab, slot, cat, idx, count) as the ledger entry. That
    // stack is the box's own, already spent by `spend_units`; a reclaim sweep that read
    // the ledger cleared it as "already in the box" (no `add`, no `loan.sub`) and the next
    // SAVE's `loan.clear()` dropped the twenty from the file — a LOSS. Inside a session
    // nothing comes back from disk, so the ledger is only for the first sweep after a
    // load; the host passes `use_ledger = false` for every later one.
    let stale = cell(0, 5, 0x0e, 3, 20);
    let mut b = Boxes::default();
    b.add(Kind::Material, 0x0e, 3, 20);
    b.extend_absorbed([stale]);
    let at_load = sweep_plan(&b, &[stale], &Kind::ALL, true);
    assert_eq!(at_load.len(), 1);
    assert!(at_load[0].already_in_box, "the load-edge sweep dedups the reloaded cell");
    let reclaim = sweep_plan(&b, &[stale], &Kind::ALL, false);
    assert_eq!(reclaim.len(), 1);
    assert!(!reclaim[0].already_in_box, "a later sweep adds the materialised stack back");
    // And without the ledger a cell still needs room like any other. ★ 106차: the filler
    // is the CLAIMED item, because a ledger hit now also requires the box to hold it.
    let mut full = Boxes::default();
    for _ in 0..CAP {
        full.add(Kind::Material, 0x0e, 3, 99);
    }
    full.extend_absorbed([stale]);
    assert!(sweep_plan(&full, &[stale], &Kind::ALL, false).is_empty());
    assert_eq!(sweep_plan(&full, &[stale], &Kind::ALL, true).len(), 1);
}

#[test]
fn a_file_from_before_the_ledger_reads_as_nothing_absorbed() {
    // 92차..98차 files carry `told <box>` lines and no ledger. ⛔ They must read as
    // "nothing absorbed": every cell such a bag still holds is added (a stale reload of a
    // pre-ledger session is the one duplicate this cannot undo, and it is bounded to that
    // one session), and the retired `told` line must not derail the parse.
    let old = format!("{FILE_HEADER}\ntold orb\norb 16 3 12\nuse 18 0 5\n");
    let b = Boxes::parse(&old);
    assert_eq!(b.stacks(Kind::Orb).len(), 1);
    assert_eq!(b.stacks(Kind::Consumable).len(), 1);
    assert!(b.absorbed().is_empty());
    // A malformed or foreign ledger line is skipped, never trusted: a bag-item cell and an
    // equipment cell can never have been absorbed.
    let noise = format!(
        "{FILE_HEADER}\nabsorbed 1 2\nabsorbed x 2 16 3 1\nabsorbed 0 0 18 22 1\nabsorbed 0 1 4 7 1\nabsorbed 0 2 16 3 1\norb 16 3 12\n"
    );
    let b = Boxes::parse(&noise);
    assert_eq!(b.stacks(Kind::Orb).len(), 1);
    assert_eq!(b.absorbed(), &[cell(0, 2, 0x10, 3, 1)]);
}

// ── 3단계 재료가방 (2026-09-15) ──────────────────────────────────────

fn boxed(kind: Kind, stacks: &[(u16, u16, u16)]) -> Boxes {
    let mut b = Boxes::default();
    for (cat, idx, n) in stacks {
        assert_eq!(b.add(kind, *cat, *idx, u32::from(*n)), 0);
    }
    b
}

#[test]
fn the_mirror_totals_one_line_per_item_whatever_the_stacking() {
    // 200 of one material is three stacks (99 + 99 + 2) but ONE mirror line: the quest
    // hooks ask per `(cat, idx)`, never per stack.
    let b = boxed(Kind::Material, &[(0x0e, 3, 200), (0x0e, 4, 5)]);
    assert_eq!(b.stacks(Kind::Material).len(), 4, "99 + 99 + 2, and one more item");
    assert_eq!(b.units_of(Kind::Material, 0x0e, 3), 200);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 9), 0);
    let mut t = b.totals(Kind::Material);
    t.sort();
    assert_eq!(t, vec![((0x0e, 3), 200), ((0x0e, 4), 5)]);
    assert!(b.totals(Kind::Orb).is_empty());
}

#[test]
fn spending_units_empties_the_smallest_stacks_first() {
    let mut b = boxed(Kind::Material, &[(0x0e, 3, 200)]);
    // 99 + 99 + 2 — taking 3 finishes the 2 and dips into a full stack.
    assert_eq!(b.spend_units(Kind::Material, 0x0e, 3, 3), 3);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 3), 197);
    assert_eq!(b.stacks(Kind::Material).len(), 2, "the emptied stack is gone");
    // A short box answers with what it had, and says so by returning less.
    assert_eq!(b.spend_units(Kind::Material, 0x0e, 3, 500), 197);
    assert!(b.stacks(Kind::Material).is_empty());
    assert_eq!(b.spend_units(Kind::Material, 0x0e, 3, 1), 0);
}

#[test]
fn spending_units_leaves_other_items_alone() {
    let mut b = boxed(Kind::Material, &[(0x0e, 3, 10), (0x0e, 4, 10), (0x0e, 0x20, 10)]);
    assert_eq!(b.spend_units(Kind::Material, 0x0e, 4, 10), 10);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 3), 10);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 0x20), 10);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 4), 0);
}

#[test]
fn the_forge_bases_are_the_two_boxed_ones_the_tables_name() {
    // 121차: `smith_dat` / `smithh_dat` / the `i15_dat` tails name exactly two recipe
    // bases that live in a bag box — 포도주 (로얄윈터하츠, 케이's quest) and 바람피리
    // (그리폰의피리). Every other base is equipment, which is never boxed.
    assert_eq!(FORGE_BASES, &[(0x0e, 0x09), (0x12, 0x05)]);
    for (cat, idx) in FORGE_BASES {
        assert!(kind_of(*cat).is_some(), "({cat:#x}, {idx:#x}) is not a box item");
        assert!(!is_bag_item(*cat, *idx));
    }
}

// ── 125차 — tables the smith's `countOf`/`0x555e0` hooks still name (125차-C) ──

#[test]
fn the_recipe_and_smith_needs_tables_are_the_forge_tails() {
    // `i15_dat` has 117 records since 138차 (the 하늘성 recipes over the network rows
    // 0x27..0x58 + 28 appended — `tools_re/add_tower_recipes.py`, count byte 0x5e483 = 0x75),
    // every one listed.
    assert_eq!(RECIPE_NEEDS.len(), 0x75);
    // ★ 138차: each tower recipe needs 닉스코어의 파편 (0x0e, 0x2e) and nothing else from a
    // box — its base is equipment (or none), and there is no second material (결정 ⑱).
    for (idx, needs) in &RECIPE_NEEDS[0x27..] {
        assert_eq!(*needs, &[(0x0e, 0x2e)][..], "tower recipe {idx:#x}");
    }
    for (i, (idx, needs)) in RECIPE_NEEDS.iter().enumerate() {
        assert_eq!(usize::from(*idx), i, "recipe rows are in idx order");
        for &(cat, idx) in *needs {
            assert!(kind_of(cat).is_some(), "recipe {i}: ({cat:#x}, {idx:#x}) is not a boxed category");
            assert!(idx != 0xff);
        }
        assert!(needs.len() <= 3);
    }
    // 그리폰의피리 (i15 record 18 = the recipe item idx 0x12, 121차): mat1 idx 2, mat2 idx 1,
    // base 바람피리 (0x0e, 0x09) — the tail `0e 09 02 03 01 03 64 12 09`.
    assert_eq!(RECIPE_NEEDS[0x12], (0x12, &[(0x0e, 0x02), (0x0e, 0x01), (0x0e, 0x09)][..]));
    // An equipment base is not a need (never boxed): record 0's base is 투구 (0x00, 0x00).
    assert_eq!(RECIPE_NEEDS[0], (0x00, &[(0x0e, 0x00)][..]));
    // 16 smiths x 5 rows. The first smith's fifth row is 로얄윈터하츠 with 포도주 as its base.
    assert_eq!(SMITH_NEEDS.len(), 16);
    assert!(SMITH_NEEDS[0].contains(&(0x12, 0x05)));
    for (i, needs) in SMITH_NEEDS.iter().enumerate() {
        assert!(!needs.is_empty(), "smith {}", i + 1);
        for &(cat, _) in *needs {
            assert!(kind_of(cat).is_some(), "smith {}: cat {cat:#x}", i + 1);
        }
        // No pair twice — the plan ranks by first mention.
        for (a, p) in needs.iter().enumerate() {
            assert!(!needs[..a].contains(p), "smith {}: {p:?} twice", i + 1);
        }
    }
    // And every boxed base the bases table knows is some recipe's or smith's need.
    for base in FORGE_BASES {
        let known = RECIPE_NEEDS.iter().any(|(_, n)| n.contains(base)) || SMITH_NEEDS.iter().any(|n| n.contains(base));
        assert!(known, "{base:?}");
    }
}

#[test]
fn a_recipe_an_old_file_keeps_in_the_material_box_is_stranded_not_lost() {
    // 125차-B. A file written before the change has `mat 15 18 1` (그리폰의피리 레시피)
    // under the 재료가방's tag. It is READ (dropping it would be a loss), reported as
    // stranded for the host to move back to the bag, written back unchanged until then,
    // until the host evicts it (125차-C no longer plans a craft window).
    let text = format!("{FILE_HEADER}\nmat 15 18 1\nmat 14 3 5\nuse 18 0 9\n");
    let b = Boxes::parse(&text);
    assert_eq!(b.units_of(Kind::Material, 0x0f, 0x12), 1, "the old recipe line is kept");
    assert_eq!(b.units_of(Kind::Material, 0x0e, 0x03), 5);
    let stranded: Vec<(Kind, u16, u16, u16)> = b.stranded().iter().map(|(k, s)| (*k, s.cat, s.idx, s.count)).collect();
    assert_eq!(stranded, vec![(Kind::Material, 0x0f, 0x12, 1)]);
    assert_eq!(Boxes::parse(&b.to_text()).stranded(), b.stranded(), "and it survives a round trip");
    assert_eq!(b.units_of(Kind::Material, 0x0f, 0x12), 1);
    // A recipe under any OTHER box's tag is still dropped — 0x0f was never theirs.
    let odd = Boxes::parse(&format!("{FILE_HEADER}\norb 15 18 1\nuse 15 18 1\n"));
    assert!(odd.stranded().is_empty());
    assert_eq!(odd.units_of(Kind::Orb, 0x0f, 0x12), 0);
    assert_eq!(odd.units_of(Kind::Consumable, 0x0f, 0x12), 0);
    // The sweep leaves a recipe CELL alone — it is a bag item now, like equipment.
    let cells = vec![cell(0, 3, 0x0f, 0x12, 1), cell(0, 4, 0x0e, 0x03, 5)];
    let sweep = sweep_plan(&Boxes::default(), &cells, &Kind::ALL, false);
    assert_eq!(sweep.len(), 1);
    assert_eq!((sweep[0].cell.cat, sweep[0].kind), (0x0e, Kind::Material));
    // And the file's absorbed ledger drops a recipe cell it may still name.
    let led = Boxes::parse(&format!("{FILE_HEADER}\nabsorbed 0 3 15 18 1\nabsorbed 0 4 14 3 5\n"));
    assert_eq!(led.absorbed().len(), 1);
    assert_eq!(led.absorbed()[0].cat, 0x0e);
}

#[test]
fn the_drop_box_verbs_are_per_box_and_fit_the_widget() {
    use Screen::Inventory;
    // ⛔ The 오브가방 has none IN THE INVENTORY: its row IS the combine, and the target
    // picker follows.
    assert!(verbs(Kind::Orb, Inventory).is_empty());
    assert_eq!(verbs(Kind::Consumable, Inventory), &[Verb::Use, Verb::Bind, Verb::Drop]);
    // 재료: nothing to "use", and a plate cannot hold one. ⛔ 꺼내기 was removed 2026-09-16
    // (the game has no such verb) — 125차-C the smith counts and spends the box itself.
    assert_eq!(verbs(Kind::Material, Inventory), &[Verb::Drop]);
    for k in Kind::ALL {
        for screen in [Screen::Inventory, Screen::OnlineBag, Screen::OnlineWarehouse] {
            // `overlay::ORB_DROP_MAX` is 4 — the box clamps past it, which would make a
            // verb unreachable rather than merely unseen.
            assert!(verbs(k, screen).len() <= 4, "{k:?} on {screen:?} would not fit the drop box");
            for v in verbs(k, screen) {
                assert!(!v.label().is_empty());
            }
        }
    }
    // 버리기 is the only one with a confirmation, and 예 is its first row.
    assert_eq!(CONFIRM_ROWS.len(), 2);
    assert_eq!(CONFIRM_ROWS[CONFIRM_YES], "예");
}

#[test]
fn every_verb_label_is_distinct_within_a_box() {
    for k in Kind::ALL {
        for screen in [Screen::Inventory, Screen::OnlineBag, Screen::OnlineWarehouse] {
            let mut seen: Vec<&str> = Vec::new();
            for v in verbs(k, screen) {
                assert!(!seen.contains(&v.label()), "{k:?}/{screen:?} repeats {}", v.label());
                seen.push(v.label());
            }
        }
    }
}

// ── the game's own per-item popup rule (115차, 2026-09-19) ────────────────────

#[test]
fn the_drop_box_follows_the_games_popup_builder_per_item() {
    use Screen::Inventory;
    let v = |idx: u16| verbs_for(Kind::Consumable, Inventory, 0x12, idx);
    // 부활의서: the builder's `cmp #0xb` → 상세보기 · 버리기 only. No 사용하기, no 퀵슬롯등록.
    assert_eq!(v(0x0b), vec![Verb::Drop]);
    // ordinary consumables keep all three — 귀환서 / 그리폰의피리 included, 비밀의상자 (0x0a:
    // the builder's test is `> 0x0a`, so it stays on the eight-row side) and 컵라면 (0x0c),
    // the one cash-tier row the builder lets through to the eight-row popup.
    for idx in [0x00, 0x04, 0x08, 0x09, 0x0a, 0x0c] {
        assert_eq!(v(idx), vec![Verb::Use, Verb::Bind, Verb::Drop], "idx {idx:#x}");
    }
    // 오브원석 and the cash tier past 0x0a: the three-row popup has no N키등록 → no 퀵슬롯등록
    for idx in [0x07, 0x0b + 2, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x15, 0x18, 0x1a] {
        assert_eq!(v(idx), vec![Verb::Use, Verb::Drop], "idx {idx:#x}");
    }
    // cat 0x0d shares the 소비가방 but every category but 0x12 gets the two-row popup
    assert_eq!(verbs_for(Kind::Consumable, Inventory, 0x0d, 0x00), vec![Verb::Drop]);
    // the per-item rule only ever REMOVES: each list is the box's list in the box's order
    for idx in 0..0x1b_u16 {
        let mut base = verbs(Kind::Consumable, Inventory).iter();
        for want in v(idx) {
            assert!(base.any(|b| *b == want), "idx {idx:#x}: {want:?} out of order or added");
        }
    }
    // the other screens have neither verb, so the rule changes nothing there
    for screen in [Screen::OnlineBag, Screen::OnlineWarehouse, Screen::Shop] {
        assert_eq!(verbs_for(Kind::Consumable, screen, 0x12, 0x0b), verbs(Kind::Consumable, screen).to_vec(), "{screen:?}");
    }
}

#[test]
fn only_the_two_return_scrolls_raise_the_games_own_dialog_on_use() {
    assert!(use_opens_window(0x12, 0x08), "귀환서");
    assert!(use_opens_window(0x12, 0x09), "그리폰의피리");
    for idx in (0..0x1b_u16).filter(|i| *i != 0x08 && *i != 0x09) {
        assert!(!use_opens_window(0x12, idx), "idx {idx:#x}");
    }
    assert!(!use_opens_window(0x0d, 0x08), "another category with the same index");
    // and both still get 사용하기 — the job simply follows the game's dialog
    assert!(game_offers_use(0x12, 0x08) && game_offers_use(0x12, 0x09));
    // 117차-D: only 귀환서's dialog is a framed message box the box window can show through
    // itself; 그리폰의피리's is a town list, for which the window steps aside instead.
    assert!(use_dialog_over_window(0x12, 0x08), "귀환서");
    assert!(!use_dialog_over_window(0x12, 0x09), "그리폰의피리 is a list");
    for idx in (0..0x1b_u16).filter(|i| *i != 0x08) {
        assert!(!use_dialog_over_window(0x12, idx), "idx {idx:#x}");
    }
    assert!(!use_dialog_over_window(0x0d, 0x08));
    // …and whichever shape, both are still the game's dialog, not the host's
    assert!(use_opens_window(0x12, 0x08) && use_opens_window(0x12, 0x09));
}

#[test]
fn only_the_potion_arms_are_run_through_a_skill_cast() {
    // ★ 164차: `useItem`'s table sends idx 0..3 to the HP arm and 4..6 to the SP arm —
    //   add, then the consume tail. Those, and nothing else, are run by the host while
    //   the game is dropping keys for a cast.
    for idx in 0..=0x06_u16 {
        assert!(use_runs_through_a_cast(0x12, idx), "potion idx {idx:#x}");
        assert!(!use_opens_window(0x12, idx), "a potion never raises a window");
    }
    // 오브원석 (7), the two return scrolls (8, 9), 비밀의상자 (0xa), 부활의서 (0xb) and every
    // cash row keep the game's own refusal.
    for idx in 0x07..0x1b_u16 {
        assert!(!use_runs_through_a_cast(0x12, idx), "idx {idx:#x}");
    }
    assert!(!use_runs_through_a_cast(0x0d, 0x00), "a 전투주문서 is not a potion");
    assert!(!use_runs_through_a_cast(0x0e, 0x00), "a material is not a potion");
}

// ── the two SETS of boxes on the online screens (96차, 2026-09-15) ──────────

#[test]
fn the_online_screens_give_every_box_the_games_own_three_verbs() {
    // ★ User: *"그 각각 가방 창에서 네트워크 창고로써의 드랍박스인 상세보기, 버리기,
    // 내리기(싱글에서는 상세보기 버리기, 올리기) 선택지가 나와야"*. Which of 올리기 /
    // 내리기 is the last one is the SIDE of the screen, not the kind of box.
    for k in Kind::ALL {
        assert_eq!(verbs(k, Screen::OnlineBag), &[Verb::Detail, Verb::Drop, Verb::Deposit], "{k:?}");
        assert_eq!(verbs(k, Screen::OnlineWarehouse), &[Verb::Detail, Verb::Drop, Verb::Withdraw], "{k:?}");
    }
    // ⛔ And the 오브가방's empty list is the INVENTORY's alone: an online orb row that
    // answered with nothing at all would be a dead row (the combine picker writes bag
    // records the inventory page owns).
    assert!(verbs(Kind::Orb, Screen::Inventory).is_empty());
    assert!(!verbs(Kind::Orb, Screen::OnlineBag).is_empty());
    assert!(!verbs(Kind::Orb, Screen::OnlineWarehouse).is_empty());
}

#[test]
fn only_the_warehouse_screen_reads_the_store() {
    // The one predicate `bagbox_host::listing` branches on: the bag's own boxes, or the
    // view of `offline::Store::items`.
    assert!(Screen::OnlineWarehouse.is_warehouse());
    assert!(!Screen::OnlineBag.is_warehouse());
    assert!(!Screen::Inventory.is_warehouse());
}

#[test]
fn a_box_opens_from_the_shell_only_on_the_online_screen_itself() {
    // ★★★ 2026-09-16 — 제보: 창고에서 오브가방을 한 번 연 뒤 슬롯 목록으로 돌아오면 슬롯을
    // 고르는 OK 마다 상자가 다시 열리고, 게임시작 슬롯 목록에서도 게임이 시작되지 않았다.
    // App state 3 (`shell`) covers the title, both slot lists and the creation screens;
    // the panel byte and the cell cursor are STALE there. Only the shell's own screen
    // word (`items::SHELL_SCREEN_ONLINE`) says the grids are on screen.
    for panel in [Some(ONLINE_PANEL_BAG), Some(ONLINE_PANEL_WAREHOUSE), Some(2), None] {
        assert_eq!(Screen::from_shell(true, false, panel, None), None, "stale panel {panel:?} off the online screen");
    }
    // On the online screen the panel decides the SET.
    assert_eq!(Screen::from_shell(true, true, Some(ONLINE_PANEL_BAG), None), Some(Screen::OnlineBag));
    assert_eq!(Screen::from_shell(true, true, Some(ONLINE_PANEL_WAREHOUSE), None), Some(Screen::OnlineWarehouse));
    // The 캐시샵 and an unreadable byte: fail closed, the key goes to the game.
    assert_eq!(Screen::from_shell(true, true, Some(2), None), None);
    assert_eq!(Screen::from_shell(true, true, None, None), None);
    // ⛔ And the SHELL never becomes the shop, whatever window id is stale there — the
    // shop is a window in the WORLD, and app state 3 has none of its own.
    assert_eq!(Screen::from_shell(true, true, Some(ONLINE_PANEL_BAG), Some(SHOP_WINDOW)), Some(Screen::OnlineBag));
    assert_eq!(Screen::from_shell(true, false, Some(ONLINE_PANEL_BAG), Some(SHOP_WINDOW)), None);
    // In the world the shell facts are not consulted at all — the inventory-page gate
    // (98차 §4) is the caller's, and it comes after this.
    for online in [false, true] {
        for panel in [Some(ONLINE_PANEL_BAG), Some(ONLINE_PANEL_WAREHOUSE), Some(2), None] {
            assert_eq!(Screen::from_shell(false, online, panel, None), Some(Screen::Inventory));
        }
    }
}

#[test]
fn the_shop_window_is_its_own_screen_in_the_world() {
    // ★★★ 105차 — 사용자: *"아이템 판매시에 3종가방에 있는걸 열수가없는데 … 판매시에
    // 3종가방 선택하면 열려서 판매 가능하게 해줘"*. In the world the window id is what
    // separates the shop's sell grid from the inventory page.
    for online in [false, true] {
        for panel in [Some(ONLINE_PANEL_BAG), Some(ONLINE_PANEL_WAREHOUSE), Some(2), None] {
            assert_eq!(Screen::from_shell(false, online, panel, Some(SHOP_WINDOW)), Some(Screen::Shop));
        }
    }
    // Every OTHER window in the world is still the inventory, including the smith's
    // window 13 (125차-C counts the box; no materialise) and an unreadable id.
    // ⛔ Fail toward the inventory, whose caller applies the page gate.
    for w in [None, Some(0u8), Some(3), Some(13), Some(15), Some(17), Some(255)] {
        assert_eq!(Screen::from_shell(false, false, None, w), Some(Screen::Inventory), "window {w:?}");
    }
    // The shop shows the BAG's boxes, not the warehouse's.
    assert!(!Screen::Shop.is_warehouse());
}

#[test]
fn the_shop_sells_and_never_throws_away() {
    // The shop's drop box is 상세보기 · 판매 for all three boxes. ⛔ NO 버리기: a
    // destructive verb next to 판매 on an 80-row list is a mis-tap that cannot be undone.
    for kind in [Kind::Orb, Kind::Material, Kind::Consumable] {
        let v = verbs(kind, Screen::Shop);
        assert_eq!(v, &[Verb::Detail, Verb::Sell], "{kind:?}");
        assert!(!v.contains(&Verb::Drop), "{kind:?} must not be able to throw things away at the shop");
    }
    // ★ The 오브가방 is EMPTY in the inventory (its row opens the combine picker) but not
    // here — at the shop an orb is a thing to sell.
    assert!(verbs(Kind::Orb, Screen::Inventory).is_empty());
    assert!(!verbs(Kind::Orb, Screen::Shop).is_empty());
    // 판매 appears on no other screen.
    for s in [Screen::Inventory, Screen::OnlineBag, Screen::OnlineWarehouse] {
        for kind in [Kind::Orb, Kind::Material, Kind::Consumable] {
            assert!(!verbs(kind, s).contains(&Verb::Sell), "{kind:?} on {s:?}");
        }
    }
}

#[test]
fn the_sell_price_is_the_games_own_quarter_and_zero_is_refused() {
    // ★★★ 판매가 = `price >> 2`, and the shop refuses a row priced 0 (`0x1e364`).
    let rows = NAMES;
    // The three bag-item cells themselves are priced 0 on purpose (72차) so the game
    // refuses 버리기 / 올리기 on them — so they must be unsellable here too, or the host
    // would pay gold for a container the game will not buy.
    for kind in [Kind::Orb, Kind::Material, Kind::Consumable] {
        let (cat, idx) = kind.bag_item();
        assert_eq!(sell_price(rows, cat, idx), 0, "{kind:?} opener must not be sellable");
        assert_eq!(sell_refusal(rows, cat, idx), Some(REFUSE_SELL), "{kind:?} opener");
    }
    // A row the table does not know has no price, so it is refused — FAIL CLOSED.
    assert_eq!(sell_price(rows, 0xff, 0xff), 0);
    assert_eq!(sell_refusal(rows, 0xff, 0xff), Some(REFUSE_SELL));
    // And where there IS a price, the quarter is exactly what the shop pays.
    let mut priced = 0usize;
    for r in rows {
        let p = price_of(rows, r.0, r.1).unwrap_or(0);
        assert_eq!(sell_price(rows, r.0, r.1), p >> 2, "cat {:#x} idx {:#x}", r.0, r.1);
        if p >= 4 {
            priced += 1;
            assert_eq!(sell_refusal(rows, r.0, r.1), None, "cat {:#x} idx {:#x} is sellable", r.0, r.1);
        }
    }
    assert!(priced > 0, "the table has to hold at least one sellable row");
}

#[test]
fn the_deposit_refusal_is_the_clients_own_rule() {
    // ★ Disassembled 2026-09-15 (LGT `0x6f4c`..`0x6fe6`) — the gates the game's own 올리기
    // arm applies, which a host 올리기 never passes through and so has to apply itself.
    //
    // 1. price low byte 0 → 「해당 아이템은 올릴 수 없습니다.」. 그리폰의피리 (cat 0x12
    //    idx 0x09) is priced 0 in the shipped DB and really is undepositable in game.
    assert_eq!(deposit_refusal(NAMES, 0x12, 0x09), Some(REFUSE_DEPOSIT_PRICE));
    // …and an ordinary priced item goes through.
    assert_eq!(deposit_refusal(NAMES, 0x0e, 0x03), None, "제련석 has a price and deposits");
    assert_eq!(deposit_refusal(NAMES, 0x10, 0x00), None, "an orb deposits");
    // 2. cat 0x12 idx 0x0b..=0x10 answer 1 WITHOUT a price (`0x6dc6`: `idx - 0xb <= 5`).
    for idx in DEPOSIT_FREE_CONSUMABLES {
        assert_eq!(deposit_refusal(NAMES, 0x12, idx), None, "cat 0x12 idx {idx:#x} is price-exempt");
    }
    // 3. cat 0x0e && idx > 0x1d — the network-only material ladder.
    assert_eq!(deposit_refusal(NAMES, 0x0e, MATERIAL_DEPOSIT_IDX_MAX), None);
    assert_eq!(deposit_refusal(NAMES, 0x0e, MATERIAL_DEPOSIT_IDX_MAX + 1), Some(REFUSE_DEPOSIT_MATERIAL));
    // ★★ 닉스코어의 파편 (85차, cat 0x0e idx 0x2e) is one of those — the tower's own
    // material cannot be deposited, exactly as the game would refuse it.
    assert_eq!(deposit_refusal(NAMES, 0x0e, 0x2e), Some(REFUSE_DEPOSIT_MATERIAL));
    // 4·5. recipes and quest items.
    assert_eq!(deposit_refusal(NAMES, 0x0f, 0x00), Some(REFUSE_DEPOSIT_RECIPE));
    assert_eq!(deposit_refusal(NAMES, 0x11, 0x00), Some(REFUSE_DEPOSIT_QUEST));
    // ⛔ FAIL CLOSED: a row the table does not know has no price, and the game reads one.
    assert_eq!(deposit_refusal(NAMES, 0x10, 0xff), Some(REFUSE_DEPOSIT_PRICE));
}

#[test]
fn the_three_bag_item_cells_can_never_be_deposited() {
    // ⛔ They are priced 0 for exactly this reason (72차: the sell-price cache gates
    // 버리기 / 올리기 / 판매 alike), so the rule above already refuses them — but the
    // three are the one thing whose loss would take a whole box with it.
    for k in Kind::ALL {
        let (cat, idx) = k.bag_item();
        assert_eq!(deposit_refusal(NAMES, cat, idx), Some(REFUSE_DEPOSIT_PRICE), "{k:?}");
    }
}

#[test]
fn the_names_table_carries_a_price_for_every_row_it_names() {
    // The price column is what `deposit_refusal` reads; a row without one would be
    // refused by the fail-closed arm rather than by the game's actual rule.
    for (cat, idx, _, name, _, price) in NAMES {
        assert_eq!(price_of(NAMES, *cat, *idx), Some(*price), "{name}");
    }
    assert_eq!(price_of(NAMES, 0x10, 0xff), None, "a row that does not exist has no price");
}

// ── the 재료가방 crash-durability loan (2026-09-15) ───────────────────────────────

#[test]
fn a_fresh_loan_is_empty_and_overlays_to_the_box_unchanged() {
    let mut boxes = Boxes::default();
    boxes.add(Kind::Material, 0x0e, 3, 40);
    let loan = Loan::default();
    assert!(loan.is_empty());
    assert_eq!(loan.overlay(&boxes), boxes, "no loan = the file is just the box");
}

#[test]
fn the_file_keeps_debited_materials_until_a_save_confirms() {
    // The scenario: a 제련 window opens, 40 stones are pushed into the bag and the box is
    // debited to 0. On disk the box must still show 40 (recoverable on a crash), because
    // the bag holding them is not yet saved.
    let mut boxes = Boxes::default();
    boxes.add(Kind::Material, 0x0e, 3, 40);
    let orig = boxes.clone();
    let mut loan = Loan::default();

    // materialise: box -> bag
    let landed = boxes.spend_units(Kind::Material, 0x0e, 3, 40);
    assert_eq!(landed, 40);
    loan.add(Kind::Material, 0x0e, 3, 40);
    assert_eq!(boxes.units_of(Kind::Material, 0x0e, 3), 0, "the live box is debited");
    assert_eq!(loan.overlay(&boxes), orig, "but the FILE still holds all 40");

    // the player crafts with 30, closes the window; the sweep reclaims the 10 leftover.
    boxes.add(Kind::Material, 0x0e, 3, 10);
    loan.sub(Kind::Material, 0x0e, 3, 10);
    assert_eq!(boxes.units_of(Kind::Material, 0x0e, 3), 10, "10 back in the live box");
    assert_eq!(loan.overlay(&boxes).units_of(Kind::Material, 0x0e, 3), 40, "file still shows 40 (the 30 are not durable yet)");

    // a GameSave lands: the bag (minus the 30, plus the result) is durable now.
    loan.clear();
    assert!(loan.is_empty());
    assert_eq!(loan.overlay(&boxes).units_of(Kind::Material, 0x0e, 3), 10, "file promotes to the true 10");
}

#[test]
fn reclaiming_more_than_was_loaned_never_underflows_the_loan() {
    // The sweep also pulls in materials the player had in the bag from other sources; the
    // loan for that (cat, idx) must not go negative.
    let mut loan = Loan::default();
    loan.add(Kind::Material, 0x0e, 3, 5);
    loan.sub(Kind::Material, 0x0e, 3, 12);
    assert!(loan.is_empty(), "5 loaned, 12 reclaimed -> loan is 0, not a wraparound");
    let boxes = { let mut b = Boxes::default(); b.add(Kind::Material, 0x0e, 3, 7); b };
    assert_eq!(loan.overlay(&boxes), boxes, "and the overlay adds nothing back");
}

#[test]
fn a_loan_for_one_category_does_not_touch_another() {
    let mut boxes = Boxes::default();
    boxes.add(Kind::Material, 0x0e, 3, 10);
    boxes.add(Kind::Orb, 0x10, 1, 4);
    let mut loan = Loan::default();
    loan.add(Kind::Material, 0x0e, 3, 10);
    let o = loan.overlay(&boxes);
    assert_eq!(o.units_of(Kind::Material, 0x0e, 3), 20);
    assert_eq!(o.units_of(Kind::Orb, 0x10, 1), 4, "the orb box is untouched by a material loan");
}

// ── the game's acquisition notice (103차) ────────────────────────────────────

/// `제련석` in EUC-KR — the material the player picks up most.
const NAME_EUCKR: [u8; 6] = [0xc1, 0xa6, 0xb7, 0xc3, 0xbc, 0xae];

#[test]
fn a_queued_notice_lands_where_the_game_would_have_put_it() {
    // The game writes at `queue[count]` and then makes the count one higher; the renderer
    // draws entries 0..count, so both halves are needed or nothing shows.
    let (off, bytes, next) = notice_plan(0, &NAME_EUCKR).unwrap();
    assert_eq!(off, NOTICE_TEXT_OFF);
    assert_eq!(next, 1);
    assert_eq!(bytes, [0xc1, 0xa6, 0xb7, 0xc3, 0xbc, 0xae, 0x00], "NUL-terminated for the guest");
    // The second notice of the same frame goes one stride further, never on top.
    let (off, _, next) = notice_plan(1, &NAME_EUCKR).unwrap();
    assert_eq!(off, NOTICE_TEXT_OFF + NOTICE_STRIDE);
    assert_eq!(next, 2);
    let (off, _, next) = notice_plan(4, &NAME_EUCKR).unwrap();
    assert_eq!(off, NOTICE_TEXT_OFF + 4 * NOTICE_STRIDE);
    assert_eq!(next, NOTICE_SLOTS);
}

#[test]
fn a_full_queue_is_refused_rather_than_written_past() {
    // ⛔ The GAME writes a sixth entry over `grid+0x354` and only then declines to count
    // it (`0x53226`: increment while `count <= 4`). That entry is never drawn, so refusing
    // loses nothing and keeps 0x1e bytes of the grid ours to not corrupt.
    assert_eq!(notice_plan(NOTICE_SLOTS, &NAME_EUCKR), None);
    assert_eq!(notice_plan(6, &NAME_EUCKR), None);
    // A count byte the game could not have written is not a licence to guess.
    assert_eq!(notice_plan(-1, &NAME_EUCKR), None);
    // And a name that encoded to nothing is not queued as a blank line.
    assert_eq!(notice_plan(0, &[]), None);
}

#[test]
fn a_long_name_is_cut_on_a_syllable_and_still_fits_the_slot() {
    // Every notice is one `NOTICE_STRIDE` slot, so a long name has to be cut — but never
    // between the two bytes of a syllable: the guest's decoder would pair the orphan with
    // the NUL and print a different character.
    let long: Vec<u8> = std::iter::repeat_n([0xc1u8, 0xa6], 20).flatten().collect();
    let (_, bytes, _) = notice_plan(0, &long).unwrap();
    assert!(bytes.len() <= NOTICE_STRIDE as usize, "the text stays inside its slot");
    assert_eq!(*bytes.last().unwrap(), 0);
    assert_eq!((bytes.len() - 1) % 2, 0, "cut on a pair boundary");
    // An odd-length mix cuts the same way: the trailing ASCII byte may go, a half pair may not.
    let mixed = [0xc1, 0xa6, b'+', 0xb7, 0xc3];
    let (_, bytes, _) = notice_plan(0, &mixed).unwrap();
    assert_eq!(bytes, [0xc1, 0xa6, b'+', 0xb7, 0xc3, 0x00]);
}

#[test]
fn the_queue_geometry_matches_the_memset_that_clears_it() {
    // `0x53760` clears exactly `NOTICE_SLOTS * NOTICE_STRIDE` bytes at `NOTICE_TEXT_OFF`,
    // and the next field starts right after — the arithmetic that says five, not six.
    assert_eq!(NOTICE_STRIDE * NOTICE_SLOTS as u32, 0x96);
    assert_eq!(NOTICE_TEXT_OFF + 0x96, 0x354);
    // The count byte sits below the texts, where the renderer reads it.
    assert!(NOTICE_COUNT_OFF < NOTICE_TEXT_OFF);
}

#[test]
fn the_object_particle_follows_the_final_consonant() {
    // 110차-B: the window's own 입수 line wraps the game's 「오브를 입수 하였습니다.」.
    assert_eq!(object_particle("마도의 오브"), "를");
    assert_eq!(object_particle("닉스코어파편"), "을");
    assert_eq!(object_particle("포션"), "을");
    assert_eq!(object_particle("과일쥬스"), "를");
    assert_eq!(object_particle("0x12/0x07"), "을(를)");
    assert_eq!(object_particle(""), "을(를)");
}

// ── 2026-09-21: new game plus takes the same rows out of the boxes ────────────

/// The three shapes the game's own `0x3af2c` removes, and nothing else. ⛔ The table is
/// read off the disassembly, so this test is the place the disassembly is written down
/// twice: change one and the other has to agree.
#[test]
fn new_game_plus_takes_exactly_what_the_game_takes() {
    // 그리폰의피리 and its recipe, by name in the generated table — the independent
    // confirmation that the two indices off the disassembly are the two the player named.
    let (_, piri, _) = name_of(NAMES, 0x12, 0x09);
    assert!(piri.contains("그리폰"), "cat 0x12 idx 9 is {piri}");
    let (_, recipe, _) = name_of(NAMES, 0x0f, 0x12);
    assert!(recipe.contains("그리폰"), "cat 0x0f idx 0x12 is {recipe}");

    assert!(new_game_plus_removes(0x12, 0x09));
    assert!(new_game_plus_removes(0x0f, 0x12));
    // Every quest item, whatever the index.
    for idx in [0u16, 1, 9, 0x12, 0xff] {
        assert!(new_game_plus_removes(0x11, idx), "quest idx {idx:#x}");
    }
    // The neighbours are kept — 귀환서 is idx 8, one below the 피리, and the recipe index
    // means something else in every other category.
    assert!(!new_game_plus_removes(0x12, 0x08), "귀환서 survives");
    assert!(!new_game_plus_removes(0x12, 0x12), "cat 0x12 idx 0x12 is not the recipe");
    assert!(!new_game_plus_removes(0x0f, 0x09), "another recipe survives");
    assert!(!new_game_plus_removes(0x0e, 0x12), "a material survives");
    assert!(!new_game_plus_removes(0x10, 0x09), "an orb survives");
}

/// The plan names the right box for each row, takes the WHOLE stack, and leaves the rest
/// of the box alone.
#[test]
fn new_game_plus_empties_only_those_rows() {
    let mut b = Boxes::default();
    assert_eq!(b.add(Kind::Consumable, 0x12, 0x09, 3), 0); // 그리폰의피리 x3
    assert_eq!(b.add(Kind::Consumable, 0x12, 0x08, 5), 0); // 귀환서 x5
    assert_eq!(b.add(Kind::Material, 0x0f, 0x12, 1), 0); // 피리 레시피 (125차-B: a stack an old file still keeps there)
    assert_eq!(b.add(Kind::Material, 0x0e, 0x03, 40), 0); // 제련석
    assert_eq!(b.add(Kind::Orb, 0x10, 0x05, 7), 0);

    let mut plan = new_game_plus_plan(&b);
    plan.sort_by_key(|(k, c, i, _)| (k.index(), *c, *i));
    assert_eq!(plan, vec![(Kind::Material, 0x0f, 0x12, 1), (Kind::Consumable, 0x12, 0x09, 3)]);

    for (kind, cat, idx, n) in plan {
        assert_eq!(b.spend_units(kind, cat, idx, n), n);
    }
    assert_eq!(b.units_of(Kind::Consumable, 0x12, 0x09), 0);
    assert_eq!(b.units_of(Kind::Material, 0x0f, 0x12), 0);
    // ⛔ Everything else is untouched: the transition is NOT a bag wipe, and reading it
    // as one would delete a player's whole collection on new game plus.
    assert_eq!(b.units_of(Kind::Consumable, 0x12, 0x08), 5);
    assert_eq!(b.units_of(Kind::Material, 0x0e, 0x03), 40);
    assert_eq!(b.units_of(Kind::Orb, 0x10, 0x05), 7);
    // Idempotent: running it again finds nothing.
    assert!(new_game_plus_plan(&b).is_empty());
}

/// A stack split across several entries goes entirely, and an empty box plans nothing.
#[test]
fn new_game_plus_takes_every_stack_of_the_row() {
    assert!(new_game_plus_plan(&Boxes::default()).is_empty());
    let mut b = Boxes::default();
    for _ in 0..3 {
        assert_eq!(b.add(Kind::Consumable, 0x12, 0x09, 99), 0);
    }
    assert_eq!(b.units_of(Kind::Consumable, 0x12, 0x09), 297);
    let plan = new_game_plus_plan(&b);
    assert_eq!(plan, vec![(Kind::Consumable, 0x12, 0x09, 297)]);
    for (kind, cat, idx, n) in plan {
        assert_eq!(b.spend_units(kind, cat, idx, n), n);
    }
    assert_eq!(b.units_of(Kind::Consumable, 0x12, 0x09), 0);
}

/// ⛔ IT IS AN EDGE, NOT A FILTER. Both rows must still be boxable afterwards — a quest
/// that hands the 피리 back mid-game has to work exactly as before.
#[test]
fn new_game_plus_does_not_bar_the_rows_from_the_boxes() {
    assert_eq!(kind_of(0x12), Some(Kind::Consumable));
    // (125차-B: the recipe is a bag item now — the game's own transition clears the bag's
    // copy, and a quest that hands it back puts it in the bag like any other recipe.)
    assert_eq!(kind_of(0x0f), None);
    assert!(!is_bag_item(0x12, 0x09), "the 피리 is not a bag-cell item");
    let mut b = Boxes::default();
    assert_eq!(b.add(Kind::Consumable, 0x12, 0x09, 1), 0);
    assert_eq!(b.units_of(Kind::Consumable, 0x12, 0x09), 1);
    // And the sweep still wants to absorb it.
    let plan = sweep_plan(&Boxes::default(), &[cell(0, 5, 0x12, 0x09, 1)], &Kind::ALL, false);
    assert_eq!(plan.len(), 1, "a fresh 피리 still sweeps into the 소비가방");
    // Its verbs are unchanged (it is one of the two that open a game window).
    assert!(use_opens_window(0x12, 0x09));
}

/// ★★★ 124차-B — 제보 2026-09-21: *"하드코어 들어가기 전에 저장을 하고 하드코어 클리어 이후
/// 저장안하고 다시 세이브슬롯을 불러오면 … 피리가 사라지고 … 다시 이전 저장데이터를
/// 불러오면 피리가 있어야해"*.
///
/// The game's half of the transition (its bag walk) is unsaved memory; a reload of the
/// pre-transition record rolls it back. So the box debit is a LOAN (97차/107차 shape):
/// the FILE (= `loan.overlay(boxes)`) keeps the rows until a GameSave commits, the box
/// itself (what the window and the quest mirror read) shows them gone at once, and a
/// load edge folds the loan back into the box (108차).
#[test]
fn new_game_plus_is_a_loan_until_a_save() {
    let mut b = Boxes::default();
    assert_eq!(b.add(Kind::Consumable, 0x12, 0x09, 1), 0); // 그리폰의피리
    assert_eq!(b.add(Kind::Consumable, 0x12, 0x08, 5), 0); // 귀환서 — the "a" of the report
    assert_eq!(b.add(Kind::Material, 0x0f, 0x12, 1), 0); // 피리 레시피 (125차-B: a stack an old file still keeps there)
    let mut loan = Loan::default();

    // The host applies the plan exactly as `BagBoxHost::new_game_plus` does.
    for (kind, cat, idx, n) in new_game_plus_plan(&b) {
        let took = b.spend_units(kind, cat, idx, n);
        assert_eq!(took, n);
        loan.add(kind, cat, idx, took);
    }
    // 클리어하고 피리가 사라지는 건 맞다: the box the window reads has none.
    assert_eq!(b.units_of(Kind::Consumable, 0x12, 0x09), 0);
    assert_eq!(b.units_of(Kind::Material, 0x0f, 0x12), 0);
    assert!(new_game_plus_plan(&b).is_empty(), "idempotent on the debited box");

    // ⛔ …but the FILE still has both rows — that is what a reload without a save finds.
    let on_disk = loan.overlay(&b);
    assert_eq!(on_disk.units_of(Kind::Consumable, 0x12, 0x09), 1, "no save yet: the 피리 survives in the file");
    assert_eq!(on_disk.units_of(Kind::Material, 0x0f, 0x12), 1, "…and so does the recipe");
    assert_eq!(on_disk.units_of(Kind::Consumable, 0x12, 0x08), 5, "the untouched row is unaffected");

    // 이전 저장데이터를 불러오면: the load edge folds the loan back (108차) — the 피리 is
    // in the box again, beside the pre-transition bag, and the file does not change.
    let mut reloaded = loan.overlay(&b);
    let mut folded = loan.clone();
    folded.clear();
    assert_eq!(reloaded.units_of(Kind::Consumable, 0x12, 0x09), 1);
    assert_eq!(folded.overlay(&reloaded).to_text(), reloaded.to_text(), "the fold changes nothing on disk (108차)");
    // And a second transition on that reloaded state takes it again — it is an edge.
    assert_eq!(new_game_plus_plan(&reloaded).len(), 2);
    for (kind, cat, idx, n) in new_game_plus_plan(&reloaded) {
        assert_eq!(reloaded.spend_units(kind, cat, idx, n), n);
    }

    // A GameSave commits the 2회차 bag: the loan lifts and the debit is real.
    loan.clear();
    let saved = loan.overlay(&b);
    assert_eq!(saved.units_of(Kind::Consumable, 0x12, 0x09), 0, "saved: the 피리 is really gone");
    assert_eq!(saved.units_of(Kind::Material, 0x0f, 0x12), 0);
    assert_eq!(saved.units_of(Kind::Consumable, 0x12, 0x08), 5);

    // ⛔ The first cut's `loan.sub`: a 피리 that had been materialised into the bag (box 0,
    // loan 1) is one the game removes from the bag itself — the plan finds nothing, and the
    // loan must be LEFT ALONE so the pre-transition save still owns it. Subtracting it
    // would erase the only record that the unit was ever the box's.
    let mut lent = Boxes::default();
    assert_eq!(lent.add(Kind::Consumable, 0x12, 0x09, 1), 0);
    let mut l3 = Loan::default();
    assert_eq!(lent.spend_units(Kind::Consumable, 0x12, 0x09, 1), 1); // materialised into the bag
    l3.add(Kind::Consumable, 0x12, 0x09, 1);
    assert!(new_game_plus_plan(&lent).is_empty(), "nothing in the box to take");
    // (host: no `loan.sub` — the loan stays.)
    assert_eq!(l3.overlay(&lent).units_of(Kind::Consumable, 0x12, 0x09), 1, "a reload without a save still has it");
    l3.clear();
    assert_eq!(l3.overlay(&lent).units_of(Kind::Consumable, 0x12, 0x09), 0, "a save makes the game's removal real");
}

/// ★★ 165차 — 리파이너의 유적 7's boss door gives 그리폰의피리 and takes it back inside ONE
/// guest call, so the drain sees the give and the take together. An intake is filed for good
/// and a take is a loan until a save (107차); without the cancel the pair left the FILE one
/// flute up, and a reload without saving — a boss fight cannot be saved — handed it back on
/// every attempt. The host applies the ring exactly the way `BagBoxHost::drain_intake` does.
#[test]
fn a_give_and_a_take_in_one_drain_cancel_instead_of_leaving_a_loan() {
    let flute = (0x12u16, 0x09u16);
    let apply = |b: &mut Boxes, loan: &mut Loan, ring: &[(u16, u16, u32, bool)]| {
        let mut fresh = FreshIntake::default();
        for &(cat, idx, n, take) in ring {
            let kind = kind_of(cat).unwrap();
            if take {
                let took = b.spend_units(kind, cat, idx, n);
                let cancelled = fresh.cancel(cat, idx, took);
                loan.add(kind, cat, idx, took - cancelled);
            } else {
                let left = b.add(kind, cat, idx, n);
                fresh.add(cat, idx, n - left);
            }
        }
    };
    // The player has no flute; the door gives one and takes it.
    let mut b = Boxes::default();
    let mut loan = Loan::default();
    apply(&mut b, &mut loan, &[(flute.0, flute.1, 1, false), (flute.0, flute.1, 1, true)]);
    assert_eq!(b.units_of(Kind::Consumable, flute.0, flute.1), 0);
    assert!(loan.is_empty(), "nothing is owed back");
    assert_eq!(loan.overlay(&b).units_of(Kind::Consumable, flute.0, flute.1), 0, "a reload finds no flute — as the game would");

    // A flute the player ALREADY had, taken by a script later, is a loan exactly as before.
    let mut b = Boxes::default();
    let mut loan = Loan::default();
    apply(&mut b, &mut loan, &[(flute.0, flute.1, 1, false)]);
    apply(&mut b, &mut loan, &[(flute.0, flute.1, 1, true)]);
    assert_eq!(b.units_of(Kind::Consumable, flute.0, flute.1), 0);
    assert_eq!(loan.overlay(&b).units_of(Kind::Consumable, flute.0, flute.1), 1, "107차: unsaved, the file keeps it");

    // Only what THIS drain gave cancels: one owned + one given, two taken → one loaned.
    let mut b = Boxes::default();
    let mut loan = Loan::default();
    apply(&mut b, &mut loan, &[(flute.0, flute.1, 1, false)]);
    apply(&mut b, &mut loan, &[(flute.0, flute.1, 1, false), (flute.0, flute.1, 2, true)]);
    assert_eq!(loan.overlay(&b).units_of(Kind::Consumable, flute.0, flute.1), 1);

    // Per item: a give of something else does not cancel the flute's take.
    let mut fresh = FreshIntake::default();
    fresh.add(0x12, 0x00, 99);
    assert_eq!(fresh.cancel(flute.0, flute.1, 1), 0);
    assert_eq!(fresh.cancel(0x12, 0x00, 120), 99, "never more than was given");
    assert_eq!(fresh.cancel(0x12, 0x00, 1), 0, "and only once");
}
