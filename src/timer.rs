//! DIV/TIMA/TMA/TAC. TIMA increments on the *falling edge* of a bit of the 16-bit
//! internal divider, which reproduces the real hardware quirks (DIV reset and TAC
//! writes can both tick TIMA).

pub struct Timer {
    div: u16,
    tima: u8,
    tma: u8,
    tac: u8,
}

impl Timer {
    pub fn new() -> Self {
        Self { div: 0xABCC, tima: 0, tma: 0, tac: 0 } // post-boot-ROM values
    }

    fn signal(&self) -> bool {
        let bit = [9, 3, 5, 7][(self.tac & 3) as usize];
        self.tac & 4 != 0 && (self.div >> bit) & 1 == 1
    }

    /// Increment TIMA; returns true when it overflowed (timer interrupt).
    fn inc(&mut self) -> bool {
        let (v, overflow) = self.tima.overflowing_add(1);
        self.tima = if overflow { self.tma } else { v };
        overflow
    }

    /// Advance by `cycles` T-cycles; returns true if a timer interrupt fired.
    pub fn tick(&mut self, cycles: u32) -> bool {
        let mut irq = false;
        for _ in 0..cycles {
            let before = self.signal();
            self.div = self.div.wrapping_add(1);
            if before && !self.signal() {
                irq |= self.inc();
            }
        }
        irq
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            _ => self.tac | 0xF8,
        }
    }

    /// Returns true if the write caused a timer interrupt.
    pub fn write(&mut self, addr: u16, v: u8) -> bool {
        match addr {
            0xFF04 => {
                let before = self.signal();
                self.div = 0;
                before && self.inc()
            }
            0xFF05 => {
                self.tima = v;
                false
            }
            0xFF06 => {
                self.tma = v;
                false
            }
            _ => {
                let before = self.signal();
                self.tac = v & 7;
                before && !self.signal() && self.inc()
            }
        }
    }
}
