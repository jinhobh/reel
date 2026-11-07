//! Ad and tracker blocking.
//!
//! Filter lists (ABP/uBO syntax) are converted by the `adblock` crate into
//! WebKit content-blocker JSON, which WebKit compiles into a bytecode DFA and
//! evaluates natively inside the network/web processes. After the one-time
//! compile, matching costs nothing on the Rust side: no per-request IPC or
//! callbacks. The compiled filter is cached on disk and memory-mapped on later
//! starts.

use std::{
    ffi::CString,
    fs,
    path::PathBuf,
    ptr,
    time::{Duration, SystemTime},
};

use adblock::{
    content_blocking::CbType,
    lists::{FilterSet, ParseOptions},
};
use futures_channel::oneshot;
use gtk::{
    gio,
    glib::{self, translate::ToGlibPtr},
};
use webkit2gtk::{UserContentManager, UserContentManagerExt};
use webkit2gtk_sys as wk;

const LISTS: &[&str] = &[
    "https://easylist.to/easylist/easylist.txt",
    "https://easylist.to/easylist/easyprivacy.txt",
    "https://ublockorigin.github.io/uAssets/filters/filters.txt",
    "https://ublockorigin.github.io/uAssets/filters/privacy.txt",
];
const FILTER_ID: &str = "reel";
const MAX_AGE: Duration = Duration::from_secs(4 * 24 * 3600);

fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("reel").join("filters")
}

fn stamp_path() -> PathBuf {
    cache_dir().join("stamp")
}

/// Path of an optional user filter list, appended to the downloaded lists.
pub fn user_list_path() -> PathBuf {
    glib::user_config_dir().join("reel").join("filters.txt")
}

/// Installs the content filter into `ucm`, then calls `ready` (once).
///
/// A cached compiled filter is used immediately. A missing cache or `force`
/// rebuilds before `ready`. A stale cache is refreshed in the background.
pub fn install(ucm: UserContentManager, force: bool, ready: impl FnOnce() + 'static) {
    let store = Store::new();
    let age = fs::metadata(stamp_path())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok());

    glib::MainContext::default().spawn_local(async move {
        if !force && age.is_some() {
            match store.load(FILTER_ID).await {
                Ok(filter) => {
                    ucm.add_filter(&filter);
                    ready();
                    if age.is_some_and(|a| a > MAX_AGE) {
                        rebuild(&store, &ucm).await;
                    }
                    return;
                }
                Err(e) => eprintln!("reel: cached filter unusable ({e}); rebuilding"),
            }
        }
        rebuild(&store, &ucm).await;
        ready();
    });
}

async fn rebuild(store: &Store, ucm: &UserContentManager) {
    let started = std::time::Instant::now();
    let mut lists = Vec::new();
    let session = soup::Session::builder().timeout(30).build();
    for url in LISTS {
        match fetch(&session, url).await {
            Ok(text) => lists.push(text),
            Err(e) => eprintln!("reel: filter list {url}: {e}"),
        }
    }
    if let Ok(text) = fs::read_to_string(user_list_path()) {
        lists.push(text);
    }
    if lists.is_empty() {
        eprintln!("reel: no filter lists available; ad blocking disabled this session");
        return;
    }

    // Parsing and conversion take a few hundred ms; keep it off the UI thread.
    let json = match gio::spawn_blocking(move || convert(lists)).await {
        Ok(Ok(json)) => json,
        Ok(Err(e)) => return eprintln!("reel: filter conversion failed: {e}"),
        Err(_) => return eprintln!("reel: filter conversion panicked"),
    };

    match store.save(FILTER_ID, glib::Bytes::from_owned(json)).await {
        Ok(filter) => {
            ucm.remove_all_filters();
            ucm.add_filter(&filter);
            let _ = fs::create_dir_all(cache_dir());
            let _ = fs::write(stamp_path(), b"");
            eprintln!("reel: filters ready in {:.1?}", started.elapsed());
        }
        Err(e) => eprintln!("reel: WebKit rejected filter: {e}"),
    }
}

async fn fetch(session: &soup::Session, url: &str) -> Result<String, String> {
    use soup::prelude::*;
    let msg = soup::Message::new("GET", url).map_err(|e| e.to_string())?;
    let body = session
        .send_and_read_future(&msg, glib::Priority::DEFAULT)
        .await
        .map_err(|e| e.to_string())?;
    if msg.status() != soup::Status::Ok {
        return Err(format!("HTTP {:?}", msg.status()));
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn convert(lists: Vec<String>) -> Result<Vec<u8>, String> {
    // Debug mode retains the original rule text, which conversion requires.
    let mut set = FilterSet::new(true);
    for list in lists {
        set.add_filter_list(list, ParseOptions::default());
    }
    let (mut rules, _) = set
        .into_content_blocking()
        .map_err(|()| "content-blocking conversion unavailable".to_string())?;
    // Generic cosmetic rules (~14k selectors) become one stylesheet applied to
    // every page, which slows style recalculation everywhere. Network rules
    // already stop the ads themselves, so keep only site-specific hiding.
    rules.retain(|r| r.action.typ != CbType::CssDisplayNone || r.trigger.if_domain.is_some());
    eprintln!("reel: compiling {} content-blocker rules", rules.len());
    serde_json::to_vec(&rules).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Minimal binding for WebKitUserContentFilterStore / WebKitUserContentFilter,
// which webkit2gtk-rs does not wrap.

pub struct Filter(*mut wk::WebKitUserContentFilter);

impl Drop for Filter {
    fn drop(&mut self) {
        unsafe { wk::webkit_user_content_filter_unref(self.0) }
    }
}

trait AddFilter {
    fn add_filter(&self, filter: &Filter);
}

impl AddFilter for UserContentManager {
    fn add_filter(&self, filter: &Filter) {
        // The manager takes its own reference.
        unsafe { wk::webkit_user_content_manager_add_filter(self.to_glib_none().0, filter.0) }
    }
}

struct Store(glib::Object);

type Reply = oneshot::Sender<Result<Filter, glib::Error>>;

impl Store {
    fn new() -> Self {
        let path = CString::new(cache_dir().join("store").to_string_lossy().as_bytes()).unwrap();
        unsafe {
            let raw = wk::webkit_user_content_filter_store_new(path.as_ptr());
            Store(glib::translate::from_glib_full(
                raw as *mut glib::gobject_ffi::GObject,
            ))
        }
    }

    fn raw(&self) -> *mut wk::WebKitUserContentFilterStore {
        let obj: *mut glib::gobject_ffi::GObject = self.0.to_glib_none().0;
        obj as *mut _
    }

    async fn load(&self, id: &str) -> Result<Filter, glib::Error> {
        let (tx, rx) = oneshot::channel::<Result<Filter, glib::Error>>();
        let id = CString::new(id).unwrap();
        unsafe {
            wk::webkit_user_content_filter_store_load(
                self.raw(),
                id.as_ptr(),
                ptr::null_mut(),
                Some(load_done),
                Box::into_raw(Box::new(tx)) as glib::ffi::gpointer,
            );
        }
        rx.await.expect("filter store callback dropped")
    }

    async fn save(&self, id: &str, json: glib::Bytes) -> Result<Filter, glib::Error> {
        let (tx, rx) = oneshot::channel::<Result<Filter, glib::Error>>();
        let id = CString::new(id).unwrap();
        unsafe {
            wk::webkit_user_content_filter_store_save(
                self.raw(),
                id.as_ptr(),
                ToGlibPtr::<*const glib::ffi::GBytes>::to_glib_none(&json).0 as *mut _,
                ptr::null_mut(),
                Some(save_done),
                Box::into_raw(Box::new(tx)) as glib::ffi::gpointer,
            );
        }
        rx.await.expect("filter store callback dropped")
    }
}

unsafe fn finish(
    data: glib::ffi::gpointer,
    filter: *mut wk::WebKitUserContentFilter,
    err: *mut glib::ffi::GError,
) {
    let tx = unsafe { Box::from_raw(data as *mut Reply) };
    let result = if filter.is_null() {
        Err(unsafe { glib::translate::from_glib_full(err) })
    } else {
        Ok(Filter(filter))
    };
    let _ = tx.send(result);
}

unsafe extern "C" fn load_done(
    src: *mut glib::gobject_ffi::GObject,
    res: *mut gio::ffi::GAsyncResult,
    data: glib::ffi::gpointer,
) {
    let mut err = ptr::null_mut();
    unsafe {
        let f = wk::webkit_user_content_filter_store_load_finish(src as *mut _, res, &mut err);
        finish(data, f, err);
    }
}

unsafe extern "C" fn save_done(
    src: *mut glib::gobject_ffi::GObject,
    res: *mut gio::ffi::GAsyncResult,
    data: glib::ffi::gpointer,
) {
    let mut err = ptr::null_mut();
    unsafe {
        let f = wk::webkit_user_content_filter_store_save_finish(src as *mut _, res, &mut err);
        finish(data, f, err);
    }
}
