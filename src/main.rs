use anyhow::{Ok, Result};
use embedded_svc::http::Method;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::prelude::*,
    http::server::{Configuration, EspHttpServer},
    io::Write,
};
use std::{thread::sleep, time::Duration};
use seven_time::{wifi, CONFIG};

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;

    let app_config = CONFIG;

    let _wifi = wifi(
        app_config.wifi_ssid,
        app_config.wifi_psk,
        peripherals.modem,
        sysloop,
    )?;

    let server_config = Configuration::default();
    let mut server = EspHttpServer::new(&server_config)?;

    server.fn_handler("/", Method::Get, move |request| {
        let mut response = request.into_ok_response()?;
        response.write_all("Ciao sono un ESP32".as_bytes())?;
        Ok(())
    })?;

    loop {
        log::info!("Still alive");
        sleep(Duration::from_millis(1000));
    }
}
