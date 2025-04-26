use anyhow::{Ok, Result};
use chrono::{DateTime, Timelike, Utc};
use embedded_svc::http::{Headers, Method};
use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::io::{Read, Write};
use esp_idf_svc::nvs::{EspDefaultNvsPartition, EspNvs, NvsDefault};
use esp_idf_svc::sntp::SyncStatus;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, EspWifi};
use esp_idf_svc::{eventloop::EspSystemEventLoop, hal::prelude::*, http::server::EspHttpServer};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use std::{collections::HashMap, thread::sleep, time::Duration};

use esp_idf_svc::http::server::Configuration as HttpServerConfiguration;
use esp_idf_svc::wifi::Configuration as WifiConfiguration;

const EEPROM_NAMESPACE: &str = "wifi_cfg";
const EEPROM_KEY_SSID: &str = "ssid";
const EEPROM_KEY_PASS: &str = "pass";
const MAX_STR_LEN: usize = 32;

const DEFAULT_SSID: &str = "SevenTime";
const DEFAULT_PASS: &str = "3D Printing <3";

const MIN_ANGLE: u32 = 0;
const MAX_ANGLE: u32 = 180;

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

    let server_config = HttpServerConfiguration::default();
    let mut server = EspHttpServer::new(&server_config)?;

    match &wifi_configuration {
        WifiConfiguration::Client(_) => {
            let mut servos = vec![
                Digit::new(25),
                Digit::new(26),
                Digit::new(27),
                Digit::new(14),
            ];

            let ntp_time = esp_idf_svc::sntp::EspSntp::new_default()?;
            // Synchronize NTP
            println!("Synchronizing with NTP Server");
            while ntp_time.get_sync_status() != SyncStatus::Completed {}
            println!("Time Sync Completed");

            let clock_state = Arc::new(Mutex::new(ClockState::new()));

            // Pass clock_state to the server for the /set_timer endpoint
            let clock_state_clone = clock_state.clone();
            build_time_server(&mut server, clock_state_clone)?;

            loop {
                let mut state = clock_state.lock().unwrap();
                state.update_mode();

                match state.mode {
                    ClockMode::Clock => {
                        let start = SystemTime::now();
                        let dt_now_utc: DateTime<Utc> = start.clone().into();

                        let digits = vec![
                            dt_now_utc.hour() / 10,
                            dt_now_utc.hour() % 10,
                            dt_now_utc.minute() / 10,
                            dt_now_utc.minute() % 10,
                        ];

                        for (digit, servo) in digits.iter().zip(servos.iter_mut()) {
                            servo.set_digit(*digit as u8);
                        }

                        sleep(Duration::from_secs(60));
                    }
                    ClockMode::Timer => {
                        if let Some(start_time) = state.start_time {
                            if let Some(duration) = state.timer_duration {
                                let elapsed = SystemTime::now()
                                    .duration_since(start_time)
                                    .unwrap_or_default();
                                let remaining = if duration > elapsed {
                                    duration - elapsed
                                } else {
                                    Duration::new(0, 0)
                                };

                                let digits = vec![
                                    remaining.as_secs() / 60 / 10,
                                    remaining.as_secs() / 60 % 10,
                                    (remaining.as_secs() % 60) / 10,
                                    (remaining.as_secs() % 60) % 10,
                                ];

                                for (digit, servo) in digits.iter().zip(servos.iter_mut()) {
                                    servo.set_digit(*digit as u8);
                                }

                                sleep(Duration::from_secs(1));
                            }
                        }
                    }
                }
            }
        }
        WifiConfiguration::AccessPoint(_) => {
            log::info!("No credentials found, starting AP mode");
            build_ap_server(&mut server, Arc::new(Mutex::new(cred_nvs)))?;
            loop {
                log::info!("Still alive");
                server.handle();
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

fn build_ap_server(
    server: &mut EspHttpServer<'_>,
    nvs: Arc<Mutex<EspNvs<NvsDefault>>>,
) -> Result<()> {
    server
        .fn_handler("/", Method::Post, move |mut request| {
            log::info!("Received POST request");
            let len = request.content_len().unwrap_or(0) as usize;

            if len > MAX_LEN {
                request.into_status_response(413)?
                    .write_all("Request too big".as_bytes())?;
                return Ok(());
            }

            let mut buf = vec![0; len];
            request.read_exact(&mut buf)?;
            request.into_ok_response()?;

            let body_str = std::str::from_utf8(&buf)?;
            let data: HashMap<&str, &str> = body_str
                .split('&')
                .filter_map(|param| param.split_once('='))
                .collect();
            log::info!("{data:?}");
            if let Some((ssid, pass)) = data
                .get("ssid")
                .and_then(|&ssid| data.get("pass").map(|&pass| (ssid, pass)))
            {
                log::info!("Setting SSID: {} and PASS: {}", ssid, pass);
                let mut lock = nvs
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Failed to lock NVS"))?;
                lock.set_str(EEPROM_KEY_SSID, ssid)?;
                lock.set_str(EEPROM_KEY_PASS, pass)?;
                Ok(())
            } else {
                log::error!("Invalid parameters");
                Err(anyhow::anyhow!("Invalid parameters"))
            }
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

pub struct Digit {
    pin: i32,
    counter: u8,
}

impl Digit {
    pub fn new(pin: i32) -> Self {
        Self { pin, counter: 0 }
    }

    pub fn tick(&mut self) {
        self.counter = (self.counter + 1) % 10;
        self.rotate_servo();
    }

    fn rotate_servo(&self) {
        // Map counter (0..=9) to angle (MIN_ANGLE..=MAX_ANGLE)
        let angle = MIN_ANGLE + ((MAX_ANGLE - MIN_ANGLE) * self.counter as u32) / 9;
        // Here you would add the code to actually set the servo PWM to the angle
        // For example: set_servo_angle(self.pin, angle);
        log::info!("Rotating servo on pin {} to angle {}", self.pin, angle);
    }

    pub fn set_digit(&mut self, digit: u8) {
        while self.counter != digit {
            self.tick();
        }
    }
}

fn build_time_server(
    server: &mut EspHttpServer<'_>,
    _clock_state: Arc<Mutex<ClockState>>,
) -> Result<()> {
    server.fn_handler("/set_timer", Method::Post, move |request| {
        let mut state = _clock_state.lock().unwrap();

        let query: HashMap<&str, &str> = request
            .uri()
            .split_once("?")
            .map(|(_, query)| {
                query
                    .split("&")
                    .map(|param| param.split_once("=").unwrap())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(minutes) = query.get("minutes").and_then(|m| m.parse::<u64>().ok()) {
            state.set_timer(Duration::from_secs(minutes * 60));
            Ok(())
        } else {
            Err(anyhow::anyhow!("Invalid parameters"))
        }
    })?;
    Ok(())
}

pub struct ClockState {
    mode: ClockMode,
    timer_duration: Option<Duration>,
    start_time: Option<SystemTime>,
}

pub enum ClockMode {
    Clock,
    Timer,
}

impl ClockState {
    pub fn new() -> Self {
        Self {
            mode: ClockMode::Clock,
            timer_duration: None,
            start_time: None,
        }
    }

    pub fn set_timer(&mut self, duration: Duration) {
        self.mode = ClockMode::Timer;
        self.timer_duration = Some(duration);
        self.start_time = Some(SystemTime::now());
    }

    pub fn update_mode(&mut self) {
        if let (ClockMode::Timer, Some(start_time), Some(duration)) =
            (&self.mode, self.start_time, self.timer_duration)
        {
            if SystemTime::now()
                .duration_since(start_time)
                .unwrap_or_default()
                >= duration
            {
                self.mode = ClockMode::Clock;
                self.timer_duration = None;
                self.start_time = None;
            }
        }
    }
}
