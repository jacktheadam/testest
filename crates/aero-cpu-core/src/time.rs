//! Virtual time source for the CPU core.
//!
//! Windows 7 userland and kernel code frequently uses `RDTSC`/`RDTSCP` for
//! profiling, spin-loop timeouts, and entropy. In a browser (or any sandboxed
//! host) we need a deterministic/virtualized timestamp counter instead of
//! relying on host cycle counters.

use std::time::Duration;

// `std::time::Instant::now()` panics on `wasm32-unknown-unknown`; use `web_time::Instant` when
// compiling for the browser/Node runtimes.
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

/// Default virtual timestamp counter frequency (3 GHz).
pub const DEFAULT_TSC_HZ: u64 = 3_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeSourceMode {
    /// TSC advances only when the emulator retires instructions/cycles.
    Deterministic,
    /// TSC is derived from host wall clock and scaled by `tsc_hz`.
    ///
    /// This mode is inherently non-deterministic and should only be used for
    /// "real time" integrations.
    WallClock { anchor: Instant, anchor_tsc: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeSource {
    tsc_hz: u64,
    tsc: u64,
    /// Monotonic virtual cycles advanced by the emulator.
    ///
    /// This is deliberately independent of the guest-visible TSC. Software may
    /// legally move `IA32_TSC` backward or forward with `WRMSR`; embeddings
    /// still need an unaffected elapsed-cycle counter to advance platform
    /// clocks without interpreting that architectural write as elapsed time.
    elapsed_cycles: u64,
    mode: TimeSourceMode,
}

impl Default for TimeSource {
    fn default() -> Self {
        Self::new_deterministic(DEFAULT_TSC_HZ)
    }
}

impl TimeSource {
    pub fn new_deterministic(tsc_hz: u64) -> Self {
        Self {
            tsc_hz,
            tsc: 0,
            elapsed_cycles: 0,
            mode: TimeSourceMode::Deterministic,
        }
    }

    pub fn new_wallclock(tsc_hz: u64) -> Self {
        let now = Instant::now();
        Self {
            tsc_hz,
            tsc: 0,
            elapsed_cycles: 0,
            mode: TimeSourceMode::WallClock {
                anchor: now,
                anchor_tsc: 0,
            },
        }
    }

    pub fn tsc_hz(&self) -> u64 {
        self.tsc_hz
    }

    pub fn set_tsc_hz(&mut self, tsc_hz: u64) {
        let current = self.read_tsc();
        self.tsc_hz = tsc_hz;
        self.set_tsc(current);
    }

    pub fn set_tsc(&mut self, tsc: u64) {
        self.tsc = tsc;

        if let TimeSourceMode::WallClock { anchor, anchor_tsc } = &mut self.mode {
            *anchor = Instant::now();
            *anchor_tsc = tsc;
        }
    }

    /// Return the monotonic number of virtual cycles explicitly advanced by
    /// the emulator.
    ///
    /// Unlike [`TimeSource::read_tsc`], this value is not changed by
    /// [`TimeSource::set_tsc`]. It is intended for short-interval accounting
    /// by machine embeddings, not as an architectural guest register.
    pub fn elapsed_cycles(&self) -> u64 {
        self.elapsed_cycles
    }

    pub fn read_tsc(&mut self) -> u64 {
        match &mut self.mode {
            TimeSourceMode::Deterministic => self.tsc,
            TimeSourceMode::WallClock { anchor, anchor_tsc } => {
                let elapsed = Instant::now().saturating_duration_since(*anchor);
                let ticks = duration_to_ticks(self.tsc_hz, elapsed);
                self.tsc = anchor_tsc.wrapping_add(ticks);
                self.tsc
            }
        }
    }

    pub fn advance_cycles(&mut self, cycles: u64) {
        self.elapsed_cycles = self.elapsed_cycles.wrapping_add(cycles);
        if matches!(&self.mode, TimeSourceMode::Deterministic) {
            self.tsc = self.tsc.wrapping_add(cycles);
            return;
        }

        // Wall-clock mode needs to re-anchor after manual advancement to keep scaling intact.
        let current = self.read_tsc();
        self.set_tsc(current.wrapping_add(cycles));
    }
}

fn duration_to_ticks(tsc_hz: u64, duration: Duration) -> u64 {
    if tsc_hz == 0 {
        return 0;
    }

    let nanos = duration.as_nanos();
    let ticks = nanos.saturating_mul(tsc_hz as u128) / 1_000_000_000u128;
    ticks.min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::TimeSource;

    #[test]
    fn architectural_tsc_writes_do_not_change_elapsed_cycle_accounting() {
        let mut time = TimeSource::new_deterministic(3_000_000_000);
        time.advance_cycles(1234);
        assert_eq!(time.elapsed_cycles(), 1234);

        time.set_tsc(7);
        assert_eq!(time.read_tsc(), 7);
        assert_eq!(
            time.elapsed_cycles(),
            1234,
            "WRMSR IA32_TSC must not masquerade as elapsed platform time"
        );

        time.advance_cycles(9);
        assert_eq!(time.read_tsc(), 16);
        assert_eq!(time.elapsed_cycles(), 1243);
    }
}
