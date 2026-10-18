//! Joypad register (P1/JOYP at 0xFF00). Bits in `dpad`/`buttons` are 1 when pressed;
//! the register itself is active-low.

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

#[derive(Default)]
pub struct Joypad {
    select: u8,
    dpad: u8,
    buttons: u8,
}

impl Joypad {
    pub fn read(&self) -> u8 {
        let mut low = 0x0F;
        if self.select & 0x10 == 0 {
            low &= !self.dpad;
        }
        if self.select & 0x20 == 0 {
            low &= !self.buttons;
        }
        0xC0 | self.select | low
    }

    pub fn write(&mut self, v: u8) {
        self.select = v & 0x30;
    }

    /// Update a button. Returns true on a fresh press (which requests the joypad interrupt).
    pub fn set(&mut self, b: Button, pressed: bool) -> bool {
        let (field, bit) = match b {
            Button::Right => (&mut self.dpad, 0),
            Button::Left => (&mut self.dpad, 1),
            Button::Up => (&mut self.dpad, 2),
            Button::Down => (&mut self.dpad, 3),
            Button::A => (&mut self.buttons, 0),
            Button::B => (&mut self.buttons, 1),
            Button::Select => (&mut self.buttons, 2),
            Button::Start => (&mut self.buttons, 3),
        };
        let was = *field & (1 << bit) != 0;
        if pressed {
            *field |= 1 << bit;
        } else {
            *field &= !(1 << bit);
        }
        pressed && !was
    }
}
