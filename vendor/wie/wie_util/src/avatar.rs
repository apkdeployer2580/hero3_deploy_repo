// ★★★ 아바타 옷장 (167차, 2026-10-06) — the numbers the two guest hooks and the host agree on.
//
// A COSTUME SET is a number `1..=SET_MAX`. Its pieces are ordinary paper-doll sheets the asset
// carries under ONE sprite number per set, [`sheet_sprite`] `= 49 + set` (50..=99 — the shipped
// sheets use 0..=22, and the game's file-name builder `0x19dc8` writes the number as two
// digits up to 99):
//
// | slot | layer | jar member |
// |---|---|---|
// | 투구 helmet | 6 | `res/hero/h600NN_bm` |
// | 갑옷 armour | 1 | `res/hero/h100NN_bm` (the torso, 3 frames) |
// | 장갑 gloves | 2 | `res/hero/h200NN_bm` (the arms, 12 frames) |
// | 신발 shoes | 3 | `res/hero/h300NN_bm` (the legs, 18 frames) |
// | 무기 weapon | 5 | `res/hero/h5ccNN_bm`, `cc` = the EQUIPPED weapon's category 04..10 |
// | 방패 shield | 7 | `res/hero/h511NN_bm` |
//
// "Layer" is the number both guest paths use for those six — the layer setter's KIND
// (`0x517bc`, joined at `0x5191a`) and the world-init loader's LAYER index (`0x4957c`, joined at
// `0x498c0`) agree on all six (`binary_patches/avatar.rs`).
//
// WHICH set a slot shows lives in the game's own script-flag array (`base+0x9e98`, 256 bytes
// the save carries both ways): byte [`FLAG_WEAR`]` + slot`, `0` = no costume. Ownership is
// the bitmap at [`FLAG_OWN`]. So a costume saves, loads and rolls back exactly like the
// character — the user's rule (2026-10-06): 저장 안 하고 다시 불러오면 획득 취소.
//
// ⛔ The ids `0xb0..=0xbc` sit in the scripts' longest free run (`0x3d..=0xbf`, 167차 corpus
// scan: 106 ids in use, none there). `tools_re/patch_guardian_npc.py::flag_ids_in_use` is the
// scanner — re-run it if an asset change ever adds scripts.
//
// Dependency-free on purpose: `wie_android`'s standalone selftest `include!`s this file.

/// The six avatar slots, in the order of [`FLAG_WEAR`]'s bytes.
pub const SLOTS: usize = 6;
pub const SLOT_HELMET: usize = 0;
pub const SLOT_ARMOR: usize = 1;
pub const SLOT_GLOVES: usize = 2;
pub const SLOT_SHOES: usize = 3;
pub const SLOT_WEAPON: usize = 4;
pub const SLOT_SHIELD: usize = 5;

/// The highest set number. `49 + 50 = 99` is the last two-digit sheet number.
pub const SET_MAX: u8 = 50;
/// [`sheet_sprite`] = `SHEET_BASE + set`.
pub const SHEET_BASE: u8 = 49;

/// First of the six worn-set bytes in the script-flag array (`0xb0..=0xb5`).
pub const FLAG_WEAR: u8 = 0xb0;
/// First byte of the owned-set bitmap (`0xb6..=0xbc`): set `n` is bit `(n-1) % 8` of byte `(n-1) / 8`.
pub const FLAG_OWN: u8 = 0xb6;
/// Bytes in the owned bitmap — enough for [`SET_MAX`] sets.
pub const OWN_BYTES: usize = 7;

/// Sheet groups: helmet, armour, gloves, shoes, the seven weapon categories (4..=10), shield.
pub const GROUPS: usize = 12;
pub const GROUP_SHIELD: usize = 11;

/// Weapon categories that have a weapon layer (스피어 4 · 스워드 5 · 나이프 6 · 건 7 · 라이플 8 ·
/// 다크 9 · 홀리 10). The shield is category 11.
pub const WEAPON_CATS: core::ops::RangeInclusive<u32> = 4..=10;
pub const SHIELD_CAT: u32 = 11;

/// `EQUIPPED[i]` (CHAR `+0x38c + i*0x3c`) an avatar slot dresses: 투구 1 · 갑옷 4 · 장갑 3 ·
/// 신발 7 · 무기 0 · 방패 5. The layer setter's `slot` argument is this index.
pub const EQUIPPED_SLOT: [u8; SLOTS] = [1, 4, 3, 7, 0, 5];

/// Which avatar slot a paper-doll layer belongs to — the setter's KIND and the loader's LAYER
/// (they agree for these six). Layer 0 (head), 4 (weapon effect) and 9 (the sub weapon) are not
/// dressed.
pub fn slot_of_layer(layer: u32) -> Option<usize> {
    match layer {
        6 => Some(SLOT_HELMET),
        1 => Some(SLOT_ARMOR),
        2 => Some(SLOT_GLOVES),
        3 => Some(SLOT_SHOES),
        5 => Some(SLOT_WEAPON),
        7 => Some(SLOT_SHIELD),
        _ => None,
    }
}

/// The sheet group a slot draws from. The weapon's group is its CATEGORY's — a costume weapon
/// is drawn per weapon kind, because the game animates each kind with its own frame set. `cat`
/// is the category the game's name builder is about to use (only read for the weapon / shield).
pub fn group_of(slot: usize, cat: u32) -> Option<usize> {
    match slot {
        SLOT_HELMET => Some(0),
        SLOT_ARMOR => Some(1),
        SLOT_GLOVES => Some(2),
        SLOT_SHOES => Some(3),
        SLOT_WEAPON if WEAPON_CATS.contains(&cat) => Some(4 + (cat - 4) as usize),
        SLOT_SHIELD if cat == SHIELD_CAT => Some(GROUP_SHIELD),
        _ => None,
    }
}

/// The sprite number set `set`'s sheets carry, or `None` for 0 / out of range.
pub fn sheet_sprite(set: u8) -> Option<u8> {
    (1..=SET_MAX).contains(&set).then(|| SHEET_BASE + set)
}

/// The file-name digits of a group: `(layer digit, category)` — `h<digit><cat:02><sprite:02>_bm`.
pub fn group_name_parts(group: usize) -> Option<(u8, u8)> {
    match group {
        0 => Some((6, 0)),
        1 => Some((1, 0)),
        2 => Some((2, 0)),
        3 => Some((3, 0)),
        4..=10 => Some((5, group as u8)),
        GROUP_SHIELD => Some((5, SHIELD_CAT as u8)),
        _ => None,
    }
}

/// The jar member for `(group, set)`, written into `out` (`res/hero/hDCCSS_bm`, 18 bytes).
pub fn sheet_member(group: usize, set: u8, out: &mut [u8; 18]) -> Option<usize> {
    let (digit, cat) = group_name_parts(group)?;
    let sprite = sheet_sprite(set)?;
    let head = b"res/hero/h";
    out[..head.len()].copy_from_slice(head);
    let mut at = head.len();
    for d in [digit, cat / 10, cat % 10, sprite / 10, sprite % 10] {
        out[at] = b'0' + d;
        at += 1;
    }
    out[at..at + 3].copy_from_slice(b"_bm");
    Some(at + 3)
}

/// Read a jar member name back: `res/hero/hDCCSS_bm` -> `(group, set)` when it is a costume
/// sheet (sprite 50..=99 of one of the twelve groups). The host builds the availability masks
/// from the jar's own directory with this, so a costume the asset does not carry is never
/// drawn — the load of a missing sheet is the one thing the hooks must never cause.
pub fn parse_sheet_member(name: &str) -> Option<(usize, u8)> {
    let rest = name.strip_prefix("res/hero/h")?.strip_suffix("_bm")?;
    let b = rest.as_bytes();
    if b.len() != 5 || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let digit = b[0] - b'0';
    let cat = (b[1] - b'0') * 10 + (b[2] - b'0');
    let sprite = (b[3] - b'0') * 10 + (b[4] - b'0');
    let group = match (digit, cat) {
        (6, 0) => 0,
        (1, 0) => 1,
        (2, 0) => 2,
        (3, 0) => 3,
        (5, c) if (4..=10).contains(&c) => c as usize,
        (5, 11) => GROUP_SHIELD,
        _ => return None,
    };
    let set = sprite.checked_sub(SHEET_BASE)?;
    (1..=SET_MAX).contains(&set).then_some((group, set))
}

/// The availability masks of a jar: bit `set - 1` of `masks[group]` for every costume sheet
/// member in its central directory. Reads the directory in place (no allocation, no inflate);
/// a damaged or foreign archive reads as "no costumes", which is the safe answer — the hooks
/// then never change a layer.
pub fn sheets_in_zip(zip: &[u8]) -> [u64; GROUPS] {
    let mut masks = [0u64; GROUPS];
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    // End-of-central-directory: the last `PK\x05\x06` within the final 64 KiB + 22 bytes.
    let floor = zip.len().saturating_sub(22 + 0xffff);
    let Some(eocd) = (floor..zip.len().saturating_sub(21)).rev().find(|&i| u32_at(i) == Some(0x0605_4b50)) else {
        return masks;
    };
    let (Some(count), Some(mut at)) = (u16_at(eocd + 10), u32_at(eocd + 16).map(|v| v as usize)) else {
        return masks;
    };
    for _ in 0..count {
        if u32_at(at) != Some(0x0201_4b50) {
            break;
        }
        let (Some(n), Some(x), Some(c)) = (u16_at(at + 28), u16_at(at + 30), u16_at(at + 32)) else {
            break;
        };
        let (n, x, c) = (usize::from(n), usize::from(x), usize::from(c));
        if let Some(name) = zip.get(at + 46..at + 46 + n).and_then(|b| core::str::from_utf8(b).ok())
            && let Some((group, set)) = parse_sheet_member(name)
        {
            masks[group] |= 1 << (set - 1);
        }
        at += 46 + n + x + c;
    }
    masks
}

/// Is set `set` owned in the bitmap `own`?
pub fn owns(own: &[u8; OWN_BYTES], set: u8) -> bool {
    if !(1..=SET_MAX).contains(&set) {
        return false;
    }
    let i = usize::from(set - 1);
    own[i / 8] & (1 << (i % 8)) != 0
}

/// `own` with set `set` added (unchanged for an out-of-range set).
pub fn with_owned(mut own: [u8; OWN_BYTES], set: u8) -> [u8; OWN_BYTES] {
    if (1..=SET_MAX).contains(&set) {
        let i = usize::from(set - 1);
        own[i / 8] |= 1 << (i % 8);
    }
    own
}

/// What the two guest hooks decide, in one place: the sheet sprite to load instead of the
/// game's, or `None` to leave the game's choice alone.
///
/// * `worn` — the set this slot shows (the host's preview, else the flag byte); 0 = none.
/// * `has_sheet(group, set)` — the jar carries that sheet (the published masks).
///
/// ⛔ Never a sprite the jar lacks: `None` for a set with no sheet in this slot's group, so a
/// costume without, say, a 라이플 drawing leaves a 라이플 user's weapon exactly as equipped.
pub fn costume_sprite(slot: usize, cat: u32, worn: u8, has_sheet: impl Fn(usize, u8) -> bool) -> Option<u8> {
    let sprite = sheet_sprite(worn)?;
    let group = group_of(slot, cat)?;
    has_sheet(group, worn).then_some(sprite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_map_to_the_six_slots_and_nothing_else() {
        let got: [Option<usize>; 10] = core::array::from_fn(|l| slot_of_layer(l as u32));
        assert_eq!(got, [None, Some(1), Some(2), Some(3), None, Some(4), Some(0), Some(5), None, None]);
    }

    #[test]
    fn member_names_round_trip() {
        let mut buf = [0u8; 18];
        for group in 0..GROUPS {
            for set in [1u8, 7, SET_MAX] {
                let n = sheet_member(group, set, &mut buf).unwrap();
                let name = core::str::from_utf8(&buf[..n]).unwrap();
                assert_eq!(parse_sheet_member(name), Some((group, set)), "{name}");
            }
        }
        let n = sheet_member(0, 1, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"res/hero/h60050_bm");
        let n = sheet_member(4 + 2, 3, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"res/hero/h50652_bm", "a 나이프 sheet");
        let n = sheet_member(GROUP_SHIELD, 50, &mut buf).unwrap();
        assert_eq!(&buf[..n], b"res/hero/h51199_bm");
    }

    #[test]
    fn shipped_sheets_are_never_costumes() {
        // The game's own sheets: 0..=22 per group, the heads, the weapon effects.
        for name in ["res/hero/h10022_bm", "res/hero/h60016_bm", "res/hero/h50909_bm", "res/hero/h51112_bm",
            "res/hero/h00003_bm", "res/hero/h40400_bm", "res/hero/h41000_bm", "res/hero/h10_cif", "res/hero/h60049_bm"]
        {
            assert_eq!(parse_sheet_member(name), None, "{name}");
        }
        // A weapon effect lookalike with a costume number is still not a dressable sheet.
        assert_eq!(parse_sheet_member("res/hero/h40450_bm"), None);
        assert_eq!(parse_sheet_member("res/hero/h50350_bm"), None, "category 3 is shoes, not a weapon");
    }

    #[test]
    fn groups_follow_the_weapon_category() {
        assert_eq!(group_of(SLOT_WEAPON, 4), Some(4));
        assert_eq!(group_of(SLOT_WEAPON, 10), Some(10));
        assert_eq!(group_of(SLOT_WEAPON, 3), None);
        assert_eq!(group_of(SLOT_WEAPON, 11), None, "the shield is not a weapon");
        assert_eq!(group_of(SLOT_SHIELD, 11), Some(GROUP_SHIELD));
        assert_eq!(group_of(SLOT_SHIELD, 4), None);
        assert_eq!(group_of(SLOT_ARMOR, 0xffff), Some(1), "armour ignores the category");
    }

    #[test]
    fn the_decision_needs_a_set_and_its_sheet() {
        let has = |g: usize, s: u8| (g == 1 && s == 2) || (g == 6 && s == 2);
        assert_eq!(costume_sprite(SLOT_ARMOR, 0, 2, has), Some(51));
        assert_eq!(costume_sprite(SLOT_ARMOR, 0, 0, has), None, "0 = no costume");
        assert_eq!(costume_sprite(SLOT_ARMOR, 0, 3, has), None, "no sheet for set 3");
        assert_eq!(costume_sprite(SLOT_WEAPON, 6, 2, has), Some(51), "나이프 sheet present");
        assert_eq!(costume_sprite(SLOT_WEAPON, 5, 2, has), None, "스워드 sheet absent -> the equipped look");
        assert_eq!(costume_sprite(SLOT_ARMOR, 0, SET_MAX + 1, |_, _| true), None, "garbage byte");
    }

    /// A two-entry stored zip, built by hand: central directory + end record only (the reader
    /// never looks at local headers).
    #[test]
    fn the_central_directory_names_the_sheets() {
        use alloc::vec::Vec;
        let names: [&[u8]; 3] = [b"res/hero/h10050_bm", b"res/hero/h10012_bm", b"res/hero/h50652_bm"];
        let mut z: Vec<u8> = Vec::new();
        z.extend_from_slice(b"local headers would be here");
        let cd = z.len();
        for name in names {
            let mut h = [0u8; 46];
            h[..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
            h[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
            z.extend_from_slice(&h);
            z.extend_from_slice(name);
        }
        let cd_len = z.len() - cd;
        let mut e = [0u8; 22];
        e[..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
        e[8..10].copy_from_slice(&3u16.to_le_bytes());
        e[10..12].copy_from_slice(&3u16.to_le_bytes());
        e[12..16].copy_from_slice(&(cd_len as u32).to_le_bytes());
        e[16..20].copy_from_slice(&(cd as u32).to_le_bytes());
        z.extend_from_slice(&e);
        let m = sheets_in_zip(&z);
        assert_eq!(m[1], 1, "armour set 1");
        assert_eq!(m[6], 1 << 2, "나이프 set 3");
        assert_eq!(m.iter().filter(|&&v| v != 0).count(), 2, "the shipped h10012 is not a costume");
        assert_eq!(sheets_in_zip(b"not a zip"), [0; GROUPS]);
        assert_eq!(sheets_in_zip(&z[..z.len() - 3]), [0; GROUPS], "a cut-off end record reads as nothing");
    }

    #[test]
    fn ownership_bitmap() {
        let own = with_owned(with_owned([0; OWN_BYTES], 1), SET_MAX);
        assert!(owns(&own, 1) && owns(&own, SET_MAX) && !owns(&own, 2));
        assert_eq!(own[0], 1);
        assert_eq!(own[6], 1 << ((SET_MAX - 1) % 8));
        assert_eq!(with_owned(own, 0), own);
        assert!(!owns(&own, 0));
        assert!(usize::from(SET_MAX) <= OWN_BYTES * 8);
        assert_eq!(usize::from(FLAG_OWN) + OWN_BYTES - 1, 0xbc);
        assert_eq!(usize::from(FLAG_WEAR) + SLOTS, usize::from(FLAG_OWN));
    }
}
