// Standalone self-test of 정렬 (136차) — the pure ordering rules behind the 「정렬」 button.
//
//   rustc --edition 2021 --test bagsort_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host, so this compiles the REAL
// source — `bagsort.rs` is dependency-free for exactly that reason — together with the
// generated level table, which is what the app hands it.

include!("wie_android/src/bagsort.rs");
mod item_levels {
    include!("wie_android/src/item_levels.rs");
}

/// The three bag-item ids in seat order — `bagbox::Kind::ALL[k].bag_item()`.
const PINNED: [(u16, u16); 3] = [(0x12, 0x16), (0x12, 0x17), (0x12, 0x19)];

fn cell(tab: u8, slot: u8, cat: u16, idx: u16) -> Cell {
    Cell { tab, slot, cat, idx, count: 1, refine: 0, key: u64::from(tab) << 8 | u64::from(slot) }
}

fn lv(cat: u16, idx: u16) -> Option<(u8, u8)> {
    item_levels::item_level(cat, idx)
}

fn dest_of(plan: &[((u8, u8), usize)], i: usize) -> (u8, u8) {
    plan.iter().find(|(_, j)| *j == i).map(|(d, _)| *d).expect("every cell is placed")
}

#[test]
fn the_part_order_is_weapons_shield_helm_armour_gloves_boots_accessory() {
    let order: Vec<u16> = [0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x00, 0x01, 0x02, 0x03, 0x0c].to_vec();
    for w in order.windows(2) {
        assert!(slot_rank(w[0]).unwrap() < slot_rank(w[1]).unwrap(), "{:#x} before {:#x}", w[0], w[1]);
    }
    for cat in [0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12] {
        assert_eq!(slot_rank(cat), None, "{cat:#x} is not equipment");
    }
}

#[test]
fn the_pins_stay_in_bag_one_cells_one_two_three() {
    // A bag with the three bag items scattered behind the equipment.
    let cells = vec![
        cell(0, 0, 0x05, 0x10), // a sword
        cell(0, 1, 0x00, 0x03), // a helm
        cell(0, 5, 0x12, 0x19), // 소비가방
        cell(1, 2, 0x12, 0x16), // 오브가방
        cell(1, 9, 0x12, 0x17), // 재료가방
    ];
    let plan = layout(&cells, &PINNED, 2, &lv);
    assert_eq!(dest_of(&plan, 3), (0, 0), "오브가방 -> bag 1 cell 1");
    assert_eq!(dest_of(&plan, 4), (0, 1), "재료가방 -> bag 1 cell 2");
    assert_eq!(dest_of(&plan, 2), (0, 2), "소비가방 -> bag 1 cell 3");
    // …and the equipment follows from cell 4 on.
    assert_eq!(dest_of(&plan, 0), (0, 3), "the sword is the first regular cell");
    assert_eq!(dest_of(&plan, 1), (0, 4));
}

#[test]
fn a_missing_pin_does_not_leave_a_hole() {
    // Only 소비가방 is present: it takes ITS seat (cell 3), and cells 1·2 go to regular
    // items — the game's compaction would pull them forward anyway.
    let cells = vec![cell(0, 4, 0x12, 0x19), cell(0, 7, 0x04, 0x02), cell(0, 9, 0x00, 0x01)];
    let plan = layout(&cells, &PINNED, 1, &lv);
    assert_eq!(dest_of(&plan, 0), (0, 2), "소비가방 keeps seat 3");
    let mut regs = [dest_of(&plan, 1), dest_of(&plan, 2)];
    regs.sort();
    assert_eq!(regs, [(0, 0), (0, 1)], "the regulars fill the two free seats before it");
}

#[test]
fn within_a_part_higher_level_comes_first_then_grade_then_idx() {
    // Three swords (cat 0x05) from the real table: the highest 요구레벨 lands first.
    let cells = vec![cell(0, 0, 0x05, 0x00), cell(0, 1, 0x05, 0x10), cell(0, 2, 0x05, 0x05)];
    let plan = layout(&cells, &[], 1, &lv);
    let levels: Vec<u8> = (0..3u8)
        .map(|s| {
            let i = plan.iter().find(|(d, _)| *d == (0, s)).unwrap().1;
            lv(cells[i].cat, cells[i].idx).unwrap().1
        })
        .collect();
    assert!(levels[0] >= levels[1] && levels[1] >= levels[2], "levels descend: {levels:?}");
    // A stub table where rows tie on level breaks the tie on grade, then idx.
    let tie = |_c: u16, i: u16| Some((if i == 3 { 4u8 } else { 1u8 }, 50u8));
    let cells = vec![cell(0, 0, 0x05, 0x07), cell(0, 1, 0x05, 0x03), cell(0, 2, 0x05, 0x01)];
    let plan = layout(&cells, &[], 1, &tie);
    assert_eq!(dest_of(&plan, 1), (0, 0), "grade 4 first");
    assert_eq!(dest_of(&plan, 2), (0, 1), "then idx 1");
    assert_eq!(dest_of(&plan, 0), (0, 2), "then idx 7");
}

#[test]
fn parts_beat_levels_and_quest_items_and_recipes_trail() {
    // A level-99 helm still sorts after a level-1 sword (부위 first), and a quest item /
    // recipe / stray potion follow all equipment in that order.
    let cells = vec![
        cell(0, 0, 0x11, 0x02), // quest
        cell(0, 1, 0x00, 0x30), // helm (high level)
        cell(0, 2, 0x0f, 0x05), // recipe
        cell(0, 3, 0x05, 0x00), // sword (low level)
        cell(0, 4, 0x12, 0x01), // a potion the hooks let through
    ];
    let plan = layout(&cells, &[], 1, &lv);
    assert_eq!(dest_of(&plan, 3), (0, 0), "sword");
    assert_eq!(dest_of(&plan, 1), (0, 1), "helm");
    assert_eq!(dest_of(&plan, 0), (0, 2), "quest item");
    assert_eq!(dest_of(&plan, 2), (0, 3), "recipe");
    assert_eq!(dest_of(&plan, 4), (0, 4), "potion");
}

#[test]
fn refine_then_key_break_a_full_tie() {
    let mut a = cell(0, 3, 0x05, 0x02);
    a.refine = 3;
    a.key = 9;
    let mut b = cell(0, 1, 0x05, 0x02);
    b.refine = 7;
    b.key = 5;
    let mut c = cell(0, 2, 0x05, 0x02);
    c.refine = 3;
    c.key = 4;
    let plan = layout(&[a, b, c], &[], 1, &lv);
    assert_eq!(dest_of(&plan, 1), (0, 0), "+7 first");
    assert_eq!(dest_of(&plan, 2), (0, 1), "then the lower instance key");
    assert_eq!(dest_of(&plan, 0), (0, 2));
}

#[test]
fn the_layout_is_dense_per_tab_and_a_fixed_point_of_the_games_compaction() {
    // 20 items spread over three tabs with holes everywhere.
    let mut cells = Vec::new();
    for i in 0..20u8 {
        cells.push(cell(i % 3, (i * 5) % 16, u16::from(i % 4), u16::from(i % 7)));
    }
    let plan = layout(&cells, &[], 3, &lv);
    assert_eq!(plan.len(), 20);
    let mut dests: Vec<(u8, u8)> = plan.iter().map(|(d, _)| *d).collect();
    dests.sort();
    dests.dedup();
    assert_eq!(dests.len(), 20, "no two cells share a destination");
    // Tab 0 gets 0..16 and tab 1 gets 0..4 — no gap inside a tab, nothing in tab 2.
    assert_eq!(dests.iter().filter(|d| d.0 == 0).count(), 16);
    assert_eq!(dests.iter().filter(|d| d.0 == 1).map(|d| d.1).max(), Some(3));
    assert!(dests.iter().all(|d| d.0 < 2));
    // Applying the plan twice changes nothing.
    let after: Vec<Cell> = plan.iter().map(|(d, i)| Cell { tab: d.0, slot: d.1, ..cells[*i] }).collect();
    let again = layout(&after, &[], 3, &lv);
    assert!(again.iter().all(|(d, i)| (after[*i].tab, after[*i].slot) == *d), "idempotent");
}

#[test]
fn more_items_than_seats_are_not_placed_past_the_bags() {
    let cells: Vec<Cell> = (0..20u8).map(|i| cell(0, i % 16, 0x05, u16::from(i))).collect();
    let plan = layout(&cells, &[], 1, &lv);
    assert_eq!(plan.len(), 16, "one bag holds sixteen");
    assert!(plan.iter().all(|(d, _)| d.0 == 0));
}

#[test]
fn quickbar_refs_follow_their_item_and_unbound_refs_stay() {
    let cells = vec![cell(1, 4, 0x05, 0x00), cell(0, 2, 0x00, 0x00)];
    let plan = layout(&cells, &[], 2, &lv);
    let sword_at = dest_of(&plan, 0);
    assert_eq!(remap_ref(item_ref(1, 4), &cells, &plan), item_ref(sword_at.0, sword_at.1));
    assert_eq!(remap_ref(item_ref(3, 9), &cells, &plan), item_ref(3, 9), "a ref at an empty cell is left alone");
    assert_eq!(moved_to(1, 4, &cells, &plan), Some(sword_at));
    assert_eq!(moved_to(4, 4, &cells, &plan), None);
}

#[test]
fn the_wire_key_orders_warehouse_rows_like_the_bag() {
    // Two swords by level, then a helm, then a material row after all equipment.
    let rows = [(0x05u8, 0x00u8, 1u8), (0x05, 0x10, 1), (0x00, 0x01, 1), (0x0e, 0x03, 40)];
    let mut keyed: Vec<(Key, usize)> = rows.iter().enumerate().map(|(p, r)| (wire_key(r.0, r.1, r.2, p as u16, &lv), p)).collect();
    keyed.sort();
    let order: Vec<usize> = keyed.into_iter().map(|(_, p)| p).collect();
    assert_eq!(order, vec![1, 0, 2, 3]);
}

#[test]
fn the_generated_table_is_sorted_and_answers_lookups() {
    let t = &item_levels::ITEM_LEVELS;
    assert!(t.windows(2).all(|w| (w[0].0, w[0].1) < (w[1].0, w[1].1)), "sorted by (cat, idx) — binary search depends on it");
    assert_eq!(item_levels::item_level(0x00, 0x00).map(|x| x.1), Some(2), "머리띠 is Lv2");
    assert_eq!(item_levels::item_level(0x12, 0x01), None, "consumables are not in it");
    // 닉스의펜던트 (i12 idx 0x32): 90 since the 142차 rework (was 95). ★ 150차: its 요구레벨 byte
    // (b6) is 1 now, but a `"` legacy row sorts by its 85/90제 TIER (`gen_item_levels.py`), so
    // the 정렬 order does not move; the boss `{` 타이탄의 파편 (0x28) still reads its b6 = 1.
    assert_eq!(item_levels::item_level(0x0c, 0x32).map(|x| x.1), Some(90), "a legacy accessory sorts by its tier");
    assert_eq!(item_levels::item_level(0x0c, 0x28).map(|x| x.1), Some(1), "a boss accessory reads b6");
    assert!(t.iter().all(|r| (1..=99).contains(&r.3)), "every 요구레벨 is 1..=99");
}
