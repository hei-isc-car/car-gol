use esp_hal::time::{Duration, Instant};

const BUTTON_DEBOUNCE_MS: u64 = 50;

pub struct DebouncedButton {
  stable_pressed: bool,
  last_raw_pressed: bool,
  last_change_at: Instant,
  press_consumed: bool,
}

impl DebouncedButton {
  pub fn new(now: Instant) -> Self {
    Self {
      stable_pressed: false,
      last_raw_pressed: false,
      last_change_at: now,
      press_consumed: false,
    }
  }

  pub fn update(&mut self, now: Instant, raw_pressed: bool) -> bool {
    if raw_pressed != self.last_raw_pressed {
      self.last_raw_pressed = raw_pressed;
      self.last_change_at = now;
    }

    if self.last_change_at.elapsed() < Duration::from_millis(BUTTON_DEBOUNCE_MS) {
      return false;
    }

    if raw_pressed != self.stable_pressed {
      self.stable_pressed = raw_pressed;

      if self.stable_pressed {
        if !self.press_consumed {
          self.press_consumed = true;
          return true;
        }
      } else {
        self.press_consumed = false;
      }
    }

    false
  }
}
