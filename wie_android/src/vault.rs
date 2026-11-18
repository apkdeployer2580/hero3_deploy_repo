//! Filesystem half of the save vault — see [`crate::savevault`] for the policy
//! and for WHY this exists (two players lost characters in one week and nothing
//! on the device kept a second copy).
//!
//! ── WHERE IT SITS ────────────────────────────────────────────────────────────
//! The record store lays a database out as `<data>/<app_id>/db/<name>/<record_id>`
//! (`database.rs`). The vault is its sibling: `<data>/<app_id>/vault/<name>/` with
//! one file per kept generation. Deriving it from the record path rather than from
//! `data_dir` is deliberate — `Database` is constructed with nothing else, and a
//! path that is always two directories up from the record cannot drift out of sync
//! with wherever the store actually lives.
//!
//! ── WHEN IT RUNS ─────────────────────────────────────────────────────────────
//! [`before_overwrite`] is called from `Database::set` and `Database::delete`,
//! with the bytes that are about to stop existing. That is on the emulator thread,
//! inside a WIPI DB call, so everything here is bounded: one small read, at most
//! one small write, and a directory listing whose size the rotation policy caps at
//! [`crate::savevault::KEEP_MAX`]. The write it piggybacks on already `fsync`s
//! 1.4 KB, which costs far more than this does.
//!
//! Every failure is logged and swallowed. A vault that cannot write must never be
//! the reason a save fails.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::saverecord::{self, Verdict};
use crate::savevault::{self, MIN_GAP_MS};

/// Last vault write per record directory, for the rate limit. Keyed by the record
/// store's name (`GameSave_11_0`), not the full path: the same name is the same
/// character however the store was opened.
static LAST_MS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Set once the first record write lands, so the restore UI can find the vault
/// without the settings code having to know the guest's app id.
static VAULT_ROOT: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The guest's app id (`PID:` of the LGT archive's `app_info`, e.g. `PD119037`) — the
/// directory the record store lives under: `<data>/<app_id>/db/<name>/<record>`.
///
/// ★★★ 99차 (2026-09-16), TODO `CG`. Before this, [`db_roots`] found the store by SCANNING
/// `data_dir` for any `<x>/db` and `heal_at_boot` / `check_at_boot` took the FIRST hit in
/// directory order. A device that had once run a KTF build has TWO such directories (the
/// KTF pid and the LGT pid), so on the S9 the six boot lines said `Absent` for every slot
/// while SLOT 1 had a character — the check had looked in the dead KTF store, and so had
/// the heal, and `CIPHER_PROVEN` stayed unarmed until the guest's own slot-list scan.
/// `lib.rs` sets this from the bundled archive BEFORE any of those run, so the root is the
/// one the emulator is about to use, not whichever entry `read_dir` happened to yield.
static APP_ID: OnceLock<String> = OnceLock::new();

/// Tell the vault which app id the running archive uses. Set once; later calls are
/// ignored (the archive does not change within a process).
pub fn set_app_id(app_id: &str) {
    let id = app_id.trim();
    if id.is_empty() {
        return;
    }
    if APP_ID.set(id.to_string()).is_ok() {
        log::info!("vault: record store app id = {id}");
    }
}

/// ⛔ TEST ONLY. The three `#[test]` fns in this file share `VAULT_ROOT`, `LAST_MS`,
/// `TABLE`, `BOOT_NOTICE` and `REFUSALS`, and libtest runs them on a thread pool (nothing
/// in this repo sets `test-threads = 1`). Each module header already says "ONE test fn,
/// because the statics are process-global" — that discipline stops at the MODULE boundary,
/// and 90차 added a third module that both READS the `VAULT_ROOT` memo (`heal_at_boot` ->
/// `db_roots`) and SETS it (`install_generation` -> `keep_pre_restore`). Whichever module
/// latches first then answers for the other's temp store, and `heal_tests`' exact
/// `refusal_count()` delta counts `redirect_tests`' seven refusals.
///
/// ⛔ Do NOT "fix" this in `db_roots` by requiring the memo to lie under `data_dir`.
/// `Path::starts_with` does not normalise, Android hands back both `/data/user/0/<pkg>`
/// and `/data/data/<pkg>` for the same directory, and the fallback on a mismatch is a
/// directory SCAN whose first entry `heal_at_boot` then WRITES into. The memo is right on
/// device; it is the tests that need isolating.
#[cfg(test)]
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Serialise one test fn and give it a clean [`VAULT_ROOT`].
///
/// `unwrap_or_else(|e| e.into_inner())` on purpose: a `Mutex` poisons on panic, and one
/// failing assertion must not become three failures that all point at the lock instead of
/// at the bug.
#[cfg(test)]
fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut r) = VAULT_ROOT.lock() {
        *r = None; // set-once is correct in production; each test owns its own store
    }
    g
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `<...>/db/<name>/<id>` -> `(<...>/vault/<name>, "<name>")`.
///
/// `None` when the path does not have that shape or the name is not a character
/// record — both mean "not our business", and the caller does nothing.
fn vault_dir_for(record_path: &Path) -> Option<(PathBuf, String)> {
    let db_dir = record_path.parent()?;
    let name = db_dir.file_name()?.to_str()?.to_string();
    if !savevault::is_vaulted(&name) {
        return None;
    }
    let db_root = db_dir.parent()?; // .../db
    let app_root = db_root.parent()?; // .../<app_id>
    let dir = app_root.join("vault").join(&name);
    if let Ok(mut root) = VAULT_ROOT.lock() {
        if root.is_none() {
            *root = Some(app_root.join("vault"));
        }
    }
    Some((dir, name))
}

/// Capture `record_path`'s CURRENT bytes before they are replaced by `new`
/// (`None` = the record is being deleted).
///
/// This is the whole point of the vault: the copy is of a version the game was
/// actually running on, taken at the one moment we are certain it is about to
/// stop existing. Nothing here judges whether the old or the new bytes are the
/// good ones — see `savevault::should_vault`.
pub fn before_overwrite(record_path: &Path, new: Option<&[u8]>) {
    let Some((dir, name)) = vault_dir_for(record_path) else {
        return;
    };
    let Ok(old) = fs::read(record_path) else {
        return; // no previous record: nothing to lose
    };

    // ★ The first save after a RESTORE would otherwise vault a byte-identical copy of
    // the generation just restored — `should_vault` cannot see that the vault already
    // holds those bytes, and its rate limit never fires because a restore is applied at
    // boot, before the emulator (and so before any vault write) exists. That duplicate
    // pushed a real generation out on every restore (66차 제보). Overwrites only: a
    // DELETE is the one event that can never be re-taken, so it keeps its pre-image even
    // when the pin already holds the same bytes.
    if new.is_some() && savevault::duplicates_pin(&old, pinned_bytes(&dir, old.len()).as_deref()) {
        log::info!("vault: {name} still holds the restored generation — not vaulting a duplicate");
        return;
    }

    let now = now_ms();
    let last = LAST_MS
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|m| m.get(&name).copied()))
        .unwrap_or(0);
    if !savevault::should_vault(&old, new, last, now, MIN_GAP_MS) {
        return;
    }

    if let Err(e) = fs::create_dir_all(&dir) {
        log::warn!("vault: cannot create {dir:?}: {e}");
        return;
    }
    let file = dir.join(savevault::gen_name(now, old.len()));
    // ★★ 72차 — ATOMIC, like the record it copies. The old comment argued a plain
    // `fs::write` was safe here because "its length is recorded in its own name, so a
    // short one is visible". Nothing ever looked: `generations()` took the length from
    // the NAME and never stat'd the file, so a torn copy was listed at its nominal size,
    // chosen by `nearest_generation`, and written verbatim onto the live record. Both
    // halves of that are fixed — this write cannot tear, and `generations()` now checks.
    match crate::database::write_atomic(&file, &old) {
        Ok(()) => {
            log::info!("vault: kept {name} @ {now} ({} bytes, replaced by {:?})", old.len(), new.map(|n| n.len()));
            if let Ok(mut g) = LAST_MS.lock() {
                g.get_or_insert_with(HashMap::new).insert(name.clone(), now);
            }
            rotate(&dir);
            keep_summary_partner(record_path, &name, now);
        }
        Err(e) => log::warn!("vault: cannot write {file:?}: {e}"),
    }
}

/// Copy the summary that was live alongside a just-vaulted payload into ITS vault,
/// under the same timestamp.
///
/// ★★★ 72차-D (TODO `BB` ⓐ) — this is what makes a restore possible at all. See
/// [`savevault::summary_partner`] for why the pair drifts apart on its own: the four
/// highest-frequency save paths call `saveGame` (`0x4748`), which writes the payload
/// and never calls `slotSummary`, so a warehouse or 제련 session fills the payload
/// vault with generations whose partner is hours old — and [`restore`] refuses those,
/// correctly but uselessly.
///
/// Everything here is already-owned facts: the bytes come off the host's own disk, the
/// timestamp is the payload capture's, and the slot comes from the record path. No
/// guest call, no record write, and nothing reads `CHAR+0xa410`.
///
/// ★ It also takes over the summary's rate-limit slot. The paired writers (제련
/// `0x254a0`, `04 3d` ack `0x47f2`) write the payload first and the summary a few ms
/// later, and the summary on disk does not change in between — so the capture that
/// would otherwise follow is the same bytes we just copied. Stamping [`LAST_MS`] here
/// declines that duplicate and, more usefully, keeps the two directories on the SAME
/// timestamps, so rotation thins them in step instead of pulling them apart.
fn keep_summary_partner(record_path: &Path, payload_name: &str, ms: u64) {
    let Some(partner_name) = savevault::summary_partner(payload_name) else {
        return; // a summary, or not a character record: nothing to pair
    };
    let Some(record_id) = record_path.file_name() else { return };
    let Some(db_root) = record_path.parent().and_then(|p| p.parent()) else { return };
    let src = db_root.join(&partner_name).join(record_id);
    let Ok(bytes) = fs::read(&src) else {
        // Normal for a brand-new character (no summary written yet) and for 초기화,
        // which deletes the two databases one after the other.
        log::info!("vault: no {partner_name} on disk to pair with {payload_name} @ {ms}");
        return;
    };
    // ⛔ Only a real summary. `slotSummary` (`0x1a6d0`) always writes exactly
    // `SUMMARY_LEN`, so anything else is the damage this feature exists to undo — and
    // `savevault::summary_pairs` would reject it later anyway, after it had taken a
    // seat in the partner's vault.
    if bytes.len() != savevault::SUMMARY_LEN {
        log::warn!(
            "vault: {partner_name} is {} bytes (expected {}) — not pairing it with {payload_name} @ {ms}",
            bytes.len(),
            savevault::SUMMARY_LEN
        );
        return;
    }
    let Some((dir, _)) = vault_dir_for(&src) else { return };
    let file = dir.join(savevault::gen_name(ms, bytes.len()));
    if file.exists() {
        return; // already paired at this instant
    }
    if let Err(e) = fs::create_dir_all(&dir) {
        log::warn!("vault: cannot create {dir:?}: {e}");
        return;
    }
    match crate::database::write_atomic(&file, &bytes) {
        Ok(()) => {
            log::info!("vault: paired {partner_name} @ {ms} with {payload_name} ({} bytes)", bytes.len());
            if let Ok(mut g) = LAST_MS.lock() {
                g.get_or_insert_with(HashMap::new).insert(partner_name, ms);
            }
            rotate(&dir);
        }
        Err(e) => log::warn!("vault: cannot write {file:?}: {e}"),
    }
}

// ── record integrity: the host reads the game's format (90차, 2026-09-13) ─────
//
// ★★★ WHY. Every "빈슬롯" is `loadSaveRecord` failing ONE test — MD5(plaintext) ==
// header — and until now the host could not run that test, so it stored whatever bytes
// the guest handed it, verbatim, and called it a successful save. `saverecord.rs` is the
// game's cipher in host code; these three uses are what it buys:
//   * `record_acceptable` — a write that the game itself could never read back is
//     REFUSED, the previous record stays, and the refusal line carries the bytes.
//   * `check_at_boot` / `heal_at_boot` — a record on disk that the game will read as
//     empty is named as such (MD5 BAD, not "32 bytes — ok") and, when the vault holds a
//     generation the game CAN read, put back before the guest ever looks.
//   * `Database::get` — the moment the guest reads a broken record is logged, so the
//     row reading 빈슬롯 and its cause sit on the same line.

/// `res/dat/des_dat` as the host uses it. Set once at boot from the shipped archive
/// (`install_table`); the rebuilt textbook table stands in until then and whenever the
/// asset cannot be read.
static TABLE: OnceLock<Vec<u8>> = OnceLock::new();

/// Adopt the archive's own `des_dat`. The rebuilt table (`saverecord::game_table`) is
/// expected to be byte-identical — a difference is logged, and the asset wins, so a
/// re-patched archive can never make every record look broken to the host.
pub fn install_table(asset: Option<&[u8]>) {
    let builtin = saverecord::game_table();
    let chosen = match asset {
        Some(t) if t.len() == saverecord::TABLE_LEN && saverecord::Des::new(t, &saverecord::KEY).is_some() => {
            if t != &builtin[..] {
                log::warn!("saverecord: the archive's des_dat differs from the rebuilt table — using the archive's");
            }
            t.to_vec()
        }
        Some(t) => {
            log::error!("saverecord: the archive's des_dat is unusable ({} bytes) — using the rebuilt table", t.len());
            builtin.to_vec()
        }
        None => {
            log::warn!("saverecord: no des_dat in the archive — using the rebuilt table");
            builtin.to_vec()
        }
    };
    // ★ fail closed, but SAY SO. If anything fixed the table before us then the archive's
    // `des_dat` is NOT in use and every verdict this process prints is the rebuilt table's
    // opinion. `let _ =` swallowed exactly that.
    if TABLE.set(chosen).is_err() {
        log::error!("saverecord: the des_dat table was already fixed before install_table ran — the archive's table is NOT in use");
    }
}

/// The table in force.
///
/// ⛔ NOT `TABLE.get_or_init` — A STAND-IN MUST NOT SEAL. The doc on [`TABLE`] says the
/// rebuilt table "stands in **until then**", and `get_or_init` made that a lie: any
/// `classify` reached before [`install_table`] fixed the rebuilt table for the whole
/// process, and `install_table`'s `TABLE.set` then lost with no log at all. The one boot
/// that reaches `classify` first is a boot with a pending restore (`apply_pending_restore`
/// runs `list` and `paired_summary`), i.e. the boot the player is counting on.
/// Harmless only because the two tables are byte-identical on the shipped archive
/// (`saverecord_selftest` pins that md5) — the day a re-patched archive diverges, the first
/// symptom would be that scheduled restore refusing itself with 「SlotSave 사본이 없습니다」.
fn table() -> &'static [u8] {
    static STANDIN: OnceLock<Vec<u8>> = OnceLock::new();
    match TABLE.get() {
        Some(t) => t.as_slice(),
        None => STANDIN.get_or_init(|| saverecord::game_table().to_vec()).as_slice(),
    }
}

/// Has this cipher validated at least one real record in this process?
///
/// ★ The one guard the write gate needs. The table and key are LGT 01.00.07's; a build
/// this host has never seen (a KTF archive, a differently keyed LGT) would make EVERY
/// record look broken, and a gate that then refused every save would be a worse bug than
/// the one it exists to catch. So the gate only REFUSES once one record has passed — the
/// boot scan of the slot list proves it within the first second on the shipped build —
/// and before that it only speaks. The boot heal needs no such guard: it acts only when
/// a vaulted generation validates, which is the same proof.
static CIPHER_PROVEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn note_proven() {
    if !CIPHER_PROVEN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::info!("saverecord: the cipher validated a real record — unreadable writes are refused from here on");
    }
}

pub fn cipher_proven() -> bool {
    CIPHER_PROVEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// One word for a record on disk, for the boot lines and the heal decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnDisk {
    /// No directory: the slot was 초기화'd or never used.
    Absent,
    /// The directory exists but record 1 does not — only `Database::delete(1)` leaves
    /// this shape, and nothing in the current guest path reaches it.
    NoRecord,
    /// The file exists but could not be read (EIO / EACCES) — not damage, not empty.
    Unreadable,
    /// Zero bytes.
    Empty,
    /// A size `slotSummary` / the writer never produces.
    BadSize(usize),
    /// The bytes fail the game's own MD5 test: the row reads 빈슬롯 or the load fails.
    Md5Bad(usize),
    /// The game will read it.
    Ok(usize),
}

impl OnDisk {
    pub fn is_ok(self) -> bool {
        matches!(self, OnDisk::Ok(_))
    }
    /// Damage the vault should try to undo. `Absent` is deliberately not damage (a
    /// 초기화'd slot must stay empty) and `Unreadable` is not evidence of anything.
    pub fn is_damage(self) -> bool {
        matches!(self, OnDisk::NoRecord | OnDisk::Empty | OnDisk::BadSize(_) | OnDisk::Md5Bad(_))
    }
}

/// Classify `<db>/<name>/1` the way the guest will experience it.
fn on_disk(db: &Path, name: &str) -> OnDisk {
    let dir = db.join(name);
    if !dir.is_dir() {
        return OnDisk::Absent;
    }
    let p = dir.join("1");
    let bytes = match fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return OnDisk::NoRecord,
        Err(_) => return OnDisk::Unreadable,
    };
    classify(name, &bytes)
}

/// [`on_disk`] on bytes already in hand.
pub fn classify(name: &str, bytes: &[u8]) -> OnDisk {
    if bytes.is_empty() {
        return OnDisk::Empty;
    }
    let summary = savevault::is_summary(name);
    let size_ok = if summary { bytes.len() == savevault::SUMMARY_LEN } else { savevault::plausible_game_record(bytes.len()) };
    if !size_ok {
        return OnDisk::BadSize(bytes.len());
    }
    match saverecord::check(bytes, table()) {
        Verdict::Valid { .. } => {
            note_proven();
            OnDisk::Ok(bytes.len())
        }
        Verdict::NoTable => OnDisk::Ok(bytes.len()),
        Verdict::TooShort | Verdict::Md5Mismatch => OnDisk::Md5Bad(bytes.len()),
    }
}

/// [`on_disk`] for one VAULTED GENERATION: what will the guest make of these bytes if a
/// restore puts them back?
///
/// ⛔ Returns [`OnDisk`], not a bool. `fs::read(..).map(..).unwrap_or(false)` was written
/// out at four sites and gave ONE answer to two different questions — "the game rejects
/// these bytes" and "this host could not OPEN the file". That is the conflation
/// [`savevault::target_len`] was written to undo in 72차, and [`list`] is the caller where
/// it costs a character: hiding a copy we merely could not read hides the only data
/// recovery UI the player has. Note that `fs::read` must OPEN the file, which
/// `generations`' `fs::metadata` never did, so this host can call a generation unreadable
/// more readily than `Database::get` (which retries once) ever would.
///
/// `NotFound` stays separate, because it is the one error that IS evidence: the copy is
/// gone, and [`rotate`] runs on the worker thread while the settings dialog is open, so a
/// generation really can vanish between `generations`' stat and this read.
///
/// ⛔ [`cipher_proven`] is deliberately NOT folded in. The boot heal acts only where this
/// says `Ok`, and that validation IS the proof the cipher fits this build (see
/// `CIPHER_PROVEN`); a guard in here would invert the heal into "install the newest
/// generation whatever it holds". [`list`] keeps the guard at its own call site, because
/// it uses this answer to HIDE a copy rather than to pick one.
fn generation_state(name: &str, path: &Path) -> OnDisk {
    match fs::read(path) {
        Ok(b) => classify(name, &b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => OnDisk::NoRecord,
        Err(_) => OnDisk::Unreadable,
    }
}

/// May this write replace a character record? `Database::set` asks BEFORE the slot
/// policy: a record the game cannot read is refused whatever slot it was aimed at.
///
/// The refusal is deliberately loud and carries the head of the bytes — a zeroed buffer,
/// a shifted record and a stale header each look different there, and that line is the
/// first evidence this project has ever had of WHAT the guest hands over when a slot
/// goes empty. The guest sees -1 (it ignores it), the deferred truncate is cancelled by
/// `stream_write`, and the previous record — the one the game could read — stays.
pub fn record_acceptable(name: &str, data: &[u8]) -> bool {
    if !savevault::is_vaulted(name) {
        return true;
    }
    let verdict = classify(name, data);
    if verdict.is_ok() {
        return true;
    }
    if !cipher_proven() {
        // Speak, but do not refuse: nothing has proved the cipher fits this build yet.
        log::error!(
            "saverecord: {name} is {verdict:?} ({} B) by this host's reading, but no record has validated in this process — stored unjudged. head = {}",
            data.len(),
            saverecord::hex_head(data, 48)
        );
        return true;
    }
    note_refusal();
    log::error!(
        "saverecord: REFUSED to store {name} — {verdict:?} ({} B): the game itself could not read these bytes back, so the record on disk is kept. head = {}",
        data.len(),
        saverecord::hex_head(data, 48)
    );
    false
}

/// Say when the guest is about to read a record it will treat as empty.
pub fn note_record_read(name: &str, bytes: &[u8]) {
    if !savevault::is_vaulted(name) {
        return;
    }
    let v = classify(name, bytes);
    if !v.is_ok() {
        log::error!(
            "saverecord: {name} is {v:?} — the guest reads it now and will report 빈슬롯 / fail the load. head = {}",
            saverecord::hex_head(bytes, 32)
        );
    }
}

/// A one-line notice for the first frame, set by [`heal_at_boot`] (the event loop does
/// not exist yet when boot runs, so it cannot toast directly).
static BOOT_NOTICE: Mutex<Option<&'static str>> = Mutex::new(None);

pub fn take_boot_notice() -> Option<&'static str> {
    BOOT_NOTICE.lock().ok().and_then(|mut g| g.take())
}

/// The newest vaulted generation of `name` that the game can read, as `(ms, path)`.
fn newest_valid_generation(vault_dir: &Path, name: &str) -> Option<(u64, PathBuf)> {
    generations(vault_dir)
        .into_iter()
        .rev()
        // ⛔ `.is_ok()` here, and NOT `!is_damage()`. This SELECTS bytes to install and the
        // caller's next act is a second `fs::read` of this same path
        // (`install_generation`), so a copy we cannot read cannot be put back whatever we
        // believe about it — and this is a `find`, so answering "maybe" would stop the
        // search and hide a readable OLDER generation behind the broken one.
        .find(|(_, _, p)| generation_state(name, p).is_ok())
        .map(|(ms, _, p)| (ms, p))
}

/// Put a vaulted generation back as `<db>/<name>/1`, parking whatever is there now.
fn install_generation(db: &Path, name: &str, ms: u64, src: &Path) -> Result<(), String> {
    let bytes = fs::read(src).map_err(|e| format!("cannot read {src:?}: {e}"))?;
    let dst_dir = db.join(name);
    fs::create_dir_all(&dst_dir).map_err(|e| format!("cannot create {name}: {e}"))?;
    let dst = dst_dir.join("1");
    // Pin first, capture second — the same order `restore` keeps, for the same reason.
    if let Some(vdir) = src.parent() {
        write_pin(vdir, ms, bytes.len());
    }
    keep_pre_restore(&dst, name);
    crate::database::write_atomic(&dst, &bytes).map_err(|e| format!("cannot write {name}: {e}"))?;
    log::warn!("vault: AUTO-RESTORED {name} from {ms} ({} bytes)", bytes.len());
    Ok(())
}

/// ★★★ Undo, at boot, the damage the player used to undo by hand.
///
/// For each slot, when either record is DAMAGE (`OnDisk::is_damage`) and the vault holds
/// a generation of it the game can read, that generation is installed — the current
/// bytes are parked in `pre-restore/` first, exactly as a manual restore does. Rules:
///
/// * A slot with BOTH directories absent is left alone: that is 초기화, or never used.
/// * `Unreadable` is left alone: acting on a record we cannot read is acting blind.
/// * A damaged payload takes the newest readable payload generation (progress since it
///   is lost, but it was unreadable anyway — the pre-image is parked).
/// * A damaged summary with an INTACT payload takes the newest readable summary
///   generation on its own. The row text may be a little stale (level, play time) but
///   the OK arm loads the intact payload, so nothing is lost; the game rewrites the
///   summary on its next paired save. When the payload is being restored too, the
///   summary nearest to that generation wins.
/// * Nothing to install → the line says so, and the slot stays as it was.
///
/// Returns a notice for the first frame when something was put back.
pub fn heal_at_boot(data_dir: &Path) -> Option<String> {
    let db = db_roots(data_dir).into_iter().next()?;
    let vault = db.parent()?.join("vault");
    let mut healed: Vec<String> = Vec::new();
    for slot in 0..crate::slotwindow::TOTAL {
        let game = format!("GameSave_11_{slot}");
        let summ = format!("SlotSave_11_{slot}");
        let g = on_disk(&db, &game);
        let s = on_disk(&db, &summ);
        if g == OnDisk::Absent && s == OnDisk::Absent {
            continue;
        }
        // ★ One record gone while its partner is still there is damage too (an
        // interrupted 초기화, or a directory that vanished): a summary restored over an
        // ABSENT payload would paint a row whose OK arm opens character creation —
        // the destructive funnel 72차 §1-2 describes — so the payload must come back
        // with it, or nothing does.
        let g_damage = g.is_damage() || (g == OnDisk::Absent && s != OnDisk::Absent);
        let s_damage = s.is_damage() || (s == OnDisk::Absent && g != OnDisk::Absent);
        if !(g_damage || s_damage) {
            continue;
        }
        log::error!("vault: slot {slot} is damaged on disk — {game}: {g:?}, {summ}: {s:?} — looking for a readable generation");
        let mut game_ms: Option<u64> = None;
        if g_damage {
            match newest_valid_generation(&vault.join(&game), &game) {
                Some((ms, p)) => match install_generation(&db, &game, ms, &p) {
                    Ok(()) => game_ms = Some(ms),
                    Err(e) => log::error!("vault: {game}: {e}"),
                },
                None => log::error!("vault: {game} is {g:?} and the vault holds no readable copy — the slot stays as it is"),
            }
        }
        // ★★ The verdict is `savevault::summary_install_allowed` and nothing else. The old
        // `g_damage && game_ms.is_none() && !g.is_ok()` was defeated by its own prefix:
        // `Unreadable` is not damage, so `g_damage` was false and control fell straight
        // through to the install — painting a row over a payload `Database::get` turns into
        // `None` for the guest (`seek(h, 0, END) == 0` → 빈슬롯). Its third term was dead
        // too: no `is_damage` variant is `Ok`, and neither is `Absent`.
        // ⛔ And note what else that column lost: with `g_damage` false the payload arm
        // above never ran, so the vault was not even SEARCHED for a generation that might
        // have repaired the slot outright.
        let payload_loadable = game_ms.is_some() || g.is_ok();
        if savevault::summary_install_allowed(payload_loadable, game_ms.is_some(), s_damage, s.is_ok()) {
            let sdir = vault.join(&summ);
            // Pair with the restored payload when there is one; otherwise the newest.
            let pick = match game_ms {
                Some(ms) => generations(&sdir)
                    .into_iter()
                    .filter(|(sms, len, _)| savevault::summary_pairs(ms, *sms, *len))
                    .filter(|(_, _, p)| generation_state(&summ, p).is_ok())
                    .min_by_key(|(sms, _, _)| sms.abs_diff(ms))
                    .map(|(sms, _, p)| (sms, p))
                    .or_else(|| newest_valid_generation(&sdir, &summ)),
                None => newest_valid_generation(&sdir, &summ),
            };
            match pick {
                Some((ms, p)) => {
                    if let Err(e) = install_generation(&db, &summ, ms, &p) {
                        log::error!("vault: {summ}: {e}");
                    }
                }
                None => log::error!("vault: {summ} is {s:?} and the vault holds no readable copy — the row will read 빈슬롯"),
            }
        } else if !payload_loadable {
            log::error!("vault: {summ} is left as it is — {game} is {g:?}, so there is no payload to pair it with");
        }
        // Say what the guest will see now.
        let g2 = on_disk(&db, &game);
        let s2 = on_disk(&db, &summ);
        log::warn!("vault: slot {slot} after healing — {game}: {g2:?}, {summ}: {s2:?}");
        if g2.is_ok() && s2.is_ok() {
            healed.push(format!("{}", slot + 1));
        }
    }
    if healed.is_empty() {
        return None;
    }
    let msg = format!("세이브 슬롯 {} 자동 복구", healed.join(","));
    if let Ok(mut g) = BOOT_NOTICE.lock() {
        *g = Some(Box::leak(msg.clone().into_boxed_str()));
    }
    Some(msg)
}

/// Every generation in `dir`, newest last. Junk files are ignored.
fn generations(dir: &Path) -> Vec<(u64, usize, PathBuf)> {
    let mut out: Vec<(u64, usize, PathBuf)> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let n = p.file_name()?.to_str()?;
                let (ms, len) = savevault::parse_gen(n)?;
                // ★ 72차: the name is a CLAIM about the length; verify it. A generation
                // whose bytes do not match its own name is a torn copy, and restoring one
                // installs a short record — which is the 빈 슬롯 the player is trying to
                // escape. A mismatch is dropped rather than repaired: we cannot know which
                // of the two numbers is the true one.
                let real = fs::metadata(&p).map(|m| m.len()).ok()?;
                if real != len as u64 {
                    log::warn!("vault: {p:?} claims {len} bytes but holds {real} — ignoring a torn generation");
                    return None;
                }
                Some((ms, len, p))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort_by_key(|(ms, _, _)| *ms);
    out
}

/// The pin sidecar for one record directory — see [`savevault::PIN_FILE`].
fn pin_path(dir: &Path) -> PathBuf {
    dir.join(savevault::PIN_FILE)
}

/// Which generation this record's pin names, or `None` when there is no pin (or it is
/// unreadable, which is the same thing: the pin only ever KEEPS data, so failing to
/// read one costs nothing but the seat it was saving).
fn read_pin(dir: &Path) -> Option<(u64, usize)> {
    savevault::parse_pin(&fs::read_to_string(pin_path(dir)).ok()?)
}

/// The pinned generation's bytes, but only when its recorded length matches `want_len`.
///
/// The length is in the pin's own file name, so the cheap check runs first and the
/// second file read happens only on a real candidate — this is called on the hot path
/// (every `stream_write` write-through), not once per restore.
fn pinned_bytes(dir: &Path, want_len: usize) -> Option<Vec<u8>> {
    let (ms, len) = read_pin(dir)?;
    if len != want_len {
        return None;
    }
    fs::read(dir.join(savevault::gen_name(ms, len))).ok()
}

/// Mark `(ms, len)` as the generation a restore brought back.
///
/// ⛔ Must be called BEFORE the pre-image capture in [`restore`]: the capture triggers a
/// `rotate` over this very directory, and without the pin already on disk that rotate
/// can delete the generation being restored (the deep-generation case in the 66차 report).
fn write_pin(dir: &Path, ms: u64, len: usize) {
    if let Err(e) = fs::create_dir_all(dir) {
        log::warn!("vault: cannot create {dir:?} for the pin: {e}");
        return;
    }
    match fs::write(pin_path(dir), savevault::pin_content(ms, len)) {
        Ok(()) => log::info!("vault: pinned {dir:?} @ {ms} ({len} bytes)"),
        Err(e) => log::warn!("vault: cannot write the pin in {dir:?}: {e}"),
    }
}

fn rotate(dir: &Path) {
    let gens = generations(dir);
    let keys: Vec<(u64, usize)> = gens.iter().map(|(ms, len, _)| (*ms, *len)).collect();
    for i in savevault::to_delete(&keys, read_pin(dir)) {
        if let Some((ms, _, path)) = gens.get(i) {
            match fs::remove_file(path) {
                Ok(()) => log::info!("vault: rotated out {dir:?} @ {ms}"),
                Err(e) => log::warn!("vault: cannot rotate {path:?}: {e}"),
            }
        }
    }
}

/// One vaulted character record, for the restore UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Record-store name, e.g. `GameSave_11_0`.
    pub name: String,
    /// When the copy was taken (unix ms).
    pub unix_ms: u64,
    /// Bytes in the copy.
    pub len: usize,
    pub path: PathBuf,
}

/// Everything the vault holds, newest first, `GameSave_*` only.
///
/// `SlotSave_*` copies exist and are restored alongside their `GameSave`, but they
/// are never OFFERED on their own: a 32-byte summary without its payload is a menu
/// entry for a character that is not there, which is precisely the state the
/// player is trying to get out of.
pub fn list(data_dir: &Path) -> Vec<Entry> {
    collect(data_dir, true)
}

/// Everything the vault holds, JUDGED BY NOTHING — the forensic view.
///
/// ★ [`export`] uses this and [`list`] does not. A generation the game cannot read is
/// exactly the artefact 90차 exists to collect — its own open question is 「게스트가 왜
/// 깨진 바이트를 넘기는가」 — and `/sdcard/Android/data/<pkg>/files` is the only channel it
/// can travel on without root. Filtering the export throws away the on-disk half of the
/// evidence on the one report that would explain it.
fn list_all(data_dir: &Path) -> Vec<Entry> {
    collect(data_dir, false)
}

fn collect(data_dir: &Path, judged: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    for root in vault_roots(data_dir) {
        let Ok(rd) = fs::read_dir(&root) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            if !name.starts_with("GameSave") {
                continue;
            }
            for (unix_ms, len, path) in generations(&e.path()) {
                // ★ 90차: never OFFER a copy the game cannot read — restoring one is the
                // 빈슬롯 the player is trying to leave.
                //
                // ⛔ …but a read that FAILS is not that verdict. The first version asked
                // `fs::read(..).map(..).unwrap_or(false)`, which folds EIO / EACCES / an
                // exhausted fd table into "the game rejects it". Judgement is WITHHELD
                // instead: the row is offered, and `restore` reads the file again as the
                // FIRST statement of its loop, so it fails with `cannot read {name}`
                // before it pins, parks or writes anything. Same rule as
                // `savevault::target_len` (72차) — unreadable is never empty.
                if judged {
                    let state = generation_state(&name, &path);
                    if state == OnDisk::NoRecord {
                        // Gone between the stat and this read: `rotate` runs on the worker
                        // thread while the dialog is open. That IS evidence of absence, so
                        // drop it — quietly, because it is not damage.
                        continue;
                    }
                    if state.is_damage() && cipher_proven() {
                        log::warn!("vault: {name} @ {unix_ms} is {state:?} — the game could not read it back, not offered");
                        continue;
                    }
                    if state == OnDisk::Unreadable {
                        log::error!("vault: {name} @ {unix_ms} ({len} B) cannot be READ by this host — offered unjudged rather than hidden");
                    }
                }
                out.push(Entry { name: name.clone(), unix_ms, len, path });
            }
        }
    }
    out.sort_by(|a, b| b.unix_ms.cmp(&a.unix_ms));
    out
}

/// Candidate vault roots. The remembered one (set by the first record write) is
/// authoritative; the scan is the fallback for a process that has not booted the
/// guest yet, which is exactly the case when the player opens the settings at the
/// title screen to recover a slot.
fn vault_roots(data_dir: &Path) -> Vec<PathBuf> {
    if let Some(root) = VAULT_ROOT.lock().ok().and_then(|g| g.clone()) {
        return vec![root];
    }
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(data_dir) {
        for e in rd.filter_map(|e| e.ok()) {
            let p = e.path().join("vault");
            if p.is_dir() {
                out.push(p);
            }
        }
    }
    out
}

/// Put a vaulted generation back, along with the matching `SlotSave` summary.
///
/// ★ The CURRENT record is vaulted first, unconditionally. A restore is a
/// destructive act on whatever is there now, and the player may pick the wrong
/// generation — so the state they restored over has to remain recoverable, or the
/// feature that exists to undo a data loss can cause one.
///
/// Returns the number of records replaced, or an error string for the toast.
pub fn restore(data_dir: &Path, entry: &Entry) -> Result<usize, String> {
    let Some(app_root) = entry.path.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) else {
        return Err("vault path shape".into());
    };
    let db_root = app_root.join("db");

    // ★★★ 72차 — THE PAIR IS THE UNIT. A restore that puts back only the payload was
    // reported to the player as a success and changed NOTHING about what they were
    // complaining about: 「빈슬롯」 on a slot ROW is decided by `SlotSave_11_<n>` alone
    // (guest scan `0x5c06` reads that record and nothing else), and the old code logged
    // `restoring the payload alone`, `continue`d, and returned `Ok(1)`.
    //
    // The partner is now chosen by VALIDITY, not by proximity. `nearest_generation` was an
    // unbounded `min_by_key` over |Δt| with no length test — and the two directories do
    // NOT rotate together, because `saveGame 0x4748` writes `GameSave_11_<n>` and never
    // calls `slotSummary`. So a payload from today could be paired with a summary from
    // days ago, or with a 7-byte torn file, and the restore would install it.
    let slot_name = entry.name.replacen("GameSave", "SlotSave", 1);
    let slot_dir = entry.path.parent().and_then(|p| p.parent()).map(|p| p.join(&slot_name));
    let slot_gen = slot_dir.as_deref().and_then(|d| paired_summary(d, entry.unix_ms));
    let Some(slot_gen) = slot_gen else {
        // ⛔ Refuse rather than half-apply. The player is told why, and the record they
        // still have is untouched — a payload with no summary is exactly the invisible
        // character this feature exists to undo.
        log::error!(
            "vault: refusing to restore {} @ {} — no usable {slot_name} copy within {} ms (a payload without its summary is an invisible character)",
            entry.name,
            entry.unix_ms,
            savevault::PAIR_WINDOW_MS
        );
        return Err(format!("{slot_name} 사본이 없습니다"));
    };

    let mut done = 0usize;
    for (name, src) in [(entry.name.clone(), entry.path.clone()), (slot_name, slot_gen)] {
        let Ok(bytes) = fs::read(&src) else {
            return Err(format!("cannot read {name}"));
        };
        let dst_dir = db_root.join(&name);
        let dst = dst_dir.join("1");
        if let Err(e) = fs::create_dir_all(&dst_dir) {
            return Err(format!("cannot create {name}: {e}"));
        }
        // ★ PIN FIRST, capture second. `before_overwrite_forced` rotates the very
        // directory `src` lives in, so until the pin is on disk that rotate is free to
        // delete the generation this restore is reading from — which is how a player
        // who picked a deep generation could not restore it a second time. Pinning is
        // also what stops the first save afterwards from spending a seat on a byte-
        // identical copy (`savevault::duplicates_pin`).
        if let Some((ms, len)) = src.file_name().and_then(|n| n.to_str()).and_then(savevault::parse_gen)
            && let Some(vdir) = src.parent()
        {
            write_pin(vdir, ms, len);
        }
        // Keep what we are about to overwrite — but NOT as an ordinary generation.
        //
        // ★★★ 72차: `before_overwrite_forced` clears the rate limit on purpose, so every
        // restore ATTEMPT stamped a fresh copy of the CURRENT (broken) record into the
        // very directory the next attempt would read from, and the `rotate` it triggers
        // then pushed one genuinely good generation out of the six unconditional seats.
        // Retrying a restore was self-defeating: after a handful of tries the whole recent
        // tier was copies of the damage. The pre-image is still kept — a restore is
        // destructive and the player may pick the wrong generation — but it lives in a
        // sibling directory that nothing rotates, lists or pairs against.
        keep_pre_restore(&dst, &name);
        if let Err(e) = crate::database::write_atomic(&dst, &bytes) {
            return Err(format!("cannot write {name}: {e}"));
        }
        log::warn!("vault: RESTORED {name} from {} ({} bytes)", entry.unix_ms, bytes.len());
        done += 1;
    }
    let _ = data_dir;
    if done == 2 {
        Ok(done)
    } else {
        Err("nothing restored".into())
    }
}

/// The newest summary generation that can honestly be called this payload's partner.
///
/// ⛔ NOT "nearest in time". See the note in [`restore`]: the two records do not rotate
/// together, so nearest-in-time silently paired across days. A candidate must be exactly
/// [`savevault::SUMMARY_LEN`] bytes and within [`savevault::PAIR_WINDOW_MS`].
fn paired_summary(dir: &Path, want_ms: u64) -> Option<PathBuf> {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("SlotSave").to_string();
    generations(dir)
        .into_iter()
        .filter(|(ms, len, _)| savevault::summary_pairs(want_ms, *ms, *len))
        // ★ 90차: a partner the game cannot read is not a partner.
        .filter(|(_, _, p)| generation_state(&name, p).is_ok())
        .min_by_key(|(ms, _, _)| ms.abs_diff(want_ms))
        .map(|(_, _, p)| p)
}

/// Park the record a restore is about to replace where rotation cannot see it.
///
/// One file per record name, overwritten by the next restore of the same record: the
/// pre-restore state is the same bytes every time a player retries, so keeping a history
/// of it is exactly the waste that was evicting their real history.
fn keep_pre_restore(record_path: &Path, name: &str) {
    let Ok(old) = fs::read(record_path) else {
        return; // nothing there to lose
    };
    if old.is_empty() {
        return;
    }
    let Some((dir, _)) = vault_dir_for(record_path) else {
        return;
    };
    let Some(root) = dir.parent() else { return };
    let out_dir = root.join("pre-restore");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        log::warn!("vault: cannot create {out_dir:?}: {e}");
        return;
    }
    let out = out_dir.join(format!("{name}.bin"));
    match crate::database::write_atomic(&out, &old) {
        Ok(()) => log::warn!("vault: kept the pre-restore {name} ({} bytes) in {out_dir:?}", old.len()),
        Err(e) => log::warn!("vault: cannot keep the pre-restore {name}: {e}"),
    }
}

/// [`before_overwrite`] without the rate limit, for the restore path and for a
/// DELETE — a deletion is the one event that can never be re-taken, so it must never
/// lose its pre-image to the rate limit (2026-08-25).
pub fn before_overwrite_forced(record_path: &Path, new: Option<&[u8]>) {
    if let Ok(mut g) = LAST_MS.lock() {
        if let Some((_, name)) = vault_dir_for(record_path) {
            g.get_or_insert_with(HashMap::new).remove(&name);
        }
    }
    before_overwrite(record_path, new);
}

/// Copy the whole vault somewhere the player can reach it with a PC.
///
/// The app is not debuggable in release, so `run-as` cannot read the internal data
/// dir — but `/sdcard/Android/data/<pkg>/files` IS readable over plain `adb`. When
/// a player reports a lost character this is how their vault gets to us, and it is
/// the only channel that does not need root.
pub fn export(data_dir: &Path, dest: &Path) -> Result<usize, String> {
    let mut n = 0;
    // ⛔ `list_all`, not `list`. The dump is evidence, not an offer — see [`list_all`].
    for e in list_all(data_dir) {
        let out_dir = dest.join(&e.name);
        if let Err(err) = fs::create_dir_all(&out_dir) {
            return Err(format!("{out_dir:?}: {err}"));
        }
        let out = out_dir.join(savevault::gen_name(e.unix_ms, e.len));
        // ⛔ NOT `fs::copy`. It opens the destination `create+truncate` and only THEN
        // streams, so a source that opens and dies mid-read — a bad flash block, the very
        // case this dump exists for — leaves a previously exported, INTACT copy truncated
        // in its place. Read fully first, then replace atomically: a read failure then
        // costs nothing that was already out.
        let bytes = match fs::read(&e.path) {
            Ok(b) => b,
            Err(err) => {
                log::warn!("vault export: cannot read {:?}: {err} — any earlier copy is left as it is", e.path);
                continue;
            }
        };
        // ★ Atomic when it can be: on a full `/sdcard` the temp file fails and the copy
        // already out there stays whole. But the destination is emulated storage, and this
        // is the ONLY channel a player's backup can leave the device on — so a `rename`
        // this filesystem refuses must not mean "exported nothing". The fallback is safe
        // for the hazard we are actually fixing: the bytes are already in hand, so a
        // failing SOURCE can no longer truncate a destination.
        let wrote = crate::database::write_atomic(&out, &bytes).or_else(|err| {
            log::warn!("vault export: atomic write of {out:?} failed ({err}) — falling back to a plain write");
            fs::write(&out, &bytes)
        });
        match wrote {
            Ok(()) => n += 1,
            Err(err) => log::warn!("vault export: {out:?}: {err}"),
        }
    }
    Ok(n)
}

/// Is a slot's live record missing or implausible while the vault holds a good
/// copy? That pair is the unambiguous "your character is gone and we can get it
/// back" state, and it is the only case worth interrupting the player for.
///
/// Returns the entry to offer.
pub fn missing_but_recoverable(data_dir: &Path) -> Option<Entry> {
    let entries = list(data_dir);
    // ★ The SAME roots `list` used, fallback scan included (2026-08-25). This used to
    // read `VAULT_ROOT` directly, which is only set once a record has been WRITTEN this
    // process — and the one caller is `check_at_boot`, which runs before the guest has
    // opened anything. So the boot notice could never fire on the launch that mattered.
    let db_root = vault_roots(data_dir).into_iter().find_map(|r| r.parent().map(|p| p.join("db")))?;
    for e in &entries {
        if !savevault::plausible_game_record(e.len) {
            continue;
        }
        // ★★ The SUMMARY counts too (2026-08-25). A slot reads as 빈 슬롯 when
        // `readSaveRecord("SlotSave_11", slot)` returns 0 — the guest's occupancy scan
        // never looks at `GameSave_11_<slot>` at all (LGT `0x5c06`..`0x5c3e`). So a
        // character can be perfectly intact on disk and still be invisible and
        // unreachable, and this check used to look only at the payload and miss it
        // entirely. `restore` puts the pair back together, which is the way out.
        let summary = db_root.join(e.name.replacen("GameSave", "SlotSave", 1)).join("1");
        let summary_len = fs::metadata(&summary).map(|m| m.len()).unwrap_or(0);
        let live = db_root.join(&e.name).join("1");
        let live_len = fs::metadata(&live).map(|m| m.len() as usize).unwrap_or(0);
        // ★ 72차: `== 0` was too narrow. `slotSummary` always writes exactly
        // `SUMMARY_LEN`, so ANY other size is damage, and the 16..31 band is the nastiest
        // shape there is — the file exists, `seek(END) > 0` passes, and the MD5 compare
        // then fails, so the slot reads empty while every size-based check calls it fine.
        if summary_len as usize != savevault::SUMMARY_LEN && savevault::plausible_game_record(live_len) {
            log::warn!(
                "vault: {} is intact but its slot summary is {summary_len} bytes (expected {}) — the slot will read as 빈 슬롯",
                e.name,
                savevault::SUMMARY_LEN
            );
            return Some(e.clone());
        }
        if !savevault::plausible_game_record(live_len) {
            log::warn!(
                "vault: {} is {live_len} bytes on disk but the vault has a {}-byte copy from {}",
                e.name,
                e.len,
                e.unix_ms
            );
            return Some(e.clone());
        }
    }
    None
}

// ── the player-facing half: offer, request, apply ────────────────────────────
//
// A restore is applied at BOOT, before the guest opens a single database, and
// never while the game is running. That is not caution for its own sake: the
// record the player wants to replace may be open, its contents mirrored in a
// guest-heap buffer that `stream_write` snapshots back to disk on the next write
// (`wie_wipi_c/src/api/database.rs`), so a live swap would be overwritten by the
// mirror seconds later — or, worse, half-overwritten. Booting into it is
// race-free and needs no guest cooperation at all.
//
// So the flow is: the dialog writes a request file, the app closes itself (which
// is now possible — see `crate::request_exit`), and the next launch applies it.

/// Where a pending restore is recorded. Plain text, one line, so a support
/// session can read or clear it with `adb shell` on a debug build.
fn request_path(data_dir: &Path) -> PathBuf {
    data_dir.join("wie_restore_request.txt")
}

/// The offer list for the dialog: one `name|unix_ms|len` line per generation,
/// newest first. Java formats the timestamp — it has the locale and the timezone,
/// and this side has neither.
pub fn offer_lines(data_dir: &Path) -> String {
    list(data_dir)
        .iter()
        .filter(|e| savevault::plausible_game_record(e.len))
        .map(|e| format!("{}|{}|{}", e.name, e.unix_ms, e.len))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Record the player's choice. `key` = `name|unix_ms` (the dialog echoes back the
/// line it offered, extra fields ignored).
///
/// Validated against what is actually on disk before it is written, so a stale
/// dialog cannot schedule a restore of a generation that has since rotated out —
/// which would otherwise be discovered at boot, with the app already closed and
/// nothing on screen to explain it.
pub fn request_restore(data_dir: &Path, key: &str) -> bool {
    let mut parts = key.split('|');
    let (Some(name), Some(ms)) = (parts.next(), parts.next()) else {
        log::warn!("vault: restore request {key:?} is not name|ms");
        return false;
    };
    let Ok(ms) = ms.trim().parse::<u64>() else {
        log::warn!("vault: restore request {key:?} has no timestamp");
        return false;
    };
    if !list(data_dir).iter().any(|e| e.name == name && e.unix_ms == ms) {
        log::warn!("vault: restore request {name} @ {ms} names no generation we hold");
        return false;
    }
    match fs::write(request_path(data_dir), format!("{name}|{ms}\n")) {
        Ok(()) => {
            log::warn!("vault: restore of {name} @ {ms} scheduled for the next launch");
            true
        }
        Err(e) => {
            log::error!("vault: cannot write the restore request: {e}");
            false
        }
    }
}

/// Apply and consume a pending restore. Call ONCE at startup, before the emulator
/// boots. Returns a line for the log/toast when something was restored.
///
/// The request file is deleted FIRST. A restore that crashes must not re-run on
/// every launch — a loop that keeps overwriting the save with the same generation
/// is indistinguishable from the bug this feature exists to undo.
pub fn apply_pending_restore(data_dir: &Path) -> Option<String> {
    let path = request_path(data_dir);
    let spec = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);

    let line = spec.lines().next()?.trim().to_string();
    let mut parts = line.split('|');
    let name = parts.next()?.to_string();
    let ms: u64 = parts.next()?.parse().ok()?;

    let entry = list(data_dir).into_iter().find(|e| e.name == name && e.unix_ms == ms)?;
    match restore(data_dir, &entry) {
        Ok(n) => Some(format!("restored {n} record(s) of {name} from {ms}")),
        Err(e) => {
            // ⛔ The request file is already gone (deleted first, deliberately). Returning
            // `None` meant the player got NOTHING: they scheduled a restore, the app
            // relaunched, and neither the caller's log line nor a toast said it had not
            // happened. Offering an `Unreadable` generation (see `collect`) makes that
            // outcome reachable on purpose, so it has to be audible.
            log::error!("vault: restore of {name} @ {ms} failed: {e}");
            if let Ok(mut g) = BOOT_NOTICE.lock() {
                *g = Some("세이브 복원에 실패했습니다");
            }
            Some(format!("restore of {name} @ {ms} FAILED: {e}"))
        }
    }
}

/// The app's private root (`<data>/<app_id>`), the parent of the `db` the records live
/// in — for host files that are keyed like a record store but must NOT live inside one
/// (the bag boxes: `crate::bagbox_host`). Same resolution as [`db_roots`].
pub fn app_root(data_dir: &Path) -> Option<PathBuf> {
    db_roots(data_dir).into_iter().next()?.parent().map(Path::to_path_buf)
}

/// Where the record store is.
///
/// Resolution order (99차): the ARCHIVE's app id ([`set_app_id`]) when that directory
/// exists — it is the store the emulator will open, by construction; then the remembered
/// root (set by the first record write); then a scan of `data_dir`, which is the fallback
/// for a process that has neither (a fresh install before the first write).
///
/// ⛔ The scan can return MORE THAN ONE root (a device that once ran a KTF build), and the
/// callers that take `.next()` — the boot check and the boot heal — used to act on whichever
/// came first. With the app id in front, the first entry is the live store.
fn db_roots(data_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(id) = APP_ID.get() {
        let db = data_dir.join(id).join("db");
        if db.is_dir() {
            out.push(db);
            return out;
        }
    }
    if let Some(root) = VAULT_ROOT.lock().ok().and_then(|g| g.clone()) {
        // ★ …and only while it still exists. The memo is set once per process and never
        // cleared, so a root that has since gone (a wiped data dir, a test's temp store)
        // would otherwise shadow the scan and answer every question with "nothing here" —
        // which for `slot_has_save` reads as EMPTY, the one answer that costs a character.
        match root.parent().map(|app| app.join("db")) {
            Some(db) if db.is_dir() => out.push(db),
            _ => {}
        }
    }
    if out.is_empty() {
        // Nothing has written a record this session (the guest boots the network UI
        // from the title screen, so this is normal): find the store by scanning.
        if let Ok(rd) = fs::read_dir(data_dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path().join("db");
                if p.is_dir() {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// Is slot `slot`'s summary missing or damaged while its character is intact?
///
/// ★★★ 72차-D (TODO `BB` ⓑ). The decision is [`savevault::summary_repair_needed`]; this
/// only supplies the two file sizes, and it distinguishes "absent" from "unreadable"
/// because that distinction is the difference between repairing and acting blind.
///
/// The record names are matched the way [`slot_has_save`] matches them — on the prefix
/// and the `_<slot>` tail — because the build number in the middle is not ours to
/// predict (KTF and LGT disagree).
pub fn summary_needs_repair(data_dir: &Path, slot: u8) -> bool {
    for db in db_roots(data_dir) {
        let Ok(rd) = fs::read_dir(&db) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            if !name.starts_with("GameSave") || !name.ends_with(&format!("_{slot}")) {
                continue;
            }
            let Some(summary_name) = savevault::summary_partner(&name) else { continue };
            let payload = fs::metadata(e.path().join("1")).map(|m| m.len() as usize).unwrap_or(0);
            let summary = fs::metadata(db.join(&summary_name).join("1"))
                .map(|m| m.len())
                .map_err(|err| err.kind() == std::io::ErrorKind::NotFound);
            // 72차-D's length test: no character-sized payload → not this bug at all.
            if !savevault::plausible_game_record(payload) {
                continue;
            }
            // ── is the ROW damaged? ──────────────────────────────────────────────────
            // Size/absence first (a `stat`, 72차-D)…
            let mut row_damage = savevault::summary_repair_needed(payload, summary);
            let mut row_state = format!("{summary:?}");
            // ★★★ 90차 후속 — SIZE IS NOT THE TEST THE GAME RUNS. A summary that is exactly
            // 32 bytes and fails the game's MD5 reads as healthy by size and is never
            // rebuilt — precisely the shape 90차 can finally SEE (`boot state SlotSave_11_0
            // — Md5Bad(32)`): the row says 빈슬롯 for the rest of the process while the
            // character beside it is intact, and 창고 (the save path that runs most) never
            // writes a summary at all (72차). One `slotSummary` call from the live
            // character regenerates correct bytes in twenty seconds.
            //
            // ⛔ `cipher_proven()` FIRST, and no read at all without it. On a build whose
            // key/table this host does not know, EVERY record classifies `Md5Bad` — a
            // repair firing there would rewrite healthy summaries up to the poll's budget,
            // which is 72차-C's rejected verdict latch.
            // ⛔ The summary (32 B) is read before the payload, so the 1408-byte DES below
            // only runs on a slot whose row is actually broken and the poll stays a `stat`
            // otherwise.
            if !row_damage && cipher_proven() {
                let s = on_disk(&db, &summary_name);
                if s.is_damage() {
                    row_damage = true;
                    row_state = format!("{s:?}");
                }
            }
            if !row_damage {
                continue;
            }
            // ── is the PAYLOAD loadable? ─────────────────────────────────────────────
            // ★★★ 99차: the payload must be `Ok` by the GAME's test, not merely a plausible
            // size, before a row is painted beside it — the same rule the boot heal has kept
            // since 90차-B (`savevault::summary_install_allowed`). A fresh row over an
            // unloadable payload is the `0xaf38` funnel: OK = 새 캐릭터 생성, `#` = 초기화.
            // Without a proven cipher the length band is the only test there is (and is
            // what 72차-D shipped with).
            let payload_loadable = if cipher_proven() {
                let g = on_disk(&db, &name);
                if !g.is_ok() {
                    log::error!(
                        "vault: {summary_name} is {row_state} but {name} is {g:?} — NOT rebuilding the row: the game could not load that payload, and a row over it would open 새 캐릭터 생성 (0xaf38)"
                    );
                }
                g.is_ok()
            } else {
                true
            };
            if savevault::summary_rebuild_allowed(payload_loadable, row_damage) {
                log::warn!(
                    "vault: {name} holds a {payload}-byte character the game can load but {summary_name} is {row_state} — the slot row will read 빈슬롯; rebuilding it from the live character"
                );
                return true;
            }
        }
    }
    false
}

/// The decrypted 16-byte body of real slot `slot`'s summary (`SlotSave_11_<slot>`), or
/// `None` when there is no record, it is not 32 bytes, or the game's own MD5 rejects it —
/// the same verdict `loadSaveRecord` gives, so the row the host paints from this agrees
/// with the row the guest would have painted itself (130차, `slotwindow`).
pub fn summary_body(data_dir: &Path, slot: u8) -> Option<Vec<u8>> {
    let db = db_roots(data_dir).into_iter().next()?;
    let bytes = fs::read(db.join(format!("SlotSave_11_{slot}")).join("1")).ok()?;
    if bytes.len() != savevault::SUMMARY_LEN {
        return None;
    }
    saverecord::plaintext(&bytes, table())
}

/// Does save slot `slot` (a REAL slot, 130차) hold a plausible character record right now?
///
/// ⛔⛔ An unreadable record reads as EMPTY here: `fs::metadata(..).unwrap_or(0)` folds
/// EIO / ENOMEM / EACCES / ENOTDIR into 0, which is not `plausible_game_record` (`read_dir`
/// on the parent proves nothing about the child — different inodes). Its siblings
/// (`write_check`, `delete_permitted`) route the same question through
/// `savevault::target_len` and get `UNREADABLE_LEN`, i.e. OCCUPIED; this one never calls it.
///
/// ★ 135차-B: that is now the RIGHT direction for every caller it has, because there is only
/// one — `savevault::creation_slot` (emulator.rs). There a TRUE "occupied" suppresses the
/// 76차 creation pin and the write falls through to `redirect_slot`, which (identity is a
/// CLASS name — 71차-L) can land the newly created character on the PREVIOUS pin's slot:
/// 76차's «슬롯 3 생성 → 슬롯 1 덮임». The two callers that wanted the opposite (unreadable ⇒
/// OCCUPIED, 90차-B §4) are gone: `backup::guard_slot_info` with the online server (99차), and
/// `offline::Env::slot_occupied` when the engine stopped asking which slot a 복구 is for
/// (135차-B — the refusal is unconditional).
///
/// ⛔ So do not "fix" the `unwrap_or(0)`: it is load-bearing for the creation path. A NEW
/// caller that needs "occupied" on a failed read must not reuse this function — ask
/// `savevault::target_len`, as the write/delete verdicts do (71차-E: 판정을 다른 파일에
/// 흩뜨리지 말 것).
pub fn slot_has_save(data_dir: &Path, slot: u8) -> bool {
    for db in db_roots(data_dir) {
        let Ok(rd) = fs::read_dir(&db) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
            // `GameSave_<build>_<slot>` — match the trailing slot number rather than
            // the build number, which differs between carriers (KTF vs LGT `_11_`).
            if !name.starts_with("GameSave") || !name.ends_with(&format!("_{slot}")) {
                continue;
            }
            let len = fs::metadata(e.path().join("1")).map(|m| m.len() as usize).unwrap_or(0);
            if savevault::plausible_game_record(len) {
                return true;
            }
            log::info!("vault: slot {slot} record {name} is {len} bytes — not a character");
        }
    }
    false
}

// ── the save-slot pin ────────────────────────────────────────────────────────
//
// ★★ WHY THIS EXISTS (2026-08-25, RE of LGT `binary.mod`).
//
// `CHAR+0xa410` is ONE word doing TWO jobs: it is the slot-list CURSOR, and it is
// the `_<slot>` suffix every `writeSaveRecord` appends (`0x1a264`: `getBase()` ->
// `+0xa3e8` -> `sprintf("%s_%d", name, slot)`). Nothing in the save/load code owns
// it — the only writers in the whole image are the title menu (`0xa876`, `0xa88a`,
// both `= 0`), the two slot screens (`0xaef8`/`0xaf08`/`0xaf1a`/`0xaf22` and
// `0xa900`/`0xa918`/`0xa92a`/`0xa932`) and the modal-2 handler (`0xbdec`, `= 0`).
//
// All of those live INSIDE the network shell the warehouse runs in, and none of
// them unloads the character. So from the warehouse the player can re-aim the
// game's save slot just by moving a highlight — and the refused-slot path leaves
// the aim there: `0xa8f0` OK on an empty slot shows 빈슬롯입니다. (`0x5c08c`) and
// returns WITHOUT putting the cursor back.
//
// From that moment every save in the session lands on the wrong slot, and the
// warehouse saves CONSTANTLY: 올리기/내리기/버리기/구입 each call `saveGame()`
// (`0x13a90`, `0x13d36`, `0x13dd8`, `0x13ec8` -> `0x4748`), and every `04 3d`
// backup ack rewrites BOTH records (`0x47e4`), which the client replays on every
// page turn. The slot that gets overwritten is whatever the highlight last touched.
//
// The host cannot patch the guest's cursor without breaking the slot screens, but
// it OWNS the record store — so it can simply refuse the write. The rule is narrow
// on purpose: refuse only when the write would land on a DIFFERENT slot than the
// live character's AND that slot already holds a record. Writing into an empty slot
// is how a new character is born, and it destroys nothing.

/// The save slot the live character belongs to; `-1` = nothing loaded.
static LIVE_SLOT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
/// Identity of the character that latch belongs to (0 = none / unreadable).
static LIVE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Published by the worker while the game is IN PLAY, where the cursor cannot drift
/// (no code outside the shell screens writes it). Entering the network shell freezes
/// the last in-play value, which is exactly the one the guard needs.
pub fn set_live_slot(slot: Option<u8>) {
    let v = slot.map_or(-1, i32::from);
    let old = LIVE_SLOT.swap(v, std::sync::atomic::Ordering::Relaxed);
    if slot.is_none() {
        LIVE_ID.store(0, std::sync::atomic::Ordering::Relaxed);
    }
    if old != v {
        log::warn!("save slot: live character is slot {v} (was {old})");
    }
    // ★ 161차: the slot lists open on the character loaded last — every pin source lands
    //   here, so this is the one place the hint is fed. A hint for the list's cursor only;
    //   the policy above never reads it back.
    if let Some(s) = slot {
        crate::set_slot_focus(s);
    }
}

/// Adopt `slot` as the live pin, unless [`savevault::pin_holds`] says the cursor
/// drifted under the same character — then the existing pin stays.
///
/// The worker used to re-read `CHAR+0xa410` on every in-play poll. That was
/// self-correcting for a missed transition, and it is how a title-screen load of
/// a *different* character gets adopted. It was also how walking the network slot
/// screen and returning to the field re-aimed the pin at the drifted cursor.
pub fn note_live_character(slot: u8, identity: u64) {
    let prev = live_slot();
    let prev_id = LIVE_ID.load(std::sync::atomic::Ordering::Relaxed);
    if savevault::pin_holds(prev, prev_id, slot, identity) {
        log::error!(
            "save slot: cursor is {slot} but the live character is still slot {} — keeping the pin",
            prev.unwrap_or(slot)
        );
        return;
    }
    LIVE_ID.store(identity, std::sync::atomic::Ordering::Relaxed);
    set_live_slot(Some(slot));
}

/// Adopt `slot` as the pin from a SHELL-state (app state 3) observation — the title
/// and slot screens, and **the online warehouse / cash shop, which are state 3 too**.
///
/// ★★★ 71차-E. [`note_live_character`] is gated on `!shell`, so the route
/// 타이틀 → 온라인 → 슬롯 → 창고 armed nothing at all and every warehouse `saveGame`
/// landed on the drifting slot cursor. This is the other half of the latch, with the
/// stricter rule the shell needs: only a character LOAD may move the pin here.
/// See [`savevault::shell_adopts`] for why it is not `pin_holds`.
pub fn note_shell_character(slot: u8, identity: u64) {
    let prev = live_slot();
    let prev_id = LIVE_ID.load(std::sync::atomic::Ordering::Relaxed);
    if !savevault::shell_adopts(prev, prev_id, identity) {
        return;
    }
    log::warn!(
        "save slot: a character was loaded in the shell — pinning slot {slot} (was {:?});          slot is the load, not the live list cursor (71차-J)",
        prev
    );
    LIVE_ID.store(identity, std::sync::atomic::Ordering::Relaxed);
    set_live_slot(Some(slot));
}

/// The guest just LOADED the character that lives in `slot`.
///
/// ★★★ 71차-L — THE ONLY EXACT PIN SOURCE THE ONLINE SCREENS HAVE.
///
/// Called from `network.rs::send` for every outbound `04 3d <slot>` frame. That frame
/// is built by the online slot list's OK arm (`0xa8f0`, key −5), which in the same
/// breath calls `deserialiseSave` (`0x1a964`) on the record it just read — so by the
/// time the bytes reach the socket, CHAR **is** that slot's character. The slot rides
/// in the frame, so nothing here depends on the slot cursor (which the very next
/// 뒤로 sets to 0) or on the identity field (which is a class name).
///
/// Re-arming on every execute is the whole point: 71차-K committed the first one and
/// froze it, so picking a second row left the pin naming a character that was no
/// longer loaded. See the tombstone on `savevault::execute_pin_slot`.
///
/// ★★★ 71차-M — AND THE SECOND CALLER IS THE RECORD STORE. The LOCAL 게임시작 slot list
/// sends no frame at all: it reads `GameSave_11_<cursor>`, hands the buffer to world
/// init and enters app state 4. That load was invisible to 71차-L, the pin stayed on
/// the previously loaded character, and `pin_holds` kept it there because
/// `identity_hash` hashes a CLASS name — so the next save was redirected onto the other
/// slot and destroyed it (device trace, vc180, 2026-08-27). `Database::get` sees both
/// paths; the predicate is `savevault::load_read_slot`.
///
/// The identity comes from the worker's most recent observation rather than being
/// zeroed: `deserialiseSave` does not write `CHAR+0xb8`, so the class name is the same
/// before and after this load, and keeping it lets `savevault::redirect_slot` go on
/// matching. Zeroing it would make every drifted-cursor write unmatchable and stop the
/// warehouse saving (70차 증발).
pub fn note_slot_loaded(slot: u8, how: &str) {
    if u32::from(slot) > crate::savevault::SLOT_MAX {
        return;
    }
    // ★ 100차: every load is an edge for the bag-box migration, INCLUDING a reload of the
    // slot that is already pinned (타이틀 → 게임시작 of the same character brings the bag
    // back from disk exactly as it was saved). Raised before the pin's own early return.
    LOAD_EDGE.store(true, std::sync::atomic::Ordering::Relaxed);
    // ★ 148차: …and for everything ELSE the worker keys to "the world was replaced" — the
    // pin latch's cursor evidence, the plate row's sync, the tower's per-world debts. Same
    // place, same reason: a same-slot reload is a replacement too.
    LOAD_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // ★ 150차 review: …and the hooks' per-character state (the timed max-HP clocks, the
    // tracked true HP, the published set / accessory words) is the LEAVING character's. This
    // is the only place that runs BEFORE the new world's init (the worker's load edge, 4a,
    // is a tick later — after that init's first `ComputeMaxHP` had already used it all).
    let (kill, hit) = wie_util::stats::forget_character();
    if (kill, hit) != (0, 0) {
        log::info!("save slot: the unloaded character's timed buff clocks (kill until {kill} ms, hit until {hit} ms) were cleared");
    }
    let prev = live_slot();
    if prev == Some(slot) {
        return;
    }
    log::warn!("save slot: the character in slot {slot} was loaded ({how}) — pin was {prev:?}");
    LIVE_ID.store(OBS_ID.load(std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Relaxed);
    set_live_slot(Some(slot));
}

// ⛔⛔ `note_execute_pin` / `refresh_pin_identity` / `PENDING_EXECUTE` WERE HERE
// AND ARE GONE (71차-L, 2026-08-26).
//
// They were 71차-K's "pin the slot the player executed into the warehouse, even if
// the name is still unreadable". The guest disassembly says that slot is the BACKUP
// TARGET, not the live character's home — see the tombstone on `execute_pin_slot` in
// `savevault.rs`. Two consequences, both destructive:
//
//   * the pin authorised writes for a character it had never identified, and
//     `write_verdict`'s `p == target` arm turned that into a silent overwrite of the
//     backed-up row (the reported "A 세이브파일이 B 세이브로 변경");
//   * `refresh_pin_identity` then stamped whatever name became readable onto that
//     pin — with no check that the name belonged to the pinned slot. Once stamped,
//     `pin_holds` saw a "proven" pin and froze it there for the rest of the process,
//     so every later save of the OTHER character was redirected onto it.
//
// ⛔ Do not add a pin source that is not an observed character LOAD. There are exactly
// four now — `note_slot_loaded` (the online slot list, via `04 3d`), `note_live_character`,
// `note_shell_character`, and `set_live_slot` itself — and `savevault::write_verdict`
// trusts the pin completely, so a source that merely GUESSES a slot is a licence to
// overwrite that slot with somebody else's character.

/// A character record was loaded since the last `take_load_edge` (100차): the bag boxes
/// migrate the freshly deserialised bag on the first quiet frame after it — see
/// `bagbox_host::BagBoxHost::frame`. Set by [`note_slot_loaded`] on BOTH load paths (the
/// online `04 3d` and the local record read), consumed once per frame by the worker.
static LOAD_EDGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `true` exactly once per load edge.
pub fn take_load_edge() -> bool {
    LOAD_EDGE.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// How many character LOADS [`note_slot_loaded`] has seen (both paths, same-slot reloads
/// included). Monotonic; readers compare it with the value they last saw.
///
/// ★★★ 148차 (2026-10-01) — WHY A COUNTER AND NOT THE SHELL. Player (S25, vc420): *"세이브
/// 로드를 슬롯 변경하면서 빠르게 진행하면 기존에 등록되어있는 퀵슬롯이 날라간다"*. With OK
/// mashed through 시스템 → 메인메뉴 → 게임시작 → SLOT 1, the game went world(slot 2) →
/// `setAppState(3)` → `readSaveRecord` → world(slot 1) in TWO ticks (327 ms + 445 ms,
/// device log 00:25:25.9–26.7), and app state 3 was visible to exactly one worker
/// iteration. Every "the shell was reached" question in the worker is a 1-in-5 (or
/// 1-in-30) SAMPLE of that word, so none of them saw it: the plate row never re-synced
/// and captured slot 1's bar into slot 2's row, and the pin latch took the OUTGOING
/// world's cursor for the incoming character. A load is an EVENT the record store sees
/// synchronously — it cannot be missed, and a single [`take_load_edge`] flag cannot be
/// shared by the several consumers that need it.
static LOAD_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn load_generation() -> u64 {
    LOAD_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// Last character-record `write_check`, unix ms. The slot-list lock waits on it.
static LAST_WRITE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn note_record_write() {
    LAST_WRITE_MS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
}

/// How many character-record writes this process has REFUSED.
///
/// ★ 72차 (F-05): a refusal is otherwise completely silent — `Database::set` returns
/// false, `stream_write` answers -1, and both the guest's writer and the host's save
/// chain discard it. The chain now samples this counter around each write step so it can
/// abort instead of writing a summary over a payload that never landed
/// (`save::Job::last_write_ok`). A monotonic counter rather than a flag: two writes are
/// in flight per save and the reader must be able to say "did anything get refused
/// BETWEEN these two points", which a flag cannot express.
static REFUSALS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn refusal_count() -> u64 {
    REFUSALS.load(std::sync::atomic::Ordering::Relaxed)
}

fn note_refusal() {
    REFUSALS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub fn last_write_ms() -> u64 {
    LAST_WRITE_MS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn live_slot() -> Option<u8> {
    match LIVE_SLOT.load(std::sync::atomic::Ordering::Relaxed) {
        n if n < 0 => None,
        n => u8::try_from(n).ok(),
    }
}

/// `GameSave_11_2` / `SlotSave_11_2` -> `Some(2)`. `None` for anything that is not a
/// character record, including `OptionSave_11`.
fn record_slot(db_name: &str) -> Option<u8> {
    if !savevault::is_vaulted(db_name) {
        return None;
    }
    db_name.rsplit('_').next()?.parse().ok()
}

// ── the worker's cursor observation, for the write REDIRECT (2026-08-25, 70차) ──
//
// The pin above says where the live character LIVES; these three say what the
// worker last SAW: the slot-list cursor, the live identity, and when. Together
// they let `write_check` land a drifted-cursor save on the pinned slot instead of
// refusing it — refusing protected the other slot but silently stopped saving the
// player (창고 올리기/내리기의 saveGame, `04 3d` ack, 필드 저장 전부), which is how
// a withdrawn item could be gone from the warehouse file while the bag that
// received it was never written (캐시템 내리기 증발 제보).

/// Slot-list cursor the worker last observed; `-1` = unreadable / none.
static OBS_CURSOR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
/// Live character identity at that observation (0 = unreadable).
static OBS_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Unix ms of that observation (0 = never).
static OBS_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The online module's SCREEN word at that observation; `-1` = unreadable / no anchor.
/// ★★★ 76차: 10/11/12 are the character-creation screens — `savevault::creation_slot`.
static OBS_SCREEN: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1);

/// Published by the worker on EVERY poll — in the shell too, where the pin latch
/// must not move (67차 rule): the redirect needs to see the drifted cursor next to
/// the unchanged identity, which is precisely the state the latch refuses to adopt.
pub fn note_cursor_observation(cursor: Option<u8>, identity: u64) {
    OBS_CURSOR.store(cursor.map_or(-1, i32::from), std::sync::atomic::Ordering::Relaxed);
    OBS_ID.store(identity, std::sync::atomic::Ordering::Relaxed);
    OBS_MS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
}

/// Published by the worker on every poll next to [`note_cursor_observation`]: the online
/// module's screen word (`items::shell_screen`), `None` where the build has no anchor.
///
/// ★★★ 76차. It is what lets [`write_check`] tell a CREATED character's first save from
/// a drifted-cursor save — the two are byte-for-byte the same observation otherwise
/// («pin A · cursor B · same class · target B»). The word keeps its last shell value
/// (10/11/12 after a creation) for the whole time the game is in the world, so this is
/// not a freshness question: the worker and the guest tick in the same loop.
pub fn note_shell_screen(screen: Option<u32>) {
    OBS_SCREEN.store(screen.map_or(-1, i64::from), std::sync::atomic::Ordering::Relaxed);
}

// ⛔ `suspend_redirect` (70차: drop the observation synchronously when a `04 3e` for an
// empty slot went to a REAL server, whose `05 3e` could replace the character in the same
// tick) WAS HERE and is gone (99차, 100 % offline): the in-app engine never sends a
// character back, so no reply can change CHAR under the redirect.

/// What `Database::set` should do with a write aimed at `record_path`.
pub enum WriteCheck {
    /// Write where the guest aimed.
    Allow,
    /// Write nothing; the guest is told the write failed (it ignores that).
    Refuse,
    /// Write the same bytes at THIS path instead: the record is the pinned
    /// character's own save behind a drifted slot cursor.
    Redirect(PathBuf),
}

/// May the guest replace the record at `record_path` right now — and if the slot
/// cursor drifted under the live character, where should the bytes actually go?
///
/// `Refuse` is returned ONLY for the 67차 case: a character record for a slot that
/// is not the live character's, which already holds something, when the redirect
/// cannot prove the write is the pinned character's own save. The caller reports
/// that to the guest as a failed write, which Hero3 ignores — so the effect is that
/// the wrong slot keeps its character and nothing else changes.
pub fn write_check(record_path: &Path) -> WriteCheck {
    let Some(name) = record_path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) else {
        return WriteCheck::Allow;
    };
    let Some(target) = record_slot(name) else {
        return WriteCheck::Allow;
    };
    // ★ 71차-K: any character-record write (allow / redirect / refuse) is activity
    // the slot-list lock is waiting to see settle.
    note_record_write();
    // ★★★ 71차-E: the DECISION lives in `savevault::write_verdict` — one pure function,
    // reachable from the standalone selftest on any host. This function only gathers the
    // facts and carries the verdict out. ⛔ Do not re-implement any part of the policy
    // here: the 2026-08-26 report happened because the one condition that mattered
    // (`!shell`) sat in a third place and nothing could test it.
    let pin = live_slot();
    // ★★ 72차: an UNREADABLE record must not look empty — see `savevault::target_len`.
    // `unwrap_or(0)` turned EIO/EACCES into "nothing to lose", and both verdicts read 0
    // as permission (the delete one ends in `fs::remove_file`).
    let len = savevault::target_len(fs::metadata(record_path).map(|m| m.len()).map_err(|e| e.kind() == std::io::ErrorKind::NotFound));
    // The redirect's own evidence: what the worker last SAW, next to who is pinned.
    let obs_cursor = match OBS_CURSOR.load(std::sync::atomic::Ordering::Relaxed) {
        n if n < 0 => None,
        n => u8::try_from(n).ok(),
    };
    let obs_id = OBS_ID.load(std::sync::atomic::Ordering::Relaxed);
    let obs_age = now_ms().saturating_sub(OBS_MS.load(std::sync::atomic::Ordering::Relaxed));
    let obs_screen = match OBS_SCREEN.load(std::sync::atomic::Ordering::Relaxed) {
        n if n < 0 => None,
        n => u32::try_from(n).ok(),
    };
    // ⛔⛔ 76차's SYNCHRONOUS CREATION PIN WAS HERE AND IS GONE (78차, 2026-09-03). ⛔⛔
    //
    // It read the shell's screen word out of the worker's observation and, when that word
    // was one of the creation screens and the drifted cursor named this write's target,
    // moved the pin onto the target and let the write through. Two things were wrong with
    // trusting that word HERE:
    //
    //   * 76차 asserted the word is reset by `setAppState(3)`'s init `0x59f0`. It is NOT —
    //     `0x59f0` fills the shell key/paint tables and stores 0 at `base+0xa438`, a
    //     different word (literal `0x5e4c`), while `0x4cf8(1)` is the screen-geometry setup.
    //     Nothing resets `base+0xa3d4`. So a creation value survives until some other shell
    //     screen happens to overwrite it, and a write arriving in that window — a warehouse
    //     `saveGame`, an `04 3d` ack — was pinned onto whatever slot the cursor had drifted
    //     to. `write_verdict`'s `p == target => Allow` then wrote the RESIDENT character
    //     over the character living there. Report, 2026-09-03: «1슬롯으로 접속 → 보스 →
    //     저장 → 메인화면 을 반복했더니 2슬롯이 1슬롯의 세이브파일로 덮어씌워지고, 가끔
    //     2슬롯이 빈슬롯으로 바뀐다» (the second half is the same pin opening
    //     `delete_verdict`).
    //   * it was redundant. A creation reaches the world through `setAppState(4)`
    //     (`0xb1c8`), and the worker sees that shell → world edge ~100 ms later while world
    //     init is still running (594 ms on the vc180 trace) — long before the new character
    //     can save. The worker arm is where the screen word is read now, ONCE, on that edge,
    //     and only onto a slot whose record is empty. See `emulator.rs` and
    //     `savevault::creation_slot`.
    //
    // ⛔ Do not restore a pin source that runs at WRITE time off a sticky guest word. The
    // pin is trusted completely by `write_verdict` and `redirect_slot`; it may only ever be
    // moved by something that observed a character actually being loaded or created.
    let pinned_id = LIVE_ID.load(std::sync::atomic::Ordering::Relaxed);
    let redirect = savevault::redirect_slot(pin, pinned_id, obs_cursor, obs_id, obs_age, target);

    // ★ 71차-L: ONE line that answers the question 71차-K had to guess at — is the live
    // character's NAME readable while the online screens are up? Every verdict below
    // depends on it (a redirect needs `observed_id == pinned_id`), and no device log so
    // far distinguishes "no character is loaded" from "the name could not be read".
    // `id` here is only ever compared, never shown to anyone, so printing it is safe.
    log::info!(
        "save slot: write to {name} ({len} B) — pin {pin:?} id {} · observed cursor {obs_cursor:?} id {} · shell screen {obs_screen:?} · age {obs_age} ms",
        if pinned_id == 0 { "unread" } else { "known" },
        if obs_id == 0 { "unread" } else { "known" },
    );
    match savevault::write_verdict(pin, target, len, redirect) {
        savevault::Verdict::Allow => WriteCheck::Allow,
        savevault::Verdict::Redirect(slot) => {
            // The rename can fail on a name we do not recognise; that falls back to a
            // refusal rather than to the drifted aim.
            if let Some(new_name) = savevault::renamed_slot(name, slot)
                && let (Some(file), Some(db_root)) = (record_path.file_name(), record_path.parent().and_then(|p| p.parent()))
            {
                log::error!(
                    "save slot: REDIRECTED a write aimed at {name} to {new_name} — the slot cursor (CHAR+0xa410)                      drifted in the network shell while the pinned character stayed loaded; the bytes are that                      character's save."
                );
                return WriteCheck::Redirect(db_root.join(new_name).join(file));
            }
            log::error!("save slot: REFUSED a write to {name} — a redirect to slot {slot} could not be named");
            note_refusal();
            WriteCheck::Refuse
        }
        savevault::Verdict::Refuse => {
            log::error!(
                "save slot: REFUSED a write to {name} ({len} B on disk) — the pinned character is slot {pin:?}                  (identity {}). An occupied record is only ever replaced by its own character (71차-E); the                  game's slot cursor (CHAR+0xa410) is not evidence of whose save this is. ★ If this fires                  during 창고 올리기/내리기 the player is not being saved (70차 증발) — the observation                  above says which half is missing.",
                if pinned_id == 0 { "unread" } else { "known" }
            );
            note_refusal();
            WriteCheck::Refuse
        }
    }
}

/// May the guest DELETE the record at `record_path`?
///
/// ★★★ 71차-E — the second hole the 2026-08-26 report exposed. `write_check` guards
/// `Database::set`, and nothing at all guarded `Database::delete`. That is not a
/// theoretical gap: `MC_dbOpenDataBase(mode 4)` arms a deferred truncate and
/// `close_database` runs `db.delete(1)` when no write ever landed — and the database
/// NAME comes from the guest's open call, i.e. from the same drifting slot cursor. So
/// an open-with-no-write on a re-aimed cursor **removed another character's record**.
///
/// The rule is deliberately simpler than the write's: a delete has no "redirect" that
/// makes any sense (deleting the pinned character because the guest aimed elsewhere
/// would be the worst outcome available), so it is allow-or-refuse.
///
/// ⛔ 초기화 is NOT this path. The game's `#` on an occupied slot goes through
/// `Database::delete` on the whole DATABASE (`delete_database`), which stays allowed —
/// the player asked for it, and 2026-08-26: *"세이브는 반드시 게임시작 시 #버튼으로
/// 삭제만 가능"*. Only the record-level delete is gated here.
pub fn delete_permitted(record_path: &Path) -> bool {
    let Some(name) = record_path.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) else {
        return true;
    };
    let Some(target) = record_slot(name) else {
        return true;
    };
    let pin = live_slot();
    // ★★ 72차: an UNREADABLE record must not look empty — see `savevault::target_len`.
    // `unwrap_or(0)` turned EIO/EACCES into "nothing to lose", and both verdicts read 0
    // as permission (the delete one ends in `fs::remove_file`).
    let len = savevault::target_len(fs::metadata(record_path).map(|m| m.len()).map_err(|e| e.kind() == std::io::ErrorKind::NotFound));
    if savevault::delete_verdict(pin, target, len) == savevault::Verdict::Allow {
        return true;
    }
    log::error!(
        "save slot: REFUSED a delete of {name} ({len} B on disk) — the pinned character is slot {pin:?}. The          guest armed a mode-4 truncate against a re-aimed slot cursor; that slot keeps its character."
    );
    false
}

/// Run the boot-time check and LOG the verdict. Called once, before the guest starts,
/// so it is about what is on disk rather than about a database the game happens to
/// have open.
///
/// ⛔ It no longer raises anything on screen (2026-08-27, player request): the 60차
/// "세이브 복원 가능" toast is gone, along with `RECOVERY_NOTICE` /
/// `take_recovery_notice`. Restoring has always been 설정 → 진단 정보 → [세이브 복원],
/// which is available whether or not a record is missing — the toast only ever named a
/// state, and it named it on every launch until the player acted on it.
pub fn check_at_boot(data_dir: &Path) {
    // ★★★ 72차 — SAY WHAT IS ON DISK, ONCE, BEFORE THE GUEST RUNS.
    //
    // The occupancy array the slot list paints from is per-process and SET-ONLY: the scan
    // at guest `0x5c06` stores `1` when `readSaveRecord("SlotSave_11", slot)` returns
    // non-zero and stores nothing at all when it returns zero, and the only writer of a
    // `0` in the whole image is the 초기화 confirm arm at `0xc02c`. So a slot that was
    // occupied at the first `setAppState(3)` cannot go back to 「빈슬롯」 within the same
    // launch — which means every 「빈 공간」 report so far has been undated. This line
    // dates them: it says which slots were already dead before the player pressed a key.
    // ★★★ 90차: the verdict is the GAME's — MD5 over the descrambled body — not a size.
    // "32 bytes — ok" used to be the last word on a summary the game read as 빈슬롯; now
    // that state prints as `Md5Bad(32)`, and `heal_at_boot` acts on it right after.
    for slot in 0..crate::slotwindow::TOTAL {
        for kind in ["GameSave", "SlotSave"] {
            let Some(db) = db_roots(data_dir).into_iter().next() else {
                continue;
            };
            let name = format!("{kind}_11_{slot}");
            log::warn!("vault: boot state {name} — {:?}", on_disk(&db, &name));
        }
    }
    if let Some(e) = missing_but_recoverable(data_dir) {
        log::error!(
            "vault: {} is MISSING or unreadable — a {}-byte copy from {} can be restored (설정 → DIAG → 세이브 복원)",
            e.name,
            e.len,
            e.unix_ms
        );
    }
}

#[cfg(test)]
mod pair_tests {
    //! ★★★ 72차-D — the pair copy on a real temp record store. ONE test fn, for the
    //! same reason `redirect_tests` is one: [`LAST_MS`] is process-global.
    //!
    //! ⛔⛔ THE RECORDS ARE THE GUEST'S OWN (`saverecord_fixtures`), NOT `[1u8; 1408]`
    //! FILLER — and that is not cosmetic. 99차 made the runtime row repair ask whether the
    //! GAME can load the payload (`summary_rebuild_allowed`), which is only asked once some
    //! record in the process has proven the cipher. With filler payloads this test
    //! therefore passed alone and FAILED after `heal_tests` (which proves it) had run:
    //! `summary_needs_repair` correctly refused to paint a row over bytes the game cannot
    //! load, and the 72차-D fixture called that a bug. Real records assert the same thing
    //! in both worlds.

    use super::*;
    use crate::saverecord_fixtures::{GAMESAVE_1408, SLOTSAVE_32};

    #[test]
    fn a_payload_generation_takes_the_live_summary_with_it() {
        let _guard = test_guard();
        let root = std::env::temp_dir().join(format!("wie_vault_pair_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let db = root.join("app").join("db");
        let vault = root.join("app").join("vault");
        // Slots 7 and 8: nothing else in this crate's tests touches those LAST_MS keys.
        let payload = db.join("GameSave_11_7").join("1");
        let summary = db.join("SlotSave_11_7").join("1");
        for p in [&payload, &summary] {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
        }
        fs::write(&payload, GAMESAVE_1408).unwrap();
        fs::write(&summary, SLOTSAVE_32).unwrap();

        // The 창고 / 제련 case: `saveGame 0x4748` replaces the payload and never calls
        // `slotSummary`, so this capture is the one that used to end up unpaired.
        before_overwrite(&payload, Some(&[3u8; 1408]));

        let gens = generations(&vault.join("GameSave_11_7"));
        assert_eq!(gens.len(), 1, "the payload pre-image was kept");
        let ms = gens[0].0;
        let partner = generations(&vault.join("SlotSave_11_7"));
        assert_eq!(partner.len(), 1, "…and the summary that was live alongside it went too");
        assert_eq!(partner[0].0, ms, "under the payload's own timestamp");
        assert!(savevault::summary_pairs(ms, partner[0].0, partner[0].1), "so `restore` can pair them");
        assert_eq!(fs::read(&partner[0].2).unwrap(), SLOTSAVE_32.to_vec(), "the bytes that were on disk");

        // ⛔ ONE WAY. The summary's own write must not drag a payload copy back, and the
        // duplicate summary capture a few ms later is declined by the limit we stamped.
        before_overwrite(&summary, Some(&[4u8; savevault::SUMMARY_LEN]));
        assert_eq!(generations(&vault.join("GameSave_11_7")).len(), 1, "the payload vault is untouched");
        assert_eq!(generations(&vault.join("SlotSave_11_7")).len(), 1, "and the summary was not vaulted twice");

        // ⛔ A summary of the wrong size is damage, not a partner: copying it would take
        // a seat and then be rejected by `summary_pairs` at restore time anyway.
        let payload8 = db.join("GameSave_11_8").join("1");
        let summary8 = db.join("SlotSave_11_8").join("1");
        for p in [&payload8, &summary8] {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
        }
        fs::write(&payload8, GAMESAVE_1408).unwrap();
        fs::write(&summary8, &SLOTSAVE_32[..savevault::SUMMARY_LEN - 1]).unwrap();
        before_overwrite(&payload8, Some(&[3u8; 1408]));
        assert_eq!(generations(&vault.join("GameSave_11_8")).len(), 1, "the payload is still kept");
        assert!(generations(&vault.join("SlotSave_11_8")).is_empty(), "a short summary is not copied");

        // A brand-new character has no summary at all yet; that is not an error.
        let payload9 = db.join("GameSave_11_9").join("1");
        fs::create_dir_all(payload9.parent().unwrap()).unwrap();
        fs::write(&payload9, GAMESAVE_1408).unwrap();
        before_overwrite(&payload9, Some(&[3u8; 1408]));
        assert_eq!(generations(&vault.join("GameSave_11_9")).len(), 1);
        assert!(generations(&vault.join("SlotSave_11_9")).is_empty());

        // ── ⓑ: which of those slots reads as 빈슬롯 while its character is intact ──
        //
        // ★ `VAULT_ROOT` is set by the pair copies above, so `db_roots` resolves to this
        // temp store rather than scanning — which is also the arrangement on a device.
        assert!(!summary_needs_repair(&root, 7), "slot 7 has a 32-byte summary: healthy");
        assert!(summary_needs_repair(&root, 8), "slot 8's summary is short — the row will read 빈슬롯");
        assert!(summary_needs_repair(&root, 9), "slot 9 has no summary at all");
        assert!(!summary_needs_repair(&root, 6), "a slot with no character record is not damage");
        // ⛔ …and slot 7 stays healthy for the RIGHT reason in both worlds: with the cipher
        // proven its 32-byte summary is a real one the game validates, so the 90차-B MD5
        // branch finds nothing either.
        // ⛔ An implausible payload is NOT this bug: there is no character to describe, so
        // baking a summary would advertise a row that cannot be loaded.
        let payload6 = db.join("GameSave_11_6").join("1");
        fs::create_dir_all(payload6.parent().unwrap()).unwrap();
        fs::write(&payload6, [1u8; 8]).unwrap();
        assert!(!summary_needs_repair(&root, 6), "an 8-byte payload has no character to summarise");

        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod redirect_tests {
    //! The write REDIRECT end to end, on a real temp record store. ONE test fn on
    //! purpose: the pin and the observation are process-global, so sequencing inside
    //! a single body is what keeps this deterministic under the parallel test runner
    //! (no other test in the crate touches these statics).

    use super::*;

    #[test]
    fn drifted_writes_land_on_the_pinned_slot_and_nowhere_else() {
        let _guard = test_guard();
        let dir = std::env::temp_dir().join(format!("wie_vault_redirect_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let db = dir.join("app").join("db");
        fs::create_dir_all(db.join("GameSave_11_0")).unwrap();
        fs::create_dir_all(db.join("GameSave_11_1")).unwrap();
        fs::write(db.join("GameSave_11_0").join("1"), b"slot zero's character").unwrap();
        fs::write(db.join("GameSave_11_1").join("1"), b"the pinned character").unwrap();
        let aimed = db.join("GameSave_11_0").join("1");

        // The warehouse case: pin slot 1, cursor drifted to 0, same identity just seen.
        set_live_slot(Some(1));
        note_live_character(1, 0xabcd);
        note_cursor_observation(Some(0), 0xabcd);
        match write_check(&aimed) {
            WriteCheck::Redirect(p) => assert_eq!(p, db.join("GameSave_11_1").join("1")),
            _ => panic!("a drifted write under a matching identity must REDIRECT"),
        }
        // The summary record redirects the same way — both come from writeSaveRecord.
        let slot_aimed = db.join("SlotSave_11_0").join("1");
        fs::create_dir_all(slot_aimed.parent().unwrap()).unwrap();
        fs::write(&slot_aimed, b"summary").unwrap();
        match write_check(&slot_aimed) {
            WriteCheck::Redirect(p) => assert_eq!(p, db.join("SlotSave_11_1").join("1")),
            _ => panic!("SlotSave must redirect with its GameSave"),
        }

        // A different live identity (05 3e restore / title load) falls back to REFUSE —
        // the occupied slot keeps its character and nothing is misdirected.
        note_cursor_observation(Some(0), 0x9999);
        assert!(matches!(write_check(&aimed), WriteCheck::Refuse));

        // (99차: the `suspend_redirect` hook — `04 3e` to a REAL server — is gone with the
        // server; an observation with no identity is the same state, and is still REFUSED.)
        note_cursor_observation(None, 0);
        assert!(matches!(write_check(&aimed), WriteCheck::Refuse));

        // An EMPTY drifted target stays allowed (a new character's first save)…
        note_cursor_observation(Some(2), 0xabcd);
        let empty_aimed = db.join("GameSave_11_2").join("1");
        fs::create_dir_all(empty_aimed.parent().unwrap()).unwrap();
        // …unless the redirect can prove the bytes are the pinned character's, in which
        // case landing them at home beats growing a duplicate in the empty slot.
        match write_check(&empty_aimed) {
            WriteCheck::Redirect(p) => assert_eq!(p, db.join("GameSave_11_1").join("1")),
            _ => panic!("a drifted write into an EMPTY slot redirects too when identity matches"),
        }
        // ⛔ 71차-E CHANGED THIS: with no proof, an empty foreign slot is REFUSED now,
        // not allowed. 70차's "a duplicate is better than a destruction" was true as far
        // as it went, but the player's rule has no exception for it — a save appearing
        // in a slot they never wrote is exactly the class of event being stamped out.
        note_cursor_observation(None, 0);
        assert!(matches!(write_check(&empty_aimed), WriteCheck::Refuse), "an unproven write may not land in ANY foreign slot");

        // Writes aimed at the pin itself never redirect.
        let home = db.join("GameSave_11_1").join("1");
        assert!(matches!(write_check(&home), WriteCheck::Allow));

        // ★★★ 71차-E — THE DELETE GATE. `MC_dbOpenDataBase(mode 4)` + no write makes
        // `close_database` call `db.delete(1)` on the database the guest NAMED, i.e. on
        // the drifted cursor's slot. Nothing guarded that before.
        assert!(!delete_permitted(&aimed), "a delete aimed at another character's record must be refused");
        assert!(delete_permitted(&home), "the pinned slot's own record may still be truncated");
        assert!(delete_permitted(&empty_aimed), "an empty record has nothing to lose");
        assert!(delete_permitted(&db.join("OptionSave_11").join("1")), "non-character records are not gated");

        // ★★★ 76차/78차 — A CHARACTER CREATED IN AN EMPTY SLOT. The pin for that character
        // is placed by the WORKER, on the shell → world edge, through the same exact-pin
        // entry the two load observers use. From this layer's point of view it is simply a
        // pin, and the new character's saves are its own.
        set_live_slot(Some(1));
        note_live_character(1, 0xabcd);
        note_cursor_observation(Some(2), 0xabcd);
        note_slot_loaded(2, "test: created in slot 2");
        assert_eq!(live_slot(), Some(2));
        assert!(matches!(write_check(&empty_aimed), WriteCheck::Allow), "a created character's first save lands in ITS slot");
        assert!(matches!(write_check(&empty_aimed), WriteCheck::Allow), "and every later save too");
        assert!(matches!(write_check(&home), WriteCheck::Refuse), "slot 1's record is now another character's");

        // ⛔⛔ 78차 — THE SCREEN WORD NO LONGER REACHES THIS LAYER. The 2026-09-03 report:
        // a sticky creation screen plus a drifted cursor used to pin the target here and
        // ALLOW the resident character over the record living in it. Same observation now,
        // with slot 1 pinned and the cursor drifted onto OCCUPIED slot 0: the drift rules
        // decide, and slot 0 keeps its character.
        set_live_slot(Some(1));
        note_live_character(1, 0xabcd);
        note_shell_screen(Some(12));
        note_cursor_observation(Some(0), 0xabcd);
        match write_check(&aimed) {
            WriteCheck::Redirect(p) => assert_eq!(p, db.join("GameSave_11_1").join("1"), "the pinned character's own home"),
            _ => panic!("a drifted write under a matching identity must REDIRECT, screen word or not"),
        }
        assert_eq!(live_slot(), Some(1), "no screen word may move the pin at write time");
        // …and with no proof at all it is refused, never allowed onto slot 0.
        note_cursor_observation(None, 0);
        assert!(matches!(write_check(&aimed), WriteCheck::Refuse));
        assert_eq!(live_slot(), Some(1));
        note_shell_screen(None);

        // With NO pin, an occupied record may not be replaced or removed at all.
        set_live_slot(None);
        assert!(matches!(write_check(&home), WriteCheck::Refuse), "no pin may not replace a character");
        assert!(!delete_permitted(&home), "no pin may not delete a character");
        assert!(matches!(write_check(&empty_aimed), WriteCheck::Allow), "a new character's first save still lands");

        let _ = fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod heal_tests {
    //! ★★★ 90차 — the record gate and the boot heal on a real temp store, with records the
    //! GUEST wrote (`saverecord_fixtures`). ONE test fn: `TABLE` / `BOOT_NOTICE` / `LAST_MS`
    //! are process-global. Slot 2 is used so nothing here shares a `LAST_MS` key with the
    //! other test modules.

    use super::*;
    use crate::saverecord_fixtures::{GAMESAVE_1408, SLOTSAVE_32};

    fn put(p: &Path, bytes: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    #[test]
    fn unreadable_records_are_refused_on_write_and_put_back_at_boot() {
        let _guard = test_guard();
        // ★★ 90차 회귀: a STAND-IN must not SEAL. `apply_pending_restore` classifies vaulted
        // generations before `install_table` runs, and a `get_or_init` inside `table()`
        // fixed the rebuilt table for the whole process while `install_table`'s
        // `TABLE.set` lost in silence.
        // ⛔ `TABLE.get().is_some()` does NOT discriminate — `get_or_init` STORES, so it is
        // true in both worlds. Only `is_none()` AFTER a classify tells them apart.
        assert!(TABLE.get().is_none(), "nothing may have sealed TABLE before install_table");
        assert!(classify("GameSave_11_2", &GAMESAVE_1408).is_ok(), "the stand-in reads a real record");
        assert!(TABLE.get().is_none(), "a stand-in must not seal — install_table must still win");
        install_table(None);
        assert!(TABLE.get().is_some(), "install_table fixes the table");
        let root = std::env::temp_dir().join(format!("wie_vault_heal_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let db = root.join("app").join("db");
        let vault = root.join("app").join("vault");

        // ── classify: the game's own records validate, and only those ──
        assert!(classify("GameSave_11_2", &GAMESAVE_1408).is_ok());
        assert!(classify("SlotSave_11_2", &SLOTSAVE_32).is_ok());
        let mut bad = SLOTSAVE_32;
        bad[20] ^= 0x40;
        assert_eq!(classify("SlotSave_11_2", &bad), OnDisk::Md5Bad(32));
        assert_eq!(classify("SlotSave_11_2", &SLOTSAVE_32[..24]), OnDisk::BadSize(24));
        assert_eq!(classify("GameSave_11_2", &GAMESAVE_1408[..1400]), OnDisk::Md5Bad(1400));
        assert_eq!(classify("GameSave_11_2", &[]), OnDisk::Empty);
        assert_eq!(classify("GameSave_11_2", &[0u8; 40]), OnDisk::BadSize(40));

        // ── the write gate: bad bytes are refused and counted, good ones pass ──
        // (`classify` above already validated a real record, so the cipher is proven.)
        assert!(cipher_proven());
        let before = refusal_count();
        assert!(!record_acceptable("SlotSave_11_2", &bad));
        assert!(!record_acceptable("GameSave_11_2", &[0u8; 1408]), "a zeroed buffer is not a save");
        assert!(record_acceptable("SlotSave_11_2", &SLOTSAVE_32));
        assert!(record_acceptable("GameSave_11_2", &GAMESAVE_1408));
        assert!(record_acceptable("OptionSave_11", &[1, 2, 3]), "non-character records are not judged");
        assert_eq!(refusal_count(), before + 2, "each refusal is visible to the save chain");

        // ── heal: a damaged summary next to an intact payload takes a readable copy ──
        put(&db.join("GameSave_11_2").join("1"), &GAMESAVE_1408);
        put(&db.join("SlotSave_11_2").join("1"), &bad);
        // Two vaulted summaries: an older readable one and a newer TORN one (must be skipped).
        put(&vault.join("SlotSave_11_2").join(savevault::gen_name(1_800_000_000_000, 32)), &SLOTSAVE_32);
        put(&vault.join("SlotSave_11_2").join(savevault::gen_name(1_800_000_100_000, 32)), &bad);
        assert_eq!(on_disk(&db, "SlotSave_11_2"), OnDisk::Md5Bad(32));
        let notice = heal_at_boot(&root);
        assert!(notice.is_some(), "the slot was put back: {notice:?}");
        assert_eq!(fs::read(db.join("SlotSave_11_2").join("1")).unwrap(), SLOTSAVE_32.to_vec(), "the READABLE generation, not the newest");
        assert_eq!(fs::read(db.join("GameSave_11_2").join("1")).unwrap(), GAMESAVE_1408.to_vec(), "the intact payload is untouched");
        assert!(root.join("app").join("vault").join("pre-restore").join("SlotSave_11_2.bin").exists(), "the damaged bytes are parked");
        assert!(take_boot_notice().is_some());
        assert!(take_boot_notice().is_none(), "…and consumed once");

        // ── heal: a slot with BOTH directories absent is 초기화 and is left alone ──
        let _ = fs::remove_dir_all(db.join("GameSave_11_2"));
        let _ = fs::remove_dir_all(db.join("SlotSave_11_2"));
        put(&vault.join("GameSave_11_2").join(savevault::gen_name(1_800_000_000_000, 1408)), &GAMESAVE_1408);
        assert!(heal_at_boot(&root).is_none());
        assert!(!db.join("GameSave_11_2").exists(), "a deliberately emptied slot must stay empty");

        // ── heal: a damaged payload takes the newest readable payload + a paired summary ──
        put(&db.join("GameSave_11_2").join("1"), &GAMESAVE_1408[..1400]);
        put(&db.join("SlotSave_11_2").join("1"), &SLOTSAVE_32);
        assert_eq!(on_disk(&db, "GameSave_11_2"), OnDisk::Md5Bad(1400));
        assert!(heal_at_boot(&root).is_some());
        assert_eq!(on_disk(&db, "GameSave_11_2"), OnDisk::Ok(1408));
        assert_eq!(on_disk(&db, "SlotSave_11_2"), OnDisk::Ok(32));

        // ── heal: a payload directory GONE next to a live summary comes back with it ──
        let _ = fs::remove_dir_all(db.join("GameSave_11_2"));
        put(&vault.join("GameSave_11_2").join(savevault::gen_name(1_800_000_000_000, 1408)), &GAMESAVE_1408);
        assert_eq!(on_disk(&db, "GameSave_11_2"), OnDisk::Absent);
        assert!(heal_at_boot(&root).is_some());
        assert_eq!(on_disk(&db, "GameSave_11_2"), OnDisk::Ok(1408), "an absent payload beside a summary is damage, not 초기화");

        // ── heal: damage with nothing readable in the vault is reported, not touched ──
        let _ = fs::remove_dir_all(&vault);
        put(&db.join("SlotSave_11_2").join("1"), &bad);
        assert!(heal_at_boot(&root).is_none());
        assert_eq!(fs::read(db.join("SlotSave_11_2").join("1")).unwrap(), bad.to_vec());

        // ── `Unreadable` never counts as damage: a directory with no record is NoRecord ──
        let _ = fs::remove_file(db.join("SlotSave_11_2").join("1"));
        assert_eq!(on_disk(&db, "SlotSave_11_2"), OnDisk::NoRecord);
        assert!(OnDisk::NoRecord.is_damage() && !OnDisk::Unreadable.is_damage() && !OnDisk::Absent.is_damage());

        // ── `generation_state`: an I/O failure is a THIRD answer, not a rejection ──
        // A directory where a file belongs forces a non-NotFound read error with no
        // privileges at all (Linux EISDIR, Windows PermissionDenied os error 5).
        let gdir = vault.join("GameSave_11_2");
        let gen_path = gdir.join(savevault::gen_name(1_800_000_000_000, 1408));
        put(&gen_path, &GAMESAVE_1408);
        assert_eq!(generation_state("GameSave_11_2", &gen_path), OnDisk::Ok(1408));
        assert_eq!(generation_state("GameSave_11_2", &gdir.join("gone.bin")), OnDisk::NoRecord);
        assert_eq!(generation_state("GameSave_11_2", &gdir), OnDisk::Unreadable);
        assert!(!generation_state("GameSave_11_2", &gdir).is_damage(), "unreadable is never a verdict on the bytes");

        // ── heal (72차 §1-2): an UNREADABLE payload never gets a summary painted over it ──
        // ⛔ The RETURN VALUE does not discriminate — `g2` stays `Unreadable` either way, so
        // the slot is never counted as healed and `heal_at_boot` returns `None` in BOTH
        // worlds. These BYTES are the test.
        let _ = fs::remove_dir_all(db.join("GameSave_11_2"));
        let _ = fs::remove_dir_all(db.join("SlotSave_11_2"));
        let _ = fs::remove_dir_all(&vault);
        fs::create_dir_all(db.join("GameSave_11_2").join("1")).unwrap(); // a directory: unreadable
        put(&db.join("SlotSave_11_2").join("1"), &bad);
        put(&vault.join("SlotSave_11_2").join(savevault::gen_name(1_800_000_000_000, 32)), &SLOTSAVE_32);
        assert_eq!(on_disk(&db, "GameSave_11_2"), OnDisk::Unreadable);
        let _ = heal_at_boot(&root);
        assert_eq!(
            fs::read(db.join("SlotSave_11_2").join("1")).unwrap(),
            bad.to_vec(),
            "a summary installed here would paint a row whose OK arm opens 새 캐릭터 생성 and whose # is 초기화"
        );
        let _ = take_boot_notice();

        // ── 90차 후속: a 32-byte summary that fails MD5 is damage the RUNTIME repair sees ──
        let _ = fs::remove_dir_all(db.join("GameSave_11_2"));
        put(&db.join("GameSave_11_2").join("1"), &GAMESAVE_1408);
        put(&db.join("SlotSave_11_2").join("1"), &bad);
        assert!(cipher_proven());
        assert!(
            summary_needs_repair(&root, 2),
            "Md5Bad(32) is exactly SUMMARY_LEN, so 72차-D's length clause alone calls it healthy"
        );
        put(&db.join("SlotSave_11_2").join("1"), &SLOTSAVE_32);
        assert!(!summary_needs_repair(&root, 2), "…and a summary the game CAN read is never rewritten");

        let _ = fs::remove_dir_all(&root);
    }
}
