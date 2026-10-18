mod bus;
mod cart;
mod cpu;
mod joypad;
mod ppu;
mod timer;

use std::path::{Path, PathBuf};
use std::{env, fs, process};

use bus::Bus;
use cart::Cartridge;
use cpu::Cpu;

const CYCLES_PER_FRAME: u32 = 70224; // 154 lines * 456 dots

/// Run until the PPU finishes a frame (or a frame's worth of cycles, if the LCD is off).
fn run_frame(cpu: &mut Cpu) {
    let mut cycles = 0;
    cpu.bus.ppu.frame_ready = false;
    while !cpu.bus.ppu.frame_ready && cycles < CYCLES_PER_FRAME {
        cycles += cpu.step();
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: rustboy <rom.gb> [--headless FRAMES] [--screenshot out.ppm]\n\n\
         Controls: arrows = D-pad, Z = A, X = B, Enter = Start, Backspace/RShift = Select, Esc = quit\n\
         --headless runs without a window and prints everything the ROM sends over the serial port."
    );
    process::exit(2)
}

fn write_ppm(path: &str, fb: &[u32]) {
    let mut out = format!("P6\n{} {}\n255\n", ppu::W, ppu::H).into_bytes();
    for px in fb {
        out.extend_from_slice(&[(px >> 16) as u8, (px >> 8) as u8, *px as u8]);
    }
    if let Err(e) = fs::write(path, out) {
        eprintln!("could not write {path}: {e}");
    }
}

fn main() {
    let mut rom_path: Option<String> = None;
    let mut headless: Option<u32> = None;
    let mut screenshot: Option<String> = None;

    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--headless" => headless = Some(args.next().and_then(|n| n.parse().ok()).unwrap_or_else(|| usage())),
            "--screenshot" => screenshot = Some(args.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            _ if rom_path.is_none() => rom_path = Some(a),
            _ => usage(),
        }
    }
    let rom_path = rom_path.unwrap_or_else(|| usage());

    let rom = fs::read(&rom_path).unwrap_or_else(|e| {
        eprintln!("cannot read {rom_path}: {e}");
        process::exit(1)
    });
    let mut cart = Cartridge::new(rom).unwrap_or_else(|e| {
        eprintln!("{rom_path}: {e}");
        process::exit(1)
    });
    let sav: PathBuf = Path::new(&rom_path).with_extension("sav");
    cart.attach_save(&sav);
    let title = cart.title.clone();
    let mut cpu = Cpu::new(Bus::new(cart));

    if let Some(frames) = headless {
        for _ in 0..frames {
            run_frame(&mut cpu);
            // Blargg-style test ROMs announce their verdict over serial.
            let s = String::from_utf8_lossy(&cpu.bus.serial);
            if s.contains("Passed") || s.contains("Failed") {
                break;
            }
        }
        print!("{}", String::from_utf8_lossy(&cpu.bus.serial));
        if let Some(path) = screenshot {
            write_ppm(&path, &cpu.bus.ppu.fb);
        }
    } else {
        #[cfg(feature = "gui")]
        run_window(&mut cpu, &title);
        #[cfg(not(feature = "gui"))]
        {
            let _ = title;
            eprintln!("built without the `gui` feature; use --headless");
        }
    }
    cpu.bus.cart.save();
}

#[cfg(feature = "gui")]
fn run_window(cpu: &mut Cpu, title: &str) {
    use joypad::Button;
    use minifb::{Key, Scale, Window, WindowOptions};

    let opts = WindowOptions { scale: Scale::X4, resize: true, ..WindowOptions::default() };
    let mut win = Window::new(&format!("RustBoy - {title}"), ppu::W, ppu::H, opts).unwrap_or_else(|e| {
        eprintln!("cannot open window: {e}");
        process::exit(1)
    });
    win.limit_update_rate(Some(std::time::Duration::from_micros(16_742))); // 59.7 Hz, the real DMG refresh rate

    let keys: [(&[Key], Button); 8] = [
        (&[Key::Right], Button::Right),
        (&[Key::Left], Button::Left),
        (&[Key::Up], Button::Up),
        (&[Key::Down], Button::Down),
        (&[Key::Z], Button::A),
        (&[Key::X], Button::B),
        (&[Key::Enter], Button::Start),
        (&[Key::Backspace, Key::RightShift], Button::Select),
    ];

    while win.is_open() && !win.is_key_down(Key::Escape) {
        for (ks, b) in keys {
            cpu.bus.press(b, ks.iter().any(|&k| win.is_key_down(k)));
        }
        run_frame(cpu);
        if let Err(e) = win.update_with_buffer(&cpu.bus.ppu.fb, ppu::W, ppu::H) {
            eprintln!("display error: {e}");
            break;
        }
    }
}
