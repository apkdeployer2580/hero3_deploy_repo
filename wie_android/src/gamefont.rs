// The GAME'S OWN font: Korean (조합형 Johab composition) and ASCII, drawn from the
// bitmaps shipped inside the game's asset.
//
// ── WHY (player request, 2026-08-20) ─────────────────────────────────────────
// "상단 톱니바퀴로 나오는 setting 창을 좀더 영웅서기 스럽게 디자인을 바꿀수있어?
//  폰트도 영웅서기에서 사용하는 폰트, 문구도 한글로 하는거지"
//
// Until now the overlay had two ways to draw text and both were dead ends for that:
//   * a 5x7 ASCII bitmap — which is why every settings row is labelled in English;
//   * `hangul.rs`, a hand-generated 12x12 Gulim set with a CLOSED syllable list, so
//     any new Korean string silently renders blank unless someone remembers to add
//     the word and regenerate (57차 shipped a toast that came out as ": ()").
// The game's own font solves both at once: it is the right typeface by definition,
// and it covers all 2350 KS X 1001 syllables.
//
// ── THE FORMAT, READ OUT OF THE GAME ─────────────────────────────────────────
// Nothing here is guessed. The file split is the LOADER's own memcpy sizes and
// destinations (guest 0x3ec2..0x3f00 for `korb`, 0x3f7e..0x3fda for `etc`), and the
// composition below is the game's own renderer at guest 0x3a5a..0x3b30, instruction
// by instruction:
//
//     johab = table[wansung_index]                 ; big-endian u16
//     cho   = remap[       (johab >> 10) & 0x1f]
//     jung  = remap[0x1f + ((johab >> 5) & 0x1f)]
//     jong  = remap[0x3e + ( johab       & 0x1f)]
//     glyph  = 초성[440 * choBul [hasJong*22 + jung] + 22 * cho]     ; COPY
//     glyph |= 중성[484 * jungBul[hasJong*20 + cho ] + 22 * jung]    ; OR
//     glyph |= 종성[616 * jongBul[jung]              + 22 * jong]    ; OR
//
// The three 벌 (set) tables and the jamo remap all live in `res/font/etc`; the
// per-set strides 440/484/616 are 20/22/28 glyphs x 22 bytes, which is also where
// the 8/4/4 set counts come from.
//
// This module is DEPENDENCY-FREE apart from the generated data, so
// `gamefont_selftest.rs` can `include!` the real source (CLAUDE.md's convention).

use crate::gamefont_data::{ENG7, ETC, KORB, TABLE};

/// Glyph cell of the Korean font: 16 px wide (2 bytes per row) x 11 rows.
pub const KOR_W: usize = 16;
pub const KOR_H: usize = 11;
/// Bytes per Korean glyph.
const KOR_STRIDE: usize = 22;
/// How much of the 16-px cell a syllable actually uses. The right columns are
/// padding in every glyph the font ships; drawing them would space Korean text out
/// by a third.
pub const KOR_ADVANCE: usize = 11;

/// ASCII cell: 8 px wide (1 byte per row) x 11 rows, first glyph is `' '`.
pub const ENG_W: usize = 8;
pub const ENG_H: usize = 11;
const ENG_STRIDE: usize = 11;
/// ASCII glyphs are drawn 8 px apart; the face is narrower than the cell but the
/// game spaces them by the cell.
pub const ENG_ADVANCE: usize = 8;

// ── the four tables inside `res/font/etc` ────────────────────────────────────
//
// FILE order, which is not the order of the addresses they are copied to — the
// loader's four memcpys (source offsets 0x00 / 0x5d / 0x89 / 0xb1) are what settles
// it, and getting it wrong renders plausible-looking but subtly broken syllables.
/// `[remap 93][초성벌 44][중성벌 40][종성벌 22]` = 199 bytes, the whole file.
fn remap() -> &'static [u8] {
    &ETC[0..93]
}
fn cho_bul() -> &'static [u8] {
    &ETC[93..137]
}
fn jung_bul() -> &'static [u8] {
    &ETC[137..177]
}
fn jong_bul() -> &'static [u8] {
    &ETC[177..199]
}

/// Where each jamo bank starts inside `korb`.
const CHO_BASE: usize = 0;
const JUNG_BASE: usize = 3520;
const JONG_BASE: usize = 3520 + 1936;
/// Bytes in one set of each bank (20 / 22 / 28 glyphs x 22 B).
const CHO_SET: usize = 440;
const JUNG_SET: usize = 484;
const JONG_SET: usize = 616;

/// Syllables in the conversion table.
const SYLLABLES: usize = 2350;

/// One rendered glyph: `KOR_H` rows of a 16-bit mask, MSB = leftmost pixel.
pub type Glyph = [u16; KOR_H];

/// Compose the Korean syllable at Wansung `index` (0..2349).
///
/// `None` for an index the table does not cover — the caller draws nothing rather
/// than a wrong syllable.
pub fn compose(index: usize) -> Option<Glyph> {
    if index >= SYLLABLES {
        return None;
    }
    let johab = ((TABLE[index * 2] as u16) << 8) | TABLE[index * 2 + 1] as u16;
    let cho = *remap().get(((johab >> 10) & 0x1f) as usize)? as usize;
    let jung = *remap().get(0x1f + ((johab >> 5) & 0x1f) as usize)? as usize;
    let jong = *remap().get(0x3e + (johab & 0x1f) as usize)? as usize;
    let has_jong = usize::from(jong != 0);

    let mut out = [0u16; KOR_H];
    let cho_set = *cho_bul().get(has_jong * 22 + jung)? as usize;
    blit(&mut out, CHO_BASE + CHO_SET * cho_set + KOR_STRIDE * cho);
    if jung != 0 {
        let set = *jung_bul().get(has_jong * 20 + cho)? as usize;
        blit(&mut out, JUNG_BASE + JUNG_SET * set + KOR_STRIDE * jung);
    }
    if jong != 0 {
        let set = *jong_bul().get(jung)? as usize;
        blit(&mut out, JONG_BASE + JONG_SET * set + KOR_STRIDE * jong);
    }
    Some(out)
}

/// OR one 22-byte glyph into the accumulator. Out-of-range offsets contribute
/// nothing, so a malformed index can never panic mid-draw.
fn blit(out: &mut Glyph, off: usize) {
    for (r, row) in out.iter_mut().enumerate() {
        let i = off + r * 2;
        if i + 1 < KORB.len() {
            *row |= ((KORB[i] as u16) << 8) | KORB[i + 1] as u16;
        }
    }
}

/// The ASCII glyph for `c`, as `ENG_H` rows of an 8-bit mask in the high byte.
///
/// Only the printable range the font ships (`' '`..`'~'`); anything else draws as a
/// space, which keeps a stray character from becoming a stray rectangle.
pub fn ascii(c: char) -> Glyph {
    let mut out = [0u16; KOR_H];
    let Some(i) = (c as usize).checked_sub(0x20) else { return out };
    if c as usize > 0x7e {
        return out;
    }
    for (r, row) in out.iter_mut().enumerate().take(ENG_H) {
        let at = i * ENG_STRIDE + r;
        if at < ENG7.len() {
            *row = (ENG7[at] as u16) << 8;
        }
    }
    out
}

/// One drawable piece of a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    /// Rows of pixels, MSB = leftmost.
    pub rows: Glyph,
    /// How far the pen moves after it, in font pixels.
    pub advance: usize,
}

/// Lay a UTF-8 string out into glyph cells, converting Korean to the game's font.
///
/// Hangul syllables are mapped through the same Wansung ordering the game's own
/// table is indexed by — `(hi - 0xB0) * 94 + (lo - 0xA1)` in EUC-KR terms, which for
/// a Unicode syllable is its position in the KS X 1001 set.
///
/// ⚠ KS X 1001 holds **2350** of Unicode's 11172 syllables, and the font has glyphs
/// for exactly those. A syllable from the cp949 extension rows (똠, 뷁, 힣 …) has no
/// glyph and is SKIPPED — the same silent-blank trap `hangul.rs` had, except the set
/// is now every syllable a Korean UI realistically needs. Keep new UI strings covered
/// by `gamefont_selftest.rs`'s vocabulary test, which fails on a blank glyph.
pub fn layout(text: &str) -> Vec<Cell> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(Cell { rows: ascii(ch), advance: ENG_ADVANCE });
        } else if let Some(i) = wansung_index(ch) {
            if let Some(rows) = compose(i) {
                out.push(Cell { rows, advance: KOR_ADVANCE });
            }
        }
    }
    out
}

/// Total width of `text` in font pixels.
pub fn width(text: &str) -> usize {
    layout(text).iter().map(|c| c.advance).sum()
}

// ── 150차: the GAME's own string drawer, metric for metric ─────────────────────
//
// [`layout`] is the overlay's layout: every ASCII cell 8 px, glyph at the cell's top — fine
// for host windows, which are the host's own. The 150차 상세보기 overlay instead draws
// INSIDE the game's own window, beside lines the game drew, so it has to place glyphs the
// way the game's drawer does. Read out of `binary.mod` (the drawer `0x3878`, reached through
// `0x3d70 → 0x3d48`; its width twin `0x3600`) and then proven on probe frames — the item
// detail window's `기절 저항 +22` · `특수공격 +30%` · `SP회복 +40%` · `마스터 / 솔티안 전용` ·
// `제련도 0/10` lines re-render with ZERO differing pixels:
//   * Hangul: the same composed glyph, advance 11;
//   * a space: nothing drawn, advance **5** (`0x3c5c adds r0,#5`);
//   * other printable ASCII: the `eng7` glyph (font 1, `[sp,#0x98] == 1` picks
//     `0x15004e4`) drawn **2 rows lower** than the Hangul cell's top (`0x3cc6 adds r3,#2`),
//     advance font width 7 + 1 = **8**, except `.` and `,` which advance **4**
//     (`0x3d0a..0x3d12`: the width minus 3).
// ⛔ Do not "fix" [`layout`] to this: every host window was laid out with it.

/// The game drawer's pen advance for a space.
pub const GAME_SPACE_ADVANCE: usize = 5;
/// The game drawer's pen advance for `.` and `,`.
pub const GAME_POINT_ADVANCE: usize = 4;
/// How many rows below the Hangul cell's top the game drawer puts an ASCII glyph.
pub const GAME_ASCII_DROP: usize = 2;

/// One glyph as the GAME's drawer places it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameCell {
    /// Rows of pixels, MSB = leftmost (the same masks [`compose`] / [`ascii`] return).
    pub rows: Glyph,
    /// How far the pen moves after it.
    pub advance: usize,
    /// How many rows below the line's top the glyph's row 0 lands.
    pub drop: usize,
}

/// Lay a UTF-8 string out the way the game's own drawer does (see above). Hangul outside
/// KS X 1001 and ASCII control characters are skipped, like [`layout`] skips what it
/// cannot draw.
pub fn game_layout(text: &str) -> Vec<GameCell> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == ' ' {
            out.push(GameCell { rows: [0; KOR_H], advance: GAME_SPACE_ADVANCE, drop: 0 });
        } else if ch.is_ascii_graphic() {
            let advance = if matches!(ch, '.' | ',') { GAME_POINT_ADVANCE } else { ENG_ADVANCE };
            out.push(GameCell { rows: ascii(ch), advance, drop: GAME_ASCII_DROP });
        } else if !ch.is_ascii()
            && let Some(rows) = wansung_index(ch).and_then(compose)
        {
            out.push(GameCell { rows, advance: KOR_ADVANCE, drop: 0 });
        }
    }
    out
}

/// Width of `text` under the game drawer's metrics — what [`game_layout`] advances by.
pub fn game_width(text: &str) -> usize {
    game_layout(text).iter().map(|c| c.advance).sum()
}

/// A precomposed Hangul syllable's position in KS X 1001 (the order the game's
/// table is in), or `None` for anything else.
///
/// KS X 1001 lists 2350 syllables in EUC-KR code order, and its order is NOT the
/// Unicode one — U+AC00..U+D7A3 has 11172 syllables, most of which the standard
/// omits. So the mapping goes through the EUC-KR code, which is what
/// `KSX1001_FROM_UNICODE` holds.
pub fn wansung_index(ch: char) -> Option<usize> {
    let code = euckr_code(ch)?;
    let (hi, lo) = ((code >> 8) as usize, (code & 0xff) as usize);
    if !(0xb0..=0xc8).contains(&hi) || !(0xa1..=0xfe).contains(&lo) {
        return None;
    }
    let i = (hi - 0xb0) * 94 + (lo - 0xa1);
    (i < SYLLABLES).then_some(i)
}

/// The game's text-colour escapes, as its string drawer tests them: the NUL-terminated
/// `"@|${}'\""` at `.text 0x5b2bc` (file offset `0x5a2f0`), read out of `binary.mod`
/// (135차-B). A name's first byte from this set picks the colour and is not drawn.
pub const COLOUR_ESCAPES: [u8; 7] = [b'@', b'|', b'$', b'{', b'}', b'\'', b'"'];

/// Decode a NUL-terminated EUC-KR string the GAME wrote into a Rust `String`.
///
/// Needed since 2026-08-25, when the host started reading item names straight out of the
/// bag record (`rec+0x394`, baked there by the guest's own `0x5283c`) to show them in a
/// dialog. The rest of the overlay's Korean is Rust source, which is already UTF-8; this
/// is the one direction that has to come back.
///
/// The inverse of [`wansung_index`], and it uses the same table for the same reason: KS X
/// 1001's Hangul block is Unicode order with the gaps removed, so the EUC-KR code IS the
/// index. Anything outside that block — a cp949 extension row, hanja, a stray byte — has
/// no glyph in this font either, so it becomes `?` rather than being silently dropped: a
/// name with a visible gap is a bug report, a name that is quietly shorter is not.
///
/// ★ A leading COLOUR MARKER is stripped. The game's item names carry their text colour
/// as a first character (see `hero3_server/CASH_SHOP.md` §2.21, where mistaking them for
/// cash markers cost a whole round), and the game's own renderer consumes it rather than
/// drawing it. The alphabet is the game's own, [`COLOUR_ESCAPES`] = the seven bytes at
/// `0x5b2bc` — ⛔ not the five the cash shop happens to use: the 133차 legacy rows are `"`
/// and 135차 found `"붉은마녀의…` printed in the orb-combine target list.
pub fn from_euckr(bytes: &[u8]) -> String {
    let bytes = bytes.split(|b| *b == 0).next().unwrap_or(&[]);
    let bytes = match bytes.split_first() {
        Some((c, rest)) if COLOUR_ESCAPES.contains(c) => rest,
        _ => bytes,
    };
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let hi = bytes[i];
        if hi < 0x80 {
            out.push(hi as char);
            i += 1;
            continue;
        }
        let Some(&lo) = bytes.get(i + 1) else {
            out.push('?');
            break;
        };
        i += 2;
        let (hi, lo) = (hi as usize, lo as usize);
        if (0xb0..=0xc8).contains(&hi)
            && (0xa1..=0xfe).contains(&lo)
            && let Some(u) = crate::gamefont_data::KSX1001.get((hi - 0xb0) * 94 + (lo - 0xa1))
            && let Some(c) = char::from_u32(0xac00 + u32::from(*u))
        {
            out.push(c);
        } else {
            out.push('?');
        }
    }
    out
}

/// EUC-KR code for a precomposed Hangul syllable.
///
/// Derived rather than tabled: KS X 1001's Hangul block is the 2350 syllables in
/// Unicode order with the gaps removed, so a sorted list of the syllables it
/// contains is enough — and that list is exactly what the game's own `table` file
/// enumerates. `KSX1001_ORDER` is generated beside the font data.
fn euckr_code(ch: char) -> Option<u16> {
    let i = crate::gamefont_data::ksx1001_index(ch)?;
    let (row, col) = (i / 94, i % 94);
    Some((((0xb0 + row) as u16) << 8) | (0xa1 + col) as u16)
}

/// The inverse of [`from_euckr`]: a host string as the bytes the GUEST reads.
///
/// Needed since 103차, where the host appends to the game's own acquisition-notice queue
/// (`bagbox::notice_plan`) — that buffer belongs to the guest and its renderer decodes
/// EUC-KR. ASCII passes through; a Hangul syllable becomes its two-byte code.
///
/// ⛔ A character with no KS X 1001 code is DROPPED, not coerced. `from_euckr` turns an
/// unmappable byte pair into a visible `?` because a name arriving from the game must not
/// look shorter than it is; going the other way there is no pair to show, and writing
/// `ch as u8` (which is what this function's ancestor in the test module did) would put a
/// lone byte into the guest's buffer that its decoder would pair with the next name byte
/// and print as an entirely different syllable.
pub fn to_euckr(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for ch in s.chars() {
        match euckr_code(ch) {
            Some(code) => out.extend_from_slice(&code.to_be_bytes()),
            None if ch.is_ascii() && !ch.is_ascii_control() => out.push(ch as u8),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(g: &Glyph) -> Vec<String> {
        g.iter()
            .map(|r| (0..KOR_W).map(|x| if r & (1 << (15 - x)) != 0 { '#' } else { '.' }).collect())
            .collect()
    }

    /// The one direction that has to come BACK from the guest: item names.
    #[test]
    fn euckr_round_trips_through_the_font_table() {
        // Anything the font can draw must survive the trip, and the trip must be the
        // exact inverse of the index the drawer uses.
        for s in ["오브결합", "투신의 머리띠", "제련석", "소켓확장", "abc 123", "붉은사자흉갑"] {
            let enc: Vec<u8> = s
                .chars()
                .flat_map(|c| match euckr_code(c) {
                    Some(code) => vec![(code >> 8) as u8, code as u8],
                    None => vec![c as u8],
                })
                .collect();
            assert_eq!(from_euckr(&enc), s, "{s:?}");
        }
        // NUL terminates, whatever follows it.
        assert_eq!(from_euckr(b"ab cd"), "ab");
        assert_eq!(from_euckr(b""), "");
        // A leading colour marker is the game's, not the name's.
        let mut with_marker = vec![b'}'];
        with_marker.extend(from_str_euckr("제련석"));
        assert_eq!(from_euckr(&with_marker), "제련석");
        // A truncated pair and an unmappable code become a visible `?`, never a
        // silently shorter name.
        assert_eq!(from_euckr(&[0xbf]), "?");
        assert_eq!(from_euckr(&[0xff, 0xff]), "?");
        // A cp949 EXTENSION syllable has no glyph in this font and must not be invented.
        assert_eq!(from_euckr(&[0x88, 0x60]), "?", "extension rows are not KS X 1001");
    }

    fn from_str_euckr(s: &str) -> Vec<u8> {
        to_euckr(s)
    }

    #[test]
    fn the_four_etc_tables_add_up_to_the_whole_file() {
        assert_eq!(remap().len(), 93);
        assert_eq!(cho_bul().len(), 44);
        assert_eq!(jung_bul().len(), 40);
        assert_eq!(jong_bul().len(), 22);
        assert_eq!(remap().len() + cho_bul().len() + jung_bul().len() + jong_bul().len(), ETC.len());
    }

    /// Every set index the tables can produce has to land inside its bank, or a
    /// syllable would compose from whatever follows the font in memory.
    #[test]
    fn every_set_index_stays_inside_its_bank() {
        assert!(cho_bul().iter().all(|s| (*s as usize) < 8), "초성 has 8 sets");
        assert!(jung_bul().iter().all(|s| (*s as usize) < 4), "중성 has 4 sets");
        assert!(jong_bul().iter().all(|s| (*s as usize) < 4), "종성 has 4 sets");
        assert_eq!(CHO_BASE + 8 * CHO_SET, JUNG_BASE);
        assert_eq!(JUNG_BASE + 4 * JUNG_SET, JONG_BASE);
        assert_eq!(JONG_BASE + 4 * JONG_SET, KORB.len());
    }

    #[test]
    fn every_syllable_in_the_table_composes_without_panicking() {
        let mut drawn = 0;
        for i in 0..SYLLABLES {
            let g = compose(i).expect("in range");
            if g.iter().any(|r| *r != 0) {
                drawn += 1;
            }
        }
        assert_eq!(drawn, SYLLABLES, "every syllable must produce ink");
    }

    /// ★ The composition read out of the game, checked against a shape a human can
    /// verify: 가 is 초성 ㄱ on the left and 중성 ㅏ on the right, with no 종성 — so
    /// the left column group and the stem at x=8 must both be lit, and the bottom
    /// row must be empty.
    #[test]
    fn ga_is_the_shape_it_should_be() {
        let g = compose(wansung_index('가').unwrap()).unwrap();
        let a = art(&g);
        assert!(a[0].starts_with(".#####"), "ㄱ's top bar starts one px in: {}", a[0]);
        assert!(a.iter().take(10).all(|r| r.as_bytes()[8] == b'#'), "ㅏ's stem at x=8");
        assert_eq!(a[10], ".".repeat(KOR_W), "가 has no 종성, so the last row is clear");
    }

    /// …and one WITH a 종성, which selects different sets for all three parts.
    #[test]
    fn a_syllable_with_a_final_consonant_fills_the_bottom() {
        let g = compose(wansung_index('강').unwrap()).unwrap();
        let a = art(&g);
        assert!(a[9..].iter().any(|r| r.contains('#')), "ㅇ must reach the bottom rows");
        assert_ne!(compose(wansung_index('가').unwrap()).unwrap(), g, "강 differs from 가");
    }

    #[test]
    fn the_wansung_order_matches_the_games_own_table() {
        // 가 is the first syllable in KS X 1001, and its Johab code is the one the
        // table's first entry carries.
        assert_eq!(wansung_index('가'), Some(0));
        let johab = ((TABLE[0] as u16) << 8) | TABLE[1] as u16;
        assert_eq!(johab, 0x8861, "가 in Johab");
        // …and the last row of the block is where the table ends. ★ That is 힝
        // (U+D79D), NOT 힣: KS X 1001 holds 2350 of Unicode's 11172 syllables, and
        // the rest exist only in the cp949 EXTENSION rows, which this font has no
        // glyphs for. `layout` skips them — see its docs.
        assert_eq!(wansung_index('힝'), Some(SYLLABLES - 1));
        assert_eq!(wansung_index('힣'), None, "힣 is a cp949 extension, not KS X 1001");
        assert_eq!(wansung_index('a'), None);
        assert_eq!(wansung_index('㈜'), None);
    }

    #[test]
    fn ascii_covers_the_printable_range_and_nothing_else() {
        assert!(ascii(' ').iter().all(|r| *r == 0), "space is blank");
        assert!(ascii('A').iter().any(|r| *r != 0));
        assert!(ascii('~').iter().any(|r| *r != 0));
        assert!(ascii('\u{1}').iter().all(|r| *r == 0));
        assert!(ascii('가').iter().all(|r| *r == 0), "non-ASCII goes through `compose`");
    }

    #[test]
    fn layout_mixes_the_two_faces_and_measures_them() {
        let cells = layout("SAVE 저장");
        assert_eq!(cells.len(), 7, "5 ASCII + 2 Hangul");
        assert_eq!(cells[0].advance, ENG_ADVANCE);
        assert_eq!(cells[5].advance, KOR_ADVANCE);
        assert_eq!(width("SAVE 저장"), 5 * ENG_ADVANCE + 2 * KOR_ADVANCE);
        assert_eq!(width(""), 0);
        // A character the font cannot draw contributes nothing at all.
        assert_eq!(width("㈜"), 0);
    }
}
