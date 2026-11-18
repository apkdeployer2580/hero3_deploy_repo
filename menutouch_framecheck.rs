// Frame cross-check for the menu-touch detectors (80차-P).
//
//   rustc --edition 2021 -O menutouch_framecheck.rs -o fc.exe && ./fc.exe <frame.rgba>...
//
// Feeds REAL framebuffers — `wie_probe --dump` writes `frame_NNNNN_WxH.rgba` (RGBA bytes),
// and a device screenshot resampled to the logical size works too — through every pixel
// detector in `wie_android/src/menutouch.rs` and prints what each one claims. This is the
// "22 probe frames + 8 device frames" cross-check the 80차 rounds did by hand, as a tool:
// two detectors claiming the same frame is the collision to look for (OPTION vs the shell
// SLOT list is the one known and ordered one).
//
// Like the selftests it `include!`s the real source, so it cannot drift from it.

include!("wie_android/src/menutouch.rs");

fn main() {
    for path in std::env::args().skip(1) {
        let (lw, lh) = match std::path::Path::new(&path)
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.rsplit('_').next())
            .and_then(|s| s.split_once('x'))
            .and_then(|(w, h)| Some((w.parse::<i32>().ok()?, h.parse::<i32>().ok()?)))
        {
            Some(v) => v,
            None => {
                eprintln!("{path}: cannot read WxH from the file name");
                continue;
            }
        };
        let bytes = std::fs::read(&path).expect("read frame");
        assert_eq!(bytes.len(), (lw * lh * 4) as usize, "{path}: size mismatch");
        let px: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|c| (u32::from(c[3]) << 24) | (u32::from(c[0]) << 16) | (u32::from(c[1]) << 8) | u32::from(c[2]))
            .collect();
        println!("== {path} ({lw}x{lh})");
        let b = badges(&px, lw, lh);
        println!("  badges          {:?}", b.iter().map(|h| h.badge).collect::<Vec<_>>());
        println!("  confirm_pair    {:?}", confirm_pair(&px, lw, lh));
        println!("  title_carousel  {:?}", title_carousel(&px, lw, lh));
        println!("  pager_header    {:?}", pager_header(&px, lw, lh).map(|h| (h.prev, h.next)));
        println!("  option_rows     {:?}", option_rows(&px, lw, lh).map(|(c, r)| (c, r.len(), r[0])));
        println!("  slot_rows       {:?}", slot_rows(&px, lw, lh).map(|(c, _)| c));
        println!("  hash_prompt     {:?}", hash_prompt(&px, lw, lh));
        println!("  footer_tokens   {:?}", footer_token_prompts(&px, lw, lh));
        println!("  ok_token_prompt {:?}", ok_token_prompt(&px, lw, lh));
        println!("  ok_prompt       {:?}", ok_prompt(&px, lw, lh));
        println!("  spinner_arrows  {:?}", spinner_arrows(&px, lw, lh));
        println!("  item_grid       {:?}", item_grid(&px, lw, lh).map(|(g, c)| (g.cols, g.rows, g.pitch_x, g.x0, g.y0, c)));
        println!("  cell_box        {:?}", cell_box(&px, lw, lh));
        println!("  status_marker   {:?}", status_marker(&px, lw, lh));
        println!("  equip_marker    {:?}", equip_marker(&px, lw, lh));
        println!("  popup_box(2)    {:?}", popup_box(&px, lw, lh, 2));
        println!("  detail_page     {:?}", detail_page(&px, lw, lh));
    }
}
