// 아바타 옷장 (167차) — the AVATAR page, drawn. Pure: a [`crate::avatar::View`] and the game's own
// EQUIPMENT panel in, one 176x160 logical-pixel patch out, blitted over the panel with the game
// blit's own mapping (`overlay::blit_logical_rect`, the 상세보기 body's road).
//
// ── WHY A PATCH OVER EQUIPMENT ────────────────────────────────────────────────
// The game never leaves its EQUIPMENT page while the wardrobe is up — the host keeps the «3»
// that would have gone on to SKILL. So the frame under the patch is the game's EQUIPMENT page,
// still painted every frame, and the two pieces of it the wardrobe wants come straight off it:
// the header (the «1» «3» badges and their chevrons) and the FIGURE BOX, which shows the
// paper doll the hooks dress — a try-on is visible the frame the layer setter runs. Everything
// else is drawn here, in the page's own palette (all measured on the EQUIPMENT frame, 167차):
//
// ```text
//   y   0..14  header (game) — the title repainted: AVATAR, the game font, centred like the game
//   y  16..74  checker section: figure box x 4..55 (copied from the game's x 59..110)
//                               list rows x 59..171, y 18 + 14 i, 13 tall (4 rows)
//   y  77..114 brown section:   이름 · 설명 · 획득, boxes x 25..171 at y 78 / 90 / 102
//   y 117..156 dark section:    the hint row (status · page n/m), the slot strip «7» cells «9»
// ```
//
// ⛔ The game's outer frame (the white rules at x 1 / x 173 / rows 15, 75-76, 115-116, 157 and
// its shadow column) is the game's own pixels: the patch starts as a copy of the panel and only
// the INSIDES are repainted.
//
// ── 168차: the MENU list's seven rows ─────────────────────────────────────────────
// [`compose_menu`] redraws the game's six-row MENU list as seven — AVATAR between EQUIPMENT and
// SKILL — in the game's own look, measured on the MENU frames (`wie_probe --dump`, 496x240):
// rows x 24..151, 15 tall, 1 px chamfer; a plain row `#cecac5` with its English label in
// `#292010`, the selected row `#311c10` with its Korean label in white; both labels are the
// GAME FONT (`gamefont::game_layout` reproduces every one pixel for pixel), pen `(88 - w/2,
// top + 2)`. The game's six rows sit at `24 + 19 i`; seven fit the same inside (y 16..142) at
// `21 + 17 i`, 5 px clear at both ends. [`menu_on_screen`] is the gate: the header reads exactly
// MENU and the six game rows are where the game draws them, one of them selected.
//
// Dependency-free apart from `crate::gamefont` and `crate::avatar` — `avatar_selftest.rs`
// `include!`s this file at its crate root too.

use crate::avatar::{Act, View};
use crate::gamefont;

pub const W: usize = 176;
pub const H: usize = 160;

// ── the palette (EQUIPMENT frame, 167차) ────────────────────────────────────────
pub const HDR: u32 = 0x21_1000;
pub const TITLE: u32 = 0xd6_b29c;
pub const BADGE: u32 = 0xd6_b294;
pub const CHEVRON: u32 = 0xb5_8263;
pub const CHECK_A: u32 = 0x94_8a7b;
pub const CHECK_B: u32 = 0x9c_928c;
pub const FRAME_BROWN: u32 = 0x73_4531;
pub const BOX_BROWN: u32 = 0x52_2d19;
pub const LABEL: u32 = 0xb5_8263;
pub const VALUE: u32 = 0xd6_b29c;
pub const DARK: u32 = 0x00_1000;
pub const CELL_LINE: u32 = 0x19_0800;
pub const CELL_FILL: u32 = 0x31_1008;
pub const CURSOR: u32 = 0xff_f300;
pub const ROW_FILL: u32 = 0x4a_3529;
pub const WHITE: u32 = 0xff_ffff;
pub const E_LINE: u32 = 0x00_0000;
pub const E_FILL: u32 = 0xa5_f37b;

// ── geometry (panel px) ─────────────────────────────────────────────────────────
/// The title's pen y and its centre (the game centres the ADVANCE on x 87: STATUS starts at
/// x 63, INVENTORY at 51 — both `87 - advance/2`).
pub const TITLE_Y: i32 = 2;
pub const TITLE_CENTRE: i32 = 87;
/// The header area the game's title occupies (cleared before AVATAR is drawn).
pub const TITLE_CLEAR: (i32, i32, i32, i32) = (28, 1, 148, 13);
/// The checker section's inside.
pub const CHECKER: (i32, i32, i32, i32) = (2, 16, 172, 74);
/// The game's figure box on EQUIPMENT (52x52, 1 px chamfer) and where the page puts it.
pub const FIG_SRC: (i32, i32) = (59, 21);
pub const FIG_DST: (i32, i32) = (4, 21);
pub const FIG_W: i32 = 52;
pub const FIG_H: i32 = 52;
/// The list.
pub const LIST_X0: i32 = 59;
pub const LIST_X1: i32 = 171;
pub const LIST_Y0: i32 = 18;
pub const LIST_PITCH: i32 = 14;
pub const LIST_H: i32 = 13;
pub const NAME_X: i32 = 63;
/// The worn mark (the game's green E, 7x7) on a row.
pub const E_X: i32 = 161;
/// The brown section and its three boxes.
pub const INFO: (i32, i32, i32, i32) = (2, 77, 172, 114);
pub const INFO_BOX_X0: i32 = 25;
pub const INFO_BOX_X1: i32 = 171;
pub const INFO_Y: [i32; 3] = [78, 90, 102];
pub const INFO_H: i32 = 12;
pub const INFO_LABELS: [&str; 3] = ["이름", "설명", "획득"];
pub const LABEL_X: i32 = 3;
pub const VALUE_X: i32 = 28;
/// The dark section: the hint row and the slot strip.
pub const BOTTOM: (i32, i32, i32, i32) = (2, 117, 172, 156);
pub const HINT_Y: i32 = 119;
pub const HINT_X: i32 = 5;
pub const PAGE_RIGHT: i32 = 170;
/// Slot cells (17x17 like the game's equipment cells), six in a row.
pub const CELL_X0: i32 = 32;
pub const CELL_Y: i32 = 135;
pub const CELL: i32 = 17;
pub const CELL_PITCH: i32 = 19;
/// The «7» / «9» plates (11x11) and their chevrons.
pub const PLATE: i32 = 11;
pub const PLATE_PREV_X: i32 = 15;
pub const PLATE_NEXT_X: i32 = 151;
pub const PLATE_Y: i32 = CELL_Y + (CELL - PLATE) / 2;

/// The plate's outline, row by row (bit 10 = the leftmost pixel) — read off the game's own
/// «1» «3» «7» «9» plates, which share it.
const PLATE_ROWS: [u16; 11] = [
    0b011_1111_1100,
    0b111_1111_1111,
    0b111_1111_1111,
    0b111_1111_1111,
    0b111_1111_1111,
    0b111_1111_1111,
    0b111_1111_1111,
    0b111_1111_1111,
    0b111_1111_1111,
    0b011_1111_1110,
    0b011_1111_1100,
];
/// `<<` — two 3-px chevrons, 5 rows, from `plate top + 3`, x from 5 (the game's own).
const CHEVRON_PREV: [&[i32]; 5] = [&[2, 6], &[1, 5], &[0, 4], &[1, 5], &[2, 6]];
pub const CHEVRON_PREV_X: i32 = 5;
/// `>>` — 7 rows with an empty tip row, from `plate top + 2`, x from 164 (the game's own).
const CHEVRON_NEXT: [&[i32]; 7] = [&[0, 4], &[1, 5], &[2, 6], &[], &[2, 6], &[1, 5], &[0, 4]];
pub const CHEVRON_NEXT_X: i32 = 164;
/// The game's green E (the candidate list's "equipped" mark), 7x7: `o` outline · `p` fill.
const E_MARK: [&str; 7] = ["ooooooo", "oopppp.", "opooooo", "oppppoo", "opooooo", "oopppp.", "ooooooo"];

/// One slot icon, as the atlas holds it (`w x h`, `0xAARRGGBB`, alpha 0 = transparent).
#[derive(Clone, Copy)]
pub struct Icon<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a [u32],
}

/// The checker colour at panel `(x, y)` — the section's own 2x2 pattern.
pub fn checker(x: i32, y: i32) -> u32 {
    if (((x - CHECKER.0) >> 1) + ((y - CHECKER.1) >> 1)) & 1 == 0 { CHECK_A } else { CHECK_B }
}

/// The list row box `i` (0..ROWS): `(x0, y0, x1, y1)` inclusive.
pub fn row_box(i: usize) -> (i32, i32, i32, i32) {
    let y = LIST_Y0 + LIST_PITCH * i as i32;
    (LIST_X0, y, LIST_X1, y + LIST_H - 1)
}

/// The slot cell `i` (0..6): `(x0, y0, x1, y1)` inclusive.
pub fn cell_box(i: usize) -> (i32, i32, i32, i32) {
    let x = CELL_X0 + CELL_PITCH * i as i32;
    (x, CELL_Y, x + CELL - 1, CELL_Y + CELL - 1)
}

struct Canvas<'a> {
    px: &'a mut [u32],
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, c: u32) {
        if (0..W as i32).contains(&x) && (0..H as i32).contains(&y) {
            self.px[y as usize * W + x as usize] = 0xff00_0000 | c;
        }
    }

    fn rect(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32), c: u32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.put(x, y, c);
            }
        }
    }

    /// A filled box whose four corner pixels are left alone (the game's 1 px chamfer).
    fn chamfered(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32), c: u32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let corner = (y == y0 || y == y1) && (x == x0 || x == x1);
                if !corner {
                    self.put(x, y, c);
                }
            }
        }
    }

    /// The outline of a chamfered box.
    fn outline(&mut self, (x0, y0, x1, y1): (i32, i32, i32, i32), c: u32) {
        for x in x0 + 1..x1 {
            self.put(x, y0, c);
            self.put(x, y1, c);
        }
        for y in y0 + 1..y1 {
            self.put(x0, y, c);
            self.put(x1, y, c);
        }
    }

    /// `text` with its pen at `(x, y)` (the line's top), the game font's own metrics, cut at
    /// column `right` (exclusive).
    fn text(&mut self, x: i32, y: i32, text: &str, ink: u32, right: i32) {
        let mut pen = x;
        for cell in gamefont::game_layout(text) {
            for (r, bits) in cell.rows.iter().enumerate() {
                if *bits == 0 {
                    continue;
                }
                let yy = y + cell.drop as i32 + r as i32;
                for c in 0..gamefont::KOR_W as i32 {
                    let xx = pen + c;
                    if bits & (1 << (15 - c)) != 0 && xx < right {
                        self.put(xx, yy, ink);
                    }
                }
            }
            pen += cell.advance as i32;
        }
    }

    /// A «N» plate at `(x, y)`: the plate in [`BADGE`], the digit cut out in `cut` — the game
    /// font's own digit, pen at `(x + 2, y)` (the game's «1»: its `..###` row sits at x + 4).
    fn plate(&mut self, x: i32, y: i32, digit: char, cut: u32) {
        for (r, bits) in PLATE_ROWS.iter().enumerate() {
            for c in 0..PLATE {
                if bits & (1 << (PLATE - 1 - c)) != 0 {
                    self.put(x + c, y + r as i32, BADGE);
                }
            }
        }
        let mut s = [0u8; 4];
        self.text(x + 2, y, digit.encode_utf8(&mut s), cut, x + PLATE);
    }

    fn chevrons(&mut self, x: i32, y: i32, rows: &[&[i32]]) {
        for (r, xs) in rows.iter().enumerate() {
            for dx in xs.iter() {
                self.put(x + dx, y + r as i32, CHEVRON);
            }
        }
    }

    fn e_mark(&mut self, x: i32, y: i32) {
        for (r, row) in E_MARK.iter().enumerate() {
            for (c, ch) in row.bytes().enumerate() {
                match ch {
                    b'o' => self.put(x + c as i32, y + r as i32, E_LINE),
                    b'p' => self.put(x + c as i32, y + r as i32, E_FILL),
                    _ => {}
                }
            }
        }
    }

    /// An icon at `(x, y)`; `dim` blends it 60 % toward the cell fill (a slot without a costume).
    fn icon(&mut self, x: i32, y: i32, icon: &Icon, dim: bool) {
        for j in 0..icon.h {
            for i in 0..icon.w {
                let p = icon.px[j * icon.w + i];
                if p >> 24 == 0 {
                    continue;
                }
                let c = if dim { blend(p & 0xff_ffff, CELL_FILL, 40) } else { p & 0xff_ffff };
                self.put(x + i as i32, y + j as i32, c);
            }
        }
    }
}

/// `a` at `pct` % over `b`, per channel.
pub fn blend(a: u32, b: u32, pct: u32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = ((a >> s) & 0xff, (b >> s) & 0xff);
        ((x * pct + y * (100 - pct)) / 100) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Compose the page. `game` = the live frame's panel (`W x H`, the frame's pixel format),
/// `icons[s]` = the slot's icon (`None` draws an empty cell). `out` gets the patch. `false`
/// on a size mismatch — nothing is drawn then.
pub fn compose(game: &[u32], v: &View, icons: &[Option<Icon>; 6], out: &mut [u32]) -> bool {
    if game.len() != W * H || out.len() != W * H {
        return false;
    }
    out.copy_from_slice(game);
    let mut cv = Canvas { px: out };

    // 1) the title
    cv.rect(TITLE_CLEAR, HDR);
    let title = "AVATAR";
    let adv = gamefont::game_width(title) as i32;
    cv.text(TITLE_CENTRE - adv / 2, TITLE_Y, title, TITLE, TITLE_CLEAR.2 + 1);

    // 2) the checker section, the figure, the list
    for y in CHECKER.1..=CHECKER.3 {
        for x in CHECKER.0..=CHECKER.2 {
            cv.put(x, y, checker(x, y));
        }
    }
    for j in 0..FIG_H {
        for i in 0..FIG_W {
            let corner = (j == 0 || j == FIG_H - 1) && (i == 0 || i == FIG_W - 1);
            if corner {
                continue;
            }
            let p = game[((FIG_SRC.1 + j) as usize) * W + (FIG_SRC.0 + i) as usize];
            cv.put(FIG_DST.0 + i, FIG_DST.1 + j, p & 0xff_ffff);
        }
    }
    for (i, row) in v.rows.iter().enumerate().take(crate::avatar::ROWS) {
        let b = row_box(i);
        cv.chamfered(b, ROW_FILL);
        if row.focused {
            cv.outline(b, CURSOR);
        }
        let ink = if row.focused { CURSOR } else { WHITE };
        cv.text(NAME_X, b.1 + 1, &row.name, ink, if row.worn { E_X - 2 } else { b.2 });
        if row.worn {
            cv.e_mark(E_X, b.1 + 3);
        }
    }

    // 3) the brown section
    cv.rect(INFO, FRAME_BROWN);
    let values = [v.name.as_str(), v.desc.as_str(), v.acquire.as_str()];
    for ((y, label), value) in INFO_Y.iter().zip(INFO_LABELS).zip(values) {
        cv.chamfered((INFO_BOX_X0, *y, INFO_BOX_X1, y + INFO_H - 1), BOX_BROWN);
        cv.text(LABEL_X, *y, label, LABEL, INFO_BOX_X0);
        cv.text(VALUE_X, *y, value, VALUE, INFO_BOX_X1);
    }

    // 4) the dark section: hint row, slot strip
    cv.rect(BOTTOM, DARK);
    let page = format!("{}/{}", v.page.0, v.page.1);
    let pw = gamefont::game_width(&page) as i32;
    cv.text(PAGE_RIGHT - pw, HINT_Y, &page, WHITE, PAGE_RIGHT + 1);
    cv.text(HINT_X, HINT_Y, &v.status, WHITE, PAGE_RIGHT - pw - 4);
    cv.chevrons(CHEVRON_PREV_X, PLATE_Y + 3, &CHEVRON_PREV);
    cv.plate(PLATE_PREV_X, PLATE_Y, '7', DARK);
    cv.plate(PLATE_NEXT_X, PLATE_Y, '9', DARK);
    cv.chevrons(CHEVRON_NEXT_X, PLATE_Y + 2, &CHEVRON_NEXT);
    for s in 0..6 {
        let b = cell_box(s);
        cv.chamfered(b, CELL_FILL);
        cv.outline(b, CELL_LINE);
        if v.open[s]
            && let Some(ic) = &icons[s]
        {
            let (ix, iy) = (b.0 + (CELL - ic.w as i32) / 2, b.1 + (CELL - ic.h as i32) / 2);
            cv.icon(ix, iy, ic, !v.dressed[s]);
        }
        if s == v.slot {
            cv.outline(b, CURSOR);
        }
    }
    true
}

/// Is the game's own EQUIPMENT page under the patch? `panel` = the live frame's panel (`W x H`).
///
/// The header's title must read exactly `EQUIPMENT` — the game font's ink, centred the way the
/// game centres titles — and every other pixel of the title area must be the header's fill. The
/// UI asks this AND that the worn grid's yellow cell frame is up (`menutouch::equip_marker`):
/// the title alone also reads true over the candidate list, the frame alone is a 16x16 box that
/// says nothing about which page it sits on.
pub fn equipment_on_screen(panel: &[u32]) -> bool {
    title_on_screen(panel, "EQUIPMENT")
}

/// Does the header read exactly `title` — the game font's ink where the game centres a title,
/// the header's fill everywhere else in [`TITLE_CLEAR`]?
pub fn title_on_screen(panel: &[u32], title: &str) -> bool {
    if panel.len() != W * H {
        return false;
    }
    let (x0, y0, x1, y1) = TITLE_CLEAR;
    let (bw, bh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let mut ink = vec![false; bw * bh];
    let mut pen = TITLE_CENTRE - gamefont::game_width(title) as i32 / 2;
    for cell in gamefont::game_layout(title) {
        for (r, bits) in cell.rows.iter().enumerate() {
            let y = TITLE_Y + cell.drop as i32 + r as i32;
            for c in 0..gamefont::KOR_W as i32 {
                let x = pen + c;
                if bits & (1 << (15 - c)) != 0 && (x0..=x1).contains(&x) && (y0..=y1).contains(&y) {
                    ink[(y - y0) as usize * bw + (x - x0) as usize] = true;
                }
            }
        }
        pen += cell.advance as i32;
    }
    (0..bh).all(|j| {
        (0..bw).all(|i| {
            let p = panel[(y0 as usize + j) * W + x0 as usize + i] & 0xff_ffff;
            p == if ink[j * bw + i] { TITLE } else { HDR }
        })
    })
}

/// What a touch on the page means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A page action.
    Act(Act),
    /// Inside the panel but on nothing that acts — the page owns the panel, so it is eaten.
    Swallow,
}

/// The panel point `(dx, dy)` → what it does. `None` = outside the panel (not the page's).
/// The header's «1» / «3» badges are the page's too (they leave it, like the keys).
pub fn hit(dx: i32, dy: i32) -> Option<Hit> {
    if !(0..W as i32).contains(&dx) || !(0..H as i32).contains(&dy) {
        return None;
    }
    let inside = |(x0, y0, x1, y1): (i32, i32, i32, i32)| (x0..=x1).contains(&dx) && (y0..=y1).contains(&dy);
    if dy <= 14 {
        if dx < 30 {
            return Some(Hit::Act(Act::Tab1));
        }
        if dx > 146 {
            return Some(Hit::Act(Act::Tab3));
        }
        return Some(Hit::Swallow);
    }
    for i in 0..crate::avatar::ROWS {
        if inside(row_box(i)) {
            return Some(Hit::Act(Act::Row(i)));
        }
    }
    for s in 0..6 {
        let (x0, y0, x1, y1) = cell_box(s);
        // A cell answers on its whole column of the strip, gaps included (half a gap each side).
        if (x0 - 1..=x1 + 1).contains(&dx) && (y0 - 4..=y1 + 4).contains(&dy) {
            return Some(Hit::Act(Act::Slot(s)));
        }
    }
    let strip = (CELL_Y - 4..=CELL_Y + CELL + 3).contains(&dy);
    if strip && dx < CELL_X0 - 1 {
        return Some(Hit::Act(Act::PrevSlot));
    }
    if strip && dx > cell_box(5).2 + 1 {
        return Some(Hit::Act(Act::NextSlot));
    }
    Some(Hit::Swallow)
}

// ── the MENU list (168차) ─────────────────────────────────────────────────────────

/// The seven rows: (English label, Korean label) — the game's own six (`InGame_txt` rows 1..6
/// and 7..12) with AVATAR / 아바타 between EQUIPMENT and SKILL.
pub const MENU_LABELS: [(&str, &str); 7] = [
    ("STATUS", "상태보기"),
    ("INVENTORY", "가방"),
    ("EQUIPMENT", "장비"),
    ("AVATAR", "아바타"),
    ("SKILL", "스킬"),
    ("QUEST", "퀘스트"),
    ("SYSTEM", "시스템"),
];
/// The list's inside (the white frame is at x 1 / 173, y 15 / 143).
pub const MENU_INSIDE: (i32, i32, i32, i32) = (2, 16, 172, 142);
pub const MENU_X0: i32 = 24;
pub const MENU_X1: i32 = 151;
pub const MENU_ROW_H: i32 = 15;
/// The game's six rows.
pub const MENU_GAME_Y0: i32 = 24;
pub const MENU_GAME_PITCH: i32 = 19;
/// Ours.
pub const MENU_Y0: i32 = 21;
pub const MENU_PITCH: i32 = 17;
/// A label's pen: `MENU_LABEL_CENTRE - width / 2`, `row top + MENU_LABEL_DY`.
pub const MENU_LABEL_CENTRE: i32 = 88;
pub const MENU_LABEL_DY: i32 = 2;
/// A row answers taps this far left and right of its box (`menutouch::MENU`'s own pad).
pub const MENU_HIT_PAD_X: i32 = 14;
pub const MENU_ROW_FILL: u32 = 0xce_cac5;
pub const MENU_ROW_INK: u32 = 0x29_2010;
pub const MENU_SEL_FILL: u32 = 0x31_1c10;
pub const MENU_SEL_INK: u32 = 0xff_ffff;

/// Row `i` of the seven: `(x0, y0, x1, y1)` inclusive.
pub fn menu_row_box(i: usize) -> (i32, i32, i32, i32) {
    let y = MENU_Y0 + MENU_PITCH * i as i32;
    (MENU_X0, y, MENU_X1, y + MENU_ROW_H - 1)
}

/// The game's own six-row MENU list, idle, under the patch? `Some(row)` = its selected row
/// (0..6). Gated on the header reading exactly MENU and on every game row being a box of the
/// row fill or the selected fill at three points clear of any label, exactly one selected,
/// with the checker between them — a frame that is anything else, or mid-transition, is not
/// drawn over.
pub fn menu_on_screen(panel: &[u32]) -> Option<u8> {
    if !title_on_screen(panel, "MENU") {
        return None;
    }
    let at = |x: i32, y: i32| panel[y as usize * W + x as usize] & 0xff_ffff;
    let mut selected = None;
    for i in 0..6 {
        let top = MENU_GAME_Y0 + MENU_GAME_PITCH * i;
        let fill = at(26, top + 7);
        if fill != MENU_ROW_FILL && fill != MENU_SEL_FILL {
            return None;
        }
        if at(40, top + 3) != fill || at(150, top + 7) != fill || at(26, top + MENU_ROW_H - 2) != fill {
            return None;
        }
        let gap = at(30, top - 2);
        if gap != CHECK_A && gap != CHECK_B {
            return None;
        }
        if fill == MENU_SEL_FILL {
            if selected.is_some() {
                return None;
            }
            selected = Some(i as u8);
        }
    }
    selected
}

/// Compose the seven-row MENU list with row `sel` selected over the game's panel `game`
/// (`W x H`). The header and the footer stay the game's; the inside is repainted. `false` on a
/// size mismatch or a row past the seven — nothing is drawn then.
pub fn compose_menu(game: &[u32], sel: u8, out: &mut [u32]) -> bool {
    if game.len() != W * H || out.len() != W * H || usize::from(sel) >= MENU_LABELS.len() {
        return false;
    }
    out.copy_from_slice(game);
    let mut cv = Canvas { px: out };
    let (x0, y0, x1, y1) = MENU_INSIDE;
    for y in y0..=y1 {
        for x in x0..=x1 {
            cv.put(x, y, checker(x, y));
        }
    }
    for (i, (eng, kor)) in MENU_LABELS.iter().enumerate() {
        let b = menu_row_box(i);
        let on = i == usize::from(sel);
        cv.chamfered(b, if on { MENU_SEL_FILL } else { MENU_ROW_FILL });
        let label = if on { kor } else { eng };
        let x = MENU_LABEL_CENTRE - gamefont::game_width(label) as i32 / 2;
        cv.text(x, b.1 + MENU_LABEL_DY, label, if on { MENU_SEL_INK } else { MENU_ROW_INK }, b.2);
    }
    true
}

/// A touch on the seven-row list at panel `(dx, dy)`: `None` = outside the panel (not the
/// list's), `Some(Some(row))` = that row, `Some(None)` = inside the panel on no row (the list
/// owns the panel — swallowed). The gap between two rows is split between them.
pub fn menu_hit(dx: i32, dy: i32) -> Option<Option<u8>> {
    if !(0..W as i32).contains(&dx) || !(0..H as i32).contains(&dy) {
        return None;
    }
    let (_, y0, _, y1) = MENU_INSIDE;
    if dx < MENU_X0 - MENU_HIT_PAD_X || dx > MENU_X1 + MENU_HIT_PAD_X || !(y0..=y1).contains(&dy) {
        return Some(None);
    }
    let last = MENU_LABELS.len() as i32 - 1;
    Some(Some(((dy - (MENU_Y0 - 1)).max(0) / MENU_PITCH).min(last) as u8))
}
