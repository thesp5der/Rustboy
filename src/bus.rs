//! The system bus: address decoding, interrupt flags, OAM DMA, serial port.

use crate::cart::Cartridge;
use crate::joypad::{Button, Joypad};
use crate::ppu::Ppu;
use crate::timer::Timer;

pub struct Bus {
    pub cart: Cartridge,
    pub ppu: Ppu,
    pub timer: Timer,
    pub joy: Joypad,
    wram: [u8; 0x2000],
    hram: [u8; 0x7F],
    /// Raw storage for the sound registers (0xFF10-0xFF3F). Audio is not emulated.
    apu: [u8; 0x30],
    pub ie: u8,
    pub iflag: u8,
    dma: u8,
    sb: u8,
    sc: u8,
    /// Every byte the game sent over the serial port (test ROMs print results here).
    pub serial: Vec<u8>,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        let mut apu = [0u8; 0x30];
        apu[0x16] = 0xF1; // NR52: sound on
        Self {
            cart,
            ppu: Ppu::new(),
            timer: Timer::new(),
            joy: Joypad::default(),
            wram: [0; 0x2000],
            hram: [0; 0x7F],
            apu,
            ie: 0,
            iflag: 0xE1,
            dma: 0xFF,
            sb: 0,
            sc: 0x7E,
            serial: Vec::new(),
        }
    }

    /// Advance all peripherals by `cycles` T-cycles.
    pub fn tick(&mut self, cycles: u32) {
        if self.timer.tick(cycles) {
            self.iflag |= 0x04;
        }
        self.ppu.tick(cycles);
        self.collect_ppu_irq();
    }

    fn collect_ppu_irq(&mut self) {
        self.iflag |= self.ppu.irq;
        self.ppu.irq = 0;
    }

    pub fn press(&mut self, b: Button, pressed: bool) {
        if self.joy.set(b, pressed) {
            self.iflag |= 0x10;
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cart.read(addr),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.cart.read_ram(addr),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize], // echo RAM
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize],
            0xFEA0..=0xFEFF => 0xFF, // unusable
            0xFF00 => self.joy.read(),
            0xFF01 => self.sb,
            0xFF02 => self.sc | 0x7E,
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.iflag | 0xE0,
            0xFF10..=0xFF3F => self.apu[(addr - 0xFF10) as usize],
            0xFF46 => self.dma,
            0xFF40..=0xFF4B => self.ppu.read_reg(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        match addr {
            0x0000..=0x7FFF => self.cart.write(addr, v),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize] = v,
            0xA000..=0xBFFF => self.cart.write_ram(addr, v),
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = v,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize] = v,
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = v,
            0xFF00 => self.joy.write(v),
            0xFF01 => self.sb = v,
            0xFF02 => {
                self.sc = v;
                if v & 0x81 == 0x81 {
                    // Internal-clock transfer: nobody is on the other end, so complete instantly.
                    self.serial.push(self.sb);
                    self.sc &= !0x80;
                }
            }
            0xFF04..=0xFF07 => {
                if self.timer.write(addr, v) {
                    self.iflag |= 0x04;
                }
            }
            0xFF0F => self.iflag = v & 0x1F,
            0xFF10..=0xFF3F => self.apu[(addr - 0xFF10) as usize] = v,
            0xFF46 => {
                // OAM DMA, performed instantly rather than over 160 cycles.
                self.dma = v;
                let base = (v as u16) << 8;
                for i in 0..0xA0u16 {
                    self.ppu.oam[i as usize] = self.read(base + i);
                }
            }
            0xFF40..=0xFF4B => {
                self.ppu.write_reg(addr, v);
                self.collect_ppu_irq();
            }
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = v,
            0xFFFF => self.ie = v,
            _ => {}
        }
    }
}
