// The tower — 하늘성 / 가디언의 전당. The FLOOR STATE MACHINE and its floor table.
//
// ── WHAT THIS FILE OWNS, AND WHY IT IS THE HOST THAT OWNS IT ─────────────────
//
// The tower is not new content inside the game. It is a host-owned progression on
// top of three guest primitives that are all measured and anchored
// (`GUARDIAN_CASTLE_FEASIBILITY.md` §A/§B, `guest_anchors::SceneWorld` and
// `guest_anchors::CreatureRecord`):
//
//   1. GO TO A FLOOR   = two byte writes. `warp_dest_off` then `warp_request_off`,
//                        and the game's own per-tick handler does the rest.
//   2. SCALE A FLOOR   = writes into the live monster records.
//   3. SEE IT CLEARED  = count records whose current HP is positive.
//
// So this file is pure arithmetic and bookkeeping: it decides WHICH floor, WHAT to
// write and WHEN a floor is done, and hands those out as [`Action`]s. It reads
// nothing and writes nothing itself, exactly like `quickpage.rs` and `orbcombine.rs`
// — which is what lets `guardian_selftest.rs` compile this real source as its own
// test crate. ⛔ Do not add a `use crate::…` here; that is the alarm going off.
//
// ── THE FIVE RULES THAT COST SOMETHING TO LEARN ──────────────────────────────
//
// Each of these is a measurement, and each one is easy to get backwards:
//
//   * A FLOOR IS A SCENE, NOT A MAP. 244 scenes share 135 maps, and the guest's
//     move primitive takes a scene. A table column holding map ids would send the
//     player somewhere else entirely — [`Floor::scene`] is named for this.
//   * THE WARP BYTE IS A FACING, NOT AN ENTRY POINT. Zero hides the player and
//     breaks skill animation; use the measured downward facing [`WARP_ENTRY`].
//   * "NO MONSTER IS ALIVE" IS NOT "CLEARED" UNTIL ONE HAS BEEN SEEN ALIVE. The
//     poll right after a warp sees the old scene's array, or a freshly-loaded one
//     before the spawn loop ran, and both read as zero alive. [`State::InFloor`]
//     therefore carries `saw_alive` and refuses to clear before it is set.
//   * THE LEVEL IS ONE SIGNED BYTE. The record field is an `i8`, so 127 is the most a
//     floor can ask for. ★ 112차 (2026-09-19, user decision): the floors run **60 → 120**
//     ([`Floor::level_for`], [`LEVEL_MAX`] 120 — ★ 137차 bends it at floor 70 = Lv 100,
//     the user's hell anchor, and the generated table weighs each floor's count against
//     a target total HP instead of the level) and — because the player now levels to
//     99 (111차) and the game's kill-EXP formula pays `1 + exp` once the player is five
//     levels above the monster — the level is what makes the deep floors worth EXP at
//     all. The game reads it in four places, all clamped: the kill-EXP `lvDiff` (±9),
//     the player's miss roll `clamp(20 + 3·(mLv−pLv), 10, 60) %` (`0x38672`), the
//     monster's hit rolls (`0x4bf8e`/`0x4bfde`, capped 90/95 %), and a ±lv/2 damage
//     jitter (`0x3855a`). So a monster 14+ levels above the player is at the caps and
//     no worse — the same rule the shipped game applies to an out-levelled player.
//   * ★★★ STATS FOLLOW THE LEVEL (112차). "HP·공격력도 몬스터 레벨이 오를테니 마찬가지로":
//     a forked record is grown from its SPAWN level to the floor's level by the game's
//     own per-level growth, measured on its own tables (the same species on `enemy_dat`
//     vs `enemyh_dat`, 153 pairs): HP x1.036, attack x1.043, defence x1.0285 per level
//     ([`HP_GROWTH`], [`ATK_GROWTH`], [`DEF_GROWTH`]). The spawn level is the first
//     sighting of `+0x70` ([`Base::lv0`]), exactly like the HP base, so re-asserting is
//     idempotent. The row's `hp% def% atk%` still multiply ON TOP (100 = growth only);
//     the generated table now leaves them at 100 and lets the level carry the curve.
//   * DEFENCE AND ATTACK HAVE A BASE AND AN EFFECTIVE COPY, and combat reads the
//     EFFECTIVE one. The game's own percentage setters do `+=` on a knob, so calling
//     them twice compounds; [`Fork`] instead carries absolute values, which makes
//     re-asserting a fork idempotent.
//   * ★★★ THE BASE FIELDS ARE ZERO ON A BOSS (88차, 2026-09-12). An ordinary record
//     has `hp_spawn == hp_max`, `def_*_base == def_*_eff`, `atk_base == atk_eff` at
//     spawn — a `bossh_dat` record does NOT: 홀리가디언 spawns with `hp_spawn 0`,
//     `def bases 0/0` under effective `0x8e/0x8e`, `atk_base 0` under effective
//     `0x441`; 멜페토 with `hp_spawn 897` under `hp_max 16050`. The 87차 fork scaled
//     FROM those base fields, so every boss floor turned its boss into a 1-HP,
//     half-defence monster — the player's "보스 HP가 너무 적고 공격력도 너무 낮아".
//     The fork now scales from a SNAPSHOT of the effective fields taken the first
//     time a record is seen on a floor ([`FloorMemory`]), which is the game's own
//     spawn value for bosses and ordinaries alike. ⛔ Never scale from `+0x82`,
//     `+0x68`, `+0x6a` or `+0x8c` again.
//   * HP IS ONE SIGNED HALFWORD. `hp_cur`/`hp_max` are `i16`, the liveness test is
//     `> 0`, and 32767 is the most a record can hold — a boss at 28k HP x 400 % is
//     113k, which used to clamp to `u16::MAX` = -1 = DEAD. [`I16_MAX`] is the ceiling
//     the record gets, and the rest is a VIRTUAL total the host SEEDS ONCE into the
//     record's spare `+0x100` word ([`Fork::vhp`], the total beside it in `+0x104`);
//     from then on the guest-side `dmg_apply_lgt` hook
//     (`wie_core_arm::binary_patches::dmgwide`) takes every hit off that word and keeps
//     `hp_cur = 32767 * remaining / total` — proportional (102차-B), never 0 while
//     anything remains, so the target box's bar drains like any other monster's — and
//     the record dies through the game's own path once the word is spent. (2026-09-18;
//     until then the host refilled the record from a poll-side pool, which a 32767 hit
//     could one-shot because the game killed the record before the next poll.)
//
// ── THE ORIGINAL-PRESERVATION INVARIANT ──────────────────────────────────────
//
// [`State::Idle`] produces no [`Action`] at all. Outside a run the guest is byte for
// byte the shipped game, the same invariant the buff table keeps.

/// How many floors the tower has. The spec asks for 100.
pub const FLOORS: u8 = 100;

/// Downward facing. LGT `0x3f2da -> 0x56134 -> 0x5609c` copies this byte into
/// the player's facing; the sprite renderer indexes it minus one (`0x4a74a`).
/// Zero made arrivals invisible. Measured with no direction input: 2 renders immediately.
pub const WARP_ENTRY: u8 = 2;

/// The ceiling this file clamps a floor's MONSTER level fork to: the top of the 60 → 120
/// curve (112차). The record field is an `i8`, so this can never exceed 127; and the
/// game's own level-difference rolls are all clamped (see the header), so a level past
/// the player's 99 costs the player the caps and nothing worse.
pub const LEVEL_MAX: i8 = 120;

/// The game's own per-level stat growth — the ratio a species' stat grows by per level
/// between its `enemy_dat` row and its `enemyh_dat` row (153 species with a level gap,
/// medians; 2026-09-19). A forked record is grown by `growth^(floor_lv - spawn_lv)`.
pub const HP_GROWTH: f64 = 1.036;
pub const ATK_GROWTH: f64 = 1.043;
pub const DEF_GROWTH: f64 = 1.0285;

/// The most EXP one floor may pay on clear (a table typo guard; a level-up at 99 is
/// ~1.02 M and `checkLevelUp` climbs at most 15 levels per call).
pub const EXP_MAX: u32 = 10_000_000;

/// `value * growth^delta`, rounded, never below 1 and never above `u32` — the raw
/// grown stat before any percentage. `delta` may be negative (a species forked BELOW
/// its own level shrinks the same way it grows).
pub fn grow(value: u16, growth: f64, delta: i32) -> u32 {
    let g = (f64::from(value) * growth.powi(delta)).round();
    if g < 1.0 { 1 } else if g > f64::from(u32::MAX) { u32::MAX } else { g as u32 }
}

/// Percentages are `100 = leave it exactly as the game made it`.
pub const PCT_NEUTRAL: u16 = 100;

/// The most a floor may multiply a stat by, as a percentage. A table typo of an
/// extra zero would otherwise ask for a silly number; the record itself is protected
/// by [`I16_MAX`] regardless (HP beyond it goes to the virtual word, the rest is clamped).
pub const PCT_MAX: u16 = 2000;

/// The most a record's `i16` stat field can hold. HP, attack and both defences are
/// all signed halfwords in the record and the game reads them signed, so anything
/// above this reads as NEGATIVE — for HP that is "dead".
pub const I16_MAX: u16 = 32767;

/// The most monsters a floor may carry (user 2026-09-16: "1~20명"). The maps ship with
/// this many spawn points; the floor's `count` column keeps the first N of them — since
/// 145차 by never creating the rest ([`Floor::spawn_cap`], the `tower_spawn_cap_lgt` hook
/// on the game's spawn loop).
///
/// ⛔ Not by zeroing the extras' HP after the load. That was the 2026-09-16 design
/// (`park_indices`, deleted): the game ran its own death for every one of them, and the
/// player saw monsters drop dead on arrival without a hit (user 2026-09-30: *"입장하자마자
/// 내가 때리지도 않았는데 죽는 버그 … 자동으로 몬스터가 죽는걸 원하지 않아"*).
pub const MAX_MONSTERS: u8 = 20;

/// How many polls [`State::Entering`] waits for the scene word to change before it
/// gives up. The warp handler consumes the request on its very next tick, so this is
/// generous by two orders of magnitude on purpose: the cost of waiting is nothing and
/// the cost of a false abort is dropping the player mid-tower.
pub const ENTER_POLLS: u32 = 120;

/// How many polls the canSave restore ([`restore_can_save`]) keeps re-asserting before
/// it gives up and says so. Generous because the leave warp takes a few frames and the
/// AREA chain reads `None` right through a scene load; bounded because a restore that
/// can never land must not spin for the rest of the session.
pub const RESTORE_POLLS: u32 = 60;

/// What to do with the game's own AREA canSave byte now that a run is over.
///
/// ⛔⛔ RESTORING IS REQUIRED, AND §D.4's "no cleanup is needed — the map loader
/// rewrites this byte on every scene load, and leaving the tower IS a scene load" IS
/// REFUTED. Device-measured on S9 with vc279 (2026-09-08): the zero the tower wrote
/// survived FOUR scene loads (town 111 -> floor 1 scene 134 -> floor 30 scene 209 ->
/// floor 4 scene 221 -> `leave` back to 111) and `area gate: can_save` stayed
/// `Some(false)` for a hundred seconds in the very town that had answered `Some(true)`
/// before the run. A SAVE tap there logged `save: refused — the game's canSave byte
/// says no here`, i.e. **one tower run broke saving for the rest of the session**.
///
/// `saved` is the byte as it read before the tower first zeroed it — captured in the
/// scene the player entered from, which is the scene [`Action::Leave`] puts them back
/// in, so writing it back restores that scene's own value rather than a guess.
/// ⚠ The one case where it IS a guess is [`Abort::LeftTheField`]: the player walked out
/// through the game's own warp tile into some third scene. Restoring an "allowed" byte
/// there is still the right trade — the alternative is a session that cannot save.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Restore {
    /// Saving is allowed again; stop.
    Done,
    /// The AREA chain could not be read. Ask again next poll — a scene load reads
    /// `None` for a few frames and that is not a failure.
    Wait,
    /// Write this byte to the AREA canSave field.
    Write(u8),
}

/// Decide [`Restore`] from what was saved and what the byte reads now.
///
/// ★ `Done` is "the byte is non-zero", NOT "the byte equals `saved`": if the game has
/// meanwhile written its own value there, that value is the game's business and the
/// tower has nothing left to give back.
pub fn restore_can_save(saved: u8, live: Option<u8>) -> Restore {
    match live {
        None => Restore::Wait,
        Some(0) => Restore::Write(saved),
        Some(_) => Restore::Done,
    }
}

/// How many polls after a warp the landing check keeps looking.
///
/// The warp is a REQUEST the game services on its next tick, and the map behind it is
/// loaded asynchronously, so the first poll or two legitimately read the OLD map — a
/// check that ran once would either fire on the wrong map or not at all. Generous for
/// the same reason [`RESTORE_POLLS`] is: at 30 frames a poll this is a couple of
/// seconds, and every poll on which the player is already standing legally costs one
/// comparison and no writes.
pub const LANDING_POLLS: u32 = 20;

/// How many FRAMES a pending arrival placement keeps looking for its scene.
///
/// ★ The same budget as [`LANDING_POLLS`] (a poll is 30 frames), but spent a frame at
/// a time. The placement has to land on the very first frame the destination scene is
/// readable, because until it does the player is standing — and, worse, being DRAWN —
/// at the previous map's coordinates; checking once every 30 frames left up to half a
/// second in which a direction key walked an off-map character around.
pub const LANDING_FRAMES: u32 = LANDING_POLLS * 30;

/// The player's DRAW offsets for a position — the game's own `0x1c234`, mirrored.
///
/// The object keeps position (`+0x10`/`+0x12`) and draw corner (`+0x16`/`+0x18`)
/// separately, so host placement updates both:
///
/// ```text
///   0x1c234(obj, x, y):   [obj+0x16] = x - ((i16)[obj+0x1c] >> 1)
///                         [obj+0x18] = y - (kind in {4,6,9} ? [obj+0x20]
///                                                           : (i16)[obj+0x20] >> 1)
/// ```
///
/// Walking and warp placement call this formula. It does not repair a zero facing:
/// that independently hides the sprite and is prevented by [`WARP_ENTRY`].
///
/// `w`/`h` are the object's `+0x1c`/`+0x20` (the placement leaves them at 0x10 / 8) and
/// `kind` is `+0x4`. Both branches agree for the small positive sizes the player uses;
/// the split is kept so the mirror stays honest against the disassembly.
pub fn draw_offsets(px: i16, py: i16, w: i16, h: i16, kind: i8) -> (i16, i16) {
    let dx = px.wrapping_sub(w >> 1);
    let dy = py.wrapping_sub(if matches!(kind, 4 | 6 | 9) { h } else { h >> 1 });
    (dx, dy)
}

/// Tile-attribute bit 7 — the game's own "you may not stand here". The same bit the
/// minimap paints grey with and the field's own walk check reads (`0x17a6f8`).
pub const TILE_BLOCKED: u8 = 0x80;

/// How far from the map centre [`landing_tile`] will look for somewhere to stand.
/// A tower floor is a 20x20 arena, so this covers every tile of one twice over; the
/// bound exists so a garbage attribute array cannot turn into a long loop.
pub const LANDING_SEARCH_MAX: i32 = 64;

/// How many tiles a player must be able to WALK to before [`landing_tile`] calls their
/// position good enough to leave alone.
///
/// Eight is chosen from the measurement, not from taste: on the map that produced the
/// 2026-09-12 report the wedge reaches 1 tile and the floor reaches 668, and the
/// tightest landing spot across all 94 floor scenes in the shipped table reaches 287.
/// ⛔ Do not raise it towards a real room size — the arenas are hundreds of tiles, but
/// a player standing in a doorway during a scene load must never be teleported.
pub const LANDING_MIN_ROOM: usize = 8;

/// WHERE THE PLAYER SHOULD BE STANDING after a tower warp, or `None` when they are
/// already somewhere legal.
///
/// ── WHY THIS EXISTS (2026-09-11, reported as "I entered the tower and I vanished")
///
/// The warp primitive is TWO BYTES: destination scene and request. The game's own
/// warps are a script opcode, `0x28 <scene> <x> <y> <dir>`, and the difference is the
/// two coordinates. Nothing in the handler (`0x3f1d4`) supplies them for us — it hands
/// the placement routine the entry-point byte and the tile size, and `0x56134` reads
/// the player's CURRENT position out of the player object — so a host-posted warp
/// lands the player on **the tile they were standing on in the previous map**.
///
/// That was survivable while floors were big maps. It stopped being survivable when
/// the floor table became "closed arenas only" (2026-09-11): every floor scene is
/// **20x20**, and the tower's entrance passer-by stands at tile **(32,19)** of a 35x36
/// town, so the player arrives outside the map. Device measurement (S25, vc289):
///
/// ```text
///     player at 0x4a85ad84: px=(480,320) tile=(30,20) map=20x20
/// ```
///
/// Off the map, the character is not drawn, the camera clamps and stays there, the
/// monsters have nothing to walk towards and no direction is walkable — which from the
/// outside is exactly "my character disappeared".
///
/// ⛔ REFUSE TO GUESS. `None` when the map size is not a plausible grid or the
/// attribute array is not the right length: a wrong landing tile teleports a player
/// who was standing somewhere perfectly good.
///
/// ★ The search starts at the CENTRE rather than near the old position. These are
/// arenas built around a middle, the monsters spawn in them, and a floor whose player
/// materialises in a corner behind a rock is a floor that reads as broken even when it
/// is not. It walks outwards in rings so the first hit is the closest legal tile to
/// the middle.
pub fn landing_tile(map_w: i32, map_h: i32, attr: &[u8], cur: (i32, i32)) -> Option<(u8, u8)> {
    // Same ceiling `minimap_render` uses: past this the pointer chain landed on
    // something that is not a map.
    if !(1..=200).contains(&map_w) || !(1..=200).contains(&map_h) {
        return None;
    }
    if attr.len() != (map_w * map_h) as usize {
        return None;
    }
    let blocked = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= map_w || y >= map_h {
            return true;
        }
        attr[(y * map_w + x) as usize] & TILE_BLOCKED != 0
    };
    // ⛔ There is deliberately NO "the tile is open, so we are done" fast path here.
    // An open tile can be an isolated pocket — `standing_inside_the_pocket_is_also_a_wedge`
    // is that case — and the walkability test below answers both shapes with one rule.
    // The cost of dropping the fast path is one flood fill capped at eight tiles, and
    // only during the [`LANDING_POLLS`] window after a warp.
    // ⛔ BLOCKED IS NOT ENOUGH, AND NEITHER IS "HAS AN OPEN NEIGHBOUR" — WALKABLE IS.
    //
    // A tile the attribute array calls blocked can still be one the player legitimately
    // walked onto, and teleporting them off it is a worse bug than the one this fixes
    // (device-measured 2026-09-11: someone standing quite happily at (21,16) got
    // moved). So the first version of this asked "is any of the four neighbours open?".
    //
    // ⛔⛔ That was still wrong, and it shipped in vc293..vc295. Device report
    // 2026-09-12, S25, floor 2 = scene 9 = map 7 `NEMESIS_FOREST_2` 45x35: the player
    // carried the town's tile (31,20) in, which on that map is WALL, and its only open
    // neighbour (32,20) is a ONE-TILE POCKET. One open neighbour, nowhere to go:
    //
    //        cols 27..36            (X = blocked)
    //   y19  X X X X X X . X X X
    //   y20  X X . X X . X X X X    <- player inside col 31, only col 32 open
    //   y21  X X X X X X X X X X
    //
    // The honest question is not "can I take one step" but "can I walk anywhere", so
    // the test is now a flood fill bounded by [`LANDING_MIN_ROOM`]. Measured on that
    // very map: the wedge reaches **1** tile and the real floor reaches **668**, and
    // across all 94 floor scenes in the shipped table the worst destination reaches
    // 287 — so the threshold is nowhere near either side of the decision.
    //
    // This also SUBSUMES the old first test: a player on an open tile that happens to
    // be an isolated pocket is just as stuck, and the neighbour rule let that through.
    let in_bounds = (0..map_w).contains(&cur.0) && (0..map_h).contains(&cur.1);
    if in_bounds && room_at_least(map_w, map_h, &blocked, cur, LANDING_MIN_ROOM) >= LANDING_MIN_ROOM {
        return None;
    }
    spawn_tile(map_w, map_h, attr)
}

/// WHERE A TOWER FLOOR PUTS THE PLAYER — the centre-out ring search on its own, with no
/// "they are already fine" early-out.
///
/// ★★★ 2026-09-21 — THE ARRIVAL TILE IS CHOSEN, NOT INHERITED. Player: *"10배수
/// 보스방에서 시작위치가 랜덤인지 아닌지 모르겠는데 이동후에 못움직일때가
/// 있어"*. It reads as random because it IS: the two-byte warp carries no
/// coordinates, so every floor started the player on whatever tile they happened to be
/// standing on in the floor before ([`landing_tile`] has the disassembly). The halls are
/// 22x27 and the throne room 26x31, so a hall tile is always in bounds for the boss
/// room and always passes [`LANDING_MIN_ROOM`] — which is why the correction below
/// never fired there and the arrival stayed wherever the last floor left it.
///
/// ⛔ Do NOT reach for this outside a tower warp. [`landing_tile`]'s early-out exists
/// so that a player standing in a doorway during an ordinary scene load is left alone;
/// this function teleports unconditionally and is only correct where the host is the
/// one that decided to move them.
pub fn arrival_tile(map_w: i32, map_h: i32, attr: &[u8]) -> Option<(u8, u8)> {
    if !(1..=200).contains(&map_w) || !(1..=200).contains(&map_h) {
        return None;
    }
    if attr.len() != (map_w * map_h) as usize {
        return None;
    }
    spawn_tile(map_w, map_h, attr)
}

/// The shared half of [`landing_tile`] and [`arrival_tile`]: the closest tile to the
/// middle of the map that the player can actually walk around in.
///
/// ⛔ The destination has to pass the same room test the caller applies, or a map
/// whose centre sits in a pocket would swap one wedge for another. Prefer a real room;
/// only if no tile in range has one fall back to "any open tile", which is what the old
/// code did and is still better than leaving the player off the map.
fn spawn_tile(map_w: i32, map_h: i32, attr: &[u8]) -> Option<(u8, u8)> {
    let blocked = |x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= map_w || y >= map_h {
            return true;
        }
        attr[(y * map_w + x) as usize] & TILE_BLOCKED != 0
    };
    let (cx, cy) = (map_w / 2, map_h / 2);
    let mut any_open = None;
    for r in 0..LANDING_SEARCH_MAX {
        for dy in -r..=r {
            for dx in -r..=r {
                // Ring only: the inside of the square was covered by a smaller `r`.
                if r > 0 && dx.abs() != r && dy.abs() != r {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if blocked(x, y) {
                    continue;
                }
                if room_at_least(map_w, map_h, &blocked, (x, y), LANDING_MIN_ROOM) >= LANDING_MIN_ROOM {
                    return Some((x as u8, y as u8));
                }
                if any_open.is_none() {
                    any_open = Some((x as u8, y as u8));
                }
            }
        }
    }
    any_open
}

/// How many open tiles are reachable from `start` by walking (4-way), counting at most
/// `cap` of them.
///
/// `start` itself is not counted and is treated as passable for the first step — the
/// whole point is that the player may be standing ON a blocked tile, which is the case
/// [`landing_tile`] exists for. The cap is what keeps this cheap: the steady state stops
/// after eight tiles, and the function only runs during the [`LANDING_POLLS`] window
/// after a warp.
fn room_at_least<F: Fn(i32, i32) -> bool>(
    map_w: i32,
    map_h: i32,
    blocked: &F,
    start: (i32, i32),
    cap: usize,
) -> usize {
    let mut seen = vec![false; (map_w * map_h) as usize];
    let mut queue = std::collections::VecDeque::from([start]);
    let mut n = 0usize;
    while let Some((x, y)) = queue.pop_front() {
        if n >= cap {
            break;
        }
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= map_w || ny >= map_h || blocked(nx, ny) {
                continue;
            }
            let i = (ny * map_w + nx) as usize;
            if seen[i] {
                continue;
            }
            seen[i] = true;
            n += 1;
            queue.push_back((nx, ny));
        }
    }
    n
}

/// May this character enter the tower at all?
///
/// ── THERE IS NO "ENDING SEEN" FLAG, AND THAT IS THE ANSWER ───────────────────
///
/// The spec gates the tower behind the ending. The game has no such flag, because it
/// does not need one: finishing the game IS the 2회차 transition, and that transition
/// leaves a persistent mark. The chain, measured end to end (2026-09-08):
///
///   * scene 241 is the ENDING STAFF ROLL (`wie_probe --host-warp 1700:241` renders
///     "GAME DESIGNER" and the credits scroll).
///   * when it finishes, the only site in the image that opens window **18** does so
///     gated on `*(u8*)(base + 0x9e15) == 241` — the warp destination byte, i.e.
///     "the place we are in is the ending" (`0x3e840`..`0x3e87c`).
///   * window 18's KEY handler (`0x3b420`, from the window KEY table `0x5d790`) is
///     the ONLY caller of the 2회차 transition `0x3af2c`, which wipes the bag, writes
///     **1** to the difficulty byte (`0x3b0bc`) and warps to scene 83
///     `NEMESIS_PLATEAU_1`.
///   * the difficulty byte round-trips through the save (`serialiseSave 0x1aed4`,
///     `deserialiseSave 0x1a9d4`, 78차-C), and its only writers are that transition,
///     the character reset (`0x3ef80`, to 0) and the save loader — so it cannot be 1
///     on a character that has not been through the ending.
///
/// Runtime proof: the difficulty byte at `0x150ddc8` went 0 -> 1 between frames 4000
/// and 6000 of a probe run that warped to 241 and pressed OK through the credits, and
/// the next frame is the hard-mode opening on NEMESIS_PLATEAU_1.
///
/// ⇒ the gate is `difficulty != 0`, one byte the app already reads for the reaver
/// drop. ⛔ Do not go looking for a story-progress field; this is it.
///
/// FAIL CLOSED: `None` (the byte could not be read) refuses. A refusal costs the
/// player a locked door they can ask about; a wrong "yes" drops a mid-game character
/// into level-66 monsters. ★ And say so in a log when it happens — the same rule
/// 78차-C wrote down after a silent refusal survived eight sessions.
pub fn entry_allowed(difficulty: Option<u8>) -> bool {
    if !HARD_ONLY {
        // ⛔ STILL FAIL CLOSED. `None` is "the difficulty byte could not be read", which
        // means the CHAR chain did not resolve — the tower must not start on a guest it
        // cannot read, whatever the difficulty rule is. Only the `!= 0` half is relaxed.
        return difficulty.is_some();
    }
    matches!(difficulty, Some(d) if d != 0)
}

/// Is the tower hard-mode only?
///
/// ★★ **FALSE FOR NOW — normal-mode characters may enter** (user decision 2026-09-08:
/// *"노말에서도 테스트할 수 있게 해줘 이후에 하드코어에서만 하늘성에 갈 수있게 할꺼야"*).
/// The tower has no in-game entrance yet (§3.5), so every entry goes through the
/// `wie_guardian.txt` control file, and requiring `force 1` on every one of them is
/// friction with no safety in it: `force` is in the same file, so it never protected
/// anything from anyone.
///
/// ⛔ **THE SHIPPING RULE IS STILL HARD-ONLY.** Flipping this one word back to `true`
/// restores it, and that is deliberately the whole change — the rule itself, the log
/// line and the tests all read this constant, so there is no second place to remember.
/// Flip it when the in-game entrance ships.
///
/// ⛔ Do not express "open on normal" by deleting [`entry_allowed`]'s call site or by
/// making the control file always force: the `None` half of that function is a real
/// gate (see above) and 78차-C is the record of what a silently-skipped gate costs.
pub const HARD_ONLY: bool = false;

/// One floor of the tower.
///
/// `scene` is a SCENE id (see the file header). The three multipliers are
/// percentages of what the game itself put in the record, so a row of
/// `100 100` with `level: None` is a floor that runs the shipped monsters untouched.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Floor {
    /// 1..=[`FLOORS`].
    pub floor: u8,
    /// The scene to warp to. One byte, because the warp destination is one byte.
    pub scene: u8,
    /// `Some(lv)` forks every monster's level; `None` leaves it alone.
    pub level: Option<i8>,
    /// Percentage applied to both max and current HP.
    pub hp_pct: u16,
    /// Percentage applied to both effective defences.
    pub def_pct: u16,
    /// A boss floor. On one, the row's three multipliers apply to the BOSS record
    /// (species >= [`BOSS_BASE`]) and the adds get the LEVEL of the floor below with
    /// neutral multipliers ([`forks`], 138차 결정 ⑬), so "boss HP x12" does not also make
    /// its escorts x12, and the 71..99 hp% ramp does not reach floor 100's escorts.
    pub boss: bool,
    /// Percentage applied to the effective attack. 100 = the game's own.
    pub atk_pct: u16,
    /// The species triple the host writes over the scene header's as the floor
    /// loads (`wie_util::stats::TOWER_SPECIES`, applied by the `tower_species_lgt`
    /// hook). `SPECIES_NONE` in a slot spawns nothing for that spawn type — which is
    /// how a row also chooses the COUNT (a hall is 4/8/12, a swarm hall 18/36/54).
    /// `None` = a legacy row: the scene's own header decides.
    pub species: Option<[u8; 3]>,
    /// How many monsters this floor spawns (user 2026-09-16). The map has up to
    /// [`MAX_MONSTERS`] spawn points; the `tower_spawn_cap_lgt` hook lets the game's spawn
    /// loop create the first `count` and sends the rest down its own empty-slot path
    /// ([`Floor::spawn_cap`], 145차). `None` = a legacy row with no count = all of them.
    /// Never set on a boss floor.
    pub count: Option<u8>,
    /// EXP the floor pays the player on clear (112차, "새 컨텐츠가 exp 직접 주게"). The
    /// host adds it to `CHAR+0xe4` and calls the game's own `checkLevelUp`, the 천상의
    /// 영약 mechanism. 0 = a legacy row = nothing. Bounded by [`EXP_MAX`].
    pub exp: u32,
}

/// "Spawn nothing for this type" in a species slot — the scene header's own value.
pub const SPECIES_NONE: u8 = 0xff;
/// Species at or above this index `res/boss/bossh_dat` (`0x3865a` tests `> 160`).
pub const BOSS_BASE: u8 = 161;

impl Floor {
    /// The level curve — 137차 (user decisions 2026-09-25): floor 1 = 60, **floor 70 =
    /// 100** (the user's anchor: a floor-70 monster is a hell Lv 100 monster, same
    /// species growth, same stats), floor 100 = 120 (홀리가디언 kept at Lv 120), two
    /// linear pieces, round half up, monotone. Kept here rather than only in the
    /// generator script so the clamp ([`LEVEL_MAX`]) and the curve cannot drift apart —
    /// `tools_re/gen_guardian_floors.py::level_for` is the same integer formula.
    pub fn level_for(floor: u8) -> i8 {
        let f = floor.clamp(1, FLOORS) as i32;
        let lv = if f <= 70 {
            60 + (2 * 40 * (f - 1) + 69) / 138
        } else {
            100 + (2 * 20 * (f - 70) + 30) / 60
        };
        lv.min(LEVEL_MAX as i32) as i8
    }

    /// Is this row usable? A table is data the player can edit, so every row is
    /// checked rather than trusted.
    pub fn valid(&self) -> bool {
        (1..=FLOORS).contains(&self.floor)
            && self.scene != 0
            && (PCT_NEUTRAL..=PCT_MAX).contains(&self.hp_pct)
            && (PCT_NEUTRAL..=PCT_MAX).contains(&self.def_pct)
            && (PCT_NEUTRAL..=PCT_MAX).contains(&self.atk_pct)
            && self.level.is_none_or(|lv| (1..=LEVEL_MAX).contains(&lv))
            // A row that overrides the species must leave at least one slot live, or the
            // floor spawns nothing and can never be seen alive, let alone cleared.
            && self.species.is_none_or(|sp| sp.iter().any(|s| *s != SPECIES_NONE))
            // A count, when present, is in 1..=MAX_MONSTERS and never on a boss floor.
            && self.count.is_none_or(|n| (1..=MAX_MONSTERS).contains(&n) && !self.boss)
            && self.exp <= EXP_MAX
    }

    /// The scene's species override as the hook wants it, if the row has one.
    pub fn species_override(&self) -> Option<(u8, [u8; 3])> {
        self.species.map(|sp| (self.scene, sp))
    }

    /// ★ 145차: `(scene, count)` for `wie_util::stats::set_tower_spawn_cap` — armed next to
    /// [`Self::species_override`] before the warp, so the floor spawns exactly `count`
    /// monsters and none has to be removed afterwards. `None` (a boss floor, a legacy row)
    /// = the scene spawns every point its species slots fill.
    pub fn spawn_cap(&self) -> Option<(u8, u8)> {
        self.count.map(|n| (self.scene, n))
    }
}

/// Apply a percentage to a stat the way a fork must: saturating at [`I16_MAX`], and
/// never below 1.
///
/// Never below 1 because 0 max HP would make the HP bar divide by zero
/// (`43 * cur / max` in the target-box painter) and 0 current HP would read as a dead
/// monster to this very file's clear test. Never above [`I16_MAX`] because the fields
/// are signed halfwords — the old `u16::MAX` ceiling put -1 into a big boss's HP.
pub fn scale(value: u16, pct: u16) -> u16 {
    scale_u32(value, pct).clamp(1, I16_MAX as u32) as u16
}

/// The unclamped product, for the virtual HP word: how much HP the floor really asks for.
pub fn scale_u32(value: u16, pct: u16) -> u32 {
    (value as u32 * pct.max(1) as u32) / 100
}

/// One monster's worth of writes. Absolute values, so re-asserting is idempotent.
///
/// The caller turns these into memory writes at
/// `array + index * SceneWorld::creature_stride + CreatureRecord::<field>`.
///
/// ⛔ Nothing here writes a BASE field (`+0x82`, `+0x68`, `+0x6a`, `+0x8c`). They are
/// not the fork's inputs any more either (see the header — they are zero on a boss);
/// the inputs are the [`FloorMemory`] snapshot, and the bases are simply left alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fork {
    /// Index into the live creature array.
    pub index: i32,
    /// `Some` only when the floor asks for a level.
    pub level: Option<i8>,
    pub hp_max: u16,
    pub hp_cur: u16,
    pub atk_eff: u16,
    /// ★ 160차 T4 — a BOSS's second and third attack values (`+0x90` / `+0x92`), grown the
    /// same way as `atk_eff`. `None` on anything that is not a known boss (species >= 161):
    /// those two halfwords are not attacks on an ordinary record and are left alone.
    pub atk_skill: Option<[u16; 2]>,
    pub def_phys_eff: u16,
    pub def_spec_eff: u16,
    /// `Some(total)` = seed the record's virtual HP word with the floor's total. Only
    /// while the word reads 0 — the hook owns it once seeded, and a re-assert would
    /// hand the player's hits back.
    pub vhp: Option<u32>,
    /// ★ 156차 (hell) · 158차 P8 (the tower too) — the record's combat-roll fields: 명중
    /// (`+0x78`, `i16`), 회피 (`+0x7a`) and 기절 저항 (`+0x96`), as [`hell_hit`] /
    /// [`hell_dodge`] / [`hell_res`] give them for this monster's level against the player's.
    /// `None` = leave the game's own value alone (a record whose level is unknown). The
    /// caller compares each against the live halfword and writes only a difference.
    pub hit: Option<i16>,
    pub dodge: Option<u16>,
    pub res: Option<u16>,
}

/// What one monster looks like to this file. Filled in by the caller from the record.
///
/// Every stat here is the LIVE effective field. The first time a floor sees a record
/// these are the game's own spawn values and [`FloorMemory`] keeps them; after that
/// they are whatever the last fork (or the game) left, and only `hp_cur` is read for
/// its value (to carry a wound and to feed the pool).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Creature {
    pub index: i32,
    /// `hp_max` (`+0x7c`) — live. Its FIRST reading on a floor is the HP base.
    pub hp_max: u16,
    /// `hp_cur` (`+0x7e`) — live, signed, clamped at 0 by the caller.
    pub hp_cur: u16,
    /// `respawn_timer` (`+0x74`) — live. [`RESPAWN_DISARMED`] when nothing is pending.
    pub respawn_timer: u16,
    /// `atk_eff` (`+0x8e`) — live. First reading = the attack base.
    pub atk_eff: u16,
    /// `+0x90` / `+0x92` — live. A boss's skill attacks (first reading = their base); on an
    /// ordinary record whatever those halfwords are, read but never written (160차 T4).
    pub atk_skill: [u16; 2],
    /// `def_phys_eff` / `def_spec_eff` (`+0x6c` / `+0x6e`) — live. First reading = base.
    pub def_phys_eff: u16,
    pub def_spec_eff: u16,
    /// The species, through `*(*(rec+0)+0x1c)+0x0e` (the chain the reaver hook walks).
    /// `None` when the chain would not read; a boss floor then treats the record as
    /// the boss (the stronger of the two forks — fail towards "hardcore").
    pub species: Option<u8>,
    /// The virtual HP word (`+0x100`) — live. 0 = unseeded; anything else is the
    /// hook's and this file only looks at it to know NOT to seed again. The caller
    /// reads an unreadable word as `u32::MAX` for the same reason.
    pub vhp: u32,
    /// The level byte (`+0x70`, `i8`) — live. Its FIRST reading on a floor is the
    /// SPAWN level the growth is measured from ([`Base::lv0`]); `None` = unreadable,
    /// and then the record gets the row's percentages with no level growth.
    pub level: Option<i8>,
}

/// The per-floor bookkeeping the fork needs: one entry per record index, made the
/// first time the record is seen on the floor and dropped when the floor is left.
///
/// ⛔ Keyed on the array INDEX, which is only meaningful for one loaded scene — the
/// array is reallocated on every scene load (`Action::Fork`'s note), so [`Run::enter`]
/// and [`Run::finish`] both clear it. A respawn into an existing index (which the
/// tower cancels anyway) keeps its snapshot: same species, same spawn values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Base {
    pub index: i32,
    /// The game's own spawn values — what a 100 % floor would leave alone.
    pub hp0: u16,
    pub atk0: u16,
    /// The first sighting of `+0x90` / `+0x92` (160차 T4 — only a boss's are ever used).
    pub atk_skill0: [u16; 2],
    pub dp0: u16,
    pub ds0: u16,
    /// The floor's total HP for this record when it does not fit the `i16` field —
    /// what gets seeded into the virtual HP word. `None` = the record holds it all.
    pub vhp_total: Option<u32>,
    /// The level the record SPAWNED with (the species' own `dat` level), from which
    /// the growth to the floor's level is measured. `None` = no growth for this record.
    pub lv0: Option<i8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FloorMemory {
    pub bases: Vec<Base>,
}

impl FloorMemory {
    pub fn clear(&mut self) {
        self.bases.clear();
    }

    /// The snapshot for `index`, taking it now if this is the first sighting.
    fn base_for(&mut self, c: &Creature, hp_total: u32) -> Base {
        if let Some(b) = self.bases.iter().find(|b| b.index == c.index) {
            return *b;
        }
        let cap = hp_total.clamp(1, I16_MAX as u32);
        let b = Base {
            index: c.index,
            hp0: c.hp_max,
            atk0: c.atk_eff,
            atk_skill0: c.atk_skill,
            dp0: c.def_phys_eff,
            ds0: c.def_spec_eff,
            vhp_total: (hp_total > cap).then_some(hp_total),
            lv0: c.level,
        };
        self.bases.push(b);
        b
    }
}

/// The value the game leaves in [`Creature::respawn_timer`] when no respawn is
/// pending, and therefore the value that cancels one.
///
/// ★★★ **A TOWER FLOOR IS NOT CLEARABLE WITHOUT THIS** (user requirement 2026-09-11:
/// *"몹 리젠과 아이템도 나오면 안되고"*). The game re-spawns a dead monster into its own
/// record after roughly 300 ticks, so the live count climbs back up and [`Run`] waits
/// for a clear that never arrives — which is exactly what the device log showed on
/// vc286 (live 19 → 13 → 14 → 15 → 16 on one floor).
/// See [`crate::guest_anchors::CreatureRecord::respawn_timer`] for the measurement.
pub const RESPAWN_DISARMED: u16 = 0xFFFF;

/// Indices whose respawn is pending, i.e. the records to write [`RESPAWN_DISARMED`] to.
///
/// ⛔ Read-compare-write: a settled floor returns an EMPTY vec, so this costs nothing
/// while the player is fighting and only speaks up for corpses.
/// ★★ CORPSES ONLY (88차 — reversing the earlier "do not restrict to dead creatures").
/// The measurement that rule rested on was ordinary records, where a LIVE monster
/// always reads `0xffff` here. A `bossh_dat` record does not: 홀리가디언 spawns with
/// `+0x74 == 0` and stays there while alive (headless, scene 246, frames 1720..1890),
/// so an unconditional disarm would write `0xffff` into a live boss's word whose
/// meaning on a boss is unmeasured. A respawn is only ever a corpse's concern.
pub fn respawn_disarms(creatures: &[Creature]) -> Vec<i32> {
    creatures
        .iter()
        .filter(|c| !c.alive() && c.respawn_timer != RESPAWN_DISARMED)
        .map(|c| c.index)
        .collect()
}

impl Creature {
    /// The liveness rule, and the only one there is: there is no dead flag, the HP
    /// writer clamps a negative result to 0, and both the AI tick and the combat
    /// targeting branch on `<= 0`.
    pub fn alive(&self) -> bool {
        self.hp_cur > 0
    }
}

/// Where a dead boss's map object is parked so it stops being drawn — far outside any
/// map (tiles are 16 px and the biggest map is 50 wide), as a signed halfword pair.
pub const CORPSE_HIDE_POS: i16 = -4096;

/// Dead BOSS records whose sprite has to be taken off the screen — the "보스 잔상".
///
/// ★★★ MEASURED (88차, headless, scene 246): an ordinary record is skipped by the draw
/// loop the moment its HP hits 0 (54 boxes killed through the game's own `0x38620`
/// path all vanished within 100 frames, respawn disarmed or not). A `bossh_dat`
/// record is NOT — 홀리가디언 at HP 0 was still drawn 1300 frames later, unchanged
/// (`+0x7e` was the only record byte the death touched). In the shipped game a boss
/// fight is ended by the scene's script (a cutscene and a warp), and the tower seals
/// those scripts, so nothing ever removes the corpse. Writing [`CORPSE_HIDE_POS`] into
/// the map object's position pair (`*(*(rec+0)+0x1c)+0x10/+0x12`, the same object the
/// species is read from) hides it, and the run carried on for 600 frames without a
/// tick error. Bosses only: the ordinary corpse handles itself, and the write is
/// read-compare-write so it lands once.
pub fn corpse_hides(creatures: &[Creature]) -> Vec<i32> {
    creatures
        .iter()
        .filter(|c| !c.alive() && c.species.is_some_and(|s| s >= BOSS_BASE))
        .map(|c| c.index)
        .collect()
}

/// ★ 145차: did this floor spawn MORE records than its `count`? `Some(records)` when it
/// did — the one sign that the `tower_spawn_cap_lgt` hook did not apply (not installed,
/// its frame check failed, the knob was not armed for this scene) — `None` otherwise.
///
/// Diagnostic only, and deliberately so. The extras are NOT removed any more: until 145차
/// [`MAX_MONSTERS`]' surplus was "parked" by writing 0 HP, the game ran its own death for
/// each one, and the player watched monsters fall over on arrival (user 2026-09-30:
/// *"어쨌든 자동으로 몬스터가 죽는걸 원하지 않아"*). A floor the cap missed is harder than
/// its row — never a floor whose monsters die on their own.
pub fn spawned_over_count(count: Option<u8>, creatures: &[Creature]) -> Option<usize> {
    let n = count?;
    (creatures.len() > usize::from(n)).then_some(creatures.len())
}

/// ★ 145차: the top-centre tower banner's content ([`Run::hud`]). The UI draws it; the
/// worker only says what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TowerHud {
    /// The floor, 1..=[`FLOORS`].
    pub floor: u8,
    /// A boss floor (every tenth).
    pub boss: bool,
    pub phase: TowerPhase,
}

/// Where the floor is in its life — the banner's second line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TowerPhase {
    /// The warp is posted and the scene has not loaded yet: no count to show.
    Entering,
    /// The floor is live: `left` monsters still standing.
    Fighting { left: u16 },
    /// Every monster is down; the run is paying out and posting the next floor.
    Cleared,
    /// The last floor is done (the wake follows).
    Completed,
    /// The run is ending any other way (death, EXIT, leaving the world).
    Left,
}

/// Which row a record on this floor is forked with: the boss's own on a boss floor,
/// the escort row (below) for its escorts, the floor's for anyone else.
///
/// ★★ 138차 결정 ⑬ (user, 2026-09-29): an escort takes the floor below's LEVEL and
/// nothing else — its multipliers are [`PCT_NEUTRAL`]. From 137차 the ordinary floors
/// 71..99 ramp hp% to 150, and inheriting that made floor 100's eight escorts 47.5k →
/// 71k each; the decision is «100층은 지금 그대로», and the 80 / 90 raids follow the
/// same rule. ⛔ Do not hand the escorts the row below whole again.
fn row_for(floor: &Floor, adds: Option<&Floor>, c: &Creature) -> Floor {
    if !floor.boss {
        return *floor;
    }
    match (c.species, adds) {
        // Unreadable species on a boss floor = treat as the boss (the stronger fork).
        (None, _) => *floor,
        (Some(sp), _) if sp >= BOSS_BASE => *floor,
        (Some(_), Some(below)) => {
            Floor { hp_pct: PCT_NEUTRAL, def_pct: PCT_NEUTRAL, atk_pct: PCT_NEUTRAL, ..*below }
        }
        (Some(_), None) => *floor,
    }
}

/// Compute the writes for one floor. Live creatures only — forking a corpse would
/// resurrect it, because this file's own clear test reads the HP it would write.
///
/// `adds` is the row whose multipliers a boss floor's escorts get (the floor below).
/// `mem` is the floor's snapshot: the first sighting of a record is taken as its base,
/// so this must be called with creatures read BEFORE any write of the same poll.
///
/// ★ 158차 P8 (user «전부 권장대로», 2026-10-02): the tower's rolls are forked like hell's —
/// `player_level` is the live character's level, and the three roll fields come from
/// [`hell_hit`] / [`hell_dodge`] / [`hell_res`] for the ROW's level against it. Until then
/// the tower kept the species' own fields, so a floor 85+ monster (Lv 111+) was hit at the
/// roll's 15 % floor by anyone without 명중 gear and crit at 2 % (the +5/level term stood
/// unopposed, `ITEM_BALANCE_AUDIT_156.md §0`), while the same Lv 101~120 monster hit the
/// player at 95 %. `None` player level = the base knobs (the player treated as the row's
/// level), the same as [`hell_forks`]. A row with no level writes no roll field.
pub fn forks(floor: &Floor, adds: Option<&Floor>, creatures: &[Creature], mem: &mut FloorMemory, player_level: Option<i8>) -> Vec<Fork> {
    let mut out = Vec::with_capacity(creatures.len());
    for c in creatures.iter().filter(|c| c.alive()) {
        let row = row_for(floor, adds, c);
        // The HP base is the first sighting's `hp_max`; `base_for` keeps it and, on
        // that first sighting, remembers the total when it will not fit the record.
        let seen = mem.bases.iter().find(|b| b.index == c.index).copied();
        let hp0 = seen.map_or(c.hp_max, |b| b.hp0);
        // ★ 112차: the level growth — from the SPAWN level (first sighting of `+0x70`)
        // to the row's level, by the game's own per-level ratio; then the row's
        // percentage on top. No row level, or an unreadable spawn level = no growth.
        let lv0 = seen.map_or(c.level, |b| b.lv0);
        let delta = level_delta(row.level, lv0);
        let hp_total = grown_pct(hp0, HP_GROWTH, delta, row.hp_pct).max(1);
        let b = mem.base_for(c, hp_total);
        let cap = hp_total.clamp(1, I16_MAX as u32) as u16;
        // An untouched monster (or one this fork already raised) is topped up to the
        // new maximum; a wounded one keeps its wound, because a fork re-asserted
        // mid-fight must not heal what the player has hit. Either way it may never
        // exceed the maximum, or the HP bar (43 * cur / max) runs past its 43 pixels.
        // With a virtual word in play the hook keeps `hp_cur` itself; this formula is
        // idempotent against it (a full record stays `cap`, a drained one keeps its
        // value), so the steady state is still zero writes.
        let untouched = c.hp_cur >= c.hp_max;
        let hp_cur = if untouched { cap } else { c.hp_cur.min(cap) };
        let level = row.level.map(|lv| lv.clamp(1, LEVEL_MAX));
        // ★ 158차 P8: the tower's roll fields, the hell way — against the player's level.
        let plv = level.map(|lv| player_level.unwrap_or(lv));
        out.push(Fork {
            index: c.index,
            level,
            hp_max: cap,
            hp_cur,
            atk_eff: clamp_stat(grown_pct(b.atk0, ATK_GROWTH, delta, row.atk_pct)),
            atk_skill: skill_attacks(c, &b, delta, row.atk_pct),
            def_phys_eff: clamp_stat(grown_pct(b.dp0, DEF_GROWTH, delta, row.def_pct)),
            def_spec_eff: clamp_stat(grown_pct(b.ds0, DEF_GROWTH, delta, row.def_pct)),
            vhp: seed_vhp(b.vhp_total, c.vhp),
            hit: level.zip(plv).map(|(m, p)| hell_hit(m, p)),
            dodge: level.zip(plv).map(|(m, p)| hell_dodge(m, p)),
            res: level.zip(plv).map(|(m, p)| hell_res(m, p)),
        });
    }
    out
}

// ── 헬모드 (133차) — the growth model is the tower's, so it lives here ────────
//
// Hell is the playthrough AFTER hardcore: the game's difficulty byte read `!= 0`
// everywhere and copied verbatim by both save directions, so the value 2 is
// invisible to the game (hard tables keep serving) and is the HOST's mark. The
// host writes it on the ENDING falling edge when the byte was already non-zero
// (a hardcore clear), and this section is what hell then does to monsters:
// every field scene's spawns are grown — by the game's own per-level ratios —
// from their hard-table spawn level to [`hell_level`], 68..100 (134차: the ladder's
// INPUT is the hard table's own 38..67, because that is what byte 2 spawns).
//
// ⛔ The tower is NOT hell: inside a run the tower's own fork owns the records
// (its ladder already goes to Lv 120), so the emulator gates the hell fork on
// the tower being idle. ⛔ No percentage rows here — hell has no floor table;
// the level growth IS the difficulty, exactly like the 112차 tower decision.

/// The difficulty-byte value that means hell. 0 = normal, 1 = hard (the game's own
/// writer at `0x3b0bc`), 2 = ours. The game never writes 2 and never distinguishes
/// it from 1 (`cmp #0` at every reader), which is what makes the value free.
pub const HELL_DIFFICULTY: u8 = 2;

/// The hard table's own spawn-level range (`res/dat/enemyh_dat`, 161 species). Hell
/// spawns from the hard table — byte 2 reads as hard — so THIS is the ladder's input,
/// not 1..67: the 133차 ladder assumed a Lv 1 starter rat and left hell 67..85 with no
/// monster in it, which stranded the Lv 73..79 rows (134차). Measured, and re-checked
/// against the asset by `tools_re/gen_helldrop_table.py --check`.
pub const HELL_SPAWN_LV_MIN: i8 = 38;
pub const HELL_SPAWN_LV_MAX: i8 = 67;

/// Where the hell ladder starts and ends: 68 = user decision 2026-09-24 («헬 몬스터는
/// 레벨 68부터 시작해야해»), 100 = the 133차 spec. Mirrored in `helldrop.rs` (the hook
/// gates its roll on this range); the generator asserts the two copies agree.
pub const HELL_LV_MIN: i8 = 68;
pub const HELL_LV_MAX: i8 = 100;

/// Is this character in hell? `None` (unreadable) is not.
pub fn is_hell(difficulty: Option<u8>) -> bool {
    difficulty == Some(HELL_DIFFICULTY)
}

/// What the 2회차 transition leaves in the difficulty byte, given the value the byte held
/// as the transition STARTED (`ending_transition_lgt` latches it before the game writes 1):
/// a non-zero pre-ending difficulty was a hardcore (or hell) clear → hell; a normal
/// clear stays the stock 2회차 (the game's own 1). `None` = leave the byte alone.
pub fn hell_promotion(pre_ending: Option<u8>) -> Option<u8> {
    match pre_ending {
        Some(b) if b != 0 => Some(HELL_DIFFICULTY),
        _ => None,
    }
}

/// The level a monster that SPAWNED at `lv0` fights at in hell: the linear map
/// `[HELL_SPAWN_LV_MIN..=HELL_SPAWN_LV_MAX] → [HELL_LV_MIN..=HELL_LV_MAX]` (38..67 →
/// 68..100, rounded to nearest), so the world's own difficulty ordering is preserved
/// (the hard table's weakest spawn lands at 68, its Lv 67 endgame at 100). The level is
/// also what the guest hook judges hell's own drops on (152차, `helldrop`): 68..78 = the
/// stock hard loot only, 79..85 a 조합서, 85+ a new item, 79+ a 닉스코어파편 — so a hard
/// spawn of 48+ (→ 79) is where the new loot starts and 53+ (→ 85) where the items do.
/// Inputs outside the hard range clamp: below is impossible in hell (byte 2 spawns the
/// hard table), above is a tower record.
pub fn hell_level(lv0: i8) -> i8 {
    let d = i32::from(lv0.clamp(HELL_SPAWN_LV_MIN, HELL_SPAWN_LV_MAX) - HELL_SPAWN_LV_MIN);
    let span_in = i32::from(HELL_SPAWN_LV_MAX - HELL_SPAWN_LV_MIN);
    let span_out = i32::from(HELL_LV_MAX - HELL_LV_MIN);
    (i32::from(HELL_LV_MIN) + (d * span_out + span_in / 2) / span_in) as i8
}

// ── 156차 — 헬 몬스터의 명중 · 회피 · 기절 저항 · 방어 (2026-10-02) ──────────────────
//
// User: *"헬몬스터들의 명중률, 회피율, 방어도, 기절을 상향해서 캐릭터의 명중, 회피, 방어무시,
// 기절 옵션들을 조금더 유효한 옵션으로 … 명중을 맞추지 않았다고 해서 miss가 너무 많이 뜰
// 필요없고 기절옵션을 맞추지 않았다고 해서 … 크리티컬이 너무 안뜰 필요는 없어"*.
//
// The game's three rolls (RE, `HELL_COMBAT_BALANCE.md` — all CONFIRMED on `binary.mod`
// and measured headlessly):
//
//   player hits   = clamp(80 + 명중 + d24 − dodge_m + 5·(pLv − mLv), 15, 95)   (`0x4bf60`)
//   player crits  = clamp(d25 + 기절 − res_m   + 1·(pLv − mLv),  2, 60)   (`0x4bed8`, ×1.5)
//   monster hits  = clamp(80 + hit_m − 회피     + 5·(mLv − pLv), 15, 95)   (`0x4bfca`)
//
// `명중` = (u8)stat+0x2c (passives 10+2·T(n) + worn options), `회피` = 민첩/3 + passives +
// options, `기절`/`저항` = worn options (+ passives); `d24`/`d25` are the weapon TREE's own
// constants (창술/단도/영탄/광아 20/1 · 검술 60/1 · 사격 0/20 · 격발 0/1). And the hard
// table's own monster fields are: dodge 10..45, crit-resist 37..87, and 명중 NEGATIVE for
// every species of Lv 49+ (−127..−44: the dat byte is 128+ and the species loader
// sign-extends it) — so a hell monster hits the player at the roll's 15 % FLOOR whatever
// the player wears, and a player without 기절 crits at the 2 % floor. Neither option could
// be "유효" against that.
//
// The fork therefore writes the three fields as functions of the monster's hell level and
// the PLAYER's level, cancelling most of the game's ±5-per-level term so each knob below
// reads as a plain percentage against a same-level player:
//
//   monster hit %  = 80 + HELL_HIT_BASE   − 회피 + (mLv − pLv)·(5 − HELL_HIT_PER_LV)   (= +1/level)
//   player hit %   = 80 + 명중 + d24 − HELL_DODGE_BASE − (mLv − pLv)·(5 − HELL_DODGE_PER_LV) (= 0/level)
//   player crit %  = d25 + 기절 − HELL_RES_BASE                     (level term cancelled)
//
// 156차: `HELL_DODGE_BASE` is the user's choice (35, 2026-10-02 — §4.1 of the doc); the
// rest are the proposal the user then took as the baseline for the item balance audit.
// Every value is one constant here, and `HELL_COMBAT_BALANCE.md §4` tabulates what they give.
// ⛔ The fields are written with the game's own representation — a NEGATIVE 명중 or
// 저항 goes in as a two's-complement halfword, which the rolls read with `ldrh` and then
// truncate-and-sign-extend the whole sum, so it acts as the negative it is.

/// Monster 명중 against a same-level player, before the player's 회피: `80 + this − 회피`.
/// 60 ⇒ a player with the usual 회피 ~70 (민첩 60 → 20, passives 50) is hit 70 % of the
/// time with no 회피 gear, 40 % with +30, 15 % (floor) with +85 or more.
pub const HELL_HIT_BASE: i32 = 60;
/// How much of the roll's own +5-per-level is cancelled on the monster's 명중 (4 of 5: a
/// monster 10 levels above still gains +10 %, not +50 %).
pub const HELL_HIT_PER_LV: i32 = 4;
/// Monster 회피 against ANY player level: `80 + 명중 + d24 − this`. 35 = the user's choice
/// (2026-10-02, `HELL_COMBAT_BALANCE.md §4.1`): only the four classes with the 검술 tree have
/// a 명중 passive worth 40 — the other six have 0 (or 사격's 조준, 20). So with NO 명중 gear a
/// gun class hits 45 % (65 % with 조준 5), a 20-tree weapon 65 %, 검술 (60) the 95 % cap;
/// every +20 of 명중 gear is +20 %p into that cap.
pub const HELL_DODGE_BASE: i32 = 35;
/// The −5-per-level on the player's hit is cancelled FULLY (user: a player without 명중
/// gear must not miss too much — and a Lv 85 player meets Lv 100 monsters in hell).
pub const HELL_DODGE_PER_LV: i32 = 5;
/// Monster 기절 저항 against a same-level player: `d25 + 기절 − this`. −12 ⇒ 13 % crits with
/// no 기절 gear (사격's d25 20 → 32 %), 60 % (cap) from +47 on.
pub const HELL_RES_BASE: i32 = -12;
/// The crit roll's own +1-per-level is cancelled fully (it is small either way).
pub const HELL_RES_PER_LV: i32 = 1;
/// 방어도 상향: the hell fork's defence percentage on top of the level growth (the tower's
/// rows use 100). 130 = ×1.3 — meaningful with 방패무시 now implemented (`dmgwide`).
pub const HELL_DEF_PCT: u16 = 130;
/// The 명중 field is an `i16` the roll sums into a 16-bit result; keep well inside.
pub const HELL_HIT_CLAMP: i32 = 200;
/// The 회피 field's clamp (both signs: the roll truncates its sum to 16 bits and
/// sign-extends it, so a two's-complement negative 회피 simply raises the player's hit —
/// which is what a monster far below the player gets, and the roll caps at 95 anyway).
pub const HELL_DODGE_CLAMP: i32 = 300;

/// The monster's 명중 for `mlv` against a player at `plv`, pure.
pub fn hell_hit(mlv: i8, plv: i8) -> i16 {
    let d = i32::from(mlv) - i32::from(plv);
    (HELL_HIT_BASE - HELL_HIT_PER_LV * d).clamp(-HELL_HIT_CLAMP, HELL_HIT_CLAMP) as i16
}

/// The monster's 회피 for `mlv` against a player at `plv`, pure — as the halfword the record
/// holds (a negative value two's-complemented, like [`hell_res`]).
pub fn hell_dodge(mlv: i8, plv: i8) -> u16 {
    let d = i32::from(mlv) - i32::from(plv);
    (HELL_DODGE_BASE - HELL_DODGE_PER_LV * d).clamp(-HELL_DODGE_CLAMP, HELL_DODGE_CLAMP) as i16 as u16
}

/// The monster's 기절 저항 for `mlv` against a player at `plv`, pure — as the halfword the
/// record holds (a negative value two's-complemented, which the roll sign-extends back).
pub fn hell_res(mlv: i8, plv: i8) -> u16 {
    let d = i32::from(plv) - i32::from(mlv);
    (HELL_RES_BASE + HELL_RES_PER_LV * d).clamp(-HELL_HIT_CLAMP, HELL_HIT_CLAMP) as i16 as u16
}

/// What the three hell rolls give, pure: `(monster hit %, player hit %, player crit %)` for
/// a monster at `mlv` against a player at `plv` with 회피 `dodge`, 명중 `hit` (the u8 sum
/// the roll reads), weapon tree constants `d24`/`d25` and 기절 `stun` — the game's own
/// formulas with the fork's fields substituted. For tables and tests; the game rolls.
pub fn hell_rates(mlv: i8, plv: i8, dodge: i32, hit: i32, d24: i32, d25: i32, stun: i32) -> (i32, i32, i32) {
    let d = i32::from(mlv) - i32::from(plv);
    let m_hit = (80 + i32::from(hell_hit(mlv, plv)) - dodge + 5 * d).clamp(15, 95);
    let p_hit = (80 + hit + d24 - i32::from(hell_dodge(mlv, plv) as i16) - 5 * d).clamp(15, 95);
    let res = i32::from(hell_res(mlv, plv) as i16);
    let p_crit = (d25 + stun - res - d).clamp(2, 60);
    (m_hit, p_hit, p_crit)
}

/// The hell fork: [`forks`] with a PER-CREATURE target level (each record grows from
/// its own spawn level to [`hell_level`] of it) and no percentage rows. Same snapshot
/// (`mem`), same idempotence (absolute targets from the first sighting, wounds kept,
/// zero writes at the steady state), same virtual-HP seeding for totals past `i16`.
///
/// ★ 156차: the three roll fields (`hit`/`dodge`/`res`) come from the record's target level
/// and `player_level` (the live character's; `None` = treat the player as the monster's
/// level, i.e. the base knobs). Defence grows with [`HELL_DEF_PCT`] on top.
pub fn hell_forks(creatures: &[Creature], mem: &mut FloorMemory, player_level: Option<i8>) -> Vec<Fork> {
    let mut out = Vec::with_capacity(creatures.len());
    for c in creatures.iter().filter(|c| c.alive()) {
        let seen = mem.bases.iter().find(|b| b.index == c.index).copied();
        let hp0 = seen.map_or(c.hp_max, |b| b.hp0);
        let lv0 = seen.map_or(c.level, |b| b.lv0);
        // An unreadable spawn level grows nothing (delta 0) and writes no level byte —
        // the fork degenerates to the identity and the plan stays empty.
        let target = lv0.map(hell_level);
        let delta = level_delta(target, lv0);
        let hp_total = grown_pct(hp0, HP_GROWTH, delta, PCT_NEUTRAL).max(1);
        let b = mem.base_for(c, hp_total);
        let cap = hp_total.clamp(1, I16_MAX as u32) as u16;
        let untouched = c.hp_cur >= c.hp_max;
        let hp_cur = if untouched { cap } else { c.hp_cur.min(cap) };
        let level = target.map(|lv| lv.clamp(1, LEVEL_MAX));
        let plv = level.map(|lv| player_level.unwrap_or(lv));
        out.push(Fork {
            index: c.index,
            level,
            hp_max: cap,
            hp_cur,
            atk_eff: clamp_stat(grown_pct(b.atk0, ATK_GROWTH, delta, PCT_NEUTRAL)),
            atk_skill: skill_attacks(c, &b, delta, PCT_NEUTRAL),
            def_phys_eff: clamp_stat(grown_pct(b.dp0, DEF_GROWTH, delta, HELL_DEF_PCT)),
            def_spec_eff: clamp_stat(grown_pct(b.ds0, DEF_GROWTH, delta, HELL_DEF_PCT)),
            vhp: seed_vhp(b.vhp_total, c.vhp),
            hit: level.zip(plv).map(|(m, p)| hell_hit(m, p)),
            dodge: level.zip(plv).map(|(m, p)| hell_dodge(m, p)),
            res: level.zip(plv).map(|(m, p)| hell_res(m, p)),
        });
    }
    out
}

/// ★ 160차 T4 (user 2026-10-03: «보스 스킬 맞고 원턴킬 당해도 괜찮아. 오히려 스킬맞고 원턴킬
/// 맞는건 의도한거야. 그래서 스킬을 피해야해.») — a boss's second and third attack values,
/// grown exactly like its first.
///
/// A boss record carries THREE attacks (`+0x8e` · `+0x90` · `+0x92`, picked by the boss's
/// state byte) and the forks only ever grew the first, so 홀리가디언's two skill attacks
/// stayed at their spawn 2,410 / 1,205 on floor 100 while its plain hit was 22,443 — the
/// skills were the SAFE part of the fight. `None` for anything that is not a known boss
/// (an ordinary record's `+0x90` / `+0x92` are other fields); a zero base stays zero (that
/// boss has no such attack — `grow` would otherwise make it 1).
fn skill_attacks(c: &Creature, b: &Base, delta: i32, pct: u16) -> Option<[u16; 2]> {
    matches!(c.species, Some(sp) if sp >= BOSS_BASE)
        .then(|| b.atk_skill0.map(|a| if a == 0 { 0 } else { clamp_stat(grown_pct(a, ATK_GROWTH, delta, pct)) }))
}

/// `floor_lv - spawn_lv`, or 0 when either side is unknown (no growth). The row's
/// level is clamped the way the fork writes it, so the growth and the byte agree.
pub fn level_delta(row_level: Option<i8>, spawn_level: Option<i8>) -> i32 {
    match (row_level, spawn_level) {
        (Some(lv), Some(lv0)) => i32::from(lv.clamp(1, LEVEL_MAX)) - i32::from(lv0),
        _ => 0,
    }
}

/// The grown stat with the row's percentage on top, unclamped: `value * growth^delta *
/// pct / 100`. HP goes through this unclamped (the virtual word takes the excess);
/// the other stats are clamped to the record by [`clamp_stat`].
pub fn grown_pct(value: u16, growth: f64, delta: i32, pct: u16) -> u32 {
    let g = grow(value, growth, delta);
    let p = u64::from(g) * u64::from(pct.max(1)) / 100;
    p.min(u64::from(u32::MAX)) as u32
}

/// Into the record's signed halfword: never 0, never above [`I16_MAX`].
pub fn clamp_stat(v: u32) -> u16 {
    v.clamp(1, I16_MAX as u32) as u16
}

/// Whether this poll seeds the record's virtual HP word. Pure.
///
/// Only a total that does not fit the record (`Some`) and only while the word still
/// reads 0: once the `dmg_apply_lgt` hook has it, every hit lowers it, and a poll that
/// wrote the total back would heal the monster by everything dealt since. A spent word
/// never reads 0 on a LIVE record (the hit that spends it also kills), so "0" is
/// exactly "not yet seeded".
///
/// ★ And a FRESH record reads 0 on every scene load (104차-B code review): the creature
/// array is `MC_knlCalloc`'d by the map loader (`0x373c2` through the `.data` stub
/// `0x1400264` = svc `0x76` Calloc, zero-filled), so a floor left mid-fight cannot hand
/// its half-spent word to the next scene's monsters and the next floor's boss never
/// inherits one — nothing clears the word because nothing has to. The respawn RE-INIT is
/// the one path that keeps it, and a respawn only ever reuses a KILLED record, whose
/// spending hit wrote 0. Measured: `--crea-set` sentinels in `+0x100`/`+0x104` on scene
/// 243 read 0 on all 20 records after a warp to 244, with the array re-allocated at the
/// SAME address (`PLAY_FEEDBACK_FIXES.md §104차-B`).
pub fn seed_vhp(total: Option<u32>, word_now: u32) -> Option<u32> {
    total.filter(|_| word_now == 0)
}

/// Why a run ended other than by finishing it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Abort {
    /// The scene word never became the floor's scene. Either the request was never
    /// consumed or the destination was refused.
    WarpTimedOut,
    /// The player died.
    PlayerDown,
    /// The player asked to leave.
    Quit,
    /// The floor table has no row for the floor we were about to enter.
    NoSuchFloor,
    /// The game left the field — a load, a title-screen trip, anything. Continuing
    /// would fork monsters in a scene the tower did not choose.
    LeftTheField,
    /// ★ 138차: floor 100 was cleared — the tower is FINISHED. Until 138차 this was a
    /// `State::Done` that answered nothing at all: no warp home (the player stood in a
    /// sealed room with no door), the run read as idle so the seal and the no-save byte
    /// were lowered INSIDE the tower, and the poll block closed on the owed rows. Now it
    /// leaves by the same road as a death or a quit, so the exit reward, the way back
    /// and `finish()` are one path (`TOWER_REWARD_BALANCE_DRAFT.md §3.1 ⓒ`).
    Completed,
}

/// The run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Not in the tower. Produces no actions at all: outside a run the guest is
    /// untouched.
    Idle,
    /// The warp has been posted and we are waiting for the scene word.
    Entering { floor: u8, polls: u32 },
    /// On the floor. `saw_alive` is the guard against clearing a floor whose spawn
    /// loop has not run yet.
    InFloor { floor: u8, saw_alive: bool },
    /// Floor done, reward not yet taken. ⛔ Still a floor the player is standing on:
    /// the same three abort checks as `InFloor` apply here (see [`Run::poll`]).
    Cleared { floor: u8 },
    /// Over, and why — [`Abort::Completed`] for a finished tower (138차: there is no
    /// separate "done" state any more; a finished run leaves like any other).
    Failed { floor: u8, why: Abort },
}

/// What the caller sees each poll. Everything here is a read the worker already does
/// or can do with the anchors in `SceneWorld` / `CreatureRecord`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Obs {
    /// `SceneWorld::scene_id_off`. `None` when it could not be read.
    pub scene: Option<u16>,
    /// `MAP + SceneWorld::map_spawn_count_off` — how many records the loader filled.
    pub spawned: i32,
    /// How many of those have positive current HP.
    pub alive: i32,
    /// STRICT: the game is in the field with NO window open — which is the only state
    /// the warp request is serviced from (§A.1b). Used to decide whether a warp may be
    /// POSTED, and nothing else.
    ///
    /// ⛔⛔ DO NOT USE THIS TO DECIDE WHETHER THE RUN IS STILL VALID. `in_field_state`
    /// is false whenever any window is up, so a player who opens their inventory on a
    /// floor is not in the field — and an abort on that would throw them out of the
    /// tower for looking at their bag. That is what [`Self::in_world`] is for.
    pub in_field: bool,
    /// LOOSE: a world is loaded (the app is not on the title / slot / online screens,
    /// which all share app state 3). This is what "the run is still valid" means:
    /// windows come and go, but leaving the world means the floor is gone.
    pub in_world: bool,
    /// The player's current HP. **`None` = could not be read, which is NOT death.**
    ///
    /// ⛔ The reading comes from `items::read_stats`, which legitimately refuses in
    /// transient states (a scene load, an unequipped character — 71차-L/80차-E are two
    /// sessions spent on exactly that). Treating a refusal as 0 would abort runs at
    /// random, and death is caught anyway by [`Self::in_world`] going false when the
    /// game's own game-over takes over.
    pub player_hp: Option<u16>,
    /// The tile the player object stands on (`items::player_position`), `None` when
    /// the chain could not be read. Only [`Run::enter`] uses it, on the FIRST hop, to
    /// remember where the run started — see [`Run::origin_tile`].
    pub player_tile: Option<(u8, u8)>,
}

/// What the caller should do about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Do nothing. Includes every poll while [`State::Idle`].
    None,
    /// Post a warp: write `scene` to `warp_dest_off`, [`WARP_ENTRY`] to
    /// `warp_entry_off`, then `1` to `warp_request_off` — destination and entry
    /// BEFORE the request, because the request is what arms it.
    Warp { scene: u8 },
    /// Re-assert the floor's stat fork. The caller re-reads the creature array first:
    /// ⛔ the array is REALLOCATED on every scene load (measured: `0x4014ceac` ->
    /// `0x40146884` across one round trip), so a cached pointer is a wild write.
    Fork { floor: u8 },
    /// The floor is clear. Pay out, then call [`Run::next_floor`] or
    /// [`Run::leave`].
    Reward { floor: u8, boss: bool },
    /// The run is over; put the player back where they came from.
    Leave { why: Abort },
}

/// The state machine plus the table.
#[derive(Clone, Debug)]
pub struct Run {
    pub state: State,
    floors: Vec<Floor>,
    /// The highest floor cleared, for the logs (138차: the per-slot [`Progress`] is the
    /// file; the worker loads this from it).
    pub best: u8,
    /// ★ 138차: the highest BOSS floor cleared in THIS run (0 = none) — what the exit
    /// reward pays on ([`exit_payout`]). Reset when a run starts from idle and by
    /// [`Run::finish`], so the caller must read it BEFORE it applies the `Leave`.
    pub run_best_boss: u8,
    /// The scene the player was standing on when the run started, so leaving can put
    /// them back. `None` when it could not be expressed as a warp destination.
    ///
    /// ~~The POSITION inside that scene is not preserved~~ → it is, since 2026-09-17:
    /// [`Self::origin_tile`] is the tile the player stood on when the first hop was
    /// posted, and the worker writes it back once the leave warp has landed in that
    /// scene (the two-byte warp carries no coordinates, so without this the player
    /// came out of the tower on whatever tile they had on the LAST FLOOR — "엉뚱한
    /// 곳", the report). ⛔ Still not an entry-point value (§B.3): the entry byte stays
    /// [`WARP_ENTRY`]; the placement is the same tile+pixel write the landing fix uses.
    origin: Option<u8>,
    /// Where in `origin` the run started — the tile in front of the passer-by, for the
    /// in-game entrance. `None` when the player object could not be read then.
    origin_tile: Option<(u8, u8)>,
    /// The current floor's record snapshot — see [`FloorMemory`].
    pub memory: FloorMemory,
}

impl Run {
    /// A run over a floor table. Invalid rows are dropped here rather than at use
    /// time, so a damaged table degrades to "fewer floors" instead of a wild warp —
    /// the same rule the buff file follows.
    pub fn new(floors: Vec<Floor>) -> Self {
        let mut floors: Vec<Floor> = floors.into_iter().filter(Floor::valid).collect();
        floors.sort_by_key(|f| f.floor);
        floors.dedup_by_key(|f| f.floor);
        Self {
            state: State::Idle,
            floors,
            best: 0,
            run_best_boss: 0,
            origin: None,
            origin_tile: None,
            memory: FloorMemory::default(),
        }
    }

    /// The row whose LEVEL a boss floor's escorts take: the ordinary floor just below
    /// (its multipliers are not theirs — [`row_for`], 138차 결정 ⑬).
    pub fn adds_row(&self, floor: u8) -> Option<&Floor> {
        floor.checked_sub(1).filter(|f| *f >= 1).and_then(|f| self.floor(f))
    }

    /// The writes for `floor` against these creatures, through this run's memory.
    /// `player_level` = the live character's (158차 P8: the roll fields are set against it).
    pub fn forks_for(&mut self, floor: u8, creatures: &[Creature], player_level: Option<i8>) -> Vec<Fork> {
        let Some(f) = self.floor(floor).copied() else { return vec![] };
        let adds = self.adds_row(floor).copied();
        forks(&f, adds.as_ref(), creatures, &mut self.memory, player_level)
    }

    /// Where [`Action::Leave`] should send the player.
    pub fn origin(&self) -> Option<u8> {
        self.origin
    }

    /// The tile inside [`Self::origin`] the run started on, to stand the player back on
    /// once the leave warp has landed there. Read it BEFORE [`Self::finish`].
    pub fn origin_tile(&self) -> Option<(u8, u8)> {
        self.origin_tile
    }

    /// The row for a floor, if the table has one.
    pub fn floor(&self, floor: u8) -> Option<&Floor> {
        self.floors.iter().find(|f| f.floor == floor)
    }

    /// How many usable rows the table ended up with.
    pub fn len(&self) -> usize {
        self.floors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.floors.is_empty()
    }

    /// Start a run at `floor`. Refuses unless the game is in the field, because the
    /// two-byte warp is only serviced from there.
    pub fn enter(&mut self, floor: u8, obs: &Obs) -> Action {
        if !obs.in_field {
            return Action::None;
        }
        match self.floor(floor) {
            None => {
                self.state = State::Failed { floor, why: Abort::NoSuchFloor };
                Action::Leave { why: Abort::NoSuchFloor }
            }
            Some(f) => {
                let scene = f.scene;
                // A run that starts from idle starts with nothing to pay (138차).
                if self.state == State::Idle {
                    self.run_best_boss = 0;
                }
                // Remember where we came from, but only on the FIRST hop of a run:
                // floor 2's origin is floor 1, and coming out on floor 1 is not
                // "putting the player back".
                if self.origin.is_none() {
                    self.origin = obs
                        .scene
                        .and_then(|s| u8::try_from(s).ok())
                        .filter(|s| *s != scene && *s != 0);
                    // The tile goes with the scene: a tile without its scene is a
                    // position on some other map.
                    self.origin_tile = self.origin.and(obs.player_tile);
                }
                // A new scene is a new creature array: nothing remembered applies.
                self.memory.clear();
                self.state = State::Entering { floor, polls: 0 };
                Action::Warp { scene }
            }
        }
    }

    /// Advance to the floor above, from [`State::Cleared`]. Finishing the last floor
    /// ends the run instead.
    pub fn next_floor(&mut self, obs: &Obs) -> Action {
        let State::Cleared { floor } = self.state else { return Action::None };
        if floor >= FLOORS {
            // ★ 138차: the tower is finished — leave like any other ending (the exit
            // reward, the way home, `finish`). See [`Abort::Completed`].
            self.state = State::Failed { floor, why: Abort::Completed };
            return Action::Leave { why: Abort::Completed };
        }
        // ⛔ WAIT, do not fail. A warp is only serviced from the field, and the player
        // may well be standing in their inventory when the last monster dies. Staying
        // in `Cleared` means the caller asks again; entering now would post a request
        // nothing consumes and `Entering` would time the run out for no reason.
        if !obs.in_field {
            return Action::None;
        }
        self.enter(floor + 1, obs)
    }

    /// Put the run away once its [`Action::Leave`] has been carried out.
    ///
    /// ⛔ THE CALLER MUST DO THIS. `Failed` keeps answering `Leave` on every poll, on
    /// purpose — a warp can take a few frames and an action that stopped being offered
    /// after one poll could be missed. So the worker warps and then says "done", and
    /// only that puts the machine back to [`State::Idle`] (where it emits nothing).
    pub fn finish(&mut self) {
        self.state = State::Idle;
        self.origin = None;
        self.origin_tile = None;
        self.memory.clear();
        self.run_best_boss = 0;
    }

    /// Give up, from anywhere.
    pub fn leave(&mut self) -> Action {
        let floor = self.current_floor();
        self.state = State::Failed { floor, why: Abort::Quit };
        Action::Leave { why: Abort::Quit }
    }

    /// The floor the run is on, or 0 when it is not on one.
    pub fn current_floor(&self) -> u8 {
        match self.state {
            State::Entering { floor, .. }
            | State::InFloor { floor, .. }
            | State::Cleared { floor }
            | State::Failed { floor, .. } => floor,
            State::Idle => 0,
        }
    }

    /// One poll. The caller does this at whatever cadence it already polls the guest
    /// at; nothing here depends on the interval.
    pub fn poll(&mut self, obs: &Obs) -> Action {
        match self.state {
            State::Idle => Action::None,
            State::Failed { why, .. } => Action::Leave { why },

            State::Entering { floor, polls } => {
                let want = self.floor(floor).map(|f| f.scene);
                // Arriving is the scene word matching the floor's scene. `spawned`
                // is deliberately NOT part of this: a floor may legitimately have no
                // monsters, and the spawn loop runs after the scene word is set.
                if want.is_some_and(|s| obs.scene == Some(s as u16)) {
                    self.state = State::InFloor { floor, saw_alive: obs.alive > 0 };
                    return Action::Fork { floor };
                }
                if polls + 1 >= ENTER_POLLS {
                    self.state = State::Failed { floor, why: Abort::WarpTimedOut };
                    return Action::Leave { why: Abort::WarpTimedOut };
                }
                self.state = State::Entering { floor, polls: polls + 1 };
                Action::None
            }

            State::InFloor { floor, saw_alive } => {
                if obs.player_hp == Some(0) {
                    self.state = State::Failed { floor, why: Abort::PlayerDown };
                    return Action::Leave { why: Abort::PlayerDown };
                }
                if !obs.in_world {
                    self.state = State::Failed { floor, why: Abort::LeftTheField };
                    return Action::Leave { why: Abort::LeftTheField };
                }
                // Left the floor without our warp — a warp tile, a script, anything.
                // Forking monsters in a scene the tower did not choose is exactly the
                // "no forks outside a run" invariant being broken.
                if self.floor(floor).map(|f| f.scene as u16) != obs.scene {
                    self.state = State::Failed { floor, why: Abort::LeftTheField };
                    return Action::Leave { why: Abort::LeftTheField };
                }
                let saw_alive = saw_alive || obs.alive > 0;
                if saw_alive && obs.alive == 0 {
                    self.state = State::Cleared { floor };
                    self.best = self.best.max(floor);
                    let boss = self.floor(floor).is_some_and(|f| f.boss);
                    if boss {
                        self.run_best_boss = self.run_best_boss.max(floor);
                    }
                    return Action::Reward { floor, boss };
                }
                self.state = State::InFloor { floor, saw_alive };
                // Re-assert while the floor is live: monsters respawn, the game
                // recomputes effective stats on its own, and the array moves.
                Action::Fork { floor }
            }

            State::Cleared { floor } => {
                // ★★ THE SAME THREE CHECKS AS `InFloor` (code review 2026-09-11). A
                // cleared floor is still a floor the player is standing on — for the
                // clear-hold, and for as long as an open window keeps `next_floor`
                // waiting — and a run that only watched the world while monsters were
                // alive could be left parked here by a player who went 시스템 → 메인메뉴.
                // The next character to reach a field (any slot, any difficulty) was
                // then answered with `enter(floor + 1)` and dropped into the tower.
                if obs.player_hp == Some(0) {
                    self.state = State::Failed { floor, why: Abort::PlayerDown };
                    return Action::Leave { why: Abort::PlayerDown };
                }
                if !obs.in_world {
                    self.state = State::Failed { floor, why: Abort::LeftTheField };
                    return Action::Leave { why: Abort::LeftTheField };
                }
                if self.floor(floor).map(|f| f.scene as u16) != obs.scene {
                    self.state = State::Failed { floor, why: Abort::LeftTheField };
                    return Action::Leave { why: Abort::LeftTheField };
                }
                Action::Reward { floor, boss: self.floor(floor).is_some_and(|f| f.boss) }
            }
        }
    }

    /// The one-line readout the overlay shows while a run is live, or `None` when
    /// there is none — so the line costs nothing outside the tower and cannot linger.
    ///
    /// ★★ WHY THIS EXISTS. The in-game entrance shipped without any sign that it had
    /// worked, and the first player through it reported *"뭐야 이동했떠니 벽에
    /// 끼어있는데?"* — they had in fact arrived on floor 1 and were standing at the
    /// warp's landing corner (§H). Nothing on screen said "하늘성 1층", the minimap
    /// and SAVE button had just vanished (by design), and the obvious reading of all
    /// that is "the game broke". One line removes the whole class of report.
    ///
    /// ⛔ ASCII digits, KS X 1001 syllables only: the game font skips a cp949
    /// extension SILENTLY (`gamefont_selftest::the_settings_vocabulary_renders`).
    ///
    /// ★ 145차: the screen no longer shows this string — the top-centre banner draws
    /// [`Self::hud`] in the damage-digit font. It is kept as that value's plain reading
    /// (and the tests pin the wording through it).
    pub fn status_line(&self, alive: i32) -> Option<String> {
        let h = self.hud(alive)?;
        let head = format!("하늘성 {}층{}", h.floor, if h.boss { " 보스" } else { "" });
        Some(match h.phase {
            TowerPhase::Entering => format!("{head} 진입"),
            TowerPhase::Cleared => format!("{head} 클리어"),
            TowerPhase::Fighting { left } => format!("{head} 남은 {left}"),
            TowerPhase::Completed => format!("{head} 완주"),
            TowerPhase::Left => format!("{head} 이탈"),
        })
    }

    /// ★ 145차: what the top-centre banner shows (user 2026-09-30: *"HP SP바 밑에 하늘성
    /// 층수와 남은 몬스터 표시되고있던거를 중앙 상단 … 데미지 폰트에 색상만 다르게"*) — the
    /// floor, whether it is a boss floor, and the phase with the live count. `None`
    /// outside a run, so the banner costs nothing anywhere else and cannot linger.
    pub fn hud(&self, alive: i32) -> Option<TowerHud> {
        let floor = self.current_floor();
        if floor == 0 {
            return None;
        }
        let boss = self.floor(floor).is_some_and(|f| f.boss);
        let phase = match self.state {
            // ⛔ Do NOT show a monster count while entering: the scene has not loaded,
            // so `alive` is still the PREVIOUS floor's — the one number that would be
            // a lie is the one a player would act on.
            State::Entering { .. } => TowerPhase::Entering,
            State::Cleared { .. } => TowerPhase::Cleared,
            State::InFloor { .. } => TowerPhase::Fighting { left: alive.clamp(0, i32::from(u16::MAX)) as u16 },
            // `Failed` is one poll wide (the caller warps and calls `finish`), but say
            // something rather than flash the floor's last value.
            State::Failed { why: Abort::Completed, .. } => TowerPhase::Completed,
            State::Failed { .. } => TowerPhase::Left,
            State::Idle => return None,
        };
        Some(TowerHud { floor, boss, phase })
    }

    /// Is a fork allowed to touch guest memory right now? The one gate the caller
    /// needs before writing anything, so that "no writes outside a run" is a single
    /// question rather than a rule to remember.
    pub fn forking(&self) -> bool {
        matches!(self.state, State::InFloor { .. })
    }

    /// ★ 143차: is `scene` one of the tower's own floor scenes (the arenas the table
    /// warps into)? Asked with NO run as well — a save made inside the tower before
    /// 143차 loads the player into an arena with nothing running, and those rooms have
    /// no door (`TOWER_REWARD_BALANCE_DRAFT.md §3.1 ⓒ`'s old `State::Done`). The EXIT
    /// tab and the SAVE gate key on this so such a player can still walk out.
    pub fn is_floor_scene(&self, scene: u16) -> bool {
        self.floors.iter().any(|f| u16::from(f.scene) == scene)
    }

    /// ★ 143차: the last floor is cleared and the run is waiting for its way out — the
    /// state [`WakePhase`] takes over from instead of [`Self::next_floor`]'s instant
    /// leave.
    pub fn at_the_top(&self) -> bool {
        matches!(self.state, State::Cleared { floor } if floor >= FLOORS)
    }
}

// ── 143차: THE WAY OUT OF A FINISHED TOWER (the wake) ───────────────────────
//
// User, 2026-09-30: *"하늘성 100층을 클리어하면 이제 하늘성밖으로 나와져야해. 화면이
// 그라데이션으로 점점 하얀색으로(의식이 돌아오는듯한 애니메이션 연출 느낌) 전체화면이 하얀색으로
// 밝아졌다가 다시 그라데이션으로 기존(하늘성에 입장하기 전) 상태로 돌아오고서 대화창으로
// "의식이 몽롱하다..." 라고 뜨게"*. (The passer-by sold the tower as a trip of the mind — 「이
// 목걸이를 손에 꽉쥐고 눈을 감으면 그 하늘성으로 의식을 잠시 이동할 수있어요」 — so finishing it
// is waking up.)
//
// Who does what:
//   * the SCREEN is the host's: a white veil over everything, raised and lowered by the
//     UI on its own clock (`overlay::draw_wake_veil`) — the worker only says when;
//   * the WAY HOME is the run's own `Leave`, posted while the screen is fully white, so
//     the tower-to-town cut is never seen;
//   * the LINE is the GAME's dialogue window: the entrance towns' entry row says it at
//     load when `items::GUARDIAN_FLAG_WAKE` is set (`tools_re/patch_guardian_npc.py`),
//     after a script wait that lets the veil clear first. ⛔ The entry row runs ONCE per
//     load (probe 2026-09-30: re-arming 0xff after the load re-ran nothing), so the flag
//     is raised BEFORE the way home is posted — [`WakeStep::GoHome`].
//
// Pure, like everything above: the worker hands in a [`WakeView`] each poll and carries
// out the [`WakeStep`] it gets back.

/// The line, word for word (user 2026-09-30). ⛔ Must equal `patch_guardian_npc.py`'s
/// `WAKE_LINE` — the host says THIS when the game cannot (`WakeStep::SayOnHost`), so the
/// two routes read the same.
pub const WAKE_LINE: &str = "의식이 몽롱하다...";
/// How long the wake waits for a quiet screen (the field, no game window, no host
/// message box or side note) before it whitens anyway. The clear's own 「하늘성 100층
/// 클리어」 line is the usual thing up (a side note since 2026-09-30 — it fades by itself);
/// whitening over it would wash it out before it is read.
pub const WAKE_QUIET_MAX_MS: u64 = 60_000;
/// Black-less fade TO white.
pub const WAKE_WHITEN_MS: u32 = 1_600;
/// Held fully white once the way home has landed — long enough for the town's own
/// entry fade (`33 00 08`, from black) to finish under it, so no black frame shows.
pub const WAKE_HOLD_MS: u64 = 700;
/// White back to the town.
pub const WAKE_CLEAR_MS: u32 = 1_800;
/// A way home that has not landed by then is given up on: the veil comes down anyway
/// (the player is not left staring at white) and the EXIT tab, which keys on the
/// SCENE, is still there to walk out with. The landing block re-posts before this.
pub const WAKE_ARRIVE_MAX_MS: u64 = 12_000;
/// After the veil is gone, the game's line has this long to open its window before the
/// host says it instead (an asset without the wake row, or a town that is not one of the
/// two entrance scenes).
pub const WAKE_SAY_MAX_MS: u64 = 8_000;
/// …and the player this long to read and close it before what the wake held back (the
/// exit reward's box) is released regardless.
pub const WAKE_READ_MAX_MS: u64 = 120_000;

/// Where the wake is. `since`/`at` are the caller's wall-clock milliseconds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WakePhase {
    /// The top floor is cleared and its hold is over; waiting for a quiet screen.
    Quiet { since: u64 },
    /// The veil is rising; fully white at `white_at`.
    Whiten { white_at: u64 },
    /// White. The way home is posted; waiting for the origin scene and the placement.
    Home { since: u64, landed: Option<u64> },
    /// The veil is coming down; gone at `clear_at`.
    Clear { clear_at: u64 },
    /// The veil is gone; waiting for the game to say the line and the player to close it.
    Say { since: u64, spoken: bool },
}

/// One poll's facts, all read by the worker.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WakeView {
    pub now: u64,
    /// The field with no window open AND no host message box up.
    pub quiet: bool,
    /// The destination scene is loaded AND the player has been stood where the run began
    /// (or there was no tile to stand on).
    pub home: bool,
    /// The game's script window (id 4) is on screen.
    pub script_open: bool,
    /// Was the wake flag raised for this return (a destination that can say the line)?
    pub flag_armed: bool,
    /// The wake flag as it reads now (`None` = unreadable).
    pub flag: Option<u8>,
}

/// What the worker should do about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WakeStep {
    None,
    /// Raise the veil over `ms` (and take every input away while it is up).
    Whiten { ms: u32 },
    /// The screen is white: raise the flag if the destination can say the line, post the
    /// run's `Leave`, and report back with [`WakePhase::home_failed`] if nothing went out.
    GoHome,
    /// Lower the veil over `ms`.
    Clear { ms: u32 },
    /// The game did not say it (or cannot): drop the flag if it is still up and say it on
    /// the host, then release what was held back. The wake is over.
    SayOnHost,
    /// Over: release what was held back (the exit reward's box).
    Done,
}

impl WakePhase {
    pub fn start(now: u64) -> Self {
        WakePhase::Quiet { since: now }
    }

    /// Nothing was posted on [`WakeStep::GoHome`] (no world, no scene table): skip the
    /// wait for a landing that cannot come and bring the veil down now.
    pub fn home_failed(&mut self, now: u64) -> WakeStep {
        *self = WakePhase::Clear { clear_at: now + u64::from(WAKE_CLEAR_MS) };
        WakeStep::Clear { ms: WAKE_CLEAR_MS }
    }

    /// Has the way home gone out (i.e. is the tower already behind the player)?
    pub fn gone_home(&self) -> bool {
        !matches!(self, WakePhase::Quiet { .. } | WakePhase::Whiten { .. })
    }

    /// Is the veil on screen (rising, held or falling)? The UI owns the pixels; this is
    /// the worker's own view of it, for the gates that must not act under it.
    pub fn veiled(&self) -> bool {
        matches!(self, WakePhase::Whiten { .. } | WakePhase::Home { .. } | WakePhase::Clear { .. })
    }
}

/// One step of the wake. Every transition is a pure function of the view and the clock.
pub fn wake_step(phase: &mut WakePhase, v: &WakeView) -> WakeStep {
    match *phase {
        WakePhase::Quiet { since } => {
            if v.quiet || v.now.saturating_sub(since) >= WAKE_QUIET_MAX_MS {
                *phase = WakePhase::Whiten { white_at: v.now + u64::from(WAKE_WHITEN_MS) };
                WakeStep::Whiten { ms: WAKE_WHITEN_MS }
            } else {
                WakeStep::None
            }
        }
        WakePhase::Whiten { white_at } => {
            if v.now >= white_at {
                *phase = WakePhase::Home { since: v.now, landed: None };
                WakeStep::GoHome
            } else {
                WakeStep::None
            }
        }
        WakePhase::Home { since, landed } => {
            let landed = match (landed, v.home) {
                (Some(t), _) => Some(t),
                (None, true) => Some(v.now),
                (None, false) => None,
            };
            let give_up = landed.is_none() && v.now.saturating_sub(since) >= WAKE_ARRIVE_MAX_MS;
            if give_up || landed.is_some_and(|t| v.now.saturating_sub(t) >= WAKE_HOLD_MS) {
                *phase = WakePhase::Clear { clear_at: v.now + u64::from(WAKE_CLEAR_MS) };
                return WakeStep::Clear { ms: WAKE_CLEAR_MS };
            }
            *phase = WakePhase::Home { since, landed };
            WakeStep::None
        }
        WakePhase::Clear { clear_at } => {
            if v.now < clear_at {
                return WakeStep::None;
            }
            if !v.flag_armed {
                // Nothing in the destination can say it: the host does, now.
                return WakeStep::SayOnHost;
            }
            *phase = WakePhase::Say { since: v.now, spoken: v.script_open };
            WakeStep::None
        }
        WakePhase::Say { since, spoken } => {
            let spoken = spoken || v.script_open;
            let waited = v.now.saturating_sub(since);
            if spoken {
                // Said; over once the player has closed it (or has had ample time to).
                if !v.script_open || waited >= WAKE_READ_MAX_MS {
                    return WakeStep::Done;
                }
            } else if waited >= WAKE_SAY_MAX_MS {
                // ⛔ A flag still at 1 means the entry row never ran the line (an asset
                // without it) — say it on the host. At 0 the script DID consume it: the
                // window came and went between polls, and saying it twice is worse.
                return if v.flag == Some(0) { WakeStep::Done } else { WakeStep::SayOnHost };
            }
            *phase = WakePhase::Say { since, spoken };
            WakeStep::None
        }
    }
}

/// The two heroes, as far as the way home cares: each has an entrance town.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hero {
    Ritz,
    Kei,
}

/// ★ 143차: where to put a player whose run has nothing to go back to — a save made inside
/// the tower before 143차 (the old `State::Done` let SAVE come back on a doorless arena),
/// or a run started somewhere its origin could not be read. The hero's entrance town, one
/// step in front of the NPC that sent them (map 60 ENZARK: the passer-by stands on (32,19)
/// with a wall to his east, so (31,19); map 4: 로빈 on (8,8), so (8,9) below him — both
/// walkable, measured off the map attributes). Unknown = 리츠's town, the first entrance.
pub fn fallback_home(hero: Option<Hero>) -> (u8, (u8, u8)) {
    match hero {
        Some(Hero::Kei) => (5, (8, 9)),
        _ => (111, (31, 19)),
    }
}

// ── THE FLOOR TABLE FILE ─────────────────────────────────────────────────────
//
// `wie_guardian_floors.txt`, one floor per line, `#` starts a comment:
//
//     # floor scene lv   hp%  def%  boss  atk%  s0   s1   s2
//     1       247   60   100  100   -     100   146  90   155
//     10      246   61   300  150   B     250   175  -    -
//
// `lv` may be `-` for "leave the monsters' own level alone". `boss` is `B` or `-`.
// `atk%` and the species triple (`-` = an empty slot) are OPTIONAL and came with 88차:
// a six-column row is a legacy row — attack 100 %, the scene's own monsters — so a
// table from an older generator still loads.
// A line that does not parse is skipped and reported, the same contract as the buff
// file: one bad line must not cost the player the rest of the table.

/// Parse one line. `None` = blank, comment, or malformed.
pub fn parse_floor(line: &str) -> Option<Floor> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return None;
    }
    let mut it = line.split_whitespace();
    let floor: u8 = it.next()?.parse().ok()?;
    let scene: u8 = it.next()?.parse().ok()?;
    let level = match it.next()? {
        "-" => None,
        s => Some(s.parse::<i8>().ok()?),
    };
    let hp_pct: u16 = it.next()?.parse().ok()?;
    let def_pct: u16 = it.next()?.parse().ok()?;
    let boss = match it.next() {
        None => false,
        Some(s) => s.eq_ignore_ascii_case("b") || s == "1",
    };
    let atk_pct: u16 = match it.next() {
        None => PCT_NEUTRAL,
        Some(s) => s.parse().ok()?,
    };
    let species = match it.next() {
        None => None,
        Some(s0) => {
            let slot = |s: &str| -> Option<u8> {
                if s == "-" { Some(SPECIES_NONE) } else { s.parse::<u8>().ok() }
            };
            Some([slot(s0)?, slot(it.next()?)?, slot(it.next()?)?])
        }
    };
    // An optional trailing count column (user 2026-09-16). It comes after the species
    // triple so a legacy 7- or 10-column row still parses. `-` = no count (boss rows).
    let count = match it.next() {
        None => None,
        Some("-") => None,
        Some(s) => Some(s.parse::<u8>().ok()?),
    };
    // An optional clear-EXP column (112차), after the count. Legacy rows pay nothing.
    let exp: u32 = match it.next() {
        None => 0,
        Some(s) => s.parse().ok()?,
    };
    // Trailing junk means the line was not what its author thought it was.
    if it.next().is_some() {
        return None;
    }
    let f = Floor { floor, scene, level, hp_pct, def_pct, boss, atk_pct, species, count, exp };
    f.valid().then_some(f)
}

/// Parse a whole table, returning the rows and the 1-based numbers of the lines that
/// were rejected (so the caller can say so in a log instead of failing silently).
pub fn parse_table(text: &str) -> (Vec<Floor>, Vec<usize>) {
    let mut rows = vec![];
    let mut bad = vec![];
    for (i, line) in text.lines().enumerate() {
        let stripped = line.split('#').next().unwrap_or("").trim();
        match parse_floor(line) {
            Some(f) => rows.push(f),
            None if stripped.is_empty() => {}
            None => bad.push(i + 1),
        }
    }
    (rows, bad)
}

/// Render a table back out, so a generator script and this parser cannot disagree
/// about the format.
pub fn write_table(floors: &[Floor]) -> String {
    let mut s = String::from("# floor scene lv hp% def% boss atk% s0 s1 s2 count exp\n");
    for f in floors {
        let lv = f.level.map(|l| l.to_string()).unwrap_or_else(|| "-".into());
        let slot = |v: u8| if v == SPECIES_NONE { "-".to_string() } else { v.to_string() };
        let sp = f.species.map(|sp| format!(" {} {} {}", slot(sp[0]), slot(sp[1]), slot(sp[2]))).unwrap_or_default();
        // The count and the EXP come after the species triple; a row with an EXP but no
        // count (a boss floor) writes `-` so the columns stay in place.
        let tail = match (f.count, f.exp) {
            (None, 0) => String::new(),
            (Some(n), 0) => format!(" {n}"),
            (None, e) => format!(" - {e}"),
            (Some(n), e) => format!(" {n} {e}"),
        };
        s.push_str(&format!(
            "{} {} {} {} {} {} {}{}{}\n",
            f.floor,
            f.scene,
            lv,
            f.hp_pct,
            f.def_pct,
            if f.boss { "B" } else { "-" },
            f.atk_pct,
            sp,
            tail
        ));
    }
    s
}

// ── THE CONTROL FILE AND THE PROGRESS FILE ───────────────────────────────────
//
// The tower has no in-game UI yet, and the reason it can be exercised anyway is the
// escape hatch this repo already uses for the reaver drop, the item grant and the
// warehouse purge: a file dropped in with `adb`, read once and deleted. The
// PROGRESS file is different — it is the player's, so it is written, not consumed.

/// What a control file can ask for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    /// Start a run at this floor.
    ///
    /// `force` skips the ending gate ([`entry_allowed`]) for testing, exactly like
    /// `wie_reaver_test.txt`'s `hard 1` does for the drop — and for the same reason:
    /// otherwise nothing about the tower can be tried without a finished playthrough.
    /// ⛔ It only lives in the control file, which is not in the APK, so a player who
    /// is handed a build has no way to arm it.
    Enter { floor: u8, force: bool },
    /// End the run and go back.
    Leave,
    /// ★ 143차 (testing only): zero the HP of every live monster on the floor, so a clear —
    /// and the top floor's wake — can be driven with `adb` on a device, where a fight
    /// cannot be scripted. The records go through `items::guardian_zero_hp_writes` (which
    /// no player path calls since 145차 — the floor count is a spawn cap now). With no live floor it
    /// ARMS instead, and the first floor to go live within about a minute takes it — push
    /// `clear`, then `floor N`: a weak test character dies a second after landing on the
    /// top floor, faster than the control file's 300-frame read. ⛔ Control file only,
    /// like `floor` / `force`: nothing in the APK can arm it.
    Clear,
}

/// Test-only override of the HARD mirror the host keeps for the entrance dialogue.
///
/// The in-game entrance only exists in hard mode now — the passer-by's condition
/// container reads `items::GUARDIAN_FLAG_HARD`, and on a normal save the default block
/// runs the shipped dialogue byte for byte. That is the shipping rule (user decision
/// 2026-09-03), but 2026-09-09's *"노말에서도 테스트할 수 있게 해줘"* still needs a way in,
/// so `hard 1` in the control file arms the mirror for this process and `hard 0`
/// disarms it.
///
/// `None` = the file said nothing, which arms nothing — the same contract as
/// `config::parse_reaver_test`. ⛔ A shipped APK has no way to set this: the control
/// file lives in the device's external files dir and is not in the archive.
pub fn parse_hard_override(text: &str) -> Option<bool> {
    let mut out = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        if let Some(key) = it.next() {
            if key.eq_ignore_ascii_case("hard") {
                out = Some(!matches!(
                    it.next().map(|v| v.to_ascii_lowercase()).as_deref(),
                    Some("0" | "off" | "false")
                ));
            }
        }
    }
    out
}

/// `0` = never set, `1` = forced on, `2` = forced off. Process-wide, like the reaver
/// test knobs, because the control file is read once and deleted.
static HARD_OVERRIDE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

pub fn set_hard_override(on: bool) {
    HARD_OVERRIDE.store(if on { 1 } else { 2 }, core::sync::atomic::Ordering::Relaxed);
}

/// `None` unless a control file set it.
pub fn hard_override() -> Option<bool> {
    match HARD_OVERRIDE.load(core::sync::atomic::Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Parse a control file. `#` comments, one key per line, last one wins:
/// ```text
/// floor 1       # start at floor 1
/// force 1       # and skip the ending gate (testing only)
/// hard 1        # pretend this character is in hard mode (entrance dialogue, testing)
/// leave         # or: end the run instead
/// clear         # or: zero every live monster on the floor (testing, 143차) — with no
///               #     live floor it arms for the next one; push it BEFORE `floor N`
/// ```
/// `None` when the file asked for nothing — the same "says nothing arms nothing"
/// contract `config::parse_reaver_test` has.
pub fn parse_command(text: &str) -> Option<Command> {
    let mut floor = None;
    let mut force = false;
    let mut leave = false;
    let mut clear = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        let val = it.next();
        match key.to_ascii_lowercase().as_str() {
            "floor" => floor = val.and_then(|v| v.parse::<u8>().ok()).filter(|f| (1..=FLOORS).contains(f)),
            // Anything but an explicit 0 / off / false means yes, like the reaver file.
            "force" => force = !matches!(val.map(|v| v.to_ascii_lowercase()).as_deref(), Some("0" | "off" | "false")),
            "leave" | "exit" | "quit" => leave = true,
            "clear" | "kill" => clear = true,
            _ => {}
        }
    }
    // ⛔ Leaving wins over entering. A file with both is a mistake, and the harmless
    // reading of a mistake is the one that stops the tower rather than starts it.
    if leave {
        return Some(Command::Leave);
    }
    // A clear never rides along with an entry in the same file (the file is read once,
    // and the floor it would clear is not loaded yet) — it is its own push, made first,
    // and arms itself for the floor that entry goes live on (`emulator.rs`).
    if clear {
        return Some(Command::Clear);
    }
    floor.map(|floor| Command::Enter { floor, force })
}

/// ★ 138차 (137차 결정 ⑤, 2026-09-25): ONE SAVE SLOT's tower progress, kept in
/// `<app root>/guardian/slot<N>.txt` (N = the save pin's real slot — `vault::live_slot`),
/// written the moment a boss floor is cleared, whatever the player saves afterwards: it
/// is a record of what that character has done, like the bag boxes beside it, not part of
/// the game's save. A 초기화 or a newly created character in the slot removes it (the
/// same two hooks the boxes use). ⛔ The old device-wide `wie_guardian_progress.txt` (one
/// `best N` for every slot) is ignored — every slot starts at 0 (the user's decision).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Progress {
    /// The highest BOSS floor this slot has ever cleared (0, 10, …, 100). Only rises — a
    /// later death on floor 20 does not take 51 away. The quick start offers from it
    /// ([`entry_floors`]).
    pub best_boss: u8,
    /// The highest floor ever cleared (information only).
    pub best: u8,
    /// How many times floor 100 was cleared (information only).
    pub clears: u16,
}

impl Progress {
    /// Note a cleared floor. `true` when anything changed, i.e. when the file is due.
    pub fn note_clear(&mut self, floor: u8, boss: bool) -> bool {
        if floor == 0 || floor > FLOORS {
            return false;
        }
        let before = *self;
        self.best = self.best.max(floor);
        if boss {
            self.best_boss = self.best_boss.max(floor);
            if floor == FLOORS {
                self.clears = self.clears.saturating_add(1);
            }
        }
        *self != before
    }
}

/// The boss floor a stored number may claim: rounded DOWN to a tenth floor and clamped,
/// because a damaged file must never offer more of the tower than was earned.
fn boss_floor(n: u32) -> u8 {
    let n = n.min(u32::from(FLOORS)) as u8;
    n - n % 10
}

/// Parse a slot's progress file. Anything unreadable is 0 — "you have cleared nothing
/// yet" is the safe reading of a damaged file (it can only offer LESS of the tower).
pub fn parse_progress(text: &str) -> Progress {
    let mut p = Progress::default();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        match (it.next(), it.next().and_then(|v| v.parse::<u32>().ok())) {
            (Some("best_boss"), Some(n)) => p.best_boss = p.best_boss.max(boss_floor(n)),
            (Some("best"), Some(n)) => p.best = p.best.max(n.min(u32::from(FLOORS)) as u8),
            (Some("clears"), Some(n)) => p.clears = p.clears.max(n.min(u32::from(u16::MAX)) as u16),
            _ => {}
        }
    }
    p
}

/// Render it back. Absolute values, one per line, like the buff file.
pub fn write_progress(p: &Progress) -> String {
    format!(
        "# 하늘성 진행도 (세이브 슬롯별)\nbest_boss {}\nbest {}\nclears {}\n",
        boss_floor(u32::from(p.best_boss)),
        p.best.min(FLOORS),
        p.clears
    )
}

/// The floors a slot may start a run on (137차 §2.1): floor 1, then the floor after
/// every boss it has cleared — `[1, 11, 21, …, best_boss + 1]`, ascending, never past 91
/// (floor 101 does not exist, so clearing 100 offers what clearing 90 did). `best_boss`
/// 0 = `[1]`, which the caller takes as "the plain entrance, no picker".
pub fn entry_floors(best_boss: u8) -> Vec<u8> {
    let mut out = vec![1];
    let mut boss = 10u8;
    while boss <= best_boss && boss < FLOORS {
        out.push(boss + 1);
        boss += 10;
    }
    out
}

// ───────────────────────── the exit reward (137차 · 138차) ─────────────────────────
//
// The player's design (2026-09-25): nothing is paid floor by floor any more (the 85차
// payout — ordinary gear to floor 20, shards or a 0.1 % jackpot above — is gone). When a
// run ENDS — a death, a quit, or floor 100 — it pays ONCE, for the highest boss it
// cleared ([`Run::run_best_boss`]); a run that went to 메인메뉴 ([`Abort::LeftTheField`])
// pays nothing (결정 ⑥). One tier per boss floor ([`REWARD_TIERS`]): 닉스코어의 파편 · one
// 조합서 of that tier's recipes · from 60 one 85제 legacy item · from 80 a small chance
// (1 / 3 / 10 %) of one of the 43 endgame rows (the 90제, 등급 >= [`ENDGAME_MIN_LV`] — ★ 150차:
// the TIER column, not the asset's 요구레벨: the reworked accessories require Lv 1 now, and the
// generators carry the 85/90 tier separately).
// No faction filter (결정 ⑨) and no duplicate handling (결정 ⑪) — both pools are drawn
// uniformly.
//
// ⛔ Nothing else in the tower gives anything: `tower_no_drops()` blocks the game's own
// loot spawn for the whole run (the reaver hook's first branch), and the floor clear EXP
// (112차) is the only per-floor payment left.
//
// Pure, like the rest of the file: the POOLS (the generated `guardian_rewards.rs`) arrive
// here as a length plus a `req_lv` accessor (150차: that field holds the 85/90 TIER), so
// `guardian_selftest.rs` can exercise the rules without the item database.

/// One step of the exit reward (`TOWER_REWARD_BALANCE_DRAFT.md §3.2`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RewardTier {
    /// The boss floor this tier is for.
    pub boss: u8,
    /// How many 닉스코어의 파편. Lv97 세트 5부위 = 120 shards = four 100층 runs (결정 ⑧).
    pub shards: u8,
    /// The 조합서 band, by the RESULT's tier (inclusive; 150차 — the 85/90 tier column, no
    /// longer the asset's 요구레벨).
    pub recipe: (u8, u8),
    /// The legacy item band (inclusive), from floor 60.
    pub legacy: Option<(u8, u8)>,
    /// Percent chance of one extra endgame row, from floor 80.
    pub endgame_pct: u8,
}

/// The table, one row per boss floor. ★ 90's 3 % is 결정 ⑰ (a geometric step between 80's
/// 1 % and 100's 10 %, both the user's numbers).
///
/// ★★ 142차 (the 85 / 90제 rework, `LEGACY_ITEMS_REWORK_85_90.md §6.3`): the legacy rows
/// sit in exactly two tiers (85 / 90 — ★ 150차: the tier column, since the reworked
/// accessories' 요구레벨 is 1), so the bands are the two tiers — 조합서 of the 85제 up to
/// floor 60 and of the 90제 from 70, a real 85제 piece from 60, and the endgame chance on
/// the whole 90제 (43 rows). Shards and chances are unchanged.
pub const REWARD_TIERS: [RewardTier; 10] = [
    RewardTier { boss: 10, shards: 3, recipe: (85, 85), legacy: None, endgame_pct: 0 },
    RewardTier { boss: 20, shards: 5, recipe: (85, 85), legacy: None, endgame_pct: 0 },
    RewardTier { boss: 30, shards: 7, recipe: (85, 85), legacy: None, endgame_pct: 0 },
    RewardTier { boss: 40, shards: 9, recipe: (85, 85), legacy: None, endgame_pct: 0 },
    RewardTier { boss: 50, shards: 12, recipe: (85, 85), legacy: None, endgame_pct: 0 },
    RewardTier { boss: 60, shards: 14, recipe: (85, 85), legacy: Some((85, 85)), endgame_pct: 0 },
    RewardTier { boss: 70, shards: 16, recipe: (90, 90), legacy: Some((85, 85)), endgame_pct: 0 },
    RewardTier { boss: 80, shards: 20, recipe: (90, 90), legacy: Some((85, 85)), endgame_pct: 1 },
    RewardTier { boss: 90, shards: 24, recipe: (90, 90), legacy: Some((85, 85)), endgame_pct: 3 },
    RewardTier { boss: 100, shards: 30, recipe: (90, 90), legacy: Some((85, 85)), endgame_pct: 10 },
];

/// 「종결」 = tier at or above this: the 90제 (142차 — 43 rows; was 95 = 31 rows when the
/// ladder ran 73..99), the same set as the hell drop's boss tier. ★ 150차: the TIER column
/// (85/90), not the asset's 요구레벨 — the reworked accessories require Lv 1 now.
pub const ENDGAME_MIN_LV: u8 = 90;

/// Does a run that ended this way pay its exit reward? Everything but 메인메뉴 (결정 ⑥):
/// a death, a quit, a finished tower — and a table or warp failure, which is not the
/// player's doing. ⛔ `LeftTheField` is also the game-over-to-title road; the owed rows
/// would die with the world there anyway (117차 — the shell drops them).
pub fn pays_exit_reward(why: Abort) -> bool {
    !matches!(why, Abort::LeftTheField)
}

/// The tier a run's best boss earns: the highest tier at or below it (a hand-edited
/// table may put a boss on an odd floor), `None` when no boss was cleared.
pub fn reward_tier(best_boss: u8) -> Option<&'static RewardTier> {
    REWARD_TIERS.iter().rev().find(|t| t.boss <= best_boss)
}

/// What one finished run pays. Indices are into the caller's pools.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ExitPayout {
    /// The tier that paid (its boss floor).
    pub boss: u8,
    pub shards: u8,
    /// One 조합서 — `None` when the pool has none in the band (e.g. an asset without the
    /// 138차 recipe rows: fail closed, the shards still pay).
    pub recipe: Option<usize>,
    /// One legacy item (index into the legacy pool), from floor 60.
    pub legacy: Option<usize>,
    /// One endgame row (index into the legacy pool) when the percent roll hit.
    pub endgame: Option<usize>,
}

/// The exit reward for a run whose best boss was `best_boss`. `seed` is mixed four ways,
/// so the three picks and the chance are independent of each other.
pub fn exit_payout(
    best_boss: u8,
    seed: u64,
    n_recipes: usize,
    recipe_lv: impl Fn(usize) -> u8,
    n_legacy: usize,
    legacy_lv: impl Fn(usize) -> u8,
) -> Option<ExitPayout> {
    let tier = reward_tier(best_boss)?;
    let r1 = mix(seed);
    let r2 = mix(r1);
    let r3 = mix(r2);
    let r4 = mix(r3);
    let recipe = pick_band(n_recipes, &recipe_lv, tier.recipe, r1);
    let legacy = tier.legacy.and_then(|band| pick_band(n_legacy, &legacy_lv, band, r2));
    let endgame = (tier.endgame_pct > 0 && r3 % 100 < u64::from(tier.endgame_pct))
        .then(|| pick_band(n_legacy, &legacy_lv, (ENDGAME_MIN_LV, u8::MAX), r4))
        .flatten();
    Some(ExitPayout { boss: tier.boss, shards: tier.shards, recipe, legacy, endgame })
}

/// One index, uniformly, among the rows whose `lv` is in `lo..=hi`; `None` when none is.
pub fn pick_band(n: usize, lv: impl Fn(usize) -> u8, (lo, hi): (u8, u8), roll: u64) -> Option<usize> {
    let eligible: Vec<usize> = (0..n).filter(|&i| (lo..=hi).contains(&lv(i))).collect();
    if eligible.is_empty() {
        return None;
    }
    Some(eligible[(roll % eligible.len() as u64) as usize])
}

/// splitmix64 — the one mixer this file needs, so the rolls are a pure function of a
/// seed the caller can log and replay.
///
/// ⛔ Not a security primitive and not seeded here: the caller mixes in the clock and
/// the frame, because a payout that depends only on the boss floor would give every
/// player of every run the same endgame rolls.
pub fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}
