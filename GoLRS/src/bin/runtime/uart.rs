use esp_hal::Blocking;
use esp_hal::usb_serial_jtag::UsbSerialJtagRx;
use log::{info, warn};

use crate::grid::{CELLS, RX_GRID_PENDING, RX_GRID_PENDING_READY};

pub const FRAME_HEADER_0: u8 = 0xAB;
pub const FRAME_HEADER_1: u8 = 0xCD;
pub const FRAME_PAYLOAD_BYTES: usize = CELLS * 4;
pub const HOST_CMD_SEND_GRID: u8 = 0x01;
pub const HOST_CMD_STEP_ONCE: u8 = 0x02;

pub static mut STEP_ONCE_PENDING: bool = false;
static mut FRAME_PAYLOAD_BUFFER: [u8; FRAME_PAYLOAD_BYTES] = [0u8; FRAME_PAYLOAD_BYTES];

pub struct GridFrameListener {
    payload_idx: usize,
    payload: *mut [u8; FRAME_PAYLOAD_BYTES],
    waiting_pattern_id: bool,
}

impl GridFrameListener {
    pub fn new() -> Self {
        Self {
            payload_idx: 0,
            payload: core::ptr::addr_of_mut!(FRAME_PAYLOAD_BUFFER),
            waiting_pattern_id: false,
        }
    }

    pub fn poll(&mut self, rx: &mut UsbSerialJtagRx<'_, Blocking>) {
        while let Ok(byte) = rx.read_byte() {
            self.push_byte(byte);
        }
    }

    pub fn frame_in_progress(&self) -> bool {
        self.waiting_pattern_id || self.payload_idx > 0
    }

    fn push_byte(&mut self, byte: u8) {
        if self.payload_idx == 0 && !self.waiting_pattern_id && byte == HOST_CMD_STEP_ONCE {
            unsafe {
                STEP_ONCE_PENDING = true;
            }
            return;
        }

        if self.payload_idx == 0 && !self.waiting_pattern_id && byte == HOST_CMD_SEND_GRID {
            self.waiting_pattern_id = true;
            return;
        }
        if self.waiting_pattern_id {
            self.waiting_pattern_id = false;
            return;
        }

        unsafe {
            (*self.payload)[self.payload_idx] = byte;
        }
        self.payload_idx += 1;

        if self.payload_idx == FRAME_PAYLOAD_BYTES {
            let payload = unsafe { &*self.payload };
            if decode_and_stage_grid(payload) {
                info!("Received and staged raw grid payload from host");
            } else {
                warn!("Discarded invalid raw grid payload from host");
            }

            self.payload_idx = 0;
        }
    }
}

pub fn take_step_once_request() -> bool {
    unsafe {
        let pending = STEP_ONCE_PENDING;
        STEP_ONCE_PENDING = false;
        pending
    }
}

fn decode_and_stage_grid(payload: &[u8; FRAME_PAYLOAD_BYTES]) -> bool {
    let pending_ptr = core::ptr::addr_of_mut!(RX_GRID_PENDING) as *mut u32;

    for cell_idx in 0..CELLS {
        let o = cell_idx * 4;
        let cell = u32::from_be_bytes([payload[o], payload[o + 1], payload[o + 2], payload[o + 3]]);

        unsafe {
            pending_ptr.add(cell_idx).write(cell);
        }
    }

    unsafe {
        RX_GRID_PENDING_READY = true;
    }
    true
}

/// Sends the grid as a raw binary frame over USB Serial/JTAG.
/// Frame format: 0xAB 0xCD | count*4 raw bytes (4 bytes per cell, big-endian u32).
#[unsafe(no_mangle)]
pub(crate) extern "C" fn rust_send_grid(ptr: *const u32, count: u32) {
    const EP1: *mut u32 = 0x6004_3000 as *mut u32;
    const EP1_CONF: *mut u32 = 0x6004_3004 as *mut u32;
    const FREE: u32 = 1 << 1;
    const FLUSH: u32 = 1 << 0;

    let cells = unsafe { core::slice::from_raw_parts(ptr, count as usize) };

    let send_byte = |b: u8| unsafe {
        while EP1_CONF.read_volatile() & FREE == 0 {}
        EP1.write_volatile(b as u32);
    };

    send_byte(FRAME_HEADER_0);
    send_byte(FRAME_HEADER_1);

    for &cell in cells {
        send_byte(((cell >> 24) & 0xFF) as u8);
        send_byte(((cell >> 16) & 0xFF) as u8);
        send_byte(((cell >> 8) & 0xFF) as u8);
        send_byte((cell & 0xFF) as u8);
    }

    unsafe { EP1_CONF.write_volatile(EP1_CONF.read_volatile() | FLUSH) };
}
