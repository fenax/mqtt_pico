//! This example test the RP Pico W on board LED.
//!
//! It does not work with the RP Pico board. See blinky.rs.

#![no_std]
#![no_main]

use core::str::FromStr;
use core::{ops, u16};

use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp as rp;
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use heapless::String;
use itertools::Itertools;
use rand::RngCore;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

use mqtt_pico::output::leds::*;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

static LED_COUNT: usize = 2;

static LED_SIGNALS: [Signal<ThreadModeRawMutex, LedStatus>; LED_COUNT] =
    [const { Signal::new() }; LED_COUNT];

#[embassy_executor::task(pool_size = 2)]
async fn led_task(
    mut led: RgbLed<'static>,
    signal: &'static Signal<ThreadModeRawMutex, LedStatus>,
) -> ! {
    led.task(signal).await
}

#[embassy_executor::task(pool_size = 2)]
async fn pwm_led_task(
    mut led: PwmRgbLed<'static>,
    signal: &'static Signal<ThreadModeRawMutex, LedStatus>,
) -> ! {
    led.task(signal).await
}

struct ChannelMessage {
    topic: String<12>,
    id: String<8>,
    data: String<8>,
    payload: String<32>,
}
static INPUT_CHANNEL: Channel<ThreadModeRawMutex, ChannelMessage, 6> = Channel::new();

#[embassy_executor::task]
async fn message_parser() -> ! {
    let mut leds = [LedStatus {
        anim: Anim::None,
        color: None,
        power: None,
    }; LED_COUNT];
    loop {
        let ChannelMessage {
            topic,
            id,
            data,
            payload,
        } = INPUT_CHANNEL.receive().await;
        let id: usize = if let Ok(v) = id.parse() {
            v
        } else {
            warn!("invalid id : {}", id);
            continue;
        };
        match topic.as_str() {
            "led" => {
                let led = if id == 0 || id > LED_SIGNALS.len() {
                    continue;
                } else {
                    &mut leds[id - 1]
                };
                match data.as_str() {
                    "color" => {
                        let len = payload.chars().count();
                        if len == 0 {
                            led.color = None;
                        } else if len == 1 {
                            match payload.chars().nth(0) {
                                None => core::unreachable!("length has been checked to be 1"),
                                Some('R') | Some('r') => led.color = Some(Color::red()),
                                Some('G') | Some('g') => led.color = Some(Color::green()),
                                Some('B') | Some('b') => led.color = Some(Color::blue()),
                                Some('0') | Some('O') | Some('o') => led.color = Some(Color::off()),
                                Some(x) => warn!("unknown color {}", x),
                            }
                        } else if len == 4 && payload.starts_with("#") {
                            let (r, g, b) = payload
                                .chars()
                                .skip(1)
                                .map(|s| s.to_digit(16).unwrap_or_default() << 4)
                                .collect_tuple()
                                .unwrap();
                            led.color = Some(Color::new(r as u8, g as u8, b as u8));
                        } else {
                            warn!("can not understand message : {}", payload);
                            continue;
                        }
                    }
                    "power" => {
                        let len = payload.chars().count();
                        if len == 0 {
                            led.power = None;
                        } else {
                            led.power = payload.parse().ok();
                        }
                    }
                    "anim" => {}
                    _ => {
                        warn!("invalid data source : {}", data);
                        continue;
                    }
                }
                LED_SIGNALS[id - 1].signal(led.clone());
            }
            _ => warn!("topic unknown : {}", topic),
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    //let fw = include_bytes!("../../../embassy/cyw43-firmware/43439A0.bin");
    //let clm = include_bytes!("../../../embassy/cyw43-firmware/43439A0_clm.bin");
    //let wifi_password = core::env!("WIFI_PASSWORD", "No wifi password set");
    //let wifi_ssid = core::env!("WIFI_SSID", "No wifi SSID set");

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download ../../cyw43-firmware/43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download ../../cyw43-firmware/43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    let chip_id_num = embassy_rp::pac::SYSINFO.chip_id().read();

    info!("CHIP ID IS : {:x}", chip_id_num.0);

    let mut chip_id: heapless::String<12> = heapless::String::new();
    core::fmt::write(&mut chip_id, format_args!("ksl-{:X}", chip_id_num.0)).unwrap();

    let mut led1 = RgbLed {
        red: Output::new(p.PIN_13, Level::High),
        green: Output::new(p.PIN_12, Level::High),
        blue: Output::new(p.PIN_11, Level::High),
    };

    /*let mut led2 = RgbLed{
        red: Output::new(p.PIN_10, Level::High),
        green: Output::new(p.PIN_9, Level::High),
        blue: Output::new(p.PIN_8, Level::High)
    };*/
    // Slice    0     1     2     3     4     5     6     7
    // channel  A  B  A  B  A  B  A  B  A  B  A  B  A  B  A  B
    // pin      00 01 02 03 04 05 06 07 08 09 10 11 12 13 14 15
    // pin      16 17 18 19 20 21 22 23 24 25 26 27 28 29

    let mut c = rp::pwm::Config::default();
    c.top = 32_768;
    c.compare_b = 8;
    c.invert_a = true;
    c.invert_b = true;

    let (red, _) = rp::pwm::Pwm::new_output_a(p.PWM_SLICE5, p.PIN_10, c.clone()).split();
    let (blue, green) = rp::pwm::Pwm::new_output_ab(p.PWM_SLICE4, p.PIN_8, p.PIN_9, c).split();

    let mut led2 = PwmRgbLed {
        red: red.unwrap(),
        green: green.unwrap(),
        blue: blue.unwrap(),
    };

    unwrap!(spawner.spawn(led_task(led1, &LED_SIGNALS[0])));
    unwrap!(spawner.spawn(pwm_led_task(led2, &LED_SIGNALS[1])));

    unwrap!(spawner.spawn(message_parser()));

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let mut rng = RoscRng;
    let seed = rng.next_u64();

    INPUT_CHANNEL
        .send(ChannelMessage {
            topic: String::from_str("led").unwrap(),
            id: String::from_str("1").unwrap(),
            data: String::from_str("power").unwrap(),
            payload: String::from_str("10").unwrap(),
        })
        .await;

    let delay = Duration::from_secs(1);
    loop {
        info!("led on!");
        control.gpio_set(0, true).await;
        Timer::after(delay).await;

        info!("led off!");
        control.gpio_set(0, false).await;
        Timer::after(delay).await;
        INPUT_CHANNEL
            .send(ChannelMessage {
                topic: String::from_str("led").unwrap(),
                id: String::from_str("1").unwrap(),
                data: String::from_str("color").unwrap(),
                payload: String::from_str("r").unwrap(),
            })
            .await;
        INPUT_CHANNEL
            .send(ChannelMessage {
                topic: String::from_str("led").unwrap(),
                id: String::from_str("2").unwrap(),
                data: String::from_str("power").unwrap(),
                payload: String::from_str("12").unwrap(),
            })
            .await;

        Timer::after(delay).await;
        INPUT_CHANNEL
            .send(ChannelMessage {
                topic: String::from_str("led").unwrap(),
                id: String::from_str("1").unwrap(),
                data: String::from_str("color").unwrap(),
                payload: String::from_str("g").unwrap(),
            })
            .await;
        INPUT_CHANNEL
            .send(ChannelMessage {
                topic: String::from_str("led").unwrap(),
                id: String::from_str("2").unwrap(),
                data: String::from_str("power").unwrap(),
                payload: String::from_str("64").unwrap(),
            })
            .await;
        Timer::after(delay).await;
        INPUT_CHANNEL
            .send(ChannelMessage {
                topic: String::from_str("led").unwrap(),
                id: String::from_str("1").unwrap(),
                data: String::from_str("color").unwrap(),
                payload: String::from_str("b").unwrap(),
            })
            .await;
        INPUT_CHANNEL
            .send(ChannelMessage {
                topic: String::from_str("led").unwrap(),
                id: String::from_str("2").unwrap(),
                data: String::from_str("power").unwrap(),
                payload: String::from_str("142").unwrap(),
            })
            .await;
    }
}
