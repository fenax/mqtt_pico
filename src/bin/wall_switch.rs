#![no_std]
#![no_main]

use cyw43::{JoinOptions, ScanOptions};
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::{TcpSocket};
use embassy_net::{IpAddress, IpEndpoint, StackResources};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::RoscRng;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIO0};
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_time::{Duration, Timer};
use heapless::{String, Vec};
use rust_mqtt::client::client::MqttClient;
use rust_mqtt::client::client_config::ClientConfig;
use rust_mqtt::utils::rng_generator::CountingRng;
use static_cell::StaticCell;
use rand::RngCore;
use {defmt_rtt as _, panic_probe as _};


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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // Wifi chip firmware

    //let fw = include_bytes!("../../../embassy/cyw43-firmware/43439A0.bin");
    //let clm = include_bytes!("../../../embassy/cyw43-firmware/43439A0_clm.bin");

    // To make flashing faster for development, you may want to flash the firmwares independently
    // at hardcoded addresses, instead of baking them into the program with `include_bytes!`:
    //     probe-rs download ../../cyw43-firmware/43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
    //     probe-rs download ../../cyw43-firmware/43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
    let fw = unsafe { core::slice::from_raw_parts(0x10100000 as *const u8, 230321) };
    let clm = unsafe { core::slice::from_raw_parts(0x10140000 as *const u8, 4752) };
    
    // Secrets
    let wifi_password = core::env!("WIFI_PASSWORD", "No wifi password set");
    let wifi_ssid = core::env!("WIFI_SSID", "No wifi SSID set");

    // Constants
    let chip_id:heapless::String<12> = {
        let chip_id_num = embassy_rp::pac::SYSINFO.chip_id().read();
        info!("CHIP ID IS : {:x}", chip_id_num.0);
        let mut chip_id: heapless::String<12> = heapless::String::new();
        core::fmt::write(&mut chip_id, format_args!("ksl-{:X}", chip_id_num.0));
        chip_id
    };
    let prefix = "embedded";


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

    let mut scanner = 
    control.scan(ScanOptions::default()).await;
    while let Some(item) = scanner.next().await {
        info!("AP: {}", core::str::from_utf8(&item.ssid[0..item.ssid_len as usize]).unwrap());
    }

    drop(scanner);
    //spawner.spawn(wifi_task(scanner));

    let config = embassy_net::Config::dhcpv4(Default::default());

    let mut rng = RoscRng;
    let seed = rng.next_u64();    
    
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(net_device, config, RESOURCES.init(StackResources::new()), seed);

    unwrap!(spawner.spawn(net_task(runner)));


    control.join(&wifi_ssid, JoinOptions::new(wifi_password)).await.unwrap();



    info!("waiting for DHCP...");
    while !stack.is_config_up() {
        Timer::after_millis(100).await;
    }
    info!("DHCP is now up!");


    
    let delay = Duration::from_secs(1);
    loop {
      
        let mut tcp_rx_buffer = [0; 1500];
        let mut tcp_tx_buffer = [0; 1500];
    
        let mut socket = TcpSocket::new(stack, &mut tcp_rx_buffer, &mut tcp_tx_buffer);
        socket.connect(IpEndpoint::new(IpAddress::v4(192, 168, 103, 2), 1883)).await.unwrap();

        let mut config = ClientConfig::new(
            rust_mqtt::client::client_config::MqttVersion::MQTTv5,
            CountingRng(20000),
        );

        config.add_max_subscribe_qos(rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS1);
        config.add_client_id(&chip_id);

        config.max_packet_size = 100;
        let mut recv_buffer = [0; 80];
        let mut write_buffer = [0; 80];
        let mut client = MqttClient::<_, 5, _>::new(
            socket,
            &mut write_buffer,
            80,
            &mut recv_buffer,
            80,
            config,
        );

        client.connect_to_broker().await.unwrap();
        let mut buff :String<32> = String::new();
        core::fmt::write(&mut buff, format_args!("{prefix}/{chip_id}/#")).expect("could not write topic, maybe prefix too long");
        client.subscribe_to_topic(&buff).await.unwrap();
        buff.clear();
        core::fmt::write(&mut buff, format_args!("{prefix}/+")).expect("prefix too long");
        client.subscribe_to_topic(&buff).await.unwrap();
        loop {
            let (topic, body) = client.receive_message().await.unwrap();
            let mut parts = topic.split('/');
            core::assert_eq!(Some(prefix),parts.next());
            match parts.next(){
                Some("time") => {}
                Some("local_time") => {}
                Some(x) if x == chip_id => {
                    match parts.next(){
                        Some("led") => 
                    }
                }
                Some(y) => {
                    debug!("unknown {}", y)
                }
                None => {
                    warn!("no second arg")
                }
            }
            client
                .send_message(
                    "hello",
                    b"hello2",
                    rust_mqtt::packet::v5::publish_packet::QualityOfService::QoS0,
                    true,
                )
                .await
                .unwrap();
            Timer::after(Duration::from_secs(500)).await;
        }
    }
}
