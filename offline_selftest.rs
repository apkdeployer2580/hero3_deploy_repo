//! Standalone selftest for `wie_android/src/offline.rs` — the in-app private
//! server (network warehouse + cash shop + recovery).
//!
//! `cargo test -p wie_android` dies at startup on this Windows host (linked
//! winit/android deps — see `items_selftest.rs`), so the engine is dependency-free
//! and this file `include!`s **the real source**. Run from the repository root:
//!
//! ```powershell
//! rustc --edition 2024 --test offline_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! The generated catalog is included as a sibling module so `crate::cash_catalog`
//! resolves the same way it does inside `wie_android` — this file IS the crate root
//! here, so the path is identical in both builds.
//!
//! ★ What these tests are FOR: every assertion below that mentions a 차 number is a
//! device-measured constraint that a fresh implementation would otherwise lose. The
//! online server learned each of them the hard way; the offline one must not have to.

mod cash_catalog {
    include!("wie_android/src/cash_catalog.rs");
}

include!("wie_android/src/offline.rs");

#[cfg(test)]
mod tests {
    use super::*;

    /// A logged-in engine with an empty warehouse.
    fn engine() -> Engine {
        let mut e = Engine::default();
        let env = Env::default();
        assert_eq!(e.handle(&[CAT_LOGIN, OP_HELLO, 0x04], &env).len(), 1, "LGT handshake");
        let mut login = vec![CAT_LOGIN, OP_AUTH];
        login.extend_from_slice(&[0xab; 41]);
        assert_eq!(e.handle(&login, &env).len(), 1, "credentials");
        e
    }

    fn payload(f: &[u8]) -> &[u8] {
        &f[4..]
    }

    fn item(key: u64, cat: u8, idx: u8) -> [u8; ITEM_LEN] {
        let mut r = [0xffu8; ITEM_LEN];
        r[..8].copy_from_slice(&key.to_le_bytes());
        r[8] = cat;
        r[9] = idx;
        r[10] = 1;
        r[11] = 0;
        r
    }

    // ── framing ──────────────────────────────────────────────────────────────

    #[test]
    fn a_frame_declares_its_own_total_length_including_the_header() {
        let f = frame(0x05, 0x14, &[1, 2, 3]);
        assert_eq!(u32::from_le_bytes([f[0], f[1], f[2], f[3]]) as usize, f.len());
        assert_eq!(f.len(), 4 + 2 + 3);
        assert_eq!(&f[4..], &[0x05, 0x14, 1, 2, 3]);
    }

    #[test]
    fn split_frames_keeps_a_partial_tail_for_the_next_read() {
        let mut s = frame(0x01, 0x01, &[4]);
        s.extend_from_slice(&frame(0x00, 0x0a, &[]));
        let whole = s.len();
        s.extend_from_slice(&[0x0a, 0x00]); // a partial third frame
        let (frames, used) = split_frames(&s);
        assert_eq!(frames.len(), 2);
        assert_eq!(used, whole, "the partial tail must NOT be consumed");
        assert_eq!(frames[0], vec![0x01, 0x01, 4]);
        assert_eq!(frames[1], vec![0x00, 0x0a]);
    }

    #[test]
    fn a_malformed_length_cannot_spin_the_splitter() {
        // total_len < 6 is unparseable and would loop forever at i += 0.
        for bad in [0u32, 1, 5] {
            let mut s = bad.to_le_bytes().to_vec();
            s.extend_from_slice(&[0x05, 0x14, 0, 0]);
            let (frames, used) = split_frames(&s);
            assert!(frames.is_empty());
            assert_eq!(used, s.len(), "a hopeless stream is discarded, not retried");
        }
    }

    // ── login ────────────────────────────────────────────────────────────────

    #[test]
    fn the_login_sequence_answers_every_step_the_client_waits_on() {
        let mut e = Engine::default();
        let env = Env::default();
        // Both carriers: KTF 0x02, LGT 0x04. An ignored handshake deadlocks the
        // session — the warehouse infinite-load of §2.18-(1').
        for carrier in [0x02u8, 0x04] {
            let mut e = Engine::default();
            let out = e.handle(&[CAT_LOGIN, OP_HELLO, carrier], &env);
            assert_eq!(payload(&out[0]), &[CAT_LOGIN, OP_HELLO], "carrier {carrier:#04x}");
        }
        // An unknown carrier is not answered rather than answered wrongly.
        assert!(e.handle(&[CAT_LOGIN, OP_HELLO, 0x09], &env).is_empty());

        let mut login = vec![CAT_LOGIN, OP_AUTH];
        login.extend_from_slice(&[0u8; 41]);
        assert_eq!(payload(&e.handle(&login, &env)[0]), &[CAT_LOGIN, OP_AUTH]);

        let notice = e.handle(&[CAT_LOGIN, OP_FINAL], &env);
        assert_eq!(notice.len(), 1);
        let p = payload(&notice[0]);
        assert_eq!(&p[..2], &[CAT_LOGIN, OP_FINAL]);
        // ★ The notice body must NOT be empty: the client's paged text-box builder
        // walks a null page list on an empty one and dies at 0x15b2da.
        assert!(p.len() > 4, "an empty notice crashes the client");
        assert_eq!(*p.last().unwrap(), 0, "the text is NUL-terminated");
        assert!(p[4..p.len() - 1].iter().any(|b| *b >= 0x80), "EUC-KR text, not ASCII");
    }

    /// The community menu's SMS sub-request. Only the notice finalises a login, so
    /// answering with a cat-0x1f ack alone makes the client close the socket.
    #[test]
    fn the_sms_sub_request_is_finalised_with_the_notice() {
        let mut e = engine();
        let out = e.handle(&[CAT_SMS, 0x00, b'S', b'M', b'S'], &Env::default());
        assert_eq!(&payload(&out[0])[..2], &[CAT_LOGIN, OP_FINAL]);
    }

    #[test]
    fn nothing_account_scoped_answers_before_the_credentials_land() {
        let mut e = Engine::default();
        let env = Env::default();
        e.handle(&[CAT_LOGIN, OP_HELLO, 0x04], &env);
        assert!(e.handle(&[CAT_WAREHOUSE, OP_WH_GOLD], &env).is_empty());
        assert!(e.handle(&[CAT_RECOVERY, OP_RC_RESTORE, 0], &env).is_empty());
    }

    // ── cash shop ────────────────────────────────────────────────────────────

    #[test]
    fn the_cash_catalog_pages_exactly_as_the_online_server_does() {
        let total = CATALOG.len();
        assert!(total > 16, "the catalog is meant to be several pages");
        let windows = cash_windows(total);
        // Every row reachable, no gaps.
        let mut seen = vec![false; total];
        for (lo, hi) in &windows {
            for i in *lo..*hi {
                seen[i] = true;
            }
        }
        assert!(seen.iter().all(|s| *s), "every catalog row must be reachable");
        for (lo, hi) in &windows {
            assert!(hi - lo <= WH_MAX_SLOTS, "a page over 16 rows overruns the client's array");
        }
        // ★ The short-tail guard: a final page of 1-2 rows is a cursor hazard, so it
        // overlaps the page before it instead.
        for n in [17usize, 18, 33, 34] {
            let w = cash_windows(n);
            let last = w[w.len() - 1];
            assert!(last.1 - last.0 > 2 || w.len() == 1, "n={n} left a hazardous {}-row tail", last.1 - last.0);
        }
    }

    #[test]
    fn a_cash_page_reply_carries_the_page_index_and_the_page_count() {
        let pages = cash_windows(CATALOG.len()).len() as u32;
        for cursor in 0..pages {
            let f = cash_page(cursor);
            let p = payload(&f);
            assert_eq!(&p[..2], &[CAT_WAREHOUSE, OP_WH_CASH_INFO]);
            let a = u32::from_le_bytes([p[2], p[3], p[4], p[5]]);
            let b = u32::from_le_bytes([p[6], p[7], p[8], p[9]]);
            let n = u16::from_le_bytes([p[10], p[11]]) as usize;
            assert_eq!(a, cursor, "A is the page index the client echoes back ±1");
            assert_eq!(b, pages, "B is the page COUNT, the same unit as A");
            assert!(n <= WH_MAX_SLOTS, "N over 16 overruns the client's fixed array");
            assert_eq!(p.len(), 12 + n * 21, "N rows of 21 bytes, no padding");
            // The bar prints "A+1 / B" and the next arrow is lit iff A+1 < B, so the
            // last page must be the one where they agree.
            assert!(a < b);
        }
        // Out of range clamps to the last page rather than answering nothing.
        assert_eq!(cash_page(9999), cash_page(pages - 1));
    }

    /// ★ Stateless paging (46차): the same cursor must always produce the same page,
    /// whatever the client did before. A pager with memory desynchronises the moment
    /// a frame is lost or replayed — and the client replays its whole open sequence
    /// on every page turn.
    #[test]
    fn the_pager_has_no_memory() {
        let a = cash_page(2);
        let _ = cash_page(0);
        let _ = cash_page(5);
        assert_eq!(cash_page(2), a);
    }

    #[test]
    fn every_catalog_row_is_a_whole_item_record() {
        for (i, row) in CATALOG.iter().enumerate() {
            assert_eq!(row.len(), ITEM_LEN);
            assert!(row[8] != 0 || row[9] != 0, "row {i} is item (0,0) = 투신의 머리띠");
        }
    }

    // ── warehouse ────────────────────────────────────────────────────────────

    #[test]
    fn a_deposit_and_its_echo_store_exactly_one_row() {
        // ★ 40차: one deposit arrives as TWO frames — `05 41 <rec>` then a
        // byte-identical `05 03 <rec>`. Storing both is the "똑같은 아이템 2개가
        // 올라간다" report.
        let mut e = engine();
        let env = Env::default();
        let rec = item(0x1122_3344_5566_7788, 0x02, 0x10);
        let mut up = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        up.extend_from_slice(&rec);
        up.push(0x10);
        let ack = e.handle(&up, &env);
        assert_eq!(payload(&ack[0]), &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x01]);
        assert_eq!(e.store.items.len(), 1);

        let mut echo = vec![CAT_WAREHOUSE, OP_WH_XFER];
        echo.extend_from_slice(&rec);
        echo.push(0x10);
        let refresh = e.handle(&echo, &env);
        assert_eq!(&payload(&refresh[0])[..2], &[CAT_WAREHOUSE, OP_WH_SLOT], "the echo answers with a list");
        assert_eq!(e.store.items.len(), 1, "the echo must not store a second copy");

        // A DIFFERENT `05 03` is a real transfer and does store.
        let mut other = vec![CAT_WAREHOUSE, OP_WH_XFER];
        other.extend_from_slice(&item(0x9999, 0x02, 0x11));
        other.push(0x10);
        e.handle(&other, &env);
        assert_eq!(e.store.items.len(), 2);
    }

    #[test]
    fn the_list_header_is_a_cursor_and_a_page_count_and_neither_can_go_negative() {
        // ★ 45차: both header bytes are read as SIGNED chars by the client
        // (`0x138e0`/`0x138e4`), so 0x80+ is negative there and kills the 7/9 keys.
        let mut e = engine();
        let env = Env::default();
        // Eight bags = eight pages, and `WH_MAX_ROWS` (512 since 120차) is the raw row
        // cap `put_item` enforces — far more than the 128 grid cells, so the ceiling
        // comes from the rows here.
        e.store.bags = WH_MAX_BAGS;
        for i in 0..WH_MAX_ROWS {
            assert!(e.store.put_item(item(i as u64 + 1, 0x02, 0x10)));
        }
        let f = e.handle(&[CAT_WAREHOUSE, OP_WH_SLOT, 3], &env);
        let p = payload(&f[0]);
        assert_eq!(&p[..2], &[CAT_WAREHOUSE, OP_WH_SLOT]);
        let (cursor, ceiling) = (p[2], p[3]);
        assert_eq!(cursor, 3, "the one-byte body is a PAGE cursor");
        assert!(ceiling as u32 <= WH_WIRE_CAP_MAX, "ceiling {ceiling} reads as negative on the client");
        assert!(cursor as u32 <= WH_WIRE_CAP_MAX);
        assert!(cursor < ceiling, "the next arrow is gated on cursor + 1 < ceiling");
        let n = u16::from_le_bytes([p[4], p[5]]) as usize;
        assert_eq!(n, WH_MAX_SLOTS);
        assert_eq!(p.len(), 6 + n * 21, "★ N REAL rows, never padded to 16 (46차-B)");
        // Page 3 really is rows 48..64 — minus the three BOX OPENERS that lead page 0
        // (95차-B), so it starts three rows earlier than the item count alone would say.
        assert_eq!(&p[6 + 1..6 + 1 + 8], &(49u64 - BOX_OPENERS.len() as u64).to_le_bytes());
    }

    #[test]
    fn a_cursor_past_the_last_page_clamps_instead_of_answering_nothing() {
        let mut e = engine();
        let env = Env::default();
        e.store.put_item(item(1, 0x02, 0x10));
        let f = e.handle(&[CAT_WAREHOUSE, OP_WH_SLOT, 200], &env);
        let p = payload(&f[0]);
        assert_eq!(p[2], 0, "one page, so the cursor clamps to 0");
        assert_eq!(p[3], 1);
    }

    #[test]
    fn withdrawing_delivers_the_item_and_removes_it() {
        let mut e = engine();
        let env = Env::default();
        let rec = item(0xdead_beef, 0x02, 0x10);
        e.store.put_item(rec);
        let mut req = vec![CAT_WAREHOUSE, OP_WH_DOWNLOAD];
        req.extend_from_slice(&rec[..8]);
        let out = e.handle(&req, &env);
        assert_eq!(out.len(), 2, "the item, then a list refresh");
        let p = payload(&out[0]);
        assert_eq!(&p[..2], &[CAT_WAREHOUSE, OP_WH_DOWNLOAD]);
        assert_eq!(&p[5..5 + ITEM_LEN], &rec, "the 20-byte record follows a 3-byte header");
        assert!(e.store.items.is_empty());

        // A key we do not hold is refused, and still answers (the client is waiting).
        let out = e.handle(&req, &env);
        assert_eq!(payload(&out[0])[2], 0, "status 0 = we did not have it");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn discarding_removes_without_delivering() {
        let mut e = engine();
        let env = Env::default();
        let rec = item(7, 0x02, 0x10);
        e.store.put_item(rec);
        let mut req = vec![CAT_WAREHOUSE, OP_WH_DISCARD];
        req.extend_from_slice(&rec[..8]);
        let out = e.handle(&req, &env);
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_DISCARD, 0x01]);
        assert!(e.store.items.is_empty());
    }

    /// ★ 2026-08-14: a bare `05 41 00` makes the client's status-0 message box read its
    /// text from past the end of the frame.
    ///
    /// ★★ 2026-08-25 — what refuses here is now the player's OWN capacity (`bags` x 16),
    /// not [`WH_MAX_ROWS`]. With `WH_MAX_BAGS` = 8 the largest warehouse anyone can buy is
    /// 128 grid cells (plus, since 120차, three boxes of `WH_BOX_CAP` each), so the row
    /// guard is unreachable through a deposit and stays only as defence in depth around
    /// `put_item`. The wire SHAPE of the refusal is what this test is really for, and that
    /// is unchanged. (Equipment rows — a GRID category — fill the grid here.)
    #[test]
    fn a_full_warehouse_refuses_with_text_rather_than_a_bare_status() {
        let mut e = engine();
        let env = Env::default();
        for i in 0..e.store.capacity() {
            assert!(e.store.put_item(item(i as u64 + 1, 0x02, 0x10)));
        }
        let mut up = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        up.extend_from_slice(&item(0xffff, 0x02, 0x10));
        up.push(0x10);
        let out = e.handle(&up, &env);
        let p = payload(&out[0]);
        assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x00, 0x00], "status 0 = refused");
        assert!(p.len() > 5 && *p.last().unwrap() == 0, "…with its own NUL-terminated text");
        assert_eq!(e.store.items.len(), e.store.capacity(), "and nothing was stored");
    }

    /// ★★ THE PLAYER'S OWN SCENARIO (2026-08-25), both halves:
    ///
    /// > "네트워크 창고에 여유칸이 없고 특정오브가 50개있을때 오브 50개를 넣으려고하면 가방
    /// >  칸없다고 안내뜨고. 49개 넣으려고하면 가능해지면서 50개짜리 네트워크 창고 안에있는
    /// >  오브가 50->99로 변경되는 방식"
    ///
    /// ⛔ Before this, the 50 case SUCCEEDED and grew the store to a 17th row — i.e. the
    /// player was handed a second page they had never bought, and 창고열쇠 bought nothing.
    ///
    /// ★★★ 120차: an orb lives in the warehouse's 오브가방, which has ITS OWN 128 cells
    /// beside the grid — so the "full warehouse" of this scenario is now a full ORB BOX.
    /// The rule inside the room is the same as ever: fill the open stack to 99, refuse
    /// the unit that would need a cell.
    #[test]
    fn a_full_warehouse_takes_what_fits_in_a_stack_and_refuses_the_rest() {
        // One bag. The 오브가방 is full: one open 50-stack and 127 full stacks of another orb.
        let mut full = Store { bags: 1, ..Store::default() };
        assert!(full.put_item(stack(1, 0x10, 0x0f, 50)));
        for i in 2..=WH_BOX_CAP as u64 {
            assert!(full.put_item(stack(i, 0x10, 0x03, STACK_MAX)));
        }
        assert_eq!(full.box_count(0), WH_BOX_CAP, "precondition: the orb box has no free cell");
        assert_eq!(full.free_cells_for(0x10), 0);
        // ★★ 96차: these rows are all of a BOX category, so the grid does not show them at
        // all — they are inside the warehouse's own box. ★★★ 120차: and they cost the BOX
        // its cells, not the grid — the grid is still empty.
        assert_eq!(full.shown_count(), 0, "every row here is boxed");
        assert_eq!(full.pages(), 1, "…so the grid is one page of three openers");
        assert_eq!(full.capacity(), 16, "the grid's own capacity is bags x 16…");
        assert_eq!(full.room_for(&item(0, 0x04, 0x02)), 16, "…and a full orb box leaves every grid cell free");
        assert_eq!(full.room_for(&stack(0, 0x12, 0x01, 1)), (WH_BOX_CAP * 99) as u32, "…and the other boxes untouched");
        // Room is exactly what the open stack can still take.
        assert_eq!(full.room_for(&stack(0, 0x10, 0x0f, 1)), 49);
        assert_eq!(full.room_for(&stack(0, 0x10, 0x05, 1)), 0, "a DIFFERENT orb has nowhere to go");

        // 49 fits — it all merges, and nothing new appears.
        let mut a = full.clone();
        assert!(a.deposit(stack(99, 0x10, 0x0f, 49)));
        assert_eq!(a.items.len(), WH_BOX_CAP, "no new cell");
        assert_eq!(a.pages(), 1, "…and the grid still shows only the three openers");
        assert_eq!(a.capacity(), 16, "⛔ the openers must NOT buy storage");
        assert_eq!(unit_count(&a.items[0]), 99, "50 -> 99");

        // 50 does not — one unit would need a cell that does not exist.
        let mut b = full.clone();
        assert!(!b.deposit(stack(98, 0x10, 0x0f, 50)));
        assert_eq!(b.items.len(), WH_BOX_CAP, "⛔ ALL-OR-NOTHING: a refused deposit changes nothing");
        assert_eq!(unit_count(&b.items[0]), 50, "…the existing stack is untouched");
        assert_eq!(b.capacity(), 16, "…and the player is not handed storage they did not buy");
    }

    /// ★★ The 올리기 dialog must offer only what WILL FIT, so the number the player picks
    /// always works. Before this it offered the whole bag stack and the all-or-nothing
    /// deposit then refused the lot.
    #[test]
    fn the_deposit_dialog_offers_only_what_fits() {
        let mut e = engine();
        e.store.bags = 1;
        // ★ 120차: the room an orb is measured against is the 오브가방's own 128 cells —
        // fill it: one open 50-stack and 127 full stacks of another orb.
        assert!(e.store.put_item(stack(1, 0x10, 0x0f, 50)));
        for i in 2..=WH_BOX_CAP as u64 {
            assert!(e.store.put_item(stack(i, 0x10, 0x03, STACK_MAX)));
        }
        assert_eq!(e.store.free_cells_for(0x10), 0, "precondition: the orb box is full");
        // The player is carrying 80 of the same orb; only 49 can land.
        let mut up = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        up.extend_from_slice(&stack(0x1234, 0x10, 0x0f, 80));
        up.push(0x00);
        let out = e.handle(&up, &Env::default());
        assert!(out.is_empty(), "the request is HELD while the dialog is up");
        let ask = e.pending_qty().expect("a quantity was asked for");
        assert_eq!(ask.max, 49, "…and the ceiling is what fits, not what is carried");
        // Answering with that ceiling merges cleanly and creates no cell.
        let out = e.answer_qty(49, &Env::default());
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x01], "accepted");
        assert_eq!(e.store.items.len(), WH_BOX_CAP, "no new cell");
        assert_eq!(unit_count(&e.store.items[0]), 99, "50 -> 99");

        // With NO room at all the request is refused outright rather than opening a
        // dialog whose only legal answer is zero.
        let mut up = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        up.extend_from_slice(&stack(0x5678, 0x10, 0x05, 10));
        up.push(0x00);
        let out = e.handle(&up, &Env::default());
        assert!(e.pending_qty().is_none(), "no dialog for an impossible deposit");
        let p = payload(&out[0]);
        assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x00, 0x00], "status 0 = refused");
        assert!(p.len() > 5 && *p.last().unwrap() == 0, "…with its own text");
    }

    /// A bag with `free` empty cells and the given `(cat, idx, count)` cells in it.
    fn bag(free: u32, cells: &[(u16, u16, u16)]) -> BagView {
        BagView { free_cells: Some(free), cells: cells.to_vec() }
    }

    /// ★★ THE OTHER HALF OF THE PLAYER'S SCENARIO — the bag side, and the one that was
    /// LOSING ITEMS.
    ///
    /// `addItemToInventory` fills a matching stack to 99 and spills into a free cell, but
    /// with neither it sets the bag-full flag and jumps to its epilogue WITHOUT loading the
    /// leftover into r0 (guest 0x13252-0x1325a -> 0x13810). It returns 0xff, the `05 04`
    /// handler retries once into the same wall (0x13d28) and discards that return — so the
    /// units vanish, after the host has already taken them out of the warehouse.
    #[test]
    fn a_withdraw_is_bounded_by_what_the_bag_can_hold() {
        // Bag: no free cell, one 50-stack of the orb we are withdrawing.
        let full_bag = bag(0, &[(0x10, 0x0f, 50)]);
        assert_eq!(bag_room(&full_bag, 0x10, 0x0f), 49, "only the open stack");
        assert_eq!(bag_room(&full_bag, 0x10, 0x03, ), 0, "a different orb has nowhere to go");

        let mut e = engine();
        assert!(e.store.put_item(stack(0x77, 0x10, 0x0f, 80)));
        let mut down = vec![CAT_WAREHOUSE, OP_WH_DOWNLOAD];
        down.extend_from_slice(&0x77u64.to_le_bytes());
        let env = Env { bag: full_bag.clone(), ..Env::default() };
        let out = e.handle(&down, &env);
        assert!(out.is_empty(), "held for a quantity");
        assert_eq!(e.pending_qty().expect("asked").max, 49, "the ceiling is what the BAG can take");

        // Answer it: 49 leave the warehouse and 49 arrive.
        let out = e.answer_qty(49, &env);
        let rec = payload(&out[0]);
        assert_eq!(rec[..2], [CAT_WAREHOUSE, OP_WH_DOWNLOAD]);
        assert_eq!(rec[5 + ITEM_COUNT_OFF], 49, "exactly what was promised");
        assert_eq!(unit_count(&e.store.items[0]), 80 - 49, "…and the rest stays in the warehouse");
    }

    /// With NO room at all the withdraw is refused before anything moves.
    #[test]
    fn a_withdraw_into_a_full_bag_is_refused_not_swallowed() {
        let mut e = engine();
        assert!(e.store.put_item(stack(0x88, 0x10, 0x03, 5)));
        let mut down = vec![CAT_WAREHOUSE, OP_WH_DOWNLOAD];
        down.extend_from_slice(&0x88u64.to_le_bytes());
        // No free cell, and the only stack is a DIFFERENT orb.
        let env = Env { bag: bag(0, &[(0x10, 0x0f, 50)]), ..Env::default() };
        let out = e.handle(&down, &env);
        let p = payload(&out[0]);
        assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_DOWNLOAD, 0x00, 0x00], "status 0 = refused");
        assert!(p.len() > 5 && *p.last().unwrap() == 0, "…with its own text");
        assert_eq!(e.store.items.len(), 1, "⛔ and the row is untouched");
        assert_eq!(unit_count(&e.store.items[0]), 5);
        // 버리기 needs no room anywhere and must still work on the same bag.
        let mut discard = vec![CAT_WAREHOUSE, OP_WH_DISCARD];
        discard.extend_from_slice(&0x88u64.to_le_bytes());
        let out = e.handle(&discard, &env);
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_DISCARD, 0x01]);
        assert!(e.store.items.is_empty());
    }

    /// An UNKNOWN bag snapshot must never refuse anything — a failed read is not a full bag.
    #[test]
    fn an_unknown_bag_snapshot_bounds_nothing() {
        // 99, not "unlimited": one `05 04` carries ONE count byte, and the client reads it
        // signed (0x135da) so 128+ would SUBTRACT from the player's stack.
        assert_eq!(bag_room(&BagView::default(), 0x10, 0x0f), u32::from(STACK_MAX));
        assert!(bag_fits(&BagView::default(), &[(0x10, 0x0f, 99), (0x10, 0x03, 99)]));
        let mut e = engine();
        assert!(e.store.put_item(stack(0x99, 0x10, 0x0f, 40)));
        let mut down = vec![CAT_WAREHOUSE, OP_WH_DOWNLOAD];
        down.extend_from_slice(&0x99u64.to_le_bytes());
        let out = e.handle(&down, &Env::default());
        assert!(out.is_empty() && e.pending_qty().expect("asked").max == 40, "the whole row is offered");
    }

    /// The cash delivery is refused up front when the bag cannot hold the lot — the guest
    /// discards what it has no room for, so "deliver as much as fits" would sell the player
    /// something they never receive.
    #[test]
    fn a_cash_delivery_the_bag_cannot_hold_is_refused() {
        // 소켓확장 x40 into a bag with no free cell and a 70-stack: only 29 would land.
        let mut e = engine();
        let env = Env { cash_qty: 40, bag: bag(0, &[(0x10, ORB_IDX_SOCKET_EXPAND as u16, 70)]), ..Env::default() };
        let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, SOCKET_EXPAND_IDX, 0x11, 0x22, 0x33];
        b.extend_from_slice(&[0u8; 8]);
        let out = e.handle(&b, &env);
        assert_eq!(out.len(), 1, "one refusal, no grant");
        let p = payload(&out[0]);
        assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00]);
        assert!(p.len() > 5 && *p.last().unwrap() == 0);

        // 29 fits exactly, and lands as one frame.
        let env = Env { cash_qty: 29, ..env };
        let out = e.handle(&b, &env);
        let recs = granted(&out);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0][ITEM_COUNT_OFF], 29);
    }

    /// ★★ 166차 — a FULL bag no longer refuses a 소켓확장 the 오브가방 can take. Each `05 04` is
    /// one whole-record offer on `addItemToInventory`'s entry hook, which takes it into the
    /// box before the bag is looked at — so the bag only has to hold what the box turns away.
    #[test]
    fn a_full_bag_delivers_a_cash_orb_the_orb_box_can_take() {
        let orb = ORB_IDX_SOCKET_EXPAND as usize;
        let full = bag(0, &[(0x05, 0x10, 1)]);
        let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, SOCKET_EXPAND_IDX, 0x11, 0x22, 0x33];
        b.extend_from_slice(&[0u8; 8]);
        // Without the box the full bag refuses, as before.
        let mut e = engine();
        let out = e.handle(&b, &Env { cash_qty: 40, bag: full.clone(), ..Env::default() });
        assert_eq!(&payload(&out[0])[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00]);
        // One free stack in the 오브가방: all 40 land, as one frame.
        let mut merge = vec![Some(0); orb + 1];
        let boxed = OrbBox { free_stacks: 1, merge: merge.clone() };
        let env = Env { cash_qty: 40, bag: full.clone(), orb_box: Some(boxed) };
        let recs = granted(&e.handle(&b, &env));
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0][ITEM_COUNT_OFF], 40);
        // No free stack, but the open stack of this orb takes 30: 30 land, 31 do not.
        merge[orb] = Some(30);
        let tight = Env { cash_qty: 30, bag: full.clone(), orb_box: Some(OrbBox { free_stacks: 0, merge: merge.clone() }) };
        assert_eq!(granted(&e.handle(&b, &tight)).len(), 1);
        let over = Env { cash_qty: 31, ..tight };
        assert_eq!(&payload(&e.handle(&b, &over)[0])[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00]);
        // An orb the hook turns away by itself goes to the bag — which is full.
        merge[orb] = None;
        let declined = Env { cash_qty: 1, bag: full, orb_box: Some(OrbBox { free_stacks: 9, merge }) };
        assert_eq!(&payload(&e.handle(&b, &declined)[0])[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00]);
    }

    /// The box is the hook's arithmetic, delivery by delivery: open stacks first, then whole
    /// stacks, and what one delivery spends the next cannot use.
    #[test]
    fn deliveries_spend_the_orb_box_in_order() {
        assert_eq!(box_intake_cost(0, 1), (1, 98));
        assert_eq!(box_intake_cost(50, 51), (1, 98));
        assert_eq!(box_intake_cost(98, 200), (2, 96));
        assert_eq!(box_intake_cost(7, 0), (0, 7));
        let full = bag(0, &[(0x05, 0x10, 1)]);
        let boxed = OrbBox { free_stacks: 1, merge: vec![Some(10), Some(0), Some(0)] };
        // Orb 0: 10 from its open stack + 89 into the free stack. Orb 1 then has no stack.
        assert!(deliveries_fit(&full, Some(&boxed), &[(ORB_ITEM_CATEGORY, 0, 99)]));
        assert!(!deliveries_fit(&full, Some(&boxed), &[(ORB_ITEM_CATEGORY, 0, 99), (ORB_ITEM_CATEGORY, 1, 1)]));
        // …but a bag with a free cell takes what the box turns away.
        let one_free = bag(1, &[]);
        assert!(deliveries_fit(&one_free, Some(&boxed), &[(ORB_ITEM_CATEGORY, 0, 99), (ORB_ITEM_CATEGORY, 1, 1)]));
        // Not an orb: the box is not asked.
        assert!(!deliveries_fit(&full, Some(&boxed), &[(0x12, 0, 1)]));
        // An unread bag bounds nothing, box or no box (the shipped rule).
        assert!(deliveries_fit(&BagView::default(), None, &[(ORB_ITEM_CATEGORY, 2, 99)]));
        // No box: exactly `bag_fits`.
        assert_eq!(deliveries_fit(&full, None, &[(ORB_ITEM_CATEGORY, 0, 1)]), bag_fits(&full, &[(ORB_ITEM_CATEGORY, 0, 1)]));
    }

    /// ★★★ THE REMAINDER LEG. What the player KEEPS comes back on a `05 04`, through the
    /// same two-placement machine that drops what it cannot place — so "deposit 1 of 50"
    /// can destroy 47 of the 49 kept. The engine now refuses to offer a split whose
    /// remainder will not survive.
    ///
    /// ⚠ And the bag it lands in is the POST-CLEAR one: the client empties the source cell
    /// and saves BEFORE it sends the commit (0x13a7a/0x13a84/0x13a90), so the model has one
    /// cell fewer of the item and one more free cell.
    #[test]
    fn the_deposit_floor_protects_the_remainder() {
        // Bag: the 50-stack being deposited, two 98-stacks of the same potion, no free cell.
        let b = bag(0, &[(0x12, 0x05, 50), (0x12, 0x05, 98), (0x12, 0x05, 98)]);
        let env = Env { bag: b, ..Env::default() };
        let mut e = engine();
        let mut up = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        up.extend_from_slice(&stack(0xaa, 0x12, 0x05, 50));
        up.push(0x00);
        let out = e.handle(&up, &env);
        assert!(out.is_empty(), "held for a count");
        let ask = e.pending_qty().expect("asked");
        // After the clear the bag is [98, 98] + one free cell, which can take 2 back.
        assert_eq!(ask.min, 48, "keep at most 2 — the floor is 50 - 2");
        assert_eq!(ask.max, 50, "the warehouse is empty, so all 50 could go");
        // The engine clamps an out-of-range answer to the floor rather than obeying it.
        let out = e.answer_qty(1, &env);
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x01]);
        assert_eq!(unit_count(&e.store.items[0]), 48, "clamped up to the floor");
    }

    /// ⛔ An UNKNOWN bag snapshot must not invent a floor — a failed read is not a full bag.
    #[test]
    fn an_unknown_bag_snapshot_sets_no_floor() {
        let mut e = engine();
        let mut up = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        up.extend_from_slice(&stack(0xbb, 0x12, 0x05, 50));
        up.push(0x00);
        assert!(e.handle(&up, &Env::default()).is_empty());
        let ask = e.pending_qty().expect("asked");
        assert_eq!((ask.min, ask.max), (1, 50), "the whole range stays available");
    }

    /// ⛔ A refused `05 03` deposit must HAND THE ITEM BACK. By the time it arrives the
    /// guest has already cleared the bag cell and saved, so a bare page refresh destroys it.
    #[test]
    fn a_refused_fallback_deposit_gives_the_item_back() {
        let mut e = engine();
        e.store.bags = 1;
        // ★ 120차: a material's room is the 재료가방 (128 cells of its own) — fill it with
        // full stacks so nothing can merge either.
        for i in 1..=WH_BOX_CAP as u64 {
            assert!(e.store.put_item(stack(i, 0x0e, 0x03, STACK_MAX)));
        }
        assert_eq!(e.store.free_cells_for(0x0e), 0, "precondition: the material box is full");
        // A `05 03` that is not the echo of a `05 41` is the fallback deposit path.
        let rec = stack(0xdead, 0x0e, 0x07, 3);
        let mut x = vec![CAT_WAREHOUSE, OP_WH_XFER];
        x.extend_from_slice(&rec);
        let out = e.handle(&x, &Env::default());
        assert_eq!(e.store.items.len(), WH_BOX_CAP, "no room, so nothing was stored");
        let back = granted(&out);
        assert_eq!(back.len(), 1, "…and the item comes straight back");
        assert_eq!(&back[0][..], &rec[..], "byte for byte");
    }

    /// ★ A row that merges entirely into stacks that already exist needs NO cell, so it
    /// must be accepted even by a store that is somehow over capacity — otherwise
    /// `room_for` offers room `deposit` refuses.
    #[test]
    fn a_pure_merge_needs_no_cell() {
        // The GRID room, over capacity: twenty quest-item stacks in a one-bag (16-cell) store.
        let mut over = Store { bags: 1, ..Store::default() };
        for i in 1..=20u64 {
            assert!(over.put_item(stack(i, 0x11, 0x01, 50)));
        }
        assert!(over.shown_count() > over.capacity(), "precondition: the grid is over capacity");
        assert_eq!(over.free_cells_for(0x11), 0);
        assert!(over.room_for(&stack(0, 0x11, 0x01, 1)) > 0, "merge room is still room");
        assert!(over.deposit(stack(99, 0x11, 0x01, 49)), "a pure merge is always allowed");
        assert_eq!(unit_count(&over.items[0]), 99);
        assert!(!over.deposit(stack(98, 0x11, 0x02, 1)), "…but a NEW grid cell is not");
        assert!(!over.deposit(item(97, 0x04, 0x02)), "…nor a piece of equipment");

        // ★★★ 120차: the same rule per BOX — a 소비가방 over its own cap (an older file, say).
        let mut box_over = Store { bags: 1, ..Store::default() };
        for i in 1..=(WH_BOX_CAP as u64 + 2) {
            assert!(box_over.put_item(stack(i, 0x12, 0x05, 50)));
        }
        assert!(box_over.box_count(2) > WH_BOX_CAP, "precondition: the box is over its cap");
        assert_eq!(box_over.free_cells_for(0x12), 0);
        assert!(box_over.deposit(stack(999, 0x12, 0x05, 49)), "a pure merge into the box is allowed");
        assert_eq!(unit_count(&box_over.items[0]), 99);
        assert!(!box_over.deposit(stack(998, 0x12, 0x06, 1)), "…but a NEW box cell is not");
        // …while the grid and the other boxes are as free as ever — the rooms are apart.
        assert_eq!(box_over.room_for(&item(0, 0x04, 0x02)), 16);
        assert_eq!(box_over.room_for(&stack(0, 0x10, 0x03, 1)), (WH_BOX_CAP * 99) as u32);
    }

    /// ⛔⛔ A stored row may never carry a count the client would read as negative — it is
    /// handed straight back on a withdraw, and 128+ SUBTRACTS from the player's stack.
    #[test]
    fn normalise_clamps_a_damaged_row_to_the_cap() {
        let mut st = Store::default();
        st.items.push(stack(1, 0x12, 0x05, 200));
        st.items.push(stack(2, 0x10, 0x03, 130));
        st.items.push(stack(3, 0x04, 0x02, 1));
        st.normalise();
        assert!(st.items.iter().all(|r| r[ITEM_COUNT_OFF] <= STACK_MAX), "{:?}", st.items);
        assert!(st.items.iter().all(|r| (r[ITEM_COUNT_OFF] as i8) >= 0), "never negative on the wire");
    }

    /// ★★★ THE CASE A "TOTAL FREE UNITS" MODEL GETS WRONG, and it destroys a unit.
    ///
    /// The client makes at most TWO placements — `0x131d0` once, then the `05 04` handler
    /// retries once (0x13d28) and discards THAT return — and the scanner prefers a merge
    /// target over a free cell BOTH times (merge scan 0x52a32-0x52aa4 runs before the free
    /// scan 0x52aa6-0x52af2). So a bag holding 50 + 50 potions and ten EMPTY cells loses a
    /// unit of a 99-potion delivery: 50->99 (leftover 50), 50->99 (leftover 1), gone.
    #[test]
    fn two_partial_stacks_beat_ten_free_cells() {
        let two_halves = bag(10, &[(0x12, 0x05, 50), (0x12, 0x05, 50)]);
        // Naive "free units" arithmetic would say 49 + 49 + 10*99 = 1088 and wave it through.
        assert!(!bag_fits(&two_halves, &[(0x12, 0x05, 99)]), "the free cells are never reached");
        // The real ceiling is 198 - 50 - 50 = 98.
        assert_eq!(bag_room(&two_halves, 0x12, 0x05), 98);
        assert!(bag_fits(&two_halves, &[(0x12, 0x05, 98)]));
        // Two 98-stacks leave room for exactly two more units, whatever else is free.
        let nearly = bag(10, &[(0x12, 0x05, 98), (0x12, 0x05, 98)]);
        assert_eq!(bag_room(&nearly, 0x12, 0x05), 2);
        // ONE partial stack is different: the leftover CAN reach a free cell.
        assert_eq!(bag_room(&bag(1, &[(0x12, 0x05, 50)]), 0x12, 0x05), u32::from(STACK_MAX));
        assert_eq!(bag_room(&bag(0, &[(0x12, 0x05, 50)]), 0x12, 0x05), 49, "…and cannot without one");
        // A FULL stack is not a merge target at all (0x52a60 refuses >= 99), so it is a
        // free-cell question again.
        assert_eq!(bag_room(&bag(1, &[(0x12, 0x05, 99)]), 0x12, 0x05), u32::from(STACK_MAX));
        assert_eq!(bag_room(&bag(0, &[(0x12, 0x05, 99)]), 0x12, 0x05), 0);
    }

    /// ⛔ Several kinds compete for the same free cells, so the fit has to be checked as a
    /// SET — one kind at a time would say yes three times over one free cell.
    #[test]
    fn a_multi_kind_delivery_shares_the_free_cells() {
        let one_cell = bag(1, &[]);
        assert!(bag_fits(&one_cell, &[(0x10, 0x01, 99)]), "one kind fits one cell");
        assert!(!bag_fits(&one_cell, &[(0x10, 0x01, 1), (0x10, 0x02, 1)]), "two kinds need two cells");
        // ⚠ A FRESH cell takes the count byte verbatim with no cap at all
        // (guest 0x135fa-0x1360e), so 100 does land — which is exactly why the wire count
        // is clamped to 99 before it ever gets here (0x135da reads it SIGNED).
        assert!(bag_fits(&one_cell, &[(0x10, 0x01, 100)]), "a fresh cell is uncapped");
        // An open stack absorbs first, so the free cell is still there for the next kind.
        let mixed = bag(1, &[(0x10, 0x01, 50)]);
        assert!(bag_fits(&mixed, &[(0x10, 0x01, 49), (0x10, 0x02, 99)]));
        assert!(!bag_fits(&mixed, &[(0x10, 0x01, 50), (0x10, 0x02, 99)]), "one over and the cell is taken");
        // Equipment never merges, and ONE `05 04` delivers exactly ONE piece — the guest
        // forces the count to 1 (0x13332). So two pieces are two frames, two cells.
        assert!(!bag_fits(&bag(2, &[]), &[(0x04, 0x02, 2)]), "one frame cannot carry two pieces");
        assert!(bag_fits(&bag(2, &[]), &[(0x04, 0x02, 1), (0x04, 0x02, 1)]), "two frames, two cells");
        assert!(!bag_fits(&bag(1, &[]), &[(0x04, 0x02, 1), (0x04, 0x02, 1)]), "…and not one cell");
        assert!(!bag_fits(&bag(0, &[(0x04, 0x02, 1)]), &[(0x04, 0x02, 1)]), "an existing piece is not a stack");
    }

    /// The same rule for every stackable kind — 포션 and 재료 are not special-cased, and
    /// EQUIPMENT still never merges.
    #[test]
    fn the_stack_rule_is_the_same_for_every_stackable_kind() {
        // ★★★ 120차: each box category fills ITS OWN room (`WH_BOX_CAP` cells) — the grid
        // stays empty and has nothing to do with it. The filler is another idx of the same
        // box, as full stacks, so the only room left is the open 50-stack.
        for (cat, idx, filler) in [(0x10u8, 0x0fu8, 0x03u8), (0x12, 0x05, 0x01), (0x0e, 0x03, 0x05), (0x0d, 0x01, 0x02)] {
            let mut st = Store { bags: 1, ..Store::default() };
            assert!(st.put_item(stack(1, cat, idx, 50)));
            for i in 2..=WH_BOX_CAP as u64 {
                assert!(st.put_item(stack(i, cat, filler, STACK_MAX)));
            }
            assert_eq!(st.free_cells_for(cat), 0, "cat {cat:#04x}: the box is full");
            assert_eq!(st.shown_count(), 0, "cat {cat:#04x}: and the grid is empty");
            assert_eq!(st.room_for(&stack(0, cat, idx, 1)), 49, "cat {cat:#04x}");
            let mut a = st.clone();
            assert!(a.deposit(stack(99, cat, idx, 49)), "cat {cat:#04x}: 49 must fit");
            assert_eq!(unit_count(&a.items[0]), 99, "cat {cat:#04x}");
            let mut b = st.clone();
            assert!(!b.deposit(stack(98, cat, idx, 50)), "cat {cat:#04x}: 50 must not");
            assert_eq!(b.items.len(), WH_BOX_CAP, "cat {cat:#04x}");
            assert_eq!(b.room_for(&item(0, 0x04, 0x02)), 16, "cat {cat:#04x}: the grid never noticed");
        }
        // ⛔ Equipment (cat <= 0x0c) never merges: each piece carries its own refine,
        // sockets and orb, so two of them are two cells or nothing.
        let mut eq = Store { bags: 1, ..Store::default() };
        assert!(eq.put_item(stack(1, 0x04, 0x02, 1)));
        assert_eq!(eq.room_for(&stack(0, 0x04, 0x02, 1)), 15, "free CELLS, not units");
        for i in 2..=16u64 {
            assert!(eq.deposit(stack(i, 0x04, 0x02, 1)), "piece {i}");
        }
        assert_eq!(eq.items.len(), 16, "sixteen separate pieces");
        assert!(!eq.deposit(stack(17, 0x04, 0x02, 1)), "and the seventeenth has nowhere to go");
    }

    /// A bulk deposit spills across as many cells as it needs, 99 at a time — and is still
    /// all-or-nothing when the last cell is missing.
    #[test]
    fn a_bulk_deposit_spills_ninety_nine_at_a_time() {
        let mut st = Store { bags: 1, ..Store::default() };
        // 250 units with nothing to merge into needs three cells — of the 소비가방's 128
        // (120차), not the grid's 16.
        assert_eq!(st.room_for(&stack(0, 0x12, 0x05, 1)), (WH_BOX_CAP * 99) as u32);
        assert!(st.deposit(stack(1, 0x12, 0x05, 99)));
        assert!(st.deposit(stack(2, 0x12, 0x05, 99)));
        assert!(st.deposit(stack(3, 0x12, 0x05, 52)));
        assert_eq!(st.items.len(), 3);
        assert_eq!(st.items.iter().map(|r| u32::from(unit_count(r))).sum::<u32>(), 250);
        assert_eq!(unit_count(&st.items[0]), 99);
        assert_eq!(unit_count(&st.items[1]), 99);
        assert_eq!(unit_count(&st.items[2]), 52);
        // Room is now 125 free box cells * 99, plus the 47 still open in the third stack —
        // and the grid's 16 cells are all still free.
        assert_eq!(st.room_for(&stack(0, 0x12, 0x05, 1)), ((WH_BOX_CAP - 3) * 99 + 47) as u32);
        assert_eq!(st.room_for(&item(0, 0x04, 0x02)), 16);
    }


    #[test]
    fn two_deposits_with_the_same_key_both_survive() {
        // ★ 34차: every purchase of one catalog row shares a fixed serial, so
        // replace-on-collision silently destroyed the first copy.
        let mut e = engine();
        let rec = item(0x4242, 0x12, 0x0a);
        assert!(e.store.put_item(rec));
        assert!(e.store.put_item(rec));
        assert_eq!(e.store.items.len(), 2);
        assert_ne!(e.store.items[0][..8], e.store.items[1][..8], "the second was re-keyed");
    }


    /// The 20-byte records a reply hands to the BAG, in order. Each is one `05 04` frame,
    /// whose handler (`0x13cec`) copies `body[3..23]` and calls `addItemToInventory`.
    fn granted(out: &[Vec<u8>]) -> Vec<[u8; ITEM_LEN]> {
        out.iter()
            .map(|f| payload(f))
            .filter(|p| p[..2] == [CAT_WAREHOUSE, OP_WH_DOWNLOAD])
            .map(|p| {
                let mut rec = [0u8; ITEM_LEN];
                rec.copy_from_slice(&p[5..5 + ITEM_LEN]);
                rec
            })
            .collect()
    }

    /// `item()` with an explicit stack count.
    fn stack(key: u64, cat: u8, idx: u8, n: u8) -> [u8; ITEM_LEN] {
        let mut r = item(key, cat, idx);
        r[ITEM_COUNT_OFF] = n;
        r
    }

    // ── cash purchases with a side effect ────────────────────────────────────

    #[test]
    fn buying_a_bag_adds_a_page_and_the_cap_refuses_with_text() {
        let mut e = engine();
        let env = Env::default();
        let buy = |idx: u8| {
            let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, idx, 0x11, 0x22, 0x33];
            b.extend_from_slice(&[0u8; 8]);
            b
        };
        assert_eq!(e.store.pages(), 1);
        for want in 2..=WH_MAX_BAGS {
            let out = e.handle(&buy(WAREHOUSE_KEY_IDX), &env);
            // ★ The list refresh comes FIRST — the page arrows read a cached ceiling, so
            //   without it the bag just paid for is unreachable. See `a_host_delivered_purchase_refreshes_the_list`.
            assert_eq!(out.len(), 2, "list refresh + ack");
            assert_eq!(payload(&out[0])[..2], [CAT_WAREHOUSE, OP_WH_SLOT]);
            assert_eq!(payload(&out[1]), &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x01]);
            assert_eq!(e.store.bags, want);
            assert_eq!(e.store.pages(), want);
            assert_eq!(payload(&out[0])[3], want.min(WH_WIRE_CAP_MAX) as u8, "…and it carries the NEW ceiling");
        }
        let out = e.handle(&buy(WAREHOUSE_KEY_IDX), &env);
        let p = payload(&out[0]);
        assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00], "status 0 = refused");
        assert!(p.len() > 5, "…with the reason in the same frame");
        assert_eq!(e.store.bags, WH_MAX_BAGS, "the cap really is a cap");
        // And eight bags still fit in a signed byte.
        assert!(e.store.pages() <= WH_WIRE_CAP_MAX);
    }

    /// ★★ 2026-08-25 — THE TWO ENGINE-DELIVERED PURCHASES GO TO THE BAG, not the
    /// warehouse (player: "캐시샵에서 구매하면 전부 싱글가방으로 넣고 네트워크 창고로는 넣는
    /// 정책은 없애야해"). They only ever went to the warehouse because that was the one
    /// place the client's 오브결합 picker could reach them; 69차 made the combine work
    /// from the bag, so the detour is gone.
    #[test]
    fn buying_an_orb_box_or_a_socket_expansion_delivers_into_the_bag() {
        let env = Env::default();
        for (idx, want) in [(ORB_BOX_IDX, None), (SOCKET_EXPAND_IDX, Some(ORB_IDX_SOCKET_EXPAND))] {
            let mut e = engine();
            let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, idx, 0x11, 0x22, 0x33];
            b.extend_from_slice(&[0u8; 8]);
            let out = e.handle(&b, &env);
            // ★ A MESSAGE, not an ack: the ack is what makes the client send `05 40` and
            //   grant itself a copy of the (cat 0x12) shop row into the 싱글 창고 (65차).
            let p = payload(out.last().unwrap());
            assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00], "idx {idx:#04x}");
            assert_eq!(&p[4..], &[&delivered_notice(1)[..], &[0u8][..]].concat(), "…naming the count");
            // ⛔ The warehouse is not touched at all any more.
            assert!(e.store.items.is_empty(), "idx {idx:#04x} put something in the warehouse");
            // …and exactly one orb went to the bag.
            let recs = granted(&out);
            assert_eq!(recs.len(), 1, "idx {idx:#04x}");
            assert_eq!(recs[0][8], ORB_ITEM_CATEGORY, "it has to be an ORB for the combine gate");
            assert_eq!(recs[0][10], 1, "one unit");
            if let Some(want) = want {
                assert_eq!(recs[0][9], want);
            } else {
                assert!(recs[0][9] < ORB_INDEX_COUNT);
            }
        }
    }

    /// Two boxes must not mint the same u64 key: the in-game combine uses it as the
    /// item's identity across a bag compaction.
    #[test]
    fn every_delivered_orb_gets_its_own_key() {
        let mut e = engine();
        let mut keys: std::collections::BTreeSet<[u8; 8]> = std::collections::BTreeSet::new();
        let mut n = 0;
        for _ in 0..6 {
            let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, ORB_BOX_IDX, 0x11, 0x22, 0x33];
            b.extend_from_slice(&[0u8; 8]);
            for rec in granted(&e.handle(&b, &Env { cash_qty: 3, ..Env::default() })) {
                let mut k = [0u8; 8];
                k.copy_from_slice(&rec[..8]);
                keys.insert(k);
                n += 1;
            }
        }
        assert_eq!(keys.len(), n, "two deliveries shared a key");
    }

    /// ★ 2026-08-20 — the quantity the PLAYER picked. The catalog carries one row
    /// per item; the count comes from the host's picker, which writes it into the
    /// client's cached row for everything the CLIENT grants — and has to be honoured
    /// here for the two the ENGINE grants, or the player pays for three and gets one.
    #[test]
    fn the_chosen_quantity_is_delivered_for_host_granted_items() {
        for idx in [ORB_BOX_IDX, SOCKET_EXPAND_IDX] {
            for qty in 1..=3u8 {
                let mut e = engine();
                let env = Env { cash_qty: qty, ..Env::default() };
                let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, idx, 0x11, 0x22, 0x33];
                b.extend_from_slice(&[0u8; 8]);
                let out = e.handle(&b, &env);
                let p = payload(out.last().unwrap());
                assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x00, 0x00], "status 0 = no client copy");
                assert_eq!(&p[4..], &[&delivered_notice(qty as usize)[..], &[0u8][..]].concat());
                assert!(e.store.items.is_empty(), "the warehouse is not involved");
                let recs = granted(&out);
                assert!(recs.iter().all(|r| r[8] == ORB_ITEM_CATEGORY));
                // ★ UNITS, not frames. One frame per KIND — each `05 04` costs the client a
                // whole saveGame, and the grant stacks up to 99 in one call.
                let units: u32 = recs.iter().map(|r| u32::from(r[10])).sum();
                assert_eq!(units, u32::from(qty), "idx {idx:#04x} qty {qty}");
                assert!(recs.iter().all(|r| r[10] <= STACK_MAX));
                if idx == SOCKET_EXPAND_IDX {
                    assert_eq!(recs.len(), 1, "one KIND of pseudo-orb is one frame");
                    assert_eq!(recs[0][10], qty);
                }
                let kinds: std::collections::BTreeSet<u8> = recs.iter().map(|r| r[9]).collect();
                assert_eq!(kinds.len(), recs.len(), "one frame per kind, never two for one kind");
            }
        }
    }

    /// A quantity of 0 (or a default-constructed Env) still delivers one — never zero.
    /// A quantity of 0 (or a default-constructed Env) still delivers one — never zero.
    #[test]
    fn a_missing_quantity_still_delivers_one() {
        let mut e = engine();
        let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, ORB_BOX_IDX, 0x11, 0x22, 0x33];
        b.extend_from_slice(&[0u8; 8]);
        let out = e.handle(&b, &Env { cash_qty: 0, ..Env::default() });
        let recs = granted(&out);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0][10], 1);
        assert!(e.store.items.is_empty());
    }

    /// ★★ 2026-08-22 — neither of the engine-delivered pair may be acked.
    ///
    /// Player: "네트워크창고에 99개 생기고 싱글창고에 1개가 생겨." The extra one was the
    /// client granting its own copy of the (cat 0x12) shop row, which it only does after
    /// the `05 40` commit — and it only sends that commit when the `05 42` reply carried a
    /// NONZERO status. So what this test pins is the shape, not the wording: status 0 plus
    /// non-empty NUL-terminated text.
    ///
    /// ⛔ This is only correct while NOTHING hangs a host effect off the bag copy. It was
    /// briefly wrong for 오브상자, whose copy used to trigger `buffs::box_reward`'s premium
    /// buff — suppressing the copy deleted the buff. That buff is gone by the player's own
    /// request ("오브상자는 그냥 오브를 얻는 용도인건데?"), and `buffs_selftest`'s
    /// `the_orb_box_pays_no_host_reward` is what keeps it gone.
    #[test]
    fn the_engine_delivered_pair_is_never_acked() {
        for idx in [ORB_BOX_IDX, SOCKET_EXPAND_IDX] {
            let mut e = engine();
            let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, idx, 0x11, 0x22, 0x33];
            b.extend_from_slice(&[0u8; 8]);
            let out = e.handle(&b, &Env { cash_qty: 7, ..Env::default() });
            // Exactly ONE `05 42` frame — the 구입중 WAIT has one exit. The `05 04` grants
            // that ride along do not release it.
            let replies: Vec<&Vec<u8>> = out.iter().filter(|f| payload(f)[1] == OP_WH_CASH_BUY).collect();
            assert_eq!(replies.len(), 1, "the 구입중 WAIT has exactly one exit");
            let p = payload(replies[0]);
            assert_eq!(p[2], 0x00, "idx {idx:#04x}: a nonzero status IS the client's own delivery");
            assert!(p.len() > 5, "status 0 with no text makes the client read past the frame");
            assert_eq!(*p.last().unwrap(), 0x00, "…and that text is NUL-terminated");
            let units: u32 = granted(&out).iter().map(|r| u32::from(r[10])).sum();
            assert_eq!(units, 7, "ours still arrived, all seven units");
        }
    }

    /// ★★ 2026-08-25 — a host-delivered purchase must REFRESH THE CLIENT'S LIST.
    ///
    /// Player: "캐시샵에서 네트워크 창고로 바로 이동되는 아이템들은 오프라인 네트워크 창고로
    /// 바로 이동 안 되고, 게임시작으로 해당 세이브를 한번 게임실행시켜야 그 뒤에 네트워크
    /// 창고로 아이템이 들어간다."
    ///
    /// The deposit was never the problem — it lands the instant `05 42` arrives. What the
    /// player saw is the client's CACHED grid (16 rows at `CHAR+0xa554`, count `+0xa542`,
    /// page ceiling `+0xa540`), which only a `05 3d` or `05 03` reply refills. The image
    /// has six senders of `05 3d` and not one of them is on the cash path, and the only
    /// unconditional refetch is the online ENTRY chain — whose single `04 3d` sender
    /// (0xa952) is gated on `loadSaveRecord("GameSave_11", slot)` succeeding. Hence
    /// "게임시작 한 번".
    ///
    /// ⛔ The list must come FIRST so the purchase message is the box left standing, and
    /// the reply must stay a status-0 notice (an ack re-opens the 65차 싱글창고 copy).
    /// ★★ 2026-08-25 — 창고열쇠 must REFRESH THE CLIENT'S LIST.
    ///
    /// The client's network-warehouse grid is a CACHE (16 rows at `CHAR+0xa57c`, count
    /// `+0xa56a`, page ceiling `+0xa568`) that only a `05 3d` or `05 03` reply refills, and
    /// the image's six senders of `05 3d` include nothing on the cash path. So a bag the
    /// player just paid for was unreachable — the page ARROWS are gated on that same stale
    /// ceiling — until the next online session.
    ///
    /// ⛔ The two orb purchases deliberately do NOT carry one any more: they go to the BAG
    /// now and never touch the warehouse, and the `05 04` handler asks for its own refresh.
    #[test]
    fn buying_a_bag_refreshes_the_list_and_the_orb_purchases_do_not() {
        let mut e = engine();
        let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, WAREHOUSE_KEY_IDX, 0x11, 0x22, 0x33];
        b.extend_from_slice(&[0u8; 8]);
        let out = e.handle(&b, &Env { cash_qty: 3, ..Env::default() });
        let list = payload(&out[0]);
        assert_eq!(list[..2], [CAT_WAREHOUSE, OP_WH_SLOT], "no list refresh");
        assert_eq!(list[2], 0, "anchored at page 0, like the 05 04 / 05 05 arms");
        assert_eq!(list[3], e.store.pages().min(WH_WIRE_CAP_MAX) as u8, "…carrying the LIVE ceiling");
        assert_eq!(payload(&out[1])[1], OP_WH_CASH_BUY, "the reply comes last");

        for idx in [ORB_BOX_IDX, SOCKET_EXPAND_IDX] {
            let mut e = engine();
            let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, idx, 0x11, 0x22, 0x33];
            b.extend_from_slice(&[0u8; 8]);
            let out = e.handle(&b, &Env { cash_qty: 2, ..Env::default() });
            assert!(
                !out.iter().any(|f| payload(f)[..2] == [CAT_WAREHOUSE, OP_WH_SLOT]),
                "idx {idx:#04x} touches no warehouse row, so it needs no list frame"
            );
        }
    }

    /// ⛔ The `tag == 0x11` gate is load-bearing: SEVEN EQUIPMENT rows in the shipped
    /// catalog key on byte 0 == `0x19`/`0x1a` (cat 0x01/0x02/0x03/0x0b). Deciding on the
    /// index alone would refuse them.
    #[test]
    fn equipment_rows_that_collide_on_the_index_are_still_acked() {
        for (idx, tag) in [(0x19u8, 0x01u8), (0x1a, 0x01), (0x19, 0x02), (0x1a, 0x02), (0x19, 0x03), (0x1a, 0x03), (0x19, 0x0b)] {
            let mut e = engine();
            let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, idx, tag, 0x22, 0x33];
            b.extend_from_slice(&[0u8; 8]);
            let out = e.handle(&b, &Env { cash_qty: 5, ..Env::default() });
            assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x01],
                       "cat {tag:#04x} idx {idx:#04x} is equipment, not the shop pair");
            assert!(e.store.items.is_empty(), "…and the engine delivers nothing for it");
        }
    }

    #[test]
    fn the_delivery_notice_carries_the_count() {
        let want = |d: &[u8]| [DELIVERED_HEAD, d, DELIVERED_TAIL].concat();
        assert_eq!(delivered_notice(1), want(b"1"));
        assert_eq!(delivered_notice(99), want(b"99"));
        // The frame terminates the text with a NUL, so the text may not contain one.
        assert!(!delivered_notice(42).contains(&0));
    }

    #[test]
    fn an_ordinary_purchase_is_approved_and_stores_nothing() {
        let mut e = engine();
        let mut b = vec![CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x0b, 0x11, 0x22, 0x33];
        b.extend_from_slice(&[0u8; 8]);
        let out = e.handle(&b, &Env::default());
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_CASH_BUY, 0x01]);
        assert!(e.store.items.is_empty(), "the client grants the item itself");
    }

    #[test]
    fn the_purchase_commit_is_echoed_so_the_client_finalises_it() {
        // The client gates on payload[0xa] != 0 and then delivers the item.
        let mut e = engine();
        let body: Vec<u8> = (1u8..=13).collect();
        let mut req = vec![CAT_WAREHOUSE, OP_WH_GOLD_DOWN];
        req.extend_from_slice(&body);
        let out = e.handle(&req, &Env::default());
        assert_eq!(payload(&out[0]), req.as_slice(), "the echo must be byte-identical");
    }

    // ── orb combine ──────────────────────────────────────────────────────────

    fn combine(e: &mut Engine, orb: &[u8; ITEM_LEN], equip: &[u8; ITEM_LEN]) -> Vec<u8> {
        let mut b = vec![CAT_WAREHOUSE, OP_WH_ORB];
        b.extend_from_slice(&orb[..8]);
        b.extend_from_slice(&equip[..8]);
        let out = e.handle(&b, &Env::default());
        assert_eq!(out.len(), 1, "★ orb combine must ALWAYS reply — the WAIT has no other exit");
        payload(&out[0]).to_vec()
    }

    fn equipment(key: u64, total: u8, filled: u8) -> [u8; ITEM_LEN] {
        let mut r = item(key, 0x02, 0x10);
        r[16] = total;
        r[17] = 0;
        r[18] = filled;
        r[19] = if filled > 0 { 3 } else { 0xff };
        r
    }

    #[test]
    fn socketing_an_orb_fills_a_hole_and_records_which_orb() {
        let mut e = engine();
        let orb = item(0xa1, WH_CAT_ORB, 5);
        let eq = equipment(0xb1, 3, 0);
        e.store.put_item(orb);
        e.store.put_item(eq);
        let p = combine(&mut e, &orb, &eq);
        assert_eq!(&p[..2], &[CAT_WAREHOUSE, OP_WH_ORB]);
        assert_eq!(&p[4..p.len() - 1], ORB_OK);
        assert_eq!(e.store.items.len(), 1, "the orb is consumed, the equipment stays");
        let out = e.store.items[0];
        assert_eq!(out[18], 1, "filled sockets");
        assert_eq!(out[19], 5, "the socketed orb's index");
        assert_eq!(out[16], 3, "★ [16] is TOTAL sockets and must not move (43차-B)");
    }

    #[test]
    fn a_socket_expansion_adds_a_hole_up_to_the_cap() {
        let mut e = engine();
        let eq = equipment(0xb2, MAX_SOCKETS - 1, 0);
        let orb = item(0xa2, WH_CAT_ORB, ORB_IDX_SOCKET_EXPAND);
        e.store.put_item(orb);
        e.store.put_item(eq);
        let p = combine(&mut e, &orb, &eq);
        assert_eq!(&p[4..p.len() - 1], ORB_EXPANDED);
        assert_eq!(e.store.items[0][16], MAX_SOCKETS);

        // At the cap it refuses AND gives both items back.
        let orb2 = item(0xa3, WH_CAT_ORB, ORB_IDX_SOCKET_EXPAND);
        e.store.put_item(orb2);
        let full = e.store.items[0];
        let p = combine(&mut e, &orb2, &full);
        assert_eq!(&p[4..p.len() - 1], ORB_SOCKET_CAP);
        assert_eq!(e.store.items.len(), 2, "a refusal must not eat either item");
    }

    #[test]
    fn every_combine_refusal_answers_and_gives_the_items_back() {
        let mut e = engine();
        // No room.
        let orb = item(0xa4, WH_CAT_ORB, 1);
        let eq = equipment(0xb4, 1, 1);
        e.store.put_item(orb);
        e.store.put_item(eq);
        let p = combine(&mut e, &orb, &eq);
        assert_eq!(&p[4..p.len() - 1], ORB_NO_ROOM);
        assert_eq!(e.store.items.len(), 2);

        // Wrong kinds: an orb where the equipment should be.
        let orb2 = item(0xa5, WH_CAT_ORB, 1);
        e.store.put_item(orb2);
        let p = combine(&mut e, &orb, &orb2);
        assert_eq!(&p[4..p.len() - 1], ORB_BAD_EQUIP);
        assert_eq!(e.store.items.len(), 3);

        // Keys we do not hold.
        let ghost = item(0xffff_ffff, WH_CAT_ORB, 1);
        let p = combine(&mut e, &ghost, &ghost);
        assert_eq!(&p[4..p.len() - 1], ORB_FAIL);
        assert_eq!(e.store.items.len(), 3);

        // A short body still answers.
        let out = e.handle(&[CAT_WAREHOUSE, OP_WH_ORB, 1, 2, 3], &Env::default());
        assert_eq!(out.len(), 1);
    }

    // ── stacks: one row can be many items ────────────────────────────────────
    //
    // ★★ The 2026-08-25 data loss. A deposit hands over a whole bag STACK in one
    // `05 41`, so a warehouse row carries a count in `[ITEM_COUNT_OFF]`. 오브 결합
    // spends ONE orb — it used to take the row, so ten went in and ten came out of
    // existence when one was socketed ("A오브 10개를 네트워크 창고에 넣고서 그중 1개를
    // 장비에 결합하면 A오브 10개가 전부 사라지는거야").

    #[test]
    fn socketing_one_orb_out_of_a_stack_spends_exactly_one() {
        let mut e = engine();
        let orbs = stack(0xc1, WH_CAT_ORB, 5, 10);
        let eq = equipment(0xd1, 3, 0);
        e.store.put_item(orbs);
        e.store.put_item(eq);
        let p = combine(&mut e, &orbs, &eq);
        assert_eq!(&p[4..p.len() - 1], ORB_OK);
        assert_eq!(e.store.items.len(), 2, "the stack is still a row of its own");
        let left = e.store.items.iter().find(|it| it[..8] == orbs[..8]).expect("the orb stack survives");
        assert_eq!(unit_count(left), 9, "★ nine orbs left, not zero and not ten");
        assert_eq!(left[9], 5, "…and they are still the same orb");
        let out = e.store.items.iter().find(|it| it[8] == 0x02).expect("the equipment");
        assert_eq!((out[18], out[19]), (1, 5), "one hole filled by orb 5");
    }

    #[test]
    fn a_stack_down_to_its_last_unit_leaves_the_warehouse() {
        let mut e = engine();
        let orbs = stack(0xc2, WH_CAT_ORB, 5, 2);
        let eq = equipment(0xd2, 3, 0);
        e.store.put_item(orbs);
        e.store.put_item(eq);
        combine(&mut e, &orbs, &eq);
        assert_eq!(unit_count(&e.store.items.iter().find(|it| it[..8] == orbs[..8]).copied().unwrap()), 1);
        let eq = *e.store.items.iter().find(|it| it[8] == 0x02).unwrap();
        combine(&mut e, &orbs, &eq);
        assert!(!e.store.items.iter().any(|it| it[..8] == orbs[..8]), "the last unit takes the row with it");
        assert_eq!(e.store.items.len(), 1, "only the equipment is left");
    }

    #[test]
    fn a_refused_combine_leaves_a_stack_exactly_as_it_was() {
        let mut e = engine();
        // No room: the equipment's holes are all filled.
        let orbs = stack(0xc3, WH_CAT_ORB, 5, 7);
        let eq = equipment(0xd3, 1, 1);
        e.store.put_item(orbs);
        e.store.put_item(eq);
        let before = e.store.items.clone();
        let p = combine(&mut e, &orbs, &eq);
        assert_eq!(&p[4..p.len() - 1], ORB_NO_ROOM);
        assert_eq!(e.store.items, before, "★ a refusal restores the stack byte for byte");

        // …and so does the one that never finds the equipment at all.
        let ghost = item(0xdead_beef, 0x02, 1);
        let p = combine(&mut e, &orbs, &ghost);
        assert_eq!(&p[4..p.len() - 1], ORB_FAIL);
        assert_eq!(
            unit_count(&e.store.items.iter().find(|it| it[..8] == orbs[..8]).copied().unwrap()),
            7,
            "the orb the engine DID take comes back into its own stack"
        );
        assert_eq!(e.store.items.len(), 2, "and not as a second row");
    }

    #[test]
    fn a_socket_expansion_stack_is_spent_one_at_a_time() {
        let mut e = engine();
        let expanders = stack(0xc4, WH_CAT_ORB, ORB_IDX_SOCKET_EXPAND, 3);
        let eq = equipment(0xd4, 1, 0);
        e.store.put_item(expanders);
        e.store.put_item(eq);
        let p = combine(&mut e, &expanders, &eq);
        assert_eq!(&p[4..p.len() - 1], ORB_EXPANDED);
        assert_eq!(unit_count(&e.store.items.iter().find(|it| it[..8] == expanders[..8]).copied().unwrap()), 2);
        assert_eq!(e.store.items.iter().find(|it| it[8] == 0x02).unwrap()[16], 2, "one more hole");
    }

    #[test]
    fn a_single_row_is_taken_verbatim_so_the_old_behaviour_is_byte_identical() {
        // ⛔ The stack fix must not perturb the overwhelmingly common case. A row of one
        // (every equipment row, every orb the shop delivers) comes out untouched.
        let mut s = Store::new();
        let one = item(0x11, WH_CAT_ORB, 5);
        s.put_item(one);
        assert_eq!(s.take_one(&one[..8]), Some(one));
        assert!(s.items.is_empty());
        // A ZERO count is one unit, not none — the client paints and merges such a row.
        let zero = stack(0x12, WH_CAT_ORB, 5, 0);
        s.put_item(zero);
        assert_eq!(unit_count(&zero), 1);
        assert_eq!(s.take_one(&zero[..8]), Some(zero));
        assert!(s.items.is_empty(), "★ a 0-count row must not become an infinite orb supply");
        // A key we do not hold, and a short key, take nothing.
        assert_eq!(s.take_one(&[0u8; 8]), None);
        assert_eq!(s.take_one(&[0u8; 4]), None);
    }

    #[test]
    fn putting_a_unit_back_merges_into_its_stack_and_saturates() {
        let mut s = Store::new();
        let mut big = item(0x13, WH_CAT_ORB, 5);
        big[ITEM_COUNT_OFF] = 255;
        s.put_item(big);
        let one = s.take_one(&big[..8]).unwrap();
        assert_eq!(unit_count(&s.items[0]), 254);
        assert!(s.put_one_back(one));
        assert_eq!(unit_count(&s.items[0]), 255, "back to where it was");
        // Once more from a full stack: the count may not wrap round to nothing.
        let one = s.take_one(&big[..8]).unwrap();
        s.items[0][ITEM_COUNT_OFF] = 255;
        assert!(s.put_one_back(one));
        assert_eq!(unit_count(&s.items[0]), 255, "★ saturating, never wrapping");
        assert_eq!(s.items.len(), 1);
    }

    // ── stacking: the client's own rule, mirrored ────────────────────────────
    //
    // Player request 2026-08-25: "포션을 5개 올리고 다시 다른 캐릭으로 또 포션 5개 올리면
    // … 포션10개짜리 1개가 되도록", "뇌제의 오브 5개를 … 5개짜리 1칸", "이중 2개만 또 다른
    // 캐릭으로 내리기". The rule is NOT invented — it is `0x529a0` / `0x131d0`.

    fn deposit(e: &mut Engine, rec: &[u8; ITEM_LEN]) -> Vec<Vec<u8>> {
        let mut b = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        b.extend_from_slice(rec);
        b.push(0);
        let out = e.handle(&b, &Env::default());
        // …and the byte-identical `05 03` echo the client always follows with.
        let mut c = vec![CAT_WAREHOUSE, OP_WH_XFER];
        c.extend_from_slice(rec);
        c.push(0);
        let echo = e.handle(&c, &Env::default());
        [out, echo].concat()
    }

    #[test]
    fn two_deposits_of_the_same_stackable_item_become_one_row() {
        let mut e = engine();
        let potion = stack(0xf1, 0x12, 0x07, 5);
        deposit(&mut e, &potion);
        // A second player's deposit: same kind, DIFFERENT key (the client's u64 is a
        // per-instance uid), which is exactly why the merge cannot key on it.
        let more = stack(0xf2, 0x12, 0x07, 5);
        deposit(&mut e, &more);
        assert_eq!(e.store.items.len(), 1, "★ 포션 5 + 포션 5 = 포션 10 in ONE cell");
        assert_eq!(unit_count(&e.store.items[0]), 10);
        assert_eq!(e.store.items[0][..8], potion[..8], "the first row keeps its key");
    }

    #[test]
    fn five_orbs_of_one_kind_become_one_cell_of_five() {
        let mut e = engine();
        for k in 0..5u64 {
            deposit(&mut e, &stack(0xa0 + k, WH_CAT_ORB, 2, 1)); // 뇌제의 오브 x1, five times
        }
        assert_eq!(e.store.items.len(), 1, "★ 1개짜리 5칸이 아니라 5개짜리 1칸");
        assert_eq!(unit_count(&e.store.items[0]), 5);
        // …and combining one of them spends exactly one (67차).
        let eq = equipment(0xb9, 3, 0);
        e.store.put_item(eq);
        let orb_row = e.store.items[0];
        combine(&mut e, &orb_row, &eq);
        let orbs = e.store.items.iter().find(|r| r[8] == WH_CAT_ORB).expect("the orb row");
        assert_eq!(unit_count(orbs), 4, "★ 5 -> 4");
    }

    #[test]
    fn equipment_never_merges_however_identical() {
        let mut e = engine();
        let a = equipment(0xe1, 3, 0);
        let mut b = equipment(0xe2, 3, 0);
        b[9] = a[9]; // same cat AND idx…
        deposit(&mut e, &a);
        deposit(&mut e, &b);
        assert_eq!(e.store.items.len(), 2, "★ 장비는 각자 제련·홈·오브를 들고 있다");
        assert!(!stacks(&a), "cat <= WH_CAT_EQUIP_MAX is the client's own gate (0x529ca)");
    }

    #[test]
    fn a_stack_never_grows_past_the_clients_own_cap() {
        let mut e = engine();
        deposit(&mut e, &stack(0xc9, 0x0e, 0x03, 99));
        deposit(&mut e, &stack(0xca, 0x0e, 0x03, 40));
        let counts: Vec<u8> = e.store.items.iter().map(unit_count).collect();
        assert_eq!(counts, vec![STACK_MAX, 40], "the overflow spills into a new row");
        assert!(counts.iter().all(|c| *c <= STACK_MAX), "★ 99 is the client's cap (0x135e2)");
        assert!(
            counts.iter().all(|c| *c < 0x80),
            "⛔ and the count is SIGNED on the client (0x135da) — 128+ would SUBTRACT"
        );
    }

    #[test]
    fn a_warehouse_written_before_merging_is_folded_on_load() {
        // ★ The device this shipped to held dozens of 소켓확장 in one-unit cells, all
        // written by a build that did not merge. Folding on LOAD is what makes the
        // feature apply to what is already there.
        let mut s = Store::new();
        for k in 0..5u64 {
            s.items.push(stack(0x50 + k, WH_CAT_ORB, ORB_IDX_SOCKET_EXPAND, 1));
        }
        s.items.push(stack(0x60, 0x05, 0x22, 1)); // a sword…
        s.items.push(stack(0x61, 0x05, 0x22, 1)); // …and another just like it
        let text = s.encode();
        let back = Store::decode(&text);
        let orbs: Vec<&[u8; ITEM_LEN]> = back.items.iter().filter(|r| r[8] == WH_CAT_ORB).collect();
        assert_eq!(orbs.len(), 1, "★ five one-unit cells become one");
        assert_eq!(unit_count(orbs[0]), 5);
        assert_eq!(orbs[0][..8], s.items[0][..8], "the first row keeps its key and its place");
        assert_eq!(back.items.iter().filter(|r| r[8] == 0x05).count(), 2, "⛔ …and equipment is left alone");
    }

    #[test]
    fn folding_on_load_respects_the_cap_and_is_idempotent() {
        let mut s = Store::new();
        for k in 0..150u64 {
            s.items.push(stack(0x1000 + k, 0x12, 0x07, 1));
        }
        let once = Store::decode(&s.encode());
        assert_eq!(once.items.iter().map(|r| u32::from(unit_count(r))).sum::<u32>(), 150, "no unit is lost");
        assert_eq!(once.items.iter().map(unit_count).collect::<Vec<_>>(), vec![STACK_MAX, 51]);
        let twice = Store::decode(&once.encode());
        assert_eq!(twice.items, once.items, "★ folding again changes nothing");
    }

    // ── the quantity ask ─────────────────────────────────────────────────────

    #[test]
    fn depositing_a_stack_asks_how_many_and_gives_the_rest_back() {
        let mut e = engine();
        let potion = stack(0xd1, 0x12, 0x07, 5);
        let mut b = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        b.extend_from_slice(&potion);
        b.push(0);
        // The client is blocked on its own WAIT, so holding the reply is free.
        assert!(e.handle(&b, &Env::default()).is_empty(), "no reply until the count arrives");
        let ask = e.pending_qty().expect("the engine asks");
        assert_eq!((ask.op, ask.max, ask.cat, ask.idx), (OP_WH_UPLOAD, 5, 0x12, 0x07));

        let out = e.answer_qty(2, &Env::default());
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x01], "…then the ack");
        assert!(e.pending_qty().is_none());
        assert_eq!(unit_count(&e.store.items[0]), 2, "only what the player chose is stored");

        // The commit echo pays the remainder back into the bag.
        let mut c = vec![CAT_WAREHOUSE, OP_WH_XFER];
        c.extend_from_slice(&potion);
        c.push(0);
        let out = e.handle(&c, &Env::default());
        let p = payload(&out[0]);
        assert_eq!(&p[..2], &[CAT_WAREHOUSE, OP_WH_DOWNLOAD], "★ a 05 04 grant, which 0x13cec applies silently");
        assert_eq!(p[5 + ITEM_COUNT_OFF], 3, "★ the three the player kept");
        assert_eq!(&payload(&out[1])[..2], &[CAT_WAREHOUSE, OP_WH_SLOT], "…and the list refresh");
        assert_eq!(e.store.items.len(), 1, "the echo stored nothing of its own");
    }

    #[test]
    fn withdrawing_can_take_part_of_a_stack() {
        let mut e = engine();
        let potion = stack(0xd2, 0x12, 0x07, 10);
        e.store.put_item(potion);
        let mut b = vec![CAT_WAREHOUSE, OP_WH_DOWNLOAD];
        b.extend_from_slice(&potion[..8]);
        assert!(e.handle(&b, &Env::default()).is_empty(), "held for the count");
        assert_eq!(e.pending_qty().map(|a| a.max), Some(10));

        let out = e.answer_qty(2, &Env::default());
        let p = payload(&out[0]);
        assert_eq!(&p[..2], &[CAT_WAREHOUSE, OP_WH_DOWNLOAD]);
        assert_eq!(p[5 + ITEM_COUNT_OFF], 2, "★ 이중 2개만 내리기");
        assert_eq!(unit_count(&e.store.items[0]), 8, "…and eight stay in the warehouse");
        assert_eq!(e.store.items[0][..8], potion[..8], "the row keeps its key and its place");
    }

    #[test]
    fn a_single_unit_is_never_asked_about_and_neither_is_a_discard() {
        let mut e = engine();
        let one = stack(0xd3, 0x12, 0x07, 1);
        let mut b = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        b.extend_from_slice(&one);
        b.push(0);
        assert!(!e.handle(&b, &Env::default()).is_empty(), "one unit has nothing to choose");
        assert!(e.pending_qty().is_none());

        // 버리기 means the cell, not a unit — asking there would be a trap.
        let many = stack(0xd4, 0x12, 0x07, 9);
        e.store.put_item(many);
        let mut d = vec![CAT_WAREHOUSE, OP_WH_DISCARD];
        d.extend_from_slice(&many[..8]);
        let out = e.handle(&d, &Env::default());
        assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_DISCARD, 0x01]);
        assert!(e.pending_qty().is_none(), "⛔ a discard is never held");
        assert!(!e.store.items.iter().any(|r| r[..8] == many[..8]), "the whole cell goes");
    }

    #[test]
    fn an_out_of_range_answer_is_clamped_rather_than_refused() {
        // ⛔ There is a client blocked on this frame: the one outcome that must never
        // happen is no reply.
        for (asked, want) in [(0u8, 1u8), (200, 6)] {
            let mut e = engine();
            let rec = stack(0xd5, 0x12, 0x07, 6);
            let mut b = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
            b.extend_from_slice(&rec);
            b.push(0);
            e.handle(&b, &Env::default());
            let out = e.answer_qty(asked, &Env::default());
            assert_eq!(payload(&out[0]), &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x01], "asked {asked}");
            assert_eq!(unit_count(&e.store.items[0]), want, "asked {asked}");
        }
    }

    #[test]
    fn answering_when_nothing_is_held_is_a_no_op() {
        let mut e = engine();
        assert!(e.answer_qty(3, &Env::default()).is_empty());
        assert!(e.pending_qty().is_none());
    }

    /// ★ CLR (player request 2026-08-26). The hold is released with a STATUS-0 reply and
    /// the warehouse is untouched — which is a complete undo for 올리기 because the client
    /// only empties the bag cell on the SUCCESS arm of its own reply handler (`0x139d6`).
    #[test]
    fn cancelling_a_deposit_moves_nothing_and_still_answers() {
        let mut e = engine();
        let potion = stack(0xd1, 0x12, 0x07, 5);
        let mut b = vec![CAT_WAREHOUSE, OP_WH_UPLOAD];
        b.extend_from_slice(&potion);
        b.push(0);
        assert!(e.handle(&b, &Env::default()).is_empty());
        assert!(e.pending_qty().is_some());

        let out = e.cancel_qty();
        assert_eq!(out.len(), 1, "⛔ a held request MUST be answered");
        let p = payload(&out[0]);
        assert_eq!(&p[..4], &[CAT_WAREHOUSE, OP_WH_UPLOAD, 0x00, 0x00], "status 0 + text");
        assert!(p[4..].starts_with(CANCELLED), "the client shows the reason");
        assert!(e.pending_qty().is_none(), "the hold is released");
        assert!(e.store.items.is_empty(), "nothing was deposited");
    }

    /// The withdraw half: the row is still in the warehouse afterwards.
    #[test]
    fn cancelling_a_withdraw_leaves_the_row_alone() {
        let mut e = engine();
        e.store.deposit(stack(0xd2, 0x12, 0x07, 9));
        let mut b = vec![CAT_WAREHOUSE, OP_WH_DOWNLOAD];
        b.extend_from_slice(&e.store.items[0][..8]);
        b.push(0);
        assert!(e.handle(&b, &Env::default()).is_empty());
        assert_eq!(e.pending_qty().expect("asked").op, OP_WH_DOWNLOAD);

        let out = e.cancel_qty();
        assert_eq!(payload(&out[0])[..3], [CAT_WAREHOUSE, OP_WH_DOWNLOAD, 0x00]);
        assert_eq!(unit_count(&e.store.items[0]), 9, "the stack is intact");
        assert!(e.pending_qty().is_none());
    }

    #[test]
    fn cancelling_when_nothing_is_held_is_a_no_op() {
        let mut e = engine();
        assert!(e.cancel_qty().is_empty());
    }

    // ── recovery — the data-safety half ──────────────────────────────────────

    #[test]
    fn a_backup_upload_is_stored_and_acked_with_text() {
        let mut e = engine();
        let env = Env::default();
        let blob = vec![0x5au8; RECOVERY_BLOB_LEN];
        let mut up = vec![CAT_RECOVERY, OP_RC_BACKUP, 1];
        up.extend_from_slice(&blob);
        let out = e.handle(&up, &env);
        let p = payload(&out[0]);
        assert_eq!(&p[..5], &[CAT_RECOVERY, OP_RC_BACKUP, 1, 0x00, 0x00]);
        assert_eq!(&p[5..p.len() - 1], SAVE_DONE);
        assert_eq!(e.store.get_backup(1), Some(blob.as_slice()));
        // The client replays the upload on every page turn; the second ack says so.
        assert!(e.dirty, "the first upload is a real change");
        e.dirty = false;
        let out = e.handle(&up, &env);
        assert_eq!(&payload(&out[0])[5..payload(&out[0]).len() - 1], SYNCED);
        // ★ …and an IDENTICAL replay must NOT mark the store dirty. The client blocks
        // on this ack before redrawing the grid, so persisting here puts a disk write
        // on every page turn — the "페이지 이동시 화면이 천천히 변경돼" regression.
        assert!(!e.dirty, "an identical replay must not trigger a write");
        // A CHANGED blob still does.
        let mut up2 = vec![CAT_RECOVERY, OP_RC_BACKUP, 1];
        up2.extend_from_slice(&[0x5bu8; RECOVERY_BLOB_LEN]);
        e.handle(&up2, &env);
        assert!(e.dirty, "a changed character must still be stored");
    }

    /// The refusal the engine answers `04 3e` with: `1f 00 00 <이용할 수 없는 기능입니다.> 00`.
    fn is_no_recovery(frame: &[u8]) -> bool {
        let p = payload(frame);
        p[0] == CAT_SMS && p[2..].windows(NO_RECOVERY.len()).any(|w| w == NO_RECOVERY)
    }

    /// ★★ THE ONE THAT MATTERS. A `04 3e` reply that carries a character is applied by
    /// the client with no occupancy check — it deserialises the 219 bytes into CHAR and
    /// re-serialises the slot. That is the confirmed mechanism behind "창고 들어가려고
    /// 했더니 비어있는 공간이라고 뜨더니 … 스토리, 장비와 아이템 전부 초기화됐습니다" (60차).
    /// 복구 is retired, so NO restore frame may leave the engine — for any row, with or
    /// without a stored backup, before or after the notice.
    #[test]
    fn a_restore_frame_is_never_sent() {
        for finalised_first in [false, true] {
            for row in [0u8, 1, 2, 3, 9, 0xff] {
                let mut e = engine();
                e.store.put_backup(row.min(2), &[0x77u8; RECOVERY_BLOB_LEN]);
                if finalised_first {
                    e.handle(&[CAT_LOGIN, OP_FINAL], &Env::default());
                }
                let out = e.handle(&[CAT_RECOVERY, OP_RC_RESTORE, row], &Env::default());
                assert!(
                    !out.iter().any(|f| payload(f)[..2] == [CAT_RECOVERY, OP_RC_RESTORE]),
                    "a restore frame left the engine for row {row} (notice first: {finalised_first})"
                );
                assert!(
                    out.len() == 1 && is_no_recovery(&out[0]),
                    "row {row} (notice first: {finalised_first}) must get exactly the refusal"
                );
            }
        }
    }

    /// ★★★ 135차-B — THE TWO DIALS, AS MEASURED. `wie_probe --net-offline 1` logged what
    /// the guest actually sends to this engine:
    ///
    ///   title 네트워크창고:          01 01 · 01 01 · 01 3d <41 B> · 01 3e   → notice
    ///   슬롯 목록 빈 칸 `#` (복구):  01 01 · 01 01 · 01 3d <41 B> · 04 3e <row>
    ///
    /// So `04 3e` is the LOGIN FINALISER of the 복구 dial, never a sub-request of a normal
    /// login. Answering it with the notice (67차-G … 135차) walked the player INTO the
    /// online slot list instead of refusing — the probe showed notice → `SLOT 1..3 /
    /// OK:선택`. It must get the refusal ALONE: the cat-0x1f box releases the WAIT and
    /// tears the socket down (`netTeardown(0)`), leaving the player on the slot screen.
    /// ⛔ And the normal finaliser must never get the refusal — that teardown is what
    /// 67차-G was protecting the login from.
    #[test]
    fn the_recovery_dial_gets_the_refusal_alone_and_the_login_finaliser_the_notice_alone() {
        // The normal login: one frame, the notice that clears WAIT, nothing that closes
        // the socket.
        let mut e = engine();
        let out = e.handle(&[CAT_LOGIN, OP_FINAL], &Env::default());
        assert_eq!(out.len(), 1);
        assert_eq!(&payload(&out[0])[..2], &[CAT_LOGIN, OP_FINAL], "01 3e is answered with the notice");
        assert!(!out.iter().any(|f| payload(f)[0] == CAT_SMS), "⛔ no cat-0x1f frame on a normal login — it closes the socket");

        // The 복구 dial, on any row the slot window can put under the cursor.
        for row in 0u8..=2 {
            let mut e = engine();
            let out = e.handle(&[CAT_RECOVERY, OP_RC_RESTORE, row], &Env::default());
            assert_eq!(out.len(), 1, "row {row}: exactly one reply");
            assert!(is_no_recovery(&out[0]), "row {row}: it is 이용할 수 없는 기능입니다.");
            assert!(
                !out.iter().any(|f| payload(f)[..2] == [CAT_LOGIN, OP_FINAL]),
                "⛔ row {row}: no notice — it finalises the login and walks the player into the online menu"
            );
        }
    }

    /// The DELETE half of `#` must be untouched: on an occupied row the key is 초기화,
    /// which is LOCAL (`0xaf56` → modal 11 → `deleteSaveRecord`) and sends nothing, so the
    /// engine can neither see nor block it (and the online slot list's `#` sends nothing
    /// either — measured, 135차-B). Pinned as a property of the whole cat-4 surface: the
    /// only frames a slot screen can produce are the 복구 dial and the backup upload, and
    /// nothing the engine answers to either of them is a delete or a character.
    #[test]
    fn nothing_the_engine_answers_can_reach_the_delete_half_of_hash() {
        let mut e = engine();
        let out = e.handle(&[CAT_RECOVERY, OP_RC_RESTORE, 1], &Env::default());
        assert!(!out.iter().any(|f| payload(f)[0] == CAT_RECOVERY), "no cat-4 frame comes back for a 복구");
        let mut up = vec![CAT_RECOVERY, OP_RC_BACKUP, 1];
        up.extend_from_slice(&[0x42u8; RECOVERY_BLOB_LEN]);
        let out = e.handle(&up, &Env::default());
        assert_eq!(out.len(), 1);
        assert_eq!(&payload(&out[0])[..3], &[CAT_RECOVERY, OP_RC_BACKUP, 1], "a backup upload is only ever acked");
    }

    #[test]
    fn a_backup_for_an_impossible_slot_is_not_stored() {
        let mut e = engine();
        let mut up = vec![CAT_RECOVERY, OP_RC_BACKUP, 7];
        up.extend_from_slice(&[9u8; RECOVERY_BLOB_LEN]);
        assert!(e.handle(&up, &Env::default()).is_empty());
        assert!(e.store.backups.is_empty());
    }

    // ── the store file ───────────────────────────────────────────────────────

    #[test]
    fn the_store_round_trips_through_its_text_form() {
        let mut s = Store::new();
        s.gold = 1_234_567;
        s.cash = 42;
        s.bags = 3;
        s.put_item(item(0x1122_3344, 0x02, 0x10));
        s.put_item(item(0x5566_7788, 0x10, 0x0f));
        s.put_backup(1, &[0xa5; RECOVERY_BLOB_LEN]);
        let text = s.encode();
        let back = Store::decode(&text);
        assert_eq!(back, s);
        assert_eq!(back.encode(), text, "encoding must be stable");
    }

    #[test]
    fn a_corrupt_store_file_degrades_instead_of_refusing_to_open() {
        // Every one of these has to leave a usable warehouse: a player locked out of
        // their own storage has no way to fix it.
        let cases = [
            "",
            "v1\n",
            "garbage",
            "gold\nbags\nitem\n",
            "gold notanumber\nbags 99\n",
            "item zz\nitem 0011\nbackup 1 ff\nbackup x y\n",
            "v1\ngold 5\nitem 001122\n",
        ];
        for c in cases {
            let s = Store::decode(c);
            assert!((1..=WH_MAX_BAGS).contains(&s.bags), "{c:?} gave bags {}", s.bags);
            assert!(s.pages() >= 1);
            assert!(s.items.iter().all(|i| i.len() == ITEM_LEN));
        }
        // A too-large bag count is clamped, not trusted: the wire byte is signed.
        assert_eq!(Store::decode("bags 200").bags, WH_MAX_BAGS);
    }

    #[test]
    fn lines_a_newer_build_wrote_survive_a_downgrade() {
        let text = "v1\ngold 7\nfuture thing here\nbags 2\n";
        let s = Store::decode(text);
        assert_eq!(s.gold, 7);
        assert!(s.encode().contains("future thing here"), "unknown lines must be preserved");
    }

    #[test]
    fn the_store_cannot_grow_past_its_row_cap_from_disk_either() {
        let mut text = String::from("v1\n");
        for i in 0..WH_MAX_ROWS + 50 {
            text.push_str(&format!("item {}\n", hex_of(&item(i as u64 + 1, 2, 3))));
        }
        assert_eq!(Store::decode(&text).items.len(), WH_MAX_ROWS);
    }

    // ── the whole conversation ───────────────────────────────────────────────

    /// The client's real open sequence, fed through `feed` as one byte stream, must
    /// produce a reply for every frame that has a WAIT behind it.
    #[test]
    fn a_full_warehouse_open_sequence_answers_every_wait() {
        let mut e = Engine::default();
        let env = Env::default();
        let mut stream = Vec::new();
        stream.extend_from_slice(&frame(CAT_LOGIN, OP_HELLO, &[0x04]));
        stream.extend_from_slice(&frame(CAT_LOGIN, OP_AUTH, &[0u8; 41]));
        stream.extend_from_slice(&frame(CAT_LOGIN, OP_FINAL, &[]));
        let mut backup = vec![0u8];
        backup.extend_from_slice(&[0x33; RECOVERY_BLOB_LEN]);
        stream.extend_from_slice(&frame(CAT_RECOVERY, OP_RC_BACKUP, &backup));
        stream.extend_from_slice(&frame(CAT_WAREHOUSE, OP_WH_SLOT, &[0]));
        stream.extend_from_slice(&frame(CAT_WAREHOUSE, OP_WH_GOLD, &[]));
        stream.extend_from_slice(&frame(CAT_WAREHOUSE, OP_WH_CASH_INFO, &[0u8; 12]));
        stream.extend_from_slice(&frame(CAT_HEARTBEAT, 0x0a, &[]));

        let out = e.feed(&stream, &env);
        let (frames, used) = split_frames(&out);
        assert_eq!(used, out.len(), "every reply is a whole frame");
        let tags: Vec<(u8, u8)> = frames.iter().map(|f| (f[0], f[1])).collect();
        assert_eq!(
            tags,
            vec![
                (CAT_LOGIN, OP_HELLO),
                (CAT_LOGIN, OP_AUTH),
                (CAT_LOGIN, OP_FINAL),
                (CAT_RECOVERY, OP_RC_BACKUP),
                (CAT_WAREHOUSE, OP_WH_SLOT),
                (CAT_WAREHOUSE, OP_WH_GOLD),
                (CAT_WAREHOUSE, OP_WH_CASH_INFO),
            ],
            "the heartbeat is the only frame that gets no answer"
        );
    }

    /// The same stream delivered one byte at a time must produce the same answers —
    /// a socket hands over whatever it likes, and the guest's sender is not obliged
    /// to align its writes to frames.
    #[test]
    fn a_byte_at_a_time_stream_produces_the_same_replies() {
        let env = Env::default();
        let mut stream = Vec::new();
        stream.extend_from_slice(&frame(CAT_LOGIN, OP_HELLO, &[0x04]));
        stream.extend_from_slice(&frame(CAT_LOGIN, OP_AUTH, &[0u8; 41]));
        stream.extend_from_slice(&frame(CAT_WAREHOUSE, OP_WH_CASH_INFO, &[0u8; 12]));

        let whole = Engine::default().feed(&stream, &env);
        let mut e = Engine::default();
        let mut piecemeal = Vec::new();
        for b in &stream {
            piecemeal.extend_from_slice(&e.feed(&[*b], &env));
        }
        assert_eq!(piecemeal, whole);
    }

    // ── the three BOX OPENERS in the warehouse grid (95차-B, 2026-09-15) ─────

    #[test]
    fn the_openers_are_the_three_bag_item_rows() {
        // ⛔ The same three `(cat, idx)` pairs `bagbox::Kind::bag_item` names. They are
        // literals on both sides on purpose — `offline.rs` carries no `use crate::` —
        // and `bagbox_selftest` pins the other copy.
        assert_eq!(BOX_OPENERS, [(0x12, 0x16), (0x12, 0x17), (0x12, 0x19)]);
        let rows = opener_rows();
        for (i, r) in rows.iter().enumerate() {
            assert_eq!((r[ITEM_CAT_OFF], r[ITEM_IDX_OFF]), BOX_OPENERS[i]);
            // ⛔ Never 0: the grid painter prints that corner number with a bare `%d`
            // and no zero gate (69차-B).
            assert_eq!(r[ITEM_COUNT_OFF], 1);
            assert!(is_opener_key(&r[..8]), "row {i} must be known by its key alone");
        }
        assert_ne!(rows[0][..8], rows[1][..8]);
        assert!(!is_opener_key(&[0u8; 8]), "an all-zero key is not an opener");
        assert!(!is_opener_key(&[0xb0, 3, 0, 0, 0, 0, 0, 0]), "only three exist");
        assert!(!is_opener_key(&[0xb0, 0, 1, 0, 0, 0, 0, 0]), "the tail must be zero");
        assert!(!is_opener_key(&[0xb0, 0, 0, 0, 0, 0, 0]), "a short key is not one");
    }

    #[test]
    fn the_openers_lead_page_zero_and_hide_nothing() {
        let mut e = engine();
        e.store.bags = 1;
        // ⛔ 96차: equipment (cat 0x01), not materials — a box category would not be on
        // the grid at all, and this test is about what the GRID shows.
        for i in 0..20u8 {
            e.store.items.push(item(u64::from(i) + 1, 0x01, i));
        }
        // 20 rows + 3 openers = 23 — two pages of sixteen.
        assert_eq!(e.store.pages(), 2);
        let env = Env::default();
        let mut seen: Vec<u8> = Vec::new();
        for page in 0..2u8 {
            let out = e.handle(&[CAT_WAREHOUSE, OP_WH_SLOT, page], &env);
            let b = payload(&out[0]);
            assert_eq!(b[2], page, "the header echoes the page");
            assert_eq!(b[3], 2, "…and the ceiling both arrows read");
            let n = usize::from(u16::from_le_bytes([b[4], b[5]]));
            // A wire row is `01` + the 20-byte record (`wire_rows`).
            let rows: Vec<&[u8]> = (0..n).map(|i| &b[6 + i * (1 + ITEM_LEN) + 1..6 + (i + 1) * (1 + ITEM_LEN)]).collect();
            if page == 0 {
                assert_eq!(n, WH_MAX_SLOTS);
                for (i, want) in BOX_OPENERS.iter().enumerate() {
                    assert_eq!((rows[i][ITEM_CAT_OFF], rows[i][ITEM_IDX_OFF]), *want, "page 0 leads with the openers");
                }
                assert_eq!(rows[3][ITEM_IDX_OFF], 0, "then the player's own first row");
            } else {
                assert_eq!(n, 7, "13 on page 0 + 7 here = all twenty");
            }
            seen.extend(rows.iter().filter(|r| !is_opener_key(&r[..8])).map(|r| r[ITEM_IDX_OFF]));
        }
        // ⛔ Every row the player owns is still reachable — that is the whole bargain
        // for spending three slots of page 0.
        assert_eq!(seen, (0..20u8).collect::<Vec<_>>());
    }

    // ── the warehouse's own boxes (96차, 2026-09-15) ──────────────────────────

    #[test]
    fn the_box_categories_are_the_ones_the_boxes_take() {
        // ⛔ Must equal the union of `bagbox::Kind::cats` — literals on both sides,
        // because `offline.rs` carries no `use crate::`. `bagbox_selftest` pins the
        // other copy.
        // (125차-B: 조합서 0x0f left the boxes — it is a grid category like equipment now.)
        for cat in [0x0d, 0x0e, 0x10, 0x12] {
            assert!(boxed_category(cat), "cat {cat:#x} belongs to a box");
        }
        // Equipment and quest items stay on the grid.
        for cat in 0x00..=0x0c {
            assert!(!boxed_category(cat), "cat {cat:#x} is equipment");
        }
        assert!(!boxed_category(0x11), "quest items stay on the grid");
        assert!(!boxed_category(0x0f), "recipes stay on the grid (125차-B)");
    }

    #[test]
    fn a_boxed_row_leaves_the_grid_and_takes_no_grid_cell() {
        // ★★ User, 2026-09-15: the warehouse has its own 오브가방·재료가방·소비가방, and
        // their contents are NOT on the grid any more. The rows never move — the boxes
        // are a VIEW (`BAG_TYPES_FEASIBILITY.md §9.1`).
        // ★★★ 120차 (user, 2026-09-20: *"네트워크 창고도 인게임 처럼"*): and a boxed row
        // is charged to its BOX, never to a grid cell — grid + box + box + box, apart.
        let mut e = engine();
        e.store.bags = 1;
        e.store.items.push(item(1, 0x01, 0x00)); // 투구 — on the grid
        e.store.items.push(item(2, 0x10, 0x03)); // 오브 — in the 오브가방
        e.store.items.push(item(3, 0x0e, 0x03)); // 제련석 — in the 재료가방
        e.store.items.push(item(4, 0x12, 0x00)); // 포션 — in the 소비가방

        let env = Env::default();
        let out = e.handle(&[CAT_WAREHOUSE, OP_WH_SLOT, 0], &env);
        let b = payload(&out[0]);
        let n = usize::from(u16::from_le_bytes([b[4], b[5]]));
        let rows: Vec<&[u8]> = (0..n).map(|i| &b[6 + i * (1 + ITEM_LEN) + 1..6 + (i + 1) * (1 + ITEM_LEN)]).collect();
        assert_eq!(n, BOX_OPENERS.len() + 1, "three openers and the one grid row");
        assert_eq!(rows[3][ITEM_CAT_OFF], 0x01, "…and that row is the 투구");

        // ⛔ 96차 charged these three rows to the grid ("otherwise free storage"); 120차
        // reversed that — the grid has spent ONE cell (the 투구), each box one of its own.
        assert_eq!(e.store.items.len(), 4);
        assert_eq!(e.store.shown_count(), 1);
        assert_eq!(e.store.capacity(), 16, "the grid's capacity is bags x 16");
        assert_eq!(e.store.room_for(&item(9, 0x01, 0x01)), 15, "16 grid cells less the ONE the grid shows");
        for bx in 0..WH_BOXES {
            assert_eq!(e.store.box_count(bx), 1, "box {bx} holds its one row");
        }
        assert_eq!(e.store.free_cells_for(0x10), WH_BOX_CAP - 1);
        assert_eq!(e.store.free_cells_for(0x0f), 15, "125차-B: a recipe is a grid item and competes for the grid cells");
        assert_eq!(e.store.free_cells_for(0x0d), WH_BOX_CAP - 1, "0x0d shares the 소비가방 with 0x12");

        // The boxes themselves: one stack each, by category.
        assert_eq!(e.store.boxed_rows(&[0x10]).len(), 1);
        assert_eq!(e.store.boxed_rows(&[0x0e, 0x0f]).len(), 1);
        assert_eq!(e.store.boxed_rows(&[0x12, 0x0d]).len(), 1);
        assert_eq!(e.store.boxed_rows(&[0x10])[0][ITEM_IDX_OFF], 0x03);
        // ⛔ And a box view never contains an opener — those are not in `items` at all.
        assert!(e.store.boxed_rows(&[0x12, 0x0d]).iter().all(|r| !is_opener_key(&r[..8])));
    }

    #[test]
    fn a_warehouse_of_nothing_but_boxed_rows_still_pages_its_openers() {
        // 200 boxed rows and the grid is one page of three openers — the design the user
        // was warned about ("오브·재료로 채운 창고는 격자가 거의 비어 보인다").
        let mut e = engine();
        e.store.bags = 8;
        for i in 0..50u64 {
            e.store.items.push(item(i + 1, 0x10, (i % 15) as u8));
        }
        assert_eq!(e.store.shown_count(), 0);
        // `bags` is still the floor, so the player can walk the pages they bought.
        assert_eq!(e.store.pages(), 8);
        let env = Env::default();
        let out = e.handle(&[CAT_WAREHOUSE, OP_WH_SLOT, 0], &env);
        let b = payload(&out[0]);
        assert_eq!(usize::from(u16::from_le_bytes([b[4], b[5]])), BOX_OPENERS.len());
        // ★★★ 120차: an empty-looking grid IS empty — all 128 grid cells are free, and the
        // orb box has spent 50 of its own 128.
        assert_eq!(e.store.room_for(&item(0, 0x04, 0x02)), 128);
        assert_eq!(e.store.free_cells_for(0x10), WH_BOX_CAP - 50);
    }

    /// ★★★ 120차 (user, 2026-09-20): *"네트워크 창고도 인게임 처럼 진행되어야해"* — the
    /// warehouse is grid + 오브가방 + 재료가방 + 소비가방, four rooms that do not share a
    /// cell. A full grid refuses equipment and nothing else; a full box refuses its own
    /// categories and nothing else.
    #[test]
    fn each_warehouse_box_has_its_own_room_beside_the_grid() {
        let mut st = Store { bags: 1, ..Store::default() };
        for i in 1..=16u64 {
            assert!(st.deposit(item(i, 0x04, 0x02)), "piece {i} of a 16-cell grid");
        }
        assert!(!st.deposit(item(17, 0x04, 0x02)), "the grid is full");
        assert!(!st.deposit(stack(18, 0x11, 0x01, 1)), "…for quest items too (a grid category)");
        // The boxes did not notice.
        assert!(st.deposit(stack(20, 0x10, 0x03, 1)), "an orb still goes into the 오브가방");
        assert!(st.deposit(stack(21, 0x0e, 0x03, 5)), "a material into the 재료가방");
        assert!(st.deposit(stack(22, 0x12, 0x00, 9)), "a potion into the 소비가방");
        assert_eq!(st.shown_count(), 16, "the grid shows exactly its sixteen pieces");
        assert_eq!(st.pages(), 2, "…which, with the three openers, is two pages");

        // Now fill the 오브가방 — 127 more stacks of another orb, full so nothing merges.
        for i in 0..(WH_BOX_CAP as u64 - 1) {
            assert!(st.deposit(stack(100 + i, 0x10, 0x05, STACK_MAX)), "orb stack {i}");
        }
        assert_eq!(st.box_count(0), WH_BOX_CAP);
        assert!(!st.deposit(stack(900, 0x10, 0x07, 1)), "the 오브가방 is full");
        assert_eq!(st.room_for(&stack(0, 0x10, 0x07, 1)), 0);
        assert!(st.deposit(stack(901, 0x0e, 0x04, 1)), "…and the 재료가방 is not");
        assert!(st.deposit(stack(902, 0x0d, 0x01, 1)), "…nor the 소비가방 (0x0d shares it)");
        // ⛔ And the runaway guard is above all of it: grid 128 + 3 x 128.
        assert!(st.items.len() < WH_MAX_ROWS);
    }

    /// The box cap is the WAREHOUSE AT ITS LARGEST (8 bags x 16 = 128), fixed — the same
    /// rule as the bag's `bagbox::CAP` (80 = 5 tabs x 16). It does not grow with 창고열쇠,
    /// exactly as the bag's boxes do not grow with 여행가방.
    #[test]
    fn the_box_cap_is_the_largest_warehouse_and_does_not_grow_with_bags() {
        assert_eq!(WH_BOX_CAP, 128);
        assert_eq!(WH_BOX_CAP, WH_MAX_BAGS as usize * WH_MAX_SLOTS);
        assert_eq!(WH_BOXES, 3);
        assert_eq!(WH_MAX_ROWS, 128 + 3 * 128, "the runaway guard covers every room at its largest");
        let one = Store { bags: 1, ..Store::default() };
        let eight = Store { bags: WH_MAX_BAGS, ..Store::default() };
        assert_eq!(one.room_for(&stack(0, 0x10, 0x03, 1)), eight.room_for(&stack(0, 0x10, 0x03, 1)));
        assert_eq!(one.room_for(&stack(0, 0x10, 0x03, 1)), (WH_BOX_CAP * 99) as u32);
        // The GRID does grow with bags.
        assert_eq!(one.room_for(&item(0, 0x04, 0x02)), 16);
        assert_eq!(eight.room_for(&item(0, 0x04, 0x02)), 128);
    }

    /// `box_of` is the same table as `bagbox::Kind` and `wie_util::stats::bagbox_box_of`
    /// — 0 오브 · 1 재료 · 2 소비 — and `boxed_category` is exactly "has a box".
    #[test]
    fn box_of_is_the_boxes_own_table() {
        assert_eq!(box_of(0x10), Some(0));
        assert_eq!(box_of(0x0e), Some(1));
        assert_eq!(box_of(0x0f), None, "125차-B: a recipe has no box");
        assert_eq!(box_of(0x12), Some(2));
        assert_eq!(box_of(0x0d), Some(2));
        for cat in (0x00..=0x0c).chain([0x11, 0x13, 0x1f, 0xff]) {
            assert_eq!(box_of(cat), None, "cat {cat:#x}");
        }
        for cat in 0x00..=0xff {
            assert_eq!(boxed_category(cat), box_of(cat).is_some(), "cat {cat:#x}");
        }
    }

    #[test]
    fn a_host_filed_stack_is_an_ordinary_row_the_client_can_take_back() {
        // What a 올리기 out of a bag box files (96차). It has to be indistinguishable from
        // a row the game itself deposited, or 내리기 and the grid would choke on it.
        let rec = new_stack(0x0e, 0x03, 42, 7);
        assert_eq!(rec[ITEM_CAT_OFF], 0x0e);
        assert_eq!(rec[ITEM_IDX_OFF], 0x03);
        assert_eq!(unit_count(&rec), 42);
        assert!(stacks(&rec), "a material stacks");
        // ⛔ Never an opener key — those are the three virtual rows.
        assert!(!is_opener_key(&rec[..8]));
        // Two mints of the same item are distinct rows, and they MERGE like the game's.
        let other = new_stack(0x0e, 0x03, 42, 8);
        assert_ne!(rec[..8], other[..8]);
        assert!(same_stack(&rec, &other));
        // The count is clamped into the client's own 99 (a 128+ row would be read as
        // NEGATIVE and subtract from the player's stack — `0x135da`).
        assert_eq!(unit_count(&new_stack(0x10, 0x01, 200, 1)), STACK_MAX);
        assert_eq!(unit_count(&new_stack(0x10, 0x01, 0, 1)), 1);

        // And it goes in and comes back out through the ordinary store paths.
        let mut st = Store::new();
        assert!(st.deposit(rec));
        assert!(st.deposit(other));
        assert_eq!(st.items.len(), 1, "84 units merge into one row");
        assert_eq!(unit_count(&st.items[0]), 84);
        let key = st.items[0][..8].to_vec();
        let back = st.take_units(&key, 10).expect("a partial withdraw");
        assert_eq!(unit_count(&back), 10);
        assert_eq!(unit_count(&st.items[0]), 74);
    }

    #[test]
    fn an_opener_row_cannot_be_withdrawn_or_discarded() {
        let mut e = engine();
        e.store.bags = 1;
        let key = opener_rows()[0][..8].to_vec();
        // It is not in the store at all, so the store itself has nothing to give.
        assert!(e.store.take_item(&key).is_none());
        let env = Env::default();
        for op in [OP_WH_DOWNLOAD, OP_WH_DISCARD] {
            let mut req = vec![CAT_WAREHOUSE, op];
            req.extend_from_slice(&key);
            let out = e.handle(&req, &env);
            let b = payload(out.first().expect("a refusal, not silence"));
            // `05 <op> 00 00 <euc-kr> 00` — status 0 WITH TEXT. ⛔ A bare 3-byte ack
            // makes the client's message box read past the end of the frame.
            assert_eq!(&b[..4], &[CAT_WAREHOUSE, op, 0x00, 0x00]);
            assert!(b.len() > 5, "the refusal must carry its reason");
        }
    }

    // ── 136차: 정렬 ─────────────────────────────────────────────────────────

    /// The store reorders by the key it is handed, stably, and every row keeps its bytes.
    #[test]
    fn sorting_rows_keeps_every_byte_and_reports_whether_anything_moved() {
        let mut s = Store::new();
        s.items = vec![item(1, 0x00, 0x01), item(2, 0x05, 0x10), item(3, 0x0e, 0x03), item(4, 0x05, 0x00)];
        // A stand-in for `bagsort::wire_key`: equipment (cat <= 0xc) by (cat desc-ish, idx desc),
        // materials after; position last so equal keys keep their order.
        let key = |cat: u8, idx: u8, _count: u8, pos: u16| (u8::from(cat > 0x0c), 0xff - cat, 0xff - idx, pos);
        assert!(s.sort_rows_by(key));
        let keys: Vec<u64> = s.items.iter().map(|r| u64::from_le_bytes(r[..8].try_into().unwrap())).collect();
        assert_eq!(keys, vec![2, 4, 1, 3]);
        for r in &s.items {
            assert_eq!(r[12..], [0xff; ITEM_LEN - 12], "no byte beyond the sorted columns changed");
        }
        assert!(!s.sort_rows_by(key), "already in order");
    }

    /// One sort orders the grid AND the three box views — they are one list.
    #[test]
    fn sorting_orders_the_grid_and_the_boxes_together() {
        let mut s = Store::new();
        s.items = vec![item(1, 0x10, 0x07), item(2, 0x00, 0x01), item(3, 0x10, 0x02), item(4, 0x0b, 0x00)];
        s.sort_rows_by(|cat: u8, idx: u8, _n: u8, pos: u16| (u8::from(boxed_category(cat)), cat, idx, pos));
        let shown: Vec<u8> = s.shown().map(|r| r[ITEM_CAT_OFF]).collect();
        assert_eq!(shown, vec![0x00, 0x0b]);
        let orbs: Vec<u8> = s.boxed_rows(&[0x10]).iter().map(|r| r[ITEM_IDX_OFF]).collect();
        assert_eq!(orbs, vec![0x02, 0x07]);
    }

    /// `page_for` against a foreign store builds the same frame the engine builds for
    /// its own — what the 정렬 refresh pushes to the client.
    #[test]
    fn page_for_matches_the_engines_own_page() {
        let mut e = engine();
        e.store.items = vec![item(9, 0x00, 0x01), item(8, 0x05, 0x02)];
        let own = e.handle(&[CAT_WAREHOUSE, OP_WH_SLOT, 0x00], &Env::default());
        assert_eq!(own.len(), 1);
        assert_eq!(own[0], Engine::page_for(&e.store, 0));
        assert_eq!(payload(&own[0])[2], 0, "cursor byte = page 0");
    }
}
