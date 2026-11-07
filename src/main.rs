//! reel: a minimal WebKitGTK browser for watching media.

mod filters;

use std::{cell::Cell, path::Path, process::Command, rc::Rc};

use gtk::{gdk, glib, prelude::*};
use webkit2gtk::{
    HardwareAccelerationPolicy, NavigationPolicyDecision, NavigationPolicyDecisionExt,
    PermissionRequestExt, PolicyDecisionExt, PolicyDecisionType, SettingsExt, URIRequestExt,
    WebContextExt, WebProcessTerminationReason, WebView, WebViewExt, WebsiteDataManagerExt,
};
use wry::{WebContext, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

const USAGE: &str = "\
usage: reel [options] [URL | FILE | search terms]

options:
  --private          ephemeral session: nothing is written to disk
  --no-adblock       disable content blocking and the YouTube ad script
  --update-filters   rebuild filter lists now instead of on schedule
  --hw-codecs        hide VP9/AV1 from sites unless a hardware decoder exists
  -h, --help         show this help

keys:
  Ctrl+L, F6         address bar          Esc           close address bar
  Alt+Left/Right     back / forward       F5, Ctrl+R    reload
  F11                fullscreen           Ctrl+M        mute
  Ctrl+= / - / 0     zoom                 Ctrl+Shift+P  keep window on top
  Ctrl+Shift+M       open page in mpv     Ctrl+Q, Ctrl+W quit

env:
  REEL_SEARCH        search URL prefix (default https://duckduckgo.com/?q=)";

// Content types listed here are reported as unsupported unless GStreamer has
// a hardware decoder for them, so sites such as YouTube fall back to a codec
// the GPU can decode instead of burning CPU on software VP9/AV1.
// Colon-separated, per WebKitSettings docs.
const HW_ONLY_TYPES: &str =
    r#"video/*; codecs="vp9*":video/*; codecs="vp09*":video/*; codecs="av01*""#;

const CSS: &[u8] = b"
#urlbar { border-radius: 0; border: none; padding: 8px 12px; font-size: 14px;
          background: rgba(18,18,18,0.96); color: #eee; caret-color: #fff; box-shadow: none; }
";

struct Opts {
    target: Option<String>,
    private: bool,
    adblock: bool,
    update_filters: bool,
    hw_codecs: bool,
}

fn parse_args() -> Opts {
    let mut o = Opts {
        target: None,
        private: false,
        adblock: true,
        update_filters: false,
        hw_codecs: false,
    };
    let mut words = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--private" => o.private = true,
            "--no-adblock" => o.adblock = false,
            "--update-filters" => o.update_filters = true,
            "--hw-codecs" => o.hw_codecs = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            s if s.starts_with("--") => {
                eprintln!("reel: unknown option {s}\n\n{USAGE}");
                std::process::exit(2);
            }
            _ => words.push(a),
        }
    }
    if !words.is_empty() {
        o.target = resolve(&words.join(" "), true);
    }
    o
}

/// Turns address-bar or command-line input into a URI. Bare relative file
/// names are only accepted from the command line.
fn resolve(input: &str, cli: bool) -> Option<String> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let path = Path::new(t);
    if (t.starts_with('/') || t.starts_with("./") || t.starts_with("~/")) || (cli && path.exists())
    {
        let expanded = t
            .strip_prefix("~/")
            .map(|rest| glib::home_dir().join(rest))
            .unwrap_or_else(|| path.to_path_buf());
        if let Ok(abs) = expanded.canonicalize() {
            return glib::filename_to_uri(abs, None).ok().map(Into::into);
        }
    }
    if t.contains("://") || t.starts_with("about:") {
        return Some(t.to_string());
    }
    let hostlike = !t.contains(char::is_whitespace)
        && (t.contains('.') || t.starts_with("localhost") || t.contains(':'));
    if hostlike {
        return Some(format!("https://{t}"));
    }
    let prefix =
        std::env::var("REEL_SEARCH").unwrap_or_else(|_| "https://duckduckgo.com/?q=".into());
    Some(format!(
        "{prefix}{}",
        glib::Uri::escape_string(t, None, false)
    ))
}

fn main() {
    let opts = parse_args();
    glib::set_prgname(Some("reel"));
    glib::set_application_name("Reel");
    // bubblewrap sandbox for web processes. wry does not expose its WebKit
    // context before the view is built (which spawns processes, after which
    // sandboxing is locked), so ask WebKit through its documented switch.
    if glib::find_program_in_path("bwrap").is_some() {
        // SAFETY: single-threaded; no other thread exists yet.
        unsafe { std::env::set_var("WEBKIT_FORCE_SANDBOX", "1") };
    } else {
        eprintln!("reel: bubblewrap not found; web processes run unsandboxed");
    }
    gtk::init().expect("failed to initialise GTK");
    gdk::set_program_class("reel");

    if let Some(s) = gtk::Settings::default() {
        s.set_gtk_application_prefer_dark_theme(true);
    }
    let css = gtk::CssProvider::new();
    let _ = css.load_from_data(CSS);
    gtk::StyleContext::add_provider_for_screen(
        &gdk::Screen::default().expect("no display"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("reel");
    window.set_icon_name(Some("reel"));
    window.set_default_size(1280, 720);
    let overlay = gtk::Overlay::new();
    window.add(&overlay);

    // --- web view -------------------------------------------------------
    // --private uses WebKit's ephemeral session: website data stays in memory.
    let mut context =
        (!opts.private).then(|| WebContext::new(Some(glib::user_data_dir().join("reel"))));
    let mut builder = match context.as_mut() {
        Some(ctx) => WebViewBuilder::new_with_web_context(ctx),
        None => WebViewBuilder::new().with_incognito(true),
    };
    builder = builder
        .with_background_color((0, 0, 0, 255))
        .with_autoplay(true)
        .with_devtools(cfg!(debug_assertions));
    if opts.adblock {
        builder = builder.with_initialization_script(include_str!("youtube.js"));
    }
    let wry_view = builder
        .build_gtk(&overlay)
        .expect("failed to create web view");
    let view: WebView = wry_view.webview();

    configure_engine(&view, &opts);
    connect_view_signals(&view, &window);

    // --- address bar ----------------------------------------------------
    let entry = gtk::Entry::new();
    entry.set_widget_name("urlbar");
    entry.set_placeholder_text(Some("URL, file, or search"));
    let bar = gtk::Revealer::new();
    bar.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    bar.set_transition_duration(90);
    bar.set_valign(gtk::Align::Start);
    bar.add(&entry);
    overlay.add_overlay(&bar);

    let hide_bar = {
        let (bar, view) = (bar.clone(), view.clone());
        move || {
            bar.set_reveal_child(false);
            view.grab_focus();
        }
    };
    let show_bar = {
        let (bar, entry, view) = (bar.clone(), entry.clone(), view.clone());
        move || {
            entry.set_text(view.uri().as_deref().unwrap_or(""));
            bar.set_reveal_child(true);
            entry.grab_focus();
        }
    };

    entry.connect_activate({
        let (view, hide) = (view.clone(), hide_bar.clone());
        move |e| {
            if let Some(uri) = resolve(&e.text(), false) {
                view.load_uri(&uri);
            }
            hide();
        }
    });
    entry.connect_key_press_event({
        let hide = hide_bar.clone();
        move |_, ev| {
            if ev.keyval() == gdk::keys::constants::Escape {
                hide();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    entry.connect_focus_out_event({
        let bar = bar.clone();
        move |_, _| {
            bar.set_reveal_child(false);
            glib::Propagation::Proceed
        }
    });

    // Hide chrome while a page element (e.g. a video) is fullscreen.
    view.connect_enter_fullscreen({
        let bar = bar.clone();
        move |_| {
            bar.set_reveal_child(false);
            false
        }
    });

    // --- keys -----------------------------------------------------------
    let fullscreen = Rc::new(Cell::new(false));
    window.connect_window_state_event({
        let fullscreen = fullscreen.clone();
        move |_, ev| {
            fullscreen.set(ev.new_window_state().contains(gdk::WindowState::FULLSCREEN));
            glib::Propagation::Proceed
        }
    });
    let on_top = Rc::new(Cell::new(false));
    window.connect_key_press_event({
        let (view, entry, wry_view) = (view.clone(), entry.clone(), wry_view);
        let show_bar = show_bar.clone();
        move |win, ev| {
            use gdk::keys::constants as k;
            let m = ev.state();
            let ctrl = m.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = m.contains(gdk::ModifierType::SHIFT_MASK);
            let alt = m.contains(gdk::ModifierType::MOD1_MASK);
            let key = ev.keyval().to_lower();

            if ctrl && (key == k::q || key == k::w) {
                gtk::main_quit();
                return glib::Propagation::Stop;
            }
            // Let the address bar keep its normal editing keys.
            if entry.has_focus() {
                return glib::Propagation::Proceed;
            }

            match () {
                _ if (ctrl && key == k::l) || key == k::F6 => show_bar(),
                _ if alt && key == k::Left => view.go_back(),
                _ if alt && key == k::Right => view.go_forward(),
                _ if key == k::F5 || (ctrl && key == k::r) => {
                    if shift {
                        view.reload_bypass_cache()
                    } else {
                        view.reload()
                    }
                }
                _ if key == k::F11 => {
                    if fullscreen.get() {
                        win.unfullscreen()
                    } else {
                        win.fullscreen()
                    }
                }
                _ if ctrl && !shift && key == k::m => view.set_is_muted(!view.is_muted()),
                _ if ctrl && (key == k::equal || key == k::plus || key == k::KP_Add) => {
                    view.set_zoom_level((view.zoom_level() * 1.1).min(5.0))
                }
                _ if ctrl && (key == k::minus || key == k::KP_Subtract) => {
                    view.set_zoom_level((view.zoom_level() / 1.1).max(0.25))
                }
                _ if ctrl && key == k::_0 => view.set_zoom_level(1.0),
                _ if ctrl && shift && key == k::p => {
                    on_top.set(!on_top.get());
                    win.set_keep_above(on_top.get());
                }
                _ if ctrl && shift && key == k::m => {
                    if let Some(uri) = view.uri().filter(|u| u.starts_with("http")) {
                        let _ = wry_view.evaluate_script(
                            "document.querySelectorAll('video,audio').forEach(m=>m.pause())",
                        );
                        // No shell; "--" stops mpv from reading the URI as an option.
                        if let Err(e) = Command::new("mpv").arg("--").arg(uri.as_str()).spawn() {
                            eprintln!("reel: could not start mpv: {e}");
                        }
                    }
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        }
    });

    // Mouse back/forward buttons.
    view.connect_button_press_event(|v, ev| match ev.button() {
        8 => {
            v.go_back();
            glib::Propagation::Stop
        }
        9 => {
            v.go_forward();
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });
    window.show_all();

    // --- first navigation -------------------------------------------------
    let start = {
        let (view, target) = (view.clone(), opts.target.clone());
        move || match &target {
            Some(uri) => view.load_uri(uri),
            None => show_bar(),
        }
    };
    if opts.adblock {
        let ucm = view
            .user_content_manager()
            .expect("no user content manager");
        if opts.target.is_some() {
            view.load_html(LOADING_HTML, None);
        }
        filters::install(ucm, opts.update_filters, start);
    } else {
        start();
    }

    gtk::main();
    drop(context);
}

const LOADING_HTML: &str = "<body style='margin:0;height:100vh;display:grid;place-items:center;\
background:#000;color:#666;font:14px system-ui'>loading filters…</body>";

fn configure_engine(view: &WebView, opts: &Opts) {
    let ctx = view.context().expect("web view has no context");
    if let Some(dm) = ctx.website_data_manager() {
        dm.set_itp_enabled(true); // Intelligent Tracking Prevention
    }

    let s = WebViewExt::settings(view).expect("web view has no settings");
    s.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Always);
    s.set_enable_mediasource(true);
    s.set_enable_media_capabilities(true);
    s.set_media_playback_requires_user_gesture(false);
    s.set_enable_fullscreen(true);
    s.set_enable_smooth_scrolling(true);
    s.set_enable_back_forward_navigation_gestures(true);
    // Watching does not need capture devices, peer connections, or DRM.
    s.set_enable_media_stream(false);
    s.set_enable_webrtc(false);
    s.set_enable_encrypted_media(false);
    if opts.hw_codecs {
        s.set_media_content_types_requiring_hardware_support(Some(HW_ONLY_TYPES));
    }
}

fn connect_view_signals(view: &WebView, window: &gtk::Window) {
    // Deny every permission prompt: camera, mic, location, notifications, ...
    view.connect_permission_request(|_, req| {
        req.deny();
        true
    });

    // Popups: user-initiated ones (target=_blank, middle click) open in place;
    // script-initiated ones are dropped.
    view.connect_create(|v, action| {
        if action.is_user_gesture()
            && let Some(uri) = action.request().and_then(|r| r.uri())
        {
            v.load_uri(&uri);
        }
        None
    });
    view.connect_decide_policy(|_, decision, kind| {
        if kind == PolicyDecisionType::NewWindowAction
            && let Some(nav) = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
            && !nav.is_user_gesture()
        {
            decision.ignore();
            return true;
        }
        false
    });

    view.connect_web_process_terminated(|v, reason| {
        eprintln!("reel: web process terminated ({reason:?}); reloading");
        if reason != WebProcessTerminationReason::TerminatedByApi {
            v.reload();
        }
    });

    let update_title = {
        let window = window.clone();
        move |v: &WebView| {
            let title = v.title().filter(|t| !t.is_empty());
            let host = v
                .uri()
                .and_then(|u| glib::Uri::parse(&u, glib::UriFlags::NONE).ok())
                .and_then(|u| u.host());
            // Keep the origin visible so a page cannot pose as another site.
            let text = match (title, host) {
                (Some(t), Some(h)) => format!("{t} — {h}"),
                (Some(t), None) => t.to_string(),
                (None, Some(h)) => h.to_string(),
                (None, None) => "reel".into(),
            };
            window.set_title(&text);
        }
    };
    view.connect_title_notify(update_title.clone());
    view.connect_uri_notify(update_title);
}
