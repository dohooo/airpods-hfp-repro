# airpods-hfp-repro

Test program for a [Handy](https://github.com/cjpais/Handy) bug report ([cjpais/Handy#2472](https://github.com/cjpais/Handy/issues/2472)): on macOS, the AirPods mic goes silent a few seconds into a recording.

It opens the same kind of streams Handy 0.10.0 opens when a recording starts, using the same cpal version (0.18.2) and nothing else. Twice a second it prints the mic level, and at the end it prints what coreaudiod and bluetoothd logged while it ran (`log show`, filtered to the call link lines, Bluetooth address masked).

## Run it

Connect your AirPods, make them the default input and output in macOS Sound settings, and keep talking while it runs. Your terminal needs microphone permission. Leave a few seconds between runs so the AirPods go back to normal playback mode.

```
cargo run -r -- control   # mic only
cargo run -r -- late      # mic, then an output stream at 48 kHz three seconds in
cargo run -r -- race      # silent output started together with the mic, plus the 48 kHz output
cargo run -r -- drop48    # like race, but the silent output is stopped after half a second
```

## What I get

AirPods Pro, macOS 27.2, MacBook Pro M4 Max.

| mode | runs | result |
|---|---|---|
| `control` | 1 | Mic fine the whole time. |
| `late` | 1 | macOS tears the call link down, the mic cuts out for about 1.5 s, then comes back. cpal gives up after 3.0 s with `Sample rate update timed out`. |
| `race` | 2 | Same teardown, at the moment the link comes up. The mic comes back 1 to 3 s later. |
| `drop48` | 2 | One run: the link comes back with `AudioInput: 0`, is released 3 s later and never set up again. The mic stream keeps delivering buffers, but frozen ones, until the program exits. This is what a failed Handy recording looks like in the system log. Other run: the 48 kHz request was handled before the mic's IO was registered (`Input IO  = NO`) and nothing happened. |

`AudioInput: N` comes from bluetoothd's link report, once per second while the call link is up. Around 66 is healthy.

The run that failed (`drop48`):

```
  -->  0.000s  silent stream: output device reports 48000 Hz, opening at 48000 Hz
  -->  0.007s  silent stream: opened in 6ms
  -->  0.503s  mic stream and silent stream started
  -->  0.505s  48 kHz output: output device reports 24000 Hz, opening at 48000 Hz
  1.0s  mic peak 0.0000  samples  19680  output device at 24000 Hz
  -->  1.021s  silent stream stopped
  1.5s  mic peak 0.0000  samples  12480  output device at 24000 Hz
  2.0s  mic peak 0.1205  samples  12000  output device at 24000 Hz
  2.5s  mic peak 0.1072  samples  12000  output device at 24000 Hz
  3.0s  mic peak 0.1118  samples  12000  output device at 24000 Hz
  -->  3.513s  48 kHz output: failed after 3.0s: Sample rate update timed out
  3.5s  mic peak 0.1428  samples  12480  output device at 24000 Hz
  4.0s  mic peak 0.1055  samples  12000  output device at 24000 Hz
  4.5s  mic peak 0.1919  samples  12000  output device at 24000 Hz
  5.1s  mic peak 0.1622  samples  12000  output device at 24000 Hz
  5.6s  mic peak 0.1278  samples  12000  output device at 24000 Hz
  6.1s  mic peak 0.1278  samples  12000  output device at 24000 Hz
  ...   the same 0.1278 in every line until the end
 12.1s  mic peak 0.1278  samples  12000  output device at 24000 Hz

What macOS logged meanwhile:
  15:16:16.491  HFPInputShimDevice: Reconfigure of Output Device required: Request Config change
  15:16:16.494  Starting IO on profile tacl, activeIO:0 ... Wait IO Start 1
  15:16:16.748  BTAudioMsgIdTransportReady, for UID: XX:XX:XX:XX:XX:XX-tacl
  15:16:16.756  TriggerSCOAudio: Request eSCO creation
  15:16:16.756  Starting IO on profile tsco, activeIO:0 ... Wait IO Start 0
  15:16:16.916  NO eSCO running while doing IO
  15:16:16.916  TriggerSCOAudio: Request eSCO creation
  15:16:16.916  Starting IO on profile tsco, activeIO:1 ... Wait IO Start 0
  15:16:17.824  BTAudioMsgIdTransportReady, for UID: XX:XX:XX:XX:XX:XX-tsco
  15:16:17.825  Dispatch Switch to sampling rate : 48000.000000 Input IO  = YES
  15:16:17.825  Transitioning out of HFP: Disconnect eSCO immediately
  15:16:17.825  TriggerSCOAudio: Request eSCO tear down
  15:16:17.827  Disconnecting audio from device XX:XX:XX:XX:XX:XX status 116
  15:16:18.058  IO running on SHIM Device and request to switch over profile out of HFP: Reject the switch 48000.000000
  15:16:18.058  NO eSCO running while doing IO
  15:16:18.058  TriggerSCOAudio: Request eSCO creation
  15:16:18.179  BTAudioMsgIdTransportReady, for UID: XX:XX:XX:XX:XX:XX-tsco
  15:16:18.970  BTUnifiedAudioDevice: Setting sample rate 24000.000000 -> 48000.000000
  15:16:19.075  IO running on SHIM Device and request to switch over profile out of HFP: Reject the switch 48000.000000
  15:16:19.196  AudioInput: 0
  15:16:20.197  AudioInput: 0
  15:16:21.110  Releasing SCO connection 0x0106
  15:16:21.110  Received voice disconnection event for device XX:XX:XX:XX:XX:XX
```

The other modes in the code (`hold`, `handy`, `early`, `drop`) are variations I tried that did not break anything. I left them in so the program is the one that produced these results.
