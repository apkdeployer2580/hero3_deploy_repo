// 세이브 슬롯 창 (130차, 2026-09-23) — TWELVE save slots (`TOTAL`) behind the game's THREE-row list.
//
// The game has exactly three slots and that number is not one constant: it is the length
// of five arrays that sit packed between other live fields (`SAVE_SLOTS_FEASIBILITY.md
// §3`), and the slot list is a three-row painter with three literal row labels. So the
// guest keeps its three rows and the HOST slides a WINDOW over the real slots:
//
// ```text
//   real slot  r = first + g          g = the guest's row 0..2, first = 0..FIRST_MAX
// ```
//
// Three things make that a window and not a lie:
//
// 1. **Record names are remapped at the record store** ([`remap_name`], applied in
//    `database.rs::get_path_for_database`). Every read, write and delete the guest does
//    goes through `sprintf("%s_%d")` → `MC_dbOpenDataBase` / `DeleteRecord` — three guest
//    functions, one host function — so `GameSave_11_<g>` on the guest's side IS
//    `GameSave_11_<first+g>` on disk. With `first == 0` the map is the identity and
//    nothing about the three original slots changes.
// 2. **The guest's own slot arrays are refilled by the host** when the window moves
//    ([`row_writes`], [`label_writes`]): the scan that fills them (`0x5c06`) only runs on
//    `setAppState(3)`, so after a slide the occupancy byte, class, level, the u16 and the
//    play-time u64 of each row are written exactly the way the guest's own store routine
//    (`0x1a734`) writes them, from the same 16-byte summary body the guest would have
//    decrypted. The three `"SLOT n"` label cells are 8 bytes each and are rewritten too.
// 3. **The pin and every save-slot policy stay in REAL-slot space.** The only places that
//    read the guest's cursor or the `04 3d` slot byte as a slot IDENTITY convert with
//    [`real_slot`] before anything else sees the value (`items::live_save_slot`,
//    `items::summary_repair_gate`, `network.rs`). Places that use the cursor as an ARRAY
//    INDEX (the autoloot flag, the guest's own play-time counter) keep the guest value.
//
// ⛔ The window moves ONLY on the two slot-list screens (shell screens 2 and 9) and only
// while no modal is up — the worker's gate. A character is never loaded from anywhere
// else, so `first` cannot change under a loaded character, and the drifted-cursor
// machinery (`savevault::redirect_slot`) covers a drifted window exactly as it covers a
// drifted cursor, because both reach it as one real slot number.
//
// ⛔ The guest's UP/DOWN on those screens are NOT forwarded: their handlers write nothing
// but the cursor word (`0xaee8` / `0xa8f0`, verified), so the host owns the cursor there
// outright — [`on_key`] — and no half-applied key can make the guest's own wrap fire. A
// forwarded press would be read by the guest a tick later than the host's decision, and
// two fast presses would then wrap the guest's three rows instead of sliding the window.
//
// This file is dependency-free (`slotwindow_selftest.rs` `include!`s it).

/// Real slots the host offers. `GameSave_11_0` … `GameSave_11_11` on disk (user, 2026-09-23:
/// "slot 12개로 만들어주고"). ⛔ Two digits at most: the label cell is 8 bytes (`"SLOT 12\0"`
/// fills it) and the guest's box hook (`wie_core_arm::binary_patches::slotlabel`) widens
/// the label's box for one extra digit only.
pub const TOTAL: u8 = 12;
/// Rows the guest draws — the width of the window. The game's own number; not a knob.
pub const VISIBLE: u8 = 3;
/// The highest row index the guest's cursor can hold.
pub const GUEST_SLOT_MAX: u8 = VISIBLE - 1;
/// The highest `first` — the window then shows the last three real slots.
pub const FIRST_MAX: u8 = TOTAL - VISIBLE;
/// The highest real slot number.
pub const REAL_SLOT_MAX: u8 = TOTAL - 1;

// ── the guest's slot arrays, as CHAR offsets (LGT; `base = CHAR + 0x28 = getBase()`) ──
//
// Measured on the shipped `binary.mod` (`SAVE_SLOTS_FEASIBILITY.md §3`): the store routine
// `0x1a734(buf, slot)` writes `buf[0..8]` to `base+4+slot*8`, `buf[8]` to `base+0xa3ef+slot`,
// `buf[9]` to `base+0xa3f8+slot` and `buf[10..12]` to `base+0xa3f2+slot*2`; the scan sets
// `base+0xa3ec+slot = 1`. The 초기화 arm (`0xc02c`) clears the occupancy byte and the u64.
// Probe-confirmed after a host save into slot 0 and a reload: occupancy `[1,0,0]`, class
// `[5,0,0]`, level `[1,0,0]`, play time `[16420,0,0]` ms = the row's `PLAY TIME 0:0:16`.

/// `getBase()` relative to CHAR.
pub const BASE_OFF: u32 = 0x28;
/// Play time in ms, u64 LE, `+ g*8`. The guest's own in-play counter lives HERE too,
/// indexed by the cursor — which is why the cursor stays a guest-space index.
pub const PLAYTIME_OFF: u32 = BASE_OFF + 0x4;
/// 1 = the row shows a character, `+ g`. Set-only on the guest's side.
pub const OCCUPIED_OFF: u32 = BASE_OFF + 0xa3ec;
/// The summary's `buf[8]` (class), `+ g`.
pub const CLASS_OFF: u32 = BASE_OFF + 0xa3ef;
/// The summary's `buf[10..12]` (u16 LE), `+ g*2`.
pub const EXTRA_OFF: u32 = BASE_OFF + 0xa3f2;
/// The summary's `buf[9]` (level), `+ g`.
pub const LEVEL_OFF: u32 = BASE_OFF + 0xa3f8;
/// The slot-list cursor, u32 (= `CHAR+0xa410`, `SaveChain::slot_off`).
pub const CURSOR_OFF: u32 = BASE_OFF + 0xa3e8;
/// Each `"SLOT n"` label is one 8-byte cell (`0x5c2c4 "SLOT 1\0\0"`, `0x5c2cc`, `0x5c2d4`).
pub const LABEL_CELL: usize = 8;
/// Bytes of the summary body this file understands (the guest allocates 16).
pub const SUMMARY_BODY_MIN: usize = 12;

/// `first` clamped into the window's range. A file from a bigger build degrades to the
/// last window, never to rows that do not exist.
pub fn clamp_first(first: u8) -> u8 {
    first.min(FIRST_MAX)
}

/// The real slot behind the guest's row `guest`, or `None` when `guest` is not a row.
pub fn real_slot(first: u8, guest: u8) -> Option<u8> {
    (guest <= GUEST_SLOT_MAX).then(|| clamp_first(first) + guest)
}

/// The guest row that shows real slot `real` in the current window, if any.
pub fn guest_slot(first: u8, real: u8) -> Option<u8> {
    let first = clamp_first(first);
    (real >= first && real < first + VISIBLE).then(|| real - first)
}

/// The record name the guest asked for, renamed to the real slot — `None` when the name
/// is not a slot record or the map is the identity (`first == 0`).
///
/// Only the two record families the guest names by slot: `GameSave_<build>_<g>` and
/// `SlotSave_<build>_<g>` with `g` a single digit `0..=VISIBLE-1`. `OptionSave_11`, a name
/// with a path separator, a two-digit tail or a tail outside the guest's rows all pass
/// through untouched — a real-slot name (`GameSave_11_7`) can only come from the host, and
/// the host never asks the repository through this door.
pub fn remap_name(first: u8, name: &str) -> Option<String> {
    let first = clamp_first(first);
    if first == 0 || name.contains(['/', '\\']) {
        return None;
    }
    let family = ["GameSave_", "SlotSave_"].into_iter().find(|p| name.starts_with(p))?;
    let rest = &name[family.len()..];
    let (build, tail) = rest.rsplit_once('_')?;
    if build.is_empty() || !build.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let g: u8 = match tail.as_bytes() {
        [d @ b'0'..=b'9'] => d - b'0',
        _ => return None,
    };
    let real = real_slot(first, g)?;
    Some(format!("{family}{build}_{real}"))
}

/// What a host-owned UP/DOWN on the slot list does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMove {
    /// The cursor moves within the window; the window stays.
    Move { cursor: u8 },
    /// The window slides one row; the cursor stays on the edge row it is on.
    Slide { first: u8 },
    /// Past the last (or first) real slot: the list wraps to the other end, like the
    /// game's own three-row list wraps.
    Wrap { first: u8, cursor: u8 },
}

/// The guest's own arm does `cursor ± 1` with a wrap inside 0..=2 and nothing else; this
/// is that arm with the wrap widened to the whole real list. `None` when the inputs are
/// not a row/window pair — then the key is not ours to answer.
pub fn on_key(first: u8, cursor: u8, up: bool) -> Option<KeyMove> {
    if cursor > GUEST_SLOT_MAX || first > FIRST_MAX {
        return None;
    }
    Some(if up {
        if cursor > 0 {
            KeyMove::Move { cursor: cursor - 1 }
        } else if first > 0 {
            KeyMove::Slide { first: first - 1 }
        } else {
            KeyMove::Wrap {
                first: FIRST_MAX,
                cursor: GUEST_SLOT_MAX,
            }
        }
    } else if cursor < GUEST_SLOT_MAX {
        KeyMove::Move { cursor: cursor + 1 }
    } else if first < FIRST_MAX {
        KeyMove::Slide { first: first + 1 }
    } else {
        KeyMove::Wrap { first: 0, cursor: 0 }
    })
}

/// The host's ▲/▼ arrows slide the window by one row and keep the cursor's row; they do
/// not wrap. `None` = already at that end (nothing to do, nothing to redraw).
pub fn on_arrow(first: u8, delta: i8) -> Option<u8> {
    let first = clamp_first(first);
    let next = match delta {
        d if d < 0 => first.checked_sub(1)?,
        d if d > 0 => first + 1,
        _ => return None,
    };
    (next <= FIRST_MAX).then_some(next)
}

/// The decrypted 16-byte summary body (`slotSummary 0x1a6d0`): `[u64 play time ms]
/// [u8 class = CHAR+0xb6][u8 level = CHAR+0xe2][i16 CHAR+0xb4][4 pad]`.
///
/// ⛔⛔ THE U64 IS STORED HIGH WORD FIRST. `0x1be94(lo, hi, buf, off)` writes the HIGH
/// 32 bits at `buf+off` and the LOW 32 bits at `buf+off+4`, each little-endian, and the
/// loader `0x1be58` reads them back the same way — so on disk the body is
/// `[hi LE][lo LE]`, while the guest's ARRAY holds a plain little-endian u64 (`[lo][hi]`).
/// Device-caught on the first S9 build (vc391): a plain LE read turned a 40-hour play
/// time into ~70 000 000 000 000 ms and the row printed `PLAY TIME -3411:-17:-27` over its
/// own label. [`row_writes`] writes the array form; this parses the disk form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub playtime_ms: u64,
    pub class: u8,
    pub level: u8,
    pub extra: u16,
}

pub fn parse_summary(body: &[u8]) -> Option<Summary> {
    if body.len() < SUMMARY_BODY_MIN {
        return None;
    }
    let hi = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
    let lo = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
    Some(Summary {
        playtime_ms: (u64::from(hi) << 32) | u64::from(lo),
        class: body[8],
        level: body[9],
        extra: u16::from_le_bytes([body[10], body[11]]),
    })
}

/// One guest write: `(address, bytes)`.
pub type GuestWrite = (u32, Vec<u8>);

/// The five stores the guest's own scan + store routine would have made for row `g`, or
/// the 초기화 arm's clears (occupancy 0, play time 0) plus zeroed class/level/extra when
/// the real slot behind the row holds no readable summary.
pub fn row_writes(char_base: u32, g: u8, s: Option<&Summary>) -> Vec<GuestWrite> {
    let g = u32::from(g.min(GUEST_SLOT_MAX));
    let (occ, pt, class, level, extra) = match s {
        Some(s) => (1u8, s.playtime_ms, s.class, s.level, s.extra),
        None => (0u8, 0u64, 0u8, 0u8, 0u16),
    };
    vec![
        (char_base + OCCUPIED_OFF + g, vec![occ]),
        (char_base + PLAYTIME_OFF + g * 8, pt.to_le_bytes().to_vec()),
        (char_base + CLASS_OFF + g, vec![class]),
        (char_base + LEVEL_OFF + g, vec![level]),
        (char_base + EXTRA_OFF + g * 2, extra.to_le_bytes().to_vec()),
    ]
}

/// The label cell for row `g` of window `first`: `"SLOT n"` NUL-padded to the cell.
/// `n` is the 1-based REAL slot, so the row itself says which slot it is.
pub fn label(first: u8, g: u8) -> [u8; LABEL_CELL] {
    let n = u32::from(clamp_first(first)) + u32::from(g.min(GUEST_SLOT_MAX)) + 1;
    let text = format!("SLOT {n}");
    let mut cell = [0u8; LABEL_CELL];
    // ⛔ The cell is 8 bytes and the string needs its NUL: a two-digit label ("SLOT 12") is
    // the widest that fits (7 + 1). `TOTAL` stays two digits or the labels stay put.
    let n = text.len().min(LABEL_CELL - 1);
    cell[..n].copy_from_slice(&text.as_bytes()[..n]);
    cell
}

/// All three label cells of window `first`, contiguous — what the guest image should
/// hold at the label base.
pub fn expected_labels(first: u8) -> [u8; LABEL_CELL * VISIBLE as usize] {
    let mut out = [0u8; LABEL_CELL * VISIBLE as usize];
    for g in 0..VISIBLE {
        let cell = label(first, g);
        let at = g as usize * LABEL_CELL;
        out[at..at + LABEL_CELL].copy_from_slice(&cell);
    }
    out
}

pub fn label_writes(label_base: u32, first: u8) -> Vec<GuestWrite> {
    (0..VISIBLE)
        .map(|g| (label_base + u32::from(g) * LABEL_CELL as u32, label(first, g).to_vec()))
        .collect()
}

/// The cursor word — written only on a wrap (a slide keeps the guest's own value).
pub fn cursor_write(char_base: u32, cursor: u8) -> GuestWrite {
    (char_base + CURSOR_OFF, u32::from(cursor.min(GUEST_SLOT_MAX)).to_le_bytes().to_vec())
}

/// `wie_save_first.txt`: one line, `first <n>`. Anything else reads as window 0 — the
/// three original slots, i.e. every build before this one.
pub fn parse_first(s: &str) -> u8 {
    let s = s.trim();
    let n = s.strip_prefix("first").map(str::trim).unwrap_or(s);
    n.parse::<u8>().map_or(0, clamp_first)
}

pub fn serialize_first(first: u8) -> String {
    format!("first {}\n", clamp_first(first))
}

/// Where the slot list lands when it OPENS (161차, 2026-10-03): `target`, the real slot the
/// player last loaded, shown and under the cursor — `(first, row)`.
///
/// The game's own 게임시작 / 온라인 picks zero the row as they set the screen word, so every
/// list used to open on row 0 of wherever the window was left. The window stays put when
/// the slot is already in view (the player scrolled there — reopening in the same place is
/// what "it remembers" looks like); out of view it slides the shortest way: a slot above
/// the window becomes the top row, one below becomes the bottom row, exactly where the
/// player's own UP / DOWN presses would have left it. A `target` beyond the last real slot
/// is the last real slot — never a row that does not exist.
pub fn focus_target(first: u8, target: u8) -> (u8, u8) {
    let first = clamp_first(first);
    let target = target.min(REAL_SLOT_MAX);
    if let Some(g) = guest_slot(first, target) {
        (first, g)
    } else if target < first {
        (target, 0)
    } else {
        (clamp_first(target - GUEST_SLOT_MAX), GUEST_SLOT_MAX)
    }
}

/// `wie_save_last.txt`: one line, `last <n>` — the real slot the player last loaded, kept
/// so the list opens on it across an app restart too. `None` when nothing was ever loaded,
/// the file is garbage, or the slot is one this build does not have (a bigger build's
/// file): then the list opens where the game itself puts it, as every build before this one.
pub fn parse_last(s: &str) -> Option<u8> {
    let s = s.trim();
    let n = s.strip_prefix("last").map(str::trim).unwrap_or(s);
    n.parse::<u8>().ok().filter(|n| *n <= REAL_SLOT_MAX)
}

pub fn serialize_last(slot: u8) -> String {
    format!("last {}\n", slot.min(REAL_SLOT_MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's `SlotSave_11_0` after a host save into slot 0 (LV1 버서커, 16 s), as
    /// DECRYPTED FROM DISK — `[hi][lo]` of the play time, then class 5, level 1. The
    /// guest's arrays hold the same play time as a plain LE u64 (`24 40 00 00 00 00 00 00`,
    /// the probe's `--dump-mem` at `base+4`): the two forms differ, and the device build
    /// that confused them is why this fixture is the disk form.
    const PROBE_BODY: [u8; 16] = [0, 0, 0, 0, 0x24, 0x40, 0, 0, 0x05, 0x01, 0, 0, 0, 0, 0, 0];
    const CHAR: u32 = 0x1503a88;
    const LABELS: u32 = 0x5c2c4;

    #[test]
    fn the_constants_are_the_probe_measured_addresses() {
        // `base+0xa3ec` = 0x150de9c, cursor `base+0xa3e8` = 0x150de98 (= CHAR+0xa410),
        // play time `base+4` = 0x1503ab4 — the three `--dump-mem` bases of the probe run.
        assert_eq!(CHAR + OCCUPIED_OFF, 0x150de9c);
        assert_eq!(CHAR + CURSOR_OFF, 0x150de98);
        assert_eq!(CHAR + CURSOR_OFF, CHAR + 0xa410, "the cursor is SaveChain::slot_off");
        assert_eq!(CHAR + PLAYTIME_OFF, 0x1503ab4);
        assert_eq!(CHAR + CLASS_OFF, 0x150de9f);
        assert_eq!(CHAR + EXTRA_OFF, 0x150dea2);
        assert_eq!(CHAR + LEVEL_OFF, 0x150dea8);
        assert_eq!(FIRST_MAX, 9);
        assert_eq!(REAL_SLOT_MAX, 11);
    }

    #[test]
    fn window_zero_is_the_identity_everywhere() {
        for g in 0..VISIBLE {
            assert_eq!(real_slot(0, g), Some(g));
            assert_eq!(guest_slot(0, g), Some(g));
        }
        for name in ["GameSave_11_0", "GameSave_11_1", "GameSave_11_2", "SlotSave_11_2", "OptionSave_11"] {
            assert_eq!(remap_name(0, name), None, "{name}: nothing is renamed on window 0");
        }
        assert_eq!(expected_labels(0), *b"SLOT 1\0\0SLOT 2\0\0SLOT 3\0\0", "the shipped image's own cells");
        assert_eq!(parse_first(""), 0);
        assert_eq!(parse_first("garbage"), 0);
    }

    #[test]
    fn real_and_guest_slots_are_inverses_inside_the_window() {
        for first in 0..=FIRST_MAX {
            for g in 0..VISIBLE {
                let r = real_slot(first, g).unwrap();
                assert!(r <= REAL_SLOT_MAX);
                assert_eq!(guest_slot(first, r), Some(g));
            }
            assert_eq!(real_slot(first, VISIBLE), None, "row 3 does not exist");
            if first > 0 {
                assert_eq!(guest_slot(first, first - 1), None, "the slot just above the window is not shown");
            }
            assert_eq!(guest_slot(first, first + VISIBLE), None);
        }
        // A `first` from a bigger build's file clamps to the last window.
        assert_eq!(real_slot(200, 0), Some(FIRST_MAX));
        assert_eq!(clamp_first(255), FIRST_MAX);
    }

    #[test]
    fn record_names_are_renamed_only_for_the_guest_rows() {
        assert_eq!(remap_name(3, "GameSave_11_0").as_deref(), Some("GameSave_11_3"));
        assert_eq!(remap_name(3, "GameSave_11_2").as_deref(), Some("GameSave_11_5"));
        assert_eq!(remap_name(7, "SlotSave_11_2").as_deref(), Some("SlotSave_11_9"));
        assert_eq!(remap_name(9, "GameSave_11_2").as_deref(), Some("GameSave_11_11"), "a two-digit real slot");
        // KTF's build number is different; the rule is the family + a numeric build.
        assert_eq!(remap_name(1, "GameSave_7_1").as_deref(), Some("GameSave_7_2"));
        // Not slot records, or not the guest's rows: untouched.
        for name in [
            "OptionSave_11",
            "GameSave_11_7",  // a real-slot name can only be the host's own
            "GameSave_11_10", // two digits
            "GameSave_11",    // no slot tail
            "GameSave_x_1",   // build is not a number
            "db/GameSave_11_1",
            "GameSave_11_1/1",
            "gamesave_11_1",
        ] {
            assert_eq!(remap_name(3, name), None, "{name}");
        }
    }

    #[test]
    fn up_and_down_walk_the_whole_list_and_wrap_at_the_ends() {
        // Walk DOWN from (0,0) through every real slot: `TOTAL` presses come back to the start.
        let (mut first, mut cursor) = (0u8, 0u8);
        let mut seen = Vec::new();
        for _ in 0..TOTAL {
            seen.push(real_slot(first, cursor).unwrap());
            match on_key(first, cursor, false).unwrap() {
                KeyMove::Move { cursor: c } => cursor = c,
                KeyMove::Slide { first: f } => first = f,
                KeyMove::Wrap { first: f, cursor: c } => {
                    first = f;
                    cursor = c;
                }
            }
        }
        assert_eq!(seen, (0..TOTAL).collect::<Vec<_>>(), "DOWN visits every real slot once, in order");
        assert_eq!((first, cursor), (0, 0), "…and wraps back to the top");
        // And UP from the top goes to the bottom row of the last window.
        assert_eq!(on_key(0, 0, true), Some(KeyMove::Wrap { first: FIRST_MAX, cursor: GUEST_SLOT_MAX }));
        // Walking UP from there visits them in reverse.
        let (mut first, mut cursor) = (FIRST_MAX, GUEST_SLOT_MAX);
        let mut seen = Vec::new();
        for _ in 0..TOTAL {
            seen.push(real_slot(first, cursor).unwrap());
            match on_key(first, cursor, true).unwrap() {
                KeyMove::Move { cursor: c } => cursor = c,
                KeyMove::Slide { first: f } => first = f,
                KeyMove::Wrap { first: f, cursor: c } => {
                    first = f;
                    cursor = c;
                }
            }
        }
        assert_eq!(seen, (0..TOTAL).rev().collect::<Vec<_>>());
    }

    #[test]
    fn a_slide_keeps_the_cursor_on_its_edge_row_and_a_move_keeps_the_window() {
        // The row highlight stays on the bottom row while the list slides under it.
        assert_eq!(on_key(0, 2, false), Some(KeyMove::Slide { first: 1 }));
        assert_eq!(on_key(6, 2, false), Some(KeyMove::Slide { first: 7 }));
        assert_eq!(on_key(1, 0, true), Some(KeyMove::Slide { first: 0 }));
        // Inside the window nothing slides.
        assert_eq!(on_key(4, 1, false), Some(KeyMove::Move { cursor: 2 }));
        assert_eq!(on_key(4, 1, true), Some(KeyMove::Move { cursor: 0 }));
        // Not a row / not a window: not ours.
        assert_eq!(on_key(0, 3, false), None);
        assert_eq!(on_key(FIRST_MAX + 1, 0, false), None);
    }

    #[test]
    fn the_arrows_slide_without_wrapping() {
        assert_eq!(on_arrow(0, -1), None);
        assert_eq!(on_arrow(0, 1), Some(1));
        assert_eq!(on_arrow(FIRST_MAX, 1), None);
        assert_eq!(on_arrow(FIRST_MAX, -1), Some(FIRST_MAX - 1));
        assert_eq!(on_arrow(3, 0), None);
        assert_eq!(on_arrow(200, 1), None, "a clamped first is already at the end");
    }

    #[test]
    fn the_probe_summary_parses_and_writes_back_the_probe_arrays() {
        let s = parse_summary(&PROBE_BODY).unwrap();
        assert_eq!(s, Summary { playtime_ms: 16420, class: 5, level: 1, extra: 0 });
        let w = row_writes(CHAR, 0, Some(&s));
        assert_eq!(w[0], (0x150de9c, vec![1]), "occupancy");
        assert_eq!(w[1], (0x1503ab4, vec![0x24, 0x40, 0, 0, 0, 0, 0, 0]), "play time u64");
        assert_eq!(w[2], (0x150de9f, vec![5]), "class");
        assert_eq!(w[3], (0x150dea8, vec![1]), "level");
        assert_eq!(w[4], (0x150dea2, vec![0, 0]), "extra u16");
        // Row 2 sits one element further along each array (u64: 8, u16: 2).
        let w2 = row_writes(CHAR, 2, Some(&s));
        assert_eq!(w2[0].0, 0x150de9e);
        assert_eq!(w2[1].0, 0x1503ab4 + 16);
        assert_eq!(w2[4].0, 0x150dea2 + 4);
        // An empty real slot clears the row the way the 초기화 arm does (and then some).
        let e = row_writes(CHAR, 1, None);
        assert_eq!(e[0], (0x150de9d, vec![0]));
        assert_eq!(e[1], (0x1503ab4 + 8, vec![0; 8]));
        assert!(e[2..].iter().all(|(_, b)| b.iter().all(|x| *x == 0)));
        // Too short to be a summary.
        assert_eq!(parse_summary(&PROBE_BODY[..11]), None);
        // ⛔ The halves are SWAPPED on disk (vc391 device regression): a body that holds
        // the array form must NOT parse to the same play time.
        let array_form: [u8; 16] = [0x24, 0x40, 0, 0, 0, 0, 0, 0, 0x05, 0x01, 0, 0, 0, 0, 0, 0];
        assert_eq!(parse_summary(&array_form).unwrap().playtime_ms, 16420u64 << 32);
        // A 40-hour play time crosses nothing — hi stays 0 — but a 50-day one uses hi.
        let mut long = PROBE_BODY;
        long[..4].copy_from_slice(&1u32.to_le_bytes());
        long[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(parse_summary(&long).unwrap().playtime_ms, (1u64 << 32) | 2);
        assert_eq!(row_writes(CHAR, 0, parse_summary(&long).as_ref())[1].1, vec![2, 0, 0, 0, 1, 0, 0, 0], "written back as a plain LE u64");
    }

    #[test]
    fn every_label_fits_its_eight_byte_cell_with_its_nul() {
        for first in 0..=FIRST_MAX {
            for g in 0..VISIBLE {
                let cell = label(first, g);
                let text = std::str::from_utf8(&cell).unwrap().trim_end_matches('\0');
                assert_eq!(text, format!("SLOT {}", first + g + 1));
                assert!(text.len() < LABEL_CELL, "{text}: the NUL needs its byte");
                assert_eq!(cell[text.len()], 0);
            }
        }
        assert_eq!(&expected_labels(FIRST_MAX)[16..], b"SLOT 12\0", "the widest label, exactly full");
        let w = label_writes(LABELS, 7);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].0, 0x5c2c4);
        assert_eq!(w[1].0, 0x5c2cc);
        assert_eq!(w[2].0, 0x5c2d4);
        assert_eq!(w[2].1, b"SLOT 10\0");
        assert_eq!(label_writes(LABELS, FIRST_MAX)[2].1, b"SLOT 12\0");
        assert_eq!(cursor_write(CHAR, 2), (0x150de98, vec![2, 0, 0, 0]));
        assert_eq!(cursor_write(CHAR, 9).1, vec![2, 0, 0, 0], "the cursor never leaves the guest's rows");
    }

    #[test]
    fn the_file_round_trips_and_garbage_reads_as_window_zero() {
        for f in 0..=FIRST_MAX {
            assert_eq!(parse_first(&serialize_first(f)), f);
        }
        assert_eq!(parse_first("first 7\r\n"), 7);
        assert_eq!(parse_first("first 99"), FIRST_MAX, "clamped, not refused");
        assert_eq!(parse_first("7"), 7, "a bare number is accepted too");
        assert_eq!(parse_first("first -1"), 0);
        assert_eq!(parse_first("first"), 0);
    }

    /// 161차: the list opens on the last loaded slot — the window moves only when it must.
    #[test]
    fn the_list_opens_on_the_last_loaded_slot_and_slides_only_when_it_must() {
        // In view: the window stays, only the row moves (SLOT 5 at the bottom of 3..5).
        assert_eq!(focus_target(2, 4), (2, 2));
        assert_eq!(focus_target(2, 2), (2, 0));
        assert_eq!(focus_target(0, 1), (0, 1));
        // Below the window: the slot becomes the bottom row, as DOWN would have left it.
        assert_eq!(focus_target(0, 4), (2, 2));
        assert_eq!(focus_target(0, 11), (FIRST_MAX, GUEST_SLOT_MAX));
        // Above the window: the slot becomes the top row, as UP would have left it.
        assert_eq!(focus_target(5, 1), (1, 0));
        assert_eq!(focus_target(9, 0), (0, 0));
        // Garbage in: the last real slot / the last window, never a row that does not exist.
        assert_eq!(focus_target(0, 200), (FIRST_MAX, GUEST_SLOT_MAX));
        assert_eq!(focus_target(200, 4), (4, 0), "a clamped first is the last window, so SLOT 5 is above it");
        // Every (window, slot) pair ends with THAT slot under the cursor, in a real window.
        for first in 0..=FIRST_MAX {
            for target in 0..TOTAL {
                let (f, g) = focus_target(first, target);
                assert!(f <= FIRST_MAX && g <= GUEST_SLOT_MAX, "window {first}, slot {target} -> ({f}, {g})");
                assert_eq!(real_slot(f, g), Some(target), "window {first}, slot {target}");
                if guest_slot(first, target).is_some() {
                    assert_eq!(f, first, "a slot already in view never moves the window");
                }
            }
        }
    }

    #[test]
    fn the_last_slot_file_round_trips_and_refuses_what_is_not_a_slot() {
        for s in 0..TOTAL {
            assert_eq!(parse_last(&serialize_last(s)), Some(s));
        }
        assert_eq!(parse_last("last 4\r\n"), Some(4));
        assert_eq!(parse_last("4"), Some(4), "a bare number is accepted too");
        assert_eq!(parse_last(""), None);
        assert_eq!(parse_last("garbage"), None);
        assert_eq!(parse_last("last"), None);
        assert_eq!(parse_last("last 12"), None, "a slot this build does not have is not focused");
        assert_eq!(parse_last("last -1"), None);
        assert_eq!(serialize_last(99), "last 11\n", "clamped on the way out");
    }
}
