//! Standalone selftest for the 아바타 옷장 (167차 · 168차): `wie_android/src/avatar.rs` (the rules,
//! the AVATAR page state and the MENU list's AVATAR row) and `wie_android/src/avatar_page.rs`
//! (the page and the seven-row MENU list, drawn). `include!`s the
//! real sources under the crate-root names the app uses — the shared numbers
//! (`wie_util::avatar` as `avatar_rules`), the generated catalog, the game font. Run from the
//! repository root:
//!
//! ```powershell
//! rustc --edition 2024 --test avatar_selftest.rs -o t.exe; .	.exe
//! ```

extern crate alloc;

#[allow(dead_code)]
mod avatar_rules {
    include!("vendor/wie/wie_util/src/avatar.rs");
}
mod avatar_catalog {
    include!("wie_android/src/avatar_catalog.rs");
}
mod gamefont_data {
    include!("wie_android/src/gamefont_data.rs");
}
#[allow(dead_code)]
mod gamefont {
    include!("wie_android/src/gamefont.rs");
}
#[allow(dead_code)]
mod avatar {
    include!("wie_android/src/avatar.rs");
}
#[allow(dead_code)]
mod avatar_page {
    include!("wie_android/src/avatar_page.rs");
}

#[cfg(test)]
mod tests {
    use crate::avatar::*;
    use crate::avatar_catalog::{Acquire, Hero, SETS, SetDef};
    use crate::avatar_rules as av;

    /// Every group carries every set's sheet.
    fn all_sheets() -> [u64; av::GROUPS] {
        [u64::MAX; av::GROUPS]
    }

    fn ctx() -> Ctx {
        Ctx {
            difficulty: Some(0),
            class: Some(5),
            weapon_cat: Some(6),
            equipped: [true; av::SLOTS],
            sheets: all_sheets(),
            hero: Some(Hero::Kei),
        }
    }

    fn set_with(acquire: Acquire) -> &'static SetDef {
        SETS.iter().find(|s| s.acquire == acquire).expect("the catalog has that kind of set")
    }

    #[test]
    fn the_catalog_is_sane() {
        let mut ids: Vec<u8> = SETS.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SETS.len(), "set ids are unique");
        assert!(SETS.iter().all(|s| (1..=av::SET_MAX).contains(&s.id)));
        assert!(SETS.iter().any(|s| s.acquire == Acquire::Starter), "a starter set exists (decision ③: hero2's way)");
        // ★ 168차: each hero starts with something of its own to wear…
        for h in [Hero::Ritz, Hero::Kei] {
            assert!(SETS.iter().any(|s| s.acquire == Acquire::Starter && s.hero == Some(h)), "{h:?} has a starter set");
        }
        // …and 167차's retired sets keep their numbers out of use: saves still hold them.
        for id in [1u8, 3] {
            assert!(set_def(id).is_none(), "set {id} was retired in 168차 — its number is never reused");
        }
        for s in SETS {
            assert!(!s.name.is_empty());
            for (w, n) in s.weapons.iter().enumerate() {
                assert!(n.is_empty() || !s.pieces[av::SLOT_WEAPON].is_empty(), "set {} weapon kind {w} without a default", s.id);
            }
        }
    }

    /// ★ 175차 (the user, 2026-10-09): each hero's OWN sets are the pool party set at character
    /// creation, two for clearing 노말 and two for clearing 하드코어 (one of them new).
    #[test]
    fn each_heros_own_sets_are_one_starter_two_normal_two_hard() {
        for h in [Hero::Ritz, Hero::Kei] {
            let own: Vec<&SetDef> = SETS.iter().filter(|s| s.hero == Some(h)).collect();
            let count = |a: Acquire| own.iter().filter(|s| s.acquire == a).count();
            assert_eq!(count(Acquire::Starter), 1, "{h:?}: only the pool party set is given at creation");
            assert!(own.iter().any(|s| s.acquire == Acquire::Starter && s.name.starts_with("수영장 파티")), "{h:?}");
            assert_eq!(count(Acquire::Clear(0)), 2, "{h:?}: two sets for clearing 노말");
            assert_eq!(count(Acquire::Clear(1)), 2, "{h:?}: two sets for clearing 하드코어");
            assert_eq!(own.len(), 5, "{h:?}");
        }
        assert_eq!(set_def(14).map(|s| (s.acquire, s.hero)), Some((Acquire::Clear(1), Some(Hero::Kei))));
        assert_eq!(set_def(15).map(|s| (s.acquire, s.hero)), Some((Acquire::Clear(1), Some(Hero::Ritz))));
        // What a clear pays one hero (and what its notice names — `on_ending` asks `offered`): the
        // shared set of that difficulty (이안 · 케네스) + the hero's own two.
        for h in [Hero::Ritz, Hero::Kei] {
            let c = Ctx { hero: Some(h), ..ctx() };
            assert_eq!(clear_rewards(0).filter(|s| offered(s, &c)).count(), 3, "{h:?}: 노말 clear");
            assert_eq!(clear_rewards(1).filter(|s| offered(s, &c)).count(), 3, "{h:?}: 하드코어 clear");
        }
    }

    /// ★ 175차: what a loaded character may not wear comes off — a set it does not own (08~13 were
    /// 기본 지급 until 175차) or the other hero's; never on an unreadable difficulty or hero.
    #[test]
    fn a_costume_the_character_does_not_own_is_named_for_taking_off() {
        let normal_kei = SETS.iter().find(|s| s.hero == Some(Hero::Kei) && s.acquire == Acquire::Clear(0)).unwrap();
        let hard_kei = SETS.iter().find(|s| s.hero == Some(Hero::Kei) && s.acquire == Acquire::Clear(1)).unwrap();
        let pool_kei = SETS.iter().find(|s| s.hero == Some(Hero::Kei) && s.acquire == Acquire::Starter).unwrap();
        let ritz = SETS.iter().find(|s| s.hero == Some(Hero::Ritz) && s.acquire == Acquire::Starter).unwrap();
        let w = Wardrobe { wear: [normal_kei.id, hard_kei.id, pool_kei.id, ritz.id, 0, 3], ..Wardrobe::default() };
        let c = ctx();
        // 노말: neither reward is earned yet; the starter stays; the other hero's set comes off; the
        // retired number 3 reads as 기본 외형 already and is not named.
        assert_eq!(unowned_worn(&w, &c), Some(vec![av::SLOT_HELMET, av::SLOT_ARMOR, av::SLOT_SHOES]));
        // A 하드코어 save has cleared 노말 (keeps the 노말 set); a 헬 save has cleared 하드코어 too.
        assert_eq!(unowned_worn(&w, &Ctx { difficulty: Some(1), ..c }), Some(vec![av::SLOT_ARMOR, av::SLOT_SHOES]));
        assert_eq!(unowned_worn(&w, &Ctx { difficulty: Some(2), ..c }), Some(vec![av::SLOT_SHOES]));
        // The bit a clear wrote (vanilla's hard clear stays at difficulty 1) keeps it on.
        let paid = Wardrobe { own: av::with_owned(av::with_owned([0; av::OWN_BYTES], normal_kei.id), hard_kei.id), ..w };
        assert_eq!(unowned_worn(&paid, &c), Some(vec![av::SLOT_SHOES]));
        // Nothing worn, nothing named; nothing on a guess.
        assert_eq!(unowned_worn(&Wardrobe::default(), &c), Some(vec![]));
        assert_eq!(unowned_worn(&w, &Ctx { difficulty: None, ..c }), None);
        assert_eq!(unowned_worn(&w, &Ctx { hero: None, ..c }), None);
    }

    #[test]
    fn decode_reads_the_flag_bytes_in_order() {
        let mut b = [0u8; Wardrobe::LEN];
        b[1] = 2;
        b[av::SLOTS] = 0b10;
        let w = Wardrobe::decode(&b).unwrap();
        assert_eq!(w.wear, [0, 2, 0, 0, 0, 0]);
        assert!(av::owns(&w.own, 2) && !av::owns(&w.own, 1));
        assert_eq!(Wardrobe::decode(&b[..Wardrobe::LEN - 1]), None);
    }

    #[test]
    fn ownership_starter_bit_and_the_difficulty_the_save_plays() {
        let w = Wardrobe::default();
        let starter = set_with(Acquire::Starter);
        let normal = set_with(Acquire::Clear(0));
        let hard = set_with(Acquire::Clear(1));
        assert!(w.owns(starter, None), "everyone owns the starter set");
        assert!(!w.owns(normal, Some(0)), "a 노말 save has not cleared 노말");
        assert!(w.owns(normal, Some(1)), "a 하드코어 save has — the reward comes from the save itself");
        assert!(w.owns(hard, Some(2)), "a 헬 save has cleared 하드코어");
        assert!(!w.owns(hard, Some(1)), "vanilla's hard clear stays at 1: only its bit says so");
        let bit = Wardrobe { own: av::with_owned([0; av::OWN_BYTES], hard.id), ..w };
        assert!(bit.owns(hard, Some(1)));
        assert!(!w.owns(normal, None), "an unreadable difficulty grants nothing");
    }

    #[test]
    fn the_list_is_basic_then_owned_sets_with_art() {
        let w = Wardrobe::default();
        let c = ctx();
        let list = entries(av::SLOT_ARMOR, &w, &c);
        assert_eq!(list[0].set, 0);
        assert_eq!(list[0].name, BASIC_NAME);
        let starter = set_with(Acquire::Starter);
        assert!(list.iter().any(|e| e.set == starter.id), "the starter set is listed");
        assert!(!list.iter().any(|e| e.set == set_with(Acquire::Clear(0)).id), "an unowned set is not");
        // No sheet in the jar for the armour group: nothing but 기본 외형.
        let bare = Ctx { sheets: [0; av::GROUPS], ..c };
        assert_eq!(entries(av::SLOT_ARMOR, &w, &bare).len(), 1);
    }

    /// ★ 168차: the pool party sets are each one hero's; an unreadable hero offers neither.
    #[test]
    fn a_heros_own_set_is_offered_to_that_hero_only() {
        let w = Wardrobe::default();
        // ★ 175차: a hero's own sets are no longer all 기본 지급 — list from a 헬 save (it has cleared
        // 노말 and 하드코어, so it owns every one of them) to test the hero rule, not ownership.
        let listed = |h: Option<Hero>| -> Vec<u8> {
            let c = Ctx { hero: h, difficulty: Some(2), ..ctx() };
            entries(av::SLOT_ARMOR, &w, &c).iter().map(|e| e.set).collect()
        };
        let only = |h: Hero| -> Vec<u8> { SETS.iter().filter(|s| s.hero == Some(h)).map(|s| s.id).collect() };
        let (kei_only, ritz_only) = (only(Hero::Kei), only(Hero::Ritz));
        assert!(!kei_only.is_empty() && !ritz_only.is_empty(), "each hero has a set of its own");
        let kei = listed(Some(Hero::Kei));
        assert!(kei_only.iter().all(|id| kei.contains(id)) && !ritz_only.iter().any(|id| kei.contains(id)));
        let ritz = listed(Some(Hero::Ritz));
        assert!(ritz_only.iter().all(|id| ritz.contains(id)) && !kei_only.iter().any(|id| ritz.contains(id)));
        let nobody = listed(None);
        assert!(!nobody.iter().any(|id| kei_only.contains(id) || ritz_only.contains(id)), "an unreadable hero offers neither");
        // A set for both heroes does not care.
        let both = SETS.iter().find(|s| s.hero.is_none()).unwrap();
        assert!(offered(both, &Ctx { hero: None, ..ctx() }));
    }

    /// ★ 168차: a worn byte naming a retired set (167차's 1 and 3) is drawn as the equipment by
    /// the hooks (no sheet), so the page calls it 기본 외형 — and wearing 기본 외형 writes the 0.
    #[test]
    fn a_retired_set_byte_reads_as_the_basic_look_and_wearing_it_cleans_up() {
        let retired = (1..=av::SET_MAX).find(|&id| set_def(id).is_none()).unwrap();
        let w = Wardrobe { wear: [retired, 0, 0, 0, 0, 0], ..Wardrobe::default() };
        assert_eq!(w.worn(av::SLOT_HELMET), 0);
        let c = ctx();
        let mut p = Page::open(&w, &c);
        let v = p.view(&w, &c);
        assert!(v.rows[0].worn && v.rows[0].focused, "기본 외형 is what shows");
        assert_eq!(v.status, "착용 중");
        assert!(!v.dressed[av::SLOT_HELMET]);
        assert_eq!(p.act(Act::Confirm, &w, &c).wear, Some((av::SLOT_HELMET, 0)), "the write that cleans the byte");
        // A live set's byte is its own.
        let ian = set_with(Acquire::Clear(0)).id;
        assert_eq!(Wardrobe { wear: [ian, 0, 0, 0, 0, 0], ..w }.worn(av::SLOT_HELMET), ian);
    }

    #[test]
    fn menu_rows_map_both_ways() {
        let game: Vec<Option<u8>> = (0..MENU_ROWS).map(menu_game_of_row).collect();
        assert_eq!(game, [Some(0), Some(1), Some(2), None, Some(3), Some(4), Some(5)]);
        assert_eq!((0..MENU_GAME_ROWS).map(menu_row_of_game).collect::<Vec<_>>(), [0, 1, 2, 4, 5, 6]);
        assert_eq!(menu_game_of_row(MENU_ROWS), None);
        assert_eq!(crate::avatar_page::MENU_LABELS.len(), usize::from(MENU_ROWS));
        assert_eq!(crate::avatar_page::MENU_LABELS[usize::from(MENU_AVATAR)], ("AVATAR", "아바타"));
        assert_eq!(crate::avatar_page::MENU_LABELS[usize::from(menu_row_of_game(MENU_GAME_EQUIP))].0, "EQUIPMENT");
        assert_eq!(crate::avatar_page::MENU_LABELS[usize::from(menu_row_of_game(MENU_GAME_SKILL))].0, "SKILL");
    }

    #[test]
    fn menu_keys_move_through_avatar_and_pass_the_rest() {
        let (down, up, ok) = (MenuKey::Down, MenuKey::Up, MenuKey::Ok);
        assert_eq!(menu_move(down, false, 2), MenuMove::Select { avatar: true, cursor: 2 }, "EQUIPMENT -> AVATAR");
        assert_eq!(menu_move(down, true, 2), MenuMove::Select { avatar: false, cursor: 3 }, "AVATAR -> SKILL");
        assert_eq!(menu_move(up, false, 3), MenuMove::Select { avatar: true, cursor: 2 }, "SKILL -> AVATAR");
        assert_eq!(menu_move(up, true, 2), MenuMove::Select { avatar: false, cursor: 2 }, "AVATAR -> EQUIPMENT");
        assert_eq!(menu_move(ok, true, 2), MenuMove::Open);
        for g in 0..MENU_GAME_ROWS {
            assert_eq!(menu_move(ok, false, g), MenuMove::Pass, "OK on game row {g} is the game's");
            if g != MENU_GAME_EQUIP {
                assert_eq!(menu_move(down, false, g), MenuMove::Pass, "DOWN on {g}");
            }
            if g != MENU_GAME_SKILL {
                assert_eq!(menu_move(up, false, g), MenuMove::Pass, "UP on {g}");
            }
        }
        // A lap each way visits all seven rows once — the game wraps its own six (measured).
        for (key, step, want) in [(down, 1u8, [0u8, 1, 2, 3, 4, 5, 6, 0]), (up, MENU_GAME_ROWS - 1, [0, 6, 5, 4, 3, 2, 1, 0])] {
            let (mut avatar, mut game) = (false, 0u8);
            let mut seen = vec![menu_selected(avatar, game)];
            for _ in 0..7 {
                match menu_move(key, avatar, game) {
                    MenuMove::Select { avatar: a, cursor } => (avatar, game) = (a, cursor),
                    MenuMove::Pass => game = (game + step) % MENU_GAME_ROWS,
                    MenuMove::Open => unreachable!(),
                }
                seen.push(menu_selected(avatar, game));
            }
            assert_eq!(seen, want, "{key:?}");
        }
    }

    #[test]
    fn menu_taps_focus_then_open() {
        assert_eq!(menu_row_tap(3, false, 0), Some(MenuTap::Focus { avatar: true, cursor: 2 }));
        assert_eq!(menu_row_tap(3, true, 2), Some(MenuTap::Activate { avatar: true, cursor: 2 }));
        assert_eq!(menu_row_tap(4, true, 2), Some(MenuTap::Focus { avatar: false, cursor: 3 }));
        assert_eq!(
            menu_row_tap(2, true, 2),
            Some(MenuTap::Focus { avatar: false, cursor: 2 }),
            "EQUIPMENT while AVATAR is selected is a focus, not EQUIPMENT's OK"
        );
        assert_eq!(menu_row_tap(6, false, 5), Some(MenuTap::Activate { avatar: false, cursor: 5 }));
        assert_eq!(menu_row_tap(1, false, 4), Some(MenuTap::Focus { avatar: false, cursor: 1 }));
        assert_eq!(menu_row_tap(MENU_ROWS, false, 0), None);
    }

    #[test]
    fn the_ui_draws_what_the_worker_proposes() {
        assert_eq!(MenuSel::Off.row(2), None, "no AVATAR row: the game's six stay");
        assert_eq!(MenuSel::Game.row(3), Some(4), "the game's SKILL is our fifth row");
        assert_eq!(MenuSel::Row(3).row(2), Some(3), "AVATAR over the game's EQUIPMENT");
    }

    #[test]
    fn the_weapon_list_follows_the_equipped_category() {
        let ian = set_with(Acquire::Clear(0));
        let w = Wardrobe { own: av::with_owned([0; av::OWN_BYTES], ian.id), ..Wardrobe::default() };
        let mut c = ctx();
        c.weapon_cat = Some(6);
        let knife = entries(av::SLOT_WEAPON, &w, &c);
        let row = knife.iter().find(|e| e.set == ian.id).expect("listed for a 나이프 user");
        assert_eq!(row.name, piece_name(ian, av::SLOT_WEAPON, Some(6)));
        // The jar has no 나이프 sheet for it (group 6): not offered.
        c.sheets[6] = 0;
        assert!(!entries(av::SLOT_WEAPON, &w, &c).iter().any(|e| e.set == ian.id));
        // No weapon in hand: no category, no costume weapons.
        c.weapon_cat = None;
        assert_eq!(entries(av::SLOT_WEAPON, &w, &c).len(), 1);
    }

    #[test]
    fn only_the_spear_classes_get_the_shield_tab() {
        let mut c = ctx();
        for class in 0..10u8 {
            c.class = Some(class);
            assert_eq!(slot_open(av::SLOT_SHIELD, &c), matches!(class, 0 | 3 | 8), "class {class}");
            assert!(slot_open(av::SLOT_ARMOR, &c));
        }
        c.class = None;
        assert!(!slot_open(av::SLOT_SHIELD, &c), "unknown class: no shield tab");
    }

    #[test]
    fn moving_tries_on_confirm_wears_and_coming_back_drops_the_try_on() {
        let w = Wardrobe::default();
        let c = ctx();
        let mut p = Page::open(&w, &c);
        assert_eq!((p.slot, p.cursor, p.preview), (av::SLOT_HELMET, 0, None));
        let o = p.act(Act::Down, &w, &c);
        let starter = set_with(Acquire::Starter).id;
        assert_eq!(o.preview, Some((av::SLOT_HELMET, starter)));
        assert_eq!(p.preview, Some(starter));
        let back = p.act(Act::Up, &w, &c);
        assert_eq!(back.drop_preview, Some(av::SLOT_HELMET), "back on the worn row: the knob goes");
        assert_eq!(p.preview, None);
        p.act(Act::Down, &w, &c);
        let worn = p.act(Act::Confirm, &w, &c);
        assert_eq!(worn.wear, Some((av::SLOT_HELMET, starter)));
        assert_eq!(p.preview, None);
        // Confirming what the slot already wears writes nothing.
        let w2 = Wardrobe { wear: [starter, 0, 0, 0, 0, 0], ..w };
        assert_eq!(p.act(Act::Confirm, &w2, &c), Outcome::default());
        // ★ 169차: the list is as long as the catalog makes it — walk to its end first.
        let last = entries(av::SLOT_HELMET, &w2, &c).len() - 1;
        while p.cursor < last {
            p.act(Act::Down, &w2, &c);
        }
        assert_eq!(p.act(Act::Down, &w2, &c), Outcome::default(), "the last row clamps");
    }

    #[test]
    fn a_second_tap_on_the_focused_row_wears_it() {
        let w = Wardrobe::default();
        let c = ctx();
        let mut p = Page::open(&w, &c);
        let first = p.act(Act::Row(1), &w, &c);
        assert!(first.preview.is_some() && first.wear.is_none(), "1st tap = focus (try on)");
        let second = p.act(Act::Row(1), &w, &c);
        assert!(second.wear.is_some(), "2nd tap = wear");
        assert_eq!(p.act(Act::Row(ROWS), &w, &c), Outcome::default(), "past the visible rows");
        let n = entries(av::SLOT_HELMET, &w, &c).len();
        if n < ROWS {
            assert_eq!(p.act(Act::Row(n), &w, &c), Outcome::default(), "past the list");
        }
    }

    #[test]
    fn switching_slots_drops_the_try_on_and_lands_on_the_worn_row() {
        let starter = set_with(Acquire::Starter).id;
        let w = Wardrobe { wear: [0, starter, 0, 0, 0, 0], ..Wardrobe::default() };
        let mut c = ctx();
        c.class = Some(5); // no shield tab
        let mut p = Page::open(&w, &c);
        p.act(Act::Down, &w, &c);
        let o = p.act(Act::NextSlot, &w, &c);
        assert_eq!(o.drop_preview, Some(av::SLOT_HELMET));
        assert_eq!(p.slot, av::SLOT_ARMOR);
        assert_eq!(entries(av::SLOT_ARMOR, &w, &c)[p.cursor].set, starter, "the cursor starts on what it wears");
        // Backwards from the helmet skips the closed shield tab and wraps to the weapon.
        let mut q = Page::open(&w, &c);
        q.act(Act::PrevSlot, &w, &c);
        assert_eq!(q.slot, av::SLOT_WEAPON);
        // A tap on a closed tab does nothing.
        assert_eq!(q.act(Act::Slot(av::SLOT_SHIELD), &w, &c), Outcome::default());
        assert_eq!(q.slot, av::SLOT_WEAPON);
    }

    #[test]
    fn leaving_names_the_key_and_drops_the_try_on() {
        let w = Wardrobe::default();
        let c = ctx();
        for (act, then) in [(Act::Close, Then::Clr), (Act::Tab1, Then::Stay), (Act::Tab3, Then::Num3)] {
            let mut p = Page::open(&w, &c);
            p.act(Act::Down, &w, &c);
            let o = p.act(act, &w, &c);
            assert_eq!(o.leave, Some(then));
            assert_eq!(o.drop_preview, Some(av::SLOT_HELMET));
        }
        let mut p = Page::open(&w, &c);
        assert_eq!(p.act(Act::Close, &w, &c).drop_preview, None, "no try-on, nothing to drop");
    }

    #[test]
    fn the_view_says_what_the_game_will_do() {
        let starter = set_with(Acquire::Starter);
        let w = Wardrobe { wear: [starter.id, 0, 0, 0, 0, 0], ..Wardrobe::default() };
        let mut c = ctx();
        let mut p = Page::open(&w, &c);
        let v = p.view(&w, &c);
        assert_eq!(v.status, "착용 중");
        assert_eq!(v.name, starter.pieces[av::SLOT_HELMET]);
        assert_eq!(v.acquire, "기본 지급");
        assert!(v.rows.iter().any(|r| r.worn && r.focused));
        assert_eq!(v.dressed, [true, false, false, false, false, false]);
        p.act(Act::Up, &w, &c);
        let v = p.view(&w, &c);
        assert_eq!(v.name, BASIC_NAME);
        assert_eq!(v.status, "미리보기 중  OK 입기");
        // 170차: a 투구 costume is drawn on the bare head — no "wear one first" note.
        c.equipped[av::SLOT_HELMET] = false;
        assert_eq!(p.view(&w, &c).status, "미리보기 중  OK 입기");
        // The weapon still shows only over a worn one.
        c.equipped[av::SLOT_WEAPON] = false;
        let mut q = Page::open(&w, &c);
        q.act(Act::Slot(av::SLOT_WEAPON), &w, &c);
        assert_eq!(q.view(&w, &c).status, "무기를 착용해야 보입니다");
        let n = entries(av::SLOT_HELMET, &w, &c).len();
        assert_eq!(v.page, (1, n.div_ceil(ROWS)), "page 1 of as many as the list fills");
    }

    #[test]
    fn particles() {
        assert_eq!(object_particle("투구"), "를");
        assert_eq!(object_particle("갑옷"), "을");
        assert_eq!(object_particle("무기"), "를");
        assert_eq!(object_particle("방패"), "를");
    }

    #[test]
    fn rewards_by_difficulty() {
        assert!(clear_rewards(0).all(|s| s.acquire == Acquire::Clear(0)));
        assert!(clear_rewards(0).count() >= 1, "노말 has a reward");
        assert!(clear_rewards(1).count() >= 1, "하드코어 has a reward");
        assert_eq!(reward_notice(set_with(Acquire::Clear(0))), format!("아바타 획득: {}", set_with(Acquire::Clear(0)).name));
    }

    /// ★ 176차: the test-grant file — `slot N` lines, hex and 0-based like the other trigger
    /// files, comments and junk ignored, a slot named twice counted once.
    #[test]
    fn the_test_grant_file_names_save_slots() {
        assert_eq!(parse_test_grant("slot 0\nslot 1\n"), [0, 1]);
        assert_eq!(parse_test_grant("# S25: Kei, Ritz\nslot 0   # SLOT 1\r\nslot 0x01\nslot 0\n"), [0, 1]);
        assert_eq!(parse_test_grant("slot a\n"), [10], "hex like wie_grant.txt");
        assert!(parse_test_grant("").is_empty());
        assert!(parse_test_grant("slot\nslot zz\ncat 0 idx 1\nslot 1 2\n").is_empty());
    }

    #[test]
    fn a_granted_slot_leaves_the_file_and_the_last_one_empties_it() {
        let text = "# test\nslot 0\nslot 1\n";
        let rest = test_grant_without(text, 0).expect("slot 0 is named");
        assert_eq!(rest, "# test\nslot 1\n");
        assert_eq!(parse_test_grant(&rest), [1]);
        assert_eq!(test_grant_without(text, 2), None, "a slot the file does not name changes nothing");
        let last = test_grant_without(&rest, 1).expect("slot 1 is named");
        assert!(parse_test_grant(&last).is_empty(), "nothing left: the host deletes the file");
    }

    #[test]
    fn the_test_grant_owns_every_catalog_set_and_nothing_else() {
        let own = own_every_set([0; av::OWN_BYTES]);
        for s in SETS {
            assert!(av::owns(&own, s.id), "set {} ({})", s.id, s.name);
        }
        for id in 1..=av::SET_MAX {
            if set_def(id).is_none() {
                assert!(!av::owns(&own, id), "set {id} is not in the catalog (a retired number stays off)");
            }
        }
        assert_eq!(own_every_set(own), own, "idempotent");
        // Bits already set stay set, whatever they are.
        let mut odd = [0u8; av::OWN_BYTES];
        odd[av::OWN_BYTES - 1] = 0x80;
        assert_eq!(own_every_set(odd)[av::OWN_BYTES - 1] & 0x80, 0x80);
        // Owning every set offers every piece the hero may wear.
        let w = Wardrobe { wear: [0; av::SLOTS], own };
        let c = ctx();
        for s in SETS.iter().filter(|s| offered(s, &c)) {
            assert!(w.owns(s, c.difficulty), "{}", s.name);
        }
    }

    /// Every string the page or its notice draws is one the GAME FONT can draw: KS X 1001
    /// syllables and printable ASCII only — anything else is silently skipped by the drawer.
    #[test]
    fn every_string_the_page_draws_is_drawable() {
        let drawable = |s: &str| s.chars().all(|c| (' '..='~').contains(&c) || crate::gamefont::wansung_index(c).is_some());
        let mut all: Vec<String> = vec![BASIC_NAME.into(), BASIC_DESC.into()];
        all.extend(SLOT_NAMES.iter().map(|s| s.to_string()));
        all.extend(crate::avatar_page::INFO_LABELS.iter().map(|s| s.to_string()));
        for (eng, kor) in crate::avatar_page::MENU_LABELS {
            all.push(eng.into());
            all.push(kor.into());
        }
        for a in [Acquire::Starter, Acquire::Clear(0), Acquire::Clear(1), Acquire::Clear(2)] {
            all.push(acquire_text(a).into());
        }
        for s in SETS {
            all.push(reward_notice(s));
        }
        all.push(TEST_GRANT_NOTE.into());
        // Every status line the view can produce.
        let w = Wardrobe::default();
        for slot in 0..av::SLOTS {
            for equipped in [true, false] {
                let mut c = ctx();
                c.class = Some(0);
                c.equipped = [equipped; av::SLOTS];
                let mut p = Page::open(&w, &c);
                p.act(Act::Slot(slot), &w, &c);
                all.push(p.view(&w, &c).status);
                p.act(Act::Down, &w, &c);
                all.push(p.view(&w, &c).status);
            }
        }
        for s in &all {
            assert!(drawable(s), "{s:?} has a glyph the game font cannot draw");
        }
    }

    #[test]
    fn the_weapon_icon_is_its_category() {
        let mut c = ctx();
        c.weapon_cat = Some(9);
        assert_eq!(slot_icon(av::SLOT_WEAPON, &c), 9);
        c.weapon_cat = None;
        assert_eq!(slot_icon(av::SLOT_WEAPON, &c), SLOT_ICON[av::SLOT_WEAPON]);
        assert_eq!(slot_icon(av::SLOT_SHIELD, &c), 11);
    }
}

/// The page, drawn (`avatar_page.rs`). The pixel facts these pin were read off the game's own
/// EQUIPMENT / STATUS / INVENTORY frames (`wie_probe --dump`, 496x240, 167차).
#[cfg(test)]
mod page_tests {
    use crate::avatar::*;
    use crate::avatar_page::*;
    use crate::gamefont;

    fn view(rows: usize, focused: usize, worn: usize, slot: usize) -> View {
        View {
            slot,
            open: [true, true, true, true, true, false],
            dressed: [false, true, false, false, false, false],
            icon: [0, 1, 2, 3, 6, 11],
            rows: (0..rows)
                .map(|i| RowView { name: format!("행 {i}"), worn: i == worn, focused: i == focused })
                .collect(),
            page: (1, 1),
            name: "이름".into(),
            desc: "설명".into(),
            acquire: "기본 지급".into(),
            status: "착용 중".into(),
        }
    }

    fn compose_flat(v: &View, fill: u32) -> Vec<u32> {
        let game = vec![0xff00_0000 | fill; W * H];
        let mut out = vec![0; W * H];
        let icon_px = vec![0xffd0_4040u32; 15 * 15];
        let icons: [Option<Icon>; 6] = core::array::from_fn(|_| Some(Icon { w: 15, h: 15, px: &icon_px }));
        assert!(compose(&game, v, &icons, &mut out));
        out
    }

    fn at(out: &[u32], x: i32, y: i32) -> u32 {
        out[y as usize * W + x as usize] & 0xff_ffff
    }

    #[test]
    fn a_wrong_size_draws_nothing() {
        let v = view(2, 0, 0, 0);
        let mut out = vec![0; W * H];
        let icons: [Option<Icon>; 6] = [None, None, None, None, None, None];
        assert!(!compose(&vec![0; W * H - 1], &v, &icons, &mut out));
        assert!(!compose(&vec![0; W * H], &v, &icons, &mut vec![0; 3]));
        assert!(out.iter().all(|&p| p == 0));
    }

    #[test]
    fn the_title_is_centred_the_way_the_game_centres_it() {
        // The game's own titles: INVENTORY starts at x 51, STATUS at x 63 (measured).
        assert_eq!(TITLE_CENTRE - gamefont::game_width("INVENTORY") as i32 / 2, 51);
        assert_eq!(TITLE_CENTRE - gamefont::game_width("STATUS") as i32 / 2, 63);
        let out = compose_flat(&view(1, 0, 0, 0), HDR);
        let x0 = TITLE_CENTRE - gamefont::game_width("AVATAR") as i32 / 2;
        assert_eq!(x0, 63);
        let ink: Vec<i32> = (TITLE_CLEAR.0..=TITLE_CLEAR.2).filter(|&x| at(&out, x, 4) == TITLE).collect();
        assert!(!ink.is_empty() && ink[0] >= x0 && *ink.last().unwrap() < x0 + 48, "{ink:?}");
        assert!((0..TITLE_Y + 2).all(|y| (TITLE_CLEAR.0..=TITLE_CLEAR.2).all(|x| at(&out, x, y) != TITLE)));
    }

    #[test]
    fn the_plates_are_the_games_own() {
        // «7» and «9» as the game draws them on INVENTORY's bottom bar (`#` = the plate ink).
        let seven = [
            ".########..", "###########", "##.......##", "##..###..##", "#######..##", "######..###",
            "######..###", "#####..####", "#####..####", ".#########.", ".########..",
        ];
        let nine = [
            ".########..", "###########", "###.....###", "##..###..##", "##..###..##", "###......##",
            "#######..##", "##.####..##", "###.....###", ".#########.", ".########..",
        ];
        let out = compose_flat(&view(1, 0, 0, 0), HDR);
        for (x0, rows) in [(PLATE_PREV_X, seven), (PLATE_NEXT_X, nine)] {
            for (r, row) in rows.iter().enumerate() {
                for (c, ch) in row.chars().enumerate() {
                    let p = at(&out, x0 + c as i32, PLATE_Y + r as i32);
                    assert_eq!(p == BADGE, ch == '#', "plate at x {x0}, row {r} col {c}");
                }
            }
        }
    }

    #[test]
    fn the_chevrons_are_the_games_own() {
        let out = compose_flat(&view(1, 0, 0, 0), HDR);
        let lit = |x: i32, y: i32| at(&out, x, y) == CHEVRON;
        // `<<` five rows from plate top + 3: the tip column is x 5 / x 9.
        assert!(lit(5, PLATE_Y + 5) && lit(9, PLATE_Y + 5) && lit(7, PLATE_Y + 3) && lit(11, PLATE_Y + 7));
        // `>>` seven rows from plate top + 2, its tip row empty.
        assert!(lit(164, PLATE_Y + 2) && lit(170, PLATE_Y + 4) && !lit(170, PLATE_Y + 5) && lit(164, PLATE_Y + 8));
    }

    #[test]
    fn the_figure_is_the_games_box_without_its_corners() {
        let mut game = vec![0xff00_0000 | HDR; W * H];
        for j in 0..FIG_H {
            for i in 0..FIG_W {
                game[((FIG_SRC.1 + j) as usize) * W + (FIG_SRC.0 + i) as usize] = 0xff12_3456 + (i + j * 64) as u32;
            }
        }
        let mut out = vec![0; W * H];
        let icons: [Option<Icon>; 6] = [None, None, None, None, None, None];
        assert!(compose(&game, &view(1, 0, 0, 0), &icons, &mut out));
        assert_eq!(at(&out, FIG_DST.0 + 5, FIG_DST.1 + 7), 0x12_3456 + 5 + 7 * 64);
        assert_eq!(at(&out, FIG_DST.0, FIG_DST.1), checker(FIG_DST.0, FIG_DST.1), "the chamfered corner keeps the checker");
        let (cx, cy) = (FIG_DST.0 + FIG_W - 1, FIG_DST.1 + FIG_H - 1);
        assert_eq!(at(&out, cx, cy), checker(cx, cy));
    }

    #[test]
    fn the_checker_phase_is_the_sections_own() {
        // EQUIPMENT row 16: x 2,3 = #948a7b, x 4,5 = #9c928c; row 18 flips; x 172 on row 16 = #9c928c.
        assert_eq!(
            (checker(2, 16), checker(4, 16), checker(2, 18), checker(2, 20), checker(172, 16)),
            (CHECK_A, CHECK_B, CHECK_B, CHECK_A, CHECK_B)
        );
    }

    #[test]
    fn rows_cursor_and_worn_mark() {
        let out = compose_flat(&view(3, 1, 2, 0), HDR);
        let (x0, y0, x1, y1) = row_box(1);
        assert_eq!(at(&out, x0 + 5, y0), CURSOR, "the focused row is framed");
        assert_eq!(at(&out, x1, y1 - 3), CURSOR);
        let (_, y2, _, _) = row_box(2);
        assert_eq!(at(&out, E_X + 2, y2 + 3 + 3), E_FILL, "the worn row carries the green E");
        assert_ne!(at(&out, E_X + 2, row_box(1).1 + 6), E_FILL, "only that row");
        let (_, y3, _, _) = row_box(3);
        assert_ne!(at(&out, x0 + 5, y3 + 5), ROW_FILL, "no fourth row drawn for a three-row list");
    }

    #[test]
    fn the_strip_dims_what_wears_nothing_and_skips_closed_slots() {
        let out = compose_flat(&view(1, 0, 0, 2), HDR);
        let mid = |s: usize| {
            let (x0, y0, _, _) = cell_box(s);
            at(&out, x0 + 8, y0 + 8)
        };
        assert_eq!(mid(1), 0xd0_4040, "dressed: full colour");
        assert_eq!(mid(0), blend(0xd0_4040, CELL_FILL, 40), "not dressed: dimmed");
        assert_eq!(mid(5), CELL_FILL, "closed (no shield tab): empty cell");
        let (x0, y0, _, _) = cell_box(2);
        assert_eq!(at(&out, x0 + 4, y0), CURSOR, "the slot on screen is framed");
    }

    #[test]
    fn taps() {
        assert_eq!(hit(-1, 10), None);
        assert_eq!(hit(10, H as i32), None);
        assert_eq!(hit(20, 7), Some(Hit::Act(Act::Tab1)), "«1»");
        assert_eq!(hit(160, 7), Some(Hit::Act(Act::Tab3)), "«3»");
        assert_eq!(hit(88, 7), Some(Hit::Swallow), "the title");
        let (x0, y0, x1, y1) = row_box(2);
        assert_eq!(hit(x0, y0), Some(Hit::Act(Act::Row(2))));
        assert_eq!(hit(x1, y1), Some(Hit::Act(Act::Row(2))));
        assert_eq!(hit(20, 40), Some(Hit::Swallow), "the figure");
        assert_eq!(hit(100, 95), Some(Hit::Swallow), "the info");
        for s in 0..6 {
            let (x0, y0, x1, y1) = cell_box(s);
            assert_eq!(hit((x0 + x1) / 2, (y0 + y1) / 2), Some(Hit::Act(Act::Slot(s))), "cell {s}");
        }
        assert_eq!(hit(PLATE_PREV_X + 5, PLATE_Y + 5), Some(Hit::Act(Act::PrevSlot)), "«7»");
        assert_eq!(hit(PLATE_NEXT_X + 5, PLATE_Y + 5), Some(Hit::Act(Act::NextSlot)), "«9»");
        assert_eq!(hit(CHEVRON_PREV_X, PLATE_Y + 5), Some(Hit::Act(Act::PrevSlot)), "`<<`");
        assert_eq!(hit(CHEVRON_NEXT_X + 6, PLATE_Y + 4), Some(Hit::Act(Act::NextSlot)), "`>>`");
    }

    /// `wie_probe --dump` (496x240), the title area x 28..148 · y 1..13, `rrggbb:n` row-major.
    const REAL_EQUIPMENT_TITLE: &[&str] = &[
        "211000:386 d6b29c:7 211000:2 d6b29c:5 211000:2 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:6 ",
        "211000:2 d6b29c:6 211000:2 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:7 211000:1 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:1 d6b29c:6 211000:51 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:1 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:1 d6b29c:3 211000:1 d6b29c:3 211000:1 d6b29c:2 211000:6 d6b29c:3 211000:2 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:53 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:1 d6b29c:1 211000:1 d6b29c:2 211000:1 d6b29c:2 211000:6 d6b29c:2 211000:1 d6b29c:1 ",
        "211000:1 d6b29c:2 211000:3 d6b29c:2 211000:53 d6b29c:6 211000:2 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:1 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:6 211000:2 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:1 d6b29c:6 211000:2 d6b29c:2 211000:2 d6b29c:3 211000:3 d6b29c:2 ",
        "211000:53 d6b29c:2 211000:6 d6b29c:2 211000:1 d6b29c:1 211000:1 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:1 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:2 211000:53 d6b29c:2 ",
        "211000:6 d6b29c:2 211000:2 d6b29c:3 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:4 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 211000:6 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:3 d6b29c:2 211000:53 d6b29c:7 211000:2 d6b29c:6 211000:2 d6b29c:5 ",
        "211000:2 d6b29c:6 211000:2 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:7 ",
        "211000:1 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:2 211000:393 ",
    ];
    /// `wie_probe --dump` (496x240), the title area x 28..148 · y 1..13, `rrggbb:n` row-major.
    const REAL_INVENTORY_TITLE: &[&str] = &[
        "211000:386 d6b29c:6 211000:2 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:1 d6b29c:7 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:6 211000:3 d6b29c:5 ",
        "211000:2 d6b29c:6 211000:2 d6b29c:2 211000:3 d6b29c:2 211000:52 d6b29c:2 211000:4 d6b29c:3 ",
        "211000:2 d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 211000:6 d6b29c:3 ",
        "211000:2 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:52 d6b29c:2 211000:4 d6b29c:2 ",
        "211000:1 d6b29c:1 211000:1 d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:6 d6b29c:2 211000:1 d6b29c:1 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:2 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:53 d6b29c:2 211000:4 d6b29c:2 211000:2 d6b29c:3 211000:2 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:2 d6b29c:6 211000:2 d6b29c:2 211000:2 d6b29c:3 211000:3 d6b29c:2 211000:4 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:1 d6b29c:6 211000:4 d6b29c:3 211000:54 d6b29c:2 211000:4 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:2 d6b29c:2 211000:1 d6b29c:2 211000:2 d6b29c:2 211000:6 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:1 d6b29c:2 211000:6 d6b29c:2 211000:54 d6b29c:2 211000:4 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:2 d6b29c:2 211000:1 d6b29c:2 211000:2 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:4 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 211000:2 d6b29c:2 ",
        "211000:5 d6b29c:2 211000:52 d6b29c:6 211000:2 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:3 ",
        "211000:3 d6b29c:7 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:3 d6b29c:2 211000:5 d6b29c:5 ",
        "211000:2 d6b29c:2 211000:3 d6b29c:2 211000:4 d6b29c:2 211000:392 ",
    ];

    fn panel_with_title(strip: &[&str]) -> Vec<u32> {
        let mut px = vec![0xff00_0000 | HDR; W * H];
        let (x0, y0, x1, _) = TITLE_CLEAR;
        let bw = (x1 - x0 + 1) as usize;
        let mut k = 0usize;
        for run in strip.concat().split_whitespace() {
            let (c, n) = run.split_once(':').unwrap();
            let c = u32::from_str_radix(c, 16).unwrap();
            for _ in 0..n.parse::<usize>().unwrap() {
                let (i, j) = (k % bw, k / bw);
                px[(y0 as usize + j) * W + x0 as usize + i] = 0xff00_0000 | c;
                k += 1;
            }
        }
        px
    }

    #[test]
    fn the_page_knows_the_games_equipment_header() {
        assert!(equipment_on_screen(&panel_with_title(REAL_EQUIPMENT_TITLE)), "the real EQUIPMENT header");
        assert!(!equipment_on_screen(&panel_with_title(REAL_INVENTORY_TITLE)), "INVENTORY's is another page");
        assert!(!equipment_on_screen(&vec![0xff00_0000 | HDR; W * H]), "an empty header");
        assert!(!equipment_on_screen(&vec![0; W * H - 1]));
        // One pixel of ink off and it is not the page any more.
        let mut px = panel_with_title(REAL_EQUIPMENT_TITLE);
        let at = (TITLE_Y as usize + 3) * W + 60;
        px[at] ^= 0x00_0101;
        assert!(!equipment_on_screen(&px));
    }

    /// `wie_probe --dump` (496x240) of the game's MENU list: the title area x 28..148 · y 1..13,
    /// the plain STATUS row's box x 24..151 · y 24..38 and the selected 스킬 row's x 24..151 ·
    /// y 81..95, `rrggbb:n` row-major (168차).
    const REAL_MENU_TITLE: &[&str] = &[
        "211000:406 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:7 211000:1 d6b29c:2 211000:3 ",
        "d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:90 d6b29c:3 211000:1 d6b29c:3 ",
        "211000:1 d6b29c:2 211000:6 d6b29c:3 211000:2 d6b29c:2 211000:1 d6b29c:2 211000:3 ",
        "d6b29c:2 211000:90 d6b29c:2 211000:1 d6b29c:1 211000:1 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:6 d6b29c:2 211000:1 d6b29c:1 211000:1 d6b29c:2 211000:1 d6b29c:2 211000:3 ",
        "d6b29c:2 211000:90 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:6 211000:2 d6b29c:2 ",
        "211000:2 d6b29c:3 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:90 d6b29c:2 211000:3 ",
        "d6b29c:2 211000:1 d6b29c:2 211000:6 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:90 d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 211000:6 ",
        "d6b29c:2 211000:3 d6b29c:2 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:90 d6b29c:2 ",
        "211000:3 d6b29c:2 211000:1 d6b29c:7 211000:1 d6b29c:2 211000:3 d6b29c:2 211000:2 ",
        "d6b29c:5 211000:411 ",
    ];
    const REAL_MENU_STATUS_ROW: &[&str] = &[
        "9c928c:1 cecac5:126 948a7b:1 cecac5:425 292010:5 cecac5:2 292010:6 cecac5:3 292010:5 ",
        "cecac5:2 292010:6 cecac5:2 292010:2 cecac5:3 292010:2 cecac5:2 292010:5 cecac5:82 ",
        "292010:2 cecac5:4 292010:1 cecac5:3 292010:2 cecac5:4 292010:2 cecac5:3 292010:2 ",
        "cecac5:3 292010:2 cecac5:4 292010:2 cecac5:3 292010:2 cecac5:1 292010:2 cecac5:4 ",
        "292010:1 cecac5:81 292010:2 cecac5:8 292010:2 cecac5:4 292010:2 cecac5:3 292010:2 ",
        "cecac5:3 292010:2 cecac5:4 292010:2 cecac5:3 292010:2 cecac5:1 292010:2 cecac5:87 ",
        "292010:5 cecac5:4 292010:2 cecac5:4 292010:7 cecac5:3 292010:2 cecac5:4 292010:2 ",
        "cecac5:3 292010:2 cecac5:2 292010:5 cecac5:87 292010:2 cecac5:3 292010:2 cecac5:4 ",
        "292010:2 cecac5:3 292010:2 cecac5:3 292010:2 cecac5:4 292010:2 cecac5:3 292010:2 ",
        "cecac5:6 292010:2 cecac5:81 292010:1 cecac5:4 292010:2 cecac5:3 292010:2 cecac5:4 ",
        "292010:2 cecac5:3 292010:2 cecac5:3 292010:2 cecac5:4 292010:2 cecac5:3 292010:2 ",
        "cecac5:1 292010:1 cecac5:4 292010:2 cecac5:82 292010:5 cecac5:4 292010:2 cecac5:4 ",
        "292010:2 cecac5:3 292010:2 cecac5:3 292010:2 cecac5:5 292010:5 cecac5:3 292010:5 ",
        "cecac5:426 948a7b:1 cecac5:126 9c928c:1 ",
    ];
    const REAL_MENU_SKILL_ROW_SELECTED: &[&str] = &[
        "9c928c:1 311c10:126 948a7b:1 311c10:186 ffffff:1 311c10:6 ffffff:5 311c10:3 ffffff:1 ",
        "311c10:112 ffffff:1 311c10:10 ffffff:1 311c10:3 ffffff:1 311c10:112 ffffff:1 311c10:6 ",
        "ffffff:5 311c10:3 ffffff:1 311c10:111 ffffff:1 311c10:1 ffffff:1 311c10:8 ffffff:1 ",
        "311c10:4 ffffff:1 311c10:109 ffffff:2 311c10:3 ffffff:2 311c10:3 ffffff:3 311c10:5 ",
        "ffffff:1 311c10:127 ffffff:1 311c10:120 ffffff:8 311c10:127 ffffff:1 311c10:108 ffffff:9 ",
        "311c10:3 ffffff:8 311c10:120 ffffff:1 311c10:127 ffffff:8 311c10:182 948a7b:1 311c10:126 ",
        "9c928c:1 ",
    ];

    /// Decode a `rrggbb:n` strip of a `w`-wide box into its rows.
    fn decode(strip: &[&str], w: usize) -> Vec<Vec<u32>> {
        let mut flat = Vec::new();
        for run in strip.concat().split_whitespace() {
            let (c, n) = run.split_once(':').unwrap();
            let c = u32::from_str_radix(c, 16).unwrap();
            flat.extend(std::iter::repeat_n(c, n.parse().unwrap()));
        }
        flat.chunks(w).map(<[u32]>::to_vec).collect()
    }

    /// The game's MENU panel: the real title, the checker inside, the six rows pasted from the
    /// real ones (`sel` selected).
    fn game_menu_panel(sel: usize) -> Vec<u32> {
        let mut px = panel_with_title(REAL_MENU_TITLE);
        let (x0, y0, x1, y1) = MENU_INSIDE;
        for y in y0..=y1 {
            for x in x0..=x1 {
                px[y as usize * W + x as usize] = 0xff00_0000 | checker(x, y);
            }
        }
        let (plain, on) = (decode(REAL_MENU_STATUS_ROW, 128), decode(REAL_MENU_SKILL_ROW_SELECTED, 128));
        for i in 0..6 {
            let top = (MENU_GAME_Y0 + MENU_GAME_PITCH * i as i32) as usize;
            for (j, row) in (if i == sel { &on } else { &plain }).iter().enumerate() {
                for (k, &c) in row.iter().enumerate() {
                    px[(top + j) * W + MENU_X0 as usize + k] = 0xff00_0000 | c;
                }
            }
        }
        px
    }

    #[test]
    fn the_menu_header_is_the_games() {
        let px = panel_with_title(REAL_MENU_TITLE);
        assert!(title_on_screen(&px, "MENU"));
        assert!(!equipment_on_screen(&px));
        assert!(!title_on_screen(&panel_with_title(REAL_EQUIPMENT_TITLE), "MENU"));
        assert_eq!(TITLE_CENTRE - gamefont::game_width("MENU") as i32 / 2, 71, "measured: MENU starts at x 71");
    }

    #[test]
    fn menu_on_screen_reads_the_games_six_rows() {
        for sel in 0..6 {
            assert_eq!(menu_on_screen(&game_menu_panel(sel)), Some(sel as u8));
        }
        let set = |px: &mut Vec<u32>, x: i32, y: i32, c: u32| px[y as usize * W + x as usize] = 0xff00_0000 | c;
        let top = |i: i32| MENU_GAME_Y0 + MENU_GAME_PITCH * i;
        // Two rows selected.
        let mut two = game_menu_panel(1);
        for (x, dy) in [(26, 7), (40, 3), (150, 7), (26, MENU_ROW_H - 2)] {
            set(&mut two, x, top(4) + dy, MENU_SEL_FILL);
        }
        assert_eq!(menu_on_screen(&two), None);
        // A gap filled (not the six-row layout), a row covered, a title off by one pixel.
        let mut gap = game_menu_panel(0);
        set(&mut gap, 30, top(2) - 2, MENU_ROW_FILL);
        assert_eq!(menu_on_screen(&gap), None);
        let mut cover = game_menu_panel(0);
        set(&mut cover, 150, top(3) + 7, 0x000000);
        assert_eq!(menu_on_screen(&cover), None);
        let mut title = game_menu_panel(0);
        let at = (TITLE_Y as usize + 3) * W + 75;
        title[at] ^= 0x00_0101;
        assert_eq!(menu_on_screen(&title), None);
        assert_eq!(menu_on_screen(&panel_with_title(REAL_EQUIPMENT_TITLE)), None);
        assert_eq!(menu_on_screen(&vec![0; W * H - 1]), None);
    }

    /// Our rows are the game's rows, moved: the plain STATUS row and the selected 스킬 row
    /// composed at their seven-row places are pixel for pixel the game's own (corners aside —
    /// they are the checker under them).
    #[test]
    fn the_seven_rows_are_drawn_the_games_way() {
        let mut out = vec![0; W * H];
        assert!(compose_menu(&game_menu_panel(3), 4, &mut out));
        let (plain, on) = (decode(REAL_MENU_STATUS_ROW, 128), decode(REAL_MENU_SKILL_ROW_SELECTED, 128));
        for (row, want) in [(0usize, &plain), (4, &on)] {
            let (x0, y0, _, _) = menu_row_box(row);
            for (j, r) in want.iter().enumerate() {
                for (k, &c) in r.iter().enumerate() {
                    if (j == 0 || j == 14) && (k == 0 || k == 127) {
                        continue;
                    }
                    assert_eq!(at(&out, x0 + k as i32, y0 + j as i32), c, "row {row} at ({k},{j})");
                }
            }
        }
        // AVATAR, plain and selected, centred like the game's labels.
        for sel in [2u8, 3] {
            let mut out = vec![0; W * H];
            assert!(compose_menu(&game_menu_panel(0), sel, &mut out));
            let (x0, y0, x1, y1) = menu_row_box(3);
            let ink = if sel == 3 { MENU_SEL_INK } else { MENU_ROW_INK };
            let xs: Vec<i32> = (x0..=x1).filter(|&x| (y0..=y1).any(|y| at(&out, x, y) == ink)).collect();
            let (a, b) = (xs[0], *xs.last().unwrap());
            assert!((a + b - 2 * MENU_LABEL_CENTRE).abs() <= 2, "AVATAR row centred, ink {a}..{b}");
            assert_eq!(at(&out, x0 + 2, y0 + 7), if sel == 3 { MENU_SEL_FILL } else { MENU_ROW_FILL });
        }
        // The header and the footer are the game's.
        assert_eq!(at(&out, 88, 4), at(&game_menu_panel(3), 88, 4));
        assert_eq!(at(&out, 3, 150), at(&game_menu_panel(3), 3, 150));
        assert!(!compose_menu(&game_menu_panel(0), MENU_ROWS, &mut out), "past the seven rows");
    }

    #[test]
    fn the_seven_rows_fit_and_taps_split_the_gaps() {
        let (_, y0, _, y1) = MENU_INSIDE;
        assert_eq!(menu_row_box(0).1 - y0, y1 - menu_row_box(6).3, "the same clearance above and below");
        assert!(menu_row_box(0).1 > y0 && menu_row_box(6).3 < y1);
        for i in 0..7 {
            let (bx0, by0, bx1, by1) = menu_row_box(i);
            assert_eq!(menu_hit(bx0, by0), Some(Some(i as u8)));
            assert_eq!(menu_hit(bx1, by1), Some(Some(i as u8)));
        }
        let g = menu_row_box(2).3;
        assert_eq!((menu_hit(88, g + 1), menu_hit(88, g + 2)), (Some(Some(2)), Some(Some(3))), "the 2 px gap is split");
        assert_eq!(menu_hit(88, y0), Some(Some(0)));
        assert_eq!(menu_hit(88, y1), Some(Some(6)));
        assert_eq!(menu_hit(88, 5), Some(None), "the header is swallowed");
        assert_eq!(menu_hit(88, 150), Some(None), "the footer");
        assert_eq!(menu_hit(3, 60), Some(None), "the frame");
        assert_eq!(menu_hit(-1, 60), None, "outside the panel");
    }

    #[test]
    fn the_layout_stays_inside_its_sections() {
        let (_, _, _, last) = row_box(ROWS - 1);
        assert!(last <= CHECKER.3, "the list ends inside the checker section");
        assert!(FIG_DST.0 + FIG_W - 1 < LIST_X0, "the figure and the list do not overlap");
        assert!(INFO_Y[2] + INFO_H - 1 <= INFO.3);
        let (_, _, x5, y5) = cell_box(5);
        assert!(x5 < PLATE_NEXT_X && cell_box(0).0 > PLATE_PREV_X + PLATE && y5 <= BOTTOM.3);
        // The widest list name the catalog tool allows (96 px) clears the worn mark…
        assert!(NAME_X + 96 <= E_X - 2);
        // …and the widest info value (143 px) fits its box.
        assert!(VALUE_X + 143 <= INFO_BOX_X1);
    }
}
