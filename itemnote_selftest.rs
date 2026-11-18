// Standalone self-test of the 상세보기 body overlay (150차, `wie_android/src/itemnote.rs`).
//
//   rustc --edition 2024 --test itemnote_selftest.rs -o t.exe; .\t.exe
//
// `cargo test -p wie_android` cannot run on this Windows host (CLAUDE.md), so this compiles
// the REAL sources — `itemnote.rs` and the game font it draws with — as their own test crate.
// `itemnote.rs` may depend on `crate::gamefont` and nothing else; `setbonus.rs` /
// `set_items.rs` / `menutouch.rs` come along only to check that the numbers agree.
//
// ★ The REAL_* strips below are the GUEST'S OWN framebuffer (`wie_probe --dump`, 496x240, the
// vc422 asset): the 상세보기 window of 율리케의목걸이 (detail-window/run2 f1990) and of
// 템플러헬름 (run3 f2230), run-length encoded `rrggbb:n`, row-major. They are what makes the
// renderer's claim — "draws exactly like the game's drawer" — a test instead of a hope:
// the game's own lines re-render with ZERO differing pixels. Do not hand-edit; to regenerate
// (the window changed), dump the frame again — `wie_probe --frames 2000 --ms 20` with the
// new-character keys (`260:OK,400:OK,…` then OK/HASH every 60 frames to 1600),
// `--host-give 1640:c:2a` (율리케의목걸이) or `1645:0:30` (템플러헬름), `1700:CLEAR` →
// INVENTORY → cursor on the item → OK → OK (상세보기), `--dump` there — and encode the
// rectangle row-major:
//     b = open(frame, 'rb').read(); px = [b[i] << 16 | b[i+1] << 8 | b[i+2] for i in range(0, len(b), 4)]
//     runs over px[(40 + dy) * 496 + 160 + dx] for dy in DY0..DY1, dx in DX0..DX1 -> "rrggbb:n"

mod gamefont_data {
    include!("wie_android/src/gamefont_data.rs");
}
#[allow(dead_code)]
mod gamefont {
    include!("wie_android/src/gamefont.rs");
}
mod itemnote {
    include!("wie_android/src/itemnote.rs");
}
#[allow(dead_code)]
mod setbonus {
    include!("wie_android/src/setbonus.rs");
}
#[allow(dead_code)]
mod set_items {
    include!("wie_android/src/set_items.rs");
}
#[allow(dead_code)]
mod menutouch {
    include!("wie_android/src/menutouch.rs");
}

use itemnote::*;

/// The accessory table's generated texts, read out of the generated source (it carries `//!`
/// docs, so it cannot be `include!`d): `(summary, detail)` of each of the 11 rows.
fn acc_texts() -> Vec<(String, String)> {
    let src = include_str!("vendor/wie/wie_util/src/acc_options.rs");
    let grab = |key: &str| -> Vec<String> {
        src.lines()
            .filter_map(|l| l.trim().strip_prefix(key))
            .map(|rest| rest.trim_end_matches(',').trim_matches('"').to_string())
            .collect()
    };
    let (s, d) = (grab("summary: "), grab("detail: "));
    assert_eq!((s.len(), d.len()), (11, 11), "acc_options.rs has 11 accessories");
    s.into_iter().zip(d).collect()
}

const LW: usize = 496;
const LH: usize = 240;
const ORIGIN: (i32, i32) = (160, 40);
const RED: u8 = b'"';

/// Decode a strip into a `w`-wide block of `0xffRRGGBB` pixels.
fn unrle(s: &str, w: usize, h: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(w * h);
    for tok in s.split_whitespace() {
        let (rgb, n) = tok.split_once(':').unwrap();
        let rgb = u32::from_str_radix(rgb, 16).unwrap() | 0xff00_0000;
        for _ in 0..n.parse::<usize>().unwrap() {
            out.push(rgb);
        }
    }
    assert_eq!(out.len(), w * h, "the strip is {w}x{h}");
    out
}

/// A 496x240 frame, black, with `strip` (w x h) painted at panel `(dx, dy)`.
fn frame_with(parts: &[(&str, i32, i32, usize, usize)]) -> Vec<u32> {
    let mut f = vec![0xff00_0000u32; LW * LH];
    for &(s, dx, dy, w, h) in parts {
        let px = unrle(s, w, h);
        for r in 0..h {
            for c in 0..w {
                let (x, y) = ((ORIGIN.0 + dx) as usize + c, (ORIGIN.1 + dy) as usize + r);
                f[y * LW + x] = px[r * w + c];
            }
        }
    }
    f
}

fn yul_frame() -> Vec<u32> {
    frame_with(&[(REAL_YUL_BODY, BODY_DX, BODY_DY, BODY_W, BODY_H), (REAL_YUL_NAME, 26, 18, 145, 17)])
}
fn tpl_frame() -> Vec<u32> {
    frame_with(&[(REAL_TPL_BODY, BODY_DX, BODY_DY, BODY_W, BODY_H), (REAL_TPL_NAME, 26, 18, 145, 17)])
}
fn fr(px: &[u32]) -> Frame<'_> {
    Frame { px, w: LW, h: LH }
}
fn patch(frame: &[u32]) -> Vec<u32> {
    let mut p = vec![0u32; BODY_W * BODY_H];
    assert!(body_patch(fr(frame), ORIGIN, &mut p));
    p
}
fn rgb_diff(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b).filter(|(x, y)| (**x & 0x00ff_ffff) != (**y & 0x00ff_ffff)).count()
}
fn line(text: &str, tone: Tone) -> NoteLine {
    NoteLine { text: text.to_string(), tone, gap: 0 }
}
fn note(keep: (i32, i32), lines: Vec<NoteLine>) -> ItemNote {
    ItemNote { name: "율리케의목걸이".into(), name_rgb: name_rgb(Some(RED)), name_prefix: None, keep, dropped: None, lines }
}

// --------------------------------------------------------------------------
// the game drawer's metrics, proven on the game's own pixels
// --------------------------------------------------------------------------

/// ★ Every line the game drew in the two windows reads back EXACTLY through the host's
/// game-metric layout — Hangul 11, space 5, ASCII `eng7` 8 px dropped 2 rows. (The 상세보기
/// report's "ASCII 7 px" was wrong: the pixels say 8, and the space 5.)
#[test]
fn the_games_own_lines_re_render_pixel_for_pixel() {
    let (y, t) = (yul_frame(), tpl_frame());
    let body = Tone::Body.rgb();
    for (dy, s) in [(63, "기절 저항 +22"), (79, "특수공격 +30%"), (95, "SP회복 +40%")] {
        assert!(reads(fr(&y), ORIGIN, (TEXT_DX, dy), s, body, BODY_BG, 172), "율리케 {s}");
    }
    for (dy, s) in [(63, "마스터 / 솔티안 전용"), (79, "방 206"), (95, "제련도 0/10"), (109, "물리방어 +30%"), (123, "특수방어 +30%")] {
        assert!(reads(fr(&t), ORIGIN, (TEXT_DX, dy), s, body, BODY_BG, 172), "템플러 {s}");
    }
    assert!(reads(fr(&y), ORIGIN, (NAME_DX, NAME_DY), "율리케의목걸이", name_rgb(Some(RED)), NAME_BOX_BG, NAME_BOX_END));
    assert!(reads(fr(&t), ORIGIN, (NAME_DX, NAME_DY), "템플러헬름", name_rgb(Some(RED)), NAME_BOX_BG, NAME_BOX_END));
    // …and nothing else reads: a prefix, a different value, the wrong ink, one row off.
    assert!(!reads(fr(&y), ORIGIN, (TEXT_DX, 63), "기절 저항 +2", body, BODY_BG, 172), "a prefix");
    assert!(!reads(fr(&y), ORIGIN, (TEXT_DX, 63), RESIST_ZERO, body, BODY_BG, 172), "+0 is not +22");
    assert!(!reads(fr(&y), ORIGIN, (TEXT_DX, 64), "기절 저항 +22", body, BODY_BG, 172), "one row off");
    assert!(!reads(fr(&y), ORIGIN, (NAME_DX, NAME_DY), "율리케의목걸", name_rgb(Some(RED)), NAME_BOX_BG, NAME_BOX_END));
    assert!(!reads(fr(&y), ORIGIN, (NAME_DX, NAME_DY), "율리케의목걸이", name_rgb(None), NAME_BOX_BG, NAME_BOX_END));
    assert!(!reads(fr(&y), ORIGIN, (TEXT_DX, 63), "", body, BODY_BG, 172), "nothing is not a line");
}

/// ★★ The whole 168x94 body, re-drawn by `compose` from HOST lines alone (the game's three
/// lines as a note that keeps nothing), is the game's body — zero pixels differ. So are the
/// "keep everything" compositions of both windows.
#[test]
fn compose_reproduces_the_games_body_exactly() {
    let y = patch(&yul_frame());
    let mut out = vec![0u32; BODY_W * BODY_H];
    let host = note((FIRST_LINE_DY, FIRST_LINE_DY), ["기절 저항 +22", "특수공격 +30%", "SP회복 +40%"].into_iter().map(|s| line(s, Tone::Body)).collect());
    assert!(compose(&y, &host, 0, &mut out));
    assert_eq!(rgb_diff(&y, &out), 0, "host-drawn lines == the game's lines");
    assert!(compose(&y, &note(GameBody::Accessory { drop_resist: false }.keep(), vec![]), 0, &mut out));
    assert_eq!(rgb_diff(&y, &out), 0, "the accessory body kept whole");
    let t = patch(&tpl_frame());
    assert!(compose(&t, &note(GameBody::Equipment { sockets: true, orb: false }.keep(), vec![]), 0, &mut out));
    assert_eq!(rgb_diff(&t, &out), 0, "the equipment body (with sockets) kept whole");
}

#[test]
fn the_drawer_metrics_are_the_games() {
    assert_eq!(gamefont::game_width("기절 저항 +22"), 11 * 4 + 5 * 2 + 8 * 3);
    assert_eq!(gamefont::game_width("1.5배"), 8 + 4 + 8 + 11, "`.` and `,` advance 4");
    assert_eq!(gamefont::game_width("A,B"), 8 + 4 + 8);
    let cells = gamefont::game_layout("가 A");
    assert_eq!(cells.iter().map(|c| (c.advance, c.drop)).collect::<Vec<_>>(), [(11, 0), (5, 0), (8, 2)]);
    assert!(cells[1].rows.iter().all(|r| *r == 0), "a space draws nothing");
    assert_eq!(gamefont::game_layout("\u{1}\u{a0}"), vec![], "control chars and NBSP are not drawn — wrap turns NBSP into a space first");
}

// --------------------------------------------------------------------------
// wrapping
// --------------------------------------------------------------------------

#[test]
fn wrap_breaks_at_spaces_and_balances_an_orphan() {
    assert_eq!(wrap("특수공격 +30%", WRAP_W), ["특수공격 +30%"]);
    assert!(wrap("", WRAP_W).is_empty());
    assert!(wrap("   ", WRAP_W).is_empty());
    // greedy would leave 「+20%」 alone; the balance pass evens the two lines
    assert_eq!(wrap("적 처치 시 20초간 최대 HP +20%", WRAP_W), ["적 처치 시 20초간", "최대 HP +20%"]);
    assert_eq!(wrap("HP가 가득 찬 적에게 첫 공격 피해 2배", WRAP_W), ["HP가 가득 찬 적에게", "첫 공격 피해 2배"]);
    // …but a last line of at least half keeps its own line
    assert_eq!(wrap("치명상 시 HP 1로 5초 무적 (재사용 100초)", WRAP_W), ["치명상 시 HP 1로 5초 무적", "(재사용 100초)"]);
    // a no-break space holds its group together and is drawn as a space
    assert_eq!(wrap("레갈리스근위 세트 (착용\u{a0}1/6)", WRAP_W), ["레갈리스근위 세트", "(착용 1/6)"]);
    assert_eq!(wrap("템플러 세트 (착용\u{a0}1/6)", WRAP_W), ["템플러 세트 (착용 1/6)"]);
    // a hard break, and a word no line can hold
    assert_eq!(wrap("가\n나", WRAP_W), ["가", "나"]);
    let long = "가".repeat(20);
    let parts = wrap(&long, WRAP_W);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts.concat(), long, "broken between characters, nothing lost");
    assert!(parts.iter().all(|p| gamefont::game_width(p) <= WRAP_W));
}

/// Every line any builder output can carry fits the host width — the whole vocabulary, the
/// 11 accessories' thirds and the six sets.
#[test]
fn every_wrapped_line_fits_the_width() {
    let mut texts: Vec<String> = acc_texts().into_iter().flat_map(|(s, d)| [s, d]).collect();
    for (sid, s) in setbonus::SETS.iter().enumerate() {
        let n = set_size(sid);
        texts.push(s.text.to_string());
        texts.push(format!("{} 세트 (착용\u{a0}{n}/{n})", s.name));
        texts.push(rule_line(n, setbonus::PIECES_FOR_BONUS));
    }
    texts.push(NATIVE_SET_TEXT.to_string());
    for t in &texts {
        let lines = wrap(t, WRAP_W);
        assert!(!lines.is_empty(), "{t}");
        for l in &lines {
            assert!(gamefont::game_width(l) <= WRAP_W, "{l:?} of {t:?} is {} px", gamefont::game_width(l));
        }
        let joined = lines.join(" ");
        assert_eq!(joined.replace(' ', ""), t.replace(['\u{a0}', ' '], ""), "wrapping loses nothing");
    }
    assert_eq!(WRAP_W, 157, "pen x 6 .. one px short of the scrollbar at dx 164");
}

// --------------------------------------------------------------------------
// the builder
// --------------------------------------------------------------------------

#[test]
fn the_kept_rows_follow_the_games_body() {
    assert_eq!(GameBody::Accessory { drop_resist: true }.keep(), (79, 106), "options 1 and 2");
    assert_eq!(GameBody::Accessory { drop_resist: false }.keep(), (63, 106));
    assert_eq!(GameBody::Equipment { sockets: true, orb: false }.keep(), (63, 143), "sockets 139..142");
    assert_eq!(GameBody::Equipment { sockets: true, orb: true }.keep(), (63, 146), "the orb line 135..145");
    assert_eq!(GameBody::Equipment { sockets: false, orb: false }.keep(), (63, 134), "option 2 at 123");
    assert_eq!(GameBody::Equipment { sockets: false, orb: true }.keep(), (63, 146));
}

/// An accessory whose resist went to 0: the game's first line goes, the two options move up,
/// and the third option lands EXACTLY where the game's third line was (panel dy 95) — the
/// three read as one list.
#[test]
fn a_reworked_accessory_drops_its_resist_line_and_adds_the_third() {
    let n = build(&NoteInput { name: "레아의목걸이", escape: Some(RED), body: GameBody::Accessory { drop_resist: true }, third: Some("적 처치 시 20초간 최대 HP +20%"), set: None, native_set: false }).unwrap();
    assert_eq!(n.dropped.as_deref(), Some(RESIST_ZERO));
    assert_eq!(n.keep, (79, 106));
    assert_eq!(n.lines, vec![line("적 처치 시 20초간", Tone::Gold), line("최대 HP +20%", Tone::Gold)]);
    assert_eq!(line_tops(&n), [95 - BODY_DY, 95 - BODY_DY + PITCH]);
    assert_eq!(n.name_rgb, 0x00ff_0000);
    // no resist to drop and nothing to add = no note; a drop alone is still a note
    let plain = NoteInput { name: "x", escape: None, body: GameBody::Accessory { drop_resist: false }, third: None, set: None, native_set: false };
    assert_eq!(build(&plain), None);
    let drop_only = build(&NoteInput { body: GameBody::Accessory { drop_resist: true }, ..plain }).unwrap();
    assert!(drop_only.lines.is_empty() && drop_only.dropped.is_some());
    // the one-line thirds stay one line
    for (_, detail) in acc_texts() {
        let n = build(&NoteInput { third: Some(&detail), ..plain }).unwrap();
        assert!(n.lines.len() <= 2 && n.lines.iter().all(|l| l.tone == Tone::Gold), "{detail}");
    }
}

/// How many pieces set `sid` has in the generated table (154차: four to six).
fn set_size(sid: usize) -> usize {
    set_items::SET_ITEMS.iter().filter(|p| usize::from(p.2) == sid).count()
}

/// 템플러's five pieces (155차: 이안의맹세 left the set — it is 빙해's now).
fn templar_pieces(worn: [bool; 5]) -> Vec<(String, String, bool)> {
    let names = ["템플러헬름", "템플러플레이트", "템플러건틀릿", "템플러그리브", "케네스방패"];
    let cats = [0x00u16, 0x01, 0x02, 0x03, 0x0b];
    (0..5).map(|i| (setbonus::slot_word(cats[i]).to_string(), names[i].to_string(), worn[i])).collect()
}

/// 빙해's six pieces (155차: the set accessory is 이안의맹세 — was 클레르의가보).
fn ice_pieces(worn: [bool; 6]) -> Vec<(String, String, bool)> {
    let names = ["빙해의투구", "빙해의갑옷", "빙해의장갑", "빙해의장화", "왕국의철벽", "이안의맹세"];
    let cats = [0x00u16, 0x01, 0x02, 0x03, 0x0b, 0x0c];
    (0..6).map(|i| (setbonus::slot_word(cats[i]).to_string(), names[i].to_string(), worn[i])).collect()
}

/// A set piece: the section header with the worn count, the rule, the effect, the pieces in
/// slot order (green worn · dim not), then the native line — each section opened by a gap.
/// 155차: 템플러 is five pieces (no accessory line), the set accessory example is 빙해 + 이안의맹세.
#[test]
fn a_set_piece_lists_its_set() {
    assert_eq!(set_size(0), 5, "템플러 (155차)");
    let owned = templar_pieces([true, false, true, true, false]);
    let pieces: Vec<Piece> = owned.iter().map(|(s, n, w)| Piece { slot: s, name: n, worn: *w }).collect();
    let s = &setbonus::SETS[0];
    let set = SetView { name: s.name, bonus: s.bonus, text: s.text, pieces: &pieces, worn: 3, need: setbonus::PIECES_FOR_BONUS };
    let n = build(&NoteInput { name: "템플러헬름", escape: Some(RED), body: GameBody::Equipment { sockets: true, orb: false }, third: None, set: Some(set), native_set: true }).unwrap();
    let text: Vec<(&str, Tone, u8)> = n.lines.iter().map(|l| (l.text.as_str(), l.tone, l.gap)).collect();
    assert_eq!(
        text,
        [
            ("템플러 세트 (착용 3/5)", Tone::Gold, GROUP_GAP),
            ("5부위 중 4부위 착용 시", Tone::Body, 0),
            ("수호의 맹세", Tone::Gold, 0),
            ("치명상 시 HP 1로 5초 무적", Tone::Body, 0),
            ("(재사용 100초)", Tone::Body, 0),
            ("투구 템플러헬름", Tone::Green, 0),
            ("갑옷 템플러플레이트", Tone::Dim, 0),
            ("장갑 템플러건틀릿", Tone::Green, 0),
            ("신발 템플러그리브", Tone::Green, 0),
            ("방패 케네스방패", Tone::Dim, 0),
            ("세트효과: 투구, 갑옷, 장갑,", Tone::Body, GROUP_GAP),
            ("신발 착용 시 최대HP, 공격,", Tone::Body, 0),
            ("방어 +20%, SP소모 -20", Tone::Body, 0),
        ]
    );
    assert!(!text.iter().any(|(t, _, _)| t.starts_with("장신구")), "템플러 has no accessory line (155차)");
    assert_eq!(n.dropped, None);
    assert_eq!(n.keep, (63, 143));
    // the set section starts after the sockets with the game's gap + the section gap
    assert_eq!(line_tops(&n)[0], MARGIN + (143 - 63) + HOST_GAP + i32::from(GROUP_GAP));
    // a set accessory (155차: 이안의맹세 is 빙해's): third first (no gap), then the set (gap)
    assert_eq!(set_size(5), 6, "빙해 (155차)");
    let owned = ice_pieces([false; 6]);
    let pieces: Vec<Piece> = owned.iter().map(|(s, n, w)| Piece { slot: s, name: n, worn: *w }).collect();
    let s = &setbonus::SETS[5];
    let set = SetView { name: s.name, bonus: s.bonus, text: s.text, pieces: &pieces, worn: 0, need: 4 };
    let a = build(&NoteInput { name: "이안의맹세", escape: Some(RED), body: GameBody::Accessory { drop_resist: true }, third: Some("피해감소 +10%"), set: Some(set), native_set: false }).unwrap();
    assert_eq!((a.lines[0].text.as_str(), a.lines[0].gap), ("피해감소 +10%", 0));
    assert_eq!((a.lines[1].text.as_str(), a.lines[1].gap), ("빙해 세트 (착용 0/6)", GROUP_GAP));
    assert_eq!(a.lines[2].text, "6부위 중 4부위 착용 시");
    assert_eq!((a.lines[3].text.as_str(), a.lines[4].text.as_str()), ("빙해의 일격", "모든 공격의 피해 1.5배"));
    assert!(a.lines[5..].iter().all(|l| l.tone == Tone::Dim), "nothing worn");
    assert_eq!(a.lines.last().unwrap().text, "장신구 이안의맹세");
    assert_eq!(a.lines.len(), 11, "third · header · rule · effect name · one effect line · six pieces");
}

/// ★ 154차: a five-piece set (크루세이더 — no accessory) counts and rules out of five, and lists
/// five pieces.
#[test]
fn a_five_piece_set_counts_out_of_five() {
    assert_eq!(set_size(1), 5, "크루세이더");
    let names = ["크루세이더후드", "크루세이더코트", "크루세이더핸드", "크루세이더워커", "닉스의문장"];
    let cats = [0x00u16, 0x01, 0x02, 0x03, 0x0b];
    let worn = [true, true, false, false, true];
    let owned: Vec<(String, String, bool)> =
        (0..5).map(|i| (setbonus::slot_word(cats[i]).to_string(), names[i].to_string(), worn[i])).collect();
    let pieces: Vec<Piece> = owned.iter().map(|(s, n, w)| Piece { slot: s, name: n, worn: *w }).collect();
    let s = &setbonus::SETS[1];
    let set = SetView { name: s.name, bonus: s.bonus, text: s.text, pieces: &pieces, worn: 3, need: setbonus::PIECES_FOR_BONUS };
    let n = build(&NoteInput { name: "크루세이더후드", escape: Some(RED), body: GameBody::Equipment { sockets: false, orb: false }, third: None, set: Some(set), native_set: true }).unwrap();
    let text: Vec<(&str, Tone)> = n.lines.iter().map(|l| (l.text.as_str(), l.tone)).collect();
    assert_eq!(text[0], ("크루세이더 세트 (착용 3/5)", Tone::Gold));
    assert_eq!(text[1], ("5부위 중 4부위 착용 시", Tone::Body));
    assert_eq!(text[2], ("기습", Tone::Gold));
    let listed: Vec<(&str, Tone)> = text.iter().copied().filter(|(t, _)| names.iter().any(|nm| t.ends_with(nm))).collect();
    assert_eq!(
        listed,
        [
            ("투구 크루세이더후드", Tone::Green),
            ("갑옷 크루세이더코트", Tone::Green),
            ("장갑 크루세이더핸드", Tone::Dim),
            ("신발 크루세이더워커", Tone::Dim),
            ("방패 닉스의문장", Tone::Green),
        ]
    );
    assert!(!text.iter().any(|(t, _)| t.starts_with("장신구")), "no accessory line");
}

// --------------------------------------------------------------------------
// scrolling
// --------------------------------------------------------------------------

#[test]
fn scrolling_is_clamped_and_the_thumb_tracks_it() {
    let short = note((79, 106), vec![line("특수공격 +30%", Tone::Gold)]);
    assert_eq!(content_height(&short), MARGIN + 27 + HOST_GAP + GLYPH_H + MARGIN);
    assert_eq!(max_scroll(&short), 0);
    assert_eq!(thumb(&short, 0), None, "it fits: no bar");
    let long = note((63, 143), (0..12).map(|i| line(&format!("줄 {i}"), Tone::Body)).collect());
    let max = max_scroll(&long);
    assert_eq!(max, content_height(&long) - BODY_H as i32);
    assert!(max > 0);
    assert_eq!(clamp_scroll(&long, -50), 0);
    assert_eq!(clamp_scroll(&long, 1 << 30), max);
    assert_eq!(clamp_scroll(&long, 17), 17);
    let (t0, h) = thumb(&long, 0).unwrap();
    assert_eq!(t0, THUMB_TOP);
    assert!(h >= THUMB_MIN && h < THUMB_BOT - THUMB_TOP);
    let (t1, h1) = thumb(&long, max).unwrap();
    assert_eq!((t1 + h1, h1), (THUMB_BOT, h), "at the end the thumb sits on the lane's bottom");
    assert_eq!(thumb(&long, 1 << 30), thumb(&long, max), "clamped on draw too");
    assert_eq!(tap_step(BODY_DY + 10), -SCROLL_LINE);
    assert_eq!(SCROLL_PAGE, BODY_H as i32 - 2 * PITCH, "a page keeps two lines of context");
    assert_eq!(clamp_scroll(&long, SCROLL_PAGE), SCROLL_PAGE.min(max));
    assert_eq!(tap_step(BODY_DY + BODY_H as i32 - 10), SCROLL_LINE);
}

/// Composition: the kept rows move up by the dropped line, host ink is clipped to the body's
/// inner rows and kept off the bar, the bar appears only when the body overflows, the corners
/// are the game's, and an unclamped scroll draws what the clamped one does.
#[test]
fn compose_moves_clips_and_bars() {
    let y = patch(&yul_frame());
    let mut out = vec![0u32; BODY_W * BODY_H];
    assert!(!compose(&y[1..], &note((63, 106), vec![]), 0, &mut out), "size mismatch");
    // drop the first line: game rows 79.. (patch 18..) land on 63.. (patch 2..)
    let drop = note((79, 106), vec![]);
    assert!(compose(&y, &drop, 0, &mut out));
    for r in 0..(106 - 79) as usize {
        assert_eq!(out[(2 + r) * BODY_W..(3 + r) * BODY_W], y[(18 + r) * BODY_W..(19 + r) * BODY_W], "row {r}");
    }
    assert!(out[(2 + 27) * BODY_W..(BODY_H - 1) * BODY_W].iter().all(|p| *p & 0x00ff_ffff == BODY_BG), "below: fill");
    for (x, yy) in [(0, 0), (BODY_W - 1, 0), (0, BODY_H - 1), (BODY_W - 1, BODY_H - 1)] {
        assert_eq!(out[yy * BODY_W + x], y[yy * BODY_W + x], "corner ({x},{yy}) is the game's");
    }
    // an overflowing body gets the bar; no host ink right of the text clip
    let long = note((63, 106), (0..10).map(|i| line(&"가".repeat(20 + i), Tone::Gold)).collect());
    assert!(max_scroll(&long) > 0);
    for s in [0, 40, max_scroll(&long)] {
        assert!(compose(&y, &long, s, &mut out));
        let bx = (BAR_DX - BODY_DX) as usize;
        for yy in BAR_TOP as usize..BAR_BOT as usize {
            for x in bx..bx + BAR_W as usize {
                let p = out[yy * BODY_W + x] & 0x00ff_ffff;
                assert!(p == BAR_TRACK || p == BAR_THUMB, "bar pixel ({x},{yy}) = {p:06x}");
            }
            assert!(out[yy * BODY_W + bx - 1] & 0x00ff_ffff != Tone::Gold.rgb(), "the text stops short of the bar");
        }
        for x in 0..BODY_W {
            assert_eq!(out[x] & 0x00ff_ffff, y[x] & 0x00ff_ffff, "row 0 is never drawn on");
            assert_eq!(out[(BODY_H - 1) * BODY_W + x] & 0x00ff_ffff, y[(BODY_H - 1) * BODY_W + x] & 0x00ff_ffff, "nor the last row");
        }
    }
    let mut again = vec![0u32; BODY_W * BODY_H];
    assert!(compose(&y, &long, max_scroll(&long), &mut out));
    assert!(compose(&y, &long, 1 << 30, &mut again));
    assert_eq!(out, again, "clamped on draw: never an empty body");
    assert!(compose(&y, &long, -99, &mut again));
    assert!(compose(&y, &long, 0, &mut out));
    assert_eq!(out, again);
    // and host ink is written opaque in the tone's colour
    assert!(out.iter().any(|p| *p == 0xff00_0000 | Tone::Gold.rgb()));
}

/// The guard: a note lands only on the window it was built for.
#[test]
fn shows_checks_the_name_and_the_dropped_line() {
    let y = yul_frame();
    let mut n = note((63, 106), vec![line("특수공격 +30%", Tone::Gold)]);
    assert!(shows(fr(&y), ORIGIN, &n));
    n.name = "레아의목걸이".into();
    assert!(!shows(fr(&y), ORIGIN, &n), "another item's note");
    n.name = "율리케의목걸이".into();
    n.dropped = Some(RESIST_ZERO.into());
    assert!(!shows(fr(&y), ORIGIN, &n), "this window still says 기절 저항 +22 — nothing to drop");
    n.dropped = Some("기절 저항 +22".into());
    assert!(shows(fr(&y), ORIGIN, &n));
    assert!(!shows(Frame { px: &y[..100], w: LW, h: LH }, ORIGIN, &n), "a short frame reads nothing");
    let t = tpl_frame();
    let tn = ItemNote { name: "템플러헬름".into(), dropped: None, ..n.clone() };
    assert!(shows(fr(&t), ORIGIN, &tn));
    assert!(!shows(fr(&y), ORIGIN, &tn));
}

/// Paint `runs` the way the game's drawer would — pen at panel `at`, one ink per run.
fn paint_runs(f: &mut [u32], at: (i32, i32), runs: &[(&str, u32)]) {
    let mut pen = ORIGIN.0 + at.0;
    for &(t, ink) in runs {
        for cell in gamefont::game_layout(t) {
            for (r, bits) in cell.rows.iter().enumerate() {
                for c in 0..gamefont::KOR_W as i32 {
                    if bits & (1 << (15 - c)) != 0 {
                        let (x, y) = (pen + c, ORIGIN.1 + at.1 + cell.drop as i32 + r as i32);
                        f[y as usize * LW + x as usize] = 0xff00_0000 | ink;
                    }
                }
            }
            pen += cell.advance as i32;
        }
    }
}

/// ★ (integration) An ORBED and REFINED item's name box: the painter's `"%s %s+%d"` draws
/// the orb label 「@투신의」 in white, a space, then the item's own `"` name in red with `+3`
/// after it — two inks on one line. The guard reads it run by run: the note must carry the
/// prefix, the suffix belongs to the name, and either ink wrong is another item.
#[test]
fn an_orbed_refined_name_reads_in_two_inks() {
    let mut f = vec![0xff00_0000u32; LW * LH];
    for y in 18..35 {
        for x in 26..171 {
            f[(ORIGIN.1 + y) as usize * LW + (ORIGIN.0 + x) as usize] = 0xff00_0000 | NAME_BOX_BG;
        }
    }
    let (white, red) = (name_rgb(Some(b'@')), name_rgb(Some(RED)));
    paint_runs(&mut f, (NAME_DX, NAME_DY), &[("투신의 ", white), ("템플러헬름+3", red)]);
    let mut n = note((63, 134), vec![line("템플러 세트", Tone::Gold)]);
    n.name = "템플러헬름+3".into();
    n.name_prefix = Some(("투신의 ".into(), white));
    assert!(shows(fr(&f), ORIGIN, &n));
    assert!(reads_runs(fr(&f), ORIGIN, (NAME_DX, NAME_DY), &[("투신의 ", white), ("템플러헬름+3", red)], NAME_BOX_BG, NAME_BOX_END));
    assert!(!shows(fr(&f), ORIGIN, &ItemNote { name_prefix: None, ..n.clone() }), "the label is part of the box");
    assert!(!shows(fr(&f), ORIGIN, &ItemNote { name: "템플러헬름".into(), ..n.clone() }), "+3 belongs to the name");
    assert!(!shows(fr(&f), ORIGIN, &ItemNote { name_prefix: Some(("투신의 ".into(), red)), ..n.clone() }), "the label's ink");
    assert!(!shows(fr(&f), ORIGIN, &ItemNote { name_rgb: white, ..n.clone() }), "the name's ink");
    // one run through `reads_runs` is exactly `reads`
    let y = yul_frame();
    assert!(reads_runs(fr(&y), ORIGIN, (NAME_DX, NAME_DY), &[("율리케의목걸이", red)], NAME_BOX_BG, NAME_BOX_END));
    assert!(!reads_runs(fr(&y), ORIGIN, (NAME_DX, NAME_DY), &[("율리케의", red), ("목걸이", white)], NAME_BOX_BG, NAME_BOX_END));
}

// --------------------------------------------------------------------------
// agreement with the rest of the host
// --------------------------------------------------------------------------

#[test]
fn the_numbers_agree_with_menutouch_and_the_font() {
    let b = menutouch::DETAIL_BODY;
    assert_eq!((b.dx, b.dy, b.w, b.h), (BODY_DX, BODY_DY, BODY_W as i32, BODY_H as i32));
    assert_eq!(menutouch::DETAIL_BODY_INK, BODY_BG);
    assert!(BAR_DX + BAR_W <= BODY_DX + BODY_W as i32, "the bar is inside the body");
    assert!(TEXT_DX + WRAP_W as i32 <= BAR_DX - 1);
    // the name tones are the game's escape table, in the game's escape order
    let tones: Vec<u32> = gamefont::COLOUR_ESCAPES.iter().map(|&c| name_rgb(Some(c))).collect();
    assert_eq!(tones, [0xffffff, 0xd8d926, 0x2ebcff, 0xdf3939, 0xdb9a1a, 0x78c437, 0xff0000]);
    assert_eq!(name_rgb(None), 0xd6b29c);
    assert_eq!(Tone::Gold.rgb(), name_rgb(Some(b'}')), "gold = the `}}` escape");
    assert_eq!(Tone::Green.rgb(), name_rgb(Some(b'\'')), "green = the `'` escape");
    // the section rule follows the set rule
    assert_eq!(rule_line(6, setbonus::PIECES_FOR_BONUS), "6부위 중 4부위 착용 시");
    assert_eq!(rule_line(5, setbonus::PIECES_FOR_BONUS), "5부위 중 4부위 착용 시");
    assert_eq!(set_items::SET_ITEMS.len(), 33, "155차: 템플러 · 크루세이더 · 코르버스 five, the other three sets six");
}

/// Every string the overlay can draw renders in the game's font: Hangul from KS X 1001,
/// ASCII the font carries (no lowercase — `eng7`'s lowercase cells are symbols), no `;`
/// (the game's line break) and no colour escape.
#[test]
fn every_host_string_is_drawable() {
    let mut texts: Vec<String> = acc_texts().into_iter().flat_map(|(s, d)| [s, d]).collect();
    for s in &setbonus::SETS {
        texts.extend([s.name.to_string(), s.bonus.to_string(), s.text.to_string(), format!("{} 세트 (착용 1/6)", s.name)]);
    }
    for c in [0x00u16, 0x01, 0x02, 0x03, 0x0b, 0x0c] {
        texts.push(setbonus::slot_word(c).to_string());
    }
    texts.extend([NATIVE_SET_TEXT.to_string(), RESIST_ZERO.to_string(), rule_line(6, 4), rule_line(5, 4), rule_line(4, 4)]);
    for t in &texts {
        for ch in t.chars() {
            assert!(!ch.is_ascii_lowercase() && ch != ';' && !"@|${}'\"".contains(ch), "{t:?}: {ch:?}");
            if ch == ' ' {
                continue;
            }
            let cells = gamefont::game_layout(&ch.to_string());
            assert_eq!(cells.len(), 1, "{t:?}: {ch:?} has no glyph");
            assert!(cells[0].rows.iter().any(|r| *r != 0), "{t:?}: {ch:?} draws blank");
        }
    }
}

// --------------------------------------------------------------------------
// REAL frames (see the header)
// --------------------------------------------------------------------------

/// 율리케의목걸이's body, panel dx 4..171 x dy 61..154.
const REAL_YUL_BODY: &str = "
     7b5952:1 522d19:166 7b5952:1 522d19:171 b58263:5 522d19:2 b58263:1 522d19:3 b58263:5 522d19:3
     b58263:1 522d19:7 b58263:5 522d19:3 b58263:1 522d19:4 b58263:1 522d19:4 b58263:1 522d19:126
     b58263:1 522d19:2 b58263:1 522d19:5 b58263:1 522d19:5 b58263:1 522d19:9 b58263:1 522d19:5
     b58263:1 522d19:1 b58263:7 522d19:1 b58263:1 522d19:126 b58263:1 522d19:2 b58263:1 522d19:5
     b58263:1 522d19:3 b58263:3 522d19:9 b58263:1 522d19:5 b58263:1 522d19:3 b58263:1 522d19:1
     b58263:1 522d19:3 b58263:2 522d19:15 b58263:5 522d19:3 b58263:5 522d19:97 b58263:1 522d19:2
     b58263:1 522d19:4 b58263:1 522d19:1 b58263:1 522d19:4 b58263:1 522d19:9 b58263:1 522d19:3
     b58263:3 522d19:2 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:9 b58263:2 522d19:4
     b58263:1 522d19:4 b58263:2 522d19:1 b58263:1 522d19:4 b58263:2 522d19:96 b58263:1 522d19:2
     b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:9 b58263:1 522d19:5
     b58263:1 522d19:3 b58263:3 522d19:3 b58263:1 522d19:9 b58263:2 522d19:9 b58263:2 522d19:6
     b58263:2 522d19:95 b58263:1 522d19:3 b58263:1 522d19:11 b58263:1 522d19:9 b58263:1 522d19:5
     b58263:1 522d19:9 b58263:1 522d19:7 b58263:6 522d19:4 b58263:4 522d19:4 b58263:4 522d19:96
     b58263:1 522d19:3 b58263:1 522d19:4 b58263:8 522d19:9 b58263:1 522d19:5 b58263:1 522d19:4
     b58263:5 522d19:10 b58263:2 522d19:5 b58263:2 522d19:6 b58263:2 522d19:98 b58263:1 522d19:4
     b58263:1 522d19:11 b58263:1 522d19:8 b58263:1 522d19:1 b58263:1 522d19:4 b58263:1 522d19:3
     b58263:1 522d19:5 b58263:1 522d19:9 b58263:2 522d19:4 b58263:2 522d19:6 b58263:2 522d19:97
     b58263:2 522d19:5 b58263:1 522d19:4 b58263:8 522d19:7 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:5 b58263:1 522d19:15 b58263:7 522d19:1 b58263:7 522d19:99
     b58263:1 522d19:4 b58263:1 522d19:22 b58263:1 522d19:4 b58263:5 522d19:135 b58263:8 522d19:989
     b58263:7 522d19:7 b58263:1 522d19:7 b58263:7 522d19:3 b58263:5 522d19:3 b58263:1 522d19:127
     b58263:1 522d19:13 b58263:1 522d19:13 b58263:1 522d19:7 b58263:1 522d19:1 b58263:3 522d19:127
     b58263:7 522d19:6 b58263:1 522d19:1 b58263:1 522d19:12 b58263:1 522d19:7 b58263:1 522d19:3
     b58263:1 522d19:15 b58263:5 522d19:3 b58263:5 522d19:2 b58263:2 522d19:3 b58263:1 522d19:91
     b58263:1 522d19:10 b58263:2 522d19:3 b58263:2 522d19:6 b58263:1 522d19:3 b58263:1 522d19:6
     b58263:1 522d19:2 b58263:3 522d19:8 b58263:2 522d19:4 b58263:1 522d19:4 b58263:2 522d19:1
     b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:90 b58263:9 522d19:13
     b58263:9 522d19:2 b58263:3 522d19:5 b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1
     b58263:2 522d19:2 b58263:3 522d19:4 b58263:2 522d19:132 b58263:1 522d19:6 b58263:6 522d19:4
     b58263:4 522d19:2 b58263:4 522d19:1 b58263:2 522d19:3 b58263:2 522d19:93 b58263:7 522d19:3
     b58263:9 522d19:4 b58263:5 522d19:12 b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1
     b58263:2 522d19:3 b58263:2 522d19:2 b58263:2 522d19:100 b58263:1 522d19:7 b58263:1 522d19:7
     b58263:1 522d19:5 b58263:1 522d19:4 b58263:8 522d19:8 b58263:2 522d19:4 b58263:1 522d19:4
     b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:97
     b58263:1 522d19:7 b58263:1 522d19:7 b58263:1 522d19:5 b58263:1 522d19:11 b58263:1 522d19:15
     b58263:5 522d19:3 b58263:5 522d19:2 b58263:1 522d19:3 b58263:2 522d19:97 b58263:1 522d19:7
     b58263:1 522d19:8 b58263:5 522d19:12 b58263:1 522d19:1152 b58263:1 522d19:4 b58263:1 522d19:4
     b58263:1 522d19:5 b58263:1 522d19:148 b58263:7 522d19:1 b58263:1 522d19:4 b58263:7 522d19:133
     b58263:5 522d19:2 b58263:6 522d19:4 b58263:1 522d19:1 b58263:1 522d19:3 b58263:1 522d19:4
     b58263:1 522d19:5 b58263:1 522d19:17 b58263:4 522d19:3 b58263:5 522d19:2 b58263:2 522d19:3
     b58263:1 522d19:95 b58263:2 522d19:4 b58263:1 522d19:1 b58263:2 522d19:3 b58263:2 522d19:2
     b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:4 b58263:1 522d19:2 b58263:1 522d19:2
     b58263:1 522d19:9 b58263:2 522d19:5 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:3
     b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:95 b58263:2 522d19:6 b58263:2 522d19:3
     b58263:2 522d19:2 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:3 b58263:9 522d19:8
     b58263:2 522d19:5 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:2 b58263:3 522d19:4
     b58263:2 522d19:97 b58263:5 522d19:2 b58263:6 522d19:4 b58263:3 522d19:3 b58263:1 522d19:18
     b58263:6 522d19:2 b58263:2 522d19:2 b58263:2 522d19:2 b58263:4 522d19:1 b58263:2 522d19:3
     b58263:2 522d19:102 b58263:2 522d19:1 b58263:2 522d19:9 b58263:1 522d19:4 b58263:1 522d19:4
     b58263:7 522d19:9 b58263:2 522d19:4 b58263:7 522d19:1 b58263:2 522d19:3 b58263:2 522d19:2
     b58263:2 522d19:98 b58263:1 522d19:4 b58263:2 522d19:1 b58263:2 522d19:9 b58263:1 522d19:4
     b58263:1 522d19:10 b58263:1 522d19:9 b58263:2 522d19:8 b58263:2 522d19:2 b58263:2 522d19:3
     b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:96 b58263:5 522d19:2 b58263:2 522d19:7
     b58263:8 522d19:10 b58263:1 522d19:19 b58263:2 522d19:3 b58263:5 522d19:2 b58263:1 522d19:3
     b58263:2 522d19:119 b58263:1 522d19:10 b58263:1 522d19:8362 7b5952:1 522d19:166 7b5952:1";
/// 템플러헬름's body, the same rectangle.
const REAL_TPL_BODY: &str = "
     7b5952:1 522d19:166 7b5952:1 522d19:171 b58263:5 522d19:2 b58263:1 522d19:7 b58263:1 522d19:6
     b58263:5 522d19:3 b58263:1 522d19:24 b58263:1 522d19:6 b58263:5 522d19:2 b58263:1 522d19:4
     b58263:3 522d19:3 b58263:1 522d19:8 b58263:5 522d19:3 b58263:1 522d19:4 b58263:5 522d19:61
     b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:7 b58263:1 522d19:6 b58263:1 522d19:7
     b58263:1 522d19:23 b58263:1 522d19:1 b58263:1 522d19:5 b58263:1 522d19:6 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:10 b58263:1 522d19:5 b58263:1 522d19:3
     b58263:1 522d19:5 b58263:1 522d19:60 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:7
     b58263:1 522d19:6 b58263:1 522d19:7 b58263:1 522d19:21 b58263:2 522d19:3 b58263:2 522d19:3
     b58263:1 522d19:6 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:2 b58263:2 522d19:9
     b58263:1 522d19:3 b58263:3 522d19:4 b58263:5 522d19:61 b58263:1 522d19:3 b58263:1 522d19:2
     b58263:1 522d19:6 b58263:1 522d19:1 b58263:1 522d19:5 b58263:1 522d19:5 b58263:3 522d19:11
     b58263:2 522d19:11 b58263:1 522d19:6 b58263:1 522d19:6 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:2 b58263:1 522d19:9 b58263:1 522d19:1 b58263:1 522d19:4 b58263:1 522d19:5
     b58263:1 522d19:1 b58263:1 522d19:62 b58263:1 522d19:3 b58263:1 522d19:2 b58263:2 522d19:3
     b58263:2 522d19:3 b58263:2 522d19:3 b58263:4 522d19:4 b58263:1 522d19:10 b58263:2 522d19:8
     b58263:9 522d19:2 b58263:4 522d19:3 b58263:1 522d19:4 b58263:3 522d19:3 b58263:1 522d19:8
     b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:2 b58263:9 522d19:59 b58263:1 522d19:3
     b58263:1 522d19:2 b58263:1 522d19:14 b58263:1 522d19:7 b58263:1 522d19:9 b58263:2 522d19:20
     b58263:1 522d19:6 b58263:1 522d19:10 b58263:1 522d19:16 b58263:1 522d19:70 b58263:1 522d19:3
     b58263:1 522d19:2 b58263:1 522d19:14 b58263:1 522d19:7 b58263:1 522d19:8 b58263:2 522d19:11
     b58263:7 522d19:3 b58263:1 522d19:6 b58263:1 522d19:32 b58263:5 522d19:61 b58263:1 522d19:3
     b58263:1 522d19:2 b58263:1 522d19:14 b58263:1 522d19:7 b58263:1 522d19:7 b58263:2 522d19:18
     b58263:1 522d19:3 b58263:1 522d19:6 b58263:1 522d19:4 b58263:1 522d19:15 b58263:1 522d19:10
     b58263:1 522d19:5 b58263:1 522d19:60 b58263:5 522d19:2 b58263:1 522d19:3 b58263:9 522d19:2
     b58263:7 522d19:1 b58263:1 522d19:6 b58263:2 522d19:13 b58263:7 522d19:3 b58263:8 522d19:4
     b58263:1 522d19:15 b58263:1 522d19:10 b58263:1 522d19:5 b58263:1 522d19:67 b58263:1 522d19:22
     b58263:1 522d19:21 b58263:1 522d19:16 b58263:1 522d19:4 b58263:7 522d19:9 b58263:8 522d19:4
     b58263:5 522d19:113 b58263:7 522d19:949 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:160
     b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:160 b58263:5 522d19:2 b58263:2 522d19:7
     b58263:5 522d19:3 b58263:5 522d19:3 b58263:5 522d19:131 b58263:1 522d19:3 b58263:1 522d19:2
     b58263:1 522d19:7 b58263:1 522d19:4 b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:1
     b58263:2 522d19:4 b58263:1 522d19:130 b58263:5 522d19:2 b58263:1 522d19:12 b58263:2 522d19:1
     b58263:2 522d19:2 b58263:3 522d19:1 b58263:2 522d19:142 b58263:1 522d19:9 b58263:4 522d19:2
     b58263:4 522d19:1 b58263:2 522d19:1 b58263:6 522d19:133 b58263:5 522d19:9 b58263:2 522d19:5
     b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:131 b58263:1 522d19:5
     b58263:1 522d19:7 b58263:2 522d19:6 b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:3
     b58263:2 522d19:131 b58263:1 522d19:5 b58263:1 522d19:7 b58263:7 522d19:2 b58263:5 522d19:3
     b58263:5 522d19:133 b58263:5 522d19:1169 b58263:5 522d19:1 b58263:1 522d19:1 b58263:1 522d19:2
     b58263:5 522d19:3 b58263:1 522d19:3 b58263:7 522d19:140 b58263:1 522d19:3 b58263:1 522d19:1
     b58263:1 522d19:6 b58263:1 522d19:1 b58263:3 522d19:3 b58263:1 522d19:146 b58263:1 522d19:3
     b58263:1 522d19:1 b58263:1 522d19:2 b58263:5 522d19:3 b58263:1 522d19:3 b58263:1 522d19:14
     b58263:5 522d19:12 b58263:3 522d19:4 b58263:5 522d19:103 b58263:1 522d19:2 b58263:2 522d19:1
     b58263:1 522d19:2 b58263:1 522d19:5 b58263:3 522d19:3 b58263:1 522d19:13 b58263:2 522d19:3
     b58263:2 522d19:6 b58263:2 522d19:2 b58263:1 522d19:1 b58263:2 522d19:3 b58263:2 522d19:3
     b58263:2 522d19:102 b58263:1 522d19:3 b58263:1 522d19:1 b58263:1 522d19:2 b58263:7 522d19:1
     b58263:1 522d19:3 b58263:7 522d19:7 b58263:2 522d19:2 b58263:3 522d19:5 b58263:2 522d19:5
     b58263:2 522d19:3 b58263:2 522d19:2 b58263:3 522d19:102 b58263:1 522d19:3 b58263:1 522d19:1
     b58263:1 522d19:10 b58263:1 522d19:6 b58263:1 522d19:10 b58263:4 522d19:1 b58263:2 522d19:4
     b58263:2 522d19:6 b58263:2 522d19:3 b58263:4 522d19:1 b58263:2 522d19:102 b58263:1 522d19:3
     b58263:1 522d19:1 b58263:1 522d19:10 b58263:1 522d19:6 b58263:1 522d19:10 b58263:2 522d19:3
     b58263:2 522d19:3 b58263:2 522d19:7 b58263:2 522d19:3 b58263:2 522d19:3 b58263:2 522d19:101
     b58263:1 522d19:1 b58263:1 522d19:2 b58263:1 522d19:1 b58263:1 522d19:3 b58263:1 522d19:13
     b58263:1 522d19:10 b58263:2 522d19:3 b58263:2 522d19:2 b58263:2 522d19:8 b58263:2 522d19:3
     b58263:2 522d19:3 b58263:2 522d19:100 b58263:1 522d19:3 b58263:1 522d19:1 b58263:1 522d19:1
     b58263:1 522d19:3 b58263:1 522d19:9 b58263:9 522d19:7 b58263:5 522d19:2 b58263:2 522d19:7
     b58263:6 522d19:2 b58263:5 522d19:107 b58263:1 522d19:1 b58263:1 522d19:3 b58263:8 522d19:821
     b58263:7 522d19:3 b58263:5 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:2
     b58263:1 522d19:4 b58263:3 522d19:4 b58263:1 522d19:127 b58263:1 522d19:5 b58263:1 522d19:7
     b58263:1 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:127 b58263:7 522d19:7 b58263:1 522d19:2
     b58263:1 522d19:3 b58263:5 522d19:2 b58263:2 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:15 b58263:5 522d19:3 b58263:5 522d19:2 b58263:2 522d19:3 b58263:1 522d19:105
     b58263:1 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:1 b58263:3 522d19:8 b58263:2 522d19:4 b58263:1 522d19:4
     b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:90
     b58263:9 522d19:2 b58263:5 522d19:2 b58263:1 522d19:3 b58263:5 522d19:2 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1
     b58263:2 522d19:2 b58263:3 522d19:4 b58263:2 522d19:95 b58263:1 522d19:6 b58263:1 522d19:6
     b58263:1 522d19:10 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:6
     b58263:6 522d19:4 b58263:4 522d19:2 b58263:4 522d19:1 b58263:2 522d19:3 b58263:2 522d19:93
     b58263:7 522d19:3 b58263:1 522d19:6 b58263:1 522d19:5 b58263:5 522d19:4 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1 b58263:2 522d19:3
     b58263:2 522d19:2 b58263:2 522d19:100 b58263:1 522d19:3 b58263:1 522d19:6 b58263:1 522d19:4
     b58263:1 522d19:5 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:8
     b58263:2 522d19:4 b58263:1 522d19:4 b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:1
     b58263:2 522d19:2 b58263:2 522d19:91 b58263:7 522d19:3 b58263:8 522d19:4 b58263:1 522d19:5
     b58263:1 522d19:4 b58263:3 522d19:4 b58263:1 522d19:15 b58263:5 522d19:3 b58263:5 522d19:2
     b58263:1 522d19:3 b58263:2 522d19:91 b58263:1 522d19:16 b58263:1 522d19:5 b58263:5 522d19:12
     b58263:1 522d19:127 b58263:7 522d19:665 b58263:7 522d19:7 b58263:1 522d19:6 b58263:1 522d19:3
     b58263:1 522d19:2 b58263:1 522d19:4 b58263:3 522d19:4 b58263:1 522d19:127 b58263:1 522d19:13
     b58263:1 522d19:6 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:127 b58263:7 522d19:6 b58263:1 522d19:1 b58263:1 522d19:5
     b58263:5 522d19:2 b58263:2 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:15
     b58263:5 522d19:3 b58263:5 522d19:2 b58263:2 522d19:3 b58263:1 522d19:91 b58263:1 522d19:10
     b58263:2 522d19:3 b58263:2 522d19:3 b58263:1 522d19:3 b58263:1 522d19:2 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:1 b58263:3 522d19:8 b58263:2 522d19:4 b58263:1 522d19:4
     b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:90
     b58263:9 522d19:13 b58263:5 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1 b58263:2 522d19:2 b58263:3 522d19:4
     b58263:2 522d19:120 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:6
     b58263:6 522d19:4 b58263:4 522d19:2 b58263:4 522d19:1 b58263:2 522d19:3 b58263:2 522d19:93
     b58263:7 522d19:3 b58263:9 522d19:4 b58263:5 522d19:4 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:2
     b58263:2 522d19:100 b58263:1 522d19:7 b58263:1 522d19:7 b58263:1 522d19:5 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:8 b58263:2 522d19:4 b58263:1 522d19:4
     b58263:2 522d19:1 b58263:2 522d19:3 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:97
     b58263:1 522d19:7 b58263:1 522d19:7 b58263:1 522d19:5 b58263:1 522d19:4 b58263:3 522d19:4
     b58263:1 522d19:15 b58263:5 522d19:3 b58263:5 522d19:2 b58263:1 522d19:3 b58263:2 522d19:97
     b58263:1 522d19:7 b58263:1 522d19:8 b58263:5 522d19:12 b58263:1 522d19:1133 000000:4 522d19:2
     000000:4 522d19:2 000000:4 522d19:152 000000:4 522d19:2 000000:4 522d19:2 000000:4 522d19:152
     000000:4 522d19:2 000000:4 522d19:2 000000:4 522d19:152 000000:4 522d19:2 000000:4 522d19:2
     000000:4 522d19:1998 7b5952:1 522d19:166 7b5952:1";
/// 율리케의목걸이's name box, panel dx 26..170 x dy 18..34.
const REAL_YUL_NAME: &str = "
     9c928c:1 311c10:143 9c928c:1 311c10:294 ff0000:5 311c10:4 ff0000:5 311c10:2 ff0000:1 311c10:3
     ff0000:5 311c10:1 ff0000:1 311c10:1 ff0000:1 311c10:3 ff0000:3 311c10:3 ff0000:1 311c10:4
     ff0000:7 311c10:3 ff0000:5 311c10:3 ff0000:1 311c10:3 ff0000:3 311c10:3 ff0000:1 311c10:72
     ff0000:1 311c10:5 ff0000:1 311c10:7 ff0000:1 311c10:2 ff0000:1 311c10:7 ff0000:1 311c10:1
     ff0000:1 311c10:1 ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:4
     ff0000:1 311c10:5 ff0000:1 311c10:7 ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:3
     ff0000:1 311c10:2 ff0000:1 311c10:73 ff0000:5 311c10:8 ff0000:1 311c10:2 ff0000:1 311c10:7
     ff0000:1 311c10:1 ff0000:1 311c10:1 ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2
     ff0000:1 311c10:4 ff0000:7 311c10:7 ff0000:1 311c10:1 ff0000:3 311c10:2 ff0000:1 311c10:3
     ff0000:1 311c10:2 ff0000:1 311c10:86 ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:7 311c10:1
     ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:7 ff0000:1 311c10:9
     ff0000:1 311c10:4 ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:71
     ff0000:9 311c10:2 ff0000:5 311c10:2 ff0000:1 311c10:7 ff0000:1 311c10:1 ff0000:1 311c10:1
     ff0000:1 311c10:3 ff0000:3 311c10:3 ff0000:1 311c10:3 ff0000:9 311c10:2 ff0000:3 311c10:5
     ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:74 ff0000:1 311c10:1
     ff0000:1 311c10:5 ff0000:1 311c10:6 ff0000:1 311c10:6 ff0000:1 311c10:2 ff0000:1 311c10:1
     ff0000:1 311c10:9 ff0000:1 311c10:22 ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2
     ff0000:1 311c10:72 ff0000:7 311c10:3 ff0000:1 311c10:6 ff0000:1 311c10:6 ff0000:1 311c10:2
     ff0000:1 311c10:1 ff0000:1 311c10:9 ff0000:1 311c10:4 ff0000:7 311c10:4 ff0000:8 311c10:2
     ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:78 ff0000:1 311c10:3 ff0000:1 311c10:6
     ff0000:1 311c10:5 ff0000:1 311c10:3 ff0000:1 311c10:1 ff0000:1 311c10:9 ff0000:1 311c10:10
     ff0000:1 311c10:11 ff0000:1 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:2 ff0000:1 311c10:72
     ff0000:7 311c10:3 ff0000:8 311c10:3 ff0000:2 311c10:4 ff0000:1 311c10:1 ff0000:1 311c10:2
     ff0000:8 311c10:10 ff0000:1 311c10:4 ff0000:8 311c10:3 ff0000:3 311c10:3 ff0000:1 311c10:72
     ff0000:1 311c10:16 ff0000:1 311c10:9 ff0000:1 311c10:1 ff0000:1 311c10:9 ff0000:1 311c10:10
     ff0000:1 311c10:4 ff0000:1 311c10:16 ff0000:1 311c10:72 ff0000:7 311c10:48 ff0000:8 311c10:369
     9c928c:1 311c10:143 9c928c:1";
/// 템플러헬름's name box.
const REAL_TPL_NAME: &str = "
     9c928c:1 311c10:143 9c928c:1 311c10:292 ff0000:5 311c10:1 ff0000:1 311c10:1 ff0000:1 311c10:3
     ff0000:7 311c10:3 ff0000:5 311c10:3 ff0000:1 311c10:4 ff0000:1 311c10:3 ff0000:1 311c10:1
     ff0000:1 311c10:3 ff0000:7 311c10:93 ff0000:1 311c10:5 ff0000:1 311c10:1 ff0000:1 311c10:4
     ff0000:1 311c10:3 ff0000:1 311c10:8 ff0000:1 311c10:3 ff0000:1 311c10:1 ff0000:8 311c10:1
     ff0000:1 311c10:9 ff0000:1 311c10:93 ff0000:7 311c10:1 ff0000:1 311c10:3 ff0000:7 311c10:7
     ff0000:1 311c10:3 ff0000:1 311c10:3 ff0000:1 311c10:1 ff0000:1 311c10:1 ff0000:2 311c10:1
     ff0000:1 311c10:3 ff0000:7 311c10:93 ff0000:1 311c10:5 ff0000:1 311c10:1 ff0000:1 311c10:17
     ff0000:1 311c10:1 ff0000:3 311c10:2 ff0000:1 311c10:3 ff0000:1 311c10:1 ff0000:1 311c10:1
     ff0000:1 311c10:3 ff0000:1 311c10:99 ff0000:7 311c10:1 ff0000:1 311c10:2 ff0000:9 311c10:2
     ff0000:5 311c10:3 ff0000:1 311c10:3 ff0000:3 311c10:2 ff0000:1 311c10:1 ff0000:1 311c10:2
     ff0000:9 311c10:98 ff0000:1 311c10:1 ff0000:1 311c10:13 ff0000:1 311c10:7 ff0000:1 311c10:8
     ff0000:1 311c10:1 ff0000:1 311c10:104 ff0000:8 311c10:3 ff0000:7 311c10:3 ff0000:1 311c10:7
     ff0000:1 311c10:3 ff0000:8 311c10:3 ff0000:7 311c10:94 ff0000:1 311c10:6 ff0000:1 311c10:9
     ff0000:1 311c10:3 ff0000:1 311c10:7 ff0000:1 311c10:10 ff0000:1 311c10:3 ff0000:1 311c10:5
     ff0000:1 311c10:94 ff0000:1 311c10:6 ff0000:1 311c10:3 ff0000:7 311c10:3 ff0000:7 311c10:1
     ff0000:1 311c10:3 ff0000:8 311c10:3 ff0000:1 311c10:5 ff0000:1 311c10:94 ff0000:8 311c10:3
     ff0000:1 311c10:17 ff0000:1 311c10:3 ff0000:1 311c10:10 ff0000:7 311c10:105 ff0000:7 311c10:15
     ff0000:8 311c10:391 9c928c:1 311c10:143 9c928c:1";
