//! This example test the RP Pico W on board LED.
//!
//! It does not work with the RP Pico board. See blinky.rs.

#![no_std]
#![no_main]

use cyw43::{Control, JoinOptions};
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_futures::select::select4;
use embassy_net::StackResources;
use embassy_rp::{bind_interrupts, Peripheral};
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer};
use static_cell::StaticCell;
use rand::RngCore;
use {defmt_rtt as _, panic_probe as _};
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDevice;




bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => InterruptHandler<PIO0>;
});

#[embassy_executor::task]
async fn cyw43_task(runner: cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

use embassy_sync::channel::Channel;
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;


static BLINK_CHANNEL : Channel<ThreadModeRawMutex, BlinkInterval, 3> = Channel::new();
static SPI_BUS: StaticCell<NoopMutex<RefCell<Spim<SPI3>>>> = StaticCell::new();

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    //let fw = include_bytes!("../../../embassy/cyw43-firmware/43439A0.bin");
    //let clm = include_bytes!("../../../embassy/cyw43-firmware/43439A0_clm.bin");

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download ../../cyw43-firmware/43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download ../../cyw43-firmware/43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(&mut pio.common, pio.sm0, pio.irq0, cs, p.PIN_24, p.PIN_29, p.DMA_CH0);

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    unwrap!(spawner.spawn(cyw43_task(runner)));

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;


    let config = embassy_net::Config::dhcpv4(Default::default());

    let mut rng = RoscRng;
    let seed = rng.next_u64();    
    
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(net_device, config, RESOURCES.init(StackResources::new()), seed);

    let spi_dev = SpiDevice::new(SPI_BUS.init(NoopMutex::new(RefCell::new(spi))));

    unwrap!(spawner.spawn(blink_task(control)));


    let mut button1 = Input::new(p.PIN_17, Pull::Up);
    let mut button2 = Input::new(p.PIN_16, Pull::Up);
    let mut button3 = Input::new(p.PIN_15, Pull::Up);
    let mut button4 = Input::new(p.PIN_14, Pull::Up);

    loop {
        select4(button1.wait_for_any_edge(), button2.wait_for_any_edge(), button3.wait_for_any_edge(), button4.wait_for_any_edge()).await;
        info!("buttons : {:?} {:?} {:?} {:?}", button1.is_high(), button2.is_high(), button3.is_high(), button4.is_high());
        match (button1.is_low(), button2.is_low(), button3.is_low(), button4.is_low()){
            (true, false, false, false) => BLINK_CHANNEL.send(BlinkInterval::Normal).await,
            (false, true, false, false) => BLINK_CHANNEL.send(BlinkInterval::Waiting).await,
            (false, false, true, false) => BLINK_CHANNEL.send(BlinkInterval::Error(3)).await,
            (false, false, false, true) => BLINK_CHANNEL.send(BlinkInterval::Error(5)).await,
            _ => {}
        }
    }
}

enum BlinkInterval{
    Normal,
    Waiting,
    Error(u8)
}

impl BlinkInterval{
    fn group_separator(&self) -> Option<Duration>{
        match self{
            BlinkInterval::Normal => None,
            BlinkInterval::Waiting => None,
            BlinkInterval::Error(_) => Some(Duration::from_millis(200))
        }
    }
    fn off_time(&self) -> Duration{
        match self{
            BlinkInterval::Normal => Duration::from_millis(500),
            BlinkInterval::Waiting => Duration::from_millis(250),
            BlinkInterval::Error(_) => Duration::from_millis(100)
        }
    }
    fn on_time(&self) -> Duration{
        match self{
            BlinkInterval::Normal => Duration::from_millis(500),
            BlinkInterval::Waiting => Duration::from_millis(250),
            BlinkInterval::Error(_) => Duration::from_millis(100)
        }
    }
    fn group_size(&self) -> u8{
        match self{
            BlinkInterval::Normal => 1,
            BlinkInterval::Waiting => 1,
            BlinkInterval::Error(x) => x.clone()
        }
    }
}


#[embassy_executor::task]
async fn blink_task(mut control: Control<'static>) -> ! {
    let mut interval = BlinkInterval::Normal;
    loop{
        if let Ok(new_interval) = BLINK_CHANNEL.try_receive(){
            interval = new_interval;
        }
        for _ in 0..interval.group_size(){
            control.gpio_set(0, true).await;
            Timer::after(interval.on_time()).await;
            control.gpio_set(0, false).await;
            Timer::after(interval.off_time()).await;
        }
        if let Some(group_separator) = interval.group_separator(){
            Timer::after(group_separator).await;
        }
    }
}
