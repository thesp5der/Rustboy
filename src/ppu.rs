//! Picture processing unit. Scanline-based: each line is rendered when it leaves
//! mode 3 (drawing), using the register values at that moment.
//!
//! Line timing (dots): mode 2 = 0..80, mode 3 = 80..252, mode 0 = 252..456; lines 144-153 are VBlank.

pub const W: usize = 160;
pub const H: usize = 144;

/// Classic DMG green shades (0x00RRGGBB), lightest to darkest.
const SHADES: [u32; 4] = [0xE0F8D0, 0x88C070, 0x346856, 0x081820];

pub struct Ppu {
    pub vram: [u8; 0x2000],
    pub oam: [u8; 0xA0],
    pub fb: Vec<u32>,
    pub frame_ready: bool,
    /// Interrupt request bits (1 = VBlank, 2 = STAT) for the bus to collect.
    pub irq: u8,

    lcdc: u8,
    stat: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    wy: u8,
    wx: u8,

    mode: u8,
    dot: u32,
    win_line: u8,
    stat_line: bool,
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            vram: [0; 0x2000],
            oam: [0; 0xA0],
            fb: vec![SHADES[0]; W * H],
            frame_ready: false,
            irq: 0,
            lcdc: 0x91, // post-boot-ROM state: LCD on, BG on
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            mode: 2,
            dot: 0,
            win_line: 0,
            stat_line: false,
        }
    }

    fn lcd_on(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    /// Recompute the STAT interrupt line; request an interrupt on a rising edge.
    pub fn update_stat(&mut self) {
        let line = self.lcd_on()
            && ((self.ly == self.lyc && self.stat & 0x40 != 0)
                || (self.mode == 0 && self.stat & 0x08 != 0)
                || (self.mode == 1 && self.stat & 0x10 != 0)
                || (self.mode == 2 && self.stat & 0x20 != 0));
        if line && !self.stat_line {
            self.irq |= 2;
        }
        self.stat_line = line;
    }

    pub fn tick(&mut self, cycles: u32) {
        if !self.lcd_on() {
            return;
        }
        self.dot += cycles;
        loop {
            match self.mode {
                2 if self.dot >= 80 => self.mode = 3,
                3 if self.dot >= 252 => {
                    self.mode = 0;
                    self.render_line();
                }
                0 if self.dot >= 456 => {
                    self.dot -= 456;
                    self.ly += 1;
                    if self.ly == 144 {
                        self.mode = 1;
                        self.irq |= 1;
                        self.frame_ready = true;
                    } else {
                        self.mode = 2;
                    }
                }
                1 if self.dot >= 456 => {
                    self.dot -= 456;
                    self.ly += 1;
                    if self.ly > 153 {
                        self.ly = 0;
                        self.win_line = 0;
                        self.mode = 2;
                    }
                }
                _ => break,
            }
            self.update_stat();
        }
        self.update_stat();
    }

    pub fn read_reg(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => {
                let on = self.lcd_on();
                0x80 | (self.stat & 0x78)
                    | if on && self.ly == self.lyc { 4 } else { 0 }
                    | if on { self.mode } else { 0 }
            }
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            _ => 0xFF,
        }
    }

    pub fn write_reg(&mut self, addr: u16, v: u8) {
        match addr {
            0xFF40 => {
                let was_on = self.lcd_on();
                self.lcdc = v;
                if was_on && !self.lcd_on() {
                    self.ly = 0;
                    self.dot = 0;
                    self.mode = 0;
                    self.win_line = 0;
                    self.stat_line = false;
                } else if !was_on && self.lcd_on() {
                    self.mode = 2;
                    self.dot = 0;
                }
            }
            0xFF41 => self.stat = v & 0x78,
            0xFF42 => self.scy = v,
            0xFF43 => self.scx = v,
            0xFF45 => self.lyc = v,
            0xFF47 => self.bgp = v,
            0xFF48 => self.obp0 = v,
            0xFF49 => self.obp1 = v,
            0xFF4A => self.wy = v,
            0xFF4B => self.wx = v,
            _ => {}
        }
        self.update_stat();
    }

    /// Colour index (0-3) of pixel (px, y) in a 256x256 tile map.
    fn tile_px(&self, map: usize, px: u8, y: u8) -> u8 {
        let id = self.vram[map + (y as usize / 8) * 32 + px as usize / 8];
        let base = if self.lcdc & 0x10 != 0 {
            id as usize * 16
        } else {
            (0x1000 + (id as i8 as i32) * 16) as usize // signed addressing around 0x9000
        };
        let a = base + (y as usize % 8) * 2;
        let bit = 7 - (px % 8);
        (((self.vram[a + 1] >> bit) & 1) << 1) | ((self.vram[a] >> bit) & 1)
    }

    fn render_line(&mut self) {
        let ly = self.ly as usize;
        if ly >= H {
            return;
        }
        let mut bg = [0u8; W]; // BG/window colour indices, needed for sprite priority
        if self.lcdc & 1 != 0 {
            let map = if self.lcdc & 0x08 != 0 { 0x1C00 } else { 0x1800 };
            let y = self.ly.wrapping_add(self.scy);
            for (x, px) in bg.iter_mut().enumerate() {
                *px = self.tile_px(map, (x as u8).wrapping_add(self.scx), y);
            }
            if self.lcdc & 0x20 != 0 && self.ly >= self.wy && self.wx <= 166 {
                let map = if self.lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 };
                let left = self.wx as i32 - 7;
                for x in left.max(0) as usize..W {
                    bg[x] = self.tile_px(map, (x as i32 - left) as u8, self.win_line);
                }
                self.win_line += 1;
            }
        }

        let mut line = [0u32; W];
        for x in 0..W {
            line[x] = SHADES[((self.bgp >> (bg[x] * 2)) & 3) as usize];
        }

        if self.lcdc & 2 != 0 {
            let h: i32 = if self.lcdc & 4 != 0 { 16 } else { 8 };
            // The hardware draws at most 10 sprites per line (first 10 in OAM order);
            // on DMG, lower X wins, ties go to the lower OAM index.
            let mut found = [(0u8, 0usize); 10];
            let mut n = 0;
            for i in 0..40 {
                let y = self.oam[i * 4] as i32 - 16;
                if (ly as i32) >= y && (ly as i32) < y + h && n < 10 {
                    found[n] = (self.oam[i * 4 + 1], i);
                    n += 1;
                }
            }
            found[..n].sort();
            let mut taken = [false; W];
            for &(sx, i) in &found[..n] {
                let o = &self.oam[i * 4..i * 4 + 4];
                let attr = o[3];
                let mut row = ly as i32 - (o[0] as i32 - 16);
                if attr & 0x40 != 0 {
                    row = h - 1 - row;
                }
                let tile = if h == 16 { o[2] & 0xFE } else { o[2] } as usize;
                let a = tile * 16 + row as usize * 2; // 8x16 tiles are contiguous in VRAM
                let (lo, hi) = (self.vram[a], self.vram[a + 1]);
                let pal = if attr & 0x10 != 0 { self.obp1 } else { self.obp0 };
                for col in 0..8u32 {
                    let x = sx as i32 - 8 + col as i32;
                    if x < 0 || x >= W as i32 || taken[x as usize] {
                        continue;
                    }
                    let bit = if attr & 0x20 != 0 { col } else { 7 - col };
                    let id = (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1);
                    if id == 0 {
                        continue; // transparent
                    }
                    taken[x as usize] = true;
                    if attr & 0x80 != 0 && bg[x as usize] != 0 {
                        continue; // behind non-zero BG
                    }
                    line[x as usize] = SHADES[((pal >> (id * 2)) & 3) as usize];
                }
            }
        }

        self.fb[ly * W..(ly + 1) * W].copy_from_slice(&line);
    }
}
