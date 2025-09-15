use anyhow::Result;
use esp_idf_hal::ledc::config::TimerConfig;
use esp_idf_hal::ledc::{LedcDriver, LedcTimerDriver};
use heapless::String;

use esp_idf_hal::delay::FreeRtos;
use esp_idf_hal::{gpio::PinDriver, peripherals::Peripherals};
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::io::EspIOError;
use esp_idf_svc::ipv4;
use esp_idf_svc::nvs::{EspNvsPartition, NvsDefault};
use esp_idf_svc::wifi::{BlockingWifi, EspWifi};

use esp_idf_svc::hal::ledc;

use embedded_svc::http::server::Method;

use serde_json::json;

use log::*;
use std::io::{Read, Write};

use embedded_websocket::{
    framer::{Framer, ReadResult},
    WebSocketSendMessageType, WebSocketServer,
};
use std::net::TcpListener;

use std::sync::{Arc, Mutex};

mod servo;
use crate::servo::Servo;

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let sys_loop = EspSystemEventLoop::take().unwrap();
    let nvs = <EspNvsPartition<NvsDefault>>::take().unwrap();

    let gpio5 = PinDriver::input(peripherals.pins.gpio5)?;
    let gpio5 = Arc::new(Mutex::new(gpio5));

    let timer_driver = LedcTimerDriver::new(
        peripherals.ledc.timer0,
        &TimerConfig::default()
            .frequency(50.into())
            .resolution(ledc::Resolution::Bits14),
    )?;
    let ledc1 = LedcDriver::new(
        peripherals.ledc.channel0,
        &timer_driver,
        peripherals.pins.gpio3,
    )?;
    let ledc2 = LedcDriver::new(
        peripherals.ledc.channel1,
        &timer_driver,
        peripherals.pins.gpio4,
    )?;

    let servo1 = Servo::new(ledc1, 90.0).unwrap();
    let servo2 = Servo::new(ledc2, 90.0).unwrap();

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
            auth_method: esp_idf_svc::wifi::AuthMethod::WPAWPA2Personal,
            ..Default::default()
        });

    wifi.set_configuration(&wifi_configuration).unwrap();
    wifi.start()?;
    let scan_results = wifi.scan()?;
    log::info!("Found {} Wi-Fi networks:", scan_results.len());

    let mut found = false;
    for ap in &scan_results {
        let ssid_str = ap.ssid.as_str(); // pas besoin de from_utf8
        log::info!("  SSID: {}, Signal: {}", ssid_str, ap.signal_strength);
        if ssid_str == SSID {
            found = true;
        }
    }
    if !found {
        log::warn!("⚠️ SSID '{}' not found during scan!", SSID);
        // ici tu peux décider de retenter un scan, attendre, ou carrément redémarrer l'ESP32
    }
    connect_wifi(&mut wifi)?;

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

    let listener = TcpListener::bind("0.0.0.0:8080")?;
    info!("WebSocket server listening on ws://{}:8080", ip_info.ip);

    for stream in listener.incoming() {
        let mut stream = stream?;
        let servo1_clone = servo1.clone();
        let servo2_clone = servo2.clone();
        let gpio5_clone = gpio5.clone();

        std::thread::Builder::new()
            .stack_size(8192)
            .spawn(move || -> anyhow::Result<()> {
                let mut websocket = WebSocketServer::new_server();

                let mut read_buf = Box::new([0u8; 1024]);
                let mut write_buf = Box::new([0u8; 1024]);
                let mut read_cursor = 0;

                let n = stream.read(&mut *read_buf).unwrap();
                let mut headers_arr = [httparse::EMPTY_HEADER; 16];
                let mut request = httparse::Request::new(&mut headers_arr);
                request.parse(&read_buf[..n]).unwrap();

                let header_iter = request.headers.iter().map(|h| (h.name, h.value));
                let websocket_context = embedded_websocket::read_http_header(header_iter)?
                    .expect("WebSocket context missing");

                let size = websocket.server_accept(
                    &websocket_context.sec_websocket_key,
                    None,
                    &mut *write_buf,
                )?;
                stream.write_all(&write_buf[..size]).unwrap();

                // 2️⃣ Créer le framer
                let mut framer = Framer::new(
                    &mut *read_buf,
                    &mut read_cursor,
                    &mut *write_buf,
                    &mut websocket,
                );

                stream.set_nonblocking(true).unwrap();
                let mut last_status = std::time::Instant::now();

                let mut old_angle = servo1_clone.get_angle();
                //for send the status the first time
                let mut old_gpio5_state = !gpio5_clone.lock().unwrap().is_high() as u8;

                loop {
                    let mut temp_buf = [0u8; 512];
                    match framer.read(&mut stream, &mut temp_buf) {
                        Ok(ReadResult::Text(text)) => {
                            info!("Received: {}", text);
                            if let Some(angle_str) = text.strip_prefix("servo1=") {
                                if let Ok(angle) = angle_str.parse::<u32>() {
                                    servo1_clone.set_angle(angle as f32).unwrap();
                                }
                            }
                            if let Some(angle_str) = text.strip_prefix("servo2=") {
                                if let Ok(angle) = angle_str.parse::<u32>() {
                                    servo2_clone.set_angle(angle as f32).unwrap();
                                }
                            }
                        }
                        Ok(ReadResult::Closed) => {
                            info!("Client disconnected");
                            break Ok(());
                        }
                        Err(embedded_websocket::framer::FramerError::Io(e))
                            if e.kind() == std::io::ErrorKind::WouldBlock =>
                        {
                            // Pas de message dispo
                        }
                        Err(e) => {
                            warn!("WebSocket error: {:?}", e);
                            break Ok(());
                        }
                        _ => {}
                    }

                    if last_status.elapsed() >= std::time::Duration::from_millis(100) {
                        let angle = servo1_clone.get_angle();
                        let gpio5_state = gpio5_clone.lock().unwrap().is_high() as u8;
                        if (angle != old_angle) || (gpio5_state != old_gpio5_state) {
                            let status = json!({
                                "angle": angle,
                                "gpio5": gpio5_state
                            });
                            framer
                                .write(
                                    &mut stream,
                                    WebSocketSendMessageType::Text,
                                    true,
                                    status.to_string().as_bytes(),
                                )
                                .unwrap();
                            last_status = std::time::Instant::now();
                            info!("status {:?} send at {:?}", status, last_status);
                            old_angle = angle;
                            old_gpio5_state = gpio5_state;
                        }
                    }
                    FreeRtos::delay_ms(10);
                }
            })?;
    }
    loop {
        //never
        info!("loop !! ");

        FreeRtos::delay_ms(1000);
    }
}
fn index_html() -> &'static str {
    include_str!("../static/index.html")
}

use std::time::{Duration, Instant};

fn connect_wifi(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    wifi.start()?;

    let mut attempt = 0;
    loop {
        attempt += 1;
        log::info!("Connecting to Wi-Fi... attempt {}", attempt);

        if let Err(e) = wifi.connect() {
            log::warn!("Wi-Fi connect error: {:?}", e);
        }

        // Timeout d'attente de 10 secondes max
        let start = Instant::now();
        loop {
            match wifi.wait_netif_up() {
                Ok(_) => {
                    log::info!("Wi-Fi connected!");
                    return Ok(());
                }
                Err(e) => {
                    if start.elapsed() > Duration::from_secs(10) {
                        log::warn!("Timeout waiting for netif_up: {:?}", e);
                        break; // on sort pour réessayer
                    }
                    FreeRtos::delay_ms(500);
                }
            }
        }
    }
}
