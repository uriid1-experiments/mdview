//! mdview — a desktop Markdown viewer/editor built on GitHub's rendering stack
//! (comrak + sanitizer + github-markdown-css), shown in a WebKitGTK webview via wry/tao.
//!
//! The page is a single-page shell (menu bar, editor, preview). Rust owns the document
//! state, config and file I/O; the page talks to it over IPC and the `mdview://` protocol.

mod render;
mod vendor;

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use notify::{EventKind, RecursiveMode, Watcher};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowBuilder};
use gtk::prelude::*;
use tao::platform::unix::WindowExtUnix;
use wry::http::{header::CONTENT_TYPE, Request, Response};
use wry::{DragDropEvent, PageLoadEvent, WebView, WebViewBuilder, WebViewBuilderExtUnix};

use render::Renderer;

const ROOT_URL: &str = "mdview://localhost/";
const SHELL_HTML: &str = include_str!("../assets/index.html");
const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");
const MAX_RECENT: usize = 10;

#[derive(Debug)]
enum UserEvent {
    Ipc(Value),
    Drop(PathBuf),
    External(String),
    FileChanged,
    PageLoaded,
    OpenPicked(Option<PathBuf>),
    SavePicked(Option<PathBuf>),
}

/// Persisted in `$XDG_CONFIG_HOME/mdview/config.json`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
struct Config {
    theme: String, // system | light | dark
    menu: bool,
    mode: String, // preview | split | editor
    zoom: f64,
    recent: Vec<String>,
    window: Option<[u32; 2]>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            menu: true,
            mode: "preview".into(),
            zoom: 1.0,
            recent: Vec::new(),
            window: None,
        }
    }
}

struct State {
    path: Option<PathBuf>,
    dirty: bool,
    /// Hash of the content we wrote last, so our own saves don't trigger "file changed".
    saved_hash: Option<u64>,
    config: Config,
}

/// Shared with the protocol handler (runs on webkit's threads).
struct Shared {
    state: Mutex<State>,
    renderer: Renderer,
    markdown_css: String,
    webkit_version: String,
}

impl Shared {
    fn base_dir(&self) -> PathBuf {
        self.state
            .lock()
            .unwrap()
            .path
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn settings_json(&self) -> Value {
        let st = self.state.lock().unwrap();
        json!({
            "theme": st.config.theme,
            "menu": st.config.menu,
            "mode": st.config.mode,
            "zoom": st.config.zoom,
        })
    }

    fn shell_page(&self) -> String {
        let initial = json!({
            "settings": self.settings_json(),
            "about": {
                "version": env!("CARGO_PKG_VERSION"),
                "webkit": self.webkit_version,
            },
        });
        let initial = initial.to_string().replace('<', "\\u003c");
        SHELL_HTML
            .replace("{{GH_CSS}}", &self.markdown_css)
            .replace("{{APP_CSS}}", APP_CSS)
            .replace("{{APP_JS}}", APP_JS)
            .replace("{{INITIAL}}", &initial)
    }

    /// The current document as JSON: source, rendered HTML and the recent-files list.
    fn doc_json(&self) -> Value {
        let st = self.state.lock().unwrap();
        let recent: Vec<Value> = st
            .config
            .recent
            .iter()
            .map(|p| json!({ "path": p, "display": display_path(Path::new(p)) }))
            .collect();
        let Some(path) = &st.path else {
            return json!({
                "path": null, "name": "", "dir": "", "display": "",
                "source": "", "html": "", "error": null, "recent": recent,
            });
        };
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let (source, html, error) = match fs::read(path) {
            Ok(bytes) => {
                let source = String::from_utf8_lossy(&bytes).into_owned();
                let html = self.renderer.render_markdown(&source, &dir);
                (source, html, Value::Null)
            }
            Err(e) => (
                String::new(),
                String::new(),
                json!(format!("Не удалось прочитать {}: {}", path.display(), e)),
            ),
        };
        json!({
            "path": path.to_string_lossy(),
            "name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            "dir": dir.to_string_lossy(),
            "display": display_path(path),
            "source": source,
            "html": html,
            "error": error,
            "recent": recent,
        })
    }
}

/// Everything the event loop needs.
struct App {
    shared: Arc<Shared>,
    webview: WebView,
    window: Window,
    watch: Watch,
    proxy: EventLoopProxy<UserEvent>,
    config_path: PathBuf,
    window_size: Option<[u32; 2]>,
    debug_script: Option<String>,
    /// The open/save dialog currently shown (kept alive until it responds).
    dialog: Option<gtk::FileChooserNative>,
    /// Editor content waiting for the "save as" dialog.
    pending_save: Option<String>,
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let dump = args.iter().position(|a| a == "--dump").map(|i| args.remove(i));
    let list_syntaxes = args.iter().position(|a| a == "--syntaxes").map(|i| args.remove(i));
    let initial = args
        .into_iter()
        .next()
        .map(PathBuf::from)
        .map(|p| p.canonicalize().unwrap_or(p));

    let renderer = Renderer::new();

    if list_syntaxes.is_some() {
        for name in renderer.syntax_names() {
            println!("{name}");
        }
        return;
    }

    // `mdview --dump file.md` prints the rendered HTML fragment to stdout.
    if dump.is_some() {
        if let Some(p) = &initial {
            let base = p.parent().unwrap_or_else(|| Path::new("."));
            match fs::read(p) {
                Ok(bytes) => print!(
                    "{}",
                    renderer.render_markdown(&String::from_utf8_lossy(&bytes), base)
                ),
                Err(e) => eprintln!("{}: {}", p.display(), e),
            }
        }
        return;
    }

    let config_path = config_path();
    let mut config = load_config(&config_path);
    if let Some(p) = &initial {
        push_recent(&mut config.recent, p);
    }

    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            path: initial.clone(),
            dirty: false,
            saved_hash: None,
            config: config.clone(),
        }),
        renderer,
        markdown_css: render::themed_github_css(),
        webkit_version: wry::webview_version().unwrap_or_else(|_| "?".into()),
    });

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // MDVIEW_WINDOW_SIZE=WxH overrides the saved window size.
    let (w, h) = std::env::var("MDVIEW_WINDOW_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .or_else(|| config.window.map(|[w, h]| (w as f64, h as f64)))
        .unwrap_or((1100.0, 820.0));

    let window = WindowBuilder::new()
        .with_title("mdview")
        .with_inner_size(LogicalSize::new(w, h))
        .build(&event_loop)
        .expect("create window");

    let builder = {
        let shared = shared.clone();
        let p_ipc = proxy.clone();
        let p_drop = proxy.clone();
        let p_nav = proxy.clone();
        let p_load = proxy.clone();
        WebViewBuilder::new()
            .with_url(ROOT_URL)
            .with_devtools(cfg!(debug_assertions))
            .with_custom_protocol("mdview".into(), move |_id, req| serve(&shared, req))
            .with_ipc_handler(move |req| {
                if let Ok(v) = serde_json::from_str::<Value>(req.body()) {
                    let _ = p_ipc.send_event(UserEvent::Ipc(v));
                }
            })
            .with_drag_drop_handler(move |event| {
                if let DragDropEvent::Drop { paths, .. } = event {
                    if let Some(p) = paths.into_iter().next() {
                        let _ = p_drop.send_event(UserEvent::Drop(p));
                    }
                    return true;
                }
                false
            })
            .with_navigation_handler(move |url| {
                if url.starts_with("mdview://") {
                    true
                } else {
                    let _ = p_nav.send_event(UserEvent::External(url));
                    false
                }
            })
            .with_on_page_load_handler(move |event, _url| {
                if matches!(event, PageLoadEvent::Finished) {
                    let _ = p_load.send_event(UserEvent::PageLoaded);
                }
            })
    };

    let webview = builder
        .build_gtk(window.default_vbox().expect("tao gtk vbox"))
        .expect("create webview");

    if (config.zoom - 1.0).abs() > f64::EPSILON {
        let _ = webview.zoom(config.zoom);
    }

    let mut app = App {
        shared,
        webview,
        window,
        watch: Watch::new(proxy.clone()),
        proxy: proxy.clone(),
        config_path,
        window_size: config.window,
        // MDVIEW_DEBUG_SCRIPT=file.js runs after every page load (used for automated UI checks).
        debug_script: std::env::var_os("MDVIEW_DEBUG_SCRIPT")
            .and_then(|p| fs::read_to_string(p).ok()),
        dialog: None,
        pending_save: None,
    };
    app.watch.watch(initial.as_deref());
    app.update_title();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if app.shared.state.lock().unwrap().dirty {
                    app.emit(json!({ "type": "closeRequested" }));
                } else {
                    app.persist();
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                let logical: LogicalSize<f64> = size.to_logical(app.window.scale_factor());
                if logical.width > 100.0 && logical.height > 100.0 {
                    app.window_size = Some([logical.width as u32, logical.height as u32]);
                }
            }
            Event::UserEvent(ev) => match ev {
                UserEvent::Ipc(msg) => app.handle_ipc(msg, control_flow),
                UserEvent::Drop(path) => app.request_open(&path),
                UserEvent::External(url) => open_external(&url),
                UserEvent::FileChanged => app.on_file_changed(),
                UserEvent::PageLoaded => {
                    if let Some(js) = &app.debug_script {
                        let _ = app.webview.evaluate_script(js);
                    }
                }
                UserEvent::OpenPicked(path) => {
                    app.dialog = None;
                    if let Some(p) = path {
                        app.open_file(&p);
                    }
                }
                UserEvent::SavePicked(path) => {
                    app.dialog = None;
                    let content = app.pending_save.take().unwrap_or_default();
                    match path {
                        Some(mut p) => {
                            if p.extension().is_none() {
                                p.set_extension("md");
                            }
                            app.save_to(&p, &content);
                        }
                        None => app.emit(json!({ "type": "saveCancelled" })),
                    }
                }
            },
            _ => {}
        }
    });
}

impl App {
    /// Send an event to the page: `window.__mdview.dispatch({...})`.
    fn emit(&self, event: Value) {
        let js = format!("window.__mdview && window.__mdview.dispatch({event});");
        let _ = self.webview.evaluate_script(&js);
    }

    fn toast(&self, message: impl Into<String>) {
        self.emit(json!({ "type": "toast", "message": message.into() }));
    }

    fn current_path(&self) -> Option<PathBuf> {
        self.shared.state.lock().unwrap().path.clone()
    }

    fn update_title(&self) {
        let st = self.shared.state.lock().unwrap();
        let name = st
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Без имени".into());
        let title = format!("{}{} — mdview", if st.dirty { "● " } else { "" }, name);
        self.window.set_title(&title);
    }

    fn persist(&self) {
        let mut st = self.shared.state.lock().unwrap();
        st.config.window = self.window_size;
        save_config(&self.config_path, &st.config);
    }

    fn handle_ipc(&mut self, msg: Value, control_flow: &mut ControlFlow) {
        let cmd = msg["cmd"].as_str().unwrap_or("");
        let string = |key: &str| msg[key].as_str().map(str::to_string);
        match cmd {
            "open" => self.show_open_dialog(),
            "openFile" => {
                if let Some(p) = string("path") {
                    self.open_file(Path::new(&p));
                }
            }
            "resolve" => {
                if let Some(href) = string("href") {
                    self.resolve_link(&href);
                }
            }
            "external" => {
                if let Some(url) = string("url") {
                    open_external(&url);
                }
            }
            "new" | "close" => self.close_file(),
            "save" => {
                let content = string("content").unwrap_or_default();
                match self.current_path() {
                    Some(p) => self.save_to(&p, &content),
                    None => self.save_as(&content),
                }
            }
            "saveAs" => self.save_as(&string("content").unwrap_or_default()),
            "reload" => self.emit(json!({ "type": "doc" })),
            "dirty" => {
                self.shared.state.lock().unwrap().dirty = msg["value"].as_bool().unwrap_or(false);
                self.update_title();
            }
            "settings" => {
                let mut st = self.shared.state.lock().unwrap();
                let patch = &msg["patch"];
                if let Some(t) = patch["theme"].as_str() {
                    st.config.theme = t.to_string();
                }
                if let Some(m) = patch["menu"].as_bool() {
                    st.config.menu = m;
                }
                if let Some(m) = patch["mode"].as_str() {
                    st.config.mode = m.to_string();
                }
                save_config(&self.config_path, &st.config);
            }
            "zoom" => {
                let mut st = self.shared.state.lock().unwrap();
                let level = match msg["action"].as_str().unwrap_or("reset") {
                    "in" => st.config.zoom * 1.1,
                    "out" => st.config.zoom / 1.1,
                    _ => 1.0,
                }
                .clamp(0.5, 3.0);
                st.config.zoom = (level * 100.0).round() / 100.0;
                let _ = self.webview.zoom(st.config.zoom);
                save_config(&self.config_path, &st.config);
                let level = st.config.zoom;
                drop(st);
                self.emit(json!({ "type": "zoom", "level": level }));
            }
            "reveal" => {
                if let Some(dir) = self.current_path().and_then(|p| p.parent().map(Path::to_path_buf)) {
                    open_external(&dir.to_string_lossy());
                }
            }
            "quit" => {
                self.persist();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    }

    /// Ask the page to open a file (it checks for unsaved edits first).
    fn request_open(&self, path: &Path) {
        self.emit(json!({ "type": "openRequest", "path": path.to_string_lossy() }));
    }

    fn open_file(&mut self, path: &Path) {
        let path = match path.canonicalize() {
            Ok(p) if p.is_file() => p,
            _ => {
                self.toast(format!("Файл не найден: {}", display_path(path)));
                self.emit(json!({ "type": "openFailed" }));
                return;
            }
        };
        {
            let mut st = self.shared.state.lock().unwrap();
            st.path = Some(path.clone());
            st.dirty = false;
            st.saved_hash = None;
            push_recent(&mut st.config.recent, &path);
            save_config(&self.config_path, &st.config);
        }
        self.watch.watch(Some(&path));
        self.update_title();
        self.emit(json!({ "type": "doc" }));
    }

    fn close_file(&mut self) {
        {
            let mut st = self.shared.state.lock().unwrap();
            st.path = None;
            st.dirty = false;
            st.saved_hash = None;
        }
        self.watch.watch(None);
        self.update_title();
        self.emit(json!({ "type": "doc" }));
    }

    fn save_to(&mut self, path: &Path, content: &str) {
        if let Err(e) = fs::write(path, content) {
            self.toast(format!("Не удалось сохранить: {e}"));
            self.emit(json!({ "type": "saveCancelled" }));
            return;
        }
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_changed = {
            let mut st = self.shared.state.lock().unwrap();
            st.saved_hash = Some(hash_str(content));
            st.dirty = false;
            let changed = st.path.as_ref() != Some(&path);
            if changed {
                st.path = Some(path.clone());
                push_recent(&mut st.config.recent, &path);
                save_config(&self.config_path, &st.config);
            }
            changed
        };
        if path_changed {
            self.watch.watch(Some(&path));
        }
        self.update_title();
        self.emit(json!({
            "type": "saved",
            "path": path.to_string_lossy(),
            "name": path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            "dir": path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
            "display": display_path(&path),
            "pathChanged": path_changed,
        }));
    }

    /// Builds a native GTK file chooser attached to our window. It is shown
    /// asynchronously; the response arrives as a `UserEvent` on the main loop.
    fn file_dialog(
        &self,
        action: gtk::FileChooserAction,
        title: &str,
        accept: &str,
    ) -> gtk::FileChooserNative {
        let dialog = gtk::FileChooserNative::new(
            Some(title),
            Some(self.window.gtk_window()),
            action,
            Some(accept),
            Some("Отмена"),
        );
        let md = gtk::FileFilter::new();
        md.set_name(Some("Markdown"));
        for ext in render::MD_EXTENSIONS {
            md.add_pattern(&format!("*.{ext}"));
        }
        dialog.add_filter(md);
        let all = gtk::FileFilter::new();
        all.set_name(Some("Все файлы"));
        all.add_pattern("*");
        dialog.add_filter(all);
        let start_dir = self
            .current_path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok());
        if let Some(dir) = start_dir {
            dialog.set_current_folder(dir);
        }
        dialog
    }

    fn show_open_dialog(&mut self) {
        let dialog = self.file_dialog(gtk::FileChooserAction::Open, "Открыть Markdown", "Открыть");
        let proxy = self.proxy.clone();
        dialog.connect_response(move |d, resp| {
            let path = (resp == gtk::ResponseType::Accept).then(|| d.filename()).flatten();
            let _ = proxy.send_event(UserEvent::OpenPicked(path));
        });
        dialog.show();
        self.dialog = Some(dialog);
    }

    fn save_as(&mut self, content: &str) {
        let dialog = self.file_dialog(gtk::FileChooserAction::Save, "Сохранить как", "Сохранить");
        dialog.set_do_overwrite_confirmation(true);
        let name = self
            .current_path()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "untitled.md".into());
        dialog.set_current_name(&name);
        self.pending_save = Some(content.to_string());
        let proxy = self.proxy.clone();
        dialog.connect_response(move |d, resp| {
            let path = (resp == gtk::ResponseType::Accept).then(|| d.filename()).flatten();
            let _ = proxy.send_event(UserEvent::SavePicked(path));
        });
        dialog.show();
        self.dialog = Some(dialog);
    }

    /// A relative link from the document: other Markdown opens in place,
    /// a directory opens its README, anything else goes to the system handler.
    fn resolve_link(&self, href: &str) {
        let base = self.shared.base_dir();
        let clean = href.split(['?', '#']).next().unwrap_or("");
        if clean.is_empty() {
            return;
        }
        let decoded = percent_decode_str(clean).decode_utf8_lossy().into_owned();
        let joined = base.join(&decoded);
        let Ok(target) = joined.canonicalize() else {
            self.toast(format!("Файл не найден: {decoded}"));
            return;
        };
        if target.is_dir() {
            for name in ["README.md", "readme.md", "Readme.md", "index.md"] {
                let candidate = target.join(name);
                if candidate.is_file() {
                    self.request_open(&candidate);
                    return;
                }
            }
            open_external(&target.to_string_lossy());
            return;
        }
        if render::is_markdown(&target) {
            self.request_open(&target);
        } else {
            open_external(&target.to_string_lossy());
        }
    }

    fn on_file_changed(&self) {
        let st = self.shared.state.lock().unwrap();
        let Some(path) = &st.path else { return };
        if let (Some(saved), Ok(bytes)) = (st.saved_hash, fs::read(path)) {
            if hash_str(&String::from_utf8_lossy(&bytes)) == saved {
                return; // our own write
            }
        }
        drop(st);
        self.emit(json!({ "type": "fileChanged" }));
    }
}

/// `mdview://localhost/...` request router.
fn serve(shared: &Shared, req: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    const HTML: &str = "text/html; charset=utf-8";
    const JSON: &str = "application/json; charset=utf-8";
    let path = req.uri().path().to_string();
    let method = req.method().as_str().to_string();

    let (status, mime, body): (u16, String, Vec<u8>) = match (method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "") => (200, HTML.into(), shared.shell_page().into_bytes()),
        ("GET", "/doc") => (200, JSON.into(), shared.doc_json().to_string().into_bytes()),
        ("POST", "/render") => {
            let md = String::from_utf8_lossy(req.body());
            let html = shared.renderer.render_markdown(&md, &shared.base_dir());
            (200, HTML.into(), html.into_bytes())
        }
        ("GET", p) if p.starts_with("/static/") => match vendor::lookup(&p["/static/".len()..]) {
            Some((mime, bytes)) => (200, mime.into(), bytes.to_vec()),
            None => (404, "text/plain".into(), b"not found".to_vec()),
        },
        ("GET", p) if p.starts_with("/asset/") => {
            let encoded = &p["/asset/".len()..];
            let file = PathBuf::from(percent_decode_str(encoded).decode_utf8_lossy().into_owned());
            match fs::read(&file) {
                Ok(bytes) if file.is_file() => {
                    let mime = mime_guess::from_path(&file)
                        .first_or_octet_stream()
                        .to_string();
                    (200, mime, bytes)
                }
                _ => (404, "text/plain".into(), b"not found".to_vec()),
            }
        }
        _ => (404, "text/plain".into(), b"not found".to_vec()),
    };

    let cache = if path.starts_with("/static/") { "max-age=31536000, immutable" } else { "no-store" };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, mime)
        .header("Cache-Control", cache)
        .body(Cow::Owned(body))
        .expect("build response")
}

fn open_external(target: &str) {
    let ok = target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with('/');
    if ok {
        let _ = Command::new("xdg-open").arg(target).spawn();
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// `/home/user/x` → `~/x`
fn display_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if let Some(rest) = s.strip_prefix(&*home) {
            if rest.is_empty() || rest.starts_with('/') {
                return format!("~{rest}");
            }
        }
    }
    s.into_owned()
}

fn push_recent(recent: &mut Vec<String>, path: &Path) {
    let p = path.to_string_lossy().into_owned();
    recent.retain(|r| r != &p);
    recent.insert(0, p);
    recent.truncate(MAX_RECENT);
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("mdview").join("config.json")
}

fn load_config(path: &Path) -> Config {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(path: &Path, config: &Config) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = fs::write(path, json);
    }
}

/// Watches the document's directory (editors replace files atomically) and
/// emits a debounced `FileChanged` when the document changes.
struct Watch {
    proxy: EventLoopProxy<UserEvent>,
    watcher: Option<notify::RecommendedWatcher>,
}

impl Watch {
    fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            proxy,
            watcher: None,
        }
    }

    fn watch(&mut self, path: Option<&Path>) {
        self.watcher = None;
        let Some(path) = path else { return };
        let Some(dir) = path.parent() else { return };
        let name = path.file_name().map(|n| n.to_os_string());

        let (tx, rx) = mpsc::channel::<()>();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(Duration::from_millis(150));
                while rx.try_recv().is_ok() {}
                if proxy.send_event(UserEvent::FileChanged).is_err() {
                    break;
                }
            }
        });

        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                // Reading the file ourselves produces Access events; only real changes count.
                let relevant = matches!(
                    ev.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Any
                );
                if relevant && ev.paths.iter().any(|p| p.file_name() == name.as_deref()) {
                    let _ = tx.send(());
                }
            }
        });
        if let Ok(mut w) = watcher {
            if w.watch(dir, RecursiveMode::NonRecursive).is_ok() {
                self.watcher = Some(w);
            }
        }
    }
}
