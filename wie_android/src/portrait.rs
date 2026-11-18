// The PORTRAIT — the player's own head for the status HUD's ring (141차, 2026-09-30;
// made a FIXED picture of the worn look in 153차, 2026-10-02).
//
// ── WHAT THE PLAYER ASKED FOR ────────────────────────────────────────────────────
// 141차: *"좌측상단 캐릭터 외형 … hero3에도 적용하고싶어"* — hero2's hero5-style status HUD,
// whose ring holds the character's own field head with the helmet showing.
// 153차: *"이게 게임 화면을 캡쳐하는건지 인게임 플레이중에 계속 뭔가 바뀌고있어. 이거 그냥
// 장비장착한 외형따라 고정되어야해. 인게임 진행되면서 장비를 바꿔서 외형이 바뀔때말고는
// 바뀌면안돼."* — the ring is the worn look, and nothing but a change of that look moves it.
//
// ── HOW HERO3 GETS THE PICTURE ────────────────────────────────────────────────────
// hero3's character is a PAPER DOLL of eight layers (139차: head, armour, gloves, boots,
// weapon, helmet, shield — each a `res/hero/*_bm` sprite, most recoloured through a
// `ph*_pa` palette the item row names), composed by the game's own state machine with
// per-layer offsets from the `*_cif` animation tables. Rather than re-implement that,
// the host READS THE COMPOSITION OFF THE FRAME: every sprite blit goes through the host
// blitter (`wie_core_arm::binary_patches::blit565`), which copies the player's own
// blits — the run that follows the player's SHADOW — into a small window around the
// player's draw corner (`wie_util::stats::PORTRAIT_PX`). That buffer is the player
// exactly as drawn — hair palette, helmet, everything — with alpha 0 where nothing
// was blitted. This file is the pure rules around it: where the window and the shadow
// go on the framebuffer, which square of it is the face, which frame may be taken, and
// when a taken picture may replace the one in the ring.
//
// ── WHY THE 141차 RING NEVER STOOD STILL (measured, `wie_probe --portrait --anim`) ──
// ① The idle pose BREATHES: four frames, one per tick (`CHAR+0x9bc8` 0,1,2,3, …), the
// head bobbing a pixel and both arms swinging — and the worker took a new picture on
// every frame of it, so the ring cycled four pictures for as long as the player stood.
// ② A HELMET was never in it: the helmet layer is drawn after the head, 18×26 at the
// draw corner −39, and the 141차 window began at −36 — a blit not wholly inside the
// window is not copied, so a helmeted player's ring showed the bare head.
// ③ A map narrower than the screen is CENTRED by the game (`0x16a90`, below), which
// `field_scroll` did not know — in every such room the window sat beside the player and
// nothing was ever captured, so the ring kept whatever the last field had left in it.
//
// ── WHAT THE FRAME LOOKS LIKE (measured, `wie_probe --blit-log`, three positions) ──
// The field is drawn world → screen with a SCROLL `S`: tiles first (16×16 from the
// theme strip), then decorations and actors in depth order — each actor as its SHADOW
// (26×17, through the blended blitter) followed by its pieces, the HEAD LAST (a helmet
// after the head). The player's shadow lands at the draw corner + (−5, −6) and the head
// 33..34 px above the corner (18×21 facing the viewer, 20×21 sideways). Everything else
// the blitter sees is excluded by the shadow trigger and by the next actor's shadow
// ending the run, not by geometry — see `wie_util::stats::portrait_takes`.
//
// ── WHY A FILE OF ITS OWN ─────────────────────────────────────────────────────────
// Dependency-free: `items.rs` `include!`s it for the phone, `wie_probe --portrait`
// `include!`s it to dump the very same window headless, and `portrait_selftest.rs` tests
// it with plain `rustc`. No `use crate::…`.

/// The window is anchored on the player's DRAW CORNER (`obj+0x16`/`+0x18`, the corner
/// the painter blits the body from) ON THE FRAMEBUFFER: this many px LEFT of it and
/// ABOVE it. Measured with `wie_probe --blit-log`: the head lands at the corner + (0..−2,
/// −33..−34) and is 18..20 × 21, a helmet at the corner + (0, −39) and 18 × 26 (all
/// seventeen `res/hero/h600NN_bm` are the same template: 23 × 28 at most), the body
/// pieces at the corner and below.
/// ★ 153차: 36 → 44 — at 36 the helmet's top six rows fell outside and the helmet was
/// never copied. The window is `wie_util::stats::PORTRAIT_W × PORTRAIT_H`.
pub const PORTRAIT_LEFT: i32 = 16;
pub const PORTRAIT_UP: i32 = 44;

/// The player's SHADOW blit — the trigger that opens a capture run: `SHADOW_W × SHADOW_H`
/// at the framebuffer draw corner + `(SHADOW_DX, SHADOW_DY)` (three positions, all
/// exact). `SHADOW_SLOP` px of tolerance either way for a mid-step scroll.
pub const SHADOW_W: i32 = 26;
pub const SHADOW_H: i32 = 17;
pub const SHADOW_DX: i32 = -5;
pub const SHADOW_DY: i32 = -6;
pub const SHADOW_SLOP: i32 = 2;
/// How many blits after the shadow may belong to the player (measured: five pieces,
/// the head last, a helmet a sixth). The run also ENDS at the next actor's shadow
/// (153차), so the cap only matters between the player and a decoration.
pub const CAPTURE_RUN: u32 = 8;

/// Where to arm the capture window for a player whose draw corner is at framebuffer
/// `(draw_x, draw_y)`.
pub fn portrait_origin(draw_x: i32, draw_y: i32) -> (i32, i32) {
    (draw_x - PORTRAIT_LEFT, draw_y - PORTRAIT_UP)
}

/// Where that player's shadow blit lands.
pub fn shadow_anchor(draw_x: i32, draw_y: i32) -> (i32, i32) {
    (draw_x + SHADOW_DX, draw_y + SHADOW_DY)
}

/// The field scroll the painter adds to every world px — the game's own rule, read off
/// the field painter (`0x16a90`), per axis:
///
/// ```text
///     s = min(cam, 0);  s = max(s, screen − map);  if s > 0 { s = (screen − map) >> 1 }
/// ```
///
/// i.e. the camera clamped so the map never shows past its edge, and a map NARROWER
/// (or shorter) than the screen CENTRED on it. ⛔ 141차 left the small map at 0 ("the
/// game does not centre it") — it does: a 288-px room on the 496-px screen draws the
/// player 104 px right of where that put the window, and nothing was ever captured
/// there. `cam` is whichever camera the game is following (`items::portrait_camera` —
/// the 「화면 이동」 option picks the smoothed or the target one); sizes in px.
pub fn field_scroll(cam: (i32, i32), map_px: (i32, i32), screen_px: (i32, i32)) -> (i32, i32) {
    let axis = |c: i32, map: i32, screen: i32| {
        let s = c.min(0).max(screen - map);
        if s > 0 { (screen - map) >> 1 } else { s }
    };
    (axis(cam.0, map_px.0, screen_px.0), axis(cam.1, map_px.1, screen_px.1))
}

/// Does a blit of `w × h` at `(x, y)` look like the player's shadow at `anchor`?
pub const fn is_shadow(x: i32, y: i32, w: i32, h: i32, anchor: (i32, i32)) -> bool {
    w == SHADOW_W && h == SHADOW_H && (x - anchor.0).abs() <= SHADOW_SLOP && (y - anchor.1).abs() <= SHADOW_SLOP
}

/// The bounding box of everything drawn into the window — `(x, y, w, h)`, `None` when
/// nothing was. A diagnostic for `wie_probe --portrait`; the ring takes [`face_crop`].
///
/// `px` is `w × h`, row-major `0xAARRGGBB`, alpha 0 = empty.
#[allow(dead_code)] // the probe's diagnostic; the phone shows `face_crop`
pub fn drawn_box(px: &[u32], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    if w == 0 || h == 0 || px.len() != (w * h) as usize {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for y in 0..h {
        for x in 0..w {
            if px[(y * w + x) as usize] >> 24 != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0).then(|| (x0, y0, x1 - x0, y1 - y0))
}

// ── the FACE square ──────────────────────────────────────────────────────────────

/// The picture the ring shows is this square — `hud_view::HV_RING`, so it lands on the
/// ring one to one (the ring centres what it is given, and shows the disc of radius 8
/// in the middle of it).
pub const FACE_SIZE: u32 = 24;
/// …centred this far from the draw corner: the middle of the head blit facing the
/// viewer (18 wide at the corner, so 9 across) and a little below its middle row, so
/// the disc (rows −28..−12) holds the face whole with the hair or the helmet above it
/// and the collar under it — 141차's framing on its idle frame (−19) moved up a row so
/// a helmet's crest shows (chosen off previews of both, −22..−18). ★ FIXED, never the
/// top of whatever is opaque (141차): a helmet or a spear tip above the head pushed the
/// face down and out of the disc, and the breathing frames moved it a pixel.
pub const FACE_DX: i32 = 9;
pub const FACE_DY: i32 = -20;

/// The face square's top-left inside the capture window.
pub const fn face_origin() -> (i32, i32) {
    let half = (FACE_SIZE / 2) as i32;
    (PORTRAIT_LEFT + FACE_DX - half, PORTRAIT_UP + FACE_DY - half)
}

/// The face square cut out of a captured window (`w × h`, row-major ARGB, alpha 0 =
/// empty): `(FACE_SIZE, FACE_SIZE, pixels)`, or `None` when the window is malformed, the
/// square does not fit in it, or nothing at all was drawn inside the square.
pub fn face_crop(px: &[u32], w: u32, h: u32) -> Option<(u32, u32, Vec<u32>)> {
    if w == 0 || h == 0 || px.len() != (w * h) as usize {
        return None;
    }
    let (fx, fy) = face_origin();
    if fx < 0 || fy < 0 || fx as u32 + FACE_SIZE > w || fy as u32 + FACE_SIZE > h {
        return None;
    }
    let (fx, fy) = (fx as u32, fy as u32);
    let mut out = Vec::with_capacity((FACE_SIZE * FACE_SIZE) as usize);
    for y in fy..fy + FACE_SIZE {
        out.extend_from_slice(&px[(y * w + fx) as usize..(y * w + fx + FACE_SIZE) as usize]);
    }
    out.iter().any(|p| p >> 24 != 0).then_some((FACE_SIZE, FACE_SIZE, out))
}

/// Is the face square of a window at `origin` wholly on a `screen_w × screen_h`
/// framebuffer? Near the screen's edge the game clips the pieces, and a picture taken
/// there would be a cut head.
pub fn face_on_screen(origin: (i32, i32), screen_w: i32, screen_h: i32) -> bool {
    let (fx, fy) = face_origin();
    let (x0, y0) = (origin.0 + fx, origin.1 + fy);
    x0 >= 0 && y0 >= 0 && x0 + FACE_SIZE as i32 <= screen_w && y0 + FACE_SIZE as i32 <= screen_h
}

// ── WHICH FRAME may be taken ─────────────────────────────────────────────────────

/// The player state byte's value for standing still (`items::PLAYER_STATE_IDLE`).
pub const IDLE_STATE: u8 = 0;
/// The facing byte's value for "towards the viewer" (`CHAR+0x9bb4`: 1 up · 2 down · 3
/// left · 4 right) — the face the ring shows; any other way is the side or the back.
pub const FACING_VIEWER: u8 = 2;
/// The idle loop's frame the ring is taken from. The pose frame counter
/// (`CHAR+0x9bc8`, i16) runs 0, 1, 2, 3, 0, … one per tick while the player stands
/// (probe, both heroes) and each value is one breathing frame, so taking ONLY this one
/// makes every picture of the same look the same picture. 0 is the loop's own first
/// frame — it exists whatever the loop's length.
pub const CANONICAL_POSE_TICK: u16 = 0;

/// May the frame just drawn be the ring's picture? Standing still, facing the viewer,
/// on the idle loop's first frame.
pub const fn canonical_frame(state: u8, facing: u8, pose_tick: u16) -> bool {
    state == IDLE_STATE && facing == FACING_VIEWER && pose_tick == CANONICAL_POSE_TICK
}

// ── WHEN a picture may replace the one in the ring ───────────────────────────────

/// The worn LOOK as one number: the class byte (`CHAR+0xb6` — which hero's base body
/// and hair) and every worn slot's `(cat, idx)` in slot order (`-1` = empty). The paper
/// doll is built from exactly these (`0x517bc` reads the worn item's row; the world's
/// init re-dresses from the worn array), so two equal keys are two equal looks. FNV-1a.
/// An accessory changes the key without changing the picture — that costs one retake of
/// the same picture, which the ring cannot tell from no change at all.
pub fn look_key(class: u16, worn: &[(i16, i16)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for b in class.to_le_bytes() {
        eat(b);
    }
    eat(worn.len() as u8);
    for (cat, idx) in worn {
        for b in cat.to_le_bytes().into_iter().chain(idx.to_le_bytes()) {
            eat(b);
        }
    }
    h
}

/// ★ 167차 아바타 옷장 — the look with the wardrobe's six WORN bytes folded in (the save's flag
/// array, `wie_util::avatar::FLAG_WEAR`): a costume changes the paper doll exactly like a new piece
/// of equipment, so it must change the key. Folded in ONLY when one is set — a character without a
/// costume keeps the key (and the remembered picture) it had before 167차.
pub fn costume_key(look: u64, worn_sets: &[u8]) -> u64 {
    if worn_sets.iter().all(|&b| b == 0) {
        return look;
    }
    let mut h = look ^ 0x0a7a_7a2c_0d7e_5e75;
    for &b in worn_sets {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Identical canonical frames needed at one spot before a picture counts as seen there —
/// two is one whole breathing loop apart, which a passing monster or a half-scrolled
/// camera does not survive.
pub const CONFIRM_FRAMES: u32 = 2;
/// Distinct spots a picture must have been seen at before it is FINAL. Until then a
/// picture seen at another spot replaces it: the one thing that can make a capture wrong
/// with every other test passed is a decoration drawn over the player right where they
/// stood (a lamp post just in front), and that does not follow the player to a second
/// tile. Once final, nothing but a new look replaces it.
pub const FINAL_SPOTS: usize = 2;

/// The ring's picture, kept per LOOK (153차). The worker offers it a picture from every
/// canonical frame (`offer`); it keeps the first one seen [`CONFIRM_FRAMES`] times at a
/// spot, lets a different one seen at ANOTHER spot replace it until two spots agree
/// ([`FINAL_SPOTS`]), and from then on ignores everything until the look changes. A new
/// look keeps the old picture up until its own is confirmed (the ring never blanks over
/// an equipment change), and a character change ([`Self::suspect`]: a load, the title)
/// hides it until the next look on the field is known ([`Self::observe`]) — kept when it
/// is the same look, dropped when it is not.
///
/// ★ It REMEMBERS ([`MEMORY_LOOKS`] looks, most recent first, saved by the worker with
/// [`encode_memory`]): a look seen before — this session or any earlier one — goes up
/// the moment it is on the field again, facing whichever way. Without it a save loaded
/// facing away from the viewer had an empty ring until the player turned round (player,
/// 2026-10-02: *"세이브를 로드 후에 움직이기 전까지 외형표시가 안나오고있는데?"*) — the
/// front view only exists while the player faces the viewer.
///
/// `P` is the picture (`crate::Portrait` on the phone); generic so this stays pure.
#[derive(Debug, Clone)]
pub struct PortraitKeeper<P> {
    /// The look `shown` was taken of.
    look: Option<u64>,
    shown: Option<P>,
    /// The spots (player tiles) `shown` was confirmed at this time round.
    spots: Vec<(i32, i32)>,
    /// Two spots agreed (now or when it was remembered): nothing replaces it.
    settled: bool,
    /// The picture being confirmed: its look, its spot, how many canonical frames in a
    /// row showed it there.
    candidate: Option<(u64, (i32, i32), P, u32)>,
    /// A character may have been swapped since `shown` was taken.
    stale: bool,
    /// `(look, settled, picture)` for every look seen, most recent first.
    memory: Vec<(u64, bool, P)>,
    /// `memory` changed since the worker last saved it.
    memory_dirty: bool,
}

/// What [`PortraitKeeper::observe`] did with the look on the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Observed {
    /// The ring already shows this look, or nothing could change.
    Same,
    /// A look seen before: its remembered picture went up at once.
    Recalled { settled: bool },
    /// After a character change, a look never seen: nothing is shown until it is captured.
    Dropped,
    /// A new look in play, never seen: the old picture stays until the new one is captured.
    Waiting,
}

impl<P: PartialEq + Clone> Default for PortraitKeeper<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: PartialEq + Clone> PortraitKeeper<P> {
    pub const fn new() -> Self {
        Self {
            look: None,
            shown: None,
            spots: Vec::new(),
            settled: false,
            candidate: None,
            stale: false,
            memory: Vec::new(),
            memory_dirty: false,
        }
    }

    /// What the ring shows — nothing while a character change is unsettled.
    pub fn shown(&self) -> Option<&P> {
        if self.stale { None } else { self.shown.as_ref() }
    }

    /// The look the shown picture was taken of.
    #[allow(dead_code)] // the probe's summary and the tests
    pub fn look(&self) -> Option<u64> {
        self.look
    }

    /// Is the shown picture final for this look (nothing will replace it)?
    pub fn is_final(&self, look: u64) -> bool {
        !self.stale && self.shown.is_some() && self.look == Some(look) && self.settled
    }

    /// Would a picture of `look` be looked at? — so the worker can skip the snapshot.
    pub fn wants(&self, look: u64) -> bool {
        !self.is_final(look)
    }

    /// A character may have changed (a record was loaded, the title was reached): the
    /// picture is hidden until [`Self::observe`] knows the look on the field.
    pub fn suspect(&mut self) {
        self.stale = true;
        self.candidate = None;
    }

    /// The look on the field is `look`. A suspicion ends — the picture stays when it is
    /// of this look (the same character back, or another one dressed the same) and goes
    /// when it is not (never someone else's head over this character's bars) — and a
    /// look seen before is recalled at once, whichever way the player is facing.
    pub fn observe(&mut self, look: u64) -> Observed {
        let was_stale = std::mem::replace(&mut self.stale, false);
        if self.look == Some(look) && self.shown.is_some() {
            return Observed::Same;
        }
        if let Some(i) = self.memory.iter().position(|(l, _, _)| *l == look) {
            let entry = self.memory.remove(i);
            let settled = entry.1;
            self.look = Some(look);
            self.shown = Some(entry.2.clone());
            self.settled = settled;
            self.spots.clear();
            self.candidate = None;
            self.memory.insert(0, entry);
            self.memory_dirty |= i != 0;
            return Observed::Recalled { settled };
        }
        if was_stale {
            self.look = None;
            self.shown = None;
            self.spots.clear();
            self.settled = false;
            return Observed::Dropped;
        }
        if self.shown.is_none() { Observed::Same } else { Observed::Waiting }
    }

    /// A picture from a canonical frame of `look`, with the player at `spot`. Returns
    /// whether the shown picture changed.
    pub fn offer(&mut self, look: u64, spot: (i32, i32), picture: P) -> bool {
        let recalled = matches!(self.observe(look), Observed::Recalled { .. });
        if self.is_final(look) {
            self.candidate = None;
            return recalled;
        }
        // Count it towards the candidate, or start a new one.
        let hits = match &mut self.candidate {
            Some((l, s, p, n)) if *l == look && *s == spot && *p == picture => {
                *n += 1;
                *n
            }
            _ => {
                self.candidate = Some((look, spot, picture.clone(), 1));
                1
            }
        };
        if hits < CONFIRM_FRAMES {
            return recalled;
        }
        self.candidate = None;
        if self.look != Some(look) || self.shown.is_none() {
            // A new look (or none yet): the first confirmed picture goes up at once.
            self.look = Some(look);
            self.shown = Some(picture);
            self.spots = vec![spot];
            self.settled = false;
            self.remember();
            return true;
        }
        if self.spots.contains(&spot) {
            // Seen again where it was already confirmed: nothing new is learned — and a
            // different picture at the SAME spot says the spot changed, not which is right.
            return recalled;
        }
        if self.shown.as_ref() == Some(&picture) {
            self.spots.push(spot);
            if self.spots.len() >= FINAL_SPOTS {
                self.settled = true;
                self.remember();
            }
            return recalled;
        }
        // Not final, and another spot shows something else: the first one was the spot's.
        self.shown = Some(picture);
        self.spots = vec![spot];
        self.remember();
        true
    }

    /// The shown picture into the memory, at the front.
    fn remember(&mut self) {
        let (Some(look), Some(picture)) = (self.look, self.shown.clone()) else { return };
        self.memory.retain(|(l, _, _)| *l != look);
        self.memory.insert(0, (look, self.settled, picture));
        self.memory.truncate(MEMORY_LOOKS);
        self.memory_dirty = true;
    }

    /// Every remembered look, most recent first — what the worker saves.
    pub fn memory(&self) -> &[(u64, bool, P)] {
        &self.memory
    }

    /// Whether the memory changed since the last call (the worker saves it then).
    pub fn take_memory_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.memory_dirty, false)
    }

    /// The memory as saved — at boot, before anything is shown.
    pub fn set_memory(&mut self, mut memory: Vec<(u64, bool, P)>) {
        memory.truncate(MEMORY_LOOKS);
        self.memory = memory;
        self.memory_dirty = false;
    }
}

/// Looks remembered across sessions — one per equipment combination worn; the oldest
/// falls out. 32 × 2.3 KB on disk.
pub const MEMORY_LOOKS: usize = 32;

/// The game asset's fingerprint (FNV-1a over the archive the APK carries): the memory is
/// thrown away when it changes — an update that recolours an item (139차) keeps the
/// item's `(cat, idx)` and so its look key, and would otherwise keep showing the old
/// colours forever.
pub fn asset_fingerprint(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

const MEMORY_MAGIC: &[u8; 4] = b"H3PR";
const MEMORY_VERSION: u8 = 1;

/// The memory file: `"H3PR" · version · asset fingerprint u64 · count u8 · count ×
/// (look u64 · settled u8 · w u8 · h u8 · w×h ARGB u32)`, all little-endian. Pictures
/// that are not `w × h` are skipped.
pub fn encode_memory(fingerprint: u64, entries: &[(u64, bool, u32, u32, &[u32])]) -> Vec<u8> {
    let entries: Vec<_> = entries
        .iter()
        .filter(|(_, _, w, h, px)| *w > 0 && *h > 0 && *w <= 255 && *h <= 255 && px.len() == (*w * *h) as usize)
        .take(MEMORY_LOOKS)
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(MEMORY_MAGIC);
    out.push(MEMORY_VERSION);
    out.extend_from_slice(&fingerprint.to_le_bytes());
    out.push(entries.len() as u8);
    for (look, settled, w, h, px) in entries {
        out.extend_from_slice(&look.to_le_bytes());
        out.push(u8::from(*settled));
        out.push(*w as u8);
        out.push(*h as u8);
        for p in *px {
            out.extend_from_slice(&p.to_le_bytes());
        }
    }
    out
}

/// The memory file back — empty when it is not one, is of another version, or was made
/// for another game asset (`fingerprint`). A truncated tail keeps the whole entries before it.
pub fn decode_memory(bytes: &[u8], fingerprint: u64) -> Vec<(u64, bool, u32, u32, Vec<u32>)> {
    let mut out = Vec::new();
    if bytes.len() < 14 || &bytes[..4] != MEMORY_MAGIC || bytes[4] != MEMORY_VERSION {
        return out;
    }
    let mut fp = [0u8; 8];
    fp.copy_from_slice(&bytes[5..13]);
    if u64::from_le_bytes(fp) != fingerprint {
        return out;
    }
    let count = bytes[13] as usize;
    let mut at = 14;
    for _ in 0..count.min(MEMORY_LOOKS) {
        if at + 11 > bytes.len() {
            break;
        }
        let mut lk = [0u8; 8];
        lk.copy_from_slice(&bytes[at..at + 8]);
        let look = u64::from_le_bytes(lk);
        let settled = bytes[at + 8] != 0;
        let (w, h) = (u32::from(bytes[at + 9]), u32::from(bytes[at + 10]));
        at += 11;
        let n = (w * h) as usize;
        if w == 0 || h == 0 || at + n * 4 > bytes.len() {
            break;
        }
        let px = bytes[at..at + n * 4].chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        at += n * 4;
        out.push((look, settled, w, h, px));
    }
    out
}

#[cfg(test)]
mod portrait_tests {
    use super::*;

    #[test]
    fn the_window_sits_up_and_left_of_the_draw_corner() {
        assert_eq!(portrait_origin(100, 200), (100 - PORTRAIT_LEFT, 200 - PORTRAIT_UP));
        // Near the map's top-left the origin goes negative and is left to the caller.
        assert_eq!(portrait_origin(0, 0), (-PORTRAIT_LEFT, -PORTRAIT_UP));
    }

    /// The three measured frames (496×240, map 35×25 = 560×400): standing at the spawn,
    /// walked right, walked down. The camera TARGET (`0x150d914`) is not the scroll when
    /// the game follows the smoothed one — the standing frame's target (56, −171) would
    /// put the map's edge on screen.
    #[test]
    fn the_scroll_is_the_camera_clamped_to_the_map() {
        let (map, screen) = ((560, 400), (496, 240));
        assert_eq!(field_scroll((54, -171), map, screen), (0, -160));
        assert_eq!(field_scroll((-74, -151), map, screen), (-64, -151));
        assert_eq!(field_scroll((82, -185), map, screen), (0, -160));
    }

    /// ★ 153차: a map narrower or shorter than the screen is CENTRED (`0x16a90`'s
    /// `(screen − map) >> 1`). Measured: the 18×17-tile room (288×272) of the second
    /// hero's start on the 496×240 screen, camera (4, −135): the player's shadow drew at
    /// (323, 178) for a draw corner of world (224, 216) — 104 = (496 − 288) / 2 across,
    /// −32 = 240 − 272 down.
    #[test]
    fn a_small_map_is_centred_on_the_screen() {
        assert_eq!(field_scroll((4, -135), (288, 272), (496, 240)), (104, -32));
        let (sx, sy) = field_scroll((4, -135), (288, 272), (496, 240));
        assert_eq!(shadow_anchor(224 + sx, 216 + sy), (323, 178));
        // Odd differences floor, the way `asrs #1` does.
        assert_eq!(field_scroll((0, 0), (287, 239), (492, 240)), (102, 0));
        // Both axes small: both centred, whatever the camera says.
        assert_eq!(field_scroll((-50, 30), (320, 200), (496, 240)), (88, 20));
        // Exactly the screen's size never scrolls.
        assert_eq!(field_scroll((-9, 9), (496, 240), (496, 240)), (0, 0));
    }

    /// Player draw corner on screen vs the blits the log showed: standing (176, 92) →
    /// shadow (171, 86), head (176, 59) 18×21; walked right (212, 81) → shadow (207, 75),
    /// head (210, 47) 20×21. The head and a helmet (corner −39, 18×26) must fall wholly
    /// inside the window; the shadow must match the anchor; an NPC shadow must not.
    #[test]
    fn the_shadow_is_the_trigger_and_head_and_helmet_fit_the_window() {
        let (ww, wh) = (48, 72); // `wie_util::stats::PORTRAIT_W × PORTRAIT_H`
        for (draw, shadow, head, hw) in [((176, 92), (171, 86), (176, 59), 18), ((212, 81), (207, 75), (210, 47), 20)] {
            assert_eq!(shadow_anchor(draw.0, draw.1), shadow);
            assert!(is_shadow(shadow.0, shadow.1, SHADOW_W, SHADOW_H, shadow_anchor(draw.0, draw.1)));
            let (ox, oy) = portrait_origin(draw.0, draw.1);
            let (lx, ly) = (head.0 - ox, head.1 - oy);
            assert!(lx >= 0 && ly >= 0, "head above/left of the window: {lx},{ly}");
            assert!(lx + hw <= ww && ly + 21 <= wh, "head past the window: {lx}+{hw}, {ly}+21");
        }
        // The helmet measured over the bare head at (176, 48) for the corner (176, 81).
        let (ox, oy) = portrait_origin(176, 81);
        let (hx, hy) = (176 - ox, 42 - oy);
        assert!(hx >= 0 && hy >= 0 && hx + 18 <= ww && hy + 26 <= wh, "the helmet must be wholly inside: {hx},{hy}");
        // An NPC's shadow a tile away is not the player's; a same-place blit of another size is not either.
        assert!(!is_shadow(171 + 16, 86, SHADOW_W, SHADOW_H, (171, 86)));
        assert!(!is_shadow(171, 86, 18, 21, (171, 86)));
        // Mid-step tolerance.
        assert!(is_shadow(173, 84, SHADOW_W, SHADOW_H, (171, 86)));
        assert!(!is_shadow(174, 86, SHADOW_W, SHADOW_H, (171, 86)));
    }

    /// The face square is fixed on the draw corner: centred on the head blit, inside the
    /// window, and the head (corner + (0, −33), 18×21) covers the ring's disc.
    #[test]
    fn the_face_square_is_fixed_on_the_head() {
        let (fx, fy) = face_origin();
        assert!(fx >= 0 && fy >= 0 && fx as u32 + FACE_SIZE <= 48 && fy as u32 + FACE_SIZE <= 72);
        // In corner terms: the square is corner + (−3, −32) .. + 24.
        assert_eq!((fx - PORTRAIT_LEFT, fy - PORTRAIT_UP), (-3, -32));
        // The head's middle column is the square's middle column.
        assert_eq!(fx - PORTRAIT_LEFT + (FACE_SIZE / 2) as i32, 9);
        // The disc (radius 8 about the square's centre) is all head rows: −28..−12.
        let centre_y = fy - PORTRAIT_UP + (FACE_SIZE / 2) as i32;
        assert!(centre_y - 8 >= -33 && centre_y + 8 <= -33 + 21);
    }

    #[test]
    fn the_face_crop_is_the_square_and_nothing_else() {
        let (w, h) = (48u32, 72u32);
        let mut px = vec![0u32; (w * h) as usize];
        let (fx, fy) = face_origin();
        let (fx, fy) = (fx as u32, fy as u32);
        // Something inside the square, something far outside it.
        px[((fy + 5) * w + fx + 7) as usize] = 0xffff_0000;
        px[(70 * w + 2) as usize] = 0xff00_ff00;
        let (cw, ch, crop) = face_crop(&px, w, h).unwrap();
        assert_eq!((cw, ch), (FACE_SIZE, FACE_SIZE));
        assert_eq!(crop[(5 * FACE_SIZE + 7) as usize], 0xffff_0000);
        assert_eq!(crop.iter().filter(|p| **p >> 24 != 0).count(), 1);
        // Nothing in the square is no picture, whatever is drawn elsewhere.
        let mut empty = vec![0u32; (w * h) as usize];
        empty[(70 * w + 2) as usize] = 0xff00_ff00;
        assert_eq!(face_crop(&empty, w, h), None);
        // A malformed or too-small window is not read.
        assert_eq!(face_crop(&px, w, h - 1), None);
        assert_eq!(face_crop(&px[..(w * 20) as usize], w, 20), None);
        // The diagnostic box spans both pixels.
        assert_eq!(drawn_box(&px, w, h), Some((2, fy + 5, fx + 8 - 2, 70 + 1 - (fy + 5))));
        assert_eq!(drawn_box(&vec![0u32; (w * h) as usize], w, h), None);
    }

    #[test]
    fn the_face_must_be_on_the_screen() {
        // The measured standing player, well inside 496×240.
        assert!(face_on_screen(portrait_origin(176, 81), 496, 240));
        // At the top edge the head is clipped by the game: no picture.
        assert!(!face_on_screen(portrait_origin(176, 20), 496, 240));
        // At the left edge too; the window itself may hang off as long as the face does not.
        assert!(!face_on_screen(portrait_origin(1, 81), 496, 240));
        assert!(face_on_screen(portrait_origin(3, 81), 496, 240));
        assert!(!face_on_screen(portrait_origin(480, 81), 496, 240));
    }

    /// Measured (`--anim` beside `--portrait`, every frame): the idle counter steps
    /// 0 → 1 → 2 → 3 → 0 one tick at a time and each value is one of FOUR different
    /// pictures. Only the first is the ring's.
    #[test]
    fn only_the_idle_loops_first_frame_facing_the_viewer_is_taken() {
        assert!(canonical_frame(0, 2, 0));
        for tick in 1..4 {
            assert!(!canonical_frame(0, 2, tick), "breathing frame {tick}");
        }
        for facing in [1, 3, 4] {
            assert!(!canonical_frame(0, facing, 0), "facing {facing}");
        }
        // Walking (1), hurt (3), dying (16): never.
        for state in [1, 3, 16] {
            assert!(!canonical_frame(state, 2, 0), "state {state}");
        }
    }

    #[test]
    fn the_look_key_is_the_class_and_every_worn_slot() {
        let bare = [(-1i16, -1i16); 9];
        let mut helm = bare;
        helm[1] = (0, 0x34);
        let k = look_key(5, &bare);
        assert_eq!(k, look_key(5, &bare), "a key is a function");
        assert_ne!(k, look_key(5, &helm), "a helmet is a new look");
        assert_ne!(k, look_key(0, &bare), "the other hero is a new look");
        // The same item in another slot is another look; so is a shorter array.
        let mut moved = bare;
        moved[3] = (0, 0x34);
        assert_ne!(look_key(5, &helm), look_key(5, &moved));
        assert_ne!(k, look_key(5, &bare[..8]));
        // cat and idx are not interchangeable.
        let mut swapped = bare;
        swapped[1] = (0x34, 0);
        assert_ne!(look_key(5, &helm), look_key(5, &swapped));
    }

    #[test]
    fn a_costume_is_a_new_look_and_no_costume_keeps_the_old_key() {
        let k = look_key(5, &[(-1i16, -1i16); 9]);
        assert_eq!(costume_key(k, &[0; 6]), k, "no costume: the pre-167차 key, so remembered pictures stay");
        let armour = costume_key(k, &[0, 2, 0, 0, 0, 0]);
        assert_ne!(armour, k);
        assert_ne!(armour, costume_key(k, &[0, 3, 0, 0, 0, 0]), "another set");
        assert_ne!(armour, costume_key(k, &[2, 0, 0, 0, 0, 0]), "another slot");
        assert_eq!(armour, costume_key(k, &[0, 2, 0, 0, 0, 0]));
    }

    type K = PortraitKeeper<u32>;

    /// The 141차 failure, end to end: a player standing still. The canonical frames all
    /// show the same picture; it goes up after two, and nothing after it changes the ring.
    #[test]
    fn standing_still_the_ring_takes_one_picture_and_keeps_it() {
        let mut k = K::new();
        assert!(!k.offer(7, (10, 10), 0xaa), "one frame is not yet a picture");
        assert_eq!(k.shown(), None);
        assert!(k.offer(7, (10, 10), 0xaa), "the second identical frame puts it up");
        assert_eq!(k.shown(), Some(&0xaa));
        for _ in 0..50 {
            assert!(!k.offer(7, (10, 10), 0xaa));
        }
        assert_eq!(k.shown(), Some(&0xaa));
        assert!(k.wants(7), "one spot is not final");
        // Seen the same at a second spot: final, and then not even looked at.
        assert!(!k.offer(7, (12, 10), 0xaa));
        assert!(!k.offer(7, (12, 10), 0xaa));
        assert!(k.is_final(7) && !k.wants(7));
        // ⛔ A final picture survives anything but a new look — even a different capture
        // confirmed at a third spot (the player under a tree, a hurt flash slipped by).
        for _ in 0..4 {
            assert!(!k.offer(7, (30, 30), 0xbb));
        }
        assert_eq!(k.shown(), Some(&0xaa));
    }

    #[test]
    fn a_flickering_capture_never_confirms() {
        let mut k = K::new();
        for i in 0..20 {
            assert!(!k.offer(7, (10, 10), if i % 2 == 0 { 0xaa } else { 0xbb }));
        }
        assert_eq!(k.shown(), None, "two pictures alternating is not one picture");
        // Nor does one that moves: every frame at another spot starts again.
        for x in 0..20 {
            assert!(!k.offer(7, (x, 10), 0xcc));
        }
        assert_eq!(k.shown(), None);
    }

    /// A lamp post in front of the player on the first spot: the ring shows it until the
    /// player is seen somewhere else — then the clean picture replaces it and, seen at a
    /// third spot, becomes final. The contaminated one never comes back.
    #[test]
    fn a_picture_spoiled_by_the_spot_is_replaced_at_the_next_spot() {
        let mut k = K::new();
        k.offer(7, (10, 10), 0xdead);
        assert!(k.offer(7, (10, 10), 0xdead));
        assert_eq!(k.shown(), Some(&0xdead));
        // Same spot, the lamp post still there — or gone: the spot changed, not the look.
        k.offer(7, (10, 10), 0xaa);
        assert!(!k.offer(7, (10, 10), 0xaa), "a different picture at the SAME spot proves nothing");
        assert_eq!(k.shown(), Some(&0xdead));
        // Another spot: the clean picture replaces it.
        k.offer(7, (14, 10), 0xaa);
        assert!(k.offer(7, (14, 10), 0xaa));
        assert_eq!(k.shown(), Some(&0xaa));
        assert!(!k.is_final(7), "replaced, so only one spot agrees");
        k.offer(7, (20, 3), 0xaa);
        k.offer(7, (20, 3), 0xaa);
        assert!(k.is_final(7));
        // Back at the lamp post: final, ignored.
        k.offer(7, (10, 10), 0xdead);
        assert!(!k.offer(7, (10, 10), 0xdead));
        assert_eq!(k.shown(), Some(&0xaa));
    }

    /// The equipment changed (a new key): the old picture stays up until the new look's is
    /// confirmed — then it changes, once, at the first spot.
    #[test]
    fn a_new_look_replaces_the_picture_once_confirmed() {
        let mut k = K::new();
        for s in [(1, 1), (1, 1), (5, 5), (5, 5)] {
            k.offer(7, s, 0xaa);
        }
        assert!(k.is_final(7));
        assert!(k.wants(8), "a new look is always looked at");
        assert!(!k.offer(8, (5, 5), 0xbb));
        assert_eq!(k.shown(), Some(&0xaa), "the old look stays up until the new one is confirmed");
        assert!(k.offer(8, (5, 5), 0xbb));
        assert_eq!((k.shown(), k.look()), (Some(&0xbb), Some(8)));
        // An accessory (a key change, the same picture): replaced by itself — no visible change.
        k.offer(9, (5, 5), 0xbb);
        assert!(k.offer(9, (5, 5), 0xbb));
        assert_eq!(k.shown(), Some(&0xbb));
    }

    /// A load or the title: the picture is hidden until the look on the field is known —
    /// then back if it is the same look, gone if it is not.
    #[test]
    fn a_character_change_never_shows_someone_elses_head() {
        let mut k = K::new();
        for s in [(1, 1), (1, 1), (5, 5), (5, 5)] {
            k.offer(7, s, 0xaa);
        }
        k.suspect();
        assert_eq!(k.shown(), None, "hidden while the world is not known");
        assert!(k.wants(7), "a suspicion is looked at");
        assert_eq!(k.observe(7), Observed::Same);
        assert_eq!(k.shown(), Some(&0xaa), "the same look: kept, still final");
        assert!(k.is_final(7));
        // Another character (another look, never seen) loaded.
        k.suspect();
        assert_eq!(k.observe(9), Observed::Dropped);
        assert_eq!(k.shown(), None, "never the last character's head");
        assert!(k.offer(9, (2, 2), 0xcc) == false && k.offer(9, (2, 2), 0xcc));
        assert_eq!(k.shown(), Some(&0xcc));
        // A suspicion settled by the first canonical frame itself works the same way.
        k.suspect();
        assert!(!k.offer(9, (3, 3), 0xcc));
        assert_eq!(k.shown(), Some(&0xcc), "the same look came back on the field");
    }

    /// ★ The player's report on the first device build: *"세이브를 로드 후에 움직이기
    /// 전까지 외형표시가 안나오고있는데?"* — a save loaded facing away from the viewer has
    /// no front view to take. A look seen before needs none: it goes up on the first look
    /// at the field, and a look the ring has never seen still waits for a front view.
    #[test]
    fn a_look_seen_before_goes_up_at_once_whichever_way_the_player_faces() {
        let mut k = K::new();
        for s in [(1, 1), (1, 1), (5, 5), (5, 5)] {
            k.offer(7, s, 0xaa);
        }
        // Another character, confirmed at one spot only.
        k.suspect();
        k.observe(9);
        k.offer(9, (2, 2), 0xcc);
        k.offer(9, (2, 2), 0xcc);
        // The first character loaded again, facing up: no canonical frame — and no need.
        k.suspect();
        assert_eq!(k.observe(7), Observed::Recalled { settled: true });
        assert_eq!(k.shown(), Some(&0xaa));
        assert!(k.is_final(7) && !k.wants(7), "remembered final is final");
        // In play the equipment goes back to a look seen before: up at once, not final.
        assert_eq!(k.observe(9), Observed::Recalled { settled: false });
        assert_eq!(k.shown(), Some(&0xcc));
        assert!(k.wants(9));
        // …and a remembered picture that is not final can still be corrected at another spot.
        k.offer(9, (6, 6), 0xcd);
        assert!(k.offer(9, (6, 6), 0xcd));
        assert_eq!(k.shown(), Some(&0xcd));
        // A look never seen, in play: the last picture stays up meanwhile.
        assert_eq!(k.observe(11), Observed::Waiting);
        assert_eq!(k.shown(), Some(&0xcd));
        // Observing the shown look again changes nothing.
        assert_eq!(k.observe(9), Observed::Same);
    }

    #[test]
    fn the_memory_keeps_the_most_recent_looks_and_says_when_to_save() {
        let mut k = K::new();
        assert!(!k.take_memory_dirty());
        for look in 0..40u64 {
            k.offer(look, (0, 0), look as u32 + 1);
            k.offer(look, (0, 0), look as u32 + 1);
        }
        assert!(k.take_memory_dirty());
        assert!(!k.take_memory_dirty(), "reported once");
        assert_eq!(k.memory().len(), MEMORY_LOOKS);
        assert_eq!(k.memory()[0].0, 39, "most recent first");
        assert!(k.memory().iter().all(|(l, _, _)| *l >= 40 - MEMORY_LOOKS as u64), "the oldest fell out");
        // A recall moves the look to the front (and that is worth saving).
        assert_eq!(k.observe(20), Observed::Recalled { settled: false });
        assert_eq!(k.memory()[0], (20, false, 21));
        assert!(k.take_memory_dirty());
        // A look that fell out is not recalled.
        k.suspect();
        assert_eq!(k.observe(3), Observed::Dropped);
        // Loading a saved memory replaces it, unsaved.
        k.set_memory(vec![(5, true, 6)]);
        assert!(!k.take_memory_dirty());
        k.suspect();
        assert_eq!(k.observe(5), Observed::Recalled { settled: true });
    }

    #[test]
    fn the_memory_file_round_trips_and_belongs_to_one_asset() {
        let a: Vec<u32> = (0..576).map(|i| 0xff00_0000 | i).collect();
        let b: Vec<u32> = (0..576).map(|i| i * 7).collect();
        let bytes = encode_memory(0x1234, &[(7, true, 24, 24, &a), (9, false, 24, 24, &b)]);
        assert_eq!(bytes.len(), 14 + 2 * (11 + 576 * 4));
        let back = decode_memory(&bytes, 0x1234);
        assert_eq!(back, vec![(7, true, 24, 24, a.clone()), (9, false, 24, 24, b.clone())]);
        // Another game asset: nothing — its colours may differ under the same keys.
        assert!(decode_memory(&bytes, 0x1235).is_empty());
        // A torn write keeps the whole entries before the tear.
        assert_eq!(decode_memory(&bytes[..bytes.len() - 5], 0x1234), vec![(7, true, 24, 24, a.clone())]);
        // Not a memory file at all.
        assert!(decode_memory(b"hello world, not ours", 0x1234).is_empty());
        assert!(decode_memory(&[], 0x1234).is_empty());
        // A malformed picture is not written.
        let one = encode_memory(1, &[(7, true, 24, 24, &a[..100]), (9, false, 24, 24, &b)]);
        assert_eq!(decode_memory(&one, 1), vec![(9, false, 24, 24, b)]);
        // The fingerprint is a function of the bytes.
        assert_eq!(asset_fingerprint(b"abc"), asset_fingerprint(b"abc"));
        assert_ne!(asset_fingerprint(b"abc"), asset_fingerprint(b"abd"));
    }
}
