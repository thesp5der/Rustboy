# RustBoy

A Game Boy (DMG) emulator in Rust. The core has no dependencies; the window is an optional feature (`minifb`).

```
cargo run --release -- game.gb                                   # play
cargo run --release --no-default-features -- test.gb --headless 4000 --screenshot out.ppm
```

Controls: arrows = D-pad, Z = A, X = B, Enter = Start, Backspace / Right Shift = Select, Esc = quit.
Battery-backed cartridges save to `<rom>.sav` on exit.

| File | Role |
|---|---|
| `src/cpu.rs` | LR35902: full opcode set + CB prefix, interrupts, HALT (and halt bug), delayed EI |
| `src/ppu.rs` | Scanline renderer: BG, window, 8x8/8x16 sprites, priority, STAT/LYC interrupts |
| `src/timer.rs` | DIV/TIMA with falling-edge behaviour |
| `src/cart.rs` | ROM-only, MBC1, MBC3 (no RTC), MBC5, save RAM |
| `src/bus.rs` | Memory map, OAM DMA, serial port, IF/IE |
| `src/joypad.rs` | P1 register |

## Verified against
- Blargg `cpu_instrs` (all 11 sub-tests) and `instr_timing`: pass
- `dmg-acid2` (PPU): renders the reference image

## Not implemented
Audio (registers are stored but no sound is produced), MBC3 real-time clock, MBC2, Game Boy Color,
and sub-instruction memory timing (peripherals tick once per instruction, so `mem_timing` and
mid-instruction PPU/timer races are not exact).
