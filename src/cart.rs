//! Cartridge: ROM + external RAM + memory bank controller (None, MBC1, MBC3, MBC5).

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum Mbc {
    None,
    Mbc1,
    Mbc3, // no real-time clock: RTC registers read as 0xFF
    Mbc5,
}

pub struct Cartridge {
    rom: Vec<u8>,
    ram: Vec<u8>,
    mbc: Mbc,
    banks: usize,
    battery: bool,
    ram_enabled: bool,
    rom_bank: usize, // MBC1: 5-bit BANK1 register; MBC3: 7 bits; MBC5: 9 bits
    ram_bank: usize, // MBC1: 2-bit BANK2 register (also upper ROM bits); MBC3/5: RAM bank
    mode1: bool,     // MBC1 banking mode
    pub title: String,
    sav_path: Option<PathBuf>,
}

impl Cartridge {
    pub fn new(mut rom: Vec<u8>) -> Result<Self, String> {
        if rom.len() < 0x150 {
            return Err("ROM is too small to contain a Game Boy header".into());
        }
        let checksum = rom[0x134..=0x14C].iter().fold(0u8, |x, &b| x.wrapping_sub(b).wrapping_sub(1));
        if checksum != rom[0x14D] {
            eprintln!("warning: header checksum mismatch (bad dump?)");
        }

        let kind = rom[0x147];
        let (mbc, battery) = match kind {
            0x00 | 0x08 => (Mbc::None, false),
            0x09 => (Mbc::None, true),
            0x01 | 0x02 => (Mbc::Mbc1, false),
            0x03 => (Mbc::Mbc1, true),
            0x0F | 0x10 | 0x13 => (Mbc::Mbc3, true),
            0x11 | 0x12 => (Mbc::Mbc3, false),
            0x19 | 0x1A | 0x1C | 0x1D => (Mbc::Mbc5, false),
            0x1B | 0x1E => (Mbc::Mbc5, true),
            k => return Err(format!("unsupported cartridge type 0x{k:02X}")),
        };

        let ram_size = match rom[0x149] {
            0x01 => 0x800,
            0x02 => 0x2000,
            0x03 => 0x8000,
            0x04 => 0x20000,
            0x05 => 0x10000,
            _ => 0,
        };

        // Pad to a whole number of 16 KiB banks so bank indexing can never go out of range.
        let padded = (rom.len() + 0x3FFF) & !0x3FFF;
        rom.resize(padded, 0xFF);

        let title = rom[0x134..0x144]
            .iter()
            .take_while(|&&b| b != 0)
            .filter(|b| b.is_ascii_graphic() || **b == b' ')
            .map(|&b| b as char)
            .collect();

        Ok(Self {
            banks: rom.len() / 0x4000,
            rom,
            ram: vec![0; ram_size],
            mbc,
            battery,
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            mode1: false,
            title,
            sav_path: None,
        })
    }

    /// Attach a save file; loads it if present. Only used for battery-backed carts.
    pub fn attach_save(&mut self, path: &Path) {
        if !self.battery || self.ram.is_empty() {
            return;
        }
        if let Ok(data) = fs::read(path) {
            let n = data.len().min(self.ram.len());
            self.ram[..n].copy_from_slice(&data[..n]);
        }
        self.sav_path = Some(path.to_path_buf());
    }

    pub fn save(&self) {
        if let Some(p) = &self.sav_path {
            if let Err(e) = fs::write(p, &self.ram) {
                eprintln!("could not write save file {}: {e}", p.display());
            }
        }
    }

    #[inline]
    fn rom_at(&self, bank: usize, off: usize) -> u8 {
        self.rom[(bank % self.banks) * 0x4000 + off]
    }

    pub fn read(&self, addr: u16) -> u8 {
        let a = addr as usize;
        if a < 0x4000 {
            let bank = if self.mbc == Mbc::Mbc1 && self.mode1 { self.ram_bank << 5 } else { 0 };
            self.rom_at(bank, a)
        } else {
            let bank = match self.mbc {
                Mbc::None => 1,
                Mbc::Mbc1 => (self.ram_bank << 5) | self.rom_bank,
                _ => self.rom_bank,
            };
            self.rom_at(bank, a - 0x4000)
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        match self.mbc {
            Mbc::None => {}
            Mbc::Mbc1 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = v & 0x0F == 0x0A,
                0x2000..=0x3FFF => self.rom_bank = ((v & 0x1F) as usize).max(1),
                0x4000..=0x5FFF => self.ram_bank = (v & 3) as usize,
                0x6000..=0x7FFF => self.mode1 = v & 1 == 1,
                _ => {}
            },
            Mbc::Mbc3 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = v & 0x0F == 0x0A,
                0x2000..=0x3FFF => self.rom_bank = ((v & 0x7F) as usize).max(1),
                0x4000..=0x5FFF => self.ram_bank = v as usize,
                _ => {}
            },
            Mbc::Mbc5 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = v & 0x0F == 0x0A,
                0x2000..=0x2FFF => self.rom_bank = (self.rom_bank & 0x100) | v as usize,
                0x3000..=0x3FFF => self.rom_bank = (self.rom_bank & 0xFF) | ((v as usize & 1) << 8),
                0x4000..=0x5FFF => self.ram_bank = (v & 0x0F) as usize,
                _ => {}
            },
        }
    }

    fn ram_index(&self, addr: u16) -> Option<usize> {
        if !self.ram_enabled || self.ram.is_empty() {
            return None;
        }
        let bank = match self.mbc {
            Mbc::None => 0,
            Mbc::Mbc1 => if self.mode1 { self.ram_bank } else { 0 },
            Mbc::Mbc3 if self.ram_bank <= 3 => self.ram_bank,
            Mbc::Mbc3 => return None, // RTC register select: unsupported
            Mbc::Mbc5 => self.ram_bank,
        };
        Some((bank * 0x2000 + (addr as usize - 0xA000)) % self.ram.len())
    }

    pub fn read_ram(&self, addr: u16) -> u8 {
        self.ram_index(addr).map_or(0xFF, |i| self.ram[i])
    }

    pub fn write_ram(&mut self, addr: u16, v: u8) {
        if let Some(i) = self.ram_index(addr) {
            self.ram[i] = v;
        }
    }
}
