use std::time::{Duration, SystemTime};

use chrono::{DateTime, Timelike, Utc};

use crate::display::DisplayContent;

const TICK_TIME: Duration = Duration::from_secs(1);

pub enum ClockMode {
    Clock,
    Timer(SystemTime, Duration),
}

pub struct Clock {
    mode: ClockMode,
    last_tick: SystemTime,
}

impl Clock {
    pub fn new() -> Self {
        Self {
            mode: ClockMode::Clock,
            last_tick: SystemTime::now(),
        }
    }

    pub fn tick(&mut self) -> Option<DisplayContent> {
        let current_tick = SystemTime::now();
        if current_tick
            .duration_since(self.last_tick)
            .unwrap_or_default() // NOTE: this can lead to undefined behavior
            >= TICK_TIME
        {
            match self.mode {
                ClockMode::Clock => {
                    let now = SystemTime::now();
                    let dt_now_utc: DateTime<Utc> = now.into();

                    let digits = [
                        dt_now_utc.hour() / 10,
                        dt_now_utc.hour() % 10,
                        dt_now_utc.minute() / 10,
                        dt_now_utc.minute() % 10,
                    ];

                    Some(digits)
                }
                ClockMode::Timer(start_time, duration) => {
                    let elapsed = SystemTime::now()
                        .duration_since(start_time)
                        .unwrap_or(Duration::MAX);

                    if elapsed > duration {
                        self.mode = ClockMode::Clock;
                        return None;
                    }

                    let elapsed = (duration - elapsed).as_secs() as u32;
                    let digits = [
                        elapsed / 60 / 10,
                        elapsed / 60 % 10,
                        (elapsed % 60) / 10,
                        (elapsed % 60) % 10,
                    ];

                    Some(digits)
                }
            }
        } else {
            None
        }
    }

    pub fn set_timer(&mut self, duration: Duration) {
        self.mode = ClockMode::Timer(SystemTime::now(), duration);
    }
}
