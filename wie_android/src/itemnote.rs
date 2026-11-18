// 150차 (2026-10-01) — the 상세보기 body overlay: WHAT the host adds to the game's own item
// detail window and HOW the scrolled body is composed. PURE: `itemnote_selftest.rs`
// `include!`s this file with `gamefont.rs` + `gamefont_data.rs` (its only dependency).
//
// ── WHY ─────────────────────────────────────────────────────────────────────
// User decisions (2026-10-01): the 11 reworked accessories must not show 「기절 저항 +0」
// and must show their THIRD option (`acc_options.rs` — it lives outside the asset row,
// whose parser reads exactly two option pairs), and every set piece must show its set
// option — in a SCROLLABLE body. The game's detail window is overlay page 7 (painter
// `0x4503c`, keys `0x46340`: only CLR does anything there, so UP/DOWN/drag are free), and
// its text body holds six lines at most — a legacy armour row already fills all six.
//
// ── HOW (report_host-ui.md 안 A) ───────────────────────────────────────────────
// The game keeps drawing its window exactly as it does; the UI cuts the body rectangle out
// of the game frame ([`body_patch`]), [`compose`]s a replacement at LOGICAL resolution —
// the game's own rows it keeps ([`ItemNote::keep`], shifted up to the body's first line),
// then the host lines rendered 1:1 in the game's own font and tones, clipped, plus a
// HELP-style scrollbar when it overflows — and blits that patch with the same scaling as
// the game frame. Pixel-exact by construction: `itemnote_selftest` re-renders lines the
// game drew (`특수공격 +30%` …) and diffs them against probe frames — zero pixels differ.
// [`shows`] guards the draw: the note is for THIS item only if the window's name box reads
// exactly its name (and, when it drops the first line, that line reads exactly
// [`RESIST_ZERO`]) — so a stale note never lands on an item of ANOTHER NAME. Two items of
// the same name (two 템플러헬름, one socketed) the pixels cannot tell apart: there the guarantee
// is the worker's — it builds the note from the very cell the painter reads
// (`items::item_note_target`), so the worst case is one frame of the previous one (150차 review).
//
// ── GEOMETRY (panel-relative logical px; wie_probe 496x240, vc422 asset) ─────────
// body fill #522d19 = dx 4..171 x dy 61..154 (168x94; rows 61/154 lose one px per corner
// to the #7b5952 border). The game's lines: pen x 6, first line top dy 63, 11 rows tall.
//   accessory:  기절 저항 +N @63 · option 1 @79 · option 2 @95           (no sockets)
//   equipment:  class/faction @63 · 방/공 @79 · 제련도 @95 · options @109 @123 ·
//               orb 「/ …」 @(42,135) · sockets 4x4 @(6,139) pitch 6
// Name box: pen (27, 21), ink = the name's colour escape (`name_rgb`), box #311c10.

use crate::gamefont;

/// The body rectangle (panel-relative). Mirrors `menutouch::DETAIL_BODY`.
pub const BODY_DX: i32 = 4;
pub const BODY_DY: i32 = 61;
pub const BODY_W: usize = 168;
pub const BODY_H: usize = 94;
/// The body's fill.
pub const BODY_BG: u32 = 0x0052_2d19;
/// Where the game's body lines start: pen x, the first line's top.
pub const TEXT_DX: i32 = 6;
pub const FIRST_LINE_DY: i32 = 63;
/// The game's line pitch in the body (11 rows of glyph + 5).
pub const PITCH: i32 = 16;
/// Glyph rows (Hangul; ASCII sits [`gamefont::GAME_ASCII_DROP`] lower but is 7 rows tall).
pub const GLYPH_H: i32 = 11;
/// Rows of fill kept above the first line and below the last one at the scroll ends.
pub const MARGIN: i32 = 2;
/// The visible content rows of the patch: one fill row stays at each edge.
pub const CLIP_TOP: i32 = 1;
pub const CLIP_BOT: i32 = BODY_H as i32 - 1;

/// The accessory body (`0x45968`): 「기절 저항 +N」 then the two options.
pub const ACC_RESIST_DY: i32 = 63;
pub const ACC_OPT1_DY: i32 = 79;
pub const ACC_BODY_END: i32 = 95 + GLYPH_H;
/// The equipment body (`0x455a4`): the two options end at 123 + 11, the sockets at
/// 139 + 4, the orb line at 135 + 11.
pub const EQUIP_OPTS_END: i32 = 123 + GLYPH_H;
pub const EQUIP_SOCKETS_END: i32 = 139 + 4;
pub const EQUIP_ORB_END: i32 = 135 + GLYPH_H;
/// Between the game's last kept row and the first host line: the game's own gap (16 - 11).
pub const HOST_GAP: i32 = PITCH - GLYPH_H;
/// Extra rows above a section header (the set section, the native-set line).
pub const GROUP_GAP: u8 = 6;
/// The host line width: pen x 6 .. one px short of the scrollbar.
pub const WRAP_W: usize = (BAR_DX - 1 - TEXT_DX) as usize;

/// The scrollbar (panel dx; drawn only when the content overflows): the game's HELP
/// window's — a 7-px #cecac5 track with #9c968c arrows and thumb (run9, 80차-L).
pub const BAR_DX: i32 = 164;
pub const BAR_W: i32 = 7;
pub const BAR_TRACK: u32 = 0x00ce_cac5;
pub const BAR_THUMB: u32 = 0x009c_968c;
/// Track rows (patch-relative, inclusive start, exclusive end) and the thumb's lane.
pub const BAR_TOP: i32 = MARGIN;
pub const BAR_BOT: i32 = BODY_H as i32 - MARGIN;
pub const THUMB_TOP: i32 = BAR_TOP + 6;
pub const THUMB_BOT: i32 = BAR_BOT - 6;
pub const THUMB_MIN: i32 = 6;

/// One scroll step of the keys and of a tap (the game's own line pitch), and a page.
pub const SCROLL_LINE: i32 = PITCH;
pub const SCROLL_PAGE: i32 = BODY_H as i32 - PITCH * 2;

/// The name box: pen, first row, and its fill.
pub const NAME_DX: i32 = 27;
pub const NAME_DY: i32 = 21;
pub const NAME_BOX_BG: u32 = 0x0031_1c10;
/// The name box's right edge (exclusive) — a name is compared up to here.
pub const NAME_BOX_END: i32 = 170;

/// The game's first accessory line once the resist is 0 — the line 150차 drops.
pub const RESIST_ZERO: &str = "기절 저항 +0";
/// The set section's rule line: `total` pieces in the set, `need` of them make the effect
/// (`setbonus::PIECES_FOR_BONUS`) — 「6부위 중 4부위 착용 시」, and for a five-piece set
/// (154차 · 155차: 템플러 · 크루세이더 · 코르버스, no accessory) 「5부위 중 4부위 착용 시」.
pub fn rule_line(total: usize, need: u8) -> String {
    format!("{total}부위 중 {need}부위 착용 시")
}
/// The native (b11) set line: the game's own 4-armour-piece bonus (`0x4f270..0x4f354`).
/// ★ 154차 (user, 2026-10-02: *"원작 세트 말고 세트효과 로 변경해주고"*): 「원작 세트」 →
/// 「세트효과」 — the player reads the bonus, not where it comes from.
pub const NATIVE_SET_TEXT: &str = "세트효과: 투구, 갑옷, 장갑, 신발 착용 시 최대HP, 공격, 방어 +20%, SP소모 -20";

/// A host line's ink. The game's own tones: body text, the `}` escape's gold, the `'`
/// escape's green (`0x5b268`), and a dim tone for a set piece that is not worn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Body,
    Gold,
    Green,
    Dim,
}

impl Tone {
    pub const fn rgb(self) -> u32 {
        match self {
            Tone::Body => 0x00b5_8263,
            Tone::Gold => 0x00db_9a1a,
            Tone::Green => 0x0078_c437,
            Tone::Dim => 0x008c_7063,
        }
    }
}

/// The ink of an item name for its colour escape (the game's table at `0x5b268`); `None`
/// = no escape, the window's default name tone.
pub const fn name_rgb(escape: Option<u8>) -> u32 {
    match escape {
        Some(b'@') => 0x00ff_ffff,
        Some(b'|') => 0x00d8_d926,
        Some(b'$') => 0x002e_bcff,
        Some(b'{') => 0x00df_3939,
        Some(b'}') => 0x00db_9a1a,
        Some(b'\'') => 0x0078_c437,
        Some(b'"') => 0x00ff_0000,
        _ => 0x00d6_b29c,
    }
}

/// One host line, already wrapped to [`WRAP_W`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteLine {
    pub text: String,
    pub tone: Tone,
    /// Extra rows above this line (a section break); 0 for an ordinary line.
    pub gap: u8,
}

/// What the overlay draws for one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemNote {
    /// The item's name as the name box shows it (no colour escape) and its ink.
    pub name: String,
    pub name_rgb: u32,
    /// What the name box draws BEFORE [`Self::name`], in its own ink — an orb's label and the
    /// painter's separating space (`"%s %s"` at `0x45474`: 「투신의 」 in `@` white). `None` =
    /// the name starts the box. [`build`] leaves it `None`; the worker fills it (the label
    /// lives in the game's string table, which only the worker can read).
    pub name_prefix: Option<(String, u32)>,
    /// The game's own body rows kept, panel dy `[from, to)`, shifted up to the first line.
    pub keep: (i32, i32),
    /// When the note drops the game's first line: what that line must read.
    pub dropped: Option<String>,
    /// The host lines after the kept rows.
    pub lines: Vec<NoteLine>,
}

/// Which body the game draws for the item — what decides the rows kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameBody {
    /// cat 0x0c. `drop_resist`: only for the 11 reworked accessories, and only while
    /// their resist is 0 — then the game's first line reads exactly [`RESIST_ZERO`].
    Accessory { drop_resist: bool },
    /// Everything with the five-row equipment body (armour, shields, weapons):
    /// `sockets` = the record has holes (`rec+0x3c6 > 0`), `orb` = one is set
    /// (`rec+0x3c8 != -1`) — the two lower rows.
    Equipment { sockets: bool, orb: bool },
}

impl GameBody {
    /// The panel rows `[from, to)` of the game's body this item keeps.
    pub const fn keep(self) -> (i32, i32) {
        match self {
            GameBody::Accessory { drop_resist: true } => (ACC_OPT1_DY, ACC_BODY_END),
            GameBody::Accessory { drop_resist: false } => (ACC_RESIST_DY, ACC_BODY_END),
            GameBody::Equipment { orb: true, .. } => (FIRST_LINE_DY, EQUIP_ORB_END),
            GameBody::Equipment { sockets: true, orb: false } => (FIRST_LINE_DY, EQUIP_SOCKETS_END),
            GameBody::Equipment { sockets: false, orb: false } => (FIRST_LINE_DY, EQUIP_OPTS_END),
        }
    }
}

/// One piece of a set, as the set section lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece<'a> {
    /// The slot word: 투구 · 갑옷 · 장갑 · 신발 · 방패 · 장신구 (`setbonus::slot_word`).
    pub slot: &'a str,
    /// The piece's name (no colour escape).
    pub name: &'a str,
    /// Worn right now.
    pub worn: bool,
}

/// The set section of a set piece (`setbonus::SETS[sid]` + the pieces of `set_items.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetView<'a> {
    /// The set's name (템플러 …).
    pub name: &'a str,
    /// The effect's name and text (`SetInfo::bonus` / `SetInfo::text`).
    pub bonus: &'a str,
    pub text: &'a str,
    /// The set's pieces (four to six, 154차), in slot order.
    pub pieces: &'a [Piece<'a>],
    /// Distinct pieces of this set worn (`setbonus::piece_counts(..)[sid]`).
    pub worn: u8,
    /// Pieces the effect needs (`setbonus::PIECES_FOR_BONUS`).
    pub need: u8,
}

/// What the worker knows about the item on page 7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteInput<'a> {
    /// The name without its colour escape, and that escape byte (`None` if none).
    pub name: &'a str,
    pub escape: Option<u8>,
    pub body: GameBody,
    /// The third option's detail text (`acc_options::AccThird::detail`).
    pub third: Option<&'a str>,
    /// The item's set, if it is a set piece.
    pub set: Option<SetView<'a>>,
    /// The native (b11) set line: an armour piece (cat 0..=3) whose row carries a native
    /// set id — the game's own 4-piece bonus.
    pub native_set: bool,
}

/// The note for one item, or `None` when the host changes nothing (the game's own window
/// is then left alone).
pub fn build(inp: &NoteInput) -> Option<ItemNote> {
    let drop = matches!(inp.body, GameBody::Accessory { drop_resist: true });
    let keep = inp.body.keep();
    let mut lines = Vec::new();
    // The third option continues the game's own option list (no break); each section after
    // it opens with a break unless nothing stands above it.
    if let Some(t) = inp.third {
        push_wrapped(&mut lines, t, Tone::Gold, 0);
    }
    let brk = |lines: &[NoteLine]| if lines.is_empty() && keep.1 <= keep.0 { 0 } else { GROUP_GAP };
    if let Some(s) = &inp.set {
        // "(착용 N/M)" is one unit: a narrow set name keeps it on the header line, a wide one
        // (레갈리스근위) moves it whole to the next line instead of splitting 「착용」 from N/M.
        // M = the set's own size (154차 · 155차: 5 for 템플러 · 크루세이더 · 코르버스, 6 for the others).
        let gap = brk(&lines);
        push_wrapped(&mut lines, &format!("{} 세트 (착용\u{a0}{}/{})", s.name, s.worn, s.pieces.len()), Tone::Gold, gap);
        push_wrapped(&mut lines, &rule_line(s.pieces.len(), s.need), Tone::Body, 0);
        push_wrapped(&mut lines, s.bonus, Tone::Gold, 0);
        push_wrapped(&mut lines, s.text, Tone::Body, 0);
        for p in s.pieces {
            let tone = if p.worn { Tone::Green } else { Tone::Dim };
            push_wrapped(&mut lines, &format!("{} {}", p.slot, p.name), tone, 0);
        }
    }
    if inp.native_set {
        let gap = brk(&lines);
        push_wrapped(&mut lines, NATIVE_SET_TEXT, Tone::Body, gap);
    }
    if lines.is_empty() && !drop {
        return None;
    }
    Some(ItemNote {
        name: inp.name.to_string(),
        name_rgb: name_rgb(inp.escape),
        name_prefix: None,
        keep,
        dropped: drop.then(|| RESIST_ZERO.to_string()),
        lines,
    })
}

/// Wrap `text` to [`WRAP_W`] and append its lines; `gap` goes on the first one only.
fn push_wrapped(lines: &mut Vec<NoteLine>, text: &str, tone: Tone, gap: u8) {
    for (i, t) in wrap(text, WRAP_W).into_iter().enumerate() {
        lines.push(NoteLine { text: t, tone, gap: if i == 0 { gap } else { 0 } });
    }
}

/// Word-wrap `text` to `max` px under the game drawer's metrics. Breaks at spaces (and
/// always at `\n`); a no-break space (U+00A0) holds two words together and is drawn as a
/// space; a word wider than a line is broken between characters. Empty text gives no lines.
///
/// Greedy, then balanced when greedy leaves an orphan — a last line under half of the one
/// before it (「적 처치 시 20초간 최대 HP」 / 「+20%」): the same number of lines at the
/// narrowest width that still needs no more (「적 처치 시 20초간」 / 「최대 HP +20%」). A
/// last line of half or more is left alone — 「(재사용 100초)」 stays a line of its own.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    text.split('\n').flat_map(|para| wrap_balanced(para, max)).collect()
}

fn wrap_balanced(text: &str, max: usize) -> Vec<String> {
    let greedy = wrap_one(text, max);
    let n = greedy.len();
    let orphan = n >= 2 && gamefont::game_width(&greedy[n - 1]) * 2 < gamefont::game_width(&greedy[n - 2]);
    if !orphan {
        return greedy;
    }
    // Line count only falls as the width grows, so the narrowest width that still gives
    // `n` lines is a binary search; below the widest word, words would break instead.
    let widest = text.split(' ').map(|w| gamefont::game_width(&w.replace('\u{a0}', " "))).max().unwrap_or(0);
    let (mut lo, mut hi) = (widest.min(max), max);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if wrap_one(text, mid).len() <= n { hi = mid } else { lo = mid + 1 }
    }
    wrap_one(text, hi)
}

fn wrap_one(text: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let space = gamefont::GAME_SPACE_ADVANCE;
    let mut line_w = 0usize;
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let word = word.replace('\u{a0}', " ");
        let w = gamefont::game_width(&word);
        if !line.is_empty() && line_w + space + w <= max {
            line.push(' ');
            line.push_str(&word);
            line_w += space + w;
            continue;
        }
        if !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line_w = 0;
        }
        if w <= max {
            line = word;
            line_w = w;
            continue;
        }
        // A word no line can hold: break it between characters.
        for ch in word.chars() {
            let cw = gamefont::game_width(ch.encode_utf8(&mut [0u8; 4]));
            if !line.is_empty() && line_w + cw > max {
                out.push(std::mem::take(&mut line).trim_end().to_string());
                line_w = 0;
            }
            if line.is_empty() && ch == ' ' {
                continue;
            }
            line.push(ch);
            line_w += cw;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// The content-space top row of every host line (content row 0 = patch row 0 at scroll 0;
/// the game's first line is at row [`MARGIN`]).
pub fn line_tops(note: &ItemNote) -> Vec<i32> {
    let kept = (note.keep.1 - note.keep.0).max(0);
    let mut tops = Vec::with_capacity(note.lines.len());
    let mut next = if kept > 0 { MARGIN + kept + HOST_GAP } else { MARGIN };
    for l in &note.lines {
        let top = next + i32::from(l.gap);
        tops.push(top);
        next = top + PITCH;
    }
    tops
}

/// The content's height in rows, margins included.
pub fn content_height(note: &ItemNote) -> i32 {
    let kept = (note.keep.1 - note.keep.0).max(0);
    let end = line_tops(note).last().map_or(MARGIN + kept, |t| t + GLYPH_H);
    end + MARGIN
}

/// How far the body can scroll (0 = it fits).
pub fn max_scroll(note: &ItemNote) -> i32 {
    (content_height(note) - BODY_H as i32).max(0)
}

/// A scroll position made valid for `note` — clamp on input AND on draw (an unclamped
/// value draws an empty body).
pub fn clamp_scroll(note: &ItemNote, scroll: i32) -> i32 {
    scroll.clamp(0, max_scroll(note))
}

/// A tap that did not drag, HELP-style: the upper half of the body scrolls up one line,
/// the lower half down. `dy` is panel-relative.
pub fn tap_step(dy: i32) -> i32 {
    if dy < BODY_DY + BODY_H as i32 / 2 { -SCROLL_LINE } else { SCROLL_LINE }
}

/// The scrollbar's thumb `(top, height)` in patch rows, `None` when the content fits.
pub fn thumb(note: &ItemNote, scroll: i32) -> Option<(i32, i32)> {
    let content = content_height(note);
    let view = BODY_H as i32;
    if content <= view {
        return None;
    }
    let lane = THUMB_BOT - THUMB_TOP;
    let h = (lane * view / content).clamp(THUMB_MIN, lane);
    let span = content - view;
    let top = THUMB_TOP + (lane - h) * clamp_scroll(note, scroll) / span;
    Some((top, h))
}

/// A logical game frame: `w * h` pixels, `0x??RRGGBB` (the UI's `last_frame`).
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub px: &'a [u32],
    pub w: usize,
    pub h: usize,
}

impl Frame<'_> {
    /// The RGB at `(x, y)`, `None` off the frame.
    fn rgb(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        self.px.get(y as usize * self.w + x as usize).map(|p| p & 0x00ff_ffff)
    }
}

/// Cut the body out of a logical frame: `origin` = the panel's top-left
/// (`menutouch::panel_origin`), `out` = `BODY_W * BODY_H` pixels. `false` (and `out`
/// untouched) when the rectangle does not fit the frame.
pub fn body_patch(f: Frame, origin: (i32, i32), out: &mut [u32]) -> bool {
    let (x0, y0) = (origin.0 + BODY_DX, origin.1 + BODY_DY);
    if x0 < 0 || y0 < 0 || out.len() != BODY_W * BODY_H || f.px.len() < f.w * f.h {
        return false;
    }
    let (x0, y0) = (x0 as usize, y0 as usize);
    if x0 + BODY_W > f.w || y0 + BODY_H > f.h {
        return false;
    }
    for r in 0..BODY_H {
        let src = (y0 + r) * f.w + x0;
        out[r * BODY_W..(r + 1) * BODY_W].copy_from_slice(&f.px[src..src + BODY_W]);
    }
    true
}

/// Compose the visible body for `note` at `scroll` into `out` (both slices `BODY_W *
/// BODY_H`, `game` = [`body_patch`] of the game frame). Pixels are the frame's format
/// (`0x??RRGGBB`); host ink is written opaque. The game's corner pixels are kept, the rest
/// starts as the body fill; the kept game rows, then the host lines, are drawn clipped to
/// rows [`CLIP_TOP`]..[`CLIP_BOT`]; the scrollbar last. `false` on a size mismatch.
pub fn compose(game: &[u32], note: &ItemNote, scroll: i32, out: &mut [u32]) -> bool {
    let (w, h) = (BODY_W as i32, BODY_H as i32);
    if game.len() != BODY_W * BODY_H || out.len() != BODY_W * BODY_H {
        return false;
    }
    let scroll = clamp_scroll(note, scroll);
    let bar = max_scroll(note) > 0;
    let bg = 0xff00_0000 | BODY_BG;
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let corner = (y == 0 || y == h - 1) && (x == 0 || x == w - 1);
            out[i] = if corner { game[i] } else { bg };
        }
    }
    // 1) the game's own rows, shifted up to the first line
    let kept = (note.keep.1 - note.keep.0).max(0);
    for i in 0..kept {
        let src = note.keep.0 - BODY_DY + i;
        let dst = MARGIN + i - scroll;
        if !(0..h).contains(&src) || !(CLIP_TOP..CLIP_BOT).contains(&dst) {
            continue;
        }
        let (s, d) = ((src * w) as usize, (dst * w) as usize);
        out[d..d + BODY_W].copy_from_slice(&game[s..s + BODY_W]);
    }
    // 2) the host lines
    let right = if bar { BAR_DX - BODY_DX - 1 } else { w - 1 };
    for (line, top) in note.lines.iter().zip(line_tops(note)) {
        let y = top - scroll;
        if y + GLYPH_H + gamefont::GAME_ASCII_DROP as i32 <= CLIP_TOP || y >= CLIP_BOT {
            continue;
        }
        draw_text(out, TEXT_DX - BODY_DX, y, &line.text, 0xff00_0000 | line.tone.rgb(), (CLIP_TOP, CLIP_BOT), right);
    }
    // 3) the scrollbar
    if bar && let Some((ttop, th)) = thumb(note, scroll) {
        let bx = BAR_DX - BODY_DX;
        let put = |out: &mut [u32], x: i32, y: i32, c: u32| {
            if (0..w).contains(&x) && (0..h).contains(&y) {
                out[(y * w + x) as usize] = 0xff00_0000 | c;
            }
        };
        for y in BAR_TOP..BAR_BOT {
            for x in bx..bx + BAR_W {
                put(out, x, y, BAR_TRACK);
            }
        }
        let mid = bx + BAR_W / 2;
        for i in 0..3 {
            for x in mid - i..=mid + i {
                put(out, x, BAR_TOP + 2 + i, BAR_THUMB);
                put(out, x, BAR_BOT - 3 - i, BAR_THUMB);
            }
        }
        for y in ttop..ttop + th {
            for x in bx + 1..bx + BAR_W - 1 {
                put(out, x, y, BAR_THUMB);
            }
        }
    }
    true
}

/// Draw `text` with its pen at patch `(x, y)` (the line's top), clipped to rows
/// `clip.0..clip.1` and columns `..right` (exclusive).
fn draw_text(out: &mut [u32], x: i32, y: i32, text: &str, ink: u32, clip: (i32, i32), right: i32) {
    let w = BODY_W as i32;
    let mut pen = x;
    for cell in gamefont::game_layout(text) {
        for (r, bits) in cell.rows.iter().enumerate() {
            let yy = y + cell.drop as i32 + r as i32;
            if *bits == 0 || yy < clip.0 || yy >= clip.1 {
                continue;
            }
            for c in 0..gamefont::KOR_W as i32 {
                let xx = pen + c;
                if bits & (1 << (15 - c)) != 0 && xx >= 0 && xx < right.min(w) {
                    out[(yy * w + xx) as usize] = ink;
                }
            }
        }
        pen += cell.advance as i32;
    }
}

/// Does the frame's line with its pen at panel `at = (dx, top dy)` read exactly `text` in
/// `ink` on `bg`? Compared over the text's advance plus a trailing 8 px (a longer string
/// with the same start does not pass) — cut at panel dx `x_end` — and 13 rows (the ASCII
/// drop included). This is the pixel-exact test of the game drawer's metrics too: a line
/// the game drew reads back through it only if [`gamefont::game_layout`] places every
/// glyph where the game did.
pub fn reads(f: Frame, origin: (i32, i32), at: (i32, i32), text: &str, ink: u32, bg: u32, x_end: i32) -> bool {
    reads_runs(f, origin, at, &[(text, ink)], bg, x_end)
}

/// [`reads`] for a line drawn in several inks one after another — the game's drawer switches
/// colour at every escape byte, wherever it stands (`applyColourEscape 0x37e4`), so an orb's
/// `@` label and the item's own `"` name share one line. Each run starts where the previous
/// one's advance ended; the trailing 8 px after the LAST run must be the background.
pub fn reads_runs(f: Frame, origin: (i32, i32), at: (i32, i32), runs: &[(&str, u32)], bg: u32, x_end: i32) -> bool {
    let laid: Vec<(Vec<gamefont::GameCell>, u32)> = runs.iter().map(|&(t, ink)| (gamefont::game_layout(t), ink)).collect();
    if laid.iter().all(|(cells, _)| cells.is_empty()) {
        return false;
    }
    let adv: i32 = laid.iter().flat_map(|(cells, _)| cells.iter()).map(|c| c.advance as i32).sum();
    let (x0, y0) = (origin.0 + at.0, origin.1 + at.1);
    let x1 = (x0 + adv + 8).min(origin.0 + x_end);
    let (cols, rows) = (x1 - x0, GLYPH_H + gamefont::GAME_ASCII_DROP as i32);
    if cols <= 0 {
        return false;
    }
    // what the game would have drawn: the ink of every pixel it inks
    let mut want: Vec<Option<u32>> = vec![None; (cols * rows) as usize];
    let mut pen = 0;
    for (cells, ink) in &laid {
        for cell in cells {
            for (r, bits) in cell.rows.iter().enumerate() {
                let yy = cell.drop as i32 + r as i32;
                for c in 0..gamefont::KOR_W as i32 {
                    let xx = pen + c;
                    if bits & (1 << (15 - c)) != 0 && yy < rows && xx < cols {
                        want[(yy * cols + xx) as usize] = Some(*ink);
                    }
                }
            }
            pen += cell.advance as i32;
        }
    }
    (0..rows).all(|yy| {
        (0..cols).all(|xx| {
            let expect = want[(yy * cols + xx) as usize].unwrap_or(bg);
            f.rgb(x0 + xx, y0 + yy) == Some(expect)
        })
    })
}

/// Is the window on `f` the one `note` was built for? Its name box must read exactly the
/// note's name in the note's ink (after its [`ItemNote::name_prefix`], when it has one), and
/// a dropped first line must read exactly what the note says it drops. Call it on the GAME
/// frame, after the page-7 check (`menutouch::detail_page`) — never on a composed one.
pub fn shows(f: Frame, origin: (i32, i32), note: &ItemNote) -> bool {
    let name_ok = match &note.name_prefix {
        Some((p, ink)) => reads_runs(
            f,
            origin,
            (NAME_DX, NAME_DY),
            &[(p.as_str(), *ink), (note.name.as_str(), note.name_rgb)],
            NAME_BOX_BG,
            NAME_BOX_END,
        ),
        None => reads(f, origin, (NAME_DX, NAME_DY), &note.name, note.name_rgb, NAME_BOX_BG, NAME_BOX_END),
    };
    if !name_ok {
        return false;
    }
    match &note.dropped {
        Some(t) => reads(f, origin, (TEXT_DX, FIRST_LINE_DY), t, Tone::Body.rgb(), BODY_BG, BODY_DX + BODY_W as i32),
        None => true,
    }
}
