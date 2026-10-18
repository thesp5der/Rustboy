mod apu;
#[cfg(feature = "audio")]
mod audio;
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
use joypad::Button;

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
        "usage: rustboy <rom.gb> [--headless FRAMES] [--screenshot out.ppm] [--wav out.wav] [--input SCRIPT]\n\n\
         Controls: arrows = D-pad, Z = A, X = B, Enter = Start, Backspace/RShift = Select, Esc = quit\n\
         --headless runs without a window and prints everything the ROM sends over the serial port.\n\
         --input scripts button presses, e.g. \"start@60+2,down@120+30\" (button@first_frame+frames).\n\
         --wav records the audio of a headless run."
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

fn write_wav(path: &str, samples: &[f32], rate: u32) {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    if let Err(e) = fs::write(path, out) {
        eprintln!("could not write {path}: {e}");
    }
}

/// Parse "start@60+2,down@120+30" into (button, first frame, frame count).
fn parse_input(script: &str) -> Vec<(Button, u32, u32)> {
    let bad = || -> ! {
        eprintln!("bad --input script (expected button@frame+frames, e.g. start@60+2)");
        process::exit(2)
    };
    script
        .split(',')
        .map(|item| {
            let (name, rest) = item.split_once('@').unwrap_or_else(|| bad());
            let (start, len) = rest.split_once('+').unwrap_or_else(|| bad());
            let button = match name {
                "right" => Button::Right,
                "left" => Button::Left,
                "up" => Button::Up,
                "down" => Button::Down,
                "a" => Button::A,
                "b" => Button::B,
                "select" => Button::Select,
                "start" => Button::Start,
                _ => bad(),
            };
            (button, start.parse().unwrap_or_else(|_| bad()), len.parse().unwrap_or_else(|_| bad()))
        })
        .collect()
}

fn main() {
    let mut rom_path: Option<String> = None;
    let mut headless: Option<u32> = None;
    let mut screenshot: Option<String> = None;
    let mut wav: Option<String> = None;
    let mut input = Vec::new();

    let mut args = env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--headless" => headless = Some(args.next().and_then(|n| n.parse().ok()).unwrap_or_else(|| usage())),
            "--screenshot" => screenshot = Some(args.next().unwrap_or_else(|| usage())),
            "--wav" => wav = Some(args.next().unwrap_or_else(|| usage())),
            "--input" => input = parse_input(&args.next().unwrap_or_else(|| usage())),
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
        let mut audio: Vec<f32> = Vec::new();
        for frame in 0..frames {
            for &(button, start, len) in &input {
                cpu.bus.press(button, frame >= start && frame < start + len);
            }
            run_frame(&mut cpu);
            let samples = cpu.bus.apu.take_samples();
            if wav.is_some() {
                audio.extend(samples);
            }
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
        if let Some(path) = wav {
            write_wav(&path, &audio, 44_100);
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
    use minifb::{Key, Scale, Window, WindowOptions};

    let opts = WindowOptions { scale: Scale::X4, resize: true, ..WindowOptions::default() };
    let mut win = Window::new(&format!("RustBoy - {title}"), ppu::W, ppu::H, opts).unwrap_or_else(|e| {
        eprintln!("cannot open window: {e}");
        process::exit(1)
    });
    #[cfg(feature = "audio")]
    let audio = audio::Audio::new();
    #[cfg(feature = "audio")]
    match &audio {
        // Audio present: the sound card's clock paces emulation (see below).
        Some(a) => cpu.bus.apu.set_sample_rate(a.rate),
        None => eprintln!("no audio device available; running silent"),
    }
    #[cfg(feature = "audio")]
    let paced_by_audio = audio.is_some();
    #[cfg(not(feature = "audio"))]
    let paced_by_audio = false;
    if !paced_by_audio {
        win.limit_update_rate(Some(std::time::Duration::from_micros(16_742))); // 59.7 Hz, the real DMG refresh rate
    }

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
        let samples = cpu.bus.apu.take_samples();
        #[cfg(feature = "audio")]
        if let Some(a) = &audio {
            a.push(&samples);
            // Keep ~50 ms queued: wait while the sound card catches up (bounded, in case it stalls).
            for _ in 0..50 {
                if a.queued_secs() <= 0.05 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        let _ = samples;
        if let Err(e) = win.update_with_buffer(&cpu.bus.ppu.fb, ppu::W, ppu::H) {
            eprintln!("display error: {e}");
            break;
        }
    }
}
