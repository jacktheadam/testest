//! Regression: after SSDT conversion Win7 RDMSR IA32_PLATFORM_ID (0x17).
use aero_cpu_core::cpuid::CpuFeatures;
use aero_cpu_core::msr::{self, MsrState};

#[test]
fn platform_id_readable() {
    let msr = MsrState::default();
    assert_eq!(msr.read(msr::IA32_PLATFORM_ID).unwrap(), 0);
    let mut msr = MsrState::default();
    let features = CpuFeatures::default();
    msr.write(&features, msr::IA32_PLATFORM_ID, 0x1234).unwrap();
    assert_eq!(msr.read(msr::IA32_FEATURE_CONTROL).unwrap(), 0);
    assert_eq!(msr.read(msr::IA32_DEBUGCTL).unwrap(), 0);
}
