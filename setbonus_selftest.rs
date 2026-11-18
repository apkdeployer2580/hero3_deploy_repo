// rustc --edition 2024 --test setbonus_selftest.rs -o t.exe; .\t.exe
//
// 142차 / 142차-C / 150차 — the set rules (85제 three + 90제 three) (`wie_android/src/setbonus.rs`)
// against the GENERATED piece table (`wie_android/src/set_items.rs`, `tools_re/gen_set_items.py
// --check`). 150차: ANY FOUR distinct pieces make the effect, and the HUD lines carry the
// two timed accessory buffs. 154차: a set is four to six pieces — 크루세이더 five (no accessory),
// 레아의목걸이 붉은마녀's, 굴베이그의탈리스만 no set piece. 155차: 템플러 · 코르버스 five too (no accessory),
// 이안의맹세 is 빙해's, 클레르의가보 · 닉스의펜던트 no set piece.

mod setbonus {
    include!("wie_android/src/setbonus.rs");
}
mod set_items {
    include!("wie_android/src/set_items.rs");
}

use set_items::SET_ITEMS;
use setbonus::*;

fn pieces(sid: u8) -> Vec<(u16, u16)> {
    SET_ITEMS.iter().filter(|p| p.2 == sid).map(|p| (p.0, p.1)).collect()
}

/// Every `k`-subset of `items`, in index order.
fn subsets(items: &[(u16, u16)], k: usize) -> Vec<Vec<(u16, u16)>> {
    let n = items.len();
    (0u32..1 << n)
        .filter(|m| m.count_ones() as usize == k)
        .map(|m| (0..n).filter(|i| m & (1 << i) != 0).map(|i| items[i]).collect())
        .collect()
}

/// ★ 154차 (user, 2026-10-02: 「4or5or6 이야 7도아니고 3도아니고」): every set is FOUR to SIX
/// pieces, at most one per slot kind (투구 · 갑옷 · 장갑 · 신발 · 방패 · 장신구), and never fewer
/// than the effect needs. Today (155차): 템플러 · 크루세이더 · 코르버스 five (no accessory), the other
/// three sets six.
#[test]
fn every_set_has_four_to_six_pieces_one_per_slot() {
    for sid in 0..SET_COUNT as u8 {
        let mut cats: Vec<u16> = pieces(sid).iter().map(|p| p.0).collect();
        cats.sort_unstable();
        let n = cats.len();
        assert!((4..=6).contains(&n), "set {sid} has {n} pieces");
        assert!(n >= usize::from(PIECES_FOR_BONUS), "set {sid} could never turn on");
        cats.dedup();
        assert_eq!(cats.len(), n, "set {sid}: two pieces in one slot kind");
        assert!(cats.iter().all(|c| [0x00, 0x01, 0x02, 0x03, 0x0b, 0x0c].contains(c)), "set {sid}: {cats:x?}");
        let want: &[u16] = if [0, 1, 3].contains(&sid) { &[0x00, 0x01, 0x02, 0x03, 0x0b] } else { &[0x00, 0x01, 0x02, 0x03, 0x0b, 0x0c] };
        assert_eq!(cats, want, "set {sid}");
    }
    assert_eq!(SET_ITEMS.len(), 3 * 6 + 3 * 5, "155차: three six-piece sets, three five-piece sets");
}

/// ★ 150차 (user, 2026-10-01: 「6개중 4개」): ANY four distinct pieces of a set make the
/// effect — every four-piece subset (15 of six, 5 of five), shield and accessory included or
/// not — and every larger one, and none of the three-piece subsets.
#[test]
fn any_four_pieces_make_the_effect_and_three_do_not() {
    assert_eq!(PIECES_FOR_BONUS, 4);
    let choose = |n: usize, k: usize| (0..k).fold(1usize, |a, i| a * (n - i) / (i + 1));
    for sid in 0..SET_COUNT as u8 {
        let bit = SETS[sid as usize].bit;
        let p = pieces(sid);
        let n = p.len();
        let four = subsets(&p, 4);
        assert_eq!(four.len(), choose(n, 4), "set {sid}");
        for s in &four {
            assert_eq!(active_mask(s, SET_ITEMS), bit, "set {sid}: {s:x?} is four pieces");
        }
        for k in 5..=n {
            for s in subsets(&p, k) {
                assert_eq!(active_mask(&s, SET_ITEMS), bit, "set {sid}: {s:x?}");
            }
        }
        let three = subsets(&p, 3);
        assert_eq!(three.len(), choose(n, 3), "set {sid}");
        for s in &three {
            assert_eq!(active_mask(s, SET_ITEMS), 0, "set {sid}: {s:x?} is only three");
        }
        // the routes the 142차 rule named still work at four: armour + shield, armour + accessory
        let armour3: Vec<_> = p.iter().copied().filter(|x| x.0 <= 0x03).take(3).collect();
        let shield = *p.iter().find(|x| x.0 == 0x0b).unwrap();
        assert_eq!(active_mask(&[armour3.clone(), vec![shield]].concat(), SET_ITEMS), bit, "set {sid}: 3 armour + shield");
        if let Some(&acc) = p.iter().find(|x| x.0 == 0x0c) {
            assert_eq!(active_mask(&[armour3, vec![acc]].concat(), SET_ITEMS), bit, "set {sid}: 3 armour + accessory");
        }
    }
    // the five-piece sets (155차: 템플러 · 크루세이더 · 코르버스 — no accessory): the four armour
    // pieces alone, or three + the shield
    for sid in [0u8, 1, 3] {
        let c = pieces(sid);
        assert_eq!(c.len(), 5, "set {sid}");
        assert_eq!(active_mask(&c[..4], SET_ITEMS), SETS[sid as usize].bit, "set {sid}: 투구 · 갑옷 · 장갑 · 신발");
    }
}

/// Pieces of different sets do not add up, a duplicated accessory counts once, and
/// anything that is not a set piece counts for nothing. (155차: the accessory half runs on
/// 빙해 — 템플러 has no accessory any more.)
#[test]
fn only_distinct_pieces_of_one_set_count() {
    let t = pieces(0);
    let c = pieces(1);
    let mixed = vec![t[0], t[1], t[2], c[2], c[3], c[4]];
    assert_eq!(piece_counts(&mixed, SET_ITEMS), [3, 3, 0, 0, 0, 0]);
    assert_eq!(active_mask(&mixed, SET_ITEMS), 0);
    let i = pieces(5);
    let acc = *i.iter().find(|x| x.0 == 0x0c).expect("빙해 has an accessory (이안의맹세)");
    let armour: Vec<_> = i.iter().copied().filter(|x| x.0 != 0x0b && x.0 != 0x0c).collect();
    let dup: Vec<_> = armour.iter().copied().chain([acc, acc]).collect();
    assert_eq!(piece_counts(&dup, SET_ITEMS)[5], 5, "4 armour + one accessory worn twice = 5");
    // ★ at four the duplicate matters: two armour + the same accessory in BOTH accessory
    // slots is three pieces, not four — no effect; a third armour piece makes it four.
    let two_plus_dup = vec![armour[0], armour[1], acc, acc];
    assert_eq!(piece_counts(&two_plus_dup, SET_ITEMS)[5], 3);
    assert_eq!(active_mask(&two_plus_dup, SET_ITEMS), 0, "the second copy is not a fourth piece");
    let three_plus_dup = vec![armour[0], armour[1], armour[2], acc, acc];
    assert_eq!(active_mask(&three_plus_dup, SET_ITEMS), SETS[5].bit);
    let junk = vec![(0x05, 0x1f), (0x0c, 0x2f), (0x00, 0x00)];
    assert_eq!(piece_counts(&junk, SET_ITEMS), [0; SET_COUNT]);
    // An 85제 half and a 90제 half make nothing either (코르버스 is five pieces since 155차).
    let k = pieces(3);
    let halves = vec![t[0], t[1], t[2], k[2], k[3], k[4]];
    assert_eq!(active_mask(&halves, SET_ITEMS), 0);
    // Two sets at once: four of each (eight slots) light both bits.
    let both = vec![t[0], t[1], t[2], t[3], c[0], c[1], c[2], c[3]];
    assert_eq!(active_mask(&both, SET_ITEMS), SETS[0].bit | SETS[1].bit);
}

/// The bits are `wie_util::stats::SET_GUARD / SET_AMBUSH / SET_BLESSING / SET_EXECUTE /
/// SET_WITCH / SET_ICE` (1 .. 32) — the order `gen_set_items.py` numbers the sets in.
#[test]
fn the_bits_are_the_hooks_bits() {
    assert_eq!(SETS.map(|s| s.bit), [1, 2, 4, 8, 16, 32]);
    assert_eq!(SETS.map(|s| s.name), ["템플러", "크루세이더", "레갈리스근위", "코르버스", "붉은마녀", "빙해"]);
    assert_eq!(GUARD_BIT, SETS[0].bit);
    assert_eq!(WITCH_BIT, SETS[4].bit);
}

/// Each set's accessory (142차 · 142차-C · ★ 154차 · ★ 155차): 레아의목걸이 belongs to 붉은마녀 (the set
/// of 「붉은 마녀」 레아 — it was 크루세이더's), 이안의맹세 to 빙해 (「빙해의 검사」 이안 — it was 템플러's);
/// 템플러 · 크루세이더 · 코르버스 have none; 굴베이그의탈리스만 · 클레르의가보 · 닉스의펜던트 are no set piece.
#[test]
fn the_sets_take_their_accessory() {
    let acc = |sid: u8| pieces(sid).into_iter().find(|p| p.0 == 0x0c);
    assert_eq!(acc(0), None, "템플러 · no accessory (155차 — 이안의맹세 left)");
    assert_eq!(acc(1), None, "크루세이더 · no accessory (154차)");
    assert_eq!(acc(2), Some((0x0c, 0x2b)), "레갈리스근위 · 레갈리스교단반지");
    assert_eq!(acc(3), None, "코르버스 · no accessory (155차 — 닉스의펜던트 left)");
    assert_eq!(acc(4), Some((0x0c, 0x2e)), "붉은마녀 · 레아의목걸이 (154차)");
    assert_eq!(acc(5), Some((0x0c, 0x2d)), "빙해 · 이안의맹세 (155차 — was 클레르의가보)");
    assert_eq!(set_of(0x0c, 0x31, SET_ITEMS), None, "굴베이그의탈리스만 is no set piece (154차)");
    assert_eq!(set_of(0x0c, 0x30, SET_ITEMS), None, "클레르의가보 is no set piece (155차)");
    assert_eq!(set_of(0x0c, 0x32, SET_ITEMS), None, "닉스의펜던트 is no set piece (155차)");
    assert_eq!(SET_ITEMS.iter().filter(|p| p.0 == 0x0c).count(), 3, "three set accessories");
}

/// ★ 150차: what the 상세보기 set section is built from — the set of a piece, its pieces in
/// slot order with their worn state, and the slot words (154차 · 155차: five for 템플러 · 크루세이더 ·
/// 코르버스).
#[test]
fn the_detail_section_helpers_list_a_set_in_slot_order() {
    assert_eq!(set_of(0x00, 0x30, SET_ITEMS), Some(0), "템플러헬름");
    assert_eq!(set_of(0x0c, 0x2e, SET_ITEMS), Some(4), "레아의목걸이 · 붉은마녀 (154차)");
    assert_eq!(set_of(0x0c, 0x2a, SET_ITEMS), None, "율리케의목걸이 is no set piece");
    assert_eq!(set_of(0x00, 0x36, SET_ITEMS), None, "예언자의눈 (legend) is no set piece");
    for sid in 0..SET_COUNT as u8 {
        let p = pieces(sid);
        let last = *p.last().unwrap();
        let worn = vec![p[1], last, (0x05, 0x1f)];
        let st = piece_states(sid, &worn, SET_ITEMS);
        assert_eq!(st.len(), p.len(), "set {sid}");
        let words: Vec<&str> = st.iter().map(|((c, _), _)| slot_word(*c)).collect();
        let want: &[&str] = if [0, 1, 3].contains(&sid) { &["투구", "갑옷", "장갑", "신발", "방패"] } else { &["투구", "갑옷", "장갑", "신발", "방패", "장신구"] };
        assert_eq!(words, want, "set {sid}");
        let flags: Vec<bool> = (0..p.len()).map(|i| i == 1 || i == p.len() - 1).collect();
        assert_eq!(st.iter().map(|s| s.1).collect::<Vec<_>>(), flags, "set {sid}");
    }
    assert_eq!(slot_word(0x05), "장비");
}

/// 마녀의 생명력's countdown: on the field line beside 수호의 맹세's, and in the menu line.
#[test]
fn the_witch_buff_counts_down() {
    let at = |now: u64, guard: u64, kill: u64| Clocks { now_ms: now, guard_until_ms: guard, guard_ready_ms: 0, kill_until_ms: kill, hit_until_ms: 0 };
    assert_eq!(field_line(WITCH_BIT, 0, 0, at(1_000, 0, 21_000)), Some("마녀의 생명력 20초".into()));
    assert_eq!(field_line(WITCH_BIT, 0, 0, at(21_000, 0, 21_000)), None, "expired");
    assert_eq!(field_line(0, 0, 0, at(1_000, 0, 21_000)), None, "the set came off");
    assert_eq!(field_line(GUARD_BIT | WITCH_BIT, 0, 0, at(1_000, 5_000, 3_500)), Some("수호의 맹세 무적 4초 / 마녀의 생명력 3초".into()));
    assert_eq!(menu_lines(WITCH_BIT, 0, 0, at(1_000, 0, 21_000)), vec!["세트 마녀의 생명력 (남은 20초)".to_string()]);
    assert_eq!(menu_lines(WITCH_BIT, 0, 0, at(30_000, 0, 21_000)), vec!["세트 마녀의 생명력".to_string()]);
}

/// ★ 150차: the accessories' timed max-HP buffs — 레아의목걸이 on the SHARED kill clock,
/// 굴베이그의탈리스만 on the hit clock — join both lines while they run, and only then.
#[test]
fn the_accessory_buffs_count_down_on_their_clocks() {
    let c = Clocks { now_ms: 8_000, guard_until_ms: 0, guard_ready_ms: 0, kill_until_ms: 20_000, hit_until_ms: 12_500 };
    assert_eq!(field_line(0, 20, 0, c), Some("처치 최대HP+20% 12초".into()));
    assert_eq!(field_line(0, 0, 20, c), Some("피격 최대HP+20% 5초".into()), "4.5 s rounds up");
    assert_eq!(field_line(0, 20, 20, c), Some("처치 최대HP+20% 12초 / 피격 최대HP+20% 5초".into()));
    // the set and 레아 run on ONE clock — both are listed, the percents add in the hook
    assert_eq!(field_line(WITCH_BIT, 20, 0, c), Some("마녀의 생명력 12초 / 처치 최대HP+20% 12초".into()));
    // two copies worn: the published percent is the sum
    assert_eq!(field_line(0, 40, 0, c), Some("처치 최대HP+40% 12초".into()));
    // not worn, or the clock is not running: nothing
    assert_eq!(field_line(0, 0, 0, c), None, "no accessory worn — a running kill clock is the set's");
    let later = Clocks { now_ms: 20_000, ..c };
    assert_eq!(field_line(0, 20, 20, later), None, "both clocks have run out");
    assert_eq!(menu_lines(0, 20, 20, c), vec!["처치 최대HP+20% (남은 12초)".to_string(), "피격 최대HP+20% (남은 5초)".to_string()]);
    assert_eq!(menu_lines(GUARD_BIT, 0, 20, c), vec!["세트 수호의 맹세".to_string(), "피격 최대HP+20% (남은 5초)".to_string()]);
    assert!(menu_lines(0, 20, 20, later).is_empty(), "an idle accessory buff is not a menu line");
}

/// The HUD lines: the field line only while invulnerable, the menu lines per set with the
/// guard's cooldown, seconds rounded up.
#[test]
fn the_lines_say_what_runs() {
    let at = |now: u64, guard_until: u64, guard_ready: u64| Clocks { now_ms: now, guard_until_ms: guard_until, guard_ready_ms: guard_ready, kill_until_ms: 0, hit_until_ms: 0 };
    assert_eq!(field_line(1, 0, 0, at(1_000, 5_500, 0)), Some("수호의 맹세 무적 5초".into()));
    assert_eq!(field_line(1, 0, 0, at(5_500, 5_500, 0)), None);
    assert_eq!(field_line(0, 0, 0, at(1_000, 5_500, 0)), None, "no set, no line");
    assert_eq!(menu_lines(1 | 4, 0, 0, at(0, 0, 0)), vec!["세트 수호의 맹세".to_string(), "세트 교단의 가호".to_string()]);
    assert_eq!(menu_lines(1, 0, 0, at(60_001, 0, 100_000)), vec!["세트 수호의 맹세 (재사용 40초)".to_string()]);
    assert!(menu_lines(0, 0, 0, at(0, 0, 100_000)).is_empty());
    assert_eq!(secs_left(0, 1), 1);
    assert_eq!(secs_left(2_000, 1_000), 0);
    assert_eq!(activation_notice(2).unwrap(), "크루세이더 세트 효과\n기습: HP가 가득 찬 적에게 첫 공격 피해 2배");
    assert_eq!(activation_notice(64), None);
    assert_eq!(activation_notice(16).unwrap(), "붉은마녀 세트 효과\n마녀의 생명력: 적 처치 시 20초간 최대 HP 20% 증가");
    assert_eq!(menu_lines(8 | 32, 0, 0, at(0, 0, 0)), vec!["세트 처형".to_string(), "세트 빙해의 일격".to_string()]);
}
