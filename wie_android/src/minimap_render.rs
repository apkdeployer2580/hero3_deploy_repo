// The game's minimap, rendered by the HOST from the guest's live map arrays.
//
// ── WHY THIS FILE IS DEPENDENCY-FREE AND `include!`d ─────────────────────────
// `wie_probe --map-check` includes this exact source and diffs its output against
// the framebuffer the guest itself paints, so the verification exercises the code
// that ships rather than a hand-kept twin of it. (Same arrangement as `endpoint.rs`
// / `tls_buf.rs`.) The module doc lives in `items.rs` at the call site, because an
// `include!`d file cannot carry `//!` comments.
//
// ── THE GUEST ROUTINE, DECODED (static disasm 2026-08-03) ────────────────────
// `openWindow(8)` -> guest 0x12c938 -> **0x17ba44**, the map window's ENTER:
//     MAP  = *(*(CHAR+0x9c90)) + 8       KTF indirect handle; the `+8` data offset
//                                        is the same convention the inventory grid
//                                        uses (`GRID_DATA_ADJ`)
//     w,h  = (i32)MAP+0x48 / +0x4c       in TILES
//     fb   = MC_grpCreateOffScreenFrameBuffer(w*2, h*2)
//     for every pixel: FillRect(fb, px,py, 1,1, c>>16, c>>8 & 0xff, c & 0xff)
//                      with c = pixel_colour(px, py)
// `pixel_colour` (0x17b8e4) — 2x supersample with a corner blend:
//     tx,ty = px>>1, py>>1 ;  base = tile_colour(tx, ty)
//     nx = px odd ? tx+1 : tx-1 ;  ny = py odd ? ty+1 : ty-1
//     c1 = tile_colour(nx, ty) ;  c2 = tile_colour(tx, ny)   (-1 -> use base)
//     blend only when base differs from BOTH and base != the global at 0x1a4bc4;
//     then each channel = (base + c1 + c2) / 3   (0x19f874 = signed divide)
// `tile_colour` (0x17b874), out of bounds -> -1:
//     blocked = (attr[ty*w+tx] & 0x80) != 0                  (0x17a6f8)
//     c = blocked ? 0x5b5b5b : 0xd9d4be
//     if !(tx==0 && ty==0) && (i8)event[ty*w+tx] > 0 -> 0x225acc   (0x17a6b4)
// with `attr = *(*(MAP+0x60)) + 8` (u8[w*h]) and `event = *(*(MAP+0x68)) + 8`
// (i8[w*h]).
//
// ── THE SAME CHAIN ON LGT (2026-08-10) ──────────────────────────────────────
// Every `*(*(x))+8` above is a WIPI allocation handle, and that indirection belongs
// to the CARRIER RUNTIME, not to the game: `wie_ktf`'s `WIPICContext::data_ptr` is
// `*(handle) + 8` while `wie_lgt`'s is the identity. So on LGT the identical game
// source produces the same chain with ONE hop per link:
//     MAP = *(CHAR+0x9c90) ; attr = *(MAP+0x60) ; event = *(MAP+0x68)
// [`MapChain::data_off`] is that switch, and it is the only build-specific thing in
// this file — the tile colours, the supersample and the blend are game logic and
// therefore shared. `wie_probe --map-check` proves it per build instead of assuming.
//
// ── WHY THESE ARRAYS ARE SAFE TO READ AT ANY TIME ───────────────────────────
// They are not map-window state. `0x17a6f8` has five callers and `0x17a6b4` six,
// and only one each belongs to the minimap — the rest are the field's own collision
// and warp checks, which run while the player walks. If they were ever stale the
// player would walk through walls. That is what makes it possible to draw the
// minimap without opening (or even touching) the guest's map window.

/// `MAP = *(*(CHAR + MAP_HANDLE_OFF)) + 8` on KTF, `*(CHAR + MAP_HANDLE_OFF)` on LGT
/// — see [`MapChain::data_off`]. The per-map object.
pub const MAP_HANDLE_OFF: u32 = 0x9c90;
const MAP_W_OFF: u32 = 0x48;
const MAP_H_OFF: u32 = 0x4c;
/// Handle of the tile-attribute array (`u8[w*h]`); bit 7 = blocked.
const MAP_ATTR_OFF: u32 = 0x60;
/// Handle of the tile-event array (`i8[w*h]`); > 0 = warp / event tile.
const MAP_EVENT_OFF: u32 = 0x68;
const ATTR_BLOCKED: u8 = 0x80;

/// The build-specific half of the pointer chain above.
///
/// Assembled by the caller rather than read from `guest_build` directly, because this
/// file is `include!`d into two crates (`wie_android::items` and `wie_probe`) whose
/// module paths differ. Both fields come straight out of `guest_anchors.rs`, so
/// there is one place where the per-build VALUES live and one line of selection at
/// each call site.
#[derive(Clone, Copy)]
pub struct MapChain {
    /// `Some(off)`: every container field in the chain is a WIPI HANDLE whose payload
    /// is `*(handle) + off` (KTF — `wie_ktf`'s `data_ptr` is `*(h) + 8`).
    /// `None`: the fields are already payload addresses (LGT — `wie_lgt`'s `data_ptr`
    /// is the identity, so a WIPI allocation IS its data address).
    pub data_off: Option<u32>,
    /// Address of the global that suppresses the corner blend, when known for this
    /// build; `None` = never suppress. See [`NOBLEND_NEVER`].
    #[allow(dead_code)]
    pub noblend_global: Option<u32>,
}

/// Sentinel used when `noblend_global` is `None`: no tile colour can equal it, so the
/// blend guard never fires. On KTF the real global holds none of the three tile
/// colours either, so this is the same behaviour — and `--map-check` is what proves
/// it on any build.
#[allow(dead_code)]
const NOBLEND_NEVER: i32 = -1;

/// The three tile colours, exactly as the guest's `tile_colour` produces them
/// (`0x00RRGGBB`; the guest splits them `>>16`, `>>8`, `&0xff` into FillRect).
#[allow(dead_code)]
const C_OPEN: i32 = 0x00d9_d4be;
#[allow(dead_code)]
const C_BLOCKED: i32 = 0x005b_5b5b;
#[allow(dead_code)]
const C_EVENT: i32 = 0x0022_5acc;

/// The object the map window reads the player's tile position from, and the two
/// bytes inside it. From the marker draw at guest 0x17b7e4, in the same function
/// that blits the minimap buffer to the screen:
///     obj = *(*(CHAR+0x9bd0)) + 8          (cached into CHAR+0x9bcc at 0x17b6f0)
///     FillRect(screen, base_x + (u8)obj[0x27] * 2,
///                      base_y + (u8)obj[0x28] * 2, 2, 2, 0xec, 0x2d, 0x2d)
/// i.e. a red 2x2 dot covering exactly the tile's 2x2 block in buffer space, which
/// is why [`stamp_player`] can place it without knowing the on-screen origin.
///
/// ★ Resolved through 0x9bd0 rather than reading the 0x9bcc cache: that cache is
/// only written BY the map window, so before the player has ever opened it the
/// cached pointer is stale or absent — and never opening that window is the whole
/// point of this file.
const PLAYER_OBJ_HANDLE_OFF: u32 = 0x9bd0;
const PLAYER_TILE_X_OFF: u32 = 0x27;
const PLAYER_TILE_Y_OFF: u32 = 0x28;
/// The PIXEL pair in the same object, which is what the game's own arrival placement
/// works in: the warp handler calls `0x56134`, which loads `(i16)[obj+0x10]` and
/// `(i16)[obj+0x12]` and hands them to `0x560a4` together with the tile size (16) and
/// its half (8). Measured in a probe run: `x = tile_x * 16 + 8` (column centre) and
/// `y = tile_y * 16` (the feet). The tile bytes above are NOT derived from these on the
/// fly, so anything that moves a player has to write both pairs.
const PLAYER_PX_X_OFF: u32 = 0x10;
const PLAYER_PX_Y_OFF: u32 = 0x12;
/// The marker's colour and size, as the guest draws them.
#[allow(dead_code)]
const C_PLAYER: u32 = 0x00ec_2d2d;
#[allow(dead_code)]
const PLAYER_DOT: i32 = 2;

/// Guest heap window, for validating every pointer hop.
const MAP_HEAP_LO: u32 = 0x4000_0000;
const MAP_HEAP_HI: u32 = 0x5000_0000;

/// Sanity ceiling on the tile grid. Hero3 maps are 176..1120 px wide at 16 px per
/// tile, i.e. 11..70 tiles; past this the pointer chain landed on something that is
/// not a map, and the render is refused rather than allocating on a garbage size.
const TILES_MAX: i32 = 200;

/// `(pixel_w, pixel_h, 0x00RRGGBB pixels)` of the current map's minimap, or `None`
/// when the map object is not resolvable (title screen, mid-load).
#[allow(dead_code)]
pub fn render_minimap_rgb(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, chain: MapChain) -> Option<(u16, u16, Vec<u32>)> {
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    // Each hop is validated: a stale or half-swapped handle must yield `None`, not a
    // render of whatever the pointer happened to land on. On a direct-pointer build
    // there is one hop instead of two, and the same in-heap test guards it.
    let deref = |p: u32| -> Option<u32> {
        let h = u32at(p).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
        match chain.data_off {
            Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v)),
            None => Some(h),
        }
    };

    let map = deref(char_ptr + MAP_HANDLE_OFF)?;
    let mw = u32at(map + MAP_W_OFF)? as i32;
    let mh = u32at(map + MAP_H_OFF)? as i32;
    if !(1..=TILES_MAX).contains(&mw) || !(1..=TILES_MAX).contains(&mh) {
        return None;
    }
    let n = (mw * mh) as usize;
    let arr = |off: u32| -> Option<Vec<u8>> {
        let p = deref(map + off)?;
        let b = read(p, n);
        (b.len() == n).then_some(b)
    };
    let attr = arr(MAP_ATTR_OFF)?;
    let event = arr(MAP_EVENT_OFF)?;
    let noblend = chain.noblend_global.and_then(u32at).map_or(NOBLEND_NEVER, |v| v as i32);

    let tile = |tx: i32, ty: i32| -> i32 {
        if tx < 0 || ty < 0 || tx >= mw || ty >= mh {
            return -1; // the guest's out-of-bounds sentinel
        }
        let i = (ty * mw + tx) as usize;
        // ★ tile (0,0) is exempt from the event colour in the guest, so it is here.
        if !(tx == 0 && ty == 0) && (event[i] as i8) > 0 {
            C_EVENT
        } else if attr[i] & ATTR_BLOCKED != 0 {
            C_BLOCKED
        } else {
            C_OPEN
        }
    };

    let (w, h) = (mw * 2, mh * 2);
    let mut px = Vec::with_capacity((w * h) as usize);
    for py in 0..h {
        for pxx in 0..w {
            let (tx, ty) = (pxx >> 1, py >> 1);
            let base = tile(tx, ty);
            let nx = if pxx & 1 != 0 { tx + 1 } else { tx - 1 };
            let ny = if py & 1 != 0 { ty + 1 } else { ty - 1 };
            let c1 = match tile(nx, ty) {
                -1 => base,
                v => v,
            };
            let c2 = match tile(tx, ny) {
                -1 => base,
                v => v,
            };
            px.push(if base != c1 && base != c2 && base != noblend {
                let ch = |s: u32| -> u32 {
                    let sum = ((base as u32 >> s) & 0xff) + ((c1 as u32 >> s) & 0xff) + ((c2 as u32 >> s) & 0xff);
                    (sum / 3).min(0xff)
                };
                ch(0) | (ch(8) << 8) | (ch(16) << 16)
            } else {
                base as u32
            });
        }
    }
    Some((w as u16, h as u16, px))
}

/// The player's tile, as the map window's marker uses it — `None` when the object
/// is not resolvable or the tile is outside the map (which is how a half-swapped
/// pointer shows up, and a dot in the wrong place is worse than no dot).
#[allow(dead_code)]
pub fn player_tile(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, map_w: i32, map_h: i32, chain: MapChain) -> Option<(i32, i32)> {
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let first = u32at(char_ptr + PLAYER_OBJ_HANDLE_OFF).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
    let obj = match chain.data_off {
        Some(off) => u32at(first).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?,
        None => first,
    };
    let byte = |off: u32| -> Option<i32> {
        let b = read(obj + off, 1);
        b.first().map(|v| i32::from(*v))
    };
    let (tx, ty) = (byte(PLAYER_TILE_X_OFF)?, byte(PLAYER_TILE_Y_OFF)?);
    (tx < map_w && ty < map_h).then_some((tx, ty))
}

/// Stamp the player marker into a buffer from [`render_minimap_rgb`].
///
/// Kept separate because the guest draws this dot onto the SCREEN, after blitting
/// the buffer — so the buffer itself never contains it, and `wie_probe --map-check`
/// has to compare the un-stamped render to stay a like-for-like diff.
#[allow(dead_code)]
pub fn stamp_player(px: &mut [u32], w: u16, h: u16, tile: (i32, i32)) {
    let (w, h) = (i32::from(w), i32::from(h));
    for dy in 0..PLAYER_DOT {
        for dx in 0..PLAYER_DOT {
            let (x, y) = (tile.0 * 2 + dx, tile.1 * 2 + dy);
            if (0..w).contains(&x) && (0..h).contains(&y) {
                px[(y * w + x) as usize] = C_PLAYER;
            }
        }
    }
}

// ── the tile classes the hero5-style view draws (141차, 2026-09-30) ───────────────
//
// `minimap_view.rs` (shared with hero2) draws a fixed-zoom window of TILE CLASSES —
// wall / floor / exit — instead of the game's own two-pixels-per-tile picture. The
// classes come from the same two arrays `render_minimap_rgb` reads, by the same rules
// the game's own `tile_colour` uses: blocked = `attr & 0x80`, an exit = a tile whose
// event byte is positive (the game paints those blue), everything else floor.
// Repeated here rather than named from `minimap_view.rs` because neither file may
// name the other (both are `include!`d into dependency-free crates); `overlay.rs`
// asserts at compile time that the two agree.
pub const MM_TILE_WALL: u8 = 0;
pub const MM_TILE_FLOOR: u8 = 1;
pub const MM_TILE_EXIT: u8 = 2;

/// `(tiles_w, tiles_h, classes)` of the current map, or `None` when the map object is
/// not resolvable (title screen, mid-load). Same pointer discipline as
/// [`render_minimap_rgb`]: every hop is validated, a stale handle yields `None`.
pub fn minimap_tiles(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, chain: MapChain) -> Option<(u16, u16, Vec<u8>)> {
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let deref = |p: u32| -> Option<u32> {
        let h = u32at(p).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
        match chain.data_off {
            Some(off) => u32at(h).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v)),
            None => Some(h),
        }
    };
    let map = deref(char_ptr + MAP_HANDLE_OFF)?;
    let mw = u32at(map + MAP_W_OFF)? as i32;
    let mh = u32at(map + MAP_H_OFF)? as i32;
    if !(1..=TILES_MAX).contains(&mw) || !(1..=TILES_MAX).contains(&mh) {
        return None;
    }
    let n = (mw * mh) as usize;
    let arr = |off: u32| -> Option<Vec<u8>> {
        let p = deref(map + off)?;
        let b = read(p, n);
        (b.len() == n).then_some(b)
    };
    let attr = arr(MAP_ATTR_OFF)?;
    let event = arr(MAP_EVENT_OFF)?;
    let cells = (0..n)
        .map(|i| {
            // ★ tile (0,0) is exempt from the event colour in the guest, so it is here.
            if i != 0 && (event[i] as i8) > 0 {
                MM_TILE_EXIT
            } else if attr[i] & ATTR_BLOCKED != 0 {
                MM_TILE_WALL
            } else {
                MM_TILE_FLOOR
            }
        })
        .collect();
    Some((mw as u16, mh as u16, cells))
}

/// The player's position in TILES, fractional, at the centre of the tile the pixel
/// pair says (`x = tile*16 + 8` is the column centre already; `y = tile*16` is the feet,
/// so half a tile is added). `None` when the object is not resolvable.
pub fn player_marker(read: &impl Fn(u32, usize) -> Vec<u8>, char_ptr: u32, chain: MapChain) -> Option<(f32, f32)> {
    let u32at = |a: u32| -> Option<u32> {
        let b = read(a, 4);
        (b.len() == 4).then(|| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let first = u32at(char_ptr + PLAYER_OBJ_HANDLE_OFF).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?;
    let obj = match chain.data_off {
        Some(off) => u32at(first).map(|v| v.wrapping_add(off)).filter(|v| (MAP_HEAP_LO..MAP_HEAP_HI).contains(v))?,
        None => first,
    };
    let i16at = |a: u32| -> Option<i16> {
        let b = read(a, 2);
        (b.len() == 2).then(|| i16::from_le_bytes([b[0], b[1]]))
    };
    let (px, py) = (i16at(obj + PLAYER_PX_X_OFF)?, i16at(obj + PLAYER_PX_Y_OFF)?);
    Some((f32::from(px) / 16.0, f32::from(py) / 16.0 + 0.5))
}
