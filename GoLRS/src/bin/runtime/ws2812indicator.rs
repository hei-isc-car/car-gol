use esp_hal::{Blocking, gpio::AnyPin, rmt::ChannelCreator};
use esp_hal_smartled::{RmtSmartLeds, Timing, buffer_size, color_order};
use smart_leds::{RGB8, SmartLedsWrite};

const BUFFER: usize = buffer_size::<RGB8>(1);
// Predefined fallback color (Green/Blue hue) used by the toggle method
const PREDEFINED_COLOR: RGB8 = RGB8 { r: 10, g: 0, b: 10 };

pub struct Ws2812Indicator {
  // 1. Buffer Size | 2. Mode (Blocking) | 3. Hardware Channel Type | 4. Color Order
  led_strip: RmtSmartLeds<'static, BUFFER, Blocking, RGB8, color_order::Grb>,
  pub active: bool,
}

// TODO: esp-hal-smartled2 0.29.0 introduced a *2 factor in pulses (why ? dunno) but did not fix timings ...
// May need to delete those and import the timings from crate once fixed if version changes
pub const WS2812B_TIMING: Timing = Timing {
  time_0_high: 400 / 2,
  time_0_low: 800 / 2,
  time_1_high: 850 / 2,
  time_1_low: 450 / 2,
  reset: 50_000 / 2,
};

impl Ws2812Indicator {
  /// Initialise la LED en lui passant le canal RMT et la broche GPIO
  pub fn new(pin: AnyPin<'static>, rmt_channel: ChannelCreator<'static, Blocking, 0>) -> Self {
    // Appended .unwrap() to safely extract the RmtSmartLeds driver from the Result enum
    let led_strip =
      RmtSmartLeds::<BUFFER, _, RGB8, color_order::Grb>::new(WS2812B_TIMING, rmt_channel, pin)
        .unwrap();

    Self {
      led_strip,
      active: false,
    }
  }

  /// Alterne entre la couleur configurée et l'état éteint
  pub fn toggle(&mut self) {
    if self.active {
      self.turn_off();
    } else {
      self.turn_on();
    }
  }

  pub fn turn_on(&mut self) {
    self.active = true;
    self
      .led_strip
      .write([PREDEFINED_COLOR].iter().cloned())
      .unwrap();
  }

  pub fn turn_off(&mut self) {
    self.active = false;
    self
      .led_strip
      .write([RGB8::new(0, 0, 0)].iter().cloned())
      .unwrap();
  }
}
