// 아바타 옷장 (167차, 2026-10-06) — the wardrobe rules and the AVATAR page, pure.
//
// The user's five decisions (2026-10-06): ① the wardrobe is its own page in the menu's «1»«3»
// tab ring, right after EQUIPMENT (STATUS · INVENTORY · EQUIPMENT · AVATAR · SKILL · QUEST)
// ② six slots — 투구 · 갑옷 · 장갑 · 신발 · 무기 · 방패 — and no "hide" switch (the 기본 외형 row
// IS the hide) ③ dedicated costume SETS, hero2's way ④ a costume obtained and not saved is gone
// when the save is loaded again ⑤ vanilla gets it too.
//
// ── WHERE THE STATE LIVES ─────────────────────────────────────────────────────
// Nowhere in the host. What a slot wears and what a character owns are bytes of the game's own
// script-flag array (`base+0x9e98`, `wie_util::avatar::{FLAG_WEAR, FLAG_OWN}`), which the save
// carries both ways — so ④ is the save's own behaviour, a deleted character takes its costumes
// with it, a new character starts empty, the vault restores them with the generation and the
// 2회차 transition keeps them (it clears the quest arrays, not the flag array — 167차 RE).
// Two kinds of set need no bit at all: a starter set is owned by everyone, and a clear
// reward is also owned by any save that already plays a HIGHER difficulty than the one the
// set rewards (a save in 하드코어 has cleared 노말) — so a character that cleared before this
// build gets its reward on its next load, from the save itself.
//
// ── THE PAGE ──────────────────────────────────────────────────────────────────
// A cursor on the current slot's list ([`entries`]: 기본 외형 + the owned sets that have this
// slot's art in the jar). Moving the cursor TRIES the row on (the host arms the hooks' preview
// knob and re-dresses that one layer, so the live figure answers); OK — or a second tap on the
// focused row — WEARS it (the host writes the flag byte). Leaving the page drops every try-on.
// ⛔ The page never writes anything itself: [`Page::act`] returns an [`Outcome`] and the host
// performs it — the same split as `bagbox` / `bagbox_host`.
//
// ── 168차 (2026-10-06) ─────────────────────────────────────────────────────────
// The user: *"케이와 리츠는 남자 캐릭터여서 남자캐릭터 아바타를 만들어야해 … 이안, 케네스,
// 할로윈-해골 가면, 수영장 파티 케이, 수영장 파티 리츠"*. Two consequences live here:
//   * a set can belong to ONE hero (`SetDef::hero`) — the pool party sets; the other hero's
//     wardrobe never lists it ([`offered`]), and an unreadable hero offers neither.
//   * set numbers are never reused (the save stores them), so 167차's retired 1 and 3 can still
//     sit in a save's worn byte. Their sheets are gone from the jar, so the hooks already draw the
//     equipment there; the page agrees: such a byte reads as 기본 외형 ([`Wardrobe::worn`]), and
//     wearing 기본 외형 writes the 0 that cleans it.
// And the MENU list gets its AVATAR row (*"메뉴리스트가 펼쳐질때는 아바타 선택지가 없어"*):
// [`menu_move`] / [`menu_row_tap`] are the seven-row list's rules, `avatar_page` draws it.
//
// ── 175차 (2026-10-09) ─────────────────────────────────────────────────────────
// The user: *"캐릭터 생성 시 수영장 스킨은 기본 지급으로 줄꺼고, 나머지2개는 노말 깨면 줄꺼고
// 나머지 1개는 하드코어 깨면 줄꺼야 … 1개더 만들어서 하드 클리어했을 때 2개 … 이미 클리어한
// 세이브파일은 클리어한 난이도에 따라 배분되면돼"*. Each hero's own sets are now one starter (the
// pool party set) + two 노말 rewards + two 하드코어 rewards (the costume.txt `acquire` lines), and a
// cleared save is paid by the ownership rule above. What 175차 adds here is the other half: a set
// that was 기본 지급 until now can sit in the worn byte of a save that does not own it any more —
// [`unowned_worn`] names those slots and the host takes them off once per load.
//
// Dependency-free apart from `wie_util::avatar` (reached as `crate::avatar_rules` — a crate-root
// alias in `lib.rs`) and the generated catalog: `avatar_selftest.rs` `include!`s all three as
// crate-root modules of the same names, so every path here resolves the same way in both builds.

use crate::avatar_catalog::{Acquire, Hero, SETS, SetDef};
use crate::avatar_rules as av;

/// Visible list rows on the page.
pub const ROWS: usize = 4;

/// The 기본 외형 row: set 0, the equipment's own look.
pub const BASIC_NAME: &str = "기본 외형";
pub const BASIC_DESC: &str = "장비의 모습 그대로";

/// The loader's own shield gate (`0x4980a`, mask `0x109` = classes 0, 3, 8 — the three spear
/// classes): only those classes ever load a shield layer, so only they get the 방패 tab.
pub const SHIELD_CLASS_MASK: u32 = 0x109;

/// Slot names, for the info line and the notes.
pub const SLOT_NAMES: [&str; av::SLOTS] = ["투구", "갑옷", "장갑", "신발", "무기", "방패"];

/// Item-icon atlas frames for the slot strip: the category's own icon (`items::icon_frame` —
/// equipment frame = category). The weapon's is its category's; this is the fallback.
pub const SLOT_ICON: [u16; av::SLOTS] = [0, 1, 2, 3, 5, 11];

/// What the host reads from the guest for one refresh of the page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ctx {
    /// The difficulty byte (`0x150ddc8`): 0 노말 · 1 하드코어 · 2 헬.
    pub difficulty: Option<u8>,
    /// The class byte (`CHAR+0xb6`).
    pub class: Option<u8>,
    /// The equipped main weapon's category (`EQUIPPED[0]`), when one is equipped.
    pub weapon_cat: Option<u8>,
    /// Per avatar slot: does its EQUIPPED cell hold an item. The game draws 무기 · 방패 only over
    /// a worn piece (170차: a 투구 costume is drawn on the bare head too).
    pub equipped: [bool; av::SLOTS],
    /// The published availability masks (`wie_util::stats::AVATAR_SHEETS`).
    pub sheets: [u64; av::GROUPS],
    /// ★ 168차: the loaded hero (`items::char_hero`, the class name at `CHAR+0xb8`). A set made
    /// for one hero is offered only when this says so — `None` offers neither.
    pub hero: Option<Hero>,
}

/// The save's avatar bytes: the six worn sets and the owned bitmap, in flag-array order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wardrobe {
    pub wear: [u8; av::SLOTS],
    pub own: [u8; av::OWN_BYTES],
}

impl Wardrobe {
    /// Bytes from [`av::FLAG_WEAR`]: six worn bytes, then the bitmap.
    pub const LEN: usize = av::SLOTS + av::OWN_BYTES;

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < Self::LEN {
            return None;
        }
        let mut w = Self::default();
        w.wear.copy_from_slice(&b[..av::SLOTS]);
        w.own.copy_from_slice(&b[av::SLOTS..Self::LEN]);
        Some(w)
    }

    /// Does this character own `set`? See the module head for the two bit-less kinds.
    pub fn owns(&self, set: &SetDef, difficulty: Option<u8>) -> bool {
        match set.acquire {
            Acquire::Starter => true,
            Acquire::Clear(d) => av::owns(&self.own, set.id) || difficulty.is_some_and(|c| c > d),
        }
    }

    /// The set slot `slot` shows as far as the page is concerned: its worn byte, or 0 when that
    /// byte names no set of the catalog (168차: a retired set — the hooks find no sheet for it
    /// and draw the equipment, so the page must not call it worn).
    pub fn worn(&self, slot: usize) -> u8 {
        let set = self.wear[slot];
        if set_def(set).is_some() { set } else { 0 }
    }
}

/// Is `set` offered to this character at all (before ownership and art)? A hero's own set only
/// to that hero (168차).
pub fn offered(set: &SetDef, ctx: &Ctx) -> bool {
    set.hero.is_none_or(|h| ctx.hero == Some(h))
}

/// ★ 175차: the slots whose worn set this character may not wear — one it does not own (08~13
/// were 기본 지급 until 175차, so a save made before can wear one it now has to earn) or one made
/// for the other hero. A retired number is not named: it already reads as 기본 외형 and the hooks
/// draw the equipment there. `None` = the difficulty or the hero cannot be read — never take a
/// costume off on a guess.
pub fn unowned_worn(w: &Wardrobe, ctx: &Ctx) -> Option<Vec<usize>> {
    ctx.difficulty?;
    ctx.hero?;
    Some(
        (0..av::SLOTS)
            .filter(|&s| set_def(w.worn(s)).is_some_and(|d| !(w.owns(d, ctx.difficulty) && offered(d, ctx))))
            .collect(),
    )
}

/// The set with number `id`.
pub fn set_def(id: u8) -> Option<&'static SetDef> {
    SETS.iter().find(|s| s.id == id)
}

/// The sets an ending clear at `difficulty` rewards (the byte the transition found).
pub fn clear_rewards(difficulty: u8) -> impl Iterator<Item = &'static SetDef> {
    SETS.iter().filter(move |s| s.acquire == Acquire::Clear(difficulty))
}

/// Is the slot's tab offered to this character? Only the shield tab is ever withheld.
pub fn slot_open(slot: usize, ctx: &Ctx) -> bool {
    slot != av::SLOT_SHIELD || ctx.class.is_some_and(|c| c < 32 && (SHIELD_CLASS_MASK >> c) & 1 != 0)
}

/// The sheet group the slot's costume pieces come from for this character — the weapon's
/// is its equipped category's (`None` with no weapon in hand).
pub fn slot_group(slot: usize, ctx: &Ctx) -> Option<usize> {
    match slot {
        av::SLOT_WEAPON => av::group_of(slot, u32::from(ctx.weapon_cat?)),
        av::SLOT_SHIELD => av::group_of(slot, av::SHIELD_CAT),
        _ => av::group_of(slot, 0),
    }
}

/// The atlas frame of the slot's icon (the weapon shows its equipped category's).
pub fn slot_icon(slot: usize, ctx: &Ctx) -> u16 {
    match (slot, ctx.weapon_cat) {
        (av::SLOT_WEAPON, Some(c)) if (4..=10).contains(&c) => u16::from(c),
        _ => SLOT_ICON[slot],
    }
}

/// A piece's name in this slot (the weapon's per category), `""` = no piece there.
pub fn piece_name(set: &SetDef, slot: usize, weapon_cat: Option<u8>) -> &'static str {
    if slot == av::SLOT_WEAPON
        && let Some(c) = weapon_cat
        && (4..=10).contains(&c)
        && !set.weapons[usize::from(c - 4)].is_empty()
    {
        return set.weapons[usize::from(c - 4)];
    }
    set.pieces[slot]
}

/// How a set is obtained, as the info line says it.
pub fn acquire_text(a: Acquire) -> &'static str {
    match a {
        Acquire::Starter => "기본 지급",
        Acquire::Clear(0) => "노말 엔딩 보상",
        Acquire::Clear(1) => "하드코어 엔딩 보상",
        Acquire::Clear(_) => "헬 엔딩 보상",
    }
}

/// One row of a slot's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// 0 = 기본 외형.
    pub set: u8,
    pub name: &'static str,
    pub desc: &'static str,
    pub acquire: &'static str,
}

/// The slot's list: 기본 외형, then every owned set offered to this hero that has a piece here
/// AND whose sheet for this slot's group the jar carries (a piece the game could not draw is not
/// offered).
pub fn entries(slot: usize, w: &Wardrobe, ctx: &Ctx) -> Vec<Entry> {
    let mut v = vec![Entry { set: 0, name: BASIC_NAME, desc: BASIC_DESC, acquire: "" }];
    let Some(group) = slot_group(slot, ctx) else { return v };
    for s in SETS {
        let name = piece_name(s, slot, ctx.weapon_cat);
        if name.is_empty() || !offered(s, ctx) || !w.owns(s, ctx.difficulty) || ctx.sheets[group] & (1u64 << (s.id - 1)) == 0 {
            continue;
        }
        v.push(Entry { set: s.id, name, desc: s.descs[slot], acquire: acquire_text(s.acquire) });
    }
    v
}

/// A page action — from a key (the host maps them) or a tap (the UI maps them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Up,
    Down,
    PrevSlot,
    NextSlot,
    /// A tap on the slot strip.
    Slot(usize),
    /// A tap on visible row `i` (0..ROWS): focus it, or wear it when it is already focused.
    Row(usize),
    /// OK.
    Confirm,
    /// CLR — back to the MENU list, like every other page.
    Close,
    /// «1» — back to EQUIPMENT.
    Tab1,
    /// «3» — on to SKILL.
    Tab3,
}

/// The key the host passes to the game after leaving the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Then {
    /// Stay on EQUIPMENT (the page sat on top of it).
    Stay,
    /// CLR — EQUIPMENT closes to the MENU list.
    Clr,
    /// «3» — EQUIPMENT moves on to SKILL.
    Num3,
}

/// What the host must do after an action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Drop this slot's try-on (and re-dress it).
    pub drop_preview: Option<usize>,
    /// Arm this try-on `(slot, set)` (and re-dress it).
    pub preview: Option<(usize, u8)>,
    /// Write this worn byte `(slot, set)` into the save's flag array.
    pub wear: Option<(usize, u8)>,
    /// Leave the page, then pass this key on.
    pub leave: Option<Then>,
}

/// The page's own state: the slot on screen, its cursor, the first visible row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub slot: usize,
    pub cursor: usize,
    pub top: usize,
    /// The try-on armed for `slot`, if any (only the current slot ever has one).
    pub preview: Option<u8>,
}

impl Page {
    /// Open on the first slot, the cursor on what it wears.
    pub fn open(w: &Wardrobe, ctx: &Ctx) -> Self {
        let mut p = Page { slot: av::SLOT_HELMET, cursor: 0, top: 0, preview: None };
        p.focus_worn(w, ctx);
        p
    }

    fn focus_worn(&mut self, w: &Wardrobe, ctx: &Ctx) {
        let list = entries(self.slot, w, ctx);
        self.cursor = list.iter().position(|e| e.set == w.worn(self.slot)).unwrap_or(0);
        self.top = 0;
        self.scroll_into_view(list.len());
    }

    fn scroll_into_view(&mut self, len: usize) {
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + ROWS {
            self.top = self.cursor + 1 - ROWS;
        }
        self.top = self.top.min(len.saturating_sub(ROWS));
    }

    /// Move the cursor to `to` and try that row on.
    fn move_to(&mut self, to: usize, list: &[Entry], w: &Wardrobe) -> Outcome {
        self.cursor = to;
        self.scroll_into_view(list.len());
        let set = list[to].set;
        // Back on what the slot already wears: the knob is not needed any more.
        if set == w.worn(self.slot) {
            return Outcome { drop_preview: self.preview.take().map(|_| self.slot), ..Outcome::default() };
        }
        self.preview = Some(set);
        Outcome { preview: Some((self.slot, set)), ..Outcome::default() }
    }

    fn switch_slot(&mut self, to: usize, w: &Wardrobe, ctx: &Ctx) -> Outcome {
        if to == self.slot || !slot_open(to, ctx) {
            return Outcome::default();
        }
        let drop_preview = self.preview.take().map(|_| self.slot);
        self.slot = to;
        self.focus_worn(w, ctx);
        Outcome { drop_preview, ..Outcome::default() }
    }

    fn step_slot(&mut self, forward: bool, w: &Wardrobe, ctx: &Ctx) -> Outcome {
        let n = av::SLOTS;
        let mut s = self.slot;
        for _ in 0..n {
            s = if forward { (s + 1) % n } else { (s + n - 1) % n };
            if slot_open(s, ctx) {
                return self.switch_slot(s, w, ctx);
            }
        }
        Outcome::default()
    }

    /// One action. The page's own fields change here; everything the GAME should see comes
    /// back as the [`Outcome`].
    pub fn act(&mut self, act: Act, w: &Wardrobe, ctx: &Ctx) -> Outcome {
        let list = entries(self.slot, w, ctx);
        self.cursor = self.cursor.min(list.len() - 1);
        match act {
            Act::Up if self.cursor > 0 => self.move_to(self.cursor - 1, &list, w),
            Act::Down if self.cursor + 1 < list.len() => self.move_to(self.cursor + 1, &list, w),
            Act::Up | Act::Down => Outcome::default(),
            Act::PrevSlot => self.step_slot(false, w, ctx),
            Act::NextSlot => self.step_slot(true, w, ctx),
            Act::Slot(s) if s < av::SLOTS => self.switch_slot(s, w, ctx),
            Act::Slot(_) => Outcome::default(),
            Act::Row(i) => {
                let at = self.top + i;
                if i >= ROWS || at >= list.len() {
                    Outcome::default()
                } else if at == self.cursor {
                    self.act(Act::Confirm, w, ctx)
                } else {
                    self.move_to(at, &list, w)
                }
            }
            Act::Confirm => {
                let set = list[self.cursor].set;
                self.preview = None;
                // The RAW byte, not `worn`: wearing 기본 외형 over a retired set's byte is the
                // write that cleans it (168차).
                if set == w.wear[self.slot] {
                    Outcome::default()
                } else {
                    Outcome { wear: Some((self.slot, set)), ..Outcome::default() }
                }
            }
            Act::Close | Act::Tab1 | Act::Tab3 => {
                let drop_preview = self.preview.take().map(|_| self.slot);
                let then = match act {
                    Act::Close => Then::Clr,
                    Act::Tab3 => Then::Num3,
                    _ => Then::Stay,
                };
                Outcome { drop_preview, leave: Some(then), ..Outcome::default() }
            }
        }
    }

    /// The page as the UI draws it.
    pub fn view(&self, w: &Wardrobe, ctx: &Ctx) -> View {
        let list = entries(self.slot, w, ctx);
        let cursor = self.cursor.min(list.len() - 1);
        let rows = list
            .iter()
            .enumerate()
            .skip(self.top)
            .take(ROWS)
            .map(|(i, e)| RowView { name: e.name.to_string(), worn: e.set == w.worn(self.slot), focused: i == cursor })
            .collect();
        let e = list[cursor];
        let slot_name = SLOT_NAMES[self.slot];
        let note = if matches!(self.slot, av::SLOT_WEAPON | av::SLOT_SHIELD) && !ctx.equipped[self.slot] {
            // The game draws these two only over a worn piece (the painter skips an empty cell);
            // a 투구 costume is a hairstyle and is drawn on the bare head (170차).
            format!("{slot_name}{} 착용해야 보입니다", object_particle(slot_name))
        } else if e.set == w.worn(self.slot) {
            "착용 중".to_string()
        } else {
            "미리보기 중  OK 입기".to_string()
        };
        let pages = list.len().div_ceil(ROWS).max(1);
        View {
            slot: self.slot,
            open: core::array::from_fn(|s| slot_open(s, ctx)),
            dressed: core::array::from_fn(|s| w.worn(s) != 0),
            icon: core::array::from_fn(|s| slot_icon(s, ctx)),
            rows,
            page: (cursor / ROWS + 1, pages),
            name: e.name.to_string(),
            desc: e.desc.to_string(),
            acquire: e.acquire.to_string(),
            status: note,
        }
    }
}

/// 을/를 after a word, by its last syllable's final consonant (투구를 · 갑옷을 · 무기를 · 방패를).
pub fn object_particle(word: &str) -> &'static str {
    match word.chars().last() {
        Some(c @ '가'..='힣') if (c as u32 - 0xac00) % 28 != 0 => "을",
        _ => "를",
    }
}

/// One visible row, as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowView {
    pub name: String,
    /// What the slot wears now (the green E).
    pub worn: bool,
    /// The cursor's row (the yellow frame).
    pub focused: bool,
}

/// Everything the UI needs to draw the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub slot: usize,
    /// Which slot tabs are offered.
    pub open: [bool; av::SLOTS],
    /// Which slots wear a costume (a dot on the strip).
    pub dressed: [bool; av::SLOTS],
    /// Atlas frame per slot.
    pub icon: [u16; av::SLOTS],
    pub rows: Vec<RowView>,
    /// `(page, pages)` of the list, 1-based.
    pub page: (usize, usize),
    /// The info box: 이름 · 설명 · 획득 (empty for 기본 외형).
    pub name: String,
    pub desc: String,
    pub acquire: String,
    /// The hint row: 착용 중 · 미리보기 중 · why the game will not show it.
    pub status: String,
}

/// The notice an ending clear's new sets get (one line per set, the game's message box).
pub fn reward_notice(set: &SetDef) -> String {
    format!("아바타 획득: {}", set.name)
}

// ── TEST GRANT (176차, 2026-10-09) ─────────────────────────────────────────────
// The user: *"테스트용으로 s25의 첫번째 케이와 두번째 리츠 세이브 슬롯에는 모든 아바타 파츠 전부
// 넣어줘"*. A dev-only trigger file (`config::AVATAR_GRANT_TRIGGER`, pushed over adb — it is not
// in the APK) names save slots; when one of them is the live character on the field the host
// sets the owned bit of EVERY catalog set and arms a save, and that slot's line leaves the file
// (`AvatarHost::test_grant`). Owning a set owns every piece of it, so this is every part. The
// other hero's bits are set too and stay invisible (`offered`), like a clear reward's.

/// The save slots a test-grant file names: `slot N` lines, N hex like every other trigger file
/// (`wie_grant.txt`, `wie_bagbox_add.txt`) and 0-based — `slot 0` is SLOT 1. `#` comments.
pub fn parse_test_grant(text: &str) -> Vec<u8> {
    let mut slots = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        if let ["slot", n] = f[..]
            && let Ok(s) = u8::from_str_radix(n.trim_start_matches("0x"), 16)
            && !slots.contains(&s)
        {
            slots.push(s);
        }
    }
    slots
}

/// The file without the `slot` line(s) naming `slot`; `None` when no line names it.
pub fn test_grant_without(text: &str, slot: u8) -> Option<String> {
    let keep: Vec<&str> = text.lines().filter(|l| parse_test_grant(l) != [slot]).collect();
    (keep.len() < text.lines().count()).then(|| keep.join("\n") + "\n")
}

/// The owned bitmap with every catalog set's bit set.
pub fn own_every_set(own: [u8; av::OWN_BYTES]) -> [u8; av::OWN_BYTES] {
    SETS.iter().fold(own, |o, s| av::with_owned(o, s.id))
}

/// The side line a test grant says.
pub const TEST_GRANT_NOTE: &str = "테스트: 모든 아바타 지급";

// ── the MENU list's AVATAR row (168차) ───────────────────────────────────────────
// The game's MENU list has six rows — STATUS · INVENTORY · EQUIPMENT · SKILL · QUEST · SYSTEM,
// cursor `CHAR+0x9c6d` — and the host shows seven, AVATAR between EQUIPMENT and SKILL: the place
// the wardrobe has in the «1»«3» ring. AVATAR has no game row. While it is selected the game's
// cursor sits on EQUIPMENT (2), the page the wardrobe opens over, so OK on AVATAR is the game's
// own OK on EQUIPMENT with the wardrobe on top. Measured (probe, 168차): the list's cursor moves
// make no sound (so the host may move the cursor by writing it — nothing is lost), UP/DOWN wrap
// at both ends, and CLR from a page returns to the row the page was ENTERED from (INVENTORY →
// «3» EQUIPMENT → CLR lands on INVENTORY) — so a page entered through AVATAR comes back to
// AVATAR.

/// Rows of the host's MENU list.
pub const MENU_ROWS: u8 = 7;
/// The AVATAR row of the seven.
pub const MENU_AVATAR: u8 = 3;
/// The game's own EQUIPMENT row — where its cursor sits while AVATAR is selected.
pub const MENU_GAME_EQUIP: u8 = 2;
/// The game's SKILL row, the one below AVATAR.
pub const MENU_GAME_SKILL: u8 = 3;
/// The game's rows (its cursor's range is `0..MENU_GAME_ROWS`).
pub const MENU_GAME_ROWS: u8 = 6;

/// The seven-row index of game row `game`.
pub fn menu_row_of_game(game: u8) -> u8 {
    if game >= MENU_AVATAR { game + 1 } else { game }
}

/// The game row a seven-row index stands for — `None` for AVATAR and out of range.
pub fn menu_game_of_row(row: u8) -> Option<u8> {
    match row {
        r if r < MENU_AVATAR => Some(r),
        r if r > MENU_AVATAR && r < MENU_ROWS => Some(r - 1),
        _ => None,
    }
}

/// The selected row of the seven.
pub fn menu_selected(on_avatar: bool, game: u8) -> u8 {
    if on_avatar { MENU_AVATAR } else { menu_row_of_game(game) }
}

/// What the worker proposes to the UI about the MENU list (`UserEvent::MenuAvatar`). The UI
/// still draws only over a frame that shows the game's own six-row list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuSel {
    /// No AVATAR row (the build cannot dress a layer): the game's six rows, untouched.
    Off,
    /// The AVATAR row is offered and the selection is the game's own, read off the frame —
    /// what the first frame of a freshly opened list shows before the worker has looked.
    Game,
    /// The AVATAR row is offered and this row of the seven is selected (the worker's model,
    /// ahead of a frame the game has not painted yet).
    Row(u8),
}

impl MenuSel {
    /// The row to highlight over a frame whose own selection is game row `game`; `None` = draw
    /// nothing of ours.
    pub fn row(self, game: u8) -> Option<u8> {
        match self {
            MenuSel::Off => None,
            MenuSel::Game => Some(menu_row_of_game(game)),
            MenuSel::Row(r) => Some(r),
        }
    }
}

/// A key the seven-row list answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKey {
    Up,
    Down,
    Ok,
}

/// What a key does to the seven-row list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuMove {
    /// Not the host's: the game takes the key (a move between two game rows, CLR, …).
    Pass,
    /// The host's: AVATAR becomes selected (`avatar`) or game row `cursor` does — written into
    /// the game's cursor when it differs. The game never sees the key.
    Select { avatar: bool, cursor: u8 },
    /// OK on AVATAR: the game's OK opens EQUIPMENT (its cursor is there) and the wardrobe
    /// opens over it.
    Open,
}

/// One key on the list. `on_avatar` = AVATAR is selected; `game` = the game's cursor.
pub fn menu_move(key: MenuKey, on_avatar: bool, game: u8) -> MenuMove {
    match (key, on_avatar) {
        (MenuKey::Down, true) => MenuMove::Select { avatar: false, cursor: MENU_GAME_SKILL },
        (MenuKey::Up, true) => MenuMove::Select { avatar: false, cursor: MENU_GAME_EQUIP },
        (MenuKey::Ok, true) => MenuMove::Open,
        (MenuKey::Down, false) if game == MENU_GAME_EQUIP => MenuMove::Select { avatar: true, cursor: MENU_GAME_EQUIP },
        (MenuKey::Up, false) if game == MENU_GAME_SKILL => MenuMove::Select { avatar: true, cursor: MENU_GAME_EQUIP },
        _ => MenuMove::Pass,
    }
}

/// A tap on one of the seven rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuTap {
    /// First tap: select it — the cursor written, no key (66차-B's two-tap rule).
    Focus { avatar: bool, cursor: u8 },
    /// A tap on the selected row: its OK (AVATAR's opens the wardrobe).
    Activate { avatar: bool, cursor: u8 },
}

/// A tap on row `row` of the seven; `None` past the list.
pub fn menu_row_tap(row: u8, on_avatar: bool, game: u8) -> Option<MenuTap> {
    let (avatar, cursor) = if row == MENU_AVATAR { (true, MENU_GAME_EQUIP) } else { (false, menu_game_of_row(row)?) };
    Some(if row == menu_selected(on_avatar, game) {
        MenuTap::Activate { avatar, cursor }
    } else {
        MenuTap::Focus { avatar, cursor }
    })
}
