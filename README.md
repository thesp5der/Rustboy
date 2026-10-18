# RustBoy

A Game Boy (DMG) emulator in Rust, with sound. The core has no dependencies; the window (`minifb`)
and sound output (`cpal`) are optional features.

```
cargo run --release -- game/pong.gb          # play the included homebrew game
cargo run --release -- yourgame.gb
cargo run --release --no-default-features -- test.gb --headless 4000 --screenshot out.ppm
```

Linux needs ALSA headers for sound: `sudo apt install libasound2-dev`
(or build without audio: `cargo run --release --no-default-features --features gui -- game.gb`).

Controls: arrows = D-pad, Z = A, X = B, Enter = Start, Backspace / Right Shift = Select, Esc = quit.
Battery-backed cartridges save to `<rom>.sav` on exit.

| File | Role |
|---|---|
| `src/cpu.rs` | LR35902: full opcode set + CB prefix, interrupts, HALT (and halt bug), delayed EI |
| `src/ppu.rs` | Scanline renderer: BG, window, 8x8/8x16 sprites, priority, STAT/LYC interrupts |
| `src/apu.rs` | 4 channels (2 square + sweep, wave, noise), frame sequencer, stereo mixer, DC filter |
| `src/audio.rs` | Sound-card output; the audio clock paces emulation |
| `src/timer.rs` | DIV/TIMA with falling-edge behaviour |
| `src/cart.rs` | ROM-only, MBC1, MBC3 (no RTC), MBC5, save RAM |
| `src/bus.rs` | Memory map, OAM DMA, serial port, IF/IE |
| `game/pong.asm` | A complete homebrew Pong used as a test game (see below) |

## The test game: Pong

`game/pong.gb` is a full game written in Game Boy assembly (RGBDS): title screen, CPU opponent,
first to 7 wins, win/lose screens, music and sound effects. Up/Down moves your paddle, Start begins.
It uses VBlank interrupts + HALT, joypad reads, OAM DMA from HRAM, sprites, background tiles, LCD
on/off, and all four sound channels (CH1 sweep = hit/win/lose, CH2 = wall, CH3 = melody, CH4 = score).

Rebuild with `game/build.sh` (needs [RGBDS](https://rgbds.gbdev.io)). `game/build.sh autoplay`
builds a self-playing variant for testing:

```
cargo run --release --no-default-features -- game/pong-autoplay.gb --headless 2400 \
    --input "start@100+3" --screenshot out.ppm --wav out.wav
```

`--input "button@first_frame+frame_count,..."` scripts the joypad; `--wav` records the audio.

## Verified against
- Blargg `cpu_instrs` (all 11) and `instr_timing`: pass
- `dmg-acid2` (PPU): renders the reference image
- Blargg `dmg_sound`: 9 of 12 pass (registers, length counter, trigger, sweep, sweep details,
  overflow on trigger, length/sweep sync, length during power, regs after power). The 3 failures
  are wave-RAM-access-while-playing tests, which need per-memory-access timing.
- Pong: title, gameplay, lose and win screens render correctly; the title melody was measured
  from a recorded WAV and matches the intended notes to within 0.2 Hz.

## Not implemented
MBC3 real-time clock, MBC2, Game Boy Color, wave-RAM access quirks while CH3 plays, and
sub-instruction memory timing (the PPU, timer and APU advance once per instruction).
