#![feature(type_alias_impl_trait)]
#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
mod ydlidar_gs2;

use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::prelude::*;
use esp_idf_svc::hal::uart::*;
use esp_idf_svc::hal::{gpio, uart};
use esp_idf_sys as _;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    esp_idf_svc::sys::link_patches();

    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Hello, world!");
    let peripherals = Peripherals::take().unwrap();

    let config = config::Config::default()
        .baudrate(921600.into())
        .parity_none()
        .stop_bits(config::StopBits::STOP1);
    let uart = UartDriver::new(
        peripherals.uart1,
        peripherals.pins.gpio4, // TX
        peripherals.pins.gpio5, // RX
        Option::<gpio::AnyIOPin>::None,
        Option::<gpio::AnyIOPin>::None,
        &config,
    )
    .unwrap();

    let mut lidar = ydlidar_gs2::YDlidar::new(uart).unwrap();

    // Exemple : envoyer une commande "scan"
    lidar.send_command(0x60, &[]).unwrap();
    let mut data: [u8; 256] = [0; 256];

    match lidar.read_response(&mut data) {
        Ok(()) => {
            //            let mess: String = data.iter().map(|&b| b as char).collect();
            //          log::info!("response : {mess}");
        }
        Err(e) => {
            log::info!("Erreur lecture lidar: {:?}", e);
        }
    }
}

/*
 * use esp_idf_svc; // initialise ESP-IDF

fn main() {
    println!("Hello from ESP32-S3 with Rust!");
}
*/
