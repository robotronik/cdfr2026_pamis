use anyhow::Result;
use esp_idf_hal::ledc::config::TimerConfig;
use esp_idf_hal::ledc::{LedcDriver, LedcTimerDriver};
use heapless::String;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::ipv4;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

use esp_idf_svc::hal::ledc;

use embedded_svc::http::server::Method;

use log::*;

use std::sync::{Arc, Mutex};

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = <EspNvsPartition<NvsDefault>>::take().unwrap();

    let timer_driver = LedcTimerDriver::new(
        peripherals.ledc.timer0,
        &TimerConfig::default()
            .frequency(50.into())
            .resolution(ledc::Resolution::Bits14),
    )?;
    let ledc = LedcDriver::new(
        peripherals.ledc.channel0,
        timer_driver,
        peripherals.pins.gpio3,
    )?;
    let ledc_driver = Arc::new(Mutex::new(ledc));

    let mut ledc = ledc_driver.lock().unwrap();

    info!("max duty is : {}", ledc.get_max_duty());

    let max_duty_cycle = ledc.get_max_duty() as u32;
    let min_duty = (25 * max_duty_cycle) / 1000;
    let max_duty = (125 * max_duty_cycle) / 1000;
    let duty_gap = max_duty - min_duty;

    fn duty_from_angle(deg: u32, min_duty: u32, duty_gap: u32) -> u32 {
        let duty = min_duty + ((deg * duty_gap) / 180);
        duty as u32
    }
    ledc.set_duty(duty_from_angle(180, min_duty, duty_gap))
        .unwrap();
    FreeRtos::delay_ms(250);
    ledc.set_duty(duty_from_angle(90, min_duty, duty_gap))
        .unwrap();
    FreeRtos::delay_ms(250);
    ledc.set_duty(duty_from_angle(0, min_duty, duty_gap))
        .unwrap();
    drop(ledc);
    const SSID: &str = env!("SSID");
    const PASSWORD: &str = env!("PASSWORD");

    log::info!("Connecting to Wi-Fi network '{SSID}'...");

    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sys_loop.clone(), Some(nvs)).unwrap(),
        sys_loop,
    )
    .unwrap();

    let ssid: String<32> = String::try_from(SSID).unwrap();
    let password: String<64> = String::try_from(PASSWORD).unwrap();
    let wifi_configuration =
        esp_idf_svc::wifi::Configuration::Client(esp_idf_svc::wifi::ClientConfiguration {
            ssid: ssid,
            password: password,
            ..Default::default()
        });

    wifi.set_configuration(&wifi_configuration).unwrap();
    wifi.start().unwrap();
    wifi.connect().unwrap();
    wifi.wait_netif_up().unwrap();

    let ip_info: ipv4::IpInfo = wifi.wifi().sta_netif().get_ip_info().unwrap();
    log::info!("Wi-Fi connected! IP address: {}", ip_info.ip);

    let mut server = EspHttpServer::new(&Default::default()).unwrap();

    server
        .fn_handler("/", Method::Get, |request| -> Result<(), EspIOError> {
            log::info!("Received request");
            let mut response = request.into_ok_response()?;
            response.write(index_html().as_bytes())?;
            Ok(())
        })
        .unwrap();

    log::info!(
        "Server is running. Open http://{} in your browser",
        ip_info.ip
    );

    let ledc_clone = ledc_driver.clone();
    server.fn_handler(
        "/servo",
        Method::Get,
        move |request| -> Result<(), EspIOError> {
            let uri = request.uri(); // "/servo?angle=90"

            // Extraire la query string après '?'
            let angle = uri
                .split('?')
                .nth(1) // prend tout après '?'
                .and_then(|q| q.strip_prefix("angle=")) // enlever "angle="
                .and_then(|s| s.parse::<u8>().ok()); // convertir en u8

            if let Some(angle) = angle {
                // Convertir angle en duty pour le servo (5% -> 10%)
                let duty = duty_from_angle(angle as u32, min_duty, duty_gap);
                log::info!("Servo angle set to {}", angle);
                ledc_clone.lock().unwrap().set_duty(duty).unwrap();
            }
            let mut response = request.into_ok_response()?;
            response.write(b"OK")?;
            Ok(())
        },
    )?;

    loop {
        /*
        for i in (0..180).step_by(5) {
            ledc_driver
                .lock()
                .unwrap()
                .set_duty(duty_from_angle(i, min_duty, duty_gap))
                .unwrap();

            FreeRtos::delay_ms(1000);
        }
        for i in (0..180).step_by(5).rev() {
            ledc_driver
                .lock()
                .unwrap()
                .set_duty(duty_from_angle(i, min_duty, duty_gap))
                .unwrap();
            FreeRtos::delay_ms(1000);
        }
        */
        FreeRtos::delay_ms(1000);
    }
}
fn index_html() -> &'static str {
    include_str!("../static/index.html")
}
