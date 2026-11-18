//! Record-store database backed by synchronous `std::fs` under
//! `<app-private data dir>/<app_id>/db/<name>/<record_id>`. Ported from
//! `wie_cli/src/database.rs`, replacing `directories::ProjectDirs` with the
//! Android app-private directory. Hero3/Hero4 saves live mostly here (WIPI DB
//! channel), so how a record reaches the disk is a save-safety question, not a
//! performance one — see [`write_atomic`], and [`crate::vault`] for the copies
//! kept of every character record that is about to be destroyed.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use wie_backend::RecordId;

/// Write `data` to `path` so that a reader NEVER sees a half-written record.
///
/// ── WHY THIS IS NOT `fs::write` (2026-08-20, 60차) ───────────────────────────
/// `fs::write` is `open(O_TRUNC)` + `write`. Between those two calls the file is
/// **zero bytes long**, and this is the file the game's save lives in: Hero3's
/// record writer hands us the whole encrypted record in one `set`, so a process
/// death, a storage error or a killed app inside that window leaves an EMPTY
/// `GameSave_11_<slot>/1` — which the game reads back as "no character"
/// (빈 공간). Two players lost characters that way in the same week, and the
/// window is exactly as wide as one `write` syscall on a 1.4 KB file.
///
/// Temp-file + `rename` closes it: `rename` is atomic within a directory, so the
/// old record stays fully readable until the new one is complete. The two
/// `sync_all` calls are what make that true across a power loss as well — the
/// file's bytes must reach storage BEFORE the directory entry that names them, or
/// a crash can publish a record whose contents were never written.
///
/// A failure here leaves the PREVIOUS record intact, which is the whole point:
/// refusing to save is recoverable, half-saving is not.
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_atomic_with(path, data, true)
}

/// The same temp-file + `rename`, with the two `sync_all` calls made optional.
///
/// ★ 104차 ③ (2026-09-18, user decision): `durable = false` is for a file that is on a
/// LATENCY-CRITICAL path and whose loss is not a loss — the bag boxes' own
/// `bagbox/slot<N>.txt` while a 소비가방 unit is being spent. The atomic publish is
/// unchanged (the reader still sees either the whole old file or the whole new one, and a
/// killed app cannot tear it, because the page cache serves both); what goes is the wait
/// for the flash, which is tens of milliseconds of a worker thread that the player feels
/// as the delay after 사용하기.
///
/// ⛔ Never pass `false` for a save record. A power cut can then lose the newest write,
/// and for `GameSave_11_<slot>` that is a character.
///
/// ★★ WHAT `false` ACTUALLY RISKS (104차-B code review, 2026-09-18) — not only "the
/// newest write is lost". Without the file `sync_all` the rename can reach the journal
/// while the new inode's DATA BLOCKS have not reached the flash, and after a power cut the
/// path then names a zero-length (or short) file: the WHOLE content is gone, not the last
/// change. "Lose both and the unit is still in the box" covered the first outcome only.
/// So a `false` write is a TRANSIENT, and the caller owes two things: a durable re-write
/// of the same bytes as soon as the latency-critical moment is over
/// (`BagBoxHost::frame` does it the frame the use job ends, `sync_slot` before the file is
/// left), and a reader that treats a headerless or empty file as damage rather than as an
/// empty box (`bagbox::Boxes::parse_file`). With both, the exposure is the ~1 s of the use
/// job, and its worst case is a loud fail-closed slot — never a silently emptied box that
/// the next durable flush writes back for good.
pub(crate) fn write_atomic_with(path: &Path, data: &[u8], durable: bool) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    // Same directory (rename cannot cross filesystems) and a fixed name: one
    // writer per record, and a leftover temp from a killed write is simply
    // overwritten by the next one rather than accumulating.
    let tmp = path.with_extension("tmp");

    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        if durable {
            f.sync_all()?;
        }
    }
    fs::rename(&tmp, path)?;

    // Durability of the rename itself. Best-effort: on some Android filesystems
    // opening a directory for sync is refused, and a failure here only means the
    // rename might not survive a power cut — the record itself is already whole,
    // so this must not turn into a failed save.
    if durable && let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

pub struct AndroidDatabaseRepository {
    base_path: PathBuf,
}

impl AndroidDatabaseRepository {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    fn get_path_for_database(&self, name: &str, app_id: &str) -> PathBuf {
        // ★★★ 130차 — THE SAVE-SLOT WINDOW'S ONE DOOR. The guest names its slot records
        //   by the ROW it shows (`GameSave_11_<g>`, g = 0..2, from `sprintf("%s_%d")` in
        //   `readSaveRecord` / `writeSaveRecord` / `deleteSaveRecord`); the record on disk
        //   is the REAL slot behind that row, `first + g`. Every open, exists, delete and
        //   `list_record_info` the guest issues resolves its name here, so this is the
        //   whole map — and with `first == 0` it is the identity, byte for byte the old
        //   behaviour for the three original slots. `Database::get`'s load pin and
        //   `vault::write_check` read the slot back off the RESOLVED path, so they see
        //   real slots without knowing this happened.
        let remapped = crate::slotwindow::remap_name(crate::slot_first(), name);
        let name: &str = remapped.as_deref().unwrap_or(name);
        let sanitized_app_id: String = app_id.chars().filter(|c| !matches!(c, '/' | '\\' | '\0')).collect();
        let app_id = if sanitized_app_id.is_empty() || sanitized_app_id == "." || sanitized_app_id == ".." {
            "_"
        } else {
            &sanitized_app_id
        };

        let name: String = name.chars().map(|c| if matches!(c, '\\' | '\0') { '_' } else { c }).collect();
        let mut normalized_name = PathBuf::new();
        for segment in name.trim_start_matches('/').split('/') {
            match segment {
                "" | "." => {}
                ".." => normalized_name.push("_"),
                segment => normalized_name.push(segment),
            }
        }
        if normalized_name.as_os_str().is_empty() {
            normalized_name.push("_");
        }

        self.base_path.join(app_id).join("db").join(normalized_name)
    }
}

#[async_trait::async_trait]
impl wie_backend::DatabaseRepository for AndroidDatabaseRepository {
    async fn open(&self, name: &str, app_id: &str) -> Box<dyn wie_backend::Database> {
        let path = self.get_path_for_database(name, app_id);
        log::info!("DB open name={name:?} app_id={app_id:?}");
        Box::new(Database::new(path).unwrap())
    }

    async fn exists(&self, name: &str, app_id: &str) -> bool {
        let r = self.get_path_for_database(name, app_id).exists();
        log::info!("DB exists name={name:?} app_id={app_id:?} -> {r}");
        r
    }

    async fn delete(&self, name: &str, app_id: &str) -> bool {
        let path = self.get_path_for_database(name, app_id);
        // ★ 130차: the hooks below key host files by slot — the REAL one, i.e. the name
        //   the path was resolved from, not the row the guest said.
        let remapped = crate::slotwindow::remap_name(crate::slot_first(), name);
        let real_name: &str = remapped.as_deref().unwrap_or(name);
        // ★★ VAULT FIRST (2026-08-25). This is 초기화 — the game's `#` on an occupied
        // slot — and on LGT it is the ONLY guest-reachable path that removes a
        // character (`deleteSaveRecord 0x1a2b8` -> SVC 0x196, whose `wie_lgt` mapping is
        // this whole-database delete, called once for `GameSave_11_<slot>` and once for
        // `SlotSave_11_<slot>`). The 60차 save vault hooks `Database::set` and
        // `Database::delete`, and NEITHER of them is on this path — so the one
        // irreversible action in the game was the one action the vault did not cover,
        // and a player who confirmed that box (or whose confirmation was synthesised)
        // had no way back. Every record in the directory is kept before it goes.
        if let Ok(rd) = fs::read_dir(&path) {
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.is_file() {
                    crate::vault::before_overwrite_forced(&p, None);
                }
            }
        }
        match fs::remove_dir_all(&path) {
            Ok(()) => {
                log::warn!("DB delete_database {name:?} -> {real_name:?} (초기화) — vaulted first");
                // The slot's bag boxes go with the character (they live beside `db`, not
                // in it — see `bagbox_host`), so a new character in this slot starts empty.
                if let Some(app_root) = path.parent().and_then(|db| db.parent()) {
                    crate::bagbox_host::BagBoxHost::forget_slot(app_root, real_name);
                    // ★ 138차: …and its 하늘성 progress (the quick-start floors), which the
                    //   next character in this slot must not inherit either.
                    if let Some(slot) = real_name.strip_prefix("GameSave_11_").and_then(|s| s.parse::<u8>().ok()) {
                        crate::config::forget_tower_progress(app_root, slot);
                    }
                }
                // …and so does its quickslot plate row (2026-09-15): the store keys rows by
                // (slot, class), and a new character of the same class in this slot would
                // otherwise inherit the deleted one's skills. `quickpage::Pages::forget_slot`.
                if let Some(slot) = real_name.strip_prefix("GameSave_11_").and_then(|s| s.parse::<u8>().ok()) {
                    crate::note_slot_reset(slot);
                }
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                log::warn!("Failed to delete database: {e}");
                false
            }
        }
    }
}

pub struct Database {
    base_path: PathBuf,
}

impl Database {
    pub fn new(base_path: PathBuf) -> std::io::Result<Self> {
        fs::create_dir_all(&base_path)?;
        Ok(Self { base_path })
    }

    fn find_empty_record_id(&self) -> RecordId {
        let mut record_id = 1; // XXX midp requires first record to be 1
        loop {
            let path = self.base_path.join(record_id.to_string());
            if !path.exists() {
                return record_id;
            }
            record_id += 1;
        }
    }

    fn get_path_for_record(&self, id: RecordId) -> PathBuf {
        self.base_path.join(id.to_string())
    }
}

#[async_trait::async_trait]
impl wie_backend::Database for Database {
    async fn next_id(&self) -> RecordId {
        let id = self.find_empty_record_id();
        log::info!("DB next_id [{:?}] -> {id}", self.base_path.file_name());
        id
    }

    async fn add(&mut self, data: &[u8]) -> RecordId {
        let id = self.find_empty_record_id();
        let path = self.get_path_for_record(id);
        // ⚠ The trait hands back a RecordId with no way to say "that failed", so a
        // failed add is reported as a successful one to the guest. Log it loudly —
        // when the storage is full this is the only trace that the save the player
        // just watched succeed does not exist.
        match write_atomic(&path, data) {
            Ok(()) => log::info!("DB add [{:?}] id={id} len={}", self.base_path.file_name(), data.len()),
            Err(e) => log::error!("DB add [{:?}] id={id} len={} FAILED: {e}", self.base_path.file_name(), data.len()),
        }
        id
    }

    async fn get(&self, id: RecordId) -> Option<Vec<u8>> {
        let path = self.get_path_for_record(id);
        // ★★★ 72차 (F-01) — AN I/O ERROR IS NOT "NO RECORD".
        //
        // `fs::read(path).ok()` collapsed EIO / EACCES / ENOMEM / EMFILE into `None`, and
        // `None` is indistinguishable from an empty slot all the way down: `open_database`
        // (mode != 4) seeds the handle from this call, so the guest's
        // `seek(h, 0, SEEK_END)` at `0x1a07e` returns 0 and `readSaveRecord` reports 빈슬롯
        // on a byte-perfect file. Worse, the load hook below sits behind the same `Some`,
        // so a transient failure while a character loads makes the load INVISIBLE — the pin
        // stays on the previous character and 71차-M's redirect then lands the next save on
        // its slot. One retry costs nothing and covers the transient cases; a persistent
        // failure is logged at `error!` so a report can name it.
        let r = match fs::read(&path) {
            Ok(v) => Some(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(first) => match fs::read(&path) {
                Ok(v) => {
                    log::error!("DB get [{:?}] id={id} failed once ({first}) and succeeded on retry", self.base_path.file_name());
                    Some(v)
                }
                Err(e) => {
                    log::error!(
                        "DB get [{:?}] id={id} FAILED: {e} — the guest will read this as an empty slot even though the file exists",
                        self.base_path.file_name()
                    );
                    None
                }
            },
        };
        // ★★★ 72차 (F9) — NEVER HAND THE GUEST A RECORD IT CANNOT SUBTRACT 0x10 FROM.
        // `loadSaveRecord 0x1a0e8` does `memcpy(buf, rec + 0x10, total - 0x10)` with no
        // lower bound, so a 1..15-byte character record is a ~4 GB unsigned copy. Hiding it
        // routes the guest down its own clean "empty" branch instead.
        let r = match (&r, self.base_path.file_name().and_then(|n| n.to_str())) {
            (Some(v), Some(name)) if !crate::savevault::record_len_safe(name, v.len()) => {
                log::error!("DB get [{name}] id={id} is {} bytes — refusing to hand the guest a record it would memcpy negatively", v.len());
                None
            }
            _ => r,
        };
        log::info!("DB get [{:?}] id={id} -> {:?} bytes", self.base_path.file_name(), r.as_ref().map(|v| v.len()));
        // ★ 90차: the moment the guest reads a record it will treat as 빈슬롯, say so — one
        // line, with the bytes, next to the `DB get` that fed it.
        if id == 1
            && let Some(data) = &r
            && let Some(name) = self.base_path.file_name().and_then(|n| n.to_str())
        {
            crate::vault::note_record_read(name, data);
        }
        // ★★★ 71차-M — READING A CHARACTER RECORD IS A LOAD, AND THIS IS THE ONLY LAYER
        // THAT SEES BOTH OF THEM. The online slot list announces itself with `04 3d`
        // (71차-L); the LOCAL 게임시작 slot list sends nothing at all — it reads
        // `GameSave_11_<cursor>`, hands the buffer to world init and enters app state 4.
        // That second load was invisible, so the pin stayed on the previous character
        // and the next save was REDIRECTED onto its slot, destroying it (device trace,
        // vc180, 2026-08-27: `DB get [GameSave_11_1] -> 1840` at 00:05:49.051, then
        // `REDIRECTED a write aimed at GameSave_11_1 to GameSave_11_0` at 00:05:51.119).
        //
        // ⛔ A write never reaches here: `MC_dbOpenDataBase(…, MC_DB_CREATE)` skips the
        // record-1 seed, and every `MC_DB_CREATE` in that trace has no `DB get` before
        // it. The predicate (`GameSave` only, record 1, plausible length) is
        // `savevault::load_read_slot` — pure, and tested against that trace.
        if let Some(data) = &r
            && let Some(name) = self.base_path.file_name().and_then(|n| n.to_str())
            && let Some(slot) = crate::savevault::load_read_slot(name, id, data.len())
        {
            crate::vault::note_slot_loaded(slot, "the guest read that slot's record");
        }
        r
    }

    async fn set(&mut self, id: RecordId, data: &[u8]) -> bool {
        let path = self.get_path_for_record(id);
        // ★★★ 90차 — THE BYTES THEMSELVES, before any question of WHICH slot. A character
        // record the game could not read back (its own MD5 test, `saverecord.rs`) is
        // refused here, loudly, and the readable record on disk stays. This is the gate
        // 67차..78차 could not have: every one of them argued about the slot number while
        // the bytes went to disk unexamined.
        if id == 1
            && let Some(name) = self.base_path.file_name().and_then(|n| n.to_str())
            && !crate::vault::record_acceptable(name, data)
        {
            return false;
        }
        // ★★ The save-slot pin (2026-08-25). The guest's `_<slot>` suffix is the slot
        // LIST CURSOR, which the network shell lets the player re-aim while a character
        // is loaded — see `vault::write_check`. Refusing protects the other slot; the
        // REDIRECT (70차) additionally lands a drifted write on the pinned slot when
        // the identity proves the bytes are that character's own save — that is what
        // makes the warehouse's own saveGame calls actually save again.
        let path = match crate::vault::write_check(&path) {
            crate::vault::WriteCheck::Refuse => return false,
            crate::vault::WriteCheck::Allow => path,
            crate::vault::WriteCheck::Redirect(p) => {
                if let Some(dir) = p.parent()
                    && let Err(e) = fs::create_dir_all(dir)
                {
                    // Refusing is the 67차 behaviour, and strictly safer than writing
                    // where the drifted cursor aimed.
                    log::error!("DB set: cannot create the redirect target {dir:?}: {e} — refusing the write instead");
                    return false;
                }
                p
            }
        };
        // Keep the version that is about to stop existing (character records only).
        crate::vault::before_overwrite(&path, Some(data));
        match write_atomic(&path, data) {
            Ok(()) => {
                log::info!("DB set [{:?}] id={id} len={}", self.base_path.file_name(), data.len());
                // ★ The bag payload (record 1 of GameSave) is now durable. The 재료가방 loan
                //   (bagbox::Loan) waits on this to promote the box file. Use the RESOLVED
                //   path's slot — write_check may have redirected it off the cursor.
                if id == 1
                    && let Some(slot) = path
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .and_then(|n| n.strip_prefix("GameSave_11_"))
                        .and_then(|s| s.parse::<u8>().ok())
                {
                    crate::note_gamesave_committed(slot);
                }
                true
            }
            Err(e) => {
                log::error!("DB set [{:?}] id={id} len={} FAILED: {e}", self.base_path.file_name(), data.len());
                false
            }
        }
    }

    async fn delete(&mut self, id: RecordId) -> bool {
        let path = self.get_path_for_record(id);
        // ★ The widest data-loss window in the whole stack: `MC_dbOpenDataBase`
        // with mode 4 (CREATE) deletes record 1 UP FRONT
        // (`wie_wipi_c/src/api/database.rs`), so from here until the guest's first
        // `stream_write` lands the character does not exist on disk. Anything that
        // goes wrong in between — a killed app, a storage error, a guest call the
        // host abandoned — makes that permanent. Vault the bytes on the way out.
        // ★★★ 71차-E: and it may not run on ANOTHER character's slot. The database name
        // comes from the guest's `MC_dbOpenDataBase` call — the drifting slot cursor — so
        // an open(mode 4) that never wrote used to remove whatever character that slot
        // held. `write_check` never saw it: it only guards `set`.
        if !crate::vault::delete_permitted(&path) {
            return false;
        }
        crate::vault::before_overwrite(&path, None);
        let ok = fs::remove_file(path).is_ok();
        log::info!("DB delete [{:?}] id={id} ok={ok}", self.base_path.file_name());
        ok
    }

    async fn get_record_ids(&self) -> Vec<RecordId> {
        let ids: Vec<RecordId> = match fs::read_dir(&self.base_path) {
            Ok(rd) => rd
                .filter_map(|x| x.ok())
                .filter(|x| x.path().is_file())
                .filter_map(|x| x.file_name().to_str().and_then(|s| s.parse().ok()))
                .collect(),
            Err(_) => Vec::new(),
        };
        log::info!("DB get_record_ids [{:?}] -> {ids:?}", self.base_path.file_name());
        ids
    }
}
