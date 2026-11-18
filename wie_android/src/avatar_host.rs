// 아바타 옷장 (167차) — the worker half of the AVATAR page.
//
// The page is `avatar::Page` (pure) drawn by `avatar_page` (pure, on the UI thread); this file
// is what touches the guest:
//
//   * OPEN — a «3» on EQUIPMENT's worn grid (`items::equipment_worn_grid_up`) is kept from the
//     game and opens the page instead of SKILL; a «1» on SKILL is passed on (the game goes back
//     to EQUIPMENT) and opens the page AHEAD of it (168차 — see [`AvatarHost::open`]'s
//     `ahead`), as does MENU's AVATAR row. So the ring reads STATUS · INVENTORY · EQUIPMENT ·
//     AVATAR · SKILL · QUEST in both directions, and EQUIPMENT never shows on the way in.
//   * KEYS while open — UP/DOWN the list, LEFT/RIGHT «7»/«9» the slot, OK wears, CLR / «1» /
//     «3» leave (CLR and «3» then go on to the game as the keys they were; «1» stays on
//     EQUIPMENT, which is where the page sat). Every other key is eaten: the page owns the
//     panel, and the game's EQUIPMENT grid under it must not move.
//   * OUTCOMES — a try-on arms the hooks' preview knob (`wie_util::stats::set_avatar_preview`)
//     and re-dresses that one layer through the game's own setter (`0x517bc`); wearing writes
//     the worn byte into the save's flag array (read-verify-then-write) — memory only until the
//     player saves, which IS the user's rule ④.
//   * CLOSE — whenever the game is no longer on EQUIPMENT's worn grid with nothing on top, or
//     the page is left: every try-on is dropped and its layer re-dressed. A page left with a key
//     the GAME acts on next (CLR, «3») is taken down by the UI on the first frame that no longer
//     shows EQUIPMENT (`UserEvent::AvatarLeave`, 168차) — never earlier, or EQUIPMENT flashes.
//   * THE MENU LIST (168차) — the game's six rows are drawn as seven with AVATAR between
//     EQUIPMENT and SKILL (`avatar::menu_move` / `menu_row_tap` are the rules, `avatar_page`
//     draws it): UP / DOWN / OK and taps that touch AVATAR are the host's, everything else is
//     the game's, and a page entered through AVATAR comes back to AVATAR.
//   * REWARDS — the 2회차 transition (`emulator.rs` calls [`AvatarHost::on_ending`] from its one
//     consumer of `wie_util::stats::ending_transition`) sets the owned bits of the sets that
//     clear rewards, and says so in the game-style notice box.
//   * ★ 175차 AUDIT — once per character load (`vault::load_generation`), on the first field
//     frame: a costume the character may not wear (`avatar::unowned_worn` — 08~13 were 기본 지급
//     until 175차) comes off.
//   * ★ 176차 TEST GRANT — a save slot named in `config::AVATAR_GRANT_TRIGGER` (adb only) owns
//     every set once it is the live character on the field, and the game's save is asked for
//     ([`AvatarHost::save_wanted`]); it runs before the audit, so nothing is taken off first.
//
// ⛔ The re-dress call is ASYNCHRONOUS (`call_guest_function` queues it for the next tick), so
// the knob must be in place BEFORE the call is queued — the hooks read it when the setter runs.

use wie_backend::{Emulator, Event, KeyCode};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;
use crate::avatar::{self, Act, Ctx, MenuKey, MenuMove, MenuSel, MenuTap, Page, Then, View, Wardrobe};
use crate::avatar_catalog::Hero;
use crate::avatar_rules as av;
use crate::items;

/// How many worker iterations a page opened AHEAD of the game's EQUIPMENT (a «1» on SKILL,
/// MENU's AVATAR row) waits for EQUIPMENT's worn grid before it gives up (the game draws
/// EQUIPMENT on its next tick; this is ~1 s of slack).
const OPENING_ITERS: u32 = 60;

/// Re-read the page's inputs this often while it is open (worker iterations). Nothing the page
/// shows can change without a key of ours — this is the safety net, not the mechanism.
const REFRESH_EVERY: u64 = 15;

/// How often (worker iterations) the 투구 cell is looked at for a 해제 under a helmet costume.
const HELMET_WATCH_EVERY: u64 = 5;

/// How often (worker iterations) a load still waiting for its wardrobe audit looks for the field.
const AUDIT_EVERY: u64 = 10;

/// How often (worker iterations) the test-grant file is looked at outside a load (a file pushed
/// while its slot is already on the field). A multiple of [`AUDIT_EVERY`].
const TEST_GRANT_EVERY: u64 = 120;

#[derive(Default)]
pub struct AvatarHost {
    page: Option<Page>,
    /// Iterations left for a page opened ahead of the game's EQUIPMENT to see its worn grid
    /// come up; 0 = armed (the page closes the moment the grid goes).
    opening: u32,
    /// What the UI was last told (`None` = taken down).
    published: Option<View>,
    /// ★ 168차 MENU list: AVATAR is the selected row (the game's cursor sits on EQUIPMENT).
    menu_avatar: bool,
    /// A host-made move onto a GAME row, published as `MenuSel::Row` until the game paints it
    /// (the UI drops it by itself once its frame agrees).
    menu_forced: Option<u8>,
    /// The list was up on the last look (its rising edge restores [`Self::entered_avatar`]).
    menu_up: bool,
    /// The pages on screen were entered through MENU's AVATAR row: back on the list, AVATAR is
    /// selected — the game's own rule (CLR returns to the row a page was entered from).
    entered_avatar: bool,
    /// What the UI was last told about the list.
    menu_published: Option<MenuSel>,
    /// ★ 170차: the 투구 cell held an item on the last look (`None` = not looked at since the
    /// world was last entered). Its worn -> empty edge is the game's 해제 of a 투구, which frees
    /// layer 6 — a helmet costume is then put back on the bare head.
    helmet_cell: Option<bool>,
    /// ★ 175차: the character-load counter as last seen, and a load whose worn costumes have not
    /// been checked against what the character owns yet ([`AvatarHost::audit`]).
    seen_load: u64,
    audit_due: bool,
    /// ★ 176차: the save slot whose test grant is in memory and waits for the game's save.
    test_save: Option<u8>,
}

/// One consistent read of everything the page decides from.
struct Snapshot {
    ctx: Ctx,
    wardrobe: Wardrobe,
    flags_addr: u32,
    worn: [(i16, i16); 9],
}

fn snapshot(emulator: &dyn Emulator) -> Option<Snapshot> {
    let read = |a: u32, l: usize| emulator.read_memory(a, l);
    let flags_addr = items::avatar_flags_addr(&read)?;
    let wardrobe = Wardrobe::decode(&items::avatar_bytes(&read)?)?;
    let worn = items::worn_cells(&read)?;
    let class = items::char_class_byte(&read);
    let weapon_cat = u8::try_from(worn[0].0).ok().filter(|c| (4..=10).contains(c));
    let equipped = core::array::from_fn(|s| worn[usize::from(av::EQUIPPED_SLOT[s])].0 >= 0);
    let sheets = core::array::from_fn(|g| {
        (1..=av::SET_MAX).fold(0u64, |m, set| if wie_util::stats::avatar_sheet(g, set) { m | 1 << (set - 1) } else { m })
    });
    let hero = items::char_hero(&read).map(|h| match h {
        crate::guardian::Hero::Ritz => Hero::Ritz,
        crate::guardian::Hero::Kei => Hero::Kei,
    });
    let ctx = Ctx { difficulty: items::difficulty_byte(&read), class, weapon_cat, equipped, sheets, hero };
    Some(Snapshot { ctx, wardrobe, flags_addr, worn })
}

impl AvatarHost {
    pub fn new() -> Self {
        Self { seen_load: crate::vault::load_generation(), ..Self::default() }
    }

    pub fn is_open(&self) -> bool {
        self.page.is_some()
    }

    /// A key the player pressed. `true` = the page took it (the game never sees it, nor its
    /// release — consumed keys never enter the worker's `held` map).
    pub fn key(&mut self, emulator: &mut dyn Emulator, code: KeyCode, proxy: &EventLoopProxy<UserEvent>) -> bool {
        if self.page.is_some() {
            let act = match code {
                KeyCode::UP | KeyCode::NUM2 => Act::Up,
                KeyCode::DOWN | KeyCode::NUM8 => Act::Down,
                KeyCode::LEFT | KeyCode::NUM4 | KeyCode::NUM7 => Act::PrevSlot,
                KeyCode::RIGHT | KeyCode::NUM6 | KeyCode::NUM9 => Act::NextSlot,
                KeyCode::OK | KeyCode::NUM5 => Act::Confirm,
                KeyCode::CLEAR => Act::Close,
                KeyCode::NUM1 => Act::Tab1,
                KeyCode::NUM3 => Act::Tab3,
                _ => return true,
            };
            // CLR and «3» leave the page AND go on to the game as themselves.
            return !matches!(self.perform(emulator, act, proxy), Some(Then::Clr | Then::Num3));
        }
        // ★ 168차: the seven-row MENU list.
        let menu_key = match code {
            KeyCode::UP | KeyCode::NUM2 => Some(MenuKey::Up),
            KeyCode::DOWN | KeyCode::NUM8 => Some(MenuKey::Down),
            KeyCode::OK | KeyCode::NUM5 => Some(MenuKey::Ok),
            _ => None,
        };
        if let Some(k) = menu_key
            && menu_offered()
            && let Some((addr, game)) = menu_list(&*emulator)
        {
            return match avatar::menu_move(k, self.menu_avatar, game) {
                MenuMove::Pass => {
                    // The game moves its own cursor (or opens its own page) — the frame it paints
                    // says which row is selected.
                    self.menu_forced = None;
                    self.menu_publish(proxy);
                    false
                }
                MenuMove::Select { avatar, cursor } => {
                    self.menu_select(emulator, addr, game, avatar, cursor, proxy);
                    true
                }
                // OK on AVATAR: the game's own OK on EQUIPMENT (its cursor is there), the
                // wardrobe opened ahead of it — the key goes on.
                MenuMove::Open => {
                    self.menu_open(emulator, addr, game, proxy);
                    false
                }
            };
        }
        let (equip, skill) = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            match code {
                KeyCode::NUM3 => (items::equipment_worn_grid_up(&read), false),
                KeyCode::NUM1 => (false, items::skill_page_up(&read)),
                _ => (false, false),
            }
        };
        if equip {
            return self.open(emulator, proxy, "«3» on EQUIPMENT", false);
        }
        if skill {
            // The «1» goes on to the game, which goes back to EQUIPMENT; the page is published
            // NOW, so the first EQUIPMENT frame is already drawn over (168차).
            self.open(emulator, proxy, "«1» on SKILL", true);
        }
        false
    }

    /// A tap on the page (`InputEvent::Avatar`). A tap that leaves the page with «3» presses
    /// the game's «3» itself — there is no key of the player's to pass on.
    pub fn tap(&mut self, emulator: &mut dyn Emulator, act: Act, proxy: &EventLoopProxy<UserEvent>) {
        if self.page.is_none() {
            return;
        }
        match self.perform(emulator, act, proxy) {
            Some(Then::Num3) => press(emulator, KeyCode::NUM3),
            Some(Then::Clr) => press(emulator, KeyCode::CLEAR),
            _ => {}
        }
    }

    /// ★ 168차: a tap on row `row` of the seven-row MENU list the UI drew
    /// (`InputEvent::MenuRow`) — 66차-B's two-tap rule: the first selects (the cursor written, no
    /// key), a tap on the selected row opens it.
    pub fn menu_row(&mut self, emulator: &mut dyn Emulator, row: u8, proxy: &EventLoopProxy<UserEvent>) {
        let Some((addr, game)) = menu_list(&*emulator) else {
            log::info!("menu row {row}: the MENU list is not up any more — tap dropped");
            return;
        };
        match avatar::menu_row_tap(row, self.menu_avatar, game) {
            None => {}
            Some(MenuTap::Focus { avatar, cursor }) => {
                log::info!("menu row {row}: focus (game cursor {game} -> {cursor}, avatar {avatar})");
                self.menu_select(emulator, addr, game, avatar, cursor, proxy);
            }
            Some(MenuTap::Activate { avatar: true, .. }) => {
                log::info!("menu row {row}: AVATAR is selected -> OK on EQUIPMENT + the wardrobe");
                self.menu_open(emulator, addr, game, proxy);
                press(emulator, KeyCode::OK);
            }
            Some(MenuTap::Activate { avatar: false, cursor }) => {
                log::info!("menu row {row}: selected -> OK");
                if cursor != game && !emulator.write_memory(addr, &[cursor]) {
                    log::warn!("menu row {row}: the cursor write at {addr:#x} failed — no OK");
                    return;
                }
                self.menu_avatar = false;
                self.menu_forced = None;
                self.menu_publish(proxy);
                press(emulator, KeyCode::OK);
            }
        }
    }

    /// Every worker iteration: close the page under a screen that is no longer EQUIPMENT's worn
    /// grid (an opening page waits for it first), refresh the published view now and then, and
    /// keep the MENU list's AVATAR row in step with the game.
    pub fn frame(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>, frame: u64) {
        if self.page.is_some() {
            let up = {
                let read = |a: u32, l: usize| emulator.read_memory(a, l);
                items::equipment_worn_grid_up(&read)
            };
            if up {
                self.opening = 0;
                if frame % REFRESH_EVERY == 0 {
                    self.publish(emulator, proxy);
                }
            } else if self.opening > 0 {
                self.opening -= 1;
                if self.opening == 0 {
                    let report = {
                        let read = |a: u32, l: usize| emulator.read_memory(a, l);
                        items::equipment_gate_report(&read)
                    };
                    log::warn!("avatar: EQUIPMENT never came up under the opening wardrobe — {report}");
                    self.close(emulator, proxy, "EQUIPMENT never came up", false);
                }
            } else {
                self.close(emulator, proxy, "the game left EQUIPMENT's worn grid", true);
            }
        }
        self.menu_frame(emulator, proxy);
        if frame % HELMET_WATCH_EVERY == 0 {
            self.helmet_watch(emulator);
        }
        let load = crate::vault::load_generation();
        if load != self.seen_load {
            self.seen_load = load;
            self.audit_due = true;
            if let Some(slot) = self.test_save.take() {
                // The reload replaced the bits before any save carried them; the slot's line
                // is still in the file, so the grant runs again when that slot is loaded.
                log::warn!("avatar: TEST GRANT — a load came before slot {slot}'s save; it waits for that slot's next load");
            }
        }
        if self.page.is_none() && frame % AUDIT_EVERY == 0 && (self.audit_due || frame % TEST_GRANT_EVERY == 0) {
            self.test_grant(emulator, proxy);
        }
        if self.audit_due && self.page.is_none() && frame % AUDIT_EVERY == 0 {
            self.audit(emulator);
        }
    }

    /// ★ 176차 TEST: while a save slot named in `config::AVATAR_GRANT_TRIGGER` is the live
    /// character on the field, own every set (read-verify-then-write the owned bitmap), say so on
    /// the side line and ask for the game's save — the bits are memory until then, like every
    /// wardrobe write. The slot's line leaves the file when that save starts ([`Self::save_started`]).
    fn test_grant(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>) {
        if self.test_save.is_some() {
            return;
        }
        let slots = crate::config::read_avatar_grant();
        let Some(slot) = crate::vault::live_slot().filter(|s| slots.contains(s)) else { return };
        let field = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            !items::shell_state(&read) && items::in_field_state(&read)
        };
        if !field {
            return;
        }
        let Some(snap) = snapshot(&*emulator) else { return };
        let own = avatar::own_every_set(snap.wardrobe.own);
        let at = snap.flags_addr + av::SLOTS as u32;
        let ok = own == snap.wardrobe.own || (emulator.read_memory(at, av::OWN_BYTES) == snap.wardrobe.own && emulator.write_memory(at, &own));
        log::warn!(
            "avatar: TEST GRANT — save slot {slot} (hero {:?}, difficulty {:?}) owns every set: bits {:02x?} -> {own:02x?} at {at:#x} (write ok = {ok})",
            snap.ctx.hero,
            snap.ctx.difficulty,
            snap.wardrobe.own
        );
        if ok {
            self.test_save = Some(slot);
            let _ = proxy.send_event(UserEvent::SideNote(avatar::TEST_GRANT_NOTE.into()));
        }
    }

    /// A test grant is in memory and wants the game's save (`emulator.rs` asks the save gate).
    pub fn save_wanted(&self) -> bool {
        self.test_save.is_some()
    }

    /// The save the test grant wanted has started: its slot's line leaves the trigger file.
    pub fn save_started(&mut self) {
        if let Some(slot) = self.test_save.take() {
            crate::config::drop_avatar_grant_slot(slot);
            log::warn!("avatar: TEST GRANT — slot {slot}'s save started; its line left {}", crate::config::AVATAR_GRANT_TRIGGER);
        }
    }

    /// ★ 175차: take off what the loaded character may not wear — a set it does not own (08~13
    /// were 기본 지급 until 175차) or the other hero's. The worn byte goes back to 0
    /// (read-verify-then-write; memory only until the player saves, like every wardrobe write — so
    /// a reload without saving loads the byte again and this takes it off again) and the layer is
    /// re-dressed by the game's own setter. It waits for the field: the world's init has dressed
    /// the layers by then and the setter needs a live world. A difficulty or hero that cannot be
    /// read yet is looked at again ([`AUDIT_EVERY`]) — never a guess.
    fn audit(&mut self, emulator: &mut dyn Emulator) {
        let field = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            !items::shell_state(&read) && items::in_field_state(&read)
        };
        if !field {
            return;
        }
        let Some(snap) = snapshot(&*emulator) else { return };
        let Some(slots) = avatar::unowned_worn(&snap.wardrobe, &snap.ctx) else { return };
        self.audit_due = false;
        let mut off = Vec::new();
        for &slot in &slots {
            let at = snap.flags_addr + slot as u32;
            let was = snap.wardrobe.wear[slot];
            let ok = emulator.read_memory(at, 1) == [was] && emulator.write_memory(at, &[0]);
            log::warn!(
                "avatar: {} wore set {was} ({}), which this character does not own (difficulty {:?}, hero {:?}) — taken off: byte {at:#x} {was} -> 0 (write ok = {ok})",
                avatar::SLOT_NAMES[slot],
                avatar::set_def(was).map_or("?", |d| d.name),
                snap.ctx.difficulty,
                snap.ctx.hero
            );
            if ok {
                off.push(slot);
            }
        }
        if !off.is_empty()
            && let Some(fresh) = snapshot(&*emulator)
        {
            for slot in off {
                redress(emulator, &fresh, slot);
            }
        }
    }

    /// ★ 170차: the game's 해제 of a 투구 frees layer 6 and returns before the join (jump table
    /// `0x5e2b0`, kind 6 -> `0x51a62`), so a helmet costume would vanish with the 투구 until the
    /// next load. On the cell's worn -> empty edge, re-dress the slot (the stand-in row puts the
    /// costume back; with no costume it is the same free the game just did).
    fn helmet_watch(&mut self, emulator: &mut dyn Emulator) {
        let worn = {
            let read = |a: u32, l: usize| emulator.read_memory(a, l);
            if items::shell_state(&read) {
                None
            } else {
                items::worn_cells(&read).map(|w| w[usize::from(av::EQUIPPED_SLOT[av::SLOT_HELMET])].1 >= 0)
            }
        };
        let was = core::mem::replace(&mut self.helmet_cell, worn);
        if was == Some(true)
            && worn == Some(false)
            && let Some(snap) = snapshot(&*emulator)
            && helmet_shown(&snap)
        {
            log::info!("avatar: the 투구 came off under a helmet costume — the costume goes back on the bare head");
            redress(emulator, &snap, av::SLOT_HELMET);
        }
    }

    /// The 2회차 transition ran; `difficulty_before` is the byte it found (0 노말 · 1 하드코어 ·
    /// 2 헬). Owns the sets that clear rewards — memory only until the player saves.
    pub fn on_ending(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>, difficulty_before: Option<u8>) {
        let Some(d) = difficulty_before else {
            log::warn!("avatar: the ending ran but its difficulty could not be read — no wardrobe reward");
            return;
        };
        let Some(snap) = snapshot(&*emulator) else {
            log::warn!("avatar: the ending ran but the save's avatar bytes could not be read — no wardrobe reward");
            return;
        };
        let mut own = snap.wardrobe.own;
        let mut new = Vec::new();
        for set in avatar::clear_rewards(d) {
            // Owned BEFORE this clear? (The transition has already raised the difficulty, so
            // the byte it found is the one that says.) ★ 175차: the clear rewards now include
            // each hero's own sets — the bit is set for all of them (harmless: the other hero's
            // never shows, `avatar::offered`), the notice names only what this hero can wear.
            if !snap.wardrobe.owns(set, Some(d)) && avatar::offered(set, &snap.ctx) {
                new.push(set);
            }
            own = av::with_owned(own, set.id);
        }
        if own != snap.wardrobe.own {
            let at = snap.flags_addr + av::SLOTS as u32;
            let ok = emulator.write_memory(at, &own);
            log::warn!("avatar: the {d}-difficulty ending pays {} set(s) — owned bits {:02x?} -> {own:02x?} at {at:#x} (write ok = {ok})", new.len(), snap.wardrobe.own);
            if !ok {
                return;
            }
        }
        for set in new {
            let _ = proxy.send_event(UserEvent::NoticeText(avatar::reward_notice(set)));
        }
    }

    /// Open the page. `ahead` = the game is not on EQUIPMENT yet (a «1» on SKILL, MENU's AVATAR
    /// row — the key that takes it there is on its way): the page is published NOW, so the UI —
    /// which draws it only over a frame that shows EQUIPMENT's worn grid — covers the very first
    /// EQUIPMENT frame, and [`Self::frame`] waits [`OPENING_ITERS`] for the grid before the
    /// usual "the game left" rule applies.
    fn open(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>, why: &str, ahead: bool) -> bool {
        let Some(snap) = snapshot(&*emulator) else {
            log::warn!("avatar: {why} — the save's avatar bytes cannot be read, the wardrobe stays shut");
            return false;
        };
        wie_util::stats::clear_avatar_preview();
        self.page = Some(Page::open(&snap.wardrobe, &snap.ctx));
        self.opening = if ahead { OPENING_ITERS } else { 0 };
        log::info!(
            "avatar: wardrobe open ({why}{}) — wears {:?}, hero {:?}, class {:?}, weapon cat {:?}, difficulty {:?}",
            if ahead { ", ahead of EQUIPMENT" } else { "" },
            snap.wardrobe.wear,
            snap.ctx.hero,
            snap.ctx.class,
            snap.ctx.weapon_cat,
            snap.ctx.difficulty
        );
        self.publish(emulator, proxy);
        true
    }

    /// Close the page. `leaving` = the game is about to show (or already shows) another page —
    /// the UI keeps the page up until its frame stops showing EQUIPMENT (`UserEvent::AvatarLeave`)
    /// instead of dropping it now and flashing EQUIPMENT for the frame in between.
    fn close(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>, why: &str, leaving: bool) {
        let Some(page) = self.page.take() else { return };
        self.opening = 0;
        if page.preview.is_some() {
            wie_util::stats::set_avatar_preview(page.slot, None);
            if let Some(snap) = snapshot(&*emulator) {
                redress(emulator, &snap, page.slot);
            }
        }
        wie_util::stats::clear_avatar_preview();
        log::info!("avatar: wardrobe closed ({why})");
        if self.published.take().is_some() {
            let _ = proxy.send_event(if leaving { UserEvent::AvatarLeave } else { UserEvent::AvatarPage(None) });
        }
    }

    /// Run one action and carry out what it asks for. `Some(then)` = the page was left.
    fn perform(&mut self, emulator: &mut dyn Emulator, act: Act, proxy: &EventLoopProxy<UserEvent>) -> Option<Then> {
        let Some(snap) = snapshot(&*emulator) else {
            self.close(emulator, proxy, "the save's avatar bytes stopped reading", false);
            return Some(Then::Stay);
        };
        let page = self.page.as_mut()?;
        let o = page.act(act, &snap.wardrobe, &snap.ctx);
        if let Some(slot) = o.drop_preview {
            wie_util::stats::set_avatar_preview(slot, None);
            redress(emulator, &snap, slot);
        }
        if let Some((slot, set)) = o.preview {
            wie_util::stats::set_avatar_preview(slot, Some(set));
            redress(emulator, &snap, slot);
        }
        if let Some((slot, set)) = o.wear {
            let at = snap.flags_addr + slot as u32;
            let ok = emulator.read_memory(at, 1) == [snap.wardrobe.wear[slot]] && emulator.write_memory(at, &[set]);
            wie_util::stats::set_avatar_preview(slot, None);
            log::info!(
                "avatar: {} wears set {set} ({}) — byte {at:#x} {} -> {set} (write ok = {ok})",
                avatar::SLOT_NAMES[slot],
                avatar::set_def(set).map_or(avatar::BASIC_NAME, |s| s.name),
                snap.wardrobe.wear[slot]
            );
            // ⛔ 172차: re-dress from a snapshot taken AFTER the write. `snap` still holds the
            // old worn byte, and with the try-on knob just cleared `helmet_shown` read that
            // old byte — 0 for a character with no 투구 on (every new character) — and called
            // the setter with `remove = 1`, the game's own 해제 of a 투구, which freed the
            // layer: the costume vanished the moment OK was pressed (S9 · S25, 2026-10-08).
            match snapshot(&*emulator) {
                Some(fresh) => redress(emulator, &fresh, slot),
                None => redress(emulator, &snap, slot),
            }
        }
        if let Some(then) = o.leave {
            // «1» stays on EQUIPMENT, which the player is meant to see at once; CLR and «3» are
            // keys the game acts on next — the page stays drawn until its frame moves on.
            self.close(emulator, proxy, "left by the player", then != Then::Stay);
            return Some(then);
        }
        self.publish(emulator, proxy);
        None
    }

    /// Tell the UI what the page shows now — only when that changed.
    fn publish(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>) {
        let Some(page) = self.page.as_ref() else { return };
        let Some(snap) = snapshot(&*emulator) else { return };
        let view = page.view(&snap.wardrobe, &snap.ctx);
        if self.published.as_ref() != Some(&view) {
            self.published = Some(view.clone());
            let _ = proxy.send_event(UserEvent::AvatarPage(Some(Box::new(view))));
        }
    }

    // ── the MENU list (168차) ───────────────────────────────────────────────────────

    /// Select AVATAR (`avatar`) or game row `cursor`, writing the game's cursor when it differs
    /// from `game` — no key: the list's moves are silent (measured), so nothing is lost.
    fn menu_select(&mut self, emulator: &mut dyn Emulator, addr: u32, game: u8, avatar: bool, cursor: u8, proxy: &EventLoopProxy<UserEvent>) {
        if cursor != game && !emulator.write_memory(addr, &[cursor]) {
            log::warn!("menu: the cursor write at {addr:#x} failed — the selection stays");
            return;
        }
        self.menu_avatar = avatar;
        self.menu_forced = (!avatar).then(|| avatar::menu_row_of_game(cursor));
        self.menu_publish(proxy);
    }

    /// AVATAR opens: the game's cursor onto EQUIPMENT (the caller's OK opens it), the wardrobe
    /// ahead of it, and the list remembers the page was entered through AVATAR.
    fn menu_open(&mut self, emulator: &mut dyn Emulator, addr: u32, game: u8, proxy: &EventLoopProxy<UserEvent>) {
        if game != avatar::MENU_GAME_EQUIP && !emulator.write_memory(addr, &[avatar::MENU_GAME_EQUIP]) {
            log::warn!("menu: the cursor write at {addr:#x} failed — AVATAR does not open");
            return;
        }
        self.entered_avatar = true;
        self.menu_publish(proxy);
        self.open(emulator, proxy, "MENU's AVATAR row", true);
    }

    /// Follow the list: its rising edge brings AVATAR back when the pages were entered through
    /// it, its falling edge ends the selection, and a game cursor that moved off EQUIPMENT by
    /// itself ends AVATAR's.
    fn menu_frame(&mut self, emulator: &mut dyn Emulator, proxy: &EventLoopProxy<UserEvent>) {
        let list = if menu_offered() { menu_list(&*emulator) } else { None };
        match list {
            Some((addr, game)) => {
                if !self.menu_up {
                    self.menu_up = true;
                    self.menu_forced = None;
                    self.menu_avatar = std::mem::take(&mut self.entered_avatar);
                    if self.menu_avatar && game != avatar::MENU_GAME_EQUIP && !emulator.write_memory(addr, &[avatar::MENU_GAME_EQUIP]) {
                        self.menu_avatar = false;
                    }
                    if self.menu_avatar {
                        log::info!("menu: back on the list from a page entered through AVATAR — AVATAR stays selected");
                    }
                } else if self.menu_avatar && game != avatar::MENU_GAME_EQUIP {
                    log::info!("menu: the game's cursor left EQUIPMENT ({game}) — AVATAR is no longer selected");
                    self.menu_avatar = false;
                }
            }
            None => {
                if self.menu_up {
                    self.menu_up = false;
                    self.menu_avatar = false;
                    self.menu_forced = None;
                }
                // Back on the field: whatever was entered through AVATAR is over.
                if self.entered_avatar {
                    let read = |a: u32, l: usize| emulator.read_memory(a, l);
                    if items::overlay_screen(&read).is_none() {
                        self.entered_avatar = false;
                    }
                }
            }
        }
        self.menu_publish(proxy);
    }

    /// Tell the UI what the list shows — only when that changed.
    fn menu_publish(&mut self, proxy: &EventLoopProxy<UserEvent>) {
        let sel = if !menu_offered() {
            MenuSel::Off
        } else if self.menu_avatar || self.entered_avatar {
            MenuSel::Row(avatar::MENU_AVATAR)
        } else if let Some(r) = self.menu_forced {
            MenuSel::Row(r)
        } else {
            MenuSel::Game
        };
        if self.menu_published != Some(sel) {
            self.menu_published = Some(sel);
            let _ = proxy.send_event(UserEvent::MenuAvatar(sel));
        }
    }
}

/// Does this build offer the AVATAR row at all? Every anchor the wardrobe needs to open over
/// EQUIPMENT and dress a layer, and the MENU list's cursor.
fn menu_offered() -> bool {
    crate::guest_build::layer_set_fn().is_some()
        && crate::guest_build::equip_mode_addr().is_some()
        && crate::guest_build::menu_cursors().is_some()
}

/// The MENU list, up and idle, and its cursor `(address, row)`.
fn menu_list(emulator: &dyn Emulator) -> Option<(u32, u8)> {
    let read = |a: u32, l: usize| emulator.read_memory(a, l);
    items::menu_list_cursor(&read)
}

/// Re-run the game's own layer setter for one avatar slot, so the figure shows what the hooks
/// now decide (the knob or the worn byte). What it passes is what the game itself passes when
/// the slot's equipment changes: the worn cell's `(cat, idx)` with `remove = 0`, or — for the
/// body layers when the cell is empty — `remove = 1`, which sets the class's own base body
/// (`0x5189a`).
///
/// ★ 170차 an EMPTY 투구: the painter now draws layer 6 without a worn 투구 while the slot shows
/// a costume (`binary_patches/avatar.rs` `handle_helmet_paint_lgt`), so the layer must hold the
/// costume — the setter is called with a stand-in 투구 row (`cat 0 · idx 0`, `remove = 0`) and
/// the join swaps its sheet for the costume's. Only when the hooks WILL swap it (a set with a
/// sheet); otherwise `remove = 1`, the game's own 해제 of a 투구 (it frees the layer and returns —
/// jump table `0x5e2b0`, kind 6 -> `0x51a62`). An empty 무기 · 방패 is not drawn (the painter
/// skips the layer), so there is nothing to re-dress.
fn redress(emulator: &mut dyn Emulator, snap: &Snapshot, slot: usize) {
    let Some(f) = crate::guest_build::layer_set_fn() else { return };
    let eq = av::EQUIPPED_SLOT[slot];
    let (cat, idx) = snap.worn[usize::from(eq)];
    if cat >= 0 && idx >= 0 {
        emulator.call_guest_function(f, &[u32::from(eq), cat as u32, idx as u32, 0]);
    } else if matches!(slot, av::SLOT_ARMOR | av::SLOT_GLOVES | av::SLOT_SHOES) {
        emulator.call_guest_function(f, &[u32::from(eq), 0, 0, 1]);
    } else if slot == av::SLOT_HELMET {
        let shown = helmet_shown(snap);
        emulator.call_guest_function(f, &[u32::from(eq), 0, 0, u32::from(!shown)]);
    }
}

/// Does the 투구 slot show a costume the jar carries — the hooks' own decision (the try-on knob,
/// else the worn byte, and a published sheet)?
fn helmet_shown(snap: &Snapshot) -> bool {
    let set = wie_util::stats::avatar_preview(av::SLOT_HELMET).unwrap_or(snap.wardrobe.wear[av::SLOT_HELMET]);
    av::costume_sprite(av::SLOT_HELMET, 0, set, wie_util::stats::avatar_sheet).is_some()
}

/// A key the page presses on the game's behalf (a tap that left with «3»).
fn press(emulator: &mut dyn Emulator, code: KeyCode) {
    emulator.handle_event(Event::Keydown(code));
    emulator.handle_event(Event::Keyup(code));
}
