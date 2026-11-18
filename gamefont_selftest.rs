//! Standalone selftest for `wie_android/src/gamefont.rs` — the game's own font.
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the renderer is
//! dependency-free apart from its generated data and this file `include!`s **the
//! real source**. Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2024 --test gamefont_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! The generated data is included as a sibling module so `crate::gamefont_data`
//! resolves the same way it does inside `wie_android` — this file IS the crate root
//! here, so the path is identical in both builds.

mod gamefont_data {
    include!("wie_android/src/gamefont_data.rs");
}

include!("wie_android/src/gamefont.rs");

/// A second pass, outside-in: these check the properties that make the font SAFE to
/// draw with, and the ones that would silently produce wrong-but-plausible glyphs.
#[cfg(test)]
mod properties {
    use super::*;

    fn art(g: &Glyph) -> Vec<String> {
        g.iter()
            .map(|r| (0..KOR_W).map(|x| if r & (1 << (15 - x)) != 0 { '#' } else { '.' }).collect())
            .collect()
    }

    /// ★ The table order is the one thing that cannot be checked by "does it look
    /// like Hangul" — a wrong order still produces jamo, just from the wrong sets.
    /// This pins it to the loader's own memcpy sources: `[remap 93][초성벌 44]
    /// [중성벌 40][종성벌 22]`, offsets 0x00 / 0x5d / 0x89 / 0xb1.
    #[test]
    fn the_etc_tables_are_sliced_where_the_loader_reads_them() {
        assert_eq!(ETC.len(), 199);
        assert_eq!(remap().len(), 93, "loader memcpy #1: 0x5d bytes from +0");
        assert_eq!(cho_bul().len(), 44, "loader memcpy #2: 0x2c bytes from +0x5d");
        assert_eq!(jung_bul().len(), 40, "loader memcpy #3: 0x28 bytes from +0x89");
        assert_eq!(jong_bul().len(), 22, "loader memcpy #4: 0x16 bytes from +0xb1");
    }

    /// Every syllable in the standard renders ink, uses only sets that exist, and
    /// stays inside the 11-px cell. A failure here is a font that would draw
    /// garbage for some rare syllable months from now.
    #[test]
    fn all_2350_syllables_compose_within_the_cell() {
        for ch in ['가', '힝'] {
            assert!(wansung_index(ch).is_some(), "{ch} is in KS X 1001");
        }
        let mut blank = Vec::new();
        for i in 0..2350 {
            let g = compose(i).expect("in range");
            if g.iter().all(|r| *r == 0) {
                blank.push(i);
            }
            // Nothing may spill past the advance the layout uses, or neighbouring
            // syllables would overlap.
            for (r, row) in g.iter().enumerate() {
                let spill = row & ((1u16 << (KOR_W - KOR_ADVANCE)) - 1);
                assert_eq!(spill, 0, "syllable {i} row {r} paints past {KOR_ADVANCE} px: {row:#06x}");
            }
        }
        assert!(blank.is_empty(), "{} syllables rendered blank: {:?}", blank.len(), &blank[..blank.len().min(8)]);
    }

    /// Round-trip: the index a syllable maps to must map back to the same syllable
    /// through the EUC-KR arithmetic the game uses.
    #[test]
    fn the_wansung_index_round_trips_for_every_syllable() {
        for i in 0..2350usize {
            let (row, col) = (i / 94, i % 94);
            let bytes = [(0xb0 + row) as u8, (0xa1 + col) as u8];
            // Decode by looking the pair up in the generated order — the same table
            // the composer indexes, so this proves the two agree.
            let u = gamefont_data::KSX1001[i] as u32 + 0xac00;
            let ch = char::from_u32(u).expect("a syllable");
            assert_eq!(wansung_index(ch), Some(i), "{ch} ({bytes:02x?})");
        }
    }

    /// Two syllables that differ only in their final consonant must select
    /// different sets for the parts they share — that is what the 벌 tables are for,
    /// and it is the part a naive implementation gets wrong.
    #[test]
    fn a_final_consonant_changes_the_sets_of_the_parts_before_it() {
        let ga = compose(wansung_index('가').unwrap()).unwrap();
        let gak = compose(wansung_index('각').unwrap()).unwrap();
        assert_ne!(ga, gak);
        // 각's ㄱ must sit higher than 가's, because a syllable with a 종성 squeezes
        // the parts above it — so the two 초성 shapes cannot be identical.
        let (a, b) = (art(&ga), art(&gak));
        let ink = |v: &Vec<String>| v.iter().filter(|r| r.contains('#')).count();
        assert!(ink(&b) >= ink(&a), "각 has more rows of ink than 가");
        // The 종성 lands in rows the syllable without one leaves to the vowel alone.
        let jong_rows: Vec<usize> = (0..KOR_H).filter(|r| a[*r] != b[*r]).collect();
        assert!(!jong_rows.is_empty(), "adding ㄱ must change some rows");
        assert!(jong_rows.iter().any(|r| *r >= 6), "the 종성 sits low: {jong_rows:?}");
    }

    /// The words the settings panel will actually draw, rendered as art so a human
    /// can read them in the test output if this ever regresses.
    ///
    /// ★ Every new overlay phrase belongs here: the game font is KS X 1001 only, so a
    /// cp949-extension syllable (똠 · 뷁 · 힣) is skipped SILENTLY and the label loses a
    /// character with nothing to show for it. "개수를 설정하세요" is the quantity dialog's
    /// title (2026-08-25), copied from the game's own 상점 window.
    #[test]
    fn the_settings_vocabulary_renders() {
        // ★ 71차-I adds the 제련 odds bar's whole vocabulary (`refine::readout`). Its own
        // selftest proves the syllables are in the KS X 1001 block; only THIS test proves
        // the game's font actually carries a glyph for each of them.
        for w in [
            "설정", "저장", "창고", "화면", "소리", "진단", "복원", "종료", "개수를", "설정하세요", "수량", //
            "성공", "파괴", "확정", "제련도", "제련할", "없습니다", "저장하고", "있습니다", //
            // 하늘성 (2026-09-08): the tower's clear toast, and (2026-09-09) every word
            // of `guardian::Run::status_line` — the overlay line that tells the player
            // they are IN the tower. §H is the report that line exists to prevent.
            "층", "클리어", "하늘성", "보스", "남은", "진입", "이탈", "입장",
            // 145차 (2026-09-30): the top-centre banner (`tower_hud::banner`) — 「N층 보스」
            // over 「남은 몬스터 M」 or one of the phase words (완주 is below).
            "몬스터",
            // 142차 (2026-09-30): the 85제 set lines and the activation box (`setbonus.rs`).
            "세트", "효과", "수호의", "맹세", "무적", "재사용", "초", "기습", "교단의", "가호",
            "치명상", "시", "가득", "찬", "적에게", "첫", "공격", "피해", "배", "받는", "감소",
            "템플러", "크루세이더", "레갈리스근위",
            // 142차-C: the 90제 sets.
            "처형", "이하의", "주는", "마녀의", "생명력", "최대", "빙해의", "일격", "공격의",
            "코르버스", "붉은마녀", "빙해", "처치", "초간", "증가", "모든", "남은",
            // 122차 (2026-09-21): the entrance refusal toasts — a refused 「예」 must say so.
            "실패", "난이도", "판독", "불가", "필드가", "아님", "표", "없음",
            // 138차 (2026-09-29): the finished tower's status line, the quick-start picker
            // (`emulator::tower_pick_rows`) and the exit-reward toast.
            "완주", "처음부터", "최고", "진행도", "보상", "닉스코어파편", "파편", "조합서", "종결",
            // 143차 (2026-09-30): the EXIT tab's question (`overlay::draw_native_confirm`), the
            // wake's line when the host has to say it, and the tower's quick-save refusal.
            "하늘성에서", "나가시겠습니까", "예", "아니오", "의식이", "몽롱하다", "하늘성에서는", "저장할",
            // 정렬 (136차, 2026-09-25): the button label and its two toasts.
            "정렬", "정렬을", "완료했습니다", "지금은", "정렬할", "수",
            // 퀵슬롯 숨김 (2026-09-10): every word the layout editor's HIDE / SHOW
            // toasts can say. The editor's own chrome is ASCII (a 5x7 bitmap font),
            // but the toast goes through the game's font like every other one.
            "퀵슬롯", "숨김", "먼저", "선택하세요", "하나는", "남겨야", "합니다", "전체", "숨긴",
            // 세이브 복원 실패 (2026-09-13, 90차 후속): `vault::apply_pending_restore` now
            // speaks when a CONSUMED restore request could not be applied — the request
            // file is deleted first, so silence there left the player with nothing at all.
            "세이브", "복원에", "실패했습니다",
            // 가방 상자 (2026-09-14): the box window's title / subtitle words and the
            // sweep and not-yet toasts (`bagbox_host`).
            "오브가방", "재료가방", "소비가방", "열기", "종", "개", "칸", "가방의", "오브를",
            "오브가방으로", "옮겼습니다", "재료가방은", "소비가방은", "다음", "단계에서", "열립니다",
            // 소비가방 (2026-09-14, 2단계): the verb picker, the dialog buttons, the use
            // job's toasts.
            "사용하기", "퀵슬롯등록", "선택", "확인", "가방에", "빈", "칸이", "하나", "필요합니다", "넣지",
            "못해", "사용을", "취소했습니다", "아이템을", "사용하는", "중입니다", "가방창이", "준비되지",
            "않았습니다", "가방창으로", "소비가방에", "것이", "퀵슬롯에", "등록했습니다",
            // 127차: the stage-and-use job's one new refusal (the box's own bag cell could
            // not take the unit) — "가방에 빈 칸이 하나 필요합니다" is gone with the free cell.
            "가방창", "칸을", "쓸", "수", "없어",

            // 가방창 UI 전면 수정 (2026-09-14): the sweep toast is now per-box, so the
            // other two boxes' wording renders for the first time.
            "재료를", "재료가방으로", "소모품을", "소비가방으로",
            // The drop box grew the game's own third verb and its confirmation
            // (2026-09-15, user: *"기존처럼 버리기 같은것도 있어야하는데"*).
            "버리기", "예", "아니오", "버렸습니다",
            // 네트워크 화면의 상자 두 벌 (2026-09-15, 96차): the online screens' three
            // verbs, the two move toasts, the write-failure toasts, the refusal the
            // server-session box answers with, and the game's own 올리기 refusals — which
            // are Rust source here, not guest bytes, so they need the same guarantee.
            "상세보기", "올리기", "내리기", "창고로", "올렸습니다", "내렸습니다",
            // (99차, 2026-09-16: 「서버 창고의 상자는 아직 열 수 없습니다」 became
            // 「창고 세션이 없습니다」 — there is no server any more.)
            "창고", "세션이", "파일을", "쓰지", "못했습니다",
            "창고가", "가득찼습니다", "해당", "아이템은", "올릴", "조합", "재료는",
            "래시피", "아이템", "퀘스트", "가방", "오브가방이", "재료가방이", "소비가방이",
            // 105차~117차 (117차 code review caught these missing): the 판매 toasts (105차),
            // the use-result toasts (110차-B — the orb 「입수」 line carries the item name's
            // 을/를), and the toast the damaged-box-file quarantine speaks (117차).
            "판매했습니다", "판매실패", "게임이", "거절했습니다", "입수", "하였습니다", "을", "를",
            "파일이", "손상되어", "가방으로", "시작합니다",
            // 117차-C: the game's own 귀환 question and refusal (InGame_txt 163 / 162), which
            // the host now draws itself over the 소비가방 window.
            "최근", "귀환장소로", "이동하겠습니까", "귀환서를", "사용할수", "없는", "진행중입니다",
            // 조작 방식 row (2026-09-22): all three sub-labels — the third, 고정 스틱, is
            // new; the other two had never been listed here.
            "조작", "방식", "자유", "스틱", "십자키", "고정",
            // 150차 (2026-10-01): the reworked accessories' third options (`acc_options.rs`
            // summary · detail), the 상세보기 set section (`itemnote.rs`) and the timed-buff HUD
            // words (`setbonus::field_line` / `menu_lines`). The FULL strings, ASCII included,
            // go through the game drawer's layout in the next test.
            "공격속도", "주는", "피해", "특수공격", "물리방어", "방패", "피해감소", "처치", "최대",
            "쿨타임", "물리공격", "피격", "회피", "적", "시", "초간", "세트효과", "세트", "투구", "갑옷",
            "장갑", "신발", "장신구", "착용", "부위", "중", "소모", "공격", "방어", "기절", "저항",
            "장비", "당", "회복", "남은", "초",
        ] {
            let cells = layout(w);
            assert_eq!(cells.len(), w.chars().count(), "{w}");
            assert!(cells.iter().all(|c| c.rows.iter().any(|r| *r != 0)), "{w} has a blank glyph");
            assert_eq!(width(w), w.chars().count() * KOR_ADVANCE);
        }
    }

    /// ★ 150차: the 상세보기 overlay draws INSIDE the game's own window, so it lays text out
    /// with the game drawer's metrics (`game_layout`) — every full string it can draw, ASCII
    /// included, must come out with a glyph per character and ink on all but the spaces.
    #[test]
    fn the_150_detail_strings_render_in_the_game_drawer() {
        for s in [
            "공격속도 +50", "주는 피해 +25%", "특수공격 +30%", "물리방어 +30%", "방패 +40", "피해감소 +10%",
            "처치 20초 최대HP+20%", "적 처치 시 20초간 최대 HP +20%", "쿨타임 -15%", "물리공격 +30%",
            "피격 20초 최대HP+20%", "피격 시 20초간 최대 HP +20%", "회피 +20",
            "기절 저항 +0", "6부위 중 4부위 착용 시", "템플러 세트 (착용 3/6)", "장신구 이안의맹세",
            "세트효과: 투구, 갑옷, 장갑, 신발 착용 시 최대HP, 공격, 방어 +20%, SP소모 -20",
            "처치 최대HP+20% 12초", "피격 최대HP+20% (남은 5초)", "5초당 HP회복 +20%", "HPMAX +50%",
        ] {
            let cells = game_layout(s);
            assert_eq!(cells.len(), s.chars().count(), "{s}: a character has no glyph");
            for (c, cell) in s.chars().zip(&cells) {
                assert_eq!(cell.rows.iter().any(|r| *r != 0), c != ' ', "{s}: {c:?}");
                assert!(!c.is_ascii_lowercase(), "{s}: eng7's lowercase cells are symbols");
            }
        }
    }

    /// ★ 패드 버튼 설정 (2026-10-09): every string the screen can draw — the settings row, the
    /// window's title and buttons, every function (`gamepad::REMAP_ROWS`, a hidden plate's chip),
    /// every button name (`gamepad::button_name`), and each status line with the longest name
    /// in it. Second form (the in-game HUD, lit): the window draws through `draw_native_text`
    /// — the HOST layout — and the first form drew through the game drawer, so both are held
    /// to it, ASCII included.
    #[test]
    fn the_pad_screen_strings_render_in_the_game_drawer() {
        for s in [
            "패드 버튼 설정", "사용자 배치", "기본", "기본값", "저장", "취소",
            "화면의 버튼을 누르고 패드 버튼을 누르세요", "패드 버튼만 누르면 하는 일을 보여줍니다",
            "시간이 지났습니다", "다시 화면의 버튼을 누르세요",
            "확인 / 공격", "취소 / 메뉴", "퀵슬롯 1", "퀵슬롯 7 (숨김)", "보조무기 (7)", "지도 (*)",
            "퀵바 페이지", "빠른저장", "없음",
            "면버튼 위 (XBOX Y)", "면버튼 아래 (XBOX A)", "면버튼 왼쪽 (XBOX X)", "면버튼 오른쪽 (XBOX B)",
            "왼쪽 어깨 (L1/LB)", "왼쪽 트리거 (L2/LT)", "오른쪽 어깨 (R1/RB)", "오른쪽 트리거 (R2/RT)",
            "가운데 왼쪽 (SELECT)", "가운데 오른쪽 (START)", "왼스틱 누르기 (L3)", "오른스틱 누르기 (R3)",
            "십자키", "십자키 가운데", "뒤로 (BACK)", "버튼 188",
            "확인 / 공격: 지금 면버튼 아래 (XBOX A) 외 1", "퀵슬롯 6: 지금 없음",
            "바꿀 패드 버튼을 누르세요 (5초)",
            "기본값으로 되돌렸습니다", "저장을 누르면 적용됩니다",
            "방향키는 바꿀 수 없습니다", "걷기와 메뉴 이동에 쓰입니다",
            "면버튼 위 (XBOX Y) = 퀵슬롯 2", "버튼 188 = 없음", "바꾸려면 화면의 버튼을 먼저 누르세요",
            "퀵슬롯 1: 이미 면버튼 왼쪽 (XBOX X) 입니다", "확인 / 공격 버튼이 없어지므로 바꿀 수 없습니다",
            "다른 패드 버튼으로 다시 해 보세요",
            "퀵슬롯 1 = 면버튼 위 (XBOX Y)", "퀵슬롯 3 = 면버튼 왼쪽 (XBOX X)",
            "지도 (*) = 없음", "회복 아이템 교체 (#) = 없음",
            "패드 버튼 설정을 저장했습니다", "패드 버튼 설정을 파일에 쓰지 못했습니다 (이번 실행에만 적용)",
        ] {
            let game: Vec<bool> = game_layout(s).iter().map(|c| c.rows.iter().any(|r| *r != 0)).collect();
            let host: Vec<bool> = layout(s).iter().map(|c| c.rows.iter().any(|r| *r != 0)).collect();
            for inked in [game, host] {
                assert_eq!(inked.len(), s.chars().count(), "{s}: a character has no glyph");
                for (c, ink) in s.chars().zip(&inked) {
                    assert_eq!(*ink, c != ' ', "{s}: {c:?}");
                    assert!(!c.is_ascii_lowercase(), "{s}: eng7's lowercase cells are symbols");
                }
            }
        }
    }

    /// ★ 150차: the game drawer's own metrics (`0x3878`; `itemnote_selftest` re-renders the
    /// game's detail lines through them with zero differing pixels) — and `layout`, which the
    /// host windows were laid out with, untouched.
    #[test]
    fn the_game_drawer_metrics_are_its_own() {
        assert_eq!((GAME_SPACE_ADVANCE, GAME_POINT_ADVANCE, GAME_ASCII_DROP), (5, 4, 2));
        assert_eq!(game_width("기절 저항 +22"), 4 * KOR_ADVANCE + 2 * GAME_SPACE_ADVANCE + 3 * ENG_ADVANCE);
        assert_eq!(game_width("1.5,"), 8 + 4 + 8 + 4);
        let a = game_layout("A");
        assert_eq!((a[0].advance, a[0].drop, a[0].rows), (ENG_ADVANCE, GAME_ASCII_DROP, ascii('A')));
        let k = game_layout("가");
        assert_eq!((k[0].advance, k[0].drop), (KOR_ADVANCE, 0));
        assert_eq!(game_layout("똠\u{1}"), vec![], "no glyph, no control characters");
        assert_eq!(width("A B"), 3 * ENG_ADVANCE, "the host layout keeps its 8-px space");
    }

    #[test]
    fn mixed_ascii_and_hangul_lays_out_left_to_right() {
        let cells = layout("A가");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].advance, ENG_ADVANCE);
        assert_eq!(cells[1].advance, KOR_ADVANCE);
        // The ASCII face lives in the high byte, so it cannot collide with the
        // Korean cell's low half.
        assert_eq!(cells[0].rows.iter().fold(0u16, |a, r| a | r) & 0x00ff, 0);
    }
}

/// The encoder the host writes the guest's own buffers with (103차).
#[cfg(test)]
mod encoding {
    use super::*;

    #[test]
    fn to_euckr_round_trips_through_the_decoder() {
        // Every name the acquisition notice can carry has to survive the trip, because the
        // guest's renderer is what reads it back.
        for s in ["제련석", "붉은용액", "투신의 오브", "포션", "닉스코어파편", "OK", "체력회복포션 3"] {
            assert_eq!(from_euckr(&to_euckr(s)), *s, "{s:?}");
        }
    }

    #[test]
    fn to_euckr_emits_the_codes_the_decoder_defines() {
        // Two bytes per syllable, in the KS X 1001 rows `0xb0..=0xc8` / cols `0xa1..=0xfe`.
        let b = to_euckr("제련석");
        assert_eq!(b.len(), 6);
        for pair in b.chunks(2) {
            assert!((0xb0..=0xc8).contains(&pair[0]), "row {:#x}", pair[0]);
            assert!((0xa1..=0xfe).contains(&pair[1]), "col {:#x}", pair[1]);
        }
        assert_eq!(to_euckr("A1 "), b"A1 ");
    }

    /// ★ 135차 §2 ④: every one of the game's seven colour escapes is consumed, not drawn —
    /// the 133차 legacy rows are `"` (the orb-combine target list printed `"붉은마녀의…`),
    /// and `|` was the other one the five-marker list missed.
    #[test]
    fn every_colour_escape_of_the_game_is_stripped() {
        // `0x5b2bc` in binary.mod, in the order the game stores it.
        assert_eq!(COLOUR_ESCAPES, *b"@|${}'\"");
        for &c in COLOUR_ESCAPES.iter() {
            let mut name = vec![c];
            name.extend_from_slice(&to_euckr("붉은마녀의장갑"));
            name.push(0);
            assert_eq!(from_euckr(&name), "붉은마녀의장갑", "escape {:?}", c as char);
        }
        // Only the FIRST byte is a colour: a quote later in a name is text.
        let mut quoted = to_euckr("A");
        quoted.push(b'"');
        assert_eq!(from_euckr(&quoted), "A\"");
    }

    #[test]
    fn a_syllable_this_font_cannot_draw_is_dropped_whole() {
        // ⛔ Never `ch as u8`: a lone byte in the guest's buffer pairs with whatever
        // follows it and decodes as an entirely different syllable. cp949 extension
        // syllables (`똠`, `뷁`, `힣`) have no KS X 1001 code and no glyph here.
        assert_eq!(to_euckr("똠"), Vec::<u8>::new());
        assert_eq!(from_euckr(&to_euckr("제뷁석")), "제석");
        // A control character is not smuggled in as ASCII either — NUL would end the string.
        assert_eq!(to_euckr("제\u{0}석"), to_euckr("제석"));
    }
}
