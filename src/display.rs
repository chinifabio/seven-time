use esp_idf_hal::ledc::LedcDriver;

const MIN_ANGLE: u32 = 0;
const MAX_ANGLE: u32 = 180;
const MAX_DIGIT: usize = 9;

pub struct Digit<'a> {
    driver: LedcDriver<'a>,
    min: u32,
    max: u32,
}

impl<'a> Digit<'a> {
    fn map(x: u32, in_min: u32, in_max: u32, out_min: u32, out_max: u32) -> u32 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }

    pub fn new(driver: LedcDriver<'a>) -> Self {
        let max_duty = driver.get_max_duty();
        let min = max_duty * 5 / 100;
        let max = max_duty * 10 / 100;
        Self { driver, min, max }
    }

    pub fn set_digit(&mut self, digit: u32) {
        if digit > MAX_DIGIT as u32 {
            log::error!("Digit out of range: {}", digit);
            return;
        }

        // 10 digits, 0-9
        let angle = MIN_ANGLE + ((MAX_ANGLE - MIN_ANGLE) / (MAX_DIGIT + 1) as u32) * digit;
        let duty = Self::map(angle, MIN_ANGLE, MAX_ANGLE, self.min, self.max);
        self.driver.set_duty(duty).unwrap();
    }
}

pub type Servos<'a> = [Digit<'a>; 4];

pub type DisplayContent = [u32; 4];

pub struct Display<'a> {
    pub servos: Servos<'a>,
}

impl Display<'_> {
    pub fn write(&mut self, digits: DisplayContent) {
        log::info!("Displaing {digits:?}");
        for (digit, servo) in digits.into_iter().zip(self.servos.iter_mut()) {
            servo.set_digit(digit);
        }
    }
}
