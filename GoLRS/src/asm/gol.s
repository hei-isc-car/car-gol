# Game of Life — RISC-V RV32IMC assembly
#
# Exported symbols:
#   gol_init(cur: *mut u32, cols: u32, rows: u32)
#       Place a blinker (3-cell horizontal oscillator) in the centre of cur[].
#       All other cells are set to 0x00000000 (dead).
#
#   gol_step(cur: *mut u32, nxt: *mut u32, cols: u32, rows: u32)
#       Compute one GOL generation from cur[] into nxt[], then send the grid
#           rust_send_grid(cur_ptr, total_u32_count)
#       and swap grids
#           rust_swap_grids()
#
# Grid encoding (matches the viewer's protocol):
#   Each cell is one u32 (4 bytes), stored row-major.
#   Alive  = 0x--??????  (white or color)
#   Dead   = 0x--000000  (black)
#
# Assemble this file for the ISA used by the ESP32-C3 target.
  # Save current assembler options so they can be restored later.
  .option push
  # Disable linker relaxation assumptions for predictable instruction/layout behavior.
  .option norelax
  # Disable compressed (RVC) instruction emission.
  .option norvc
  # Declare the target architecture for this object.
  .attribute arch, "rv32im"
  # Put executable code in the text section.
  .section .text, "ax", @progbits
  # Export entry marker symbol for the assembler block.
  .globl __gol_asm_start
__gol_asm_start:

# ─────────────────────────────────────────────────────────────────────────────
# gol_init(cur: *mut u32, cols: u32, rows: u32)
#
# TODO: Arguments and usage comments
#
# ─────────────────────────────────────────────────────────────────────────────
  .globl gol_init
  .type  gol_init, @function
gol_init:
  ret

# ─────────────────────────────────────────────────────────────────────────────
# gol_step(cur: *mut u32, nxt: *mut u32, cols: u32, rows: u32)
#
# TODO: Arguments and usage comments
#
# ─────────────────────────────────────────────────────────────────────────────
  .globl gol_step
  .type  gol_step, @function
gol_step:
    ret

  .globl __gol_asm_end
__gol_asm_end:
  # Restore assembler options saved by .option push at file start.
  .option pop
