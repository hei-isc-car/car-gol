##################################################
# Variables
#
project_dir   := justfile_directory()
wait_cmd      := if os() == "windows" { "timeout /t 2 /nobreak > nul" } else { "sleep 2" }
host_target   := if os() == "windows" {
  if arch() == "x86_64" {
    "x86_64-pc-windows-msvc"
  } else {
    "aarch64-pc-windows-msvc"
  }
} else if os() == "macos" {
  if arch() == "x86_64" {
    "x86_64-apple-darwin"
  } else {
    "aarch64-apple-darwin"
  }
} else {
  if arch() == "x86_64" {
    "x86_64-unknown-linux-gnu"
  } else {
    "aarch64-unknown-linux-gnu"
  }
}

set shell := ["bash", "-uc"]
set windows-shell := ["cmd.exe", "/c"]

DEF_PORT := "COM3" # dev/ttyUSB0
DEF_BAUD := "115200"

##################################################
# COMMANDS
#
# List all commands
default:
  just --list

# Information about the environment
@info:
  echo "Environment Informations\n------------------------\n"
  echo "    OS          : {{os()}}({{arch()}})"
  echo "    Projectdir  : {{project_dir}}"


# Install rust toolchain deps
@_rust_deps:
  rustup toolchain install stable
  rustup component add rust-src --toolchain stable
  rustup component add clippy --toolchain stable
  rustup component add rustfmt --toolchain stable
  rustup target add riscv32imc-unknown-none-elf --toolchain stable

# Install flash deps
@_flash_deps: _rust_deps
  cargo install espflash --locked

# Install debug deps
@_debug_deps: _rust_deps
  cargo install probe-rs-tools --locked

# Format GoLRS
@_fmt_golrs: _rust_deps
  echo "Formatting GoLRS"
  cd GoLRS && cargo fmt

# Format GoLRS HIL
@_fmt_hil: _rust_deps
  echo "Formatting GoLRS/hil"
  cd GoLRS/hil && cargo fmt

# Lint GoLRS
@_clippy_golrs: _rust_deps
  echo "Linting GoLRS"
  cd GoLRS && cargo clippy --release -- -D warnings

# Lint GoLRS HIL
@_clippy_hil: _rust_deps
  echo "Linting GoLRS/hil"
  cd GoLRS/hil && cargo clippy --release --target {{host_target}} -- -D warnings

# Format all Rust crates
@all-fmt: _fmt_golrs _fmt_hil

# Lint all Rust crates
@all-clippy: _clippy_golrs _clippy_hil

# Compile esp-rs project without flashing
@_build_raw: _flash_deps
  echo "Building esp-rs project"
  cd GoLRS && cargo build --release

# Build esp-rs project without flashing
@gol-build: _fmt_golrs _clippy_golrs _build_raw

# Build GoLRS debug-ready firmware for VS Code debugging
@gol-debug-build: _fmt_golrs _clippy_golrs _debug_deps
  echo "Building GoLRS debug-ready release firmware"
  cd GoLRS && cargo build --release

# Flash esp-rs board
@_flash_raw: _flash_deps
  echo "Flashing esp-rs board"
  cd GoLRS && cargo run --release

# Flash esp-rs board
@gol-flash: _fmt_golrs _clippy_golrs _flash_raw

# Run gol-viewer
[windows]
@gol-viewer:
  echo "Run gol-viewer"
  @powershell -NoProfile -Command "$f = Get-ChildItem 'GoLViewer\GoLViewer_*_*-*-windows-*' | Sort-Object {[version](($_.BaseName -split '_')[1])} | Select-Object -Last 1; & $f.FullName"

# Run gol-viewer
[linux]
@gol-viewer:
  echo "Run gol-viewer"
  @latest=$$(find GoLViewer -maxdepth 1 -type f -name 'GoLViewer_*_*-*-linux-*' \
    | sort -t_ -k2 -V \
    | tail -n1); \
  echo "Running $$latest"; \
  exec "$$latest"
  
# Run gol-viewer
[macos]
@gol-viewer:
  echo "Run gol-viewer"
  @latest=$$(find GoLViewer -maxdepth 1 -type f -name 'GoLViewer_*_*-*-macos-*' \
    | sort -t_ -k2 -V \
    | tail -n1); \
  echo "Running $$latest"; \
  exec "$$latest"

# Flash the board and run the viewer
@run: _fmt_golrs _clippy_golrs _flash_raw gol-viewer

# Run hardware-in-the-loop oracle tests against board assembly implementation
# Full flow: flash firmware, wait for serial reconnect, then run oracle checker
@_test_hil_raw port=DEF_PORT baud=DEF_BAUD: _flash_raw
  {{wait_cmd}}
  cd GoLRS/hil && cargo run --release --target {{host_target}} -- --port {{port}} --baud {{baud}}

# Run hardware-in-the-loop oracle tests against board assembly implementation
@test port=DEF_PORT baud=DEF_BAUD: _fmt_golrs _clippy_golrs _fmt_hil _clippy_hil
  echo Running hardware-in-the-loop oracle tests with port={{port}} and baud={{baud}}
  just _test_hil_raw port={{port}} baud={{baud}}