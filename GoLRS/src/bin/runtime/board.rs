use crate::buttons::DebouncedButton;
use esp_hal::gpio::{Input, Output, Pull};
use esp_hal::time::Instant;

#[cfg(feature = "board-lcdkit")]
mod ws2812indicator;
#[cfg(feature = "board-lcdkit")]
use crate::lcd::LcdDisplay;
#[cfg(feature = "board-lcdkit")]
use esp_hal::{Blocking, gpio::AnyPin, rmt::ChannelCreator};
#[cfg(feature = "board-lcdkit")]
use ws2812indicator::Ws2812Indicator;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoardProfile {
  pub name: &'static str,
  pub button_pull: Pull,
  pub step_decrease_button_gpio: u8,
  pub step_increase_button_gpio: u8,
  pub status_led_gpio: u8,
  pub fixed_pins: &'static [&'static str],
  pub wired_peripherals: &'static [&'static str],
}

pub struct BoardIo {
  #[cfg(feature = "board-devkit-rust-2")]
  inner: DevKitIo,
  #[cfg(feature = "board-lcdkit")]
  inner: LcdKitIo,
}

#[cfg(all(feature = "board-devkit-rust-2", feature = "board-lcdkit"))]
compile_error!("Enable only one GoLRS board profile feature at a time.");

#[cfg(not(any(feature = "board-devkit-rust-2", feature = "board-lcdkit")))]
compile_error!("Enable one GoLRS board profile feature.");

#[cfg(feature = "board-devkit-rust-2")]
pub const BOARD: BoardProfile = BoardProfile {
  name: "ESP32-C3-DevKit-RUST-2",
  button_pull: Pull::Down,
  step_decrease_button_gpio: 2,
  step_increase_button_gpio: 0,
  status_led_gpio: 7,
  fixed_pins: &[
    "GPIO2: external step control input / board RGB LED on the devkit",
    "GPIO7: user LED",
    "GPIO8: I2C SCL for the IMU and temperature/humidity sensor",
    "GPIO9: BOOT button",
    "GPIO10: I2C SDA for the IMU and temperature/humidity sensor",
    "GPIO18: USB D-",
    "GPIO19: USB D+",
    "GPIO20: UART0 TX",
    "GPIO21: UART0 RX",
  ],
  wired_peripherals: &[
    "ICM-42670-P IMU on I2C",
    "SHTC3 temperature/humidity sensor on I2C",
    "WS2812 RGB LED on GPIO2",
    "User LED on GPIO7",
    "Boot button on GPIO9",
    "USB Serial/JTAG over USB-C",
  ],
};

#[cfg(feature = "board-lcdkit")]
pub const BOARD: BoardProfile = BoardProfile {
  name: "ESP32-C3-LCDkit",
  button_pull: Pull::Down,
  step_decrease_button_gpio: 9,
  step_increase_button_gpio: 10,
  status_led_gpio: 8,
  fixed_pins: &[
    "GPIO0:  LCD_SDA",
    "GPIO1:  LCD_SCL",
    "GPIO2:  LCD_D/C",
    "GPIO3:  AUDIO_PA",
    "GPIO4:  IR_RX/IR_TX",
    "GPIO5:  LCD_BL_CTRL",
    "GPIO6:  ENCODER_B",
    "GPIO7:  LCD_CS",
    "GPIO8:  RGB LED",
    "GPIO9:  ENCODER_SW",
    "GPIO10: ENCODER_A",
    "GPIO18: USB D-",
    "GPIO19: USB D+",
  ],
  wired_peripherals: &[
    "1.28 inch GC9A01 LCD subboard",
    "Rotary encoder switch on GPIO9",
    "Rotary encoder A on GPIO10",
    "Rotary encoder B on GPIO6",
    "RGB LED on GPIO8",
    "LCD backlight control on GPIO5",
    "Audio amplifier on GPIO3",
    "Infrared TX/RX on GPIO4",
    "USB Serial/JTAG over USB-C",
  ],
};

#[cfg(feature = "board-devkit-rust-2")]
struct DevKitIo {
  step_decrease_input: Input<'static>,
  step_decrease_state: DebouncedButton,
  step_increase_input: Input<'static>,
  step_increase_state: DebouncedButton,
  activity_led: Output<'static>,
  pending_decrease_steps: u8,
  pending_increase_steps: u8,
}

#[cfg(feature = "board-lcdkit")]
struct LcdKitIo {
  encoder_a: Input<'static>,
  encoder_b: Input<'static>,
  encoder_press: Input<'static>,
  encoder_press_state: DebouncedButton,

  activity_led: Ws2812Indicator,

  lcd: LcdDisplay,

  last_encoder_state: u8,
  encoder_step_accumulator: i8,
  pending_decrease_steps: u8,
  pending_increase_steps: u8,
  pending_step_once: bool,
}

impl BoardIo {
  #[cfg(feature = "board-devkit-rust-2")]
  pub fn new(
    step_decrease_input: Input<'static>,
    step_increase_input: Input<'static>,
    activity_led: Output<'static>,
  ) -> Self {
    Self {
      inner: DevKitIo {
        step_decrease_input,
        step_decrease_state: DebouncedButton::new(Instant::now()),
        step_increase_input,
        step_increase_state: DebouncedButton::new(Instant::now()),
        activity_led,
        pending_decrease_steps: 0,
        pending_increase_steps: 0,
      },
    }
  }

  #[cfg(feature = "board-lcdkit")]
  #[allow(clippy::too_many_arguments)]
  pub fn new(
    encoder_a: Input<'static>,
    encoder_b: Input<'static>,
    encoder_press: Input<'static>,
    activity_led: AnyPin<'static>,
    rmt_channel: ChannelCreator<'static, Blocking, 0>,
    lcd_spi: esp_hal::spi::master::Spi<'static, esp_hal::Blocking>,
    lcd_dc: Output<'static>,
    lcd_cs: Output<'static>,
    lcd_backlight: Output<'static>,
  ) -> Self {
    Self {
      inner: LcdKitIo {
        encoder_a,
        encoder_b,
        encoder_press,

        encoder_press_state: DebouncedButton::new(Instant::now()),

        activity_led: Ws2812Indicator::new(activity_led, rmt_channel),

        lcd: LcdDisplay::new(lcd_spi, lcd_dc, lcd_cs, lcd_backlight),

        last_encoder_state: 0,
        encoder_step_accumulator: 0,
        pending_decrease_steps: 0,
        pending_increase_steps: 0,
        pending_step_once: false,
      },
    }
  }

  pub fn poll(&mut self, now: Instant) {
    #[cfg(feature = "board-devkit-rust-2")]
    {
      let inner = &mut self.inner;
      if inner
        .step_decrease_state
        .update(now, inner.step_decrease_input.is_high())
      {
        inner.pending_decrease_steps = inner.pending_decrease_steps.saturating_add(1);
      }
      if inner
        .step_increase_state
        .update(now, inner.step_increase_input.is_high())
      {
        inner.pending_increase_steps = inner.pending_increase_steps.saturating_add(1);
      }
    }

    #[cfg(feature = "board-lcdkit")]
    {
      let inner = &mut self.inner;
      let encoder_state =
        ((inner.encoder_a.is_high() as u8) << 1) | (inner.encoder_b.is_high() as u8);

      let delta = -encoder_delta(inner.last_encoder_state, encoder_state);
      inner.last_encoder_state = encoder_state;

      inner.encoder_step_accumulator += delta;
      // QEI interface but one click on encoder does 2 steps
      if inner.encoder_step_accumulator >= 2 {
        inner.pending_increase_steps = inner.pending_increase_steps.saturating_add(1);
        inner.encoder_step_accumulator = 0;
      } else if inner.encoder_step_accumulator <= -2 {
        inner.pending_decrease_steps = inner.pending_decrease_steps.saturating_add(1);
        inner.encoder_step_accumulator = 0;
      }

      if inner
        .encoder_press_state
        .update(now, inner.encoder_press.is_high())
      {
        inner.pending_step_once = true;
      }
    }
  }

  pub fn take_step_down(&mut self) -> bool {
    #[cfg(feature = "board-devkit-rust-2")]
    {
      let inner = &mut self.inner;
      if inner.pending_decrease_steps > 0 {
        inner.pending_decrease_steps -= 1;
        return true;
      }
      return false;
    }

    #[cfg(feature = "board-lcdkit")]
    {
      let inner = &mut self.inner;
      if inner.pending_decrease_steps > 0 {
        inner.pending_decrease_steps -= 1;
        return true;
      }
      false
    }
  }

  pub fn take_step_up(&mut self) -> bool {
    #[cfg(feature = "board-devkit-rust-2")]
    {
      let inner = &mut self.inner;
      if inner.pending_increase_steps > 0 {
        inner.pending_increase_steps -= 1;
        return true;
      }
      return false;
    }

    #[cfg(feature = "board-lcdkit")]
    {
      let inner = &mut self.inner;
      if inner.pending_increase_steps > 0 {
        inner.pending_increase_steps -= 1;
        return true;
      }
      false
    }
  }

  pub fn take_step_once(&mut self) -> bool {
    #[cfg(feature = "board-devkit-rust-2")]
    {
      false
    }

    #[cfg(feature = "board-lcdkit")]
    {
      let inner = &mut self.inner;
      let pending = inner.pending_step_once;
      inner.pending_step_once = false;
      pending
    }
  }

  pub fn pulse_activity_led(&mut self) {
    self.inner.activity_led.toggle();
  }

  pub fn set_activity_led(&mut self, on: bool) {
    #[cfg(feature = "board-devkit-rust-2")]
    {
      if on {
        self.inner.activity_led.set_high();
      } else {
        self.inner.activity_led.set_low();
      }
    }

    #[cfg(feature = "board-lcdkit")]
    {
      if on {
        self.inner.activity_led.turn_on();
      } else {
        self.inner.activity_led.turn_off();
      }
    }
  }

  pub fn wired_peripherals(&self) -> &'static [&'static str] {
    BOARD.wired_peripherals
  }

  #[cfg(feature = "board-lcdkit")]
  pub fn lcd(&mut self) -> &mut LcdDisplay {
    &mut self.inner.lcd
  }
}

#[cfg(feature = "board-lcdkit")]
fn encoder_delta(previous: u8, current: u8) -> i8 {
  const TABLE: [[i8; 4]; 4] = [[0, -1, 1, 0], [1, 0, 0, -1], [-1, 0, 0, 1], [0, 1, -1, 0]];

  TABLE[previous as usize][current as usize]
}
