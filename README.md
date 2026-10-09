# casparcg-osc-companion-timer

A small desktop app that listens to CasparCG's OSC output and writes the
**clip name and/or time** of selected channel/layers into **Bitfocus Companion
custom variables**. Show them on any button with `$(custom:<name>)`.

```
CasparCG --OSC/UDP--> casparcg-osc-companion-timer --OSC/UDP--> Companion
                      (filters, counts, dedupes)               /custom-variable/<name>/value
```

Example result on a button: `AMB - 00:07` (clip name, time remaining).

## Requirements

- **Rust 1.85 or newer** (to build)
- **Windows** (used and tested), **Linux** and **macOS** (Apple Silicon): builds
  are provided with each release, but they have **not been tested yet**.
  Please report problems.
- **CasparCG** with OSC output enabled
- **Bitfocus Companion** with its OSC listener enabled

### Companion version

Tested with **Companion 5.0.0** on Windows.

The app uses one Companion feature: the OSC command
`/custom-variable/<name>/value <text>`, described on the *OSC control* page of
the Companion user guide. It does not depend on how buttons are drawn. If your
version behaves differently, check that page of the user guide first.

## Build and run

```sh
cargo run --release
```

The built program is `target/release/casparcg-osc-companion-timer`
(`.exe` on Windows). Tests: `cargo test`

### macOS notes

The macOS build is not signed or notarised, so macOS may refuse to open it
("developer cannot be verified"). Extract the archive and run it from
Terminal, or allow it under System Settings > Privacy & Security. It is a
plain executable, not an `.app` bundle, and only for Apple Silicon (arm64).

## Settings

You enter everything in the app (see Setup below). Your settings are remembered
between runs in a `config.json` file that the app writes by itself, so there is
nothing to edit by hand:

- Windows: `%APPDATA%\casparcg-osc-companion-timer\config.json`
- Linux: `~/.config/casparcg-osc-companion-timer/config.json`

Delete that file to reset the app to its defaults.

## Setup

### 1. CasparCG: send OSC to the app

In `casparcg.config`, add a predefined OSC client that points at the machine
running this app. The port must match the app's *CasparCG OSC in* port
(default `6250`):

```xml
<osc>
  <predefined-clients>
    <predefined-client>
      <address>127.0.0.1</address>
      <port>6250</port>
    </predefined-client>
  </predefined-clients>
</osc>
```

This snippet is written from memory. Check it against the configuration
reference for your CasparCG version. Restart the CasparCG server after changing
the config.

### 2. Companion: enable OSC and create variables

1. Make sure Companion's OSC listener is enabled in its settings, and note the
   port (default `12321`).
2. Create one **custom variable** per binding, for example `timer1`. Create it
   before you start sending.
3. On a button, show the variable in the button's text element:
   `$(custom:timer1)`.

### 3. The app

1. Set **CasparCG OSC in** to the same UDP port as in `casparcg.config`.
2. Set **Companion OSC** to the host and port of the machine running Companion
   (`127.0.0.1:12321` if it is the same machine). Press **Apply**.
3. Press **+ Add binding** for each timer you want and fill in:
   - **Channel** and **Layer**: the CasparCG layer to follow. You add these
     yourself; nothing is auto-detected.
   - **Clip name** and **Time** checkboxes: what to include in the text.
     At least one must stay on.
   - **Count up / Count down** and the **time format** (`mm:ss` or
     `hh:mm:ss`). These only apply when *Time* is on.
   - **Companion custom variable**: the name you created in step 2.

To show both elapsed and remaining time for the same layer, add two bindings
for that layer with two different variable names.

Connection settings take effect when you press **Apply**. Bindings take effect
immediately.

## What gets sent

The text is built from the options you switched on:

| Clip name | Time | Text |
|-----------|------|------|
| on | on | `AMB 00:07` |
| on | off | `AMB` |
| off | on | `00:07` |
| off | off | nothing is sent (the binding shows a red message) |

How the time is calculated:

| Mode | Time shown | Rounding |
|------|------------|----------|
| Count up | Elapsed | Rounded down |
| Count down | Remaining | Rounded up, so it reads `00:01` until the last second is used, then `00:00` |

- If the clip has no known length (live input, HTML template), a countdown
  shows `--:--` (or `--:--:--`).
- If a layer is empty, or sends nothing for 2 seconds, the variable is set to
  an empty string. This also applies when only *Time* is on.
- A new value is sent only when the text changes, so about once per second per
  binding while a clip plays.
- Only the **foreground** of a layer is read:
  `/channel/<c>/stage/layer/<l>/foreground/file/name` and `.../file/time`
  (elapsed and total, in seconds).

## Status in the app

Each binding shows its state and the exact text it is sending:

- **no data yet**: nothing has arrived for this channel/layer. Check the
  channel and layer numbers, the port, and `casparcg.config`.
- **receiving data**: messages arrive.
- **last seen N s ago**: the layer went quiet.

A red message under a binding means it sends nothing until you fix it:

- the variable name is empty, or contains characters other than ASCII letters,
  digits, `_` and `-`
- another binding uses the same variable
- both *Clip name* and *Time* are off

## Troubleshooting

- **Nothing shows on the button.** Test Companion on its own: use the Generic
  OSC module on a button to send `/custom-variable/timer1/value` with a
  *string* argument. If that does not work either, the problem is in Companion
  (OSC listener disabled, wrong port, firewall, variable not created, or the
  button does not show `$(custom:timer1)`).
- **Companion or CasparCG is on another machine.** Use the right IP in the app
  and in `casparcg.config`, and allow the UDP ports through the firewall
  (Windows Firewall may ask the first time the app receives data).
- **"Cannot listen on UDP port ..."**: another program already uses the port.
- **Everything is empty at startup.** That is expected: a binding clears its
  variable until the layer reports a clip.

## Known limitations

- Renaming a binding's variable does not clear the old variable; it keeps its
  last text.
- After Companion restarts, a variable gets its value back on the next change
  of the text (within a second while a clip is playing).
- One Companion target, foreground layers only, no frame display.
- The variable name check is deliberately strict and is not Companion's
  official naming rule.

## Code layout

| Module | Responsibility |
|--------|----------------|
| `caspar_in` | Decode CasparCG OSC into clip updates, only for bound layers |
| `layer_state` | Latest name and time per (channel, layer), with a staleness timeout |
| `binding` | Pure rendering of the text, validation, dedupe |
| `companion_out` | `CompanionSink` gateway; UDP OSC implementation |
| `engine` | Ties the above together |
| `worker` | UDP listener thread, independent of the GUI |
| `config` | JSON settings |
| `app` (binary) | egui shell |
