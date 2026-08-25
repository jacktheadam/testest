use super::ops_data::{calc_ea, op_bits, read_op_sized};
use super::ExecOutcome;
use crate::exception::{AssistReason, Exception};
use crate::linear_mem::{
    read_u16_wrapped, read_u32_wrapped, read_u64_wrapped, write_u16_wrapped, write_u32_wrapped,
    write_u64_wrapped,
};
use crate::mem::CpuBus;
use crate::state::{
    mask_bits, CpuMode, CpuState, FLAG_AF, FLAG_CF, FLAG_PF, FLAG_SF, FLAG_ZF, RFLAGS_IF,
    RFLAGS_IOPL_MASK,
};
use aero_x86::{DecodedInst, Instruction, Mnemonic, OpKind, Register};

pub fn handles_mnemonic(m: Mnemonic) -> bool {
    matches!(
        m,
        Mnemonic::Jmp
            | Mnemonic::Call
            | Mnemonic::Ret
            | Mnemonic::Retf
            | Mnemonic::Jo
            | Mnemonic::Jno
            | Mnemonic::Jb
            | Mnemonic::Jae
            | Mnemonic::Je
            | Mnemonic::Jne
            | Mnemonic::Jbe
            | Mnemonic::Ja
            | Mnemonic::Js
            | Mnemonic::Jns
            | Mnemonic::Jp
            | Mnemonic::Jnp
            | Mnemonic::Jl
            | Mnemonic::Jge
            | Mnemonic::Jle
            | Mnemonic::Jg
            | Mnemonic::Loop
            | Mnemonic::Loope
            | Mnemonic::Loopne
            | Mnemonic::Jcxz
            | Mnemonic::Jecxz
            | Mnemonic::Jrcxz
            | Mnemonic::Nop
            | Mnemonic::Pause
            | Mnemonic::Prefetchnta
            | Mnemonic::Prefetcht0
            | Mnemonic::Prefetcht1
            | Mnemonic::Prefetcht2
            | Mnemonic::Prefetchw
            | Mnemonic::Prefetch
            | Mnemonic::Clflush
            | Mnemonic::Reservednop
            | Mnemonic::Push
            | Mnemonic::Pop
            | Mnemonic::Pusha
            | Mnemonic::Pushad
            | Mnemonic::Popa
            | Mnemonic::Popad
            | Mnemonic::Pushf
            | Mnemonic::Pushfd
            | Mnemonic::Pushfq
            | Mnemonic::Popf
            | Mnemonic::Popfd
            | Mnemonic::Popfq
            | Mnemonic::Enter
            | Mnemonic::Leave
            | Mnemonic::Lahf
            | Mnemonic::Sahf
    )
}

pub fn exec<B: CpuBus>(
    state: &mut CpuState,
    bus: &mut B,
    decoded: &DecodedInst,
    next_ip: u64,
) -> Result<ExecOutcome, Exception> {
    let instr = &decoded.instr;
    match instr.mnemonic() {
        Mnemonic::Nop | Mnemonic::Pause => Ok(ExecOutcome::Continue),
        // Cache-control hints. PREFETCHh (0F 18 /0-3) and PREFETCH/W (0F 0D) are
        // architecturally non-faulting hints — the memory operand is never
        // dereferenced and bad addresses are ignored — so a plain NOP is a
        // correct implementation. `Reservednop` (0F 18 /4-7, 0F 19-1F) is the
        // same shape: a multi-byte NOP encoding.
        //
        // CLFLUSH (0F AE /7) is a cache-line writeback+invalidate; with no host
        // cache coherence to model, treating it as a NOP is architecturally
        // acceptable and matches how we advertise CPUID.1:EDX.CLFSH for Win7.
        //
        // Regression witness: the Win7 x64 kernel's prefetch memcpy loop
        // (`prefetchnta [rdx+rcx*4]` at 1,673,734,232 inst of the install boot).
        Mnemonic::Prefetchnta
        | Mnemonic::Prefetcht0
        | Mnemonic::Prefetcht1
        | Mnemonic::Prefetcht2
        | Mnemonic::Prefetchw
        | Mnemonic::Prefetch
        | Mnemonic::Clflush
        | Mnemonic::Reservednop => Ok(ExecOutcome::Continue),
        Mnemonic::Jmp => {
            if is_far_branch(instr) {
                if matches!(state.mode, CpuMode::Real | CpuMode::Vm86) {
                    // Real-mode far jump (ptr16:16).
                    let selector = instr.far_branch_selector();
                    let offset = instr.far_branch16() as u64;
                    state.write_reg(Register::CS, selector as u64);
                    state.set_rip(offset);
                    return Ok(ExecOutcome::Branch);
                }

                return Ok(ExecOutcome::Assist(AssistReason::Privileged));
            }
            if instr.op_kind(0) == OpKind::Memory && is_far_ptr_mem(instr) {
                // Indirect far jump through an m16:16/m16:32 memory pointer (FF /5).
                if matches!(state.mode, CpuMode::Real | crate::state::CpuMode::Vm86) {
                    let (offset, selector) = read_far_ptr_mem(state, bus, instr, next_ip)?;
                    state.write_reg(Register::CS, selector);
                    state.set_rip(offset);
                    return Ok(ExecOutcome::Branch);
                }
                return Ok(ExecOutcome::Assist(AssistReason::Privileged));
            }
            let target = branch_target(state, bus, instr, next_ip)?;
            state.set_rip(target);
            Ok(ExecOutcome::Branch)
        }
        Mnemonic::Call => {
            if is_far_branch(instr) {
                if matches!(state.mode, CpuMode::Real | CpuMode::Vm86) {
                    // Real-mode far call (ptr16:16): push CS, push IP.
                    let selector = instr.far_branch_selector();
                    let offset = instr.far_branch16() as u64;
                    let cs = state.read_reg(Register::CS);
                    push(state, bus, cs, 2)?;
                    push(state, bus, next_ip, 2)?;
                    state.write_reg(Register::CS, selector as u64);
                    state.set_rip(offset);
                    return Ok(ExecOutcome::Branch);
                }
                return Ok(ExecOutcome::Assist(AssistReason::Privileged));
            }
            if instr.op_kind(0) == OpKind::Memory && is_far_ptr_mem(instr) {
                // Indirect far call through an m16:16/m16:32 memory pointer (FF /3).
                if matches!(state.mode, CpuMode::Real | crate::state::CpuMode::Vm86) {
                    let (offset, selector) = read_far_ptr_mem(state, bus, instr, next_ip)?;
                    let cs = state.read_reg(Register::CS);
                    push(state, bus, cs, 2)?;
                    push(state, bus, next_ip, 2)?;
                    state.write_reg(Register::CS, selector);
                    state.set_rip(offset);
                    return Ok(ExecOutcome::Branch);
                }
                return Ok(ExecOutcome::Assist(AssistReason::Privileged));
            }
            // Intel orders the near indirect CALL as: read the target operand, then
            // push the return address. Reading after the push computes rsp-relative
            // effective addresses (e.g. `call qword ptr [rsp+0x80]`) against the
            // already-decremented stack pointer, fetching the target one slot low
            // (8 bytes in 64-bit, 4 in 32-bit, 2 in 16-bit).
            //
            // Regression witness: the Win7 x64 boot loader's import-thunk dispatch
            // (`ff 94 24 80 00 00 00`) read `[rsp+0x78]` instead of `[rsp+0x80]`
            // and called into a pointer-table data page.
            let target = branch_target(state, bus, instr, next_ip)?;
            let ret_size = state.bitness() / 8;
            push(state, bus, next_ip, ret_size)?;
            state.set_rip(target);
            Ok(ExecOutcome::Branch)
        }
        Mnemonic::Ret => {
            let pop_imm = if instr.op_count() == 1 && instr.op_kind(0) == OpKind::Immediate16 {
                instr.immediate16() as u32
            } else {
                0
            };
            let ret_size = state.bitness() / 8;
            let target = pop(state, bus, ret_size)? & mask_bits(state.bitness());
            let sp = state.stack_ptr().wrapping_add(pop_imm as u64);
            state.set_stack_ptr(sp);
            state.set_rip(target);
            Ok(ExecOutcome::Branch)
        }
        Mnemonic::Retf => {
            if matches!(state.mode, CpuMode::Real | crate::state::CpuMode::Vm86) {
                // Real-mode far return: pop offset, pop selector (16-bit each in 16-bit mode;
                // 0x66 makes the offset 32-bit), plus optional imm16 stack adjustment.
                let pop_imm = if instr.op_count() == 1 && instr.op_kind(0) == OpKind::Immediate16 {
                    instr.immediate16() as u32
                } else {
                    0
                };
                let off_size = match instr.code() {
                    aero_x86::Code::Retfd | aero_x86::Code::Retfd_imm16 => 4,
                    aero_x86::Code::Retfq | aero_x86::Code::Retfq_imm16 => 8,
                    _ => 2,
                };
                let offset = pop(state, bus, off_size)? & mask_bits(state.bitness());
                let selector = pop(state, bus, 2)?;
                if std::env::var_os("AERO_SEG_DEBUG").is_some() {
                    eprintln!(
                        "[retf] popped offset={offset:#x} selector={selector:#06x} (cpl={} cs={:#06x} sp={:#x})",
                        state.cpl(),
                        state.read_reg(Register::CS),
                        state.stack_ptr(),
                    );
                }
                let sp = state.stack_ptr().wrapping_add(pop_imm as u64);
                state.set_stack_ptr(sp);
                state.write_reg(Register::CS, selector);
                state.set_rip(offset);
                return Ok(ExecOutcome::Branch);
            }
            Ok(ExecOutcome::Assist(AssistReason::Privileged))
        }
        Mnemonic::Lahf => {
            // AH := SF:ZF:0:AF:0:PF:1:CF. Windows MBR uses this to keep INT 13h
            // CF across `add sp, 10h` after the EDD DAP teardown.
            let flags = state.rflags();
            let ah = ((flags & FLAG_CF) as u8)
                | 0x02
                | if flags & FLAG_PF != 0 { 0x04 } else { 0 }
                | if flags & FLAG_AF != 0 { 0x10 } else { 0 }
                | if flags & FLAG_ZF != 0 { 0x40 } else { 0 }
                | if flags & FLAG_SF != 0 { 0x80 } else { 0 };
            state.write_reg(Register::AH, u64::from(ah));
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Sahf => {
            let ah = state.read_reg(Register::AH) as u8;
            state.set_flag(FLAG_CF, ah & 0x01 != 0);
            state.set_flag(FLAG_PF, ah & 0x04 != 0);
            state.set_flag(FLAG_AF, ah & 0x10 != 0);
            state.set_flag(FLAG_ZF, ah & 0x40 != 0);
            state.set_flag(FLAG_SF, ah & 0x80 != 0);
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Push => {
            let bits = op_bits(state, instr, 0)?;
            let v = read_op_sized(state, bus, instr, 0, bits, next_ip)?;
            push(state, bus, v, bits / 8)?;
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Enter => {
            // ENTER imm16, imm8: create a nested stack frame.
            //
            // Semantics (all stack-address arithmetic uses the stack address size):
            //   push rbp (operand size)
            //   frame_temp := rsp
            //   if nesting > 0: push (nesting-1) saved frame pointers read from SS:rbp, then
            //   push frame_temp
            //   rbp := frame_temp
            //   rsp -= imm16
            let frame_size = instr.immediate16() as u64;
            let level = u32::from(instr.immediate8_2nd() & 0x1f);
            let size = state.bitness() / 8;
            let bp_reg = match state.bitness() {
                16 => Register::BP,
                32 => Register::EBP,
                _ => Register::RBP,
            };
            let sp_bits = state.stack_ptr_bits();
            let saved_bp = state.read_reg(bp_reg) & mask_bits(state.bitness());
            push(state, bus, saved_bp, size)?;
            let frame_temp = state.stack_ptr() & mask_bits(sp_bits);
            if level > 0 {
                let mut tmp_bp = saved_bp & mask_bits(sp_bits);
                for _ in 1..level {
                    tmp_bp = tmp_bp.wrapping_sub(size as u64) & mask_bits(sp_bits);
                    let addr =
                        state.apply_a20(state.seg_base_reg(Register::SS).wrapping_add(tmp_bp));
                    let v = match size {
                        2 => read_u16_wrapped(state, bus, addr)? as u64,
                        4 => read_u32_wrapped(state, bus, addr)? as u64,
                        8 => read_u64_wrapped(state, bus, addr)?,
                        _ => return Err(Exception::InvalidOpcode),
                    };
                    push(state, bus, v, size)?;
                }
                push(state, bus, frame_temp, size)?;
            }
            state.write_reg(bp_reg, frame_temp);
            let sp = state.stack_ptr().wrapping_sub(frame_size) & mask_bits(sp_bits);
            state.set_stack_ptr(sp);
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Leave => {
            // LEAVE: rsp := rbp (stack address size), then pop rbp (operand size).
            let sp_bits = state.stack_ptr_bits();
            let bp_reg = match state.bitness() {
                16 => Register::BP,
                32 => Register::EBP,
                _ => Register::RBP,
            };
            let bp = state.read_reg(bp_reg) & mask_bits(sp_bits);
            state.set_stack_ptr(bp);
            let size = state.bitness() / 8;
            let v = pop(state, bus, size)?;
            state.write_reg(bp_reg, v & mask_bits(state.bitness()));
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Pop => {
            if state.mode != crate::state::CpuMode::Bit16
                && instr.op_kind(0) == OpKind::Register
                && is_seg_reg(instr.op0_register())
            {
                return Ok(ExecOutcome::Assist(AssistReason::Privileged));
            }
            let bits = op_bits(state, instr, 0)?;
            let v = pop(state, bus, bits / 8)?;
            super::ops_data::write_op_sized(state, bus, instr, 0, v, bits, next_ip)?;
            // `POP SS` creates an interrupt shadow for the following instruction.
            if instr.op_kind(0) == OpKind::Register && instr.op0_register() == Register::SS {
                Ok(ExecOutcome::ContinueInhibitInterrupts)
            } else {
                Ok(ExecOutcome::Continue)
            }
        }
        Mnemonic::Pusha | Mnemonic::Pushad => {
            // Operand size comes from the mnemonic (66-prefix), not CS bitness.
            // Windows 7's MBR runs `66 60` / `66 61` (PUSHAD/POPAD) in 16-bit
            // real mode; treating those as #UD parks RIP at 0x659 forever.
            // PUSHA/PUSHAD are invalid in 64-bit mode.
            if state.bitness() == 64 {
                return Err(Exception::InvalidOpcode);
            }
            let bits = match instr.mnemonic() {
                Mnemonic::Pushad => 32,
                _ => 16,
            };
            let sz = bits / 8;
            let sp_before = state.stack_ptr();
            let regs = if bits == 16 {
                [
                    Register::AX,
                    Register::CX,
                    Register::DX,
                    Register::BX,
                    Register::SP,
                    Register::BP,
                    Register::SI,
                    Register::DI,
                ]
            } else {
                [
                    Register::EAX,
                    Register::ECX,
                    Register::EDX,
                    Register::EBX,
                    Register::ESP,
                    Register::EBP,
                    Register::ESI,
                    Register::EDI,
                ]
            };
            for r in regs {
                let v = if r == Register::SP || r == Register::ESP {
                    sp_before
                } else {
                    state.read_reg(r)
                };
                push(state, bus, v, sz)?;
            }
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Popa | Mnemonic::Popad => {
            if state.bitness() == 64 {
                return Err(Exception::InvalidOpcode);
            }
            let bits = match instr.mnemonic() {
                Mnemonic::Popad => 32,
                _ => 16,
            };
            let sz = bits / 8;
            let regs = if bits == 16 {
                [
                    Register::DI,
                    Register::SI,
                    Register::BP,
                    Register::SP,
                    Register::BX,
                    Register::DX,
                    Register::CX,
                    Register::AX,
                ]
            } else {
                [
                    Register::EDI,
                    Register::ESI,
                    Register::EBP,
                    Register::ESP,
                    Register::EBX,
                    Register::EDX,
                    Register::ECX,
                    Register::EAX,
                ]
            };
            for r in regs {
                let v = pop(state, bus, sz)?;
                // POPA/POPAD ignore the value popped into SP/ESP.
                if r == Register::SP || r == Register::ESP {
                    continue;
                }
                state.write_reg(r, v);
            }
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Pushf | Mnemonic::Pushfd | Mnemonic::Pushfq => {
            // PUSHF width is encoded in the mnemonic itself (16/32/64), including the effects of
            // any 0x66 operand-size override.
            let bits = match instr.mnemonic() {
                Mnemonic::Pushfd => 32,
                Mnemonic::Pushfq => 64,
                _ => 16,
            };
            let v = state.rflags() & mask_bits(bits);
            push(state, bus, v, bits / 8)?;
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Popf | Mnemonic::Popfd | Mnemonic::Popfq => {
            // POPF width from the mnemonic (16/32/64), same as PUSHF.
            let bits = match instr.mnemonic() {
                Mnemonic::Popfd => 32,
                Mnemonic::Popfq => 64,
                _ => 16,
            };
            let v = pop(state, bus, bits / 8)? & mask_bits(bits);
            let old = state.rflags();

            // POPF can update more than the arithmetic status flags. In particular, real-mode
            // firmware and OS kernels frequently use `pushf; cli; ...; popf` to restore IF.
            //
            // In protected/long mode, updates to IOPL/IF are privilege gated:
            // - IOPL can only be modified at CPL0.
            // - IF can only be modified when CPL <= IOPL.
            let mut write_mask = crate::state::FLAG_CF
                | crate::state::FLAG_PF
                | crate::state::FLAG_AF
                | crate::state::FLAG_ZF
                | crate::state::FLAG_SF
                | crate::state::RFLAGS_TF
                | crate::state::FLAG_DF
                | crate::state::FLAG_OF
                | RFLAGS_IF
                | RFLAGS_IOPL_MASK
                | crate::state::RFLAGS_AC
                | crate::state::RFLAGS_ID;
            write_mask &= mask_bits(bits);

            // Do not allow POPF to toggle virtualization/virtual interrupt bits in this model.
            write_mask &=
                !(crate::state::RFLAGS_VM | crate::state::RFLAGS_VIF | crate::state::RFLAGS_VIP);

            if !matches!(state.mode, CpuMode::Real | CpuMode::Vm86) {
                let cpl = state.cpl();
                if cpl != 0 {
                    write_mask &= !RFLAGS_IOPL_MASK;
                }

                let iopl = ((old & RFLAGS_IOPL_MASK) >> 12) as u8;
                if cpl > iopl {
                    write_mask &= !RFLAGS_IF;
                }
            }

            let new = (old & !write_mask) | (v & write_mask);
            state.set_rflags(new);
            Ok(ExecOutcome::Continue)
        }
        Mnemonic::Loop | Mnemonic::Loope | Mnemonic::Loopne => {
            let addr_bits = state.bitness();
            let reg = match addr_bits {
                16 => Register::CX,
                32 => Register::ECX,
                64 => Register::RCX,
                _ => Register::ECX,
            };
            let mut count = state.read_reg(reg) & mask_bits(addr_bits);
            count = count.wrapping_sub(1) & mask_bits(addr_bits);
            state.write_reg(reg, count);
            let zf = state.get_flag(FLAG_ZF);
            let cond = match instr.mnemonic() {
                Mnemonic::Loop => count != 0,
                Mnemonic::Loope => count != 0 && zf,
                Mnemonic::Loopne => count != 0 && !zf,
                _ => false,
            };
            if cond {
                let target = instr.near_branch_target();
                state.set_rip(target);
            } else {
                state.set_rip(next_ip);
            }
            Ok(ExecOutcome::Branch)
        }
        Mnemonic::Jcxz | Mnemonic::Jecxz | Mnemonic::Jrcxz => {
            let (reg, bits) = match instr.mnemonic() {
                Mnemonic::Jcxz => (Register::CX, 16),
                Mnemonic::Jecxz => (Register::ECX, 32),
                Mnemonic::Jrcxz => (Register::RCX, 64),
                _ => (Register::ECX, 32),
            };
            let v = state.read_reg(reg) & mask_bits(bits);
            if v == 0 {
                state.set_rip(instr.near_branch_target());
            } else {
                state.set_rip(next_ip);
            }
            Ok(ExecOutcome::Branch)
        }
        m if is_jcc(m) => {
            if super::ops_data::eval_cond(state, m) {
                state.set_rip(instr.near_branch_target());
            } else {
                state.set_rip(next_ip);
            }
            Ok(ExecOutcome::Branch)
        }
        _ => Err(Exception::InvalidOpcode),
    }
}

fn is_far_branch(instr: &Instruction) -> bool {
    matches!(instr.op_kind(0), OpKind::FarBranch16 | OpKind::FarBranch32)
}

fn is_seg_reg(reg: Register) -> bool {
    matches!(
        reg,
        Register::ES | Register::CS | Register::SS | Register::DS | Register::FS | Register::GS
    )
}

/// Whether the instruction's memory operand is a far pointer (ptr16:16/32/64).
fn is_far_ptr_mem(instr: &Instruction) -> bool {
    matches!(
        instr.memory_size(),
        aero_x86::MemorySize::SegPtr16
            | aero_x86::MemorySize::SegPtr32
            | aero_x86::MemorySize::SegPtr64
    )
}

/// Read a far pointer (offset + selector) from the instruction's memory operand.
fn read_far_ptr_mem<B: CpuBus>(
    state: &mut CpuState,
    bus: &mut B,
    instr: &Instruction,
    next_ip: u64,
) -> Result<(u64, u64), Exception> {
    let off_bits = match instr.memory_size() {
        aero_x86::MemorySize::SegPtr16 => 16,
        aero_x86::MemorySize::SegPtr32 => 32,
        aero_x86::MemorySize::SegPtr64 => 64,
        _ => return Err(Exception::InvalidOpcode),
    };
    let addr = calc_ea(state, instr, next_ip, true)?;
    let off = super::ops_data::read_mem(state, bus, addr, off_bits)?;
    let sel = super::ops_data::read_mem(state, bus, addr.wrapping_add((off_bits / 8) as u64), 16)?;
    Ok((off & mask_bits(off_bits), sel))
}

fn is_jcc(m: Mnemonic) -> bool {
    matches!(
        m,
        Mnemonic::Jo
            | Mnemonic::Jno
            | Mnemonic::Jb
            | Mnemonic::Jae
            | Mnemonic::Je
            | Mnemonic::Jne
            | Mnemonic::Jbe
            | Mnemonic::Ja
            | Mnemonic::Js
            | Mnemonic::Jns
            | Mnemonic::Jp
            | Mnemonic::Jnp
            | Mnemonic::Jl
            | Mnemonic::Jge
            | Mnemonic::Jle
            | Mnemonic::Jg
    )
}

fn branch_target<B: CpuBus>(
    state: &mut CpuState,
    bus: &mut B,
    instr: &Instruction,
    next_ip: u64,
) -> Result<u64, Exception> {
    match instr.op_kind(0) {
        OpKind::NearBranch16 | OpKind::NearBranch32 | OpKind::NearBranch64 => {
            Ok(instr.near_branch_target())
        }
        OpKind::Register => Ok(state.read_reg(instr.op0_register()) & mask_bits(state.bitness())),
        OpKind::Memory => {
            let bits = op_bits(state, instr, 0)?;
            let addr = calc_ea(state, instr, next_ip, true)?;
            let v = super::ops_data::read_mem(state, bus, addr, bits)?;
            Ok(v & mask_bits(state.bitness()))
        }
        _ => Err(Exception::InvalidOpcode),
    }
}

fn push<B: CpuBus>(
    state: &mut CpuState,
    bus: &mut B,
    val: u64,
    size: u32,
) -> Result<(), Exception> {
    // Commit RSP only after the store succeeds. Decrementing first and then
    // writing left RSP permanently low when the store #PF'd (e.g. stack-grow
    // #PF on CALL's return-address push). IRET restarted the CALL with the
    // half-applied push still on the stack → double-push → /GS cookie check
    // read the wrong slot → STATUS_STACK_BUFFER_OVERRUN → SESSION5 0x71.
    let sp_bits = state.stack_ptr_bits();
    let new_sp = state.stack_ptr().wrapping_sub(size as u64) & mask_bits(sp_bits);
    let addr = state.apply_a20(state.seg_base_reg(Register::SS).wrapping_add(new_sp));
    match size {
        2 => write_u16_wrapped(state, bus, addr, val as u16)?,
        4 => write_u32_wrapped(state, bus, addr, val as u32)?,
        8 => write_u64_wrapped(state, bus, addr, val)?,
        _ => return Err(Exception::InvalidOpcode),
    }
    state.set_stack_ptr(new_sp);
    Ok(())
}

fn pop<B: CpuBus>(state: &mut CpuState, bus: &mut B, size: u32) -> Result<u64, Exception> {
    let sp_bits = state.stack_ptr_bits();
    let sp = state.stack_ptr();
    let addr = state.apply_a20(state.seg_base_reg(Register::SS).wrapping_add(sp));
    let v = match size {
        2 => read_u16_wrapped(state, bus, addr)? as u64,
        4 => read_u32_wrapped(state, bus, addr)? as u64,
        8 => read_u64_wrapped(state, bus, addr)?,
        _ => return Err(Exception::InvalidOpcode),
    };
    let new_sp = sp.wrapping_add(size as u64) & mask_bits(sp_bits);
    state.set_stack_ptr(new_sp);
    Ok(v)
}
