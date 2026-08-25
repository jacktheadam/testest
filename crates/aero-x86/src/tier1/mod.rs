//! Minimal Tier-1 decode / normalization layer.
//!
//! This module exists primarily to support the Tier-1 JIT front-end unit tests
//! without requiring the full interpreter decode pipeline.
//!
//! This decoder only supports a subset of x86-64 sufficient for building and
//! testing basic-block discovery + translation. It is **not** intended to be
//! complete or particularly fast.

use aero_types::{Cond, Gpr, Width};
mod decode;
mod display;

pub use decode::{decode_one, decode_one_mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reg {
    pub gpr: Gpr,
    pub width: Width,
    pub high8: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Address {
    pub base: Option<Gpr>,
    pub index: Option<Gpr>,
    pub scale: u8,
    pub disp: i32,
    pub rip_relative: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    Reg(Reg),
    Imm(u64),
    Mem(Address),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOp {
    Add,
    Adc,
    Sub,
    Sbb,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Sar,
    Imul,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftOp {
    Rol,
    Ror,
    Shl,
    Shr,
    Sar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstKind {
    Nop,
    Mov {
        dst: Operand,
        src: Operand,
        width: Width,
    },
    Lea {
        dst: Reg,
        addr: Address,
        width: Width,
    },
    Alu {
        op: AluOp,
        dst: Operand,
        src: Operand,
        width: Width,
    },
    Shift {
        op: ShiftOp,
        dst: Operand,
        count: u8,
        width: Width,
    },
    /// Group2 shift/rotate by `CL` (`0xD2` / `0xD3`).
    ShiftCl {
        op: ShiftOp,
        dst: Operand,
        width: Width,
    },
    Cmp {
        lhs: Operand,
        rhs: Operand,
        width: Width,
    },
    Test {
        lhs: Operand,
        rhs: Operand,
        width: Width,
    },
    Inc {
        dst: Operand,
        width: Width,
    },
    Dec {
        dst: Operand,
        width: Width,
    },
    Not {
        dst: Operand,
        width: Width,
    },
    /// Two's complement negate (`F6 /3`, `F7 /3`): `dst = 0 - dst`.
    Neg {
        dst: Operand,
        width: Width,
    },
    Bswap {
        dst: Reg,
        width: Width,
    },
    /// Three-operand `IMUL r, r/m, imm` (`0x69` / `0x6B`): `dst = src * imm`.
    Imul3 {
        dst: Reg,
        src: Operand,
        imm: u64,
        width: Width,
    },
    Push {
        src: Operand,
    },
    Pop {
        dst: Operand,
    },
    JmpRel {
        target: u64,
    },
    JccRel {
        cond: Cond,
        target: u64,
    },
    CallRel {
        target: u64,
    },
    Ret,
    Setcc {
        cond: Cond,
        dst: Operand,
    },
    Cmovcc {
        cond: Cond,
        dst: Reg,
        src: Operand,
        width: Width,
    },
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedInst {
    pub rip: u64,
    pub len: u8,
    pub kind: InstKind,
    next_rip: u64,
}

impl DecodedInst {
    #[must_use]
    pub fn next_rip(&self) -> u64 {
        self.next_rip
    }

    #[must_use]
    pub fn is_block_terminator(&self) -> bool {
        matches!(
            self.kind,
            InstKind::JmpRel { .. }
                | InstKind::JccRel { .. }
                | InstKind::CallRel { .. }
                | InstKind::Ret
                | InstKind::Invalid
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub message: &'static str,
}

impl std::error::Error for DecodeError {}
