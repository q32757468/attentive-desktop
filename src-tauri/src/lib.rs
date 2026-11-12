use attentive_notifier::{
    create_notifier_router, NotificationDispatcher, NotificationRequest, NotifierServerOptions,
    WindowsToastDispatcher, DEFAULT_MAX_BODY_BYTES, DEFAULT_NOTIFIER_HOST, DEFAULT_NOTIFIER_PORT,
};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr, UdpSocket},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};
use tauri::{async_runtime::JoinHandle, AppHandle, Manager, State as TauriState};
#[cfg(desktop)]
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    WindowEvent,
};
use tokio::{net::TcpListener, sync::Mutex};
use uuid::Uuid;

const AUTOSTART_ARG: &str = "--from-autostart";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
    #[serde(default)]
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            max_body_bytes: default_max_body_bytes(),
            autostart: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    pub host: String,
    pub port: u16,
    pub endpoint: String,
    pub lan_endpoint: Option<String>,
    pub last_error: Option<String>,
}

impl ServerStatus {
    fn offline(config: &AppConfig, last_error: Option<String>) -> Self {
        let endpoint = local_endpoint(&config.host, config.port);
        Self {
            running: false,
            host: config.host.clone(),
            port: config.port,
            lan_endpoint: lan_endpoint(&config.host, config.port, &endpoint),
            endpoint,
            last_error,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub config: AppConfig,
    pub status: ServerStatus,
    pub autostart: bool,
    pub can_enable_autostart: bool,
}

struct RuntimeState {
    config: RwLock<AppConfig>,
    status: RwLock<ServerStatus>,
    server: Mutex<Option<ServerHandle>>,
    restart_lock: Mutex<()>,
    config_warning: RwLock<Option<String>>,
    config_path: PathBuf,
}

struct ServerHandle {
    task: JoinHandle<()>,
}

#[derive(Debug, Clone, Default)]
struct CliOverrides {
    host: Option<String>,
    port: Option<u16>,
    started_from_autostart: bool,
}

#[tauri::command]
async fn get_settings(
    app: AppHandle,
    state: TauriState<'_, Arc<RuntimeState>>,
) -> Result<SettingsSnapshot, String> {
    snapshot(&app, state.inner()).await
}

#[tauri::command]
async fn save_settings(
    app: AppHandle,
    state: TauriState<'_, Arc<RuntimeState>>,
    mut settings: AppConfig,
) -> Result<SettingsSnapshot, String> {
    validate_config(&settings)?;

    let actual_autostart = current_autostart(&app).unwrap_or(settings.autostart);
    settings.autostart = actual_autostart;

    restart_server(state.inner(), settings.clone()).await?;
    if let Err(error) = write_config(&state.config_path, &settings) {
        set_config_warning(state.inner(), error.clone());
        return Err(error);
    }
    clear_config_warning(state.inner());
    snapshot(&app, state.inner()).await
}

#[tauri::command]
async fn set_autostart(
    app: AppHandle,
    state: TauriState<'_, Arc<RuntimeState>>,
    enabled: bool,
) -> Result<SettingsSnapshot, String> {
    set_autostart_state(&app, enabled)?;

    let config_to_save = {
        let mut config = state
            .config
            .write()
            .map_err(|_| "配置锁不可用".to_string())?;
        config.autostart = enabled;
        config.clone()
    };
    if let Err(error) = write_config(&state.config_path, &config_to_save) {
        set_config_warning(state.inner(), error.clone());
        return Err(error);
    }
    clear_config_warning(state.inner());

    snapshot(&app, &state).await
}

#[tauri::command]
async fn test_notification(title: String, body: String) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("通知标题不能为空".to_string());
    }
    if body.trim().is_empty() {
        return Err("通知内容不能为空".to_string());
    }

    let mut request = NotificationRequest::new(title, body);
    request.source = Some("settings".to_string());
    WindowsToastDispatcher
        .dispatch(request, Uuid::new_v4().to_string())
        .await
}

pub fn run() {
    let overrides = match parse_cli(std::env::args().skip(1).collect()) {
        Ok(CliResult::Help) => {
            println!("{}", notifier_help());
            return;
        }
        Ok(CliResult::Overrides(overrides)) => overrides,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !args.iter().any(|arg| arg == AUTOSTART_ARG) {
                show_main_window(app);
            }
        }));
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ));
    }

    builder
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            set_autostart,
            test_notification
        ])
        .setup(move |app| {
            let config_path = app
                .path()
                .app_config_dir()
                .map_err(|error| format!("无法找到应用配置目录: {error}"))?
                .join("settings.json");

            let (mut persisted_config, mut config_warning) = load_config(&config_path);
            let mut config = persisted_config.clone();
            apply_environment_overrides(&mut config);
            apply_cli_overrides(&mut config, &overrides);

            if let Some(actual_autostart) = current_autostart(&app.handle()) {
                config.autostart = actual_autostart;
                if persisted_config.autostart != actual_autostart {
                    persisted_config.autostart = actual_autostart;
                    if config_warning.is_none() {
                        if let Err(error) = write_config(&config_path, &persisted_config) {
                            config_warning = Some(format!("无法同步开机自动启动状态: {error}"));
                        }
                    }
                }
            }

            validate_config(&config).map_err(|error| format!("配置无效: {error}"))?;

            let state = Arc::new(RuntimeState {
                status: RwLock::new(ServerStatus::offline(&config, config_warning.clone())),
                config: RwLock::new(config.clone()),
                server: Mutex::new(None),
                restart_lock: Mutex::new(()),
                config_warning: RwLock::new(config_warning),
                config_path,
            });
            app.manage(state.clone());

            #[cfg(desktop)]
            {
                let show_item = MenuItemBuilder::with_id("show", "显示窗口").build(app)?;
                let quit_item =
                    MenuItemBuilder::with_id("quit", "退出 Attentive Desktop").build(app)?;
                let menu = MenuBuilder::new(app)
                    .items(&[&show_item, &quit_item])
                    .build()?;
                let mut tray_builder = TrayIconBuilder::new()
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .tooltip("Attentive Desktop · 通知服务")
                    .on_menu_event(|app, event| {
                        if event.id() == "show" {
                            show_main_window(app);
                        } else if event.id() == "quit" {
                            app.exit(0);
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main_window(tray.app_handle());
                        }
                    });
                if let Some(icon) = app.default_window_icon().cloned() {
                    tray_builder = tray_builder.icon(icon);
                }
                tray_builder.build(app)?;

                if !overrides.started_from_autostart {
                    show_main_window(&app.handle());
                }
            }

            tauri::async_runtime::spawn(async move {
                if let Err(error) = restart_server(&state, config).await {
                    set_runtime_error(&state, error);
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn snapshot(app: &AppHandle, state: &Arc<RuntimeState>) -> Result<SettingsSnapshot, String> {
    let autostart = current_autostart(app).unwrap_or_else(|| {
        state
            .config
            .read()
            .map(|config| config.autostart)
            .unwrap_or(false)
    });

    let mut config = state
        .config
        .read()
        .map_err(|_| "配置锁不可用".to_string())?
        .clone();
    config.autostart = autostart;

    let status = state
        .status
        .read()
        .map_err(|_| "状态锁不可用".to_string())?
        .clone();
    let mut status = status;
    status.endpoint = local_endpoint(&status.host, status.port);
    status.lan_endpoint = lan_endpoint(&status.host, status.port, &status.endpoint);

    Ok(SettingsSnapshot {
        config,
        status,
        autostart,
        can_enable_autostart: !cfg!(debug_assertions),
    })
}

async fn restart_server(state: &Arc<RuntimeState>, config: AppConfig) -> Result<(), String> {
    validate_config(&config)?;

    let _restart_guard = state.restart_lock.lock().await;

    let (same_listener, can_reuse_listener) = {
        let current_config = state
            .config
            .read()
            .map_err(|_| "配置锁不可用".to_string())?;
        let status = state
            .status
            .read()
            .map_err(|_| "状态锁不可用".to_string())?;
        let same_listener = status.running
            && config.port != 0
            && current_config.host == config.host
            && current_config.port == config.port;
        (
            same_listener,
            same_listener && current_config.max_body_bytes == config.max_body_bytes,
        )
    };

    if can_reuse_listener {
        update_config(state, &config)?;
        let warning = current_config_warning(state);
        let mut status = state
            .status
            .write()
            .map_err(|_| "状态锁不可用".to_string())?;
        status.host = config.host;
        status.endpoint = local_endpoint(&status.host, status.port);
        status.lan_endpoint = lan_endpoint(&status.host, status.port, &status.endpoint);
        status.last_error = warning;
        return Ok(());
    }

    // Keep the current notifier available when a new address cannot be bound.
    // If only the body limit changed, the current listener must be stopped
    // before the replacement router can bind the same address.
    let listener = if same_listener {
        None
    } else {
        Some(
            TcpListener::bind(bind_address(&config.host, config.port))
                .await
                .map_err(|error| format!("无法监听 {}:{}: {error}", config.host, config.port))?,
        )
    };

    let old_server = {
        let mut server = state.server.lock().await;
        server.take()
    };
    if let Some(old_server) = old_server {
        old_server.task.abort();
        let _ = old_server.task.await;
    }

    let listener = match listener {
        Some(listener) => listener,
        None => match TcpListener::bind(bind_address(&config.host, config.port)).await {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!("无法监听 {}:{}: {error}", config.host, config.port);
                set_runtime_error(state, message.clone());
                return Err(message);
            }
        },
    };
    let local_address = listener
        .local_addr()
        .map_err(|error| format!("无法读取服务地址: {error}"))?;

    let router = create_notifier_router(
        NotifierServerOptions::default().with_max_body_bytes(config.max_body_bytes),
    );
    let status_state = Arc::clone(state);
    let task = tauri::async_runtime::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            set_runtime_error(&status_state, format!("通知服务已停止: {error}"));
        }
    });

    update_config(state, &config)?;
    let warning = current_config_warning(state);
    {
        let mut status = state
            .status
            .write()
            .map_err(|_| "状态锁不可用".to_string())?;
        *status = ServerStatus {
            running: true,
            host: config.host.clone(),
            port: local_address.port(),
            endpoint: local_endpoint(&config.host, local_address.port()),
            lan_endpoint: lan_endpoint(
                &config.host,
                local_address.port(),
                &local_endpoint(&config.host, local_address.port()),
            ),
            last_error: warning,
        };
    }
    {
        let mut server = state.server.lock().await;
        *server = Some(ServerHandle { task });
    }

    Ok(())
}

fn update_config(state: &Arc<RuntimeState>, config: &AppConfig) -> Result<(), String> {
    let mut current_config = state
        .config
        .write()
        .map_err(|_| "配置锁不可用".to_string())?;
    *current_config = config.clone();
    Ok(())
}

fn current_config_warning(state: &Arc<RuntimeState>) -> Option<String> {
    state
        .config_warning
        .read()
        .ok()
        .and_then(|warning| warning.clone())
}

fn set_config_warning(state: &Arc<RuntimeState>, error: String) {
    if let Ok(mut warning) = state.config_warning.write() {
        *warning = Some(error.clone());
    }
    if let Ok(mut status) = state.status.write() {
        status.last_error = Some(error);
    }
}

fn clear_config_warning(state: &Arc<RuntimeState>) {
    if let Ok(mut warning) = state.config_warning.write() {
        *warning = None;
    }
    if let Ok(mut status) = state.status.write() {
        if status.running {
            status.last_error = None;
        }
    }
}

fn set_runtime_error(state: &Arc<RuntimeState>, error: String) {
    if let Ok(config) = state.config.read() {
        if let Ok(mut status) = state.status.write() {
            *status = ServerStatus::offline(&config, Some(error));
        }
    }
}

fn validate_config(config: &AppConfig) -> Result<(), String> {
    if config.host.trim().is_empty() {
        return Err("监听地址不能为空".to_string());
    }
    if config.max_body_bytes == 0 {
        return Err("请求体上限必须大于 0".to_string());
    }
    Ok(())
}

fn load_config(path: &Path) -> (AppConfig, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(contents) => match serde_json::from_str::<AppConfig>(&contents) {
            Ok(config) => (config, None),
            Err(error) => (
                AppConfig::default(),
                Some(format!("配置文件损坏，已使用默认值: {error}")),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (AppConfig::default(), None),
        Err(error) => (
            AppConfig::default(),
            Some(format!("无法读取配置文件，已使用默认值: {error}")),
        ),
    }
}

fn write_config(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("无法创建配置目录: {error}"))?;
    }
    let contents =
        serde_json::to_string_pretty(config).map_err(|error| format!("无法序列化配置: {error}"))?;
    let temporary_path = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("settings.json"),
        Uuid::new_v4()
    ));

    if let Err(error) = std::fs::write(&temporary_path, format!("{contents}\n")) {
        return Err(format!("无法保存配置: {error}"));
    }

    let result = replace_config_file(&temporary_path, path);
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(windows)]
fn replace_config_file(temporary_path: &Path, path: &Path) -> Result<(), String> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        },
    };

    let to_wide = |value: &OsStr| {
        value
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let temporary_path = to_wide(temporary_path.as_os_str());
    let path = to_wide(path.as_os_str());

    unsafe {
        MoveFileExW(
            PCWSTR(temporary_path.as_ptr()),
            PCWSTR(path.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| format!("无法保存配置: {error}"))
    }
}

#[cfg(not(windows))]
fn replace_config_file(temporary_path: &Path, path: &Path) -> Result<(), String> {
    std::fs::rename(temporary_path, path).map_err(|error| format!("无法保存配置: {error}"))
}

fn current_autostart(app: &AppHandle) -> Option<bool> {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        return app.autolaunch().is_enabled().ok();
    }

    #[cfg(not(desktop))]
    {
        let _ = app;
        None
    }
}

fn set_autostart_state(app: &AppHandle, enabled: bool) -> Result<(), String> {
    if enabled && cfg!(debug_assertions) {
        return Err("开发版本不能开启开机自动启动，请安装并使用正式版本设置自启".to_string());
    }

    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        if enabled {
            manager.enable().map_err(|error| error.to_string())?;
        } else {
            manager.disable().map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    #[cfg(not(desktop))]
    {
        let _ = (app, enabled);
        Err("当前平台不支持开机自动启动".to_string())
    }
}

fn apply_environment_overrides(config: &mut AppConfig) {
    if let Ok(host) = std::env::var("ATTENTIVE_NOTIFIER_HOST") {
        if !host.trim().is_empty() {
            config.host = host;
        }
    }
    if let Ok(port) = std::env::var("ATTENTIVE_NOTIFIER_PORT") {
        config.port = parse_port(&port).unwrap_or(DEFAULT_NOTIFIER_PORT);
    }
}

fn apply_cli_overrides(config: &mut AppConfig, overrides: &CliOverrides) {
    if let Some(host) = overrides.host.as_ref() {
        config.host = host.clone();
    }
    if let Some(port) = overrides.port {
        config.port = port;
    }
}

enum CliResult {
    Help,
    Overrides(CliOverrides),
}

fn parse_cli(argv: Vec<String>) -> Result<CliResult, String> {
    let mut result = CliOverrides::default();
    let mut index = 0;
    while index < argv.len() {
        let token = &argv[index];
        if token == "--" {
            index += 1;
            continue;
        }
        if token == "--help" || token == "-h" {
            return Ok(CliResult::Help);
        }
        if token == AUTOSTART_ARG {
            result.started_from_autostart = true;
            index += 1;
            continue;
        }

        let (name, inline_value) = token
            .split_once('=')
            .map_or((token.as_str(), None), |(name, value)| (name, Some(value)));
        if name != "--host" && name != "--port" {
            return Err(format!("unknown option: {token}"));
        }

        let value = if let Some(value) = inline_value {
            value.to_string()
        } else {
            index += 1;
            argv.get(index)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))?
        };
        if value.is_empty() || value.starts_with("--") {
            return Err(format!("{name} requires a value"));
        }

        if name == "--host" {
            result.host = Some(value);
        } else {
            result.port = Some(
                parse_port(&value)
                    .ok_or_else(|| "--port must be an integer between 0 and 65535".to_string())?,
            );
        }
        index += 1;
    }
    Ok(CliResult::Overrides(result))
}

fn parse_port(value: &str) -> Option<u16> {
    if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    value.parse::<u16>().ok()
}

fn notifier_help() -> String {
    format!(
        "Usage: attentive-desktop [options]\n\nStarts the Windows notification HTTP service.\n\nOptions:\n  --host <address>  Listen address (default: {DEFAULT_NOTIFIER_HOST})\n  --port <port>     Listen port (default: {DEFAULT_NOTIFIER_PORT})\n  -h, --help        Show this help"
    )
}

fn bind_address(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn local_endpoint(host: &str, port: u16) -> String {
    let display_host = match host {
        "0.0.0.0" | "::" => "127.0.0.1",
        host => host.trim_matches(['[', ']']),
    };
    if display_host.contains(':') {
        format!("http://[{display_host}]:{port}")
    } else {
        format!("http://{display_host}:{port}")
    }
}

fn lan_endpoint(host: &str, port: u16, local_url: &str) -> Option<String> {
    if !host_accepts_lan_connections(host) {
        return None;
    }

    let ip = lan_ipv4()?;
    let endpoint = local_endpoint(&ip.to_string(), port);
    (endpoint != local_url).then_some(endpoint)
}

fn host_accepts_lan_connections(host: &str) -> bool {
    let normalized = host.trim().trim_matches(['[', ']']);
    match normalized.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_unspecified() || !address.is_loopback(),
        Ok(IpAddr::V6(address)) => address.is_unspecified() || !address.is_loopback(),
        Err(_) => false,
    }
}

fn lan_ipv4() -> Option<Ipv4Addr> {
    // Connecting a UDP socket does not send a packet. The OS only uses the
    // destination to select the active network interface, so the destination
    // does not need to accept traffic.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(address) if !address.is_unspecified() && !address.is_loopback() => Some(address),
        _ => None,
    }
}

#[cfg(desktop)]
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn default_host() -> String {
    DEFAULT_NOTIFIER_HOST.to_string()
}

fn default_port() -> u16 {
    DEFAULT_NOTIFIER_PORT
}

fn default_max_body_bytes() -> usize {
    DEFAULT_MAX_BODY_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use attentive_notifier::{validate_notification_request, HEALTH_PATH, NOTIFICATIONS_PATH};
    use serde_json::{json, Value};
    use std::{
        io::{Read, Write},
        net::{SocketAddr, TcpStream},
        sync::Mutex as StdMutex,
        time::Duration,
    };

    #[test]
    fn validates_the_protocol_shape() {
        let request = validate_notification_request(json!({
            "title": "Build complete",
            "body": "The build passed",
            "source": "test",
            "action": {
                "type": "open-uri",
                "uri": "vscode://32757468.attentive-vscode/focus"
            },
            "metadata": { "buildId": 1 }
        }))
        .expect("valid request");

        assert_eq!(request.title, "Build complete");
        assert_eq!(request.body, "The build passed");
        assert_eq!(request.source.as_deref(), Some("test"));
        assert_eq!(
            request.action.expect("action").uri,
            "vscode://32757468.attentive-vscode/focus"
        );
    }

    #[test]
    fn keeps_protocol_validation_messages_stable() {
        assert_eq!(
            validate_notification_request(json!({ "title": "Missing body" }))
                .unwrap_err()
                .message,
            "body is required"
        );
        assert_eq!(
            validate_notification_request(json!({
                "title": "Build",
                "body": "Done",
                "url": "https://example.com"
            }))
            .unwrap_err()
            .message,
            "url is no longer supported; use action"
        );
    }

    #[test]
    fn supports_cli_port_zero_and_inline_values() {
        let CliResult::Overrides(overrides) = parse_cli(vec![
            "--host=127.0.0.1".to_string(),
            "--port".to_string(),
            "0".to_string(),
        ])
        .expect("valid args") else {
            panic!("expected overrides");
        };

        assert_eq!(overrides.host.as_deref(), Some("127.0.0.1"));
        assert_eq!(overrides.port, Some(0));
    }

    #[test]
    fn recognizes_autostart_launch_argument() {
        let CliResult::Overrides(overrides) =
            parse_cli(vec![AUTOSTART_ARG.to_string()]).expect("valid args")
        else {
            panic!("expected overrides");
        };

        assert!(overrides.started_from_autostart);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serves_the_notifier_http_protocol() {
        let submitted = Arc::new(StdMutex::new(Vec::<(NotificationRequest, String)>::new()));
        let submitted_for_dispatch = Arc::clone(&submitted);
        let options = NotifierServerOptions::default().with_sync_dispatcher(
            move |request, notification_id| {
                let submitted = Arc::clone(&submitted_for_dispatch);
                submitted
                    .lock()
                    .expect("dispatcher capture lock")
                    .push((request, notification_id));
                Ok(())
            },
        );
        let (address, task) = start_test_server(options).await;

        let (status, payload) = raw_http_request(
            address,
            "POST",
            NOTIFICATIONS_PATH,
            r#"{"title":"Build complete","body":"The build passed","source":"test"}"#,
        );
        assert_eq!(status, 201);
        let notification_id = payload["notificationId"].as_str().expect("notification id");
        assert!(Uuid::parse_str(notification_id).is_ok());
        assert_eq!(submitted.lock().expect("dispatcher capture lock").len(), 1);

        let (status, payload) = raw_http_request(address, "GET", HEALTH_PATH, "");
        assert_eq!(status, 200);
        assert_eq!(payload, json!({ "status": "ok" }));

        let (status, payload) = raw_http_request(address, "GET", NOTIFICATIONS_PATH, "");
        assert_eq!(status, 405);
        assert_eq!(payload["error"]["code"], "METHOD_NOT_ALLOWED");

        let (status, payload) = raw_http_request(address, "POST", NOTIFICATIONS_PATH, "not json");
        assert_eq!(status, 400);
        assert_eq!(payload["error"]["code"], "INVALID_REQUEST");

        let (status, payload) = raw_http_request(address, "GET", "/not-found", "");
        assert_eq!(status, 404);
        assert_eq!(payload["error"]["code"], "NOT_FOUND");

        task.abort();
        let _ = task.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn returns_payload_too_large_and_dispatcher_errors() {
        let options = NotifierServerOptions::default()
            .with_max_body_bytes(64)
            .with_sync_dispatcher(|_request, _notification_id| {
                Err("synthetic dispatcher failure".to_string())
            });
        let (address, task) = start_test_server(options).await;

        let (status, payload) = raw_http_request(
            address,
            "POST",
            NOTIFICATIONS_PATH,
            r#"{"title":"Build complete","body":"This body is intentionally much too large for the configured limit"}"#,
        );
        assert_eq!(status, 413);
        assert_eq!(payload["error"]["code"], "INVALID_REQUEST");

        let (status, payload) = raw_http_request(
            address,
            "POST",
            NOTIFICATIONS_PATH,
            r#"{"title":"Build complete","body":"Done"}"#,
        );
        assert_eq!(status, 500);
        assert_eq!(payload["error"]["code"], "INTERNAL_ERROR");
        assert_eq!(payload["error"]["message"], "unable to submit notification");

        task.abort();
        let _ = task.await;
    }

    async fn start_test_server(
        options: NotifierServerOptions,
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let address = listener.local_addr().expect("test listener address");
        let router = create_notifier_router(options);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        tokio::task::yield_now().await;
        (address, task)
    }

    fn raw_http_request(address: SocketAddr, method: &str, path: &str, body: &str) -> (u16, Value) {
        let mut stream = TcpStream::connect(address).expect("connect test listener");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set test read timeout");
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .expect("write test request");

        let mut response = Vec::new();
        let header_end = loop {
            let mut buffer = [0u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("read test response");
            assert!(bytes_read > 0, "response ended before headers");
            response.extend_from_slice(&buffer[..bytes_read]);
            if let Some(position) = response.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&response[..header_end]).into_owned();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .expect("response content length");
        while response.len() < header_end + content_length {
            let mut buffer = [0u8; 4096];
            let bytes_read = stream.read(&mut buffer).expect("read test body");
            assert!(bytes_read > 0, "response ended before body");
            response.extend_from_slice(&buffer[..bytes_read]);
        }
        let status = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse::<u16>().ok())
            .expect("response status");
        let payload = serde_json::from_slice(&response[header_end..]).expect("JSON response");
        (status, payload)
    }
}
