// Save-vault policy: WHICH record files are worth keeping copies of, how the
// copies are named, and which ones to drop when there are too many.
//
// ── WHY A VAULT EXISTS AT ALL (2026-08-20, 60차) ─────────────────────────────
// Two players lost characters in one week, and both reports end the same way:
// "메인메뉴로 나가니 캐릭터가 빈 공간이라고 뜨고 껐다 키니 완전히 비어있었습니다".
// A slot reads as 빈 공간 when its record is gone or unreadable, and until now
// NOTHING on the device kept a second copy — the game overwrites one record per
// slot in place, the server-side backup is a 219-byte fragment (header + 9
// equipment records + tail, so no story, no inventory), and `PRIMARY KEY (acct,
// slot)` on the server means even that fragment has no history. So the first bad
// write was final.
//
// The vault fixes that at the only layer that sees every write: the record store.
// `database.rs` calls into it with the bytes that are ABOUT TO BE DESTROYED —
// before an overwrite and before a delete — so the copy taken is of a version the
// game itself considered good enough to have been running on.
//
// ★ Pre-image, not post-image. Snapshotting what was just written would also have
// caught the two reports (their corrupt slot was written at some point), but it
// cannot distinguish "the newest write is the good one" from "the newest write is
// the wipe". Keeping the version we are about to lose needs no judgement about
// which is which, and the player can pick by timestamp.
//
// ★ Whole record files, ciphertext and all. The record on disk is
// `encrypt(payload) + 0x10 header` (`save.rs`), so the host cannot read a name or
// a level out of it — but it also does not have to: restoring is a file copy, no
// guest call, no crypto, no allocation, and it works while the game is at the
// title screen. Anything that needed the plaintext would need the guest running.
//
// This module is DEPENDENCY-FREE so `savevault_selftest.rs` can `include!` the
// real source (same contract as `endpoint.rs` / `backup.rs` / `padhud.rs` — see
// CLAUDE.md). All filesystem work lives in `vault.rs`.

/// Records the vault keeps copies of.
///
/// Only the two that ARE the character. Hero3 LGT names them `GameSave_11_<slot>`
/// (the ~1.4-1.9 KB encrypted payload) and `SlotSave_11_<slot>` (the 32-byte
/// summary the main menu's slot list reads) — both confirmed as strings in the
/// shipped `binary.mod` at fo 0x5abe8 / 0x5abf4, with `%s_%d` at 0x5ba48 as the
/// `_<slot>` formatter. KTF uses the same two base names.
///
/// `OptionSave_11` is deliberately NOT here: losing it costs a settings screen,
/// and vaulting it would double the write traffic for nothing.
pub const VAULTED_PREFIXES: [&str; 2] = ["GameSave", "SlotSave"];

/// Is this record-store name one of the character records?
///
/// Matched on the PREFIX rather than the full name because the slot suffix and the
/// build number (`_11_`) are not ours to predict — KTF and LGT disagree on the
/// middle number, and a future build could add a slot.
pub fn is_vaulted(db_name: &str) -> bool {
    let leaf = db_name.rsplit(['/', '\\']).next().unwrap_or(db_name);
    VAULTED_PREFIXES.iter().any(|p| leaf.starts_with(p))
}

/// Is this record-store name a slot SUMMARY (`SlotSave_*`)? The 32-byte record the slot
/// row is decided by — `vault::classify` sizes it against [`SUMMARY_LEN`] rather than the
/// payload band.
pub fn is_summary(db_name: &str) -> bool {
    let leaf = db_name.rsplit(['/', '\\']).next().unwrap_or(db_name);
    leaf.starts_with(VAULTED_PREFIXES[1])
}

/// Plausible length for a `GameSave_*` record: the encrypted payload plus the
/// writer's 0x10 header.
///
/// The payload length is CONTENT-dependent, not a constant (`save.rs`: 1392 B in
/// the archived backup, 1808 B for a live LV.12 character), so this is a BAND.
///
/// ⛔ "Its only job is to keep the notice from firing" — the sentence that used to be here
/// — has been untrue since long before 90차. FIVE gates share this band and each fails in
/// a different direction: [`load_read_slot`] (the save PIN — a miss leaves a stale pin,
/// which is the vc180 mechanism), `vault::slot_has_save` (a miss reads EMPTY to
/// `creation_slot`, its one caller since 135차-B), [`summary_repair_needed`], `vault::offer_lines` /
/// `vault::missing_but_recoverable`, and since 90차 `vault::classify` — where it also
/// refuses a WRITE and votes `is_damage`.
///
/// ⛔⛔ And the CEILING is not a readability test: the game applies NO size test to a
/// record — `loadSaveRecord 0x1a0e8` asks only `MD5(descramble(rec[16..])) == rec[0..16]`
/// — so a record above this ceiling is one the game WILL read. Unreachable today:
/// `serialiseSave 0x1ad88` accumulates `232 + bags*272 + …` and `bags <= 5`
/// (`orbcombine::MAX_TABS`, whose count byte sits immediately after the fifth tab), i.e.
/// ~4.2 KB worst case against a 8208 ceiling, and `save.rs` has enforced the same 8192 on
/// the host's own chain since 19차 without once reporting it. ⛔ Do NOT drop the ceiling
/// in `vault::classify` alone: that would be the first code able to put an above-ceiling
/// record ON DISK, and the two fail-OPEN gates above would then misread it in the
/// direction that costs a character. It comes out of all five call sites in one change, or
/// out of none (71차-E: 판정을 다른 파일에 흩뜨리지 말 것).
///
/// ★ The FLOOR is not a size check at all, it is a KIND check: a 32-byte `slotSummary`
/// sitting where a `GameSave_11_<n>` belongs PASSES the game's MD5 test, and
/// `deserialiseSave` would then read a ~1.4 KB structure out of a 16-byte buffer.
pub fn plausible_game_record(len: usize) -> bool {
    (64 + 0x10..=8192 + 0x10).contains(&len)
}

/// The writer's header: MD5(plaintext payload) ‖ ciphertext payload (guest `0x1a1d4`).
pub const RECORD_HEADER: usize = 0x10;

/// `SlotSave_11_<n>` is always exactly this: the 16-byte digest plus the 16-byte summary
/// `slotSummary` (`0x1a6d0`) mallocs. Every writer passes the length as a literal
/// (`movs r2,#0x10` at the 제련 commit `0x254bc`), so anything else is damage.
pub const SUMMARY_LEN: usize = RECORD_HEADER + 0x10;

/// ★★★ 72차 — MAY THE HOST HAND THIS RECORD TO THE GUEST AT ALL?
///
/// `loadSaveRecord` (`0x1a0e8`) computes the payload size as `total - 0x10` and passes it
/// straight to `memcpy` / the descrambler / MD5 **with no lower bound** (`0x1a11c
/// subs r2,#0x10`). The only size test upstream is `seek(h, 0, SEEK_END) <= 0` at
/// `0x1a07e`, i.e. `total >= 1`. So a record of 1..15 bytes makes `total - 0x10` negative,
/// which reaches `memcpy` as a ~4 GB unsigned count. That is the documented F9, and it is
/// reachable from any torn write the host lets through.
///
/// The host cannot patch the guest cheaply, but it owns the record store, so it simply
/// never presents such a record: `0` takes the guest's clean "빈 슬롯" branch, `>= 0x10`
/// is arithmetically safe. ⛔ Do NOT widen this into a plausibility filter — hiding a
/// 60-byte GameSave would be indistinguishable from deleting it, and a wrong-but-large
/// record still fails MD5 and reads as empty without corrupting anything.
pub fn record_len_safe(db_name: &str, len: usize) -> bool {
    !is_vaulted(db_name) || len == 0 || len >= RECORD_HEADER
}

/// What `write_verdict`/`delete_verdict` should see when the record's size cannot be read.
///
/// Not zero. `fs::metadata(...).unwrap_or(0)` collapsed EIO / EACCES / ENOMEM into "there
/// is nothing there", and both verdicts read 0 as "nothing to lose":
/// `delete_verdict(_, _, 0) => Allow` then ends in `fs::remove_file`. That is the
/// fail-OPEN direction. The doctrine is *"anything we cannot read or do not understand
/// counts as OCCUPIED, because the cost of a false 'empty' is a character"*.
///
/// ⛔ This doc used to cite `vault::slot_has_save` as where that doctrine was already
/// stated. It is NOT implemented there — that function still folds every non-NotFound
/// `fs::metadata` error into 0 — and since 135차-B that is deliberate: its one remaining
/// caller (`creation_slot`) wants a failed read to mean EMPTY (see its doc). Do not read this
/// line as evidence that the question is settled everywhere; it is settled HERE, in
/// `write_verdict` and in `delete_verdict`.
pub const UNREADABLE_LEN: u64 = u64::MAX;

/// Translate a `fs::metadata` outcome into the `target_len` the verdicts take.
/// `Ok(n)` = the size · `Err(true)` = definitely not there · `Err(false)` = unreadable.
pub fn target_len(meta: Result<u64, bool>) -> u64 {
    match meta {
        Ok(n) => n,
        Err(true) => 0,
        Err(false) => UNREADABLE_LEN,
    }
}

/// How far apart a payload generation and its summary generation may be and still be
/// treated as the same save.
///
/// The two records are written by two separate `writeSaveRecord` calls a few milliseconds
/// apart (제련: `0x254ae` then `0x254c0`), so their vault timestamps differ — but only by
/// that. A minute is three orders of magnitude of slack and still refuses the failure this
/// exists to catch: `nearest_generation` used to pair a payload from today with a summary
/// from days ago, purely because that was the nearest one left in a directory the two
/// records rotate out of at different rates (`saveGame 0x4748` writes GameSave ONLY).
pub const PAIR_WINDOW_MS: u64 = 60_000;

/// Is this summary generation a usable partner for a payload taken at `payload_ms`?
pub fn summary_pairs(payload_ms: u64, summary_ms: u64, summary_len: usize) -> bool {
    summary_len == SUMMARY_LEN && payload_ms.abs_diff(summary_ms) <= PAIR_WINDOW_MS
}

/// The summary record that belongs with payload record `db_name`:
/// `GameSave_11_2` -> `SlotSave_11_2`. `None` for anything that is not a payload
/// record, which includes a summary — the mapping is deliberately one-way.
///
/// ★★★ 72차-D — WHY THE VAULT COPIES ONE RECORD WHEN THE OTHER ONE CHANGES.
///
/// 72차 made [`summary_pairs`] a precondition of restoring, because a payload restored
/// without its summary changes nothing the player can see: the slot ROW is decided by
/// `SlotSave_11_<n>` alone (guest scan `0x5c06`). That was right, and it turned a
/// silent no-op into an honest refusal — but it also made the refusal the COMMON case,
/// because the two records do not rotate together. `saveGame` (`0x4748`) writes the
/// payload alone on the four highest-frequency paths (창고 올리기 `0x6ea0`, 내리기
/// `0x13a90`/`0x13d36`, 캐시 지급 `0x13dd8`/`0x13ec8`), so a warehouse session fills the
/// payload vault with generations that have no partner anywhere near them in time.
///
/// The fix costs nothing: at the instant a payload pre-image is taken, the summary
/// **on disk right then** is by construction the one that was live alongside it, so
/// copying it into the sibling vault directory under the SAME timestamp makes
/// `summary_pairs` hold by construction. No guest call, no record write, and the slot
/// cursor is never consulted.
///
/// ⛔ ONE WAY ONLY. A summary write must not drag a 1.4 KB payload copy along with it:
/// the payload's own pre-image is already vaulted by the write that precedes every
/// paired summary write (제련 `0x254a0`, `04 3d` ack `0x47f2`), so the reverse direction
/// would only duplicate what is already there — at forty times the size.
pub fn summary_partner(db_name: &str) -> Option<String> {
    let leaf = db_name.rsplit(['/', '\\']).next().unwrap_or(db_name);
    // `VAULTED_PREFIXES` is `[payload, summary]`; naming them here keeps this in step
    // with `is_vaulted`, which is what decides whether either one gets a vault at all.
    let [payload, summary] = VAULTED_PREFIXES;
    let tail = leaf.strip_prefix(payload)?;
    // A slot tail has to be there and has to be a number, for the same reason
    // [`renamed_slot`] insists on one: never invent a record name.
    let _: u8 = tail.rsplit('_').next()?.parse().ok()?;
    Some(format!("{summary}{tail}"))
}

/// ★★★ 72차-D (TODO `BB` ⓑ) — MAY THE HOST REGENERATE THIS SLOT'S SUMMARY?
///
/// ⓐ above makes a broken slot RESTORABLE. This is the other half: making it not need
/// restoring. A slot row reads 「빈슬롯」 when `readSaveRecord("SlotSave_11", slot)`
/// returns zero (guest scan `0x5c06`), and the payload is not consulted at all — so a
/// perfectly good 1.8 KB character is invisible whenever its 32-byte summary is gone or
/// damaged. The game itself only rewrites a summary on the five paired paths (제련
/// `0x254a0`, 일반조합, the `04 3d`/`04 3e` acks, `0x3b4xx`, `0x48xxx`), and the four
/// highest-frequency saves are not among them, so a slot can stay invisible for a whole
/// session while its owner keeps playing and saving.
///
/// `payload_len` = what `GameSave_11_<slot>` holds. `summary` = `Ok(n)` for a readable
/// summary of `n` bytes, `Err(true)` when it is definitely absent, `Err(false)` when it
/// could not be read.
///
/// Narrow on purpose — every clause below is a way this could destroy something:
/// * ⛔ **no plausible payload, no repair.** Writing a summary for a slot with no
///   character makes the ROW paint an entry whose OK arm cannot load anything: the local
///   게임시작 arm reads `GameSave_11_<cursor>`, and when that read fails it silently
///   switches the shell to 새 캐릭터 생성 (`0xaf38`). The row would lie about what
///   pressing it does.
/// * ⛔ **a healthy summary is never rewritten.** This is a repair, not a policy — a
///   host that rewrites the summary on every save is 72차-C's proposal 4 (the verdict
///   latch), and its risk analysis says no.
/// * ⛔ **unreadable is not damage.** `fs::metadata` failing with EIO/EACCES is the one
///   case where writing would be acting blind, and the doctrine in this module is
///   already settled (see [`target_len`]): what we cannot read is treated as intact.
pub fn summary_repair_needed(payload_len: usize, summary: Result<u64, bool>) -> bool {
    if !plausible_game_record(payload_len) {
        return false;
    }
    match summary {
        // Any size but exactly `SUMMARY_LEN` is damage — `slotSummary` passes the length
        // as a literal at every call site, so the game cannot produce another one. The
        // 16..31 band is the worst shape: the file exists and `seek(END) > 0` passes, so
        // the guest gets as far as the MD5 compare and only then reads the slot as empty.
        Ok(n) => n as usize != SUMMARY_LEN,
        Err(true) => true,
        Err(false) => false,
    }
}

/// ★★★ 72차 §1-2 in one predicate: may the BOOT HEAL install a summary for this slot?
///
/// The ROW is what a summary paints, and a row painted over a payload the guest cannot
/// LOAD is the destructive funnel — the 게임시작 OK arm reads `GameSave_11_<cursor>`
/// directly and, when that read fails, silently switches the shell to 10 = 새 캐릭터 생성
/// (`0xaf38`), while `#` on an OCCUPIED row is 초기화 (`0xaf56`) instead of the inert 복구
/// modal. So the question is never "is the payload damaged", it is "will the OK arm find
/// something it can load": `payload_restored` = one was just put back, `payload_loadable`
/// = that, or one was already readable.
///
/// ⛔ Do NOT gate this on the payload being DAMAGED again. `vault::OnDisk::Unreadable` is
/// deliberately not damage — we must not act on a record we cannot read — and that made
/// the old `g_damage &&` guard false, so it skipped its own arm and installed the summary
/// anyway. It was the ONE column of the whole (payload, summary) table that reached the
/// funnel.
///
/// ★ The rule lives HERE, in the dependency-free file, because 71차-E: `vault.rs` is
/// `include!`d by no standalone selftest, so a condition left there is one no Windows host
/// can test — and a condition living where no test could reach it is exactly how save
/// slots got destroyed on 2026-08-26.
pub fn summary_install_allowed(payload_loadable: bool, payload_restored: bool, summary_damaged: bool, summary_ok: bool) -> bool {
    payload_loadable && (summary_damaged || (payload_restored && !summary_ok))
}

/// ★★★ 99차 (2026-09-16) — the RUNTIME twin of [`summary_install_allowed`]: may the worker's
/// 20-second repair (`emulator::summary_repair_poll`) rebuild a slot summary from the live
/// character?
///
/// The boot heal has had the "payload must be LOADABLE" guard since 90차-B; the runtime
/// repair only ever asked [`summary_repair_needed`], which is LENGTH-only on the payload. So
/// a payload that is the right size but fails the game's MD5 — an unloadable character —
/// could still get a fresh row painted beside it, and that row is the `0xaf38` funnel
/// (OK = 새 캐릭터 생성, `#` = 초기화). In practice the repair's own gate made that hard to
/// reach (it needs a live character whose cursor equals the pin, and an unloadable payload
/// cannot be loaded), but the predicate should not rely on the caller's luck.
///
/// `payload_loadable` = the game's own MD5 says yes when the cipher is proven, else the
/// length band (the boot check's own fallback); `summary_damaged` = missing, wrong size, or
/// (cipher proven) MD5-bad. Nothing else may be substituted for either input.
pub fn summary_rebuild_allowed(payload_loadable: bool, summary_damaged: bool) -> bool {
    payload_loadable && summary_damaged
}

/// Generation file name: `<unix_ms>-<len>.bin`, zero-padded so a lexicographic
/// directory listing is already in time order (13 digits covers every ms
/// timestamp until the year 33658, and 14 keeps it sorted past that).
pub fn gen_name(unix_ms: u64, len: usize) -> String {
    format!("{unix_ms:014}-{len}.bin")
}

/// Inverse of [`gen_name`]. `None` for anything else in the directory (a leftover
/// `.tmp`, a file a curious player dropped in), so junk is ignored rather than
/// deleted or mistaken for a backup.
pub fn parse_gen(file_name: &str) -> Option<(u64, usize)> {
    let stem = file_name.strip_suffix(".bin")?;
    let (ms, len) = stem.split_once('-')?;
    if ms.len() < 14 || !ms.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((ms.parse().ok()?, len.parse().ok()?))
}

/// How many of the newest generations are kept unconditionally.
///
/// Six at the 5-minute autosave cadence is half an hour of history, which covers
/// "I noticed immediately". The day tier below covers "I noticed tomorrow".
pub const KEEP_NEWEST: usize = 6;
/// How many distinct calendar days keep their newest generation.
pub const KEEP_DAYS: usize = 6;
/// Hard ceiling per record, so a pathological write loop cannot fill storage.
///
/// ★ 72차: raised 16 -> 24 to make room for [`SPREAD_MS`]. Worst case is
/// 6 + 6 + 6 + 1 pin = 19 files of at most ~2 KB, i.e. ~40 KB per record.
pub const KEEP_MAX: usize = 24;

/// ★★★ 72차 — THE AGE LADDER, AND WHY THE VAULT NEEDED ONE.
///
/// The doc on [`KEEP_NEWEST`] states the design assumption out loud: "six at the
/// 5-minute autosave cadence is half an hour of history". 제련 breaks that assumption
/// by a factor of thirty. Every refine attempt — success, failure, 파괴 and 초기화 all
/// converge on the same tail at guest `0x25454` — writes a full GameSave+SlotSave pair,
/// so the save rate is bounded only by the player's thumb, and [`MIN_GAP_MS`] then lets
/// the vault take a generation every ten seconds. Six seats spent 10 s apart means the
/// unconditional tier can only ever span **sixty seconds**, and `rotate` runs after
/// every capture, so everything between one minute ago and the previous calendar day is
/// deleted WHILE THE PLAYER GRINDS.
///
/// That is not a theory. Running the shipped `should_vault` + `to_delete` over four days
/// of 5-minute autosaves followed by a 20-minute refine spree left exactly ten survivors:
/// 8 s, 20 s, 32 s, 44 s, 56 s, 68 s — and then 23.7 h, 47.7 h, 71.7 h, **95.7 h**. The
/// player's report was *"4일 전에 저장했던 거 제외하면 빈공간"*, and 95.7 h is 3.99 days.
/// The number came out of this policy, not out of their play habits.
///
/// So the recent tier is now time-spread as well as count-based. The rungs below are BAND
/// EDGES, and the rule is **keep the OLDEST generation in each band** — not the newest.
///
/// ⛔ That distinction is the whole mechanism, and getting it backwards produces a policy
/// that looks right and protects nothing. Rotation runs after EVERY capture, so a rule
/// must survive being applied ten thousand times as the clock moves. "Keep the newest
/// generation at least a minute old" re-elects a different generation every tick and
/// deletes last tick's, so nothing ever ages past the first rung and the ladder collapses
/// to exactly the sixty-second window it was meant to fix. "Keep the oldest in each band"
/// instead lets one generation MIGRATE: it is kept while it crosses `[1 min, 5 min)`, and
/// the moment it graduates into `[5 min, 15 min)` it is the oldest there and is kept
/// again, while a fresh graduate from the newest-6 takes its place below. That is ordinary
/// exponential thinning, and it is stable under repeated pruning.
///
/// ⛔ Do not replace the unconditional newest-6 with the ladder — "I noticed immediately"
/// still needs a dense head. The ladder is what covers "I noticed twenty minutes in".
pub const SPREAD_MS: [u64; 6] = [
    60_000,      // 1 minute
    5 * 60_000,  // 5 minutes
    15 * 60_000, // 15 minutes
    60 * 60_000, // 1 hour
    4 * 3_600_000,
    12 * 3_600_000,
];

/// Milliseconds in a day, for the day tier. Local-time boundaries would be nicer
/// for the player but need a timezone this module has no way to know; a UTC day
/// still spreads the kept generations across days, which is the point.
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

/// Sidecar naming the one generation per record that a RESTORE brought back.
///
/// ★★ Why a pin exists at all (2026-08-25, 66차 제보 "복원한 세이브가 복구 슬롯을
/// 덮는다"): a restore used to cost the player THREE of their six seats, and in the
/// worst case ate the generation they had just restored.
///   1. `vault::restore` captures the pre-restore bytes as a new generation — correct,
///      but that is one seat;
///   2. the `rotate` that capture triggers runs over the SAME directory the source
///      generation lives in, so restoring a deep generation could delete it outright;
///   3. the first save afterwards vaults a byte-identical copy of what was restored —
///      `should_vault` cannot see that it already has those bytes, and the 10-second
///      rate limit never fires because the restore is applied at boot, before the
///      emulator exists, so `last_ms` is always 0.
/// The pin closes all three: [`to_delete`] keeps the pinned generation outside
/// the tiers, and [`duplicates_pin`] declines the duplicate at (3).
///
/// It is a plain text file rather than a marker generation because every enumerator in
/// `vault.rs` filters on [`parse_gen`], which requires a `.bin` suffix — so the sidecar
/// is invisible to listing, export and rotation without a single change to them.
pub const PIN_FILE: &str = "pinned.txt";

/// The pin file's body: the generation's own file name, one line.
pub fn pin_content(unix_ms: u64, len: usize) -> String {
    format!("{}\n", gen_name(unix_ms, len))
}

/// Inverse of [`pin_content`]. Tolerates `\r\n` and trailing junk lines, because the
/// only thing that must survive a hand edit is "which generation is pinned".
pub fn parse_pin(text: &str) -> Option<(u64, usize)> {
    text.lines().map(str::trim).find(|l| !l.is_empty()).and_then(parse_gen)
}

/// Is `old` byte-for-byte the generation the pin names?
///
/// `pin_bytes` is the pinned generation's content, or `None` when there is no pin or
/// it names a file that is gone. Fails CLOSED — an unreadable pin means "vault it",
/// which is the behaviour that existed before the pin did.
pub fn duplicates_pin(old: &[u8], pin_bytes: Option<&[u8]>) -> bool {
    !old.is_empty() && pin_bytes == Some(old)
}

/// Which generations to delete, given every generation currently on disk.
///
/// Input does not need to be sorted. Output is a list of indices INTO `gens`, so
/// the caller keeps its own file names and never has to re-derive them.
///
/// The policy is two tiers, and the reason it is not simply "keep the newest N" is
/// that the two reports arrived a day apart from the incident: a player who plays
/// an hour and only then opens the app to find an empty slot has already pushed
/// every 5-minute autosave out of a flat window. So:
///   * the newest [`KEEP_NEWEST`] survive, whatever their age, and
///   * the newest generation of each of the most recent [`KEEP_DAYS`] days
///     survives, which leaves one restore point per day going back a week.
/// Everything else goes, oldest first, and [`KEEP_MAX`] trims the survivors if the
/// day tier somehow keeps more than the ceiling.
/// `pinned` names one generation by `(unix_ms, len)` that is kept whatever the tiers
/// say — see [`PIN_FILE`]. It is held OUTSIDE the tiers, not inside them: it does not
/// consume one of the [`KEEP_NEWEST`] seats and it does not seed the day tier. That is
/// the whole point — a restored save must not make the player's recent history one
/// generation shallower, which is exactly what the 66차 report described. `None` is
/// the ordinary case, and a pin naming a generation that is not in `gens` (the player
/// deleted it, or the pin is stale) behaves identically to `None`.
pub fn to_delete(gens: &[(u64, usize)], pinned: Option<(u64, usize)>) -> Vec<usize> {
    // Which index the pin names, if any. First match wins — two generations with the
    // same (ms, len) are the same bytes as far as the player is concerned.
    let pin_idx = pinned.and_then(|p| gens.iter().position(|g| *g == p));

    let mut order: Vec<usize> = (0..gens.len()).filter(|i| Some(*i) != pin_idx).collect();
    // Newest first. Ties broken by index so the result is deterministic.
    order.sort_by(|a, b| gens[*b].0.cmp(&gens[*a].0).then(a.cmp(b)));

    let mut keep = vec![false; gens.len()];
    let mut days_seen: Vec<u64> = Vec::new();

    // Tier 1 — the dense head.
    for &i in order.iter().take(KEEP_NEWEST) {
        keep[i] = true;
        let day = gens[i].0 / DAY_MS;
        if !days_seen.contains(&day) {
            days_seen.push(day);
        }
    }

    // Tier 2 — the age ladder (72차). Ages are measured from the NEWEST generation, not
    // from the wall clock: the caller has one, this function must stay pure, and "how
    // deep does my history go" is a question about the generations themselves.
    let newest = order.first().map_or(0, |&i| gens[i].0);
    let age = |i: usize| newest.saturating_sub(gens[i].0);
    for (k, &lo) in SPREAD_MS.iter().enumerate() {
        // The band this rung owns, `[lo, hi)`. The top band stops at a day, where the day
        // tier below takes over.
        let hi = SPREAD_MS.get(k + 1).copied().unwrap_or(DAY_MS);
        // `order` is newest-first, so scanning it backwards finds the OLDEST member of the
        // band — the one that will graduate into the next band and be kept there. See the
        // note on `SPREAD_MS` for why the newest would be wrong.
        if let Some(&i) = order.iter().rev().find(|&&i| (lo..hi).contains(&age(i))) {
            keep[i] = true;
            let day = gens[i].0 / DAY_MS;
            if !days_seen.contains(&day) {
                days_seen.push(day);
            }
        }
    }

    // Tier 3 — one per calendar day, for "I noticed tomorrow".
    for (rank, &i) in order.iter().enumerate() {
        if rank < KEEP_NEWEST || keep[i] {
            continue;
        }
        let day = gens[i].0 / DAY_MS;
        if !days_seen.contains(&day) && days_seen.len() < KEEP_DAYS {
            days_seen.push(day);
            keep[i] = true;
        }
    }

    // Ceiling: if the tiers between them kept more than KEEP_MAX, drop the oldest
    // of the kept set until they fit. The pin counts toward the ceiling (storage is
    // storage) but is never the one dropped — only non-pinned survivors are trimmed.
    let pin_seats = usize::from(pin_idx.is_some());
    let mut kept: Vec<usize> = order.iter().copied().filter(|i| keep[*i]).collect();
    while kept.len() + pin_seats > KEEP_MAX {
        match kept.pop() {
            Some(oldest) => keep[oldest] = false,
            None => break,
        }
    }

    // `order` never contains the pinned index, so the pin cannot appear here — which
    // is the force-keep, expressed by construction rather than by a second pass.
    order.into_iter().filter(|i| !keep[*i]).collect()
}

/// Should the bytes about to be destroyed be vaulted at all?
///
/// `old` = what is on disk now, `new` = what is about to replace it (`None` = the
/// record is being deleted). The refusals each exist because they would otherwise
/// fill the vault with copies that cannot help anyone:
///
///  * an empty old record — there is nothing to lose;
///  * identical content — the game re-writes the same record on every warehouse
///    page turn (device trace 2026-08-11, see `hero3_store.put_backup`), and
///    write-through fires per `stream_write`, so most calls change nothing;
///  * a rate limit, so a guest that writes in a tight loop cannot turn the vault
///    into the write amplifier. `last_ms == 0` means "nothing vaulted yet", which
///    always passes.
///
/// ⛔ …and NONE of them applies to a DELETE (`new == None`) except the empty check.
/// See the arm below: a delete is the last thing that ever happens to a record.
///
/// ★ It deliberately does NOT refuse when `new` is longer or "looks better". The
/// vault's job is to keep what existed, not to judge which version is good — that
/// judgement is what the player makes when they pick a generation to restore.
pub fn should_vault(old: &[u8], new: Option<&[u8]>, last_ms: u64, now_ms: u64, min_gap_ms: u64) -> bool {
    if old.is_empty() {
        return false;
    }
    let Some(new) = new else {
        // ★★ A DELETION IS NEVER RATE-LIMITED (2026-08-25). The limit exists so a guest
        // writing in a loop cannot turn the vault into a write amplifier — but a delete
        // is not one of a series, it is the last thing that ever happens to that record,
        // and the copy we decline to take here is a copy that can never be taken again.
        // The window this closes is real and narrow: the warehouse rewrites a save on
        // every deposit and every page turn, so a 초기화 (or a failed write that used to
        // become one) landing within ten seconds of one of those found the limit armed
        // and left NOTHING behind.
        return true;
    };
    if new == old {
        return false;
    }
    last_ms == 0 || now_ms.saturating_sub(last_ms) >= min_gap_ms
}

/// Minimum spacing between two vault writes for the SAME record.
///
/// Ten seconds: shorter than any legitimate save cadence (the autosave is five
/// minutes, a manual save is a button press) and long enough that a runaway
/// writer costs one small copy per ten seconds instead of thousands.
pub const MIN_GAP_MS: u64 = 10_000;

/// Keep the existing save-slot pin when the list cursor moved but the loaded
/// character did not.
///
/// `CHAR+0xa410` is the slot-list cursor. The 67차 pin froze it only while
/// `shell` was true; returning to the field re-read the drifted value and the
/// next save was allowed onto the wrong slot. Holding on a matching identity
/// closes that.
///
/// ★ 71차-L: an UNREADABLE live identity (`identity == 0`) now HOLDS a proven pin
/// instead of letting the caller adopt the cursor. The old rule ("never hold on a
/// zero identity, because refusing to adopt a newly loaded character would drop
/// their saves") was written when an unproven pin still authorised writes. It no
/// longer does — [`write_verdict`] requires `pinned_id != 0` to replace an occupied
/// record — so adopting on no evidence now converts a working pin into one that
/// refuses everything, while holding costs nothing: the next readable poll adopts.
/// An unproven pin (`latched_id == 0`) still yields, because there is nothing to keep.
pub fn pin_holds(latched: Option<u8>, latched_id: u64, cursor: u8, identity: u64) -> bool {
    let Some(s) = latched else { return false };
    // An unproven pin has nothing worth keeping — let the caller adopt.
    if latched_id == 0 {
        return false;
    }
    // ⛔ An unreadable live identity is not evidence about ANYTHING, so it may not
    // move the slot and it may not clear the name either. The `s == cursor` case
    // matters as much as the drifted one: adopting there would store `identity = 0`
    // over a known name, and an unproven pin refuses every write (71차-L).
    if identity == 0 {
        return true;
    }
    s != cursor && latched_id == identity
}

/// May an observation made in the SHELL (app state 3) adopt `cursor` as the pin?
///
/// ★★★ 71차-E — THE HOLE THAT DESTROYED A SLOT. The 67차 pin latched only while
/// `shell` was false, and 66차-C had already established that **app state 3 is not
/// "outside the world": the title/slot shell AND the online screens (창고 · 캐시샵)
/// all live there**. So the route
///
///   타이틀 → 온라인 → 슬롯 고르기 → 창고
///
/// never leaves state 3, the pin was NEVER latched, and `vault::write_check` returns
/// `Allow` unconditionally when there is no pin. Every `saveGame` the warehouse fires
/// (올리기 · 내리기 · 구입 · `04 3d` ack) then wrote the LOADED character at whatever
/// the slot-list cursor happened to point at. Player report 2026-08-26: pick slot 1,
/// enter the warehouse, cancel back, pick slot 2 — the game keeps character 1 loaded
/// (67차: **캐릭터는 언로드되지 않는다**) — and slot 2's character was replaced.
///
/// The rule, and why it is not just `pin_holds` again: in the shell the cursor is
/// never evidence, so the ONLY thing that may move the pin is a character LOAD, and a
/// load is exactly "the identity changed".
///
/// * `identity == 0` (unreadable) → **never adopt**. `pin_holds` adopts on an
///   unreadable identity on purpose — in play, refusing to follow a freshly loaded
///   character would drop their saves — but in the shell that same fallback is the
///   destructive direction: it would pin a drifted cursor to an unknown character.
/// * no pin yet → adopt. This is the first character of the session, and at the moment
///   its name appears the cursor still is the slot it was loaded from.
/// * identity differs from the latched one → adopt: a different character was loaded.
/// * otherwise → hold. The player is walking the slot list with the same character
///   still resident, which is precisely the reported case.
pub fn shell_adopts(latched: Option<u8>, latched_id: u64, identity: u64) -> bool {
    identity != 0 && (latched.is_none() || identity != latched_id)
}

/// Which slot a newly-visible identity should be pinned to.
///
/// ★★★ 71차-J — vc177 on S25 printed **no** `pinning slot` in the shell, then
/// `live character is slot 1 (was -1)` on the first in-play poll, then
/// `REDIRECTED a write aimed at GameSave_11_0 to GameSave_11_1`. The loaded
/// character was slot 0 (1번); the pin had adopted the drifted cursor (2번);
/// the redirect wrote character 1 onto slot 2.
///
/// A load is an identity that was not showing last poll (`prev_id != identity`,
/// including `prev_id == 0`). The cursor at THAT previous poll is the one
/// `loadSaveRecord` read: the game has no other slot word, and any later ↑↓/CLR
/// is what the player does after the load. The current cursor is only used when
/// there is no previous observation at all.
///
/// `already_pinned` = an execute pin is already in place (possibly with identity
/// 0 — the warehouse/slot screen never showed a name in vc177/vc178). A name
/// that then appears is the SAME character becoming readable, not a load;
/// moving the pin here is how vc178 wrote slot 1 onto slot 2.
///
/// `None` = this poll is not a load. The caller must not move the pin.
pub fn load_pin_slot(
    cursor: Option<u8>,
    identity: u64,
    prev_cursor: Option<u8>,
    prev_id: u64,
    already_pinned: bool,
) -> Option<u8> {
    if identity == 0 || identity == prev_id {
        return None;
    }
    if already_pinned && prev_id == 0 {
        return None;
    }
    prev_cursor.or(cursor)
}

/// The `prev_cursor` [`load_pin_slot`] may use: the previous poll's cursor — unless a
/// character LOAD was observed since that poll (`loaded` = the pin that load set), in which
/// case the previous poll described the OUTGOING world and the load's own slot is the only
/// evidence there is.
///
/// ★★★ 148차 (2026-10-01, S25 vc420, 「빠르게 슬롯을 바꿔 로드하면 퀵슬롯이 날아간다」).
/// [`load_pin_slot`]'s premise — "the cursor at the previous poll is the one
/// `loadSaveRecord` read" — assumes some poll saw the SLOT LIST in between. OK mashed
/// through 메인메뉴 → 게임시작 → SLOT 1 went world(slot 2) → shell → world(slot 1) in two
/// ticks, so the previous poll was still slot 2's world:
///
/// ```text
/// 00:25:26.231  the character in slot 0 was loaded — pin was Some(1)    ← exact, correct
/// 00:25:26.694  live character is slot 1 (was 0)                         ← this rule, prev cursor 1
/// 00:25:26.732  cursor is 0 but the live character is still slot 1 — keeping the pin  ×40
/// ```
///
/// The class name changed (slot 1's character is another class), so `load_pin_slot` fired
/// with the stale cursor, and `pin_holds` then defended the wrong pin for the rest of the
/// session — the bag boxes switched to slot 2's file, the plate row kept slot 2's key, and
/// a save made there would have been REDIRECTED onto `GameSave_11_1`.
pub fn pin_evidence_cursor(prev_cursor: Option<u8>, loaded: Option<u8>) -> Option<u8> {
    loaded.or(prev_cursor)
}

// ⛔⛔ `execute_pin_slot` WAS HERE AND IS GONE (71차-L, 2026-08-26). ⛔⛔
//
// 71차-K committed "the slot the player executed into the warehouse" ONCE, on the
// socket's false->true edge, and then froze it: `if latched.is_some() { return None }`.
// The player's report is what that costs:
//
//   네트워크 접속 → A선택 → 뒤로 → B선택 → 뒤로 → 뒤로(메인메뉴)
//   … 저장완료되고나서 A세이브파일이 B세이브로 변경이 되는 이슈
//
// EXECUTING A ROW ON THE ONLINE SLOT LIST IS A CHARACTER LOAD. The arm (`0xa8f0`,
// key −5, shell key table `0x150e838` index 2) is, in order:
//
//   rec = readSaveRecord("GameSave_11"(0x5bbb4), *(CHAR+0xa410))   ; 0xa93e
//   if (!rec) { showModal("빈슬롯입니다."(0x5c08c), 7); return }    ; 0xa948
//   putByte 0x04 ; putByte 0x3d ; putByte (u8)cursor               ; 0xa952…0xa966
//   putBytes(rec, 0xdc)                                            ; 0xa96a  (the backup)
//   deserialiseSave(rec)                                           ; 0xa976 → 0x1a964 ★
//   send() ; showModal("슬롯 정보 서버에 저장중입니다.", 3)          ; 0xa97c
//
// `0x1a964` is the character loader — it writes CHAR+0xb6/0xe2/0xf2/0xf8, the whole
// equipment block, and the bag through `grid = *(CHAR+0x9e28)` — and the literal
// `0x1a965` appears in exactly TWO places in the image: here, and world init
// `0x3ac5c`. So the second execute (B) really does replace the live character, and a
// pin that refuses to move is pointing at the character that is no longer loaded.
//
// The rest follows from the guest: the last 뒤로 (`0xa900`) sets the cursor to 0 and
// tears the socket down, and `writeSaveRecord` (`0x1a264`) takes its `_<slot>` suffix
// from that cursor — so the state after the reported sequence is CHAR = B, cursor = 0.
// Every write then aims at slot 0, and the frozen pin either ALLOWED it (pin == 0, the
// value the 온라인 row itself writes at `0xa88a`) or REDIRECTED it there. The
// reporter's 금고 recorded the result: `GameSave_11_0`'s pre-image went 1408 B → 1824 B
// at 2026-08-26 18:37 KST, six minutes after vc179 was installed.
//
// ★★★ THE REPLACEMENT IS EXACT, NOT A GUESS: `vault::note_slot_loaded`, driven by the
// `04 3d <slot>` frame in `network.rs::send`. That frame is built by the arm above and
// carries the slot as a byte, so the host learns the loaded slot from the wire instead
// of inferring it from a cursor — and it is re-armed on EVERY execute, which is the
// half 71차-K was missing. Together with the in-play latch it covers both callers of
// `0x1a964`.
//
// ⛔ Do not reintroduce a pin source that is not an observed LOAD. [`write_verdict`]'s
// `p == target => Allow` arm and [`redirect_slot`] both trust the pin completely, and
// they cannot be gated on `pinned_id` instead — that field is a CLASS name, not a
// character name (see [`redirect_slot`]).

/// The slot whose character a record READ is about to load — `None` when the read is
/// not a load.
///
/// ★★★ 71차-M (2026-08-27) — THE SECOND LOAD PATH, AND THE ONE THAT DESTROYED 세이브1.
///
/// 71차-L pinned on the `04 3d` frame, which the ONLINE slot list sends. The LOCAL
/// 게임시작 slot list (`0xaee8`, key −5) sends nothing: it calls
/// `readSaveRecord("GameSave_11", cursor)`, stores the buffer at `*(base+0)` and runs
/// `setAppState(4)`, and world init deserialises it. So that load was invisible, the
/// pin stayed on the previous character, and — because [`identity_hash`] hashes a CLASS
/// name — `pin_holds` saw "same character" and kept it. Device log, vc180:
///
/// ```text
/// 00:05:49.051  DB get [GameSave_11_1] id=1 -> Some(1840)   ← 게임시작 → 세이브2
/// 00:05:49.639  emu hitch 594 ms (world init)
/// 00:05:49.6xx  save slot: cursor is 1 but the live character is still slot 0 — keeping the pin
/// 00:05:51.119  REDIRECTED a write aimed at GameSave_11_1 to GameSave_11_0
/// ```
///
/// The record store sees BOTH loads, so this is the signal that covers them. Verified
/// against the same device log, where it has no false positives:
/// * a WRITE never reads — `MC_dbOpenDataBase(…, MC_DB_CREATE)` skips the record-1 seed
///   (vendor `open_database`), and all eleven `MC_DB_CREATE` lines in that trace have no
///   `DB get` before them;
/// * all eleven `DB get [GameSave_11_*] id=1` lines are followed by a load — five by an
///   `04 3d`, six by a state-3→4 world init.
///
/// ⛔ `GameSave` ONLY. `SlotSave_11_<n>` is the 32-byte summary, and `setAppState(3)`
/// scans EVERY slot's summary to decide which rows read 빈슬롯 (`0x5c06`) — pinning on
/// those would walk the pin across all three slots every time the list is drawn.
/// ⛔ Record 1 only, and the length must be a plausible save: a truncated or empty read
/// is not a character.
pub fn load_read_slot(db_name: &str, record_id: u32, len: usize) -> Option<u8> {
    if record_id != 1 || !plausible_game_record(len) {
        return None;
    }
    let leaf = db_name.rsplit(['/', '\\']).next().unwrap_or(db_name);
    if !leaf.starts_with("GameSave") {
        return None;
    }
    leaf.rsplit_once('_')?.1.parse().ok()
}

// ⛔ `SAVE_LOCK_*` / `save_lock_release` WERE HERE (71차-K: "freeze the slot list after
// 네트워크 → 뒤로 until the disconnect save finishes") and are gone (136차, 2026-09-25).
// The premise was false: the game writes NO save on the way out of the warehouse (probe,
// both CLRs, 4 s: zero record accesses; the GameSave mtime is the login ack's). The lock
// therefore always fell to its 800 ms "nothing arrived" timeout, and the 「저장하고
// 있습니다」 box it showed was announcing a write that never happened.

/// The online module's screen ids that are the CHARACTER-CREATION screens
/// (`items::shell_screen`): **10** class select (`0xafb4`) · **11** portrait (`0xb044`) ·
/// **12** confirm 예/아니오 (`0xb174`). The local 게임시작 slot list (screen 9, `0xaee8`)
/// opens 10 when `readSaveRecord("GameSave_11", cursor)` returns 0 (`0xaf38..0xaf40`), and
/// 12's OK-with-예 is the only way from any of them into the world (`0xb1c8..0xb1cc` →
/// `setAppState(4)`).
pub const CREATION_SCREENS: [u32; 3] = [10, 11, 12];

/// Shell screens whose key handlers themselves read or write a character record.
///
/// **2** = 온라인 슬롯 목록 (`0xa8f0` → `readSaveRecord` + `04 3d`) · **9** = 게임시작
/// 슬롯 목록 (`0xaee8` → `readSaveRecord` + `setAppState(4)`). Warehouse/cash (3) is
/// NOT here — that is why 72차-D kept summary repair off `save_gate`.
pub const SLOT_LIST_SCREENS: [u32; 2] = [2, 9];

/// True on the two slot-list screens. Host-issued `slotSummary` / `writeSaveRecord`
/// must not run there: the game is about to `readSaveRecord` on the same OK, and a
/// nested guest call leaves world init wedged (2026-09-03: 「세이브 슬롯 누르면 게임
/// 시작이 멈춘다」).
pub fn on_slot_list(shell_screen: Option<u32>) -> bool {
    shell_screen.is_some_and(|s| SLOT_LIST_SCREENS.contains(&s))
}

/// True on the character-creation screens ([`CREATION_SCREENS`]). Cheap: it is the guard
/// that decides whether the caller needs to go and stat a record for [`creation_slot`].
pub fn on_creation_screen(shell_screen: Option<u32>) -> bool {
    shell_screen.is_some_and(|s| CREATION_SCREENS.contains(&s))
}

/// The slot a character being CREATED belongs to — the THIRD load-equivalent pin source.
///
/// ★★★ 76차 (2026-09-03) — THE HOLE. The pin's sources were two observed LOADS: the
/// outgoing `04 3d <slot>` frame and a `GameSave_11_<n>` record read
/// ([`load_read_slot`]). A character created in an EMPTY slot is neither — the record read
/// fails (that is what sends the game to screen 10), so nothing re-aimed the pin. With a
/// character of the SAME CLASS loaded earlier in the process the identity did not change
/// either, [`pin_holds`] kept the old pin, and [`redirect_slot`] saw «pin 0 · cursor 2 ·
/// same identity · target 2» — the exact signature of a drifted-cursor save — and landed the
/// new character's first save on the old slot. Report: «슬롯 1·2 에 캐릭터가 있는데 슬롯 3 에
/// 새로 만들면 슬롯 1 이 덮인다».
///
/// The one guest fact that separates "created here" from "drifted here" is the shell's
/// screen word (`CHAR+0xa3fc` = `base+0xa3d4`): the creation screens leave it at 10/11/12
/// and nothing in app state 4/5 writes it — the whole-image scan for `str rX,[rY,#0x38]`
/// preceded by a small immediate finds every writer of that word inside the shell module
/// (`0xa74e`..`0xbe50`), and the only ones that write 10/11/12 are the creation handlers
/// themselves (`0xaf3e`, `0xb030`, `0xb06e`, `0xb150`, `0xb184`, `0xb1d4`).
///
/// ⛔⛔ 78차 CORRECTION — **THE WORD IS NEVER RESET.** 76차 claimed «`setAppState(3)`'s init
/// `0x59f0` resets it to 0 before any screen is drawn again». That is FALSE: `0x59f0` fills
/// the shell key/paint tables, stores 0 at `base+0xa438` (literal `0x5e4c`) — a different
/// word — and its only other side effects are `0x4cf8(1)`, which is the screen-geometry
/// setup, and the slot-occupancy scan at `0x5c06`. So a creation value persists until some
/// OTHER shell screen handler happens to overwrite it. A pin source that trusts a word with
/// no reset, and that reads the drifting slot cursor for its slot, is exactly the thing
/// 71차-L's tombstone forbids.
///
/// **So the pin may only ever be created onto an EMPTY record.** That is not a weakening of
/// 76차: the game makes you 초기화 a slot before you may create in it, so a created
/// character's slot is empty by construction, and its first save is the write that fills it.
/// With `target_occupied` the stale-screen case can no longer authorise replacing a
/// character — which is the 2026-09-03 report «1슬롯으로 반복 플레이했는데 2슬롯이 1슬롯의
/// 세이브로 덮였다 / 가끔 2슬롯이 빈슬롯이 된다».
///
/// `Some(cursor)` when the last shell screen was a creation screen AND that slot holds no
/// character, else `None`.
///
/// ⛔ It still says nothing on its own about WHEN to pin. The caller must combine it with
/// either "the game just ENTERED the world" (worker poll, the `shell` → `!shell` edge) or
/// "a character record is being written" (`vault::write_check`). While the player is still
/// ON screens 10–12 the pin must not move: they can CLR back to the slot list (`0xafc0`:
/// 10 → 9) or answer 아니오 (`0xb1d2`: 12 → 11) and then load someone else or walk into the
/// warehouse with the old character still resident — a pin moved early would then redirect
/// THAT character's saves into the empty slot (a duplicate) while its own slot stopped
/// updating (70차 증발).
pub fn creation_slot(shell_screen: Option<u32>, cursor: Option<u8>, target_occupied: bool) -> Option<u8> {
    if !on_creation_screen(shell_screen) || target_occupied {
        return None;
    }
    cursor
}

// ── the `04 3d` frame: the online slot list's character LOAD, as seen on the wire ──
//
// ★ 99차 (2026-09-16): moved here from `backup.rs` when the host-driven recovery backup
// (Track C, private server) was removed with the rest of the online features. What
// remains of that module is exactly this recogniser, because it is not about backups at
// all — it is a SAVE-SLOT PIN SOURCE (`vault::note_slot_loaded`), and the pin policy
// lives in this file.

/// The highest REAL slot number (130차: twelve slots, `slotwindow::TOTAL`). Everything
/// that carries a slot IDENTITY — the pin, a record name's tail, the redirect target — is
/// in this space. `lib.rs` pins this literal to `slotwindow::REAL_SLOT_MAX`.
pub const SLOT_MAX: u32 = 11;

/// The highest slot number the GUEST can name: its list has three rows, and its cursor,
/// its `04 3d` frame byte and its record names all say `0..=2`. A value from the guest is
/// checked against THIS and then turned into a real slot with `slotwindow::real_slot`;
/// checking it against [`SLOT_MAX`] would accept a byte the guest can never produce.
pub const GUEST_SLOT_MAX: u32 = 2;

/// Bytes of the decrypted record the `04 3d` upload carries — the save's FIRST block, the
/// one `serialiseSave` writes with `0x1973d0(&cursor=0, buf)`: 25 bytes of header + 9
/// equipment records of 20 + a 14-byte tail.
#[allow(dead_code)] // the recogniser deliberately does not check it (see below); the selftest does
pub const BACKUP_LEN: usize = 0xdb; // 219

/// `(cat, op)` of the backup UPLOAD — and, on this client, of a character LOAD.
pub const BACKUP_UPLOAD: (u8, u8) = (0x04, 0x3d);

/// The slot a `04 3d` backup-upload frame names, or `None` if this is not one.
///
/// ★★★ 71차-L — THE SAVE-SLOT PIN'S EXACT SOURCE.
///
/// The online slot list's OK arm (`0xa8f0`, key −5) reads the row's record, builds
/// this frame from it, **and calls `deserialiseSave` (`0x1a964`) on the same buffer**
/// before sending — so a `04 3d` on the wire means "the character in this slot is now
/// the live one". `0x1a964` has exactly two callers in the image (that arm and world
/// init `0x3ac5c`), so this frame plus the in-play latch cover every character load,
/// and the slot rides in the frame instead of being inferred from the slot cursor —
/// which the very next 뒤로 (`0xa900`) resets to 0.
///
/// ★ The frame goes out whoever is listening: in the 100 % offline world (99차) the
/// in-app engine receives it, so the pin source is unchanged.
///
/// ⛔ Do not tighten this into a total-length equality. The guest's arm passes `0xdc`
/// to its blob writer, but the frame on the wire is 226 bytes ([`BACKUP_LEN`] 219 plus
/// the header, slot and cat/op — device-captured). A `227` check would never match,
/// the pin would stop being re-armed, and nothing would say so. The self-consistent
/// length prefix and the `(cat, op)` pair are already exact: `0x3d` appears at six other
/// guest sites and every one of them is `05 3d` (the warehouse page request), while
/// `01 3d` is the login.
pub fn backup_upload_slot(frame: &[u8]) -> Option<u8> {
    if frame.len() <= 7 {
        return None;
    }
    let declared = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if declared != frame.len() || (frame[4], frame[5]) != BACKUP_UPLOAD {
        return None;
    }
    (u32::from(frame[6]) <= GUEST_SLOT_MAX).then_some(frame[6])
}

/// FNV-1a 64 of a character-name slice. Stable across gold/bag/level changes;
/// changes when `loadSaveRecord` replaces the name.
pub fn identity_hash(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// How old a worker cursor observation may be before [`redirect_slot`] stops
/// trusting it. The observation is refreshed every worker poll (~100 ms); a value
/// this far past that only trips when the worker itself has stalled — and a stalled
/// worker means the identity may be stale, which is the one thing a redirect must
/// never act on.
pub const REDIRECT_FRESH_MS: u64 = 3_000;

/// Where a save-record write aimed at `target` should actually LAND: the pinned
/// slot, when everything says the write is the pinned character's own save behind
/// a drifted slot cursor. `None` = no opinion, fall back to allow/refuse.
///
/// ── WHY REDIRECT AND NOT JUST REFUSE (2026-08-25, 70차) ──────────────────────
/// `CHAR+0xa410` is the slot-list CURSOR, and entering the online menu zeroes it
/// (`0xa876`/`0xa88a`) while the character stays loaded. From that moment EVERY
/// save the game makes — the warehouse's own `saveGame()` on 올리기/내리기/구입,
/// the `04 3d` backup-ack rewrite, and every field save after returning — is aimed
/// at the wrong slot. The 67차 pin refused those writes, which protected the other
/// slot but silently STOPPED SAVING the player: a withdrawn item was gone from the
/// warehouse file instantly while the bag that received it was never written, so
/// killing the app made the item vanish (the 캐시템 내리기 증발 report). The write
/// is by construction the pinned character's serialized state — `writeSaveRecord`
/// has no other source — so the correct act is to land it where that character
/// lives.
///
/// Every clause fails CLOSED (toward "no redirect", i.e. toward 67차 behaviour):
/// * a pin must exist;
/// * the worker's last observation must be FRESH ([`REDIRECT_FRESH_MS`]);
/// * if the pin has an identity, the observed live identity must equal it — a
///   `05 3e` restore or a title-screen load of another character changes
///   `CHAR+0xb8` and kills this clause on the next poll (and the restore path
///   suspends the observation synchronously, see `backup::guard_slot_info`'s
///   caller);
/// * the observed cursor must equal the write's own target — the write was formed
///   from the same word we observed, so a mismatch means the cursor moved under us.
///
/// ★★★ 71차-L — WHAT `pinned_id` IS, AND WHY `0 == 0` IS ALLOWED TO MATCH.
///
/// `identity_hash(CHAR+0xb8)` is NOT a character name. The only code in the image that
/// writes `CHAR+0xb8` is `0x49e64`, called from world init `0x3ab68` alone, and its
/// argument is the CLASS byte at `CHAR+0xb6` indexed into `res/dat/char_dat` — so the
/// value is 리츠 / 케이, and `deserialiseSave` (`0x1a964`) never touches it. Two
/// consequences the policy has to live with:
///   * two characters of the SAME class are indistinguishable here, and
///   * a session that reached the online warehouse without ever entering the world has
///     the field still at zero, so BOTH ids read 0.
/// Refusing the `0 == 0` pairing therefore does not buy safety — it just stops the
/// warehouse from saving in exactly the flow the player uses (70차 증발). The safety
/// comes from the PIN being exact instead: `vault::note_slot_loaded` re-aims it on
/// every `04 3d` (the online slot list's own load), and the in-play latch covers the
/// other loader. ⛔ Do not weaken that and lean on the identity instead.
pub fn redirect_slot(
    pin: Option<u8>,
    pinned_id: u64,
    observed_cursor: Option<u8>,
    observed_id: u64,
    observed_age_ms: u64,
    target: u8,
) -> Option<u8> {
    let p = pin?;
    if p == target || observed_age_ms > REDIRECT_FRESH_MS {
        return None;
    }
    if observed_cursor != Some(target) {
        return None;
    }
    // Both sides must agree. `0 == 0` counts (see above): it means "no class name has
    // been written yet", which is a state, not an unknown — and the pin is exact.
    (observed_id == pinned_id).then_some(p)
}

/// What may happen to a character record aimed at some slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Do it where the guest aimed.
    Allow,
    /// Do nothing. The record on disk keeps its character.
    Refuse,
    /// Write the same bytes at THIS slot instead — the pinned character's own save
    /// behind a drifted cursor. Never produced for a delete.
    Redirect(u8),
}

/// ★★★ THE WHOLE SAVE-SLOT POLICY, in one pure function (71차-E).
///
/// The player's rule, 2026-08-26, after a slot was destroyed: *"어떠한 경우에서도 현재
/// 있는 세이브가 다른 세이브로 대체되는 케이스는 발생해선 안돼."* It is stated here as a
/// single decision so it can be READ and TESTED as one, instead of being spread across
/// `vault::write_check` where only a Linux `cargo test` could reach it.
///
/// The invariant: **an occupied character record is only ever replaced by its own
/// character.** Everything else is refused, or redirected to where the writer belongs.
///
/// * `pin` / `pinned_id` — the loaded character's slot and identity, or `None`.
/// * `target` / `target_len` — the slot the guest aimed at, and what is on disk there.
/// * `redirect` — [`redirect_slot`]'s answer, already computed from the observation.
///
/// ★★★ 71차-L — `Some(p) if p == target => Allow` is only ever safe because the pin is
/// EXACT. It asks where the pin points and nothing else, so a pin that was a GUESS
/// turned this arm into a silent overwrite of whatever row the guess named: that is
/// how 71차-K's execute pin destroyed a slot. The guard is therefore not a clause
/// here — it is the rule that `LIVE_SLOT` may only ever be written by an observed
/// character LOAD (`vault::note_slot_loaded` on `04 3d`, and the two in-play latches).
/// ⛔ Do not add a pin source that is not a load. Gating this arm on `pinned_id`
/// instead does NOT work — see [`redirect_slot`] for why that field is a class name.
pub fn write_verdict(pin: Option<u8>, target: u8, target_len: u64, redirect: Option<u8>) -> Verdict {
    match pin {
        // The pinned character's own slot: always its own to overwrite.
        Some(p) if p == target => Verdict::Allow,
        // Someone else's slot. The bytes are still a real save, so land them where they
        // belong if the observation proves whose they are; otherwise leave both alone.
        Some(_) => match redirect {
            Some(slot) => Verdict::Redirect(slot),
            // ⛔ An EMPTY foreign slot used to be allowed here ("worst case is a
            // duplicate, not a destruction"). It is refused now: a duplicate the player
            // never made is still a save appearing by itself, and 71차-E's whole point
            // is that the drifted cursor decides nothing any more.
            None if target_len == 0 => Verdict::Refuse,
            None => Verdict::Refuse,
        },
        // No pin. A loaded character always arms one (71차-E), so the only legitimate
        // writer here is a BRAND-NEW character's first save — and the game makes you
        // 초기화 a slot before you may create in it, so its target is empty.
        None if target_len == 0 => Verdict::Allow,
        None => Verdict::Refuse,
    }
}

/// The same policy for a DELETE. There is no redirect: deleting the pinned character's
/// record because the guest aimed at someone else's would be the worst outcome
/// available. ⛔ 초기화 (`#`) does not come through here — that is a whole-database
/// delete, and it stays allowed because the player asked for it.
pub fn delete_verdict(pin: Option<u8>, target: u8, target_len: u64) -> Verdict {
    match pin {
        Some(p) if p == target => Verdict::Allow,
        _ if target_len == 0 => Verdict::Allow,
        _ => Verdict::Refuse,
    }
}

/// `GameSave_11_0` re-aimed at `slot`: `GameSave_11_2`. `None` when the name is not
/// a character record or has no `_<slot>` tail to rewrite — never invent a name.
pub fn renamed_slot(db_name: &str, slot: u8) -> Option<String> {
    if !is_vaulted(db_name) {
        return None;
    }
    let (head, tail) = db_name.rsplit_once('_')?;
    let _: u8 = tail.parse().ok()?;
    Some(format!("{head}_{slot}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: u64 = DAY_MS;

    #[test]
    fn only_the_two_character_records_are_vaulted() {
        for n in ["GameSave_11_0", "GameSave_11_2", "SlotSave_11_0", "GameSave_1_0"] {
            assert!(is_vaulted(n), "{n} is a character record");
        }
        for n in ["OptionSave_11", "wie_scratch", "", "Hero3Net", "gamesave_11_0"] {
            assert!(!is_vaulted(n), "{n} is not");
        }
        // A nested name must be judged on its leaf, not its path.
        assert!(is_vaulted("sub/GameSave_11_0"));
        assert!(!is_vaulted("GameSave/other"));
        // ★ 90차: the summary is sized against `SUMMARY_LEN`, the payload against the band.
        assert!(is_summary("SlotSave_11_2") && is_summary("sub/SlotSave_11_0"));
        assert!(!is_summary("GameSave_11_2") && !is_summary("OptionSave_11") && !is_summary("SlotSave/other"));
    }

    #[test]
    fn generation_names_round_trip_and_sort_chronologically() {
        let a = gen_name(1_800_000_000_000, 1408);
        let b = gen_name(1_800_000_001_000, 1824);
        assert_eq!(parse_gen(&a), Some((1_800_000_000_000, 1408)));
        assert_eq!(parse_gen(&b), Some((1_800_000_001_000, 1824)));
        assert!(a < b, "lexicographic order must be time order: {a} !< {b}");
        // Junk in the directory is ignored, not misread.
        for j in ["", "1.bin", "x.bin", "1-2.bin", "00000000000001-.bin", "0000000000000a-1.bin", "1408.bin"] {
            assert_eq!(parse_gen(j), None, "{j:?} must not parse");
        }
    }

    /// ★★★ 72차-D (TODO `BB` ⓐ) — the pair copy's naming rule.
    #[test]
    fn the_summary_partner_is_named_from_the_payload_and_never_the_other_way() {
        assert_eq!(summary_partner("GameSave_11_0").as_deref(), Some("SlotSave_11_0"));
        assert_eq!(summary_partner("GameSave_11_2").as_deref(), Some("SlotSave_11_2"));
        // The build number is not ours to predict — KTF and LGT disagree on it.
        assert_eq!(summary_partner("GameSave_1_0").as_deref(), Some("SlotSave_1_0"));
        assert_eq!(summary_partner("db/GameSave_11_1").as_deref(), Some("SlotSave_11_1"));
        // ⛔ No reverse direction: a summary write must not drag a 1.4 KB payload copy.
        assert_eq!(summary_partner("SlotSave_11_0"), None);
        // Not a character record, or no slot tail to carry across.
        assert_eq!(summary_partner("OptionSave_11"), None);
        assert_eq!(summary_partner("GameSave"), None);
        assert_eq!(summary_partner("GameSave_11_x"), None);
        assert_eq!(summary_partner("gamesave_11_0"), None);
        // Whatever it produces must be a name the rest of the vault already handles.
        let p = summary_partner("GameSave_11_1").expect("a payload record has a partner");
        assert!(is_vaulted(&p), "the partner must itself be a vaulted record");
        assert_eq!(renamed_slot(&p, 2).as_deref(), Some("SlotSave_11_2"));
    }

    /// ★★★ 72차 §1-2 — the boot heal may never paint a row over a payload the guest
    /// cannot load. `vault.rs` is `include!`d by no standalone selftest, so this is the
    /// only place the rule is checked on the Windows host that builds the APK.
    #[test]
    fn a_summary_is_only_ever_installed_over_a_payload_the_game_can_load() {
        // (payload_loadable, payload_restored, summary_damaged, summary_ok)
        assert!(summary_install_allowed(true, false, true, false), "intact payload, damaged summary: the 72차-D case");
        assert!(summary_install_allowed(true, true, true, false), "payload just restored, damaged summary");
        assert!(summary_install_allowed(true, true, false, false), "payload restored beside an absent summary");
        assert!(!summary_install_allowed(true, true, false, true), "a healthy summary is never rewritten");
        assert!(!summary_install_allowed(true, false, false, true), "nothing wrong: no write");

        // ⛔ THE COLUMN THE OLD `g_damage &&` GUARD LET THROUGH. The payload cannot be
        // loaded, so the row would advertise a character whose OK arm opens 새 캐릭터 생성
        // (`0xaf38`) and whose `#` is 초기화 (`0xaf56`) rather than the inert 복구 modal.
        assert!(!summary_install_allowed(false, false, true, false), "UNREADABLE payload + damaged summary");
        assert!(!summary_install_allowed(false, false, false, false), "absent payload, nothing to pair");
        for restored in [false, true] {
            for dmg in [false, true] {
                for ok in [false, true] {
                    assert!(
                        !summary_install_allowed(false, restored, dmg, ok),
                        "no loadable payload must never install a summary: {restored} {dmg} {ok}"
                    );
                }
            }
        }
    }

    /// A wire frame the way the guest builds one: `[u32 LE total][cat][op][body]`.
    fn wire(cat: u8, op: u8, body: &[u8]) -> Vec<u8> {
        let total = (6 + body.len()) as u32;
        let mut v = total.to_le_bytes().to_vec();
        v.push(cat);
        v.push(op);
        v.extend_from_slice(body);
        v
    }

    /// ★★★ 71차-L — the pin source, held to the frame the device actually produces
    /// (226 bytes: 4 + 2 + slot + 219). A `227` equality would have made the recogniser
    /// dead on arrival and silently un-pinned the save. (Moved from `backup_selftest.rs`
    /// in 99차 with the function.)
    #[test]
    fn a_backup_upload_names_the_slot_whose_character_was_just_loaded() {
        let upload = |slot: u8| {
            let mut body = vec![slot];
            body.extend_from_slice(&[0xab; BACKUP_LEN]);
            wire(0x04, 0x3d, &body)
        };
        for slot in 0..=GUEST_SLOT_MAX as u8 {
            let f = upload(slot);
            assert_eq!(f.len(), 226, "the device-captured size");
            assert_eq!(backup_upload_slot(&f), Some(slot));
        }
        // ⛔ Everything else on the wire must be invisible to it.
        assert_eq!(backup_upload_slot(&wire(0x05, 0x3d, &[0])), None, "warehouse page request");
        assert_eq!(backup_upload_slot(&wire(0x01, 0x3d, &[0; 16])), None, "login");
        assert_eq!(backup_upload_slot(&wire(0x04, 0x3e, &[1])), None, "slot info / 복구");
        assert_eq!(backup_upload_slot(&wire(0x01, 0x3c, &[])), None, "hello");
        // A slot the game does not have is not a load.
        let mut bad = upload(0);
        bad[6] = GUEST_SLOT_MAX as u8 + 1;
        assert_eq!(backup_upload_slot(&bad), None);
        // A frame whose declared length disagrees with the buffer is not understood.
        let mut torn = upload(1);
        torn.push(0);
        assert_eq!(backup_upload_slot(&torn), None);
        // Too short to carry a slot at all.
        assert_eq!(backup_upload_slot(&wire(0x04, 0x3d, &[2])), None);
    }

    /// ★★★ 99차 — the runtime repair obeys the same rule as the boot heal: no fresh row
    /// over a payload the game cannot load, and no rewrite of a healthy row.
    #[test]
    fn the_runtime_repair_never_paints_a_row_over_an_unloadable_payload() {
        assert!(summary_rebuild_allowed(true, true), "loadable payload, damaged row: rebuild");
        assert!(!summary_rebuild_allowed(true, false), "a healthy row is never rewritten");
        assert!(!summary_rebuild_allowed(false, true), "MD5-bad / unreadable payload: the row would be the 0xaf38 funnel");
        assert!(!summary_rebuild_allowed(false, false), "nothing there, nothing to do");
        // The two predicates must agree wherever both apply (no payload was restored, and
        // a damaged summary is by definition not `ok`).
        for loadable in [false, true] {
            for dmg in [false, true] {
                assert_eq!(
                    summary_rebuild_allowed(loadable, dmg),
                    summary_install_allowed(loadable, false, dmg, !dmg),
                    "boot heal and runtime repair disagree at loadable={loadable} damaged={dmg}"
                );
            }
        }
    }

    /// ★★★ 72차-D (TODO `BB` ⓑ) — when the host may rebuild a slot summary.
    #[test]
    fn a_summary_is_rebuilt_only_when_it_is_damaged_and_the_character_is_there() {
        const GOOD: usize = 1408;
        // The two shapes the report describes: no summary at all, and a summary of the
        // wrong size (the 16..31 band passes `seek(END) > 0` and then fails the MD5).
        assert!(summary_repair_needed(GOOD, Err(true)), "absent summary over a real character");
        assert!(summary_repair_needed(GOOD, Ok(0)), "a zero-byte summary is the same 빈슬롯");
        for n in [1u64, 16, 31, 33, 64, 1408] {
            assert!(summary_repair_needed(GOOD, Ok(n)), "{n} B is not a summary");
        }
        // ⛔ A healthy pair is never touched — a host that rewrites summaries on every
        // save is 72차-C's rejected proposal 4.
        assert!(!summary_repair_needed(GOOD, Ok(SUMMARY_LEN as u64)));
        // ⛔ No character, no row. The local 게임시작 arm reads the PAYLOAD, and when
        // that read fails it switches the shell to 새 캐릭터 생성 (`0xaf38`) — so a row
        // painted by a summary alone lies about what pressing it does.
        for absent in [0usize, 1, 32, 63, 1 << 20] {
            assert!(!summary_repair_needed(absent, Err(true)), "{absent} B payload is not a character");
            assert!(!summary_repair_needed(absent, Ok(0)));
        }
        // ⛔ Unreadable is not damage (the [`target_len`] doctrine).
        assert!(!summary_repair_needed(GOOD, Err(false)), "EIO/EACCES must not trigger a write");
    }

    /// The copy is stamped with the payload generation's OWN timestamp, so pairing is
    /// not a coincidence that a slow disk could break — it holds by construction.
    #[test]
    fn a_partner_copied_under_the_payloads_timestamp_always_pairs() {
        let t = 1_800_000_000_000u64;
        assert!(summary_pairs(t, t, SUMMARY_LEN), "the copy pairs with what it was copied for");
        // …which is exactly why the copy refuses a summary of any other size: it would
        // be written, listed, chosen as the nearest partner, and then rejected.
        for bad in [0usize, 1, RECORD_HEADER, SUMMARY_LEN - 1, SUMMARY_LEN + 1, 1408] {
            assert!(!summary_pairs(t, t, bad), "{bad} B is not a slot summary");
        }
    }

    #[test]
    fn a_record_length_band_rather_than_a_constant() {
        // save.rs: the archived backup was 1392 + 0x10, a live LV.12 save 1808 + 0x10.
        assert!(plausible_game_record(1392 + 0x10));
        assert!(plausible_game_record(1808 + 0x10));
        // The failures the notice exists for.
        assert!(!plausible_game_record(0));
        assert!(!plausible_game_record(32)); // a SlotSave-sized file where a GameSave belongs
        assert!(!plausible_game_record(1 << 20));
    }

    #[test]
    fn the_newest_six_always_survive() {
        let gens: Vec<(u64, usize)> = (0..10).map(|i| (1_000_000 + i * 60_000, 1408)).collect();
        let del = to_delete(&gens, None);
        for i in 10 - KEEP_NEWEST..10 {
            assert!(!del.contains(&i), "generation {i} is one of the newest and must survive");
        }
        // ★ 72차: this used to assert `del.len() == 10 - KEEP_NEWEST`, i.e. that NOTHING
        // outside the newest six survives a single day. [`SPREAD_MS`] deliberately breaks
        // that: these ten are one minute apart, so the 1-minute and 5-minute bands each
        // hold their oldest member back. Keeping MORE than the tiers strictly require is
        // the point of the ladder, so the assertion is now a ceiling, not an equality.
        assert!(del.len() <= 10 - KEEP_NEWEST, "the ladder may keep extra, never fewer");
        assert!(del.len() < 10 - KEEP_NEWEST, "…and here it does keep extra");
    }

    /// ★ The regression this pins: the reports arrived a DAY after the incident, so
    /// a flat "keep the newest six" window would already have thrown the good copy
    /// away. One generation per day has to survive.
    #[test]
    fn one_generation_per_day_survives_a_busy_session() {
        let mut gens = Vec::new();
        // Three days ago, two days ago, yesterday: one save each.
        for d in 1..=3u64 {
            gens.push((10 * D - d * D, 1408));
        }
        // Today: a long session, twenty autosaves.
        for i in 0..20u64 {
            gens.push((10 * D + i * 300_000, 1408));
        }
        let del = to_delete(&gens, None);
        for (i, g) in gens.iter().enumerate() {
            if g.0 < 10 * D {
                assert!(!del.contains(&i), "the day-tier generation at {} was deleted", g.0);
            }
        }
        assert!(gens.len() - del.len() <= KEEP_MAX, "the ceiling has to hold");
        assert!(gens.len() - del.len() >= KEEP_NEWEST + 3);
    }

    #[test]
    fn the_ceiling_holds_even_with_many_days() {
        // One save a day for a month: the day tier can only keep KEEP_DAYS of them.
        let gens: Vec<(u64, usize)> = (0..30u64).map(|i| (i * D, 1408)).collect();
        let del = to_delete(&gens, None);
        let kept = gens.len() - del.len();
        assert!(kept <= KEEP_MAX, "kept {kept} > {KEEP_MAX}");
        // And the newest is never among the casualties.
        assert!(!del.contains(&29));
    }

    #[test]
    fn nothing_is_deleted_while_the_vault_is_small() {
        for n in 0..=KEEP_NEWEST {
            let gens: Vec<(u64, usize)> = (0..n as u64).map(|i| (i * 1000, 1408)).collect();
            assert!(to_delete(&gens, None).is_empty(), "{n} generations should all survive");
        }
    }

    #[test]
    fn identical_and_empty_writes_are_not_vaulted() {
        let old = b"abcd".as_slice();
        assert!(!should_vault(old, Some(b"abcd"), 0, 1_000_000, MIN_GAP_MS), "no change, no copy");
        assert!(!should_vault(b"", Some(b"abcd"), 0, 1_000_000, MIN_GAP_MS), "nothing to lose");
        assert!(should_vault(old, Some(b"abcde"), 0, 1_000_000, MIN_GAP_MS), "a real change");
        // A DELETE is always worth capturing: mode-4 open deletes record 1 before
        // anything is written, which is the widest data-loss window there is.
        assert!(should_vault(old, None, 0, 1_000_000, MIN_GAP_MS));
    }

    #[test]
    fn the_rate_limit_spaces_copies_without_ever_blocking_the_first() {
        assert!(should_vault(b"a", Some(b"b"), 0, 0, MIN_GAP_MS), "last_ms 0 = never vaulted");
        assert!(!should_vault(b"a", Some(b"b"), 1_000_000, 1_000_000 + MIN_GAP_MS - 1, MIN_GAP_MS));
        assert!(should_vault(b"a", Some(b"b"), 1_000_000, 1_000_000 + MIN_GAP_MS, MIN_GAP_MS));
        // A clock that jumps backwards must not open the floodgates.
        assert!(!should_vault(b"a", Some(b"b"), 1_000_000, 500_000, MIN_GAP_MS));
    }

    // ── the restore pin (66차: 복원이 복구 슬롯을 덮는다) ──────────────────────

    #[test]
    fn the_pin_marker_can_never_be_read_as_a_generation() {
        // The sidecar shares the vault directory with the generations, so the one thing
        // that must never happen is an enumerator taking it for one of them.
        assert_eq!(parse_gen(PIN_FILE), None);
        assert_eq!(parse_pin(&pin_content(1_800_000_000_000, 1408)), Some((1_800_000_000_000, 1408)));
        // Hand-edited files: CRLF, a blank first line, trailing junk.
        let text = format!("\r\n  {}  \r\nsomething a player typed\n", gen_name(1_700_000_000_000, 64));
        assert_eq!(parse_pin(&text), Some((1_700_000_000_000, 64)));
        assert_eq!(parse_pin(""), None);
        assert_eq!(parse_pin("not a generation\n"), None);
    }

    #[test]
    fn duplicates_pin_is_exact_bytes_or_nothing() {
        assert!(duplicates_pin(b"abcd", Some(b"abcd")));
        assert!(!duplicates_pin(b"abcd", Some(b"abce")), "same length, different content");
        assert!(!duplicates_pin(b"abcd", Some(b"abcd0")));
        assert!(!duplicates_pin(b"abcd", None), "no pin = normal policy");
        assert!(!duplicates_pin(b"", Some(b"")), "an empty record is never 'the restored one'");
    }

    #[test]
    fn a_pinned_generation_is_never_rotated_out() {
        // Thirty generations ten seconds apart — a refine grind. Index 1 is deep enough to
        // be outside the newest six and is NOT its band's representative (index 0, the
        // oldest, is), so without a pin it is exactly the kind of generation rotation
        // takes. ⚠ 72차: index 0 is no longer a valid subject — the age ladder keeps the
        // oldest member of each band, so the oldest generation of all is now protected by
        // the policy itself and could not tell a pin apart from an ordinary survivor.
        let gens: Vec<(u64, usize)> = (0..30u64).map(|i| (1_800_000_000_000 + i * 10_000, 1408)).collect();
        let pin = gens[1];
        assert!(to_delete(&gens, None).contains(&1), "precondition: this one normally goes");
        assert!(!to_delete(&gens, Some(pin)).contains(&1), "the pin must survive");
    }

    #[test]
    fn the_pin_does_not_consume_one_of_the_newest_six() {
        let gens: Vec<(u64, usize)> = (0..20u64).map(|i| (1_800_000_000_000 + i * 60_000, 1408)).collect();
        let dead = to_delete(&gens, Some(gens[0]));
        // The six newest are still all there — the pin was held outside the tiers.
        for i in 14..20 {
            assert!(!dead.contains(&i), "generation {i} is one of the newest six");
        }
        assert!(!dead.contains(&0), "and the pin as well");
        // Exactly one more generation survives than without the pin.
        assert_eq!(dead.len() + 1, to_delete(&gens, None).len());
    }

    #[test]
    fn the_ceiling_still_holds_with_a_pin() {
        // Thirty daily generations: the day tier alone would keep more than the ceiling.
        let gens: Vec<(u64, usize)> = (0..30u64).map(|i| (1_800_000_000_000 + i * DAY_MS, 1408)).collect();
        let pin = gens[0];
        let dead = to_delete(&gens, Some(pin));
        assert!(gens.len() - dead.len() <= KEEP_MAX, "the pin must not raise the ceiling");
        assert!(!dead.contains(&0), "…and must not be the one trimmed to meet it");
    }

    #[test]
    fn a_pin_we_no_longer_hold_changes_nothing() {
        let gens: Vec<(u64, usize)> = (0..20u64).map(|i| (1_800_000_000_000 + i * 60_000, 1408)).collect();
        // A pin naming a generation that is not on disk (deleted by hand, or stale).
        assert_eq!(to_delete(&gens, Some((1, 1))), to_delete(&gens, None), "a pin naming a generation we do not hold");
        // …and a pin whose timestamp exists but whose LENGTH does not is equally stale:
        // the pair is the identity, because two records of different sizes are two
        // different characters however close their timestamps.
        assert_eq!(to_delete(&gens, Some((gens[3].0, 1))), to_delete(&gens, None), "a pin with the wrong length");
    }

    /// ★★★ 71차-E: the shell latch. Every line here is a step of the reported repro.
    #[test]
    fn the_shell_pins_on_a_load_and_never_on_a_cursor_walk() {
        // 타이틀 → 온라인: nothing loaded yet, so nothing to pin.
        assert!(!shell_adopts(None, 0, 0), "no character resolved — must not pin");
        // 슬롯 1 을 골라 로드 → 이름이 나타난다. 첫 캐릭터이므로 채택.
        assert!(shell_adopts(None, 0, 0xaaa), "first load of the session pins its slot");
        // 창고 안, 같은 캐릭터: 커서가 그대로든 움직였든 유지.
        assert!(!shell_adopts(Some(0), 0xaaa, 0xaaa), "same character — hold");
        // 취소 → 슬롯 2 로 이동. 게임은 캐릭터를 언로드하지 않으므로 신원이 같다.
        // ⛔ 이것이 슬롯을 파괴하던 순간이다.
        assert!(!shell_adopts(Some(0), 0xaaa, 0xaaa), "cursor walked to another slot — HOLD");
        // 실제로 다른 캐릭터가 로드되면 그때는 따라간다.
        assert!(shell_adopts(Some(0), 0xaaa, 0xbbb), "a different character really was loaded");
        // 신원을 못 읽는 순간에는 절대 채택하지 않는다 — in-play 규칙과 반대 방향이다.
        assert!(!shell_adopts(Some(0), 0xaaa, 0), "unreadable identity — never adopt in the shell");
        assert!(!shell_adopts(None, 0, 0), "…not even with no pin at all");
        // 핀의 신원이 안 읽혔던 상태에서 읽히는 신원이 오면 로드로 본다.
        assert!(shell_adopts(Some(2), 0, 0xaaa), "an unreadable latch id yields to a readable one");
    }

    /// ★★★ 71차-J: the slot we pin is the cursor FROM THE POLL BEFORE the name
    /// appeared, not the cursor now. The vc177 overwrite is this function returning
    /// `Some(1)` for a character that was loaded at slot 0.
    #[test]
    fn load_pin_slot_uses_the_cursor_from_before_the_name_appeared() {
        // 슬롯 1 을 고른 채 로드 → 다음 폴에 이름이 뜨고 커서는 이미 2번.
        assert_eq!(load_pin_slot(Some(1), 0xaaa, Some(0), 0, false), Some(0));
        // 같은 캐릭터, 커서만 움직임 → 로드가 아님.
        assert_eq!(load_pin_slot(Some(1), 0xaaa, Some(0), 0xaaa, false), None);
        assert_eq!(load_pin_slot(Some(1), 0xaaa, Some(1), 0xaaa, false), None);
        // 이름 없는 폴은 핀을 움직이지 않는다.
        assert_eq!(load_pin_slot(Some(1), 0, Some(0), 0, false), None);
        // 이전 관측이 없으면 (프로세스 첫 폴) 지금 커서가 유일한 단서.
        assert_eq!(load_pin_slot(Some(0), 0xaaa, None, 0, false), Some(0));
        // 다른 캐릭터가 로드됨: 직전 폴의 커서가 그 로드가 읽은 워드.
        assert_eq!(load_pin_slot(Some(2), 0xbbb, Some(1), 0xaaa, false), Some(1));
        // 타이틀에서 2번을 고른 채 게임시작: 직전 커서 1, 이름 등장 → 핀 1.
        assert_eq!(load_pin_slot(Some(1), 0xbbb, Some(1), 0, false), Some(1));
        // ★ 71차-K: 실행으로 이미 핀이 있으면 이름이 떠도 핀을 안 옮긴다.
        // vc178 가 이걸로 1번 캐릭터를 2번에 썼다.
        assert_eq!(load_pin_slot(Some(1), 0xaaa, Some(0), 0, true), None);
        // 진짜 다른 캐릭터 로드는 핀이 있어도 따른다.
        assert_eq!(load_pin_slot(Some(1), 0xbbb, Some(1), 0xaaa, true), Some(1));
    }

    /// ★★★ 71차-L — THE REPORTED SEQUENCE, as a test.
    ///
    /// > 네트워크 접속 → A선택 → 뒤로가기 → B선택 → 뒤로가기 → 뒤로가기(메인메뉴)
    /// > … 저장완료되고나서 A세이브파일이 B세이브로 변경이 되는 이슈
    ///
    /// A is slot 0, B is slot 1. Executing a row on the online slot list LOADS that
    /// row's character (`0xa976` → `deserialiseSave 0x1a964`) and sends `04 3d <row>`,
    /// so the host re-aims the pin from the frame — that is `vault::note_slot_loaded`,
    /// modelled here as the `pin` variable simply following each execute. The last
    /// 뒤로 (`0xa900`) then sets the slot cursor to 0 while CHAR still holds B, which is
    /// why every write after it aims at slot 0.
    ///
    /// ⛔ 71차-K froze the pin on the FIRST execute, so it still read slot 0 while B was
    /// loaded — and slot 0 is exactly where the drifted cursor aims. `p == target` made
    /// that a silent Allow, and B overwrote A.
    #[test]
    fn the_second_execute_moves_the_pin_so_the_drifted_cursor_cannot_eat_slot_a() {
        // No class name has been written: this session went title → 온라인 → 슬롯,
        // never through world init, so CHAR+0xb8 is still zero on both sides.
        const UNSET: u64 = 0;

        // A선택 — slot 0 is loaded, and the ack's saveGame aims at the cursor, still 0.
        let pin = Some(0u8);
        assert_eq!(write_verdict(pin, 0, 1408, redirect_slot(pin, UNSET, Some(0), UNSET, 100, 0)), Verdict::Allow);

        // B선택 — slot 1 is loaded. THE PIN MUST MOVE. Its own saves are ordinary.
        let pin = Some(1u8);
        assert_eq!(write_verdict(pin, 1, 1824, redirect_slot(pin, UNSET, Some(1), UNSET, 100, 1)), Verdict::Allow);

        // 뒤로가기(메인메뉴) — cursor := 0 with B still loaded. THIS is the destructive
        // aim, and the whole report. It must land on slot 1, never on slot 0.
        let redirect = redirect_slot(pin, UNSET, Some(0), UNSET, 100, 0);
        assert_eq!(redirect, Some(1), "B's bytes belong in B's slot");
        assert_eq!(
            write_verdict(pin, 0, 1408, redirect),
            Verdict::Redirect(1),
            "slot 0 keeps A, and B is still saved — refusing here is the 70차 증발"
        );

        // ⛔ THE REGRESSION, stated as the thing that must not happen: with the pin
        // frozen at the first execute, the same write is a bare Allow onto slot 0.
        assert_eq!(
            write_verdict(Some(0), 0, 1408, redirect_slot(Some(0), UNSET, Some(0), UNSET, 100, 0)),
            Verdict::Allow,
            "documented so it is obvious WHY the pin must follow every load — this input must never occur"
        );
    }

    /// ★★★ 71차-M — the record READ is the load signal, and it must be picky.
    ///
    /// Every assertion here is a line from the vc180 device trace that produced the
    /// 세이브1 loss, or a read that appears in that same trace and must NOT move the pin.
    #[test]
    fn a_character_record_read_names_the_slot_being_loaded() {
        // The exact read that preceded the destroyed write.
        assert_eq!(load_read_slot("GameSave_11_1", 1, 1840), Some(1));
        assert_eq!(load_read_slot("GameSave_11_0", 1, 1408), Some(0));
        assert_eq!(load_read_slot("GameSave_11_2", 1, 1408), Some(2));
        // ⛔ The 빈슬롯 scan reads EVERY slot's summary on every slot screen — following
        // it would walk the pin across all three slots each time the list is drawn.
        assert_eq!(load_read_slot("SlotSave_11_0", 1, 32), None);
        assert_eq!(load_read_slot("SlotSave_11_1", 1, 1840), None, "not even at a save-like length");
        // Not a character record at all.
        assert_eq!(load_read_slot("OptionSave_11", 1, 32), None);
        // Only record 1 is the character.
        assert_eq!(load_read_slot("GameSave_11_1", 2, 1840), None);
        // A truncated / empty / absurd read is not a load.
        assert_eq!(load_read_slot("GameSave_11_1", 1, 0), None);
        assert_eq!(load_read_slot("GameSave_11_1", 1, 8), None);
        assert_eq!(load_read_slot("GameSave_11_1", 1, 1_000_000), None);
        // A name with no slot tail cannot be aimed.
        assert_eq!(load_read_slot("GameSave", 1, 1408), None);
        assert_eq!(load_read_slot("GameSave_11_x", 1, 1408), None);
        // Full paths are accepted (the caller passes the directory leaf, but be robust).
        assert_eq!(load_read_slot("db/GameSave_11_2", 1, 1408), Some(2));
    }

    /// ★★★ 71차-L: `pinned_id` is a CLASS name, so it can be zero for a whole session
    /// and it cannot tell two characters of the same class apart.
    ///
    /// The only writer of `CHAR+0xb8` is `0x49e64`, reached from world init `0x3ab68`
    /// alone, and its index is the class byte at `CHAR+0xb6` into `res/dat/char_dat`.
    /// `deserialiseSave` never touches it. So the redirect must keep working with the
    /// field at zero — refusing there would stop the warehouse saving in exactly the
    /// flow the player uses — and it must NOT be treated as proof of identity.
    #[test]
    fn the_identity_field_is_a_class_name_not_a_character_name() {
        // Zero on both sides is the normal state of 타이틀 → 온라인 → 슬롯 → 창고.
        assert_eq!(redirect_slot(Some(1), 0, Some(0), 0, 100, 0), Some(1));
        // A mismatch still blocks: a 05 3e restore or a title load of another class.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0xbbb, 100, 0), None);
        assert_eq!(redirect_slot(Some(1), 0, Some(0), 0xbbb, 100, 0), None);
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0, 100, 0), None);
        // ⛔ Two characters of the SAME class hash the same. The pin must therefore come
        // from an observed LOAD, never from the identity agreeing.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0xaaa, 100, 0), Some(1));
    }

    // (`save_lock_releases_after_a_quiet_write_or_a_cap` went with the lock, 136차.)

    /// ★★★ 71차-E: the one rule the player stated — an occupied record is only ever
    /// replaced by its own character. Every branch, in one place.
    #[test]
    fn an_occupied_record_is_only_ever_replaced_by_its_own_character() {
        // The pinned character's own slot.
        assert_eq!(write_verdict(Some(1), 1, 1408, None), Verdict::Allow);
        assert_eq!(write_verdict(Some(1), 1, 0, None), Verdict::Allow, "…even when it is empty");
        // Another slot, and the observation proves whose bytes these are.
        assert_eq!(write_verdict(Some(1), 2, 1408, Some(1)), Verdict::Redirect(1));
        // Another slot with no proof: both records keep what they have.
        assert_eq!(write_verdict(Some(1), 2, 1408, None), Verdict::Refuse);
        // ⛔ An EMPTY foreign slot too. 70차 allowed this ("a duplicate, not a
        // destruction") and the player's rule does not have that exception.
        assert_eq!(write_verdict(Some(1), 2, 0, None), Verdict::Refuse);
        // No pin: only a brand-new character, whose slot was 초기화'd, may write.
        assert_eq!(write_verdict(None, 2, 0, None), Verdict::Allow);
        assert_eq!(write_verdict(None, 2, 1408, None), Verdict::Refuse, "no pin may not replace a character");
    }

    /// A delete never redirects — it would remove the very character it was meant to
    /// protect. `MC_dbOpenDataBase(mode 4)` + no write is what reaches this.
    #[test]
    fn a_delete_can_only_ever_touch_the_pinned_slot() {
        assert_eq!(delete_verdict(Some(1), 1, 1408), Verdict::Allow);
        assert_eq!(delete_verdict(Some(1), 2, 1408), Verdict::Refuse);
        assert_eq!(delete_verdict(None, 2, 1408), Verdict::Refuse);
        // Nothing there to lose.
        assert_eq!(delete_verdict(Some(1), 2, 0), Verdict::Allow);
        assert_eq!(delete_verdict(None, 2, 0), Verdict::Allow);
        // ⛔ No input produces a Redirect.
        for pin in [None, Some(0), Some(1), Some(2)] {
            for target in 0..=2u8 {
                for len in [0u64, 1408] {
                    assert!(!matches!(delete_verdict(pin, target, len), Verdict::Redirect(_)));
                }
            }
        }
    }

    #[test]
    fn pin_holds_only_when_the_cursor_drifted_under_the_same_character() {
        assert!(!pin_holds(None, 1, 0, 1), "no latch yet — adopt");
        assert!(!pin_holds(Some(1), 1, 1, 1), "cursor still matches — refresh is fine");
        // ★ 71차-L: …but a refresh must never write a ZERO name over a known one, so an
        // unreadable live identity holds even when the cursor is right where it should be.
        assert!(pin_holds(Some(1), 0xaaa, 1, 0), "cursor matches but the name went unreadable");
        assert!(pin_holds(Some(1), 0xaaa, 0, 0xaaa), "same character, cursor walked away");
        assert!(!pin_holds(Some(1), 0xaaa, 2, 0xbbb), "a different character was loaded");
        assert!(!pin_holds(Some(1), 0, 0, 0xaaa), "unreadable latch id — adopt");
        // ★ 71차-L: an unreadable LIVE id now holds a proven pin. Adopting on no
        // evidence used to be the cheap direction; since 71차-L an unproven pin
        // refuses every write, so adopting is the expensive one.
        assert!(pin_holds(Some(1), 0xaaa, 0, 0), "unreadable live id — keep the proven pin");
    }

    /// ★★★ 148차 — the device trace, poll by poll (S25 vc420, 2026-10-01 00:25:23–30).
    /// Slot 1 = class A (0xa), slot 0 = class B (0xb). The worker's pin latch is
    /// `load_pin_slot` → `pin_holds`, with `note_slot_loaded` setting the pin in between.
    #[test]
    fn a_load_between_two_polls_does_not_let_the_outgoing_cursor_name_the_pin() {
        let (a, b) = (0xa_u64, 0xb_u64);
        // Last poll in slot 1's world: cursor 1, class A. Then, inside two ticks, the record
        // store sees `GameSave_11_0` read → the pin is slot 0 (latched name = the OLD class,
        // `OBS_ID`, because world init has not written the new one yet).
        let (prev_cursor, prev_id) = (Some(1u8), a);
        let (pin, pin_id) = (Some(0u8), a);
        // First poll after the load: world init has run — cursor 0, class B.
        let (cursor, identity) = (Some(0u8), b);

        // THE BUG: the previous poll's cursor names the outgoing world.
        let stale = load_pin_slot(cursor, identity, prev_cursor, prev_id, true);
        assert_eq!(stale, Some(1), "vc420: `live character is slot 1 (was 0)`");
        assert!(!pin_holds(pin, pin_id, 1, identity), "…and nothing held it back");
        // Once adopted, the wrong pin is DEFENDED: the name matches, the cursor 'drifted'.
        assert!(pin_holds(Some(1), b, 0, b), "`cursor is 0 but the live character is still slot 1` x40");

        // THE FIX: the load itself is the evidence.
        let evidence = pin_evidence_cursor(prev_cursor, pin);
        assert_eq!(evidence, Some(0));
        let s = load_pin_slot(cursor, identity, evidence, prev_id, true);
        assert_eq!(s, Some(0), "the new name is stamped onto the loaded slot");
        assert!(!pin_holds(pin, pin_id, 0, identity), "adopted — same slot, new name");
        // The polls after it agree and move nothing.
        assert_eq!(load_pin_slot(cursor, identity, cursor, identity, true), None);
        assert!(!pin_holds(Some(0), b, 0, b), "cursor matches the pin — a refresh, not a hold");

        // World init a poll LATE (the name still the old one on the first poll): nothing
        // fires, and the next poll's own previous cursor is already the loaded one.
        assert_eq!(load_pin_slot(cursor, a, evidence, prev_id, true), None);
        assert_eq!(load_pin_slot(cursor, b, cursor, a, true), Some(0));

        // No load since the last poll: the rule is exactly the old one.
        assert_eq!(pin_evidence_cursor(Some(2), None), Some(2));
        assert_eq!(pin_evidence_cursor(None, None), None);
    }

    #[test]
    fn identity_hash_is_stable_and_name_sensitive() {
        assert_eq!(identity_hash(b"\xc4\xc9\xc0\xcc"), identity_hash(b"\xc4\xc9\xc0\xcc"));
        assert_ne!(identity_hash(b"\xc4\xc9\xc0\xcc"), identity_hash(b"\xb1\xe8\xb8\xb2"));
        assert_ne!(identity_hash(b""), 0, "empty is a real hash, not the unreadable sentinel");
    }

    #[test]
    fn redirect_fires_only_for_the_pinned_character_behind_a_fresh_drifted_cursor() {
        // The warehouse case this exists for: pin slot 1, cursor zeroed by the online
        // menu, same character observed a moment ago, write aimed at slot 0.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0xaaa, 100, 0), Some(1));
        // Field saves after returning from the shell drift the same way.
        assert_eq!(redirect_slot(Some(2), 0xaaa, Some(0), 0xaaa, 2_999, 0), Some(2));
        // Writes already aimed at the pin need no redirect.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(1), 0xaaa, 100, 1), None);
    }

    #[test]
    fn redirect_refuses_every_uncertain_input() {
        assert_eq!(redirect_slot(None, 0xaaa, Some(0), 0xaaa, 100, 0), None, "no pin");
        // ★ 71차-K: an execute pin with no name yet STILL redirects — otherwise
        // every warehouse save behind the zeroed cursor is refused (70차 증발).
        // ★ 71차-L: `0 == 0` matches — it means "no class name has been written yet",
        // which is the normal state of a session that went straight to the warehouse.
        // The pin is exact (`vault::note_slot_loaded`), so it does not need the name.
        assert_eq!(redirect_slot(Some(1), 0, Some(0), 0, 100, 0), Some(1), "no class name yet, pin is exact");
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0, 100, 0), None, "live identity unreadable (title screen)");
        // A 05 3e restore / title load replaced the character: the observed identity
        // no longer matches the pin, so the write is NOT the pinned character's.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(2), 0xbbb, 100, 2), None, "different character loaded");
        // A stalled worker means a stale identity — never act on it.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0xaaa, REDIRECT_FRESH_MS + 1, 0), None, "stale observation");
        // The write targets a slot we never observed the cursor on: it moved under us.
        assert_eq!(redirect_slot(Some(1), 0xaaa, Some(0), 0xaaa, 100, 2), None, "cursor/target mismatch");
        assert_eq!(redirect_slot(Some(1), 0xaaa, None, 0xaaa, 100, 0), None, "no cursor observed");
    }

    /// ★★★ 76차 — the report, step by step, against the real policy. Slot 0's character
    /// (class 케이) was loaded earlier in the process; the player now creates ANOTHER 케이 in
    /// empty slot 2. No record read succeeds (that is what opens the creation screens), so
    /// no load pins slot 2; the identity is the same class hash, so the pin holds on 0; and
    /// the first save, aimed at 2, is exactly what a drifted-cursor save looks like.
    #[test]
    fn a_created_character_of_the_same_class_was_redirected_onto_the_old_slot() {
        let class = identity_hash(b"\xc4\xc9\xc0\xcc");
        assert_eq!(load_read_slot("GameSave_11_2", 1, 0), None, "an empty slot is not a load");
        assert_eq!(load_pin_slot(Some(2), class, Some(2), class, true), None, "same class: not seen as a load");
        assert!(pin_holds(Some(0), class, 2, class), "…and the pin stays on slot 0");
        let redirect = redirect_slot(Some(0), class, Some(2), class, 100, 2);
        assert_eq!(redirect, Some(0), "the drift signature matches");
        assert_eq!(write_verdict(Some(0), 2, 0, redirect), Verdict::Redirect(0), "THE BUG: the new character lands on slot 0");
        // The creation screens name the slot. Pinned there, the same write is the character's own.
        let pin = creation_slot(Some(12), Some(2), false);
        assert_eq!(pin, Some(2));
        assert_eq!(write_verdict(pin, 2, 0, redirect_slot(pin, class, Some(2), class, 100, 2)), Verdict::Allow);
        // A DIFFERENT class never needed this: the identity change is a load to `load_pin_slot`.
        let other = identity_hash(b"\xb8\xae\xc3\xf7");
        assert_eq!(load_pin_slot(Some(2), other, Some(2), class, true), Some(2));
        assert!(!pin_holds(Some(0), class, 2, other));
        // A fresh process (no pin) never needed it either — but pins from the first write now.
        assert_eq!(write_verdict(None, 2, 0, None), Verdict::Allow);
    }

    #[test]
    fn creation_slot_needs_a_creation_screen_and_a_cursor() {
        for s in CREATION_SCREENS {
            assert!(on_creation_screen(Some(s)));
            assert_eq!(creation_slot(Some(s), Some(1), false), Some(1), "screen {s} is a creation screen");
            assert_eq!(creation_slot(Some(s), Some(0), false), Some(0));
        }
        // Main menu · online list · warehouse · 게임시작 list · login notice · shell init value.
        for s in [0, 1, 2, 3, 9, 13, 0xffff_ffff] {
            assert!(!on_creation_screen(Some(s)));
            assert_eq!(creation_slot(Some(s), Some(1), false), None, "screen {s} is not a creation screen");
        }
        assert!(!on_creation_screen(None));
        assert_eq!(creation_slot(None, Some(1), false), None, "no anchor (KTF) → inert");
        assert_eq!(creation_slot(Some(12), None, false), None, "no cursor → nothing to pin");
    }

    /// ★★★ 78차 — the 2026-09-03 report: «1슬롯으로 접속 → 보스 → 저장 → 메인화면 을 반복하니
    /// 2슬롯이 1슬롯의 세이브로 덮였고, 가끔 2슬롯이 빈슬롯이 된다».
    ///
    /// The shell's screen word has NO reset (see [`creation_slot`]), so once the player has
    /// been on a creation screen the word can still read 10/11/12 much later. Without the
    /// occupancy guard, the creation pin then re-aims at whatever slot the drifting cursor
    /// names, and `write_verdict`'s `p == target => Allow` writes the RESIDENT character
    /// over the character that lives there.
    #[test]
    fn a_stale_creation_screen_may_not_pin_an_occupied_slot() {
        let class = identity_hash(b"\xc4\xc9\xc0\xcc");
        // Slot 1 is loaded and pinned; the cursor has drifted onto slot 2, which is occupied.
        assert_eq!(creation_slot(Some(12), Some(2), true), None, "an occupied slot was never created just now");
        // …so the policy falls back to the drift rules, which protect slot 2.
        let redirect = redirect_slot(Some(1), class, Some(2), class, 100, 2);
        assert_eq!(redirect, Some(1));
        assert_eq!(write_verdict(Some(1), 2, 1408, redirect), Verdict::Redirect(1), "slot 2 keeps its character");
        // With no proof at all it is refused outright, never allowed.
        assert_eq!(write_verdict(Some(1), 2, 1408, None), Verdict::Refuse);
        // ⛔ THE BUG, for the record: had the pin moved, the write would have been allowed.
        assert_eq!(write_verdict(Some(2), 2, 1408, None), Verdict::Allow);
        // …and the delete gate would have opened on slot 2 as well — the 「빈슬롯」 half.
        assert_eq!(delete_verdict(Some(2), 2, 1408), Verdict::Allow);
        assert_eq!(delete_verdict(Some(1), 2, 1408), Verdict::Refuse);
        // The 76차 case itself is untouched: an EMPTY slot named by the creation screens.
        assert_eq!(creation_slot(Some(12), Some(2), false), Some(2));
        assert_eq!(write_verdict(Some(2), 2, 0, None), Verdict::Allow);
    }

    #[test]
    fn on_slot_list_is_only_the_two_screens_whose_ok_reads_a_record() {
        assert!(on_slot_list(Some(2)), "온라인 슬롯 목록");
        assert!(on_slot_list(Some(9)), "게임시작 슬롯 목록");
        for s in [0, 1, 3, 10, 11, 12, 13] {
            assert!(!on_slot_list(Some(s)), "screen {s} is not mid-load");
        }
        assert!(!on_slot_list(None), "no anchor → do not block warehouse repair");
    }

    #[test]
    fn renamed_slot_rewrites_only_a_real_slot_tail() {
        assert_eq!(renamed_slot("GameSave_11_0", 2).as_deref(), Some("GameSave_11_2"));
        assert_eq!(renamed_slot("SlotSave_11_1", 0).as_deref(), Some("SlotSave_11_0"));
        assert_eq!(renamed_slot("OptionSave_11", 1), None, "not a character record");
        assert_eq!(renamed_slot("GameSave_11_x", 1), None, "no numeric slot tail to rewrite");
        assert_eq!(renamed_slot("GameSave", 1), None, "no tail at all");
    }
}
