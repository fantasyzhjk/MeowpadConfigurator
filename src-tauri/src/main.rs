// #![cfg_attr(
//     all(not(debug_assertions), target_os = "windows"),
//     windows_subsystem = "windows"
// )]

use anyhow::{anyhow, Result as AnyResult};
use hidapi_rusb::HidApi;
use log::*;
use meowpad::*;
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use once_cell::sync::Lazy;
use reqwest::Client;
use std::borrow::BorrowMut;
use std::env;
use std::fs;
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::api::path;
use tauri::Manager;

static CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
});
static mut HID_API: Lazy<HidApi> = Lazy::new(|| HidApi::new().unwrap());
static DEVICE: Mutex<Option<Meowpad>> = Mutex::new(None);

/// blocking_dialog
macro_rules! message_dialog {
    ( $title:expr, $message:expr ) => {{
        use tauri::api::dialog::{MessageDialogBuilder, MessageDialogButtons, MessageDialogKind};
        let (s, r) = mpsc::channel();
        MessageDialogBuilder::new($title, $message)
            .buttons(MessageDialogButtons::Ok)
            .kind(MessageDialogKind::Info)
            .show(move |_| s.send(()).unwrap());
        r.recv().unwrap();
    }};
}

/// non_blocking_dialog_with_fn
macro_rules! message_dialog_f {
    ( $title:expr, $message:expr, $f:expr ) => {{
        use tauri::api::dialog::{MessageDialogBuilder, MessageDialogButtons, MessageDialogKind};
        MessageDialogBuilder::new($title, $message)
            .buttons(MessageDialogButtons::Ok)
            .kind(MessageDialogKind::Info)
            .show($f);
    }};
}

fn init_logger(default_level: &str) {
    use env_logger::{Builder, Env};
    let mut builder = Builder::from_env(Env::default().filter_or("LOG_LEVEL", default_level));
    builder
        .format(|buf, record| {
            let level = record.level();
            writeln!(
                buf,
                "[{}] {}",
                buf.default_level_style(level).value(level),
                record.args()
            )
        })
        .init();
}

#[tauri::command]
async fn get_default_config(_app: tauri::AppHandle) -> Config {
    let config = cbor::CONFIG::default();
    config.try_into().expect("???????????????????????????????????????..")
}

#[tauri::command]
async fn get_config(_app: tauri::AppHandle) -> Result<Config, String> {
    || -> AnyResult<_> {
        let mut _d = DEVICE.lock().unwrap();
        let d = _d.as_mut().ok_or_else(|| anyhow!("??????????????????"))?;
        d.load_config()
            .map_err(|e| anyhow!("?????????????????????, {}", e))?;
        d.config()
            .try_into()
            .map_err(|e| anyhow!("?????????????????????, {}", e))
    }()
    .map_err(|e| format!("{}", e))
}

#[tauri::command]
async fn get_device_info(_app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    || -> AnyResult<_> {
        let mut _d = DEVICE.lock().unwrap();
        let d = _d.as_mut().ok_or_else(|| anyhow!("??????????????????"))?;
        let name = d
            .get_device_name()
            .map_err(|e| anyhow!("????????????????????????, {}", e))?;
        let version = d
            .get_firmware_version()
            .map_err(|e| anyhow!("???????????????????????????, {}", e))?;
        info!("???????????????{}", name);
        info!("???????????????{}", version);

        Ok(serde_json::json!({
            "name": name,
            "version": version
        }))
    }()
    .map_err(|e| format!("{}", e))
}

#[tauri::command]
async fn save_config(_app: tauri::AppHandle, config: Config) -> Result<(), String> {
    || -> AnyResult<_> {
        let mut _d = DEVICE.lock().unwrap();
        let d = _d.as_mut().ok_or_else(|| anyhow!("??????????????????"))?;
        d.config = Some(config.into());
        d.write_config()?;
        d.flush()?;
        Ok(())
    }()
    .map_err(|e| format!("{}", e))
}

#[tauri::command]
async fn get_version(_app: tauri::AppHandle) -> Result<Version, String> {
    Version::get().await.map_err(|e| format!("{}", e))
}

#[tauri::command]
async fn get_firmware_version(_app: tauri::AppHandle) -> &'static str {
    FIRMWARE_VERSION
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Version {
    version: String,
    download_url: String,
    latest_firmware_version: String,
    latest_firmware_download_url: String,
}

impl Version {
    async fn get() -> reqwest::Result<Version> {
        CLIENT
            .get("https://desu.life/device/configurator_version")
            .send()
            .await?
            .json::<Version>()
            .await
    }
}

use tauri::api::shell;

#[tauri::command]
async fn check_update(window: tauri::Window, version: Version) -> bool {
    if compare_version(VERSION, &version.version) < 0 {
        window.hide().unwrap();
        message_dialog_f!(
            "Meowpad Configurator",
            "?????????????????????????????????????????????",
            move |_| {
                shell::open(&window.shell_scope(), version.download_url, None).unwrap();
                window.close().unwrap();
            }
        );
        return false;
    }
    true
}

#[tauri::command]
async fn connect(_app: tauri::AppHandle) -> Result<(), String> {
    _connect().map_err(|e| format!("{}", e))
}

fn _connect() -> AnyResult<()> {
    info!("????????????!");
    let found_device = find_device();

    match found_device {
        Some(device) => {
            let mut _d = DEVICE.lock().unwrap();
            *_d = Some(device);
            Ok(())
        }
        None => {
            warn!("?????????????????????????????????");
            Err(anyhow!("??????????????????????????????????????????Meowpad"))
        }
    }
}

fn find_device() -> Option<Meowpad> {
    // connect
    let api = unsafe { HID_API.borrow_mut() };
    api.refresh_devices().unwrap();
    let (vid, pid) = (0x5D3E, 0x7490);
    let meowpad = api
        .device_list()
        .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
        .filter_map(|d| {
            let device = match d.open_device(api) {
                Ok(d) => Meowpad::new(
                    d,
                    path::cache_dir()
                        .map(|mut p| {
                            p.push(".meowkey");
                            p
                        })
                        .unwrap_or_else(|| PathBuf::from(".meowkey")),
                ),
                Err(_) => return None,
            };
            // ????????????
            match device.ping() {
                Ok(r) if !r => return None,
                Err(_) => return None,
                _ => (),
            }
            info!("???????????????");
            debug!("Name: {}", d.product_string().unwrap_or("None"));
            debug!(
                "Manufacturer: {}",
                d.manufacturer_string().unwrap_or("None")
            );
            debug!("Addr: {}", d.path().to_string_lossy());
            debug!("{:?}", d);
            Some(device)
        })
        .next();

    meowpad
}

pub fn compare_version(version1: &str, version2: &str) -> i32 {
    use std::cmp::Ordering::{Greater, Less};

    let (mut it_1, mut it_2) = (version1.split('.'), version2.split('.'));
    loop {
        let (s1, s2) = (it_1.next(), it_2.next());
        if s1.is_none() && s2.is_none() {
            break;
        }

        let s1 = s1.unwrap_or("0").trim_start_matches('0');
        let s2 = s2.unwrap_or("0").trim_start_matches('0');
        if s1.len() != s2.len() {
            return (s1.len() as i32 - s2.len() as i32).signum();
        }

        for (c1, c2) in s1.chars().zip(s2.chars()) {
            match c1.cmp(&c2) {
                Greater => return 1,
                Less => return -1,
                _ => (),
            }
        }
    }
    0
}

static VERSION: &str = "0.2.0";
static FIRMWARE_VERSION: &str = "0.1.2";

fn main() -> AnyResult<()> {
    panic::set_hook(Box::new(|e| {
        use std::backtrace::Backtrace;
        let emessage = format!(
            "???????????????????????????????????????????????????\n{}\n{}",
            e,
            Backtrace::force_capture()
        );
        eprintln!("{emessage}");
        message_dialog!("Meowpad Configurator", &emessage)
    }));

    init_logger("INFO");

    let action = env::args().nth(1).unwrap_or_default();
    match action.as_str() {
        "--reset" => {
            _connect()?;
            let mut _d = DEVICE.lock().unwrap();
            let mut d = _d.take().unwrap();
            info!("???????????????{}", d.get_device_name()?);
            info!("???????????????{}", d.get_firmware_version()?);
            d.reset_config()?;
            warn!("??????????????????")
        }
        "--console" => {
            _connect()?;
            let mut _d = DEVICE.lock().unwrap();
            let mut d = _d.take().unwrap();
            info!("???????????????{}", d.get_device_name()?);
            info!("???????????????{}", d.get_firmware_version()?);
            d.load_config()?;
            info!("?????????????????????{:?}", d.config());
            let mut f = std::fs::File::create("meowpad.toml")?;
            f.write_all(&toml::to_vec(&d.config())?)?;

            let (tx, rx) = mpsc::channel();
            let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_secs(2)).unwrap();
            watcher
                .watch("meowpad.toml", RecursiveMode::NonRecursive)
                .unwrap();

            warn!("???????????????????????????meowpad.toml???????????????????????????????????????????????????");

            loop {
                match rx.recv() {
                    Ok(DebouncedEvent::Write(_)) => {
                        warn!(" * ????????????????????????????????????????????? ...");
                        d.config.iter_mut().for_each(|c| {
                            *c = toml::from_str(&fs::read_to_string("meowpad.toml").unwrap())
                                .unwrap();
                        });
                        d.write_config()?;
                    }
                    Err(e) => error!("watch error: {:?}", e),
                    _ => (),
                }
            }
        }
        _ => {
            tauri::Builder::default()
                .setup(|_app| {
                    #[cfg(debug_assertions)] // only include this code on debug builds
                    {
                        let window = _app.get_window("main").unwrap();
                        window.open_devtools();
                        window.set_fullscreen(false).unwrap();
                    }
                    Ok(())
                })
                .invoke_handler(tauri::generate_handler![
                    connect,
                    get_config,
                    save_config,
                    get_default_config,
                    get_device_info,
                    check_update,
                    get_version,
                    get_firmware_version
                ])
                .run(tauri::generate_context!())
                .expect("error while running tauri application");
        }
    }
    Ok(())
}
