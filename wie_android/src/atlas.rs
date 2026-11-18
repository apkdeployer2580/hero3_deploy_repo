//! Decoded quickbar icon atlas, embedded from `icons.bin` (built by
//! `tools_re/gen_h3_icons.py`, which carries its own exact `_bm` decoder — the
//! repo's convert_bm_v2 mis-reads palette[0] as a frame marker and scrambles
//! every colour; see that script's header).
//! These icons are number-free (the game's hotkey/count digits come from a
//! separate `number_bm` font sheet, never from the icon sheets), so drawing
//! from here gives clean icons — unlike cropping the composited game HUD cell.
//!
//! Binary format (see gen_h3_icons.py):
//!   magic "H3IC"; item_count u16 LE;
//!   item icons[item_count]: w u8, h u8, px[w*h] u32 LE (0xAARRGGBB, a=0 transparent);
//!   skill_sheet_count u8; per sheet: cell_count u8, cells[..]: w u8, h u8, px[w*h] u32.

use std::sync::OnceLock;

static ICONS_BIN: &[u8] = include_bytes!("icons.bin");

pub struct Icon {
    pub w: u16,
    pub h: u16,
    /// `w*h` pixels, row-major, `0xAARRGGBB`; alpha 0 = transparent (skip).
    pub px: Vec<u32>,
}

pub struct Atlas {
    pub items: Vec<Icon>,
    /// `skills[sheet 0..6 = s4..s10][cell]`.
    pub skills: Vec<Vec<Icon>>,
}

impl Atlas {
    /// The item_bm frame `i`, if in range.
    pub fn item(&self, i: u16) -> Option<&Icon> {
        self.items.get(i as usize)
    }
    /// The skill sheet `sheet` (0..6), cell `cell`, if in range.
    pub fn skill(&self, sheet: u8, cell: u8) -> Option<&Icon> {
        self.skills.get(sheet as usize).and_then(|s| s.get(cell as usize))
    }
}

/// Parse the embedded atlas once. Returns an empty atlas on any malformation
/// (the HUD then falls back to hotkey-char placeholders — never panics).
pub fn atlas() -> &'static Atlas {
    static A: OnceLock<Atlas> = OnceLock::new();
    A.get_or_init(|| parse(ICONS_BIN).unwrap_or(Atlas { items: Vec::new(), skills: Vec::new() }))
}

fn parse(d: &[u8]) -> Option<Atlas> {
    if d.len() < 6 || &d[0..4] != b"H3IC" {
        return None;
    }
    let rd_u8 = |off: &mut usize| -> Option<u8> {
        let v = *d.get(*off)?;
        *off += 1;
        Some(v)
    };
    let item_count = u16::from_le_bytes([*d.get(4)?, *d.get(5)?]);
    let mut off = 6usize;

    fn read_icon(d: &[u8], off: &mut usize) -> Option<Icon> {
        let w = *d.get(*off)? as usize;
        let h = *d.get(*off + 1)? as usize;
        *off += 2;
        let n = w * h;
        let end = *off + n * 4;
        if end > d.len() {
            return None;
        }
        let mut px = Vec::with_capacity(n);
        for k in 0..n {
            let b = *off + k * 4;
            px.push(u32::from_le_bytes([d[b], d[b + 1], d[b + 2], d[b + 3]]));
        }
        *off = end;
        Some(Icon {
            w: w as u16,
            h: h as u16,
            px,
        })
    }

    let mut items = Vec::with_capacity(item_count as usize);
    for _ in 0..item_count {
        items.push(read_icon(d, &mut off)?);
    }
    let sheet_count = rd_u8(&mut off)?;
    let mut skills = Vec::with_capacity(sheet_count as usize);
    for _ in 0..sheet_count {
        let cells = rd_u8(&mut off)?;
        let mut sheet = Vec::with_capacity(cells as usize);
        for _ in 0..cells {
            sheet.push(read_icon(d, &mut off)?);
        }
        skills.push(sheet);
    }
    Some(Atlas { items, skills })
}
