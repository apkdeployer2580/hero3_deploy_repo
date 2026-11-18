// Standalone self-test of the tower's floor state machine.
//
//   rustc --edition 2021 --test guardian_selftest.rs -o t.exe && ./t.exe
//
// `cargo test -p wie_android` cannot run on the Windows host (the linked
// winit/android deps abort at startup), so this compiles the REAL source — not a
// copy — as its own test crate. `guardian.rs` is dependency-free for exactly that
// reason; if it ever grows a `use crate::...` this file stops building, which is the
// intended alarm.
//
// What is worth testing here is not the arithmetic — it is the five rules the RE
// paid for, listed in `guardian.rs`'s header. Every test below is one of them or one
// of the traps around them.

include!("wie_android/src/guardian.rs");

/// A floor that changes nothing, so a test can name only what it cares about.
fn plain(floor: u8, scene: u8) -> Floor {
    Floor { floor, scene, level: None, hp_pct: 100, def_pct: 100, boss: false, atk_pct: 100, species: None, count: None, exp: 0 }
}

/// A live ordinary record as the probe dumps it: the effective fields ARE the spawn
/// values (bases equal effectives on an ordinary record).
fn crea(index: i32, hp: u16, atk: u16, dp: u16, ds: u16) -> Creature {
    Creature {
        index,
        hp_max: hp,
        hp_cur: hp,
        respawn_timer: RESPAWN_DISARMED,
        atk_eff: atk,
        atk_skill: [0, 0],
        def_phys_eff: dp,
        def_spec_eff: ds,
        species: Some(118),
        vhp: 0,
        level: None,
    }
}

/// `forks` with fresh memory — for the tests where the first sighting is the point.
fn forks1(f: &Floor, cs: &[Creature]) -> Vec<Fork> {
    forks(f, None, cs, &mut FloorMemory::default(), None)
}

/// The observation for "standing on floor `n`'s scene with `alive` monsters up".
fn on(scene: u8, spawned: i32, alive: i32) -> Obs {
    Obs {
        scene: Some(scene as u16),
        spawned,
        alive,
        in_field: true,
        in_world: true,
        player_hp: Some(100),
        player_tile: Some((32, 19)),
    }
}

// ── the entry-point byte ─────────────────────────────────────────────────────

#[test]
fn arrivals_have_a_valid_downward_facing() {
    assert_eq!(WARP_ENTRY, 2);
    assert!((1..=4).contains(&WARP_ENTRY));
}

// ── a floor is a SCENE ───────────────────────────────────────────────────────

/// The warp destination is one byte, so a scene id must fit one — and the field is
/// typed `u8` so that a map id (which can also be small) at least cannot be a `u16`
/// by accident. 244 scenes ship.
#[test]
fn a_floors_scene_fits_the_one_byte_destination() {
    let f = plain(1, 205);
    assert!(u8::try_from(243u32).is_ok());
    assert_eq!(f.scene, 205);
    // Scene 0 is refused: it is what an unparsed / zeroed row looks like, and
    // warping to it would look like a working warp.
    assert!(!plain(1, 0).valid());
}

// ── the level is one signed byte ─────────────────────────────────────────────

/// The spec's "floor 100 = Lv80" is NOT reachable through the level field: the record
/// holds an `i8` and the game clamps its own levels to 70. The curve therefore tops
/// out, and the tests say so out loud so nobody re-derives 80 later.
#[test]
fn the_level_curve_stops_at_the_games_own_ceiling() {
    // 137차: floor 1 = 60, floor 70 = 100 (the user's hell anchor), floor 100 = 120 —
    // two linear pieces, round half up, the generator's own integer formula.
    assert_eq!(Floor::level_for(1), 60);
    assert_eq!(Floor::level_for(70), 100, "floor 70 is a hell Lv 100 monster");
    assert_eq!(Floor::level_for(100), LEVEL_MAX);
    assert_eq!(LEVEL_MAX, 120);
    assert_eq!(Floor::level_for(10), 65);
    assert_eq!(Floor::level_for(30), 77);
    assert_eq!(Floor::level_for(50), 88);
    assert_eq!(Floor::level_for(69), 99);
    assert_eq!(Floor::level_for(71), 101);
    assert_eq!(Floor::level_for(80), 107);
    assert_eq!(Floor::level_for(90), 113);
    assert_eq!(Floor::level_for(99), 119);
    // Monotone, so no floor is easier than the one below it.
    for f in 2..=FLOORS {
        assert!(Floor::level_for(f) >= Floor::level_for(f - 1), "floor {f}");
    }
    // And every value fits the record's signed byte.
    for f in 1..=FLOORS {
        assert!(i8::try_from(Floor::level_for(f)).is_ok());
    }
}

/// A table may not ask for a level the game cannot hold, and a fork clamps anyway —
/// belt and braces, because the table is player-editable.
#[test]
fn a_level_outside_the_games_range_is_refused_and_clamped() {
    let mut f = plain(1, 205);
    f.level = Some(LEVEL_MAX + 1);
    assert!(!f.valid());
    f.level = Some(0);
    assert!(!f.valid());
    f.level = Some(LEVEL_MAX);
    assert!(f.valid());

    // Even if a row got through, the fork clamps.
    let bad = Floor { level: Some(127), ..plain(1, 205) };
    let c = Creature {
        index: 0,
        hp_max: 100,
        hp_cur: 100,
        respawn_timer: RESPAWN_DISARMED,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 10,
        def_spec_eff: 10,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    assert_eq!(forks1(&bad, &[c])[0].level, Some(LEVEL_MAX));
}

// ── the fork is idempotent, and computed from the BASE ───────────────────────

/// The game's own percentage setters do `+=` on a knob, so calling one twice
/// compounds. A host fork must not: applying it, letting the game recompute, and
/// applying it again has to land on the same numbers.
///
/// ⛔⛔ THE FIRST DRAFT OF `guardian.rs` FAILED THIS, and that is why the test is
/// written as a loop rather than as one comparison. It scaled max HP from the LIVE
/// max HP, so a 150% floor walked species 118 up 2564 -> 3846 -> 5769 -> … once per
/// re-assert. The fix is that every forked value is computed from a field the game
/// writes once at spawn and never again (`hp_spawn`, `def_*_base`).
#[test]
fn re_asserting_a_fork_lands_on_the_same_numbers() {
    let f = Floor { hp_pct: 150, def_pct: 200, atk_pct: 300, ..plain(1, 205) };
    // Species 118 아일기버로크, as the probe dumped it.
    let mut c = crea(0, 2564, 398, 69, 65);
    let mut mem = FloorMemory::default();
    let first = forks(&f, None, &[c], &mut mem, None)[0];
    assert_eq!(first.hp_max, 3846);
    assert_eq!(first.atk_eff, 1194);
    assert_eq!(first.def_phys_eff, 138);
    assert_eq!(first.def_spec_eff, 130);

    // Now let the writes land and do it again, ten times over — the record's live
    // fields move (they are ALL this fork's own outputs now), the memory does not.
    for pass in 0..10 {
        c = Creature {
            hp_max: first.hp_max,
            hp_cur: first.hp_cur,
            atk_eff: first.atk_eff,
            def_phys_eff: first.def_phys_eff,
            def_spec_eff: first.def_spec_eff,
            ..c
        };
        let again = forks(&f, None, &[c], &mut mem, None)[0];
        assert_eq!(again, first, "pass {pass} drifted");
    }
}

/// ★★★ THE BASE FIELDS ARE NOT INPUTS (88차). A `bossh_dat` record spawns with ZERO in
/// `hp_spawn` / `def_*_base` / `atk_base` (홀리가디언, headless scene 246) — so a fork
/// computed from those made every boss a 1-HP, 1-defence, 1-attack monster, which is
/// what the player reported. `Creature` no longer even carries them: the fork's base
/// is the first sighting of the EFFECTIVE fields, and this pins that a boss-shaped
/// record (big effectives, whatever the bases were) forks to big numbers.
#[test]
fn a_boss_record_forks_from_its_effective_fields() {
    let f = Floor { hp_pct: 150, def_pct: 200, atk_pct: 300, boss: true, ..plain(10, 246) };
    // 홀리가디언 as dumped: hp 17000, atk_eff 0x441, def_eff 0x8e/0x8e, bases all 0.
    let c = Creature { species: Some(175), ..crea(0, 17000, 0x441, 0x8e, 0x8e) };
    let fk = forks1(&f, &[c])[0];
    assert_eq!(fk.hp_max, 25500);
    assert_eq!(fk.hp_cur, 25500);
    assert_eq!(fk.atk_eff, 0x441 * 3);
    assert_eq!(fk.def_phys_eff, 0x8e * 2);
    assert_eq!(fk.def_spec_eff, 0x8e * 2);
}

/// ★ 160차 T4 (user 2026-10-03: «스킬맞고 원턴킬 맞는건 의도한거야. 그래서 스킬을 피해야해»):
/// a boss carries THREE attack values (`+0x8e` · `+0x90` · `+0x92`) and the forks used to
/// grow only the first — 홀리가디언's skill attacks stayed at their spawn numbers while the
/// plain hit went to 22k. They grow together now, on a known boss only.
#[test]
fn a_bosss_skill_attacks_grow_with_its_first_attack() {
    let f = Floor { atk_pct: 300, boss: true, ..plain(10, 246) };
    // 홀리가디언 as the probe dumped it in scene 231: 1089 · 1089 · 544.
    let boss = Creature { species: Some(175), atk_skill: [0x441, 0x220], ..crea(0, 17000, 0x441, 0x8e, 0x8e) };
    let fk = forks1(&f, &[boss])[0];
    assert_eq!(fk.atk_eff, 0x441 * 3);
    assert_eq!(fk.atk_skill, Some([0x441 * 3, 0x220 * 3]), "the same growth as the first attack");

    // An ordinary record's +0x90 / +0x92 are not attacks: never written, whatever they hold.
    let escort = Creature { species: Some(143), atk_skill: [0x3c, 0x43], ..crea(1, 500, 100, 20, 20) };
    assert_eq!(forks1(&f, &[escort])[0].atk_skill, None);
    // An unreadable species on a boss floor takes the boss ROW (the stronger fork) but
    // still writes no skill field — the write needs to KNOW it is a boss record.
    let unknown = Creature { species: None, atk_skill: [7, 7], ..crea(2, 500, 100, 20, 20) };
    assert_eq!(forks1(&f, &[unknown])[0].atk_skill, None);
    // A boss with no such attack (0) keeps 0 — `grow` would have made it 1.
    let partial = Creature { species: Some(169), atk_skill: [4280, 0], ..crea(3, 24210, 1712, 241, 241) };
    assert_eq!(forks1(&f, &[partial])[0].atk_skill, Some([4280 * 3, 0]));

    // The level growth applies too (floor 100: Lv 67 -> 120), clamped to the record's i16,
    // and a second poll over the raised record plans the same numbers (the base is the
    // first sighting, like every other stat).
    let top = Floor { level: Some(LEVEL_MAX), boss: true, ..plain(100, 246) };
    let spawn = Creature { species: Some(175), level: Some(67), atk_skill: [2410, 1205], ..crea(0, 28182, 2410, 259, 259) };
    let mut mem = FloorMemory::default();
    let first = forks(&top, None, &[spawn], &mut mem, Some(99))[0];
    let [s1, s2] = first.atk_skill.unwrap();
    assert_eq!(s1, first.atk_eff, "the same base grows to the same value");
    assert!(s1 > 20_000 && s2 > 10_000 && s2 < s1, "{s1} / {s2}");
    let raised = Creature { atk_eff: first.atk_eff, atk_skill: [s1, s2], hp_max: first.hp_max, hp_cur: first.hp_cur, level: first.level, ..spawn };
    assert_eq!(forks(&top, None, &[raised], &mut mem, Some(99))[0].atk_skill, Some([s1, s2]));

    // Hell grows them the same way.
    let mut hmem = FloorMemory::default();
    let h = hell_forks(&[spawn], &mut hmem, Some(99))[0];
    let [h1, h2] = h.atk_skill.unwrap();
    assert_eq!(h1, h.atk_eff);
    assert!(h2 > 1205);
    assert_eq!(hell_forks(&[Creature { species: Some(52), ..spawn }], &mut FloorMemory::default(), Some(99))[0].atk_skill, None);
}

/// The memory is what makes it idempotent: once a record has been seen, its later
/// (forked) effectives are NOT re-read as a base, however many times the game or the
/// fork rewrites them.
#[test]
fn the_first_sighting_is_the_base_and_later_sightings_are_not() {
    let f = Floor { hp_pct: 200, ..plain(1, 205) };
    let c = crea(0, 442, 100, 58, 54);
    let mut mem = FloorMemory::default();
    let fk = forks(&f, None, &[c], &mut mem, None)[0];
    assert_eq!(fk.hp_max, 884);
    // The writes landed; a second pass sees 884 in the record and must not double it.
    let seen = Creature { hp_max: fk.hp_max, hp_cur: fk.hp_cur, ..c };
    assert_eq!(forks(&f, None, &[seen], &mut mem, None)[0].hp_max, 884);
    // Fresh memory (a new floor) WOULD take 884 as the base — which is why `Run::enter`
    // and `Run::finish` both clear it.
    assert_eq!(forks1(&f, &[seen])[0].hp_max, 1768);
}

/// A monster standing at full health arrives on the floor at full (scaled) health —
/// the two live HP fields agree, so the bar reads full rather than a sliver.
#[test]
fn an_untouched_monster_arrives_at_full_scaled_health() {
    let f = Floor { hp_pct: 300, ..plain(1, 205) };
    let c = Creature {
        index: 0,
        hp_max: 442,
        hp_cur: 442,
        respawn_timer: RESPAWN_DISARMED,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 58,
        def_spec_eff: 54,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    let fk = forks1(&f, &[c])[0];
    assert_eq!(fk.hp_max, 1326);
    assert_eq!(fk.hp_cur, fk.hp_max, "at full health the two agree");
}

/// A wounded monster stays wounded — a re-assert mid-fight must not heal it — and its
/// current HP may never exceed the new maximum, or the HP bar (`43 * cur / max`)
/// overflows its 43 pixels.
#[test]
fn a_fork_does_not_heal_and_never_exceeds_the_maximum() {
    let f = Floor { hp_pct: 150, ..plain(1, 205) };
    let c = Creature {
        index: 0,
        hp_max: 1000,
        hp_cur: 250,
        respawn_timer: RESPAWN_DISARMED,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 10,
        def_spec_eff: 10,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    let fk = forks1(&f, &[c])[0];
    assert_eq!(fk.hp_max, 1500);
    assert_eq!(fk.hp_cur, 250, "the wound survives the fork");
    assert!(fk.hp_cur <= fk.hp_max);

    // And a monster wounded BELOW what the new maximum would be still cannot end up
    // above it: a floor that scales HP down clamps the current value with it. Through
    // the same memory, so the base is still the 1000 the record spawned with.
    let mut mem = FloorMemory::default();
    let _ = forks(&f, None, &[c], &mut mem, None);
    let wounded = Creature { hp_max: 1500, hp_cur: 1400, ..c };
    let fk = forks(&Floor { hp_pct: 100, ..f }, None, &[wounded], &mut mem, None)[0];
    assert_eq!(fk.hp_max, 1000);
    assert!(fk.hp_cur <= fk.hp_max);
}

/// ⛔ Never zero. A zero maximum divides by zero in the bar, and a zero current HP is
/// exactly what this file reads as "dead" — a fork that produced it would clear the
/// floor it was scaling.
#[test]
fn no_scaled_stat_can_ever_reach_zero() {
    assert_eq!(scale(1, 100), 1);
    assert_eq!(scale(0, 100), 1);
    assert_eq!(scale(1, 100 - 1), 1, "rounding down still cannot reach zero");
    for v in [1u16, 2, 7, 442, 3427, u16::MAX] {
        for pct in [100u16, 101, 150, PCT_MAX] {
            assert!(scale(v, pct) >= 1, "{v} x {pct}%");
        }
    }
}

/// A 16-bit stat and a percentage multiply, so the cap on the percentage is what
/// stops a table typo from wrapping. `PCT_MAX` is refused at the row level and
/// saturates at the arithmetic level.
#[test]
fn an_absurd_percentage_is_refused_and_saturates() {
    assert!(!Floor { hp_pct: PCT_MAX + 1, ..plain(1, 205) }.valid());
    assert!(!Floor { def_pct: PCT_MAX + 1, ..plain(1, 205) }.valid());
    assert!(!Floor { hp_pct: PCT_NEUTRAL - 1, ..plain(1, 205) }.valid(), "below neutral is not a buff");
    assert!(!Floor { atk_pct: PCT_MAX + 1, ..plain(1, 205) }.valid());
    // ★ Saturates at the SIGNED halfword ceiling: `u16::MAX` in an `i16` HP field is
    // -1, i.e. a dead monster — the old ceiling, and how a 28k-HP boss at 400 % died
    // on arrival.
    assert_eq!(scale(u16::MAX, PCT_MAX), I16_MAX, "saturates at i16::MAX, does not wrap");
    assert_eq!(scale(28607, 400), I16_MAX);
}

/// Corpses are left alone: writing HP into one would resurrect it, because this
/// file's own clear test reads the field the fork would write.
#[test]
fn a_dead_monster_is_not_forked() {
    let f = Floor { hp_pct: 200, ..plain(1, 205) };
    let live = Creature {
        index: 0,
        hp_max: 100,
        hp_cur: 100,
        respawn_timer: RESPAWN_DISARMED,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 5,
        def_spec_eff: 5,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    let dead = Creature { index: 1, hp_cur: 0, ..live };
    let out = forks1(&f, &[live, dead]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].index, 0);
    assert!(!dead.alive());
    assert!(live.alive());
}

// ── the clear rule, and the trap in front of it ──────────────────────────────

/// ⛔⛔ The poll right after a warp can see zero monsters alive — the old scene's
/// array, or a freshly loaded one before the spawn loop ran. Clearing on that would
/// finish every floor instantly. So a floor is only clearable once something has
/// been SEEN alive on it.
#[test]
fn a_floor_is_not_cleared_before_a_monster_has_been_seen_alive() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    let field = on(0, 0, 0);
    assert_eq!(run.enter(1, &field), Action::Warp { scene: 205 });

    // Arrive, but the spawn loop has not run: zero alive.
    assert_eq!(run.poll(&on(205, 0, 0)), Action::Fork { floor: 1 });
    assert_eq!(run.state, State::InFloor { floor: 1, saw_alive: false });
    // Any number of such polls must NOT clear the floor.
    for _ in 0..10 {
        assert_eq!(run.poll(&on(205, 0, 0)), Action::Fork { floor: 1 });
    }
    assert!(matches!(run.state, State::InFloor { saw_alive: false, .. }));

    // Now the monsters exist, and only after that does killing them clear it.
    assert_eq!(run.poll(&on(205, 17, 17)), Action::Fork { floor: 1 });
    assert!(matches!(run.state, State::InFloor { saw_alive: true, .. }));
    assert_eq!(run.poll(&on(205, 17, 0)), Action::Reward { floor: 1, boss: false });
    assert_eq!(run.state, State::Cleared { floor: 1 });
    assert_eq!(run.best, 1);
}

/// Arriving is the SCENE word, not the spawn count: a floor is allowed to have no
/// monsters, and the spawn loop runs after the scene id is stored.
#[test]
fn arrival_is_decided_by_the_scene_word_alone() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    // The right count but the wrong scene is not arrival.
    assert_eq!(run.poll(&on(9, 17, 17)), Action::None);
    assert!(matches!(run.state, State::Entering { .. }));
    // The right scene with nothing in it IS.
    assert_eq!(run.poll(&on(205, 0, 0)), Action::Fork { floor: 1 });
}

/// A warp that never lands has to end the run rather than hang it.
#[test]
fn a_warp_that_never_lands_times_out() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    let nowhere = on(9, 0, 0);
    for _ in 0..ENTER_POLLS - 1 {
        assert_eq!(run.poll(&nowhere), Action::None);
    }
    assert_eq!(run.poll(&nowhere), Action::Leave { why: Abort::WarpTimedOut });
    assert_eq!(run.state, State::Failed { floor: 1, why: Abort::WarpTimedOut });
}

/// Leaving the floor by any route the tower did not choose ends the run — otherwise
/// the next fork would scale monsters in someone else's scene.
#[test]
fn wandering_out_of_the_floor_ends_the_run() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    assert_eq!(run.poll(&on(206, 3, 3)), Action::Leave { why: Abort::LeftTheField });
}

/// Leaving the WORLD ends the run — the title, the slot list and the online screens
/// all share app state 3, and a floor cannot survive any of them.
#[test]
fn losing_the_world_ends_the_run() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    let shell = Obs { in_world: false, ..on(205, 17, 17) };
    assert_eq!(run.poll(&shell), Action::Leave { why: Abort::LeftTheField });
}

/// ⛔⛔ BUT OPENING A WINDOW MUST NOT. `in_field_state` is false whenever anything is
/// on screen, so a player who opens their bag on a floor is "not in the field" — and
/// aborting on that would throw them out of the tower for looking at their inventory.
/// The run keeps going, and it keeps re-asserting the fork.
#[test]
fn opening_a_window_does_not_end_the_run() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    let menu = Obs { in_field: false, ..on(205, 17, 17) };
    for _ in 0..20 {
        assert_eq!(run.poll(&menu), Action::Fork { floor: 1 });
    }
    assert!(run.forking());
}

/// A KNOWN zero ends the run.
#[test]
fn the_player_dying_ends_the_run() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    let down = Obs { player_hp: Some(0), ..on(205, 17, 17) };
    assert_eq!(run.poll(&down), Action::Leave { why: Abort::PlayerDown });
}

/// ⛔ An UNREADABLE one does not. `read_stats` refuses in transient states, and two
/// sessions (71차-L, 80차-E) went on exactly that class of false refusal; a run that
/// ended whenever a read hiccupped would be unusable.
#[test]
fn an_unreadable_hp_reading_is_not_a_death() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    let unknown = Obs { player_hp: None, ..on(205, 17, 17) };
    for _ in 0..20 {
        assert_eq!(run.poll(&unknown), Action::Fork { floor: 1 });
    }
}

/// A floor cleared while a window is open waits for the field instead of burning the
/// run on a warp nothing would service.
#[test]
fn advancing_waits_for_the_field() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 1, 1));
    let clear_menu = Obs { in_field: false, ..on(205, 1, 0) };
    assert_eq!(run.poll(&clear_menu), Action::Reward { floor: 1, boss: false });
    for _ in 0..5 {
        assert_eq!(run.next_floor(&clear_menu), Action::None);
        assert_eq!(run.state, State::Cleared { floor: 1 }, "still waiting, not failed");
    }
    assert_eq!(run.next_floor(&on(205, 1, 0)), Action::Warp { scene: 206 });
}

// ── the invariant: nothing happens outside a run ─────────────────────────────

/// ⛔ Outside a run the guest must be byte for byte the shipped game. `Idle` produces
/// no action of any kind, and `forking()` — the single gate the caller asks before
/// writing anything — is false everywhere but on a floor.
#[test]
fn an_idle_run_never_asks_for_anything() {
    let mut run = Run::new(vec![plain(1, 205)]);
    assert_eq!(run.state, State::Idle);
    assert!(!run.forking());
    for obs in [on(205, 17, 17), on(0, 0, 0), Obs { in_field: false, ..on(1, 0, 0) }] {
        assert_eq!(run.poll(&obs), Action::None);
        assert!(!run.forking());
    }
}

#[test]
fn forking_is_only_open_while_standing_on_a_floor() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    assert!(!run.forking(), "not while the warp is still in flight");
    run.poll(&on(205, 17, 17));
    assert!(run.forking());
    run.poll(&on(205, 17, 0));
    assert!(!run.forking(), "not once it is cleared");
    run.leave();
    assert!(!run.forking());
}

/// A run may only start from the field: the two-byte warp is only serviced there.
#[test]
fn a_run_cannot_start_outside_the_field() {
    let mut run = Run::new(vec![plain(1, 205)]);
    let shell = Obs { in_field: false, ..on(1, 0, 0) };
    assert_eq!(run.enter(1, &shell), Action::None);
    assert_eq!(run.state, State::Idle, "and it stays idle");
}

// ── progression ─────────────────────────────────────────────────────────────

#[test]
fn clearing_a_floor_leads_to_the_next_one() {
    let mut run = Run::new(vec![plain(1, 205), Floor { boss: true, ..plain(2, 231) }]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    assert_eq!(run.poll(&on(205, 17, 0)), Action::Reward { floor: 1, boss: false });
    assert_eq!(run.next_floor(&on(205, 17, 0)), Action::Warp { scene: 231 });
    run.poll(&on(231, 1, 1));
    assert_eq!(run.poll(&on(231, 1, 0)), Action::Reward { floor: 2, boss: true });
    assert_eq!(run.best, 2);
}

/// A run that reaches a floor the table has no row for stops cleanly instead of
/// warping to scene 0.
#[test]
fn a_missing_floor_row_stops_the_run() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(205, 17, 17));
    run.poll(&on(205, 17, 0));
    assert_eq!(run.next_floor(&on(205, 17, 0)), Action::Leave { why: Abort::NoSuchFloor });
}

/// Clearing the last floor finishes the tower rather than looking for floor 101.
#[test]
fn clearing_the_top_floor_finishes_the_tower() {
    // ★ 138차: finishing is a LEAVE like any other ending — the old `State::Done`
    // answered nothing, so floor 100 left the player in a sealed room with no way home
    // and dropped the owed rows (`Abort::Completed`).
    let mut run = Run::new(vec![Floor { boss: true, ..plain(FLOORS, 231) }]);
    run.enter(FLOORS, &on(0, 0, 0));
    run.poll(&on(231, 1, 1));
    assert_eq!(run.poll(&on(231, 1, 0)), Action::Reward { floor: FLOORS, boss: true });
    assert_eq!(run.run_best_boss, FLOORS, "the run remembers its best boss");
    assert_eq!(run.next_floor(&on(231, 1, 0)), Action::Leave { why: Abort::Completed });
    assert_eq!(run.state, State::Failed { floor: FLOORS, why: Abort::Completed });
    // It keeps asking until the caller finishes it — the same contract as every Leave.
    assert_eq!(run.poll(&on(231, 1, 0)), Action::Leave { why: Abort::Completed });
    assert_eq!(run.status_line(0).as_deref(), Some("하늘성 100층 보스 완주"));
    run.finish();
    assert_eq!(run.state, State::Idle);
    assert_eq!(run.run_best_boss, 0, "finish forgets the run");
}

/// ★ 138차: the exit reward pays on the best BOSS of THIS run — so a run that starts
/// from idle starts from zero, and an ordinary floor never raises it.
#[test]
fn the_runs_best_boss_counts_bosses_of_this_run_only() {
    let mut run = Run::new(vec![
        Floor { boss: true, ..plain(10, 246) },
        plain(11, 243),
    ]);
    run.run_best_boss = 90; // a stale value from somewhere
    run.enter(10, &on(0, 0, 0));
    assert_eq!(run.run_best_boss, 0, "a run from idle starts with nothing");
    run.poll(&on(246, 1, 1));
    run.poll(&on(246, 1, 0));
    assert_eq!(run.run_best_boss, 10);
    // Advancing is not a new run.
    assert!(matches!(run.next_floor(&on(246, 1, 0)), Action::Warp { .. }));
    assert_eq!(run.run_best_boss, 10, "the next hop keeps the run's best");
    run.poll(&on(243, 3, 3));
    run.poll(&on(243, 3, 0));
    assert_eq!(run.run_best_boss, 10, "an ordinary floor is not a boss");
}

// ── the table file ──────────────────────────────────────────────────────────

#[test]
fn a_table_round_trips_through_its_own_format() {
    let rows = vec![
        Floor { floor: 1, scene: 205, level: Some(60), hp_pct: 110, def_pct: 100, boss: false, atk_pct: 100, species: None, count: None, exp: 0 },
        Floor { floor: 10, scene: 231, level: None, hp_pct: 150, def_pct: 115, boss: true, atk_pct: 250, species: Some([175, SPECIES_NONE, SPECIES_NONE]), count: None, exp: 0 },
        Floor { floor: 11, scene: 247, level: Some(61), hp_pct: 120, def_pct: 100, boss: false, atk_pct: 100, species: Some([146, 90, 155]), count: None, exp: 0 },
    ];
    let (back, bad) = parse_table(&write_table(&rows));
    assert!(bad.is_empty());
    assert_eq!(back, rows);
}

/// 88차's columns are OPTIONAL: a six-column row from the older generator is still a
/// floor (attack 100 %, the scene's own monsters), and the new ones parse the way
/// the generator writes them — `-` for an empty slot.
#[test]
fn the_species_and_attack_columns_are_optional_and_strict() {
    let six = parse_floor("1 205 60 100 100 -").unwrap();
    assert_eq!(six.atk_pct, 100);
    assert_eq!(six.species, None);
    let seven = parse_floor("1 205 60 100 100 - 180").unwrap();
    assert_eq!(seven.atk_pct, 180);
    assert_eq!(seven.species, None);
    let ten = parse_floor("10 246 61 300 150 B 250 175 - -").unwrap();
    assert_eq!(ten.species, Some([175, SPECIES_NONE, SPECIES_NONE]));
    assert_eq!(ten.species_override(), Some((246, [175, SPECIES_NONE, SPECIES_NONE])));
    // Two of three slots is a torn row, not a row; so is a row with nothing live.
    assert!(parse_floor("10 246 61 300 150 B 250 175 -").is_none());
    assert!(parse_floor("10 246 61 300 150 B 250 - - -").is_none());
    assert!(parse_floor("10 246 61 300 150 B 250 175 - - junk").is_none());
    assert!(six.species_override().is_none());
}

/// One damaged line must not cost the player the rest of the table — the buff file's
/// contract — and the caller has to be able to SAY which line, so the numbers come
/// back rather than being swallowed.
#[test]
fn a_damaged_line_costs_only_that_line() {
    let text = "\
# a comment
1 205 60 100 100 -
this is not a floor
2 206 - 120 100 B

3 207 60 100 100 - trailing junk
4 208 60 100 100
";
    let (rows, bad) = parse_table(text);
    assert_eq!(rows.iter().map(|f| f.floor).collect::<Vec<_>>(), vec![1, 2, 4]);
    assert_eq!(bad, vec![3, 6], "the prose line and the one with junk after it");
    assert_eq!(rows[1].level, None);
    assert!(rows[1].boss);
    assert!(!rows[2].boss, "a missing boss column is not a boss");
}

/// Rows the table cannot mean are dropped at construction, so no later code has to
/// re-check them — and a duplicate floor number resolves to one row rather than
/// making the run's behaviour depend on iteration order.
#[test]
fn a_run_drops_rows_it_cannot_use() {
    let run = Run::new(vec![
        plain(1, 205),
        plain(0, 205),                                  // floor 0 does not exist
        plain(FLOORS + 1, 205),                         // nor does 101
        plain(2, 0),                                    // scene 0
        Floor { hp_pct: 0, ..plain(3, 207) },           // below neutral
        plain(1, 209),                                  // duplicate of floor 1
    ]);
    assert_eq!(run.len(), 1);
    assert_eq!(run.floor(1).unwrap().scene, 205, "the first row for a floor wins");
    assert!(run.floor(2).is_none());
    assert!(!run.is_empty());
}

/// An empty table is a run that cannot start, not a panic.
#[test]
fn an_empty_table_is_a_run_that_cannot_start() {
    let mut run = Run::new(vec![]);
    assert!(run.is_empty());
    assert_eq!(run.enter(1, &on(0, 0, 0)), Action::Leave { why: Abort::NoSuchFloor });
    assert_eq!(run.poll(&on(0, 0, 0)), Action::Leave { why: Abort::NoSuchFloor });
}

/// The floor a run reports is the one the UI would print, and 0 when there is none.
#[test]
fn the_current_floor_is_zero_outside_a_run() {
    let mut run = Run::new(vec![plain(7, 205)]);
    assert_eq!(run.current_floor(), 0);
    run.enter(7, &on(0, 0, 0));
    assert_eq!(run.current_floor(), 7);
    run.poll(&on(205, 1, 1));
    assert_eq!(run.current_floor(), 7);
    run.leave();
    assert_eq!(run.current_floor(), 7, "the floor it failed on is still the answer");
}

// ── the entry gate ──────────────────────────────────────────────────────────

/// The gate is the difficulty byte, because the ending IS the 2회차 transition and
/// that transition is what sets it. See `guardian.rs::entry_allowed` for the chain.
///
/// ⛔ FAIL CLOSED: an unreadable byte refuses. The cost of a wrong "yes" is a
/// mid-game character standing in front of level-66 monsters.
///
/// ★ Written against `HARD_ONLY` rather than against a hardcoded expectation, so that
/// flipping that one constant back to `true` cannot leave a test asserting the old
/// behaviour — the mistake 71차-C recorded (a hand-written mirror pinning a bug green).
#[test]
fn the_tower_is_gated_on_the_difficulty_byte_and_fails_closed() {
    // The half that is TRUE EITHER WAY, and the one that actually protects anything:
    // no readable difficulty byte means no readable character, so no run.
    assert!(!entry_allowed(None), "unreadable must refuse, whatever HARD_ONLY says");

    // Hard mode is allowed under both settings. ⛔ Not `== 1`: the game's own six
    // readers of this byte all test `cmp #0`, so any non-zero value means hard mode
    // (78차-C wrote this rule down after tightening it once).
    assert!(entry_allowed(Some(1)), "1 is what the 2회차 transition writes");
    for d in [2u8, 3, 0x81, 0xff] {
        assert!(entry_allowed(Some(d)), "{d} is non-zero, so it is hard mode");
    }

    // And normal mode follows the constant.
    assert_eq!(
        entry_allowed(Some(0)),
        !HARD_ONLY,
        "0 is normal mode; it is allowed exactly while the tower is not hard-only"
    );
}

/// The state the tower ships in. ⛔ A REMINDER, not a rule: when the in-game entrance
/// lands and `HARD_ONLY` goes back to `true`, this test is updated with it. It exists
/// so that flip cannot happen silently.
#[test]
fn the_tower_is_open_on_normal_for_now() {
    assert!(
        !HARD_ONLY,
        "HARD_ONLY was flipped back to true - if that was intended, update this test          and the note on the constant (user decision 2026-09-08 opened it for testing)"
    );
    assert!(entry_allowed(Some(0)), "a normal-mode character can enter while testing");
}

// ── the control file and the progress file ──────────────────────────────────

#[test]
fn a_control_file_that_says_nothing_asks_for_nothing() {
    assert_eq!(parse_command(""), None);
    assert_eq!(parse_command("# just a comment\n\n"), None);
    assert_eq!(parse_command("force 1\n"), None, "force alone is not a request");
}

#[test]
fn a_control_file_enters_a_floor() {
    assert_eq!(parse_command("floor 7\n"), Some(Command::Enter { floor: 7, force: false }));
    assert_eq!(
        parse_command("floor 7\nforce 1\n"),
        Some(Command::Enter { floor: 7, force: true })
    );
    // Explicit off is off, like the reaver file.
    assert_eq!(
        parse_command("floor 7\nforce 0\n"),
        Some(Command::Enter { floor: 7, force: false })
    );
    // Out of range is not a floor.
    assert_eq!(parse_command("floor 0\n"), None);
    assert_eq!(parse_command("floor 101\n"), None);
    assert_eq!(parse_command("floor nonsense\n"), None);
}

/// ⛔ A file that says both must STOP the tower, not start it: of the two readings of
/// a mistake, that is the harmless one.
#[test]
fn leaving_wins_over_entering() {
    assert_eq!(parse_command("floor 3\nleave\n"), Some(Command::Leave));
    assert_eq!(parse_command("leave\nfloor 3\n"), Some(Command::Leave));
    assert_eq!(parse_command("quit\n"), Some(Command::Leave));
}

#[test]
fn progress_round_trips_and_a_damaged_file_reads_as_nothing_cleared() {
    // ★ 138차: a slot's progress is three numbers; the quick start reads `best_boss`.
    let p = Progress { best_boss: 50, best: 57, clears: 2 };
    assert_eq!(parse_progress(&write_progress(&p)), p);
    assert_eq!(parse_progress(""), Progress::default());
    assert_eq!(parse_progress("best_boss\n"), Progress::default());
    assert_eq!(parse_progress("garbage\nbest_boss oops\n"), Progress::default());
    // Clamped to the tower's height, and the highest line wins rather than the last.
    assert_eq!(parse_progress("best 200\n").best, FLOORS);
    assert_eq!(parse_progress("best_boss 90\nbest_boss 30\n").best_boss, 90);
    // ⛔ A damaged boss number can only offer LESS of the tower: rounded down to a boss.
    assert_eq!(parse_progress("best_boss 57\n").best_boss, 50);
    assert_eq!(parse_progress("best_boss 250\n").best_boss, FLOORS);
    // ⛔ The old device-wide file (`best N` only) reads as no boss cleared — 결정 ⑤.
    assert_eq!(parse_progress("# 하늘성 진행도\nbest 42\n").best_boss, 0);
}

/// ★ 138차: the progress only rises, a boss floor is what moves `best_boss`, and floor
/// 100 counts the finished towers.
#[test]
fn a_cleared_floor_raises_the_slots_progress_and_never_lowers_it() {
    let mut p = Progress::default();
    assert!(p.note_clear(7, false));
    assert_eq!(p, Progress { best_boss: 0, best: 7, clears: 0 });
    assert!(p.note_clear(10, true));
    assert_eq!(p.best_boss, 10);
    assert!(p.note_clear(50, true));
    assert!(!p.note_clear(20, true), "a lower boss changes nothing");
    assert!(!p.note_clear(49, false));
    assert_eq!(p.best_boss, 50);
    assert!(p.note_clear(FLOORS, true));
    assert_eq!((p.best_boss, p.clears), (FLOORS, 1));
    assert!(p.note_clear(FLOORS, true), "a second finish is counted");
    assert_eq!(p.clears, 2);
    assert!(!p.note_clear(0, true) && !p.note_clear(FLOORS + 1, true), "out of range");
}

/// ★ 138차: the quick-start floors — 1, then the floor after every boss cleared, and
/// never floor 101.
#[test]
fn the_quick_start_offers_the_floor_after_every_cleared_boss() {
    assert_eq!(entry_floors(0), vec![1], "nothing cleared = the plain entrance");
    assert_eq!(entry_floors(9), vec![1]);
    assert_eq!(entry_floors(10), vec![1, 11]);
    assert_eq!(entry_floors(50), vec![1, 11, 21, 31, 41, 51]);
    assert_eq!(entry_floors(90), vec![1, 11, 21, 31, 41, 51, 61, 71, 81, 91]);
    assert_eq!(entry_floors(FLOORS), entry_floors(90), "clearing 100 offers what 90 did");
}

// ── coming back out ─────────────────────────────────────────────────────────

/// Leaving has to put the player back where they started, so the run remembers the
/// scene it was entered FROM — and only that one: floor 2's "origin" is floor 1, and
/// coming out onto floor 1 is not putting anybody back.
#[test]
fn the_run_remembers_where_the_player_came_from() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    assert_eq!(run.origin(), None);
    run.enter(1, &on(9, 0, 0));
    assert_eq!(run.origin(), Some(9));
    run.poll(&on(205, 1, 1));
    run.poll(&on(205, 1, 0));
    run.next_floor(&on(205, 1, 0));
    assert_eq!(run.origin(), Some(9), "still the town, not floor 1");
}

/// And the TILE inside that scene (2026-09-17, report: "행인 npc앞에서 나타나야하는데
/// 엉뚱한곳에서 나오고있어"). The warp carries no coordinates, so the player came back
/// on the last floor's tile; the run now keeps the tile it was entered from — first hop
/// only, like the scene — and `finish` clears it with the scene.
#[test]
fn the_run_remembers_the_tile_it_was_entered_from() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    assert_eq!(run.origin_tile(), None);
    run.enter(1, &Obs { player_tile: Some((31, 19)), ..on(9, 0, 0) });
    assert_eq!(run.origin_tile(), Some((31, 19)));
    run.poll(&on(205, 1, 1));
    run.poll(&on(205, 1, 0));
    // Floor 2 is entered from floor 1's tile (32,19 in the fixture) — not the origin.
    run.next_floor(&on(205, 1, 0));
    assert_eq!(run.origin_tile(), Some((31, 19)), "still the town tile, not floor 1's");
    run.leave();
    run.finish();
    assert_eq!(run.origin_tile(), None, "gone with the origin scene");
    // ⛔ A tile is meaningless without its scene: no origin, no tile.
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &Obs { player_tile: Some((3, 4)), ..on(205, 0, 0) });
    assert_eq!(run.origin(), None);
    assert_eq!(run.origin_tile(), None);
    // And an unreadable player object at entry leaves the tile unknown, scene kept.
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &Obs { player_tile: None, ..on(9, 0, 0) });
    assert_eq!(run.origin(), Some(9));
    assert_eq!(run.origin_tile(), None);
}

/// A scene the warp cannot express is no origin at all — better to have none and say
/// so than to warp to a truncated destination.
#[test]
fn an_origin_the_warp_cannot_express_is_refused() {
    let mut run = Run::new(vec![plain(1, 205)]);
    let far = Obs { scene: Some(300), ..on(0, 0, 0) };
    run.enter(1, &far);
    assert_eq!(run.origin(), None);

    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &Obs { scene: None, ..on(0, 0, 0) });
    assert_eq!(run.origin(), None);

    // And standing on the floor's own scene is not an origin either.
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(205, 0, 0));
    assert_eq!(run.origin(), None);
}

/// ⛔ `Failed` keeps offering `Leave` until the caller says it is done — a warp takes
/// frames, and an action offered once could be missed. `finish()` is what stops it,
/// and after it the machine is inert again.
#[test]
fn a_failed_run_keeps_asking_to_leave_until_it_is_finished() {
    let mut run = Run::new(vec![plain(1, 205)]);
    run.enter(1, &on(9, 0, 0));
    run.poll(&on(205, 1, 1));
    let down = Obs { player_hp: Some(0), ..on(205, 1, 1) };
    assert_eq!(run.poll(&down), Action::Leave { why: Abort::PlayerDown });
    // Still asking, three polls later.
    for _ in 0..3 {
        assert_eq!(run.poll(&down), Action::Leave { why: Abort::PlayerDown });
    }
    assert_eq!(run.origin(), Some(9), "so the caller can still read where to go");
    run.finish();
    assert_eq!(run.state, State::Idle);
    assert_eq!(run.poll(&down), Action::None);
    assert!(!run.forking());
    assert_eq!(run.origin(), None);
}

/// Clearing a floor keeps offering the reward for the same reason — the worker pays
/// once and then advances; it is not a one-shot edge.
#[test]
fn a_cleared_floor_keeps_offering_its_reward() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    run.enter(1, &on(9, 0, 0));
    run.poll(&on(205, 1, 1));
    let clear = on(205, 1, 0);
    for _ in 0..4 {
        assert_eq!(run.poll(&clear), Action::Reward { floor: 1, boss: false });
    }
    assert_eq!(run.next_floor(&clear), Action::Warp { scene: 206 });
}

/// ⛔⛔ BUT A CLEARED FLOOR IS STILL A FLOOR THE PLAYER IS STANDING ON. The clear-hold
/// and an open window both keep the machine in `Cleared`, and a player who goes
/// 시스템 → 메인메뉴 from there leaves the world with the run still parked on the floor.
/// Before this check (code review 2026-09-11) the next character to reach a field —
/// any slot, any difficulty — was answered with `enter(floor + 1)` and dropped into
/// the tower.
#[test]
fn leaving_the_world_from_a_cleared_floor_ends_the_run() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    run.enter(1, &on(9, 0, 0));
    run.poll(&on(205, 1, 1));
    assert_eq!(run.poll(&on(205, 1, 0)), Action::Reward { floor: 1, boss: false });
    let shell = Obs { in_world: false, in_field: false, ..on(205, 1, 0) };
    assert_eq!(run.poll(&shell), Action::Leave { why: Abort::LeftTheField });
    assert!(matches!(run.state, State::Failed { floor: 1, .. }));
    run.finish();
    // The next field the worker sees belongs to somebody else: nothing may happen.
    assert_eq!(run.next_floor(&on(50, 3, 3)), Action::None);
    assert_eq!(run.poll(&on(50, 3, 3)), Action::None);
    assert_eq!(run.current_floor(), 0);
}

/// The same for death and for being moved off the floor by anything but our warp.
#[test]
fn a_cleared_floor_still_watches_the_player_and_the_scene() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    run.enter(1, &on(9, 0, 0));
    run.poll(&on(205, 1, 1));
    run.poll(&on(205, 1, 0));
    let down = Obs { player_hp: Some(0), ..on(205, 1, 0) };
    assert_eq!(run.poll(&down), Action::Leave { why: Abort::PlayerDown });

    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    run.enter(1, &on(9, 0, 0));
    run.poll(&on(205, 1, 1));
    run.poll(&on(205, 1, 0));
    assert_eq!(run.poll(&on(206, 0, 0)), Action::Leave { why: Abort::LeftTheField });
}

/// …and, exactly as on a live floor, an open window and an unreadable HP are NOT the
/// end of it: the reward keeps being offered and `next_floor` keeps waiting.
#[test]
fn a_cleared_floor_survives_a_window_and_an_unreadable_hp() {
    let mut run = Run::new(vec![plain(1, 205), plain(2, 206)]);
    run.enter(1, &on(9, 0, 0));
    run.poll(&on(205, 1, 1));
    run.poll(&on(205, 1, 0));
    let menu = Obs { in_field: false, player_hp: None, ..on(205, 1, 0) };
    for _ in 0..20 {
        assert_eq!(run.poll(&menu), Action::Reward { floor: 1, boss: false });
        assert_eq!(run.next_floor(&menu), Action::None);
        assert_eq!(run.state, State::Cleared { floor: 1 });
    }
    assert_eq!(run.next_floor(&on(205, 1, 0)), Action::Warp { scene: 206 });
}

// ── the generated table, cross-checked against the Rust parser ──────────────

/// ★★ THE REAL ARTEFACT, not a sample. `tools_re/gen_guardian_floors.py` writes this
/// file and its own `--check` verifies it against its own parser; this test verifies
/// it against the parser the APP uses. Two parsers of one file is exactly how the cash
/// catalogue and the game font are kept from drifting, and a generator that produced a
/// table the app silently dropped rows from would otherwise look fine on both sides.
///
/// (`include_str!` rather than a runtime read, so the test has no working-directory
/// dependency beyond the one the invocation already has.)
/// The eleven `{` red-name species of `enemyh_dat` (`helldrop_table::HELL_NAMED_SPECIES`).
const NAMED: [u8; 11] = [29, 35, 36, 38, 39, 52, 135, 138, 141, 142, 144];

/// 138차 결정 ⑯: the floors that field a 네임드 — odd, 71..=99.
fn named_floor(floor: u8) -> bool {
    (71..=99).contains(&floor) && floor % 2 == 1
}

#[test]
fn the_generated_floor_table_parses_completely() {
    let text = include_str!("wie_guardian_floors.txt");
    let (rows, bad) = parse_table(text);
    assert!(bad.is_empty(), "line(s) {bad:?} of the generated table did not parse");
    assert_eq!(rows.len(), FLOORS as usize, "one row per floor");

    // A run must accept every row — `Run::new` drops what it cannot use, so an equal
    // count is the statement that nothing was dropped.
    let run = Run::new(rows.clone());
    assert_eq!(run.len(), FLOORS as usize);

    // And the properties the generator promises, re-checked here rather than trusted.
    for (i, f) in rows.iter().enumerate() {
        assert_eq!(f.floor as usize, i + 1, "floors are 1..=100 in order");
        assert!(f.valid());
        assert_eq!(f.boss, f.floor % 10 == 0, "every tenth floor is a boss floor");
        assert!(f.level.is_some_and(|lv| (60..=LEVEL_MAX).contains(&lv)));
        // 112차: the level IS the curve — every row carries `Floor::level_for`. 137차:
        // defence and attack stay 100 (the level carries them; attack is already one-shot
        // territory in the 90s), and hp% is 100 except the 71..99 ramp to 150.
        assert_eq!(f.level, Some(Floor::level_for(f.floor)), "floor {}", f.floor);
        assert_eq!((f.def_pct, f.atk_pct), (100, 100), "floor {}: growth only", f.floor);
        if f.boss || f.floor <= 70 {
            assert_eq!(f.hp_pct, 100, "floor {}: no ramp", f.floor);
        } else {
            assert!((101..=150).contains(&f.hp_pct), "floor {}: the 71..99 ramp", f.floor);
        }
        assert!(f.exp > 0, "floor {} pays EXP on clear", f.floor);
    }
    let ramp: Vec<u16> = rows.iter().filter(|f| !f.boss && f.floor > 70).map(|f| f.hp_pct).collect();
    assert!(ramp.windows(2).all(|w| w[1] >= w[0]), "the hp ramp never falls");
    assert_eq!(rows[FLOORS as usize - 2].hp_pct, 150, "floor 99 is the top of the ramp");
    // Monotone difficulty: never easier than the floor below, and every ten-floor
    // group strictly above the one before it — by LEVEL now (112차), which is what
    // the game's own growth turns into HP and attack.
    for w in rows.windows(2) {
        assert!(w[1].level >= w[0].level, "floor {}", w[1].floor);
        assert!(w[1].exp >= w[0].exp || w[0].boss, "floor {}: clear EXP never falls (a boss pays more)", w[1].floor);
    }
    // (~0.6 a floor, so a group boundary may share a level: never lower, and every
    // group STARTS above the one before it.)
    for g in 1..10u8 {
        let lo: Vec<Option<i8>> = rows.iter().filter(|f| (f.floor - 1) / 10 == g - 1).map(|f| f.level).collect();
        let hi: Vec<Option<i8>> = rows.iter().filter(|f| (f.floor - 1) / 10 == g).map(|f| f.level).collect();
        assert!(lo.iter().max() <= hi.iter().min(), "group {g} is below group {}", g - 1);
        assert!(hi.iter().min() > lo.iter().min(), "group {g} does not start above group {}", g - 1);
    }
    // 137차 결정 ①: the count WEIGHS the floor's species against a target total HP (the
    // generator's `--check` re-weighs it on the asset); what this side can hold it to is
    // the range — at least three (a one- or two-monster hall means nothing), at most the
    // map's twenty, and at most five on a named floor (spawn order 0,1,2,0,1 keeps
    // exactly one slot-2 monster, the named one).
    for f in rows.iter().filter(|f| !f.boss) {
        let n = f.count.expect("ordinary count");
        let cap = if named_floor(f.floor) { 5 } else { MAX_MONSTERS };
        assert!((3..=cap).contains(&n), "floor {}: count {n} outside 3..={cap}", f.floor);
    }
    // Boss floors carry no count (never parked) and pay more than an ordinary floor
    // of the same level would.
    assert!(rows.iter().filter(|f| f.boss).all(|f| f.count.is_none()), "a boss floor carries a count");
    for f in rows.iter().filter(|f| f.boss) {
        let lv = f.level.unwrap() as u32;
        assert_eq!(f.exp, lv * 1000, "boss floor {} pays lv x 1000", f.floor);
    }
    for f in rows.iter().filter(|f| !f.boss) {
        let lv = f.level.unwrap() as u32;
        assert_eq!(f.exp, lv * 300, "floor {} pays lv x 300", f.floor);
    }
    assert_eq!(rows[0].level, Some(60), "the tower opens at 60");
    assert_eq!(rows[FLOORS as usize - 1].level, Some(LEVEL_MAX));

    // 88차: every floor names its own monsters, and the player's rules hold on the file
    // itself — a real boss in slot 0 of every boss floor with the boss multipliers, no
    // line-up repeated from the floor below, and (second pass) NO harmless critter
    // anywhere: the nineteen `'` species measured by `gen_tower_rooms.attackless()`.
    const HARMLESS: [u8; 19] = [90, 92, 102, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160];
    let mut prev: Vec<u8> = vec![];
    let mut last_named = None;
    for f in &rows {
        let sp = f.species.expect("every generated row carries a species triple");
        let live: Vec<u8> = sp.iter().copied().filter(|s| *s != SPECIES_NONE).collect();
        assert!(!live.is_empty(), "floor {}", f.floor);
        assert!(live.iter().all(|s| !prev.contains(s)), "floor {} repeats the floor below", f.floor);
        assert!(live.iter().all(|s| !HARMLESS.contains(s)), "floor {}: a box or a critter", f.floor);
        if f.boss {
            assert!(live[0] >= BOSS_BASE, "floor {}: slot 0 is the boss", f.floor);
            // 137차 결정 ③ + 138차 ⑭: 10..70 alone; 80 / 90 / 100 the fixed raid pairs.
            let raid: &[u8] = match f.floor {
                80 => &[173, 134, 140],   // 오르도 + 커브스쿠툼 · 아이언비스트 (the beasts)
                90 => &[168, 34, 12],     // 큐 + 솔티안버서커 · 솔티안워락 (his army)
                100 => &[175, 143, 129],  // 홀리가디언 + 커브아르보르 · 식인꽃 (as shipped)
                _ => &[],
            };
            if raid.is_empty() {
                assert_eq!(live.len(), 1, "floor {}: a boss below 80 stands alone", f.floor);
            } else {
                assert_eq!(live, raid, "floor {}: the raid line-up", f.floor);
            }
        } else {
            assert!(live.iter().all(|s| *s < BOSS_BASE), "floor {}: no boss on an ordinary floor", f.floor);
            assert_eq!(live.len(), 3, "floor {}: an ordinary floor uses all three slots", f.floor);
            // 138차 결정 ⑯: slot 2 of every ODD floor 71..99 is a 네임드, and only there;
            // two neighbouring named floors never share one.
            if named_floor(f.floor) {
                assert!(NAMED.contains(&sp[2]), "floor {}: slot 2 is a 네임드", f.floor);
                assert!(!NAMED.contains(&sp[0]) && !NAMED.contains(&sp[1]), "floor {}: one 네임드", f.floor);
                assert_ne!(last_named, Some(sp[2]), "floor {}: same 네임드 as the named floor below", f.floor);
                last_named = Some(sp[2]);
            } else {
                assert!(live.iter().all(|s| !NAMED.contains(s)), "floor {}: a 네임드 off the named floors", f.floor);
            }
        }
        prev = live;
    }
    assert_eq!(rows[FLOORS as usize - 1].species.map(|s| s[0]), Some(175), "홀리가디언 is the last boss");
}

// ── The canSave restore (2026-09-08, S9 vc279) ───────────────────────────────
//
// These pin the rule that §D.4 got wrong. The claim was "no cleanup is needed, the map
// loader rewrites the byte on every scene load"; the device said otherwise, and one
// tower run left the town it came from refusing every save.

#[test]
fn a_finished_run_gives_the_can_save_byte_back() {
    // Still zero => write the value the tower found before it zeroed it.
    assert_eq!(restore_can_save(1, Some(0)), Restore::Write(1));
    // Whatever the original value was, that is what goes back - not a hardcoded 1.
    assert_eq!(restore_can_save(0x7f, Some(0)), Restore::Write(0x7f));
}

#[test]
fn an_unreadable_area_chain_is_not_a_finished_restore() {
    // ⛔ `None` is "ask again", NOT "done". A scene load reads None for a few frames,
    // and treating that as success is how the byte would stay zero forever.
    assert_eq!(restore_can_save(1, None), Restore::Wait);
}

#[test]
fn a_non_zero_byte_is_left_alone_whatever_its_value() {
    // Done is "saving is allowed again", not "the byte equals what we saved": if the
    // game wrote its own value there, that value is the game's business.
    assert_eq!(restore_can_save(1, Some(1)), Restore::Done);
    assert_eq!(restore_can_save(1, Some(9)), Restore::Done);
    assert_eq!(restore_can_save(0x80, Some(1)), Restore::Done);
}

#[test]
fn the_restore_budget_outlasts_a_scene_load() {
    // The leave warp plus the scene load it triggers is a handful of polls; the budget
    // has to cover that with room to spare, and still be finite so a restore that can
    // never land reports itself instead of spinning for the session.
    assert!(RESTORE_POLLS >= ENTER_POLLS / 4, "too tight to ride out a scene load");
    assert!(RESTORE_POLLS <= 600, "an unbounded retry is a silent stuck state");
}

// ── the overlay status line (2026-09-09) ─────────────────────────────────────
//
// It exists because the in-game entrance shipped mute: the first player through it
// arrived on floor 1, saw the minimap and SAVE button vanish, and reported being
// stuck in a wall (§H). These pin what the line says and, more importantly, when it
// says nothing at all.

fn one_floor_run() -> Run {
    Run::new(vec![
        Floor { level: Some(60), ..plain(1, 134) },
        Floor { level: Some(60), boss: true, ..plain(2, 137) },
    ])
}

#[test]
fn there_is_no_status_line_outside_a_run() {
    let r = one_floor_run();
    assert_eq!(r.status_line(0), None, "Idle must be silent");
    assert_eq!(r.status_line(9), None, "and the monster count cannot conjure one");
}

#[test]
fn the_status_line_names_the_floor_and_what_is_left() {
    let mut r = one_floor_run();
    let obs = Obs { scene: Some(111), spawned: 0, alive: 0, in_field: true, in_world: true, player_hp: Some(10), player_tile: None };
    r.enter(1, &obs);
    // ⛔ NOT a monster count while entering - the scene has not loaded, so the number
    // in hand belongs to wherever the player just came from.
    assert_eq!(r.status_line(7).as_deref(), Some("하늘성 1층 진입"));

    let arrive = Obs { scene: Some(134), alive: 19, spawned: 19, ..obs };
    r.poll(&arrive);
    assert_eq!(r.status_line(19).as_deref(), Some("하늘성 1층 남은 19"));
    // A negative reading (an unreadable array) must not print a minus sign.
    assert_eq!(r.status_line(-3).as_deref(), Some("하늘성 1층 남은 0"));
}

#[test]
fn a_boss_floor_says_so_and_a_cleared_one_stops_counting() {
    let mut r = one_floor_run();
    let obs = Obs { scene: Some(111), spawned: 0, alive: 0, in_field: true, in_world: true, player_hp: Some(10), player_tile: None };
    r.enter(2, &obs);
    assert_eq!(r.status_line(0).as_deref(), Some("하늘성 2층 보스 진입"));
    r.poll(&Obs { scene: Some(137), alive: 1, spawned: 1, ..obs });
    assert_eq!(r.status_line(1).as_deref(), Some("하늘성 2층 보스 남은 1"));
    r.poll(&Obs { scene: Some(137), alive: 0, spawned: 1, ..obs });
    assert_eq!(r.status_line(0).as_deref(), Some("하늘성 2층 보스 클리어"));
}

#[test]
fn leaving_says_so_rather_than_flashing_the_last_count() {
    let mut r = one_floor_run();
    let obs = Obs { scene: Some(111), spawned: 0, alive: 0, in_field: true, in_world: true, player_hp: Some(10), player_tile: None };
    r.enter(1, &obs);
    r.leave();
    assert_eq!(r.status_line(19).as_deref(), Some("하늘성 1층 이탈"));
    r.finish();
    assert_eq!(r.status_line(19), None, "and `finish` puts the line away");
}

// ── 인게임 입장구의 하드 모드 노브 (2026-09-10) ───────────────────────────────
//
// 입장 대화는 하드 전용이다 — 행인의 조건 컨테이너가 `items::GUARDIAN_FLAG_HARD` 를
// 읽고, 노말이면 기본 블록이 **출고본 그대로**를 돌린다. 그래서 노말 세이브로는 그
// 대화에 도달할 수가 없고, 2026-09-09 의 「노말에서도 테스트할 수 있게」가 여전히 통로를
// 요구한다. 그 통로가 제어 파일의 `hard` 한 줄이다.
// ⛔ 기본은 **미설정**이다 — 아무것도 안 적은 파일은 아무것도 무장하지 않는다.

#[test]
fn the_hard_knob_is_unset_unless_the_file_says_so() {
    assert_eq!(parse_hard_override(""), None);
    assert_eq!(parse_hard_override("floor 1\nforce 1\n"), None, "a run request arms no knob");
    assert_eq!(parse_hard_override("# hard 1\n"), None, "a comment is not a setting");
    assert_eq!(parse_hard_override("hardly 1\n"), None, "and the key has to be the whole word");
}

#[test]
fn the_hard_knob_reads_like_the_reaver_file() {
    assert_eq!(parse_hard_override("hard 1"), Some(true));
    assert_eq!(parse_hard_override("hard"), Some(true), "a bare key means yes");
    assert_eq!(parse_hard_override("HARD 1"), Some(true), "case-insensitive, like `force`");
    for off in ["hard 0", "hard off", "hard FALSE"] {
        assert_eq!(parse_hard_override(off), Some(false), "{off} must disarm");
    }
}

#[test]
fn the_last_hard_line_wins_and_it_coexists_with_a_run_request() {
    assert_eq!(parse_hard_override("hard 1\nhard 0\n"), Some(false));
    let text = "hard 1\nfloor 3\nforce 1\n";
    assert_eq!(parse_hard_override(text), Some(true));
    assert_eq!(
        parse_command(text),
        Some(Command::Enter { floor: 3, force: true }),
        "the knob must not eat the command in the same file"
    );
    // And `hard` on its own is a legitimate file that asks for no run at all.
    assert_eq!(parse_command("hard 1\n"), None);
}

#[test]
fn leaving_still_beats_everything_in_a_file_that_also_sets_the_knob() {
    let text = "hard 1\nfloor 2\nleave\n";
    assert_eq!(parse_hard_override(text), Some(true));
    assert_eq!(parse_command(text), Some(Command::Leave), "⛔ leaving wins over entering");
}

// ── the respawn timer: a floor is not clearable while the game keeps re-arming ──

/// `0xFFFF` is the disarmed value the game leaves on every LIVE monster, so a settled
/// floor must produce no writes at all.
#[test]
fn a_settled_floor_cancels_nothing() {
    let live = |i: i32| Creature {
        index: i,
        hp_max: 100,
        hp_cur: 100,
        respawn_timer: RESPAWN_DISARMED,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 10,
        def_spec_eff: 10,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    assert!(respawn_disarms(&[live(0), live(1), live(2)]).is_empty());
}

/// A corpse whose timer has started is the whole point: it names that record.
///
/// ⛔ The measured behaviour is that the game RE-ARMS it (a single write does not
/// hold), so this is re-asserted every poll — the test states the values the device
/// and the probe actually showed, including a timer of 0.
#[test]
fn every_armed_timer_is_named_however_far_along_it_is() {
    let c = |i: i32, hp: u16, t: u16| Creature {
        index: i,
        hp_max: 100,
        hp_cur: hp,
        respawn_timer: t,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 10,
        def_spec_eff: 10,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    // 0 = just armed, 0x0129 = the value the probe read one dump before the respawn.
    let got = respawn_disarms(&[
        c(0, 100, RESPAWN_DISARMED), // untouched
        c(1, 0, 0),                  // dead, timer just started
        c(2, 0, 0x0129),             // dead, about to come back
        // ★ 88차: a LIVE record with this word at 0 is left alone. 홀리가디언 spawns
        // with `+0x74 == 0` and keeps it while alive (headless, scene 246), so the
        // word is not "armed" on a boss and writing 0xffff into it is a guess.
        c(3, 50, 4),
        c(4, 17000, 0),              // the boss shape exactly
    ]);
    assert_eq!(got, vec![1, 2]);
}

/// ⛔ The disarm must NOT be folded into `forks`. `forks` filters to the living
/// (forking a corpse would resurrect it), and the records that need cancelling are
/// exactly the dead ones — so a floor of corpses would produce zero disarms.
#[test]
fn the_disarm_covers_the_dead_which_the_fork_list_never_does() {
    let dead = |i: i32| Creature {
        index: i,
        hp_max: 100,
        hp_cur: 0,
        respawn_timer: 7,
        atk_eff: 100,
        atk_skill: [0, 0],
        def_phys_eff: 10,
        def_spec_eff: 10,
        species: Some(118),
        vhp: 0,
        level: None,
    };
    let all = [dead(0), dead(1)];
    assert!(forks1(&plain(1, 205), &all).is_empty(), "corpses must not be forked");
    assert_eq!(respawn_disarms(&all), vec![0, 1], "but they must be disarmed");
}

// ── 착지 타일 (2026-09-11) ───────────────────────────────────────────────────
//
// The bug these pin: a host-posted warp carries no destination coordinates, so the
// player keeps the tile they stood on in the previous map. Device reading that
// started this (S25, vc289, floor 1 = scene 212, a 20x20 arena):
//
//     player at 0x4a85ad84: px=(480,320) tile=(30,20) map=20x20

/// An open field with a one-tile wall all the way round, which is the shape of every
/// closed arena the floor table uses.
fn walled(w: i32, h: i32) -> Vec<u8> {
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 { TILE_BLOCKED } else { 0 }
        })
        .collect()
}

/// The reported case, exactly: outside the map is not a tile at all, and the fix is a
/// tile inside it.
#[test]
fn a_player_who_landed_off_the_map_is_moved_onto_it() {
    let a = walled(20, 20);
    let got = landing_tile(20, 20, &a, (30, 20)).expect("off the map must be corrected");
    assert_eq!(got, (10, 10), "the centre of an open arena is the landing tile");
}

/// ⛔ The steady state must be NO move. This is what makes the check free to run on
/// every poll, and it is also what stops it fighting a player who is walking around.
#[test]
fn a_player_standing_somewhere_legal_is_left_alone() {
    let a = walled(20, 20);
    for tile in [(1, 1), (10, 10), (18, 18), (5, 12)] {
        assert_eq!(landing_tile(20, 20, &a, tile), None, "{tile:?} is walkable");
    }
}

/// Inside the map but WALLED IN is the same failure from the player's side: they cannot
/// move and the game does not draw them. Entering from a town tile that happens to be
/// in range is how this one arrives.
#[test]
fn a_player_who_landed_walled_in_is_moved_too() {
    let a = walled(20, 20);
    // Corners of the border ring: every neighbour is wall too.
    assert_eq!(landing_tile(20, 20, &a, (0, 0)), Some((10, 10)));
    assert_eq!(landing_tile(20, 20, &a, (19, 19)), Some((10, 10)));
}

/// ⛔ …but a blocked tile the player can still WALK OFF is left alone. The attribute
/// array and the game do not always agree about where a player may stand, and a device
/// run on 2026-09-11 teleported someone who was standing quite happily at (21,16).
#[test]
fn a_blocked_tile_with_a_way_out_is_left_alone() {
    let a = walled(20, 20);
    // A border tile that is not a corner: its inward neighbour is open.
    assert_eq!(landing_tile(20, 20, &a, (0, 10)), None);
    assert_eq!(landing_tile(20, 20, &a, (10, 19)), None);
}

/// The search is a ring walk outwards from the centre, so a blocked centre yields the
/// nearest open tile rather than nothing.
#[test]
fn a_blocked_centre_falls_back_to_the_nearest_open_tile() {
    let mut a = walled(20, 20);
    // Wall off the middle 3x3.
    for y in 9..=11 {
        for x in 9..=11 {
            a[(y * 20 + x) as usize] = TILE_BLOCKED;
        }
    }
    let got = landing_tile(20, 20, &a, (30, 20)).expect("there is still room");
    let (x, y) = (i32::from(got.0), i32::from(got.1));
    assert_eq!(a[(y * 20 + x) as usize] & TILE_BLOCKED, 0, "the answer must be walkable");
    assert!((x - 10).abs() <= 2 && (y - 10).abs() <= 2, "and next to the middle, got {got:?}");
}

/// ⛔ REFUSE rather than guess. A wrong landing tile teleports a player who was fine.
#[test]
fn an_unreadable_map_moves_nobody() {
    let a = walled(20, 20);
    assert_eq!(landing_tile(20, 20, &a[..399], (30, 20)), None, "short attribute array");
    assert_eq!(landing_tile(0, 20, &a, (30, 20)), None, "zero width");
    assert_eq!(landing_tile(20, 0, &a, (30, 20)), None, "zero height");
    assert_eq!(landing_tile(201, 20, &a, (30, 20)), None, "not a Hero3 map");
}

/// A map with nowhere to stand must produce nothing rather than an arbitrary tile.
#[test]
fn a_map_with_no_open_tile_produces_no_move() {
    let a = vec![TILE_BLOCKED; 400];
    assert_eq!(landing_tile(20, 20, &a, (30, 20)), None);
}

// ───────────────────── the exit reward (137차 · 138차 player design) ─────────────────

/// The legacy pool as the generator lays it out (req levels of the 78 `"` rows) — since
/// the 142차 rework exactly two tiers: 85제 × 35 · 90제 × 43.
fn legacy_levels() -> Vec<u8> {
    let mut v = vec![];
    for (lv, n) in [(85u8, 35), (90, 43)] {
        v.extend(std::iter::repeat(lv).take(n));
    }
    v
}

/// ★ THE TABLE ITSELF (§3.2): a tier per boss floor, shards rising to 30, the endgame
/// chance only from 80 and 1 / 3 / 10 % (결정 ⑰ for 90).
#[test]
fn the_reward_table_is_one_tier_per_boss_floor() {
    let bosses: Vec<u8> = REWARD_TIERS.iter().map(|t| t.boss).collect();
    assert_eq!(bosses, (1..=10).map(|k| k * 10).collect::<Vec<u8>>());
    let shards: Vec<u8> = REWARD_TIERS.iter().map(|t| t.shards).collect();
    assert_eq!(shards, vec![3, 5, 7, 9, 12, 14, 16, 20, 24, 30]);
    let pct: Vec<u8> = REWARD_TIERS.iter().map(|t| t.endgame_pct).collect();
    assert_eq!(pct, vec![0, 0, 0, 0, 0, 0, 0, 1, 3, 10]);
    assert!(REWARD_TIERS.iter().all(|t| (t.legacy.is_some()) == (t.boss >= 60)), "real items from 60");
    // 142차: the two tiers are the bands — 85제 recipes to 60, 90제 recipes from 70, the real
    // item always 85제 (the 90제 comes through the endgame chance).
    for t in &REWARD_TIERS {
        assert_eq!(t.recipe, if t.boss >= 70 { (90, 90) } else { (85, 85) }, "boss {}", t.boss);
        assert!(t.legacy.is_none_or(|b| b == (85, 85)), "boss {}", t.boss);
    }
}

/// No boss cleared = nothing; a boss between tiers pays the tier below (a hand table).
#[test]
fn the_tier_is_the_best_boss_or_the_one_below() {
    assert!(reward_tier(0).is_none());
    assert!(reward_tier(9).is_none());
    assert_eq!(reward_tier(10).map(|t| t.boss), Some(10));
    assert_eq!(reward_tier(57).map(|t| t.boss), Some(50));
    assert_eq!(reward_tier(FLOORS).map(|t| t.boss), Some(100));
    assert!(exit_payout(0, 1, 0, |_| 0, 0, |_| 0).is_none(), "no boss, no reward");
}

/// ⛔ 메인메뉴 (결정 ⑥) is the one ending that does not pay.
#[test]
fn every_ending_but_the_main_menu_pays() {
    assert!(pays_exit_reward(Abort::PlayerDown));
    assert!(pays_exit_reward(Abort::Quit));
    assert!(pays_exit_reward(Abort::Completed));
    assert!(pays_exit_reward(Abort::WarpTimedOut));
    assert!(!pays_exit_reward(Abort::LeftTheField));
}

/// The picks stay in their bands, the band rows are all reachable, and an empty recipe
/// pool (an asset without the 138차 rows) still pays the shards and the real item.
#[test]
fn the_exit_picks_stay_in_their_bands() {
    let legacy = legacy_levels();
    assert_eq!(legacy.len(), 78);
    // Pretend recipe pool: one recipe per result tier.
    let recipes = [85u8, 90];
    for seed in 0..500u64 {
        let p = exit_payout(60, seed, recipes.len(), |i| recipes[i], legacy.len(), |i| legacy[i]).unwrap();
        assert_eq!((p.boss, p.shards), (60, 14));
        assert_eq!(p.recipe.map(|i| recipes[i]), Some(85), "60's recipes are the 85제");
        let lv = legacy[p.legacy.expect("60 pays a real item")];
        assert_eq!(lv, 85, "seed {seed}: Lv{lv}");
        assert!(p.endgame.is_none(), "no endgame below 80");
        let p = exit_payout(50, seed, recipes.len(), |i| recipes[i], legacy.len(), |i| legacy[i]).unwrap();
        assert!(p.legacy.is_none(), "no real item below 60");
        assert_eq!(p.recipe.map(|i| recipes[i]), Some(85));
        let p = exit_payout(70, seed, recipes.len(), |i| recipes[i], legacy.len(), |i| legacy[i]).unwrap();
        assert_eq!(p.recipe.map(|i| recipes[i]), Some(90), "70's recipes are the 90제");
    }
    let seen: std::collections::BTreeSet<usize> = (0..4000u64)
        .filter_map(|s| exit_payout(70, s, 0, |_| 0, legacy.len(), |i| legacy[i]).unwrap().legacy)
        .collect();
    assert_eq!(seen.len(), 35, "all thirty-five 85제 rows are reachable");
    // An empty recipe pool is `None` for the recipe and nothing else.
    let p = exit_payout(100, 7, 0, |_| 0, legacy.len(), |i| legacy[i]).unwrap();
    assert_eq!((p.shards, p.recipe), (30, None));
    assert!(p.legacy.is_some());
}

/// The endgame chance fires about as often as the table says, and only lands on the 43
/// 90제 rows.
#[test]
fn the_endgame_chance_matches_the_table() {
    let legacy = legacy_levels();
    for (boss, pct) in [(80u8, 1u32), (90, 3), (100, 10)] {
        let mut hits = 0u32;
        for seed in 0..20_000u64 {
            let p = exit_payout(boss, seed, 0, |_| 0, legacy.len(), |i| legacy[i]).unwrap();
            if let Some(i) = p.endgame {
                assert!(legacy[i] >= ENDGAME_MIN_LV);
                hits += 1;
            }
        }
        let want = 20_000 * pct / 100;
        assert!(hits * 10 > want * 7 && hits * 10 < want * 13, "boss {boss}: {hits} hits for ~{want}");
    }
    assert_eq!(legacy.iter().filter(|lv| **lv >= ENDGAME_MIN_LV).count(), 43, "142차: the 90제");
}

/// ⛔ An empty pool is `None` rather than an index into nothing.
#[test]
fn an_empty_band_answers_none() {
    assert_eq!(pick_band(0, |_| 0u8, (73, 75), 0), None);
    assert_eq!(pick_band(3, |_| 50u8, (73, 75), 0), None);
}

/// The mixer has to actually mix: neighbouring seeds must not give neighbouring
/// rolls, or two runs a frame apart would share an endgame verdict.
#[test]
fn the_mixer_spreads_neighbouring_seeds() {
    let rolls: Vec<u16> = (0..64).map(|s| (mix(s) % 1000) as u16).collect();
    let distinct: std::collections::BTreeSet<u16> = rolls.iter().copied().collect();
    assert!(distinct.len() > 55, "only {} distinct rolls out of 64", distinct.len());
    assert_ne!(mix(0), 0);
}

// ─────────── the wedge that shipped in vc293..vc295 (device report 2026-09-12) ───────────

/// The REAL terrain, transcribed from `res/map/map7_mp` (scene 9 = floor 2,
/// `NEMESIS_FOREST_2` 45x35) around the tile the player carried in from the town.
/// `#` = blocked. Rows are y 17..23, columns x 27..36.
const MAP7_WEDGE: [&str; 7] = [
    "##########",
    "#.########",
    "######.###",
    "##.##.####",   // y20: the player stood in col 31, and col 32 is a 1-tile pocket
    "##########",
    "...######.",
    "..........",
];

/// Build a 45x35 attribute array that is blocked everywhere except the transcribed
/// window, so the test is about the window and nothing else.
fn map7_like() -> Vec<u8> {
    let (w, h) = (45i32, 35i32);
    let mut a = vec![TILE_BLOCKED; (w * h) as usize];
    for (row, line) in MAP7_WEDGE.iter().enumerate() {
        let y = 17 + row as i32;
        for (col, c) in line.chars().enumerate() {
            let x = 27 + col as i32;
            if c == '.' {
                a[(y * w + x) as usize] = 0;
            }
        }
    }
    a
}

/// ⛔⛔ THE BUG. One open neighbour is not a way out when that neighbour is a one-tile
/// pocket. vc293..vc295 answered `None` here and the player could not move at all.
#[test]
fn a_player_wedged_next_to_a_one_tile_pocket_is_moved() {
    let a = map7_like();
    // Sanity: this is the measured shape — the tile is wall, and exactly one neighbour
    // is open, which is why the old neighbour test passed it.
    let blocked = |x: i32, y: i32| a[(y * 45 + x) as usize] & TILE_BLOCKED != 0;
    assert!(blocked(31, 20), "the player's tile is wall");
    assert!(!blocked(32, 20), "and it does have an open neighbour");
    assert_eq!(
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .filter(|(dx, dy)| !blocked(31 + dx, 20 + dy))
            .count(),
        1,
        "exactly one — that is what fooled the old rule"
    );
    assert!(landing_tile(45, 35, &a, (31, 20)).is_some(),
            "a wedge with a dead-end neighbour must be corrected");
}

/// And the pocket itself is no better a place to stand, even though it IS open — which
/// the very first version of this check (`!blocked(cur) => None`) would have allowed.
#[test]
fn standing_inside_the_pocket_is_also_a_wedge() {
    let a = map7_like();
    assert!(landing_tile(45, 35, &a, (32, 20)).is_some(),
            "an open tile you cannot walk out of is still a wedge");
}

/// ⛔ The destination must not be another pocket. Here the map centre is open but
/// isolated, and the only real room is elsewhere.
#[test]
fn the_landing_tile_is_never_itself_a_pocket() {
    let (w, h) = (20i32, 20i32);
    let mut a = vec![TILE_BLOCKED; (w * h) as usize];
    a[(10 * w + 10) as usize] = 0; // a lone open tile exactly at the centre
    // …and a real room in the corner.
    for y in 2..7 {
        for x in 2..7 {
            a[(y * w + x) as usize] = 0;
        }
    }
    let got = landing_tile(w, h, &a, (30, 20)).expect("there is a room to land in");
    assert_ne!(got, (10, 10), "the lone centre tile is a pocket, not a landing spot");
    let blocked = |x: i32, y: i32| a[(y * w + x) as usize] & TILE_BLOCKED != 0;
    assert!(!blocked(i32::from(got.0), i32::from(got.1)));
    assert!(room_at_least(w, h, &blocked, (i32::from(got.0), i32::from(got.1)), LANDING_MIN_ROOM)
                >= LANDING_MIN_ROOM,
            "and it is somewhere you can walk");
}

/// The threshold has to sit well clear of both sides of the real decision (1 vs 668 on
/// the reported map), so that neither a wedge nor a floor is a near miss.
#[test]
fn the_room_threshold_is_not_a_near_miss() {
    let a = map7_like();
    let blocked = |x: i32, y: i32| a[(y * 45 + x) as usize] & TILE_BLOCKED != 0;
    assert_eq!(room_at_least(45, 35, &blocked, (31, 20), 1000), 1,
               "the wedge reaches exactly one tile");
    // An open arena reaches far more than the threshold.
    let open = walled(20, 20);
    let ob = |x: i32, y: i32| open[(y * 20 + x) as usize] & TILE_BLOCKED != 0;
    assert!(room_at_least(20, 20, &ob, (10, 10), 1000) > 300);
    assert!(LANDING_MIN_ROOM > 1 && LANDING_MIN_ROOM < 100);
}

/// ⛔ REGRESSION: the 2026-09-11 finding still has to hold — a blocked tile you can
/// actually walk off is left alone. The flood fill is what keeps that true.
#[test]
fn the_2026_09_11_finding_still_holds() {
    let a = walled(20, 20);
    for tile in [(0, 10), (10, 19), (19, 5)] {
        assert_eq!(landing_tile(20, 20, &a, tile), None,
                   "{tile:?} is wall but opens onto the whole arena");
    }
}

// ── the virtual HP word (88차 → 2026-09-18): more HP than an i16 can hold ──────

/// A floor that asks for more HP than the record can hold caps the record at
/// `I16_MAX` and asks for the total to be SEEDED into the virtual word; the fork's
/// OUTPUT is still a full bar.
#[test]
fn hp_beyond_the_record_ceiling_is_seeded_into_the_virtual_word() {
    let f = Floor { hp_pct: 400, boss: true, ..plain(10, 246) };
    let c = Creature { species: Some(175), ..crea(0, 28000, 1000, 100, 100) };
    let mut mem = FloorMemory::default();
    let fk = forks(&f, None, &[c], &mut mem, None)[0];
    assert_eq!(fk.hp_max, I16_MAX);
    assert_eq!(fk.hp_cur, I16_MAX);
    assert_eq!(fk.vhp, Some(112_000), "the whole total, record included");
    let b = mem.bases[0];
    assert_eq!(b.hp0, 28000);
    assert_eq!(b.vhp_total, Some(112_000));
    // A floor that fits needs no virtual word at all.
    let mut mem2 = FloorMemory::default();
    let fk2 = forks(&Floor { hp_pct: 100, ..f }, None, &[c], &mut mem2, None)[0];
    assert_eq!(mem2.bases[0].vhp_total, None);
    assert_eq!(fk2.vhp, None);
}

/// ⛔ The seed lands ONCE. After it the hook owns the word — every hit lowers it —
/// and a poll that wrote the total back would heal the monster by everything dealt.
#[test]
fn the_virtual_word_is_seeded_only_while_it_reads_zero() {
    assert_eq!(seed_vhp(Some(112_000), 0), Some(112_000));
    assert_eq!(seed_vhp(Some(112_000), 112_000), None, "already seeded");
    assert_eq!(seed_vhp(Some(112_000), 5_000), None, "the hook has drained it");
    assert_eq!(seed_vhp(Some(112_000), u32::MAX), None, "unreadable reads as seeded");
    assert_eq!(seed_vhp(None, 0), None, "no total, no seed");
    // Through `forks`: the poll after the seed sees the word non-zero and asks nothing.
    let f = Floor { hp_pct: 400, boss: true, ..plain(10, 246) };
    let c = Creature { species: Some(175), ..crea(0, 28000, 1000, 100, 100) };
    let mut mem = FloorMemory::default();
    let first = forks(&f, None, &[c], &mut mem, None)[0];
    assert_eq!(first.vhp, Some(112_000));
    let seeded = Creature { hp_max: first.hp_max, hp_cur: first.hp_cur, vhp: 112_000, ..c };
    assert_eq!(forks(&f, None, &[seeded], &mut mem, None)[0].vhp, None);
}

/// The fork's `hp_cur` never fights the hook: a record the hook has drained keeps
/// its value, a full one stays at the ceiling — zero writes either way.
#[test]
fn a_drained_virtual_record_keeps_the_hooks_hp() {
    let f = Floor { hp_pct: 400, boss: true, ..plain(10, 246) };
    let c = Creature { species: Some(175), ..crea(0, 28000, 1000, 100, 100) };
    let mut mem = FloorMemory::default();
    let first = forks(&f, None, &[c], &mut mem, None)[0];
    // The hook took the total down to 22 000 and shows it in the record.
    let drained = Creature { hp_max: I16_MAX, hp_cur: 22_000, vhp: 22_000, ..c };
    let fk = forks(&f, None, &[drained], &mut mem, None)[0];
    assert_eq!(fk.hp_cur, 22_000, "the wound the hook left stands");
    assert_eq!(fk.hp_max, I16_MAX);
    assert_eq!(fk.vhp, None);
    // And a corpse is not even looked at.
    let dead = Creature { hp_cur: 0, vhp: 0, ..drained };
    assert!(forks(&f, None, &[dead], &mut mem, None).is_empty());
    let _ = first;
}

/// The seeded record survives re-asserts: ten polls with no damage change nothing.
#[test]
fn an_undamaged_seeded_record_is_stable_across_polls() {
    let f = Floor { hp_pct: 400, boss: true, ..plain(10, 246) };
    let c = Creature { species: Some(175), ..crea(0, 28000, 1000, 100, 100) };
    let mut mem = FloorMemory::default();
    let first = forks(&f, None, &[c], &mut mem, None)[0];
    let seen = Creature { hp_max: first.hp_max, hp_cur: first.hp_cur, vhp: 112_000, ..c };
    let settled = Fork { vhp: None, ..first };
    for _ in 0..10 {
        assert_eq!(forks(&f, None, &[seen], &mut mem, None)[0], settled);
    }
    assert_eq!(mem.bases[0].vhp_total, Some(112_000));
}

// ── boss floors: the row is the boss's, the escorts get the floor below ──────

/// On a boss floor the row's multipliers land on the BOSS (species >= 161) and the
/// adds are forked with the previous floor's row — "boss HP x12" must not also make
/// four escorts x12. An unreadable species is treated as the boss (the harder fork).
#[test]
fn a_boss_floor_forks_the_boss_with_its_row_and_the_adds_with_the_floor_below() {
    let below = Floor { hp_pct: 130, def_pct: 110, atk_pct: 100, ..plain(9, 249) };
    let row = Floor { hp_pct: 300, def_pct: 200, atk_pct: 250, boss: true, ..plain(10, 246) };
    let boss = Creature { species: Some(175), ..crea(0, 10000, 1000, 100, 100) };
    let add = Creature { species: Some(97), ..crea(1, 1000, 500, 50, 50) };
    let unknown = Creature { species: None, ..crea(2, 1000, 500, 50, 50) };
    let out = forks(&row, Some(&below), &[boss, add, unknown], &mut FloorMemory::default(), None);
    assert_eq!((out[0].hp_max, out[0].atk_eff, out[0].def_phys_eff), (30000, 2500, 200));
    // ★ 138차 결정 ⑬: the escort takes the floor below's LEVEL only — its 130 % / 110 %
    // are not the escort's, so it stays at the game's own numbers here (no level set).
    assert_eq!((out[1].hp_max, out[1].atk_eff, out[1].def_phys_eff), (1000, 500, 50));
    assert_eq!((out[2].hp_max, out[2].atk_eff, out[2].def_phys_eff), (3000, 1250, 100), "unknown = boss");
    // With no row below, everyone gets the boss row.
    let out = forks(&row, None, &[add], &mut FloorMemory::default(), None);
    assert_eq!(out[0].hp_max, 3000);
    // And an ordinary floor ignores the species entirely.
    let out = forks(&below, Some(&row), &[boss], &mut FloorMemory::default(), None);
    assert_eq!(out[0].hp_max, 13000);
}

/// ★ 138차 결정 ⑬, the other half: the escort IS grown to the floor below's level — only
/// the percentages are dropped. Floor 99 of the 137차 table ramps hp% to 150, and floor
/// 100's eight escorts must stay what they were («100층은 지금 그대로»).
#[test]
fn a_raid_escort_takes_the_floor_belows_level_but_not_its_percentages() {
    let below = Floor { level: Some(119), hp_pct: 150, def_pct: 100, atk_pct: 100, ..plain(99, 245) };
    let row = Floor { level: Some(120), boss: true, ..plain(100, 246) };
    let add = Creature { species: Some(143), level: Some(66), ..crea(1, 7289, 1135, 80, 80) };
    let out = forks(&row, Some(&below), &[add], &mut FloorMemory::default(), None);
    let lv = Some(119);
    assert_eq!(out[0].level, lv, "the escort is forked to the floor below's level");
    let grown = grown_pct(7289, HP_GROWTH, 119 - 66, PCT_NEUTRAL);
    assert_eq!(out[0].hp_max as u32, grown.min(I16_MAX as u32), "grown, but not by the row's 150 %");
    assert!(grown > I16_MAX as u32, "the escort's total goes to the virtual word");
    assert_eq!(out[0].vhp, Some(grown), "the virtual total is the neutral one");
    // And a 150 % row would have been half again as much — the number the decision refused.
    assert!(grown_pct(7289, HP_GROWTH, 119 - 66, 150) > grown * 14 / 10);
}

/// `Run::adds_row` is the floor below, and `forks_for` goes through the run's own
/// memory — which `enter` and `finish` both clear, because a new scene is a new
/// creature array and index 0 of the next floor is not index 0 of this one.
#[test]
fn the_runs_memory_is_cleared_on_entry_and_on_finish() {
    let mut run = Run::new(vec![
        Floor { hp_pct: 200, ..plain(1, 243) },
        Floor { hp_pct: 200, ..plain(2, 244) },
    ]);
    assert!(run.adds_row(1).is_none());
    assert_eq!(run.adds_row(2).map(|f| f.floor), Some(1));
    run.enter(1, &on(0, 0, 0));
    run.poll(&on(243, 1, 1));
    let c = crea(0, 500, 100, 10, 10);
    assert_eq!(run.forks_for(1, &[c], None)[0].hp_max, 1000);
    assert_eq!(run.memory.bases.len(), 1);
    // Floor 2: index 0 is a different monster with a different base.
    run.poll(&on(243, 1, 0));
    run.next_floor(&on(243, 0, 0));
    assert!(run.memory.bases.is_empty(), "cleared on enter");
    run.poll(&on(244, 1, 1));
    let other = crea(0, 700, 100, 10, 10);
    assert_eq!(run.forks_for(2, &[other], None)[0].hp_max, 1400);
    run.finish();
    assert!(run.memory.bases.is_empty(), "cleared on finish");
}

/// A row that overrides the species must leave a slot live — a floor that spawns
/// nothing can never be seen alive, let alone cleared.
#[test]
fn a_species_row_with_no_live_slot_is_refused() {
    assert!(!Floor { species: Some([SPECIES_NONE; 3]), ..plain(1, 243) }.valid());
    assert!(Floor { species: Some([146, SPECIES_NONE, SPECIES_NONE]), ..plain(1, 243) }.valid());
}

// ── the boss corpse (88차): a dead boss keeps being drawn until it is parked ──

/// Only a DEAD record whose species is a `bossh_dat` boss is parked; a live boss, an
/// ordinary corpse (the game hides those itself) and an unreadable species are not.
#[test]
fn only_dead_bosses_are_parked_off_the_map() {
    let live_boss = Creature { species: Some(175), ..crea(0, 17000, 1000, 100, 100) };
    let dead_boss = Creature { hp_cur: 0, ..live_boss };
    let dead_box = Creature { species: Some(146), hp_cur: 0, ..crea(2, 100, 1, 1, 1) };
    let dead_unknown = Creature { species: None, hp_cur: 0, ..crea(3, 100, 1, 1, 1) };
    let mut dead_boss = dead_boss;
    dead_boss.index = 1;
    assert_eq!(corpse_hides(&[live_boss, dead_boss, dead_box, dead_unknown]), vec![1]);
    assert!(CORPSE_HIDE_POS < -(50 * 16), "well outside the widest map");
}

// ── the per-floor monster count (2026-09-16) — capped at SPAWN since 145차 ────

/// ★ 145차: the count is armed as a SPAWN CAP for the floor's own scene, next to the
/// species triple; a boss floor or a legacy row arms nothing.
#[test]
fn a_counted_floor_arms_its_spawn_cap_for_its_own_scene() {
    let f = parse_floor("5 244 61 120 100 - 100 146 90 155 12").expect("row with a count");
    assert_eq!(f.spawn_cap(), Some((244, 12)));
    assert_eq!(f.species_override(), Some((244, [146, 90, 155])), "keyed on the same scene");
    let legacy = parse_floor("5 244 61 120 100 - 100 146 90 155").expect("legacy 10-column row");
    assert_eq!(legacy.spawn_cap(), None, "no count = the scene spawns everything its slots fill");
}

/// ★ 145차: surplus monsters are only ever REPORTED — the host never removes one (the
/// old parking made them die on arrival). `Some(records)` only when the floor holds more
/// than its count, i.e. the spawn cap did not apply.
#[test]
fn a_floor_over_its_count_is_reported_never_trimmed() {
    let mons: Vec<Creature> = (0..6).map(|i| crea(i, 100, 10, 5, 5)).collect();
    assert_eq!(spawned_over_count(Some(2), &mons), Some(6), "the cap missed: six records for a count of 2");
    assert_eq!(spawned_over_count(Some(6), &mons), None, "exactly the count");
    assert_eq!(spawned_over_count(Some(20), &mons), None);
    assert_eq!(spawned_over_count(None, &mons), None, "no count, nothing to compare with");
    // Dead records still count: they are records the spawn loop created.
    let mut some_dead = mons.clone();
    some_dead[0].hp_cur = 0;
    assert_eq!(spawned_over_count(Some(5), &some_dead), Some(6));
}

// ── the top-centre banner (145차) ─────────────────────────────────────────────

/// `hud` is the banner's content: nothing outside a run, the floor and "entering" with NO
/// count while the scene loads, the live count on the floor, then the cleared state.
#[test]
fn the_banner_shows_the_floor_and_the_live_count() {
    let mut r = one_floor_run();
    assert_eq!(r.hud(9), None, "Idle must be silent");
    let obs = Obs { scene: Some(111), spawned: 0, alive: 0, in_field: true, in_world: true, player_hp: Some(10), player_tile: None };
    r.enter(1, &obs);
    assert_eq!(r.hud(7), Some(TowerHud { floor: 1, boss: false, phase: TowerPhase::Entering }));
    r.poll(&Obs { scene: Some(134), alive: 19, spawned: 19, ..obs });
    assert_eq!(r.hud(19), Some(TowerHud { floor: 1, boss: false, phase: TowerPhase::Fighting { left: 19 } }));
    assert_eq!(r.hud(-3).map(|h| h.phase), Some(TowerPhase::Fighting { left: 0 }), "no negative count");
    r.leave();
    assert_eq!(r.hud(19).map(|h| h.phase), Some(TowerPhase::Left));
    r.finish();
    assert_eq!(r.hud(19), None);
}

/// A boss floor says so, and a cleared floor stops counting.
#[test]
fn the_banner_marks_a_boss_floor_and_its_clear() {
    let mut r = one_floor_run();
    let obs = Obs { scene: Some(111), spawned: 0, alive: 0, in_field: true, in_world: true, player_hp: Some(10), player_tile: None };
    r.enter(2, &obs);
    r.poll(&Obs { scene: Some(137), alive: 1, spawned: 1, ..obs });
    assert_eq!(r.hud(1), Some(TowerHud { floor: 2, boss: true, phase: TowerPhase::Fighting { left: 1 } }));
    r.poll(&Obs { scene: Some(137), alive: 0, spawned: 1, ..obs });
    assert_eq!(r.hud(0), Some(TowerHud { floor: 2, boss: true, phase: TowerPhase::Cleared }));
}

/// The count column round-trips through the parser and is bounded and boss-exclusive.
#[test]
fn the_count_column_parses_and_is_validated() {
    let f = parse_floor("5 244 61 120 100 - 100 146 90 155 12").expect("row with a count");
    assert_eq!(f.count, Some(12));
    assert_eq!(f.species, Some([146, 90, 155]));
    // A legacy 10-column row has no count.
    assert_eq!(parse_floor("5 244 61 120 100 - 100 146 90 155").unwrap().count, None);
    // Out of range, and a count on a boss floor, are both refused.
    assert!(parse_floor("5 244 61 120 100 - 100 146 90 155 21").is_none(), "count > MAX");
    assert!(parse_floor("5 244 61 120 100 - 100 146 90 155 0").is_none(), "count 0");
    assert!(parse_floor("10 246 62 300 150 B 250 175 - - 5").is_none(), "count on a boss floor");
}

// ── 112차: the level curve carries the stats ─────────────────────────────────

/// The game's own per-level growth, measured on its own tables: the same species is
/// ×1.036 HP, ×1.043 attack and ×1.0285 defence per level between `enemy_dat` and
/// `enemyh_dat`. A record forked from its spawn level to the floor's level grows by
/// exactly that, so a Lv 60 rat has Lv 60 stats and a Lv 120 knight has Lv 120 stats.
#[test]
fn a_forked_record_grows_from_its_spawn_level_to_the_floors() {
    let f = Floor { level: Some(120), ..plain(100, 246) };
    let mut c = crea(0, 7289, 1135, 700, 700);      // 커브이라 on the hard table, Lv 66
    c.level = Some(66);
    let fk = forks1(&f, &[c]);
    // 54 levels of growth: HP x6.75 (does not fit the i16 — the virtual word takes it),
    // attack x9.7, defence x4.56 — all from `grow`, so the test computes them the same way.
    let hp = grow(7289, HP_GROWTH, 54);
    assert!((49_000..=49_500).contains(&hp), "hp {hp}");
    assert_eq!(fk[0].hp_max, I16_MAX);
    assert_eq!(fk[0].vhp, Some(hp));
    assert_eq!(fk[0].atk_eff, grow(1135, ATK_GROWTH, 54) as u16);
    assert!((10_900..=11_100).contains(&fk[0].atk_eff), "atk {}", fk[0].atk_eff);
    assert_eq!(fk[0].def_phys_eff, grow(700, DEF_GROWTH, 54) as u16);
    assert!((3_150..=3_250).contains(&fk[0].def_phys_eff), "def {}", fk[0].def_phys_eff);
    assert_eq!(fk[0].level, Some(120));
}

/// The growth runs both ways and the percentages sit on top of it.
#[test]
fn the_growth_shrinks_below_the_spawn_level_and_stacks_with_the_row_percentage() {
    let mut c = crea(0, 1000, 500, 200, 200);
    c.level = Some(66);
    // Six levels DOWN: x1.036^-6 = 0.809.
    let down = forks1(&Floor { level: Some(60), ..plain(1, 243) }, &[c]);
    assert_eq!(down[0].hp_max, 809);
    assert_eq!(down[0].atk_eff, grow(500, ATK_GROWTH, -6) as u16);
    // Same level: growth is exactly 1, the row's percentage is all that applies.
    let same = forks1(&Floor { level: Some(66), hp_pct: 150, atk_pct: 200, ..plain(1, 243) }, &[c]);
    assert_eq!((same[0].hp_max, same[0].atk_eff), (1500, 1000));
    // Growth AND percentage: 10 levels up (x1.424) and 150 %.
    let both = forks1(&Floor { level: Some(76), hp_pct: 150, ..plain(1, 243) }, &[c]);
    assert_eq!(both[0].hp_max, (grow(1000, HP_GROWTH, 10) * 150 / 100) as u16);
}

/// No row level, or an unreadable spawn level, means no growth — the percentages alone,
/// which is exactly what every pre-112차 test above still expects.
#[test]
fn no_level_means_no_growth() {
    let mut c = crea(0, 1000, 500, 200, 200);
    c.level = Some(30);
    let no_row = forks1(&Floor { hp_pct: 120, ..plain(1, 243) }, &[c]);
    assert_eq!((no_row[0].hp_max, no_row[0].atk_eff), (1200, 500));
    let mut unread = crea(0, 1000, 500, 200, 200);
    unread.level = None;
    let no_spawn = forks1(&Floor { level: Some(120), ..plain(1, 243) }, &[unread]);
    assert_eq!((no_spawn[0].hp_max, no_spawn[0].atk_eff), (1000, 500));
    assert_eq!(no_spawn[0].level, Some(120), "the byte is still forked");
    assert_eq!(level_delta(None, Some(5)), 0);
    assert_eq!(level_delta(Some(120), None), 0);
    assert_eq!(level_delta(Some(120), Some(66)), 54);
    assert_eq!(level_delta(Some(127), Some(120)), 0, "the row level is clamped before the delta");
}

/// The spawn level is a FIRST-SIGHTING fact like the HP base: after the fork has
/// written the floor's level into the byte, the next poll must not grow from 120 again.
#[test]
fn the_spawn_level_is_remembered_so_growth_does_not_compound() {
    let f = Floor { level: Some(120), ..plain(100, 246) };
    let mut c = crea(0, 1000, 500, 200, 200);
    c.level = Some(66);
    let mut mem = FloorMemory::default();
    let first = forks(&f, None, &[c], &mut mem, None);
    // The game now shows the forked values and the forked level.
    let mut again = c;
    again.level = Some(120);
    again.hp_max = first[0].hp_max;
    again.hp_cur = first[0].hp_cur;
    again.atk_eff = first[0].atk_eff;
    again.def_phys_eff = first[0].def_phys_eff;
    again.def_spec_eff = first[0].def_spec_eff;
    again.vhp = first[0].vhp.unwrap_or(0);
    let second = forks(&f, None, &[again], &mut mem, None);
    assert_eq!(first[0].hp_max, second[0].hp_max);
    assert_eq!(first[0].atk_eff, second[0].atk_eff);
    assert_eq!(first[0].def_phys_eff, second[0].def_phys_eff);
    assert_eq!(second[0].vhp, None, "the word is seeded once");
    assert_eq!(mem.bases[0].lv0, Some(66));
}

/// `grow` itself: exact at delta 0, rounded, floored at 1, and monotone in the delta.
#[test]
fn grow_is_exact_at_zero_rounded_and_never_zero() {
    assert_eq!(grow(1234, HP_GROWTH, 0), 1234);
    assert_eq!(grow(100, HP_GROWTH, 1), 104);        // 103.6 rounds up
    assert_eq!(grow(100, ATK_GROWTH, 1), 104);       // 104.3 rounds down
    assert_eq!(grow(1, HP_GROWTH, -100), 1);
    assert_eq!(grow(0, HP_GROWTH, 50), 1, "a zero base still forks to a live stat");
    for d in -60..60 {
        assert!(grow(1000, HP_GROWTH, d + 1) >= grow(1000, HP_GROWTH, d));
    }
}

// ── 112차: the clear-EXP column ──────────────────────────────────────────────

/// Twelve columns parse, eleven still parse (a legacy row pays nothing), a boss row
/// writes `-` for its missing count so the EXP column stays in place, and the value
/// is bounded.
#[test]
fn the_exp_column_is_optional_bounded_and_round_trips() {
    let ord = parse_floor("3 245 61 100 100 - 100 12 47 - 20 18300").expect("12 columns");
    assert_eq!((ord.count, ord.exp), (Some(20), 18_300));
    let boss = parse_floor("10 246 65 100 100 B 100 169 - - - 65000").expect("boss row with `-` count");
    assert_eq!((boss.count, boss.exp, boss.boss), (None, 65_000, true));
    let legacy = parse_floor("3 245 61 100 100 - 100 12 47 - 20").expect("11 columns");
    assert_eq!(legacy.exp, 0);
    assert!(parse_floor("3 245 61 100 100 - 100 12 47 - 20 99999999").is_none(), "above EXP_MAX");
    assert!(parse_floor("3 245 61 100 100 - 100 12 47 - 20 18300 junk").is_none(), "trailing junk");
    let text = write_table(&[ord, boss, legacy]);
    let (rows, bad) = parse_table(&text);
    assert!(bad.is_empty(), "{text}");
    assert_eq!(rows, vec![ord, boss, legacy]);
}

// ── 2026-09-21: the arrival is placed, and the sprite is placed with it ───────

/// The draw corner is the game's `0x1c234` and nothing else. Both branches of the
/// vertical rule, and the convention the placement leaves the box at (16 x 8).
#[test]
fn the_draw_corner_mirrors_the_games_own_formula() {
    // Tile (10, 14) -> px (168, 224). Half the 16-wide box comes off the x.
    assert_eq!(draw_offsets(168, 224, 0x10, 8, 0).0, 160);
    // kind 4 / 6 / 9 take the whole height, everything else half of it.
    for kind in [4, 6, 9] {
        assert_eq!(draw_offsets(168, 224, 0x10, 8, kind), (160, 216), "kind {kind}");
    }
    for kind in [0, 1, 3, 5, 7, 8, 10, -1] {
        assert_eq!(draw_offsets(168, 224, 0x10, 8, kind), (160, 220), "kind {kind}");
    }
    // An odd box halves the way an arithmetic shift does, negatives included.
    assert_eq!(draw_offsets(100, 100, 7, 7, 0), (97, 97));
    assert_eq!(draw_offsets(100, 100, -8, -8, 0), (104, 104));
    // ⛔ The corner is never simply the position: a caller that writes the pixel pair
    // and leaves `+0x16`/`+0x18` alone is the invisible-character bug.
    assert_ne!(draw_offsets(168, 224, 0x10, 8, 0), (168, 224));
}

/// `arrival_tile` has NO "they are already fine" early-out — that is the whole
/// difference from [`landing_tile`], and it is what makes a floor start where the host
/// chose rather than where the last floor happened to leave the player.
#[test]
fn the_arrival_tile_is_chosen_even_when_the_player_is_fine() {
    let a = vec![0u8; 22 * 27];
    // Standing in the middle of an open room: the landing check leaves them alone …
    assert_eq!(landing_tile(22, 27, &a, (3, 3)), None);
    // … and the arrival still names the middle.
    assert_eq!(arrival_tile(22, 27, &a), Some((11, 13)));
    // Both answer the same tile once the player IS wedged, so the two never disagree
    // about where the room is.
    assert_eq!(landing_tile(22, 27, &a, (90, 90)), arrival_tile(22, 27, &a));
}

/// A hall's centre pillar does not swallow the arrival: the ring search steps off it,
/// and the tile it picks is one the player can actually walk around in.
#[test]
fn the_arrival_tile_steps_off_a_centre_pillar() {
    // The shipped halls: 22x27, walls all round, one blocked tile dead centre.
    let (w, h) = (22i32, 27i32);
    let mut a = vec![TILE_BLOCKED; (w * h) as usize];
    for y in 5..25 {
        for x in 1..21 {
            a[(y * w + x) as usize] = 0;
        }
    }
    let (cx, cy) = (w / 2, h / 2);
    a[(cy * w + cx) as usize] = TILE_BLOCKED;
    let got = arrival_tile(w, h, &a).expect("a 20x20 arena has somewhere to stand");
    assert_ne!(got, (cx as u8, cy as u8), "the pillar itself is never the answer");
    let blocked = |x: i32, y: i32| {
        x < 0 || y < 0 || x >= w || y >= h || a[(y * w + x) as usize] & TILE_BLOCKED != 0
    };
    assert!(!blocked(i32::from(got.0), i32::from(got.1)));
    assert!(
        room_at_least(w, h, &blocked, (i32::from(got.0), i32::from(got.1)), LANDING_MIN_ROOM)
            >= LANDING_MIN_ROOM
    );
    // Next to the pillar, not across the room: the arenas are built around a middle.
    assert!((i32::from(got.0) - cx).abs() <= 1 && (i32::from(got.1) - cy).abs() <= 1);
}

/// ⛔ REFUSE TO GUESS, exactly as `landing_tile` does: a geometry that is not a Hero3
/// map teleports nobody.
#[test]
fn the_arrival_tile_refuses_a_geometry_it_cannot_read() {
    let a = vec![0u8; 20 * 20];
    assert_eq!(arrival_tile(20, 20, &a[..399]), None, "short attribute array");
    assert_eq!(arrival_tile(0, 20, &a), None, "zero width");
    assert_eq!(arrival_tile(20, 0, &a), None, "zero height");
    assert_eq!(arrival_tile(201, 20, &a), None, "not a Hero3 map");
    assert_eq!(arrival_tile(20, 20, &vec![TILE_BLOCKED; 400]), None, "nowhere to stand");
}

/// The frame budget is the poll budget, spent a frame at a time.
#[test]
fn the_landing_budget_is_the_same_length_in_either_unit() {
    assert_eq!(LANDING_FRAMES, LANDING_POLLS * 30);
    assert!(LANDING_FRAMES >= 300, "a scene load must fit inside it");
}

// ── 헬모드 (133차) ────────────────────────────────────────────────────────────

/// The ladder (134차): linear over the HARD table's own spawn range, `[38..67] →
/// [68..100]`, monotone, clamped at both ends, and its floor is within the drop band of
/// the lowest legacy rung (73 - 6 = 67 <= 68) so no hell level is a dead stretch.
#[test]
fn the_hell_ladder_spans_68_to_100_over_the_hard_tables_spawn_range() {
    assert_eq!(HELL_LV_MIN, 68, "user decision 2026-09-24: 헬 몬스터는 레벨 68부터");
    assert_eq!((HELL_SPAWN_LV_MIN, HELL_SPAWN_LV_MAX), (38, 67), "enemyh_dat's measured range");
    assert_eq!(hell_level(HELL_SPAWN_LV_MIN), HELL_LV_MIN);
    assert_eq!(hell_level(HELL_SPAWN_LV_MAX), HELL_LV_MAX);
    // Monotone: a harder field stays harder in hell, and one hard level never jumps
    // more than two hell levels (32 outputs over 29 input steps).
    let mut prev = HELL_LV_MIN;
    for lv0 in HELL_SPAWN_LV_MIN..=HELL_SPAWN_LV_MAX {
        let h = hell_level(lv0);
        assert!((HELL_LV_MIN..=HELL_LV_MAX).contains(&h), "lv0 {lv0} -> {h}");
        assert!(h >= prev, "lv0 {lv0}: {h} < {prev}");
        assert!(h - prev <= 2, "lv0 {lv0}: a one-level step in the hard table jumps {prev} -> {h}");
        prev = h;
    }
    // Spot values of the rounded map (the page the user was given quotes these).
    assert_eq!(hell_level(53), 85);
    assert_eq!(hell_level(61), 93);
    assert_eq!(hell_level(65), 98);
    // Below the hard floor cannot happen in hell (byte 2 spawns the hard table) and
    // clamps to the floor; above it is a tower record and clamps to the ceiling.
    assert_eq!(hell_level(1), HELL_LV_MIN);
    assert_eq!(hell_level(37), HELL_LV_MIN);
    assert_eq!(hell_level(120), HELL_LV_MAX);
    assert_eq!(hell_level(-3), HELL_LV_MIN);
    // 73 - 6 = 67 <= 68: even the weakest hell monster reaches the Lv73 rows (the 133차
    // hole — hell 67..85 had no monster and the Lv 73..79 rows never dropped — is closed).
    assert!(i32::from(HELL_LV_MIN) + 6 >= 73);
    assert!(HELL_LV_MAX <= LEVEL_MAX, "the fork's own clamp must not undo the ladder");
}

/// The promotion rule: only a non-zero pre-ending difficulty (a hardcore or hell
/// clear) opens hell, and hell stays hell. A normal clear keeps the stock 2회차.
#[test]
fn hell_opens_after_a_hardcore_clear_and_only_then() {
    assert_eq!(hell_promotion(Some(1)), Some(HELL_DIFFICULTY));
    assert_eq!(hell_promotion(Some(HELL_DIFFICULTY)), Some(HELL_DIFFICULTY), "hell stays hell");
    assert_eq!(hell_promotion(Some(0)), None, "a normal clear is the game's own 2회차");
    assert_eq!(hell_promotion(None), None, "an unreadable byte promotes nothing");
    assert!(is_hell(Some(2)) && !is_hell(Some(1)) && !is_hell(Some(0)) && !is_hell(None));
    assert_eq!(HELL_DIFFICULTY, 2, "the game's own writers use 0 and 1; 2 is the free value");
}

/// The hell fork grows every live creature from ITS OWN spawn level — per creature,
/// not per floor — with the same growth ratios and virtual-HP seeding as the tower.
#[test]
fn the_hell_fork_grows_each_creature_from_its_own_spawn_level() {
    let weak = Creature {
        index: 0, hp_max: 75, hp_cur: 75, respawn_timer: 0xffff,
        atk_eff: 64, atk_skill: [0, 0], def_phys_eff: 32, def_spec_eff: 29, species: Some(18), vhp: 0,
        level: Some(4),
    };
    let strong = Creature { index: 1, hp_max: 16000, hp_cur: 16000, level: Some(67), ..weak };
    let dead = Creature { index: 2, hp_cur: 0, ..weak };
    let mut mem = FloorMemory::default();
    let forks = hell_forks(&[weak, strong, dead], &mut mem, Some(90));
    assert_eq!(forks.len(), 2, "a corpse is not forked");
    let f0 = forks.iter().find(|f| f.index == 0).unwrap();
    let f1 = forks.iter().find(|f| f.index == 1).unwrap();
    assert_eq!(f0.level, Some(hell_level(4)));
    assert_eq!(f1.level, Some(HELL_LV_MAX));
    // The weak record grew by its own delta (67 - 4 = 63 levels)…
    let want0 = grown_pct(75, HP_GROWTH, i32::from(hell_level(4)) - 4, PCT_NEUTRAL);
    assert_eq!(u32::from(f0.hp_max), want0.min(u32::from(I16_MAX)));
    assert_eq!(f0.vhp, None, "75 * 1.036^63 fits the record");
    // …and the strong one overflowed i16, so the virtual word takes the excess.
    let want1 = grown_pct(16000, HP_GROWTH, i32::from(HELL_LV_MAX) - 67, PCT_NEUTRAL);
    assert!(want1 > u32::from(I16_MAX));
    assert_eq!(u32::from(f1.hp_max), u32::from(I16_MAX));
    assert_eq!(f1.vhp, Some(want1), "the excess is seeded into the virtual word");
    assert!(f0.atk_eff > 64 && f0.def_phys_eff > 32 && f0.def_spec_eff > 29);
}

/// Idempotence: re-running the fork on records it already raised computes the SAME
/// absolute targets (zero writes), a wound is kept, and an unreadable spawn level
/// degenerates to the identity instead of guessing.
#[test]
fn the_hell_fork_is_idempotent_and_keeps_wounds() {
    let spawn = Creature {
        index: 0, hp_max: 500, hp_cur: 500, respawn_timer: 0xffff,
        atk_eff: 100, atk_skill: [0, 0], def_phys_eff: 50, def_spec_eff: 40, species: Some(30), vhp: 0,
        level: Some(30),
    };
    let mut mem = FloorMemory::default();
    let first = hell_forks(&[spawn], &mut mem, Some(90))[0];
    // The next poll sees the RAISED record; the snapshot keeps the spawn base.
    let raised = Creature {
        hp_max: first.hp_max, hp_cur: first.hp_cur, atk_eff: first.atk_eff,
        def_phys_eff: first.def_phys_eff, def_spec_eff: first.def_spec_eff,
        level: first.level, ..spawn
    };
    let second = hell_forks(&[raised], &mut mem, Some(90))[0];
    assert_eq!(first, second, "the steady state must plan the same bytes");
    // A wound survives the re-assert.
    let wounded = Creature { hp_cur: 17, ..raised };
    let third = hell_forks(&[wounded], &mut mem, Some(90))[0];
    assert_eq!(third.hp_cur, 17);
    assert_eq!(third.hp_max, first.hp_max);
    // No spawn level = no growth, no level byte: the identity fork.
    let blind = Creature { level: None, index: 9, ..spawn };
    let f = hell_forks(&[blind], &mut mem, Some(90));
    let fb = f.iter().find(|f| f.index == 9).unwrap();
    assert_eq!(fb.level, None);
    assert_eq!((fb.hp_max, fb.atk_eff), (500, 100), "no guessed growth");
    assert_eq!((fb.hit, fb.dodge, fb.res), (None, None, None), "no level, no roll fields either");
}


// ── 156차: the hell monsters' combat rolls ──────────────────────────────────────

/// Both forks set the game's 명중 · 회피 · 기절 저항 for every record whose level they know,
/// against the player's level — the hell fork since 156차, the tower's since 158차 P8 (until
/// then a floor-85+ monster was hit at the 15 % floor by anyone without 명중 gear).
#[test]
fn both_forks_set_the_roll_fields_against_the_player() {
    let c = Creature { species: Some(18), ..crea(0, 500, 100, 50, 40) };
    let c = Creature { level: Some(50), ..c };
    let f = Floor { level: Some(70), ..plain(1, 243) };
    // The tower: the ROW's level against the player's.
    let mut mem = FloorMemory::default();
    let t = forks(&f, None, &[c], &mut mem, Some(90))[0];
    assert_eq!(t.level, Some(70));
    assert_eq!(t.hit, Some(hell_hit(70, 90)));
    assert_eq!(t.dodge, Some(hell_dodge(70, 90)));
    assert_eq!(t.res, Some(hell_res(70, 90)));
    // Floor 100 (Lv 120) vs Lv 99: the +1/level on the monster's hit remains, the player's
    // hit and crit have their level term cancelled — the same knobs hell uses.
    let top = Floor { level: Some(120), ..plain(100, 246) };
    let t120 = forks(&top, None, &[c], &mut FloorMemory::default(), Some(99))[0];
    assert_eq!(t120.hit, Some(hell_hit(120, 99)));
    let (m_hit, p_hit, p_crit) = hell_rates(120, 99, 70, 0, 20, 1, 0);
    assert_eq!(m_hit, 140 - 70 + 21, "21 levels up: the monster's hit keeps its +1/level (91 % on a 회피-70 player)");
    assert_eq!(p_hit, 65, "but the player's hit no longer sinks to the 15 % floor");
    assert_eq!(p_crit, 13);
    // No player level = the base knobs; no row level = no roll field at all.
    let t0 = forks(&f, None, &[c], &mut FloorMemory::default(), None)[0];
    assert_eq!(t0.hit, Some(HELL_HIT_BASE as i16));
    let nolv = forks(&plain(1, 243), None, &[c], &mut FloorMemory::default(), Some(90))[0];
    assert_eq!((nolv.hit, nolv.dodge, nolv.res), (None, None, None));
    let mut mem = FloorMemory::default();
    let h = hell_forks(&[c], &mut mem, Some(90))[0];
    let mlv = hell_level(50);
    assert_eq!(h.level, Some(mlv));
    assert_eq!(h.hit, Some(hell_hit(mlv, 90)));
    assert_eq!(h.dodge, Some(hell_dodge(mlv, 90)));
    assert_eq!(h.res, Some(hell_res(mlv, 90)));
    // No player level = the monster's own (the base knobs).
    let mut mem = FloorMemory::default();
    let h0 = hell_forks(&[c], &mut mem, None)[0];
    assert_eq!(h0.hit, Some(HELL_HIT_BASE as i16));
    assert_eq!(h0.dodge, Some(HELL_DODGE_BASE as u16));
    assert_eq!(h0.res, Some(HELL_RES_BASE as i16 as u16));
    // Defence grows with the hell percentage on top of the level growth.
    let delta = i32::from(mlv) - 50;
    assert_eq!(u32::from(h.def_phys_eff), grown_pct(50, DEF_GROWTH, delta, HELL_DEF_PCT));
    assert!(h.def_phys_eff > t.def_phys_eff || HELL_DEF_PCT <= 100);
}

/// The knobs read as percentages against a same-level player, and the level term is
/// mostly cancelled: a monster 10 levels above gains +10 % to hit and costs the player
/// −10 % to hit, not ±50 %. The game's own formulas with the fork's fields substituted.
#[test]
fn the_hell_rolls_are_the_knobs_against_a_same_level_player() {
    // 회피 70 (민첩 60 → 20 + passives 50), 명중 0 (six of the ten classes have no 명중
    // passive), a 20-tree weapon, no 기절 — every value below stays inside the clamps.
    let (m, p, c) = hell_rates(90, 90, 70, 0, 20, 1, 0);
    assert_eq!(m, 80 + HELL_HIT_BASE - 70);
    assert_eq!(p, 80 + 0 + 20 - HELL_DODGE_BASE);
    assert_eq!(c, 1 - HELL_RES_BASE);
    // Ten levels above the player: only the UNCANCELLED share of the game's ±5/level is
    // left — +10 % for the monster to hit, nothing on the player's hit, nothing on the crit.
    let (m10, p10, c10) = hell_rates(100, 90, 70, 0, 20, 1, 0);
    assert_eq!(m10, m + (5 - HELL_HIT_PER_LV) * 10);
    assert_eq!(p10, p - (5 - HELL_DODGE_PER_LV) * 10);
    assert_eq!(c10, c);
    // …and a Lv 85 player meets a Lv 100 monster at the same player-hit % as a Lv 99 one.
    assert_eq!(hell_rates(100, 85, 70, 0, 20, 1, 0).1, hell_rates(100, 99, 70, 0, 20, 1, 0).1);
    // The player's options move the rolls one for one, into the game's own clamps.
    assert_eq!(hell_rates(90, 90, 70 + 30, 0, 20, 1, 0).0, m - 30, "+30 회피");
    assert_eq!(hell_rates(90, 90, 70 + 200, 0, 20, 1, 0).0, 15, "the floor");
    assert_eq!(hell_rates(90, 90, 70, 0 + 50, 20, 1, 0).1, 95, "+50 명중 hits the cap");
    assert_eq!(hell_rates(90, 90, 70, 0, 60, 1, 0).1, 95, "검술's 60 caps without any option");
    assert_eq!(hell_rates(90, 90, 70, 0, 0, 1, 0).1, p - 20, "사격's 0");
    assert_eq!(hell_rates(90, 90, 70, 0, 20, 1, 100).2, 60, "+100 기절 is the cap");
    assert_eq!(hell_rates(90, 90, 70, 0, 20, 20, 0).2, c + 19, "사격's crit 20");
    // A monster far below a Lv 99 player still hits sometimes, is hit exactly as often as
    // any other (the level term is gone from that roll), and crits the same.
    let (m68, p68, c68) = hell_rates(68, 99, 70, 0, 20, 1, 0);
    assert!(m68 >= 15 && m68 < m, "{m68}");
    assert_eq!(p68, p);
    assert_eq!(c68, c);
}

/// The user's table (2026-10-02, `HELL_COMBAT_BALANCE.md §4.1`): with NO 명중 gear a gun
/// class without the 조준 passive hits 45 %, with it (or +20 gear) 65 %, +40 → 85 %; a 20-tree
/// weapon without a 명중 passive 65 % → 85 % → 95 %; 검술 is capped without anything.
#[test]
fn the_users_hit_table_holds() {
    let hit = |명중: i32, d24: i32| hell_rates(70, 70, 70, 명중, d24, 1, 0).1;
    assert_eq!((hit(0, 0), hit(20, 0), hit(40, 0)), (45, 65, 85), "사격");
    assert_eq!((hit(0, 20), hit(20, 20), hit(40, 20)), (65, 85, 95), "창 · 단도 · 영탄 · 광아");
    assert_eq!(hit(0, 60), 95, "검술");
    assert_eq!(hit(40, 20), 95, "검술 집중 5 on a 20-tree weapon");
    // and the level term stays cancelled: Lv 85 vs Lv 100 reads the same.
    assert_eq!(hell_rates(100, 85, 70, 0, 0, 1, 0).1, 45);
    // the crit side the user asked about: 사격 d25 20 → 32 %, everything else 13 %.
    assert_eq!(hell_rates(70, 70, 70, 0, 0, 20, 0).2, 32);
    assert_eq!(hell_rates(70, 70, 70, 0, 0, 1, 0).2, 13);
}

/// The fields go into the record in the game's own representation: 명중 is signed, 회피 and
/// 저항 may go negative and are stored two's-complement (the rolls truncate their sum to
/// 16 bits and sign-extend it, so a negative acts as the negative it is).
#[test]
fn the_hell_roll_fields_are_stored_the_way_the_rolls_read_them() {
    assert_eq!(hell_hit(68, 99), (HELL_HIT_BASE + HELL_HIT_PER_LV * 31) as i16);
    assert!(hell_hit(100, 70) < HELL_HIT_BASE as i16);
    assert_eq!(hell_dodge(100, 70) as i16, (HELL_DODGE_BASE - HELL_DODGE_PER_LV * 30) as i16, "a monster far above: negative, two's complement");
    assert!(hell_dodge(100, 70) > 0x8000);
    assert!((hell_dodge(68, 99) as i16) > HELL_DODGE_BASE as i16);
    let r = hell_res(90, 90);
    assert_eq!(r as i16, HELL_RES_BASE as i16);
    assert!(r > 0x8000, "a negative resist is a two's-complement halfword");
    assert_eq!(hell_res(68, 99) as i16, (HELL_RES_BASE + HELL_RES_PER_LV * 31) as i16);
}


// ── 143차: the way out of a finished tower (the wake) ───────────────────────

fn wv(now: u64) -> WakeView {
    WakeView { now, ..WakeView::default() }
}

/// The top floor waits for a quiet screen — the clear's own message box, a menu the
/// player opened — before the veil rises, and rises anyway once the cap is spent.
#[test]
fn the_wake_whitens_only_on_a_quiet_screen_or_after_the_cap() {
    let mut p = WakePhase::start(1_000);
    assert_eq!(wake_step(&mut p, &wv(2_000)), WakeStep::None, "a box is still up");
    assert!(!p.veiled() && !p.gone_home());
    assert_eq!(wake_step(&mut p, &WakeView { quiet: true, ..wv(3_000) }), WakeStep::Whiten { ms: WAKE_WHITEN_MS });
    assert_eq!(p, WakePhase::Whiten { white_at: 3_000 + u64::from(WAKE_WHITEN_MS) });
    assert!(p.veiled());
    let mut q = WakePhase::start(0);
    assert_eq!(wake_step(&mut q, &wv(WAKE_QUIET_MAX_MS - 1)), WakeStep::None);
    assert_eq!(wake_step(&mut q, &wv(WAKE_QUIET_MAX_MS)), WakeStep::Whiten { ms: WAKE_WHITEN_MS });
}

/// ⛔ The way home goes out ONLY once the screen is fully white — earlier and the cut
/// from the arena to the town is on screen.
#[test]
fn the_way_home_goes_out_on_full_white() {
    let mut p = WakePhase::Whiten { white_at: 5_000 };
    assert_eq!(wake_step(&mut p, &wv(4_999)), WakeStep::None);
    assert!(!p.gone_home());
    assert_eq!(wake_step(&mut p, &wv(5_000)), WakeStep::GoHome);
    assert_eq!(p, WakePhase::Home { since: 5_000, landed: None });
    assert!(p.gone_home() && p.veiled());
}

/// The veil stays up until the town has landed AND the hold has run — the town's own
/// fade-in from black happens under it.
#[test]
fn the_veil_holds_until_the_landing_plus_the_hold() {
    let mut p = WakePhase::Home { since: 0, landed: None };
    assert_eq!(wake_step(&mut p, &wv(300)), WakeStep::None, "not landed yet");
    assert_eq!(wake_step(&mut p, &WakeView { home: true, ..wv(400) }), WakeStep::None);
    assert_eq!(p, WakePhase::Home { since: 0, landed: Some(400) });
    // The landing is latched: a later poll that cannot see it does not undo it.
    assert_eq!(wake_step(&mut p, &wv(400 + WAKE_HOLD_MS - 1)), WakeStep::None);
    assert_eq!(wake_step(&mut p, &wv(400 + WAKE_HOLD_MS)), WakeStep::Clear { ms: WAKE_CLEAR_MS });
    assert_eq!(p, WakePhase::Clear { clear_at: 400 + WAKE_HOLD_MS + u64::from(WAKE_CLEAR_MS) });
}

/// A landing that never comes does not leave the player staring at white.
#[test]
fn a_landing_that_never_comes_is_given_up() {
    let mut p = WakePhase::Home { since: 100, landed: None };
    assert_eq!(wake_step(&mut p, &wv(100 + WAKE_ARRIVE_MAX_MS - 1)), WakeStep::None);
    assert_eq!(wake_step(&mut p, &wv(100 + WAKE_ARRIVE_MAX_MS)), WakeStep::Clear { ms: WAKE_CLEAR_MS });
    // And nothing posted at all goes straight to the clear.
    let mut q = WakePhase::Home { since: 0, landed: None };
    assert_eq!(q.home_failed(50), WakeStep::Clear { ms: WAKE_CLEAR_MS });
    assert_eq!(q, WakePhase::Clear { clear_at: 50 + u64::from(WAKE_CLEAR_MS) });
}

/// A destination that cannot say the line (not an entrance town) gets it from the host
/// the moment the veil is gone; one that can waits for the game.
#[test]
fn the_host_says_it_only_where_the_game_cannot() {
    let mut p = WakePhase::Clear { clear_at: 1_000 };
    assert_eq!(wake_step(&mut p, &wv(999)), WakeStep::None);
    assert_eq!(wake_step(&mut p, &wv(1_000)), WakeStep::SayOnHost, "no flag was raised");
    let mut q = WakePhase::Clear { clear_at: 1_000 };
    assert_eq!(wake_step(&mut q, &WakeView { flag_armed: true, flag: Some(0), ..wv(1_000) }), WakeStep::None);
    assert_eq!(q, WakePhase::Say { since: 1_000, spoken: false });
}

/// The game said it: the wake ends when the player has closed the window, and what it
/// held back (the exit reward's box) comes after, not on top of the line.
#[test]
fn the_games_line_ends_the_wake_when_it_is_closed() {
    let armed = |now: u64, open: bool| WakeView { flag_armed: true, flag: Some(0), script_open: open, ..wv(now) };
    let mut p = WakePhase::Say { since: 0, spoken: false };
    assert_eq!(wake_step(&mut p, &armed(500, false)), WakeStep::None, "not yet said");
    assert_eq!(wake_step(&mut p, &armed(900, true)), WakeStep::None, "being read");
    assert_eq!(p, WakePhase::Say { since: 0, spoken: true });
    assert_eq!(wake_step(&mut p, &armed(60_000, true)), WakeStep::None, "still being read");
    assert_eq!(wake_step(&mut p, &armed(60_100, false)), WakeStep::Done);
    // A window left open forever does not hold the reward's box forever.
    let mut q = WakePhase::Say { since: 0, spoken: true };
    assert_eq!(wake_step(&mut q, &armed(WAKE_READ_MAX_MS, true)), WakeStep::Done);
}

/// ⛔ A line never seen: said on the host only if the flag is still up (the entry row
/// never ran it). A consumed flag means the game DID say it between two polls — saying
/// it again would be the double the rule exists to prevent.
#[test]
fn an_unseen_line_is_said_on_the_host_only_if_the_flag_is_still_up() {
    let up = WakeView { flag_armed: true, flag: Some(1), ..wv(WAKE_SAY_MAX_MS) };
    let mut p = WakePhase::Say { since: 0, spoken: false };
    assert_eq!(wake_step(&mut p, &WakeView { now: WAKE_SAY_MAX_MS - 1, ..up }), WakeStep::None);
    assert_eq!(wake_step(&mut p, &up), WakeStep::SayOnHost);
    let mut q = WakePhase::Say { since: 0, spoken: false };
    assert_eq!(wake_step(&mut q, &WakeView { flag: Some(0), ..up }), WakeStep::Done);
    // Unreadable is not "consumed": the host says it rather than lose the line.
    let mut r = WakePhase::Say { since: 0, spoken: false };
    assert_eq!(wake_step(&mut r, &WakeView { flag: None, ..up }), WakeStep::SayOnHost);
}

/// The whole wake, end to end, on a 250-ms poll: a box closed at 2 s, the town landing
/// the poll after the way home, the line opening 3 s after the veil is gone and closed
/// 2 s later.
#[test]
fn the_wake_runs_start_to_finish() {
    let mut p = WakePhase::start(0);
    let mut steps = vec![];
    let mut landed = false;
    let mut said_at: Option<u64> = None;
    let mut now = 0u64;
    while now < 60_000 {
        let open = said_at.is_some_and(|t| now >= t && now < t + 2_000);
        let v = WakeView { now, quiet: now >= 2_000, home: landed, script_open: open, flag_armed: true, flag: Some(u8::from(!landed)) };
        let s = wake_step(&mut p, &v);
        match s {
            WakeStep::GoHome => landed = true,
            WakeStep::Clear { ms } => said_at = Some(now + u64::from(ms) + 3_000),
            _ => {}
        }
        if s != WakeStep::None {
            steps.push(s);
        }
        if matches!(s, WakeStep::Done | WakeStep::SayOnHost) {
            break;
        }
        now += 250;
    }
    assert_eq!(
        steps,
        vec![WakeStep::Whiten { ms: WAKE_WHITEN_MS }, WakeStep::GoHome, WakeStep::Clear { ms: WAKE_CLEAR_MS }, WakeStep::Done]
    );
    assert!(said_at.is_some_and(|t| now >= t + 2_000), "done only after the line was closed");
}

/// The top floor is the one state the wake takes over, and the table's arenas are known
/// scenes even with no run going (a pre-143차 save made inside one).
#[test]
fn the_top_floor_and_the_floor_scenes_are_recognised() {
    let mut run = Run::new(vec![plain(1, 243), Floor { boss: true, ..plain(FLOORS, 246) }]);
    assert!(run.is_floor_scene(243) && run.is_floor_scene(246));
    assert!(!run.is_floor_scene(111) && !run.is_floor_scene(5), "the entrance towns are not the tower");
    assert!(!run.at_the_top());
    run.enter(FLOORS, &on(111, 0, 0));
    run.poll(&on(246, 9, 9));
    assert!(!run.at_the_top(), "a live floor 100 is not finished");
    assert_eq!(run.poll(&on(246, 9, 0)), Action::Reward { floor: FLOORS, boss: true });
    assert!(run.at_the_top());
    // The wake ends it through the ordinary leave (the exit reward, the way home, finish).
    assert_eq!(run.next_floor(&on(246, 9, 0)), Action::Leave { why: Abort::Completed });
    assert!(!run.at_the_top());
    // An ordinary cleared floor is not the top.
    let mut low = Run::new(vec![plain(1, 243), plain(2, 244)]);
    low.enter(1, &on(111, 0, 0));
    low.poll(&on(243, 3, 3));
    low.poll(&on(243, 3, 0));
    assert!(!low.at_the_top());
}

/// The fallback home is each hero's entrance town, one step in front of the NPC.
#[test]
fn the_fallback_home_is_the_heros_entrance_town() {
    assert_eq!(fallback_home(Some(Hero::Ritz)), (111, (31, 19)));
    assert_eq!(fallback_home(Some(Hero::Kei)), (5, (8, 9)));
    assert_eq!(fallback_home(None), (111, (31, 19)), "unknown = the first entrance");
}

/// ★ 143차: the `clear` test lever parses, and loses to `leave` and wins over `floor` —
/// a file is read once, and the floor a clear would act on is not loaded at entry.
#[test]
fn the_clear_lever_parses_and_leave_still_wins() {
    assert_eq!(parse_command("clear\n"), Some(Command::Clear));
    assert_eq!(parse_command("kill # same thing\n"), Some(Command::Clear));
    assert_eq!(parse_command("clear\nleave\n"), Some(Command::Leave));
    assert_eq!(parse_command("floor 100\nforce 1\nclear\n"), Some(Command::Clear));
    assert_eq!(parse_command("floor 100\nforce 1\n"), Some(Command::Enter { floor: 100, force: true }));
    assert_eq!(parse_command("# nothing\n"), None);
}
