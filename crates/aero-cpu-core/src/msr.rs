//! Model-specific register (MSR) state.
//!
//! Windows heavily relies on MSRs for fast system calls, GS base swapping, and
//! APIC programming. We model the subset required for Windows 7 boot/runtime.

use crate::cpuid::{bits as cpuid_bits, CpuFeatures};
use crate::state;
use crate::Exception;
use std::sync::atomic::{AtomicBool, Ordering};

/// When true (set via `AERO_IGNORE_UNKNOWN_MSRS=1`), an unhandled RDMSR returns
/// 0 and an unhandled WRMSR is a no-op, instead of raising `#GP(0)`.
///
/// This mirrors QEMU/KVM's `IGNORE_MSRS=1` bring-up aid: Windows probes many
/// MSRs whose presence is not gated on a CPUID bit the emulator advertises, and
/// faulting on each one aborts Phase-0. Default is OFF (architectural #GP) so
/// the emulator stays spec-correct unless explicitly opted in for bring-up.
///
/// Parsed once into an `AtomicBool` so the hot MSR path pays only a single
/// `Relaxed` load per access (no per-call env lookup, no `OnceLock` probe).
static IGNORE_UNKNOWN_MSRS: AtomicBool = AtomicBool::new(false);
static IGNORE_UNKNOWN_MSRS_INIT: AtomicBool = AtomicBool::new(false);

fn ignore_unknown_msrs() -> bool {
    if !IGNORE_UNKNOWN_MSRS_INIT.swap(true, Ordering::Relaxed) {
        let on = match std::env::var("AERO_IGNORE_UNKNOWN_MSRS") {
            Ok(v) => matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            ),
            Err(_) => false,
        };
        IGNORE_UNKNOWN_MSRS.store(on, Ordering::Relaxed);
    }
    IGNORE_UNKNOWN_MSRS.load(Ordering::Relaxed)
}

// Common MSR indices used by Windows 7.
pub const IA32_TSC: u32 = 0x0000_0010;
pub const IA32_APIC_BASE: u32 = 0x0000_001B;

pub const IA32_SYSENTER_CS: u32 = 0x0000_0174;
pub const IA32_SYSENTER_ESP: u32 = 0x0000_0175;
pub const IA32_SYSENTER_EIP: u32 = 0x0000_0176;

pub const IA32_EFER: u32 = 0xC000_0080;
pub const IA32_STAR: u32 = 0xC000_0081;
pub const IA32_LSTAR: u32 = 0xC000_0082;
pub const IA32_CSTAR: u32 = 0xC000_0083;
pub const IA32_FMASK: u32 = 0xC000_0084;

pub const IA32_FS_BASE: u32 = 0xC000_0100;
pub const IA32_GS_BASE: u32 = 0xC000_0101;
pub const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;
pub const IA32_TSC_AUX: u32 = 0xC000_0103;

// IA32_MISC_ENABLE — read by the Windows kernel during early init.
// Bits we report (safe defaults for bring-up):
//   bit  0: fast-string enable (default 1)
//   bit  3: TM1 enable
//   bit  7: performance monitoring (default 1)
//   bit 11: branch-trace-storage unavailable (read-only, 1)
//   bit 12: PEBS unavailable (read-only, 1)
//   bit 16: Enhanced SpeedStep enable
//   bit 20: Enhanced SpeedStep select lock
pub const IA32_MISC_ENABLE: u32 = 0x0000_01A0;
const IA32_MISC_ENABLE_DEFAULT: u64 =
    (1 << 0) | (1 << 3) | (1 << 7) | (1 << 11) | (1 << 12) | (1 << 16) | (1 << 20);

// IA32_BIOS_SIGN_ID — microcode update signature. Read by Windows kernel.
// Writes are silently ignored (microcode update trigger, not relevant for bring-up).
pub const IA32_BIOS_SIGN_ID: u32 = 0x0000_008B;

// IA32_PLATFORM_ID — package/platform id. Win7 reads this right after SSDT
// conversion while finishing processor identification (was #GP at
// 0xfffff8000be21318, ecx=0x17).
pub const IA32_PLATFORM_ID: u32 = 0x0000_0017;

// IA32_FEATURE_CONTROL — VMX/SENTER lock register; often read even when VMX
// is not advertised. Return unlocked-with-no-enables.
pub const IA32_FEATURE_CONTROL: u32 = 0x0000_003A;

// IA32_DEBUGCTL — branch-trace / LBR control. Reads/writes accepted as 0.
pub const IA32_DEBUGCTL: u32 = 0x0000_01D9;

// Memory-type / machine-check MSRs that Win7 programs once CPUID.1:EDX advertises
// MTRR/PAT/MCE/MCA. We do not model real cache attributes or MCA banks; accept
// reads/writes with safe defaults so Phase-0 can complete.
pub const IA32_MTRRCAP: u32 = 0x0000_00FE;
pub const IA32_MTRR_DEF_TYPE: u32 = 0x0000_02FF;
pub const IA32_MTRR_PHYSBASE0: u32 = 0x0000_0200;
pub const IA32_MTRR_PHYSMASK0: u32 = 0x0000_0201;
// Variable MTRR pairs 0..9 occupy 0x200..=0x213.
pub const IA32_MTRR_FIX64K_00000: u32 = 0x0000_0250;
pub const IA32_MTRR_FIX16K_80000: u32 = 0x0000_0258;
pub const IA32_MTRR_FIX16K_A0000: u32 = 0x0000_0259;
pub const IA32_MTRR_FIX4K_C0000: u32 = 0x0000_0268;
pub const IA32_MTRR_FIX4K_F8000: u32 = 0x0000_026F;
pub const IA32_PAT: u32 = 0x0000_0277;
pub const IA32_MCG_CAP: u32 = 0x0000_0179;
pub const IA32_MCG_STATUS: u32 = 0x0000_017A;
pub const IA32_MCG_CTL: u32 = 0x0000_017B;
// MCA bank MSRs: IA32_MCi_CTL/STATUS/ADDR/MISC for bank i at 0x400+4*i …
pub const IA32_MC0_CTL: u32 = 0x0000_0400;
pub const IA32_MC0_MISC_END: u32 = 0x0000_0413; // banks 0..4 fully covered

/// True if `msr` is in the MTRR / PAT / MCA range we stub for Win7 bring-up.
fn is_memory_type_or_mca_msr(msr: u32) -> bool {
    matches!(
        msr,
        IA32_MTRRCAP
            | IA32_MTRR_DEF_TYPE
            | IA32_PAT
            | IA32_MCG_CAP
            | IA32_MCG_STATUS
            | IA32_MCG_CTL
            | IA32_MTRR_FIX64K_00000
            | IA32_MTRR_FIX16K_80000
            | IA32_MTRR_FIX16K_A0000
    ) || (IA32_MTRR_PHYSBASE0..=0x0000_0213).contains(&msr)
        || (IA32_MTRR_FIX4K_C0000..=IA32_MTRR_FIX4K_F8000).contains(&msr)
        || (IA32_MC0_CTL..=IA32_MC0_MISC_END).contains(&msr)
}

fn memory_type_or_mca_read_default(msr: u32) -> u64 {
    match msr {
        // VCNT=8 variable ranges, FIX=1, WC=1 — a common QEMU/Intel-like cap.
        IA32_MTRRCAP => 0x508,
        // Default PAT: WB, WT, UC-, UC, WB, WT, UC-, UC (Intel reset value).
        IA32_PAT => 0x0007_0406_0007_0406,
        // MCG_CAP: one bank, no CTL — enough for Win7 to skip complex MCA setup.
        IA32_MCG_CAP => 0x1,
        _ => 0,
    }
}

// IA32_EFER bits (subset).
pub const EFER_SCE: u64 = 1 << 0;
pub const EFER_LME: u64 = 1 << 8;
pub const EFER_LMA: u64 = 1 << 10;
pub const EFER_NXE: u64 = 1 << 11;

/// Canonical MSR storage used by [`crate::state::CpuState`].
///
/// We keep the backing fields inside `state::CpuState` so the interpreter/JIT ABI
/// remains stable, but expose MSR indices and helpers from this module.
pub type MsrState = state::MsrState;

impl state::MsrState {
    /// Read an MSR value.
    ///
    /// Unknown MSRs raise `#GP(0)` instead of being silently ignored.
    pub fn read(&self, msr: u32) -> Result<u64, Exception> {
        match msr {
            IA32_EFER => Ok(self.efer),
            IA32_STAR => Ok(self.star),
            IA32_LSTAR => Ok(self.lstar),
            IA32_CSTAR => Ok(self.cstar),
            IA32_FMASK => Ok(self.fmask),
            IA32_SYSENTER_CS => Ok(self.sysenter_cs),
            IA32_SYSENTER_ESP => Ok(self.sysenter_esp),
            IA32_SYSENTER_EIP => Ok(self.sysenter_eip),
            IA32_FS_BASE => Ok(self.fs_base),
            IA32_GS_BASE => Ok(self.gs_base),
            IA32_KERNEL_GS_BASE => Ok(self.kernel_gs_base),
            IA32_APIC_BASE => Ok(self.apic_base),
            IA32_TSC_AUX => Ok(u64::from(self.tsc_aux)),
            IA32_MISC_ENABLE => Ok(IA32_MISC_ENABLE_DEFAULT),
            IA32_BIOS_SIGN_ID => Ok(0),
            IA32_PLATFORM_ID => Ok(0),
            IA32_FEATURE_CONTROL => Ok(0), // unlocked, no enables
            IA32_DEBUGCTL => Ok(0),
            // Thermal/power MSRs adjacent to MISC_ENABLE. The Win7 HAL may probe
            // these during thermal init; returning 0 (no thermal sensors, no
            // power limits) is safe and prevents #GP on bring-up.
            0x0000_001A2 | 0x0000_01A4 | 0x0000_01A5 | 0x0000_01A6 => Ok(0),
            // EBC_FREQUENCY_ID: read by HAL clock calibration when CPUID.0x16
            // is unavailable (which it is in Aero). Return 0 = no bus ratio info.
            0x0000_002A => Ok(0),
            m if is_memory_type_or_mca_msr(m) => Ok(memory_type_or_mca_read_default(m)),
            _ if ignore_unknown_msrs() => Ok(0),
            _ => Err(Exception::gp0()),
        }
    }

    /// Write an MSR value.
    ///
    /// Unknown MSRs raise `#GP(0)`.
    pub fn write(&mut self, features: &CpuFeatures, msr: u32, value: u64) -> Result<(), Exception> {
        match msr {
            IA32_EFER => {
                // Keep CPUID/MSR coherent: if a feature is not advertised, mask its controlling
                // EFER bit rather than letting the guest enable it.
                let mut next = value;
                // LMA is read-only (controlled by paging mode); preserve the stored bit.
                next = (next & !EFER_LMA) | (self.efer & EFER_LMA);

                if (features.ext1_edx & cpuid_bits::EXT1_EDX_SYSCALL) == 0 {
                    next &= !EFER_SCE;
                }
                if (features.ext1_edx & cpuid_bits::EXT1_EDX_LM) == 0 {
                    next &= !EFER_LME;
                }
                if (features.ext1_edx & cpuid_bits::EXT1_EDX_NX) == 0 {
                    next &= !EFER_NXE;
                }

                self.efer = next;
                Ok(())
            }
            IA32_STAR => {
                self.star = value;
                Ok(())
            }
            IA32_LSTAR => {
                self.lstar = value;
                Ok(())
            }
            IA32_CSTAR => {
                self.cstar = value;
                Ok(())
            }
            IA32_FMASK => {
                self.fmask = value;
                Ok(())
            }
            IA32_SYSENTER_CS => {
                self.sysenter_cs = value;
                Ok(())
            }
            IA32_SYSENTER_ESP => {
                self.sysenter_esp = value;
                Ok(())
            }
            IA32_SYSENTER_EIP => {
                self.sysenter_eip = value;
                Ok(())
            }
            IA32_FS_BASE => {
                self.fs_base = value;
                Ok(())
            }
            IA32_GS_BASE => {
                self.gs_base = value;
                Ok(())
            }
            IA32_KERNEL_GS_BASE => {
                self.kernel_gs_base = value;
                Ok(())
            }
            IA32_APIC_BASE => {
                self.apic_base = value;
                Ok(())
            }
            IA32_TSC_AUX => {
                self.tsc_aux = (value & 0xFFFF_FFFF) as u32;
                Ok(())
            }
            IA32_MISC_ENABLE => {
                // Accept writes silently (the guest sets enable bits);
                // we don't model any of these features, so this is a no-op.
                Ok(())
            }
            IA32_BIOS_SIGN_ID => {
                // Microcode update trigger; silently accept the write.
                Ok(())
            }
            IA32_PLATFORM_ID => {
                // Read-only on real hardware for most bits; ignore writes.
                Ok(())
            }
            IA32_FEATURE_CONTROL => {
                // Lock bit is sticky on real silicon; for bring-up accept any write.
                Ok(())
            }
            IA32_DEBUGCTL => Ok(()),
            m if is_memory_type_or_mca_msr(m) => {
                // MTRR/PAT/MCA programming is accepted but not modeled: guest
                // cache attributes stay WB-equivalent for bring-up.
                let _ = m;
                let _ = value;
                let _ = features;
                Ok(())
            }
            _ if ignore_unknown_msrs() => Ok(()),
            _ => Err(Exception::gp0()),
        }
    }
}
