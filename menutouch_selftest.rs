// Standalone self-test of the menu-touch geometry (66차).
//
//   rustc --edition 2021 --test menutouch_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host (the linked
// winit/android deps abort at startup), so this compiles the REAL source — not a
// copy — as its own test crate. `menutouch.rs` is dependency-free for exactly that
// reason; if it ever grows a `use crate::...` this file stops building, which is the
// intended alarm.
//
// ★ Like `padhud_selftest.rs` and unlike `items_selftest.rs` / `landscape_selftest.rs`,
// this is NOT a hand-written mirror. It includes the source, so it cannot drift.
//
// The numbers asserted here are the PROBE MEASUREMENTS (wie_probe on the shipped LGT
// asset, 66차), not values re-derived the way the code derives them:
//
//   496x240 -> panel x 160..335  y  40..199      560x228 -> panel x 192..367  y 34..193
//   menu rows      dy 24 43 62 81 100 119, each 15 tall, ink x 24..151
//   inventory cell cursor boxes  x 13..29 / 70..86 …  y 22..38 / 41..57   (17x17, pitch 19)
//   badge ink 0xd6b294:  tabs   dy 2..12   on 인벤·장비·스킬·퀘스트·상태
//                        pages  dy 131..141 (인벤)  ·  dy 145..155 (스킬·퀘스트)
//                        none at all on the six-row MENU list

include!("wie_android/src/menutouch.rs");

// --------------------------------------------------------------------------
// helpers: build a synthetic logical framebuffer with badge ink where the probe
// found it, so the scanner is tested against the measured layout.
// --------------------------------------------------------------------------

const BG: u32 = 0xff00_0000;
const INK: u32 = 0xff00_0000 | BADGE_INK;
/// The header TITLE ink, which differs from the badge ink by 8 in the blue channel.
/// The scanner must not take it for a badge (it is why the x columns are narrow).
const TITLE_INK: u32 = 0xffd6_b29c;
/// The game's selection-border yellow, opaque.
const INK_YEL: u32 = 0xff00_0000 | CELL_BOX_INK;

struct Fb {
    px: Vec<u32>,
    lw: i32,
    lh: i32,
}

impl Fb {
    fn new(lw: i32, lh: i32) -> Self {
        Fb { px: vec![BG; (lw * lh) as usize], lw, lh }
    }
    fn set(&mut self, x: i32, y: i32, v: u32) {
        if x >= 0 && x < self.lw && y >= 0 && y < self.lh {
            self.px[(y * self.lw + x) as usize] = v;
        }
    }
    /// Paint an 11x11 badge glyph at panel-relative `(col_x, dy0)`: a filled block is
    /// plenty (the real glyph is ~90 ink px inside the same 11x11).
    fn badge(&mut self, col_x: i32, dy0: i32) {
        let (ox, oy) = panel_origin(self.lw, self.lh).unwrap();
        for dy in dy0..dy0 + 11 {
            for i in 0..BADGE_W {
                self.set(ox + col_x + i, oy + dy, INK);
            }
        }
    }
    /// The measured badge sets, by screen.
    fn tabs(&mut self) {
        self.badge(BADGE_L_X, 2);
        self.badge(BADGE_R_X, 2);
    }
    fn pages(&mut self, dy0: i32) {
        self.badge(BADGE_L_X, dy0);
        self.badge(BADGE_R_X, dy0);
    }
    fn found(&self) -> Vec<BadgeHit> {
        badges(&self.px, self.lw, self.lh)
    }
    fn at(&self, dx: i32, dy: i32) -> Option<Badge> {
        badge_at(&self.found(), dx, dy)
    }
}

// --------------------------------------------------------------------------
// the measured constants themselves
// --------------------------------------------------------------------------

/// ★ Every other test in this file derives its expectations FROM these constants, so a
/// mistyped re-measurement would keep the whole suite green while sending taps to the
/// wrong row, cell or column. (Demonstrated: `CELL_Y0 = 30` passes all of them and maps
/// the top of the game's visual row 1 to cell row 0.) So the numbers are pinned here as
/// LITERALS, exactly as the probe read them off the dumped frames — this is the test
/// that has to fail when a constant changes.
#[test]
fn the_constants_are_the_probe_numbers() {
    // Panel: 176x160, centred.
    assert_eq!((PANEL_W, PANEL_H), (176, 160));
    // Menu list: bands at dy 24 43 62 81 100 119, 15 tall, six of them.
    assert_eq!((MENU.y0, MENU.pitch_y, MENU.h, MENU.rows), (24, 19, 15, 6));
    assert_eq!((MENU.x0, MENU.w, MENU.cols), (24, 128, 1));
    // Inventory grid: cursor boxes (13 + 19c, 22 + 19r, 17x17), 8 x 2.
    assert_eq!((INVEN.x0, INVEN.y0, INVEN.pitch_x, INVEN.w), (13, 22, 19, 17));
    assert_eq!((INVEN.cols, INVEN.rows, PAGES), (8, 2, 8));
    // 스킬 3x2: yellow border 42x21 at x 34/81/128, y 19/43 (device-measured).
    assert_eq!((SKILL.x0, SKILL.pitch_x, SKILL.w, SKILL.cols), (34, 47, 42, 3));
    assert_eq!((SKILL.y0, SKILL.pitch_y, SKILL.h, SKILL.rows), (19, 24, 21, 2));
    // 퀘스트 6 rows: border 163x16 at y 23 + 19i, x 6.
    assert_eq!((QUEST.x0, QUEST.w, QUEST.cols), (6, 163, 1));
    assert_eq!((QUEST.y0, QUEST.pitch_y, QUEST.h, QUEST.rows), (23, 19, 16, 6));
    // SYSTEM 4 rows: pitch 20, NOT the menu's 19.
    assert_eq!((SYSTEM.x0, SYSTEM.w, SYSTEM.cols), (34, 108, 1));
    assert_eq!((SYSTEM.y0, SYSTEM.pitch_y, SYSTEM.h, SYSTEM.rows), (32, 20, 15, 4));
    // Badges: ink 0xd6b294 in two 11-px columns at dx 15 and dx 151.
    assert_eq!(BADGE_INK, 0x00d6_b294);
    assert_eq!((BADGE_L_X, BADGE_R_X, BADGE_W), (15, 151, 11));
}

/// The two cursor boxes the probe actually measured, by LITERAL coordinate: the frame
/// diff between cell 0 and cell 3 changed exactly 100 px, in the boxes
/// x 13..29 / x 70..86 at y 22..38. Row 1 is the same, one pitch lower.
#[test]
fn the_measured_cursor_boxes_land_on_their_own_cells() {
    for (dx, dy, want) in [
        (13, 22, 0),
        (29, 38, 0),
        (70, 22, 3),
        (86, 38, 3),
        (70, 41, 11),
        (86, 57, 11),
        (146, 41, 15),
        (162, 57, 15),
    ] {
        assert_eq!(INVEN.hit(dx, dy), Some(want), "the measured box corner ({dx},{dy})");
    }
    // …and the row/badge bands, by literal dy, on the same footing.
    assert_eq!(MENU.hit(88, 24), Some(0), "the first measured band");
    assert_eq!(MENU.hit(88, 119), Some(5), "the last measured band");
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    fb.pages(131);
    assert_eq!(fb.at(20, 7), Some(Badge::Tab1), "tab ink measured at dy 2..12");
    assert_eq!(fb.at(20, 136), Some(Badge::Page7), "inventory page ink measured at dy 131..141");
}

// --------------------------------------------------------------------------
// the panel
// --------------------------------------------------------------------------

/// ★ The whole feature rides on this: the panel is a FIXED 176x160 centred both ways,
/// so every offset below is resolution-independent. Two of these four are 66차 probe
/// measurements (496x240 and 560x228, read straight off dumped frames); the other two
/// are 63차-B's.
#[test]
fn the_panel_is_176x160_centred_at_every_measured_logical_size() {
    assert_eq!(panel_origin(496, 240), Some((160, 40)));
    assert_eq!(panel_origin(496, 228), Some((160, 34)));
    assert_eq!(panel_origin(560, 240), Some((192, 40)));
    assert_eq!(panel_origin(560, 228), Some((192, 34)));
    assert_eq!(panel_origin(440, 220), Some((132, 30)));
    // The portrait/classic framebuffer the app boots with before the window size is
    // known has to work too — it is bigger than the panel in both axes.
    assert_eq!(panel_origin(240, 320), Some((32, 80)));
}

/// A framebuffer smaller than the panel gets NO geometry rather than a negative
/// origin — the game does not draw the panel that way, so we refuse to guess.
#[test]
fn a_framebuffer_smaller_than_the_panel_has_no_geometry() {
    assert_eq!(panel_origin(175, 240), None);
    assert_eq!(panel_origin(496, 159), None);
    assert_eq!(panel_origin(0, 0), None);
    assert_eq!(panel_point(100, 100, 50, 50), None);
}

#[test]
fn panel_point_is_the_measured_translation_and_rejects_the_outside() {
    // 496x240: the panel's own top-left corner, its last pixel, and one past each edge.
    assert_eq!(panel_point(496, 240, 160, 40), Some((0, 0)));
    assert_eq!(panel_point(496, 240, 335, 199), Some((175, 159)));
    assert_eq!(panel_point(496, 240, 159, 100), None);
    assert_eq!(panel_point(496, 240, 336, 100), None);
    assert_eq!(panel_point(496, 240, 200, 39), None);
    assert_eq!(panel_point(496, 240, 200, 200), None);
    // …and the same point in the other measured size lands on the same offset.
    assert_eq!(panel_point(560, 228, 192 + 80, 34 + 30), Some((80, 30)));
}

/// ★★★ 80차-F — the FOOTER variant takes the same points plus the band just under the
/// panel, because the game draws the footer prompt's glyphs there (measured `#` dy
/// 155..161, panel ends at 160). Sideways it is exactly as strict as [`panel_point`].
#[test]
fn panel_point_footer_takes_the_band_below_the_panel_and_nothing_else() {
    // Everything `panel_point` takes, it takes, unchanged.
    for (x, y) in [(160, 40), (335, 199), (200, 100)] {
        assert_eq!(panel_point_footer(496, 240, x, y), panel_point(496, 240, x, y), "({x},{y})");
    }
    // The overhang: `dy` comes back GREATER than the panel's own height on purpose —
    // that is the signal `app::menu_tap_decide` splits on.
    assert_eq!(panel_point_footer(496, 240, 200, 200), Some((40, 160)));
    assert_eq!(panel_point_footer(496, 240, 200, 40 + FOOTER_BOX_BOT - 1), Some((40, FOOTER_BOX_BOT - 1)));
    assert_eq!(panel_point_footer(496, 240, 200, 40 + FOOTER_BOX_BOT), None, "one row past the band");
    // ⛔ Not one pixel wider than the panel, and nothing ABOVE it either.
    assert_eq!(panel_point_footer(496, 240, 159, 200), None);
    assert_eq!(panel_point_footer(496, 240, 336, 200), None);
    assert_eq!(panel_point_footer(496, 240, 200, 39), None);
    assert_eq!(panel_point_footer(0, 0, 0, 0), None);
}

// --------------------------------------------------------------------------
// the six-row MENU list
// --------------------------------------------------------------------------

/// The measured ink bands, dead centre of each.
#[test]
fn every_measured_row_band_maps_to_its_own_row() {
    for (i, top) in [24, 43, 62, 81, 100, 119].into_iter().enumerate() {
        for dy in top..top + MENU.h {
            assert_eq!(MENU.hit(88, dy), Some(i as u8), "dy {dy} is inside row {i} (ink {top}..)");
        }
    }
}

/// The 4-px gap between two ink bands is nobody's ink — but a thumb cannot avoid it, so
/// it must never be a MISS. The two neighbours split it, half each: that is the nearest-
/// cell rule, and losing the tap there was the "터치가 완벽하지 않으면" complaint that
/// started this work.
#[test]
fn every_pixel_inside_the_list_belongs_to_a_row() {
    for dy in MENU.y0..MENU.y0 + MENU.pitch_y * MENU.rows {
        let got = MENU.hit(88, dy).expect("no dy inside the list may be a miss");
        let row = ((dy - MENU.y0) / MENU.pitch_y) as u8;
        if (dy - MENU.y0) % MENU.pitch_y < MENU.h {
            assert_eq!(got, row, "dy {dy} is inside row {row}'s own ink");
        } else {
            assert!(got == row || got == row + 1, "gap dy {dy} went to {got}, not a neighbour");
        }
    }
}

/// ⛔ THE most important test in this file. The game clamps this cursor only inside its
/// KEY arms; its OK arm has no range check, so a 6 dispatches `.data 0x14001d4` index 7
/// and opens a screen the menu itself cannot reach (measured, 63차-B §7-1).
#[test]
fn row_at_can_never_answer_past_the_last_row() {
    for dy in -50..400 {
        for dx in [-5, 0, 9, 10, 88, 165, 166, 300] {
            if let Some(r) = MENU.hit(dx, dy) {
                assert!(r <= 5, "MENU.hit({dx},{dy}) = {r}, which the game cannot survive");
            }
        }
    }
}

#[test]
fn taps_outside_the_list_are_not_rows() {
    // The hit region is the TILED grid (each row owns its pitch) grown by `pad` on each
    // side; one px past that is nothing at all.
    let gap = MENU.pitch_y - MENU.h;
    let top = MENU.y0 - gap / 2 - MENU.pad_y;
    let bottom = MENU.y0 - gap / 2 + MENU.pitch_y * MENU.rows + MENU.pad_y;
    assert_eq!(MENU.hit(88, top - 1), None);
    assert_eq!(MENU.hit(88, bottom), None);
    assert_eq!(MENU.hit(88, top), Some(0), "the top of the slack still means row 0");
    assert_eq!(MENU.hit(88, bottom - 1), Some(5), "the bottom of the slack still means row 5");
    // Left/right of the row strip (the panel is 176 wide; the ink is 24..151).
    assert_eq!(MENU.hit(MENU.x0 - MENU.pad_x - 1, 30), None);
    assert_eq!(MENU.hit(MENU.x0 + MENU.w + MENU.pad_x, 30), None);
}

/// The row strip is wider than the ink but never leaves the panel — a target that
/// spilled out would eat the game screen beside the panel.
#[test]
fn the_row_strip_stays_inside_the_panel_and_covers_the_ink() {
    assert!(MENU.x0 - MENU.pad_x >= 0 && MENU.x0 + MENU.w + MENU.pad_x <= PANEL_W);
    assert!(MENU.x0 <= 24 && MENU.x0 + MENU.w >= 152, "the strip must contain the measured ink x 24..151");
}

// --------------------------------------------------------------------------
// the inventory grid
// --------------------------------------------------------------------------

/// Every one of the 16 measured cursor boxes maps to its own index, corner to corner.
/// The boxes are the probe's: `(13 + 19c, 22 + 19r, 17, 17)`.
#[test]
fn every_measured_cell_box_maps_to_its_own_index() {
    for r in 0..INVEN.rows {
        for c in 0..INVEN.cols {
            let (x0, y0) = (INVEN.x0 + INVEN.pitch_x * c, INVEN.y0 + INVEN.pitch_y * r);
            let want = Some((r * INVEN.cols + c) as u8);
            for (dx, dy) in [
                (x0, y0),
                (x0 + INVEN.w - 1, y0),
                (x0, y0 + INVEN.h - 1),
                (x0 + INVEN.w - 1, y0 + INVEN.h - 1),
                (x0 + INVEN.w / 2, y0 + INVEN.h / 2),
            ] {
                assert_eq!(INVEN.hit(dx, dy), want, "cell ({r},{c}) box corner ({dx},{dy})");
            }
        }
    }
}

/// The 2-px gutter between two boxes is not a miss: it goes to one of the two
/// neighbours, never to nothing.
#[test]
fn the_gutters_between_cells_are_never_a_miss() {
    for dy in INVEN.y0..INVEN.y0 + INVEN.pitch_y * INVEN.rows - 1 {
        for dx in INVEN.x0..INVEN.x0 + INVEN.pitch_x * INVEN.cols - 1 {
            assert!(INVEN.hit(dx, dy).is_some(), "({dx},{dy}) inside the grid answered None");
        }
    }
}

#[test]
fn cell_at_can_never_answer_past_the_last_cell() {
    for dy in -50..200 {
        for dx in -50..250 {
            if let Some(v) = INVEN.hit(dx, dy) {
                assert!(v <= 15, "INVEN.hit({dx},{dy}) = {v}");
            }
        }
    }
}

/// Nothing above the grid (that band is the tab badges) and nothing below it (that is
/// the item detail box) may read as a cell.
#[test]
fn the_grid_does_not_spill_into_the_badge_row_or_the_detail_box() {
    let top = INVEN.y0 - (INVEN.pitch_y - INVEN.h) / 2;
    assert_eq!(INVEN.hit(20, top - 1), None);
    assert_eq!(INVEN.hit(20, 12), None, "the «1» badge row is not a cell");
    let bottom = top + INVEN.pitch_y * INVEN.rows;
    assert_eq!(INVEN.hit(20, bottom), None);
    assert_eq!(INVEN.hit(20, 70), None, "the detail box is not a cell");
    // And the columns stop at the 8th.
    assert_eq!(INVEN.hit(INVEN.x0 + INVEN.pitch_x * INVEN.cols + 2, 30), None);
}


/// The selection borders the DEVICE showed, each next to the cursor value the whole-record
/// watcher reported at that moment (66차-B). This is the test that pins the three new
/// grids to reality rather than to each other.
#[test]
fn the_measured_selection_borders_map_to_their_own_index() {
    // 스킬 3x2 — five of the six were seen directly (value 4 follows from the pitch).
    for (x, y, idx) in [(34, 19, 0u8), (81, 19, 1), (128, 19, 2), (34, 43, 3), (128, 43, 5)] {
        assert_eq!(SKILL.hit(x, y), Some(idx), "skill border at ({x},{y})");
        assert_eq!(SKILL.hit(x + SKILL.w - 1, y + SKILL.h - 1), Some(idx), "its far corner");
    }
    // 퀘스트 — the border of row 1 was x 6..168, y 42..57.
    assert_eq!(QUEST.hit(6, 42), Some(1));
    assert_eq!(QUEST.hit(168, 57), Some(1));
    assert_eq!(QUEST.hit(88, 23), Some(0), "the first row band");
    assert_eq!(QUEST.hit(88, 23 + 19 * 5), Some(5), "the last row band");
    // SYSTEM — the four bands, and the pitch that is 20 rather than 19.
    for (i, top) in [32, 52, 72, 92].into_iter().enumerate() {
        assert_eq!(SYSTEM.hit(88, top), Some(i as u8), "system band {i}");
        assert_eq!(SYSTEM.hit(88, top + SYSTEM.h - 1), Some(i as u8));
    }
    // Past the last row plus its slack there is nothing — a fifth row does not exist.
    let past = SYSTEM.y0 - (SYSTEM.pitch_y - SYSTEM.h) / 2 + SYSTEM.pitch_y * SYSTEM.rows + SYSTEM.pad_y;
    assert_eq!(SYSTEM.hit(88, past), None, "there is no fifth row");
    assert_eq!(SYSTEM.hit(88, past - 1), Some(3), "…and the slack still belongs to the last one");
}

/// ⛔ No grid may EVER answer an index it does not hold — that byte is written straight
/// into the character record and the game's OK arm re-reads it with no range check.
#[test]
fn no_grid_can_answer_outside_itself() {
    for g in [MENU, INVEN, SKILL, QUEST, SYSTEM] {
        for dy in -30..PANEL_H + 30 {
            for dx in -30..PANEL_W + 30 {
                if let Some(v) = g.hit(dx, dy) {
                    assert!(g.holds(v), "{g:?} answered {v} at ({dx},{dy})");
                }
            }
        }
    }
}

/// The SHARED list widget (상점 · 제련 · …), pinned to the three device samples. The row
/// block is CENTRED, so the first row moves with the row count — that is the whole reason
/// this is a function and not a table.
#[test]
fn the_shared_widget_rows_are_the_three_device_samples() {
    // 상점 구매/판매: 2 rows -> row 0 at dy 48, row 1 at dy 69.
    let g2 = choice_grid(2).unwrap();
    assert_eq!((g2.y0, g2.pitch_y, g2.h, g2.rows), (48, 21, 20, 2));
    assert_eq!((g2.x0, g2.w), (19, 139));
    assert_eq!(g2.hit(88, 48), Some(0));
    assert_eq!(g2.hit(88, 67), Some(0), "row 0 ends at dy 67");
    assert_eq!(g2.hit(88, 69), Some(1), "row 1 starts at dy 69");
    assert_eq!(g2.hit(88, 88), Some(1));
    // 상점 카테고리 and 제련 선택지: 3 rows -> row 0 at dy 37, row 1 at dy 58.
    let g3 = choice_grid(3).unwrap();
    assert_eq!(g3.y0, 37);
    assert_eq!(g3.hit(88, 37), Some(0));
    assert_eq!(g3.hit(88, 58), Some(1));
    assert_eq!(g3.hit(88, 79), Some(2));
    // The block really is centred: its middle does not move with the count. (The two
    // differ by one px because half a pitch is 10.5 and the origin floors — which is what
    // the device showed: 48 for two rows, 37 for three.)
    let mid = |g: &Grid| 2 * g.y0 + g.pitch_y * g.rows;
    assert!((mid(&g2) - mid(&g3)).abs() <= 1, "the two counts share a centre");
    // Nothing at all for a count the game never shows.
    assert_eq!(choice_grid(0).map(|g| g.rows), None);
    assert_eq!(choice_grid(CHOICE_ROWS_MAX + 1).map(|g| g.rows), None);
    assert_eq!(choice_grid(0xff).map(|g| g.rows), None);
    // …and every count in range keeps its rows inside the panel. ★★ This assertion used
    // to read `y0 >= -PANEL_H && bottom <= PANEL_H + PANEL_H`, which is a whole panel of
    // slack in both directions: it passed at `y0 = -99`, where the grid claims all 160 dy
    // and every tap in the window is a row. 66차-C review. The invariant is the real one —
    // the block is INSIDE the panel — and the probe only uses coordinates `panel_point`
    // can actually produce (dy 0..PANEL_H); a negative dy proves nothing about a real tap.
    for rows in 1..=CHOICE_ROWS_MAX {
        let g = choice_grid(rows).unwrap();
        assert!(g.y0 >= 0, "rows={rows}: row 0 at dy {} is above the panel", g.y0);
        assert!(g.y0 + g.pitch_y * g.rows <= PANEL_H, "rows={rows}: the block runs past the panel");
        for i in 0..g.rows {
            let dy = g.y0 + g.pitch_y * i;
            assert!((0..PANEL_H).contains(&dy));
            assert_eq!(g.hit(88, dy), Some(i as u8), "rows={rows} row {i}");
        }
        // …and it never claims the whole panel: something outside the block must miss.
        assert!((0..PANEL_H).any(|dy| g.hit(88, dy).is_none()), "rows={rows} claims every dy");
    }
}

/// ⛔⛔ A row count we have never measured is REFUSED, not guessed (66차-C review).
///
/// The widget's count byte (`CHAR+0xa285`) has no known writer and keeps the PREVIOUS
/// screen's value, and this is the one tap path with no pixel cross-check — so a count of
/// 7..=255 must produce no grid at all. What it produced before: 7 rows started at dy -4
/// and 9+ rows accepted EVERY dy in the panel, so a tap on the window frame resolved to a
/// row; the game focuses row 0 when a list opens, so that tap became a synthesised OK on a
/// 상점/제련 row the finger never touched.
#[test]
fn an_unmeasured_row_count_is_refused_rather_than_guessed() {
    for rows in 7u8..=64 {
        assert!(choice_grid(rows).is_none(), "rows={rows} must be refused");
    }
    assert!(choice_grid(0xff).is_none());
    // The last accepted count still sits fully inside the panel, with room to spare.
    let g = choice_grid(CHOICE_ROWS_MAX).unwrap();
    assert_eq!((g.y0, g.rows), (6, 6));
    assert_eq!(g.y0 + g.pitch_y * g.rows, 132);
}

/// The `예/아니오` confirm, pinned to its device measurement: the same widget lying
/// HORIZONTALLY. Two cells at x 32..83 and 92..143, y 71..84, and RIGHT is what moves the
/// cursor (DOWN did nothing on the device).
#[test]
fn the_confirm_cells_are_the_device_measurement() {
    let g = confirm_grid(2).unwrap();
    assert_eq!((g.x0, g.w, g.pitch_x, g.cols), (32, 52, 60, 2));
    assert_eq!((g.y0, g.h, g.rows), (71, 14, 1));
    // 예 (left) and 아니오 (right), corner to corner.
    assert_eq!(g.hit(32, 71), Some(0));
    assert_eq!(g.hit(83, 84), Some(0));
    assert_eq!(g.hit(92, 71), Some(1));
    assert_eq!(g.hit(143, 84), Some(1));
    // The 8-px gutter between them belongs to one of the two, never to nothing.
    for dx in 84..92 {
        assert!(g.hit(dx, 78).is_some(), "the gutter at dx {dx} is a miss");
    }
    // ⛔ Only two cells. A three-way confirm was never seen, and one sample cannot pin
    // where a third cell would sit.
    assert!(confirm_grid(1).is_none());
    assert!(confirm_grid(3).is_none());
    assert!(confirm_grid(0).is_none());
}



/// The smith's quantity spinner, drawn as the device showed it: ▲ apex dy 36 widening to
/// x 114..120 at dy 39, ▼ base x 114..120 at dy 56 narrowing to dy 59.
///
/// ⛔ The same ink fills the SELECTED cell of a `예/아니오` confirm, 52 px wide — if the
/// scan took long runs it would call that screen a spinner and swallow its taps.
#[test]
fn the_spinner_is_two_small_triangles_and_nothing_else() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let ink = 0xff00_0000 | SPIN_INK;
    let mut fb = Fb::new(496, 240);
    for (dy, a, b) in [(36, 117, 117), (37, 116, 118), (38, 115, 119), (39, 114, 120)] {
        for dx in a..=b {
            fb.set(ox + dx, oy + dy, ink);
        }
    }
    for (dy, a, b) in [(56, 114, 120), (57, 115, 119), (58, 116, 118), (59, 117, 117)] {
        for dx in a..=b {
            fb.set(ox + dx, oy + dy, ink);
        }
    }
    let (up, down) = spinner_arrows(&fb.px, 496, 240).expect("both triangles");
    assert!(up.contains(117, 37), "the ▲ ink is inside its target");
    assert!(down.contains(117, 58), "the ▼ ink is inside its target");
    assert!(!up.contains(117, 47) && !down.contains(117, 47), "the quantity digits are not a button");
    assert!(up.dy + up.h <= 44 && down.dy >= 51, "the targets must not reach the digit row");
    // A wide run of the same ink (the confirm dialog's selected cell) is NOT a spinner.
    let mut conf = Fb::new(496, 240);
    for dy in 71..85 {
        for dx in 32..84 {
            conf.set(ox + dx, oy + dy, ink);
        }
    }
    assert_eq!(spinner_arrows(&conf.px, 496, 240), None, "a confirm cell is not a spinner");
    // One triangle alone is not a spinner either, and neither is an empty frame.
    let mut one = Fb::new(496, 240);
    for dx in 114..=120 {
        one.set(ox + dx, oy + 39, ink);
    }
    assert_eq!(spinner_arrows(&one.px, 496, 240), None);
    assert_eq!(spinner_arrows(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(spinner_arrows(&[], 496, 240), None);
}

/// The game's own `OK : …` footer prompt is the button (player, 2026-08-23). What makes
/// it safe is that the ORANGE ink only appears where the prompt is: the device showed
/// 66 px of it in the smith's material window and 134 in its spinner, and ZERO in the
/// bag/quest/system screens — whose white frame lines would have fooled a white-only test.
#[test]
fn the_ok_prompt_strip_needs_the_prompt_ink() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let ink = 0xff00_0000 | OK_PROMPT_INK;
    let mut fb = Fb::new(496, 240);
    // The measured prompt: orange glyph pixels on dy 150..156, right of the white "OK :".
    for dy in 150..157 {
        for dx in 88..120 {
            fb.set(ox + dx, oy + dy, ink);
        }
    }
    let r = ok_prompt(&fb.px, 496, 240).expect("the prompt is drawn");
    assert_eq!((r.dx, r.w), (0, PANEL_W), "the whole strip is the button");
    assert!(r.contains(88, 152) && r.contains(20, 150), "anywhere along the strip");
    assert!(!r.contains(88, 130), "and not the rows above it");
    // White frame lines alone are NOT a prompt — that is the trap this test exists for.
    let mut white = Fb::new(496, 240);
    for dy in 140..160 {
        for dx in 0..PANEL_W {
            if dy == 143 || dx == 1 || dx == 173 {
                white.set(ox + dx, oy + dy, 0xffff_ffff);
            }
        }
    }
    assert_eq!(ok_prompt(&white.px, 496, 240), None);
    // A couple of stray orange pixels are not a prompt either.
    let mut few = Fb::new(496, 240);
    few.set(ox + 90, oy + 152, ink);
    few.set(ox + 91, oy + 152, ink);
    assert_eq!(ok_prompt(&few.px, 496, 240), None);
    assert_eq!(ok_prompt(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(ok_prompt(&[], 496, 240), None);
}

/// ★★★ 80차-D — the game's own `# : 초기화` footer prompt (player, 2026-09-05).
///
/// The pixels below are the DEVICE (S9, logical 492x239, vc248) drawing the 게임시작 slot
/// list footer, which is the case that makes this arm necessary: it holds TWO prompts,
/// `OK : 이어하기` on the left and `# : 초기화` on the right, so a detector that answers
/// with the whole strip answers a tap on 「초기화」 with OK — i.e. 「이어하기」.
///
///   cream `OK` sprite   dx  0..8, 10..17, 21     (0xf7e3b5 family)
///   white colon         dx 24..25
///   white `이어하기`     dx 31..71
///   ---- 54 blank columns ----
///   white `#`           dx 126..132, dy 155..161
///   white colon         dx 136..137
///   orange `초기화`      dx 143..173             (0xde9a19)
fn slot_footer(fb: &mut Fb) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let cream = 0xfff7_e3b5;
    let white = 0xffff_ffff;
    let orange = 0xff00_0000 | OK_PROMPT_INK;
    fn band(fb: &mut Fb, ox: i32, oy: i32, a: i32, b: i32, y0: i32, y1: i32, v: u32) {
        for dy in y0..=y1 {
            for dx in a..=b {
                fb.set(ox + dx, oy + dy, v);
            }
        }
    }
    // the panel's own bottom frame line, which sits one row ABOVE the band
    band(fb, ox, oy, 0, PANEL_W - 1, 149, 149, white);
    // `OK` sprite + colon + 이어하기. ★ 80차-F re-measured the sprite's own ink runs on
    // the device (dx 0..8 and 10..15, then EIGHT blank columns before the colon) — the
    // gap between the token and its colon is what `ok_token_prompt` anchors on.
    for (a, b) in [(0, 8), (10, 15)] {
        band(fb, ox, oy, a, b, 151, 161, cream);
    }
    band(fb, ox, oy, 24, 25, 154, 155, white);
    band(fb, ox, oy, 24, 25, 157, 158, white);
    for (a, b) in [(31, 35), (38, 38), (42, 46), (48, 50), (52, 58), (60, 61), (64, 68), (71, 71)] {
        band(fb, ox, oy, a, b, 151, 160, white);
    }
    // the `#` glyph, exactly as measured: two full bars and five stem rows
    for dy in 155..=161 {
        let full = dy == 157 || dy == 159;
        for i in 0..HASH_W {
            if full || matches!(i, 1 | 2 | 4 | 5) {
                fb.set(ox + 126 + i, oy + dy, white);
            }
        }
    }
    band(fb, ox, oy, 136, 137, 156, 157, white);
    band(fb, ox, oy, 136, 137, 159, 160, white);
    for (a, b) in [(143, 151), (154, 158), (161, 161), (164, 173)] {
        band(fb, ox, oy, a, b, 153, 162, orange);
    }
}

/// The ONLINE slot list's footer, measured on the same device in the same session:
/// `OK : 선택` and `CLR : 메인메뉴`. This is the screen `ok_token_prompt` exists to be safe
/// on — answering 「메인메뉴」 with OK would LOAD the slot (71차-L).
fn online_footer(fb: &mut Fb) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let cream = 0xfff7_e3b5;
    let white = 0xffff_ffff;
    fn band(fb: &mut Fb, ox: i32, oy: i32, a: i32, b: i32, y0: i32, y1: i32, v: u32) {
        for dy in y0..=y1 {
            for dx in a..=b {
                fb.set(ox + dx, oy + dy, v);
            }
        }
    }
    band(fb, ox, oy, 0, PANEL_W - 1, 149, 149, white);
    // `OK` : 선택
    for (a, b) in [(0, 8), (10, 15)] {
        band(fb, ox, oy, a, b, 151, 161, cream);
    }
    band(fb, ox, oy, 24, 25, 154, 155, white);
    band(fb, ox, oy, 24, 25, 157, 158, white);
    for (a, b) in [(31, 35), (37, 39), (42, 50)] {
        band(fb, ox, oy, a, b, 151, 160, white);
    }
    // `CLR` : 메인메뉴 — three letter runs, THIRTY-SEVEN px up to its colon.
    for (a, b) in [(90, 96), (100, 101), (111, 116)] {
        band(fb, ox, oy, a, b, 151, 161, cream);
    }
    band(fb, ox, oy, 125, 126, 154, 155, white);
    band(fb, ox, oy, 125, 126, 157, 158, white);
    for (a, b) in [(132, 138), (140, 140), (143, 147), (151, 151), (154, 160), (162, 162), (165, 173)] {
        band(fb, ox, oy, a, b, 151, 160, white);
    }
}

/// ★★★ 80차-F — 「이어하기」 is a target at last, and it is one WITHOUT a live character:
/// the token is identified, not assumed. Player: *"게임시작 초기화 및 이어하기 터치영역도
/// 너무 작아서 어디를 터치해야 버튼이 동작하는지 모르겠어"*.
#[test]
fn the_ok_token_prompt_claims_the_run_the_game_led_with_ok() {
    let mut fb = Fb::new(496, 240);
    slot_footer(&mut fb);
    let r = ok_token_prompt(&fb.px, 496, 240).expect("`OK : 이어하기` is drawn");
    assert_eq!((r.dx, r.dx + r.w - 1), (0, 71 + FOOTER_PAD), "the `OK : 이어하기` run only");
    assert_eq!((r.dy, r.dy + r.h - 1), (FOOTER_BOX_TOP, FOOTER_BOX_BOT - 1));
    assert!(r.contains(5, 155), "the `OK` sprite itself");
    assert!(r.contains(50, PANEL_H + 5), "and 「이어하기」 below the panel");
    // ⛔ The `#` run belongs to `hash_prompt`, and the two may not both claim a column.
    let h = hash_prompt(&fb.px, 496, 240).unwrap();
    assert!(!r.contains(128, 155) && !r.contains(160, 155), "the `#` run is not ours");
    for dx in 0..PANEL_W {
        assert!(!(r.contains(dx, 155) && h.contains(dx, 155)), "column {dx} is claimed twice");
    }
}

/// ⛔⛔ THE WHOLE POINT: on the online slot list the SECOND prompt is `CLR : 메인메뉴`, and
/// answering it with OK would load the slot — which per 71차-L has the `04 3d` ack overwrite
/// it. The prefix width is what tells them apart: `OK :` = 26, `CLR :` = 37.
#[test]
fn the_ok_token_prompt_refuses_the_clr_run() {
    let mut fb = Fb::new(496, 240);
    online_footer(&mut fb);
    let r = ok_token_prompt(&fb.px, 496, 240).expect("`OK : 선택` is drawn");
    assert_eq!((r.dx, r.dx + r.w - 1), (0, 50 + FOOTER_PAD), "only the `OK` run");
    assert!(r.contains(5, 155) && r.contains(45, 155));
    for dx in 87..PANEL_W {
        assert!(!r.contains(dx, 155), "column {dx} is part of `CLR : 메인메뉴`");
    }
    // …and the measurement the threshold rests on, asserted as a number.
    assert!(FOOTER_TOKEN_MAX_W >= 26 && FOOTER_TOKEN_MAX_W < 37, "the OK/CLR split moved");
}

/// Fail closed. Anything this cannot read is somebody else's problem — the caller falls
/// back to the level-gated strip detector, so a `None` here never removes a target.
#[test]
fn the_ok_token_prompt_fails_closed() {
    assert_eq!(ok_token_prompt(&Fb::new(496, 240).px, 496, 240), None, "an empty footer");
    assert_eq!(ok_token_prompt(&[], 496, 240), None);
    assert_eq!(ok_token_prompt(&Fb::new(496, 240).px, 0, 0), None);
    // A footer with the token but NO colon gap is not a prompt we can key.
    let mut solid = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for dx in 0..60 {
        for dy in 151..160 {
            solid.set(ox + dx, oy + dy, 0xffff_ffff);
        }
    }
    assert_eq!(ok_token_prompt(&solid.px, 496, 240), None, "one solid run has no colon");
    // TWO prompts that both look like `OK : …` -> ambiguous -> nothing. (The game never
    // draws that, but "which one is the OK?" has no answer, so the answer is no answer.)
    let mut twice = Fb::new(496, 240);
    let mut paint = |at: i32| {
        for (a, b) in [(0, 8), (10, 15), (31, 35), (37, 39), (42, 50)] {
            for dy in 151..160 {
                for dx in a..=b {
                    twice.set(ox + at + dx, oy + dy, 0xffff_ffff);
                }
            }
        }
        // the colon, as the game draws it: two short dashes (80차-P shape rule)
        for dy in [154, 155, 157, 158] {
            for dx in 24..=25 {
                twice.set(ox + at + dx, oy + dy, 0xffff_ffff);
            }
        }
    };
    paint(0);
    paint(90);
    assert_eq!(ok_token_prompt(&twice.px, 496, 240), None, "two candidates -> refuse");
}

/// Resolution independence, like every other detector here.
#[test]
fn the_ok_token_prompt_travels_with_the_centred_panel() {
    let mut a = Fb::new(496, 240);
    slot_footer(&mut a);
    let mut b = Fb::new(560, 228);
    slot_footer(&mut b);
    assert_eq!(ok_token_prompt(&a.px, 496, 240), ok_token_prompt(&b.px, 560, 228));
}

#[test]
fn the_hash_prompt_is_found_by_the_glyphs_own_bitmap() {
    let mut fb = Fb::new(496, 240);
    slot_footer(&mut fb);
    let h = hash_prompt(&fb.px, 496, 240).expect("the `#` prompt is drawn");
    // the run, padded by 3 and clamped to the panel: `#` at 126, 초기화 ends at 173
    assert_eq!((h.dx, h.dx + h.w - 1), (123, 175), "the `# : 초기화` run only");
    // ★★ 80차-F — the whole footer band, and it reaches BELOW the panel: the `#` glyph
    // itself is measured at dy 155..161 and the panel ends at 160, so the last two rows
    // of the very thing being tapped used to be dropped by `panel_point`.
    assert_eq!((h.dy, h.dy + h.h - 1), (FOOTER_BOX_TOP, FOOTER_BOX_BOT - 1));
    assert!(h.contains(128, PANEL_H + 5), "the rows below the panel are still the prompt");
    assert!(h.contains(128, 155), "the `#` itself");
    assert!(h.contains(160, 155), "and the 「초기화」 it labels");
    // ⛔ THE POINT OF THE ARM: the left prompt is not ours.
    assert!(!h.contains(60, 155), "not 「이어하기」");
    assert!(!h.contains(5, 155), "not the `OK` sprite");
    assert!(!h.contains(128, 140), "and not the rows above the footer");
}

/// ⛔ `ok_prompt` and `hash_prompt` must not overlap — the strip detector clips itself.
///
/// This is the invariant that makes the two arms safe in either order, and it is the one
/// the 게임시작 slot list needs: the previous character still RESOLVES on that screen
/// (79차-F), so `stats.level > 0` does not keep `ok_prompt` off it.
#[test]
fn the_two_footer_prompts_own_disjoint_columns() {
    let mut fb = Fb::new(496, 240);
    slot_footer(&mut fb);
    let h = hash_prompt(&fb.px, 496, 240).unwrap();
    let o = ok_prompt(&fb.px, 496, 240).expect("the orange ink is there, so the strip is a prompt");
    assert_eq!((o.dx, o.w), (0, h.dx), "the strip stops where the `#` run starts");
    for dx in 0..PANEL_W {
        assert!(!(o.contains(dx, 155) && h.contains(dx, 155)), "column {dx} is claimed twice");
    }
    assert!(o.contains(60, 155) && !o.contains(128, 155));
    // ★ 80차-F — both halves span the SAME rows, and those rows run past the panel.
    assert_eq!((o.dy, o.dy + o.h - 1), (h.dy, h.dy + h.h - 1));
    assert!(o.contains(60, PANEL_H + 5), "the `OK` half reaches below the panel too");
}

/// Fail closed. None of these is a `# : …` prompt.
#[test]
fn the_hash_prompt_refuses_everything_that_is_not_the_glyph() {
    let white = 0xffff_ffff;
    let (ox, oy) = panel_origin(496, 240).unwrap();
    // A `#` with no colon after it is not a prompt (nor is a lone one anywhere else).
    let mut lone = Fb::new(496, 240);
    for dy in 155..=161 {
        let full = dy == 157 || dy == 159;
        for i in 0..HASH_W {
            if full || matches!(i, 1 | 2 | 4 | 5) {
                lone.set(ox + 126 + i, oy + dy, white);
            }
        }
    }
    assert_eq!(hash_prompt(&lone.px, 496, 240), None, "no colon -> no prompt");
    // ★ A solid 7-wide run is not a `#`: it has the bars and NONE of the stems. The
    // measured 이어하기 has a run exactly 7 wide (dx 52..58), so this is not academic.
    let mut solid = Fb::new(496, 240);
    for dy in 151..160 {
        for dx in 52..=58 {
            solid.set(ox + dx, oy + dy, white);
        }
        for dx in 62..=63 {
            solid.set(ox + dx, oy + dy, white);
        }
    }
    assert_eq!(hash_prompt(&solid.px, 496, 240), None, "a filled block is not a glyph");
    // ⛔ The band starts BELOW the panel's own content. `PLAY TIME` lives at dy 141..147
    // and a `#`-shaped thing up there is not a footer prompt.
    let mut high = Fb::new(496, 240);
    for dy in 138..=144 {
        let full = dy == 140 || dy == 142;
        for i in 0..HASH_W {
            if full || matches!(i, 1 | 2 | 4 | 5) {
                high.set(ox + 126 + i, oy + dy, white);
            }
        }
        for dx in 136..=137 {
            high.set(ox + dx, oy + dy, white);
        }
    }
    assert_eq!(hash_prompt(&high.px, 496, 240), None, "above the band -> not ours");
    assert_eq!(hash_prompt(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(hash_prompt(&[], 496, 240), None);
    assert_eq!(hash_prompt(&Fb::new(496, 240).px, 0, 0), None);
}

/// The same footer at the other measured logical size — the panel is centred and fixed
/// 176x160, so every number above is resolution-independent.
#[test]
fn the_hash_prompt_travels_with_the_centred_panel() {
    let mut a = Fb::new(496, 240);
    let mut b = Fb::new(560, 228);
    slot_footer(&mut a);
    slot_footer(&mut b);
    assert_eq!(hash_prompt(&a.px, 496, 240), hash_prompt(&b.px, 560, 228));
}

/// A `예 / 아니요` confirm the game's own module drew (제련's "착용중인 장비입니다"). Its
/// selection is in NO byte of the character record — RIGHT moves it with nothing changing —
/// so the pixels are the whole mechanism.
///
/// ⛔ The button is a filled box with TEXT on it, so a "longest run" test fails: the device
/// showed dy 100's runs as 29..43 / 45..47 / 49 / 51 / 53..68, longest 16 px of a 40 px box.
/// The span plus a fill ratio is what survives the glyphs.
#[test]
fn a_module_drawn_confirm_is_read_from_its_two_boxes() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let sel = 0xff00_0000 | CONFIRM2_SEL_INK;
    let bg = 0xff00_0000 | CONFIRM2_BG_INK;
    let mut fb = Fb::new(496, 240);
    // The measured pair: 예 at x 29..68 selected, 아니요 at x 105..144 on the dialog bg.
    for dy in 94..107 {
        for dx in 29..69 {
            fb.set(ox + dx, oy + dy, sel);
        }
        for dx in 105..145 {
            fb.set(ox + dx, oy + dy, bg);
        }
    }
    // …with the glyphs breaking the selected box the way the device showed.
    for dx in [44, 48, 50, 52] {
        for dy in 96..106 {
            fb.set(ox + dx, oy + dy, 0xffff_ffff);
        }
    }
    let (s, o) = confirm_pair(&fb.px, 496, 240).expect("both buttons");
    assert_eq!((s.dx, s.w), (29, 40), "the selected one is where the ink is");
    assert_eq!((o.dx, o.w), (105, 40), "the other one is its mirror");
    assert!(s.contains(40, 100) && o.contains(120, 100));
    assert!(!s.contains(90, 100), "the gap between them is neither");

    // Mirrored: 아니요 selected instead.
    let mut fb2 = Fb::new(496, 240);
    for dy in 94..107 {
        for dx in 105..145 {
            fb2.set(ox + dx, oy + dy, sel);
        }
        for dx in 29..69 {
            fb2.set(ox + dx, oy + dy, bg);
        }
    }
    let (s2, o2) = confirm_pair(&fb2.px, 496, 240).unwrap();
    assert_eq!(s2.dx, 105);
    assert_eq!(o2.dx, 29);

    // ⛔ A lone box with nothing on its mirror is NOT this dialog — that is what keeps a
    // selected list row (which is also a filled box) from being read as a confirm.
    let mut lone = Fb::new(496, 240);
    for dy in 94..107 {
        for dx in 29..69 {
            lone.set(ox + dx, oy + dy, sel);
        }
    }
    assert_eq!(confirm_pair(&lone.px, 496, 240), None);
    assert_eq!(confirm_pair(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(confirm_pair(&[], 496, 240), None);
}

/// A message box's centred `OK` (제련 결과창 · 상점 대화창). What keeps it from firing on the
/// `예/아니요` dialog — whose body is the SAME grey and also holds white glyphs — is the text
/// WIDTH: `OK` is 15 px and centred, that dialog's labels span 115 px across two buttons.
#[test]
fn the_message_box_ok_is_narrow_and_centred() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let band = 0xff00_0000 | OK_BTN_BAND_INK;
    let white = 0xff00_0000 | OK_BTN_TEXT_INK;
    let mut fb = Fb::new(496, 240);
    for dy in 101..115 {
        for dx in 14..162 {
            fb.set(ox + dx, oy + dy, band);
        }
    }
    for dy in 105..111 {
        for dx in 81..96 {
            fb.set(ox + dx, oy + dy, white);
        }
    }
    let r = ok_button(&fb.px, 496, 240).expect("the button is drawn");
    assert!(r.contains(88, 107) && r.contains(20, 102), "anywhere along the band");
    assert!(!r.contains(88, 90), "and not the message above it");

    // The same grey band with text spread across it (two buttons) is NOT an OK button.
    let mut wide = Fb::new(496, 240);
    for dy in 94..107 {
        for dx in 20..155 {
            wide.set(ox + dx, oy + dy, band);
        }
    }
    for dy in 98..104 {
        for dx in [30, 31, 40, 41, 120, 121, 140, 141] {
            wide.set(ox + dx, oy + dy, white);
        }
    }
    assert_eq!(ok_button(&wide.px, 496, 240), None, "spread-out labels are not an OK");
    // A band with no text at all is not a button either.
    let mut bare = Fb::new(496, 240);
    for dy in 101..115 {
        for dx in 14..162 {
            bare.set(ox + dx, oy + dy, band);
        }
    }
    assert_eq!(ok_button(&bare.px, 496, 240), None);
    assert_eq!(ok_button(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(ok_button(&[], 496, 240), None);
}

/// ★★ The ITEM GRID, derived from the game's own slot borders. This is the test for the
/// 2026-08-23 report ("일반 조합창에서 … 엉뚱한 아이템이 포커스돼 … 가운데 창도 버그처럼"):
/// the bag is 8x2 at pitch 19 and 일반조합 is **5x1 at pitch 29** with the SAME 17-px cells,
/// so a hardcoded bag shape wrote an out-of-range cursor there.
///
/// Both layouts below are the device's: slot top borders are 15-px runs of the slot ink,
/// and the cell the cursor sits on has NO border run (the yellow box covers it), which is
/// why the pitch is taken as the smallest gap and the count from the span.
#[test]
fn the_item_grid_is_derived_from_the_slot_borders() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let slot = 0xff00_0000 | SLOT_INK;
    let yel = INK_YEL;
    // A grid drawer: borders at every cell except `skip`, and the yellow box on `skip`.
    let build = |x0: i32, y0: i32, pitch: i32, cols: i32, rows: i32, skip: i32| {
        let mut fb = Fb::new(496, 240);
        for r in 0..rows {
            for c in 0..cols {
                let (x, y) = (x0 + pitch * c, y0 + 19 * r);
                if r * cols + c == skip {
                    // the cursor's yellow border, as the game draws it
                    for i in 1..16 {
                        fb.set(ox + x + i, oy + y, yel);
                        fb.set(ox + x + i, oy + y + 16, yel);
                    }
                    for j in 1..16 {
                        fb.set(ox + x, oy + y + j, yel);
                        fb.set(ox + x + 16, oy + y + j, yel);
                    }
                } else {
                    for i in 0..15 {
                        fb.set(ox + x + 1 + i, oy + y, slot);
                    }
                }
            }
        }
        fb
    };
    // 가방: 8 x 2 at pitch 19, cursor on cell 0 (the device's sell window).
    let fb = build(13, 18, 19, 8, 2, 0);
    let (g, idx) = item_grid(&fb.px, 496, 240).expect("the bag grid");
    assert_eq!((g.x0, g.y0, g.pitch_x, g.cols, g.rows), (13, 18, 19, 8, 2));
    assert_eq!(idx, 0, "the cursor's own cell, computed from pixels alone");
    assert_eq!(g.hit(13 + 8, 18 + 8), Some(0));
    assert_eq!(g.hit(13 + 19 * 7 + 8, 18 + 19 + 8), Some(15), "the last cell");

    // 일반조합: 5 x 1 at pitch 29, cursor on cell 3 — the reported bug's exact state.
    let fb = build(21, 18, 29, 5, 1, 3);
    let (g, idx) = item_grid(&fb.px, 496, 240).expect("the combine grid");
    assert_eq!((g.x0, g.y0, g.pitch_x, g.cols, g.rows), (21, 18, 29, 5, 1));
    assert_eq!(idx, 3);
    // The 5th icon is cell 4 — NOT the 14 the bag's pitch would have written.
    assert_eq!(g.hit(21 + 29 * 4 + 8, 18 + 8), Some(4));
    assert!(g.holds(4) && !g.holds(5), "five cells, no more");

    // A screen with no slot borders at all is not an item grid.
    assert_eq!(item_grid(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(item_grid(&[], 496, 240), None);
    // …and neither is one with borders but no cursor box (the game draws none while its
    // cursor is out of range — which is the state the bug left the window in).
    let mut noboxes = Fb::new(496, 240);
    for c in 0..5 {
        for i in 0..15 {
            noboxes.set(ox + 22 + 29 * c + i, oy + 18, slot);
        }
    }
    assert_eq!(item_grid(&noboxes.px, 496, 240), None);
}

/// ★★★ 80차-E — **the count label eats the cursor box's BOTTOM bar, and how much it eats
/// depends on how many digits the count has.** 66차-B measured one screen (a one-glyph
/// label, 10 px of bar left) and wrote the floor at 8; the network warehouse's cursor sat
/// on a stack of 33 and left **5**, so `cell_box` answered `None` — and per 79차-F that
/// `None` means the opposite thing ("this is a row list, not a grid"), which is how a
/// whole screen's item taps went missing.
///
/// The two shapes are the device's (S9, vc250, same cursor cell 3 at dx 70):
///
/// ```text
///   dy 23        dx 71..85   top bar, 15 px          (both screens)
///   dy 24..38    dx 70, 86   sides                   (both screens)
///   dy 39        dx 71..80   bottom, 10 px   싱글가방   count `E`
///   dy 39        dx 71..75   bottom,  5 px   네트워크가방 count `33`
/// ```
#[test]
fn the_count_label_may_eat_most_of_the_boxs_bottom_bar() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let yel = INK_YEL;
    let box_at = |bottom_len: i32| {
        let mut fb = Fb::new(496, 240);
        for i in 0..15 {
            fb.set(ox + 71 + i, oy + 23, yel);
        }
        for j in 24..39 {
            fb.set(ox + 70, oy + j, yel);
            fb.set(ox + 86, oy + j, yel);
        }
        for i in 0..bottom_len {
            fb.set(ox + 71 + i, oy + 39, yel);
        }
        fb
    };
    // The 싱글가방 shape 66차-B measured, and the 네트워크가방 shape it did not.
    for len in [10, 5, CELL_EDGE_ANY] {
        assert_eq!(
            cell_box(&box_at(len).px, 496, 240),
            Some((70, 23)),
            "a bottom bar of {len} px is still the cursor box"
        );
    }
    // ⛔ …and the box still needs a FULL top bar: a truncated run must never be taken
    // for one, which is what keeps a yellow item name out of this.
    let mut short_top = Fb::new(496, 240);
    for i in 0..5 {
        short_top.set(ox + 71 + i, oy + 23, yel);
        short_top.set(ox + 71 + i, oy + 39, yel);
    }
    for j in 24..39 {
        short_top.set(ox + 70, oy + j, yel);
        short_top.set(ox + 76, oy + j, yel);
    }
    assert_eq!(cell_box(&short_top.px, 496, 240), None, "a 5-px top bar is not a 17-px box");
    // No bottom bar at all is not a box either — the sides alone could be item art.
    let mut no_bottom = Fb::new(496, 240);
    for i in 0..15 {
        no_bottom.set(ox + 71 + i, oy + 23, yel);
    }
    for j in 24..39 {
        no_bottom.set(ox + 70, oy + j, yel);
        no_bottom.set(ox + 86, oy + j, yel);
    }
    assert_eq!(cell_box(&no_bottom.px, 496, 240), None);
}

/// The same defect from the caller's side: the grid must still be derivable when the
/// cursor sits on a two-digit stack. ⛔ This is the one that matters — `item_grid` starts
/// with `cell_box`, so a refused box takes the whole grid with it.
#[test]
fn the_grid_survives_a_cursor_on_a_stacked_item() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let slot = 0xff00_0000 | SLOT_INK;
    let yel = INK_YEL;
    let mut fb = Fb::new(496, 240);
    // 8x2 bag frames at the device's origin, cursor on cell 3 (no frame there).
    for r in 0..2 {
        for c in 0..8 {
            let (x, y) = (13 + 19 * c, 23 + 19 * r);
            if r * 8 + c == 3 {
                for i in 0..15 {
                    fb.set(ox + x + 1 + i, oy + y, yel);
                }
                for j in 1..16 {
                    fb.set(ox + x, oy + y + j, yel);
                    fb.set(ox + x + 16, oy + y + j, yel);
                }
                // the bottom bar the `33` label truncated to 5 px
                for i in 0..5 {
                    fb.set(ox + x + 1 + i, oy + y + 16, yel);
                }
            } else {
                for i in 0..15 {
                    fb.set(ox + x + 1 + i, oy + y, slot);
                }
            }
        }
    }
    let (g, idx) = item_grid(&fb.px, 496, 240).expect("the network bag grid");
    assert_eq!((g.x0, g.y0, g.pitch_x, g.cols, g.rows), (13, 23, 19, 8, 2));
    assert_eq!(idx, 3, "the cursor's own cell, from pixels alone");
    assert_eq!(g.hit(13 + 19 * 3 + 8, 23 + 8), Some(3));
}

/// ⛔⛔ A row whose empty frames are every OTHER cell must not be read as a grid of half
/// the columns. The runs are the game's EMPTY-slot frames, so a partly-filled bag can
/// leave gaps that are all a MULTIPLE of the true pitch — and "the rest are multiples of
/// the smallest" is then vacuously true. The worst case survives the worker's cross-check
/// too: with the cursor on cell 0 the wrong grid also reports 0, so `drawn == live` and a
/// tap on physical cell 2 focuses cell 1 instead.
///
/// `SLOT_PITCH_MAX` is what refuses it: no measured layout puts a whole cell's worth of
/// gutter between two cells (bag 19, 일반조합 29, cells 17 in both).
#[test]
fn a_pitch_wider_than_a_whole_cell_is_refused_rather_than_guessed() {
    assert!(INVEN.pitch_x <= SLOT_PITCH_MAX && 29 <= SLOT_PITCH_MAX, "both measured pitches must still pass");
    assert!(INVEN.pitch_x * 2 > SLOT_PITCH_MAX, "…and twice the bag's pitch must not");

    let (ox, oy) = panel_origin(496, 240).unwrap();
    let slot = 0xff00_0000 | SLOT_INK;
    // The bag as the device draws it, but only cells 2/4/6 are empty and the cursor sits
    // on cell 0 — every surviving gap is 2 x 19.
    let mut fb = Fb::new(496, 240);
    for c in [2, 4, 6] {
        for i in 0..15 {
            fb.set(ox + 13 + 19 * c + 1 + i, oy + 18, slot);
        }
    }
    for i in 1..16 {
        fb.set(ox + 13 + i, oy + 18, INK_YEL);
        fb.set(ox + 13 + i, oy + 18 + 16, INK_YEL);
    }
    for j in 1..16 {
        fb.set(ox + 13, oy + 18 + j, INK_YEL);
        fb.set(ox + 13 + 16, oy + 18 + j, INK_YEL);
    }
    match item_grid(&fb.px, 496, 240) {
        None => {}
        Some((g, _)) => assert_eq!(
            g.pitch_x, 19,
            "a 38-px pitch means two cells were folded into one column ({g:?})"
        ),
    }
}

/// ⛔ The bag page is an INDEX into the cursor array (`CHAR+0x9c72 + page`), so a page the
/// game does not have must never be written. The array runs into 장비's own three cursor
/// bytes from bag 6 on — the game's own layout, and we write only what it writes.
#[test]
fn the_bag_page_count_is_the_games_own() {
    assert_eq!(PAGES, 8);
}

// --------------------------------------------------------------------------
// the badge scan — stage 1, the one that needs no guest anchor at all
// --------------------------------------------------------------------------

/// INVENTORY as measured: tabs at dy 2..12 and page badges at dy 131..141.
#[test]
fn the_inventory_badge_set_is_found_and_classified() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    fb.pages(131);
    let hits = fb.found();
    assert_eq!(hits.len(), 4, "{hits:?}");
    // Left column carries 1 and 7, right column 3 and 9.
    assert_eq!(fb.at(BADGE_L_X + 5, 7), Some(Badge::Tab1));
    assert_eq!(fb.at(BADGE_R_X + 5, 7), Some(Badge::Tab3));
    assert_eq!(fb.at(BADGE_L_X + 5, 136), Some(Badge::Page7));
    assert_eq!(fb.at(BADGE_R_X + 5, 136), Some(Badge::Page9));
}

/// SKILL/QUEST put the page badges 14 px lower. ★ This is why the dy is SCANNED and
/// never assumed — the one thing 63차-B flagged as screen-dependent.
#[test]
fn the_page_badges_are_found_wherever_the_screen_puts_them() {
    for dy0 in [131, 145] {
        let mut fb = Fb::new(496, 240);
        fb.tabs();
        fb.pages(dy0);
        assert_eq!(fb.at(BADGE_L_X + 5, dy0 + 5), Some(Badge::Page7), "page badge at dy {dy0}");
        assert_eq!(fb.at(BADGE_R_X + 5, dy0 + 5), Some(Badge::Page9), "page badge at dy {dy0}");
    }
}

/// EQUIP and STATUS have no pages: two badges, and nothing in the lower half.
#[test]
fn a_screen_with_no_pages_gets_only_the_two_tab_targets() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    let hits = fb.found();
    assert_eq!(hits.len(), 2, "{hits:?}");
    assert!(hits.iter().all(|h| matches!(h.badge, Badge::Tab1 | Badge::Tab3)));
    assert_eq!(fb.at(BADGE_L_X + 5, 136), None);
}

/// The six-row MENU list draws no badges at all — measured. So it gets no targets, and
/// the row taps below have the panel to themselves.
#[test]
fn the_menu_list_gets_no_badge_targets() {
    let fb = Fb::new(496, 240);
    assert!(fb.found().is_empty());
}

/// Resolution independence, tested the way the probe tested it: the same panel-relative
/// ink at another logical size must produce the same panel-relative targets.
#[test]
fn the_scan_is_resolution_independent() {
    let mut a = Fb::new(496, 240);
    a.tabs();
    a.pages(131);
    let mut b = Fb::new(560, 228);
    b.tabs();
    b.pages(131);
    assert_eq!(a.found(), b.found());
}

/// The target is grown past the ink (thumbs), but it may not leave the panel and may
/// not reach the neighbouring column.
#[test]
fn the_badge_target_is_grown_but_stays_inside_the_panel() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    fb.pages(131);
    for h in fb.found() {
        assert!(h.dx >= 0 && h.dx + h.w <= PANEL_W, "{h:?} leaves the panel horizontally");
        assert!(h.dy >= 0 && h.dy + h.h <= PANEL_H, "{h:?} leaves the panel vertically");
        assert!(h.w >= BADGE_W && h.h >= 11, "{h:?} is smaller than the ink it covers");
    }
    // Grown by PAD on the ink's INNER side and by PAD + the chevron width on the OUTER
    // one (80차-F) — the left column's `<<` is to its left, the right column's `>>` to
    // its right.
    let l = fb.found().into_iter().find(|h| h.badge == Badge::Tab1).unwrap();
    assert_eq!((l.dx, l.w), (BADGE_L_X - BADGE_PAD - BADGE_ARROW_W, BADGE_W + 2 * BADGE_PAD + BADGE_ARROW_W));
    let r = fb.found().into_iter().find(|h| h.badge == Badge::Tab3).unwrap();
    assert_eq!(r.dx, BADGE_R_X - BADGE_PAD);
    assert_eq!(r.dx + r.w - 1, (BADGE_R_X + BADGE_W - 1 + BADGE_PAD + BADGE_ARROW_W).min(PANEL_W - 1));
    // The two columns are 136 px apart, so no growth can make them touch.
    assert!(l.dx + l.w < r.dx, "the two badge targets overlap: {l:?} {r:?}");
}

/// ★★★ 80차-F — the game draws `<<` and `>>` OUTSIDE the plate, and the player aims at
/// them (2026-09-05: *"1,3,7,9 문구박스에서 >> << 이 버튼까지 포함해서 터치영역을 차지하도록"*).
/// Measured on the S9 at logical 492x239, header AND footer alike:
/// `<<` dx 5..11 · «1»/«7» dx 15..25 · «3»/«9» dx 151..161 · `>>` dx 163..171.
#[test]
fn the_badge_target_reaches_the_chevrons_the_game_drew() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    fb.pages(131);
    // The measured chevron columns, on both bands.
    for (dx, dy, want) in [
        (5, 7, Badge::Tab1),
        (11, 7, Badge::Tab1),
        (163, 7, Badge::Tab3),
        (171, 7, Badge::Tab3),
        (5, 136, Badge::Page7),
        (171, 136, Badge::Page9),
    ] {
        assert_eq!(fb.at(dx, dy), Some(want), "the chevron at ({dx},{dy}) is not on the target");
    }
    // …and the gap between a plate and its chevrons, which used to be dead.
    assert_eq!(fb.at(13, 7), Some(Badge::Tab1));
    assert_eq!(fb.at(162, 7), Some(Badge::Tab3));
}

/// ⛔ THE ONE THING THE VERTICAL GROWTH MAY NOT TOUCH: the item grid's first row.
/// Badges are tested BEFORE the grid in `app::menu_tap_decide`, so an overlap would make
/// the whole top row of the bag untappable — measured grid origins are dy 22 and dy 23.
#[test]
fn the_tab_badge_target_stops_short_of_the_item_grid() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    let l = fb.found().into_iter().find(|h| h.badge == Badge::Tab1).unwrap();
    assert_eq!((l.dy, l.dy + l.h - 1), (0, 12 + BADGE_PAD_Y), "the tab band is dy 2..12");
    assert!(l.dy + l.h <= 22, "{l:?} reaches the item grid's first row (dy 22)");
    assert_eq!(fb.at(BADGE_L_X + 5, 22), None, "dy 22 belongs to the grid");
}

/// The badge targets must not overlap the row strip of the MENU list, because that
/// screen has no badges and the strip covers the whole list. (Belt and braces: the
/// scan already answers nothing there.)
#[test]
fn a_page_badge_target_does_not_swallow_a_whole_menu_row() {
    let mut fb = Fb::new(496, 240);
    fb.pages(131);
    let h = fb.found().into_iter().find(|h| h.badge == Badge::Page7).unwrap();
    // It covers dy 127..145 in the LEFT badge column only — the row strip is 156 px
    // wide, so a row is still reachable beside it.
    assert!(h.w < MENU.w, "a badge target as wide as the row strip would hide a row");
}

// --------------------------------------------------------------------------
// fail-closed
// --------------------------------------------------------------------------

/// No frame, a stale frame from another size, or a frame from before the resize: no
/// targets. ★ This is the one that keeps a menu tap from firing a key at random when
/// `last_frame` and `logical()` disagree for a frame after a rotation.
#[test]
fn a_frame_of_the_wrong_length_yields_no_targets() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    assert!(badges(&fb.px, 560, 228).is_empty());
    assert!(badges(&fb.px[..100], 496, 240).is_empty());
    assert!(badges(&[], 496, 240).is_empty());
    assert!(badges(&fb.px, 0, 0).is_empty());
    assert!(badges(&fb.px, -496, 240).is_empty());
}

/// A couple of stray ink pixels are not a badge (the game's own dithering, an item
/// icon that happens to use the colour).
#[test]
fn a_few_stray_ink_pixels_are_not_a_badge() {
    let mut fb = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for dy in 2..13 {
        fb.set(ox + BADGE_L_X + 1, oy + dy, INK); // one column of 11 = 11 px < MIN_INK
    }
    assert!(fb.found().is_empty());
}

/// A tall block of ink in the badge column is not a badge either — the glyph is 11 px,
/// and refusing everything else is what keeps an item sprite from becoming a button.
#[test]
fn a_band_that_is_too_tall_is_not_a_badge() {
    let mut fb = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for dy in 2..2 + BADGE_BAND_MAX_H + 1 {
        for i in 0..BADGE_W {
            fb.set(ox + BADGE_L_X + i, oy + dy, INK);
        }
    }
    assert!(fb.found().is_empty());
}

/// Ink in the middle of the panel is neither a tab nor a page. We do not know what it
/// would be, so it gets no target at all.
#[test]
fn ink_between_the_two_known_bands_is_ignored() {
    let mut fb = Fb::new(496, 240);
    fb.badge(BADGE_L_X, 60);
    assert!(fb.found().is_empty());
    assert_eq!(fb.at(BADGE_L_X + 5, 65), None);
}

/// The header title ink differs from the badge ink by 8 in one channel. If the scan
/// ever matched it, every screen would sprout a fake target — and `«1»` sits in the
/// same 11-px band as the title.
#[test]
fn the_header_title_ink_is_not_badge_ink() {
    assert_ne!(TITLE_INK & 0x00ff_ffff, BADGE_INK);
    let mut fb = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for dy in 2..13 {
        for i in 0..BADGE_W {
            fb.set(ox + BADGE_L_X + i, oy + dy, TITLE_INK);
        }
    }
    assert!(fb.found().is_empty());
}

/// Alpha is ignored (the frame is 0xAARRGGBB and the guest's is always opaque, but a
/// masked compare is what makes that assumption harmless).
#[test]
fn the_scan_compares_rgb_and_ignores_alpha() {
    let mut fb = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for dy in 2..13 {
        for i in 0..BADGE_W {
            fb.set(ox + BADGE_L_X + i, oy + dy, BADGE_INK); // alpha 0
        }
    }
    assert_eq!(fb.at(BADGE_L_X + 5, 7), Some(Badge::Tab1));
}

/// `badge_at` on an empty scan, and a point in the panel but on no badge.
#[test]
fn badge_at_answers_none_where_there_is_no_badge() {
    let mut fb = Fb::new(496, 240);
    fb.tabs();
    assert_eq!(fb.at(88, 80), None);
    assert_eq!(badge_at(&[], 20, 7), None);
}


// --------------------------------------------------------------------------
// the ACTION POPUP (드랍박스) — 79차-C
//
// The numbers below are the probe frames, not a re-derivation: `wie_probe` on the
// shipped LGT asset at 496x240, driven to the field and then CLR -> menu cursor 1
// -> OK -> inventory cell 0 -> OK, dumping frames 8400/8800 (no popup) and
// 9000/9200/9350 (popup, `CHAR+0x9c85` = 2 rows, `CHAR+0x9c86` = row 0 selected).
//
//   frame diff bbox        x 192..265   y 62..103      -> 74 wide, 42 = 17*2+8 tall
//   exact-73 black runs    (192, 62) and (192, 102)    -> and NONE in the other frames
//   row fill 0x7b5952      x 198..259                  -> x+6, 62 wide
//   selected inset 0x4a3121 over row 0
// --------------------------------------------------------------------------

/// Paint the popup exactly as the painter does: fill the box, black `drawRect` (which
/// covers `POPUP_RULE_W`, not `POPUP_W` — that is the whole detector), then the rows.
fn paint_popup(fb: &mut Fb, bx: i32, by: i32, rows: i32) {
    let h = POPUP_ROW_H * rows + POPUP_CHROME_H;
    // ⛔ `Fb::new` fills with opaque BLACK, which IS the rule ink — on that background
    // every scanline is one enormous black run and nothing can be found. The real frame
    // never looks like that (the popup sits on the menu panel, whose surround measured
    // as the `9c928c`/`948a7b` checker), so paint the surround first. This is not
    // test-rigging: it is the measured background, and `an_empty_frame_has_no_popup`
    // keeps the all-black case honest by asserting the detector finds nothing there.
    for y in (by - 4).max(0)..(by + h + 4).min(fb.lh) {
        for x in (bx - 4).max(0)..(bx + POPUP_W + 4).min(fb.lw) {
            fb.set(x, y, 0xff9c_928c);
        }
    }
    for y in by..by + h {
        for x in bx..bx + POPUP_W {
            fb.set(x, y, 0xffbd_b2ad);
        }
    }
    // The black rect: top and bottom edges are POPUP_RULE_W long, and the left/right
    // columns run between them. Measured: x=192 black y 62..102, x=265 stays body.
    for x in bx..bx + POPUP_RULE_W {
        fb.set(x, by, 0xff00_0000);
        fb.set(x, by + h - 2, 0xff00_0000);
    }
    for y in by..by + h - 1 {
        fb.set(bx, y, 0xff00_0000);
        fb.set(bx + POPUP_RULE_W - 1, y, 0xff00_0000);
    }
    for i in 0..rows {
        for y in by + POPUP_ROW_Y0 + POPUP_ROW_H * i..by + POPUP_ROW_Y0 + POPUP_ROW_H * (i + 1) {
            for x in bx + POPUP_ROW_X0..bx + POPUP_ROW_X0 + POPUP_ROW_W {
                fb.set(x, y, 0xff7b_5952);
            }
        }
    }
}

/// The constants, as literals, for the same reason the rest of this file pins its
/// own: a mistyped re-measurement would otherwise keep every test below green while
/// aiming taps at the wrong row.
#[test]
fn the_popup_constants_are_the_measured_ones() {
    assert_eq!(POPUP_W, 74);
    assert_eq!(POPUP_RULE_W, 73);
    assert_eq!(POPUP_ROW_H, 17);
    assert_eq!(POPUP_CHROME_H, 8);
    assert_eq!(POPUP_ROW_Y0, 4);
    assert_eq!(POPUP_ROW_X0, 6);
    assert_eq!(POPUP_ROW_W, 62);
    assert_eq!(POPUP_MAX_ROWS, 8);
    // 74 x (17n + 8): the probe's 2-row popup was 74 x 42.
    assert_eq!(POPUP_ROW_H * 2 + POPUP_CHROME_H, 42);
}

#[test]
fn the_probe_frame_is_found_where_the_probe_found_it() {
    let mut fb = Fb::new(496, 240);
    paint_popup(&mut fb, 192, 62, 2);
    assert_eq!(popup_box(&fb.px, fb.lw, fb.lh, 2), Some((192, 62)));
}

/// The row count is part of the SEARCH, not just of the answer: the two rules have to
/// be `17n + 6` apart. Asking with the wrong count must find nothing rather than
/// return a box whose rows would then be mapped wrong.
#[test]
fn a_box_is_only_found_for_the_row_count_it_actually_has() {
    let mut fb = Fb::new(496, 240);
    paint_popup(&mut fb, 192, 62, 2);
    for rows in 1..=POPUP_MAX_ROWS {
        let want = (rows == 2).then_some((192, 62));
        assert_eq!(popup_box(&fb.px, fb.lw, fb.lh, rows), want, "rows={rows}");
    }
}

/// ...and the probe's own negative: no popup on screen, no box. (Frames 8400 and 8800
/// had zero exact-73 black runs.)
#[test]
fn an_empty_frame_has_no_popup() {
    let fb = Fb::new(496, 240);
    for rows in 1..=POPUP_MAX_ROWS {
        assert_eq!(popup_box(&fb.px, fb.lw, fb.lh, rows), None);
    }
}

/// A black run of the WRONG length is not a rule. This is what keeps the game's own
/// black chrome — which is everywhere — out of the detector.
#[test]
fn black_of_another_width_is_not_a_popup() {
    for w in [POPUP_RULE_W - 1, POPUP_RULE_W + 1, POPUP_W] {
        let mut fb = Fb::new(496, 240);
        paint_popup(&mut fb, 192, 62, 2);
        // Repaint the two rules at the wrong width, over the correct popup.
        for y in [62, 102] {
            for x in 192..192 + POPUP_W + 2 {
                fb.set(x, y, 0xff9c_928c);
            }
            for x in 192..192 + w {
                fb.set(x, y, 0xff00_0000);
            }
        }
        assert_eq!(popup_box(&fb.px, fb.lw, fb.lh, 2), None, "w={w}");
    }
}

/// ...and a correctly-sized PAIR of rules that is not a box does not pass either: the
/// body colour on the right column and the bottom row has to be there too.
#[test]
fn two_bare_rules_without_a_box_are_not_a_popup() {
    let mut fb = Fb::new(496, 240);
    for y in (62 - 4)..(62 + 46) {
        for x in 188..270 {
            fb.set(x, y, 0xff9c_928c);
        }
    }
    for y in [62, 102] {
        for x in 192..192 + POPUP_RULE_W {
            fb.set(x, y, 0xff00_0000);
        }
    }
    assert_eq!(popup_box(&fb.px, fb.lw, fb.lh, 2), None);
}

/// Every row of the probe's popup, and the two kinds of miss.
#[test]
fn rows_map_to_the_bands_the_painter_draws() {
    let (bx, by, rows) = (192, 62, 2);
    // Row i covers y in [by+4+17i, by+4+17(i+1)).
    assert_eq!(popup_row_at(bx, by, rows, 198, 66), Some(0));
    assert_eq!(popup_row_at(bx, by, rows, 259, 82), Some(0));
    assert_eq!(popup_row_at(bx, by, rows, 198, 83), Some(1));
    assert_eq!(popup_row_at(bx, by, rows, 259, 99), Some(1));
    // Above the first row (the box's own top chrome) and below the last: not a row.
    assert_eq!(popup_row_at(bx, by, rows, 200, 65), None);
    assert_eq!(popup_row_at(bx, by, rows, 200, 100), None);
    // Left of x+6 and right of x+6+62: the box's side chrome.
    assert_eq!(popup_row_at(bx, by, rows, 197, 70), None);
    assert_eq!(popup_row_at(bx, by, rows, 260, 70), None);
}

/// ⛔ The mapper REFUSES rather than clamps. The popup cursor is written straight into
/// `CHAR+0x9c86`, whose only range check lives in the game's own key arm — the same
/// trap §7-1 of `TOUCH_MENU_FEASIBILITY.md` measured on the menu cursor.
#[test]
fn no_point_can_produce_a_row_past_the_count() {
    for rows in 1..=POPUP_MAX_ROWS {
        for y in 0..240 {
            for x in [192, 198, 230, 259, 265] {
                if let Some(r) = popup_row_at(192, 62, rows, x, y) {
                    assert!(i32::from(r) < rows, "rows={rows} produced row {r}");
                }
            }
        }
    }
    // A count outside the label-buffer domain answers nothing at all.
    for rows in [0, -1, POPUP_MAX_ROWS + 1, 99] {
        assert_eq!(popup_row_at(192, 62, rows, 200, 70), None, "rows={rows}");
        assert_eq!(popup_contains(192, 62, rows, 200, 70), false, "rows={rows}");
    }
}

/// The box test is wider than the row test on purpose: a tap on the popup's own
/// chrome must be SWALLOWED, not passed down to the inventory grid underneath it.
#[test]
fn the_box_swallows_its_own_chrome() {
    let (bx, by, rows) = (192, 62, 2);
    for (x, y) in [(192, 62), (265, 62), (192, 103), (265, 103), (200, 65), (200, 100)] {
        assert!(popup_contains(bx, by, rows, x, y), "({x},{y}) is inside the box");
        // ...and none of those chrome points is a row.
        if (x, y) != (200, 65) && (x, y) != (200, 100) {
            assert_eq!(popup_row_at(bx, by, rows, x, y), None);
        }
    }
    for (x, y) in [(191, 62), (266, 62), (192, 61), (192, 104)] {
        assert!(!popup_contains(bx, by, rows, x, y), "({x},{y}) is outside");
    }
}

/// The popup is drawn at whatever x/y its CALLER computed — two of the three shift it
/// when the list is tall (`0x427be` adds 14, `0x4456a` subtracts 25) — so the detector
/// must not care where it is. Same box, moved: still found.
#[test]
fn the_box_is_found_wherever_the_caller_put_it() {
    for (bx, by, rows) in [(0, 0, 1), (10, 30, 8), (496 - 74, 240 - (17 * 3 + 8), 3), (192, 48, 4)] {
        let mut fb = Fb::new(496, 240);
        paint_popup(&mut fb, bx, by, rows);
        assert_eq!(popup_box(&fb.px, fb.lw, fb.lh, rows), Some((bx, by)), "at ({bx},{by}) rows={rows}");
    }
}


// --------------------------------------------------------------------------
// the STATUS stat grid — 79차-C
//
// Probe, LGT asset, stat points forged with `--poke CHAR+0x18a = 10`:
//   cursor 0 -> panel dx 80  dy 69     cursor 1 -> dx 141 dy 69
//   cursor 2 -> dx 80  dy 83           cursor 3 -> dx 141 dy 83     all 31 x 12
//   cursor 4..15 -> the game draws no marker at all  (the grid really is 2x2)
// and the same panel-relative numbers at 496x240 AND 560x228.
// --------------------------------------------------------------------------

fn paint_status_marker(fb: &mut Fb, cell: u8) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let (col, row) = (i32::from(cell) % STATUS.cols, i32::from(cell) / STATUS.cols);
    let (x0, y0) = (STATUS.x0 + STATUS.pitch_x * col, STATUS.y0 + STATUS.pitch_y * row);
    for dy in y0..y0 + STATUS.h {
        for dx in x0..x0 + STATUS.w {
            // Only the border is really cyan; a filled block is a superset and the
            // detector works off the bounding box either way.
            let edge = dx == x0 || dx == x0 + STATUS.w - 1 || dy == y0 || dy == y0 + STATUS.h - 1;
            if edge {
                fb.set(ox + dx, oy + dy, 0xff00_0000 | STATUS_MARK_INK);
            }
        }
    }
}

#[test]
fn the_status_grid_is_the_measured_one() {
    assert_eq!((STATUS.x0, STATUS.y0), (80, 69));
    assert_eq!((STATUS.w, STATUS.h), (31, 12));
    assert_eq!((STATUS.pitch_x, STATUS.pitch_y), (61, 14));
    assert_eq!((STATUS.cols, STATUS.rows), (2, 2));
    assert_eq!(STATUS.cells(), 4);
    assert_eq!(STATUS_MARK_INK, 0x005a_ebff);
}

/// Every measured cursor, at both measured resolutions.
#[test]
fn the_marker_reads_back_the_cell_the_probe_poked() {
    for (lw, lh) in [(496, 240), (560, 228)] {
        for cell in 0..STATUS.cells() as u8 {
            let mut fb = Fb::new(lw, lh);
            paint_status_marker(&mut fb, cell);
            assert_eq!(status_marker(&fb.px, fb.lw, fb.lh), Some(cell), "{lw}x{lh} cell {cell}");
        }
    }
}

/// No marker = no stat list (nothing to spend) = the tap must do nothing. This is the
/// fail-closed half, and it is the reason the cross-check exists at all.
#[test]
fn no_marker_means_no_answer() {
    let fb = Fb::new(496, 240);
    assert_eq!(status_marker(&fb.px, fb.lw, fb.lh), None);
}

/// Cyan that is not a cell-sized box at a cell origin is not the marker.
#[test]
fn stray_cyan_is_not_the_marker() {
    // Wrong size.
    let mut fb = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for dx in 80..80 + 20 {
        fb.set(ox + dx, oy + 69, 0xff00_0000 | STATUS_MARK_INK);
    }
    assert_eq!(status_marker(&fb.px, fb.lw, fb.lh), None, "a 20-px line is not a cell");
    // Right size, wrong place (one pixel off a cell origin).
    let mut fb = Fb::new(496, 240);
    for dy in 70..70 + STATUS.h {
        for dx in 80..80 + STATUS.w {
            fb.set(ox + dx, oy + dy, 0xff00_0000 | STATUS_MARK_INK);
        }
    }
    assert_eq!(status_marker(&fb.px, fb.lw, fb.lh), None, "off-origin is not a cell");
}

/// The grid maps taps the way the cursor is laid out: `row * 2 + col`, which is why
/// one DOWN moved the probe's cursor from 0 to 2.
#[test]
fn the_status_grid_is_row_major_over_two_columns() {
    assert_eq!(STATUS.hit(80 + 15, 69 + 6), Some(0));
    assert_eq!(STATUS.hit(141 + 15, 69 + 6), Some(1));
    assert_eq!(STATUS.hit(80 + 15, 83 + 6), Some(2));
    assert_eq!(STATUS.hit(141 + 15, 83 + 6), Some(3));
    // ...and nothing it returns can be a cursor the game refuses to draw.
    for dy in 0..PANEL_H {
        for dx in 0..PANEL_W {
            if let Some(c) = STATUS.hit(dx, dy) {
                assert!(STATUS.holds(c), "({dx},{dy}) -> {c}");
                assert!(i32::from(c) < STATUS.cells());
            }
        }
    }
}


// --------------------------------------------------------------------------
// the EQUIPMENT 3x3 grid — 79차-D
//
// DEVICE, not probe: the probe's bare character has nothing for the cursor to skip.
// On a character wearing all nine pieces, the game's own keys gave
//   RIGHT x10 -> idx 0 1 2 3 4 5 6 7 8 0   (col 0 1 2 0 1 2 0 1 2, row 0 0 0 1 1 1 2 2 2)
//   DOWN  x5  -> idx 0 3 6 6 6              (clamps, does not wrap)
// and the yellow border, converted from the 2340x1080 screenshot into panel px with a
// scale checked against the menu row's known `dx 24..151 / dy 24..38`, sat at
//   idx 0 -> dx 5.12..20.81  dy 21.10..36.93
//   idx 4 -> dx 23.14..38.82 dy 39.04..54.88
//   idx 8 -> dx 41.16..56.84 dy 57.20..72.82
// --------------------------------------------------------------------------

fn paint_equip_marker_in(fb: &mut Fb, g: &Grid, cell: u8) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let (col, row) = (i32::from(cell) % g.cols, i32::from(cell) / g.cols);
    let (x0, y0) = (g.x0 + g.pitch_x * col, g.y0 + g.pitch_y * row);
    for dy in y0..y0 + g.h {
        for dx in x0..x0 + g.w {
            if dx == x0 || dx == x0 + g.w - 1 || dy == y0 || dy == y0 + g.h - 1 {
                fb.set(ox + dx, oy + dy, 0xff00_0000 | CELL_BOX_INK);
            }
        }
    }
}

fn paint_equip_marker(fb: &mut Fb, cell: u8) {
    paint_equip_marker_in(fb, &EQUIP, cell);
}

#[test]
fn the_equip_grid_is_the_measured_one() {
    assert_eq!((EQUIP.x0, EQUIP.y0), (5, 21));
    assert_eq!((EQUIP.w, EQUIP.h), (16, 16));
    assert_eq!((EQUIP.pitch_x, EQUIP.pitch_y), (18, 18));
    assert_eq!((EQUIP.cols, EQUIP.rows), (3, 3));
    assert_eq!(EQUIP.cells(), 9);
}

/// The device walk, as arithmetic: `idx = row*3 + col` over all nine cells.
#[test]
fn the_equip_grid_is_row_major_over_three_columns() {
    for cell in 0..9u8 {
        let (col, row) = (i32::from(cell) % 3, i32::from(cell) / 3);
        let (cx, cy) = (EQUIP.x0 + EQUIP.pitch_x * col + EQUIP.w / 2, EQUIP.y0 + EQUIP.pitch_y * row + EQUIP.h / 2);
        assert_eq!(EQUIP.hit(cx, cy), Some(cell), "cell {cell}");
    }
}

#[test]
fn the_equip_marker_reads_back_every_measured_cell() {
    for (lw, lh) in [(496, 228), (496, 240), (560, 228)] {
        for cell in 0..EQUIP.cells() as u8 {
            let mut fb = Fb::new(lw, lh);
            paint_equip_marker(&mut fb, cell);
            assert_eq!(
                equip_marker(&fb.px, fb.lw, fb.lh),
                Some(EquipMark { candidates: false, cell }),
                "{lw}x{lh} cell {cell}"
            );
        }
    }
}

/// ⛔ The inventory cell cursor uses the SAME yellow ink. It must not be mistaken for
/// an equipment slot: its cells are 17x17 at pitch 19 from a different origin, and the
/// marker demands an exact 16x16 at an exact EQUIP origin.
#[test]
fn the_inventory_cursor_is_not_an_equipment_marker() {
    for cell in 0..16u8 {
        let mut fb = Fb::new(496, 228);
        let (ox, oy) = panel_origin(496, 228).unwrap();
        let (col, row) = (i32::from(cell) % INVEN.cols, i32::from(cell) / INVEN.cols);
        let (x0, y0) = (INVEN.x0 + INVEN.pitch_x * col, INVEN.y0 + INVEN.pitch_y * row);
        for dy in y0..y0 + INVEN.h {
            for dx in x0..x0 + INVEN.w {
                if dx == x0 || dx == x0 + INVEN.w - 1 || dy == y0 || dy == y0 + INVEN.h - 1 {
                    fb.set(ox + dx, oy + dy, 0xff00_0000 | CELL_BOX_INK);
                }
            }
        }
        assert_eq!(equip_marker(&fb.px, fb.lw, fb.lh), None, "inventory cell {cell}");
    }
}

// --------------------------------------------------------------------------
// the TITLE CAROUSEL and the OPTION window (80차-K)
// --------------------------------------------------------------------------

/// The measured carousel: two 12-wide arrow runs at dx 41..52 and 123..134, dy 135..139.
fn carousel(fb: &mut Fb) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    for (a, b) in [(41, 52), (123, 134)] {
        // A chevron is sparse VERTICALLY (a `<` is a diagonal), but every column of the
        // run carries ink somewhere — which is what the run scan looks at (probe: the
        // measured runs are contiguous over all 12 columns).
        for dx in a..=b {
            let dy = 135 + (dx - a) % 5;
            fb.set(ox + dx, oy + dy, 0xff00_0000 | CAROUSEL_INK);
            fb.set(ox + dx, oy + 139 - (dx - a) % 5, 0xff00_0000 | CAROUSEL_INK);
        }
    }
}

#[test]
fn the_title_carousel_is_three_targets_on_one_line() {
    let mut fb = Fb::new(496, 240);
    carousel(&mut fb);
    let c = title_carousel(&fb.px, 496, 240).expect("the carousel is drawn");
    // The whole line, split at the arrows — measured on six screens (probe).
    assert_eq!((c.prev.dx, c.prev.dx + c.prev.w - 1), (0, 55));
    assert_eq!((c.next.dx, c.next.dx + c.next.w - 1), (120, PANEL_W - 1));
    assert_eq!((c.pick.dx, c.pick.dx + c.pick.w - 1), (56, 119));
    assert_eq!(c.prev.dy, c.pick.dy);
    assert_eq!(c.prev.h, c.pick.h);
    // The arrows themselves are inside their own targets, and the three are disjoint.
    assert!(c.prev.contains(46, 137) && c.next.contains(128, 137));
    for dx in 0..PANEL_W {
        let n = i32::from(c.prev.contains(dx, 137)) + i32::from(c.pick.contains(dx, 137)) + i32::from(c.next.contains(dx, 137));
        assert_eq!(n, 1, "column {dx} is claimed {n} times");
    }
}

/// Fail closed — and the clause that matters is the WIDTH: the title art carries a stray
/// 2 px run of the same ink (probe, the 게임방법 frame at dx 38..39).
#[test]
fn the_carousel_refuses_anything_but_two_arrow_runs() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    assert_eq!(title_carousel(&Fb::new(496, 240).px, 496, 240), None, "nothing drawn");
    assert_eq!(title_carousel(&[], 496, 240), None);
    assert_eq!(title_carousel(&Fb::new(496, 240).px, 0, 0), None);
    // One arrow only.
    let mut lone = Fb::new(496, 240);
    for dy in 135..=139 {
        for dx in 41..=52 {
            lone.set(ox + dx, oy + dy, 0xff00_0000 | CAROUSEL_INK);
        }
    }
    assert_eq!(title_carousel(&lone.px, 496, 240), None, "one arrow is not a carousel");
    // …and a stray speck alongside the real pair changes nothing.
    let mut stray = Fb::new(496, 240);
    carousel(&mut stray);
    for dx in 38..=39 {
        stray.set(ox + dx, oy + 65, 0xff00_0000 | CAROUSEL_INK);
    }
    assert_eq!(title_carousel(&stray.px, 496, 240), title_carousel(&{ let mut f = Fb::new(496, 240); carousel(&mut f); f }.px, 496, 240));
}

/// The measured HELP header: two 7x11 triangles with the dark title strip between them.
fn help_window(fb: &mut Fb) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    for dy in 19..=32 {
        for dx in 32..=143 {
            fb.set(ox + dx, oy + dy, 0xff00_0000 | PAGER_TITLE_INK);
        }
    }
    for a in [18, 150] {
        for dy in 21..=31 {
            for dx in a..a + 7 {
                fb.set(ox + dx, oy + dy, 0xff00_0000 | PAGER_TRI_INK);
            }
        }
    }
}

/// ★★★ 80차-L — 게임방법 (HELP). Player: *"게임 방법창 진행하자"*.
/// The keys are PROBE-CONFIRMED, not assumed: LEFT/RIGHT paged 「기본 인터페이스」 →
/// 「네오 솔티아 소개」 and DOWN scrolled the body (the thumb moved with it).
#[test]
fn the_help_window_pages_with_its_arrows_and_scrolls_with_its_body() {
    let mut fb = Fb::new(496, 240);
    help_window(&mut fb);
    let h = pager_header(&fb.px, 496, 240).expect("the HELP header is drawn");
    // The arrows own the header's outer thirds; the title strip between them is the pick.
    assert!(h.prev.contains(21, 26) && h.next.contains(153, 26), "the triangles are in their own targets");
    assert!(!h.prev.contains(88, 26) && !h.next.contains(88, 26), "the title strip is not an arrow");
    assert!(h.pick.contains(88, 26), "the title strip is the pick");
    assert!(!h.pick.contains(21, 26) && !h.pick.contains(153, 26), "the pick may not eat an arrow");
    assert!(h.prev.dx + h.prev.w <= h.next.dx, "the two arrows overlap");
    // The body splits into scroll-up and scroll-down, and covers the measured extent.
    assert_eq!(h.up.dy, PAGER_BODY_TOP);
    assert_eq!(h.down.dy + h.down.h - 1, PAGER_BODY_BOT);
    assert_eq!(h.up.dy + h.up.h, h.down.dy, "the two halves must not leave a gap");
    assert!(h.up.contains(88, PAGER_BODY_TOP + 2) && h.down.contains(88, PAGER_BODY_BOT - 2));
    // ⛔ The header and the body may not overlap — a page arrow that also scrolls is a
    // control that does two things.
    assert!(h.prev.dy + h.prev.h <= h.up.dy, "the header runs into the body");
}

/// Fail closed, and the clause that carries it is the TITLE STRIP: two dark triangles on
/// their own are not rare, but "two triangles with a filled strip between them" is.
#[test]
fn the_pager_header_refuses_a_header_it_cannot_see() {
    assert_eq!(pager_header(&Fb::new(496, 240).px, 496, 240), None, "nothing drawn");
    assert_eq!(pager_header(&[], 496, 240), None);
    assert_eq!(pager_header(&Fb::new(496, 240).px, 0, 0), None);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    // Triangles with NO strip between them.
    let mut bare = Fb::new(496, 240);
    for a in [18, 150] {
        for dy in 21..=31 {
            for dx in a..a + 7 {
                bare.set(ox + dx, oy + dy, 0xff00_0000 | PAGER_TRI_INK);
            }
        }
    }
    assert_eq!(pager_header(&bare.px, 496, 240), None, "no title strip -> not the HELP window");
    // One triangle only.
    let mut lone = Fb::new(496, 240);
    help_window(&mut lone);
    for dy in 21..=31 {
        for dx in 150..157 {
            lone.set(ox + dx, oy + dy, 0xff00_0000);
        }
    }
    assert_eq!(pager_header(&lone.px, 496, 240), None, "one arrow is not a pager");
}

/// ★★★ 80차-N — the SAME header drives 새 캐릭터 생성's 국가(FORCE) and 직업(CLASS).
/// FORCE is the one that pins the fill threshold: its title strip carries the nation's
/// FLAG ICON, so the middle row is only 84/124 = 0.68 filled (CLASS 0.85, HELP 0.81).
/// At 3/4 that screen was missed outright (probe-measured).
#[test]
fn the_pager_header_survives_an_icon_in_its_title_strip() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let mut fb = Fb::new(496, 240);
    help_window(&mut fb);
    // Punch the flag icon out of the strip — sized to reproduce FORCE's measured 0.68.
    for dy in 19..=32 {
        for dx in 34..61 {
            fb.set(ox + dx, oy + dy, 0xffff_0000);
        }
    }
    assert!(pager_header(&fb.px, 496, 240).is_some(), "an icon in the title strip is still a title strip");
    // …but a strip that is mostly NOT the title ink is not this widget.
    let mut gone = Fb::new(496, 240);
    help_window(&mut gone);
    for dy in 19..=32 {
        for dx in 34..130 {
            gone.set(ox + dx, oy + dy, 0xffff_0000);
        }
    }
    assert_eq!(pager_header(&gone.px, 496, 240), None, "no title strip -> not the widget");
}

/// ★★★ 80차-N — the confirm is found from its OUTLINE and the fill is only VERIFIED.
///
/// 새 캐릭터 생성's 「이대로 진행합니까?」 puts `CONFIRM3_SEL_INK` in the screen's own chrome
/// as well, so the old global bbox of that ink came out 152x123 and the window was invisible
/// to all three recognisers (probe-measured). The outline is clean on that screen, and the
/// panel-centre fold lands on a rect that is 93 % filled.
#[test]
fn the_shell_confirm_is_found_from_its_outline_even_with_the_fill_ink_scattered() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let mut fb = Fb::new(496, 240);
    // the measured pair: outline right, fill left
    for dy in 107..=121 {
        for dx in 114..=152 {
            fb.set(ox + dx, oy + dy, 0xff00_0000 | CONFIRM3_OUT_INK);
        }
        for dx in 23..=61 {
            fb.set(ox + dx, oy + dy, 0xff00_0000 | CONFIRM3_SEL_INK);
        }
    }
    // …and the same ink smeared over the window chrome, which is what broke the old rule.
    for dy in 0..6 {
        for dx in 10..162 {
            fb.set(ox + dx, oy + dy, 0xff00_0000 | CONFIRM3_SEL_INK);
        }
    }
    let (sel, other) = confirm_pair(&fb.px, 496, 240).expect("the outline is clean, so the pair is findable");
    assert_eq!((sel.dx, sel.w), (23, 39));
    assert_eq!((other.dx, other.w), (114, 39));
    assert!(sel.dx + sel.w < other.dx, "the two buttons overlap");

    // ⛔ The fill has to actually be there: an outline with nothing on its mirror is not a pair.
    let mut lone = Fb::new(496, 240);
    for dy in 107..=121 {
        for dx in 114..=152 {
            lone.set(ox + dx, oy + dy, 0xff00_0000 | CONFIRM3_OUT_INK);
        }
    }
    assert_eq!(confirm_pair(&lone.px, 496, 240), None, "no fill on the mirror -> no pair");
}

/// ★★★ 80차-N — when the footer has a `#` prompt, the OTHER prompt is the `OK` one.
///
/// The colon-isolation rule 80차-F used is one antialias pixel wide: on the true guest
/// framebuffer the `OK` sprite reads dx 0..8 · 10..17 · 21, so the gap before the colon is
/// 2, not the 8 a re-sampled device screenshot showed. The empty-slot footer
/// (`OK : 캐릭터생성` + `# : 복구`) was invisible because of it.
///
/// The replacement is not a pixel measurement at all: the guest's own footer strings
/// (80차-D's exhaustive list) pair `#` only with OK-led prompts, and `CLR` never shares a
/// footer with `#`.
#[test]
fn a_footer_with_a_hash_prompt_makes_its_partner_the_ok_prompt() {
    let mut fb = Fb::new(496, 240);
    slot_footer(&mut fb);
    let h = hash_prompt(&fb.px, 496, 240).expect("the `#` prompt is drawn");
    let o = ok_token_prompt(&fb.px, 496, 240).expect("so its partner is the OK prompt");
    assert!(o.dx + o.w <= h.dx, "the two prompts must not overlap");
    assert!(o.contains(5, 155), "the `OK` token is in it");
    assert!(!o.contains(128, 155), "the `#` run is not");
    // ⛔ …and it must survive a token the colon rule cannot read. Fill the gap the rule
    // needs, the way the real framebuffer's antialiasing does.
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let mut fuzzy = Fb::new(496, 240);
    slot_footer(&mut fuzzy);
    for dy in 152..160 {
        for dx in [16, 17, 21, 22, 23] {
            fuzzy.set(ox + dx, oy + dy, 0xfff7_e3b5);
        }
    }
    assert_eq!(ok_token_prompt(&fuzzy.px, 496, 240), Some(o), "the hash partner does not need the colon");
}

/// The measured OPTION window: seven rows, the cursor being the one that is NOT bright.
///
/// The panel ground is the game's parchment checker (`#9c968c`/`#948a7b`) and the cursor row
/// is its dark dither (`#4a3d31`/`#4a3929`) — both measured. Painting the ground matters:
/// the detector tells "the cursor is the top row" from "there is no row up there" by asking
/// whether that band is DARK, and a black test canvas answers yes to both.
fn option_window_at(fb: &mut Fb, cursor: u8, y0: i32, h: i32, pitch: i32) {
    let tops: Vec<i32> = (0..OPTION_ROWS).map(|r| y0 + r * pitch).collect();
    option_window_tops(fb, cursor, &tops, h);
}

fn option_window_tops(fb: &mut Fb, cursor: u8, tops: &[i32], h: i32) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    for dy in 0..PANEL_H {
        for dx in 0..PANEL_W {
            fb.set(ox + dx, oy + dy, if (dx + dy) & 1 == 0 { 0xff9c_968c } else { 0xff94_8a7b });
        }
    }
    for (row, &top) in tops.iter().enumerate() {
        for dy in top..top + h {
            for dx in OPTION_X0..OPTION_X0 + OPTION_W {
                let ink = if row as u8 == cursor {
                    if (dx + dy) & 1 == 0 { 0xff4a_3d31 } else { 0xff4a_3929 }
                } else {
                    0xff00_0000 | OPTION_ROW_INK
                };
                fb.set(ox + dx, oy + dy, ink);
            }
        }
    }
}

fn option_window(fb: &mut Fb, cursor: u8) {
    option_window_at(fb, cursor, OPTION_Y0, OPTION_H, OPTION_PITCH);
}

#[test]
fn the_option_window_reports_its_seven_rows_and_the_dark_one() {
    for cursor in 0..OPTION_ROWS as u8 {
        let mut fb = Fb::new(496, 240);
        option_window(&mut fb, cursor);
        let (cur, rows) = option_rows(&fb.px, 496, 240).expect("the OPTION window is drawn");
        assert_eq!(cur, cursor);
        assert_eq!(rows.len(), OPTION_ROWS as usize);
        for (i, r) in rows.iter().enumerate() {
            assert_eq!((r.dx, r.w), (OPTION_X0, OPTION_W));
            assert_eq!(r.dy, OPTION_Y0 + i as i32 * OPTION_PITCH);
            assert!(r.contains(OPTION_X0 + 5, r.dy + 5), "row {i} does not contain its own middle");
        }
        // …and the value arrows are inside the cursor's row, on the side the game drew them.
        assert!(rows[usize::from(cursor)].contains(OPTION_VALUE_X0 + 2, rows[usize::from(cursor)].dy + 5));
    }
}

/// ⛔⛔⛔ 80차-O — AND THE COLLISION THAT ACTUALLY SHIPPED: the in-game six-row MENU list.
///
/// Player, 2026-09-05: *"인게임 메뉴창에서 현재 포커스된 항목을 한번 더누르면 들어가져야하는데
/// 안들어가져"*. Device (vc266): `menu tap: panel (88,49) -> option owns the panel (cursor 1)`
/// — the OPTION arm was eating every tap on the game's own menu.
///
/// Row count did NOT separate them, and the arithmetic says why: MENU rows are 15 tall at
/// pitch **19**, OPTION's bands are 14 tall at pitch **17**, so one OPTION band straddles
/// TWO menu rows and still comes out "bright" (measured band 4 = 4 rows of MENU row 3 +
/// 6 rows of MENU row 4 = 1120 px). Six bright bands, one dark where the menu cursor is —
/// exactly what this detector was looking for.
///
/// What separates them is the GAPS: at pitch 19 the menu's rows fall right across four of
/// OPTION's six inter-row gaps (device: dy 73 · 90 · 107 · 124 read 112 · 99 · 98 · 92 px).
#[test]
fn the_option_window_is_not_the_in_game_menu_list() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let mut menu = Fb::new(496, 240);
    // The measured in-game MENU list: six rows, dy 24 at pitch 19, 15 tall, x 24..151,
    // with the cursor (row 1) drawn dark. Same ink as OPTION's rows.
    for row in 0..6 {
        if row == 1 {
            continue;
        }
        let y0 = 24 + row * 19;
        for dy in y0..y0 + 15 {
            for dx in 24..=151 {
                menu.set(ox + dx, oy + dy, 0xff00_0000 | OPTION_ROW_INK);
            }
        }
    }
    assert_eq!(option_rows(&menu.px, 496, 240), None, "the game's own MENU list is not the OPTION window");
    // …and this is why: the menu only ever paints FIVE bright bands, because one of its six
    // rows is the cursor. Counting found bands is what separates them; a fixed grid could not.
    let lit = |dy: i32| (32..144).filter(|&dx| menu.px[((oy + dy) * 496 + ox + dx) as usize] & 0x00ff_ffff == OPTION_ROW_INK).count() as i32;
    let bands = (0..PANEL_H).filter(|&dy| lit(dy) >= OPTION_RUN_MIN && lit(dy - 1) < OPTION_RUN_MIN).count();
    assert_eq!(bands, 5, "the menu draws five bright bands, not six");
}

/// ⛔⛔⛔ 80차-O — THE SAME WINDOW IS NOT THE SAME SIZE ON THE DEVICE.
///
/// `wie_probe` (496x240) draws it at `dy0 24 · h 14 · pitch 17`; the S9 draws it at
/// `dy0 19 · h 14 · pitch **18**` (in-game 시스템 → 환경설정, measured — the cursor row's
/// fourteen dark-pixel counts are byte-identical between the two frames, so this is the
/// game's own geometry and not a sampling artefact).
///
/// 80차-K nailed the probe's numbers down as constants. On the device that grid landed one
/// row too high: the band above row 0 is panel ground, which reads "not bright" — so the
/// detector still found a window, but called the cursor 0 every single time. The gap check
/// vc267 added then killed it outright.
/// ⛔⛔ …and the pitch is not even constant INSIDE one window: the S9 draws the seven rows at
/// `19 · 36 · 54 · 72 · 90 · 108 · 126` — one gap of 17 and five of 18. The first cut of 80차-O
/// asked the hole to be exactly `2 * pitch` and so refused every cursor but the first
/// (device: `option row 0 -> 1: DOWN x1`, then `is not a row or a cell on this screen`).
#[test]
fn the_option_window_survives_a_pitch_that_is_not_constant() {
    let tops = [19, 36, 54, 72, 90, 108, 126];
    for cursor in 0..OPTION_ROWS as u8 {
        let mut fb = Fb::new(496, 240);
        option_window_tops(&mut fb, cursor, &tops, 14);
        let (cur, rows) = option_rows(&fb.px, 496, 240).unwrap_or_else(|| panic!("measured S9 OPTION, cursor {cursor}"));
        assert_eq!(cur, cursor);
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.dy, tops[i], "row {i}");
        }
    }
}

#[test]
fn the_option_window_is_measured_not_assumed() {
    for (y0, h, pitch) in [(OPTION_Y0, OPTION_H, OPTION_PITCH), (19, 14, 18), (24, 13, 16), (25, 15, 20)] {
        for cursor in 0..OPTION_ROWS as u8 {
            let mut fb = Fb::new(496, 240);
            option_window_at(&mut fb, cursor, y0, h, pitch);
            let (cur, rows) = option_rows(&fb.px, 496, 240).unwrap_or_else(|| panic!("OPTION at {y0}/{h}/{pitch} cursor {cursor}"));
            assert_eq!(cur, cursor, "cursor at {y0}/{h}/{pitch}");
            assert_eq!(rows.len(), OPTION_ROWS as usize);
            for (i, r) in rows.iter().enumerate() {
                assert_eq!(r.dy, y0 + i as i32 * pitch, "row {i} at {y0}/{h}/{pitch}");
                assert!(r.contains(OPTION_X0 + 5, y0 + i as i32 * pitch + 5));
            }
        }
    }
}

/// ⛔⛔ THE COLLISION THIS ARM EXISTS FOR: the shell SLOT list is three rows of the SAME
/// colour at the SAME pitch, two pixels higher — so `slot_rows` answers "cursor 0" on the
/// OPTION window (verified on the probe's own frame). Requiring all SEVEN rows is what
/// tells them apart, and `app::menu_tap_decide` tests OPTION first because of it.
#[test]
fn the_option_window_and_the_slot_list_are_told_apart_by_the_row_count() {
    let mut opt = Fb::new(496, 240);
    option_window(&mut opt, 0);
    assert!(option_rows(&opt.px, 496, 240).is_some());
    assert!(slot_rows(&opt.px, 496, 240).is_some(), "the collision is real — this is why the order matters");

    // The slot list itself must NOT be read as an OPTION window.
    let mut slots = Fb::new(496, 240);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    for row in 0..3 {
        if row == 1 {
            continue;
        }
        let y0 = SLOT_ROW_TOP + row * SLOT_ROW_PITCH;
        for dy in y0..y0 + SLOT_ROW_H {
            for dx in SLOT_ROW_X0..SLOT_ROW_X0 + SLOT_ROW_W {
                slots.set(ox + dx, oy + dy, 0xff00_0000 | SLOT_ROW_LIGHT);
            }
        }
    }
    assert_eq!(option_rows(&slots.px, 496, 240), None, "three rows are not seven");
    assert!(slot_rows(&slots.px, 496, 240).is_some());
}

#[test]
fn the_option_window_refuses_a_frame_it_cannot_read() {
    assert_eq!(option_rows(&Fb::new(496, 240).px, 496, 240), None, "nothing drawn");
    assert_eq!(option_rows(&[], 496, 240), None);
    assert_eq!(option_rows(&Fb::new(496, 240).px, 0, 0), None);
    // Two dark rows -> which one is the cursor has no answer.
    let mut two = Fb::new(496, 240);
    option_window(&mut two, 0);
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let y0 = OPTION_Y0 + 3 * OPTION_PITCH;
    for dy in y0..y0 + OPTION_H {
        for dx in OPTION_X0..OPTION_X0 + OPTION_W {
            two.set(ox + dx, oy + dy, 0xff00_0000);
        }
    }
    assert_eq!(option_rows(&two.px, 496, 240), None, "two dark rows -> refuse");
    // A hole with nothing dark in it is not a cursor either — six rows and a gap of ground.
    let mut hole = Fb::new(496, 240);
    option_window(&mut hole, 3);
    for dy in y0..y0 + OPTION_H {
        for dx in OPTION_X0..OPTION_X0 + OPTION_W {
            hole.set(ox + dx, oy + dy, if (dx + dy) & 1 == 0 { 0xff9c_968c } else { 0xff94_8a7b });
        }
    }
    assert_eq!(option_rows(&hole.px, 496, 240), None, "an empty hole is not the cursor");
}

/// ★★★ 80차-I — the CANDIDATE list 「장비장착」 opens on the right. Same size, same
/// cursor bytes, 112 px further right; the origin is the only thing that tells them
/// apart, which is why the marker has to say WHICH grid it is on.
///
/// Player: *"포커스 된상태에서 장비장착하면 우측에 현재 장비들이뜨는데 거기서는 또
/// 터치가안돼"* — the old marker only knew the LEFT grid, so on the candidate pane it
/// answered `None` and the worker refused every tap.
#[test]
fn the_candidate_marker_is_told_from_the_slot_marker_by_its_origin() {
    for (lw, lh) in [(496, 228), (496, 240), (560, 228)] {
        for cell in 0..EQUIP_CAND.cells() as u8 {
            let mut fb = Fb::new(lw, lh);
            paint_equip_marker_in(&mut fb, &EQUIP_CAND, cell);
            assert_eq!(
                equip_marker(&fb.px, fb.lw, fb.lh),
                Some(EquipMark { candidates: true, cell }),
                "{lw}x{lh} candidate {cell}"
            );
        }
    }
    // The two grids differ ONLY in x, and they may not touch.
    assert_eq!(EQUIP_CAND.y0, EQUIP.y0);
    assert_eq!((EQUIP_CAND.w, EQUIP_CAND.h), (EQUIP.w, EQUIP.h));
    assert_eq!((EQUIP_CAND.cols, EQUIP_CAND.rows), (EQUIP.cols, EQUIP.rows));
    assert_eq!(EQUIP_CAND.x0, 117, "measured on the device (vc258)");
    assert!(
        EQUIP.x0 + EQUIP.pitch_x * EQUIP.cols < EQUIP_CAND.x0,
        "the worn-slot grid must end before the candidate grid starts"
    );
    // …and a tap in one is never a cell of the other.
    for cell in 0..EQUIP.cells() {
        let (col, row) = (cell % EQUIP.cols, cell / EQUIP.cols);
        let (cx, cy) = (EQUIP.x0 + EQUIP.pitch_x * col + EQUIP.w / 2, EQUIP.y0 + EQUIP.pitch_y * row + EQUIP.h / 2);
        assert_eq!(EQUIP_CAND.hit(cx, cy), None, "worn cell {cell} is inside the candidate grid");
        let (cx, cy) = (EQUIP_CAND.x0 + EQUIP_CAND.pitch_x * col + EQUIP_CAND.w / 2, cy);
        assert_eq!(EQUIP.hit(cx, cy), None, "candidate cell {cell} is inside the worn grid");
    }
}

/// No border on screen = not the equipment screen = the tap does nothing.
#[test]
fn no_equip_marker_means_no_answer() {
    let fb = Fb::new(496, 228);
    assert_eq!(equip_marker(&fb.px, fb.lw, fb.lh), None);
}

/// Nothing the grid returns can be a cursor past the nine cells.
#[test]
fn no_point_can_produce_an_equip_cell_past_the_grid() {
    for dy in 0..PANEL_H {
        for dx in 0..PANEL_W {
            if let Some(c) = EQUIP.hit(dx, dy) {
                assert!(EQUIP.holds(c) && i32::from(c) < 9, "({dx},{dy}) -> {c}");
            }
        }
    }
}


// --------------------------------------------------------------------------
// a FULL bag has no empty frames — 79차-E
//
// `item_grid` derives the grid from the game's own EMPTY cell frames, and it needs
// two runs on a row. A full bag draws none, so on a LV70 save the inventory was not
// tappable at all (device report: "가방 들어갔을때 템 선택(포커스 이동)은 터치가 안되고
// 있어", while the action popup on top of it WAS tappable — that path is a different
// scan). The fallback anchors the known bag grid on the cursor border.
// --------------------------------------------------------------------------

/// Paint just the yellow cell cursor at `cell` of `g`, and no empty frames at all.
fn paint_cursor_only(fb: &mut Fb, g: &Grid, cell: u8) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let (col, row) = (i32::from(cell) % g.cols, i32::from(cell) / g.cols);
    let (x0, y0) = (g.x0 + g.pitch_x * col, g.y0 + g.pitch_y * row);
    // The shape the game draws (measured, 66차-B): corners empty — the horizontal
    // edges are inset by one and the verticals run between them.
    for i in 1..g.w - 1 {
        fb.set(ox + x0 + i, oy + y0, INK_YEL);
        fb.set(ox + x0 + i, oy + y0 + g.h - 1, INK_YEL);
    }
    for j in 1..g.h - 1 {
        fb.set(ox + x0, oy + y0 + j, INK_YEL);
        fb.set(ox + x0 + g.w - 1, oy + y0 + j, INK_YEL);
    }
}

#[test]
fn origin_index_wants_an_exact_cell_origin() {
    assert_eq!(INVEN.origin_index(INVEN.x0, INVEN.y0), Some(0));
    assert_eq!(INVEN.origin_index(INVEN.x0 + INVEN.pitch_x * 5, INVEN.y0), Some(5));
    assert_eq!(INVEN.origin_index(INVEN.x0 + INVEN.pitch_x, INVEN.y0 + INVEN.pitch_y), Some(9));
    // one pixel off is not a cell of this grid
    assert_eq!(INVEN.origin_index(INVEN.x0 + 1, INVEN.y0), None);
    assert_eq!(INVEN.origin_index(INVEN.x0, INVEN.y0 - 1), None);
    // past the last column / row
    assert_eq!(INVEN.origin_index(INVEN.x0 + INVEN.pitch_x * 8, INVEN.y0), None);
    assert_eq!(INVEN.origin_index(INVEN.x0, INVEN.y0 + INVEN.pitch_y * 2), None);
}

#[test]
fn a_full_bag_still_yields_the_bag_grid() {
    for (lw, lh) in [(496, 240), (496, 228), (560, 240)] {
        for cell in 0..16u8 {
            let mut fb = Fb::new(lw, lh);
            paint_cursor_only(&mut fb, &INVEN, cell);
            let (g, idx) = item_grid(&fb.px, fb.lw, fb.lh).unwrap_or_else(|| panic!("{lw}x{lh} cell {cell}"));
            assert_eq!(idx, cell);
            assert_eq!((g.x0, g.y0, g.pitch_x, g.cols, g.rows), (INVEN.x0, INVEN.y0, INVEN.pitch_x, INVEN.cols, INVEN.rows));
            // and the tap maps back to the same cell
            let (col, row) = (i32::from(cell) % g.cols, i32::from(cell) / g.cols);
            assert_eq!(g.hit(g.x0 + g.pitch_x * col + g.w / 2, g.y0 + g.pitch_y * row + g.h / 2), Some(cell));
        }
    }
}

/// ⛔ The fallback must not claim a grid it has never measured. 일반조합 is 5x1 at pitch
/// 29; only its cell 3 (dx 108) happens to sit on an INVEN origin, and there the index
/// it would report (5) is not the game's cursor (3) — the worker's cross-check refuses
/// it. Every other cell is simply not on the bag grid.
#[test]
fn a_full_craft_window_is_not_mistaken_for_the_bag() {
    let craft = Grid { x0: 21, y0: INVEN.y0, w: 17, h: 17, pitch_x: 29, pitch_y: 19, cols: 5, rows: 1, pad_x: 0, pad_y: 0 };
    for cell in 0..5u8 {
        let mut fb = Fb::new(496, 240);
        paint_cursor_only(&mut fb, &craft, cell);
        match item_grid(&fb.px, fb.lw, fb.lh) {
            None => {}
            Some((_, idx)) => assert_ne!(idx, cell, "craft cell {cell} answered as its own index"),
        }
    }
}

// ── the SHELL confirm window (80차-B) ────────────────────────────────────────
//
// ★★★ THE REGRESSION TEST FOR THE 2026-09-05 REPORT — *"confirm류 팝업들에서
// 터치안먹고있고"*. The 예/아니요 box on 「네트워크 창고에 접속하시겠습니까?」 is drawn with a
// DIFFERENT palette from the 제련 one `confirm_pair` was measured on (66차-B), so the
// detector saw nothing there even after the live-character gate stopped refusing it.
//
// Measured on the device (S9, vc246, offline): selected = dx 25..64 / dy 91..106 filled
// with 0x4a3121, unselected = dx 111..150 / dy 91..105 as an OUTLINE in 0x9c7d73.

/// Paint the shell confirm exactly as the device draws it.
fn shell_confirm(fb: &mut Fb, sel_left: bool) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let (fill_x, out_x) = if sel_left { (25, 111) } else { (111, 25) };
    for dy in 91..=106 {
        for dx in fill_x..fill_x + 40 {
            fb.set(ox + dx, oy + dy, CONFIRM3_SEL_INK);
        }
    }
    // ⛔ An outline, not a fill: full top and bottom rows, 2px sides between them. A
    // detector that wants "40 ink px on every row" would miss it — that is why
    // `confirm_pair_shell` uses a bounding box.
    for dx in out_x..out_x + 40 {
        fb.set(ox + dx, oy + 91, CONFIRM3_OUT_INK);
        fb.set(ox + dx, oy + 105, CONFIRM3_OUT_INK);
    }
    for dy in 92..105 {
        for i in 0..2 {
            fb.set(ox + out_x + i, oy + dy, CONFIRM3_OUT_INK);
            fb.set(ox + out_x + 39 - i, oy + dy, CONFIRM3_OUT_INK);
        }
    }
}

#[test]
fn the_shell_confirm_window_is_found_and_its_buttons_are_the_right_way_round() {
    for sel_left in [true, false] {
        let mut fb = Fb::new(496, 240);
        shell_confirm(&mut fb, sel_left);
        let (sel, other) = confirm_pair(&fb.px, fb.lw, fb.lh).expect("shell confirm not found");
        assert_eq!((sel.w, other.w), (40, 40), "both buttons are 40 wide");
        assert_eq!(sel.dx, if sel_left { 25 } else { 111 }, "the FILLED one is the selected one");
        assert_eq!(other.dx, if sel_left { 111 } else { 25 });
        // The whole point: tapping either one has to land inside it.
        assert!(sel.contains(sel.dx + 5, 99));
        assert!(other.contains(other.dx + 5, 99));
        assert!(!sel.contains(other.dx + 5, 99), "the two must not overlap");
    }
}

/// The in-game `예 / 아니오` confirm — the SHARED LIST WIDGET in horizontal mode, which is
/// a different thing from the two modal painters above (80차-G).
///
/// Measured on the device (S9, vc255, 인게임 → 시스템 → 세이브 → 「저장하시겠습니까?」):
/// selected cell filled with `CONFIRM3_SEL_INK` at dx 32..83 / dy 71..84 (52x14, fill 0.94
/// with the white 「예」 on top), and the unselected side carrying **no box at all** — just
/// dim `#a5827b` text. The `focus probe` in the same frame: `axis=0 rows=2 cur=0 choice=1`.
fn widget_confirm(fb: &mut Fb, sel_left: bool) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let g = confirm_grid(2).unwrap();
    let sel = if sel_left { 0 } else { 1 };
    let x0 = g.x0 + sel * g.pitch_x;
    for dy in g.y0..g.y0 + g.h {
        for dx in x0..x0 + g.w {
            fb.set(ox + dx, oy + dy, 0xff00_0000 | CONFIRM3_SEL_INK);
        }
    }
    // the label on top of it, and the dim label on the other side — neither is a box
    for dx in x0 + 24..x0 + 30 {
        for dy in g.y0 + 2..g.y0 + 12 {
            fb.set(ox + dx, oy + dy, 0xffff_ffff);
        }
    }
    let other = g.x0 + (1 - sel) * g.pitch_x;
    for dx in other + 10..other + 42 {
        for dy in g.y0 + 2..g.y0 + 12 {
            fb.set(ox + dx, oy + dy, 0xffa5_827b);
        }
    }
}

/// ★★★ 80차-G — the box the player could not press. Player: *"인게임 -> 시스템 -> 세이브 …
/// 예 혹은 아니오 선택해도 팝업창에서 동작을 안하고 갑자기 뒤에있는 화면에 터치가돼"*.
#[test]
fn the_widget_confirm_is_read_from_the_one_filled_cell() {
    for sel_left in [true, false] {
        let mut fb = Fb::new(496, 240);
        widget_confirm(&mut fb, sel_left);
        let cell = confirm_cell(&fb.px, 496, 240).expect("one cell is filled");
        assert_eq!(cell, u8::from(!sel_left));
        let (sel, other) = confirm_pair(&fb.px, 496, 240).expect("both buttons");
        let g = confirm_grid(2).unwrap();
        assert_eq!((sel.w, sel.h), (g.w, g.h), "the button is the widget's own cell");
        assert_eq!(sel.dx, if sel_left { CONFIRM_X[0] } else { CONFIRM_X[1] }, "the FILLED one is selected");
        assert_eq!(other.dx, if sel_left { CONFIRM_X[1] } else { CONFIRM_X[0] });
        assert!(sel.contains(sel.dx + 5, CONFIRM_Y + 5) && other.contains(other.dx + 5, CONFIRM_Y + 5));
        assert!(!sel.contains(other.dx + 5, CONFIRM_Y + 5), "the two must not overlap");
        // …and the two boxes are exactly the grid the worker uses, so the UI's key and the
        // worker's cursor write can never disagree about which cell was tapped.
        assert_eq!(g.hit(sel.dx + 5, CONFIRM_Y + 5), Some(cell));
    }
}

/// Fail closed, and each clause is a thing the game or another screen actually draws.
#[test]
fn the_widget_confirm_refuses_anything_that_is_not_that_box() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let g = confirm_grid(2).unwrap();
    let paint = |fb: &mut Fb, x0: i32, y0: i32, w: i32, h: i32| {
        for dy in y0..y0 + h {
            for dx in x0..x0 + w {
                fb.set(ox + dx, oy + dy, 0xff00_0000 | CONFIRM3_SEL_INK);
            }
        }
    };
    // ⛔ BOTH cells filled: the game never does that, and "which one is selected" has no
    // answer — so neither does this.
    let mut both = Fb::new(496, 240);
    paint(&mut both, g.x0, g.y0, g.w, g.h);
    paint(&mut both, g.x0 + g.pitch_x, g.y0, g.w, g.h);
    assert_eq!(confirm_cell(&both.px, 496, 240), None, "two filled cells -> no answer");
    // ⛔ The right size in the wrong place — a selected list row elsewhere on the panel.
    let mut moved = Fb::new(496, 240);
    paint(&mut moved, g.x0, g.y0 + 20, g.w, g.h);
    assert_eq!(confirm_cell(&moved.px, 496, 240), None, "off the measured row");
    let mut shifted = Fb::new(496, 240);
    paint(&mut shifted, g.x0 + 6, g.y0, g.w, g.h);
    assert_eq!(confirm_cell(&shifted.px, 496, 240), None, "off the measured column");
    // ⛔ An OUTLINE of the right box is not a filled cell (the 79차-C popup chrome).
    let mut hollow = Fb::new(496, 240);
    for dx in g.x0..g.x0 + g.w {
        hollow.set(ox + dx, oy + g.y0, 0xff00_0000 | CONFIRM3_SEL_INK);
        hollow.set(ox + dx, oy + g.y0 + g.h - 1, 0xff00_0000 | CONFIRM3_SEL_INK);
    }
    assert_eq!(confirm_cell(&hollow.px, 496, 240), None, "an outline is not a fill");
    assert_eq!(confirm_cell(&Fb::new(496, 240).px, 496, 240), None);
    assert_eq!(confirm_cell(&[], 496, 240), None);
    assert_eq!(confirm_cell(&Fb::new(496, 240).px, 0, 0), None);
}

/// The three recognisers must not fight: each screen is claimed by exactly one, and the
/// widget one is tried LAST because its cell is 52 wide where the modal boxes are 40.
#[test]
fn the_three_confirm_recognisers_stay_out_of_each_others_way() {
    let mut widget = Fb::new(496, 240);
    widget_confirm(&mut widget, true);
    assert!(confirm_pair_widget(&widget.px, 496, 240).is_some());
    let mut shell = Fb::new(496, 240);
    shell_confirm(&mut shell, true);
    assert_eq!(confirm_cell(&shell.px, 496, 240), None, "the 40x16 modal box is not a widget cell");
    assert!(confirm_pair(&shell.px, 496, 240).is_some(), "…and it is still the shell's");
}

/// Resolution independence, like every other detector here.
#[test]
fn the_widget_confirm_travels_with_the_centred_panel() {
    let mut a = Fb::new(496, 240);
    widget_confirm(&mut a, true);
    let mut b = Fb::new(560, 228);
    widget_confirm(&mut b, true);
    assert_eq!(confirm_cell(&a.px, 496, 240), confirm_cell(&b.px, 560, 228));
    assert_eq!(confirm_pair(&a.px, 496, 240), confirm_pair(&b.px, 560, 228));
}

#[test]
fn a_lone_confirm_ink_blob_is_not_a_confirm_window() {
    // Fail closed: one button without its partner, and a partner of the wrong size.
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let mut only_sel = Fb::new(496, 240);
    for dy in 91..=106 {
        for dx in 25..65 {
            only_sel.set(ox + dx, oy + dy, CONFIRM3_SEL_INK);
        }
    }
    assert!(confirm_pair(&only_sel.px, only_sel.lw, only_sel.lh).is_none(), "no partner");

    // ⛔ And the ACTION POPUP must not read as a confirm: its selected row uses the same
    // 0x4a3121 (79차-C measured the same palette) but is 62 px wide, outside 30..=50.
    let mut popup = Fb::new(496, 240);
    for dy in 91..=106 {
        for dx in 25..25 + 62 {
            popup.set(ox + dx, oy + dy, CONFIRM3_SEL_INK);
        }
        for dx in 111..111 + 40 {
            popup.set(ox + dx, oy + dy, CONFIRM3_OUT_INK);
        }
    }
    assert!(confirm_pair(&popup.px, popup.lw, popup.lh).is_none(), "62px row is not a button");
}

// ── the shell SLOT list (80차-C) ─────────────────────────────────────────────
//
// Player: *"네트워크창고와 게임시작에서 캐릭터 터치로 포커스 이동하는거 되어야된다"*.
// Both lists are the SAME widget, measured byte-identical on the device (S9, vc247):
// rows at dx 32..144, tops dy 22/39/56, the unselected ones a solid 0xcecac5 and the
// selected one a four-shade dither that no single ink can match.

/// Paint the SLOT list with the cursor on `cursor`.
fn slot_list(fb: &mut Fb, cursor: usize) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    // The measured dither of the selected row — four shades in a 2x2 checker.
    const DITHER: [u32; 4] = [0x004a_3d31, 0x004a_3a2b, 0x004a_3929, 0x004a_3c2f];
    for i in 0..SLOT_ROWS {
        let top = SLOT_ROW_TOP + SLOT_ROW_PITCH * i as i32;
        for dy in top..top + SLOT_ROW_H {
            for dx in SLOT_ROW_X0..SLOT_ROW_X0 + SLOT_ROW_W {
                let v = if i == cursor {
                    DITHER[((dx & 1) + 2 * (dy & 1)) as usize]
                } else {
                    SLOT_ROW_LIGHT
                };
                fb.set(ox + dx, oy + dy, v);
            }
        }
    }
}

#[test]
fn the_slot_list_reports_its_rows_and_which_one_the_cursor_is_on() {
    for cursor in 0..SLOT_ROWS {
        let mut fb = Fb::new(496, 240);
        slot_list(&mut fb, cursor);
        let (found, rows) = slot_rows(&fb.px, fb.lw, fb.lh).expect("slot list not found");
        assert_eq!(found, cursor, "the dithered row is the cursor");
        for (i, r) in rows.iter().enumerate() {
            assert_eq!(r.dy, SLOT_ROW_TOP + SLOT_ROW_PITCH * i as i32);
            // A tap in the middle of each row has to land in exactly that row.
            let mid = r.dy + SLOT_ROW_H / 2;
            assert!(r.contains(SLOT_ROW_X0 + 10, mid));
            assert_eq!(rows.iter().filter(|o| o.contains(SLOT_ROW_X0 + 10, mid)).count(), 1);
        }
    }
}

#[test]
fn a_slot_list_needs_exactly_one_cursor_row() {
    // ⛔ Fail closed. Nothing painted, everything light, or two dark rows are all "not
    // this screen" — the shell draws this list mid-repaint too.
    let blank = Fb::new(496, 240);
    assert!(slot_rows(&blank.px, blank.lw, blank.lh).is_none(), "a blank panel is not a list");

    let (ox, oy) = panel_origin(496, 240).unwrap();
    let mut all_light = Fb::new(496, 240);
    for i in 0..SLOT_ROWS {
        let top = SLOT_ROW_TOP + SLOT_ROW_PITCH * i as i32;
        for dy in top..top + SLOT_ROW_H {
            for dx in SLOT_ROW_X0..SLOT_ROW_X0 + SLOT_ROW_W {
                all_light.set(ox + dx, oy + dy, SLOT_ROW_LIGHT);
            }
        }
    }
    assert!(slot_rows(&all_light.px, all_light.lw, all_light.lh).is_none(), "no cursor");

    let mut two_dark = Fb::new(496, 240);
    slot_list(&mut two_dark, 0);
    let top = SLOT_ROW_TOP + SLOT_ROW_PITCH; // blank out row 1 as well
    for dy in top..top + SLOT_ROW_H {
        for dx in SLOT_ROW_X0..SLOT_ROW_X0 + SLOT_ROW_W {
            two_dark.set(ox + dx, oy + dy, 0x0011_2233);
        }
    }
    assert!(slot_rows(&two_dark.px, two_dark.lw, two_dark.lh).is_none(), "two cursors");
}

// --------------------------------------------------------------------------
// ★★★ 80차-P — REAL FRAMES AS FIXTURES. Everything above is synthetic paint, which pins the
// numbers a session measured but cannot catch the next session re-deriving them. These two
// strips are the guest's own framebuffer (`wie_probe --dump`, 496x240, panel-relative rows
// 146..164, run-length encoded `rrggbb:n`): the game-start slot list with an EMPTY slot
// (`OK : 캐릭터생성` + `# : 복구`) and the title's OPTION window (`CLR : 메인메뉴` alone).
// Regenerate with the PIL snippet in `PLAY_FEEDBACK_FIXES.md §80차-P` if the game's footer
// ever changes; do not hand-edit.
// --------------------------------------------------------------------------

const REAL_SLOT_LIST_FOOTER: (i32, &str) = (
    146,
    "ffffff:1 311408:9 d68e63:2 311408:6 d68e63:7 311408:1 d68e63:2 311408:3 d68e63:2 311408:4 \n     d68e63:2 311408:10 d68e63:2 311408:4 d68e63:6 311408:2 d68e63:2 311408:3 d68e63:2 311408:1 \n     d68e63:7 311408:48 a59a84:5 311408:11 a59a84:5 311408:11 a59a84:5 311408:12 ffffff:1 ffffff:1 \n     311408:174 ffffff:1 ffffff:175 080000:1 080000:4 100000:1 080000:3 100000:4 080000:1 100000:7 \n     190800:2 100000:6 190800:1 100000:1 190800:2 100000:2 190800:3 100000:1 190800:3 210800:8 \n     291000:6 311800:3 211000:1 291000:2 211000:1 291000:1 211000:2 190800:4 211000:2 190800:1 \n     211000:1 291000:3 190800:1 211000:1 311800:1 211000:2 291000:6 190800:2 291000:1 190800:1 \n     291000:1 311800:1 291000:4 211000:2 311800:1 211000:1 190800:1 291000:3 211000:1 190800:1 \n     211000:2 190800:4 211000:2 291000:1 211000:1 291000:2 211000:1 311800:4 291000:6 210800:8 \n     190800:3 100000:1 190800:3 100000:2 190800:2 100000:1 190800:1 100000:6 190800:2 100000:7 \n     080000:1 100000:4 080000:3 100000:1 080000:1 080000:3 634931:1 735942:2 523921:1 080000:1 \n     100000:7 080000:2 100000:7 190800:3 100000:1 190800:3 de9a19:5 190800:1 de9a19:1 190800:1 \n     de9a19:1 190800:1 210800:1 de9a19:5 190800:2 291000:1 de9a19:1 291000:1 210800:1 de9a19:5 \n     211000:3 de9a19:1 291000:1 211000:1 190800:2 de9a19:1 211000:1 190800:2 de9a19:1 211000:1 \n     de9a19:1 211000:4 de9a19:1 190800:2 211000:1 291000:2 de9a19:1 211000:1 311808:8 211000:1 \n     291000:3 211000:1 190800:2 211000:8 190800:2 211000:1 190800:3 211000:1 291000:2 211000:3 \n     291000:5 210800:2 291000:3 190800:2 210800:1 190800:1 210800:2 190800:1 210800:1 190800:7 \n     100000:1 190800:5 100000:1 190800:3 100000:7 080000:2 100000:7 080000:2 100000:1 080000:2 \n     080000:1 634931:1 decaa5:2 ceb28c:1 decaa5:1 f7e3b5:1 ad9a73:1 080000:1 100000:6 080000:2 \n     100000:1 080000:1 100000:6 190800:2 100000:1 190800:7 de9a19:1 190800:1 de9a19:1 190800:1 \n     de9a19:1 190800:3 210800:2 190800:1 de9a19:1 190800:1 080000:1 190800:1 de9a19:1 291000:1 \n     210800:1 de9a19:1 210800:1 190800:1 211000:4 190800:1 de9a19:1 211000:3 190800:1 de9a19:1 \n     211000:3 de9a19:1 211000:1 de9a19:1 190800:2 291000:2 de9a19:1 211000:1 291000:4 de9a19:1 \n     291000:1 311808:8 291000:6 211000:2 291000:2 190800:3 211000:5 190800:2 211000:4 190800:1 \n     211000:4 190800:1 210800:4 291000:2 190800:1 080000:1 190800:1 210800:1 190800:1 210800:2 \n     190800:15 100000:1 190800:2 100000:1 de9a19:1 100000:4 080000:1 de9a19:1 080000:2 100000:2 \n     de9a19:7 100000:2 080000:1 5a4129:1 efdbad:1 8c7152:1 080000:3 9c8263:1 f7e3b5:1 9c8263:1 \n     080000:1 ad9a73:1 ceb28c:1 080000:1 100000:1 5a4129:1 735942:1 100000:11 190800:1 100000:1 \n     190800:6 de9a19:1 190800:1 de9a19:1 190800:1 de9a19:1 190800:2 de9a19:5 190800:1 100000:1 \n     190800:1 de9a19:1 210800:2 de9a19:1 210800:1 190800:6 de9a19:1 211000:4 de9a19:1 190800:2 \n     211000:1 de9a19:3 190800:2 211000:2 de9a19:1 211000:2 190800:1 de9a19:3 291000:1 311808:2 \n     291000:1 311808:2 291000:1 311808:2 291000:1 211000:1 291000:2 190800:1 211000:2 291000:1 \n     211000:2 190800:2 211000:4 190800:2 211000:5 190800:7 210800:6 190800:1 100000:1 190800:1 \n     210800:1 100000:1 190800:1 210800:2 190800:13 100000:1 190800:1 100000:3 de9a19:7 100000:1 \n     080000:2 100000:1 080000:1 100000:1 080000:2 100000:1 080000:1 de9a19:1 080000:3 ad9a73:2 \n     080000:5 ad9a73:1 efdbad:1 100000:1 735942:1 f7e3b5:1 080000:1 523921:1 d6c29c:1 9c8263:1 \n     080000:1 100000:7 de9a19:2 100000:3 190800:2 de9a19:5 190800:1 de9a19:1 190800:1 de9a19:1 \n     190800:2 de9a19:1 190800:6 210800:1 de9a19:1 210800:2 de9a19:1 190800:5 de9a19:3 190800:3 \n     de9a19:1 190800:1 de9a19:1 190800:2 de9a19:1 190800:1 de9a19:1 211000:3 de9a19:1 211000:1 \n     de9a19:1 100800:2 211000:1 190800:1 de9a19:1 311808:1 291000:2 211000:4 291000:2 311808:1 \n     291000:1 190800:1 211000:1 100800:2 190800:1 211000:5 190800:13 100800:1 190800:7 210800:4 \n     190800:8 ffffff:2 190800:1 ffffff:2 190800:7 100000:4 190800:1 de9a19:1 100000:5 de9a19:1 \n     080000:3 100000:1 080000:1 100000:1 080000:1 100000:1 080000:2 de9a19:1 080000:3 ceb28c:1 \n     9c8263:1 080000:5 735942:1 f7e3b5:1 080000:1 735942:1 f7e3b5:1 735942:1 decaa5:1 d6c29c:1 \n     523921:1 080000:2 100000:4 080000:2 100000:4 190800:1 100000:5 190800:1 de9a19:1 190800:1 \n     de9a19:3 190800:2 de9a19:7 190800:1 de9a19:1 190800:2 de9a19:4 100800:4 de9a19:1 190800:2 \n     de9a19:1 190800:3 de9a19:1 190800:1 de9a19:1 190800:1 de9a19:1 190800:2 de9a19:1 190800:2 \n     211000:1 de9a19:1 190800:1 211000:2 de9a19:1 291000:2 211000:1 190800:1 211000:2 190800:1 \n     211000:1 291000:3 211000:2 190800:1 100800:1 211000:1 190800:16 100800:7 190800:7 100000:2 \n     190800:3 100000:2 190800:1 ffffff:2 190800:1 ffffff:2 190800:2 100000:2 ffffff:2 100000:1 \n     190800:1 100000:4 de9a19:1 080000:1 100000:1 de9a19:1 100000:2 de9a19:1 080000:8 100000:1 \n     080000:1 de9a19:1 080000:3 ceb28c:1 9c8263:1 080000:3 100000:1 080000:1 523921:1 f7e3b5:1 \n     080000:1 735942:1 f7e3b5:2 ad9a73:1 523921:1 100000:1 080000:1 100000:6 080000:2 100000:5 \n     190800:1 100000:3 de9a19:1 100000:1 190800:1 de9a19:1 190800:1 de9a19:1 190800:2 100000:2 \n     190800:6 de9a19:1 190800:2 de9a19:1 190800:1 100800:1 190800:1 100800:4 de9a19:1 100800:2 \n     190800:6 de9a19:1 190800:1 de9a19:1 190800:2 100800:2 190800:5 211000:1 de9a19:1 190800:2 \n     211000:6 190800:2 211000:2 190800:5 100800:2 190800:11 100800:7 190800:1 100800:1 190800:12 \n     100000:2 ffffff:7 190800:1 100000:3 190800:1 100000:5 de9a19:9 100000:1 080000:3 100000:1 \n     080000:3 de9a19:1 080000:1 100000:1 080000:2 ceb28c:1 ad9a73:1 080000:5 634931:1 f7e3b5:1 \n     080000:1 735942:1 f7e3b5:2 ad9a73:1 080000:5 100000:5 de9a19:2 100000:1 080000:2 100000:5 \n     de9a19:1 100000:2 de9a19:1 190800:1 de9a19:1 190800:3 100000:2 190800:8 de9a19:1 190800:1 \n     100800:6 de9a19:1 100800:2 190800:2 de9a19:6 100800:2 190800:1 100800:2 de9a19:6 190800:4 \n     211000:1 190800:3 100000:2 190800:3 100000:2 190800:3 100800:1 100000:4 100800:1 190800:1 \n     100800:2 100000:6 100800:3 100000:3 100800:1 190800:1 100000:1 190800:5 100000:1 190800:2 \n     100000:3 190800:2 ffffff:2 100000:1 ffffff:2 080000:13 100000:4 080000:16 9c8263:1 efdbad:1 \n     523921:1 000000:4 9c8263:1 ad9a73:1 000000:1 735942:1 f7e3b5:1 8c7152:1 f7e3b5:1 ad9a73:1 \n     000000:18 de9a19:1 000000:3 de9a19:1 000000:1 de9a19:1 000000:3 de9a19:8 080000:2 de9a19:1 \n     080000:2 000000:1 080000:2 000000:2 de9a19:1 080000:3 de9a19:1 000000:5 080000:1 de9a19:1 \n     080000:2 000000:1 de9a19:1 000000:2 080000:4 de9a19:1 000000:2 080000:2 000000:2 080000:1 \n     000000:3 080000:1 000000:5 080000:2 000000:3 080000:1 000000:12 080000:2 000000:2 080000:2 \n     000000:1 080000:2 000000:1 080000:8 000000:1 ffffff:7 000000:3 ffffff:2 000000:6 de9a19:7 \n     000000:3 de9a19:9 000000:2 523921:1 efdbad:1 d6c29c:1 5a4129:1 000000:2 9c8263:1 decaa5:1 \n     5a4129:1 000000:1 7b614a:1 f7e3b5:1 000000:1 7b614a:1 efdbad:1 d6c29c:1 735942:1 000000:14 \n     de9a19:2 000000:4 de9a19:1 000000:1 de9a19:1 000000:1 080000:2 000000:2 080000:5 de9a19:1 \n     080000:2 de9a19:7 000000:1 de9a19:1 080000:1 000000:2 de9a19:1 000000:6 de9a19:1 000000:2 \n     080000:1 de9a19:1 000000:2 080000:2 000000:2 de9a19:1 000000:2 080000:1 000000:3 080000:1 \n     000000:27 080000:3 000000:1 080000:13 000000:1 080000:2 ffffff:2 000000:1 ffffff:2 000000:18 \n     de9a19:1 000000:7 de9a19:1 000000:6 000000:1 523921:1 ad9a73:1 decaa5:1 d6c29c:2 ad9a73:1 \n     523921:1 000000:2 8c7152:1 9c8263:1 000000:2 523921:1 ad9a73:1 efdbad:1 ceb28c:1 8c7152:1 \n     735942:1 8c7152:1 ad9a73:1 000000:15 de9a19:1 000000:1 de9a19:1 000000:2 080000:3 000000:1 \n     080000:4 de9a19:1 080000:8 000000:1 080000:1 de9a19:1 080000:1 000000:3 de9a19:6 000000:5 \n     de9a19:6 000000:7 080000:1 000000:11 080000:3 000000:19 080000:12 000000:1 080000:1 ffffff:2 \n     080000:1 ffffff:2 000000:18 de9a19:1 000000:7 de9a19:1 000000:6 000000:17 735942:1 9c8263:3 \n     523921:1 000000:18 080000:4 000000:1 080000:12 000000:19 080000:3 000000:25 080000:2 000000:4 \n     080000:1 000000:16 080000:6 000000:2 080000:2 000000:1 080000:2 000000:21 de9a19:1 000000:7 \n     de9a19:1 000000:6 000000:41 080000:2 000000:1 080000:2 000000:2 080000:6 000000:16 080000:1 \n     000000:4 080000:2 000000:30 080000:1 000000:28 080000:1 000000:39 000000:44 080000:1 000000:28 \n     080000:1 000000:102 000000:176",
);
const REAL_TITLE_OPTION_FOOTER: (i32, &str) = (
    146,
    "ffffff:1 948a7b:1 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:3 ffffff:1 ffffff:1 \n     9c968c:1 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 \n     9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:2 9c968c:2 948a7b:1 ffffff:1 ffffff:175 \n     948a7b:1 080000:4 100000:1 080000:3 100000:4 080000:1 100000:7 190800:2 100000:6 190800:1 \n     100000:1 190800:2 100000:2 190800:3 100000:1 190800:3 210800:8 291000:6 311800:3 211000:1 \n     291000:2 211000:1 291000:1 211000:2 190800:4 211000:2 190800:1 211000:1 291000:3 190800:1 \n     211000:1 311800:1 211000:2 291000:6 190800:2 291000:1 190800:1 291000:1 311800:1 291000:4 \n     211000:2 311800:1 211000:1 190800:1 291000:3 211000:1 190800:1 211000:2 190800:4 211000:2 \n     291000:1 211000:1 291000:2 211000:1 311800:4 291000:6 210800:8 190800:3 100000:1 190800:3 \n     100000:2 190800:2 100000:1 190800:1 100000:6 190800:2 100000:7 080000:1 100000:4 080000:3 \n     100000:1 080000:1 080000:5 100000:1 080000:2 100000:7 080000:2 100000:7 190800:3 100000:1 \n     190800:5 100000:1 190800:7 210800:1 190800:1 210800:2 190800:1 210800:1 190800:2 291000:3 \n     210800:2 291000:1 523921:1 9c8263:1 7b614a:1 211000:3 291000:2 211000:1 190800:3 211000:1 \n     190800:2 211000:8 190800:2 211000:1 291000:3 211000:1 311808:3 ffffff:5 211000:1 ffffff:1 \n     291000:1 ffffff:1 211000:1 190800:2 ffffff:3 211000:4 ffffff:1 190800:2 ffffff:5 291000:1 \n     ffffff:1 211000:1 ffffff:1 211000:1 291000:2 ffffff:1 291000:2 210800:2 291000:3 190800:2 \n     210800:1 190800:1 210800:2 190800:1 210800:1 190800:7 100000:1 190800:5 100000:1 190800:3 \n     100000:7 080000:2 100000:7 080000:2 100000:1 080000:2 080000:4 100000:4 080000:1 100000:6 \n     080000:2 100000:1 080000:1 100000:6 190800:2 100000:1 190800:15 210800:2 190800:1 210800:1 \n     523921:1 8c7152:1 9c8263:2 735942:1 210800:3 8c7152:1 bdaa84:1 ceb28c:1 211000:2 190800:1 \n     211000:4 190800:2 211000:5 190800:3 291000:2 211000:2 291000:6 311808:3 ffffff:1 311808:3 \n     ffffff:1 291000:1 ffffff:1 291000:1 ffffff:1 291000:2 ffffff:1 211000:1 291000:2 ffffff:1 \n     190800:2 211000:1 ffffff:1 211000:2 ffffff:1 190800:2 211000:1 ffffff:1 211000:1 ffffff:1 \n     190800:1 ffffff:1 211000:3 ffffff:1 210800:4 291000:2 190800:1 080000:1 190800:1 210800:1 \n     190800:1 210800:2 190800:15 100000:1 190800:2 100000:6 080000:1 100000:1 080000:2 100000:6 \n     080000:1 100000:4 080000:1 080000:6 100000:1 080000:1 100000:1 080000:2 100000:1 080000:1 \n     100000:1 080000:2 100000:11 190800:1 100000:1 190800:13 210800:2 190800:1 100000:1 5a4129:1 \n     decaa5:1 ceb28c:1 8c7152:1 ad9a73:1 efdbad:1 9c8263:1 210800:2 190800:1 9c8263:1 ceb28c:1 \n     190800:4 211000:5 ad9a73:1 ceb28c:1 9c8263:1 bdaa84:1 ceb28c:1 7b614a:1 190800:2 211000:2 \n     291000:1 211000:2 190800:1 291000:2 211000:1 291000:1 311808:2 291000:1 ffffff:1 311808:1 \n     291000:1 311808:1 ffffff:1 291000:1 ffffff:1 291000:1 ffffff:1 190800:1 211000:1 ffffff:1 \n     291000:1 211000:2 ffffff:1 190800:1 211000:2 ffffff:1 211000:1 190800:1 ffffff:1 211000:3 \n     ffffff:1 211000:1 ffffff:1 190800:1 ffffff:1 190800:3 ffffff:1 210800:6 190800:1 100000:1 \n     190800:1 210800:1 100000:1 190800:1 210800:2 190800:13 100000:1 190800:1 100000:11 080000:2 \n     100000:1 080000:1 100000:1 080000:2 100000:1 080000:1 100000:1 080000:3 080000:9 100000:1 \n     080000:1 100000:1 080000:1 100000:1 080000:3 100000:7 190800:1 100000:4 190800:17 bdaa84:1 \n     ceb28c:1 190800:1 210800:2 bdaa84:1 634931:1 190800:3 735942:1 ceb28c:1 190800:1 100800:1 \n     190800:7 735942:1 f7e3b5:1 190800:2 7b614a:1 efdbad:1 211000:5 190800:1 100800:2 ffffff:2 \n     291000:1 311808:1 291000:2 211000:1 ffffff:1 211000:2 291000:1 ffffff:3 190800:1 ffffff:1 \n     100800:2 ffffff:1 211000:3 ffffff:1 211000:1 190800:2 ffffff:1 190800:2 ffffff:1 190800:3 \n     ffffff:3 190800:1 ffffff:1 190800:3 ffffff:7 210800:1 190800:20 100000:4 190800:1 100000:7 \n     080000:3 100000:1 080000:1 100000:1 080000:1 100000:1 080000:6 080000:8 100000:1 080000:9 \n     100000:4 080000:2 100000:4 190800:1 100000:5 190800:8 100000:2 190800:2 f7e3b5:1 7b614a:1 \n     100000:1 190800:6 100800:1 735942:1 ceb28c:1 100800:4 190800:5 735942:1 f7e3b5:1 190800:2 \n     5a4129:1 f7e3b5:1 190800:5 211000:1 100800:1 190800:1 211000:2 291000:3 211000:1 190800:1 \n     ffffff:1 211000:1 190800:1 211000:1 ffffff:1 291000:1 ffffff:1 211000:1 ffffff:1 190800:1 \n     100800:1 211000:1 ffffff:3 190800:4 ffffff:1 190800:2 ffffff:1 190800:3 ffffff:1 190800:1 \n     ffffff:1 100800:1 ffffff:1 100800:4 190800:7 100000:2 190800:3 100000:2 190800:8 100000:5 \n     190800:1 100000:4 080000:2 100000:4 080000:9 100000:1 080000:5 080000:5 100000:1 080000:5 \n     100000:1 080000:3 100000:1 080000:1 100000:6 080000:2 100000:5 190800:1 100000:3 190800:1 \n     100000:1 190800:6 100000:2 190800:1 735942:1 f7e3b5:1 523921:1 190800:7 100800:1 735942:1 \n     ceb28c:1 100800:6 190800:3 735942:1 f7e3b5:1 9c8263:1 ad9a73:1 decaa5:1 9c8263:1 190800:2 \n     100800:2 190800:5 211000:2 190800:2 211000:2 ffffff:1 211000:3 ffffff:1 190800:1 ffffff:1 \n     211000:1 ffffff:1 190800:4 100800:2 190800:4 ffffff:1 190800:2 ffffff:1 190800:3 ffffff:1 \n     100800:1 ffffff:1 100800:1 ffffff:1 100800:2 190800:1 100800:1 190800:12 100000:2 190800:6 \n     100000:1 190800:1 100000:3 190800:1 100000:5 080000:2 100000:6 080000:1 100000:1 080000:3 \n     100000:1 080000:5 100000:1 080000:2 080000:19 100000:8 080000:2 100000:8 190800:6 100000:2 \n     190800:1 f7e3b5:1 7b614a:1 190800:7 100800:1 735942:1 ceb28c:1 100800:6 190800:2 100800:1 \n     735942:1 f7e3b5:2 d6c29c:1 5a4129:1 100800:2 190800:1 100800:2 190800:3 100800:1 ffffff:2 \n     190800:4 211000:1 ffffff:1 190800:2 100000:1 ffffff:1 190800:1 ffffff:1 190800:1 ffffff:1 \n     100000:1 190800:3 100800:1 100000:4 100800:1 190800:1 100800:2 ffffff:1 100000:3 ffffff:1 \n     100000:1 ffffff:1 100800:1 ffffff:1 100000:2 ffffff:9 100000:1 190800:2 100000:3 190800:2 \n     100000:4 080000:14 100000:4 080000:16 000000:43 080000:2 000000:1 bdaa84:1 ceb28c:1 080000:2 \n     000000:1 080000:1 7b614a:1 5a4129:1 080000:2 735942:1 ceb28c:1 080000:1 000000:3 080000:3 \n     000000:2 735942:1 f7e3b5:1 8c7152:1 f7e3b5:1 ad9a73:1 523921:1 080000:2 000000:4 080000:4 \n     000000:3 080000:2 ffffff:1 000000:1 080000:1 000000:1 ffffff:1 000000:1 ffffff:1 000000:1 \n     ffffff:1 000000:3 ffffff:1 080000:1 000000:3 080000:1 000000:4 ffffff:1 000000:3 ffffff:1 \n     000000:1 ffffff:1 000000:1 ffffff:1 080000:1 000000:2 080000:1 ffffff:1 000000:1 080000:2 \n     ffffff:1 080000:8 000000:2 080000:2 000000:36 000000:41 080000:2 000000:2 080000:1 5a4129:1 \n     decaa5:1 ceb28c:1 9c8263:2 ceb28c:1 bdaa84:1 000000:1 080000:2 7b614a:1 ceb28c:1 080000:1 \n     000000:2 080000:2 000000:1 523921:1 5a4129:1 000000:1 7b614a:1 f7e3b5:1 000000:1 7b614a:1 \n     efdbad:1 d6c29c:1 735942:1 000000:1 080000:1 000000:3 080000:2 000000:5 080000:1 000000:1 \n     ffffff:5 000000:1 ffffff:1 000000:1 ffffff:1 000000:3 ffffff:1 000000:9 ffffff:5 000000:1 \n     ffffff:1 000000:1 ffffff:1 080000:2 000000:1 080000:1 ffffff:1 080000:3 ffffff:1 080000:7 \n     000000:1 080000:3 000000:37 000000:42 080000:3 000000:1 080000:1 523921:1 8c7152:1 9c8263:2 \n     634931:1 080000:4 9c8263:1 ceb28c:1 080000:1 000000:1 080000:3 523921:1 d6c29c:1 735942:1 \n     000000:1 8c7152:1 9c8263:1 000000:2 523921:1 ad9a73:1 efdbad:1 ceb28c:1 8c7152:1 735942:1 \n     8c7152:1 ad9a73:1 000000:11 080000:1 000000:3 ffffff:1 000000:1 ffffff:1 000000:3 ffffff:8 \n     000000:8 ffffff:1 000000:1 ffffff:1 000000:4 ffffff:1 080000:3 ffffff:1 080000:8 000000:1 \n     080000:4 000000:35 000000:40 080000:4 000000:1 080000:10 9c8263:1 ceb28c:1 decaa5:1 9c8263:5 \n     decaa5:1 ad9a73:1 000000:9 735942:1 9c8263:3 523921:1 000000:25 080000:2 000000:4 080000:1 \n     000000:16 080000:6 000000:2 080000:2 000000:1 080000:2 000000:36 000000:41 080000:2 000000:1 \n     080000:2 000000:2 080000:6 000000:1 5a4129:1 9c8263:7 ad9a73:1 000000:6 080000:1 000000:4 \n     080000:2 000000:30 080000:1 000000:28 080000:1 000000:39 000000:44 080000:1 000000:28 080000:1 \n     000000:102 000000:176",
);

/// Paint a run-length-encoded real strip at the panel's rows `y0..`.
fn paint_real(fb: &mut Fb, strip: (i32, &str)) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let (y0, s) = strip;
    let (mut dx, mut dy) = (0, y0);
    for tok in s.split_whitespace() {
        let (rgb, n) = tok.split_once(':').unwrap();
        let rgb = u32::from_str_radix(rgb, 16).unwrap() | 0xff00_0000;
        for _ in 0..n.parse::<i32>().unwrap() {
            fb.set(ox + dx, oy + dy, rgb);
            dx += 1;
            if dx == PANEL_W {
                dx = 0;
                dy += 1;
            }
        }
    }
    assert_eq!((dx, dy), (0, y0 + 18), "the strip is 18 full panel rows");
}

/// The real empty-slot footer: `#` at dx 141..147 / dy 153..159 (NOT the device's 155..161
/// — the glyph's row moves with the panel origin's rounding), `OK` sprite with its
/// anti-aliased tail, cream colon, orange 복구.
#[test]
fn the_real_slot_list_footer_yields_hash_and_ok_and_nothing_else() {
    let mut fb = Fb::new(496, 240);
    paint_real(&mut fb, REAL_SLOT_LIST_FOOTER);
    let h = hash_prompt(&fb.px, 496, 240).expect("`# : 복구` is drawn");
    assert_eq!((h.dx, h.w), (134, 42), "the `#` prompt's run, padded by {FOOTER_PAD}");
    let toks = footer_token_prompts(&fb.px, 496, 240);
    assert_eq!(toks.len(), 1, "one identified token: {toks:?}");
    assert_eq!(toks[0].0, FooterToken::Ok);
    let o = toks[0].1;
    assert_eq!((o.dx, o.w), (0, 87), "`OK : 캐릭터생성` only");
    assert!(o.dx + o.w <= h.dx, "the two prompts must not overlap");
    assert_eq!(ok_token_prompt(&fb.px, 496, 240), Some(o));
    // the strip detector clips itself to the left of the `#` run
    let strip = ok_prompt(&fb.px, 496, 240).expect("orange 복구 lights the strip detector");
    assert_eq!(strip.dx + strip.w, h.dx, "clipped exactly at the `#` prompt");
    // and nothing else on the panel claims this frame
    assert_eq!(title_carousel(&fb.px, 496, 240), None);
    assert_eq!(option_rows(&fb.px, 496, 240), None);
    assert_eq!(pager_header(&fb.px, 496, 240), None);
}

/// ★★★ The real title-OPTION footer: `CLR : 메인메뉴` ALONE, cream sprite, its `R` leaving
/// one anti-aliased pixel two columns before the colon. 80차-F's 4-column colon gap could
/// not read it; the shape rule (two dashes) does, and the prefix comes out at 37 — the same
/// 37 the device measured on the online slot list. This is the frame on which a strip
/// detector would have answered 「메인메뉴」 with OK.
#[test]
fn the_real_option_footer_identifies_clr_and_never_ok() {
    let mut fb = Fb::new(496, 240);
    paint_real(&mut fb, REAL_TITLE_OPTION_FOOTER);
    let toks = footer_token_prompts(&fb.px, 496, 240);
    assert_eq!(toks.len(), 1, "one identified token: {toks:?}");
    assert_eq!(toks[0].0, FooterToken::Clr, "the sprite's prefix is 37 px: {toks:?}");
    let c = toks[0].1;
    assert_eq!(c.dx, 46 - FOOTER_PAD, "the `C` starts at dx 46");
    assert!(c.contains(50, 155) && c.contains(100, 155) && c.contains(125, PANEL_H + 5), "sprite, word, overhang");
    assert_eq!(ok_token_prompt(&fb.px, 496, 240), None, "there is no OK on this footer");
    assert_eq!(hash_prompt(&fb.px, 496, 240), None);
    assert_eq!(ok_prompt(&fb.px, 496, 240), None, "no orange action name either");
}

/// The synthetic online footer, re-read through the generalised detector: both tokens.
#[test]
fn the_footer_tokens_split_the_online_footer_into_ok_and_clr() {
    let mut fb = Fb::new(496, 240);
    online_footer(&mut fb);
    let toks = footer_token_prompts(&fb.px, 496, 240);
    assert_eq!(toks.len(), 2, "{toks:?}");
    assert_eq!((toks[0].0, toks[0].1.dx, toks[0].1.dx + toks[0].1.w - 1), (FooterToken::Ok, 0, 50 + FOOTER_PAD));
    assert_eq!((toks[1].0, toks[1].1.dx, toks[1].1.dx + toks[1].1.w - 1), (FooterToken::Clr, 90 - FOOTER_PAD, (173 + FOOTER_PAD).min(PANEL_W - 1)));
    assert!(toks[0].1.dx + toks[0].1.w <= toks[1].1.dx, "words do not overlap");
    // the CLR band is where the measurements are, with room either side
    assert!(FOOTER_CLR_TOKEN_MIN_W > FOOTER_TOKEN_MAX_W, "OK and CLR bands must not overlap");
    assert!((FOOTER_CLR_TOKEN_MIN_W..=FOOTER_CLR_TOKEN_MAX_W).contains(&37), "37 is the measured CLR prefix");
}

/// ★★ 80차-P — the `#` glyph's ROW is not a constant (probe 153..159, device 155..161), and
/// 80차-D's scan stopped at the panel's bottom edge: on the device it saw five of the seven
/// rows, with exactly `HASH_BAR_ROWS_MIN` bars. One more row down and the `#` was lost —
/// and with it 「초기화」, which then fell to the strip detector as OK (review, 2026-09-06).
#[test]
fn the_hash_prompt_survives_the_glyph_moving_down_past_the_panel_edge() {
    let (ox, oy) = panel_origin(496, 240).unwrap();
    let white = 0xffff_ffff;
    let draw = |fb: &mut Fb, top: i32| {
        for dy in top..top + 7 {
            let full = dy == top + 2 || dy == top + 4;
            for i in 0..HASH_W {
                if full || matches!(i, 1 | 2 | 4 | 5) {
                    fb.set(ox + 126 + i, oy + dy, white);
                }
            }
        }
        // its colon, two dashes, to the right
        for dy in [top + 1, top + 2, top + 4, top + 5] {
            fb.set(ox + 136, oy + dy, white);
            fb.set(ox + 137, oy + dy, white);
        }
    };
    for top in [153, 155, 156, 157] {
        let mut fb = Fb::new(496, 240);
        draw(&mut fb, top);
        let h = hash_prompt(&fb.px, 496, 240).unwrap_or_else(|| panic!("`#` with its top at dy {top} must be found"));
        assert_eq!(h.dx, 126 - FOOTER_PAD, "same run whichever row it sits on");
        assert_eq!((h.dy, h.h), (FOOTER_BOX_TOP, FOOTER_BOX_BOT - FOOTER_BOX_TOP), "the hit box is the whole band");
    }
    // …but not arbitrarily far: the scan band ends at FOOTER_SCAN_BOT, above the game's
    // background, where a stray white pixel would break the bitmap test.
    assert_eq!(FOOTER_SCAN_BOT, PANEL_H + 4);
    let mut fb = Fb::new(496, 240);
    draw(&mut fb, PANEL_H + 2);
    assert_eq!(hash_prompt(&fb.px, 496, 240), None, "a glyph mostly below the scan band is not read");

}

/// ★★★ 2026-09-15, device (S9 vc320): the 가방칸 셋's icon is drawn over the cursor's
/// TOP-LEFT corner, so on the warehouse grid the top bar came back nine pixels long
/// instead of fifteen — `cell_box_report` printed
/// `runs 2 | dy23 x20..28 len9 w11 width | dy39 x14..23 len10 w12 width`.
/// The old rule measured the box off the TOP bar and refused it, and `item_grid` goes
/// with `cell_box`, so the whole grid stopped answering taps (player: *"여전히
/// 상세보기, 버리기, 올리기밖에없어"*).
///
/// ⛔ The box is its VERTICALS. Either bar may be cut — the bottom by the stack-count
/// label (80차-E), the top by the item's art — so they only say where to look.
#[test]
fn a_cursor_box_is_found_when_the_art_eats_the_top_bar() {
    let (lw, lh) = (496, 240);
    let (ox, oy) = panel_origin(lw, lh).expect("the panel");
    // The real cell: left 13, right 29, top 23, bottom 39 — the device numbers above.
    let (left, right, top, bot) = (13, 29, 23, 39);
    let mut px = vec![0u32; (lw * lh) as usize];
    let mut put = |dx: i32, dy: i32| px[((oy + dy) * lw + ox + dx) as usize] = 0xff00_0000 | CELL_BOX_INK;
    // Top bar, its left six pixels COVERED by the icon (x14..19 missing).
    for dx in 20..=28 {
        put(dx, top);
    }
    // Bottom bar, shortened on the RIGHT by the count label (the 80차-E shape).
    for dx in 14..=23 {
        put(dx, bot);
    }
    // The verticals, which nothing draws over.
    for dy in top + 1..bot {
        put(left, dy);
        put(right, dy);
    }
    assert_eq!(cell_box(&px, lw, lh), Some((left, top)), "the verticals say where the box is");
    // …and each vertical is counted ON ITS OWN. Device (S9 vc321): `L9 R10 both4/16` —
    // the art eats the left one near the top and the count label the right one near the
    // bottom, so the rows they survive on barely overlap.
    let mut px2 = vec![0u32; (lw * lh) as usize];
    {
        let mut put = |dx: i32, dy: i32| px2[((oy + dy) * lw + ox + dx) as usize] = 0xff00_0000 | CELL_BOX_INK;
        for dx in 20..=28 {
            put(dx, top);
        }
        for dx in 14..=23 {
            put(dx, bot);
        }
        for dy in top + 1..=top + 9 {
            put(left, dy);
        }
        for dy in bot - 10..bot {
            put(right, dy);
        }
    }
    assert_eq!(cell_box(&px2, lw, lh), Some((left, top)), "L9 + R10 with four rows in common IS a box");
    // …and the report names the two bars for what they are.
    let r = cell_box_report(&px, lw, lh);
    assert!(r.contains("runs 2"), "{r}");
}

/// ⛔ The looser pairing must not invent a box out of two unrelated runs: the
/// verticals are the confirmation and nothing else is.
#[test]
fn two_stray_bars_without_verticals_are_not_a_box() {
    let (lw, lh) = (496, 240);
    let (ox, oy) = panel_origin(lw, lh).expect("the panel");
    let mut px = vec![0u32; (lw * lh) as usize];
    let mut put = |dx: i32, dy: i32| px[((oy + dy) * lw + ox + dx) as usize] = 0xff00_0000 | CELL_BOX_INK;
    for dx in 14..=28 {
        put(dx, 23);
        put(dx, 39);
    }
    assert_eq!(cell_box(&px, lw, lh), None, "no verticals, no box");
}

// --------------------------------------------------------------------------
// ★ 150차 — the 상세보기 window (overlay page 7), REAL: 율리케의목걸이's detail window
// (`wie_probe --dump`, detail-window/run2 f1990, 496x240, vc422 asset), panel row 15 and rows
// 56..158 run-length encoded (the recipe is in `itemnote_selftest.rs`'s header — the same
// probe run). The detector reads exactly those rows; everything else below is a mutation of
// this real frame, so a negative can only pass for the reason it names.
// Cross-checked with `menutouch_framecheck.rs` over every probe frame of the 150차 RE runs:
// 31 detail windows (armour · weapon · accessory · orb · card · consumable, inventory and
// equipment-candidate entry) all `true`, 235 other screens all `false`, no other detector
// claiming a detail window.
// --------------------------------------------------------------------------

const REAL_DETAIL_ROW15: &str = "211000:1 ffffff:173 211000:2";
const REAL_DETAIL_ROWS_56_158: &str = "
     211000:1 ffffff:173 211000:3 ffffff:173 7b5952:1 211000:2 ffffff:1 7b5952:171 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:171 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:171 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:3 522d19:166 7b5952:2 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:3 b58263:5 522d19:2
     b58263:1 522d19:3 b58263:5 522d19:3 b58263:1 522d19:7 b58263:5 522d19:3 b58263:1 522d19:4
     b58263:1 522d19:4 b58263:1 522d19:119 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:7 b58263:1 522d19:2 b58263:1 522d19:5 b58263:1 522d19:5 b58263:1 522d19:9 b58263:1
     522d19:5 b58263:1 522d19:1 b58263:7 522d19:1 b58263:1 522d19:119 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:7 b58263:1 522d19:2 b58263:1 522d19:5 b58263:1 522d19:3
     b58263:3 522d19:9 b58263:1 522d19:5 b58263:1 522d19:3 b58263:1 522d19:1 b58263:1 522d19:3
     b58263:2 522d19:15 b58263:5 522d19:3 b58263:5 522d19:90 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:7 b58263:1 522d19:2 b58263:1 522d19:4 b58263:1 522d19:1 b58263:1
     522d19:4 b58263:1 522d19:9 b58263:1 522d19:3 b58263:3 522d19:2 b58263:1 522d19:3 b58263:1
     522d19:2 b58263:1 522d19:9 b58263:2 522d19:4 b58263:1 522d19:4 b58263:2 522d19:1 b58263:1
     522d19:4 b58263:2 522d19:89 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:7
     b58263:1 522d19:2 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:3 b58263:1 522d19:9
     b58263:1 522d19:5 b58263:1 522d19:3 b58263:3 522d19:3 b58263:1 522d19:9 b58263:2 522d19:9
     b58263:2 522d19:6 b58263:2 522d19:89 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:6 b58263:1 522d19:3 b58263:1 522d19:11 b58263:1 522d19:9 b58263:1 522d19:5 b58263:1
     522d19:9 b58263:1 522d19:7 b58263:6 522d19:4 b58263:4 522d19:4 b58263:4 522d19:90 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:6 b58263:1 522d19:3 b58263:1 522d19:4
     b58263:8 522d19:9 b58263:1 522d19:5 b58263:1 522d19:4 b58263:5 522d19:10 b58263:2 522d19:5
     b58263:2 522d19:6 b58263:2 522d19:93 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:5 b58263:1 522d19:4 b58263:1 522d19:11 b58263:1 522d19:8 b58263:1 522d19:1 b58263:1
     522d19:4 b58263:1 522d19:3 b58263:1 522d19:5 b58263:1 522d19:9 b58263:2 522d19:4 b58263:2
     522d19:6 b58263:2 522d19:94 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:3
     b58263:2 522d19:5 b58263:1 522d19:4 b58263:8 522d19:7 b58263:1 522d19:3 b58263:1 522d19:3
     b58263:1 522d19:3 b58263:1 522d19:5 b58263:1 522d19:15 b58263:7 522d19:1 b58263:7 522d19:89
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:10 b58263:1 522d19:4 b58263:1
     522d19:22 b58263:1 522d19:4 b58263:5 522d19:120 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:15 b58263:8 522d19:145 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:4 b58263:7 522d19:7 b58263:1 522d19:7
     b58263:7 522d19:3 b58263:5 522d19:3 b58263:1 522d19:123 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:4 b58263:1 522d19:13 b58263:1 522d19:13 b58263:1 522d19:7 b58263:1
     522d19:1 b58263:3 522d19:123 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:4
     b58263:7 522d19:6 b58263:1 522d19:1 b58263:1 522d19:12 b58263:1 522d19:7 b58263:1 522d19:3
     b58263:1 522d19:15 b58263:5 522d19:3 b58263:5 522d19:2 b58263:2 522d19:3 b58263:1 522d19:87
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:4 b58263:1 522d19:10 b58263:2
     522d19:3 b58263:2 522d19:6 b58263:1 522d19:3 b58263:1 522d19:6 b58263:1 522d19:2 b58263:3
     522d19:8 b58263:2 522d19:4 b58263:1 522d19:4 b58263:2 522d19:1 b58263:2 522d19:3 b58263:2
     522d19:1 b58263:2 522d19:2 b58263:2 522d19:87 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:3 b58263:9 522d19:13 b58263:9 522d19:2 b58263:3 522d19:5 b58263:1 522d19:8
     b58263:2 522d19:9 b58263:2 522d19:1 b58263:2 522d19:2 b58263:3 522d19:4 b58263:2 522d19:88
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:44 b58263:1 522d19:6 b58263:6
     522d19:4 b58263:4 522d19:2 b58263:4 522d19:1 b58263:2 522d19:3 b58263:2 522d19:89 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:4 b58263:7 522d19:3 b58263:9 522d19:4
     b58263:5 522d19:12 b58263:1 522d19:8 b58263:2 522d19:9 b58263:2 522d19:1 b58263:2 522d19:3
     b58263:2 522d19:2 b58263:2 522d19:90 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:10 b58263:1 522d19:7 b58263:1 522d19:7 b58263:1 522d19:5 b58263:1 522d19:4 b58263:8
     522d19:8 b58263:2 522d19:4 b58263:1 522d19:4 b58263:2 522d19:1 b58263:2 522d19:3 b58263:2
     522d19:1 b58263:2 522d19:2 b58263:2 522d19:87 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:10 b58263:1 522d19:7 b58263:1 522d19:7 b58263:1 522d19:5 b58263:1 522d19:11
     b58263:1 522d19:15 b58263:5 522d19:3 b58263:5 522d19:2 b58263:1 522d19:3 b58263:2 522d19:87
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:10 b58263:1 522d19:7 b58263:1
     522d19:8 b58263:5 522d19:12 b58263:1 522d19:123 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:21 b58263:1 522d19:4 b58263:1 522d19:4 b58263:1 522d19:5
     b58263:1 522d19:130 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:18 b58263:7
     522d19:1 b58263:1 522d19:4 b58263:7 522d19:130 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:3 b58263:5 522d19:2 b58263:6 522d19:4 b58263:1 522d19:1 b58263:1 522d19:3
     b58263:1 522d19:4 b58263:1 522d19:5 b58263:1 522d19:17 b58263:4 522d19:3 b58263:5 522d19:2
     b58263:2 522d19:3 b58263:1 522d19:93 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:2 b58263:2 522d19:4 b58263:1 522d19:1 b58263:2 522d19:3 b58263:2 522d19:2 b58263:1
     522d19:3 b58263:1 522d19:2 b58263:1 522d19:4 b58263:1 522d19:2 b58263:1 522d19:2 b58263:1
     522d19:9 b58263:2 522d19:5 b58263:2 522d19:1 b58263:2 522d19:2 b58263:2 522d19:3 b58263:2
     522d19:1 b58263:2 522d19:2 b58263:2 522d19:93 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:2 b58263:2 522d19:6 b58263:2 522d19:3 b58263:2 522d19:2 b58263:1 522d19:3
     b58263:1 522d19:2 b58263:1 522d19:3 b58263:9 522d19:8 b58263:2 522d19:5 b58263:2 522d19:1
     b58263:2 522d19:2 b58263:2 522d19:2 b58263:3 522d19:4 b58263:2 522d19:94 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:3 b58263:5 522d19:2 b58263:6 522d19:4 b58263:3
     522d19:3 b58263:1 522d19:18 b58263:6 522d19:2 b58263:2 522d19:2 b58263:2 522d19:2 b58263:4
     522d19:1 b58263:2 522d19:3 b58263:2 522d19:95 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:7 b58263:2 522d19:1 b58263:2 522d19:9 b58263:1 522d19:4 b58263:1 522d19:4
     b58263:7 522d19:9 b58263:2 522d19:4 b58263:7 522d19:1 b58263:2 522d19:3 b58263:2 522d19:2
     b58263:2 522d19:96 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:2 b58263:1
     522d19:4 b58263:2 522d19:1 b58263:2 522d19:9 b58263:1 522d19:4 b58263:1 522d19:10 b58263:1
     522d19:9 b58263:2 522d19:8 b58263:2 522d19:2 b58263:2 522d19:3 b58263:2 522d19:1 b58263:2
     522d19:2 b58263:2 522d19:93 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:3
     b58263:5 522d19:2 b58263:2 522d19:7 b58263:8 522d19:10 b58263:1 522d19:19 b58263:2 522d19:3
     b58263:5 522d19:2 b58263:1 522d19:3 b58263:2 522d19:93 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:26 b58263:1 522d19:10 b58263:1 522d19:130 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2
     522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1
     7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1
     7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1
     ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2
     ffffff:1 7b5952:2 522d19:168 7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:2 522d19:168
     7b5952:1 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:3 522d19:166 7b5952:2 ffffff:1 7b5952:1
     211000:2 ffffff:1 7b5952:171 ffffff:1 7b5952:1 211000:2 ffffff:1 7b5952:171 ffffff:1 7b5952:1
     211000:2 ffffff:173 7b5952:1 211000:2 7b5952:174 211000:1";

/// Paint `rows` full panel rows of a strip from panel row `dy0`.
fn paint_rows(fb: &mut Fb, dy0: i32, rows: i32, strip: &str) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    let (mut dx, mut dy) = (0, dy0);
    for tok in strip.split_whitespace() {
        let (rgb, n) = tok.split_once(':').unwrap();
        let rgb = u32::from_str_radix(rgb, 16).unwrap() | 0xff00_0000;
        for _ in 0..n.parse::<i32>().unwrap() {
            fb.set(ox + dx, oy + dy, rgb);
            dx += 1;
            if dx == PANEL_W {
                dx = 0;
                dy += 1;
            }
        }
    }
    assert_eq!((dx, dy), (0, dy0 + rows), "the strip is {rows} full panel rows");
}

fn real_detail(lw: i32, lh: i32) -> Fb {
    let mut fb = Fb::new(lw, lh);
    paint_rows(&mut fb, 15, 1, REAL_DETAIL_ROW15);
    paint_rows(&mut fb, 56, 103, REAL_DETAIL_ROWS_56_158);
    fb
}

/// Set a panel-relative pixel (a mutation of the real frame).
fn poke(fb: &mut Fb, dx: i32, dy: i32, rgb: u32) {
    let (ox, oy) = panel_origin(fb.lw, fb.lh).unwrap();
    fb.set(ox + dx, oy + dy, 0xff00_0000 | rgb);
}

#[test]
fn the_real_detail_window_is_the_detail_page() {
    let fb = real_detail(496, 240);
    assert!(detail_page(&fb.px, fb.lw, fb.lh));
    // the panel is centred, so the same window at another logical size is found too
    let other = real_detail(560, 228);
    assert!(detail_page(&other.px, other.lw, other.lh));
    // and no other detector claims it
    assert_eq!(badges(&fb.px, fb.lw, fb.lh).len(), 0);
    assert_eq!(item_grid(&fb.px, fb.lw, fb.lh), None);
    assert_eq!(cell_box(&fb.px, fb.lw, fb.lh), None);
    assert_eq!(slot_rows(&fb.px, fb.lw, fb.lh).map(|r| r.0), None);
    assert_eq!(option_rows(&fb.px, fb.lw, fb.lh).map(|r| r.0), None);
    assert_eq!(confirm_pair(&fb.px, fb.lw, fb.lh), None);
    assert_eq!(ok_prompt(&fb.px, fb.lw, fb.lh), None);
    // the rectangle the overlay redraws: inside the panel, the clear rows inside it
    let b = DETAIL_BODY;
    assert_eq!((b.dx, b.dy, b.w, b.h), (4, 61, 168, 94));
    assert!(b.dx + b.w <= PANEL_W && b.dy + b.h <= PANEL_H);
    assert!(DETAIL_CLEAR_ROWS.iter().all(|&y| b.contains(b.dx, y)));
}

/// Every negative is the real window with ONE thing the page-7 window would never show.
#[test]
fn anything_but_the_detail_window_is_not_the_detail_page() {
    let blank = Fb::new(496, 240);
    assert!(!detail_page(&blank.px, 496, 240), "an empty frame");
    assert!(!detail_page(&blank.px[1..], 496, 240), "a frame of the wrong size");
    // ★ the ONLINE DETAIL screen (싱글가방 · 창고, shell screen 4) draws the same box but ends it
    // at dy 142 (border 143..144, white 145) with the map below — a different window
    let mut online = real_detail(496, 240);
    for dx in 0..PANEL_W {
        for dy in 143..145 {
            poke(&mut online, dx, dy, DETAIL_BORDER_INK);
        }
        poke(&mut online, dx, 145, DETAIL_OUTLINE_INK);
        for dy in 146..159 {
            poke(&mut online, dx, dy, 0x19_0800);
        }
    }
    assert!(!detail_page(&online.px, 496, 240), "the online DETAIL box");
    // a long description running over the frame (the game does not clip it) — not a body the
    // overlay may redraw
    let mut overflow = real_detail(496, 240);
    poke(&mut overflow, 80, 153, 0xb5_8263);
    assert!(!detail_page(&overflow.px, 496, 240), "ink on the last clear row");
    let mut border = real_detail(496, 240);
    poke(&mut border, 90, 155, DETAIL_BODY_INK);
    assert!(!detail_page(&border.px, 496, 240), "one border pixel off");
    let mut side = real_detail(496, 240);
    poke(&mut side, 172, 100, DETAIL_BODY_INK);
    assert!(!detail_page(&side.px, 496, 240), "the right border broken");
    let mut no_top = real_detail(496, 240);
    poke(&mut no_top, 88, 15, 0x21_1000);
    assert!(!detail_page(&no_top.px, 496, 240), "the top box's outline missing");
    // and the real footers of the other windows
    let mut slot = Fb::new(496, 240);
    paint_real(&mut slot, REAL_SLOT_LIST_FOOTER);
    assert!(!detail_page(&slot.px, 496, 240));
    let mut opt = Fb::new(496, 240);
    paint_real(&mut opt, REAL_TITLE_OPTION_FOOTER);
    assert!(!detail_page(&opt.px, 496, 240));
}
