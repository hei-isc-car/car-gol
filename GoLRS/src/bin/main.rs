#![no_std]
#![no_main]
#![deny(
  clippy::mem_forget,
  reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[path = "runtime/board.rs"]
mod board;
#[path = "runtime/buttons.rs"]
mod buttons;
#[path = "runtime/grid.rs"]
mod grid;
#[cfg(feature = "board-lcdkit")]
#[path = "runtime/lcd.rs"]
mod lcd;
#[path = "runtime/uart.rs"]
mod uart;

use esp_backtrace as _; // Unused import for panic handler setup
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use grid::{
  CELLS, COLS, ROWS, current_and_next_ptrs, initial_grid_ptr, randomize_startup_grids,
  try_apply_pending_grid,
};
use log::{debug, info, warn};
use uart::{GridFrameListener, rust_send_grid, take_step_once_request};

#[cfg(feature = "board-lcdkit")]
use esp_hal::{
  gpio::{Level, Output, OutputConfig},
  rmt::Rmt,
  spi::master::{Config as SpiConfig, Spi},
  time::Rate,
};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

const STEP_PERIOD_MIN_MS: u32 = 100;
const STEP_PERIOD_MAX_MS: u32 = 2500;
const STEP_PERIOD_STEP_MS: u32 = 100;

// ---------------------------------------------------------------------------
// ASM functions
// ---------------------------------------------------------------------------
core::arch::global_asm!(include_str!("../asm/gol.s"), options(raw));

unsafe extern "C" {
  static __gol_asm_start: u8;
  static __gol_asm_end: u8;

  /// Place blinker oscillator in the centre of the grid and zero everything else.
  unsafe fn gol_init(cur: *mut u32, cols: u32, rows: u32);

  /// Compute one GOL generation from cur into nxt, then call
  /// rust_send_grid(nxt, total) and rust_swap_grids().
  unsafe fn gol_step(cur: *mut u32, nxt: *mut u32, cols: u32, rows: u32);
}

// ---------------------------------------------------------------------------

#[allow(
  clippy::large_stack_frames,
  reason = "main may hold grid pointers on stack"
)]
#[main]
fn main() -> ! {
  // generator version: 1.3.0
  // generator parameters: --chip esp32c3 -o esp32c3-mini-1 -o unstable-hal -o log -o esp-backtrace -o vscode -o stable-x86_64-pc-windows-msvc
  esp_println::logger::init_logger_from_env();
  warn!("Starting GOL on {}", board::BOARD.name);
  info!(
    "Buttons: decrease GPIO{}, increase GPIO{}, LED GPIO{}",
    board::BOARD.step_decrease_button_gpio,
    board::BOARD.step_increase_button_gpio,
    board::BOARD.status_led_gpio
  );

  for peripheral in board::BOARD.wired_peripherals {
    debug!("Board peripheral: {}", peripheral);
  }

  let config = esp_hal::Config::default();
  let peripherals = esp_hal::init(config);
  let mut board_io = {
    #[cfg(feature = "board-devkit-rust-2")]
    {
      board::BoardIo::new(
        esp_hal::gpio::Input::new(
          peripherals.GPIO2,
          esp_hal::gpio::InputConfig::default().with_pull(board::BOARD.button_pull),
        ),
        esp_hal::gpio::Input::new(
          peripherals.GPIO0,
          esp_hal::gpio::InputConfig::default().with_pull(board::BOARD.button_pull),
        ),
        esp_hal::gpio::Output::new(
          peripherals.GPIO7,
          esp_hal::gpio::Level::Low,
          esp_hal::gpio::OutputConfig::default(),
        ),
      )
    }

    #[cfg(feature = "board-lcdkit")]
    {
      // RMT driver at 80 MHz (required for WS2812 timing accuracy)
      let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).expect("Failed to initialize RMT0");
      let lcd_sda = peripherals.GPIO0;
      let lcd_scl = peripherals.GPIO1;
      let lcd_dc = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
      let lcd_cs = Output::new(peripherals.GPIO7, Level::High, OutputConfig::default());
      let lcd_backlight = Output::new(peripherals.GPIO5, Level::Low, OutputConfig::default());
      let lcd_spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
          .with_frequency(Rate::from_mhz(10))
          .with_mode(esp_hal::spi::Mode::_0),
      )
      .expect("Failed to initialize LCD SPI")
      .with_sck(lcd_scl)
      .with_mosi(lcd_sda);

      board::BoardIo::new(
        esp_hal::gpio::Input::new(
          peripherals.GPIO10,
          esp_hal::gpio::InputConfig::default().with_pull(esp_hal::gpio::Pull::Up),
        ),
        esp_hal::gpio::Input::new(
          peripherals.GPIO6,
          esp_hal::gpio::InputConfig::default().with_pull(esp_hal::gpio::Pull::Up),
        ),
        esp_hal::gpio::Input::new(
          peripherals.GPIO9,
          esp_hal::gpio::InputConfig::default().with_pull(esp_hal::gpio::Pull::Up),
        ),
        // WS2812
        peripherals.GPIO8.into(),
        rmt.channel0,
        // LCD
        lcd_spi,
        lcd_dc,
        lcd_cs,
        lcd_backlight,
      )
    }
  };

  board_io.set_activity_led(true);
  #[cfg(feature = "board-lcdkit")]
  {
    board_io.lcd().show_splash();
    board_io.lcd().backlight_on();
  }

  let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
  let (mut usb_rx, _) = usb_serial.split();
  let mut frame_listener = GridFrameListener::new();

  for pin in board_io.wired_peripherals() {
    debug!("Fixed board pin: {}", pin);
  }

  // Initialise: place blinker in grid A (current).
  debug!("Initialising grid");
  randomize_startup_grids();
  let cur_ptr = initial_grid_ptr();
  let (_cur_ptr, _nxt_ptr) = current_and_next_ptrs();
  debug!(" * cur_ptr = {:p}, nxt_ptr = {:p}", cur_ptr, _nxt_ptr);
  let gol_start = core::ptr::addr_of!(__gol_asm_start);
  let gol_end = core::ptr::addr_of!(__gol_asm_end);
  let gol_init_addr = gol_init as *const ();
  let gol_step_addr = gol_step as *const ();
  debug!(" * ASM region = {:p}..{:p}", gol_start, gol_end);
  debug!(" * gol_init   = {:p}", gol_init_addr);
  debug!(" * gol_step   = {:p}", gol_step_addr);
  debug!(" * Initialising grid with blinker oscillator in the centre");
  unsafe { gol_init(cur_ptr, COLS as u32, ROWS as u32) };
  info!("Running GOL main loop");

  let mut step_period_ms: u32 = 1000;
  let mut last_step = Instant::now();

  loop {
    let now = Instant::now();
    let step_period = Duration::from_millis(step_period_ms as u64);

    board_io.poll(now);

    if board_io.take_step_down() {
      step_period_ms = step_period_ms
        .saturating_sub(STEP_PERIOD_STEP_MS)
        .max(STEP_PERIOD_MIN_MS);
      info!("Step period decreased to {} ms", step_period_ms);
    }

    if board_io.take_step_up() {
      step_period_ms = step_period_ms
        .saturating_add(STEP_PERIOD_STEP_MS)
        .min(STEP_PERIOD_MAX_MS);
      info!("Step period increased to {} ms", step_period_ms);
    }

    // Drain any host bytes and stage a full grid if one arrives.
    frame_listener.poll(&mut usb_rx);

    // Pick current / next pointers based on the swap flag.
    let (cur, nxt) = current_and_next_ptrs();

    // Apply incoming grid once, instead of computing a new generation.
    if try_apply_pending_grid(cur) {
      // Push the applied grid immediately so the viewer updates right away.
      rust_send_grid(cur as *const u32, CELLS as u32);
      info!("Applied host grid and sent immediate display update");
      // Restart timing after host override to avoid an immediate local step.
      last_step = Instant::now();
    }

    if (take_step_once_request() || board_io.take_step_once())
      && !frame_listener.frame_in_progress()
    {
      unsafe { gol_step(cur, nxt, COLS as u32, ROWS as u32) };
      #[cfg(feature = "board-lcdkit")]
      unsafe {
        board_io.lcd().show_game_of_life::<32, 32>(nxt);
      };
      board_io.pulse_activity_led();
      last_step = Instant::now();
      continue;
    }

    if !frame_listener.frame_in_progress() && last_step.elapsed() >= step_period {
      // Compute next gen, send over UART, swap.
      unsafe { gol_step(cur, nxt, COLS as u32, ROWS as u32) };
      #[cfg(feature = "board-lcdkit")]
      unsafe {
        board_io.lcd().show_game_of_life::<32, 32>(nxt);
      };
      board_io.pulse_activity_led();
      last_step = Instant::now();
    }
  }
}
