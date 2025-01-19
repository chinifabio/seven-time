use anyhow::{Ok, Result};
use embedded_svc::http::Method;
use esp_idf_svc::sys;
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    hal::prelude::*,
    http::server::{Configuration, EspHttpServer},
    io::Write,
};
use seven_time::{wifi, CONFIG};
use std::{collections::HashMap, ffi::CString, ptr, thread::sleep, time::Duration};

fn main() -> Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    unsafe {
        sys::esp!(sys::nvs_flash_init())?;
    }
    let namespace = CString::new("storage").unwrap();
    let mut handle: sys::nvs_handle_t = sys::nvs_handle::default();
    unsafe {
        sys::esp!(sys::nvs_open(
            namespace.as_ptr(),
            sys::nvs_open_mode_t_NVS_READWRITE,
            &mut handle
        ))?;
    }

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

    server
        .fn_handler("/set", Method::Get, move |request| {
            let query: HashMap<&str, &str> = request
                .uri()
                .split_once("?")
                .map(|(_, query)| {
                    query
                        .split("&")
                        .map(|param| param.split_once("=").unwrap())
                        .collect()
                })
                .unwrap();
            query
                .get("n")
                .map(|&s| {
                    let key = CString::new("n").unwrap();
                    let val = CString::new(s).unwrap();
                    unsafe {
                        sys::esp!(sys::nvs_set_str(handle, key.as_ptr(), val.as_ptr())).ok();
                        sys::esp!(sys::nvs_commit(handle)).ok();
                    }
                })
                .ok_or("Dammi una n")
        })?
        .fn_handler("/get", Method::Get, move |request| {
            let key = CString::new("n").unwrap();
            let mut buffer_length: usize = 0;
            unsafe {
                sys::esp!(sys::nvs_get_str(
                    handle,
                    key.as_ptr(),
                    ptr::null_mut(),
                    &mut buffer_length
                ))?;
            }
            let mut buffer: Vec<u8> = vec![0; buffer_length];
            unsafe {
                sys::esp!(sys::nvs_get_str(
                    handle,
                    key.as_ptr(),
                    buffer.as_mut_ptr() as *mut i8,
                    &mut buffer_length
                ))?;
            }
            let retrieved_string = String::from_utf8(buffer).expect("Stringa non UTF-8 recuperata");
            let mut respose = request.into_ok_response()?;
            respose.write_all(retrieved_string.as_bytes())?;
            Ok(())
        })?;

    loop {
        log::info!("Still alive");
        sleep(Duration::from_millis(1000));
    }
}
