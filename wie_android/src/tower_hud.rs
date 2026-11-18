// ★ 145차 (2026-09-30) — the 하늘성 BANNER: the floor and the monsters left, at the top
// centre of the field, in the game's own DAMAGE DIGITS with only the colour changed.
//
// User: *"HP SP바 밑에 하늘성 층수와 남은 몬스터 표시되고있던거를 중앙 상단(너무 위쪽에
// 붙이진말고 적당히 10%정도 떨어진 위치정도)에 데미지 폰트에 색상만 다르게 해서 층수와 남은
// 몬스터를 표시해주고싶어"*.
//
// Pure: this file composes the banner as a small INDEXED bitmap in the game's logical
// pixels — the same grid the game's own damage numbers are drawn on — and the overlay
// scales it by the game→window factor and colours the indices. No drawing, no guest
// memory, no crate imports except the generated digit table, so the whole composition is
// tested standalone (`tower_hud_selftest.rs`).
//
// ── THE FONT ─────────────────────────────────────────────────────────────────
// The damage font has DIGITS ONLY (`res/comm/number_bm`: 0-9, a big 20x20 and a small
// 14x13 set; `res/comm/attmsg_bm` adds block/critical/miss and nothing else). The Korean
// words (층 · 보스 · 남은 몬스터 · 클리어 …) come from the game's own font (`gamefont`,
// 16x11 cells) and are dressed the way the digits are: the glyph filled with the digits'
// three shades, row by row, and wrapped in the digits' outline — 1 px, 2 px on the right
// (measured on the digits: right edges 2 px in 279 of 339 runs, left edges 1 px in 257
// of 340). An 11-row glyph plus a 1-px outline above and below is 13 rows, the small
// digit set's exact height, so a word and a small number share one baseline.
//
// ── THREE INKS (147차, 2026-10-01) ───────────────────────────────────────────
// User: *"현재 층 색과, 남은 몬스터에서 숫자부분만 색 다르게 해줘 … xx층, 남은 몬스터,
// x(남은몬스터 숫자) 각각 색상이 달라서 3개의 색상으로 보이게"*. Every piece is TINTED
// into one of three groups ([`Ink`]) before it is composed, and the index carries the
// group: `4g + 1..=3` the dark / mid / light fill of group `g`, `4g + 4` its outline.
// The overlay owns the colours (`overlay::TOWER_HUD_INK`, one row per group).
//
// Indices of an untinted piece (what `digits` and `text` return — group 0): 0
// transparent · 1 dark · 2 mid · 3 light fill · 4 outline (the digit table's).

pub const INK_NONE: u8 = 0;
pub const INK_OUTLINE: u8 = 4;

/// The banner's colour groups — one per thing the user wanted told apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// 「N층」 (and 「보스」): the headline.
    Floor = 0,
    /// 「남은 몬스터」, and the status word that replaces the count (진입 · 클리어 …).
    Label = 1,
    /// The count itself.
    Count = 2,
}

/// How many [`Ink`] groups there are (the overlay's palette has one row per group).
pub const INK_GROUPS: usize = 3;

/// The shade of an index within its group: 1 dark · 2 mid · 3 light · 4 outline (0 = none).
pub fn shade_of(v: u8) -> u8 {
    if v == INK_NONE { INK_NONE } else { (v - 1) % 4 + 1 }
}

/// The [`Ink`] group of an index (`Ink as usize`); 0 for a transparent pixel.
pub fn group_of(v: u8) -> usize {
    if v == INK_NONE { 0 } else { usize::from((v - 1) / 4) }
}

fn is_rim(v: u8) -> bool {
    shade_of(v) == INK_OUTLINE
}

/// A piece recoloured into `ink`'s group: every painted pixel keeps its shade and takes
/// the group. Idempotent, and re-tinting moves a piece from one group to another.
pub fn tint(src: &Bitmap, ink: Ink) -> Bitmap {
    let g = ink as u8;
    Bitmap { w: src.w, h: src.h, px: src.px.iter().map(|&v| if v == INK_NONE { v } else { shade_of(v) + 4 * g }).collect() }
}

/// The rows of a game-font glyph: 11 rows of a 16-bit mask, MSB = leftmost pixel
/// (`gamefont::Glyph`).
pub const GLYPH_H: usize = 11;
pub type GlyphRows = [u16; GLYPH_H];

/// The fill shade of each glyph row: rows 1..=11 of the small digit set (its rows 0 and
/// 12 are outline), the dominant shade per row across the ten digits. Measured on the
/// generated table and re-derived by the selftest, so a regenerated font cannot drift
/// away from it unnoticed.
pub const TEXT_RAMP: [u8; GLYPH_H] = [3, 3, 2, 3, 2, 2, 2, 1, 2, 1, 1];

/// Columns between a number and the word after it, and rows between the two lines — in
/// logical pixels, i.e. the game's own grid.
pub const WORD_GAP: usize = 1;
pub const LINE_GAP: usize = 2;

/// An indexed bitmap in logical pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Bitmap {
    pub fn new(w: usize, h: usize) -> Self {
        Bitmap { w, h, px: vec![INK_NONE; w * h] }
    }

    pub fn get(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h { self.px[y * self.w + x] } else { INK_NONE }
    }

    /// Paint `v` at (x, y). Fill beats outline, outline beats nothing: where two pieces
    /// touch, a glyph's body is never covered by its neighbour's rim (whatever the groups).
    fn put(&mut self, x: usize, y: usize, v: u8) {
        if v == INK_NONE || x >= self.w || y >= self.h {
            return;
        }
        let p = &mut self.px[y * self.w + x];
        if *p == INK_NONE || (is_rim(*p) && !is_rim(v)) {
            *p = v;
        }
    }

    fn blit(&mut self, src: &Bitmap, ox: usize, oy: usize) {
        for y in 0..src.h {
            for x in 0..src.w {
                self.put(ox + x, oy + y, src.get(x, y));
            }
        }
    }

    /// Is any pixel painted? (An empty string draws nothing and takes no room.)
    pub fn is_empty(&self) -> bool {
        self.px.iter().all(|&p| p == INK_NONE)
    }
}

/// Which digit set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// 20x20 — the floor number.
    Big,
    /// 14x13 — the count, the same height as a word.
    Small,
}

fn digit_cell(d: usize, size: Size) -> (usize, usize, &'static [&'static str]) {
    match size {
        Size::Big => (crate::damage_digits::BIG_W, crate::damage_digits::BIG_H, &crate::damage_digits::BIG[d]),
        Size::Small => (crate::damage_digits::SMALL_W, crate::damage_digits::SMALL_H, &crate::damage_digits::SMALL[d]),
    }
}

/// A number in the damage font, advancing by the cell width the game's own renderer
/// (`0x1e6c`) uses. Anything that is not an ASCII digit is skipped.
pub fn digits(text: &str, size: Size) -> Bitmap {
    let ds: Vec<usize> = text.bytes().filter(u8::is_ascii_digit).map(|b| usize::from(b - b'0')).collect();
    let (cw, ch, _) = digit_cell(0, size);
    let mut out = Bitmap::new(cw * ds.len(), if ds.is_empty() { 0 } else { ch });
    for (i, &d) in ds.iter().enumerate() {
        let (_, _, rows) = digit_cell(d, size);
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.bytes().enumerate() {
                let v = match c {
                    b'1'..=b'4' => c - b'0',
                    _ => INK_NONE,
                };
                out.put(i * cw + x, y, v);
            }
        }
    }
    out
}

/// How much each glyph stroke is thickened, rightwards (and each advance widened by, so
/// neighbours do not merge). The digits' strokes are 3-5 px of fill; a 1-px font stroke
/// inside the same rim reads as an outline with a thread of colour in it (compared on a
/// render of the real banner at the device's ×4.5), and one extra column is what brings
/// the words to the digits' weight.
pub const BOLD: usize = 1;

/// Game-font text dressed as the digits are: each glyph's mask ([`BOLD`]-thickened)
/// filled row by row with [`TEXT_RAMP`] and wrapped in the outline (1 px, 2 px on the
/// right). `cells` are the font's (rows, advance) pairs (`gamefont::layout`), so the
/// spacing is the game's plus [`BOLD`]. Height = 1 + 11 + 1 = the small digit set's.
pub fn text(cells: &[(GlyphRows, usize)]) -> Bitmap {
    let advance: usize = cells.iter().map(|(_, a)| *a + BOLD).sum();
    if advance == 0 {
        return Bitmap::new(0, 0);
    }
    // 1 column of outline on the left; on the right the last glyph may reach past its
    // advance into its 16-px cell, plus 2 of outline — a full cell of slack, trimmed below.
    // 1 row above and below.
    let w = 1 + advance + 16;
    let h = GLYPH_H + 2;
    let mut mask = vec![false; w * h];
    let mut pen = 1usize;
    for (rows, adv) in cells {
        for (y, bits) in rows.iter().enumerate() {
            for c in 0..16usize {
                if bits & (1 << (15 - c)) != 0 {
                    for b in 0..=BOLD {
                        if pen + c + b < w {
                            mask[(y + 1) * w + pen + c + b] = true;
                        }
                    }
                }
            }
        }
        pen += adv + BOLD;
    }
    let at = |x: isize, y: isize| -> bool {
        x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && mask[y as usize * w + x as usize]
    };
    let mut out = Bitmap::new(w, h);
    let mut right = 0usize;
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            if at(xi, yi) {
                out.put(x, y, TEXT_RAMP[y - 1]);
                right = right.max(x);
            } else {
                // The outline: every 8-neighbour of the mask, and the second column to the
                // right of it (the digits' heavier right rim).
                let near = (-1..=1).any(|dy| (-1..=1).any(|dx| (dx, dy) != (0, 0) && at(xi + dx, yi + dy)));
                if near || at(xi - 2, yi) {
                    out.put(x, y, INK_OUTLINE);
                    right = right.max(x);
                }
            }
        }
    }
    trim_right(out, right + 1)
}

fn trim_right(src: Bitmap, w: usize) -> Bitmap {
    let w = w.min(src.w);
    let mut out = Bitmap::new(w, src.h);
    for y in 0..src.h {
        for x in 0..w {
            out.px[y * w + x] = src.get(x, y);
        }
    }
    out
}

/// Pieces side by side, BOTTOM-aligned (a number and its unit share a baseline), `gap`
/// transparent columns apart. Empty pieces take no room.
pub fn hcat(parts: &[Bitmap], gap: usize) -> Bitmap {
    let parts: Vec<&Bitmap> = parts.iter().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Bitmap::new(0, 0);
    }
    let h = parts.iter().map(|p| p.h).max().unwrap_or(0);
    let w = parts.iter().map(|p| p.w).sum::<usize>() + gap * (parts.len() - 1);
    let mut out = Bitmap::new(w, h);
    let mut x = 0;
    for p in parts {
        out.blit(p, x, h - p.h);
        x += p.w + gap;
    }
    out
}

/// Lines stacked, each centred on the widest, `gap` rows apart. Empty lines take no room.
pub fn vcat(lines: &[Bitmap], gap: usize) -> Bitmap {
    let lines: Vec<&Bitmap> = lines.iter().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return Bitmap::new(0, 0);
    }
    let w = lines.iter().map(|l| l.w).max().unwrap_or(0);
    let h = lines.iter().map(|l| l.h).sum::<usize>() + gap * (lines.len() - 1);
    let mut out = Bitmap::new(w, h);
    let mut y = 0;
    for l in lines {
        out.blit(l, (w - l.w) / 2, y);
        y += l.h + gap;
    }
    out
}

/// The banner's second line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status<'a> {
    /// 「남은 몬스터 N」 — the floor is live.
    Left(u16),
    /// A word instead of a count (진입 · 클리어 · 완주 · 이탈).
    Word(&'a str),
}

/// The whole banner: 「N층」 (+ 「보스」) over the status line, each piece in its [`Ink`].
/// `glyphs` lays a string out in the game's font (`gamefont::layout` as (rows, advance)
/// pairs).
pub fn banner(floor: u8, boss: bool, status: Status<'_>, glyphs: &dyn Fn(&str) -> Vec<(GlyphRows, usize)>) -> Bitmap {
    let mut first = vec![tint(&digits(&floor.to_string(), Size::Big), Ink::Floor), tint(&text(&glyphs("층")), Ink::Floor)];
    if boss {
        first.push(tint(&text(&glyphs("보스")), Ink::Floor));
    }
    let first = hcat(&first, WORD_GAP);
    let second = match status {
        Status::Left(n) => hcat(
            &[tint(&text(&glyphs("남은 몬스터")), Ink::Label), tint(&digits(&n.to_string(), Size::Small), Ink::Count)],
            WORD_GAP + 2,
        ),
        Status::Word(w) => tint(&text(&glyphs(w)), Ink::Label),
    };
    vcat(&[first, second], LINE_GAP)
}

/// The banner's size against the game's own damage numbers (147차, user 2026-10-01:
/// *"현재층과 남은 몬스터 둘다 폰트 조금 줄여주고"*). 1.0 drew its digits exactly as big
/// as the ones the game pops over a monster (145차), and that read as too large.
pub const SCALE: f64 = 0.8;

/// Window pixels per banner pixel: the game→window factor (`game_h / logical_h`, the
/// size of a game pixel on screen) times [`SCALE`]. 0 when there is no game rect.
pub fn pixel_scale(game_h: u32, logical_h: u32) -> f64 {
    if logical_h == 0 { 0.0 } else { game_h as f64 / logical_h as f64 * SCALE }
}

/// Where the banner goes, in window pixels: centred on the game rect, its top edge
/// [`TOP_FRACTION`] of the game rect's height down. `scale` = window pixels per banner
/// pixel ([`pixel_scale`]). Returns `(x, y)` of the top-left corner.
pub fn place(game_x: u32, game_y: u32, game_w: u32, game_h: u32, scale: f64, bitmap_w: usize) -> (f64, f64) {
    let w = bitmap_w as f64 * scale;
    let x = game_x as f64 + (game_w as f64 - w) / 2.0;
    let y = game_y as f64 + game_h as f64 * TOP_FRACTION;
    (x.max(0.0), y)
}

/// The banner's top edge as a fraction of the game rect's height. 145차 hung it 10 %
/// down (*"너무 위쪽에 붙이진말고 적당히 10%정도"*); 147차 lifts it a little
/// (*"전체적으로 위치 조금만 위로 올려줘"*). The one thing above it is the EXIT tab, flush
/// with the top edge and 58 of 1080 px tall on the S9 (5.4 %) — at 7 % the banner's first
/// ink (the big digits' two blank top rows below it) still clears the tab by ~25 px.
pub const TOP_FRACTION: f64 = 0.07;
