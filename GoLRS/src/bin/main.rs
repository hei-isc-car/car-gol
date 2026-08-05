#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[path = "runtime/buttons.rs"]
mod buttons;
#[path = "runtime/grid.rs"]
mod grid;
#[path = "runtime/uart.rs"]
mod uart;

use buttons::DebouncedButton;
use esp_backtrace as _; // Unused import for panic handler setup
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_hal::usb_serial_jtag::UsbSerialJtag;
use grid::{
    CELLS, COLS, ROWS, current_and_next_ptrs, initial_grid_ptr, randomize_startup_grids,
    try_apply_pending_grid,
};
use log::{debug, info, warn};
use uart::{GridFrameListener, rust_send_grid, take_step_once_request};

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
    warn!("Starting GOL on ESP32-C3 Mini-1");

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);
    let button_config = InputConfig::default().with_pull(Pull::Down);
    let _btn0 = Input::new(peripherals.GPIO3, button_config);
    let btn1 = Input::new(peripherals.GPIO2, button_config);
    let _btn2 = Input::new(peripherals.GPIO1, button_config);
    let btn3 = Input::new(peripherals.GPIO0, button_config);
    let mut grid_led = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());
    let usb_serial = UsbSerialJtag::new(peripherals.USB_DEVICE);
    let (mut usb_rx, _) = usb_serial.split();
    let mut frame_listener = GridFrameListener::new();
    let mut btn1_state = DebouncedButton::new(Instant::now());
    let mut btn3_state = DebouncedButton::new(Instant::now());

    // The following pins are used to bootstrap the chip. They are available
    // for use, but check the datasheet of the module for more information on them.
    // - GPIO2
    // - GPIO8
    // - GPIO9
    // These GPIO pins are in use by some feature of the module and should not be used.
    let _ = peripherals.GPIO11;
    let _ = peripherals.GPIO12;
    let _ = peripherals.GPIO13;
    let _ = peripherals.GPIO14;
    let _ = peripherals.GPIO15;
    let _ = peripherals.GPIO16;
    let _ = peripherals.GPIO17;

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

        if btn1_state.update(now, btn1.is_high()) {
            step_period_ms = step_period_ms
                .saturating_sub(STEP_PERIOD_STEP_MS)
                .max(STEP_PERIOD_MIN_MS);
            info!("Step period decreased to {} ms", step_period_ms);
        }

        if btn3_state.update(now, btn3.is_high()) {
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

        if take_step_once_request() && !frame_listener.frame_in_progress() {
            unsafe { gol_step(cur, nxt, COLS as u32, ROWS as u32) };
            grid_led.toggle();
            last_step = Instant::now();
            continue;
        }

        if !frame_listener.frame_in_progress() && last_step.elapsed() >= step_period {
            // Compute next gen, send over UART, swap.
            unsafe { gol_step(cur, nxt, COLS as u32, ROWS as u32) };
            grid_led.toggle();
            last_step = Instant::now();
        }
    }
}
