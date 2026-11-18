// The OFFLINE private server: the network warehouse, the cash shop and character
// recovery, served from inside the app with no socket and no python.
//
// ── WHY (player request, 2026-08-20) ─────────────────────────────────────────
// "오프라인으로 캐시샵과 네트워크 공용창고 기능을 이용할 수 있도록 하는걸 진행해야
// 될거같아". Three things make that worth doing beyond convenience:
//
//   * Android destroys the socket the moment the app is cached (ECONNABORTED, os
//     error 103), and the only cure is a foreground service the user has declined.
//     An in-process server cannot be killed by the freezer.
//   * The warehouse UI is the game's own, and it is the ONLY way to reach several
//     features (orb socketing, the cash shop, 소켓확장). Losing the server loses them.
//   * Every byte the server can send is a byte that can damage the save — the
//     `04 3e` restore reply overwrites a character on arrival. Offline, the engine
//     runs where the save records are, so it can simply refuse to send one.
//
// ── HOW IT IS WIRED ──────────────────────────────────────────────────────────
// `network.rs` grows a `State::Local`: `connect` creates an [`Engine`], `send`
// feeds it the guest's bytes, `recv` drains its replies. The guest's WIPI socket
// calls are unchanged, so the game cannot tell the difference — which is the point,
// because the client's WAIT modals are released by REPLY FRAMES and nothing else.
//
// ── FIDELITY ─────────────────────────────────────────────────────────────────
// Every reply here mirrors `hero3_server/hero3_protocol.py`, and the comments carry
// the device-measured constraints forward, because each of them cost a device
// session to learn and a fresh implementation loses them silently:
//
//   * `N` is the number of REAL rows, never padded to 16 — the client draws N cells
//     and resolves each from its own item DB, so a zeroed pad row renders as the DB's
//     item (0,0) = 투신의 머리띠 (46차-B).
//   * The two list header bytes are a CURSOR and a PAGE COUNT, both read as SIGNED
//     chars, so neither may exceed 0x7F (45차).
//   * The cash cursor is a PAGE INDEX echoed back ±1; the same cursor must always
//     give the same page (46차).
//   * A notice with an EMPTY body CRASHES the client's paged text builder.
//   * A cat-5 failure ack must carry its text in the same frame (`05 <op> 00 00
//     <euc-kr> 00`), or the client's message box reads past the end of the frame.
//
// This module is DEPENDENCY-FREE apart from the generated catalog table, so
// `offline_selftest.rs` can `include!` the real source (CLAUDE.md's convention).
// Storage I/O lives in `network.rs`; everything here is pure.

use crate::cash_catalog::CATALOG;

// ── wire constants (PROTOCOL.md, hero3_protocol.py) ──────────────────────────

pub const CAT_HEARTBEAT: u8 = 0x00;
pub const CAT_LOGIN: u8 = 0x01;
pub const CAT_RECOVERY: u8 = 0x04;
pub const CAT_WAREHOUSE: u8 = 0x05;
pub const CAT_SMS: u8 = 0x1f;

pub const OP_HELLO: u8 = 0x01;
pub const OP_AUTH: u8 = 0x3d;
pub const OP_FINAL: u8 = 0x3e;

pub const OP_RC_BACKUP: u8 = 0x3d;
pub const OP_RC_RESTORE: u8 = 0x3e;

pub const OP_WH_XFER: u8 = 0x03;
pub const OP_WH_DOWNLOAD: u8 = 0x04;
pub const OP_WH_DISCARD: u8 = 0x05;
pub const OP_WH_ORB: u8 = 0x06;
pub const OP_WH_GOLD: u8 = 0x14;
pub const OP_WH_SLOT: u8 = 0x3d;
pub const OP_WH_MONEY: u8 = 0x3e;
pub const OP_WH_CASH_INFO: u8 = 0x3f;
pub const OP_WH_GOLD_DOWN: u8 = 0x40;
pub const OP_WH_UPLOAD: u8 = 0x41;
pub const OP_WH_CASH_BUY: u8 = 0x42;

/// One item on the wire.
pub const ITEM_LEN: usize = 20;
/// Record byte `[10]` — the STACK COUNT the row carries.
///
/// ★★ A WAREHOUSE ROW IS NOT ONE ITEM (2026-08-25, player report: "A오브 10개를
/// 네트워크 창고에 넣고서 그중 1개를 장비에 결합하면 A오브 10개가 전부 사라지는거야").
/// A deposit hands over a whole bag STACK in a single `05 41`, so one row can hold N
/// units — it is the same byte the client paints in the cell's corner (`0xff80`) and
/// the same one the grant path stores as the delivered stack size (`0x1360e strh`).
/// Anything here that consumes ONE unit must therefore go through [`Store::take_one`]
/// and not through [`Store::take_item`], which takes the row and everything in it.
pub const ITEM_COUNT_OFF: usize = 10;
/// The wire record's category and index bytes (72차: `cat +8`, `idx +9`, `count +10`).
pub const ITEM_CAT_OFF: usize = 8;
pub const ITEM_IDX_OFF: usize = 9;
/// The client's list arrays are a fixed 16 rows and it does NOT clamp — N > 16
/// overruns them.
pub const WH_MAX_SLOTS: usize = 16;
/// One 창고열쇠 bag = one page.
pub const WH_MAX_BAGS: u32 = 8;
/// Stacks per WAREHOUSE BOX (오브가방 · 재료가방 · 소비가방 of the network warehouse) —
/// each box is its own room BESIDE the grid, not a share of it (★★★ 120차, user decision
/// 2026-09-20: *"네트워크 창고도 인게임 처럼 진행되어야해"*). The size follows the bag's
/// rule (`bagbox::CAP` = 80 = the bag at its largest, 5 tabs x 16): the warehouse at its
/// largest is 8 bags x 16 = 128, so a box holds 128 stacks — FIXED, not per bag bought,
/// exactly as the bag's boxes do not grow with 여행가방.
///
/// ⛔ 96차..119차 counted the box rows against `bags x 16` ("otherwise the boxes are free
/// storage") — a one-bag warehouse with sixteen orbs in it was FULL while its grid showed
/// nothing but the three openers. The user's model is the bag's: grid + box + box + box.
pub const WH_BOX_CAP: usize = WH_MAX_BAGS as usize * WH_MAX_SLOTS;
/// How many warehouse boxes there are (오브 · 재료 · 소비) — [`box_of`] indexes them.
pub const WH_BOXES: usize = 3;
/// Absolute per-account row cap — a runaway guard, NOT the player's capacity.
///
/// ★ 2026-08-25: what a deposit is actually measured against is [`Store::capacity`]
/// (`bags` x 16) for the grid and, since 120차, [`WH_BOX_CAP`] per box — so the most an
/// account can legitimately hold is `128 + 3 x 128` = 512 rows, and this guard is
/// unreachable through a deposit. It survives as defence in depth around
/// [`Store::put_item`], which `put_backup` and the loader also use.
pub const WH_MAX_ROWS: usize = WH_MAX_BAGS as usize * WH_MAX_SLOTS + WH_BOXES * WH_BOX_CAP;
/// Units one row may hold — the client's own stack cap.
///
/// `0x135e2 cmp #0x63` caps a bag stack at 99 and hands the leftover back; `0x52a60`
/// refuses to even pick a cell already over 98 as a merge target. A row bigger than that
/// could not be granted in one piece, and a row of 128+ would be read as a NEGATIVE
/// count (`0x135da`) and SUBTRACT from the player's stack.
pub const STACK_MAX: u8 = 99;
/// Both list header bytes are SIGNED chars on the client (`0x138e0`/`0x138e4`), so
/// 0x80 and up are negative there — which is what killed the 7/9 page keys at eight
/// bags (45차). Clamp, never mask.
pub const WH_WIRE_CAP_MAX: u32 = 0x7f;
/// Cash rows per page.
pub const CASH_PAGE_SIZE: usize = WH_MAX_SLOTS;
/// The recovery blob the client uploads and expects back.
pub const RECOVERY_BLOB_LEN: usize = 0xdb;
/// Slots the game has.
pub const SLOT_MAX: u8 = 2;

/// Cash-shop rows with a side effect at PURCHASE (the id is `body[0]` with
/// `body[1] == 0x11`, the tag `_cash_item_record` seeds into every row's key).
pub const WAREHOUSE_KEY_IDX: u8 = 0x13;
pub const ORB_BOX_IDX: u8 = 0x19;
pub const SOCKET_EXPAND_IDX: u8 = 0x1a;

/// Item categories the orb-combine arm checks.
pub const WH_CAT_ORB: u8 = 0x10;
pub const WH_CAT_EQUIP_MAX: u8 = 0x0c;
pub const ORB_ITEM_CATEGORY: u8 = 0x10;
pub const ORB_INDEX_COUNT: u8 = 15;
/// The orb index that ADDS a socket instead of filling one (소켓확장, 44차).
pub const ORB_IDX_SOCKET_EXPAND: u8 = 0x0f;
/// Sockets one piece of equipment can ever have.
pub const MAX_SOCKETS: u8 = 5;

// ── EUC-KR strings ───────────────────────────────────────────────────────────
//
// Pre-encoded byte arrays rather than a conversion: this module must stay
// dependency-free, and every one of these goes to the client verbatim. The Korean
// each one spells is in the doc comment above it; regenerate with
// `python -c "print(', '.join(hex(b) for b in '…'.encode('euc-kr')))"`.

/// `오프라인 창고 접속완료` — the login notice. MUST be non-empty: the client's paged
/// text-box builder walks a null page list on an empty one and dies at 0x15b2da.
pub const MOTD: &[u8] = &[
    0xbf, 0xc0, 0xc7, 0xc1, 0xb6, 0xf3, 0xc0, 0xce, 0x20, 0xc3, 0xa2, 0xb0, 0xed, 0x20, 0xc1, 0xa2, 0xbc, 0xd3, 0xbf, 0xcf, 0xb7, 0xe1,
];
/// `이용할 수 없는 기능입니다.` — the answer to 복구, which is retired (60차).
///
/// The player asked for the slot screen's `#` to keep DELETING an occupied slot while
/// 복구 on an empty one says the feature is gone. The host does not have to tell the
/// two apart from guest state — and an attempt to do that got `[1,1,1]` occupancy for
/// a screen showing one character. **A 복구 is defined by the request it sends**, and
/// a delete is purely local, so the refusal belongs here, where the request arrives.
pub const NO_RECOVERY: &[u8] = &[
    0xc0, 0xcc, 0xbf, 0xeb, 0xc7, 0xd2, 0x20, 0xbc, 0xf6, 0x20, 0xbe, 0xf8, 0xb4, 0xc2, 0x20, 0xb1, 0xe2, 0xb4, 0xc9, 0xc0, 0xd4, 0xb4, 0xcf, 0xb4,
    0xd9, 0x2e,
];
/// `창고가 가득찼습니다.`
pub const WH_FULL: &[u8] = &[
    0xc3, 0xa2, 0xb0, 0xed, 0xb0, 0xa1, 0x20, 0xb0, 0xa1, 0xb5, 0xe6, 0xc3, 0xa1, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e,
];
/// `더 이상 가방을 늘릴 수 없습니다.`
pub const BAG_CAP: &[u8] = &[
    0xb4, 0xf5, 0x20, 0xc0, 0xcc, 0xbb, 0xf3, 0x20, 0xb0, 0xa1, 0xb9, 0xe6, 0xc0, 0xbb, 0x20, 0xb4, 0xc3, 0xb8, 0xb1, 0x20, 0xbc, 0xf6, 0x20, 0xbe,
    0xf8, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e,
];
/// `오브 결합에 실패했습니다.`
pub const ORB_FAIL: &[u8] = &[
    0xbf, 0xc0, 0xba, 0xea, 0x20, 0xb0, 0xe1, 0xc7, 0xd5, 0xbf, 0xa1, 0x20, 0xbd, 0xc7, 0xc6, 0xd0, 0xc7, 0xdf, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9,
    0x2e,
];
/// `오브를 결합할 수 없는 장비입니다.`
pub const ORB_BAD_EQUIP: &[u8] = &[
    0xbf, 0xc0, 0xba, 0xea, 0xb8, 0xa6, 0x20, 0xb0, 0xe1, 0xc7, 0xd5, 0xc7, 0xd2, 0x20, 0xbc, 0xf6, 0x20, 0xbe, 0xf8, 0xb4, 0xc2, 0x20, 0xc0, 0xe5,
    0xba, 0xf1, 0xc0, 0xd4, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e,
];
/// `더 이상 홈을 늘릴 수 없습니다.`
pub const ORB_SOCKET_CAP: &[u8] = &[
    0xb4, 0xf5, 0x20, 0xc0, 0xcc, 0xbb, 0xf3, 0x20, 0xc8, 0xa8, 0xc0, 0xbb, 0x20, 0xb4, 0xc3, 0xb8, 0xb1, 0x20, 0xbc, 0xf6, 0x20, 0xbe, 0xf8, 0xbd,
    0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e,
];
/// `빈 오브 홈이 없습니다.`
pub const ORB_NO_ROOM: &[u8] = &[
    0xba, 0xf3, 0x20, 0xbf, 0xc0, 0xba, 0xea, 0x20, 0xc8, 0xa8, 0xc0, 0xcc, 0x20, 0xbe, 0xf8, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e,
];
/// `오브 결합에 성공했습니다.`
pub const ORB_OK: &[u8] = &[
    0xbf, 0xc0, 0xba, 0xea, 0x20, 0xb0, 0xe1, 0xc7, 0xd5, 0xbf, 0xa1, 0x20, 0xbc, 0xba, 0xb0, 0xf8, 0xc7, 0xdf, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9,
    0x2e,
];
/// `오브 홈이 하나 늘었습니다.`
pub const ORB_EXPANDED: &[u8] = &[
    0xbf, 0xc0, 0xba, 0xea, 0x20, 0xc8, 0xa8, 0xc0, 0xcc, 0x20, 0xc7, 0xcf, 0xb3, 0xaa, 0x20, 0xb4, 0xc3, 0xbe, 0xfa, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4,
    0xd9, 0x2e,
];
/// `가방에 ` — the head of the delivery notice, followed by `<n>개 넣었습니다.`.
///
/// ★★ It said `창고에` until 2026-08-25. The two engine-delivered purchases now land in
/// the player's BAG (see [`Engine::cash_buy`]), so the message had to follow them.
///
/// See [`delivered_notice`]: this is the message those purchases answer with, and
/// answering with a MESSAGE at all is what stops the client granting its own inert copy
/// of the shop row (65차).
pub const DELIVERED_HEAD: &[u8] = &[0xb0, 0xa1, 0xb9, 0xe6, 0xbf, 0xa1, 0x20];
/// `개 넣었습니다.`
pub const DELIVERED_TAIL: &[u8] = &[0xb0, 0xb3, 0x20, 0xb3, 0xd6, 0xbe, 0xfa, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e];
/// `가방이 가득찼습니다.` — a withdraw (or a delivery) the player has no room for.
pub const BAG_FULL: &[u8] = &[
    0xb0, 0xa1, 0xb9, 0xe6, 0xc0, 0xcc, 0x20, 0xb0, 0xa1, 0xb5, 0xe6, 0xc3, 0xa1, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e,
];
/// `취소되었습니다.` — the player pressed CLR on a quantity picker.
///
/// ★ Answering the held 올리기/내리기 with STATUS 0 is what makes this a real cancel and
/// not just a closed dialog: the client's `05 41` handler (`0x139d6`) tests the status
/// byte FIRST and only clears the bag cell, saves and sends its `05 03` commit on the
/// success arm — a zero sends it straight to `showModal(body+2, 7, 1, 1)` (`0x13ee6`)
/// with nothing moved. 내리기 has not touched anything at that point either.
pub const CANCELLED: &[u8] = &[0xc3, 0xeb, 0xbc, 0xd2, 0xb5, 0xc7, 0xbe, 0xfa, 0xbd, 0xc0, 0xb4, 0xcf, 0xb4, 0xd9, 0x2e];
/// `저장 완료` — the first backup commit on a connection.
pub const SAVE_DONE: &[u8] = &[0xc0, 0xfa, 0xc0, 0xe5, 0x20, 0xbf, 0xcf, 0xb7, 0xe1];
/// `동기화` — a repeat backup commit (the client replays it on every page turn).
pub const SYNCED: &[u8] = &[0xb5, 0xbf, 0xb1, 0xe2, 0xc8, 0xad];

// ── framing ──────────────────────────────────────────────────────────────────

/// `[u32 LE total_len][cat][op][body…]`, total_len INCLUDING the 4-byte header.
pub fn frame(cat: u8, op: u8, body: &[u8]) -> Vec<u8> {
    let total = 4 + 2 + body.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.push(cat);
    out.push(op);
    out.extend_from_slice(body);
    out
}

/// Split a byte stream into whole payloads (`[cat][op][body…]`), returning the
/// number of bytes consumed. A partial tail is left for the next read.
///
/// A declared length under 6 cannot be a frame (the client's own dispatcher drops
/// `total_len <= 5` silently, PROTOCOL.md §1) and would loop here, so it discards
/// the buffer: a stream that malformed is not going to resynchronise.
pub fn split_frames(buf: &[u8]) -> (Vec<Vec<u8>>, usize) {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 6 <= buf.len() {
        let total = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
        if total < 6 {
            return (out, buf.len());
        }
        if i + total > buf.len() {
            break;
        }
        out.push(buf[i + 4..i + total].to_vec());
        i += total;
    }
    (out, i)
}

// ── persistent state ─────────────────────────────────────────────────────────

/// Everything the offline server remembers between launches.
///
/// Text, one record per line, because it costs nothing and a player who loses items
/// can be asked to paste the file. Unknown lines are PRESERVED on rewrite so a file
/// written by a newer build survives a downgrade.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    pub gold: u32,
    pub cash: u32,
    /// 창고열쇠 purchases. 1 = the default single bag.
    pub bags: u32,
    pub items: Vec<[u8; ITEM_LEN]>,
    /// `(slot, 219-byte blob)` — the recovery backups, newest per slot.
    pub backups: Vec<(u8, Vec<u8>)>,
    /// Lines this build did not understand, kept verbatim.
    pub unknown: Vec<String>,
}

fn hex_of(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    s
}

fn bytes_of(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let h = hex.as_bytes();
    let mut out = Vec::with_capacity(hex.len() / 2);
    for i in (0..h.len()).step_by(2) {
        let hi = (h[i] as char).to_digit(16)?;
        let lo = (h[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

impl Store {
    /// A brand-new offline warehouse: one bag, no gold, nothing in it.
    pub fn new() -> Self {
        Self {
            bags: 1,
            ..Default::default()
        }
    }

    pub fn encode(&self) -> String {
        let mut out = String::from("v1\n");
        out.push_str(&format!("gold {}\n", self.gold));
        out.push_str(&format!("cash {}\n", self.cash));
        out.push_str(&format!("bags {}\n", self.bags.clamp(1, WH_MAX_BAGS)));
        for it in &self.items {
            out.push_str(&format!("item {}\n", hex_of(it)));
        }
        for (slot, blob) in &self.backups {
            out.push_str(&format!("backup {} {}\n", slot, hex_of(blob)));
        }
        for line in &self.unknown {
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Parse, tolerating anything. A corrupt or truncated file must degrade to "some
    /// rows are missing", never to a refusal to open the warehouse — the player would
    /// have no way back in.
    pub fn decode(text: &str) -> Self {
        let mut s = Self::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line == "v1" {
                continue;
            }
            let Some((key, rest)) = line.split_once(' ') else {
                s.unknown.push(line.to_string());
                continue;
            };
            match key {
                "gold" => s.gold = rest.trim().parse().unwrap_or(0),
                "cash" => s.cash = rest.trim().parse().unwrap_or(0),
                "bags" => s.bags = rest.trim().parse::<u32>().unwrap_or(1).clamp(1, WH_MAX_BAGS),
                "item" => {
                    if let Some(b) = bytes_of(rest.trim())
                        && b.len() == ITEM_LEN
                        && s.items.len() < WH_MAX_ROWS
                    {
                        let mut rec = [0u8; ITEM_LEN];
                        rec.copy_from_slice(&b);
                        s.items.push(rec);
                    }
                }
                "backup" => {
                    if let Some((slot, hex)) = rest.trim().split_once(' ')
                        && let (Ok(slot), Some(blob)) = (slot.parse::<u8>(), bytes_of(hex.trim()))
                        && slot <= SLOT_MAX
                        && blob.len() == RECOVERY_BLOB_LEN
                    {
                        s.backups.retain(|(sl, _)| *sl != slot);
                        s.backups.push((slot, blob));
                    }
                }
                _ => s.unknown.push(line.to_string()),
            }
        }
        s.normalise();
        s
    }

    /// Fold rows that the client would consider one stack into one row.
    ///
    /// ★ Run on LOAD, not only on deposit (2026-08-25), because a warehouse filled
    /// before deposits merged is full of one-unit rows of the same thing — the device
    /// this was written on held dozens of 소켓확장 in separate cells. Leaving them split
    /// would mean the feature only applies to items deposited from now on, and the
    /// player would still be discarding them one cell at a time.
    ///
    /// Same rule as [`Self::deposit`] and the same 99 ceiling; the FIRST row of a kind
    /// keeps its key and its place, so the ordering the player sees barely moves.
    pub fn normalise(&mut self) {
        let mut out: Vec<[u8; ITEM_LEN]> = Vec::with_capacity(self.items.len());
        for mut rec in core::mem::take(&mut self.items) {
            // ⛔⛔ CLAMP FIRST, whatever the row came from. A count above 99 can only be a
            // damaged store or a row written by a build that did not have this rule — and
            // the client reads that byte SIGNED (guest 0x135da), so a 128+ row handed back
            // on a withdraw would SUBTRACT from the player's own stack. Everything else
            // here rests on this one invariant.
            if stacks(&rec) && rec[ITEM_COUNT_OFF] > STACK_MAX {
                rec[ITEM_COUNT_OFF] = STACK_MAX;
            }
            let left0 = unit_count(&rec);
            let mut left = left0;
            if stacks(&rec) {
                for row in out.iter_mut() {
                    if left == 0 {
                        break;
                    }
                    if !same_stack(row, &rec) {
                        continue;
                    }
                    let add = STACK_MAX.saturating_sub(unit_count(row)).min(left);
                    row[ITEM_COUNT_OFF] = unit_count(row) + add;
                    left -= add;
                }
            }
            if left > 0 {
                let mut keep = rec;
                keep[ITEM_COUNT_OFF] = left;
                out.push(keep);
            }
        }
        self.items = out;
    }

    /// GRID cells the player has actually PAID FOR — `bags` x 16, and never more than the
    /// per-account safety cap. ★ 120차: this is the grid's room only; the three boxes have
    /// [`WH_BOX_CAP`] each — see [`Self::free_cells_for`].
    ///
    /// ★★ 2026-08-25. Until now the only ceiling on a deposit was [`WH_MAX_ROWS`] (200),
    /// which is a runaway guard and not a capacity: a warehouse with one bag would happily
    /// take a 17th row, and `pages()` would then report TWO pages. The player got a page
    /// they never bought, and 창고열쇠 — the item whose entire purpose is to sell that
    /// page — bought nothing. Measured, not assumed: a 16-row one-bag store accepted a
    /// deposit that needed a new cell and came back `rows=17 pages=2`.
    ///
    /// ⛔ `pages()` deliberately still reports `max(bags, rows/16)`. A store that is
    /// already over capacity (an older build, or a bags count that shrank) must keep
    /// showing those rows or the player could not reach their own items — the cap belongs
    /// on what goes IN, not on what can be seen.
    pub fn capacity(&self) -> usize {
        (self.bags as usize * WH_MAX_SLOTS).min(WH_MAX_ROWS)
    }

    /// Rows in warehouse box `bx` (0 오브 · 1 재료 · 2 소비 — [`box_of`]).
    pub fn box_count(&self, bx: usize) -> usize {
        self.items.iter().filter(|r| box_of(r[ITEM_CAT_OFF]) == Some(bx)).count()
    }

    /// Free CELLS in the room an item of category `cat` goes to: its box (each
    /// [`WH_BOX_CAP`], counted apart from the grid), or the grid (`capacity()` less the
    /// rows the grid shows). ★★★ 120차 — this is the whole of "the warehouse works like the
    /// bag": grid + box + box + box, none of them sharing cells with another.
    ///
    /// Saturating on purpose: a room that is somehow OVER its cap (an older build's file,
    /// a bags count that shrank) reports 0 free cells, so a deposit needing a new row is
    /// refused while a pure merge (no new row) still goes through — [`Self::deposit`].
    pub fn free_cells_for(&self, cat: u8) -> usize {
        match box_of(cat) {
            Some(bx) => WH_BOX_CAP.saturating_sub(self.box_count(bx)),
            None => self.capacity().saturating_sub(self.shown_count()),
        }
    }

    /// How many UNITS of `rec` this warehouse can still accept, counting both the room
    /// left in matching stacks and the room in free cells — the cells of the room `rec`
    /// goes to (its box, or the grid).
    ///
    /// This is what makes "fill to 99 first, then spill" answerable BEFORE anything moves:
    /// the quantity dialog can offer exactly this many and no more, so the player never
    /// picks a number that gets refused (player request 2026-08-25 — "49개 넣으려고하면
    /// 가능해지면서 50개짜리 … 오브가 50->99로 변경").
    pub fn room_for(&self, rec: &[u8; ITEM_LEN]) -> u32 {
        let free_cells = self.free_cells_for(rec[ITEM_CAT_OFF]) as u32;
        if !stacks(rec) {
            // Equipment never merges (each piece carries its own refine / sockets / orb),
            // so a cell holds exactly one.
            return free_cells;
        }
        let in_stacks: u32 = self
            .items
            .iter()
            .filter(|row| same_stack(row, rec))
            .map(|row| u32::from(STACK_MAX.saturating_sub(unit_count(row))))
            .sum();
        in_stacks + free_cells * u32::from(STACK_MAX)
    }

    /// Store one deposit. `false` = it does not fit, and NOTHING was changed.
    ///
    /// ★ A key COLLISION is re-keyed, not replaced (34차): every purchase of one
    /// catalog row carries the same fixed serial, so replacing would silently eat the
    /// first copy of a duplicated cash item. The re-key keeps both rows, and only the
    /// low byte moves — invisible to the client, which round-trips the key as an
    /// opaque handle.
    pub fn put_item(&mut self, record: [u8; ITEM_LEN]) -> bool {
        if self.items.len() >= WH_MAX_ROWS {
            return false;
        }
        let mut rec = record;
        let mut guard = 0u32;
        while self.items.iter().any(|it| it[..8] == rec[..8]) {
            rec[0] = rec[0].wrapping_add(1);
            guard += 1;
            if guard > 256 {
                return false;
            }
        }
        self.items.push(rec);
        true
    }

    /// Remove and return the row whose 8-byte key matches — the WHOLE row, stack and
    /// all. Correct for 내리기 and 버리기, where the player means the cell; ⛔ wrong for
    /// anything that consumes a single unit — use [`Self::take_one`] there.
    pub fn take_item(&mut self, key: &[u8]) -> Option<[u8; ITEM_LEN]> {
        if key.len() < 8 {
            return None;
        }
        let i = self.items.iter().position(|it| it[..8] == key[..8])?;
        Some(self.items.remove(i))
    }

    /// Take exactly ONE unit out of the row whose 8-byte key matches.
    ///
    /// ★★ THE 오브 결합 DATA LOSS (2026-08-25). 오브 결합 consumes one orb, but it used
    /// to reach for [`Self::take_item`] — so socketing one orb out of a deposited stack
    /// of ten destroyed all ten, and the failure paths "put the item back" as a single
    /// unit, which lost the other nine just as thoroughly. A stack keeps its place, its
    /// key and its remaining units; only a row down to its last unit leaves the store.
    ///
    /// The returned record is always ONE unit (`[ITEM_COUNT_OFF] == 1`) when it came out
    /// of a stack. A row that was already single is returned VERBATIM, bytes untouched —
    /// that is the overwhelmingly common case (every equipment row, every orb the shop
    /// delivers) and it must stay byte-identical to what the old code did.
    pub fn take_one(&mut self, key: &[u8]) -> Option<[u8; ITEM_LEN]> {
        self.take_units(key, 1)
    }

    /// Take `want` units (clamped to what the row holds) out of the row `key` names.
    ///
    /// The row keeps the remainder in place, with its key and position untouched; a row
    /// taken down to nothing leaves the store. Asking for the whole row — or for more —
    /// returns it VERBATIM, which is what 내리기 and 버리기 have always done.
    pub fn take_units(&mut self, key: &[u8], want: u8) -> Option<[u8; ITEM_LEN]> {
        if key.len() < 8 {
            return None;
        }
        let i = self.items.iter().position(|it| it[..8] == key[..8])?;
        let held = unit_count(&self.items[i]);
        if want >= held {
            return Some(self.items.remove(i));
        }
        self.items[i][ITEM_COUNT_OFF] = held - want.max(1);
        let mut out = self.items[i];
        out[ITEM_COUNT_OFF] = want.max(1);
        Some(out)
    }

    /// Store a deposit, MERGING it into the rows the client would merge it into.
    ///
    /// ── THE RULE IS THE CLIENT'S OWN (LGT RE, 2026-08-25) ────────────────────
    /// `addItemToInventory` (`0x131d0`) hands the incoming record to the bag's slot
    /// scanner (`0x529a0`), and that scanner:
    ///   * splits on **`cat > 0x0c`** (`0x529ca cmp #0xc / bgt`) — equipment finds an
    ///     EMPTY cell and never merges, because every piece carries its own refine
    ///     level, socket count and socketed orb;
    ///   * matches a merge target on **`(cat, idx)` and nothing else** (`0x52a72`,
    ///     `0x52a86`), skipping any cell already holding more than 98 (`0x52a60`);
    ///   * and the merge itself caps at **99** (`0x135e2 cmp #0x63`), handing the
    ///     leftover back to the caller, which grants it again into another cell
    ///     (`0x13d24`).
    /// Mirroring it exactly is what makes a merged warehouse row safe to hand back:
    /// the client will accept it into one bag stack on the way out.
    ///
    /// ⚠ AND THE COUNT IS SIGNED ON THE CLIENT (`0x135da lsls/asrs #0x18`): a row of
    /// 128+ would be read as NEGATIVE and SUBTRACT from the player's stack. [`STACK_MAX`]
    /// keeps every row inside the game's own 99, which is well clear of that.
    ///
    /// `false` = the per-account row cap is full and NOTHING was stored (the merge is
    /// computed dry first, so a refusal cannot leave half a deposit behind).
    pub fn deposit(&mut self, record: [u8; ITEM_LEN]) -> bool {
        if !stacks(&record) {
            // One cell, one piece — but still only if a cell is free (equipment is never
            // of a box category, so this is always the grid's room).
            if self.free_cells_for(record[ITEM_CAT_OFF]) == 0 {
                return false;
            }
            return self.put_item(record);
        }
        // Dry run: how much merges into existing rows, and how many new rows the rest
        // needs. A deposit that cannot fit must not be half-applied.
        let mut left = u32::from(unit_count(&record));
        for row in self.items.iter() {
            if left == 0 {
                break;
            }
            if same_stack(row, &record) {
                left -= u32::from(STACK_MAX.saturating_sub(unit_count(row))).min(left);
            }
        }
        let new_rows = left.div_ceil(u32::from(STACK_MAX)) as usize;
        // ⛔ The player's OWN capacity, not the runaway guard — see [`Self::capacity`] and,
        // for a box category, [`WH_BOX_CAP`] (120차: the room is the one `record` goes to).
        // All-or-nothing: the client has been told to empty the bag cell by the time the
        // commit arrives, so a half-applied deposit is a lost item.
        //
        // ★ Only NEW CELLS are charged. A deposit that merges entirely into stacks that
        // already exist needs no cell at all, so it must succeed even on a store that is
        // already over capacity (an older build, or a bags count that shrank) — otherwise
        // `room_for` would offer room that `deposit` then refuses, and the player would be
        // stuck with items they can neither store nor stack.
        if new_rows > 0 && new_rows > self.free_cells_for(record[ITEM_CAT_OFF]) {
            return false;
        }

        let mut left = unit_count(&record);
        for row in self.items.iter_mut() {
            if left == 0 {
                break;
            }
            if !same_stack(row, &record) {
                continue;
            }
            let add = STACK_MAX.saturating_sub(unit_count(row)).min(left);
            if add == 0 {
                continue;
            }
            row[ITEM_COUNT_OFF] = unit_count(row) + add;
            left -= add;
        }
        while left > 0 {
            let mut rec = record;
            let n = left.min(STACK_MAX);
            rec[ITEM_COUNT_OFF] = n;
            if !self.put_item(rec) {
                return false;
            }
            left -= n;
        }
        true
    }

    /// Undo a [`Self::take_one`]: merge the unit back into the stack it came out of, or
    /// re-file it as its own row when that stack is gone.
    ///
    /// ⛔ Only ever for a record that came back UNCHANGED. A modified unit (the socketed
    /// equipment) must go through [`Self::put_item`], which re-keys it away from the
    /// remainder — merging it would fold a different item into a stack of the old one.
    pub fn put_one_back(&mut self, rec: [u8; ITEM_LEN]) -> bool {
        if let Some(row) = self.items.iter_mut().find(|it| it[..8] == rec[..8]) {
            row[ITEM_COUNT_OFF] = unit_count(row).saturating_add(unit_count(&rec));
            return true;
        }
        self.put_item(rec)
    }

    pub fn put_backup(&mut self, slot: u8, blob: &[u8]) -> bool {
        if slot > SLOT_MAX || blob.len() != RECOVERY_BLOB_LEN {
            return false;
        }
        self.backups.retain(|(sl, _)| *sl != slot);
        self.backups.push((slot, blob.to_vec()));
        true
    }

    pub fn get_backup(&self, slot: u8) -> Option<&[u8]> {
        self.backups.iter().find(|(sl, _)| *sl == slot).map(|(_, b)| b.as_slice())
    }

    /// Pages the client may walk = the `ceiling` byte of both list headers.
    ///
    /// `max(bags, pages-worth-of-rows)`. Since 2026-08-25 a DEPOSIT is capped at
    /// [`Self::capacity`] (`bags` x 16), so this normally equals `bags` — but a store that
    /// is already over capacity (filled by an older build, or a bags count that shrank)
    /// must keep showing those rows or the player could not reach their own items. The cap
    /// belongs on what goes IN, not on what can be seen.
    pub fn pages(&self) -> u32 {
        // ⛔ The three opener rows ride at the head of page 0 (95차-B), so they push the
        // player's own rows along and the LAST page can be one further out than the item
        // count alone would say. ★ 96차: and the BOX-CATEGORY rows are not on the grid at
        // all (they live inside the boxes), so it is the SHOWN rows that are paged.
        let row_pages = (self.shown_count() + BOX_OPENERS.len()).div_ceil(WH_MAX_SLOTS) as u32;
        self.bags.max(row_pages).max(1)
    }

    // ── the grid vs. the boxes (96차, 2026-09-15 · 120차, 2026-09-20) ────────────
    //
    // ★★ User: the warehouse has its own 오브가방·재료가방·소비가방, holding the
    // warehouse's orbs, materials and consumables — so those rows are NOT on the grid
    // any more. They stay in `items` (no migration, no second store: the boxes are a
    // VIEW of the row list — `BAG_TYPES_FEASIBILITY.md §9.1`), and what changes is what
    // the grid SHOWS and, since 120차, WHICH ROOM a row is charged to.
    //
    // ★★★ 120차: the boxes are counted APART from the grid — `capacity()` (`bags x 16`)
    // is charged only by the rows the grid shows, and each box has `WH_BOX_CAP` of its
    // own (`free_cells_for`). 96차 had charged the box rows against the grid's cells
    // ("otherwise free storage"), which made a one-bag warehouse FULL while its grid
    // showed only the three openers; the user's model is the bag's — grid + box + box +
    // box (`§120차`). `pages()` and `Engine::rows` still look at the shown subset only.

    /// The rows the GRID shows: everything that is not of a box category.
    pub fn shown(&self) -> impl Iterator<Item = &[u8; ITEM_LEN]> + '_ {
        self.items.iter().filter(|r| !boxed_category(r[ITEM_CAT_OFF]))
    }

    pub fn shown_count(&self) -> usize {
        self.shown().count()
    }

    /// The rows one warehouse BOX shows: those whose category is in `cats`, in store
    /// order. Each comes with its key, which is how a verb names it later.
    pub fn boxed_rows(&self, cats: &[u8]) -> Vec<[u8; ITEM_LEN]> {
        self.items.iter().filter(|r| cats.contains(&r[ITEM_CAT_OFF])).copied().collect()
    }

    /// ★ 136차 (2026-09-25) — 정렬: reorder the rows by `key(cat, idx, count, position)`,
    /// STABLY, keeping every byte of every row (keys included — a row's identity is its
    /// key, and the box windows and verbs name rows by it). The order rule itself lives in
    /// `bagsort::wire_key` and comes in as the closure, because this file has no
    /// `use crate::` beyond the catalog and its selftest `include!`s it alone. Returns
    /// whether anything moved — the caller persists and refreshes the client only then.
    ///
    /// Grid rows and box-category rows are one list here on purpose: the grid shows
    /// `shown()` in list order and each box shows `boxed_rows()` in list order, so one
    /// sort of the list orders all four views at once.
    pub fn sort_rows_by<K: Ord>(&mut self, key: impl Fn(u8, u8, u8, u16) -> K) -> bool {
        let before = self.items.clone();
        let mut keyed: Vec<(K, usize)> = self
            .items
            .iter()
            .enumerate()
            .map(|(pos, r)| (key(r[ITEM_CAT_OFF], r[ITEM_IDX_OFF], r[ITEM_COUNT_OFF], pos as u16), pos))
            .collect();
        keyed.sort();
        self.items = keyed.into_iter().map(|(_, pos)| before[pos]).collect();
        self.items != before
    }
}

/// Is `cat` one of the categories the boxes take (오브 0x10 · 재료 0x0e · 소비
/// 0x12/0x0d)? ⛔ Must equal the union of `bagbox::Kind::cats` — a literal on both
/// sides because this file carries no `use crate::`; each selftest pins its own copy.
/// ★ 125차-B (2026-09-22, user decision): 조합서 0x0f is NOT boxed any more — recipes
/// live in the plain bag, where the smith's 특수조합 screen reads them.
pub fn boxed_category(cat: u8) -> bool {
    box_of(cat).is_some()
}

/// Which warehouse box `cat` lives in — 0 오브 · 1 재료 · 2 소비 — or `None` for a grid
/// category (equipment, quest items). ⛔ The same table as `bagbox::Kind::index` /
/// `wie_util::stats::bagbox_box_of`; a literal here for the same reason as above.
pub fn box_of(cat: u8) -> Option<usize> {
    match cat {
        0x10 => Some(0),
        0x0e => Some(1),
        0x12 | 0x0d => Some(2),
        _ => None,
    }
}

// ── the three BOX OPENERS in the warehouse grid (95차-B, 2026-09-15) ─────────
//
// ★★ User, 2026-09-15: *"네트워크 창고 내에 오브가방이랑 소비가방 재료가방 없는데?"* — the
// three cells are BAG cells, so the warehouse's own grid never had them. They are put
// there as VIRTUAL rows: the head of page 0, never stored, so nothing about the player's
// items changes and no key can ever collide with a real one.
//
// ⛔ `(cat, idx)` must match `bagbox::Kind::bag_item` — the client draws these rows from
// its own item table, so a wrong pair would draw the wrong name. Both lists are pinned by
// their own selftest (`the_masks_name_the_boxes_and_keep_the_three_cells` there, and
// [`BOX_OPENERS`]'s own test here).
// ⛔ They are NOT deposits: `Store::items` never holds them, so 오브결합 target lists,
// capacity, normalise and the save file are all untouched.
pub const BOX_OPENERS: [(u8, u8); 3] = [(0x12, 0x16), (0x12, 0x17), (0x12, 0x19)];

/// The key byte that marks a row as an opener. The client echoes a row's 8-byte key back
/// for 내리기 / 버리기 / 오브결합, and no stored row can have this one — real keys are
/// minted by [`Engine::minted`] and by the game itself.
pub const OPENER_KEY_TAG: u8 = 0xb0;

/// Is this the key of a virtual opener row?
pub fn is_opener_key(key: &[u8]) -> bool {
    key.len() >= 8 && key[0] == OPENER_KEY_TAG && key[1] < BOX_OPENERS.len() as u8 && key[2..8].iter().all(|b| *b == 0)
}

/// The three rows, in [`BOX_OPENERS`] order.
pub fn opener_rows() -> [[u8; ITEM_LEN]; 3] {
    let mut out = [[0u8; ITEM_LEN]; 3];
    for (i, (cat, idx)) in BOX_OPENERS.iter().enumerate() {
        out[i][0] = OPENER_KEY_TAG;
        out[i][1] = i as u8;
        out[i][ITEM_CAT_OFF] = *cat;
        out[i][ITEM_IDX_OFF] = *idx;
        out[i][ITEM_COUNT_OFF] = 1;
    }
    out
}

/// How many units a row holds.
///
/// A `0` counts as one: the client's grid painter prints that corner number with a bare
/// `%d` and no zero gate (`0x120d8` -> `0x12154`), and its merge scan treats such a row
/// as a live target — so a zero on the wire is a row the player can see and pick up, and
/// treating it as "empty" here would let the same row be consumed for ever.
pub fn unit_count(rec: &[u8; ITEM_LEN]) -> u8 {
    rec[ITEM_COUNT_OFF].max(1)
}

/// Does this kind of item stack at all?
///
/// The client's own split (`0x529ca`): equipment is `cat <= WH_CAT_EQUIP_MAX` and each
/// piece is its own instance (refine, sockets, socketed orb), so two of them are never
/// interchangeable. Everything above — scrolls `0x0d`, materials `0x0e`, orbs `0x10`,
/// consumables `0x12` — merges.
pub fn stacks(rec: &[u8; ITEM_LEN]) -> bool {
    rec[8] > WH_CAT_EQUIP_MAX
}

/// Would the client put these two in the SAME bag stack? `(cat, idx)`, exactly as its
/// slot scanner does (`0x52a72` / `0x52a86`) — no other byte is consulted, because for a
/// stackable kind the instance holds nothing else.
pub fn same_stack(a: &[u8; ITEM_LEN], b: &[u8; ITEM_LEN]) -> bool {
    stacks(a) && stacks(b) && a[8] == b[8] && a[9] == b[9]
}

// ── the engine ───────────────────────────────────────────────────────────────

/// What the engine needs to know about the world outside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// How many the player chose in the host's quantity picker (1 = the default).
    ///
    /// The client grants a stack from its own cached row, which the host writes
    /// (`items::cash_qty_writes`) — but the two purchases the ENGINE delivers
    /// (오브상자 / 소켓확장, both delivered into the BAG since 2026-08-25) have no cached
    /// row to read,
    /// so the count has to reach them here or the player would pay for three and
    /// receive one.
    pub cash_qty: u8,
    /// What the player's BAG can still take, as the worker last saw it.
    ///
    /// ★★ Why the engine needs this at all (2026-08-25): a withdraw hands the bag a `05 04`
    /// frame, and the guest's `addItemToInventory` (0x131d0) fills a matching stack to 99
    /// and spills the rest into a free cell — but when there is neither, it sets the
    /// bag-full flag at `grid+0x1669` and **branches straight to its epilogue at 0x13810,
    /// skipping the `ldr r0,[sp,#0x28]` that would load the leftover** (0x13252-0x1325a).
    /// So it returns 0xff, and the `05 04` handler's single retry (0x13d28) hits the same
    /// wall and DISCARDS its return. The units are simply gone — and the host has already
    /// taken them out of the warehouse by then.
    ///
    /// The host cannot read guest memory from here (this module is dependency-free and
    /// `network.rs` has no emulator handle), so the WORKER publishes a snapshot and it
    /// arrives in `Env`. Empty = "not known yet", which [`bag_room`] treats as unlimited —
    /// i.e. exactly the behaviour that shipped before, so a missing snapshot can never be
    /// the thing that refuses a legitimate withdraw.
    pub bag: BagView,
    /// ★★ 166차 — the host's 오브가방 as the intake hook will see it, or `None` (box off,
    /// unknown): then only [`Env::bag`] counts, which is what shipped before. See
    /// [`deliveries_fit`].
    pub orb_box: Option<OrbBox>,
}

/// ★★ 166차 — what the 오브가방 can still take, as the bag-box intake hook will decide it.
///
/// Every orb the engine delivers is a `05 04`, and its handler places the record with
/// `addItemToInventory` (`0x131d0`) — whose ENTRY hook offers the whole record to this box
/// before the bag is looked at. So a full bag is no reason to refuse a 소켓확장 the box can
/// hold; the bag only has to hold what the box turns away.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrbBox {
    /// Free stacks the box can still open.
    pub free_stacks: u32,
    /// By orb index: the units its open stacks can still take, or `None` for an orb the hook
    /// turns away by itself (`wie_util::stats::bagbox_view`).
    pub merge: Vec<Option<u32>>,
}

impl OrbBox {
    /// The hook's decision for one delivery of `n` units of orb `idx`, applied to this copy:
    /// all `n` or nothing, exactly as `bagbox_offer` reserves.
    fn take(&mut self, idx: u8, n: u32) -> bool {
        let Some(Some(merge)) = self.merge.get(usize::from(idx)).copied() else {
            return false;
        };
        let (stacks, after) = box_intake_cost(merge, n);
        if stacks > self.free_stacks {
            return false;
        }
        self.free_stacks -= stacks;
        self.merge[usize::from(idx)] = Some(after);
        true
    }
}

/// What taking `n` units costs a box whose stacks of the item can still take `merge`:
/// `(new stacks, merge room after)` — fill the open stacks, then whole stacks of
/// [`STACK_MAX`].
///
/// ⛔ A copy of `wie_util::stats::bagbox_intake_cost` (this module takes no dependencies);
/// `lib.rs` holds the two equal at compile time.
pub const fn box_intake_cost(merge: u32, n: u32) -> (u32, u32) {
    let cap = STACK_MAX as u32;
    let from_merge = if n < merge { n } else { merge };
    let rest = n - from_merge;
    let stacks = rest.div_ceil(cap);
    (stacks, merge - from_merge + stacks * cap - rest)
}

/// ★★ 166차 — will every delivery of `want`, sent in order as its own `05 04`, land whole?
///
/// An orb lands when the 오브가방 takes it ([`OrbBox`] — whole, or not at all), and anything
/// the box turns away must fit the bag exactly as [`bag_fits`] models it. An unread bag is
/// "no limit known", as everywhere in this module.
///
/// ⚠ Conservative on purpose: when the bag places part of a record, the guest re-offers the
/// leftover and the hook may take that into the box — this answers as if it could not.
pub fn deliveries_fit(bag: &BagView, orb_box: Option<&OrbBox>, want: &[(u8, u8, u8)]) -> bool {
    if bag.free_cells.is_none() {
        return true;
    }
    let mut sim = bag.clone();
    let mut boxed = orb_box.cloned();
    want.iter().all(|&(cat, idx, n)| {
        let n = u32::from(n);
        if cat == ORB_ITEM_CATEGORY
            && let Some(b) = boxed.as_mut()
            && b.take(idx, n)
        {
            return true;
        }
        place(&mut sim, cat, idx, n) == n
    })
}

/// A snapshot of the player's bag, small enough to copy every few frames: how many cells
/// are free, and what each occupied cell holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BagView {
    /// `None` = the worker has not managed to read the bag (boot, a teardown, a build
    /// whose anchors are unknown). Callers must treat it as "no limit known".
    pub free_cells: Option<u32>,
    /// `(cat, idx, count)` for every occupied cell.
    pub cells: Vec<(u16, u16, u16)>,
}

/// ★★★ HOW THE GUEST ACTUALLY PLACES A `05 04` DELIVERY — and it is NOT "spill until it
/// fits". Modelling it wrongly is how a withdraw silently destroys units.
///
/// `addItemToInventory` (0x131d0) makes ONE placement; the `05 04` handler (0x13cec) looks
/// at the leftover and calls it a SECOND time (0x13d28), then DISCARDS that return. So a
/// delivery gets exactly two placements and whatever is still left is gone.
///
/// And the scan order is merge-first over EVERY tab before any free cell is considered
/// (scanner 0x529a0: merge scan 0x52a32-0x52aa4, then the free scan 0x52aa6-0x52af2). A
/// cell is a merge target iff its count is <= 98 (`0x52a5e/0x52a60 cmp #0x62; bgt`) and its
/// (cat, idx) match — nothing else is read.
///
/// The bite: a bag holding 50 + 50 potions and TEN EMPTY CELLS destroys one unit of a
/// 99-potion delivery. Placement 1 merges into the first 50 (99, leftover 50), placement 2
/// merges into the second 50 (99, leftover 1), and the free cells are never reached. A
/// model that counts total free units would authorise that delivery.
///
/// Returns the units actually DELIVERED, and mutates `bag` as the guest would.
fn place(bag: &mut BagView, cat: u8, idx: u8, n: u32) -> u32 {
    let Some(free) = bag.free_cells.as_mut() else {
        return n;
    };
    if n == 0 {
        return 0;
    }
    if cat <= WH_CAT_EQUIP_MAX {
        // Equipment never merges and its count is forced to 1 (guest 0x13332).
        if *free == 0 {
            return 0;
        }
        *free -= 1;
        bag.cells.push((u16::from(cat), u16::from(idx), 1));
        return 1;
    }
    let matches: Vec<usize> = bag
        .cells
        .iter()
        .enumerate()
        .filter(|(_, (c, i, k))| *c == u16::from(cat) && *i == u16::from(idx) && u32::from(*k) <= 98)
        .map(|(at, _)| at)
        .collect();
    let cap = u32::from(STACK_MAX);

    // Placement one.
    let leftover = match matches.first() {
        Some(&at) => {
            let total = u32::from(bag.cells[at].2) + n;
            bag.cells[at].2 = total.min(cap) as u16;
            total.saturating_sub(cap)
        }
        None => {
            if *free == 0 {
                return 0; // 0x131d0 returns 0xff and NOTHING was placed
            }
            *free -= 1;
            bag.cells.push((u16::from(cat), u16::from(idx), n.min(u32::from(u16::MAX)) as u16));
            return n;
        }
    };
    if leftover == 0 {
        return n;
    }
    // Placement two — and it prefers a merge target over a free cell just like the first,
    // which is why free cells cannot rescue a two-partial-stack bag.
    match matches.get(1) {
        Some(&at) => {
            let total = u32::from(bag.cells[at].2) + leftover;
            bag.cells[at].2 = total.min(cap) as u16;
            n - total.saturating_sub(cap)
        }
        None => {
            if *free == 0 {
                return n - leftover;
            }
            *free -= 1;
            bag.cells.push((u16::from(cat), u16::from(idx), leftover.min(u32::from(u16::MAX)) as u16));
            n
        }
    }
}

/// Will the bag take ALL of `want`, in order, without the client destroying a unit?
///
/// `true` when the snapshot is unknown (see [`BagView::free_cells`]), so a failed read can
/// never refuse something the player already paid for.
///
/// ⛔ Checked as a SEQUENCE, not one kind at a time: each `05 04` changes the bag the next
/// one lands in, and several kinds compete for the same free cells.
pub fn bag_fits(bag: &BagView, want: &[(u8, u8, u8)]) -> bool {
    if bag.free_cells.is_none() {
        return true;
    }
    let mut sim = bag.clone();
    want.iter()
        .all(|(cat, idx, n)| place(&mut sim, *cat, *idx, u32::from(*n)) == u32::from(*n))
}

/// The bag as it will be when a deposit COMMITS: the source cell is already gone.
///
/// ⚠ This matters and the obvious version is wrong. The client clears the bag cell and
/// SAVES before it sends the `05 03` commit (guest 0x13a7a / 0x13a84 / 0x13a90, then the
/// frame is built at 0x13a94), so the bag the remainder lands in has one cell FEWER of the
/// item and one MORE free cell than the snapshot says. Modelling the raw snapshot answers
/// "49 fit" where the truth is 2.
///
/// `None` when the snapshot does not contain the source cell — the caller must then fail
/// closed and owe the bag nothing it cannot model.
fn bag_after_clear(bag: &BagView, rec: &[u8; ITEM_LEN], held: u8) -> Option<BagView> {
    let free = bag.free_cells?;
    let at = bag
        .cells
        .iter()
        .position(|(c, i, n)| *c == u16::from(rec[8]) && *i == u16::from(rec[9]) && *n == u16::from(held))?;
    let mut out = bag.clone();
    out.cells.remove(at);
    out.free_cells = Some(free + 1);
    Some(out)
}

/// The largest single delivery of `(cat, idx)` the bag can take without losing a unit.
///
/// Capped at [`STACK_MAX`], because one `05 04` frame carries one count byte and the client
/// reads it SIGNED (0x135da) — 128 and up would SUBTRACT from the player's stack.
pub fn bag_room(bag: &BagView, cat: u8, idx: u8) -> u32 {
    if bag.free_cells.is_none() {
        return u32::from(STACK_MAX);
    }
    // Monotone in n, so the largest safe n is found by walking down from the cap. At most
    // 99 iterations of a handful of comparisons — this runs once per withdraw, not per frame.
    (0..=u32::from(STACK_MAX))
        .rev()
        .find(|n| bag_fits(bag, &[(cat, idx, *n as u8)]))
        .unwrap_or(0)
}

/// Per-connection state plus the persistent store.
#[derive(Debug, Clone)]
pub struct Engine {
    pub store: Store,
    /// Set by `01 3d`. Offline there is only one account, so this is a formality —
    /// but the client's own sequence depends on the acks, so the state machine has to
    /// be the same shape.
    authed: bool,
    /// Whether this connection has already committed a backup, which selects the
    /// commit-ack wording (`저장 완료` vs `동기화`) exactly as the server does.
    backup_uploaded: bool,
    /// How many items this connection has minted, so two purchases in one session do not
    /// hand the bag two records with the same u64 key.
    ///
    /// ★ The key matters MORE now than it did in the warehouse: the in-game combine uses
    /// it as the item's identity across a bag compaction (`orbcombine::Cell::same_item`).
    /// It is in-memory only — a key needs to be distinct among the items alive at once,
    /// not unique for all time, and the game's own keys are no better than that.
    minted: u32,
    /// The `05 41` deposit whose `05 03` echo has not arrived yet.
    ///
    /// ★ ONE deposit arrives as TWO frames (device wire trace, 40차): `05 41 <rec>`
    /// then a byte-identical `05 03 <rec>`. Storing both is the "똑같은 아이템 2개가
    /// 올라간다" report, so the echo is consumed once and stores nothing.
    deposit_echo: Option<[u8; ITEM_LEN]>,
    /// Units the engine still owes the bag from a PARTIAL deposit, emitted with the
    /// `05 03` commit. See [`Engine::give_back`].
    giveback: Option<[u8; ITEM_LEN]>,
    /// A request the engine is holding because it needs a count the wire cannot carry.
    ask: Option<QtyAsk>,
    /// The payload that is being held, replayed verbatim once the count arrives.
    held: Option<Vec<u8>>,
    /// The count the player chose, consumed by the replayed payload.
    chosen: Option<u8>,
    /// Bytes received but not yet a whole frame.
    inbox: Vec<u8>,
    /// Set whenever the store changed and needs writing back.
    pub dirty: bool,
}

/// A quantity the engine cannot get from the wire and has to ask the player for.
///
/// ── WHY THE ENGINE ASKS, AND WHY IT HOLDS THE REPLY ──────────────────────────
/// 올리기 and 내리기 carry no count. 올리기 sends the WHOLE bag stack (the client builds
/// the record from the instance and then clears the cell outright — `0x139d6` ->
/// `0x4676c`), and 내리기 is just a key. So the number can only come from the host, and
/// the only moment it can be asked for is while the client is already blocked on its own
/// WAIT for our reply. Holding the reply is therefore free: the game is drawing its
/// "기다리는중" box either way, and answering late is indistinguishable from a slow disk.
///
/// ⛔ Every hold MUST be answered. [`Engine::answer_qty`] is the only way out, and
/// `network.rs` puts a deadline on it that answers [`QtyAsk::max`] — the pre-2026-08-25
/// behaviour — so a dialog the player walks away from cannot wedge the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QtyAsk {
    /// The warehouse op that is waiting: [`OP_WH_UPLOAD`] or [`OP_WH_DOWNLOAD`].
    pub op: u8,
    /// Units available — the ceiling of the picker, and the answer a timeout gives.
    pub max: u8,
    /// The FLOOR of the picker, normally 1.
    ///
    /// ★★ Why a deposit can have one (2026-08-25): what the player does NOT deposit comes
    /// back on a `05 04`, through the same two-placement machine that drops whatever it
    /// cannot place. So "keep 49 of a 50-stack" can destroy 47 of them. The floor is the
    /// smallest deposit whose REMAINDER the bag can still take.
    pub min: u8,
    /// `(cat, idx)` of the item, so the UI can name it.
    pub cat: u8,
    pub idx: u8,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new(Store::new())
    }
}

impl Engine {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            authed: false,
            backup_uploaded: false,
            minted: 0,
            deposit_echo: None,
            giveback: None,
            ask: None,
            held: None,
            chosen: None,
            inbox: Vec::new(),
            dirty: false,
        }
    }

    /// The quantity the engine is waiting on, if any. `network.rs` polls this.
    pub fn pending_qty(&self) -> Option<QtyAsk> {
        self.ask
    }

    /// The player answered. Replays the held request with `n` and returns its replies.
    ///
    /// A count of 0, or one over the maximum, is clamped rather than refused: there is a
    /// client blocked on this frame, and the one thing that must not happen is no reply.
    pub fn answer_qty(&mut self, n: u8, env: &Env) -> Vec<Vec<u8>> {
        let Some(ask) = self.ask.take() else {
            return Vec::new();
        };
        let Some(held) = self.held.take() else {
            return Vec::new();
        };
        self.chosen = Some(n.clamp(1, ask.max.max(1)));
        let out = self.handle(&held, env);
        self.chosen = None;
        out
    }

    /// The player pressed CLR: drop the held request and tell the client it was cancelled.
    ///
    /// ★ Player request 2026-08-26 — "수량선택창 떳을떄 CLR버튼 누르면 구매나 올리기 등
    /// 행동이 취소되고 창이 닫혀야돼". This is the SECOND way out of a hold, and it is the
    /// cheap one: nothing has moved yet for either op (올리기 clears the bag cell in its
    /// REPLY handler, on the success arm only), so refusing is a complete undo.
    ///
    /// ⛔ Still an ANSWER. The client is blocked on this frame — see [`QtyAsk`].
    pub fn cancel_qty(&mut self) -> Vec<Vec<u8>> {
        let Some(ask) = self.ask.take() else {
            return Vec::new();
        };
        self.held = None;
        vec![fail(ask.op, CANCELLED)]
    }

    /// Hold `payload` and ask the player for a count. Returns the empty reply the caller
    /// hands back to the guest, which simply keeps waiting.
    fn hold_for_qty(&mut self, payload: &[u8], ask: QtyAsk) -> Vec<Vec<u8>> {
        self.held = Some(payload.to_vec());
        self.ask = Some(ask);
        Vec::new()
    }

    /// `05 04 <u16><u8><20B>` — hand `rec` to the client's bag.
    ///
    /// ★ This is how a PARTIAL deposit gives the remainder back (2026-08-25). The client
    /// clears the whole bag cell when it deposits (`0x139d6` -> `0x4676c`), so keeping
    /// only part of a stack means we owe the rest — and the withdraw reply is exactly the
    /// frame that pays it: `0x13cec` copies the record, calls `addItemToInventory`, feeds
    /// the leftover back through it a second time if the bag stack overflowed 99, saves,
    /// and asks for a fresh list. It shows NO message box (verified: its only calls are
    /// `0x131d0` twice, the `0x55048` redraw and `0x4748` save), so the giveback is
    /// silent — the player just sees the count they chose go up and the rest stay put.
    fn give_back(rec: [u8; ITEM_LEN]) -> Vec<u8> {
        let mut b = vec![0u8, 0u8, 0u8];
        b.extend_from_slice(&rec);
        frame(CAT_WAREHOUSE, OP_WH_DOWNLOAD, &b)
    }

    /// Feed bytes from the guest; get bytes to hand back to it.
    pub fn feed(&mut self, bytes: &[u8], env: &Env) -> Vec<u8> {
        self.inbox.extend_from_slice(bytes);
        let (frames, used) = split_frames(&self.inbox);
        self.inbox.drain(..used);
        let mut out = Vec::new();
        for payload in frames {
            for reply in self.handle(&payload, env) {
                out.extend_from_slice(&reply);
            }
        }
        out
    }

    /// One request in, zero or more replies out.
    pub fn handle(&mut self, payload: &[u8], env: &Env) -> Vec<Vec<u8>> {
        if payload.len() < 2 {
            return Vec::new();
        }
        let (cat, op) = (payload[0], payload[1]);
        let body = &payload[2..];
        match cat {
            // The client sends `00 0a` every five seconds and ignores any answer.
            CAT_HEARTBEAT => Vec::new(),
            CAT_LOGIN => self.login(op, body),
            // The community menu's SMS sub-request. Only the notice finalises a login,
            // so answer it exactly like the other sub-requests (RE wf_e61c4ddc).
            CAT_SMS => self.finish_login(),
            CAT_RECOVERY => self.recovery(op, body),
            CAT_WAREHOUSE => self.warehouse(op, body, env),
            _ => Vec::new(),
        }
    }

    fn login(&mut self, op: u8, body: &[u8]) -> Vec<Vec<u8>> {
        match op {
            // `01 01 <carrier>`: KTF sends 0x02, LGT 0x04. Accept both — an ignored
            // handshake deadlocks the session, which is the warehouse infinite-load of
            // MIGRATION_LGT.md §2.18-(1').
            OP_HELLO if matches!(body.first(), Some(0x02) | Some(0x04)) => {
                vec![frame(CAT_LOGIN, OP_HELLO, &[])]
            }
            OP_AUTH => {
                self.authed = true;
                vec![frame(CAT_LOGIN, OP_AUTH, &[])]
            }
            OP_FINAL => self.finish_login(),
            _ => Vec::new(),
        }
    }

    /// The one reply that clears the client's "서버 응답 기다리는중" WAIT and sets
    /// `NET+0x38 = 0xd` (PROTOCOL.md §3). Body: two ignored bytes, then the notice
    /// text, NUL-terminated.
    fn finish_login(&mut self) -> Vec<Vec<u8>> {
        let mut body = vec![0u8, 0u8];
        body.extend_from_slice(MOTD);
        body.push(0);
        vec![frame(CAT_LOGIN, OP_FINAL, &body)]
    }

    fn recovery(&mut self, op: u8, body: &[u8]) -> Vec<Vec<u8>> {
        if !self.authed {
            return Vec::new();
        }
        match op {
            // `04 3d <slot><219B>` — the client uploading its own character.
            OP_RC_BACKUP if body.len() >= 1 + RECOVERY_BLOB_LEN => {
                let slot = body[0];
                let blob = &body[1..1 + RECOVERY_BLOB_LEN];
                // ★ ONLY a CHANGED blob is worth persisting (device-caught 2026-08-20,
                // and the python server carries the same fast path for the same
                // reason). The client replays this upload on EVERY warehouse and
                // cash-shop page turn, and it BLOCKS on our ack before sending the
                // `05 3d` that redraws the grid — so writing the whole warehouse file
                // here put a disk write and a rename directly on the page-turn latency
                // the player feels: "7,9로 페이지 이동시 화면이 천천히 변경돼".
                // Browsing does not change the character, so those replays are
                // byte-identical; anything that DOES change still takes the write path.
                let changed = self.store.get_backup(slot) != Some(blob);
                if !self.store.put_backup(slot, blob) {
                    return Vec::new();
                }
                self.dirty |= changed;
                let text = if self.backup_uploaded { SYNCED } else { SAVE_DONE };
                self.backup_uploaded = true;
                // `04 3d <slot><status=0><reserved><text\0>` — the commit ack the
                // client answers with `05 3d 00`.
                let mut b = vec![slot, 0x00, 0x00];
                b.extend_from_slice(text);
                b.push(0);
                vec![frame(CAT_RECOVERY, OP_RC_BACKUP, &b)]
            }
            // `04 3e <row>` — 복구, and ONLY 복구. The answer is the refusal ALONE.
            //
            // ★★★ 135차-B, MEASURED — the probe drives the guest into the app's own
            // engine (`wie_probe --net-offline`, every frame logged with the clock):
            //
            //   title 네트워크창고:          01 01 · 01 01 · 01 3d <41 B> · **01 3e**
            //   슬롯 목록 빈 칸 `#` (복구):  01 01 · 01 01 · 01 3d <41 B> · **04 3e <row>**
            //   메인메뉴로 나갔다 재진입:    close_all, new socket, same as the first line
            //   온라인 슬롯 목록의 `#`:      nothing at all
            //
            // So `04 3e` has ONE sender and it is the LOGIN FINALISER of the 복구 dial:
            // it arrives before any notice, and nothing follows it but heartbeats. The
            // arm that used to sit here answered that frame with the NOTICE (on the
            // belief, 67차-G, that the TITLE entry also finalises with `04 3e 00` — it
            // finalises with `01 3e`, above), and the notice is exactly what walks the
            // player INTO the online slot list: 복구 on an empty slot opened the network
            // menu instead of saying the feature is gone, on every slot, since 67차-G
            // (probe: notice → OK → `SLOT 1..3 / OK:선택 CLR:메인메뉴`). The refusal
            // branch behind it — occupancy-gated and post-login only — was never reached.
            //
            // The refusal alone is the whole answer. `1f 00 <text>` releases the WAIT and
            // the LGT cat-0x1f handler runs `netTeardown(0)` before it draws the box, so
            // the player is left on the slot screen they pressed `#` on with the socket
            // closed — what a genuine 복구 dial needs (the next 네트워크 entry re-dials,
            // as the measurement's re-entry line shows). That teardown is what 67차-G was
            // guarding the LOGIN against, and it still is: a login that finalises with
            // `01 3e` never reaches this arm.
            //
            // ⛔ No notice here, before or after the refusal: the notice finalises the
            // login (`NET+0x38 = 0xd`) and a notice + refusal pair lands the player in the
            // online menu on a dead socket — the 67차-G report.
            // ⛔ No occupancy question either (the old `Env::slot_occupied`). A restore is
            // never sent (60차/99차 — the save vault replaced it), so "is that slot
            // occupied" could only choose between the refusal and SILENCE, and silence
            // wedges a client that is waiting on its WAIT box (135차 §2 ①: with the 130차
            // slot window the guest row is not the real slot, so the answer was also
            // asked about the wrong slot).
            //
            // `1f 00 <status=0> <text> 00` — the same bytes 60차 sent.
            OP_RC_RESTORE => {
                let mut b = vec![0u8];
                b.extend_from_slice(NO_RECOVERY);
                b.push(0);
                vec![frame(CAT_SMS, 0x00, &b)]
            }
            _ => Vec::new(),
        }
    }

    fn warehouse(&mut self, op: u8, body: &[u8], env: &Env) -> Vec<Vec<u8>> {
        if !self.authed {
            return Vec::new();
        }
        match op {
            OP_WH_GOLD => vec![frame(CAT_WAREHOUSE, OP_WH_GOLD, &self.store.gold.to_le_bytes())],

            // `05 3d <cursor:u8>` — list refresh AND the recovery-save completion. The
            // one-byte body is a PAGE cursor (builders 0x7898 / 0x77c6); a short body
            // means page 0, which is the completion path.
            OP_WH_SLOT => vec![self.page(u32::from(body.first().copied().unwrap_or(0)))],

            // `05 3f <12B>` — the cash catalog. body[4..8] is a u32 LE PAGE index.
            OP_WH_CASH_INFO => {
                let cursor = if body.len() >= 8 {
                    u32::from_le_bytes([body[4], body[5], body[6], body[7]])
                } else {
                    0
                };
                vec![cash_page(cursor)]
            }

            // `05 42 <u64 amount><u32 id>` — buy. Free: always approved, except the
            // rows with a side effect that can legitimately fail.
            OP_WH_CASH_BUY => self.cash_buy(body, env),

            // `05 40` — the purchase commit. Echoing the request back is what finalises
            // it: the client gates on `body[0] != 0` (`0x13e4c`) and then grants the item.
            //
            // ★ `body[0]` is the ECHOED KEY'S FIRST BYTE — i.e. the item index — not an
            // "amount field" (the older comment here and in the python server both said
            // amount; the builder at `0x13e26` puts `row[0..8]` there). No catalog row has
            // index 0, which is the only reason a verbatim echo reads as success.
            //
            // ★ Neither 오브상자 nor 소켓확장 reaches this arm any more: their `05 42`
            // answers with a message, so the client never sends the commit — see [`notice`]
            // and the arm in [`Engine::cash_buy`].
            OP_WH_GOLD_DOWN => vec![frame(CAT_WAREHOUSE, OP_WH_GOLD_DOWN, body)],

            // `05 41 <20B><slot>` — deposit. The record carries the WHOLE bag stack.
            OP_WH_UPLOAD if body.len() >= ITEM_LEN => {
                let mut rec = [0u8; ITEM_LEN];
                rec.copy_from_slice(&body[..ITEM_LEN]);
                let held = unit_count(&rec);
                // ★ Ask how many, once, for a stack the player might not want all of
                // (player request 2026-08-25). One unit, or a kind that cannot stack,
                // has nothing to choose — those go straight through as before.
                // ★★ WHAT ACTUALLY FITS, not what the player is carrying (2026-08-25).
                // The dialog used to offer the whole bag stack, so a player with a full
                // warehouse and one open 50-stack could pick 50 — and the deposit, being
                // all-or-nothing, then refused the lot. Offering the real ceiling means the
                // number they choose always works. `room_for` counts the space left in
                // matching stacks AND in free cells, which is exactly the "fill to 99 first,
                // then spill" rule expressed as a number.
                let room = self.store.room_for(&rec).min(u32::from(u8::MAX)) as u8;
                if room == 0 {
                    return vec![fail(OP_WH_UPLOAD, WH_FULL)];
                }
                let offer = held.min(room);
                // ★★ …AND A FLOOR. Whatever the player keeps is handed back on a `05 04`,
                // which drops what it cannot place — so keeping 49 of a 50-stack can
                // destroy 47 of them (measured). The floor is the smallest deposit whose
                // remainder the bag can still take, computed against the bag as it will be
                // AFTER the client clears the source cell.
                let back_room = if env.bag.free_cells.is_none() {
                    // ⛔ UNKNOWN IS NOT FULL. A snapshot the worker could not read must never
                    // be the thing that takes the player's choice away — same rule as
                    // `bag_room` and `bag_fits`.
                    u8::MAX
                } else {
                    // A snapshot that does not contain the source cell is STALE (the player
                    // moved something since). Model the raw bag instead of the post-clear
                    // one: that is one free cell short of the truth, i.e. conservative, and
                    // erring toward "deposit more" only ever costs a choice, never an item.
                    let after = bag_after_clear(&env.bag, &rec, held);
                    let model = after.as_ref().unwrap_or(&env.bag);
                    bag_room(model, rec[8], rec[9]).min(u32::from(u8::MAX)) as u8
                };
                let floor = held.saturating_sub(back_room).max(1);
                if floor > offer {
                    // Neither end can be satisfied — and nothing has moved yet.
                    return vec![fail(OP_WH_UPLOAD, WH_FULL)];
                }
                let want = match self.chosen {
                    Some(n) => n,
                    // ★ `held > 1`, not `offer > 1`: when only one unit fits, the player is
                    // about to make a PARTIAL deposit they did not ask for, and a dialog
                    // showing `1` is how they find that out. Skipping it was a silent
                    // partial.
                    None if held > 1 && stacks(&rec) && floor < offer => {
                        let mut whole = vec![CAT_WAREHOUSE, op];
                        whole.extend_from_slice(body);
                        return self.hold_for_qty(
                            &whole,
                            QtyAsk {
                                op: OP_WH_UPLOAD,
                                max: offer,
                                min: floor,
                                cat: rec[8],
                                idx: rec[9],
                            },
                        );
                    }
                    // `floor == offer` leaves nothing to choose — deposit it and say so.
                    None => offer,
                };
                let want = want.clamp(floor, offer);
                // ⛔ The ECHO memo is the record the CLIENT sent, not the split one: the
                // `05 03` that follows is built from the same bag instance and is
                // byte-identical to this frame (40차). Matching on our split copy would
                // miss it and store the deposit twice.
                self.deposit_echo = Some(rec);
                let mut stored = rec;
                stored[ITEM_COUNT_OFF] = want;
                if !self.store.deposit(stored) {
                    return vec![fail(OP_WH_UPLOAD, WH_FULL)];
                }
                self.dirty = true;
                // What the player kept. The client has already been told to clear the
                // whole cell, so this is owed back — paid on the commit below.
                if want < held {
                    let mut back = rec;
                    back[ITEM_COUNT_OFF] = held - want;
                    self.giveback = Some(back);
                }
                vec![frame(CAT_WAREHOUSE, OP_WH_UPLOAD, &[0x01])]
            }

            // `05 03 <20B><slot>` — the echo of the deposit above, or (short body) a
            // list request on the warehouse-open path.
            OP_WH_XFER if body.len() >= ITEM_LEN => {
                let mut rec = [0u8; ITEM_LEN];
                rec.copy_from_slice(&body[..ITEM_LEN]);
                if self.deposit_echo == Some(rec) {
                    self.deposit_echo = None;
                    // The commit is the moment the bag cell is really gone, so it is the
                    // moment to hand back what the player did not deposit.
                    let mut out = Vec::new();
                    if let Some(back) = self.giveback.take() {
                        out.push(Self::give_back(back));
                    }
                    out.push(self.page(0));
                    return out;
                }
                // ⛔ A REFUSAL HAS TO HAND THE ITEM BACK. By the time this arrives the
                // guest has already cleared the bag cell AND saved (0x13cec's sibling
                // path), so answering with a bare page refresh would destroy the item.
                // `05 04` is the same frame a withdraw uses and it grants without a box.
                // ⛔ A remainder still pending here belongs to a deposit whose echo never
                // came (a socket recreated mid-transfer). Leaving it armed would pay it out
                // against THIS record's commit, duplicating units; dropping it is correct
                // because the client never cleared that cell.
                self.giveback = None;
                if self.store.deposit(rec) {
                    self.dirty = true;
                    vec![self.page(0)]
                } else {
                    vec![Self::give_back(rec), self.page(0)]
                }
            }
            OP_WH_XFER => {
                // `05 03` list: header is ceiling u8, count u16 LE (one byte shorter
                // than the `05 3d` refresh — it has no cursor).
                let rows = self.rows(0);
                let (body_rows, n) = wire_rows(&rows);
                let mut b = vec![self.store.pages().min(WH_WIRE_CAP_MAX) as u8];
                b.extend_from_slice(&(n as u16).to_le_bytes());
                b.extend_from_slice(&body_rows);
                vec![frame(CAT_WAREHOUSE, OP_WH_XFER, &b)]
            }

            // `05 04 <u64 key>` withdraw / `05 05 <u64 key>` discard.
            OP_WH_DOWNLOAD | OP_WH_DISCARD if body.len() >= 8 => {
                // ★ A BOX OPENER is not an item: it cannot be taken out or thrown away.
                // The host normally opens the box before the client ever asks (the tap
                // router answers the second tap itself), so this is the keypad / gamepad
                // path — and it must refuse WITH TEXT: a bare status-0 ack makes the
                // client's message box read past the end of the frame.
                if is_opener_key(&body[..8]) {
                    return vec![fail(op, NO_RECOVERY)];
                }
                // ★ 내리기 may take PART of a stack (player request 2026-08-25) — the
                // row keeps the rest. 버리기 is deliberately NOT asked: the player means
                // the cell, and a discard that silently kept nine of ten would be worse
                // than one that took the lot.
                let held = self.store.items.iter().find(|it| it[..8] == body[..8]).map(unit_count).unwrap_or(0);
                // ★★ WHAT THE BAG CAN ACTUALLY TAKE (2026-08-25). The guest DISCARDS a
                // withdraw it has no room for — see [`Env::bag`] — and by then the host has
                // already taken the units out of the warehouse. So the offer is bounded
                // here, and a withdraw with no room at all is refused before anything moves.
                // 버리기 is not bounded: it destroys the row and needs no room anywhere.
                let room = if op == OP_WH_DOWNLOAD {
                    let row = self.store.items.iter().find(|it| it[..8] == body[..8]).copied();
                    match row {
                        Some(row) => bag_room(&env.bag, row[8], row[9]).min(u32::from(u8::MAX)) as u8,
                        None => u8::MAX,
                    }
                } else {
                    u8::MAX
                };
                if op == OP_WH_DOWNLOAD && held > 0 && room == 0 {
                    return vec![fail(OP_WH_DOWNLOAD, BAG_FULL)];
                }
                let offer = held.min(room.max(1));
                let want = match self.chosen {
                    Some(n) => n,
                    None if op == OP_WH_DOWNLOAD && offer > 1 => {
                        let row = *self.store.items.iter().find(|it| it[..8] == body[..8]).expect("held > 0");
                        let mut whole = vec![CAT_WAREHOUSE, op];
                        whole.extend_from_slice(body);
                        return self.hold_for_qty(
                            &whole,
                            QtyAsk {
                                op: OP_WH_DOWNLOAD,
                                max: offer,
                                min: 1,
                                cat: row[8],
                                idx: row[9],
                            },
                        );
                    }
                    None => offer.max(1),
                };
                let rec = match op {
                    OP_WH_DOWNLOAD => self.store.take_units(&body[..8], want.min(offer.max(1))),
                    _ => self.store.take_item(&body[..8]),
                };
                if rec.is_some() {
                    self.dirty = true;
                }
                match (op, rec) {
                    (OP_WH_DOWNLOAD, Some(rec)) => vec![Self::give_back(rec), self.page(0)],
                    (_, rec) => vec![frame(CAT_WAREHOUSE, op, &[u8::from(rec.is_some())]), self.page(0)],
                }
            }

            // `05 3e` — money down. Answer with the balance.
            OP_WH_MONEY if body.len() >= 8 => {
                let mut b = Vec::with_capacity(8);
                b.extend_from_slice(&self.store.gold.to_le_bytes());
                b.extend_from_slice(&self.store.cash.to_le_bytes());
                vec![frame(CAT_WAREHOUSE, OP_WH_MONEY, &b)]
            }

            OP_WH_ORB => self.orb_combine(body),
            _ => Vec::new(),
        }
    }

    /// The rows on `page`, clamped into the pages that exist.
    ///
    /// ★★ The three BOX OPENERS come first (95차-B) — page 0 shows them and then thirteen
    /// of the player's rows, and everything after is shifted by three. They are virtual:
    /// [`Store::items`] does not hold them, so nothing else in the engine sees them.
    /// ★★ And only the SHOWN rows follow (96차): a warehouse full of orbs and materials
    /// looks nearly empty on the grid — that is the design, its contents are in the boxes.
    fn rows(&self, page: u32) -> Vec<[u8; ITEM_LEN]> {
        Self::rows_for(&self.store, page)
    }

    /// [`Self::rows`] against any store — the 136차 정렬 refresh builds a page from the
    /// SHARED store while the engine's own field holds the swapped-out one.
    pub fn rows_for(store: &Store, page: u32) -> Vec<[u8; ITEM_LEN]> {
        let pages = store.pages();
        let page = page.min(pages.saturating_sub(1)) as usize;
        let start = page * WH_MAX_SLOTS;
        // ★ 96차: box-category rows are not on the grid — they are inside the boxes the
        // three openers open (`Store::shown`).
        opener_rows()
            .into_iter()
            .chain(store.shown().copied())
            .skip(start)
            .take(WH_MAX_SLOTS)
            .collect()
    }

    /// One `05 3d` list refresh. Header: cursor u8, ceiling u8, count u16 LE.
    fn page(&self, cursor: u32) -> Vec<u8> {
        Self::page_for(&self.store, cursor)
    }

    /// [`Self::page`] against any store (see [`Self::rows_for`]).
    pub fn page_for(store: &Store, cursor: u32) -> Vec<u8> {
        let pages = store.pages();
        let page = cursor.min(pages.saturating_sub(1));
        let rows = Self::rows_for(store, page);
        let (body_rows, n) = wire_rows(&rows);
        let mut b = vec![page.min(WH_WIRE_CAP_MAX) as u8, pages.min(WH_WIRE_CAP_MAX) as u8];
        b.extend_from_slice(&(n as u16).to_le_bytes());
        b.extend_from_slice(&body_rows);
        frame(CAT_WAREHOUSE, OP_WH_SLOT, &b)
    }

    fn cash_buy(&mut self, body: &[u8], env: &Env) -> Vec<Vec<u8>> {
        // The row's identity: `_cash_item_record` seeds every key as
        // `[idx, tag, count-or-0x22, 0x33, 0, 0, 0, 0]`, and the client echoes the key
        // back — so the key says WHICH ROW was bought, including which stack size.
        let (idx, tag) = (body.first().copied().unwrap_or(0), body.get(1).copied().unwrap_or(0));
        let qty = usize::from(env.cash_qty.max(1));
        if tag == 0x11 {
            match idx {
                // 창고열쇠 — one more bag, i.e. one more page.
                WAREHOUSE_KEY_IDX => {
                    if self.store.bags >= WH_MAX_BAGS {
                        // ★ Refuse with TEXT, not a bare status-0 ack: the client's
                        // status-0 arm opens a message box whose text it reads from
                        // this frame, and a 3-byte ack makes it read past the end.
                        return vec![fail(OP_WH_CASH_BUY, BAG_CAP)];
                    }
                    self.store.bags += 1;
                    self.dirty = true;
                    // ★★ …and tell the client its warehouse just got a page bigger.
                    // The page arrows are gated on a CACHED ceiling byte
                    // (`CHAR+0xa568`, written only by a `05 3d` / `05 03` reply), so
                    // without this refresh the bag the player just paid for is
                    // unreachable until the next online session — the arrow gates at
                    // guest `0x7878` / `0x77b6` read it and simply refuse.
                    //
                    // ★ 창고열쇠 is the LAST purchase that needs this. The other two used
                    // to deliver into the warehouse and needed it as well; since
                    // 2026-08-25 they go to the bag instead and never touch a row.
                    //
                    // ⚠ `CHAR+0xa568` is a CHAR offset. The literal in the disassembly
                    // reads 0x28 lower — every static access on LGT goes through the
                    // thunk at `0x1068`, whose literal is `0x1503ab0` = CHAR + 0x28.
                    return vec![self.page(0), frame(CAT_WAREHOUSE, OP_WH_CASH_BUY, &[0x01])];
                }
                // ★★ 오브상자 / 소켓확장 — DELIVERED INTO THE PLAYER'S BAG (2026-08-25,
                // player decision: "2번으로 인해 싱글가방에서도 진행이 가능하니까 캐시샵에서
                // 구매하면 전부 싱글가방으로 넣고 네트워크 창고로는 넣는 정책은 없애야해").
                //
                // These two used to be deposited into the NETWORK WAREHOUSE, because the
                // warehouse was the only place the client's 오브결합 picker could reach
                // them (44차). 69차 removed that constraint — the combine now runs from
                // the bag — so the warehouse detour is pure friction and is gone.
                //
                // ★ The delivery is a wire frame, not a guest write: `05 04` is the
                // WITHDRAW reply, and its handler `0x13cec` is completely stateless —
                // memset a 20-byte scratch, copy the record out of `body[3..23]`, call
                // `0x131d0` (addItemToInventory), re-call it once with the leftover if the
                // stack overflowed 99, then `bl 0x4748` (saveGame) and ask for a fresh
                // list. It needs no pending request of its own, it opens no message box
                // (device-measured), and this repo already sends it unsolicited for the
                // remainder of a partial deposit. So the whole delivery reuses a path that
                // is already proven on hardware.
                //
                // ⛔ Still NOT an ack. An ack makes the client send `05 40`, whose grant
                // arm hands over the (cat 0x12) SHOP row — the inert 싱글 창고 copy 65차
                // removed. The status-0 notice below is what keeps that from happening.
                ORB_BOX_IDX | SOCKET_EXPAND_IDX => {
                    // One frame per KIND, not per unit: each `05 04` costs the client a
                    // full saveGame, and `0x131d0` stacks up to 99 in one go. 소켓확장 is
                    // always one kind; a bulk 오브상자 rolls several.
                    let mut kinds: Vec<(u8, u8)> = Vec::new();
                    for unit in 0..qty {
                        let orb_idx = if idx == SOCKET_EXPAND_IDX {
                            ORB_IDX_SOCKET_EXPAND
                        } else {
                            // No RNG in a dependency-free module, and none is needed: the
                            // index only has to vary between boxes.
                            (self.minted.wrapping_add(unit as u32) as u8).wrapping_mul(7) % ORB_INDEX_COUNT
                        };
                        match kinds.iter_mut().find(|(k, _)| *k == orb_idx) {
                            Some((_, n)) => *n = n.saturating_add(1),
                            None => kinds.push((orb_idx, 1)),
                        }
                    }
                    // ★★ REFUSE BEFORE TAKING THE MONEY if the bag cannot hold the lot.
                    // The guest DISCARDS what it has no room for (see [`Env::bag`]), so
                    // delivering "as much as fits" would quietly sell the player something
                    // they never receive. Checked as a SET — several kinds of orb compete
                    // for the same free cells.
                    // ★★ 166차: the 오브가방 counts too — a full bag with room in the box is
                    // not a reason to refuse ([`deliveries_fit`]).
                    let want: Vec<(u8, u8, u8)> = kinds.iter().map(|(orb_idx, n)| (ORB_ITEM_CATEGORY, *orb_idx, *n)).collect();
                    if !deliveries_fit(&env.bag, env.orb_box.as_ref(), &want) {
                        return vec![fail(OP_WH_CASH_BUY, BAG_FULL)];
                    }
                    let mut out: Vec<Vec<u8>> = Vec::with_capacity(kinds.len() + 1);
                    for (orb_idx, n) in &kinds {
                        let mut rec = new_orb(*orb_idx, self.minted);
                        // ⚠ Never above the client's own cap: `0x135da` sign-extends this
                        // byte, so 128+ would SUBTRACT from the player's stack.
                        rec[ITEM_COUNT_OFF] = (*n).min(STACK_MAX);
                        self.minted = self.minted.wrapping_add(1);
                        out.push(Self::give_back(rec));
                    }
                    // The purchase message goes LAST so it is the box left standing.
                    out.push(notice(OP_WH_CASH_BUY, &delivered_notice(qty)));
                    return out;
                }
                _ => {}
            }
        }
        vec![frame(CAT_WAREHOUSE, OP_WH_CASH_BUY, &[0x01])]
    }

    /// `05 06 <u64 orb key><u64 equipment key>` — socket an orb into a piece of gear.
    ///
    /// ALWAYS replies: the client's "오브 결합중" WAIT is released by the message this
    /// frame carries and by nothing else, so a silent refusal is an unrecoverable hang.
    fn orb_combine(&mut self, body: &[u8]) -> Vec<Vec<u8>> {
        if body.len() < 16 {
            return vec![orb_result(ORB_FAIL)];
        }
        let (orb_key, equip_key) = (&body[0..8], &body[8..16]);
        // ★★ ONE UNIT EACH, not one ROW each (2026-08-25). See [`ITEM_COUNT_OFF`]: a
        // deposit puts a whole bag stack into a single row, and 결합 spends exactly one
        // orb — the player watched ten go in and one get socketed, and all ten vanished.
        // ⛔ Do not "simplify" these back to `take_item`.
        let orb = self.store.take_one(orb_key);
        let equip = self.store.take_one(equip_key);
        let (Some(orb), Some(equip)) = (orb, equip) else {
            // Put back whatever we did take: a failed combine must not eat an item.
            for rec in [orb, equip].into_iter().flatten() {
                self.store.put_one_back(rec);
            }
            return vec![orb_result(ORB_FAIL)];
        };

        if orb[8] != WH_CAT_ORB || equip[8] > WH_CAT_EQUIP_MAX {
            self.store.put_one_back(orb);
            self.store.put_one_back(equip);
            return vec![orb_result(ORB_BAD_EQUIP)];
        }

        // Record layout: [16] total sockets, [17] refine, [18] filled, [19] socketed
        // orb id. ★ [16]/[18] were the other way round in the RE until a real device
        // record settled it (43차-B).
        let (total, filled) = (equip[16], equip[18]);

        if orb[9] == ORB_IDX_SOCKET_EXPAND {
            if total >= MAX_SOCKETS {
                self.store.put_one_back(orb);
                self.store.put_one_back(equip);
                return vec![orb_result(ORB_SOCKET_CAP)];
            }
            let mut result = equip;
            result[16] = total + 1;
            self.store.put_item(result);
            self.dirty = true;
            return vec![orb_result(ORB_EXPANDED)];
        }

        if total == 0 || filled >= total {
            self.store.put_one_back(orb);
            self.store.put_one_back(equip);
            return vec![orb_result(ORB_NO_ROOM)];
        }

        let mut result = equip;
        result[19] = orb[9];
        result[18] = (filled + 1).min(total);
        self.store.put_item(result);
        self.dirty = true;
        vec![orb_result(ORB_OK)]
    }
}

/// `05 <op> 00 00 <euc-kr text> 00` — a cat-5 refusal that carries its own reason.
///
/// The client's status-0 arm (`0x13ee6`) opens a message box reading from body[2] of
/// THIS frame, so a bare `05 <op> 00` makes it read past the end.
pub fn fail(op: u8, text: &[u8]) -> Vec<u8> {
    let mut b = vec![0x00, 0x00];
    b.extend_from_slice(text);
    b.push(0);
    frame(CAT_WAREHOUSE, op, &b)
}

/// `05 <op> 00 00 <euc-kr text> 00` — the same wire shape as [`fail`], used for news
/// that is GOOD.
///
/// ★★ Status 0 does TWO things, and the second one is the whole reason this exists.
/// The client opens its own message box on the text (`0x13ee6` = `showModal(body+2, 7,
/// 1, 1)`) — and it also stops there. The `05 40` purchase commit is sent ONLY from the
/// `05 42` success arm (`0x13de0` gates on the same `body[0]`), and the grant that puts
/// a copy of the shop row into the player's 싱글 창고 hangs off the `05 40` REPLY
/// (`0x13e4c` → `0x131d0`). So a status-0 answer to `05 42` is not just a message: it is
/// the off switch for the client's own delivery. The 창고열쇠 cap refusal in
/// [`Engine::cash_buy`] has relied on that since 44차 ("no `05 40` commit follows a
/// failed `05 42`") — this is the same lever, used for a success.
pub fn notice(op: u8, text: &[u8]) -> Vec<u8> {
    fail(op, text)
}

/// `창고에 <n>개 넣었습니다.` — where the orbs went, for the two purchases WE hand over.
///
/// The count is in the text on purpose: it is the only place the player can see that
/// the picker's number reached the engine, and the engine is the only party that knows
/// it (the count never travels over the wire — `items::cash_qty_writes`).
pub fn delivered_notice(n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(DELIVERED_HEAD.len() + 3 + DELIVERED_TAIL.len());
    out.extend_from_slice(DELIVERED_HEAD);
    out.extend_from_slice(n.to_string().as_bytes());
    out.extend_from_slice(DELIVERED_TAIL);
    out
}

/// `05 06 00 00 <euc-kr text> 00` — the orb-combine result message.
pub fn orb_result(text: &[u8]) -> Vec<u8> {
    let mut b = vec![0x00, 0x00];
    b.extend_from_slice(text);
    b.push(0);
    frame(CAT_WAREHOUSE, OP_WH_ORB, &b)
}

/// N rows of 21 bytes `{flag=1, 20-byte record}`, capped at 16.
///
/// ⛔ DO NOT PAD TO 16 (46차-B, device-refuted). The client draws N cells and resolves
/// each one's name from its own item DB by the record's (cat, idx) — a zeroed pad row
/// is not a blank, it is item (0,0) = 투신의 머리띠, and the device came back with a
/// warehouse full of them.
pub fn wire_rows(records: &[[u8; ITEM_LEN]]) -> (Vec<u8>, usize) {
    let n = records.len().min(WH_MAX_SLOTS);
    let mut rows = Vec::with_capacity(n * (1 + ITEM_LEN));
    for rec in &records[..n] {
        rows.push(0x01);
        rows.extend_from_slice(rec);
    }
    (rows, n)
}

/// The fixed page windows the cash cursor walks.
///
/// One guard, inherited from the server: a final window of one or two rows is a
/// cursor hazard (its PREV cursor collides with the value a first open uses), so it
/// OVERLAPS the page before it instead. Every row stays reachable.
pub fn cash_windows(total: usize) -> Vec<(usize, usize)> {
    if total == 0 {
        return vec![(0, 0)];
    }
    let mut w: Vec<(usize, usize)> = (0..total)
        .step_by(CASH_PAGE_SIZE)
        .map(|lo| (lo, (lo + CASH_PAGE_SIZE).min(total)))
        .collect();
    let n = w.len();
    if n > 1 && w[n - 1].1 - w[n - 1].0 <= 2 {
        w[n - 1] = (total.saturating_sub(CASH_PAGE_SIZE), total);
    }
    w
}

/// `05 3f` — one page of the cash catalog.
///
/// `A` is the PAGE INDEX and `B` the PAGE COUNT, in the same unit, because the client
/// echoes A back ±1 for next/prev and prints `A+1 / B` in the bottom bar (`sprintf`
/// at 0xfd98). Stateless: the same cursor always gives the same page, so a lost or
/// duplicated request cannot desynchronise the pager.
pub fn cash_page(cursor: u32) -> Vec<u8> {
    let windows = cash_windows(CATALOG.len());
    let page = (cursor as usize).min(windows.len() - 1);
    let (lo, hi) = windows[page];
    let rows: Vec<[u8; ITEM_LEN]> = CATALOG[lo..hi].to_vec();
    let (body_rows, n) = wire_rows(&rows);
    let mut b = Vec::with_capacity(10 + body_rows.len());
    b.extend_from_slice(&(page as u32).to_le_bytes());
    b.extend_from_slice(&(windows.len() as u32).to_le_bytes());
    b.extend_from_slice(&(n as u16).to_le_bytes());
    b.extend_from_slice(&body_rows);
    frame(CAT_WAREHOUSE, OP_WH_CASH_INFO, &b)
}

/// A fresh 20-byte record for `count` units of a STACKABLE `(cat, idx)` — what a host
/// 올리기 out of a bag box files in the warehouse (96차). The shape is [`new_orb`]'s, which
/// the cash shop has delivered into bags on the device since 44차: key, cat, idx, count,
/// a zero, and `0xff` for the equipment-only tail the client ignores on a stackable.
///
/// The key's tag byte is `0xa1` (the engine's own mints are `0xa0`), so a row the host
/// filed can be told apart in the store file; `seq` is masked to the 24 bits the layout
/// has for it. ⛔ Never an opener key: `is_opener_key` wants byte 0 == 0xb0 and a zero
/// tail, and byte 4 here is never zero.
pub fn new_stack(cat: u8, idx: u8, count: u8, seq: u32) -> [u8; ITEM_LEN] {
    let mut rec = [0xffu8; ITEM_LEN];
    let key = 0x0000_01a1_0000_0000u64 | (u64::from(seq & 0x00ff_ffff) << 8) | u64::from(idx);
    rec[..8].copy_from_slice(&key.to_le_bytes());
    rec[ITEM_CAT_OFF] = cat;
    rec[ITEM_IDX_OFF] = idx;
    rec[ITEM_COUNT_OFF] = count.clamp(1, STACK_MAX);
    rec[11] = 0x00;
    rec
}

/// A fresh 20-byte ORB record. `seq` only has to make the key unique within this
/// warehouse — the client treats it as an opaque handle.
pub fn new_orb(idx: u8, seq: u32) -> [u8; ITEM_LEN] {
    let mut rec = [0xffu8; ITEM_LEN];
    let key = 0x0000_01a0_0000_0000u64 | (u64::from(seq) << 8) | u64::from(idx);
    rec[..8].copy_from_slice(&key.to_le_bytes());
    rec[8] = ORB_ITEM_CATEGORY;
    rec[9] = idx;
    rec[10] = 0x01;
    rec[11] = 0x00;
    rec
}
