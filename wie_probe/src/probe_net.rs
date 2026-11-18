//! `--net-offline 1` — the in-app server, headless.
//!
//! The probe normally has no network (`Platform::network` = `None`), so every WIPI net
//! slot fails and nothing behind the game's 네트워크 / 복구 prompts can be measured
//! without a device. This backs the guest's sockets with the APP'S OWN offline engine —
//! `wie_android/src/offline.rs`, `include!`d, not a copy — and logs every frame in both
//! directions with the virtual clock, so the order of a conversation (which request
//! arrives before the login notice and which after) is a probe run, not an argument
//! from two disassembly readings.
//!
//! ★ 135차-B is what it was built for: it measured that `04 3e` is sent by ONE path
//! (the game-start slot list's `#` on an empty slot) and always as the LOGIN FINALISER,
//! while the title 네트워크창고 entry finalises with `01 3e` — see
//! `offline::Engine::recovery`.
//!
//! What it deliberately does NOT reproduce from `network.rs`: the 400 ms auto-dismiss
//! of the login notice (press OK in `--keys` instead), the quantity dialog, and the
//! guest text patches the app applies offline (`emulator::offline_guest_patches` — the
//! probe shows the original 통화료 prompt).

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI32, AtomicU64, Ordering},
    },
};

/// Same value `network.rs` returns for "healthy, nothing to read yet".
const WOULDBLOCK: i32 = -19;
/// The guest's receive object is 8 bytes smaller than the length it asks for
/// (`network.rs::GUEST_RECV_SLACK`) — clamp the same way.
const GUEST_RECV_SLACK: usize = 8;

pub struct ProbeNet {
    socks: Mutex<HashMap<i32, (crate::offline::Engine, Vec<u8>)>>,
    fresh: Mutex<HashSet<i32>>,
    armed: Mutex<HashSet<i32>>,
    next_fd: AtomicI32,
    env: crate::offline::Env,
    clock: Arc<AtomicU64>,
}

impl ProbeNet {
    pub fn new(clock: Arc<AtomicU64>) -> Self {
        // The app's defaults: a purchase of 1, and a bag nobody has read yet (= no limit).
        let env = crate::offline::Env { cash_qty: 1, ..crate::offline::Env::default() };
        eprintln!("[net] in-app server armed — every connect lands on offline::Engine");
        Self {
            socks: Mutex::new(HashMap::new()),
            fresh: Mutex::new(HashSet::new()),
            armed: Mutex::new(HashSet::new()),
            next_fd: AtomicI32::new(3),
            env,
            clock,
        }
    }

    fn log_frames(&self, dir: &str, fd: i32, bytes: &[u8]) {
        let ms = self.clock.load(Ordering::SeqCst);
        let (frames, _) = crate::offline::split_frames(bytes);
        if frames.is_empty() {
            eprintln!("[net] t={ms} fd {fd} {dir} {} byte(s), no whole frame", bytes.len());
        }
        for f in frames {
            let head: Vec<String> = f.iter().take(24).map(|b| format!("{b:02x}")).collect();
            eprintln!(
                "[net] t={ms} fd {fd} {dir} cat={:02x} op={:02x} len={} | {}{}",
                f.first().copied().unwrap_or(0),
                f.get(1).copied().unwrap_or(0),
                f.len(),
                head.join(" "),
                if f.len() > 24 { " …" } else { "" }
            );
        }
    }
}

impl wie_backend::Network for ProbeNet {
    fn socket(&self, af: i32, ty: i32) -> i32 {
        let fd = self.next_fd.fetch_add(1, Ordering::Relaxed);
        self.fresh.lock().unwrap().insert(fd);
        eprintln!("[net] t={} socket({af}, {ty}) -> fd {fd}", self.clock.load(Ordering::SeqCst));
        fd
    }

    fn connect(&self, fd: i32, addr: u32, port_be: u16, name: &str) -> i32 {
        if !self.fresh.lock().unwrap().remove(&fd) {
            return -1;
        }
        let o = addr.to_le_bytes();
        eprintln!(
            "[net] t={} connect(fd {fd}, {name:?}) {}.{}.{}.{}:{} -> in-app server",
            self.clock.load(Ordering::SeqCst),
            o[0],
            o[1],
            o[2],
            o[3],
            u16::from_be(port_be)
        );
        self.socks.lock().unwrap().insert(fd, (crate::offline::Engine::default(), Vec::new()));
        0
    }

    fn poll_connect(&self, fd: i32) -> i32 {
        if self.socks.lock().unwrap().contains_key(&fd) { 1 } else { -1 }
    }

    fn send(&self, fd: i32, data: &[u8]) -> i32 {
        self.log_frames("->", fd, data);
        let out = {
            let mut socks = self.socks.lock().unwrap();
            let Some((engine, outbox)) = socks.get_mut(&fd) else {
                return -1;
            };
            let out = engine.feed(data, &self.env);
            outbox.extend_from_slice(&out);
            out
        };
        if !out.is_empty() {
            self.log_frames("<-", fd, &out);
        }
        data.len() as i32
    }

    fn recv(&self, fd: i32, buf: &mut [u8]) -> i32 {
        let safe = buf.len().saturating_sub(GUEST_RECV_SLACK);
        if safe == 0 {
            return WOULDBLOCK;
        }
        let mut socks = self.socks.lock().unwrap();
        let Some((_, outbox)) = socks.get_mut(&fd) else {
            return -1;
        };
        let n = safe.min(outbox.len());
        if n == 0 {
            return WOULDBLOCK;
        }
        buf[..n].copy_from_slice(&outbox[..n]);
        outbox.drain(..n);
        n as i32
    }

    fn arm_read(&self, fd: i32) -> bool {
        self.armed.lock().unwrap().insert(fd)
    }

    fn poll_readable(&self, fd: i32) -> i32 {
        let verdict = match self.socks.lock().unwrap().get(&fd) {
            Some((_, outbox)) => i32::from(!outbox.is_empty()),
            None if self.fresh.lock().unwrap().contains(&fd) => 0,
            None => -1,
        };
        if verdict != 0 {
            self.armed.lock().unwrap().remove(&fd);
        }
        verdict
    }

    fn close(&self, fd: i32) {
        self.armed.lock().unwrap().remove(&fd);
        self.fresh.lock().unwrap().remove(&fd);
        if self.socks.lock().unwrap().remove(&fd).is_some() {
            eprintln!("[net] t={} close fd {fd}", self.clock.load(Ordering::SeqCst));
        }
    }

    fn close_all(&self) {
        self.armed.lock().unwrap().clear();
        self.fresh.lock().unwrap().clear();
        let n = self.socks.lock().unwrap().drain().count();
        if n > 0 {
            eprintln!("[net] t={} close_all dropped {n} socket(s)", self.clock.load(Ordering::SeqCst));
        }
    }
}
