use embedded_graphics::{
  Pixel,
  draw_target::DrawTarget,
  geometry::{OriginDimensions, Size},
  mono_font::{MonoFont, MonoTextStyle, ascii::FONT_10X20},
  pixelcolor::Rgb565,
  prelude::*,
  text::Text,
};

use embedded_hal_bus::spi::ExclusiveDevice;

use display_interface_spi::SPIInterface;

use esp_hal::{Blocking, delay::Delay, gpio::Output, spi::master::Spi};

use gc9a01::{Gc9a01, display::DisplayResolution240x240, rotation::DisplayRotation};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type LcdSpiDevice =
  ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, embedded_hal_bus::spi::NoDelay>;

type LcdInterface = SPIInterface<LcdSpiDevice, Output<'static>>;

type LcdDriver = Gc9a01<LcdInterface, DisplayResolution240x240, gc9a01::mode::BasicMode>;

const TILE_WIDTH: usize = 10;
const TILE_HEIGHT: usize = 20;

struct TileBuffer {
  pixels: [Rgb565; TILE_WIDTH * TILE_HEIGHT],
}
impl TileBuffer {
  fn new(color: Rgb565) -> Self {
    Self {
      pixels: [color; TILE_WIDTH * TILE_HEIGHT],
    }
  }

  fn as_u16_iter(&self) -> impl Iterator<Item = u16> + '_ {
    self.pixels.iter().map(|color| color.into_storage())
  }
}

impl OriginDimensions for TileBuffer {
  fn size(&self) -> Size {
    Size::new(TILE_WIDTH as u32, TILE_HEIGHT as u32)
  }
}

impl DrawTarget for TileBuffer {
  type Color = Rgb565;
  type Error = core::convert::Infallible;

  fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
  where
    I: IntoIterator<Item = Pixel<Self::Color>>,
  {
    for Pixel(point, color) in pixels {
      if point.x < 0 || point.y < 0 || point.x >= TILE_WIDTH as i32 || point.y >= TILE_HEIGHT as i32
      {
        continue;
      }

      let x = point.x as usize;
      let y = point.y as usize;

      self.pixels[y * TILE_WIDTH + x] = color;
    }

    Ok(())
  }
}

// ---------------------------------------------------------------------------
// LCD
// ---------------------------------------------------------------------------

const LOGO_WIDTH: u16 = 100;
const LOGO_HEIGHT: u16 = 100;
static LOGO_RGB565: &[u8] = include_bytes!("../../res/car-gol.bin");
static LOGO_RGB565_FULLW: &[u8] = include_bytes!("../../res/car-gol-fullw.bin");

pub struct LcdDisplay {
  display: LcdDriver,
  backlight: Output<'static>,
}

#[allow(dead_code)]
impl LcdDisplay {
  /// Initialize the ESP32-C3-LCDKit LCD.
  ///
  /// The GPIO and SPI configuration is intentionally done by main().
  ///
  /// LCDKit:
  ///
  /// GPIO0 = MOSI / SDA
  /// GPIO1 = SCLK / SCL
  /// GPIO2 = D/C
  /// GPIO7 = CS
  /// GPIO5 = Backlight
  ///
  /// Display:
  ///
  /// GC9A01
  /// 240 x 240
  pub fn new(
    spi: Spi<'static, Blocking>,
    dc: Output<'static>,
    cs: Output<'static>,
    backlight: Output<'static>,
  ) -> Self {
    // Turn:
    //
    //     SPI bus + CS
    //
    // into an embedded-hal SpiDevice.
    //
    // The GC9A01 interface uses this SpiDevice to communicate with
    // the display.
    let spi_device =
      ExclusiveDevice::new_no_delay(spi, cs).expect("Failed to create LCD SPI device");

    // ---------------------------------------------------------------
    // Display interface
    // ---------------------------------------------------------------

    let interface = SPIInterface::new(spi_device, dc);

    // ---------------------------------------------------------------
    // GC9A01
    // ---------------------------------------------------------------

    let mut display = Gc9a01::new(
      interface,
      DisplayResolution240x240,
      DisplayRotation::Rotate180,
    );

    // ---------------------------------------------------------------
    // Controller initialization
    // ---------------------------------------------------------------

    let mut delay = Delay::new();

    display
      .init_with_addr_mode(&mut delay)
      .expect("Failed to initialize GC9A01");

    Self { display, backlight }
  }

  fn draw_char(
    &mut self,
    x: i32,
    y: i32,
    c: char,
    font: &'static MonoFont<'static>,
    foreground: Rgb565,
    background: Rgb565,
  ) {
    let mut tile = TileBuffer::new(background);

    let style = MonoTextStyle::new(font, foreground);

    let mut text = [0u8; 4];
    let s = c.encode_utf8(&mut text);

    Text::new(s, Point::new(0, TILE_HEIGHT as i32 - 6), style)
      .draw(&mut tile)
      .expect("Failed to render character");

    if x < 0 || y < 0 || x + TILE_WIDTH as i32 > 240 || y + TILE_HEIGHT as i32 > 240 {
      return;
    }

    self
      .display
      .set_pixels(
        (x as u16, y as u16),
        (
          (x + TILE_WIDTH as i32 - 1) as u16,
          (y + TILE_HEIGHT as i32 - 1) as u16,
        ),
        &mut tile.as_u16_iter(),
      )
      .expect("Failed to send character to LCD");
  }

  pub fn draw_text(&mut self, text: &str, x: i32, y: i32, color: Rgb565) {
    let mut cursor_x = x;

    for c in text.chars() {
      self.draw_char(cursor_x, y, c, &FONT_10X20, color, Rgb565::BLACK);

      cursor_x += TILE_WIDTH as i32;
    }
  }

  pub fn draw_logo(&mut self, x: u16, y: u16) {
    let expected_size = LOGO_WIDTH as usize * LOGO_HEIGHT as usize * 2;

    assert_eq!(LOGO_RGB565.len(), expected_size, "Invalid logo RGB565 size");

    let mut pixels = LOGO_RGB565
      .as_chunks::<2>()
      .0
      .iter()
      .map(|b| u16::from_be_bytes([b[0], b[1]]));

    self
      .display
      .set_pixels(
        (x, y),
        (x + LOGO_WIDTH - 1, y + LOGO_HEIGHT - 1),
        &mut pixels,
      )
      .expect("Failed to draw logo");
  }

  /// Display the CAR_GOL startup screen.
  pub fn show_splash(&mut self) {
    self.clear();
    self.draw_text("CAR_GOL", 85, 30, Rgb565::WHITE);
    self.draw_logo(70, 60);
  }

  pub fn show_splash_with(&mut self, text: &str, x: i32, y: i32, color: Rgb565) {
    self.show_splash();
    // Split text based on newlines and draw each line separately with a vertical offset
    // Since it is no_std, we cannot use split() directly, so we will iterate over the characters and handle newlines manually
    let mut line_start = 0;
    let mut line_number = 0;
    for (i, c) in text.chars().enumerate() {
      if c == '\n' || i == text.len() - 1 {
        let line_end = if c == '\n' { i } else { i + 1 };
        let line = &text[line_start..line_end];
        self.draw_text(line, x, y + (line_number * (TILE_HEIGHT + 2) as i32), color);
        line_start = i + 1;
        line_number += 1;
      }
    }
  }

  pub fn show_logo_fullwidth(&mut self) {
    let expected_size = 240 * 240 * 2;

    assert_eq!(
      LOGO_RGB565_FULLW.len(),
      expected_size,
      "Invalid logo RGB565 size"
    );

    let mut pixels = LOGO_RGB565_FULLW
      .as_chunks::<2>()
      .0
      .iter()
      .map(|b| u16::from_be_bytes([b[0], b[1]]));

    self
      .display
      .set_pixels((0, 0), (239, 239), &mut pixels)
      .expect("Failed to draw logo");
  }

  /// Clear the display to black.
  pub fn clear(&mut self) {
    self.fill(Rgb565::BLACK);
  }

  /// Fill the display with a solid color.
  pub fn fill(&mut self, color: Rgb565) {
    let width = 241usize;
    let height = 241usize;

    let count = width * height;

    let mut pixels = core::iter::repeat_n(color.into_storage(), count);

    self
      .display
      .set_pixels((0, 0), (240, 240), &mut pixels)
      .expect("Failed to fill LCD");
  }

  /// Turn the LCD backlight on.
  pub fn backlight_on(&mut self) {
    self.backlight.set_high();
  }

  /// Turn the LCD backlight off.
  pub fn backlight_off(&mut self) {
    self.backlight.set_low();
  }

  /// Set the LCD backlight state.
  pub fn set_backlight(&mut self, on: bool) {
    if on {
      self.backlight.set_high();
    } else {
      self.backlight.set_low();
    }
  }

  fn fill_rect(&mut self, x: u16, y: u16, width: u16, height: u16, color: Rgb565) {
    let count = width as usize * height as usize;

    let mut pixels = core::iter::repeat_n(color.into_storage(), count);

    self
      .display
      .set_pixels((x, y), (x + width - 1, y + height - 1), &mut pixels)
      .expect("Failed to draw LCD rectangle");
  }

  pub unsafe fn show_game_of_life<const GRID_WIDTH: usize, const GRID_HEIGHT: usize>(
    &mut self,
    cells: *const u32,
    draw_grid: bool,
  ) {
    assert!(GRID_WIDTH > 0);
    assert!(GRID_HEIGHT > 0);

    const SCREEN_WIDTH: usize = 240;
    const SCREEN_HEIGHT: usize = 240;

    // The display is circular, so keep the square grid
    // comfortably inside the visible area.
    const MAX_GRID_SIZE: usize = 160;

    let cell_size = core::cmp::min(MAX_GRID_SIZE / GRID_WIDTH, MAX_GRID_SIZE / GRID_HEIGHT);

    assert!(cell_size >= 2);

    let grid_width = GRID_WIDTH * cell_size;
    let grid_height = GRID_HEIGHT * cell_size;

    // Center the grid on the display.
    let offset_x = (SCREEN_WIDTH - grid_width) / 2;
    let offset_y = (SCREEN_HEIGHT - grid_height) / 2;

    // Clear the display.
    self.fill(Rgb565::BLACK);

    // The caller promises that `cells` points to at least
    // GRID_WIDTH * GRID_HEIGHT valid u32 values.
    let cells = unsafe { core::slice::from_raw_parts(cells, GRID_WIDTH * GRID_HEIGHT) };

    // ---------------------------------------------------------------------
    // Draw cell contents.
    //
    // Each cell gets a 1-pixel border, leaving the interior
    // (cell_size - 2) x (cell_size - 2) for the actual cell color.
    // ---------------------------------------------------------------------

    if cell_size > 2 {
      let inner_size = (cell_size - 2) as u16;

      for y in 0..GRID_HEIGHT {
        for x in 0..GRID_WIDTH {
          let xrgb = cells[y * GRID_WIDTH + x];

          // xRGB = 0xXXRRGGBB
          let r = ((xrgb >> 16) & 0xff) as u8;
          let g = ((xrgb >> 8) & 0xff) as u8;
          let b = (xrgb & 0xff) as u8;

          let color = Rgb565::new(r, g, b);

          let px = offset_x + x * cell_size + 1;
          let py = offset_y + y * cell_size + 1;

          self.fill_rect(px as u16, py as u16, inner_size, inner_size, color);
        }
      }
    }

    if draw_grid {
      // ---------------------------------------------------------------------
      // Draw the grid.
      //
      // Vertical lines
      // ---------------------------------------------------------------------
      for x in 0..=GRID_WIDTH {
        let px = offset_x + x * cell_size;

        self.fill_rect(
          px as u16,
          offset_y as u16,
          1,
          grid_height as u16 + 1,
          Rgb565::WHITE,
        );
      }

      // ---------------------------------------------------------------------
      // Horizontal lines
      // ---------------------------------------------------------------------

      for y in 0..=GRID_HEIGHT {
        let py = offset_y + y * cell_size;

        self.fill_rect(
          offset_x as u16,
          py as u16,
          grid_width as u16 + 1,
          1,
          Rgb565::WHITE,
        );
      }
    }
  }
}
