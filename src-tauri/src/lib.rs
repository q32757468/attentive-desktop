use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, Method, StatusCode},
    response::Response,
    routing::any,
    Router,
};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
#[cfg(windows)]
use std::process::Command;
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, UdpSocket},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, RwLock,
    },
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

const NOTIFICATIONS_PATH: &str = "/api/v1/notifications";
const HEALTH_PATH: &str = "/health";
const DEFAULT_NOTIFIER_HOST: &str = "0.0.0.0";
const DEFAULT_NOTIFIER_PORT: u16 = 8765;
const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_OPEN_URI_LENGTH: usize = 4096;
#[cfg(windows)]
const APP_USER_MODEL_ID: &str = "Attentive.Desktop";
const APP_DISPLAY_NAME: &str = "Attentive Desktop";

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
    body_limit: Arc<AtomicUsize>,
    config_warning: RwLock<Option<String>>,
    config_path: PathBuf,
}

struct ServerHandle {
    task: JoinHandle<()>,
}

#[derive(Clone)]
struct HttpState {
    max_body_bytes: Arc<AtomicUsize>,
    dispatcher: NotificationDispatcher,
}

#[derive(Debug, Clone)]
struct NotificationRequest {
    #[cfg_attr(not(windows), allow(dead_code))]
    title: String,
    #[cfg_attr(not(windows), allow(dead_code))]
    body: String,
    source: Option<String>,
    #[cfg_attr(not(windows), allow(dead_code))]
    action: Option<OpenUriAction>,
    _metadata: Option<Map<String, Value>>,
}

#[derive(Debug, Clone)]
struct OpenUriAction {
    #[cfg_attr(not(windows), allow(dead_code))]
    uri: String,
}

#[derive(Debug, Clone, Default)]
struct CliOverrides {
    host: Option<String>,
    port: Option<u16>,
}

type DispatchFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
type NotificationDispatcher =
    Arc<dyn Fn(NotificationRequest, String) -> DispatchFuture + Send + Sync>;

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
async fn test_notification() -> Result<(), String> {
    let request = NotificationRequest {
        title: APP_DISPLAY_NAME.to_string(),
        body: "通知服务连接正常。你可以开始接收来自编辑器和脚本的提醒。".to_string(),
        source: Some("settings".to_string()),
        action: None,
        _metadata: None,
    };
    dispatch_notification(request, Uuid::new_v4().to_string()).await
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
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }));
        builder = builder.plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(Vec::<&str>::new()),
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

            #[cfg(windows)]
            register_toast_app();

            let state = Arc::new(RuntimeState {
                status: RwLock::new(ServerStatus::offline(&config, config_warning.clone())),
                config: RwLock::new(config.clone()),
                server: Mutex::new(None),
                restart_lock: Mutex::new(()),
                body_limit: Arc::new(AtomicUsize::new(config.max_body_bytes)),
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

    let can_reuse_listener = {
        let current_config = state
            .config
            .read()
            .map_err(|_| "配置锁不可用".to_string())?;
        let status = state
            .status
            .read()
            .map_err(|_| "状态锁不可用".to_string())?;
        status.running
            && config.port != 0
            && current_config.host == config.host
            && current_config.port == config.port
    };

    if can_reuse_listener {
        state
            .body_limit
            .store(config.max_body_bytes, Ordering::Relaxed);
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

    // Bind the replacement before touching the current server. A failed bind
    // must leave the existing notifier available for the caller.
    let listener = TcpListener::bind(bind_address(&config.host, config.port))
        .await
        .map_err(|error| format!("无法监听 {}:{}: {error}", config.host, config.port))?;
    let local_address = listener
        .local_addr()
        .map_err(|error| format!("无法读取服务地址: {error}"))?;

    let old_server = {
        let mut server = state.server.lock().await;
        server.take()
    };
    if let Some(old_server) = old_server {
        old_server.task.abort();
    }

    state
        .body_limit
        .store(config.max_body_bytes, Ordering::Relaxed);
    let router = build_router(HttpState {
        max_body_bytes: Arc::clone(&state.body_limit),
        dispatcher: default_dispatcher(),
    });
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

fn build_router(state: HttpState) -> Router {
    Router::new()
        .route(HEALTH_PATH, any(handle_health))
        .route(NOTIFICATIONS_PATH, any(handle_notifications))
        .fallback(handle_not_found)
        .with_state(state)
}

fn default_dispatcher() -> NotificationDispatcher {
    Arc::new(|request, notification_id| Box::pin(dispatch_notification(request, notification_id)))
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

async fn handle_health(request: Request) -> Response {
    if request.method() != Method::GET {
        return method_not_allowed();
    }
    json_response(StatusCode::OK, json!({ "status": "ok" }))
}

async fn handle_notifications(State(state): State<HttpState>, request: Request) -> Response {
    if request.method() != Method::POST {
        return method_not_allowed();
    }

    let max_body_bytes = state.max_body_bytes.load(Ordering::Relaxed);
    let body = match read_body(request.into_body(), max_body_bytes).await {
        Ok(body) => body,
        Err(error) => {
            return json_response(error.status, api_error("INVALID_REQUEST", error.message))
        }
    };

    let value = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(_) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                api_error("INVALID_REQUEST", "request body must be valid JSON"),
            )
        }
    };
    let notification = match validate_notification_request(value) {
        Ok(notification) => notification,
        Err(message) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                api_error("INVALID_REQUEST", message),
            )
        }
    };

    let notification_id = Uuid::new_v4().to_string();
    if let Err(error) = (state.dispatcher)(notification.clone(), notification_id.clone()).await {
        eprintln!(
            "Unable to submit notification: notificationId={notification_id} source={:?} error={error}",
            notification.source
        );
        return json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            api_error("INTERNAL_ERROR", "unable to submit notification"),
        );
    }

    println!(
        "Notification submitted: notificationId={notification_id}{}",
        notification
            .source
            .as_ref()
            .map(|source| format!(" source={source}"))
            .unwrap_or_default()
    );
    json_response(
        StatusCode::CREATED,
        json!({ "notificationId": notification_id }),
    )
}

async fn handle_not_found(_request: Request) -> Response {
    json_response(
        StatusCode::NOT_FOUND,
        api_error("NOT_FOUND", "route not found"),
    )
}

async fn read_body(mut body: Body, max_body_bytes: usize) -> Result<Vec<u8>, RequestBodyError> {
    let mut chunks = Vec::new();
    let mut byte_length = 0usize;

    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| RequestBodyError::bad_request())?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        byte_length = byte_length.saturating_add(data.len());
        if byte_length > max_body_bytes {
            return Err(RequestBodyError::too_large());
        }
        chunks.extend_from_slice(&data);
    }

    if chunks.is_empty() {
        return Err(RequestBodyError::bad_request());
    }
    Ok(chunks)
}

async fn dispatch_notification(
    request: NotificationRequest,
    _notification_id: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || show_windows_toast(request))
            .await
            .map_err(|error| format!("Windows notification worker failed: {error}"))??;
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let _ = request;
        Err("Windows notifications require a Windows notifier host".to_string())
    }
}

#[cfg(windows)]
fn register_toast_app() {
    let _ = winrt_toast_reborn::register(APP_USER_MODEL_ID, APP_DISPLAY_NAME, None);
}

#[cfg(windows)]
fn show_windows_toast(request: NotificationRequest) -> Result<(), String> {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use winrt_toast_reborn::{Toast, ToastManager};

    let mut toast = Toast::new();
    toast.text1(request.title).text2(request.body);

    let opened = Arc::new(AtomicBool::new(false));
    let action_uri = request.action.map(|action| action.uri);
    if let Some(uri) = action_uri.as_deref() {
        toast.launch(uri);
    }
    let opened_on_activation = Arc::clone(&opened);
    let manager = ToastManager::new(APP_USER_MODEL_ID).on_activated(None, move |_action| {
        if let Some(uri) = action_uri.as_deref() {
            if !opened_on_activation.swap(true, Ordering::SeqCst) {
                let _ = open_uri_with_explorer(uri);
            }
        }
    });

    manager
        .show(&toast)
        .map_err(|error| format!("Windows toast submission failed: {error}"))
}

#[cfg(windows)]
fn open_uri_with_explorer(uri: &str) -> Result<(), String> {
    Command::new("explorer.exe")
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("unable to open notification action: {error}"))
}

fn validate_notification_request(value: Value) -> Result<NotificationRequest, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "request body must be a JSON object".to_string())?;
    let title = require_non_empty_string(object.get("title"), "title")?;
    let body = require_non_empty_string(object.get("body"), "body")?;

    if object.contains_key("url") {
        return Err("url is no longer supported; use action".to_string());
    }

    let source = object
        .get("source")
        .map(|value| require_string(Some(value), "source"))
        .transpose()?;

    let action = object.get("action").map(validate_action).transpose()?;

    let metadata = object
        .get("metadata")
        .map(|value| {
            value
                .as_object()
                .cloned()
                .ok_or_else(|| "metadata must be a JSON object".to_string())
        })
        .transpose()?;

    Ok(NotificationRequest {
        title,
        body,
        source,
        action,
        _metadata: metadata,
    })
}

fn validate_action(value: &Value) -> Result<OpenUriAction, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "action must be a JSON object".to_string())?;
    if object.get("type") != Some(&Value::String("open-uri".to_string())) {
        return Err("action.type must be \"open-uri\"".to_string());
    }
    let uri = require_string(object.get("uri"), "action.uri")?;
    if !is_open_uri(&uri) {
        return Err(format!(
            "action.uri must use http, https, or vscode and be at most {MAX_OPEN_URI_LENGTH} characters"
        ));
    }
    Ok(OpenUriAction { uri })
}

fn require_non_empty_string(value: Option<&Value>, field: &str) -> Result<String, String> {
    let result = require_string(value, field)?;
    if result.trim().is_empty() {
        return Err(format!("{field} is required"));
    }
    Ok(result)
}

fn require_string(value: Option<&Value>, field: &str) -> Result<String, String> {
    match value {
        None => Err(format!("{field} is required")),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("{field} must be a string")),
    }
}

fn is_open_uri(value: &str) -> bool {
    if value.is_empty() || value.encode_utf16().count() > MAX_OPEN_URI_LENGTH {
        return false;
    }
    let Ok(uri) = url::Url::parse(value) else {
        return false;
    };
    matches!(uri.scheme(), "http" | "https" | "vscode")
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

fn api_error(code: &str, message: impl Into<String>) -> Value {
    json!({ "error": { "code": code, "message": message.into() } })
}

fn method_not_allowed() -> Response {
    json_response(
        StatusCode::METHOD_NOT_ALLOWED,
        api_error("METHOD_NOT_ALLOWED", "method not allowed"),
    )
}

fn json_response(status: StatusCode, payload: Value) -> Response {
    let body = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CONTENT_LENGTH, body.len())
        .body(Body::from(body))
        .expect("valid JSON response headers")
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

#[derive(Debug)]
struct RequestBodyError {
    status: StatusCode,
    message: &'static str,
}

impl RequestBodyError {
    fn bad_request() -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: "request body must be valid JSON",
        }
    }

    fn too_large() -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            message: "request body is too large",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            validate_notification_request(json!({ "title": "Missing body" })).unwrap_err(),
            "body is required"
        );
        assert_eq!(
            validate_notification_request(json!({
                "title": "Build",
                "body": "Done",
                "url": "https://example.com"
            }))
            .unwrap_err(),
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serves_the_notifier_http_protocol() {
        let submitted = Arc::new(StdMutex::new(Vec::<(NotificationRequest, String)>::new()));
        let submitted_for_dispatch = Arc::clone(&submitted);
        let dispatcher: NotificationDispatcher = Arc::new(move |request, notification_id| {
            let submitted = Arc::clone(&submitted_for_dispatch);
            Box::pin(async move {
                submitted
                    .lock()
                    .expect("dispatcher capture lock")
                    .push((request, notification_id));
                Ok(())
            })
        });
        let (address, task) = start_test_server(DEFAULT_MAX_BODY_BYTES, dispatcher).await;

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
        let dispatcher: NotificationDispatcher = Arc::new(|_request, _notification_id| {
            Box::pin(async { Err("synthetic dispatcher failure".to_string()) })
        });
        let (address, task) = start_test_server(64, dispatcher).await;

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
        max_body_bytes: usize,
        dispatcher: NotificationDispatcher,
    ) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener");
        let address = listener.local_addr().expect("test listener address");
        let state = HttpState {
            max_body_bytes: Arc::new(AtomicUsize::new(max_body_bytes)),
            dispatcher,
        };
        let router = build_router(state);
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
