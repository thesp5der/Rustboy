//! Audio processing unit: two square channels (CH1 with frequency sweep), a programmable
//! wave channel (CH3) and a noise channel (CH4), clocked by a 512 Hz frame sequencer.
//!
//! The APU is ticked in T-cycles (4.194304 MHz). Every `cycles_per_sample` cycles it mixes the
//! four channels into one stereo sample (interleaved L, R in `samples`); the frontend drains
//! them with `take_samples()` once per frame and feeds them to the sound card or a WAV file.

const CPU_HZ: f64 = 4_194_304.0;
const DUTY: [u8; 4] = [0b0000_0001, 0b1000_0001, 0b1000_0111, 0b0111_1110];

/// Length counter: silences the channel when it counts down to zero.
struct Len {
    counter: u16,
    enabled: bool,
    max: u16,
}

impl Len {
    fn new(max: u16) -> Self {
        Self { counter: 0, enabled: false, max }
    }
    /// Returns true when the counter just expired (the channel must be disabled).
    fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            self.counter == 0
        } else {
            false
        }
    }
    /// `first_half`: the frame sequencer's next step will NOT clock length counters.
    fn trigger(&mut self, first_half: bool) {
        if self.counter == 0 {
            self.counter = self.max;
            if self.enabled && first_half {
                self.counter -= 1;
            }
        }
    }
    /// Setting the length-enable bit in the first half of a length period clocks the counter once
    /// immediately (hardware quirk). Returns true if that extra clock expired the counter.
    fn set_enabled(&mut self, enabled: bool, first_half: bool) -> bool {
        let was = self.enabled;
        self.enabled = enabled;
        if !was && enabled && first_half && self.counter > 0 {
            self.counter -= 1;
            self.counter == 0
        } else {
            false
        }
    }
}

/// Volume envelope (CH1, CH2, CH4).
struct Env {
    init: u8,
    up: bool,
    period: u8,
    timer: u8,
    vol: u8,
}

impl Env {
    fn new() -> Self {
        Self { init: 0, up: false, period: 0, timer: 0, vol: 0 }
    }
    fn write(&mut self, v: u8) {
        self.init = v >> 4;
        self.up = v & 8 != 0;
        self.period = v & 7;
    }
    fn reload(&self) -> u8 {
        if self.period == 0 { 8 } else { self.period }
    }
    fn trigger(&mut self) {
        self.vol = self.init;
        self.timer = self.reload();
    }
    fn clock(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
            if self.timer == 0 {
                self.timer = self.reload();
                if self.period != 0 {
                    if self.up && self.vol < 15 {
                        self.vol += 1;
                    } else if !self.up && self.vol > 0 {
                        self.vol -= 1;
                    }
                }
            }
        }
    }
}

struct Square {
    enabled: bool,
    dac: bool,
    duty: u8,
    duty_pos: u8,
    freq: u16,
    timer: u32,
    len: Len,
    env: Env,
    // frequency sweep (only meaningful for CH1; CH2 leaves these at zero)
    sweep_period: u8,
    sweep_neg: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_on: bool,
    neg_used: bool, // a sweep calculation ran in negate mode since the last trigger
    shadow: u16,
}

impl Square {
    fn new() -> Self {
        Self {
            enabled: false,
            dac: false,
            duty: 0,
            duty_pos: 0,
            freq: 0,
            timer: 0,
            len: Len::new(64),
            env: Env::new(),
            sweep_period: 0,
            sweep_neg: false,
            sweep_shift: 0,
            sweep_timer: 0,
            sweep_on: false,
            neg_used: false,
            shadow: 0,
        }
    }

    fn write(&mut self, reg: usize, v: u8, fh: bool) {
        match reg {
            0 => {
                if self.sweep_neg && v & 8 == 0 && self.neg_used {
                    self.enabled = false; // leaving negate mode after a negate calculation kills the channel
                }
                self.sweep_period = (v >> 4) & 7;
                self.sweep_neg = v & 8 != 0;
                self.sweep_shift = v & 7;
            }
            1 => {
                self.duty = v >> 6;
                self.len.counter = 64 - (v & 0x3F) as u16;
            }
            2 => {
                self.env.write(v);
                self.dac = v & 0xF8 != 0;
                if !self.dac {
                    self.enabled = false;
                }
            }
            3 => self.freq = (self.freq & 0x700) | v as u16,
            _ => {
                self.freq = (self.freq & 0xFF) | ((v as u16 & 7) << 8);
                if self.len.set_enabled(v & 0x40 != 0, fh) && v & 0x80 == 0 {
                    self.enabled = false;
                }
                if v & 0x80 != 0 {
                    self.trigger(fh);
                }
            }
        }
    }

    fn trigger(&mut self, fh: bool) {
        self.enabled = self.dac;
        self.len.trigger(fh);
        self.neg_used = false;
        self.timer = (2048 - self.freq as u32) * 4;
        self.env.trigger();
        self.shadow = self.freq;
        self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
        self.sweep_on = self.sweep_period != 0 || self.sweep_shift != 0;
        if self.sweep_shift != 0 && self.sweep_calc() > 2047 {
            self.enabled = false;
        }
    }

    fn sweep_calc(&mut self) -> u16 {
        self.neg_used |= self.sweep_neg;
        let delta = self.shadow >> self.sweep_shift;
        if self.sweep_neg { self.shadow.wrapping_sub(delta) } else { self.shadow + delta }
    }

    fn clock_sweep(&mut self) {
        if self.sweep_timer > 0 {
            self.sweep_timer -= 1;
            if self.sweep_timer == 0 {
                self.sweep_timer = if self.sweep_period == 0 { 8 } else { self.sweep_period };
                if self.sweep_on && self.sweep_period != 0 {
                    let new = self.sweep_calc();
                    if new > 2047 {
                        self.enabled = false;
                    } else if self.sweep_shift != 0 {
                        self.shadow = new;
                        self.freq = new;
                        if self.sweep_calc() > 2047 {
                            self.enabled = false;
                        }
                    }
                }
            }
        }
    }

    fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = (2048 - self.freq as u32) * 4;
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && (DUTY[self.duty as usize] >> (7 - self.duty_pos)) & 1 == 1 {
            self.env.vol
        } else {
            0
        }
    }
}

struct Wave {
    enabled: bool,
    dac: bool,
    len: Len,
    vol_code: u8,
    freq: u16,
    timer: u32,
    pos: u8,
    ram: [u8; 16],
}

impl Wave {
    fn new() -> Self {
        Self { enabled: false, dac: false, len: Len::new(256), vol_code: 0, freq: 0, timer: 0, pos: 0, ram: [0; 16] }
    }

    fn write(&mut self, reg: usize, v: u8, fh: bool) {
        match reg {
            0 => {
                self.dac = v & 0x80 != 0;
                if !self.dac {
                    self.enabled = false;
                }
            }
            1 => self.len.counter = 256 - v as u16,
            2 => self.vol_code = (v >> 5) & 3,
            3 => self.freq = (self.freq & 0x700) | v as u16,
            _ => {
                self.freq = (self.freq & 0xFF) | ((v as u16 & 7) << 8);
                if self.len.set_enabled(v & 0x40 != 0, fh) && v & 0x80 == 0 {
                    self.enabled = false;
                }
                if v & 0x80 != 0 {
                    self.enabled = self.dac;
                    self.len.trigger(fh);
                    self.timer = (2048 - self.freq as u32) * 2;
                    self.pos = 0;
                }
            }
        }
    }

    fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = (2048 - self.freq as u32) * 2;
            self.pos = (self.pos + 1) & 31;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled {
            return 0;
        }
        let byte = self.ram[(self.pos / 2) as usize];
        let nibble = if self.pos % 2 == 0 { byte >> 4 } else { byte & 0x0F };
        nibble >> [4, 0, 1, 2][self.vol_code as usize]
    }
}

struct Noise {
    enabled: bool,
    dac: bool,
    len: Len,
    env: Env,
    shift: u8,
    narrow: bool, // 7-bit LFSR mode
    divisor: u8,
    timer: u32,
    lfsr: u16,
}

impl Noise {
    fn new() -> Self {
        Self { enabled: false, dac: false, len: Len::new(64), env: Env::new(), shift: 0, narrow: false, divisor: 0, timer: 0, lfsr: 0x7FFF }
    }

    fn period(&self) -> u32 {
        let base = if self.divisor == 0 { 8 } else { self.divisor as u32 * 16 };
        base << self.shift
    }

    fn write(&mut self, reg: usize, v: u8, fh: bool) {
        match reg {
            1 => self.len.counter = 64 - (v & 0x3F) as u16,
            2 => {
                self.env.write(v);
                self.dac = v & 0xF8 != 0;
                if !self.dac {
                    self.enabled = false;
                }
            }
            3 => {
                self.shift = v >> 4;
                self.narrow = v & 8 != 0;
                self.divisor = v & 7;
            }
            _ => {
                if self.len.set_enabled(v & 0x40 != 0, fh) && v & 0x80 == 0 {
                    self.enabled = false;
                }
                if v & 0x80 != 0 {
                    self.enabled = self.dac;
                    self.len.trigger(fh);
                    self.env.trigger();
                    self.timer = self.period();
                    self.lfsr = 0x7FFF;
                }
            }
        }
    }

    fn tick(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            self.timer = self.period();
            let x = (self.lfsr & 1) ^ ((self.lfsr >> 1) & 1);
            self.lfsr = (self.lfsr >> 1) | (x << 14);
            if self.narrow {
                self.lfsr = (self.lfsr & !0x40) | (x << 6);
            }
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.lfsr & 1 == 0 { self.env.vol } else { 0 }
    }
}

/// Unreadable bits of NR10..NR51 read back as 1.
const READ_MASK: [u8; 22] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // (unused) NR21-NR24
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // (unused) NR41-NR44
    0x00, 0x00, // NR50, NR51
];

pub struct Apu {
    power: bool,
    regs: [u8; 0x17],
    ch1: Square,
    ch2: Square,
    ch3: Wave,
    ch4: Noise,
    frame_cycles: u32,
    frame_step: u8,
    cycles_per_sample: f64,
    sample_timer: f64,
    hpf_factor: f32,
    hpf: [f32; 2],
    samples: Vec<f32>,
}

impl Apu {
    pub fn new(sample_rate: u32) -> Self {
        let mut apu = Self {
            power: true,
            regs: [0; 0x17],
            ch1: Square::new(),
            ch2: Square::new(),
            ch3: Wave::new(),
            ch4: Noise::new(),
            frame_cycles: 0,
            frame_step: 0,
            cycles_per_sample: 0.0,
            sample_timer: 0.0,
            hpf_factor: 0.0,
            hpf: [0.0; 2],
            samples: Vec::new(),
        };
        apu.set_sample_rate(sample_rate);
        // Register values left behind by the DMG boot ROM (trigger bits left clear).
        for (addr, v) in [
            (0xFF10, 0x80), (0xFF11, 0xBF), (0xFF12, 0xF3), (0xFF14, 0x3F),
            (0xFF16, 0x3F), (0xFF17, 0x00), (0xFF19, 0x3F),
            (0xFF1A, 0x7F), (0xFF1B, 0xFF), (0xFF1C, 0x9F), (0xFF1E, 0x3F),
            (0xFF20, 0xFF), (0xFF21, 0x00), (0xFF22, 0x00), (0xFF23, 0x3F),
            (0xFF24, 0x77), (0xFF25, 0xF3),
        ] {
            apu.write(addr, v);
        }
        apu.ch1.enabled = true; // NR52 reads 0xF1 after boot (silent: envelope volume is 0)
        apu
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.cycles_per_sample = CPU_HZ / rate as f64;
        // High-pass filter modelling the DMG's output capacitor (removes the DC offset).
        self.hpf_factor = 0.999958f32.powf(self.cycles_per_sample as f32);
    }

    /// Drain the interleaved stereo samples produced since the last call.
    pub fn take_samples(&mut self) -> Vec<f32> {
        std::mem::take(&mut self.samples)
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF26 => {
                0x70 | (self.power as u8) << 7
                    | (self.ch1.enabled as u8)
                    | (self.ch2.enabled as u8) << 1
                    | (self.ch3.enabled as u8) << 2
                    | (self.ch4.enabled as u8) << 3
            }
            0xFF10..=0xFF25 => {
                let i = (addr - 0xFF10) as usize;
                self.regs[i] | READ_MASK[i]
            }
            0xFF30..=0xFF3F => self.ch3.ram[(addr - 0xFF30) as usize],
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        if (0xFF30..=0xFF3F).contains(&addr) {
            self.ch3.ram[(addr - 0xFF30) as usize] = v;
            return;
        }
        if addr == 0xFF26 {
            let on = v & 0x80 != 0;
            if self.power && !on {
                self.power_off();
            } else if !self.power && on {
                self.frame_step = 0;
            }
            self.power = on;
            return;
        }
        if !self.power {
            // While powered down only the length counters (not duty bits) accept writes on DMG.
            match addr {
                0xFF11 => self.ch1.len.counter = 64 - (v & 0x3F) as u16,
                0xFF16 => self.ch2.len.counter = 64 - (v & 0x3F) as u16,
                0xFF1B => self.ch3.len.counter = 256 - v as u16,
                0xFF20 => self.ch4.len.counter = 64 - (v & 0x3F) as u16,
                _ => {}
            }
            return;
        }
        if !(0xFF10..=0xFF25).contains(&addr) {
            return;
        }
        let i = (addr - 0xFF10) as usize;
        self.regs[i] = v;
        let fh = self.frame_step & 1 == 1;
        match i {
            0..=4 => self.ch1.write(i, v, fh),
            6..=9 => self.ch2.write(i - 5, v, fh),
            10..=14 => self.ch3.write(i - 10, v, fh),
            16..=19 => self.ch4.write(i - 15, v, fh),
            _ => {} // NR50 / NR51 are applied at mix time
        }
    }

    fn power_off(&mut self) {
        self.regs = [0; 0x17];
        let ram = self.ch3.ram; // wave RAM and length counters survive power-off on DMG
        let lens = [self.ch1.len.counter, self.ch2.len.counter, self.ch3.len.counter, self.ch4.len.counter];
        self.ch1 = Square::new();
        self.ch2 = Square::new();
        self.ch3 = Wave::new();
        self.ch3.ram = ram;
        self.ch4 = Noise::new();
        self.ch1.len.counter = lens[0];
        self.ch2.len.counter = lens[1];
        self.ch3.len.counter = lens[2];
        self.ch4.len.counter = lens[3];
    }

    fn clock_frame(&mut self) {
        match self.frame_step {
            0 | 4 => self.clock_length(),
            2 | 6 => {
                self.clock_length();
                self.ch1.clock_sweep();
            }
            7 => {
                self.ch1.env.clock();
                self.ch2.env.clock();
                self.ch4.env.clock();
            }
            _ => {}
        }
        self.frame_step = (self.frame_step + 1) & 7;
    }

    fn clock_length(&mut self) {
        if self.ch1.len.clock() { self.ch1.enabled = false }
        if self.ch2.len.clock() { self.ch2.enabled = false }
        if self.ch3.len.clock() { self.ch3.enabled = false }
        if self.ch4.len.clock() { self.ch4.enabled = false }
    }

    /// Advance by `cycles` T-cycles.
    pub fn tick(&mut self, cycles: u32) {
        for _ in 0..cycles {
            if self.power {
                self.ch1.tick();
                self.ch2.tick();
                self.ch3.tick();
                self.ch4.tick();
                self.frame_cycles += 1;
                if self.frame_cycles == 8192 {
                    self.frame_cycles = 0;
                    self.clock_frame();
                }
            }
            self.sample_timer += 1.0;
            if self.sample_timer >= self.cycles_per_sample {
                self.sample_timer -= self.cycles_per_sample;
                self.emit();
            }
        }
    }

    fn emit(&mut self) {
        let (mut l, mut r) = (0.0f32, 0.0f32);
        if self.power {
            let outs = [self.ch1.output(), self.ch2.output(), self.ch3.output(), self.ch4.output()];
            let (nr50, nr51) = (self.regs[0x14], self.regs[0x15]);
            for (i, &o) in outs.iter().enumerate() {
                let v = o as f32 / 15.0;
                if nr51 & (0x10 << i) != 0 { l += v }
                if nr51 & (1 << i) != 0 { r += v }
            }
            l *= (((nr50 >> 4) & 7) + 1) as f32 / 8.0 / 4.0;
            r *= ((nr50 & 7) + 1) as f32 / 8.0 / 4.0;
        }
        let mut out = [0.0f32; 2];
        for (i, x) in [l, r].into_iter().enumerate() {
            let y = x - self.hpf[i];
            self.hpf[i] = x - y * self.hpf_factor;
            out[i] = (y * 2.0).clamp(-1.0, 1.0);
        }
        if self.samples.len() >= 1 << 20 {
            self.samples.clear(); // nobody is draining: don't grow without bound
        }
        self.samples.extend_from_slice(&out);
    }
                                  }
