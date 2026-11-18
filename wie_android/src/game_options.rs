use crate::menutouch::{self, Box2};
use std::{
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
};
use wie_backend::KeyCode;

pub const RATES: [u8; 6] = [1, 2, 3, 4, 5, 6];
pub const RESPAWN_RATES: [u8; 6] = [1, 2, 3, 4, 5, 6];
static RESPAWN: AtomicU8 = AtomicU8::new(1);
static EXP: AtomicU8 = AtomicU8::new(1);
static LOOT: AtomicBool = AtomicBool::new(true);
static STATE: Mutex<State> = Mutex::new(State {
    active: false,
    cursor: 0,
    top: 0,
    guest: 0,
    rows: Vec::new(),
});
struct State {
    active: bool,
    cursor: usize,
    top: usize,
    guest: usize,
    rows: Vec<Box2>,
}
pub fn respawn_rate() -> u8 {
    RESPAWN.load(Ordering::Relaxed)
}
pub fn exp_rate() -> u8 {
    EXP.load(Ordering::Relaxed)
}
pub fn auto_loot() -> bool {
    LOOT.load(Ordering::Relaxed)
}
pub fn load(dir: &Path) {
    let text = std::fs::read_to_string(dir.join("wie_game_options.txt")).unwrap_or_default();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if let (Some(key), Some(value)) = (words.next(), words.next()) {
            match key {
                "respawn" => RESPAWN.store(value.parse().ok().filter(|v| RESPAWN_RATES.contains(v)).unwrap_or(1), Ordering::Relaxed),
                "exp" => EXP.store(parse_rate(value), Ordering::Relaxed),
                "loot" => LOOT.store(value != "0", Ordering::Relaxed),
                _ => {}
            }
        }
    }
}
fn parse_rate(s: &str) -> u8 {
    s.parse().ok().filter(|v| RATES.contains(v)).unwrap_or(1)
}
fn save(dir: &Path) {
    if let Err(e) = std::fs::write(
        dir.join("wie_game_options.txt"),
        format!("respawn {}\nexp {}\nloot {}\n", respawn_rate(), exp_rate(), u8::from(auto_loot())),
    ) {
        log::warn!("game options save: {e}");
    }
}
fn cycle(rates: &[u8], value: u8, dir: i32) -> u8 {
    let i = rates.iter().position(|v| *v == value).unwrap_or(0) as i32;
    rates[(i + dir).rem_euclid(rates.len() as i32) as usize]
}
pub fn observe(px: &[u32], w: i32, h: i32) {
    let detected = menutouch::option_rows(px, w, h);
    let mut s = STATE.lock().unwrap();
    if let Some((cursor, rows)) = detected {
        if !s.active {
            s.cursor = cursor as usize;
            s.top = 0;
        }
        s.active = true;
        s.guest = cursor as usize;
        s.rows = rows;
    } else {
        s.active = false;
        s.rows.clear();
    }
}
pub fn cursor() -> Option<usize> {
    let s = STATE.lock().unwrap();
    s.active.then_some(s.cursor)
}
pub fn tap_row(dx: i32, dy: i32) -> Option<(usize, usize)> {
    let s = STATE.lock().unwrap();
    if !s.active {
        return None;
    }
    if (164..175).contains(&dx) {
        let first = s.rows.first()?;
        let last = s.rows.last()?;
        let height = last.dy + last.h - first.dy;
        if dy >= first.dy && dy < first.dy + height {
            return Some((s.cursor, ((dy - first.dy) * 11 / height).clamp(0, 10) as usize));
        }
    }
    s.rows
        .iter()
        .position(|r| dy >= r.dy && dy < r.dy + r.h && (12..164).contains(&dx))
        .map(|slot| (s.cursor, s.top + slot))
}
pub fn key(code: KeyCode, down: bool, dir: &Path) -> Option<Vec<KeyCode>> {
    let mut s = STATE.lock().unwrap();
    if !s.active {
        return None;
    }
    if down && code == KeyCode::CLEAR {
        s.active = false;
        return None;
    }
    if !matches!(code, KeyCode::UP | KeyCode::DOWN | KeyCode::LEFT | KeyCode::RIGHT | KeyCode::OK) {
        return None;
    }
    if !down {
        return None;
    }
    if matches!(code, KeyCode::UP | KeyCode::DOWN) {
        let delta = if code == KeyCode::UP { -1 } else { 1 };
        s.cursor = (s.cursor as i32 + delta).rem_euclid(11) as usize;
        if s.cursor < s.top {
            s.top = s.cursor;
        }
        if s.cursor >= s.top + 7 {
            s.top = s.cursor - 6;
        }
        let target = s.cursor.min(6);
        let old = s.guest;
        s.guest = target;
        return Some(if target >= old {
            vec![KeyCode::DOWN; target - old]
        } else {
            vec![KeyCode::UP; old - target]
        });
    }
    if s.cursor < 7 {
        return None;
    }
    let delta = if code == KeyCode::LEFT { -1 } else { 1 };
    match s.cursor {
        7 => RESPAWN.store(cycle(&RESPAWN_RATES, respawn_rate(), delta), Ordering::Relaxed),
        8 => LOOT.store(!auto_loot(), Ordering::Relaxed),
        9 => EXP.store(cycle(&RATES, exp_rate(), delta), Ordering::Relaxed),
        _ => {
            let now = if delta > 0 {
                crate::stick_mode().next()
            } else {
                crate::stick_mode().next().next()
            };
            crate::set_stick_mode(now);
            crate::config::write_stick_mode(dir, now);
        }
    }
    save(dir);
    Some(Vec::new())
}
fn text(px: &mut [u32], w: usize, x: i32, y: i32, value: &str, ink: u32) {
    let mut pen = x;
    for cell in crate::gamefont::layout(value) {
        for (dy, bits) in cell.rows.iter().enumerate() {
            for dx in 0..16 {
                if bits & (0x8000 >> dx) != 0 {
                    let (xx, yy) = (pen + dx, y + dy as i32);
                    if xx >= 0 && xx < w as i32 && yy >= 0 {
                        if let Some(p) = px.get_mut(yy as usize * w + xx as usize) {
                            *p = ink;
                        }
                    }
                }
            }
        }
        pen += cell.advance as i32;
    }
}
pub fn render(raw: &[u32], w: i32, h: i32) -> Option<Vec<u32>> {
    let s = STATE.lock().unwrap();
    if !s.active || s.rows.len() != 7 {
        return None;
    }
    let (ox, oy) = menutouch::panel_origin(w, h)?;
    let mut out = raw.to_vec();
    let values = [
        rate_label(respawn_rate()),
        if auto_loot() { "ON" } else { "OFF" },
        rate_label(exp_rate()),
        match crate::stick_mode() {
            crate::StickMode::Float => "자유",
            crate::StickMode::Dpad => "십자키",
            crate::StickMode::Pinned => "고정",
        },
    ];
    let labels = ["몬스터 리젠속도", "오토루팅", "경험치배율", "조작방식"];
    for slot in 0..7 {
        let row = s.top + slot;
        let target = s.rows[slot];
        let selected = row == s.cursor;
        for dy in 0..target.h {
            for dx in 12..164 {
                let at = ((oy + target.dy + dy) * w + ox + dx) as usize;
                if let Some(p) = out.get_mut(at) {
                    *p = if selected { 0xff4a3929 } else { 0xffcecac5 };
                }
            }
        }
        if row < 7 {
            let source = s.rows[row];
            for dy in 0..target.h.min(source.h) {
                for dx in 0..source.w {
                    let from = ((oy + source.dy + dy) * w + ox + source.dx + dx) as usize;
                    let to = ((oy + target.dy + dy) * w + ox + source.dx + dx) as usize;
                    if let (Some(pixel), Some(dst)) = (raw.get(from), out.get_mut(to)) {
                        let rgb = pixel & 0xffffff;
                        *dst = if !selected && row == s.guest {
                            if rgb == 0x4a3929 || rgb == 0x4a3d31 {
                                0xffcecac5
                            } else if rgb == 0xffffff {
                                0xff292018
                            } else {
                                *pixel
                            }
                        } else {
                            *pixel
                        };
                    }
                }
            }
        } else {
            let ink = if selected { 0xffffffff } else { 0xff292018 };
            let value = values[row - 7];
            text(&mut out, w as usize, ox + 14, oy + target.dy + 1, labels[row - 7], ink);
            if selected {
                text(&mut out, w as usize, ox + 108, oy + target.dy + 1, "<", ink);
                text(&mut out, w as usize, ox + 154, oy + target.dy + 1, ">", ink);
            }
            text(
                &mut out,
                w as usize,
                ox + 151 - crate::gamefont::width(value) as i32,
                oy + target.dy + 1,
                value,
                ink,
            );
        }
    }
    let track_x = ox + 168;
    let y0 = oy + s.rows[0].dy;
    let height = s.rows[6].dy + s.rows[6].h - s.rows[0].dy;
    for y in 0..height {
        for x in 0..2 {
            let at = ((y0 + y) * w + track_x + x) as usize;
            if let Some(p) = out.get_mut(at) {
                *p = if y >= s.top as i32 * height / 11 && y < (s.top + 7) as i32 * height / 11 {
                    0xffe6c49a
                } else {
                    0xff493829
                };
            }
        }
    }
    Some(out)
}
fn rate_label(rate: u8) -> &'static str {
    match rate {
        2 => "X2",
        3 => "X3",
        4 => "X4",
        5 => "X5",
        6 => "X6",
        _ => "기존",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scrolling_settings_persist_and_leave_native_rows_to_the_game() {
        let dir = std::env::temp_dir().join(format!("hero3-options-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (496, 240);
        let (ox, oy) = menutouch::panel_origin(w, h).unwrap();
        let mut raw = vec![0xff806040; (w * h) as usize];
        for row in 0..7 {
            for dy in 0..14 {
                for dx in 32..144 {
                    raw[((oy + 19 + row * 18 + dy) * w + ox + dx) as usize] = if row == 0 { 0xff4a3929 } else { 0xffcecac5 };
                }
            }
        }
        observe(&raw, w, h);
        assert_eq!(cursor(), Some(0));
        assert!(key(KeyCode::RIGHT, true, &dir).is_none());
        for expected in 1..11 {
            assert!(key(KeyCode::DOWN, true, &dir).is_some());
            assert_eq!(cursor(), Some(expected));
        }
        assert_eq!(STATE.lock().unwrap().top, 4);
        let rendered = render(&raw, w, h).unwrap();
        assert_ne!(raw, rendered);
        if let Ok(path) = std::env::var("HERO3_OPTIONS_PREVIEW") {
            let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
            for pixel in &rendered {
                ppm.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8]);
            }
            std::fs::write(path, ppm).unwrap();
        }
        assert_eq!(tap_row(130, 19 + 6 * 18), Some((10, 10)));
        assert_eq!(tap_row(169, 19), Some((10, 0)));
        assert_eq!(tap_row(169, 19 + 6 * 18 + 13), Some((10, 10)));
        assert_eq!(key(KeyCode::DOWN, true, &dir).unwrap().len(), 6);
        assert_eq!(cursor(), Some(0));
        key(KeyCode::UP, true, &dir);
        assert_eq!(cursor(), Some(10));
        for _ in 0..3 {
            key(KeyCode::UP, true, &dir);
        }
        for expected in [2, 3, 4, 5, 6, 1] {
            key(KeyCode::RIGHT, true, &dir);
            assert_eq!(respawn_rate(), expected);
        }
        key(KeyCode::LEFT, true, &dir);
        key(KeyCode::DOWN, true, &dir);
        key(KeyCode::OK, true, &dir);
        assert!(!auto_loot());
        key(KeyCode::DOWN, true, &dir);
        for expected in [2, 3, 4, 5, 6] {
            key(KeyCode::RIGHT, true, &dir);
            assert_eq!(exp_rate(), expected);
        }
        RESPAWN.store(1, Ordering::Relaxed);
        EXP.store(1, Ordering::Relaxed);
        LOOT.store(true, Ordering::Relaxed);
        load(&dir);
        assert_eq!((respawn_rate(), exp_rate(), auto_loot()), (6, 6, false));
        assert!(key(KeyCode::CLEAR, true, &dir).is_none());
        assert_eq!(cursor(), None);
        assert!(render(&raw, w, h).is_none());
        std::fs::remove_file(dir.join("wie_game_options.txt")).unwrap();
        let _ = std::fs::remove_dir(dir);
    }
}
