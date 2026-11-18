//! Standalone test for the guest page table's **lazy commit**.
//!
//! ```powershell
//! rustc --edition 2021 --test page_table_selftest.rs -o t.exe; .\t.exe
//! ```
//!
//! Same arrangement as `landscape_selftest.rs` and friends, for the same reason: `cargo test`
//! cannot build this repo's vendored wie workspace on the Windows host (its dev-dependency
//! graph pulls `parking_lot`, whose `windows` import libraries need a working `dlltool`, and
//! the bundled gnu one cannot spawn its assembler). So the shipping source is `include!`d
//! here — this is the real `PageTable`, not a copy of it.
//!
//! WHAT IS BEING PROTECTED. `map` used to allocate every page in its range, which meant the
//! guest heap's `map(0x40000000, 0x10000000)` committed 256 MB before the first frame. On the
//! player's device that showed up as 533 MB process RSS and a `reason=3 (LOW_MEMORY)` kill in
//! `dumpsys activity exit-info` — the app being killed in the background is what a player
//! experiences as the game "튕김". These tests pin the three properties the fix rests on:
//! mapping allocates nothing, reading an unwritten page is zero and still allocates nothing,
//! and an unmapped access still faults instead of quietly mapping itself.

extern crate alloc;

include!("vendor/wie/wie_core_arm/src/engine/page_table.rs");

const HEAP_BASE: u32 = 0x4000_0000;
/// The real `wie_core_arm::core::HEAP_SIZE`: 256 MB, i.e. 4096 pages.
const HEAP_SIZE: usize = 0x1000_0000;

// ── the fix itself ──────────────────────────────────────────────────────────

#[test]
fn mapping_the_whole_guest_heap_allocates_nothing() {
    let mut mem = PageTable::new();
    mem.map(HEAP_BASE, HEAP_SIZE);

    assert_eq!(mem.committed_pages(), 0, "map must not allocate");
    assert_eq!(HEAP_SIZE / PAGE_SIZE, 4096, "the range under test is the real 256 MB heap");
}

#[test]
fn mapped_is_mapped_even_with_no_backing_page() {
    // `ArmCore`'s guest-pointer checks call `is_mapped` before reading; if that answered "no"
    // for an untouched page the emulator would reject valid heap pointers.
    let mut mem = PageTable::new();
    mem.map(HEAP_BASE, HEAP_SIZE);

    assert!(mem.is_mapped(HEAP_BASE, HEAP_SIZE));
    assert!(mem.is_mapped(HEAP_BASE + HEAP_SIZE as u32 - 4, 4));
    assert!(!mem.is_mapped(HEAP_BASE + HEAP_SIZE as u32, 4), "one past the end is not mapped");
    assert!(!mem.is_mapped(HEAP_BASE - 4, 4), "one before the start is not mapped");
}

#[test]
fn writing_commits_exactly_the_pages_touched() {
    let mut mem = PageTable::new();
    mem.map(HEAP_BASE, HEAP_SIZE);

    mem.write_range(HEAP_BASE, &[1, 2, 3, 4]).unwrap();
    assert_eq!(mem.committed_pages(), 1);

    // Same page again: no new allocation.
    mem.write_range(HEAP_BASE + 0x100, &[5]).unwrap();
    assert_eq!(mem.committed_pages(), 1);

    // A write that straddles a page boundary commits both halves.
    let straddle = HEAP_BASE + PAGE_SIZE as u32 * 8 - 2;
    mem.write_range(straddle, &[9, 9, 9, 9]).unwrap();
    assert_eq!(mem.committed_pages(), 3);

    let mut buf = [0u8; 4];
    mem.read_range(straddle, 4, &mut buf).unwrap();
    assert_eq!(buf, [9, 9, 9, 9]);
}

#[test]
fn reading_an_uncommitted_page_is_zero_and_allocates_nothing() {
    let mut mem = PageTable::new();
    mem.map(HEAP_BASE, HEAP_SIZE);

    // `result` deliberately starts non-zero: the read has to WRITE the zeroes, not skip.
    let mut buf = [0xffu8; 0x2000];
    assert_eq!(mem.read_range(HEAP_BASE + 0x30000, 0x2000, &mut buf).unwrap(), 0x2000);
    assert!(buf.iter().all(|b| *b == 0));
    assert_eq!(mem.committed_pages(), 0, "a read must never commit");
}

#[test]
fn a_read_spanning_committed_and_uncommitted_pages_joins_them() {
    // The case a naive "if the page is missing, return zeros" would get wrong: one range whose
    // first page has data and whose second never did.
    let mut mem = PageTable::new();
    mem.map(HEAP_BASE, 0x30000);

    let boundary = HEAP_BASE + PAGE_SIZE as u32;
    mem.write_range(boundary - 4, &[0xaa; 4]).unwrap();
    assert_eq!(mem.committed_pages(), 1);

    let mut buf = [0x55u8; 8];
    mem.read_range(boundary - 4, 8, &mut buf).unwrap();
    assert_eq!(buf, [0xaa, 0xaa, 0xaa, 0xaa, 0, 0, 0, 0]);
    assert_eq!(mem.committed_pages(), 1, "the read of the second page must not have committed it");
}

// ── faults still fault ──────────────────────────────────────────────────────

#[test]
fn unmapped_read_and_write_fault_with_the_guest_address() {
    let mut mem = PageTable::new();
    mem.map(0x10000, 0x10000);

    let mut buf = [0u8; 0x100];
    assert_eq!(mem.read_range(0x50000, 0x100, &mut buf), Err(0x50000));
    assert_eq!(mem.write_range(0x50000, &[1; 0x100]), Err(0x50000));
    assert_eq!(mem.committed_pages(), 0, "a faulting write must not have mapped anything");
}

#[test]
fn a_range_that_runs_off_the_end_of_the_mapping_faults_at_the_boundary() {
    let mut mem = PageTable::new();
    mem.map(0x10000, 0x10000);

    // Two pages' worth starting inside the mapping: the first half is fine, the second is not,
    // and the reported address is where it actually went wrong.
    let mut buf = alloc::vec![0u8; PAGE_SIZE * 2];
    assert_eq!(mem.read_range(0x18000, PAGE_SIZE * 2, &mut buf), Err(0x20000));
}

#[test]
fn page_for_read_and_write_agree_with_the_range_helpers() {
    let mut mem = PageTable::new();
    mem.map(HEAP_BASE, 0x20000);

    let mapped = PageTable::page_index(HEAP_BASE);
    let unmapped = PageTable::page_index(HEAP_BASE + 0x20000);

    assert_eq!(mem.page_for_read(mapped), Ok(None), "mapped, never written");
    assert_eq!(mem.page_for_read(unmapped), Err(()));
    assert!(mem.page_for_write(unmapped).is_err());

    mem.page_for_write(mapped).unwrap()[7] = 0x42;
    assert_eq!(mem.page_for_read(mapped).unwrap().unwrap()[7], 0x42);
    assert_eq!(mem.committed_pages(), 1);
}

// ── the address arithmetic ──────────────────────────────────────────────────

#[test]
fn page_index_matches_the_page_size() {
    assert_eq!(PAGE_SIZE, 0x10000);
    assert_eq!(PAGE_MASK, 0xffff);
    assert_eq!(PAGE_COUNT, 0x10000, "4 GB of 64 KB pages — every u32 address has an index");
    assert_eq!(PageTable::page_index(0), 0);
    assert_eq!(PageTable::page_index(PAGE_MASK), 0);
    assert_eq!(PageTable::page_index(PAGE_SIZE as u32), 1);
    assert_eq!(PageTable::page_index(u32::MAX), PAGE_COUNT - 1);
}

#[test]
fn map_rounds_out_to_whole_pages() {
    // A sub-page request still makes the whole page usable, which is what every small
    // `core.map(0x2000, 0x40)` in the binary-patch table relies on.
    let mut mem = PageTable::new();
    mem.map(0x2000, 0x40);

    assert!(mem.is_mapped(0x0, PAGE_SIZE));
    mem.write_range(0xfffc, &[1, 2, 3, 4]).unwrap();
    assert_eq!(mem.committed_pages(), 1);
}

/// Nothing outside the requested range may become mapped — a single stray bit in the bitmap
/// would turn a guest bug into silent corruption instead of a fault.
#[test]
fn map_sets_only_the_requested_bits() {
    let mut mem = PageTable::new();
    mem.map(0x30000, 0x20000); // pages 3 and 4

    for index in 0..8 {
        assert_eq!(mem.is_page_mapped(index), index == 3 || index == 4, "page {index}");
    }
}
