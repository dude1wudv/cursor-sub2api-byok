#[cfg(not(dev))]
use crate::frontend;
use crate::startup::{self, StartupDiagnostics};
use crate::tray;
use cursor_server::{
    config::RuntimePaths,
    local_app::{instance::ControllerInstance, CursorHarness},
    App, Config, Result,
};
use std::{
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{
    async_runtime::JoinHandle, AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};
use tokio_util::sync::CancellationToken;
pub(crate) const MAIN_WINDOW_LABEL: &str = "main";

fn parse_args(args: impl Iterator<Item = std::ffi::OsString>) -> Result<(RuntimePaths, bool)> {
    let mut args = args;
    let (mut data, mut cursor, mut restore) = (None, None, false);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--restore") if !restore => restore = true,
            Some("--data-dir") if data.is_none() => {
                data = Some(std::path::PathBuf::from(args.next().ok_or_else(|| {
                    cursor_server::Error::Config("--data-dir requires an absolute path".into())
                })?))
            }
            Some("--cursor-user-data-dir") if cursor.is_none() => {
                cursor = Some(std::path::PathBuf::from(args.next().ok_or_else(|| {
                    cursor_server::Error::Config(
                        "--cursor-user-data-dir requires an absolute path".into(),
                    )
                })?))
            }
            _ => {
                return Err(cursor_server::Error::Config(
                    "unknown or duplicate startup argument".into(),
                ))
            }
        }
    }
    Ok((RuntimePaths::resolve(data, cursor)?, restore))
}

struct DesktopRuntime {
    shutdown: CancellationToken,
    server: Mutex<Option<JoinHandle<Result<()>>>>,
    exiting: AtomicBool,
    exit_ready: AtomicBool,
    server_addr: std::net::SocketAddr,
    control_token: String,
    webview_data: std::path::PathBuf,
    harness: CursorHarness,
}

pub(crate) fn open_main_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
        return Ok(());
    }
    let state = app.state::<DesktopRuntime>();
    let address = state.server_addr;
    let url: url::Url = format!("http://{address}/__byok-api__/")
        .parse()
        .expect("loopback URL");
    let allowed = url.origin().ascii_serialization();
    let origin_json = serde_json::to_string(&allowed).expect("origin JSON");
    let token = serde_json::to_string(&state.control_token).expect("token JSON");
    WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::External(url))
        .title("Cursor Sub2API BYOK")
        .inner_size(940.0, 760.0).min_inner_size(780.0, 560.0)
        .center().decorations(true).resizable(true)
        .data_directory(state.webview_data.clone())
        .initialization_script(format!("if (location.origin === {origin_json}) Object.defineProperty(window, '__SUB2API_CONTROL_TOKEN__', {{value: {token}, writable: false, configurable: false, enumerable: false}});"))
        .on_navigation(move |url| {
            if url.origin().ascii_serialization() == allowed { return true; }
            if matches!(url.as_str(), "https://microedulab.com/" | "https://github.com/dude1wudv/cursor-sub2api-byok" | "https://github.com/leookun/cursor-byok") {
                open_about_link(url.as_str());
            }
            false
        })
        .build()?;
    Ok(())
}

// Only fixed About links reach this helper; external pages never receive the control token.
fn open_about_link(url: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let target: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }
    #[cfg(not(windows))]
    let _ = url;
}

pub fn run() -> ExitCode {
    let (paths, restore) = match parse_args(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let _instance = match ControllerInstance::acquire() {
        Ok(guard) => guard,
        Err(error) => {
            if restore {
                eprintln!("{error}");
            } else {
                show_error(&error.to_string());
            }
            return ExitCode::FAILURE;
        }
    };
    if restore {
        return match tauri::async_runtime::block_on(cursor_server::local_app::restore_paths(&paths))
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Restore failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let diagnostics = match StartupDiagnostics::initialize(&paths.data_dir) {
        Ok(value) => value,
        Err(error) => {
            startup::report_logging_failure(error.as_ref());
            return ExitCode::FAILURE;
        }
    };
    let app = tauri::Builder::default()
        .setup(move |app| {
            let mut config = Config::desktop_with_paths(paths.clone())?;
            config.app_version = env!("CARGO_PKG_VERSION").into();
            #[cfg(dev)]
            {
                config.console = Some(cursor_server::config::ConsoleSource::Proxy(
                    "http://127.0.0.1:1420".parse()?,
                ));
            }
            let server = tauri::async_runtime::block_on(App::new(config))?;
            #[cfg(not(dev))]
            let server = server.merge_router(frontend::router(app.handle().clone()));
            let listener = tauri::async_runtime::block_on(server.bind())?;
            let address = listener.local_addr()?;
            let token = server.control_token().to_owned();
            let harness = server.harness();
            if let Err(error) = tauri::async_runtime::block_on(harness.recover_pending()) {
                tracing::warn!(%error, "pending recovery requires user action");
            }
            let shutdown = CancellationToken::new();
            let server_shutdown = shutdown.clone();
            let task = tauri::async_runtime::spawn(server.serve_on(listener, server_shutdown));
            app.manage(DesktopRuntime {
                shutdown,
                server: Mutex::new(Some(task)),
                exiting: AtomicBool::new(false),
                exit_ready: AtomicBool::new(false),
                server_addr: address,
                control_token: token,
                webview_data: paths.data_dir.join("webview2"),
                harness,
            });
            open_main_window(app.handle())?;
            tray::create(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    tracing::warn!(%error, "could not hide controller window to tray");
                }
            }
        })
        .build(tauri::generate_context!());
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            diagnostics.report_fatal(&error);
            return ExitCode::FAILURE;
        }
    };
    app.run(|app, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            let runtime = app.state::<DesktopRuntime>();
            if runtime.exit_ready.load(Ordering::Acquire) {
                return;
            }
            api.prevent_exit();
            if runtime.exiting.swap(true, Ordering::AcqRel) {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let runtime = app.state::<DesktopRuntime>();
                if let Err(error) = runtime.harness.disable().await {
                    runtime.exiting.store(false, Ordering::Release);
                    let _ = open_main_window(&app);
                    show_error(&format!("退出已取消。\n{error}"));
                    return;
                }
                runtime.shutdown.cancel();
                let server = runtime.server.lock().expect("server lock").take();
                if let Some(server) = server {
                    match tokio::time::timeout(Duration::from_secs(12), server).await {
                        Ok(Ok(Ok(()))) => {}
                        _ => tracing::warn!("server shutdown did not complete cleanly"),
                    }
                }
                runtime.exit_ready.store(true, Ordering::Release);
                app.exit(0);
            });
        }
    });
    ExitCode::SUCCESS
}
fn show_error(message: &str) {
    rfd::MessageDialog::new()
        .set_title("Cursor Sub2API BYOK")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
