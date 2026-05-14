use crate::memory::Memory;
use crate::{Error, Result};

pub const FLAG_CF: u32 = 0x0001;
pub const FLAG_PF: u32 = 0x0004;
pub const FLAG_AF: u32 = 0x0010;
pub const FLAG_ZF: u32 = 0x0040;
pub const FLAG_SF: u32 = 0x0080;
pub const FLAG_TF: u32 = 0x0100;
pub const FLAG_IF: u32 = 0x0200;
pub const FLAG_DF: u32 = 0x0400;
pub const FLAG_OF: u32 = 0x0800;

const X87_STATUS_C0: u16 = 1 << 8;
const X87_STATUS_C1: u16 = 1 << 9;
const X87_STATUS_C2: u16 = 1 << 10;
const X87_STATUS_C3: u16 = 1 << 14;
const X87_STATUS_RESULT: u16 = X87_STATUS_C0 | X87_STATUS_C1 | X87_STATUS_C2 | X87_STATUS_C3;
const MXCSR_DEFAULT: u32 = 0x0000_1f80;
const MXCSR_SUPPORTED_MASK: u32 = 0x0000_ffc0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reg {
    Eax = 0,
    Ecx = 1,
    Edx = 2,
    Ebx = 3,
    Esp = 4,
    Ebp = 5,
    Esi = 6,
    Edi = 7,
}

impl Reg {
    #[inline(always)]
    fn from_u3(v: u8) -> Self {
        match v & 7 {
            0 => Self::Eax,
            1 => Self::Ecx,
            2 => Self::Edx,
            3 => Self::Ebx,
            4 => Self::Esp,
            5 => Self::Ebp,
            6 => Self::Esi,
            _ => Self::Edi,
        }
    }
}

#[derive(Clone)]
pub struct Cpu {
    regs: [u32; 8],
    pub eip: u32,
    pub eflags: u32,
    seg: [u16; 6],
    seg_base: [u32; 6],
    x87: Vec<f64>,
    mmx: [u64; 8],
    x87_control: u16,
    x87_status: u16,
    mxcsr: u32,
    #[cfg(debug_assertions)]
    debug_call_stack: Vec<DebugCallFrame>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Debug)]
struct DebugCallFrame {
    call_site: u32,
    target: u32,
    ret_addr: u32,
    call_esp: u32,
    ret_stack_esp: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Continue,
    JumpedToHle,
    Halted,
}

#[derive(Clone, Copy, Debug, Default)]
struct Prefixes {
    op16: bool,
    addr16: bool,
    rep: Option<RepPrefix>,
    seg: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepPrefix {
    Repe,
    Repne,
}

#[derive(Clone, Copy, Debug)]
enum Rm {
    Reg(u8),
    Mem(u32),
}

#[derive(Clone, Copy, Debug)]
struct ModRm {
    raw: u8,
    reg: u8,
    rm: Rm,
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            regs: [0; 8],
            eip: 0,
            eflags: 0x2,
            seg: [0; 6],
            seg_base: [0; 6],
            x87: Vec::new(),
            mmx: [0; 8],
            x87_control: 0x037f,
            x87_status: 0,
            mxcsr: MXCSR_DEFAULT,
            #[cfg(debug_assertions)]
            debug_call_stack: Vec::new(),
        }
    }

    #[inline(always)]
    pub fn reg(&self, reg: Reg) -> u32 {
        self.regs[reg as usize]
    }

    #[inline(always)]
    pub fn set_reg(&mut self, reg: Reg, value: u32) {
        self.regs[reg as usize] = value;
    }

    pub fn set_segment_base(&mut self, seg: usize, base: u32) {
        self.seg_base[seg] = base;
    }

    pub(crate) fn push_x87_return(&mut self, value: f64) {
        self.x87_push(value);
    }

    pub(crate) fn pop_x87_trunc_i64(&mut self) -> i64 {
        self.x87_pop().trunc() as i64
    }

    pub(crate) fn map_x87_top(&mut self, f: impl FnOnce(f64) -> f64) {
        if let Some(value) = self.x87.last_mut() {
            *value = f(*value);
        }
    }

    pub fn step(&mut self, mem: &mut Memory) -> Result<StepOutcome> {
        let mut prefixes = Prefixes::default();
        let start_eip = self.eip;
        loop {
            let b = self.fetch_u8(mem)?;
            match b {
                // Operand-size override selects 16-bit operands in this 32-bit emulator.
                0x66 => prefixes.op16 = true,
                // Address-size override is recorded and rejected when decoding ModR/M.
                0x67 => prefixes.addr16 = true,
                // REPNE/REPNZ controls repeated CMPS/SCAS string comparisons.
                0xf2 => prefixes.rep = Some(RepPrefix::Repne),
                // REP/REPE/REPZ controls repeated string operations.
                0xf3 => prefixes.rep = Some(RepPrefix::Repe),
                // ES segment override.
                0x26 => prefixes.seg = Some(0),
                // CS segment override.
                0x2e => prefixes.seg = Some(1),
                // SS segment override.
                0x36 => prefixes.seg = Some(2),
                // DS segment override.
                0x3e => prefixes.seg = Some(3),
                // FS segment override.
                0x64 => prefixes.seg = Some(4),
                // GS segment override.
                0x65 => prefixes.seg = Some(5),
                // LOCK is accepted but ignored because execution is single-threaded.
                0xf0 => {}
                op => return self.exec_opcode(mem, prefixes, op, start_eip),
            }
        }
    }

    fn exec_opcode(
        &mut self,
        mem: &mut Memory,
        prefixes: Prefixes,
        op: u8,
        start_eip: u32,
    ) -> Result<StepOutcome> {
        let width = if prefixes.op16 { 16 } else { 32 };
        match op {
            // ADD r/m,reg and reg,r/m forms.
            0x00..=0x03 => self.exec_rm_reg_op(mem, prefixes, op, AluOp::Add)?,
            // ADD AL,imm8.
            0x04 => {
                let imm = self.fetch_u8(mem)? as u32;
                let old = self.get_reg8(0);
                let res = self.add(old as u32, imm, 8, false);
                self.set_reg8(0, res as u8);
            }
            // ADD AX/EAX,imm.
            0x05 => {
                let imm = self.fetch_imm(mem, width)?;
                let old = self.read_reg_width(Reg::Eax, width);
                let res = self.add(old, imm, width, false);
                self.write_reg_width(Reg::Eax, width, res);
            }
            // PUSH ES.
            0x06 => self.push_u32(mem, self.seg[0] as u32)?,
            // POP ES.
            0x07 => self.seg[0] = self.pop_u32(mem)? as u16,
            // OR r/m,reg and reg,r/m forms.
            0x08..=0x0b => self.exec_rm_reg_op(mem, prefixes, op, AluOp::Or)?,
            // OR AL,imm8.
            0x0c => {
                let imm = self.fetch_u8(mem)? as u32;
                let res = (self.get_reg8(0) as u32) | imm;
                self.set_logic_flags(res, 8);
                self.set_reg8(0, res as u8);
            }
            // OR AX/EAX,imm.
            0x0d => {
                let imm = self.fetch_imm(mem, width)?;
                let res = self.read_reg_width(Reg::Eax, width) | imm;
                self.set_logic_flags(res, width);
                self.write_reg_width(Reg::Eax, width, res);
            }
            // PUSH CS.
            0x0e => self.push_u32(mem, self.seg[1] as u32)?,
            // Two-byte 0F opcode escape.
            0x0f => self.exec_0f(mem, prefixes)?,
            // ADC r/m,reg and reg,r/m forms.
            0x10..=0x13 => self.exec_rm_reg_op(mem, prefixes, op - 0x10, AluOp::Adc)?,
            // ADC AL,imm8.
            0x14 => {
                let imm = self.fetch_u8(mem)? as u32;
                let res = self.add(self.get_reg8(0) as u32, imm, 8, self.flag(FLAG_CF));
                self.set_reg8(0, res as u8);
            }
            // ADC AX/EAX,imm.
            0x15 => {
                let imm = self.fetch_imm(mem, width)?;
                let res = self.add(
                    self.read_reg_width(Reg::Eax, width),
                    imm,
                    width,
                    self.flag(FLAG_CF),
                );
                self.write_reg_width(Reg::Eax, width, res);
            }
            // PUSH SS.
            0x16 => self.push_u32(mem, self.seg[2] as u32)?,
            // POP SS.
            0x17 => self.seg[2] = self.pop_u32(mem)? as u16,
            // SBB r/m,reg and reg,r/m forms.
            0x18..=0x1b => self.exec_rm_reg_op(mem, prefixes, op - 0x18, AluOp::Sbb)?,
            // SBB AL,imm8.
            0x1c => {
                let imm = self.fetch_u8(mem)? as u32;
                let res = self.sub(self.get_reg8(0) as u32, imm, 8, self.flag(FLAG_CF));
                self.set_reg8(0, res as u8);
            }
            // SBB AX/EAX,imm.
            0x1d => {
                let imm = self.fetch_imm(mem, width)?;
                let res = self.sub(
                    self.read_reg_width(Reg::Eax, width),
                    imm,
                    width,
                    self.flag(FLAG_CF),
                );
                self.write_reg_width(Reg::Eax, width, res);
            }
            // PUSH DS.
            0x1e => self.push_u32(mem, self.seg[3] as u32)?,
            // POP DS.
            0x1f => self.seg[3] = self.pop_u32(mem)? as u16,
            // AND r/m,reg and reg,r/m forms.
            0x20..=0x23 => self.exec_rm_reg_op(mem, prefixes, op - 0x20, AluOp::And)?,
            // AND AL,imm8.
            0x24 => {
                let imm = self.fetch_u8(mem)? as u32;
                let res = (self.get_reg8(0) as u32) & imm;
                self.set_logic_flags(res, 8);
                self.set_reg8(0, res as u8);
            }
            // AND AX/EAX,imm.
            0x25 => {
                let imm = self.fetch_imm(mem, width)?;
                let res = self.read_reg_width(Reg::Eax, width) & imm;
                self.set_logic_flags(res, width);
                self.write_reg_width(Reg::Eax, width, res);
            }
            // Decimal/ASCII adjust opcodes are not needed by current games.
            0x27 | 0x2f | 0x37 | 0x3f => return self.unsupported(op, start_eip),
            // SUB r/m,reg and reg,r/m forms.
            0x28..=0x2b => self.exec_rm_reg_op(mem, prefixes, op - 0x28, AluOp::Sub)?,
            // SUB AL,imm8.
            0x2c => {
                let imm = self.fetch_u8(mem)? as u32;
                let res = self.sub(self.get_reg8(0) as u32, imm, 8, false);
                self.set_reg8(0, res as u8);
            }
            // SUB AX/EAX,imm.
            0x2d => {
                let imm = self.fetch_imm(mem, width)?;
                let res = self.sub(self.read_reg_width(Reg::Eax, width), imm, width, false);
                self.write_reg_width(Reg::Eax, width, res);
            }
            // XOR r/m,reg and reg,r/m forms.
            0x30..=0x33 => self.exec_rm_reg_op(mem, prefixes, op - 0x30, AluOp::Xor)?,
            // XOR AL,imm8.
            0x34 => {
                let imm = self.fetch_u8(mem)? as u32;
                let res = (self.get_reg8(0) as u32) ^ imm;
                self.set_logic_flags(res, 8);
                self.set_reg8(0, res as u8);
            }
            // XOR AX/EAX,imm.
            0x35 => {
                let imm = self.fetch_imm(mem, width)?;
                let res = self.read_reg_width(Reg::Eax, width) ^ imm;
                self.set_logic_flags(res, width);
                self.write_reg_width(Reg::Eax, width, res);
            }
            // CMP r/m,reg and reg,r/m forms.
            0x38..=0x3b => self.exec_rm_reg_op(mem, prefixes, op - 0x38, AluOp::Cmp)?,
            // CMP AL,imm8.
            0x3c => {
                let imm = self.fetch_u8(mem)? as u32;
                let _ = self.sub(self.get_reg8(0) as u32, imm, 8, false);
            }
            // CMP AX/EAX,imm.
            0x3d => {
                let imm = self.fetch_imm(mem, width)?;
                let _ = self.sub(self.read_reg_width(Reg::Eax, width), imm, width, false);
            }
            // INC r16/r32 preserves CF.
            0x40..=0x47 => {
                let reg = Reg::from_u3(op - 0x40);
                let old = self.read_reg_width(reg, width);
                let keep_cf = self.eflags & FLAG_CF;
                let res = self.add(old, 1, width, false);
                self.eflags = (self.eflags & !FLAG_CF) | keep_cf;
                self.write_reg_width(reg, width, res);
            }
            // DEC r16/r32 preserves CF.
            0x48..=0x4f => {
                let reg = Reg::from_u3(op - 0x48);
                let old = self.read_reg_width(reg, width);
                let keep_cf = self.eflags & FLAG_CF;
                let res = self.sub(old, 1, width, false);
                self.eflags = (self.eflags & !FLAG_CF) | keep_cf;
                self.write_reg_width(reg, width, res);
            }
            // PUSH r32.
            0x50..=0x57 => self.push_u32(mem, self.reg(Reg::from_u3(op - 0x50)))?,
            // POP r32.
            0x58..=0x5f => {
                let value = self.pop_u32(mem)?;
                self.set_reg(Reg::from_u3(op - 0x58), value);
            }
            // PUSHA pushes general registers, using the original ESP value.
            0x60 => {
                let old_esp = self.reg(Reg::Esp);
                for reg in [
                    Reg::Eax,
                    Reg::Ecx,
                    Reg::Edx,
                    Reg::Ebx,
                    Reg::Esp,
                    Reg::Ebp,
                    Reg::Esi,
                    Reg::Edi,
                ] {
                    let v = if reg == Reg::Esp {
                        old_esp
                    } else {
                        self.reg(reg)
                    };
                    self.push_u32(mem, v)?;
                }
            }
            // POPA restores general registers and discards the saved ESP slot.
            0x61 => {
                let edi = self.pop_u32(mem)?;
                let esi = self.pop_u32(mem)?;
                let ebp = self.pop_u32(mem)?;
                let _skip_esp = self.pop_u32(mem)?;
                let ebx = self.pop_u32(mem)?;
                let edx = self.pop_u32(mem)?;
                let ecx = self.pop_u32(mem)?;
                let eax = self.pop_u32(mem)?;
                self.set_reg(Reg::Edi, edi);
                self.set_reg(Reg::Esi, esi);
                self.set_reg(Reg::Ebp, ebp);
                self.set_reg(Reg::Ebx, ebx);
                self.set_reg(Reg::Edx, edx);
                self.set_reg(Reg::Ecx, ecx);
                self.set_reg(Reg::Eax, eax);
            }
            // PUSH imm16/imm32.
            0x68 => {
                let imm = self.fetch_imm(mem, width)?;
                self.push_u32(mem, sign_width(imm, width))?;
            }
            // IMUL r, r/m, imm.
            0x69 => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let imm = self.fetch_imm(mem, width)?;
                let lhs = self.read_rm(mem, modrm.rm, width)?;
                let res = (sign_width(lhs, width) as i32 as i64)
                    .wrapping_mul(sign_width(imm, width) as i32 as i64)
                    as u32;
                self.write_reg_width(Reg::from_u3(modrm.reg), width, res);
                self.set_imul_flags(res, width);
            }
            // PUSH imm8 sign-extended.
            0x6a => {
                let imm = self.fetch_i8(mem)? as i32 as u32;
                self.push_u32(mem, imm)?;
            }
            // IMUL r, r/m, imm8.
            0x6b => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let imm = self.fetch_i8(mem)? as i32 as u32;
                let lhs = self.read_rm(mem, modrm.rm, width)?;
                let res =
                    (sign_width(lhs, width) as i32 as i64).wrapping_mul(imm as i32 as i64) as u32;
                self.write_reg_width(Reg::from_u3(modrm.reg), width, res);
                self.set_imul_flags(res, width);
            }
            // Jcc rel8 conditional branches.
            0x70..=0x7f => {
                let rel = self.fetch_i8(mem)? as i32;
                if self.cond(op & 0x0f) {
                    self.eip = self.eip.wrapping_add(rel as u32);
                }
            }
            // Group 1 immediate ALU: ADD/OR/ADC/SBB/AND/SUB/XOR/CMP.
            0x80 | 0x81 | 0x83 => self.exec_group1(mem, prefixes, op)?,
            // TEST r/m,reg.
            0x84 | 0x85 => {
                let bits = if op == 0x84 { 8 } else { width };
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let lhs = self.read_rm(mem, modrm.rm, bits)?;
                let rhs = self.read_reg_width(Reg::from_u3(modrm.reg), bits);
                self.set_logic_flags(lhs & rhs, bits);
            }
            // XCHG r/m,reg.
            0x86 | 0x87 => {
                let bits = if op == 0x86 { 8 } else { width };
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let lhs = self.read_rm(mem, modrm.rm, bits)?;
                let reg = Reg::from_u3(modrm.reg);
                let rhs = self.read_reg_width(reg, bits);
                self.write_rm(mem, modrm.rm, bits, rhs)?;
                self.write_reg_width(reg, bits, lhs);
            }
            // MOV r/m,reg and reg,r/m forms.
            0x88..=0x8b => self.exec_mov_rm_reg(mem, prefixes, op)?,
            // MOV r/m16,Sreg.
            0x8c => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let value = self.seg[(modrm.reg & 5) as usize] as u32;
                self.write_rm(mem, modrm.rm, 16, value)?;
            }
            // LEA computes the effective address without reading memory.
            0x8d => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let Rm::Mem(addr) = modrm.rm else {
                    return Err(Error::Cpu(format!(
                        "lea with register at {:08x}",
                        start_eip
                    )));
                };
                self.write_reg_width(Reg::from_u3(modrm.reg), width, addr);
            }
            // MOV Sreg,r/m16.
            0x8e => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let value = self.read_rm(mem, modrm.rm, 16)? as u16;
                self.seg[(modrm.reg & 5) as usize] = value;
            }
            // POP r/m.
            0x8f => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                if modrm.reg != 0 {
                    return self.unsupported(op, start_eip);
                }
                let value = self.pop_u32(mem)?;
                self.write_rm(mem, modrm.rm, width, value)?;
            }
            // NOP and WAIT/FWAIT; x87 exceptions are not modeled.
            0x90 | 0x9b => {}
            // XCHG EAX,r32.
            0x91..=0x97 => {
                let reg = Reg::from_u3(op - 0x90);
                let eax = self.reg(Reg::Eax);
                let other = self.reg(reg);
                self.set_reg(Reg::Eax, other);
                self.set_reg(reg, eax);
            }
            // SAHF copies AH into status flags.
            0x9e => {
                let ah = self.get_reg8(4);
                self.set_flag(FLAG_SF, (ah & 0x80) != 0);
                self.set_flag(FLAG_ZF, (ah & 0x40) != 0);
                self.set_flag(FLAG_AF, (ah & 0x10) != 0);
                self.set_flag(FLAG_PF, (ah & 0x04) != 0);
                self.set_flag(FLAG_CF, (ah & 0x01) != 0);
            }
            // LAHF copies status flags into AH.
            0x9f => {
                let mut ah = 0x02u8;
                if self.flag(FLAG_SF) {
                    ah |= 0x80;
                }
                if self.flag(FLAG_ZF) {
                    ah |= 0x40;
                }
                if self.flag(FLAG_AF) {
                    ah |= 0x10;
                }
                if self.flag(FLAG_PF) {
                    ah |= 0x04;
                }
                if self.flag(FLAG_CF) {
                    ah |= 0x01;
                }
                self.set_reg8(4, ah);
            }
            // CBW/CWDE sign-extends AL->AX or AX->EAX.
            0x98 => {
                if prefixes.op16 {
                    let al = self.get_reg8(0) as i8 as i16 as u16;
                    self.write_reg_width(Reg::Eax, 16, al as u32);
                } else {
                    let ax = self.read_reg_width(Reg::Eax, 16) as i16 as i32 as u32;
                    self.set_reg(Reg::Eax, ax);
                }
            }
            // CWD/CDQ sign-extends AX/EAX into DX/EDX.
            0x99 => {
                if prefixes.op16 {
                    let ax = self.read_reg_width(Reg::Eax, 16);
                    self.write_reg_width(Reg::Edx, 16, if (ax & 0x8000) != 0 { 0xffff } else { 0 });
                } else {
                    self.set_reg(
                        Reg::Edx,
                        if (self.reg(Reg::Eax) & 0x8000_0000) != 0 {
                            0xffff_ffff
                        } else {
                            0
                        },
                    );
                }
            }
            // PUSHF pushes EFLAGS with the reserved bit set.
            0x9c => self.push_u32(mem, self.eflags | 0x2)?,
            // POPF restores writable EFLAGS bits and keeps reserved bit set.
            0x9d => self.eflags = (self.pop_u32(mem)? | 0x2) & !0xffc0_0000,
            // MOV AL,moffs8.
            0xa0 => {
                let raw = self.fetch_u32(mem)?;
                let addr = self.apply_seg(prefixes, raw);
                let v = mem.read_u8(addr)?;
                self.set_reg8(0, v);
            }
            // MOV AX/EAX,moffs.
            0xa1 => {
                let raw = self.fetch_u32(mem)?;
                let addr = self.apply_seg(prefixes, raw);
                let v = if prefixes.op16 {
                    mem.read_u16(addr)? as u32
                } else {
                    mem.read_u32(addr)?
                };
                self.write_reg_width(Reg::Eax, width, v);
            }
            // MOV moffs8,AL.
            0xa2 => {
                let raw = self.fetch_u32(mem)?;
                let addr = self.apply_seg(prefixes, raw);
                mem.write_u8(addr, self.get_reg8(0))?;
            }
            // MOV moffs,AX/EAX.
            0xa3 => {
                let raw = self.fetch_u32(mem)?;
                let addr = self.apply_seg(prefixes, raw);
                if prefixes.op16 {
                    mem.write_u16(addr, self.read_reg_width(Reg::Eax, 16) as u16)?;
                } else {
                    mem.write_u32(addr, self.reg(Reg::Eax))?;
                }
            }
            // MOVS/CMPS string forms.
            0xa4..=0xa7 => self.exec_string(mem, prefixes, op)?,
            // TEST AL,imm8.
            0xa8 => {
                let imm = self.fetch_u8(mem)? as u32;
                self.set_logic_flags(self.get_reg8(0) as u32 & imm, 8);
            }
            // TEST AX/EAX,imm.
            0xa9 => {
                let imm = self.fetch_imm(mem, width)?;
                self.set_logic_flags(self.read_reg_width(Reg::Eax, width) & imm, width);
            }
            // STOS/LODS/SCAS string forms.
            0xaa..=0xaf => self.exec_string(mem, prefixes, op)?,
            // MOV r8,imm8.
            0xb0..=0xb7 => {
                let imm = self.fetch_u8(mem)?;
                self.set_reg8(op - 0xb0, imm);
            }
            // MOV r16/r32,imm.
            0xb8..=0xbf => {
                let imm = self.fetch_imm(mem, width)?;
                self.write_reg_width(Reg::from_u3(op - 0xb8), width, imm);
            }
            // Group 2 shifts/rotates with imm8 count.
            0xc0 | 0xc1 => self.exec_shift_group(mem, prefixes, op)?,
            // RET imm16 pops return address and callee-cleanup bytes.
            0xc2 => {
                let pop = self.fetch_u16(mem)? as u32;
                let ret_stack_esp = self.reg(Reg::Esp);
                let ret_addr = mem.read_u32(ret_stack_esp)?;
                self.debug_finish_call_return(start_eip, ret_addr, ret_stack_esp, pop, "ret")?;
                self.eip = ret_addr;
                self.set_reg(Reg::Esp, ret_stack_esp.wrapping_add(4).wrapping_add(pop));
            }
            // RET pops a near return address.
            0xc3 => {
                let ret_stack_esp = self.reg(Reg::Esp);
                let ret_addr = mem.read_u32(ret_stack_esp)?;
                self.debug_finish_call_return(start_eip, ret_addr, ret_stack_esp, 0, "ret")?;
                self.eip = ret_addr;
                self.set_reg(Reg::Esp, ret_stack_esp.wrapping_add(4));
            }
            // MOV r/m,imm.
            0xc6 | 0xc7 => {
                let bits = if op == 0xc6 { 8 } else { width };
                let modrm = self.fetch_modrm(mem, prefixes)?;
                if modrm.reg != 0 {
                    return self.unsupported(op, start_eip);
                }
                let imm = if bits == 8 {
                    self.fetch_u8(mem)? as u32
                } else {
                    self.fetch_imm(mem, bits)?
                };
                self.write_rm(mem, modrm.rm, bits, imm)?;
            }
            // ENTER builds a stack frame for old compiler prologues.
            0xc8 => {
                let frame = self.fetch_u16(mem)? as u32;
                let nesting = self.fetch_u8(mem)?;
                self.push_u32(mem, self.reg(Reg::Ebp))?;
                let frame_temp = self.reg(Reg::Esp);
                for _ in 1..nesting {
                    self.set_reg(Reg::Ebp, self.reg(Reg::Ebp).wrapping_sub(4));
                    self.push_u32(mem, mem.read_u32(self.reg(Reg::Ebp))?)?;
                }
                if nesting != 0 {
                    self.push_u32(mem, frame_temp)?;
                }
                self.set_reg(Reg::Ebp, frame_temp);
                self.set_reg(Reg::Esp, self.reg(Reg::Esp).wrapping_sub(frame));
            }
            // LEAVE tears down an EBP-based stack frame.
            0xc9 => {
                self.set_reg(Reg::Esp, self.reg(Reg::Ebp));
                let v = self.pop_u32(mem)?;
                self.set_reg(Reg::Ebp, v);
            }
            // INT3 halts the emulator for debugger-style traps.
            0xcc => return Ok(StepOutcome::Halted),
            // INT n is surfaced as unsupported software interrupt for now.
            0xcd => {
                let int_no = self.fetch_u8(mem)?;
                return Err(Error::Cpu(format!(
                    "software interrupt {int_no:02x} at {start_eip:08x}"
                )));
            }
            // Group 2 shifts/rotates with count 1 or CL.
            0xd0..=0xd3 => self.exec_shift_group(mem, prefixes, op)?,
            // XLATB translates AL through DS:[EBX+AL] or the segment override.
            0xd7 => {
                let offset = if prefixes.addr16 {
                    self.read_reg_width(Reg::Ebx, 16)
                        .wrapping_add(self.get_reg8(0) as u32)
                        & 0xffff
                } else {
                    self.reg(Reg::Ebx).wrapping_add(self.get_reg8(0) as u32)
                };
                self.set_reg8(0, mem.read_u8(self.apply_seg(prefixes, offset))?);
            }
            // x87 floating-point opcode range.
            0xd8..=0xdf => self.exec_x87(mem, prefixes, op, start_eip)?,
            // LOOPNZ/LOOPZ/LOOP rel8 decrement ECX and branch by condition.
            0xe0..=0xe2 => {
                let rel = self.fetch_i8(mem)? as i32;
                let ecx = self.reg(Reg::Ecx).wrapping_sub(1);
                self.set_reg(Reg::Ecx, ecx);
                let take = match op {
                    0xe0 => ecx != 0 && !self.flag(FLAG_ZF),
                    0xe1 => ecx != 0 && self.flag(FLAG_ZF),
                    _ => ecx != 0,
                };
                if take {
                    self.eip = self.eip.wrapping_add(rel as u32);
                }
            }
            // JCXZ/JECXZ rel8.
            0xe3 => {
                let rel = self.fetch_i8(mem)? as i32;
                if self.reg(Reg::Ecx) == 0 {
                    self.eip = self.eip.wrapping_add(rel as u32);
                }
            }
            // CALL rel32 records debug stack state and may enter an HLE thunk.
            0xe8 => {
                let rel = self.fetch_i32(mem)? as u32;
                let ret_addr = self.eip;
                let esp_before_call = self.reg(Reg::Esp);
                self.push_u32(mem, self.eip)?;
                let target = self.eip.wrapping_add(rel);
                self.debug_record_call(start_eip, target, ret_addr, esp_before_call)?;
                self.eip = target;
                return Ok(StepOutcome::JumpedToHle);
            }
            // JMP rel32.
            0xe9 => {
                let rel = self.fetch_i32(mem)? as u32;
                self.eip = self.eip.wrapping_add(rel);
            }
            // JMP rel8.
            0xeb => {
                let rel = self.fetch_i8(mem)? as i32 as u32;
                self.eip = self.eip.wrapping_add(rel);
            }
            // HLT stops the emulator.
            0xf4 => return Ok(StepOutcome::Halted),
            // CMC complements carry.
            0xf5 => self.set_flag(FLAG_CF, !self.flag(FLAG_CF)),
            // Group 3 TEST/NOT/NEG/MUL/IMUL/DIV/IDIV.
            0xf6 | 0xf7 => self.exec_group_f6(mem, prefixes, op)?,
            // CLC clears carry.
            0xf8 => self.set_flag(FLAG_CF, false),
            // STC sets carry.
            0xf9 => self.set_flag(FLAG_CF, true),
            // CLI clears interrupt-enable; interrupts are not modeled.
            0xfa => self.set_flag(FLAG_IF, false),
            // STI sets interrupt-enable; interrupts are not modeled.
            0xfb => self.set_flag(FLAG_IF, true),
            // CLD clears string direction.
            0xfc => self.set_flag(FLAG_DF, false),
            // STD sets string direction.
            0xfd => self.set_flag(FLAG_DF, true),
            // Group FE/FF INC/DEC/CALL/JMP/PUSH.
            0xfe | 0xff => self.exec_group_ff(mem, prefixes, op, start_eip)?,
            _ => return self.unsupported(op, start_eip),
        }
        Ok(StepOutcome::Continue)
    }

    fn exec_0f(&mut self, mem: &mut Memory, prefixes: Prefixes) -> Result<()> {
        let op = self.fetch_u8(mem)?;
        let start_eip = self.eip.wrapping_sub(2);
        let width = if prefixes.op16 { 16 } else { 32 };
        match op {
            // RDTSC returns a deterministic zero timestamp for replay stability.
            0x31 => {
                let t = 0u64;
                self.set_reg(Reg::Eax, t as u32);
                self.set_reg(Reg::Edx, (t >> 32) as u32);
            }
            // CMOVcc r16/32,r/m16/32 conditionally copies without changing flags.
            0x40..=0x4f => {
                let take = self.cond(op & 0x0f);
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let value = self.read_rm(mem, modrm.rm, width)?;
                if take {
                    self.write_reg_width(Reg::from_u3(modrm.reg), width, value);
                }
            }
            // Jcc rel32 conditional branches.
            0x80..=0x8f => {
                let rel = self.fetch_i32(mem)? as u32;
                if self.cond(op & 0x0f) {
                    self.eip = self.eip.wrapping_add(rel);
                }
            }
            // SETcc r/m8 writes 0 or 1 from the selected condition.
            0x90..=0x9f => {
                let cond = self.cond(op & 0x0f) as u32;
                let modrm = self.fetch_modrm(mem, prefixes)?;
                self.write_rm(mem, modrm.rm, 8, cond)?;
            }
            // PUSH FS.
            0xa0 => self.push_u32(mem, self.seg[4] as u32)?,
            // CPUID reports a small Pentium-class feature set.
            0xa2 => {
                let leaf = self.reg(Reg::Eax);
                match leaf {
                    0 => {
                        self.set_reg(Reg::Eax, 1);
                        self.set_reg(Reg::Ebx, 0x756e_6547);
                        self.set_reg(Reg::Edx, 0x4965_6e69);
                        self.set_reg(Reg::Ecx, 0x6c65_746e);
                    }
                    _ => {
                        self.set_reg(Reg::Eax, 0x0000_0633);
                        self.set_reg(Reg::Ebx, 0);
                        self.set_reg(Reg::Ecx, 0);
                        self.set_reg(Reg::Edx, 0x0080_0001);
                    }
                }
            }
            // MOVD mm,r/m32 transfers a scalar dword into an MMX register.
            0x6e => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                self.mmx[modrm.reg as usize] = self.read_rm(mem, modrm.rm, 32)? as u64;
            }
            // MOVQ mm,mm/m64 transfers a packed 64-bit MMX value.
            0x6f => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                self.mmx[modrm.reg as usize] = self.read_mmx_rm64(mem, modrm.rm)?;
            }
            // MOVD r/m32,mm transfers the low dword of an MMX register.
            0x7e => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                self.write_rm(mem, modrm.rm, 32, self.mmx[modrm.reg as usize] as u32)?;
            }
            // MOVQ mm/m64,mm stores a packed 64-bit MMX value.
            0x7f => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                self.write_mmx_rm64(mem, modrm.rm, self.mmx[modrm.reg as usize])?;
            }
            // MMX packed unpack/compare/logic and qword shifts used by Smacker.
            0x60 | 0x61 | 0x62 | 0x68 | 0x74 | 0xd3 | 0xdb | 0xdf | 0xeb | 0xf3 => {
                self.exec_mmx_rm_op(mem, prefixes, op)?
            }
            // MMX shift-by-immediate group.
            0x73 => self.exec_mmx_imm_shift(mem, prefixes, start_eip)?,
            // EMMS clears MMX/x87 alias state so later x87 code sees an empty stack.
            0x77 => self.x87.clear(),
            // POP FS.
            0xa1 => self.seg[4] = self.pop_u32(mem)? as u16,
            // BT/BTS/BTR/BTC bit-test group.
            0xa3 | 0xab | 0xb3 | 0xbb => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let bit = self.read_reg_width(Reg::from_u3(modrm.reg), width);
                self.exec_bit_test_modify(mem, modrm.rm, width, bit, op)?;
            }
            // Group 8 BT/BTS/BTR/BTC r/m,imm8 bit-test operations.
            0xba => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let bit = self.fetch_u8(mem)? as u32;
                let op = match modrm.reg {
                    4 => 0xa3,
                    5 => 0xab,
                    6 => 0xb3,
                    7 => 0xbb,
                    _ => return self.unsupported(op, start_eip),
                };
                self.exec_bit_test_modify(mem, modrm.rm, width, bit, op)?;
            }
            // Group 15 FXSAVE/FXRSTOR/LDMXCSR/STMXCSR/fence/cache operations.
            0xae => self.exec_group_0f_ae(mem, prefixes, start_eip)?,
            // SHLD/SHRD r/m,reg,imm8 double-precision shifts.
            0xa4 | 0xac => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let count = self.fetch_u8(mem)?;
                self.exec_double_shift(mem, modrm, width, count, op == 0xa4)?;
            }
            // SHLD/SHRD r/m,reg,CL double-precision shifts.
            0xa5 | 0xad => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let count = self.get_reg8(1);
                self.exec_double_shift(mem, modrm, width, count, op == 0xa5)?;
            }
            // PUSH GS.
            0xa8 => self.push_u32(mem, self.seg[5] as u32)?,
            // POP GS.
            0xa9 => self.seg[5] = self.pop_u32(mem)? as u16,
            // IMUL r, r/m two-operand signed multiply.
            0xaf => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let lhs = self.read_reg_width(Reg::from_u3(modrm.reg), width);
                let rhs = self.read_rm(mem, modrm.rm, width)?;
                let res = (sign_width(lhs, width) as i32 as i64)
                    .wrapping_mul(sign_width(rhs, width) as i32 as i64)
                    as u32;
                self.write_reg_width(Reg::from_u3(modrm.reg), width, res);
                self.set_imul_flags(res, width);
            }
            // CMPXCHG r/m,reg compares with AL/AX/EAX and stores on equality.
            0xb0 | 0xb1 => {
                let bits = if op == 0xb0 { 8 } else { width };
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let dst = self.read_rm(mem, modrm.rm, bits)?;
                let acc = self.read_reg_width(Reg::Eax, bits);
                if acc == dst {
                    let src = self.read_reg_width(Reg::from_u3(modrm.reg), bits);
                    self.write_rm(mem, modrm.rm, bits, src)?;
                    self.set_flag(FLAG_ZF, true);
                } else {
                    self.write_reg_width(Reg::Eax, bits, dst);
                    self.set_flag(FLAG_ZF, false);
                }
            }
            // XADD r/m,reg stores r/m+reg in r/m and the old r/m in reg.
            0xc0 | 0xc1 => {
                let bits = if op == 0xc0 { 8 } else { width };
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let reg = Reg::from_u3(modrm.reg);
                let dst = self.read_rm(mem, modrm.rm, bits)?;
                let src = self.read_reg_width(reg, bits);
                let res = self.add(dst, src, bits, false);
                self.write_rm(mem, modrm.rm, bits, res)?;
                self.write_reg_width(reg, bits, dst);
            }
            // MOVZX/MOVSX byte/word into word/dword.
            0xb6 | 0xb7 | 0xbe | 0xbf => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let src_bits = if op == 0xb6 || op == 0xbe { 8 } else { 16 };
                let src = self.read_rm(mem, modrm.rm, src_bits)?;
                let value = if op == 0xbe || op == 0xbf {
                    sign_width(src, src_bits)
                } else {
                    src
                };
                self.write_reg_width(Reg::from_u3(modrm.reg), width, value);
            }
            // BSF/BSR r, r/m scans for the first or last set bit; only ZF is defined.
            0xbc | 0xbd => {
                let modrm = self.fetch_modrm(mem, prefixes)?;
                let src = self.read_rm(mem, modrm.rm, width)? & width_mask(width);
                if src == 0 {
                    self.set_flag(FLAG_ZF, true);
                } else {
                    let index = if op == 0xbc {
                        src.trailing_zeros()
                    } else {
                        31 - src.leading_zeros()
                    };
                    self.write_reg_width(Reg::from_u3(modrm.reg), width, index);
                    self.set_flag(FLAG_ZF, false);
                }
            }
            // BSWAP r32 reverses register byte order; flags are unchanged.
            0xc8..=0xcf => {
                let reg = Reg::from_u3(op - 0xc8);
                self.set_reg(reg, self.reg(reg).swap_bytes());
            }
            _ => {
                return Err(Error::Cpu(format!(
                    "unsupported opcode 0f {op:02x} at {:08x}",
                    self.eip.wrapping_sub(2)
                )));
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn exec_mov_rm_reg(&mut self, mem: &mut Memory, prefixes: Prefixes, op: u8) -> Result<()> {
        let bits = if op == 0x88 || op == 0x8a {
            8
        } else if prefixes.op16 {
            16
        } else {
            32
        };
        let modrm = self.fetch_modrm(mem, prefixes)?;
        let reg = Reg::from_u3(modrm.reg);
        match op {
            // MOV r/m,reg stores the register into memory or another register.
            0x88 | 0x89 => {
                let value = self.read_reg_width(reg, bits);
                self.write_rm(mem, modrm.rm, bits, value)?;
            }
            // MOV reg,r/m loads from memory or another register.
            0x8a | 0x8b => {
                let value = self.read_rm(mem, modrm.rm, bits)?;
                self.write_reg_width(reg, bits, value);
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    #[inline(always)]
    fn exec_rm_reg_op(
        &mut self,
        mem: &mut Memory,
        prefixes: Prefixes,
        op: u8,
        alu: AluOp,
    ) -> Result<()> {
        let bits = if (op & 1) == 0 {
            8
        } else if prefixes.op16 {
            16
        } else {
            32
        };
        let modrm = self.fetch_modrm(mem, prefixes)?;
        let direction_to_reg = (op & 2) != 0;
        let reg = Reg::from_u3(modrm.reg);
        let (lhs, rhs) = if direction_to_reg {
            (
                self.read_reg_width(reg, bits),
                self.read_rm(mem, modrm.rm, bits)?,
            )
        } else {
            (
                self.read_rm(mem, modrm.rm, bits)?,
                self.read_reg_width(reg, bits),
            )
        };
        let value = self.exec_alu(alu, lhs, rhs, bits);
        if alu != AluOp::Cmp {
            if direction_to_reg {
                self.write_reg_width(reg, bits, value);
            } else {
                self.write_rm(mem, modrm.rm, bits, value)?;
            }
        }
        Ok(())
    }

    fn exec_group1(&mut self, mem: &mut Memory, prefixes: Prefixes, op: u8) -> Result<()> {
        let bits = if op == 0x80 {
            8
        } else if prefixes.op16 {
            16
        } else {
            32
        };
        let modrm = self.fetch_modrm(mem, prefixes)?;
        let lhs = self.read_rm(mem, modrm.rm, bits)?;
        let imm = match op {
            0x80 => self.fetch_u8(mem)? as u32,
            0x81 => self.fetch_imm(mem, bits)?,
            0x83 => sign_width(self.fetch_u8(mem)? as u32, 8),
            _ => unreachable!(),
        };
        let alu = match modrm.reg {
            // /0 ADD r/m,imm.
            0 => AluOp::Add,
            // /1 OR r/m,imm.
            1 => AluOp::Or,
            // /2 ADC r/m,imm.
            2 => AluOp::Adc,
            // /3 SBB r/m,imm.
            3 => AluOp::Sbb,
            // /4 AND r/m,imm.
            4 => AluOp::And,
            // /5 SUB r/m,imm.
            5 => AluOp::Sub,
            // /6 XOR r/m,imm.
            6 => AluOp::Xor,
            // /7 CMP r/m,imm.
            7 => AluOp::Cmp,
            _ => unreachable!(),
        };
        let value = self.exec_alu(alu, lhs, imm, bits);
        if alu != AluOp::Cmp {
            self.write_rm(mem, modrm.rm, bits, value)?;
        }
        Ok(())
    }

    fn exec_group_f6(&mut self, mem: &mut Memory, prefixes: Prefixes, op: u8) -> Result<()> {
        let bits = if op == 0xf6 {
            8
        } else if prefixes.op16 {
            16
        } else {
            32
        };
        let modrm = self.fetch_modrm(mem, prefixes)?;
        match modrm.reg {
            // /0 and /1 TEST r/m,imm.
            0 | 1 => {
                let lhs = self.read_rm(mem, modrm.rm, bits)?;
                let imm = if bits == 8 {
                    self.fetch_u8(mem)? as u32
                } else {
                    self.fetch_imm(mem, bits)?
                };
                self.set_logic_flags(lhs & imm, bits);
            }
            // /2 NOT r/m.
            2 => {
                let v = !self.read_rm(mem, modrm.rm, bits)?;
                self.write_rm(mem, modrm.rm, bits, mask_width(v, bits))?;
            }
            // /3 NEG r/m.
            3 => {
                let lhs = self.read_rm(mem, modrm.rm, bits)?;
                let value = self.sub(0, lhs, bits, false);
                self.write_rm(mem, modrm.rm, bits, value)?;
            }
            // /4 MUL unsigned accumulator by r/m.
            4 => {
                let rhs = self.read_rm(mem, modrm.rm, bits)? as u64;
                if bits == 8 {
                    let res = self.get_reg8(0) as u64 * rhs;
                    self.write_reg_width(Reg::Eax, 16, res as u32);
                    self.set_flag(FLAG_CF, res > 0xff);
                    self.set_flag(FLAG_OF, res > 0xff);
                } else {
                    let lhs = self.read_reg_width(Reg::Eax, bits) as u64;
                    let res = lhs * rhs;
                    self.write_reg_width(Reg::Eax, bits, res as u32);
                    if bits == 16 {
                        self.write_reg_width(Reg::Edx, 16, (res >> 16) as u32);
                        self.set_flag(FLAG_CF, (res >> 16) != 0);
                        self.set_flag(FLAG_OF, (res >> 16) != 0);
                    } else {
                        self.set_reg(Reg::Edx, (res >> 32) as u32);
                        self.set_flag(FLAG_CF, (res >> 32) != 0);
                        self.set_flag(FLAG_OF, (res >> 32) != 0);
                    }
                }
            }
            // /5 IMUL signed accumulator by r/m.
            5 => {
                let rhs = sign_width(self.read_rm(mem, modrm.rm, bits)?, bits) as i32 as i64;
                let lhs = if bits == 8 {
                    self.get_reg8(0) as i8 as i64
                } else {
                    sign_width(self.read_reg_width(Reg::Eax, bits), bits) as i32 as i64
                };
                let res = lhs.wrapping_mul(rhs);
                if bits == 8 {
                    self.write_reg_width(Reg::Eax, 16, res as u32);
                } else {
                    self.write_reg_width(Reg::Eax, bits, res as u32);
                    self.write_reg_width(Reg::Edx, bits, (res >> bits) as u32);
                }
                self.set_imul_flags(res as u32, bits);
            }
            // /6 DIV unsigned accumulator by r/m.
            6 => self.exec_div(mem, modrm.rm, bits, false)?,
            // /7 IDIV signed accumulator by r/m.
            7 => self.exec_div(mem, modrm.rm, bits, true)?,
            _ => unreachable!(),
        }
        Ok(())
    }

    fn exec_group_ff(
        &mut self,
        mem: &mut Memory,
        prefixes: Prefixes,
        op: u8,
        start_eip: u32,
    ) -> Result<()> {
        let bits = if op == 0xfe {
            8
        } else if prefixes.op16 {
            16
        } else {
            32
        };
        let modrm = self.fetch_modrm(mem, prefixes)?;
        match (op, modrm.reg) {
            // FE/FF /0 INC r/m preserves CF.
            (0xfe, 0) | (0xff, 0) => {
                let old = self.read_rm(mem, modrm.rm, bits)?;
                let keep_cf = self.eflags & FLAG_CF;
                let v = self.add(old, 1, bits, false);
                self.eflags = (self.eflags & !FLAG_CF) | keep_cf;
                self.write_rm(mem, modrm.rm, bits, v)?;
            }
            // FE/FF /1 DEC r/m preserves CF.
            (0xfe, 1) | (0xff, 1) => {
                let old = self.read_rm(mem, modrm.rm, bits)?;
                let keep_cf = self.eflags & FLAG_CF;
                let v = self.sub(old, 1, bits, false);
                self.eflags = (self.eflags & !FLAG_CF) | keep_cf;
                self.write_rm(mem, modrm.rm, bits, v)?;
            }
            // FF /2 CALL r/m32.
            (0xff, 2) => {
                let target = self.read_rm(mem, modrm.rm, 32)?;
                let ret_addr = self.eip;
                let esp_before_call = self.reg(Reg::Esp);
                self.push_u32(mem, self.eip)?;
                self.debug_record_call(start_eip, target, ret_addr, esp_before_call)?;
                self.eip = target;
            }
            // FF /4 JMP r/m32.
            (0xff, 4) => {
                self.eip = self.read_rm(mem, modrm.rm, 32)?;
            }
            // FF /6 PUSH r/m32.
            (0xff, 6) => {
                let value = self.read_rm(mem, modrm.rm, 32)?;
                self.push_u32(mem, value)?;
            }
            _ => {
                return Err(Error::Cpu(format!(
                    "unsupported group ff/{} at {:08x}",
                    modrm.reg, self.eip
                )));
            }
        }
        Ok(())
    }

    fn exec_shift_group(&mut self, mem: &mut Memory, prefixes: Prefixes, op: u8) -> Result<()> {
        let bits = if op == 0xc0 || op == 0xd0 || op == 0xd2 {
            8
        } else if prefixes.op16 {
            16
        } else {
            32
        };
        let modrm = self.fetch_modrm(mem, prefixes)?;
        let count = match op {
            // C0/C1 use an immediate shift count.
            0xc0 | 0xc1 => self.fetch_u8(mem)? & 0x1f,
            // D0/D1 shift by one.
            0xd0 | 0xd1 => 1,
            // D2/D3 shift by CL.
            0xd2 | 0xd3 => self.get_reg8(1) & 0x1f,
            _ => unreachable!(),
        };
        if count == 0 {
            return Ok(());
        }
        let value = self.read_rm(mem, modrm.rm, bits)?;
        let res = self.shift_op(value, count, bits, modrm.reg)?;
        self.write_rm(mem, modrm.rm, bits, res)?;
        Ok(())
    }

    fn exec_double_shift(
        &mut self,
        mem: &mut Memory,
        modrm: ModRm,
        bits: u32,
        count: u8,
        left: bool,
    ) -> Result<()> {
        let count = (count & 0x1f) as u32;
        if count == 0 {
            return Ok(());
        }

        let dst = self.read_rm(mem, modrm.rm, bits)?;
        let src = self.read_reg_width(Reg::from_u3(modrm.reg), bits);
        let mask = width_mask(bits);
        let c = if count > bits {
            // Intel leaves this case undefined for 16-bit operands; make it
            // deterministic so old game bit-packers cannot poison the run.
            ((count - 1) % bits) + 1
        } else {
            count
        };

        let value = if left {
            ((dst << c) | (src >> (bits - c))) & mask
        } else {
            ((dst >> c) | (src << (bits - c))) & mask
        };

        self.set_flag(
            FLAG_CF,
            if left {
                ((dst >> (bits - c)) & 1) != 0
            } else {
                ((dst >> (c - 1)) & 1) != 0
            },
        );
        self.set_szp_flags(value, bits);
        if c == 1 {
            let result_msb = (value & sign_bit(bits)) != 0;
            let overflow = if left {
                result_msb ^ self.flag(FLAG_CF)
            } else {
                ((dst & sign_bit(bits)) != 0) ^ result_msb
            };
            self.set_flag(FLAG_OF, overflow);
        }

        self.write_rm(mem, modrm.rm, bits, value)
    }

    fn exec_string(&mut self, mem: &mut Memory, prefixes: Prefixes, op: u8) -> Result<()> {
        let elem = match op {
            0xa5 | 0xa7 | 0xab | 0xad | 0xaf => {
                if prefixes.op16 {
                    2
                } else {
                    4
                }
            }
            _ => 1,
        };
        let mut count = if prefixes.rep.is_some() {
            self.reg(Reg::Ecx)
        } else {
            1
        };
        while count != 0 {
            match op {
                // MOVS copies DS:ESI to ES:EDI.
                0xa4 | 0xa5 => {
                    let v = self.read_mem_width(mem, self.reg(Reg::Esi), elem * 8)?;
                    self.write_mem_width(mem, self.reg(Reg::Edi), elem * 8, v)?;
                    self.bump_si_di(elem, true, true);
                }
                // CMPS compares DS:ESI with ES:EDI and honors REP conditions.
                0xa6 | 0xa7 => {
                    let lhs = self.read_mem_width(mem, self.reg(Reg::Esi), elem * 8)?;
                    let rhs = self.read_mem_width(mem, self.reg(Reg::Edi), elem * 8)?;
                    self.sub(lhs, rhs, elem * 8, false);
                    self.bump_si_di(elem, true, true);
                    if let Some(rep) = prefixes.rep {
                        let z = self.flag(FLAG_ZF);
                        if (rep == RepPrefix::Repe && !z) || (rep == RepPrefix::Repne && z) {
                            count -= 1;
                            break;
                        }
                    }
                }
                // STOS stores AL/AX/EAX to ES:EDI.
                0xaa | 0xab => {
                    let v = self.read_reg_width(Reg::Eax, elem * 8);
                    self.write_mem_width(mem, self.reg(Reg::Edi), elem * 8, v)?;
                    self.bump_di(elem);
                }
                // LODS loads DS:ESI into AL/AX/EAX.
                0xac | 0xad => {
                    let v = self.read_mem_width(mem, self.reg(Reg::Esi), elem * 8)?;
                    self.write_reg_width(Reg::Eax, elem * 8, v);
                    self.bump_si(elem);
                }
                // SCAS compares AL/AX/EAX with ES:EDI and honors REP conditions.
                0xae | 0xaf => {
                    let lhs = self.read_reg_width(Reg::Eax, elem * 8);
                    let rhs = self.read_mem_width(mem, self.reg(Reg::Edi), elem * 8)?;
                    self.sub(lhs, rhs, elem * 8, false);
                    self.bump_di(elem);
                    if let Some(rep) = prefixes.rep {
                        let z = self.flag(FLAG_ZF);
                        if (rep == RepPrefix::Repe && !z) || (rep == RepPrefix::Repne && z) {
                            count -= 1;
                            break;
                        }
                    }
                }
                _ => unreachable!(),
            }
            count -= 1;
            if prefixes.rep.is_none() {
                break;
            }
        }
        if prefixes.rep.is_some() {
            self.set_reg(Reg::Ecx, count);
        }
        Ok(())
    }

    fn exec_x87(
        &mut self,
        mem: &mut Memory,
        prefixes: Prefixes,
        op: u8,
        start_eip: u32,
    ) -> Result<()> {
        let modrm = self.fetch_modrm(mem, prefixes)?;
        if let Rm::Mem(addr) = modrm.rm {
            match (op, modrm.reg) {
                // D8 /0../7 apply or compare single-precision memory operands with ST0.
                (0xd8, 0..=7) => {
                    let rhs = f32::from_bits(mem.read_u32(addr)?) as f64;
                    self.x87_apply_mem_or_compare(rhs, modrm.reg, modrm.reg == 3);
                    return Ok(());
                }
                // FLD m32real pushes a single-precision float.
                (0xd9, 0) => {
                    let bits = mem.read_u32(addr)?;
                    self.x87_push(f32::from_bits(bits) as f64);
                    return Ok(());
                }
                // FLDENV m14/28byte restores the x87 environment.
                (0xd9, 4) => {
                    self.x87_load_env(mem, addr, prefixes.op16)?;
                    return Ok(());
                }
                // FST/FSTP m32real stores ST0 as f32; /3 pops after storing.
                (0xd9, 2) | (0xd9, 3) => {
                    let v = self.x87_peek() as f32;
                    mem.write_u32(addr, v.to_bits())?;
                    if modrm.reg == 3 {
                        self.x87_pop();
                    }
                    return Ok(());
                }
                // FLDCW loads the x87 control word.
                (0xd9, 5) => {
                    self.x87_control = mem.read_u16(addr)?;
                    return Ok(());
                }
                // FNSTENV m14/28byte stores the modeled x87 environment.
                (0xd9, 6) => {
                    self.x87_store_env(mem, addr, prefixes.op16)?;
                    return Ok(());
                }
                // FNSTCW stores the x87 control word.
                (0xd9, 7) => {
                    mem.write_u16(addr, self.x87_control)?;
                    return Ok(());
                }
                // DA /0../7 apply or compare 32-bit integer memory operands with ST0.
                (0xda, 0..=7) => {
                    let rhs = mem.read_u32(addr)? as i32 as f64;
                    self.x87_apply_mem_or_compare(rhs, modrm.reg, modrm.reg == 3);
                    return Ok(());
                }
                // FILD m32int sign-extends a 32-bit integer into ST0.
                (0xdb, 0) => {
                    let v = mem.read_u32(addr)? as i32 as f64;
                    self.x87_push(v);
                    return Ok(());
                }
                // FISTTP m32int stores truncated ST0 and pops.
                (0xdb, 1) => {
                    let int = self.x87_trunc_int() as i32 as u32;
                    mem.write_u32(addr, int)?;
                    self.x87_pop();
                    return Ok(());
                }
                // FIST/FISTP m32int stores rounded ST0; /3 pops after storing.
                (0xdb, 2) | (0xdb, 3) => {
                    let int = self.x87_round_int() as i32 as u32;
                    mem.write_u32(addr, int)?;
                    if modrm.reg == 3 {
                        self.x87_pop();
                    }
                    return Ok(());
                }
                // FLD m80real loads an 80-bit extended float, approximated as f64.
                (0xdb, 5) => {
                    let v = self.x87_load_f80(mem, addr)?;
                    self.x87_push(v);
                    return Ok(());
                }
                // FSTP m80real stores ST0 as 80-bit extended float and pops.
                (0xdb, 7) => {
                    self.x87_store_f80(mem, addr, self.x87_peek())?;
                    self.x87_pop();
                    return Ok(());
                }
                // DC /0../7 apply or compare double-precision memory operands with ST0.
                (0xdc, 0..=7) => {
                    let rhs = self.x87_load_f64(mem, addr)?;
                    self.x87_apply_mem_or_compare(rhs, modrm.reg, modrm.reg == 3);
                    return Ok(());
                }
                // FSTSW m2byte stores the modeled x87 status word.
                (0xdd, 7) => {
                    mem.write_u16(addr, self.x87_status_word())?;
                    return Ok(());
                }
                // FLD m64real pushes a double-precision float.
                (0xdd, 0) => {
                    self.x87_push(self.x87_load_f64(mem, addr)?);
                    return Ok(());
                }
                // FISTTP m64int stores truncated ST0 and pops.
                (0xdd, 1) => {
                    let int = self.x87_trunc_int() as i64 as u64;
                    mem.write_u32(addr, int as u32)?;
                    mem.write_u32(addr + 4, (int >> 32) as u32)?;
                    self.x87_pop();
                    return Ok(());
                }
                // FST/FSTP m64real stores ST0 as f64; /3 pops after storing.
                (0xdd, 2) | (0xdd, 3) => {
                    self.x87_store_f64(mem, addr, self.x87_peek())?;
                    if modrm.reg == 3 {
                        self.x87_pop();
                    }
                    return Ok(());
                }
                // FRSTOR m94/108byte restores the x87 environment and registers.
                (0xdd, 4) => {
                    self.x87_restore_state(mem, addr, prefixes.op16)?;
                    return Ok(());
                }
                // FSAVE/FNSAVE m94/108byte saves state and resets the x87 unit.
                (0xdd, 6) => {
                    self.x87_save_state(mem, addr, prefixes.op16)?;
                    return Ok(());
                }
                // DE /0../7 apply or compare 16-bit integer memory operands with ST0.
                (0xde, 0..=7) => {
                    let rhs = mem.read_u16(addr)? as i16 as f64;
                    self.x87_apply_mem_or_compare(rhs, modrm.reg, modrm.reg == 3);
                    return Ok(());
                }
                // FILD m16int sign-extends a 16-bit integer and pushes it as ST0.
                (0xdf, 0) => {
                    let v = mem.read_u16(addr)? as i16 as f64;
                    self.x87_push(v);
                    return Ok(());
                }
                // FISTTP m16int stores truncated ST0 and pops.
                (0xdf, 1) => {
                    let int = self.x87_trunc_int() as i16 as u16;
                    mem.write_u16(addr, int)?;
                    self.x87_pop();
                    return Ok(());
                }
                // FIST/FISTP m16int stores rounded ST0; /3 pops after storing.
                (0xdf, 2) | (0xdf, 3) => {
                    let int = self.x87_round_int() as i16 as u16;
                    mem.write_u16(addr, int)?;
                    if modrm.reg == 3 {
                        self.x87_pop();
                    }
                    return Ok(());
                }
                // FBLD m80bcd loads a packed BCD integer into ST0.
                (0xdf, 4) => {
                    let v = self.x87_load_bcd(mem, addr)?;
                    self.x87_push(v);
                    return Ok(());
                }
                // FILD m64int sign-extends a 64-bit integer into ST0.
                (0xdf, 5) => {
                    let bits =
                        mem.read_u32(addr)? as u64 | ((mem.read_u32(addr + 4)? as u64) << 32);
                    let v = bits as i64 as f64;
                    self.x87_push(v);
                    return Ok(());
                }
                // FBSTP m80bcd stores ST0 as packed BCD and pops.
                (0xdf, 6) => {
                    self.x87_store_bcd(mem, addr, self.x87_peek())?;
                    self.x87_pop();
                    return Ok(());
                }
                // FISTP m64int stores rounded ST0 and pops.
                (0xdf, 7) => {
                    let int = self.x87_round_int() as i64 as u64;
                    mem.write_u32(addr, int as u32)?;
                    mem.write_u32(addr + 4, (int >> 32) as u32)?;
                    self.x87_pop();
                    return Ok(());
                }
                _ => {}
            }
        }
        match (op, modrm.raw) {
            // FNSTSW AX reports the modeled x87 status word.
            (0xdf, 0xe0) => {
                self.write_reg_width(Reg::Eax, 16, self.x87_status_word() as u32);
                Ok(())
            }
            // FLD ST(i) duplicates an x87 stack entry.
            (0xd9, 0xc0..=0xc7) => {
                let idx = (modrm.raw - 0xc0) as usize;
                let v = self.x87_get(idx);
                self.x87_push(v);
                Ok(())
            }
            // FXCH ST(i) exchanges ST0 and another x87 stack entry.
            (0xd9, 0xc8..=0xcf) => {
                self.x87_exchange((modrm.raw - 0xc8) as usize);
                Ok(())
            }
            // FNOP is an x87 no-op.
            (0xd9, 0xd0) => Ok(()),
            // FSTP ST(i) stores ST0 to another x87 stack entry and pops.
            (0xd9, 0xd8..=0xdf) => {
                self.x87_set((modrm.raw - 0xd8) as usize, self.x87_peek());
                self.x87_pop();
                Ok(())
            }
            // D8 register-register arithmetic/compare with ST0 and ST(i).
            (0xd8, 0xc0..=0xff) => {
                let subop = (modrm.raw >> 3) & 7;
                let rhs = self.x87_get((modrm.raw & 7) as usize);
                self.x87_apply_mem_or_compare(rhs, subop, subop == 3);
                Ok(())
            }
            // DC register-register arithmetic writes the result to ST(i).
            (0xdc, 0xc0..=0xff) => {
                let subop = (modrm.raw >> 3) & 7;
                let idx = (modrm.raw & 7) as usize;
                if subop == 2 || subop == 3 {
                    self.x87_set_compare_status(self.x87_peek(), self.x87_get(idx));
                    if subop == 3 {
                        self.x87_pop();
                    }
                } else {
                    self.x87_apply_to_index(idx, subop);
                }
                Ok(())
            }
            // FCHS negates ST0.
            (0xd9, 0xe0) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = -*v;
                }
                Ok(())
            }
            // FABS replaces ST0 with its absolute value.
            (0xd9, 0xe1) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = v.abs();
                }
                Ok(())
            }
            // FTST compares ST0 with zero and records x87 condition bits.
            (0xd9, 0xe4) => {
                self.x87_set_compare_status(self.x87_peek(), 0.0);
                Ok(())
            }
            // FXAM classifies ST0 into x87 condition bits.
            (0xd9, 0xe5) => {
                self.x87_fxam();
                Ok(())
            }
            // FLD1 pushes constant 1.0.
            (0xd9, 0xe8) => {
                self.x87_push(1.0);
                Ok(())
            }
            // FLDL2T pushes log2(10).
            (0xd9, 0xe9) => {
                self.x87_push(std::f64::consts::LOG2_10);
                Ok(())
            }
            // FLDL2E pushes log2(e).
            (0xd9, 0xea) => {
                self.x87_push(std::f64::consts::LOG2_E);
                Ok(())
            }
            // FLDPI pushes pi.
            (0xd9, 0xeb) => {
                self.x87_push(std::f64::consts::PI);
                Ok(())
            }
            // FLDLG2 pushes log10(2).
            (0xd9, 0xec) => {
                self.x87_push(std::f64::consts::LOG10_2);
                Ok(())
            }
            // FLDLN2 pushes ln(2).
            (0xd9, 0xed) => {
                self.x87_push(std::f64::consts::LN_2);
                Ok(())
            }
            // FLDZ pushes constant 0.0.
            (0xd9, 0xee) => {
                self.x87_push(0.0);
                Ok(())
            }
            // F2XM1 replaces ST0 with 2^ST0 - 1.
            (0xd9, 0xf0) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = v.exp2() - 1.0;
                }
                Ok(())
            }
            // FYL2X computes ST1 * log2(ST0), stores to ST1, and pops ST0.
            (0xd9, 0xf1) => {
                let st0 = self.x87_peek();
                let out = self.x87_get(1) * st0.log2();
                self.x87_set(1, out);
                self.x87_pop();
                Ok(())
            }
            // FPTAN replaces ST0 with tan(ST0) and pushes 1.0.
            (0xd9, 0xf2) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = v.tan();
                }
                self.x87_push(1.0);
                self.x87_status &= !X87_STATUS_C2;
                Ok(())
            }
            // FPATAN computes atan2(ST1, ST0), stores to ST1, and pops ST0.
            (0xd9, 0xf3) => {
                let out = self.x87_get(1).atan2(self.x87_peek());
                self.x87_set(1, out);
                self.x87_pop();
                Ok(())
            }
            // FXTRACT pushes the significand and leaves the exponent in ST1.
            (0xd9, 0xf4) => {
                let st0 = self.x87_peek();
                if st0 == 0.0 || !st0.is_finite() {
                    self.x87_set(0, st0);
                    self.x87_push(0.0);
                } else {
                    let exp = st0.abs().log2().floor();
                    let sig = st0 / exp.exp2();
                    self.x87_set(0, exp);
                    self.x87_push(sig);
                }
                Ok(())
            }
            // FPREM1 computes IEEE partial remainder; this model completes immediately.
            (0xd9, 0xf5) => {
                self.x87_fprem(true);
                Ok(())
            }
            // FDECSTP decrements the x87 TOP pointer; this flat model treats it as no-op.
            (0xd9, 0xf6) => {
                self.x87_status &= !X87_STATUS_C1;
                Ok(())
            }
            // FINCSTP increments the x87 TOP pointer; this flat model treats it as no-op.
            (0xd9, 0xf7) => {
                self.x87_status &= !X87_STATUS_C1;
                Ok(())
            }
            // FPREM computes partial remainder; this model completes immediately.
            (0xd9, 0xf8) => {
                self.x87_fprem(false);
                Ok(())
            }
            // FYL2XP1 computes ST1 * log2(ST0 + 1), stores to ST1, and pops ST0.
            (0xd9, 0xf9) => {
                let out = self.x87_get(1) * (self.x87_peek() + 1.0).log2();
                self.x87_set(1, out);
                self.x87_pop();
                Ok(())
            }
            // FSQRT replaces ST0 with its square root.
            (0xd9, 0xfa) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = v.sqrt();
                }
                Ok(())
            }
            // FSINCOS replaces ST0 with sin(ST0) and pushes cos(ST0).
            (0xd9, 0xfb) => {
                let st0 = self.x87_peek();
                self.x87_set(0, st0.sin());
                self.x87_push(st0.cos());
                self.x87_status &= !X87_STATUS_C2;
                Ok(())
            }
            // FRNDINT rounds ST0 according to the control word.
            (0xd9, 0xfc) => {
                let rounded = self.x87_round_int();
                if let Some(v) = self.x87.last_mut() {
                    *v = rounded;
                }
                Ok(())
            }
            // FSCALE scales ST0 by 2^trunc(ST1).
            (0xd9, 0xfd) => {
                let out = self.x87_peek() * self.x87_get(1).trunc().exp2();
                self.x87_set(0, out);
                Ok(())
            }
            // FSIN replaces ST0 with sin(ST0).
            (0xd9, 0xfe) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = v.sin();
                }
                self.x87_status &= !X87_STATUS_C2;
                Ok(())
            }
            // FCOS replaces ST0 with cos(ST0).
            (0xd9, 0xff) => {
                if let Some(v) = self.x87.last_mut() {
                    *v = v.cos();
                }
                self.x87_status &= !X87_STATUS_C2;
                Ok(())
            }
            // DA register-register FCMOVcc conditionally copies ST(i) to ST0.
            (0xda, 0xc0..=0xdf) => {
                let subop = (modrm.raw >> 3) & 3;
                let idx = (modrm.raw & 7) as usize;
                let take = match subop {
                    0 => self.flag(FLAG_CF),
                    1 => self.flag(FLAG_ZF),
                    2 => self.flag(FLAG_CF) || self.flag(FLAG_ZF),
                    _ => self.flag(FLAG_PF),
                };
                if take {
                    self.x87_set(0, self.x87_get(idx));
                }
                Ok(())
            }
            // FUCOMPP compares ST0 with ST1 and pops both operands.
            (0xda, 0xe9) => {
                self.x87_set_compare_status(self.x87_peek(), self.x87_get(1));
                self.x87_pop();
                self.x87_pop();
                Ok(())
            }
            // DB register-register FCMOVcc conditionally copies ST(i) to ST0.
            (0xdb, 0xc0..=0xdf) => {
                let subop = (modrm.raw >> 3) & 3;
                let idx = (modrm.raw & 7) as usize;
                let take = match subop {
                    0 => !self.flag(FLAG_CF),
                    1 => !self.flag(FLAG_ZF),
                    2 => !(self.flag(FLAG_CF) || self.flag(FLAG_ZF)),
                    _ => !self.flag(FLAG_PF),
                };
                if take {
                    self.x87_set(0, self.x87_get(idx));
                }
                Ok(())
            }
            // FNCLEX clears modeled x87 exception and condition bits.
            (0xdb, 0xe2) => {
                self.x87_status = 0;
                Ok(())
            }
            // FNINIT resets the modeled x87 stack and control word.
            (0xdb, 0xe3) => {
                self.x87_init();
                Ok(())
            }
            // FENI/FDISI/FSETPM are old 287 controls; they are no-ops on 387+.
            (0xdb, 0xe0 | 0xe1 | 0xe4) => Ok(()),
            // FUCOMI compares ST0 with ST(i) and updates integer flags.
            (0xdb, 0xe8..=0xef) => {
                self.x87_set_compare_flags(self.x87_peek(), self.x87_get((modrm.raw & 7) as usize));
                Ok(())
            }
            // FCOMI compares ST0 with ST(i) and updates integer flags.
            (0xdb, 0xf0..=0xf7) => {
                self.x87_set_compare_flags(self.x87_peek(), self.x87_get((modrm.raw & 7) as usize));
                Ok(())
            }
            // FCOMPP compares ST0 with ST1 and discards both operands.
            (0xde, 0xd9) => {
                self.x87_set_compare_status(self.x87_peek(), self.x87_get(1));
                self.x87_pop();
                self.x87_pop();
                Ok(())
            }
            // DE register-register arithmetic/compare, popping when the opcode form does.
            (0xde, 0xc0..=0xff) => {
                let subop = (modrm.raw >> 3) & 7;
                let idx = (modrm.raw & 7) as usize;
                if subop == 2 {
                    self.x87_set_compare_status(self.x87_peek(), self.x87_get(idx));
                    self.x87_pop();
                } else if subop != 3 {
                    self.x87_apply_pop(idx, subop);
                } else {
                    return Err(Error::Cpu(format!(
                        "unsupported x87 opcode {op:02x} / raw {:02x} at {start_eip:08x}",
                        modrm.raw
                    )));
                }
                Ok(())
            }
            // FFREE ST(i) marks a stack entry empty; this flat model keeps the value.
            (0xdd, 0xc0..=0xc7) => Ok(()),
            // FXCH ST(i) exchanges ST0 and another x87 stack entry.
            (0xdd, 0xc8..=0xcf) => {
                self.x87_exchange((modrm.raw - 0xc8) as usize);
                Ok(())
            }
            // FST ST(i) stores ST0 to another x87 stack entry.
            (0xdd, 0xd0..=0xd7) => {
                self.x87_set((modrm.raw - 0xd0) as usize, self.x87_peek());
                Ok(())
            }
            // FSTP ST(i) stores ST0 to another x87 stack entry and pops.
            (0xdd, 0xd8..=0xdf) => {
                self.x87_set((modrm.raw - 0xd8) as usize, self.x87_peek());
                self.x87_pop();
                Ok(())
            }
            // FUCOM ST(i) compares ST0 with ST(i).
            (0xdd, 0xe0..=0xe7) => {
                self.x87_set_compare_status(
                    self.x87_peek(),
                    self.x87_get((modrm.raw & 7) as usize),
                );
                Ok(())
            }
            // FUCOMP ST(i) compares ST0 with ST(i) and pops ST0.
            (0xdd, 0xe8..=0xef) => {
                self.x87_set_compare_status(
                    self.x87_peek(),
                    self.x87_get((modrm.raw & 7) as usize),
                );
                self.x87_pop();
                Ok(())
            }
            // FFREEP ST(i) marks an entry empty and pops ST0; value storage is ignored here.
            (0xdf, 0xc0..=0xc7) => {
                self.x87_pop();
                Ok(())
            }
            // FXCH ST(i) exchanges ST0 and another x87 stack entry.
            (0xdf, 0xc8..=0xcf) => {
                self.x87_exchange((modrm.raw - 0xc8) as usize);
                Ok(())
            }
            // FSTP ST(i) stores ST0 to another x87 stack entry and pops.
            (0xdf, 0xd0..=0xdf) => {
                self.x87_set((modrm.raw & 7) as usize, self.x87_peek());
                self.x87_pop();
                Ok(())
            }
            // FUCOMIP compares ST0 with ST(i), updates integer flags, and pops ST0.
            (0xdf, 0xe8..=0xef) => {
                self.x87_set_compare_flags(self.x87_peek(), self.x87_get((modrm.raw & 7) as usize));
                self.x87_pop();
                Ok(())
            }
            // FCOMIP compares ST0 with ST(i), updates integer flags, and pops ST0.
            (0xdf, 0xf0..=0xf7) => {
                self.x87_set_compare_flags(self.x87_peek(), self.x87_get((modrm.raw & 7) as usize));
                self.x87_pop();
                Ok(())
            }
            _ => Err(Error::Cpu(format!(
                "unsupported x87 opcode {op:02x} / raw {:02x} at {start_eip:08x}",
                modrm.raw
            ))),
        }
    }

    fn x87_init(&mut self) {
        self.x87.clear();
        self.x87_control = 0x037f;
        self.x87_status = 0;
    }

    fn x87_status_word(&self) -> u16 {
        self.x87_status
    }

    fn x87_push(&mut self, value: f64) {
        if self.x87.len() == 8 {
            self.x87.remove(0);
        }
        self.x87.push(value);
    }

    fn x87_pop(&mut self) -> f64 {
        self.x87.pop().unwrap_or(0.0)
    }

    fn x87_peek(&self) -> f64 {
        *self.x87.last().unwrap_or(&0.0)
    }

    fn x87_get(&self, index: usize) -> f64 {
        let len = self.x87.len();
        if index >= len {
            0.0
        } else {
            self.x87[len - 1 - index]
        }
    }

    fn x87_set(&mut self, index: usize, value: f64) {
        let len = self.x87.len();
        if len == 0 {
            if index == 0 {
                self.x87_push(value);
            }
            return;
        }
        if index < len {
            let slot = len - 1 - index;
            self.x87[slot] = value;
        }
    }

    fn x87_exchange(&mut self, index: usize) {
        let len = self.x87.len();
        if index >= len {
            return;
        }
        let top = len - 1;
        let other = len - 1 - index;
        self.x87.swap(top, other);
    }

    fn x87_round_int(&self) -> f64 {
        let v = self.x87_peek();
        // x87 control-word bits 10..11 select nearest/down/up/truncate rounding.
        match (self.x87_control >> 10) & 3 {
            1 => v.floor(),
            2 => v.ceil(),
            3 => v.trunc(),
            _ => v.round(),
        }
    }

    fn x87_trunc_int(&self) -> f64 {
        self.x87_peek().trunc()
    }

    fn x87_set_compare_status(&mut self, lhs: f64, rhs: f64) {
        self.x87_status &= !(X87_STATUS_C0 | X87_STATUS_C2 | X87_STATUS_C3);
        if lhs.is_nan() || rhs.is_nan() {
            self.x87_status |= X87_STATUS_C0 | X87_STATUS_C2 | X87_STATUS_C3;
        } else if lhs == rhs {
            self.x87_status |= X87_STATUS_C3;
        } else if lhs < rhs {
            self.x87_status |= X87_STATUS_C0;
        }
    }

    fn x87_set_compare_flags(&mut self, lhs: f64, rhs: f64) {
        self.set_flag(FLAG_CF, false);
        self.set_flag(FLAG_ZF, false);
        self.set_flag(FLAG_PF, false);
        self.set_flag(FLAG_OF, false);
        self.set_flag(FLAG_SF, false);
        self.set_flag(FLAG_AF, false);
        if lhs.is_nan() || rhs.is_nan() {
            self.set_flag(FLAG_CF, true);
            self.set_flag(FLAG_ZF, true);
            self.set_flag(FLAG_PF, true);
        } else if lhs == rhs {
            self.set_flag(FLAG_ZF, true);
        } else if lhs < rhs {
            self.set_flag(FLAG_CF, true);
        }
    }

    fn x87_apply_mem_or_compare(&mut self, rhs: f64, op: u8, pop_on_compare: bool) {
        if op == 2 || op == 3 {
            self.x87_set_compare_status(self.x87_peek(), rhs);
            if pop_on_compare {
                self.x87_pop();
            }
        } else {
            self.x87_apply_mem(rhs, op);
        }
    }

    fn x87_apply_mem(&mut self, rhs: f64, op: u8) {
        let lhs = self.x87_peek();
        let out = match op {
            // FADD.
            0 => lhs + rhs,
            // FMUL.
            1 => lhs * rhs,
            // FSUB.
            4 => lhs - rhs,
            // FSUBR.
            5 => rhs - lhs,
            // FDIV.
            6 => lhs / rhs,
            // FDIVR.
            7 => rhs / lhs,
            _ => lhs,
        };
        if let Some(top) = self.x87.last_mut() {
            *top = out;
        } else {
            self.x87_push(out);
        }
    }

    fn x87_apply_to_index(&mut self, index: usize, op: u8) {
        let st0 = self.x87_peek();
        let sti = self.x87_get(index);
        let out = match op {
            // FADD.
            0 => st0 + sti,
            // FMUL.
            1 => st0 * sti,
            // FSUB for the DC register form stores ST0 - ST(i) into ST(i).
            4 => st0 - sti,
            // FSUBR for the DC register form stores ST(i) - ST0 into ST(i).
            5 => sti - st0,
            // FDIV for the DC register form stores ST0 / ST(i) into ST(i).
            6 => st0 / sti,
            // FDIVR for the DC register form stores ST(i) / ST0 into ST(i).
            7 => sti / st0,
            _ => sti,
        };
        self.x87_set(index, out);
    }

    fn x87_apply_pop(&mut self, index: usize, op: u8) {
        let len = self.x87.len();
        if len == 0 {
            return;
        }
        let st0 = self.x87_peek();
        let target = len.saturating_sub(1 + index);
        let lhs = self.x87[target];
        let out = match op {
            // FADDP.
            0 => lhs + st0,
            // FMULP.
            1 => lhs * st0,
            // FSUBRP for the DE pop form.
            4 => st0 - lhs,
            // FSUBP for the DE pop form.
            5 => lhs - st0,
            // FDIVRP for the DE pop form.
            6 => st0 / lhs,
            // FDIVP for the DE pop form.
            7 => lhs / st0,
            _ => lhs,
        };
        self.x87[target] = out;
        self.x87_pop();
    }

    fn x87_fprem(&mut self, ieee: bool) {
        let st0 = self.x87_peek();
        let st1 = self.x87_get(1);
        self.x87_status &= !X87_STATUS_RESULT;
        if st1 == 0.0 || st0.is_nan() || st1.is_nan() {
            self.x87_set(0, f64::NAN);
            return;
        }
        let quotient = st0 / st1;
        let q = if ieee {
            quotient.round()
        } else {
            quotient.trunc()
        };
        let remainder = st0 - st1 * q;
        self.x87_set(0, remainder);

        let qbits = if q.is_finite() && q >= i64::MIN as f64 && q <= i64::MAX as f64 {
            q as i64 as u64
        } else {
            0
        };
        if (qbits & 1) != 0 {
            self.x87_status |= X87_STATUS_C1;
        }
        if (qbits & 2) != 0 {
            self.x87_status |= X87_STATUS_C3;
        }
        if (qbits & 4) != 0 {
            self.x87_status |= X87_STATUS_C0;
        }
        self.x87_status &= !X87_STATUS_C2;
    }

    fn x87_fxam(&mut self) {
        let st0 = self.x87_peek();
        self.x87_status &= !X87_STATUS_RESULT;
        if st0.is_sign_negative() {
            self.x87_status |= X87_STATUS_C1;
        }
        if self.x87.is_empty() {
            self.x87_status |= X87_STATUS_C3 | X87_STATUS_C0;
        } else if st0.is_nan() {
            self.x87_status |= X87_STATUS_C0;
        } else if st0 == 0.0 {
            self.x87_status |= X87_STATUS_C3;
        } else if st0.is_infinite() {
            self.x87_status |= X87_STATUS_C2 | X87_STATUS_C0;
        } else {
            self.x87_status |= X87_STATUS_C2;
        }
    }

    fn x87_load_f64(&self, mem: &Memory, addr: u32) -> Result<f64> {
        let bits = mem.read_u32(addr)? as u64 | ((mem.read_u32(addr + 4)? as u64) << 32);
        Ok(f64::from_bits(bits))
    }

    fn x87_store_f64(&self, mem: &mut Memory, addr: u32, value: f64) -> Result<()> {
        let bits = value.to_bits();
        mem.write_u32(addr, bits as u32)?;
        mem.write_u32(addr + 4, (bits >> 32) as u32)
    }

    fn x87_load_f80(&self, mem: &Memory, addr: u32) -> Result<f64> {
        let mantissa = mem.read_u32(addr)? as u64 | ((mem.read_u32(addr + 4)? as u64) << 32);
        let sign_exp = mem.read_u16(addr + 8)?;
        let sign = (sign_exp & 0x8000) != 0;
        let exp = sign_exp & 0x7fff;
        let mut value = if exp == 0 {
            if mantissa == 0 {
                0.0
            } else {
                (mantissa as f64 / 2.0_f64.powi(63)) * 2.0_f64.powi(-16382)
            }
        } else if exp == 0x7fff {
            if mantissa == 0x8000_0000_0000_0000 {
                f64::INFINITY
            } else {
                f64::NAN
            }
        } else {
            (mantissa as f64 / 2.0_f64.powi(63)) * 2.0_f64.powi(exp as i32 - 16383)
        };
        if sign {
            value = -value;
        }
        Ok(value)
    }

    fn x87_store_f80(&self, mem: &mut Memory, addr: u32, value: f64) -> Result<()> {
        let bits = value.to_bits();
        let sign = ((bits >> 63) as u16) << 15;
        let abs = value.abs();
        let (mantissa, sign_exp) = if abs == 0.0 {
            (0, sign)
        } else if abs.is_infinite() {
            (0x8000_0000_0000_0000, sign | 0x7fff)
        } else if abs.is_nan() {
            (0xc000_0000_0000_0000, 0x7fff)
        } else {
            let exp = abs.log2().floor() as i32;
            let normalized = abs / 2.0_f64.powi(exp);
            let scaled = (normalized * 2.0_f64.powi(63)).round();
            let (mantissa, exp) = if scaled >= u64::MAX as f64 {
                (0x8000_0000_0000_0000, exp + 1)
            } else {
                (scaled as u64, exp)
            };
            let biased = (exp + 16383).clamp(0, 0x7fff) as u16;
            (mantissa, sign | biased)
        };
        mem.write_u32(addr, mantissa as u32)?;
        mem.write_u32(addr + 4, (mantissa >> 32) as u32)?;
        mem.write_u16(addr + 8, sign_exp)
    }

    fn x87_load_bcd(&self, mem: &Memory, addr: u32) -> Result<f64> {
        let mut value = 0f64;
        for i in (0..9).rev() {
            let b = mem.read_u8(addr + i)?;
            value = value * 100.0 + ((b >> 4) as f64) * 10.0 + ((b & 0x0f) as f64);
        }
        if (mem.read_u8(addr + 9)? & 0x80) != 0 {
            value = -value;
        }
        Ok(value)
    }

    fn x87_store_bcd(&self, mem: &mut Memory, addr: u32, value: f64) -> Result<()> {
        let negative = value.is_sign_negative();
        let mut digits = value.abs().round() as u128;
        for i in 0..9 {
            let low = (digits % 10) as u8;
            digits /= 10;
            let high = (digits % 10) as u8;
            digits /= 10;
            mem.write_u8(addr + i, low | (high << 4))?;
        }
        mem.write_u8(addr + 9, if negative { 0x80 } else { 0 })
    }

    fn x87_tag_word(&self) -> u16 {
        let mut tag = 0u16;
        for i in 0..8 {
            let bits = if i >= self.x87.len() {
                3
            } else {
                let v = self.x87_get(i);
                if v == 0.0 {
                    1
                } else if !v.is_finite() {
                    2
                } else {
                    0
                }
            };
            tag |= bits << (i * 2);
        }
        tag
    }

    fn x87_load_env(&mut self, mem: &Memory, addr: u32, op16: bool) -> Result<()> {
        self.x87_control = mem.read_u16(addr)?;
        self.x87_status = mem.read_u16(if op16 { addr + 2 } else { addr + 4 })? & !(7 << 11);
        Ok(())
    }

    fn x87_store_env(&self, mem: &mut Memory, addr: u32, op16: bool) -> Result<()> {
        if op16 {
            mem.write_u16(addr, self.x87_control)?;
            mem.write_u16(addr + 2, self.x87_status_word())?;
            mem.write_u16(addr + 4, self.x87_tag_word())?;
            mem.write_u16(addr + 6, 0)?;
            mem.write_u16(addr + 8, 0)?;
            mem.write_u16(addr + 10, 0)?;
            mem.write_u16(addr + 12, 0)?;
        } else {
            mem.write_u32(addr, 0xffff_0000 | self.x87_control as u32)?;
            mem.write_u32(addr + 4, 0xffff_0000 | self.x87_status_word() as u32)?;
            mem.write_u32(addr + 8, 0xffff_0000 | self.x87_tag_word() as u32)?;
            mem.write_u32(addr + 12, 0)?;
            mem.write_u16(addr + 16, 0)?;
            mem.write_u16(addr + 18, 0)?;
            mem.write_u32(addr + 20, 0)?;
            mem.write_u32(addr + 24, 0xffff_0000)?;
        }
        Ok(())
    }

    fn x87_save_state(&mut self, mem: &mut Memory, addr: u32, op16: bool) -> Result<()> {
        self.x87_store_env(mem, addr, op16)?;
        let mut reg_addr = addr + if op16 { 14 } else { 28 };
        for i in 0..8 {
            self.x87_store_f80(mem, reg_addr, self.x87_get(i))?;
            reg_addr += 10;
        }
        self.x87_init();
        Ok(())
    }

    fn x87_restore_state(&mut self, mem: &Memory, addr: u32, op16: bool) -> Result<()> {
        self.x87_load_env(mem, addr, op16)?;
        let mut values = Vec::with_capacity(8);
        let mut reg_addr = addr + if op16 { 14 } else { 28 };
        for _ in 0..8 {
            values.push(self.x87_load_f80(mem, reg_addr)?);
            reg_addr += 10;
        }
        self.x87.clear();
        for value in values.into_iter().rev() {
            self.x87_push(value);
        }
        Ok(())
    }

    fn exec_div(&mut self, mem: &mut Memory, rm: Rm, bits: u32, signed: bool) -> Result<()> {
        let divisor = self.read_rm(mem, rm, bits)?;
        if divisor == 0 {
            return Err(Error::Cpu(format!("divide by zero at {:08x}", self.eip)));
        }
        if signed {
            if bits == 32 {
                let dividend = ((self.reg(Reg::Edx) as u64) << 32) | self.reg(Reg::Eax) as u64;
                let q = (dividend as i64) / (divisor as i32 as i64);
                let r = (dividend as i64) % (divisor as i32 as i64);
                self.set_reg(Reg::Eax, q as u32);
                self.set_reg(Reg::Edx, r as u32);
            } else {
                let dividend = (((self.read_reg_width(Reg::Edx, 16) as u32) << 16)
                    | self.read_reg_width(Reg::Eax, 16)) as i32;
                let q = dividend / divisor as i16 as i32;
                let r = dividend % divisor as i16 as i32;
                self.write_reg_width(Reg::Eax, 16, q as u32);
                self.write_reg_width(Reg::Edx, 16, r as u32);
            }
        } else if bits == 32 {
            let dividend = ((self.reg(Reg::Edx) as u64) << 32) | self.reg(Reg::Eax) as u64;
            self.set_reg(Reg::Eax, (dividend / divisor as u64) as u32);
            self.set_reg(Reg::Edx, (dividend % divisor as u64) as u32);
        } else {
            let dividend = ((self.read_reg_width(Reg::Edx, 16) as u32) << 16)
                | self.read_reg_width(Reg::Eax, 16);
            self.write_reg_width(Reg::Eax, 16, dividend / divisor);
            self.write_reg_width(Reg::Edx, 16, dividend % divisor);
        }
        Ok(())
    }

    fn shift_op(&mut self, value: u32, count: u8, bits: u32, op: u8) -> Result<u32> {
        let mask = width_mask(bits);
        let v = value & mask;
        let res = match op {
            // /0 ROL rotates left within the selected width.
            0 => {
                let c = (count as u32) % bits;
                let r = ((v << c) | (v >> (bits - c))) & mask;
                self.set_flag(FLAG_CF, (r & 1) != 0);
                r
            }
            // /1 ROR rotates right within the selected width.
            1 => {
                let c = (count as u32) % bits;
                let r = ((v >> c) | (v << (bits - c))) & mask;
                self.set_flag(FLAG_CF, (r & sign_bit(bits)) != 0);
                r
            }
            // /2 RCL and /3 RCR rotate through carry.
            2 | 3 => {
                let mut r = v;
                let mut cf = self.flag(FLAG_CF);
                for _ in 0..count {
                    if op == 2 {
                        let new_cf = (r & sign_bit(bits)) != 0;
                        r = ((r << 1) & mask) | (cf as u32);
                        cf = new_cf;
                    } else {
                        let new_cf = (r & 1) != 0;
                        r = (r >> 1) | ((cf as u32) << (bits - 1));
                        cf = new_cf;
                    }
                }
                self.set_flag(FLAG_CF, cf);
                if count == 1 {
                    let msb = (r & sign_bit(bits)) != 0;
                    self.set_flag(FLAG_OF, msb ^ cf);
                }
                r & mask
            }
            // /4 SHL and /6 SAL shift left; x86 treats SAL as SHL.
            4 | 6 => {
                let r = (v << count) & mask;
                let cf = ((v << (count - 1)) & sign_bit(bits)) != 0;
                self.set_shift_result_flags(r, bits);
                self.set_flag(FLAG_CF, cf);
                if count == 1 {
                    self.set_flag(FLAG_OF, ((r ^ v) & sign_bit(bits)) != 0);
                }
                r
            }
            // /5 SHR shifts right with zero-fill.
            5 => {
                let r = v >> count;
                let cf = ((v >> (count - 1)) & 1) != 0;
                self.set_shift_result_flags(r, bits);
                self.set_flag(FLAG_CF, cf);
                if count == 1 {
                    self.set_flag(FLAG_OF, (v & sign_bit(bits)) != 0);
                }
                r
            }
            // /7 SAR shifts right preserving the sign bit.
            7 => {
                let signed = sign_width(v, bits) as i32;
                let r = (signed >> count) as u32 & mask;
                let cf = ((v >> (count - 1)) & 1) != 0;
                self.set_shift_result_flags(r, bits);
                self.set_flag(FLAG_CF, cf);
                if count == 1 {
                    self.set_flag(FLAG_OF, false);
                }
                r
            }
            _ => return Err(Error::Cpu(format!("unsupported shift group /{op}"))),
        };
        Ok(res)
    }

    fn exec_bit_test_modify(
        &mut self,
        mem: &mut Memory,
        rm: Rm,
        bits: u32,
        bit_offset: u32,
        op: u8,
    ) -> Result<()> {
        let (target, bit) = match rm {
            Rm::Reg(_) => (rm, bit_offset & (bits - 1)),
            Rm::Mem(addr) => {
                let signed = bit_offset as i32;
                let word = signed.div_euclid(bits as i32);
                let bit = signed.rem_euclid(bits as i32) as u32;
                let byte_offset = word.wrapping_mul((bits / 8) as i32) as u32;
                (Rm::Mem(addr.wrapping_add(byte_offset)), bit)
            }
        };
        let value = self.read_rm(mem, target, bits)?;
        let mask = 1u32 << bit;
        self.set_flag(FLAG_CF, (value & mask) != 0);
        let new_value = match op {
            // BT only reports the selected bit in CF.
            0xa3 => value,
            // BTS reports the bit and sets it.
            0xab => value | mask,
            // BTR reports the bit and clears it.
            0xb3 => value & !mask,
            // BTC reports the bit and complements it.
            0xbb => value ^ mask,
            _ => unreachable!(),
        };
        if op != 0xa3 {
            self.write_rm(mem, target, bits, new_value)?;
        }
        Ok(())
    }

    fn exec_group_0f_ae(
        &mut self,
        mem: &mut Memory,
        prefixes: Prefixes,
        start_eip: u32,
    ) -> Result<()> {
        let modrm = self.fetch_modrm(mem, prefixes)?;
        match modrm.reg {
            // FXSAVE m512 stores x87/MMX/SSE state. wemu does not model XMM yet,
            // but runtimes commonly inspect the control words and MXCSR fields.
            0 => {
                let Rm::Mem(addr) = modrm.rm else {
                    return self.unsupported_0f(0xae, start_eip);
                };
                self.fxsave_minimal(mem, addr)?;
            }
            // FXRSTOR m512 restores the subset of state that wemu currently models.
            1 => {
                let Rm::Mem(addr) = modrm.rm else {
                    return self.unsupported_0f(0xae, start_eip);
                };
                self.fxrstor_minimal(mem, addr)?;
            }
            // LDMXCSR m32 loads SSE control/status; unsupported bits are masked.
            2 => {
                let Rm::Mem(addr) = modrm.rm else {
                    return self.unsupported_0f(0xae, start_eip);
                };
                self.mxcsr = mem.read_u32(addr)? & MXCSR_SUPPORTED_MASK;
            }
            // STMXCSR m32 stores the modeled SSE control/status register.
            3 => {
                let Rm::Mem(addr) = modrm.rm else {
                    return self.unsupported_0f(0xae, start_eip);
                };
                mem.write_u32(addr, self.mxcsr & MXCSR_SUPPORTED_MASK)?;
            }
            // XSAVE m512 is accepted as an FXSAVE-equivalent subset.
            4 => {
                let Rm::Mem(addr) = modrm.rm else {
                    return self.unsupported_0f(0xae, start_eip);
                };
                self.fxsave_minimal(mem, addr)?;
            }
            // LFENCE is encoded as 0F AE E8; other /5 forms are XRSTOR.
            5 => {
                if modrm.raw != 0xe8 {
                    let Rm::Mem(addr) = modrm.rm else {
                        return self.unsupported_0f(0xae, start_eip);
                    };
                    self.fxrstor_minimal(mem, addr)?;
                }
            }
            // MFENCE is encoded as 0F AE F0; other /6 forms are XSAVEOPT.
            6 => {
                if modrm.raw != 0xf0 {
                    let Rm::Mem(addr) = modrm.rm else {
                        return self.unsupported_0f(0xae, start_eip);
                    };
                    self.fxsave_minimal(mem, addr)?;
                }
            }
            // SFENCE is encoded as 0F AE F8; other /7 forms are CLFLUSH.
            7 => {}
            _ => unreachable!(),
        }
        Ok(())
    }

    fn fxsave_minimal(&self, mem: &mut Memory, addr: u32) -> Result<()> {
        mem.write_u16(addr, self.x87_control)?;
        mem.write_u16(addr + 2, self.x87_status)?;
        let tag = if self.x87.len() >= 8 {
            0xff
        } else {
            (1u16 << self.x87.len()) - 1
        };
        mem.write_u8(addr + 4, tag as u8)?;
        mem.write_u32(addr + 24, self.mxcsr & MXCSR_SUPPORTED_MASK)?;
        mem.write_u32(addr + 28, MXCSR_SUPPORTED_MASK)?;
        Ok(())
    }

    fn fxrstor_minimal(&mut self, mem: &Memory, addr: u32) -> Result<()> {
        self.x87_control = mem.read_u16(addr)?;
        self.x87_status = mem.read_u16(addr + 2)?;
        self.mxcsr = mem.read_u32(addr + 24)? & MXCSR_SUPPORTED_MASK;
        Ok(())
    }

    #[inline(always)]
    fn fetch_modrm(&mut self, mem: &Memory, prefixes: Prefixes) -> Result<ModRm> {
        if prefixes.addr16 {
            return Err(Error::Cpu(format!(
                "16-bit addressing mode unsupported at {:08x}",
                self.eip
            )));
        }
        let raw = self.fetch_u8(mem)?;
        let mode = raw >> 6;
        let reg = (raw >> 3) & 7;
        let rm_field = raw & 7;
        let rm = if mode == 3 {
            Rm::Reg(rm_field)
        } else {
            let mut sib_base_is_disp32 = false;
            let base_addr = if rm_field == 4 {
                let sib = self.fetch_u8(mem)?;
                let scale = 1u32 << (sib >> 6);
                let index = (sib >> 3) & 7;
                let base = sib & 7;
                let index_val = if index == 4 {
                    0
                } else {
                    self.reg(Reg::from_u3(index)).wrapping_mul(scale)
                };
                let base_val = if base == 5 && mode == 0 {
                    sib_base_is_disp32 = true;
                    0
                } else {
                    self.reg(Reg::from_u3(base))
                };
                base_val.wrapping_add(index_val)
            } else if rm_field == 5 && mode == 0 {
                0
            } else {
                self.reg(Reg::from_u3(rm_field))
            };
            let disp = match mode {
                0 if rm_field == 5 || sib_base_is_disp32 => self.fetch_u32(mem)?,
                1 => self.fetch_i8(mem)? as i32 as u32,
                2 => self.fetch_i32(mem)? as u32,
                _ => 0,
            };
            let addr = self.apply_seg(prefixes, base_addr.wrapping_add(disp));
            Rm::Mem(addr)
        };
        Ok(ModRm { raw, reg, rm })
    }

    #[inline(always)]
    fn apply_seg(&self, prefixes: Prefixes, addr: u32) -> u32 {
        if let Some(seg) = prefixes.seg {
            addr.wrapping_add(self.seg_base[seg])
        } else {
            addr
        }
    }

    #[inline(always)]
    fn read_rm(&self, mem: &Memory, rm: Rm, bits: u32) -> Result<u32> {
        match rm {
            Rm::Reg(r) => Ok(self.read_reg_width(Reg::from_u3(r), bits)),
            Rm::Mem(addr) => self.read_mem_width(mem, addr, bits),
        }
    }

    #[inline(always)]
    fn write_rm(&mut self, mem: &mut Memory, rm: Rm, bits: u32, value: u32) -> Result<()> {
        match rm {
            Rm::Reg(r) => {
                self.write_reg_width(Reg::from_u3(r), bits, value);
                Ok(())
            }
            Rm::Mem(addr) => self.write_mem_width(mem, addr, bits, value),
        }
    }

    #[inline(always)]
    fn read_mem_width(&self, mem: &Memory, addr: u32, bits: u32) -> Result<u32> {
        match bits {
            8 => Ok(mem.read_u8(addr)? as u32),
            16 => Ok(mem.read_u16(addr)? as u32),
            32 => mem.read_u32(addr),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn write_mem_width(
        &mut self,
        mem: &mut Memory,
        addr: u32,
        bits: u32,
        value: u32,
    ) -> Result<()> {
        match bits {
            8 => mem.write_u8(addr, value as u8)?,
            16 => mem.write_u16(addr, value as u16)?,
            32 => mem.write_u32(addr, value)?,
            _ => unreachable!(),
        }
        Ok(())
    }

    #[inline(always)]
    fn read_mmx_rm64(&self, mem: &Memory, rm: Rm) -> Result<u64> {
        match rm {
            Rm::Reg(r) => Ok(self.mmx[r as usize]),
            Rm::Mem(addr) => {
                let lo = mem.read_u32(addr)? as u64;
                let hi = mem.read_u32(addr.wrapping_add(4))? as u64;
                Ok(lo | (hi << 32))
            }
        }
    }

    #[inline(always)]
    fn read_mmx_rm32_low(&self, mem: &Memory, rm: Rm) -> Result<u64> {
        match rm {
            Rm::Reg(r) => Ok(self.mmx[r as usize]),
            Rm::Mem(addr) => Ok(mem.read_u32(addr)? as u64),
        }
    }

    #[inline(always)]
    fn write_mmx_rm64(&mut self, mem: &mut Memory, rm: Rm, value: u64) -> Result<()> {
        match rm {
            Rm::Reg(r) => {
                self.mmx[r as usize] = value;
                Ok(())
            }
            Rm::Mem(addr) => {
                mem.write_u32(addr, value as u32)?;
                mem.write_u32(addr.wrapping_add(4), (value >> 32) as u32)?;
                Ok(())
            }
        }
    }

    fn exec_mmx_rm_op(&mut self, mem: &mut Memory, prefixes: Prefixes, op: u8) -> Result<()> {
        let modrm = self.fetch_modrm(mem, prefixes)?;
        let dst = self.mmx[modrm.reg as usize];
        let src = if matches!(op, 0x60 | 0x61 | 0x62) {
            self.read_mmx_rm32_low(mem, modrm.rm)?
        } else {
            self.read_mmx_rm64(mem, modrm.rm)?
        };
        self.mmx[modrm.reg as usize] = match op {
            0x60 => mmx_punpcklbw(dst, src),
            0x61 => mmx_punpcklwd(dst, src),
            0x62 => mmx_punpckldq(dst, src),
            0x68 => mmx_punpckhbw(dst, src),
            0x74 => mmx_pcmpeqb(dst, src),
            0xd3 => mmx_shift_qword(dst, src, false),
            0xdb => dst & src,
            0xdf => (!dst) & src,
            0xeb => dst | src,
            0xf3 => mmx_shift_qword(dst, src, true),
            _ => unreachable!(),
        };
        Ok(())
    }

    fn exec_mmx_imm_shift(
        &mut self,
        mem: &mut Memory,
        prefixes: Prefixes,
        start_eip: u32,
    ) -> Result<()> {
        let modrm = self.fetch_modrm(mem, prefixes)?;
        let imm = self.fetch_u8(mem)? as u64;
        let Rm::Reg(r) = modrm.rm else {
            return Err(Error::Cpu(format!(
                "unsupported MMX shift memory operand at {start_eip:08x}"
            )));
        };
        self.mmx[r as usize] = match modrm.reg {
            2 => mmx_shift_qword(self.mmx[r as usize], imm, false),
            6 => mmx_shift_qword(self.mmx[r as usize], imm, true),
            _ => {
                return Err(Error::Cpu(format!(
                    "unsupported opcode 0f 73 /{} at {start_eip:08x}",
                    modrm.reg
                )));
            }
        };
        Ok(())
    }

    #[inline(always)]
    fn read_reg_width(&self, reg: Reg, bits: u32) -> u32 {
        match bits {
            8 => self.get_reg8(reg as u8) as u32,
            16 => self.reg(reg) & 0xffff,
            32 => self.reg(reg),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn write_reg_width(&mut self, reg: Reg, bits: u32, value: u32) {
        match bits {
            8 => self.set_reg8(reg as u8, value as u8),
            16 => {
                let old = self.reg(reg);
                self.set_reg(reg, (old & 0xffff_0000) | (value & 0xffff));
            }
            32 => self.set_reg(reg, value),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn get_reg8(&self, idx: u8) -> u8 {
        let reg = Reg::from_u3(idx & 3);
        let value = self.reg(reg);
        if idx < 4 {
            value as u8
        } else {
            (value >> 8) as u8
        }
    }

    #[inline(always)]
    fn set_reg8(&mut self, idx: u8, value: u8) {
        let reg = Reg::from_u3(idx & 3);
        let old = self.reg(reg);
        let new = if idx < 4 {
            (old & !0xff) | value as u32
        } else {
            (old & !0xff00) | ((value as u32) << 8)
        };
        self.set_reg(reg, new);
    }

    #[inline(always)]
    fn push_u32(&mut self, mem: &mut Memory, value: u32) -> Result<()> {
        let esp = self.reg(Reg::Esp).wrapping_sub(4);
        self.set_reg(Reg::Esp, esp);
        mem.write_u32(esp, value)
    }

    #[inline(always)]
    fn pop_u32(&mut self, mem: &Memory) -> Result<u32> {
        let esp = self.reg(Reg::Esp);
        let value = mem.read_u32(esp)?;
        self.set_reg(Reg::Esp, esp.wrapping_add(4));
        Ok(value)
    }

    #[cfg(debug_assertions)]
    fn debug_record_call(
        &mut self,
        call_site: u32,
        target: u32,
        ret_addr: u32,
        esp_before_call: u32,
    ) -> Result<()> {
        let ret_stack_esp = self.reg(Reg::Esp);
        let expected = esp_before_call.wrapping_sub(4);
        if ret_stack_esp != expected {
            return Err(Error::Cpu(format!(
                "debug call stack assert at {call_site:08x}: call target={target:08x} ret={ret_addr:08x} left esp={ret_stack_esp:08x}, expected {expected:08x}"
            )));
        }
        self.debug_call_stack.push(DebugCallFrame {
            call_site,
            target,
            ret_addr,
            call_esp: esp_before_call,
            ret_stack_esp,
        });
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    fn debug_record_call(
        &mut self,
        _call_site: u32,
        _target: u32,
        _ret_addr: u32,
        _esp_before_call: u32,
    ) -> Result<()> {
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn debug_finish_call_return(
        &mut self,
        ret_site: u32,
        ret_addr: u32,
        ret_stack_esp: u32,
        arg_bytes: u32,
        kind: &str,
    ) -> Result<()> {
        let Some(frame) = self.debug_call_stack.last() else {
            return Ok(());
        };
        if ret_stack_esp != frame.ret_stack_esp {
            if ret_addr == frame.ret_addr && ret_stack_esp < frame.ret_stack_esp {
                // MSVC SEH prolog helpers build a registration frame, push the
                // original return address, and RET through that synthetic slot.
                self.debug_call_stack.pop();
                return Ok(());
            }
            return Err(Error::Cpu(format!(
                "debug stack imbalance before {kind} at {ret_site:08x}: esp={ret_stack_esp:08x}, expected return slot {expected:08x} from call {call_site:08x} -> {target:08x} ret={ret_addr_expected:08x}",
                expected = frame.ret_stack_esp,
                call_site = frame.call_site,
                target = frame.target,
                ret_addr_expected = frame.ret_addr,
            )));
        }
        if ret_addr != frame.ret_addr {
            return Err(Error::Cpu(format!(
                "debug return address mismatch at {ret_site:08x}: stack ret={ret_addr:08x}, expected {expected:08x} from call {call_site:08x} -> {target:08x}",
                expected = frame.ret_addr,
                call_site = frame.call_site,
                target = frame.target,
            )));
        }
        let actual_post_esp = ret_stack_esp.wrapping_add(4).wrapping_add(arg_bytes);
        let expected_min_post_esp = frame.call_esp;
        if actual_post_esp < expected_min_post_esp {
            return Err(Error::Cpu(format!(
                "debug stack under-pop at {ret_site:08x}: post-esp={actual_post_esp:08x}, call esp was {expected_min_post_esp:08x}"
            )));
        }
        self.debug_call_stack.pop();
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    pub fn debug_finish_call_return(
        &mut self,
        _ret_site: u32,
        _ret_addr: u32,
        _ret_stack_esp: u32,
        _arg_bytes: u32,
        _kind: &str,
    ) -> Result<()> {
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn debug_replace_top_call(
        &mut self,
        call_site: u32,
        target: u32,
        ret_addr: u32,
        call_esp: u32,
        ret_stack_esp: u32,
    ) -> Result<()> {
        let Some(frame) = self.debug_call_stack.last_mut() else {
            return Ok(());
        };
        frame.call_site = call_site;
        frame.target = target;
        frame.ret_addr = ret_addr;
        frame.call_esp = call_esp;
        frame.ret_stack_esp = ret_stack_esp;
        Ok(())
    }

    #[cfg(debug_assertions)]
    pub fn debug_push_synthetic_call(
        &mut self,
        call_site: u32,
        target: u32,
        ret_addr: u32,
        call_esp: u32,
        ret_stack_esp: u32,
    ) -> Result<()> {
        self.debug_call_stack.push(DebugCallFrame {
            call_site,
            target,
            ret_addr,
            call_esp,
            ret_stack_esp,
        });
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    pub fn debug_replace_top_call(
        &mut self,
        _call_site: u32,
        _target: u32,
        _ret_addr: u32,
        _call_esp: u32,
        _ret_stack_esp: u32,
    ) -> Result<()> {
        Ok(())
    }

    #[cfg(not(debug_assertions))]
    pub fn debug_push_synthetic_call(
        &mut self,
        _call_site: u32,
        _target: u32,
        _ret_addr: u32,
        _call_esp: u32,
        _ret_stack_esp: u32,
    ) -> Result<()> {
        Ok(())
    }

    #[inline(always)]
    fn fetch_u8(&mut self, mem: &Memory) -> Result<u8> {
        let v = mem.read_u8(self.eip)?;
        self.eip = self.eip.wrapping_add(1);
        Ok(v)
    }

    #[inline(always)]
    fn fetch_i8(&mut self, mem: &Memory) -> Result<i8> {
        Ok(self.fetch_u8(mem)? as i8)
    }

    #[inline(always)]
    fn fetch_u16(&mut self, mem: &Memory) -> Result<u16> {
        let v = mem.read_u16(self.eip)?;
        self.eip = self.eip.wrapping_add(2);
        Ok(v)
    }

    #[inline(always)]
    fn fetch_u32(&mut self, mem: &Memory) -> Result<u32> {
        let v = mem.read_u32(self.eip)?;
        self.eip = self.eip.wrapping_add(4);
        Ok(v)
    }

    #[inline(always)]
    fn fetch_i32(&mut self, mem: &Memory) -> Result<i32> {
        Ok(self.fetch_u32(mem)? as i32)
    }

    #[inline(always)]
    fn fetch_imm(&mut self, mem: &Memory, bits: u32) -> Result<u32> {
        match bits {
            16 => Ok(self.fetch_u16(mem)? as u32),
            32 => self.fetch_u32(mem),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn exec_alu(&mut self, op: AluOp, lhs: u32, rhs: u32, bits: u32) -> u32 {
        match op {
            // ADD without carry-in.
            AluOp::Add => self.add(lhs, rhs, bits, false),
            // ADC includes carry-in.
            AluOp::Adc => self.add(lhs, rhs, bits, self.flag(FLAG_CF)),
            // SUB and CMP share flag calculation; CMP suppresses the writeback.
            AluOp::Sub | AluOp::Cmp => self.sub(lhs, rhs, bits, false),
            // SBB includes borrow via CF.
            AluOp::Sbb => self.sub(lhs, rhs, bits, self.flag(FLAG_CF)),
            // AND clears carry/overflow through logic flag handling.
            AluOp::And => {
                let r = lhs & rhs;
                self.set_logic_flags(r, bits);
                r
            }
            // OR clears carry/overflow through logic flag handling.
            AluOp::Or => {
                let r = lhs | rhs;
                self.set_logic_flags(r, bits);
                r
            }
            // XOR clears carry/overflow through logic flag handling.
            AluOp::Xor => {
                let r = lhs ^ rhs;
                self.set_logic_flags(r, bits);
                r
            }
        }
    }

    #[inline(always)]
    fn add(&mut self, lhs: u32, rhs: u32, bits: u32, carry: bool) -> u32 {
        let mask = width_mask(bits);
        let c = carry as u64;
        let wide = (lhs & mask) as u64 + (rhs & mask) as u64 + c;
        let res = (wide as u32) & mask;
        self.set_szp_flags(res, bits);
        self.set_flag(FLAG_CF, wide > mask as u64);
        self.set_flag(FLAG_AF, ((lhs ^ rhs ^ res) & 0x10) != 0);
        self.set_flag(
            FLAG_OF,
            ((!(lhs ^ rhs) & (lhs ^ res)) & sign_bit(bits)) != 0,
        );
        res
    }

    #[inline(always)]
    fn sub(&mut self, lhs: u32, rhs: u32, bits: u32, borrow: bool) -> u32 {
        let mask = width_mask(bits);
        let b = borrow as u32;
        let rhsb = (rhs & mask).wrapping_add(b);
        let res = lhs.wrapping_sub(rhsb) & mask;
        self.set_szp_flags(res, bits);
        self.set_flag(FLAG_CF, (lhs & mask) < rhsb);
        self.set_flag(FLAG_AF, ((lhs ^ rhsb ^ res) & 0x10) != 0);
        self.set_flag(
            FLAG_OF,
            (((lhs ^ rhsb) & (lhs ^ res)) & sign_bit(bits)) != 0,
        );
        res
    }

    #[inline(always)]
    fn set_logic_flags(&mut self, res: u32, bits: u32) {
        self.set_szp_flags(res, bits);
        self.set_flag(FLAG_CF, false);
        self.set_flag(FLAG_OF, false);
        self.set_flag(FLAG_AF, false);
    }

    #[inline(always)]
    fn set_shift_result_flags(&mut self, res: u32, bits: u32) {
        self.set_szp_flags(res, bits);
        self.set_flag(FLAG_OF, false);
        self.set_flag(FLAG_AF, false);
    }

    #[inline(always)]
    fn set_szp_flags(&mut self, res: u32, bits: u32) {
        let masked = res & width_mask(bits);
        self.set_flag(FLAG_ZF, masked == 0);
        self.set_flag(FLAG_SF, (masked & sign_bit(bits)) != 0);
        self.set_flag(FLAG_PF, (masked as u8).count_ones() % 2 == 0);
    }

    #[inline(always)]
    fn set_imul_flags(&mut self, res: u32, bits: u32) {
        let sign = sign_width(res, bits);
        let overflow = match bits {
            8 => sign as i32 != sign as i8 as i32,
            16 => sign as i32 != sign as i16 as i32,
            32 => false,
            _ => false,
        };
        self.set_flag(FLAG_CF, overflow);
        self.set_flag(FLAG_OF, overflow);
    }

    #[inline(always)]
    fn cond(&self, cc: u8) -> bool {
        match cc {
            0x0 => self.flag(FLAG_OF),
            0x1 => !self.flag(FLAG_OF),
            0x2 => self.flag(FLAG_CF),
            0x3 => !self.flag(FLAG_CF),
            0x4 => self.flag(FLAG_ZF),
            0x5 => !self.flag(FLAG_ZF),
            0x6 => self.flag(FLAG_CF) || self.flag(FLAG_ZF),
            0x7 => !self.flag(FLAG_CF) && !self.flag(FLAG_ZF),
            0x8 => self.flag(FLAG_SF),
            0x9 => !self.flag(FLAG_SF),
            0xa => self.flag(FLAG_PF),
            0xb => !self.flag(FLAG_PF),
            0xc => self.flag(FLAG_SF) != self.flag(FLAG_OF),
            0xd => self.flag(FLAG_SF) == self.flag(FLAG_OF),
            0xe => self.flag(FLAG_ZF) || (self.flag(FLAG_SF) != self.flag(FLAG_OF)),
            0xf => !self.flag(FLAG_ZF) && (self.flag(FLAG_SF) == self.flag(FLAG_OF)),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn bump_si_di(&mut self, elem: u32, si: bool, di: bool) {
        if si {
            self.bump_si(elem);
        }
        if di {
            self.bump_di(elem);
        }
    }

    #[inline(always)]
    fn bump_si(&mut self, elem: u32) {
        let delta = if self.flag(FLAG_DF) {
            0u32.wrapping_sub(elem)
        } else {
            elem
        };
        self.set_reg(Reg::Esi, self.reg(Reg::Esi).wrapping_add(delta));
    }

    #[inline(always)]
    fn bump_di(&mut self, elem: u32) {
        let delta = if self.flag(FLAG_DF) {
            0u32.wrapping_sub(elem)
        } else {
            elem
        };
        self.set_reg(Reg::Edi, self.reg(Reg::Edi).wrapping_add(delta));
    }

    #[inline(always)]
    fn flag(&self, flag: u32) -> bool {
        (self.eflags & flag) != 0
    }

    #[inline(always)]
    fn set_flag(&mut self, flag: u32, value: bool) {
        if value {
            self.eflags |= flag;
        } else {
            self.eflags &= !flag;
        }
        self.eflags |= 0x2;
    }

    fn unsupported<T>(&self, op: u8, start_eip: u32) -> Result<T> {
        Err(Error::Cpu(format!(
            "unsupported opcode {op:02x} at {start_eip:08x}"
        )))
    }

    fn unsupported_0f<T>(&self, op: u8, start_eip: u32) -> Result<T> {
        Err(Error::Cpu(format!(
            "unsupported opcode 0f {op:02x} at {start_eip:08x}"
        )))
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AluOp {
    Add,
    Adc,
    Sub,
    Sbb,
    And,
    Or,
    Xor,
    Cmp,
}

#[inline(always)]
fn width_mask(bits: u32) -> u32 {
    match bits {
        8 => 0xff,
        16 => 0xffff,
        32 => 0xffff_ffff,
        _ => unreachable!(),
    }
}

#[inline(always)]
fn sign_bit(bits: u32) -> u32 {
    1u32 << (bits - 1)
}

#[inline(always)]
fn mask_width(value: u32, bits: u32) -> u32 {
    value & width_mask(bits)
}

#[inline(always)]
fn sign_width(value: u32, bits: u32) -> u32 {
    match bits {
        8 => value as u8 as i8 as i32 as u32,
        16 => value as u16 as i16 as i32 as u32,
        32 => value,
        _ => unreachable!(),
    }
}

fn mmx_punpcklbw(dst: u64, src: u64) -> u64 {
    let mut out = 0u64;
    for i in 0..4 {
        let d = (dst >> (i * 8)) & 0xff;
        let s = (src >> (i * 8)) & 0xff;
        out |= d << (i * 16);
        out |= s << (i * 16 + 8);
    }
    out
}

fn mmx_punpckhbw(dst: u64, src: u64) -> u64 {
    let mut out = 0u64;
    for i in 0..4 {
        let shift = (i + 4) * 8;
        let d = (dst >> shift) & 0xff;
        let s = (src >> shift) & 0xff;
        out |= d << (i * 16);
        out |= s << (i * 16 + 8);
    }
    out
}

fn mmx_punpcklwd(dst: u64, src: u64) -> u64 {
    let mut out = 0u64;
    for i in 0..2 {
        let d = (dst >> (i * 16)) & 0xffff;
        let s = (src >> (i * 16)) & 0xffff;
        out |= d << (i * 32);
        out |= s << (i * 32 + 16);
    }
    out
}

fn mmx_punpckldq(dst: u64, src: u64) -> u64 {
    (dst & 0xffff_ffff) | ((src & 0xffff_ffff) << 32)
}

fn mmx_pcmpeqb(dst: u64, src: u64) -> u64 {
    let mut out = 0u64;
    for i in 0..8 {
        let shift = i * 8;
        if ((dst ^ src) >> shift) & 0xff == 0 {
            out |= 0xffu64 << shift;
        }
    }
    out
}

fn mmx_shift_qword(value: u64, count: u64, left: bool) -> u64 {
    if count > 63 {
        0
    } else if left {
        value << count
    } else {
        value >> count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::PagePerm;

    const CODE: u32 = 0x0001_0000;
    const STACK: u32 = 0x0002_0000;

    #[test]
    fn mov_add_ret() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.map(STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        mem.write_bytes(CODE, &[0xb8, 1, 0, 0, 0, 0x83, 0xc0, 2, 0xc3])
            .unwrap();
        mem.write_u32(STACK + 0xffc, 0x2000).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Esp, STACK + 0xffc);
        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.reg(Reg::Eax), 3);
        assert_eq!(cpu.eip, 0x2000);
    }

    #[test]
    fn x87_fild_m16int() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.map(STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        mem.write_bytes(CODE, &[0xdf, 0x44, 0x24, 0x0c]).unwrap();
        mem.write_u16(STACK + 0xffc, (-123i16) as u16).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Esp, STACK + 0xff0);
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.x87_peek(), -123.0);
    }

    #[test]
    fn x87_fistp_m32int() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.map(STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        mem.write_bytes(CODE, &[0xdb, 0x1c, 0x24]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Esp, STACK + 0xff0);
        cpu.x87_push(-123.5);
        cpu.step(&mut mem).unwrap();
        assert_eq!(mem.read_u32(STACK + 0xff0).unwrap() as i32, -124);
        assert!(cpu.x87.is_empty());
    }

    #[test]
    fn x87_ftst_updates_status_word() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0xd9, 0xe4, 0xdf, 0xe0]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(-1.0);

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.reg(Reg::Eax) & 0xffff, X87_STATUS_C0 as u32);
    }

    #[test]
    fn x87_fstsw_m16_stores_status_word() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x2000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        let status_addr = CODE + 0x1000;
        let mut code = vec![0xd9, 0xe4, 0xdd, 0x3d];
        code.extend_from_slice(&status_addr.to_le_bytes());
        mem.write_bytes(CODE, &code).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(0.0);

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();

        assert_eq!(mem.read_u16(status_addr).unwrap(), X87_STATUS_C3);
    }

    #[test]
    fn x87_fsqrt_updates_st0() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0xd9, 0xfa]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(9.0);

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.x87_peek(), 3.0);
    }

    #[test]
    fn x87_fprem_completes_and_clears_c2() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0xd9, 0xf8, 0xdf, 0xe0]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(3.0);
        cpu.x87_push(10.0);

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.x87_peek(), 1.0);
        assert_eq!(cpu.reg(Reg::Eax) & X87_STATUS_C2 as u32, 0);
        assert_ne!(cpu.reg(Reg::Eax) & X87_STATUS_C1 as u32, 0);
        assert_ne!(cpu.reg(Reg::Eax) & X87_STATUS_C3 as u32, 0);
    }

    #[test]
    fn x87_fxch_swaps_st0_with_sti() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0xd9, 0xc9]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(2.0);
        cpu.x87_push(1.0);

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.x87_get(0), 2.0);
        assert_eq!(cpu.x87_get(1), 1.0);
    }

    #[test]
    fn x87_dc_register_sub_and_div_write_sti_with_v86_order() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0xdc, 0xe1, 0xdc, 0xf1]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(10.0);
        cpu.x87_push(2.0);

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.x87_get(0), 2.0);
        assert_eq!(cpu.x87_get(1), -8.0);

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.x87_get(0), 2.0);
        assert_eq!(cpu.x87_get(1), -0.25);
    }

    #[test]
    fn x87_m80real_round_trips_through_memory() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x2000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        let data = CODE + 0x1000;
        let mut code = vec![0xdb, 0x3d];
        code.extend_from_slice(&data.to_le_bytes());
        code.extend_from_slice(&[0xdb, 0x2d]);
        code.extend_from_slice(&data.to_le_bytes());
        mem.write_bytes(CODE, &code).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(1.5);

        cpu.step(&mut mem).unwrap();
        assert!(cpu.x87.is_empty());
        cpu.step(&mut mem).unwrap();

        assert!((cpu.x87_peek() - 1.5).abs() < 0.000001);
    }

    #[test]
    fn x87_fldenv_restores_condition_status() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x2000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        let env = CODE + 0x1000;
        let mut code = vec![0xd9, 0xe4, 0xd9, 0x35];
        code.extend_from_slice(&env.to_le_bytes());
        code.extend_from_slice(&[0xdb, 0xe3, 0xd9, 0x25]);
        code.extend_from_slice(&env.to_le_bytes());
        code.extend_from_slice(&[0xdf, 0xe0]);
        mem.write_bytes(CODE, &code).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.x87_push(-1.0);

        for _ in 0..5 {
            cpu.step(&mut mem).unwrap();
        }

        assert_eq!(
            cpu.reg(Reg::Eax) & X87_STATUS_C0 as u32,
            X87_STATUS_C0 as u32
        );
    }

    #[test]
    fn bswap_r32() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0x0f, 0xc8]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Eax, 0x1234_5678);
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.reg(Reg::Eax), 0x7856_3412);
    }

    #[test]
    fn xlatb_translates_al_from_ebx_table() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0xd7]).unwrap();
        mem.write_u8(CODE + 0x123, 0xab).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Eax, 0x1234_5623);
        cpu.set_reg(Reg::Ebx, CODE + 0x100);

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.reg(Reg::Eax), 0x1234_56ab);
    }

    #[test]
    fn bit_scan_sets_index_and_zf() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0x0f, 0xbc, 0xc1, 0x0f, 0xbd, 0xd1])
            .unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Ecx, 0x8000_0010);

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.reg(Reg::Eax), 4);
        assert!(!cpu.flag(FLAG_ZF));

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.reg(Reg::Edx), 31);
        assert!(!cpu.flag(FLAG_ZF));
    }

    #[test]
    fn bit_scan_zero_source_sets_zf_and_preserves_destination() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0x0f, 0xbc, 0xc1]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Eax, 0x1234_5678);
        cpu.set_reg(Reg::Ecx, 0);

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.reg(Reg::Eax), 0x1234_5678);
        assert!(cpu.flag(FLAG_ZF));
    }

    #[test]
    fn xadd_m32_r32_exchanges_and_sets_add_flags() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x2000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        let data = CODE + 0x1000;
        let mut code = vec![0x0f, 0xc1, 0x0d];
        code.extend_from_slice(&data.to_le_bytes());
        mem.write_bytes(CODE, &code).unwrap();
        mem.write_u32(data, 0xffff_ffff).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Ecx, 2);

        cpu.step(&mut mem).unwrap();

        assert_eq!(mem.read_u32(data).unwrap(), 1);
        assert_eq!(cpu.reg(Reg::Ecx), 0xffff_ffff);
        assert!(cpu.flag(FLAG_CF));
        assert!(!cpu.flag(FLAG_ZF));
    }

    #[test]
    fn xadd_r16_r16_preserves_upper_halves() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0x66, 0x0f, 0xc1, 0xd1]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Ecx, 0xaaaa_0003);
        cpu.set_reg(Reg::Edx, 0xbbbb_0004);

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.reg(Reg::Ecx), 0xaaaa_0007);
        assert_eq!(cpu.reg(Reg::Edx), 0xbbbb_0003);
        assert!(!cpu.flag(FLAG_CF));
        assert!(!cpu.flag(FLAG_ZF));
    }

    #[test]
    fn inc_dec_r16_preserve_upper_halves_and_set_16_bit_flags() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(CODE, &[0x66, 0x49, 0x66, 0x40]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Ecx, 0xaaaa_0001);
        cpu.set_reg(Reg::Eax, 0xbbbb_ffff);
        cpu.set_flag(FLAG_CF, true);

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.reg(Reg::Ecx), 0xaaaa_0000);
        assert!(cpu.flag(FLAG_ZF));
        assert!(cpu.flag(FLAG_CF));

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.reg(Reg::Eax), 0xbbbb_0000);
        assert!(cpu.flag(FLAG_ZF));
        assert!(cpu.flag(FLAG_CF));
    }

    #[test]
    fn shr_preserves_carry_for_following_adc() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.write_bytes(
            CODE,
            &[
                0xb9, 5, 0, 0, 0, // mov ecx,5
                0xd1, 0xe9, // shr ecx,1
                0x83, 0xd1, 0, // adc ecx,0
            ],
        )
        .unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();
        assert!(cpu.flag(FLAG_CF));
        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.reg(Reg::Ecx), 3);
    }

    #[test]
    fn rich4_odd_word_row_copy_keeps_tail_word() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x5000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        let src = CODE + 0x1202;
        let dst = CODE + 0x2400;
        let mut code = vec![0xbe];
        code.extend_from_slice(&src.to_le_bytes()); // mov esi,src
        code.push(0xbf);
        code.extend_from_slice(&dst.to_le_bytes()); // mov edi,dst
        code.extend_from_slice(&[0xb9, 6, 0, 0, 0]); // mov ecx,6 words
        code.extend_from_slice(&[0xf7, 0xc6, 3, 0, 0, 0]); // test esi,3
        code.extend_from_slice(&[0x74, 0x05]); // jz aligned
        code.extend_from_slice(&[0x66, 0xa5]); // movsw
        code.push(0x49); // dec ecx
        code.extend_from_slice(&[0x74, 0x0a]); // je done
        code.extend_from_slice(&[0xd1, 0xe9]); // aligned: shr ecx,1
        code.extend_from_slice(&[0xf3, 0xa5]); // rep movsd
        code.extend_from_slice(&[0x83, 0xd1, 0]); // adc ecx,0
        code.extend_from_slice(&[0xf3, 0x66, 0xa5]); // rep movsw
        mem.write_bytes(CODE, &code).unwrap();
        let expected: Vec<u8> = (1..=12).collect();
        mem.write_bytes(src, &expected).unwrap();
        mem.write_bytes(dst, &[0xee; 12]).unwrap();
        let mut cpu = Cpu::new();
        cpu.eip = CODE;

        for _ in 0..12 {
            cpu.step(&mut mem).unwrap();
        }

        assert_eq!(mem.read_bytes(dst, 12).unwrap(), expected);
    }

    #[test]
    fn mmx_punpcklbw_memory_source_reads_only_dword() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.map(STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        mem.map(0x0003_0000, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        mem.map(0x0004_0000, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        let src_qword = 0x0003_0100;
        let src_dword = 0x0003_0ffc;
        let dst = 0x0004_0000u32;
        mem.write_bytes(src_qword, &[0, 1, 2, 3, 4, 5, 6, 7])
            .unwrap();
        mem.write_bytes(src_dword, &[0xa0, 0xa1, 0xa2, 0xa3])
            .unwrap();

        let mut code = Vec::new();
        code.extend_from_slice(&[0x0f, 0x6f, 0x05]);
        code.extend_from_slice(&src_qword.to_le_bytes()); // movq mm0,[src_qword]
        code.extend_from_slice(&[0x0f, 0x60, 0x05]);
        code.extend_from_slice(&src_dword.to_le_bytes()); // punpcklbw mm0,[src_dword]
        code.extend_from_slice(&[0x0f, 0x7f, 0x05]);
        code.extend_from_slice(&dst.to_le_bytes()); // movq [dst],mm0
        code.push(0xc3);
        mem.write_bytes(CODE, &code).unwrap();
        mem.write_u32(STACK + 0xffc, 0).unwrap();

        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Esp, STACK + 0xffc);
        for _ in 0..4 {
            cpu.step(&mut mem).unwrap();
        }

        let bytes = mem.read_bytes(dst, 8).unwrap();
        assert_eq!(
            u64::from_le_bytes(bytes.try_into().unwrap()),
            0xa303_a202_a101_a000
        );
    }

    #[test]
    fn mmx_smacker_scalar_logic_and_qword_shifts() {
        let mut mem = Memory::new();
        mem.map(
            CODE,
            0x1000,
            PagePerm::READ | PagePerm::WRITE | PagePerm::EXEC,
        )
        .unwrap();
        mem.map(STACK, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        mem.map(0x0003_0000, 0x1000, PagePerm::READ | PagePerm::WRITE)
            .unwrap();
        let src = 0x0003_0000;
        let dst = 0x0003_0010u32;
        mem.write_bytes(src, &0x0123_4567_89ab_cdefu64.to_le_bytes())
            .unwrap();

        let mut code = Vec::new();
        code.extend_from_slice(&[0x0f, 0x6f, 0x05]);
        code.extend_from_slice(&src.to_le_bytes()); // movq mm0,[src]
        code.extend_from_slice(&[0xb8, 0x0f, 0x00, 0x00, 0x00]); // mov eax,15
        code.extend_from_slice(&[0x0f, 0x6e, 0xc8]); // movd mm1,eax
        code.extend_from_slice(&[0x0f, 0x6f, 0xd0]); // movq mm2,mm0
        code.extend_from_slice(&[0x0f, 0xdb, 0xd1]); // pand mm2,mm1
        code.extend_from_slice(&[0x0f, 0xdf, 0xc1]); // pandn mm0,mm1
        code.extend_from_slice(&[0x0f, 0xeb, 0xc2]); // por mm0,mm2
        code.extend_from_slice(&[0x0f, 0x73, 0xf0, 0x04]); // psllq mm0,4
        code.extend_from_slice(&[0x0f, 0x73, 0xd0, 0x04]); // psrlq mm0,4
        code.extend_from_slice(&[0x0f, 0x7e, 0xc0]); // movd eax,mm0
        code.extend_from_slice(&[0x0f, 0x7f, 0x05]);
        code.extend_from_slice(&dst.to_le_bytes()); // movq [dst],mm0
        code.push(0xc3);
        mem.write_bytes(CODE, &code).unwrap();
        mem.write_u32(STACK + 0xffc, 0).unwrap();

        let mut cpu = Cpu::new();
        cpu.eip = CODE;
        cpu.set_reg(Reg::Esp, STACK + 0xffc);
        for _ in 0..12 {
            cpu.step(&mut mem).unwrap();
        }

        let expected =
            (((!0x0123_4567_89ab_cdefu64) & 0x0f) | (0x0123_4567_89ab_cdefu64 & 0x0f)) << 4 >> 4;
        let bytes = mem.read_bytes(dst, 8).unwrap();
        assert_eq!(u64::from_le_bytes(bytes.try_into().unwrap()), expected);
        assert_eq!(cpu.reg(Reg::Eax), expected as u32);
    }
}
