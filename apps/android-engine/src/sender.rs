//! Runs inside the AAudio data callback: channel selection, gain, mute,
//! packetising, in-place AES-GCM and one `sendmmsg` per callback. Everything
//! is preallocated; the only lock is taken when the peer changes.

use std::net::{SocketAddr, UdpSocket};
use std::os::fd::AsRawFd;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use toaster_core::phone::{Peer, LEASE_MS};
use toaster_core::wire::{self, flags, Profile};
use toaster_core::now_ms;

use crate::engine::Shared;

const MAX_BATCH: usize = 32;

pub struct Sender {
    shared: Arc<Shared>,
    socket: Arc<UdpSocket>,
    capture_channels: usize,
    send_channels: u8,
    /// Send channel -> capture channel, for left = first / swapped.
    map: [[usize; 4]; 2],
    base_flags: u8,
    frames: usize,
    pending: Vec<i16>,
    filled: usize,
    slots: Vec<[u8; wire::MAX_DATAGRAM]>,
    lens: [usize; MAX_BATCH],
    count: usize,
    sequence: u32,
    peer_gen: u64,
    peer: Option<Arc<Peer>>,
    peaks: [u16; 4],
    level_tick: u32,
}

impl Sender {
    pub fn new(
        shared: Arc<Shared>,
        socket: Arc<UdpSocket>,
        capture_channels: usize,
        send_channels: u8,
        map: [[usize; 4]; 2],
        base_flags: u8,
    ) -> Sender {
        let frames = wire::frames_for(send_channels) as usize;
        Sender {
            shared,
            socket,
            capture_channels,
            send_channels,
            map,
            base_flags,
            frames,
            pending: vec![0; frames * send_channels as usize],
            filled: 0,
            slots: vec![[0u8; wire::MAX_DATAGRAM]; MAX_BATCH],
            lens: [0; MAX_BATCH],
            count: 0,
            sequence: 0,
            peer_gen: u64::MAX,
            peer: None,
            peaks: [0; 4],
            level_tick: 0,
        }
    }

    fn refresh_peer(&mut self, now: u64) -> bool {
        let gen = self.shared.peer_gen.load(Ordering::Acquire);
        if gen != self.peer_gen {
            if let Ok(p) = self.shared.peer.try_lock() {
                self.peer = p.clone();
                self.peer_gen = gen;
            }
        }
        self.peer.is_some() && now.saturating_sub(self.shared.last_hello.load(Ordering::Acquire)) < LEASE_MS
    }

    pub fn on_audio(&mut self, input: &[i16]) {
        let now = now_ms();
        let active = self.refresh_peer(now);
        let muted = self.shared.muted.load(Ordering::Relaxed);
        let gain = self.shared.gain_pct.load(Ordering::Relaxed) as i32;
        let map = self.map[self.shared.left.load(Ordering::Relaxed).min(1) as usize];
        let fast = now < self.shared.fast_until.load(Ordering::Relaxed);
        let profile = if fast { Profile::LowLatency } else { Profile::from_index(self.shared.profile.load(Ordering::Relaxed)) };
        let mut f = self.base_flags | profile.bits();
        if muted {
            f |= flags::MUTED;
        }
        if fast {
            f |= flags::FAST_START;
        }
        let ch = self.send_channels as usize;
        for frame in input.chunks_exact(self.capture_channels) {
            let base = self.filled * ch;
            for c in 0..ch {
                let v = if muted { 0 } else { (frame[map[c]] as i32 * gain / 100).clamp(-32768, 32767) };
                self.pending[base + c] = v as i16;
                let a = v.unsigned_abs().min(32767) as u16;
                if a > self.peaks[c] {
                    self.peaks[c] = a;
                }
            }
            self.filled += 1;
            if self.filled == self.frames {
                self.filled = 0;
                if active && self.count < MAX_BATCH {
                    self.seal(f);
                }
            }
        }
        let bundle = (profile.bundle_ms() as usize * wire::RATE as usize / 1000) / self.frames;
        if self.count > 0 && (self.count >= bundle.max(1) || !active) {
            self.flush();
        }
        // Publish levels ~10x per second.
        self.level_tick += input.len() as u32 / self.capture_channels as u32;
        if self.level_tick >= wire::RATE / 10 {
            self.level_tick = 0;
            let s = &self.shared;
            let decay = |old: u32, new: u16| (new as u32).max(old * 7 / 10);
            s.left_peak.store(decay(s.left_peak.load(Ordering::Relaxed), self.peaks[0]), Ordering::Relaxed);
            s.right_peak.store(decay(s.right_peak.load(Ordering::Relaxed), self.peaks[1.min(ch - 1)]), Ordering::Relaxed);
            s.peak.store(*self.peaks[..ch].iter().max().unwrap() as u32, Ordering::Relaxed);
            self.peaks = [0; 4];
        }
    }

    fn seal(&mut self, f: u8) {
        let Some(peer) = self.peer.as_ref() else { return };
        if self.sequence == u32::MAX {
            // Never reuse a GCM nonce: the session must be restarted.
            self.shared.fail("Bitte Mikrofon neu starten (Sitzungsgrenze erreicht).");
            return;
        }
        let slot = &mut self.slots[self.count];
        wire::write_pcm(&self.pending, &mut slot[wire::HEADER..]);
        let len = peer.key.seal_audio(slot, self.send_channels, f, peer.session, self.sequence);
        self.sequence += 1;
        self.lens[self.count] = len;
        self.count += 1;
        self.shared.packets.fetch_add(1, Ordering::Relaxed);
    }

    fn flush(&mut self) {
        let n = std::mem::take(&mut self.count);
        let Some(peer) = self.peer.as_ref() else { return };
        let SocketAddr::V4(dest) = peer.addr else { return };
        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        addr.sin_family = libc::AF_INET as libc::sa_family_t;
        addr.sin_port = dest.port().to_be();
        addr.sin_addr.s_addr = u32::from(*dest.ip()).to_be();
        let mut iov: [libc::iovec; MAX_BATCH] = unsafe { std::mem::zeroed() };
        let mut msgs: [libc::mmsghdr; MAX_BATCH] = unsafe { std::mem::zeroed() };
        for i in 0..n {
            iov[i] = libc::iovec { iov_base: self.slots[i].as_mut_ptr() as *mut _, iov_len: self.lens[i] };
            msgs[i].msg_hdr.msg_name = &mut addr as *mut _ as *mut _;
            msgs[i].msg_hdr.msg_namelen = std::mem::size_of::<libc::sockaddr_in>() as u32;
            msgs[i].msg_hdr.msg_iov = &mut iov[i];
            msgs[i].msg_hdr.msg_iovlen = 1;
        }
        // Non-blocking: a full socket buffer drops audio instead of stalling capture.
        unsafe {
            libc::sendmmsg(self.socket.as_raw_fd(), msgs.as_mut_ptr(), n as u32, libc::MSG_DONTWAIT);
        }
    }
}
