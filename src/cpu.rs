//! Sharp LR35902 (the Game Boy's Z80-ish CPU).
//!
//! Instructions are decoded from the opcode bit fields (x = bits 7-6, y = bits 5-3,
//! z = bits 2-0, p = y>>1, q = y&1) instead of a 256-entry table; this is the standard
//! decomposition of the Z80 opcode map. Every instruction returns its length in T-cycles.

use crate::bus::Bus;

const Z: u8 = 0x80;
const N: u8 = 0x40;
const H: u8 = 0x20;
const C: u8 = 0x10;

pub struct Cpu {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: bool,
    ei_pending: bool,
    halted: bool,
    halt_bug: bool,
    pub bus: Bus,
}

fn flags(z: bool, n: bool, h: bool, c: bool) -> u8 {
    (z as u8) << 7 | (n as u8) << 6 | (h as u8) << 5 | (c as u8) << 4
}

impl Cpu {
    /// Create a CPU in the state the boot ROM leaves behind (DMG).
    pub fn new(bus: Bus) -> Self {
        Self {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
            ime: false,
            ei_pending: false,
            halted: false,
            halt_bug: false,
            bus,
        }
    }

    // ---- register helpers ----
    fn bc(&self) -> u16 { (self.b as u16) << 8 | self.c as u16 }
    fn de(&self) -> u16 { (self.d as u16) << 8 | self.e as u16 }
    fn hl(&self) -> u16 { (self.h as u16) << 8 | self.l as u16 }
    fn set_hl(&mut self, v: u16) { self.h = (v >> 8) as u8; self.l = v as u8; }

    fn hl_inc(&mut self) -> u16 { let v = self.hl(); self.set_hl(v.wrapping_add(1)); v }
    fn hl_dec(&mut self) -> u16 { let v = self.hl(); self.set_hl(v.wrapping_sub(1)); v }

    /// 16-bit register pair by index: 0=BC 1=DE 2=HL 3=SP
    fn rp(&self, i: u8) -> u16 {
        match i { 0 => self.bc(), 1 => self.de(), 2 => self.hl(), _ => self.sp }
    }
    fn set_rp(&mut self, i: u8, v: u16) {
        let (hi, lo) = ((v >> 8) as u8, v as u8);
        match i {
            0 => { self.b = hi; self.c = lo }
            1 => { self.d = hi; self.e = lo }
            2 => { self.h = hi; self.l = lo }
            _ => self.sp = v,
        }
    }

    /// 8-bit operand by index: 0=B 1=C 2=D 3=E 4=H 5=L 6=(HL) 7=A
    fn get_r(&mut self, i: u8) -> u8 {
        match i {
            0 => self.b, 1 => self.c, 2 => self.d, 3 => self.e,
            4 => self.h, 5 => self.l,
            6 => { let a = self.hl(); self.bus.read(a) }
            _ => self.a,
        }
    }
    fn set_r(&mut self, i: u8, v: u8) {
        match i {
            0 => self.b = v, 1 => self.c = v, 2 => self.d = v, 3 => self.e = v,
            4 => self.h = v, 5 => self.l = v,
            6 => { let a = self.hl(); self.bus.write(a, v) }
            _ => self.a = v,
        }
    }

    fn cond(&self, i: u8) -> bool {
        match i & 3 {
            0 => self.f & Z == 0,
            1 => self.f & Z != 0,
            2 => self.f & C == 0,
            _ => self.f & C != 0,
        }
    }

    // ---- memory / stack ----
    fn fetch(&mut self) -> u8 {
        let v = self.bus.read(self.pc);
        if self.halt_bug {
            self.halt_bug = false; // HALT bug: the byte after HALT is read twice
        } else {
            self.pc = self.pc.wrapping_add(1);
        }
        v
    }
    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch() as u16;
        let hi = self.fetch() as u16;
        hi << 8 | lo
    }
    fn push(&mut self, v: u16) {
        self.sp = self.sp.wrapping_sub(1);
        self.bus.write(self.sp, (v >> 8) as u8);
        self.sp = self.sp.wrapping_sub(1);
        self.bus.write(self.sp, v as u8);
    }
    fn pop(&mut self) -> u16 {
        let lo = self.bus.read(self.sp) as u16;
        self.sp = self.sp.wrapping_add(1);
        let hi = self.bus.read(self.sp) as u16;
        self.sp = self.sp.wrapping_add(1);
        hi << 8 | lo
    }

    // ---- ALU ----
    fn alu(&mut self, op: u8, v: u8) {
        let a = self.a;
        let carry = (self.f & C != 0) as u8;
        match op {
            0 | 1 => {
                let c = if op == 1 { carry } else { 0 };
                let r = a as u16 + v as u16 + c as u16;
                self.a = r as u8;
                self.f = flags(self.a == 0, false, (a & 0xF) + (v & 0xF) + c > 0xF, r > 0xFF);
            }
            2 | 3 | 7 => {
                let c = if op == 3 { carry } else { 0 };
                let r = a as i16 - v as i16 - c as i16;
                let res = r as u8;
                self.f = flags(res == 0, true, (a & 0xF) as i16 - (v & 0xF) as i16 - (c as i16) < 0, r < 0);
                if op != 7 { self.a = res }
            }
            4 => { self.a &= v; self.f = flags(self.a == 0, false, true, false) }
            5 => { self.a ^= v; self.f = flags(self.a == 0, false, false, false) }
            _ => { self.a |= v; self.f = flags(self.a == 0, false, false, false) }
        }
    }

    fn inc8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_add(1);
        self.f = (self.f & C) | flags(r == 0, false, v & 0xF == 0xF, false);
        r
    }
    fn dec8(&mut self, v: u8) -> u8 {
        let r = v.wrapping_sub(1);
        self.f = (self.f & C) | flags(r == 0, true, v & 0xF == 0, false);
        r
    }
    fn add_hl(&mut self, v: u16) {
        let hl = self.hl();
        let (r, c) = hl.overflowing_add(v);
        self.f = (self.f & Z) | flags(false, false, (hl & 0xFFF) + (v & 0xFFF) > 0xFFF, c);
        self.set_hl(r);
    }
    /// SP + signed immediate, with the quirky 8-bit flag calculation.
    fn add_sp(&mut self, e: u16) -> u16 {
        let sp = self.sp;
        self.f = flags(false, false, (sp & 0xF) + (e & 0xF) > 0xF, (sp & 0xFF) + (e & 0xFF) > 0xFF);
        sp.wrapping_add(e)
    }
    fn daa(&mut self) {
        let (mut a, n, h) = (self.a, self.f & N != 0, self.f & H != 0);
        let mut c = self.f & C != 0;
        if !n {
            if c || a > 0x99 { a = a.wrapping_add(0x60); c = true }
            if h || a & 0x0F > 9 { a = a.wrapping_add(6) }
        } else {
            if c { a = a.wrapping_sub(0x60) }
            if h { a = a.wrapping_sub(6) }
        }
        self.a = a;
        self.f = (self.f & N) | flags(a == 0, false, false, c);
    }

    /// CB-prefix rotate/shift group: 0 RLC 1 RRC 2 RL 3 RR 4 SLA 5 SRA 6 SWAP 7 SRL
    fn rot(&mut self, y: u8, v: u8) -> u8 {
        let cin = (self.f & C != 0) as u8;
        let (r, c) = match y {
            0 => (v.rotate_left(1), v >> 7),
            1 => (v.rotate_right(1), v & 1),
            2 => ((v << 1) | cin, v >> 7),
            3 => ((v >> 1) | (cin << 7), v & 1),
            4 => (v << 1, v >> 7),
            5 => ((v >> 1) | (v & 0x80), v & 1),
            6 => (v.rotate_left(4), 0),
            _ => (v >> 1, v & 1),
        };
        self.f = flags(r == 0, false, false, c == 1);
        r
    }

    fn exec_cb(&mut self) -> u32 {
        let op = self.fetch();
        let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
        let v = self.get_r(z);
        let mem = z == 6;
        match x {
            0 => {
                let r = self.rot(y, v);
                self.set_r(z, r);
                if mem { 16 } else { 8 }
            }
            1 => {
                self.f = (self.f & C) | H | if v & (1 << y) == 0 { Z } else { 0 };
                if mem { 12 } else { 8 }
            }
            2 => {
                self.set_r(z, v & !(1 << y));
                if mem { 16 } else { 8 }
            }
            _ => {
                self.set_r(z, v | (1 << y));
                if mem { 16 } else { 8 }
            }
        }
    }

    fn jr(&mut self, e: i8) {
        self.pc = self.pc.wrapping_add(e as i16 as u16);
    }

    /// Execute one instruction (or interrupt dispatch / halted idle) and tick the bus.
    /// Returns the T-cycles consumed.
    pub fn step(&mut self) -> u32 {
        let pending = self.bus.ie & self.bus.iflag & 0x1F;
        if self.halted {
            if pending != 0 {
                self.halted = false;
            } else {
                self.bus.tick(4);
                return 4;
            }
        }
        if self.ime && pending != 0 {
            self.ime = false;
            let bit = pending.trailing_zeros();
            self.bus.iflag &= !(1u8 << bit);
            let pc = self.pc;
            self.push(pc);
            self.pc = 0x40 + bit as u16 * 8;
            self.bus.tick(20);
            return 20;
        }
        // EI takes effect after the instruction that follows it.
        let enable_ime = std::mem::take(&mut self.ei_pending);
        let op = self.fetch();
        let cycles = self.exec(op);
        if enable_ime {
            self.ime = true;
        }
        self.bus.tick(cycles);
        cycles
    }

    fn exec(&mut self, op: u8) -> u32 {
        let (x, y, z) = (op >> 6, (op >> 3) & 7, op & 7);
        let (p, q) = (y >> 1, y & 1);
        match x {
            // ---------------- x = 0 ----------------
            0 => match z {
                0 => match y {
                    0 => 4,
                    1 => {
                        let a = self.fetch16();
                        let sp = self.sp;
                        self.bus.write(a, sp as u8);
                        self.bus.write(a.wrapping_add(1), (sp >> 8) as u8);
                        20
                    }
                    2 => { self.fetch(); 4 } // STOP (treated as a NOP)
                    3 => { let e = self.fetch() as i8; self.jr(e); 12 }
                    _ => {
                        let e = self.fetch() as i8;
                        if self.cond(y - 4) { self.jr(e); 12 } else { 8 }
                    }
                },
                1 => if q == 0 {
                    let v = self.fetch16();
                    self.set_rp(p, v);
                    12
                } else {
                    let v = self.rp(p);
                    self.add_hl(v);
                    8
                },
                2 => {
                    let a = match p { 0 => self.bc(), 1 => self.de(), 2 => self.hl_inc(), _ => self.hl_dec() };
                    if q == 0 {
                        let v = self.a;
                        self.bus.write(a, v);
                    } else {
                        self.a = self.bus.read(a);
                    }
                    8
                }
                3 => {
                    let v = self.rp(p);
                    self.set_rp(p, if q == 0 { v.wrapping_add(1) } else { v.wrapping_sub(1) });
                    8
                }
                4 => { let v = self.get_r(y); let r = self.inc8(v); self.set_r(y, r); if y == 6 { 12 } else { 4 } }
                5 => { let v = self.get_r(y); let r = self.dec8(v); self.set_r(y, r); if y == 6 { 12 } else { 4 } }
                6 => { let v = self.fetch(); self.set_r(y, v); if y == 6 { 12 } else { 8 } }
                _ => {
                    let cin = (self.f & C != 0) as u8;
                    match y {
                        0 => { let c = self.a >> 7; self.a = self.a.rotate_left(1); self.f = flags(false, false, false, c == 1) }
                        1 => { let c = self.a & 1; self.a = self.a.rotate_right(1); self.f = flags(false, false, false, c == 1) }
                        2 => { let c = self.a >> 7; self.a = (self.a << 1) | cin; self.f = flags(false, false, false, c == 1) }
                        3 => { let c = self.a & 1; self.a = (self.a >> 1) | (cin << 7); self.f = flags(false, false, false, c == 1) }
                        4 => self.daa(),
                        5 => { self.a = !self.a; self.f |= N | H }
                        6 => self.f = (self.f & Z) | C,
                        _ => self.f = (self.f & Z) | ((self.f & C) ^ C),
                    }
                    4
                }
            },
            // ---------------- x = 1: LD r,r' / HALT ----------------
            1 => {
                if op == 0x76 {
                    let pending = self.bus.ie & self.bus.iflag & 0x1F;
                    if !self.ime && pending != 0 {
                        self.halt_bug = true;
                    } else {
                        self.halted = true;
                    }
                    return 4;
                }
                let v = self.get_r(z);
                self.set_r(y, v);
                if y == 6 || z == 6 { 8 } else { 4 }
            }
            // ---------------- x = 2: ALU A,r ----------------
            2 => {
                let v = self.get_r(z);
                self.alu(y, v);
                if z == 6 { 8 } else { 4 }
            }
            // ---------------- x = 3 ----------------
            _ => match z {
                0 => match y {
                    0..=3 => if self.cond(y) { self.pc = self.pop(); 20 } else { 8 },
                    4 => { let a = 0xFF00 | self.fetch() as u16; let v = self.a; self.bus.write(a, v); 12 }
                    5 => { let e = self.fetch() as i8 as i16 as u16; self.sp = self.add_sp(e); 16 }
                    6 => { let a = 0xFF00 | self.fetch() as u16; self.a = self.bus.read(a); 12 }
                    _ => { let e = self.fetch() as i8 as i16 as u16; let v = self.add_sp(e); self.set_hl(v); 12 }
                },
                1 => if q == 0 {
                    let v = self.pop();
                    if p == 3 { self.a = (v >> 8) as u8; self.f = v as u8 & 0xF0 } else { self.set_rp(p, v) }
                    12
                } else {
                    match p {
                        0 => { self.pc = self.pop(); 16 }
                        1 => { self.pc = self.pop(); self.ime = true; 16 } // RETI
                        2 => { self.pc = self.hl(); 4 }
                        _ => { self.sp = self.hl(); 8 }
                    }
                },
                2 => match y {
                    0..=3 => { let a = self.fetch16(); if self.cond(y) { self.pc = a; 16 } else { 12 } }
                    4 => { let a = 0xFF00 | self.c as u16; let v = self.a; self.bus.write(a, v); 8 }
                    5 => { let a = self.fetch16(); let v = self.a; self.bus.write(a, v); 16 }
                    6 => { let a = 0xFF00 | self.c as u16; self.a = self.bus.read(a); 8 }
                    _ => { let a = self.fetch16(); self.a = self.bus.read(a); 16 }
                },
                3 => match y {
                    0 => { self.pc = self.fetch16(); 16 }
                    1 => self.exec_cb(),
                    6 => { self.ime = false; self.ei_pending = false; 4 } // DI
                    7 => { self.ei_pending = true; 4 }                    // EI
                    _ => 4, // invalid opcode: treat as NOP
                },
                4 => match y {
                    0..=3 => {
                        let a = self.fetch16();
                        if self.cond(y) { let pc = self.pc; self.push(pc); self.pc = a; 24 } else { 12 }
                    }
                    _ => 4,
                },
                5 => if q == 0 {
                    let v = if p == 3 { (self.a as u16) << 8 | self.f as u16 } else { self.rp(p) };
                    self.push(v);
                    16
                } else if p == 0 {
                    let a = self.fetch16();
                    let pc = self.pc;
                    self.push(pc);
                    self.pc = a;
                    24
                } else {
                    4
                },
                6 => { let v = self.fetch(); self.alu(y, v); 8 }
                _ => { let pc = self.pc; self.push(pc); self.pc = y as u16 * 8; 16 } // RST
            },
        }
    }
          }
