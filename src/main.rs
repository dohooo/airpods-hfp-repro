//! Test program for a Handy bug report: on macOS, the AirPods mic goes silent a
//! few seconds into a recording.
//!
//!   cargo run -r -- control   mic only
//!   cargo run -r -- late      mic, then a 48 kHz output once the headset is in call mode
//!   cargo run -r -- hold      like `late`, with a silent output stream already running
//!   cargo run -r -- handy     the three streams Handy 0.10.0 opens when a recording starts
//!   cargo run -r -- early     mic, then a 48 kHz output while the call link is still coming up
//!   cargo run -r -- race      like `early`, with a silent output started together with the mic
//!   cargo run -r -- drop      silent output started together with the mic, stopped 0.5 s later
//!   cargo run -r -- drop48    like `drop`, plus the 48 kHz output of `early`
//!
//! Make the AirPods the default input AND output first, and keep talking while
//! it runs so the level column means something.

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const HEALTHY_BEFORE_OUTPUT: Duration = Duration::from_secs(3);

static STARTED_AT: OnceLock<Instant> = OnceLock::new();

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Control,
    Late,
    Hold,
    Handy,
    Early,
    Race,
    Drop,
    Drop48,
}

fn main() {
    let mode = match std::env::args().nth(1).as_deref() {
        Some("control") => Mode::Control,
        Some("late") | None => Mode::Late,
        Some("hold") => Mode::Hold,
        Some("handy") => Mode::Handy,
        Some("early") => Mode::Early,
        Some("race") => Mode::Race,
        Some("drop") => Mode::Drop,
        Some("drop48") => Mode::Drop48,
        Some(other) => {
            eprintln!("unknown mode `{other}`: use control, late, hold, handy, early, race, drop or drop48");
            return;
        }
    };
    let mode_name = match mode {
        Mode::Control => "control (mic only)",
        Mode::Late => "late (48 kHz output once the headset is in call mode)",
        Mode::Hold => "hold (silent output running, then a 48 kHz output)",
        Mode::Handy => "handy (mic, silent output and start sound, like Handy 0.10.0)",
        Mode::Early => "early (48 kHz output while the call link is still coming up)",
        Mode::Race => "race (silent output started with the mic, then a 48 kHz output while the call link is still coming up)",
        Mode::Drop => "drop (silent output started with the mic and stopped 0.5 s later)",
        Mode::Drop48 => "drop48 (silent output started with the mic and stopped 0.5 s later, plus a 48 kHz output while the call link is still coming up)",
    };
    let run_for = Duration::from_secs(if matches!(mode, Mode::Control | Mode::Late | Mode::Hold) { 16 } else { 12 });

    let host = cpal::default_host();
    let input = host.default_input_device().expect("no default input device");
    let output = host.default_output_device().expect("no default output device");
    let in_config = input.default_input_config().expect("no input config");
    let in_rate = in_config.sample_rate();
    println!("input : {input}  ({in_rate} Hz)");
    println!("output: {output}  ({} Hz right now)", output_rate(&output));
    println!("mode  : {mode_name}\n");

    // Peak level of the mic since the last print, and how many samples arrived.
    let peak = Arc::new(AtomicU32::new(0));
    let samples = Arc::new(AtomicU64::new(0));
    let heard = Arc::new(AtomicBool::new(false));
    let (peak_cb, samples_cb, heard_cb) = (peak.clone(), samples.clone(), heard.clone());
    let started_at = *STARTED_AT.get_or_init(Instant::now);
    // `race`: have a silent output ready before the mic, so it can be started
    // within a millisecond of it.
    let prebuilt = if matches!(mode, Mode::Race | Mode::Drop | Mode::Drop48) {
        build_output(&output, None, "silent stream")
    } else {
        None
    };
    let mic = input
        .build_input_stream(
            in_config.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let block_peak = data.iter().fold(0f32, |m, s| m.max(s.abs()));
                peak_cb.fetch_max(block_peak.to_bits(), Ordering::Relaxed);
                samples_cb.fetch_add(data.len() as u64, Ordering::Relaxed);
                heard_cb.store(true, Ordering::Relaxed);
            },
            |e| eprintln!("mic stream error: {e}"),
            None,
        )
        .expect("could not open the mic");
    mic.play().expect("could not start the mic");
    if let Some(stream) = &prebuilt {
        let _ = stream.play();
    }
    note(if prebuilt.is_some() { "mic stream and silent stream started" } else { "mic stream started" });

    if matches!(mode, Mode::Early | Mode::Race | Mode::Drop48) {
        // Ask for 48 kHz the moment the headset's output reports the mic's rate,
        // which is while the call link is still being set up.
        let out = output.clone();
        thread::spawn(move || {
            let give_up = Instant::now() + Duration::from_secs(5);
            while output_rate(&out) != in_rate && Instant::now() < give_up {
                thread::sleep(Duration::from_millis(2));
            }
            open_output(&out, Some(48_000), "48 kHz output", run_for);
        });
    }

    if mode == Mode::Handy {
        // AirPods Mode: a silent output stream on the headset, opened right
        // after the mic.
        let out = output.clone();
        thread::spawn(move || open_output(&out, None, "silent stream", run_for));
        // Start sound: an output stream opened as soon as the mic delivers its
        // first samples, closed again when the sound is over.
        let out = output.clone();
        let heard = heard.clone();
        thread::spawn(move || {
            while !heard.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(1));
            }
            open_output(&out, None, "start sound", Duration::from_millis(500));
        });
    }

    let mut silent_stopped = false;
    let mut hold_opened = false;
    let mut output_opened = false;
    let mut in_call_mode_since: Option<Instant> = None;
    while started_at.elapsed() < run_for {
        thread::sleep(Duration::from_millis(500));
        let level = f32::from_bits(peak.swap(0, Ordering::Relaxed));
        let got = samples.swap(0, Ordering::Relaxed);
        let out_rate = output_rate(&output);
        println!(
            "{:5.1}s  mic peak {:.4}  samples {:6}  output device at {} Hz",
            started_at.elapsed().as_secs_f32(),
            level,
            got,
            out_rate
        );

        // `drop`: stop the silent stream half a second after it started.
        if matches!(mode, Mode::Drop | Mode::Drop48) && !silent_stopped {
            silent_stopped = true;
            if let Some(stream) = &prebuilt {
                let _ = stream.pause();
                note("silent stream stopped");
            }
        }

        // The headset is in call mode once its output runs at the mic's rate.
        if out_rate == in_rate {
            in_call_mode_since.get_or_insert_with(Instant::now);
        }
        if mode == Mode::Hold && !hold_opened && in_call_mode_since.is_some() {
            hold_opened = true;
            let out = output.clone();
            thread::spawn(move || open_output(&out, None, "silent stream", run_for));
        }
        let ready = in_call_mode_since.is_some_and(|t| t.elapsed() >= HEALTHY_BEFORE_OUTPUT);
        if matches!(mode, Mode::Late | Mode::Hold) && !output_opened && ready {
            output_opened = true;
            let out = output.clone();
            thread::spawn(move || open_output(&out, Some(48_000), "48 kHz output", run_for));
        }
    }
    drop(mic);
    drop(prebuilt);

    println!("\nWhat macOS logged meanwhile:");
    print_system_log(started_at.elapsed() + Duration::from_secs(2));
}

fn note(message: impl AsRef<str>) {
    let at = STARTED_AT.get().map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
    println!("  --> {at:6.3}s  {}", message.as_ref());
}

fn output_rate(output: &cpal::Device) -> u32 {
    output.default_output_config().map(|c| c.sample_rate()).unwrap_or(0)
}

/// Builds an output stream of silence on the headset without starting it.
/// `rate: None` uses whatever the device reports right now, the way rodio and
/// handy-recorder do it. `Some(48_000)` is what that turns into when the config
/// was read before the headset switched to call mode.
fn build_output(output: &cpal::Device, rate: Option<u32>, what: &str) -> Option<cpal::Stream> {
    let Ok(default) = output.default_output_config() else {
        note(format!("{what}: could not read the output config"));
        return None;
    };
    let config = cpal::StreamConfig {
        channels: default.channels(),
        sample_rate: rate.unwrap_or(default.sample_rate()),
        buffer_size: cpal::BufferSize::Default,
    };
    note(format!(
        "{what}: output device reports {} Hz, opening at {} Hz",
        default.sample_rate(),
        config.sample_rate
    ));
    let asked_at = Instant::now();
    let result = output.build_output_stream(
        config,
        |data: &mut [f32], _: &cpal::OutputCallbackInfo| data.fill(0.0),
        |e| eprintln!("output stream error: {e}"),
        None,
    );
    match result {
        Ok(stream) => {
            note(format!("{what}: opened in {:.0?}", asked_at.elapsed()));
            Some(stream)
        }
        Err(e) => {
            note(format!("{what}: failed after {:.1?}: {e}", asked_at.elapsed()));
            None
        }
    }
}

/// Builds an output stream, starts it and keeps it for a while.
fn open_output(output: &cpal::Device, rate: Option<u32>, what: &str, keep_for: Duration) {
    if let Some(stream) = build_output(output, rate, what) {
        let _ = stream.play();
        thread::sleep(keep_for);
        drop(stream);
        note(format!("{what}: closed"));
    }
}

fn print_system_log(window: Duration) {
    let wanted = [
        "Reconfigure of Output Device",
        "BTAudioMsgIdTransportReady",
        "Disconnecting audio from device",
        "Dispatch Switch",
        "Transitioning out of HFP",
        "Reject the switch",
        "Request eSCO",
        "NO eSCO running",
        "Starting IO on profile",
        "Setting sample rate",
        "Releasing SCO connection",
        "voice disconnection event",
        "LinkQualityReport",
    ];
    let messages: Vec<String> =
        wanted.iter().map(|w| format!(r#"eventMessage CONTAINS "{w}""#)).collect();
    let predicate = format!(
        r#"(process == "coreaudiod" OR process == "bluetoothd") AND ({})"#,
        messages.join(" OR ")
    );
    let last = format!("{}s", window.as_secs());
    let out = Command::new("/usr/bin/log")
        .args(["show", "--info", "--style", "compact", "--last", &last, "--predicate", &predicate])
        .output();
    let Ok(out) = out else {
        println!("  (could not run `log show`)");
        return;
    };
    let mut printed = 0;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // Keep the time and the message; hide the headset's address.
        let Some((head, message)) = line.split_once("] ") else { continue };
        let time = head.split_whitespace().nth(1).unwrap_or("");
        let message = if let Some(i) = message.find("AudioInput:") {
            // One per second while the call link is up: how much mic audio came in.
            let n: String = message[i + 11..]
                .chars()
                .skip_while(|c| *c == ' ')
                .take_while(|c| c.is_ascii_digit())
                .collect();
            format!("AudioInput: {n}")
        } else if message.contains("A2DP LinkQualityReport") {
            "A2DP LinkQualityReport (headset is in playback mode)".to_string()
        } else {
            hide_address(message)
        };
        println!("  {time}  {message}");
        printed += 1;
    }
    if printed == 0 {
        println!("  (nothing matched; `log show` may need an admin account)");
    }
}

fn hide_address(message: &str) -> String {
    // Bluetooth addresses look like AA:BB:CC:DD:EE:FF or AA-BB-CC-DD-EE-FF.
    let bytes = message.as_bytes();
    let mut out = String::with_capacity(message.len());
    let mut i = 0;
    while i < bytes.len() {
        let is_address = i + 17 <= bytes.len()
            && (0..17).all(|k| {
                let b = bytes[i + k];
                if k % 3 == 2 { b == b':' || b == b'-' } else { b.is_ascii_hexdigit() }
            });
        if is_address {
            out.push_str("XX:XX:XX:XX:XX:XX");
            i += 17;
        } else {
            let ch = message[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}
