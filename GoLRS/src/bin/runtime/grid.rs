pub const ROWS: usize = 32;
pub const COLS: usize = 32;
pub const CELLS: usize = ROWS * COLS;

use esp_hal::time::Instant;

pub static mut GRID_A: [u32; CELLS] = [0u32; CELLS];
pub static mut GRID_B: [u32; CELLS] = [0u32; CELLS];

pub static mut CUR_IS_B: bool = false;

pub static mut RX_GRID_PENDING: [u32; CELLS] = [0u32; CELLS];
pub static mut RX_GRID_PENDING_READY: bool = false;

const SPLITMIX64_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

pub fn initial_grid_ptr() -> *mut u32 {
  core::ptr::addr_of_mut!(GRID_A) as *mut u32
}

pub fn randomize_startup_grids() {
  let mut state = startup_seed();

  unsafe {
    fill_grid(core::ptr::addr_of_mut!(GRID_A) as *mut u32, &mut state);
    fill_grid(core::ptr::addr_of_mut!(GRID_B) as *mut u32, &mut state);
    CUR_IS_B = false;
    RX_GRID_PENDING_READY = false;
  }
}

fn startup_seed() -> u64 {
  let cycle_seed = Instant::now().duration_since_epoch().as_micros();
  let addr_seed = (core::ptr::addr_of!(GRID_A) as usize as u64)
    ^ ((core::ptr::addr_of!(GRID_B) as usize as u64) << 1)
    ^ ((core::ptr::addr_of!(CUR_IS_B) as usize as u64) << 2);
  let seed = cycle_seed ^ addr_seed ^ 0xD1B5_4A32_D192_ED03;

  seed | 1
}

unsafe fn fill_grid(dst: *mut u32, state: &mut u64) {
  for idx in 0..CELLS {
    unsafe {
      dst.add(idx).write(next_random_u32(state));
    }
  }
}

fn next_random_u32(state: &mut u64) -> u32 {
  *state = state.wrapping_add(SPLITMIX64_GAMMA);

  let mut word = *state;
  word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
  word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);

  (word ^ (word >> 31)) as u32
}

pub fn current_and_next_ptrs() -> (*mut u32, *mut u32) {
  unsafe {
    if CUR_IS_B {
      (
        core::ptr::addr_of_mut!(GRID_B) as *mut u32,
        core::ptr::addr_of_mut!(GRID_A) as *mut u32,
      )
    } else {
      (
        core::ptr::addr_of_mut!(GRID_A) as *mut u32,
        core::ptr::addr_of_mut!(GRID_B) as *mut u32,
      )
    }
  }
}

pub fn try_apply_pending_grid(dst: *mut u32) -> bool {
  let has_pending = unsafe { RX_GRID_PENDING_READY };
  if !has_pending {
    return false;
  }

  let pending_ptr = core::ptr::addr_of!(RX_GRID_PENDING) as *const u32;
  unsafe {
    core::ptr::copy_nonoverlapping(pending_ptr, dst, CELLS);
    RX_GRID_PENDING_READY = false;
  }

  true
}

#[unsafe(no_mangle)]
pub(crate) extern "C" fn rust_swap_grids() {
  unsafe {
    CUR_IS_B = !CUR_IS_B;
  }
}
