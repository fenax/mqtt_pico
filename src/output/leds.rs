use embassy_rp::{
    gpio::{Level, Output},
    pwm::{PwmOutput, SetDutyCycle},
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, signal::Signal};

#[derive(Clone, Copy)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl core::ops::Mul<u8> for Color {
    type Output = Color;

    fn mul(self, rhs: u8) -> Self::Output {
        Color {
            red: ((self.red as u32 * rhs as u32) / 256) as u8,
            green: ((self.green as u32 * rhs as u32) / 256) as u8,
            blue: ((self.blue as u32 * rhs as u32) / 256) as u8,
        }
    }
}

impl Color {
    pub fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
    pub fn red() -> Self {
        Self::new(255, 0, 0)
    }
    pub fn green() -> Self {
        Self::new(0, 255, 0)
    }
    pub fn blue() -> Self {
        Self::new(0, 0, 255)
    }
    pub fn off() -> Self {
        Self::new(0, 0, 0)
    }
}

#[derive(Clone, Copy)]
pub enum Anim {
    None,
    Blink,
    Pulse,
}

#[derive(Clone, Copy)]
pub struct LedStatus {
    pub color: Option<Color>,
    pub power: Option<u8>,
    pub anim: Anim,
}

pub struct PwmRgbLed<'a> {
    pub red: PwmOutput<'a>,
    pub green: PwmOutput<'a>,
    pub blue: PwmOutput<'a>,
}
impl PwmRgbLed<'_> {
    pub fn set_color(&mut self, Color { red, green, blue }: Color) {
        self.set(red, green, blue);
    }
    pub fn set(&mut self, red: u8, green: u8, blue: u8) {
        self.red
            .set_duty_cycle_fraction(red as u16 * red as u16, u16::MAX)
            .unwrap();
        self.green
            .set_duty_cycle_fraction(green as u16 * green as u16, u16::MAX)
            .unwrap();
        self.blue
            .set_duty_cycle_fraction(blue as u16 * blue as u16, u16::MAX)
            .unwrap();
    }
    pub async fn task(&mut self, signal: &'static Signal<ThreadModeRawMutex, LedStatus>) -> ! {
        loop {
            let message = signal.wait().await;
            let color = match (message.color, message.power) {
                (Some(color), Some(power)) => color * power,
                (Some(color), None) => color,
                (None, Some(power)) => Color::new(power, power, power),
                (None, None) => Color::off(),
            };
            self.set_color(color);
        }
    }
}
pub struct RgbLed<'a> {
    pub red: Output<'a>,
    pub green: Output<'a>,
    pub blue: Output<'a>,
}

impl RgbLed<'_> {
    pub fn binary_to_level(value: bool) -> Level {
        match value {
            true => Level::Low,
            false => Level::High,
        }
    }
    pub fn set_color(&mut self, Color { red, green, blue }: Color) {
        self.set(red > 0, green > 0, blue > 0);
    }
    pub fn set(&mut self, red: bool, green: bool, blue: bool) {
        self.red.set_level(Self::binary_to_level(red));
        self.green.set_level(Self::binary_to_level(green));
        self.blue.set_level(Self::binary_to_level(blue));
    }
    pub async fn task(&mut self, signal: &'static Signal<ThreadModeRawMutex, LedStatus>) -> ! {
        loop {
            let message = signal.wait().await;
            let color = match (message.color, message.power) {
                (Some(color), Some(power)) => color * power,
                (Some(color), None) => color,
                (None, Some(power)) => Color::new(power, power, power),
                (None, None) => Color::off(),
            };
            self.set_color(color);
        }
    }
}
