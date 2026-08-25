//! Helpers for architecturally correct linear-memory accesses.
//!
//! Tier-0 and the assist layer often form a masked linear address using
//! [`crate::state::CpuState::apply_a20`], then perform a multi-byte bus access
//! (`read_u16`, `read_u32`, ...). The default `CpuBus` scalar helpers advance the
//! address with plain `+ 1`, which breaks architectural wrapping semantics when
//! a multi-byte access crosses:
//! - the 4GiB boundary in non-long modes (32-bit linear address space), or
//! - the A20 gate alias boundary in real/v8086 mode with A20 disabled.
//!
//! The helpers in this module fix that by re-applying [`CpuState::apply_a20`] to
//! each byte address (`addr + i`) so every byte is accessed through the correct
//! masked linear address.

use crate::exception::Exception;
use crate::mem::CpuBus;
use crate::state::{CpuMode, CpuState};
#[cfg(not(test))]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Fast-reject for the common case where no memory debug hooks are configured.
/// Initialized on first watch/stream call; stays false for production bring-up runs.
#[cfg(not(test))]
static MEM_DEBUG_ACTIVE: AtomicBool = AtomicBool::new(false);
#[cfg(not(test))]
static MEM_DEBUG_INIT: OnceLock<()> = OnceLock::new();

fn mem_debug_configured() -> bool {
    std::env::var_os("AERO_WATCH_WRITE").is_some()
        || std::env::var_os("AERO_WATCH_VALUE").is_some()
        || std::env::var_os("AERO_WATCH_READ").is_some()
        || std::env::var_os("AERO_WRITE_STREAM").is_some()
}

#[cfg(not(test))]
#[inline]
fn ensure_mem_debug_init() {
    MEM_DEBUG_INIT.get_or_init(|| {
        MEM_DEBUG_ACTIVE.store(mem_debug_configured(), Ordering::Relaxed);
    });
}

/// Whether any memory debug hook is armed.
///
/// The latch is deliberately one-shot: this is on the hot path of every guest
/// memory access, so production runs must pay one relaxed load and no `getenv`.
/// Tests set the environment inside the process and run in a shared address
/// space, so under `cfg(test)` the check is re-evaluated per call — otherwise
/// whichever test touched memory first would decide the answer for all of them.
#[cfg(not(test))]
#[inline]
fn mem_debug_active() -> bool {
    ensure_mem_debug_init();
    MEM_DEBUG_ACTIVE.load(Ordering::Relaxed)
}

#[cfg(test)]
fn mem_debug_active() -> bool {
    mem_debug_configured()
}

/// Optional memory-write watchpoints (bring-up/debug).
/// - `AERO_WATCH_WRITE`  (comma-separated linear addresses, hex or dec): log any
///   write that covers one of them.
/// - `AERO_WATCH_VALUE`  (integer): log any write whose little-endian value matches.
///
/// Both log the writing RIP, the destination address, length and value. This is
/// the single choke point shared by ALU/MOV stores, push/pop, string ops and the
/// assist layer, so it catches a write regardless of which instruction issued it.
#[cfg(not(test))]
static WATCH_WRITE: OnceLock<Vec<u64>> = OnceLock::new();
#[cfg(not(test))]
static WATCH_VALUE: OnceLock<Option<u64>> = OnceLock::new();
#[cfg(not(test))]
static WATCH_READ: OnceLock<Vec<u64>> = OnceLock::new();

/// Number of `AERO_WATCH_READ` hits so far (also lets tests verify the watch
/// fires without scraping stderr).
pub(crate) static WATCH_READ_HITS: AtomicU64 = AtomicU64::new(0);

fn parse_watch(var: &str) -> Option<u64> {
    let v = std::env::var(var).ok()?;
    let v = v.trim();
    u64::from_str_radix(v.trim_start_matches("0x"), 16)
        .ok()
        .or_else(|| v.parse::<u64>().ok())
}

/// Comma-separated linear addresses (hex or dec) for bring-up watchpoints.
fn parse_watch_list(var: &str) -> Vec<u64> {
    std::env::var(var)
        .map(|v| {
            v.split(',')
                .filter_map(|p| {
                    let p = p.trim();
                    u64::from_str_radix(p.trim_start_matches("0x"), 16)
                        .ok()
                        .or_else(|| p.parse::<u64>().ok())
                })
                .collect()
        })
        .unwrap_or_default()
}

// The parsed watch lists are latched once for the same reason the active flag
// is: they are consulted per guest memory access. Under `cfg(test)` they are
// re-parsed per call so a test that arms a watchpoint is not at the mercy of
// whichever sibling test happened to touch memory first.
#[cfg(not(test))]
fn watch_read_addrs() -> &'static [u64] {
    WATCH_READ.get_or_init(|| parse_watch_list("AERO_WATCH_READ"))
}

#[cfg(test)]
fn watch_read_addrs() -> &'static [u64] {
    Box::leak(parse_watch_list("AERO_WATCH_READ").into_boxed_slice())
}

#[cfg(not(test))]
fn watch_write_addrs() -> &'static [u64] {
    WATCH_WRITE.get_or_init(|| parse_watch_list("AERO_WATCH_WRITE"))
}

#[cfg(test)]
fn watch_write_addrs() -> &'static [u64] {
    Box::leak(parse_watch_list("AERO_WATCH_WRITE").into_boxed_slice())
}

#[inline]
pub(crate) fn watch_read(state: &CpuState, addr: u64, len: usize, val_le: u64) {
    if !mem_debug_active() {
        return;
    }
    let addrs = watch_read_addrs();
    if addrs.is_empty() {
        return;
    }
    let l = len as u64;
    if l == 0 {
        return;
    }
    for &w in addrs {
        if addr <= w && w < addr + l {
            WATCH_READ_HITS.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "[watch-read] [{}] rip={:#x} cr3={:#x} [{addr:#x}] len={l} val={val_le:#x}",
                crate::interp::tier0::exec::current_insn_index(),
                state.rip(),
                state.control.cr3
            );
            break;
        }
    }
}

#[cfg(not(test))]
fn watch_write_value() -> Option<u64> {
    *WATCH_VALUE.get_or_init(|| parse_watch("AERO_WATCH_VALUE"))
}

#[cfg(test)]
fn watch_write_value() -> Option<u64> {
    parse_watch("AERO_WATCH_VALUE")
}

#[inline]
fn watch_write(state: &CpuState, addr: u64, len: usize, val_le: u64) {
    if !mem_debug_active() {
        return;
    }
    let addrs = watch_write_addrs();
    if !addrs.is_empty() {
        let l = len as u64;
        if l > 0 {
            for &w in addrs {
                if addr <= w && w < addr + l {
                    eprintln!(
                        "[watch-addr] [{}] rip={:#x} cr3={:#x} [{addr:#x}] len={l} val={val_le:#x}",
                        crate::interp::tier0::exec::current_insn_index(),
                        state.rip(),
                        state.control.cr3
                    );
                    break;
                }
            }
        }
    }
    if let Some(v) = watch_write_value() {
        let l = len as u64;
        if l > 0 && l <= 8 {
            let mask = if l == 8 {
                u64::MAX
            } else {
                (1u64 << (l * 8)) - 1
            };
            if val_le & mask == v & mask {
                eprintln!(
                    "[watch-val] rip={:#x} cr3={:#x} [{addr:#x}] len={l} val={val_le:#x}",
                    state.rip(),
                    state.control.cr3
                );
            }
        }
    }
    write_stream_log(state, addr, len, val_le);
}

/// Optional full memory-write stream (bring-up/debug, forward-divergence tooling).
/// When `AERO_WRITE_STREAM=<path>` is set, every wrapped write appends a record
/// `rip addr len value` (hex, space-separated, in execution order) to that file.
/// This is the our-emulator half of a forward first-divergence diff (see the wiki
/// the Windows 7 bring-up record): the first record that disagrees with a reference stream is the
/// root cause. Buffered; flushed periodically and on a clean exit. Heavy — use for
/// bounded debugging runs, not production.
static WRITE_STREAM: OnceLock<Option<Mutex<std::io::BufWriter<std::fs::File>>>> = OnceLock::new();
static WRITE_STREAM_COUNT: AtomicU64 = AtomicU64::new(0);

fn write_stream() -> Option<&'static Mutex<std::io::BufWriter<std::fs::File>>> {
    WRITE_STREAM
        .get_or_init(|| {
            let path = std::env::var("AERO_WRITE_STREAM").ok()?;
            let f = std::fs::File::create(path).ok()?;
            Some(Mutex::new(std::io::BufWriter::new(f)))
        })
        .as_ref()
}

#[inline]
fn write_stream_log(state: &CpuState, addr: u64, len: usize, val_le: u64) {
    let Some(w) = write_stream() else { return };
    use std::io::Write as _;
    let mut g = w.lock().unwrap_or_else(|e| e.into_inner());
    let _ = writeln!(g, "{:#x} {:#x} {} {:#x}", state.rip(), addr, len, val_le);
    // Flush periodically so the stream survives a hard kill / timeout.
    if WRITE_STREAM_COUNT
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(1 << 20)
    {
        let _ = g.flush();
    }
}

#[inline]
fn wrapped_segment_len(state: &CpuState, raw_addr: u64, remaining: usize) -> usize {
    debug_assert!(remaining > 0);

    // Long mode: no architectural masking. The only discontinuity comes from
    // overflowing the u64 address space.
    if state.mode == CpuMode::Long {
        let rem_u64 = u64::try_from(remaining).unwrap_or(u64::MAX);
        if raw_addr.checked_add(rem_u64.saturating_sub(1)).is_some() {
            return remaining;
        }

        // Safe: overflow implies `raw_addr != 0`, so `u64::MAX - raw_addr + 1` fits in u64.
        let until_wrap = (u64::MAX - raw_addr) + 1;
        let until_wrap_usize = usize::try_from(until_wrap).unwrap_or(usize::MAX);
        return remaining.min(until_wrap_usize);
    }

    // Non-long modes: linear addresses are 32-bit.
    let addr32 = raw_addr & 0xFFFF_FFFF;
    let mut max = 0x1_0000_0000u64 - addr32; // bytes until 32-bit wrap (inclusive)

    // With A20 disabled (real/v8086): bit 20 is forced low. Contiguity in masked
    // address space requires staying within a single 1MiB window (no bit-20
    // boundary crossing).
    if !state.a20_enabled && matches!(state.mode, CpuMode::Real | CpuMode::Vm86) {
        let low20 = addr32 & 0x000F_FFFF;
        let until_a20 = 0x1_00000u64 - low20;
        max = max.min(until_a20);
    }

    let max_usize = usize::try_from(max).unwrap_or(usize::MAX);
    remaining.min(max_usize)
}

#[inline]
pub(crate) fn contiguous_masked_start(state: &CpuState, addr: u64, len: usize) -> Option<u64> {
    if len == 0 {
        return Some(state.apply_a20(addr));
    }

    // A wrapped range is safe to use bus bulk helpers for iff it is contiguous in
    // masked linear address space. `wrapped_segment_len` gives the maximal
    // contiguous prefix; use it as a single source of truth for contiguity.
    if wrapped_segment_len(state, addr, len) == len {
        Some(state.apply_a20(addr))
    } else {
        None
    }
}

pub fn read_bytes_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    dst: &mut [u8],
) -> Result<(), Exception> {
    if dst.is_empty() {
        return Ok(());
    }

    if let Some(start) = contiguous_masked_start(state, addr, dst.len()) {
        bus.read_bytes(start, dst)?;
        let mut first8 = [0u8; 8];
        first8[..dst.len().min(8)].copy_from_slice(&dst[..dst.len().min(8)]);
        watch_read(state, addr, dst.len(), u64::from_le_bytes(first8));
        return Ok(());
    }

    // Slow path: split into contiguous runs in masked linear address space. This
    // avoids per-byte `read_u8` overhead for large reads that cross an
    // architectural wrap boundary (32-bit linear wrap, A20 alias wrap).
    let mut offset = 0usize;
    while offset < dst.len() {
        let raw = addr.wrapping_add(offset as u64);
        let start = state.apply_a20(raw);
        let len = wrapped_segment_len(state, raw, dst.len() - offset);
        bus.read_bytes(start, &mut dst[offset..offset + len])?;
        offset += len;
    }
    let mut first8 = [0u8; 8];
    first8[..dst.len().min(8)].copy_from_slice(&dst[..dst.len().min(8)]);
    watch_read(state, addr, dst.len(), u64::from_le_bytes(first8));
    Ok(())
}

pub fn write_bytes_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    src: &[u8],
) -> Result<(), Exception> {
    if src.is_empty() {
        return Ok(());
    }

    let mut first8 = [0u8; 8];
    first8[..src.len().min(8)].copy_from_slice(&src[..src.len().min(8)]);
    watch_write(state, addr, src.len(), u64::from_le_bytes(first8));
    // For block copies (e.g. `rep movs` initializing a table), the watched value
    // may sit at any offset within the run; scan for it so it is not missed.
    if mem_debug_active() {
        if let Some(v) = watch_write_value() {
            let nbytes = (src.len().min(8)) as u32;
            let needle = &v.to_le_bytes()[..nbytes as usize];
            if nbytes > 0 && src.len() >= nbytes as usize {
                for off in 0..=(src.len() - nbytes as usize) {
                    if &src[off..off + nbytes as usize] == needle {
                        eprintln!(
                            "[watch-val] rip={:#x} [{:#x}] (block copy) val={v:#x}",
                            state.rip(),
                            addr + off as u64
                        );
                        break;
                    }
                }
            }
        }
    }

    if let Some(start) = contiguous_masked_start(state, addr, src.len()) {
        return bus.write_bytes(start, src);
    }
    // Split the access into contiguous runs in masked linear address space, then
    // preflight *all* runs before committing any writes. This keeps multi-byte
    // stores atomic w.r.t page faults even when the architectural address space
    // wraps (32-bit linear wrap, A20 alias wrap).
    //
    // Important: the number of discontinuities is tiny under x86's linear masking
    // rules (32-bit wrap + optional A20 wrap), so compute segment boundaries with
    // arithmetic rather than scanning byte-by-byte. We also avoid allocating a
    // temporary `Vec` by running two passes: preflight, then commit.

    // Pass 1: preflight all segments.
    let mut offset = 0usize;
    while offset < src.len() {
        let raw = addr.wrapping_add(offset as u64);
        let start = state.apply_a20(raw);
        let len = wrapped_segment_len(state, raw, src.len() - offset);
        bus.preflight_write_bytes(start, len)?;
        offset += len;
    }

    // Pass 2: commit writes.
    let mut offset = 0usize;
    while offset < src.len() {
        let raw = addr.wrapping_add(offset as u64);
        let start = state.apply_a20(raw);
        let len = wrapped_segment_len(state, raw, src.len() - offset);
        bus.write_bytes(start, &src[offset..offset + len])?;
        offset += len;
    }
    Ok(())
}

pub fn read_u16_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
) -> Result<u16, Exception> {
    if let Some(start) = contiguous_masked_start(state, addr, 2) {
        let v = bus.read_u16(start)?;
        watch_read(state, addr, 2, u64::from(v));
        return Ok(v);
    }
    let mut buf = [0u8; 2];
    read_bytes_wrapped(state, bus, addr, &mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

pub fn read_u32_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
) -> Result<u32, Exception> {
    if let Some(start) = contiguous_masked_start(state, addr, 4) {
        let v = bus.read_u32(start)?;
        watch_read(state, addr, 4, u64::from(v));
        return Ok(v);
    }
    let mut buf = [0u8; 4];
    read_bytes_wrapped(state, bus, addr, &mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

pub fn read_u64_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
) -> Result<u64, Exception> {
    if let Some(start) = contiguous_masked_start(state, addr, 8) {
        let v = bus.read_u64(start)?;
        watch_read(state, addr, 8, v);
        return Ok(v);
    }
    let mut buf = [0u8; 8];
    read_bytes_wrapped(state, bus, addr, &mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

pub fn read_u128_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
) -> Result<u128, Exception> {
    if let Some(start) = contiguous_masked_start(state, addr, 16) {
        let v = bus.read_u128(start)?;
        watch_read(state, addr, 16, v as u64);
        return Ok(v);
    }
    let mut buf = [0u8; 16];
    read_bytes_wrapped(state, bus, addr, &mut buf)?;
    Ok(u128::from_le_bytes(buf))
}

pub fn write_u16_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    value: u16,
) -> Result<(), Exception> {
    watch_write(state, addr, 2, u64::from(value));
    if let Some(start) = contiguous_masked_start(state, addr, 2) {
        return bus.write_u16(start, value);
    }
    write_bytes_wrapped(state, bus, addr, &value.to_le_bytes())
}

pub fn write_u32_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    value: u32,
) -> Result<(), Exception> {
    watch_write(state, addr, 4, u64::from(value));
    if let Some(start) = contiguous_masked_start(state, addr, 4) {
        return bus.write_u32(start, value);
    }
    write_bytes_wrapped(state, bus, addr, &value.to_le_bytes())
}

pub fn write_u64_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    value: u64,
) -> Result<(), Exception> {
    watch_write(state, addr, 8, value);
    if let Some(start) = contiguous_masked_start(state, addr, 8) {
        return bus.write_u64(start, value);
    }
    write_bytes_wrapped(state, bus, addr, &value.to_le_bytes())
}

pub fn write_u128_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    value: u128,
) -> Result<(), Exception> {
    watch_write(state, addr, 16, value as u64);
    if let Some(start) = contiguous_masked_start(state, addr, 16) {
        return bus.write_u128(start, value);
    }
    write_bytes_wrapped(state, bus, addr, &value.to_le_bytes())
}

pub fn fetch_wrapped<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    addr: u64,
    max_len: usize,
) -> Result<[u8; 15], Exception> {
    let len = max_len.min(15);
    if len == 0 {
        return Ok([0u8; 15]);
    }

    if let Some(start) = contiguous_masked_start(state, addr, len) {
        return bus.fetch(start, len);
    }

    let mut buf = [0u8; 15];
    // Slow path: split into contiguous runs in masked linear address space. This
    // keeps instruction fetch efficient in the rare case where the architectural
    // address space wraps (32-bit wrap in non-long modes, A20 alias wrap).
    let mut offset = 0usize;
    while offset < len {
        let raw = addr.wrapping_add(offset as u64);
        let start = state.apply_a20(raw);
        let seg_len = wrapped_segment_len(state, raw, len - offset);
        // Use `CpuBus::fetch` (execute access) even on the slow path so paging-aware
        // busses (NX bit, supervisor execute restrictions) observe the correct access type.
        let chunk = bus.fetch(start, seg_len)?;
        buf[offset..offset + seg_len].copy_from_slice(&chunk[..seg_len]);
        offset += seg_len;
    }
    Ok(buf)
}

/// Instruction fetch helper for segmented code: fetches bytes starting at `seg_base + ip`.
///
/// This is similar to [`fetch_wrapped`], but additionally models 16-bit IP wrapping within the
/// segment when executing 16-bit code (real/v8086 or 16-bit protected mode):
///
/// `linear[i] = seg_base + ((ip + i) & 0xFFFF)`
///
/// Each resulting linear address is then passed through [`CpuState::apply_a20`] so the fetch still
/// observes 32-bit linear wrapping (non-long modes) and the A20 alias boundary (real/v8086).
///
/// Note: This is only used for instruction fetch; data accesses use address-size-specific offset
/// masking at the effective-address calculation stage.
pub(crate) fn fetch_wrapped_seg_ip<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    seg_base: u64,
    ip: u64,
    max_len: usize,
) -> Result<[u8; 15], Exception> {
    let bitness = state.bitness();
    if bitness != 16 {
        let addr = state.apply_a20(seg_base.wrapping_add(ip));
        return fetch_wrapped(state, bus, addr, max_len);
    }

    let len = max_len.min(15);
    if len == 0 {
        return Ok([0u8; 15]);
    }

    let ip = ip & 0xFFFF;
    let mut buf = [0u8; 15];

    let mut offset = 0usize;
    while offset < len {
        let ip_off = ip.wrapping_add(offset as u64) & 0xFFFF;
        let raw_addr = seg_base.wrapping_add(ip_off);

        // Limit the segment so we don't cross the 16-bit IP boundary.
        let remaining = len - offset;
        let until_ip_wrap = (0x1_0000u64 - ip_off).min(remaining as u64) as usize;

        // Further split by linear wrapping rules (32-bit wrap, A20 alias wrap, u64 overflow).
        let seg_len = wrapped_segment_len(state, raw_addr, until_ip_wrap);

        let start = state.apply_a20(raw_addr);
        let chunk = bus.fetch(start, seg_len)?;
        buf[offset..offset + seg_len].copy_from_slice(&chunk[..seg_len]);
        offset += seg_len;
    }

    Ok(buf)
}

/// Fetch the largest decode window that remains within the current 4 KiB page
/// and architectural address run.
///
/// Decoder lookahead is not an architectural memory access: a complete
/// instruction at the end of a mapped page must execute even when the following
/// page is not present. Callers should first try to decode the returned prefix
/// and request the full 15-byte window with [`fetch_wrapped_seg_ip`] only when
/// the decoder reports that this prefix is incomplete.
#[inline]
pub(crate) fn fetch_wrapped_seg_ip_first_page<B: CpuBus>(
    state: &CpuState,
    bus: &mut B,
    seg_base: u64,
    ip: u64,
    max_len: usize,
) -> Result<([u8; 15], usize), Exception> {
    let max_len = max_len.min(15);
    if max_len == 0 {
        return Ok(([0; 15], 0));
    }

    // Windows spends essentially all of bring-up in long mode. Keep this path
    // at least as cheap as the former unconditional 15-byte fetch: long mode
    // has neither 16-bit IP wrapping nor A20/32-bit linear masking.
    if state.mode == CpuMode::Long {
        let linear = seg_base.wrapping_add(ip);
        if linear <= u64::MAX - 14 {
            let until_page_boundary = 0x1000usize - (linear as usize & 0xfff);
            let len = max_len.min(until_page_boundary);
            return Ok((bus.fetch(linear, len)?, len));
        }
    }

    let bitness = state.bitness();
    let effective_ip = if bitness == 16 { ip & 0xffff } else { ip };
    let raw_addr = seg_base.wrapping_add(effective_ip);

    let mut len = max_len;
    if bitness == 16 {
        len = len.min((0x1_0000u64 - effective_ip) as usize);
    }
    len = wrapped_segment_len(state, raw_addr, len);

    let linear = state.apply_a20(raw_addr);
    let until_page_boundary = 0x1000usize - (linear as usize & 0xfff);
    len = len.min(until_page_boundary);

    let bytes = fetch_wrapped_seg_ip(state, bus, seg_base, ip, len)?;
    Ok((bytes, len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::FlatTestBus;
    use std::sync::atomic::Ordering;

    // Positive control for the read watchpoint: a read of a watched address
    // increments the hit counter; a read of an unwatched address does not.
    #[test]
    fn watch_read_fires_only_on_covered_address() {
        std::env::set_var("AERO_WATCH_READ", "0x1000,4099"); // hex + decimal forms
        let state = CpuState::new(CpuMode::Long);
        let mut bus = FlatTestBus::new(0x4000);
        bus.load(0x1000, &0xDEAD_BEEFu32.to_le_bytes());

        let before = WATCH_READ_HITS.load(Ordering::Relaxed);
        let v = read_u32_wrapped(&state, &mut bus, 0x1000).expect("read");
        assert_eq!(v, 0xDEAD_BEEF);
        assert_eq!(
            WATCH_READ_HITS.load(Ordering::Relaxed),
            before + 1,
            "watched read should increment the hit counter"
        );

        let before2 = WATCH_READ_HITS.load(Ordering::Relaxed);
        let _ = read_u32_wrapped(&state, &mut bus, 0x2000).expect("read");
        assert_eq!(
            WATCH_READ_HITS.load(Ordering::Relaxed),
            before2,
            "unwatched read must not increment the hit counter"
        );
    }
}
