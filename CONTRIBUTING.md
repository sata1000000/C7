# Contributing to C7

## Development Setup

### Linux:

1. Clone the repo.

   ```bash
   git clone https://github.com/sata1000000/C7.git
   cd C7
   ```

2. Install Rust and GTK4 dependencies.

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config build-essential
   ```

3. Build and run.

   ```bash
   cargo run
   ```

### macOS:

1. Clone the repo.

   ```bash
   git clone https://github.com/sata1000000/C7.git
   cd C7
   ```

2. Install Rust and GTK4 dependencies with [Homebrew](https://brew.sh/).

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   brew install gtk4 libadwaita pkg-config adwaita-icon-theme hicolor-icon-theme
   ```

3. Build and run.

   ```bash
   cargo run
   ```

### Windows:

1. Install [MSYS2](https://www.msys2.org/). Use the UCRT64 environment.

2. Clone the repo in UCRT64.

   ```bash
   git clone https://github.com/sata1000000/C7.git
   cd C7
   ```

3. Install dependencies in UCRT64.

   ```bash
   pacman -S mingw-w64-ucrt-x86_64-gtk4 mingw-w64-ucrt-x86_64-libadwaita mingw-w64-ucrt-x86_64-pkg-config mingw-w64-ucrt-x86_64-gcc
   ```

4. Install Rust in UCRT64.

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustup default stable-x86_64-pc-windows-gnu
   ```

5. Build and run in UCRT64, with `.cargo/bin` in PATH.

   ```bash
   export PATH="/c/Users/$USER/.cargo/bin:$PATH"
   cargo run
   ```

## Project Tour

The most important places:

```
crates/
├── c7_app/
│   └── src/
│       ├── main.rs                   C7's entry point.
│       └── ui/
│           ├── screens/              Feature screens (sample upload, kit editor, etc.).
│           ├── modals/               Modal screens (settings, about, etc.).
│           ├── main_window.rs        Top-level navigation, device selector, menu.
│           ├── base_module.rs        Base class for all feature screens.
│           ├── widgets.rs            Custom GTK widgets.
│           └── mixins/               Shared UI Behaviors (MIDI listener, etc.).
│
├── c7_core/
│   ├── src/
│   │   ├── midi.rs                   MIDI functions.
│   │   ├── sysex.rs                  SysEx functions.
│   │   ├── device_config.rs          Settings persistence, platform-aware config paths.
│   │   ├── c7_file_interfacing.rs    `.c7` file format (JSON via serde_json).
│   │   ├── dsp_utils.rs              DSP related functions.
│   │   ├── wavetable_utils.rs        Wavetable related functions.
│   │   ├── bpm_detector.rs           Unnecessarily complicated function to get the BPM.
│   │   ├── sds.rs                    Sample Dump Standard (SDS) encode/decode.
│   │   └── bridge_interfacing.rs     Monitor server that C7 Bridge reports to.
│   └── devices/
│       ├── machinedrum/              Machinedrum configuration files.
│       └── monomachine/              Monomachine configuration files.
│
└── c7_bridge/
    ├── build.rs                      Discovers devices and adds their parameters to C7 Bridge.
    └── src/
        └── plugin.rs                 C7 Bridge CLAP plugin.
```

### Device JSON Files

The JSON files in [crates/c7_core/devices/](crates/c7_core/devices/) are the source of truth for all SysEx commands and MIDI CCs. If you are adding support for a new parameter, command, or machine, update the JSON rather than hardcoding values in Rust.

### Device Markdown Files

`crates/c7_core/devices/<DEVICE>/<DEVICE>_midi_official.md` and `<DEVICE>_sysex_official.md` are ripped from the device manuals and converted to markdown. DO NOT EDIT THESE. The intention is to have the data be 1:1 from the source material.

## Project Design Philosophy

- Readability is prioritized first. Standard Rust conventions are prioritized second.
- Never add error handling or fallbacks for cases that can't happen. Fallbacks are lazy. Fix the code to never need fallbacks. Trust the JSON.
- Code is mostly device-agnostic. Device features should be gated by using the `has_gate()` and `is_device()` functions.
- Never use `#[allow()]` to work around compiler issues.
- Always include `///` [doc comments](https://doc.rust-lang.org/reference/comments.html#r-comments.doc.syntax) above every function.
- Doc comments generally follow [RFC 1574](https://rust-lang.github.io/rfcs/1574-more-api-documentation-conventions.html) conventions.
- Add comments to any code that isn't [self-documenting](https://en.wikipedia.org/wiki/Self-documenting_code).

### AI Policy

- AI usage is allowed. It's expected that the human behind the PR is able to explain every decision made in the diff.
- PRs must be authored by a human. An AI assistant can't be listed as an author or co-author.

## Before Making Changes

- Run `cargo fmt` to make sure that everything is [rustfmt](https://doc.rust-lang.org/stable/edition-guide/rust-2024/rustfmt.html) compliant.
- Run `cargo clippy` to make sure that there are no errors or warnings.
- Test any edge cases of the feature you touched.
- There is no test suite. Validate by running C7 against real hardware.