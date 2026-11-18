// Host-owned timed buffs bought from the cash shop.
//
// Two of the game's original cash items have no client-side effect at all — their
// `useItem` jump-table index falls past the table's `cmp #0x10` bound (guest
// 0x153f02) and lands on the shared tail 0x154542, which only decrements the stack
// and removes the record at zero. The game therefore consumes them and does
// nothing: they were server-side buffs in the original online game. So the HOST
// owns their effect, and this module owns the clock.
//
// | cat 0x12 idx | original | what it does here |
// |---|---|---|
// | 0x11 럭키스톤 | 네트워크 재료 드랍 2배 (1일) | **EXP x2, 1 week** |
// | 0x12 럭키쥬엘 | 네트워크 재료 드랍 2배 (1주일) | **EXP x4, 1 week** |
// | 0x18 초월의문장 | 장비레벨제한 무시 (1주일) | **equip level requirement -5, 1 week** |
//
// The player uses them the normal way: their inventory popup's row 0 IS `사용하기`
// (the builder at guest 0x12d12a picks text id 0x21 for every item except 0x0b),
// `popup_choose` routes row 0 to `useItem(tab, slot)`, and the tail consumes one.
// The host notices the confirmation and starts the clock here.
//
// ★ Expiry is stored as absolute UNIX MILLISECONDS, not as "ticks remaining":
// the app is killed and relaunched constantly on Android, and a tick counter would
// silently pause a "1 week" buff whenever the player was not playing. It also means
// a buff cannot be extended by keeping the app closed.
//
// ⚠ These durations are NOT part of the game's save file — they are ours. Losing
// the host config loses the buff; that is the accepted trade for not having to
// reproduce the save format's encryption (see `RE_SAVE_LEVELCAP_EXP_POINTS.md` §2).
//
// Dependency-free on purpose: `buffs_selftest.rs` at the repo root `include!`s this
// file, so **do not add imports here**.

/// One week in milliseconds — the duration all three items carry in the original
/// item DB (`dat/i18_dat`: "사용후 1주일간").
pub const WEEK_MS: u64 = 7 * 24 * 60 * 60 * 1000;
/// One hour in milliseconds — the unit the random-box rewards are measured in.
pub const HOUR_MS: u64 = 60 * 60 * 1000;

/// Consumable/cash category in the item DB (`i18_dat`).
pub const CASH_CAT: u16 = 0x12;
/// 럭키스톤 -> EXP x2.
pub const IDX_EXP_X2: u16 = 0x11;
/// 럭키쥬엘 -> EXP x4.
pub const IDX_EXP_X4: u16 = 0x12;
/// 초월의문장 -> equip level requirement relief.
pub const IDX_LEVEL_RELIEF: u16 = 0x18;

// ── random boxes (2026-08-05) ────────────────────────────────────────────────
//
// Cash items whose `useItem` arm is the consume-only tail (guest 0x154542): the two
// 비밀의상자 (and, until 2026-08-22, the 오브상자). Their original payouts were
// server-side item drops, and the host had NO safe way to create an inventory
// record (the 0x3c-byte bag record layout is only partially RE'd, and writing a
// guessed one risks bag corruption — the same reason 오브 결합 was refused, see
// TRACK_C_PRIVATE_SERVER.md 세션 3-i). So the boxes pay out of the effects the
// host CAN deliver safely: the two proven code patches (EXP multiplier, equip
// relief) on short timers, and a full HP/SP restore (the same two field writes
// the game's own revive at guest 0x124458 performs). The 오브원석 item itself is
// obtainable natively instead — it is listed in the cash shop, and it is one of the
// rows the CLIENT still grants, over its own device-verified `05 40` path.
//
// ★★ 2026-08-22 — THE 오브상자 IS NO LONGER ONE OF THEM (65차, player: "오브상자
// 프리미엄 버프 토스트 없애줘 그거 왜있는거야? 오브상자는 그냥 오브를 얻는 용도인건데?").
// They are right, and the premise above EXPIRED: the buff pool was a 2026-08-05
// stand-in for a capability we did not have — dropping a real orb — and 44차 built
// exactly that. `offline::new_orb` / the server's `_new_orb_record` write a known
// 20-byte cat-0x10 record into the NETWORK warehouse, one per unit, which is both
// safe (it never touches the bag) and what the item's own description says it does
// ("랜덤하게 오브가 드랍되는 상자"). A buff on top was a substitute outliving its
// substitution. ⛔ Do not put `IDX_ORB_BOX` back in the pool.

/// 비밀의상자 — "무엇이 나올지 모르는 상자" (two DB entries, same behaviour).
pub const IDX_SECRET_BOX_A: u16 = 0x0a;
pub const IDX_SECRET_BOX_B: u16 = 0x14;
/// 오브상자 — "랜덤하게 오브가 드랍되는 상자". **Not a host-paid box** (2026-08-22): the
/// engine delivers a real orb per unit straight into the player's BAG (2026-08-25; it went
/// to the network warehouse until the in-game combine made that detour pointless), so the
/// host owes nothing.
/// See [`is_orb_box`] and the comment above.
pub const IDX_ORB_BOX: u16 = 0x19;

/// 천상의 영약 (2026-08-13, player request "즉시 레벨 +1업") — DB record 0x15, the
/// former 신의의문장 (a guild-proc no-op on this client; the asset retitles it, see
/// `tools_re/rename_cash_items.py`). Not a timed buff and not a box: its whole
/// effect is one deferred guest call — `exp = expToNext + 1`, then the game's own
/// `checkLevelUp` (`guest_build::level_up_fn`) once the player is back on the
/// field, so points/caps/HP·SP refills/visuals are all game code.
pub const IDX_LEVEL_ELIXIR: u16 = 0x15;

/// Is `(cat, idx)` the 천상의 영약? (Kept next to `grant_for`/`is_box` so every
/// cash-item arm the host answers for is declared in one screenful.)
pub fn is_level_elixir(cat: u16, idx: u16) -> bool {
    cat == CASH_CAT && idx == IDX_LEVEL_ELIXIR
}

// ── 소켓확장 (2026-08-14) ────────────────────────────────────────────────────
//
// 34차 delisted this item as an undeliverable no-op (its `useItem` arm is the
// consume-only tail at guest 0x154542 — the original "장비 소켓 +1" was a dead
// online-server effect). 38차 brought it back as a HOST effect: one byte written to
// `+0x3c6` of the first equipped item that had room.
//
// ★ THAT DESIGN WAS WRONG AND IS GONE (44차). The player's actual ask was
//   "네트워크 창고에서 소켓확장 아이템을 사용하면 **오브처럼** 소켓확장을 시킬 아이템을
//   선택할 수 있게 해야해" — the point is CHOOSING the target, and choosing it among
//   everything in the warehouse rather than only among what is currently worn. Picking
//   "the first equipped item with room" answered neither half.
//
// It now runs entirely through the game's own 오브결합 picker instead: the server
// delivers the item into the network warehouse wearing category 0x10, the client's own
// gate (`0x7674 cmp r3,#0x10`) offers 오브결합 on it, and the server's `_orb_combine`
// recognises the index and adds a socket. No host write, no guessing at a target — see
// `hero3_server/hero3_protocol.py::ORB_IDX_SOCKET_EXPAND`.
//
// What remains here is recognition only, so that a stale copy sitting in someone's bag
// from before this change says something instead of silently vanishing.
/// 소켓확장 — "장비의 소켓을 하나 늘린다". Still the cash-shop row's index; the item is
/// now delivered to the network warehouse as an orb, not used from the bag.
pub const IDX_SOCKET_EXPAND: u16 = 0x1a;

/// Is `(cat, idx)` the 소켓확장?
pub fn is_socket_expand(cat: u16, idx: u16) -> bool {
    cat == CASH_CAT && idx == IDX_SOCKET_EXPAND
}

/// What to say when an inert copy of the SHOP ROW (not the orb) is used from the bag.
///
/// ★ These are leftovers from builds older than 65차, which used to ack the purchase and
/// let the client grant itself the cat-0x12 row. Nothing creates them any more.
///
/// `&'static str`: `UserEvent::Notice` carries one (the line is raised on the worker and
/// read on the UI thread). Since 140차 it is shown as the game-style message box, through
/// the game's own font (`gamefont`, every KS X 1001 syllable) — the old "ASCII only" rule
/// came from the 5x7 overlay font that no longer draws any of these.
pub const SOCKET_EXPAND_BAG_NOTICE: &str = "예전 소켓확장 사본입니다. 다시 구매해 주세요.";

/// What a random box pays out. Every arm is an effect the host already delivers
/// through a proven primitive — nothing here invents a new guest write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxReward {
    /// A timed EXP buff (multiplier 2 or 4) for `ms` milliseconds.
    Exp { mult: u8, ms: u64 },
    /// Timed equip-level relief for `ms` milliseconds.
    Relief { ms: u64 },
    /// Full HP + SP, the revive routine's own two writes.
    FullRestore,
}

/// Is `(cat, idx)` one of the random boxes?
pub fn is_box(cat: u16, idx: u16) -> bool {
    cat == CASH_CAT && matches!(idx, IDX_SECRET_BOX_A | IDX_SECRET_BOX_B)
}

/// Is `(cat, idx)` the 오브상자? Its real payout is the orb the ENGINE delivers into the
/// bag, so the host pays nothing for it — this exists only so an INERT copy of the shop
/// row, left in someone's bag by a build older than 65차, can say so rather than looking
/// broken. Exactly like
/// [`is_socket_expand`] / [`SOCKET_EXPAND_BAG_NOTICE`].
pub fn is_orb_box(cat: u16, idx: u16) -> bool {
    cat == CASH_CAT && idx == IDX_ORB_BOX
}

/// What to say when an 오브상자 is used from the BAG. See [`is_orb_box`].
pub const ORB_BOX_BAG_NOTICE: &str = "예전 오브상자 사본입니다. 다시 구매해 주세요.";

/// The reward a box pays for a given `roll` (any u64 — the caller supplies the
/// randomness; `roll % pool_len` picks the arm, so every arm is reachable and the
/// mapping is deterministic and testable).
pub fn box_reward(cat: u16, idx: u16, roll: u64) -> Option<BoxReward> {
    if cat != CASH_CAT {
        return None;
    }
    let pool: &[BoxReward] = match idx {
        IDX_SECRET_BOX_A | IDX_SECRET_BOX_B => &[
            BoxReward::FullRestore,
            BoxReward::Exp { mult: 2, ms: 6 * HOUR_MS },
            BoxReward::Exp { mult: 2, ms: 24 * HOUR_MS },
            BoxReward::Relief { ms: 24 * HOUR_MS },
            BoxReward::Exp { mult: 4, ms: 6 * HOUR_MS },
        ],
        _ => return None,
    };
    Some(pool[(roll % pool.len() as u64) as usize])
}

/// The line for a box reward, shown as the game-style message box (140차). Static
/// because `UserEvent::Notice` carries a `&'static str`; ASCII by habit — the box goes
/// through the game's font now, so nothing forbids Hangul here any more.
pub fn box_reward_notice(r: BoxReward) -> &'static str {
    match r {
        BoxReward::FullRestore => "BOX - FULL RESTORE",
        BoxReward::Exp { mult: 4, ms } if ms == 12 * HOUR_MS => "BOX - EXP X4 12H",
        BoxReward::Exp { mult: 4, .. } => "BOX - EXP X4 6H",
        BoxReward::Exp { mult: 2, ms } if ms == 48 * HOUR_MS => "BOX - EXP X2 48H",
        BoxReward::Exp { mult: 2, ms } if ms == 24 * HOUR_MS => "BOX - EXP X2 24H",
        BoxReward::Exp { .. } => "BOX - EXP X2 6H",
        BoxReward::Relief { ms } if ms == 48 * HOUR_MS => "BOX - EQUIP LV-5 48H",
        BoxReward::Relief { .. } => "BOX - EQUIP LV-5 24H",
    }
}

/// How many levels 초월의문장 shaves off an item's requirement (player choice
/// 2026-08-04: "낄 수 있는 아이템 레벨제한이 5레벨 줄어듭니다").
///
/// ★ Bounded by the patch's encoding, not by taste: the relief is applied by
/// rewriting `adds r1,r3,#0` into `subs r1,r3,#imm3` at guest 0x13d0aa, and Thumb-1
/// `imm3` is three bits. 8 or more would need a different instruction.
pub const RELIEF_LEVELS: u8 = 5;
pub const RELIEF_LEVELS_MAX: u8 = 7;

/// What a used cash item grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    /// EXP multiplier (2 or 4).
    Exp(u8),
    /// Equipment level-requirement relief, in levels.
    LevelRelief(u8),
}

/// Push one expiry stamp out by `ms`, from whichever is later: the stamp we already
/// hold, or now. Written out rather than inlined so "stacking adds, it never resets"
/// is one statement in one place.
fn extend_by(slot: &mut u64, now: u64, ms: u64) {
    *slot = (*slot).max(now).saturating_add(ms);
}

/// The grant for a `(category, index)` pair, or `None` for every other item.
pub fn grant_for(cat: u16, idx: u16) -> Option<Grant> {
    if cat != CASH_CAT {
        return None;
    }
    match idx {
        IDX_EXP_X2 => Some(Grant::Exp(2)),
        IDX_EXP_X4 => Some(Grant::Exp(4)),
        IDX_LEVEL_RELIEF => Some(Grant::LevelRelief(RELIEF_LEVELS)),
        _ => None,
    }
}

/// Persisted buff state: one expiry stamp per multiplier tier, plus the relief.
///
/// The two EXP tiers are tracked SEPARATELY rather than as one "current multiplier
/// + expiry", so that using an x2 while an x4 is running cannot shorten the x4 (and
/// vice versa). The effective multiplier is then simply the largest live one — see
/// [`Buffs::exp_multiplier`]. There is only one divisor byte in the guest, so the
/// tiers cannot compose; taking the maximum is the only behaviour that never takes
/// something away from the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Buffs {
    /// UNIX ms at which EXP x2 stops. 0 = never granted.
    pub exp_x2_until: u64,
    /// UNIX ms at which EXP x4 stops.
    pub exp_x4_until: u64,
    /// UNIX ms at which the equip level relief stops.
    pub relief_until: u64,
}

impl Buffs {
    /// Effective EXP multiplier at `now`: 4, 2, or 1 (no buff).
    ///
    /// Only 1 / 2 / 4 are representable: the patch replaces the `movs r1,#100`
    /// divisor at guest 0x14654e, and 100 must stay exactly divisible or the
    /// multiplier would drift (100/3 = 33 is x3.03, not x3).
    pub fn exp_multiplier(&self, now: u64) -> u8 {
        if self.exp_x4_until > now {
            4
        } else if self.exp_x2_until > now {
            2
        } else {
            1
        }
    }

    /// Levels of equip-requirement relief at `now` (0 = none).
    pub fn relief_levels(&self, now: u64) -> u8 {
        if self.relief_until > now { RELIEF_LEVELS } else { 0 }
    }

    /// Apply a grant at `now` for the standard one-week duration. See [`apply_timed`].
    pub fn apply(&mut self, grant: Grant, now: u64) {
        self.apply_timed(grant, now, WEEK_MS);
    }

    /// Apply a grant at `now` for `dur_ms`, extending from whichever is later: the
    /// current expiry or now. Stacking two of the same buff therefore adds the full
    /// duration rather than throwing the remainder away. The random boxes come
    /// through here with their own (shorter) timers.
    pub fn apply_timed(&mut self, grant: Grant, now: u64, dur_ms: u64) {
        match grant {
            Grant::Exp(2) => extend_by(&mut self.exp_x2_until, now, dur_ms),
            Grant::Exp(4) => extend_by(&mut self.exp_x4_until, now, dur_ms),
            // Any other multiplier cannot be represented by the divisor patch, so it
            // is refused rather than silently rounded.
            Grant::Exp(_) => {}
            Grant::LevelRelief(_) => extend_by(&mut self.relief_until, now, dur_ms),
        }
    }

    /// Is anything live? (Used to decide whether the host has to keep re-asserting
    /// its guest patches at all.)
    pub fn any_live(&self, now: u64) -> bool {
        self.exp_multiplier(now) != 1 || self.relief_levels(now) != 0
    }

    /// Whole minutes left on the longest live buff, for the HUD / log.
    pub fn minutes_left(&self, now: u64) -> u64 {
        let far = self.exp_x2_until.max(self.exp_x4_until).max(self.relief_until);
        far.saturating_sub(now) / 60_000
    }

    /// One HUD line per LIVE buff, strongest first — what the overlay shows under
    /// the gauges while a game window is open (player request 2026-08-05).
    ///
    /// ★ 2026-08-06 — the CASH ITEM's Korean name, not the effect (player: "초월의
    /// 문장, 경험의 보주 등등 현재 버프 적용되고있는거 한글로 표시했으면 좋겠어").
    /// This used to be ASCII effect names ("EXP X2") purely because the overlay's 5x7
    /// font had no Hangul; `crate::hangul` now carries 12x12 bitmaps for exactly the
    /// syllables below.
    ///
    /// ⚠ Every non-ASCII character produced here MUST have a glyph — the overlay
    /// silently skips one that does not, which would print a hole rather than fail.
    /// `tools_re/gen_hangul_glyphs.py` derives its syllable set from the same words,
    /// and the selftest asserts `hangul::covers()` over every line this can return.
    ///
    /// The names match `hero3_server/CASH_SHOP.md`: 0x11 경험의비석 (x2), 0x12
    /// 경험의보주 (x4), 0x18 초월의문장 (equip level requirement -5).
    pub fn status_lines(&self, now: u64) -> Vec<String> {
        let mut out = Vec::new();
        for (label, until) in [
            ("경험의보주", self.exp_x4_until),
            ("경험의비석", self.exp_x2_until),
            ("초월의문장", self.relief_until),
        ] {
            if until > now {
                out.push(format!("{label} {}", fmt_time_left(until - now)));
            }
        }
        out
    }
}

// ── uncommitted cash use (2026-08-14) ────────────────────────────────────────
//
// A cash item's EFFECT and its CONSUMPTION land on disk in two different places at
// two different times. The host writes the effect the instant the player confirms
// (`wie_buffs.txt`, `wie_levelups.txt`); the stack decrement is the GUEST's, and it
// only reaches disk when a save runs — which cannot happen with the inventory
// window open, so the host arms a retry that fires once the player is back on a
// saveable field.
//
// Kill the app in that gap and the two disagree: the guest save still holds the
// item, the host files already hold the effect. That is a free duplicate, and it is
// exactly what the player reported ("경험의 보주를 사용한 뒤에 저장을 안 하면 …
// 다시 경험의 보주를 사용할 수 있다").
//
// The fix is a rollback note, not a lock. Before the FIRST uncommitted use the host
// writes down the state it is leaving; any completed save throws the note away. If
// the app dies with the note still on disk, the guest save is by definition the
// pre-use one, so restoring the note puts both sides back in step together — the
// item comes back and so does the un-spent effect. Nothing is invented and nothing
// is held back from the player: the effect still applies immediately while playing.
//
// Locking the other way (defer the effect until the save lands, §39차 5-b) was the
// alternative and is worse: the player pays for 경험의 보주 and gets nothing until
// they walk back to a field, which is a visible regression for the common case in
// order to fix the rare one.

/// The host-side state a completed save has already accounted for.
///
/// Only the two host-owned files can diverge from the guest save: the timed buffs
/// and the pending 천상의 영약 level-ups. Everything else a cash item does (소켓확장,
/// 부활서's full restore) is a write into guest RAM, which rolls back with the guest
/// save on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CashBaseline {
    pub buffs: Buffs,
    pub levelups: u32,
}

/// Serialise a baseline in the same one-`key value`-per-line shape the buff file
/// already uses, so the two are readable side by side when debugging on a device.
pub fn serialise_baseline(b: &CashBaseline) -> String {
    format!(
        "exp_x2 {}\nexp_x4 {}\nrelief {}\nlevelups {}\n",
        b.buffs.exp_x2_until, b.buffs.exp_x4_until, b.buffs.relief_until, b.levelups
    )
}

/// Parse a baseline, or `None` if the note is not completely intact.
///
/// ⚠ The policy here is the OPPOSITE of [`Buffs`]'s own file, which degrades a bad
/// line to "no buff". A baseline is applied by OVERWRITING live state, so a half-read
/// note would wipe a week the player actually paid for. Refusing to roll back leaves
/// at worst one duplicated consumable; rolling back from garbage takes something real
/// away. All four keys must be present and parse, or this returns `None`.
pub fn parse_baseline(text: &str) -> Option<CashBaseline> {
    let (mut exp_x2, mut exp_x4, mut relief, mut levelups) = (None, None, None, None);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else {
            continue;
        };
        match key {
            "exp_x2" => exp_x2 = val.parse::<u64>().ok(),
            "exp_x4" => exp_x4 = val.parse::<u64>().ok(),
            "relief" => relief = val.parse::<u64>().ok(),
            "levelups" => levelups = val.parse::<u32>().ok(),
            _ => {}
        }
    }
    Some(CashBaseline {
        buffs: Buffs { exp_x2_until: exp_x2?, exp_x4_until: exp_x4?, relief_until: relief? },
        levelups: levelups?,
    })
}

/// `ms` as a short remaining-time tag: "6일 23시간", "5시간 12분" or "42분". Never
/// empty and never "0분" while any time remains — a live buff that printed nothing
/// would read as expired.
fn fmt_time_left(ms: u64) -> String {
    let mins = ms / 60_000;
    let (d, h, m) = (mins / 1440, (mins % 1440) / 60, mins % 60);
    if d > 0 {
        format!("{d}일 {h}시간")
    } else if h > 0 {
        format!("{h}시간 {m}분")
    } else {
        format!("{}분", m.max(1))
    }
}

// ── guest patch encodings ────────────────────────────────────────────────────
//
// All are size-neutral rewrites (one halfword, or on LGT's equip check a two-halfword
// pair of the same total size), verified against the original bytes before writing
// (the same contract `items::hud_bottom_patch` uses). Kept here beside the buff that
// drives them so the encoding and its bound live together. Which SHAPE a build takes
// is `guest_build::equip_relief_site()` / `exp_divisor_site()`.

/// `subs r1, r3, #levels` — replaces `adds r1, r3, #0` (0x1C19) at guest 0x13d0aa so
/// the equip check compares against `requiredLevel - levels`.
///
/// Thumb-1 `subs rd,rn,#imm3` = `0x1E00 | imm3<<6 | rn<<3 | rd`; here rn=3, rd=1.
/// `levels == 0` gives back the original `adds r1,r3,#0`, i.e. the un-patched game.
pub fn equip_relief_halfword(levels: u8) -> Option<u16> {
    if levels > RELIEF_LEVELS_MAX {
        return None;
    }
    if levels == 0 {
        return Some(0x1C19);
    }
    Some(0x1E00 | (u16::from(levels) << 6) | (3 << 3) | 1)
}

/// The LGT shape of the same relief: `[ldrh r1,[r4,#0] ; adds r1,#levels]` replacing
/// the shipped `[movs r3,#0 ; ldrsh r1,[r4,r3]]` at guest `0x477ce`, so the equip check
/// compares `playerLevel + levels` against the shipped requirement. `levels == 0` gives
/// back the original pair — the exact `ldrsh` form — so un-patching is byte-perfect.
///
/// Thumb-1: `movs rd,#imm8` = `0x2000|rd<<8|imm8`; `ldrsh rt,[rn,rm]` =
/// `0x5E00|rm<<6|rn<<3|rt`; `ldrh rt,[rn,#imm5*2]` = `0x8800|imm5<<6|rn<<3|rt`;
/// `adds rdn,#imm8` = `0x3000|rdn<<8|imm8`. Here rn=4 (the level's address), rt=rdn=1.
///
/// ★ The PLAYER LEVEL is biased up rather than the requirement biased down, and that
/// choice is load-bearing rather than stylistic — `guest_anchors::EquipReliefSite::
/// LevelBias` carries the argument. Short version: rewriting the requirement load
/// turns an `ldrsb` into an `ldrb`, which would flip any item whose requirement byte is
/// ≥ 0x80 from allowed to blocked; the level side loses nothing, because the field is
/// `CHAR+0xe2` and the game caps it at `items::LEVEL_CAP` (70; 99 since the 111차 patch),
/// so `ldrh` and `ldrsh` agree on every value it can hold.
///
/// The `adds` immediate could hold 255 where KTF's `imm3` holds 7, but the bound stays
/// [`RELIEF_LEVELS_MAX`] so the two builds cannot drift apart on what an item grants.
pub fn equip_relief_level_bias(levels: u8) -> Option<[u16; 2]> {
    if levels > RELIEF_LEVELS_MAX {
        return None;
    }
    if levels == 0 {
        return Some([0x2300, 0x5EE1]);
    }
    Some([0x8821, 0x3100 | u16::from(levels)])
}

/// `movs r1, #(100 / mult)` — replaces `movs r1, #100` (0x2164) at guest 0x14654e,
/// the divisor of the kill-EXP formula `gain = base * expField * factor / 100`.
///
/// Only exact divisors of 100 are accepted, so the multiplier is exact.
pub fn exp_divisor_halfword(mult: u8) -> Option<u16> {
    if mult == 0 || 100 % u32::from(mult) != 0 {
        return None;
    }
    Some(0x2100 | (100 / u16::from(mult)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000_000;

    #[test]
    fn only_the_three_repurposed_items_grant_anything() {
        assert_eq!(grant_for(0x12, 0x11), Some(Grant::Exp(2)));
        assert_eq!(grant_for(0x12, 0x12), Some(Grant::Exp(4)));
        assert_eq!(grant_for(0x12, 0x18), Some(Grant::LevelRelief(5)));
        // Neighbours that must stay untouched: 포션(0), 오브원석(7), 부활의서(0x0b),
        // 여행가방(0x0e), 스탯리셋(0x0f), 스킬리셋(0x10), 명예의문장(0x17), 오브상자(0x19).
        for idx in [0u16, 1, 7, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x13, 0x17, 0x19] {
            assert_eq!(grant_for(0x12, idx), None, "idx {idx:#x} must not grant a buff");
        }
        // Another category with the same indices must not match.
        for cat in [0u16, 1, 0x0d, 0x0f, 0x11, 0x13, 0x15] {
            assert_eq!(grant_for(cat, 0x11), None, "cat {cat:#x} must not grant a buff");
        }
    }

    #[test]
    fn the_level_elixir_is_its_own_arm_and_nothing_else_claims_it() {
        assert!(is_level_elixir(0x12, IDX_LEVEL_ELIXIR));
        assert!(!is_level_elixir(0x12, 0x16), "배신의문장 stays a no-op");
        assert!(!is_level_elixir(0x0d, IDX_LEVEL_ELIXIR), "category must match too");
        // The record it repurposes must not double as a buff or a box: exactly one
        // host arm may answer for one (cat, idx), or a use would grant twice.
        assert_eq!(grant_for(0x12, IDX_LEVEL_ELIXIR), None);
        assert!(!is_box(0x12, IDX_LEVEL_ELIXIR));
    }

    #[test]
    fn a_week_is_a_week_and_it_expires() {
        let mut b = Buffs::default();
        assert_eq!(b.exp_multiplier(T0), 1);
        b.apply(Grant::Exp(2), T0);
        assert_eq!(b.exp_x2_until, T0 + WEEK_MS);
        assert_eq!(b.exp_multiplier(T0), 2);
        assert_eq!(b.exp_multiplier(T0 + WEEK_MS - 1), 2);
        // Strictly `>`: at the stamp itself it is over.
        assert_eq!(b.exp_multiplier(T0 + WEEK_MS), 1);
    }

    #[test]
    fn x4_wins_while_live_and_x2_survives_underneath() {
        let mut b = Buffs::default();
        b.apply(Grant::Exp(2), T0);
        b.apply(Grant::Exp(4), T0 + 1000);
        assert_eq!(b.exp_multiplier(T0 + 2000), 4);
        // The x4 ends first only if it was granted earlier; here it ends last, so
        // check the other order too: an x2 granted later must not shorten the x4.
        let mut c = Buffs::default();
        c.apply(Grant::Exp(4), T0);
        c.apply(Grant::Exp(2), T0 + WEEK_MS / 2);
        assert_eq!(c.exp_x4_until, T0 + WEEK_MS);
        assert_eq!(c.exp_multiplier(T0 + WEEK_MS - 1), 4);
        // Once the x4 lapses the x2 is still running and takes over.
        assert_eq!(c.exp_multiplier(T0 + WEEK_MS), 2);
    }

    #[test]
    fn stacking_the_same_item_adds_a_full_week_instead_of_resetting() {
        let mut b = Buffs::default();
        b.apply(Grant::Exp(2), T0);
        b.apply(Grant::Exp(2), T0 + 1000);
        assert_eq!(b.exp_x2_until, T0 + WEEK_MS + WEEK_MS, "the remainder was thrown away");
        // Using one AFTER it lapsed starts from now, not from the stale stamp.
        let mut c = Buffs::default();
        c.apply(Grant::Exp(2), T0);
        let late = T0 + 3 * WEEK_MS;
        c.apply(Grant::Exp(2), late);
        assert_eq!(c.exp_x2_until, late + WEEK_MS);
    }

    #[test]
    fn relief_is_five_levels_and_independent_of_exp() {
        let mut b = Buffs::default();
        assert_eq!(b.relief_levels(T0), 0);
        b.apply(Grant::LevelRelief(RELIEF_LEVELS), T0);
        assert_eq!(b.relief_levels(T0), 5);
        assert_eq!(b.exp_multiplier(T0), 1, "relief must not touch the EXP tiers");
        assert_eq!(b.relief_levels(T0 + WEEK_MS), 0);
    }

    /// The two halfwords the host writes into guest RAM. Pinned as literals because
    /// a wrong encoding here is a guest crash, and because the ORIGINAL values are
    /// what the writer compares against before touching anything.
    #[test]
    fn patch_halfwords_match_the_hand_decoded_thumb() {
        // `adds r1,r3,#0` — the instruction at guest 0x13d0aa in the shipped binary.
        assert_eq!(equip_relief_halfword(0), Some(0x1C19));
        // `subs r1,r3,#5` — what -5 relief writes.
        assert_eq!(equip_relief_halfword(5), Some(0x1F59));
        // The whole imm3 range, so an off-by-one in the shift is impossible.
        let want = [0x1C19u16, 0x1E59, 0x1E99, 0x1ED9, 0x1F19, 0x1F59, 0x1F99, 0x1FD9];
        for (levels, w) in want.iter().enumerate() {
            assert_eq!(equip_relief_halfword(levels as u8), Some(*w), "levels {levels}");
        }
        assert_eq!(equip_relief_halfword(8), None, "imm3 cannot hold 8");

        // `movs r1,#100` — the divisor at guest 0x14654e in the shipped binary.
        assert_eq!(exp_divisor_halfword(1), Some(0x2164));
        assert_eq!(exp_divisor_halfword(2), Some(0x2132)); // #50  -> exactly x2
        assert_eq!(exp_divisor_halfword(4), Some(0x2119)); // #25  -> exactly x4
        // Anything that does not divide 100 exactly is refused rather than rounded.
        for m in [3u8, 6, 7, 8, 9, 11, 0] {
            assert_eq!(exp_divisor_halfword(m), None, "x{m} is not an exact divisor of 100");
        }
        // Every accepted multiplier really is 100/divisor.
        for m in [1u8, 2, 4, 5, 10, 20, 25, 50, 100] {
            let hw = exp_divisor_halfword(m).unwrap();
            assert_eq!(hw & 0xff00, 0x2100, "x{m} is not a `movs r1,#imm8`");
            assert_eq!(100 / u32::from(hw & 0xff), u32::from(m));
        }
    }

    /// The LGT pair, pinned the same way. A wrong encoding here is a guest crash on the
    /// build it ships to, and the ORIGINAL pair is what the writer compares against
    /// before touching anything. Both halfwords below were also read back out of
    /// RUNNING guest memory (`wie_probe --poke 7850:0x477ce:21880531` logged
    /// `0023e15e`), so this is pinned against measurement and not only disassembly.
    #[test]
    fn lgt_relief_level_bias_matches_the_hand_decoded_thumb() {
        // `movs r3,#0 ; ldrsh r1,[r4,r3]` — the shipped pair at LGT guest 0x477ce.
        assert_eq!(equip_relief_level_bias(0), Some([0x2300, 0x5EE1]));
        // `ldrh r1,[r4,#0] ; adds r1,#5` — what -5 relief writes.
        assert_eq!(equip_relief_level_bias(5), Some([0x8821, 0x3105]));
        // The whole accepted range keeps the load fixed and moves only the imm8.
        for levels in 1..=RELIEF_LEVELS_MAX {
            let [load, add] = equip_relief_level_bias(levels).unwrap();
            assert_eq!(load, 0x8821, "levels {levels}: the ldrh must not vary");
            assert_eq!(add & 0xff00, 0x3100, "levels {levels}: not an adds r1,#imm8");
            assert_eq!((add & 0xff) as u8, levels);
        }
        // Same bound as the KTF encoder, though imm8 could hold more — the two
        // builds must agree on what an item grants.
        assert_eq!(equip_relief_level_bias(RELIEF_LEVELS_MAX + 1), None);
        // ★ The rewrite must never land on the REQUIREMENT load. That is the `ldrsb`
        // whose signedness 7차 rejected dropping (see `equip_relief_level_bias`), and
        // its bytes live two halfwords further on, so an off-by-one in the table would
        // reintroduce exactly the bug this design avoids.
        for levels in 0..=RELIEF_LEVELS_MAX {
            let pair = equip_relief_level_bias(levels).unwrap();
            assert_ne!(pair[0], 0x7893, "levels {levels}: that is the rejected ldrb rewrite");
            assert_ne!(pair, [0x2302, 0x56D3], "levels {levels}: that is the requirement load");
        }
    }

    #[test]
    fn boxes_are_exactly_the_two_no_arm_chests() {
        assert!(is_box(0x12, 0x0a));
        assert!(is_box(0x12, 0x14));
        // ⛔ 오브상자 (0x19) was the THIRD until 2026-08-22. It pays nothing from the host
        //    now: the engine drops a real orb per unit into the network warehouse, which is
        //    what the item says it does, and the buff pool was only ever a stand-in for
        //    that. See `is_orb_box`.
        assert!(!is_box(0x12, 0x19), "the 오브상자 is not a host-paid box");
        // The buff items and neighbours are NOT boxes, and no other category is.
        for idx in [0u16, 7, 0x0b, 0x0d, 0x11, 0x12, 0x13, 0x18] {
            assert!(!is_box(0x12, idx), "idx {idx:#x} must not be a box");
        }
        assert!(!is_box(0x0d, 0x19));
    }

    #[test]
    fn box_rewards_are_deterministic_and_every_arm_is_reachable() {
        for idx in [IDX_SECRET_BOX_A, IDX_SECRET_BOX_B] {
            let mut seen = Vec::new();
            for roll in 0..64u64 {
                let r = box_reward(0x12, idx, roll).expect("a box always pays");
                assert_eq!(box_reward(0x12, idx, roll), Some(r), "same roll, same reward");
                if !seen.contains(&r) {
                    seen.push(r);
                }
            }
            assert!(seen.len() >= 4, "box {idx:#x} reached only {} arms", seen.len());
            // Every timed arm must be REPRESENTABLE: the patches only encode x2/x4
            // and relief 0..=7, so an unencodable pool entry would silently no-op.
            for r in &seen {
                match *r {
                    BoxReward::Exp { mult, ms } => {
                        assert!(exp_divisor_halfword(mult).is_some(), "x{mult} unencodable");
                        assert!(ms > 0);
                    }
                    BoxReward::Relief { ms } => assert!(ms > 0),
                    BoxReward::FullRestore => {}
                }
                // Every reward has a toast, and it is font-safe ASCII (no ':').
                let t = box_reward_notice(*r);
                assert!(!t.is_empty() && t.is_ascii() && !t.contains(':'), "bad toast {t:?}");
            }
        }
        assert_eq!(box_reward(0x12, 0x0b, 0), None, "부활의서 is not a box");
        assert_eq!(box_reward(0x11, IDX_SECRET_BOX_A, 0), None, "wrong category");
        assert_eq!(box_reward(0x12, IDX_ORB_BOX, 0), None, "⛔ the 오브상자 pays nothing");
    }

    #[test]
    fn timed_grants_stack_by_their_own_duration() {
        let mut b = Buffs::default();
        b.apply_timed(Grant::Exp(2), T0, 6 * HOUR_MS);
        assert_eq!(b.exp_x2_until, T0 + 6 * HOUR_MS);
        assert_eq!(b.exp_multiplier(T0 + 6 * HOUR_MS - 1), 2);
        assert_eq!(b.exp_multiplier(T0 + 6 * HOUR_MS), 1);
        // A box grant on top of a week-long item extends from the LATER stamp.
        let mut c = Buffs::default();
        c.apply(Grant::Exp(2), T0);
        c.apply_timed(Grant::Exp(2), T0 + 1000, 6 * HOUR_MS);
        assert_eq!(c.exp_x2_until, T0 + WEEK_MS + 6 * HOUR_MS);
    }

    #[test]
    fn status_lines_name_live_buffs_by_their_korean_item_name() {
        let mut b = Buffs::default();
        assert!(b.status_lines(T0).is_empty());
        b.apply(Grant::Exp(2), T0);
        b.apply(Grant::LevelRelief(RELIEF_LEVELS), T0);
        let lines = b.status_lines(T0);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "경험의비석 7일 0시간");
        assert_eq!(lines[1], "초월의문장 7일 0시간");
        // The x4 outranks the x2, so it is listed first — and it is a DIFFERENT item.
        b.apply(Grant::Exp(4), T0);
        assert_eq!(b.status_lines(T0)[0], "경험의보주 7일 0시간");
        // The overlay font still has no ':' (it is drawn from the 5x7 ASCII table).
        for l in b.status_lines(T0) {
            assert!(!l.contains(':'), "bad line {l:?}");
        }
        // Sub-day and sub-hour formats, and the never-"0분" floor.
        let mut s = Buffs::default();
        s.apply_timed(Grant::Exp(2), T0, 5 * HOUR_MS + 12 * 60_000);
        assert_eq!(s.status_lines(T0), vec!["경험의비석 5시간 12분".to_string()]);
        let mut m = Buffs::default();
        m.apply_timed(Grant::Exp(2), T0, 90_000);
        assert_eq!(m.status_lines(T0), vec!["경험의비석 1분".to_string()]);
        let mut tiny = Buffs::default();
        tiny.apply_timed(Grant::Exp(2), T0, 500);
        assert_eq!(tiny.status_lines(T0), vec!["경험의비석 1분".to_string()], "live must never print 0분");
        assert!(tiny.status_lines(T0 + 500).is_empty());
    }

    #[test]
    fn any_live_and_minutes_left_track_the_longest_buff() {
        let mut b = Buffs::default();
        assert!(!b.any_live(T0));
        b.apply(Grant::Exp(2), T0);
        assert!(b.any_live(T0));
        assert_eq!(b.minutes_left(T0), 7 * 24 * 60);
        assert!(!b.any_live(T0 + WEEK_MS));
        assert_eq!(b.minutes_left(T0 + WEEK_MS), 0);
    }
}
