// Standalone selftest for `wie_android/src/tower_hud.rs` (145차) — the 하늘성 banner's
// composition in the game's damage-digit font, against the GENERATED digit table
// (`wie_android/src/damage_digits.rs`, `python tools_re/gen_damage_digits.py --check`).
//
//   rustc --edition 2021 --test tower_hud_selftest.rs -o t.exe; .\t.exe
//
// No game font here: the words' glyphs are synthetic masks, handed in the way the
// overlay hands `gamefont::layout`'s cells. The font itself is `gamefont_selftest`'s.

mod damage_digits {
    include!("wie_android/src/damage_digits.rs");
}
include!("wie_android/src/tower_hud.rs");

use std::collections::HashMap;

/// A glyph that is one solid 4x4 block in the middle of the cell's rows 3..=6.
fn block_glyph() -> GlyphRows {
    let mut g = [0u16; GLYPH_H];
    for row in g.iter_mut().take(7).skip(3) {
        *row = 0b0111_1000_0000_0000; // columns 1..=4
    }
    g
}

/// Every character becomes the block glyph, 11 px apart (the game font's advance).
fn fake_glyphs(s: &str) -> Vec<(GlyphRows, usize)> {
    s.chars().map(|c| (if c == ' ' { [0u16; GLYPH_H] } else { block_glyph() }, if c == ' ' { 8 } else { 11 })).collect()
}

fn rows_of(set: &[[&str; 13]; 10]) -> Vec<Vec<u8>> {
    let mut out = vec![];
    for d in set {
        for (y, r) in d.iter().enumerate() {
            if out.len() <= y {
                out.push(vec![]);
            }
            out[y].extend(r.bytes());
        }
    }
    out
}

// ── the generated table ──────────────────────────────────────────────────────

#[test]
fn the_digit_tables_are_ten_cells_of_the_renderers_advance() {
    assert_eq!((damage_digits::BIG_W, damage_digits::BIG_H), (20, 20));
    assert_eq!((damage_digits::SMALL_W, damage_digits::SMALL_H), (14, 13));
    for d in damage_digits::BIG.iter() {
        for r in d.iter() {
            assert_eq!(r.len(), damage_digits::BIG_W);
            assert!(r.bytes().all(|c| matches!(c, b'.' | b'1'..=b'4')), "{r}");
        }
    }
    for d in damage_digits::SMALL.iter() {
        for r in d.iter() {
            assert_eq!(r.len(), damage_digits::SMALL_W);
            assert!(r.bytes().all(|c| matches!(c, b'.' | b'1'..=b'4')), "{r}");
        }
    }
    // Every digit carries all three fill shades inside its outline.
    let flat: Vec<String> = damage_digits::SMALL.iter().map(|d| d.concat()).chain(damage_digits::BIG.iter().map(|d| d.concat())).collect();
    for (i, all) in flat.iter().enumerate() {
        for c in ['1', '2', '3', '4'] {
            assert!(all.contains(c), "digit #{i} has no shade {c}");
        }
    }
}

/// `TEXT_RAMP` is the small digits' own per-row shade, re-derived here: the dominant
/// fill shade of rows 1..=11 across the ten digits.
#[test]
fn the_text_ramp_is_the_small_digits_rows() {
    let rows = rows_of(&damage_digits::SMALL);
    let mut derived = vec![];
    for row in rows.iter().take(12).skip(1) {
        let mut n: HashMap<u8, usize> = HashMap::new();
        for &c in row {
            if matches!(c, b'1'..=b'3') {
                *n.entry(c - b'0').or_default() += 1;
            }
        }
        let best = n.into_iter().max_by_key(|&(shade, count)| (count, shade)).map(|(s, _)| s).unwrap();
        derived.push(best);
    }
    assert_eq!(derived, TEXT_RAMP.to_vec());
}

// ── the pieces ───────────────────────────────────────────────────────────────

#[test]
fn a_number_is_its_digits_at_the_renderers_advance() {
    let b = digits("12", Size::Big);
    assert_eq!((b.w, b.h), (40, 20));
    let s = digits("7", Size::Small);
    assert_eq!((s.w, s.h), (14, 13));
    // Pixel-exact copy of the table: the 7's first outline row.
    let row0: String = (0..14).map(|x| match s.get(x, 0) { 0 => '.', v => (b'0' + v) as char }).collect();
    assert_eq!(row0, damage_digits::SMALL[7][0]);
    // Anything that is not a digit is skipped; nothing at all is an empty bitmap.
    assert_eq!(digits("1a2", Size::Small).w, 28);
    assert!(digits("", Size::Big).is_empty());
}

#[test]
fn a_word_is_filled_by_row_and_rimmed_like_the_digits() {
    let t = text(&[(block_glyph(), 11)]);
    assert_eq!(t.h, GLYPH_H + 2, "the small digits' height");
    // The block (font columns 1..=4, bolded to 1..=5) sits at x 2..=6, rows 4..=7.
    for y in 4..=7 {
        for x in 2..=(6) {
            assert_eq!(t.get(x, y), TEXT_RAMP[y - 1], "fill at ({x},{y})");
        }
        assert_eq!(t.get(1, y), INK_OUTLINE, "1-px rim on the left");
        assert_eq!(t.get(7, y), INK_OUTLINE, "rim on the right");
        assert_eq!(t.get(8, y), INK_OUTLINE, "the right rim is 2 px, like the digits'");
        assert_eq!(t.get(9, y), INK_NONE);
        assert_eq!(t.get(0, y), INK_NONE);
    }
    assert_eq!(t.get(4, 3), INK_OUTLINE, "rim above");
    assert_eq!(t.get(4, 8), INK_OUTLINE, "rim below");
    assert_eq!(t.get(1, 3), INK_OUTLINE, "diagonal corners too");
    assert_eq!(t.get(4, 2), INK_NONE);
    // Trimmed to the last painted column.
    assert_eq!(t.w, 9);
    // Nothing to draw takes no room.
    assert!(text(&[]).is_empty());
}

#[test]
fn a_number_and_its_unit_share_the_bottom_row() {
    let n = digits("3", Size::Big);
    let u = text(&[(block_glyph(), 11)]);
    let line = hcat(&[n.clone(), u.clone()], WORD_GAP);
    assert_eq!(line.h, 20);
    assert_eq!(line.w, n.w + WORD_GAP + u.w);
    // The unit's rows land at the bottom: its first rim row is 20 - 13 + 3.
    assert_eq!(line.get(n.w + WORD_GAP + 4, 20 - 13 + 3), INK_OUTLINE);
    assert_eq!(line.get(n.w + WORD_GAP + 4, 20 - 13 + 2), INK_NONE);
    // Empty pieces take no room.
    assert_eq!(hcat(&[n.clone(), Bitmap::new(0, 0)], 5).w, n.w);
}

#[test]
fn lines_are_centred_on_the_widest() {
    let wide = digits("100", Size::Big);
    let narrow = digits("7", Size::Small);
    let b = vcat(&[wide.clone(), narrow.clone()], LINE_GAP);
    assert_eq!(b.w, wide.w);
    assert_eq!(b.h, wide.h + LINE_GAP + narrow.h);
    let x0 = (wide.w - narrow.w) / 2;
    let y0 = wide.h + LINE_GAP;
    for y in 0..narrow.h {
        for x in 0..narrow.w {
            let v = narrow.get(x, y);
            if v != INK_NONE {
                assert_eq!(b.get(x0 + x, y0 + y), v);
            }
        }
    }
}

// ── the three inks (147차) ────────────────────────────────────────────────────

#[test]
fn an_index_is_a_group_and_a_shade() {
    // Untinted pieces are group 0 with the digit table's shades — what 145차 drew.
    for v in 1..=4u8 {
        assert_eq!((group_of(v), shade_of(v)), (0, v));
    }
    assert_eq!((group_of(INK_NONE), shade_of(INK_NONE)), (0, INK_NONE));
    // Every group spans exactly four indices, outline last.
    for g in 0..INK_GROUPS as u8 {
        for s in 1..=4u8 {
            let v = 4 * g + s;
            assert_eq!((group_of(v), shade_of(v)), (usize::from(g), s), "index {v}");
        }
    }
    // The enum is the overlay's palette row.
    assert_eq!([Ink::Floor as usize, Ink::Label as usize, Ink::Count as usize], [0, 1, 2]);
    assert_eq!(INK_GROUPS, 3);
}

#[test]
fn tinting_keeps_the_shape_and_the_shades() {
    let t = text(&[(block_glyph(), 11)]);
    for ink in [Ink::Floor, Ink::Label, Ink::Count] {
        let c = tint(&t, ink);
        assert_eq!((c.w, c.h), (t.w, t.h));
        for y in 0..t.h {
            for x in 0..t.w {
                let (a, b) = (t.get(x, y), c.get(x, y));
                assert_eq!(a == INK_NONE, b == INK_NONE, "transparency moved at ({x},{y})");
                if a != INK_NONE {
                    assert_eq!(shade_of(b), shade_of(a), "shade changed at ({x},{y})");
                    assert_eq!(group_of(b), ink as usize);
                }
            }
        }
        // Idempotent, and a re-tint moves the piece to the new group wholesale.
        assert_eq!(tint(&c, ink), c);
        assert_eq!(tint(&c, Ink::Floor), tint(&t, Ink::Floor));
    }
}

#[test]
fn a_fill_beats_a_rim_of_any_group() {
    // A Count-group fill pixel under a Label-group rim keeps its fill, and a rim never
    // lands on a fill — the 145차 rule, now across groups.
    let mut b = Bitmap::new(2, 1);
    let fill = 4 * Ink::Count as u8 + 2;
    let rim = 4 * Ink::Label as u8 + INK_OUTLINE;
    b.put(0, 0, fill);
    b.put(0, 0, rim);
    assert_eq!(b.get(0, 0), fill);
    b.put(1, 0, rim);
    b.put(1, 0, fill);
    assert_eq!(b.get(1, 0), fill, "a fill replaces a rim");
}

/// The groups present in columns `x0..x1` of a bitmap (painted pixels only).
fn groups_in(b: &Bitmap, x0: usize, x1: usize, y0: usize, y1: usize) -> Vec<usize> {
    let mut out = vec![];
    for y in y0..y1 {
        for x in x0..x1 {
            let v = b.get(x, y);
            if v != INK_NONE && !out.contains(&group_of(v)) {
                out.push(group_of(v));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_floor_the_label_and_the_count_are_three_colours() {
    let b = banner(12, false, Status::Left(7), &fake_glyphs);
    let first = hcat(&[digits("12", Size::Big), text(&fake_glyphs("층"))], WORD_GAP);
    let label = text(&fake_glyphs("남은 몬스터"));
    let count = digits("7", Size::Small);
    let second = hcat(&[label.clone(), count.clone()], WORD_GAP + 2);
    // The whole first line is the floor's ink.
    let x0 = (b.w - first.w) / 2;
    assert_eq!(groups_in(&b, x0, x0 + first.w, 0, first.h), vec![Ink::Floor as usize]);
    // The second line: the label's columns are the label's ink, the count's the count's.
    let (sx, sy) = ((b.w - second.w) / 2, first.h + LINE_GAP);
    assert_eq!(groups_in(&b, sx, sx + label.w, sy, b.h), vec![Ink::Label as usize]);
    let cx = sx + label.w + WORD_GAP + 2;
    assert_eq!(groups_in(&b, cx, cx + count.w, sy, b.h), vec![Ink::Count as usize]);
    // All three are present, and nothing else is.
    assert_eq!(groups_in(&b, 0, b.w, 0, b.h), vec![0, 1, 2]);
}

#[test]
fn a_boss_word_is_the_floors_and_a_status_word_is_the_labels() {
    let boss = banner(100, true, Status::Word("클리어"), &fake_glyphs);
    let first_h = 20;
    assert_eq!(groups_in(&boss, 0, boss.w, 0, first_h), vec![Ink::Floor as usize], "「100층 보스」 is one colour");
    assert_eq!(groups_in(&boss, 0, boss.w, first_h + LINE_GAP, boss.h), vec![Ink::Label as usize], "a status word takes the label's ink");
    // No count on the line = no count ink anywhere.
    assert!(!groups_in(&boss, 0, boss.w, 0, boss.h).contains(&(Ink::Count as usize)));
}

// ── the banner ───────────────────────────────────────────────────────────────

#[test]
fn the_banner_is_the_floor_over_the_count() {
    let b = banner(12, false, Status::Left(7), &fake_glyphs);
    let first = hcat(&[tint(&digits("12", Size::Big), Ink::Floor), tint(&text(&fake_glyphs("층")), Ink::Floor)], WORD_GAP);
    let second = hcat(&[tint(&text(&fake_glyphs("남은 몬스터")), Ink::Label), tint(&digits("7", Size::Small), Ink::Count)], WORD_GAP + 2);
    assert_eq!(b, vcat(&[first.clone(), second.clone()], LINE_GAP));
    assert_eq!(b.h, 20 + LINE_GAP + 13);
    assert_eq!(b.w, first.w.max(second.w));
}

#[test]
fn a_boss_floor_adds_the_word_and_a_status_word_replaces_the_count() {
    let plain = banner(100, false, Status::Word("클리어"), &fake_glyphs);
    let boss = banner(100, true, Status::Word("클리어"), &fake_glyphs);
    assert!(boss.w > plain.w, "「보스」 widens the first line");
    let word = banner(3, false, Status::Word("클리어"), &fake_glyphs);
    let count = banner(3, false, Status::Left(20), &fake_glyphs);
    assert_ne!(word, count);
    assert_eq!(word.h, count.h, "both second lines are the small set's height");
}

#[test]
fn the_banner_is_four_fifths_of_the_damage_numbers() {
    // 147차: 0.8 of the game→window factor (145차 drew at exactly that factor).
    assert!((SCALE - 0.8).abs() < 1e-12);
    assert!((pixel_scale(1080, 240) - 3.6).abs() < 1e-9, "the S9: x4.5 -> x3.6");
    assert!((pixel_scale(1440, 240) - 4.8).abs() < 1e-9);
    assert_eq!(pixel_scale(1080, 0), 0.0, "no logical height, nothing drawn");
    // Smaller than the game's own numbers, but never smaller than half of them.
    assert!(SCALE < 1.0 && SCALE >= 0.5);
}

#[test]
fn the_banner_hangs_seven_percent_down_and_centred() {
    // The S9's game rect (2220x1080, logical 496x240 -> x4.5, the banner x3.6) and an
    // 82-px banner.
    let s = pixel_scale(1080, 240);
    let (x, y) = place(0, 0, 2220, 1080, s, 82);
    assert!((y - 75.6).abs() < 1e-9, "7 % of 1080");
    assert!((x - (2220.0 - 82.0 * s) / 2.0).abs() < 1e-9);
    // Offset game rects are respected.
    let (x2, y2) = place(100, 40, 2000, 1000, 4.0, 50);
    assert!((y2 - 110.0).abs() < 1e-9);
    assert!((x2 - (100.0 + (2000.0 - 200.0) / 2.0)).abs() < 1e-9);
    // A banner wider than the rect never goes negative.
    assert_eq!(place(0, 0, 100, 100, 4.0, 500).0, 0.0);
    // Higher than 145차's 10 %, and the first ink (the big digits' two blank top rows
    // below the edge) still clears the EXIT tab (144차: 58 px of the S9's 1080, flush
    // with the top) with room to spare.
    assert!(TOP_FRACTION < 0.10);
    let blank_rows = damage_digits::BIG.iter().map(|d| d.iter().take_while(|r| r.bytes().all(|c| c == b'.')).count()).min().unwrap();
    assert_eq!(blank_rows, 2);
    assert!(y + blank_rows as f64 * s - 58.0 >= 20.0, "gap under the EXIT tab");
}
