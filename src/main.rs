use anyhow::{Ok, Result};
use embedded_svc::http::{Headers, Method};
use esp_idf_hal::ledc::config::TimerConfig;
use esp_idf_hal::ledc::{LedcDriver, LedcTimerDriver, Resolution};
use esp_idf_svc::io::{Read, Write};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sntp::SyncStatus;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, hal::prelude::*, http::server::EspHttpServer};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::{thread::sleep, time::Duration};

use esp_idf_svc::http::server::Configuration as HttpServerConfiguration;
use esp_idf_svc::wifi::Configuration as WifiConfiguration;

use crate::clock::Clock;
use crate::display::{Digit, Display};

mod clock;
mod display;

const EEPROM_NAMESPACE: &str = "wifi_cfg";
const EEPROM_KEY_SSID: &str = "ssid";
const EEPROM_KEY_PASS: &str = "pass";
const MAX_STR_LEN: usize = 32;

const DEFAULT_SSID: &str = "SevenTime";
const DEFAULT_PASS: &str = "3D Printing <3";

const HTML_PAGE: &str = include_str!("../html/index.html");
const MAX_LEN: usize = 128;

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    log::info!("Initializing peripherals");
    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let nvs = EspDefaultNvsPartition::take()?;

    log::info!("Reading credentials from EEPROM");
    let cred_nvs = EspNvs::new(nvs.clone(), EEPROM_NAMESPACE, true)?;
    let mut ssid_buffer: [u8; MAX_STR_LEN] = [0; MAX_STR_LEN];
    let mut pass_buffer: [u8; MAX_STR_LEN] = [0; MAX_STR_LEN];
    let ssid = cred_nvs.get_str(EEPROM_KEY_SSID, &mut ssid_buffer)?;
    let pass = cred_nvs.get_str(EEPROM_KEY_PASS, &mut pass_buffer)?;

    log::info!("Starting WiFi...");
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(peripherals.modem, sysloop.clone(), Some(nvs))?,
        sysloop,
    )?;

    let wifi_configuration = match (ssid, pass) {
        (Some(ssid), Some(pass)) => {
            log::info!("Credentials found, setting connection to {}", ssid);
            WifiConfiguration::Client(ClientConfiguration {
                ssid: ssid.try_into().unwrap(),
                password: pass.try_into().unwrap(),
                auth_method: AuthMethod::WPA2Personal,
                ..Default::default()
            })
        }
        _ => {
            log::info!("No credentials found, setting AP mode");
            WifiConfiguration::AccessPoint(esp_idf_svc::wifi::AccessPointConfiguration {
                ssid: DEFAULT_SSID.try_into().unwrap(),
                password: DEFAULT_PASS.try_into().unwrap(),
                auth_method: AuthMethod::WPA2Personal,
                max_connections: 4,
                ..Default::default()
            })
        }
    };

    wifi.set_configuration(&wifi_configuration)?;
    wifi.start()?;
    if let WifiConfiguration::Client(_) = &wifi_configuration {
        log::info!("Connecting to WiFi...");
        wifi.connect()?;
    } else {
        log::info!("Starting AP mode...");
    }
    wifi.wait_netif_up()?;
    log::info!(
        "Wifi connected with IP: {:?}",
        wifi.wifi().sta_netif().get_ip_info()?
    );

    let server_config = HttpServerConfiguration::default();
    let mut server = EspHttpServer::new(&server_config)?;

    match &wifi_configuration {
        WifiConfiguration::Client(_) => {
            let ntp_time = esp_idf_svc::sntp::EspSntp::new_default()?;
            println!("Synchronizing with NTP Server");
            while ntp_time.get_sync_status() != SyncStatus::Completed {}
            println!("Time Sync Completed");

            let timer_driver = LedcTimerDriver::new(
                peripherals.ledc.timer0,
                &TimerConfig::default()
                    .frequency(Hertz::from(50))
                    .resolution(Resolution::Bits12),
            )?;
            let mut display = Display {
                servos: [
                    Digit::new(LedcDriver::new(
                        peripherals.ledc.channel0,
                        &timer_driver,
                        peripherals.pins.gpio33,
                    )?),
                    Digit::new(LedcDriver::new(
                        peripherals.ledc.channel1,
                        &timer_driver,
                        peripherals.pins.gpio25,
                    )?),
                    Digit::new(LedcDriver::new(
                        peripherals.ledc.channel2,
                        &timer_driver,
                        peripherals.pins.gpio26,
                    )?),
                    Digit::new(LedcDriver::new(
                        peripherals.ledc.channel3,
                        &timer_driver,
                        peripherals.pins.gpio27,
                    )?),
                ],
            };

            let clock_ref = Arc::new(Mutex::new(Clock::new()));
            let clock_ref_clone = clock_ref.clone();
            build_time_server(&mut server, clock_ref_clone)?;

            loop {
                let content = clock_ref
                    .lock()
                    .expect("Failed to lock clock to tick")
                    .tick();
                if let Some(digits) = content {
                    display.write(digits);
                }
            }
        }
        WifiConfiguration::AccessPoint(_) => {
            log::info!("No credentials found, starting AP mode");
            build_ap_server(&mut server, Arc::new(Mutex::new(cred_nvs)))?;
            loop {
                log::info!("Waiting ...");
                sleep(Duration::from_secs(10));
            }
        }
        _ => {
            log::error!("Impossible configuration state");
            loop {
                log::info!("Still alive");
                sleep(Duration::from_secs(10));
            }
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize)]
struct SetCredentialData {
    ssid: String,
    pass: String,
}

fn build_ap_server(
    server: &mut EspHttpServer<'_>,
    nvs: Arc<Mutex<EspNvs<NvsDefault>>>,
) -> Result<()> {
    server
        .fn_handler("/", Method::Post, move |mut request| {
            log::info!("Received POST request");
            let len = request.content_len().unwrap_or(0) as usize;

            if len > MAX_LEN {
                request
                    .into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok(());
            }

            let mut buf = vec![0; len];
            request.read_exact(&mut buf)?;
            request.into_ok_response()?;
            let data: SetCredentialData = serde_json::from_slice(&buf)?;

            let mut lock = nvs
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to lock credentials NVS"))?;
            lock.set_str(EEPROM_KEY_SSID, data.ssid.as_str())?;
            lock.set_str(EEPROM_KEY_PASS, data.pass.as_str())?;
            Ok(())
        })?
        .fn_handler("/", Method::Get, move |request| {
            let html = HTML_PAGE;
            let mut response = request.into_ok_response()?;
            response.write(html.as_bytes())?;
            response.release();
            Ok(())
        })?;
    Ok(())
}

#[derive(Default, Debug, Clone, Deserialize)]
struct SetTimerData {
    minutes: u64,
}

fn build_time_server(server: &mut EspHttpServer<'_>, clock: Arc<Mutex<Clock>>) -> Result<()> {
    server.fn_handler("/set_timer", Method::Post, move |mut request| {
        let len = request.content_len().unwrap_or(0) as usize;

        if len > MAX_LEN {
            request
                .into_status_response(413)?
                .write_all("Request too big".as_bytes())?;
            return Ok(());
        }

        let mut buf = vec![0; len];
        request.read_exact(&mut buf)?;
        request.into_ok_response()?;
        let data: SetTimerData = serde_json::from_slice(&buf)?;

        log::info!("Setting timer for {} minutes", data.minutes);
        let duration = Duration::from_secs(data.minutes * 60);
        let mut state = clock.lock().expect("Failed to lock clock to start a tiemr");
        state.set_timer(duration);

        Ok(())
    })?;
    Ok(())
}
