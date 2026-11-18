//! The WIPI net API, served ENTIRELY in-app.
//!
//! ★★★ 99차 (2026-09-16) — 100 % OFFLINE. There is no socket in this module any more: no
//! TCP, no TLS, no DNS, no server endpoint, no account id. The game's online screens
//! (네트워크 창고 · 캐시샵 · the title's 온라인 slot list) still exist and still speak the
//! original wire protocol through `MC_netSocket/Connect/Send/Recv/Close`, but every
//! connect now lands on the in-app engine (`offline.rs`) and every reply comes from it.
//! The user's decision: *"모든 게임은 100% 오프라인으로만 동작하게 할꺼야"*; the private
//! server (Track C, `hero3_server/`, `TRACK_C_PRIVATE_SERVER.md`) is no longer something
//! this client can reach, and `NET` / `서버 주소` / `ACCOUNT` left the settings panel with it.
//!
//! The guest contract this must satisfy is unchanged (established by device tracing +
//! disassembly of `client.bin`, `TRACK_C_PRIVATE_SERVER.md` §3):
//!
//! ```text
//!   slot 2   socket(af=2, type=1) -> fd
//!   slot 30  connect(fd, addr, port, name, cb, param)  -> 0/-19, result via cb
//!   slot 31  send(fd, buf, len)  -> bytes accepted, negative = teardown
//!   slot 32  recv(fd, buf, max)  -> n>0 | -19 no data | 0/other negative = peer gone
//!   slot 33  close(fd)
//! ```
//!
//! Two properties still shape it:
//!
//! 1. **Nothing may block.** Everything here is an in-memory exchange on the emulator
//!    thread; the only I/O is the warehouse file, written on a real change.
//! 2. **`recv` must distinguish "no data yet" from "peer gone".** The guest treats -19
//!    as healthy-but-idle and *anything else* non-positive as a dead connection. The
//!    in-app server never dies, so -19 is the only non-data answer it ever gives.
//!
//! ⛔ What is deliberately NOT here any more, so nobody re-adds it by reflex: the resume
//! probe (`check_resume` — Android resetting sockets in the background), the 30 s stuck-
//! request watchdog, the connect notices, the account-id injection into `01 3d`, the
//! `04 3e` restore guard (`backup::guard_slot_info` — the engine refuses 복구 itself) and
//! the `suspend_redirect` hook. All of them existed only because a REAL peer could
//! misbehave; an engine in the same process cannot.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
    time::{Duration, Instant},
};

/// `M_E_WOULDBLOCK` — see the module docs. Must match
/// `wie_wipi_c::api::net::M_E_WOULDBLOCK`.
pub const WOULDBLOCK: i32 = -19;
/// Generic failure the guest treats as fatal for the socket.
pub const ERROR: i32 = -1;

/// How long a "nothing on the socket" answer may be reused before the host looks
/// again. This is the fix for the warehouse frame drop (`MIGRATION_LGT.md` §2.24), and
/// it applies to the in-app server just as it did to a real socket (device-caught
/// 2026-08-20: "네트워크 창고 프레임이 갑자기 좀 다운된거같아").
///
/// The guest's server-wait loop polls `recv` about 25,000 times a second. At this window
/// the host walks the socket map at most 500 times a second — 98 % fewer — and the worst
/// case a reply can sit uncollected is 2 ms. A reply that already exists is NEVER
/// withheld: the engine produces it synchronously inside `send`, so by the time the
/// guest polls the outbox is already non-empty, and a non-empty outbox clears the gate.
const RECV_EMPTY_REUSE: Duration = Duration::from_millis(2);

/// Instrumentation for the throttle above. `POLLS` counts what the guest asked for and
/// `ABSORBED` what the gate answered without walking the map. `absorbed/polls` near 1.0
/// is the healthy shape; `ABSORBED` at 0 while `POLLS` is in the tens of thousands per
/// second is the pre-fix behaviour §2.24 measured.
static RECV_POLLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RECV_ABSORBED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A warehouse / cash-shop session is live: the guest has an in-app socket. Used to
/// lock the slot list on the way back (71차-K) and by the box windows (96차).
/// `socket()` alone (Fresh) is not a session.
static WAREHOUSE_OPEN: AtomicBool = AtomicBool::new(false);

fn publish_session(sockets: &HashMap<i32, State>) {
    WAREHOUSE_OPEN.store(sockets.values().any(|s| matches!(s, State::Local(_))), Ordering::Relaxed);
}

/// Is the guest inside a warehouse / cash-shop session?
pub fn warehouse_open() -> bool {
    WAREHOUSE_OPEN.load(Ordering::Relaxed)
}

/// Is the live warehouse session one whose store the host can read and move items in
/// (the box windows, 96차)? Since 99차 every session is the in-app engine, so this is
/// [`warehouse_open`] — kept as a name because the callers ask a different question
/// ("may I open the 창고 box here?") than the slot-list lock does.
pub fn offline_session_live() -> bool {
    warehouse_open()
}

/// `(polls, absorbed)` since the last call, resetting the counters so the caller reports
/// an interval rate. Read by the 30 s gauge in `emulator.rs`.
pub fn recv_stats_take() -> (u64, u64) {
    (RECV_POLLS.swap(0, Ordering::Relaxed), RECV_ABSORBED.swap(0, Ordering::Relaxed))
}

/// Bytes withheld from every `recv` so the guest cannot overrun its own receive
/// object. See [`AndroidNetwork::recv`] for the derivation — it is exactly the
/// 8-byte gap between what the guest asks for and what its 1028-byte buffer
/// holds. The largest frame this permits is 1016 bytes, which is also the cap the
/// engine's frames respect (`offline.rs`).
const GUEST_RECV_SLACK: usize = 8;

/// Result of polling a connect. The in-app server is connected from the instant it
/// exists — there is no handshake to wait for — so there is no `Pending` any more.
pub enum ConnectPoll {
    Connected,
    Failed,
}

enum State {
    /// Created by `socket()` but not yet connected.
    Fresh,
    /// The IN-APP server (`offline.rs`). No socket exists at all: `send` feeds the
    /// engine and `recv` drains what it produced.
    ///
    /// ── WHY A SOCKET STATE AND NOT A SEPARATE PATH ─────────────────────────
    /// The guest's warehouse UI is driven entirely by REPLY FRAMES — every screen it
    /// opens is released by one, and there is no timeout behind any of them. Serving
    /// those frames through the same `socket/connect/send/recv/close` the original
    /// server used means the game cannot tell the difference.
    Local(Box<LocalServer>),
}

/// The in-app server bound to one guest socket.
pub struct LocalServer {
    engine: crate::offline::Engine,
    /// Replies produced but not yet collected by the guest's `recv`.
    outbox: Vec<u8>,
    data_dir: PathBuf,
}

/// How long after the offline login notice goes out before the host dismisses it
/// for the player.
///
/// Long enough that the guest has certainly opened the box (it does so on the tick
/// that consumes the frame), short enough not to read as a pause.
const NOTICE_DISMISS_AFTER: Duration = Duration::from_millis(400);

/// When the in-app server last sent the login notice, if it is still unanswered.
///
/// ── WHY THE HOST DISMISSES IT (player, 2026-08-20) ──────────────────────────
/// "오프라인으로 진행하니까 로딩같은거 필요없지않아? 공지도 필요없고". They are right:
/// the NOTICE box is a server announcement board, and an in-app server has nothing
/// to announce — but the frame that carries it is also the ONLY thing that finalises
/// the login and clears the client's WAIT (PROTOCOL.md §3), and an EMPTY notice
/// crashes the client's paged text builder. So the notice still has to be sent, and
/// the host presses OK on it instead of the player.
///
/// Cleared by any frame arriving from the guest, which is what a player who
/// dismissed it first would produce — so the synthetic OK can never land on the
/// screen after it.
static OFFLINE_NOTICE: Mutex<Option<Instant>> = Mutex::new(None);

/// `true` exactly once, when the offline login notice has been on screen long enough
/// to be dismissed. The caller synthesises the OK.
pub fn offline_notice_due() -> bool {
    let mut g = OFFLINE_NOTICE.lock().unwrap();
    match *g {
        Some(at) if at.elapsed() >= NOTICE_DISMISS_AFTER => {
            *g = None;
            true
        }
        _ => false,
    }
}

/// The quantity the in-app warehouse is holding a reply for, and since when.
///
/// 올리기/내리기 carry no count on the wire — 올리기 sends the whole bag stack and clears
/// the cell, 내리기 is just a key — so the number can only come from the player, and the
/// only moment to ask is while the client is already blocked on its own WAIT. See
/// `offline::QtyAsk`.
static WH_QTY: Mutex<Option<(crate::offline::QtyAsk, Instant)>> = Mutex::new(None);
/// The player's answer, consumed once by [`LocalServer::pump`].
static WH_QTY_ANSWER: Mutex<Option<QtyReply>> = Mutex::new(None);

/// What the player did with the quantity picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QtyReply {
    /// OK — move this many.
    Take(u8),
    /// CLR — move nothing (player request 2026-08-26).
    Cancel,
}
/// How long a held request waits before it takes the whole stack by itself.
///
/// Generous — the player is reading a dialog, and there is no cost to waiting — but
/// bounded, because the alternative to answering is a game that never comes back.
const WH_QTY_TIMEOUT: Duration = Duration::from_secs(60);

/// A count the HOST is asking for — a box window's 올리기 / 내리기 (96차), which moves
/// units between the bag box file and the offline store without any guest frame.
///
/// Kept apart from [`WH_QTY`] on purpose: `LocalServer::publish_qty_ask` owns that slot
/// and clears it whenever the ENGINE has nothing pending (which is on every heartbeat).
/// The two never coexist — the list dialog swallows the input a grid 올리기 would need —
/// so [`wh_qty_pending`] simply shows whichever one is up.
static HOST_QTY: Mutex<Option<crate::offline::QtyAsk>> = Mutex::new(None);

/// What the in-app warehouse — or a host box move — is asking the player for, if
/// anything. Polled by the emulator worker, which publishes it to the UI.
pub fn wh_qty_pending() -> Option<crate::offline::QtyAsk> {
    WH_QTY.lock().unwrap().map(|(ask, _)| ask).or(*HOST_QTY.lock().unwrap())
}

/// Ask the player for a count on the host's own behalf (a box window's 올리기 / 내리기).
/// Any answer left over from an earlier question is dropped first, for the same reason
/// `publish_qty_ask` drops it: it was chosen for a different item.
pub fn host_qty_ask(ask: crate::offline::QtyAsk) {
    *WH_QTY_ANSWER.lock().unwrap() = None;
    *HOST_QTY.lock().unwrap() = Some(ask);
}

/// Withdraw the host's question (the window closed under it). The picker follows on the
/// worker's next publish.
pub fn host_qty_withdraw() {
    if HOST_QTY.lock().unwrap().take().is_some() {
        *WH_QTY_ANSWER.lock().unwrap() = None;
    }
}

/// The player's answer to the HOST's question, consumed once. `None` while they are still
/// deciding, or when no host question is up (the engine's own answers never come through
/// here — `pump` takes those, and only while the engine is the one asking).
pub fn host_qty_reply() -> Option<QtyReply> {
    let mut host = HOST_QTY.lock().unwrap();
    host.as_ref()?;
    let reply = WH_QTY_ANSWER.lock().unwrap().take()?;
    *host = None;
    Some(reply)
}

/// The player picked `n`. Applied on the next `recv`, which a blocked client does
/// constantly.
pub fn wh_qty_answer(n: u8) {
    *WH_QTY_ANSWER.lock().unwrap() = Some(QtyReply::Take(n));
}

/// The player pressed CLR: the held 올리기/내리기 is abandoned, nothing moves.
pub fn wh_qty_cancel() {
    *WH_QTY_ANSWER.lock().unwrap() = Some(QtyReply::Cancel);
}

/// The player's bag as the WORKER last saw it, for the offline engine's capacity checks.
///
/// Only the emulator thread may read guest memory, and `LocalServer` has no handle to the
/// emulator — so the worker publishes a snapshot here and `live_env` picks it up. Stale by
/// at most a few frames, which is fine: it only ever bounds a number the player is about
/// to be ASKED for, and every path re-checks before it moves anything.
static BAG_VIEW: Mutex<Option<crate::offline::BagView>> = Mutex::new(None);

/// Publish the bag snapshot. Worker only.
pub fn set_bag_view(view: crate::offline::BagView) {
    *BAG_VIEW.lock().unwrap() = Some(view);
}

/// The offline warehouse itself, shared by every local socket in the process.
///
/// ★ The guest opens TWO sockets back to back (device trace: fd 3 then fd 4, the
/// second closed immediately). Giving each its own copy of the warehouse would mean
/// two divergent views of the same file, and the last writer would silently discard
/// the other's deposit. Session state — the login handshake, the deposit echo, the
/// partial-frame buffer — stays PER SOCKET, because those really are per connection;
/// only the stored items are shared.
static OFFLINE_STORE: Mutex<Option<crate::offline::Store>> = Mutex::new(None);

/// ★ 136차 (2026-09-25) — frames the HOST wants the client to receive without having asked:
/// the `05 3d` page refresh after a 정렬 of the warehouse. Drained into the outbox of the
/// socket the warehouse conversation runs on ([`WAREHOUSE_FD`]) by `poll_readable`, which
/// the guest calls every tick while the online screen is up — the same road a reply held
/// for a quantity takes home (`LocalServer::pump`).
static PENDING_PUSH: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());
/// The fd that last SENT a `05 xx` warehouse frame — the guest opens two sockets in a
/// session and only one of them talks warehouse. `-1` = none yet.
static WAREHOUSE_FD: AtomicI32 = AtomicI32::new(-1);

/// ★ 136차 — 정렬 the shared warehouse by `key(cat, idx, count, position)` (`bagsort::wire_key`),
/// persist it when anything moved, and queue a page-0 refresh for the client.
///
/// Returns `Some(moved)`; `None` when there is no warehouse session (nothing to refresh —
/// the store would be sorted into a file nobody is looking at, which the button never
/// asks for since it is only drawn on the 창고 panel). The store is the one every socket
/// shares (`OFFLINE_STORE`), borrowed here exactly as `LocalServer::feed` borrows it — and
/// on the same thread, since the guest only runs inside the worker's `tick`.
pub fn sort_warehouse<K: Ord>(data_dir: &std::path::Path, key: impl Fn(u8, u8, u8, u16) -> K) -> Option<bool> {
    if !warehouse_open() {
        return None;
    }
    let mut shared = OFFLINE_STORE.lock().unwrap();
    let store = shared.get_or_insert_with(|| crate::config::read_offline_store(data_dir));
    let moved = store.sort_rows_by(key);
    if moved && !crate::config::write_offline_store(data_dir, store) {
        log::warn!("offline: the sorted warehouse could not be written — the order lives until the next write");
    }
    // Page 0 whether or not anything moved: the tap is the player asking to see the list
    // in order, and the header byte of a `05 3d` also puts their page cursor back to 1.
    PENDING_PUSH.lock().unwrap().push(crate::offline::Engine::page_for(store, 0));
    Some(moved)
}

/// Read or change the OFFLINE warehouse in place — the box windows' 창고 view and the host
/// 올리기 / 내리기 (96차). `f` returns `(result, changed)`; a change is written back at
/// once, and the second value says whether that write LANDED, so a caller that debits
/// something else afterwards can refuse to when it did not.
///
/// Same shared store every local socket uses (see [`OFFLINE_STORE`]), so a warehouse
/// session that is open right now cannot write this back out again from a stale copy.
pub fn with_offline_store<R>(data_dir: &std::path::Path, f: impl FnOnce(&mut crate::offline::Store) -> (R, bool)) -> (R, bool) {
    let mut shared = OFFLINE_STORE.lock().unwrap();
    let store = shared.get_or_insert_with(|| crate::config::read_offline_store(data_dir));
    let (r, changed) = f(store);
    let persisted = !changed || crate::config::write_offline_store(data_dir, store);
    (r, persisted)
}

impl LocalServer {
    fn new(data_dir: PathBuf) -> Self {
        {
            let mut shared = OFFLINE_STORE.lock().unwrap();
            let store = shared.get_or_insert_with(|| crate::config::read_offline_store(&data_dir));
            log::warn!(
                "net: OFFLINE warehouse — {} item(s), {} bag(s), {} backup(s)",
                store.items.len(),
                store.bags,
                store.backups.len()
            );
        }
        Self {
            engine: crate::offline::Engine::default(),
            outbox: Vec::new(),
            data_dir,
        }
    }

    /// What the engine is told about the world outside it — read LIVE on every call.
    ///
    /// ★ The count is the LATCHED one, not the live spinner. The engine reads it when the
    /// purchase frame arrives, which is after the confirm box has been answered — and the
    /// live value can already have been reset by then. See `crate::cash_qty_committed`.
    /// The bag snapshot is live for the same reason: it bounds a number the player is
    /// about to be asked for, and the player changes it by picking an item up.
    ///
    /// ⛔ 135차-B removed the one fact that needed a cache here: `Env::slot_occupied`, a
    /// record-store scan re-read every two seconds. `04 3e` is answered with the refusal
    /// unconditionally now (`offline::Engine::recovery`), so nothing asks which save slots
    /// hold a character — and both fields left are one atomic read each.
    fn live_env() -> crate::offline::Env {
        crate::offline::Env {
            cash_qty: crate::cash_qty_committed(),
            bag: BAG_VIEW.lock().unwrap().clone().unwrap_or_default(),
            orb_box: Self::orb_box_view(),
        }
    }

    /// ★★ 166차 — the 오브가방 as the bag-box intake hook sees it right now: one read of the
    /// knobs the host publishes every frame (`wie_util::stats::bagbox_view`), sized for the
    /// largest purchase the engine delivers — one `05 04` per orb kind.
    fn orb_box_view() -> Option<crate::offline::OrbBox> {
        use crate::offline::{ORB_IDX_SOCKET_EXPAND, ORB_ITEM_CATEGORY};
        let kinds = u32::from(ORB_IDX_SOCKET_EXPAND) + 1;
        let (free_stacks, merge) = wie_util::stats::bagbox_view(u32::from(ORB_ITEM_CATEGORY), kinds, kinds as usize)?;
        Some(crate::offline::OrbBox { free_stacks, merge })
    }

    /// Hand the engine the guest's bytes and keep whatever it answers.
    ///
    /// The store is written back on the spot when it changed. That is a file write on
    /// the emulator thread, but only on a real change (a deposit, a purchase, a
    /// backup) — browsing and page turns are pure reads, and those are what the guest
    /// does thousands of times.
    fn feed(&mut self, data: &[u8]) {
        let env = Self::live_env();
        // The two host-delivered purchases (오브상자 / 소켓확장) are the only place this
        // number is read, and it travelled a different road from the client-granted
        // ones — so say it out loud on the frame that uses it.
        if data.len() >= 6 && data[4] == 0x05 && data[5] == 0x42 {
            log::info!("offline: cash buy idx {:#04x} with qty {}", data.get(6).copied().unwrap_or(0), env.cash_qty);
        }
        // Borrow the shared warehouse for the duration of this call, then give it
        // back. The engine owns a `Store` so it can be tested standalone; the swap is
        // what makes every socket see the same one.
        let mut shared = OFFLINE_STORE.lock().unwrap();
        let store = shared.get_or_insert_with(|| crate::config::read_offline_store(&self.data_dir));
        core::mem::swap(&mut self.engine.store, store);
        let out = self.engine.feed(data, &env);
        core::mem::swap(&mut self.engine.store, store);

        // Arm / disarm the notice auto-dismiss. A REAL inbound frame means the guest
        // is talking again, so a notice it was showing has already been answered —
        // but the heartbeat does not count: it fires every five seconds regardless of
        // what is on screen, and letting it disarm the dismiss would make the feature
        // work or not depending on where in that cycle the notice happened to land.
        {
            let mut notice = OFFLINE_NOTICE.lock().unwrap();
            if !only_heartbeats(data) {
                *notice = None;
            }
            let mut i = 0usize;
            while i + 6 <= out.len() {
                let total = u32::from_le_bytes([out[i], out[i + 1], out[i + 2], out[i + 3]]) as usize;
                if total < 6 || i + total > out.len() {
                    break;
                }
                if out[i + 4] == 0x01 && out[i + 5] == 0x3e {
                    *notice = Some(Instant::now());
                }
                i += total;
            }
        }

        self.outbox.extend_from_slice(&out);
        if self.engine.dirty {
            self.engine.dirty = false;
            crate::config::write_offline_store(&self.data_dir, store);
        }
        drop(shared);
        self.publish_qty_ask();
    }

    /// Tell the UI what the engine is waiting for, or that it is waiting for nothing.
    fn publish_qty_ask(&self) {
        let mut g = WH_QTY.lock().unwrap();
        match (self.engine.pending_qty(), g.as_ref()) {
            (Some(ask), None) => {
                // ⛔ Drop anything left over from the LAST question before asking a new one.
                // An answer that arrived after its ask was withdrawn would otherwise sit
                // here and resolve this one instantly, with a number chosen for a different
                // item.
                *WH_QTY_ANSWER.lock().unwrap() = None;
                log::info!("offline: asking how many — op {:#04x}, up to {} (cat {:#04x} idx {:#04x})", ask.op, ask.max, ask.cat, ask.idx);
                *g = Some((ask, Instant::now()));
            }
            (None, Some(_)) => *g = None,
            _ => {}
        }
    }

    /// Finish a request that was held for a quantity, once the player has answered (or
    /// waited too long). Called from the guest's `recv`/`poll_readable`, because a
    /// blocked client sends nothing but heartbeats and those are five seconds apart.
    fn pump(&mut self) {
        let Some(ask) = self.engine.pending_qty() else {
            return;
        };
        let answered = WH_QTY_ANSWER.lock().unwrap().take();
        let reply = match answered {
            Some(r) => r,
            // ⛔ A held request MUST be answered. If the dialog is never touched — the
            // player put the phone down, the app was backgrounded — the deadline gives
            // the whole stack, which is exactly what this did before it could ask.
            None => match *WH_QTY.lock().unwrap() {
                Some((_, at)) if at.elapsed() >= WH_QTY_TIMEOUT => {
                    log::warn!("offline: nobody answered the count in {WH_QTY_TIMEOUT:?} — taking all {}", ask.max);
                    QtyReply::Take(ask.max)
                }
                _ => return,
            },
        };
        // CLR needs none of the machinery below — no store, no bag view, nothing moves.
        if reply == QtyReply::Cancel {
            let out = self.engine.cancel_qty();
            for f in &out {
                self.outbox.extend_from_slice(f);
            }
            log::info!("offline: count cancelled — {} reply frame(s)", out.len());
            self.publish_qty_ask();
            return;
        }
        let QtyReply::Take(n) = reply else { unreachable!() };
        let env = Self::live_env();
        let mut shared = OFFLINE_STORE.lock().unwrap();
        let store = shared.get_or_insert_with(|| crate::config::read_offline_store(&self.data_dir));
        core::mem::swap(&mut self.engine.store, store);
        let out = self.engine.answer_qty(n, &env);
        core::mem::swap(&mut self.engine.store, store);
        for f in &out {
            self.outbox.extend_from_slice(f);
        }
        if self.engine.dirty {
            self.engine.dirty = false;
            crate::config::write_offline_store(&self.data_dir, store);
        }
        drop(shared);
        log::info!("offline: count {n} answered — {} reply frame(s)", out.len());
        self.publish_qty_ask();
    }

    /// Move up to `buf.len()` bytes of pending replies into `buf`.
    fn drain(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.outbox.len());
        buf[..n].copy_from_slice(&self.outbox[..n]);
        self.outbox.drain(..n);
        n
    }
}

pub struct AndroidNetwork {
    sockets: Mutex<HashMap<i32, State>>,
    next_fd: AtomicI32,
    /// fds with an outstanding `MC_netSetReadCB` registration. One entry per
    /// armed socket, cleared when the waiter fires.
    read_armed: Mutex<HashSet<i32>>,
    /// When the host last walked the map for `fd` and found nothing to hand over.
    /// Drives [`RECV_EMPTY_REUSE`]. Absent = look now.
    recv_gate: Mutex<HashMap<i32, Instant>>,
    /// App-private dir: where the warehouse file lives (`config::offline_store_path`).
    data_dir: PathBuf,
}

impl Default for AndroidNetwork {
    fn default() -> Self {
        Self::new(PathBuf::new())
    }
}

impl AndroidNetwork {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            sockets: Mutex::new(HashMap::new()),
            // Start at 3 so guest fds never look like stdio handles; the value
            // is opaque to the guest but keeps logs readable.
            next_fd: AtomicI32::new(3),
            read_armed: Mutex::new(HashSet::new()),
            recv_gate: Mutex::new(HashMap::new()),
            data_dir,
        }
    }

    /// See [`wie_backend::Network::arm_read`].
    pub fn arm_read(&self, fd: i32) -> bool {
        let newly = self.read_armed.lock().unwrap().insert(fd);
        if newly {
            log::debug!("net: fd {fd} read-callback armed");
        }
        newly
    }

    /// See [`wie_backend::Network::poll_readable`].
    ///
    /// The in-app server is readable exactly when it has produced something. It cannot
    /// fail, so there is no "readable because it died" case. ★ `pump` first: a request
    /// held for a quantity has no other way back in — a blocked client sends nothing but
    /// its five-second heartbeat.
    ///
    /// -1 is reserved for "this fd is gone" (the guest closed it), which is the one case
    /// where firing the callback would be wrong.
    pub fn poll_readable(&self, fd: i32) -> i32 {
        let verdict = {
            let mut sockets = self.sockets.lock().unwrap();
            match sockets.get_mut(&fd) {
                Some(State::Local(local)) => {
                    local.pump();
                    // ★ 136차: hand over what the host queued for the warehouse socket.
                    if fd == WAREHOUSE_FD.load(Ordering::Relaxed) {
                        let pushed: Vec<Vec<u8>> = core::mem::take(&mut *PENDING_PUSH.lock().unwrap());
                        for f in &pushed {
                            local.outbox.extend_from_slice(f);
                            log::info!("net: fd {fd} <- host push {} bytes{}", f.len(), frame_tag(f));
                        }
                    }
                    i32::from(!local.outbox.is_empty())
                }
                // Fresh: nothing to read yet, but healthy.
                Some(State::Fresh) => 0,
                None => -1,
            }
        };
        if verdict != 0 {
            self.read_armed.lock().unwrap().remove(&fd);
        }
        verdict
    }

    /// WIPI slot 2. `af`/`ty` are accepted for the trace but only AF_INET +
    /// SOCK_STREAM is meaningful here — Hero3 only ever asks for (2, 1).
    pub fn socket(&self, af: i32, ty: i32) -> i32 {
        if af != 2 || ty != 1 {
            log::warn!("net: socket({af}, {ty}) — only AF_INET/SOCK_STREAM is supported");
            return ERROR;
        }
        let fd = self.next_fd.fetch_add(1, Ordering::Relaxed);
        self.sockets.lock().unwrap().insert(fd, State::Fresh);
        log::info!("net: socket() -> fd {fd}");
        fd
    }

    /// WIPI slot 30. Every connect lands on the in-app server: no DNS, no socket, no
    /// handshake. The address the game asks for (its dead 2008 Hanbit server, as the
    /// game's own `inet_addr` output) is logged and otherwise ignored.
    pub fn connect(&self, fd: i32, addr: u32, port_be: u16, name: &str) -> i32 {
        let o = addr.to_le_bytes();
        let mut sockets = self.sockets.lock().unwrap();
        match sockets.get(&fd) {
            Some(State::Fresh) => {}
            Some(State::Local(_)) => {
                log::warn!("net: connect on fd {fd} which is already connected");
                return ERROR;
            }
            None => {
                log::warn!("net: connect on unknown fd {fd}");
                return ERROR;
            }
        }
        log::warn!(
            "net: connect(fd {fd}, {name:?}) {}.{}.{}.{}:{} -> IN-APP SERVER (100% offline)",
            o[0],
            o[1],
            o[2],
            o[3],
            u16::from_be(port_be)
        );
        sockets.insert(fd, State::Local(Box::new(LocalServer::new(self.data_dir.clone()))));
        publish_session(&sockets);
        0
    }

    /// Poll a connect. Called from the WIPI layer's completion task. The in-app server is
    /// connected from the instant it exists (device-caught 2026-08-20: answering the
    /// completion callback with anything but "connected" made the game tear the socket
    /// down before sending a byte).
    pub fn poll_connect(&self, fd: i32) -> ConnectPoll {
        match self.sockets.lock().unwrap().get(&fd) {
            Some(State::Local(_)) => ConnectPoll::Connected,
            _ => ConnectPoll::Failed,
        }
    }

    /// WIPI slot 31. Returns bytes accepted, or a negative error. The in-app server
    /// always accepts everything — there is no socket to be busy.
    pub fn send(&self, fd: i32, data: &[u8]) -> i32 {
        // ★★★ 71차-L — THE SAVE-SLOT PIN'S ONLY EXACT SOURCE ON THE ONLINE SCREENS.
        //
        // `04 3d <slot>` is the backup upload, and the guest arm that builds it — the
        // online slot list's OK (`0xa8f0`, key −5) — is ALSO the character loader:
        //
        //     rec = readSaveRecord("GameSave_11", cursor)
        //     putByte 0x04 ; putByte 0x3d ; putByte (u8)cursor ; putBytes(rec, 0xdc)
        //     deserialiseSave(rec)     ; 0xa976 -> 0x1a964, CHAR is now that character
        //     send()                   ; <- we are here, so CHAR already agrees
        //
        // `0x1a964` has exactly two callers in the whole image (this arm and world init
        // `0x3ac5c`), so this frame plus the in-play latch see every character load —
        // and the slot arrives as a BYTE IN THE FRAME, not as the slot cursor the very
        // next 뒤로 (`0xa900`) resets to 0.
        //
        // The shape is `[u32 LE len][04][3d][slot][219 B blob]` (226 B on the wire).
        // `0x3d` is emitted at six other guest sites, but every one of them is `05 3d`
        // (the warehouse page request) and `01 3d` is the login. Only `0xa95a` writes it
        // under cat 4. The recogniser is `savevault::backup_upload_slot` — pure, and held
        // to the frame the device actually produces by `savevault_selftest.rs`.
        // ★ 99차: the frame still goes out in the offline world — the guest builds it
        // regardless of who is listening — so this pin source is unchanged.
        // ★ 130차: the byte in the frame is the guest's ROW; the pin lives in REAL slots.
        if let Some(row) = crate::savevault::backup_upload_slot(data)
            && let Some(slot) = crate::slotwindow::real_slot(crate::slot_first(), row)
        {
            crate::vault::note_slot_loaded(slot, "04 3d on the online slot list");
        }

        let mut sockets = self.sockets.lock().unwrap();
        let Some(State::Local(local)) = sockets.get_mut(&fd) else {
            log::warn!("net: send on fd {fd} which is not connected");
            return ERROR;
        };
        // ★ 136차: remember which socket the WAREHOUSE talks on, so a host-pushed page
        //   refresh (`sort_warehouse`) lands where the client reads it.
        if data.len() >= 6 && data[4] == crate::offline::CAT_WAREHOUSE {
            WAREHOUSE_FD.store(fd, Ordering::Relaxed);
        }
        local.feed(data);
        log::info!("net: send fd {fd} {} bytes -> in-app server{}", data.len(), frame_tag(data));
        data.len() as i32
    }

    /// WIPI slot 32. `n > 0` = data, [`WOULDBLOCK`] = nothing yet (socket still
    /// healthy). The in-app server never closes, so 0 / negative never happen here.
    ///
    /// The 0-vs-WOULDBLOCK distinction is load-bearing: returning 0 for "no data"
    /// makes the game tear down a perfectly good connection.
    ///
    /// ★ The guest over-asks by 8 bytes and we must not take it at its word.
    /// Its receive object is one 1028-byte allocation (client.bin image+0x7e0fc
    /// `ldr r0, =0x404`) whose wire bytes start at obj+0xc, yet the pump calls
    /// `recv(fd, obj + 12 + used, 1024 - used)`. Satisfying that request in full
    /// writes through obj+1035 — eight bytes past the object. Since
    /// `used = 1024 - max`, the space actually left is
    /// `1028 - (12 + used) = max - 8`, so delivery is clamped to that. This is a
    /// latent bug in the original game; it never fired against the original
    /// server but is trivially reachable from any other one.
    ///
    /// ★ THROTTLED (2026-08-13, `MIGRATION_LGT.md` §2.24, and again for the in-app
    /// server on 2026-08-20). While the game waits on a reply its wait loop calls this
    /// ~25,000 times a second; an EMPTY outbox is answered from the timestamp gate for
    /// [`RECV_EMPTY_REUSE`] without touching the socket map. See there for why that
    /// cannot delay a reply that already exists.
    pub fn recv(&self, fd: i32, buf: &mut [u8]) -> i32 {
        RECV_POLLS.fetch_add(1, Ordering::Relaxed);
        let safe_len = buf.len().saturating_sub(GUEST_RECV_SLACK);
        if safe_len == 0 {
            // The guest left itself no usable room; nothing can be delivered
            // safely, but the socket is fine, so report "not yet".
            return WOULDBLOCK;
        }
        let buf = &mut buf[..safe_len];

        if let Some(last) = self.recv_gate.lock().unwrap().get(&fd).copied()
            && last.elapsed() < RECV_EMPTY_REUSE
            && self.local_idle(fd)
        {
            RECV_ABSORBED.fetch_add(1, Ordering::Relaxed);
            return WOULDBLOCK;
        }
        let mut sockets = self.sockets.lock().unwrap();
        let Some(State::Local(local)) = sockets.get_mut(&fd) else {
            log::warn!("net: recv on fd {fd} which is not connected");
            return ERROR;
        };
        // ★ See `poll_readable`: this is the other place a held request resumes.
        local.pump();
        let n = local.drain(buf);
        if n == 0 {
            // Healthy but idle. The in-app server never "closes", so this is the
            // only non-data answer it can give — stamped so the next few thousand
            // polls are answered without touching this map at all.
            drop(sockets);
            self.recv_gate.lock().unwrap().insert(fd, Instant::now());
            return WOULDBLOCK;
        }
        self.recv_gate.lock().unwrap().remove(&fd);
        log::info!("net: recv fd {fd} {n} bytes from the in-app server{}", frame_tag(&buf[..n]));
        n as i32
    }

    /// Is `fd` the in-app server with nothing to hand over?
    ///
    /// Separate from the drain path so the throttle above can ask it while holding
    /// the socket lock for the length of one lookup.
    fn local_idle(&self, fd: i32) -> bool {
        matches!(self.sockets.lock().unwrap().get(&fd), Some(State::Local(l)) if l.outbox.is_empty())
    }

    /// WIPI slot 33.
    pub fn close(&self, fd: i32) {
        self.read_armed.lock().unwrap().remove(&fd);
        self.recv_gate.lock().unwrap().remove(&fd);
        let mut sockets = self.sockets.lock().unwrap();
        if sockets.remove(&fd).is_some() {
            log::info!("net: close fd {fd}");
        }
        publish_session(&sockets);
    }

    /// WIPI slot 1 (`MC_netClose`) — drop every socket.
    pub fn close_all(&self) {
        self.read_armed.lock().unwrap().clear();
        self.recv_gate.lock().unwrap().clear();
        // ★ 136차: a push queued for a session that just ended has nowhere to go.
        PENDING_PUSH.lock().unwrap().clear();
        WAREHOUSE_FD.store(-1, Ordering::Relaxed);
        let n = {
            let mut sockets = self.sockets.lock().unwrap();
            let n = sockets.drain().count();
            publish_session(&sockets);
            n
        };
        if n > 0 {
            log::info!("net: close_all dropped {n} socket(s)");
        }
    }
}

/// The frame's `cat`/`op` header as a log fragment, or empty when the buffer is not
/// even a whole header.
///
/// ★ HEADER ONLY, never the payload (player report 2026-08-06: "#으로 초기화하고 #으로
/// 다시 복구 선택하면 창고로 접속이 되고있어"). That symptom is a conversation between
/// the guest client and the private server, and the host is the only place that sees
/// both halves of it — but the payload carries the 16-byte account id and the 219 B
/// character save, so logging it would put both in logcat, where anything on the phone
/// can read them. `cat`/`op` alone splits the two candidate causes: a `04 3e` sent with
/// no `04 3e` coming back means the DEPLOYED server does not have the RESTORE reply
/// (the code and its selftest here do), while `04 3e` in both directions moves the bug
/// to how the client handles the reply.
///
/// Frame = `[u32 LE len][cat][op]…`, the same layout the login injection above relies on.
/// Is every whole frame in `data` a heartbeat (`00 0a`)?
///
/// Used to decide whether the guest is really "talking again" — see the notice
/// auto-dismiss. A buffer with no complete frame in it counts as heartbeats-only,
/// because a partial frame says nothing yet either.
fn only_heartbeats(data: &[u8]) -> bool {
    let mut i = 0usize;
    while i + 6 <= data.len() {
        let total = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        if total < 6 || i + total > data.len() {
            break;
        }
        if data[i + 4] != crate::offline::CAT_HEARTBEAT {
            return false;
        }
        i += total;
    }
    true
}

fn frame_tag(data: &[u8]) -> String {
    if data.len() < 6 {
        return String::new();
    }
    format!(" cat={:02x} op={:02x}", data[4], data[5])
}
impl wie_backend::Network for AndroidNetwork {
    fn socket(&self, af: i32, ty: i32) -> i32 {
        AndroidNetwork::socket(self, af, ty)
    }

    fn connect(&self, fd: i32, addr: u32, port_be: u16, name: &str) -> i32 {
        AndroidNetwork::connect(self, fd, addr, port_be, name)
    }

    fn poll_connect(&self, fd: i32) -> i32 {
        match AndroidNetwork::poll_connect(self, fd) {
            ConnectPoll::Connected => 1,
            ConnectPoll::Failed => -1,
        }
    }

    fn send(&self, fd: i32, data: &[u8]) -> i32 {
        AndroidNetwork::send(self, fd, data)
    }

    fn recv(&self, fd: i32, buf: &mut [u8]) -> i32 {
        AndroidNetwork::recv(self, fd, buf)
    }

    fn arm_read(&self, fd: i32) -> bool {
        AndroidNetwork::arm_read(self, fd)
    }

    fn poll_readable(&self, fd: i32) -> i32 {
        AndroidNetwork::poll_readable(self, fd)
    }

    fn close(&self, fd: i32) {
        AndroidNetwork::close(self, fd)
    }

    fn close_all(&self) {
        AndroidNetwork::close_all(self)
    }
}
