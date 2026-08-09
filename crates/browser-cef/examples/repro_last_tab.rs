//! Local repro harness for the last-tab close crash (EXC_BAD_ACCESS in
//! `cef::do_message_loop_work` after close-last-tab → recreate).
//!
//! Run: `cargo run -p anycode-browser-cef --features host --example repro_last_tab`
//!
//! Drives the exact host.rs paths the Workbench uses, without Tauri:
//! show → new_tab → close non-last → close last → recreate → pump for a while.

#[allow(unused_imports)]
use block2 as _block2_link;
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use objc2::{msg_send, ClassType};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use anycode_browser_cef::{
    close_tab, do_message_loop_work, ensure_initialized, hide, list_tabs, new_tab,
    set_schedule_pump_callback, show_in_parent, EmbedRect,
};

type MainFn = Box<dyn FnOnce() + Send>;
static MAIN_TX: OnceLock<mpsc::Sender<MainFn>> = OnceLock::new();
static MAIN_RX: OnceLock<Mutex<mpsc::Receiver<MainFn>>> = OnceLock::new();

extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async(queue: *mut c_void, block: &block2::Block<dyn Fn()>);
}

fn drain_main_jobs() {
    let Some(rx) = MAIN_RX.get() else { return };
    let Ok(g) = rx.lock() else { return };
    while let Ok(job) = g.try_recv() {
        job();
    }
}

fn wake_main_queue() {
    let block = block2::RcBlock::new(drain_main_jobs);
    unsafe { dispatch_async(&_dispatch_main_q as *const c_void as *mut c_void, &*block) };
}

/// Run `f` on the main thread and wait for completion.
fn run_on_main<F: FnOnce() + Send + 'static>(f: F) {
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let tx = MAIN_TX.get().expect("main dispatcher not set").clone();
    tx.send(Box::new(move || {
        f();
        let _ = done_tx.send(());
    }))
    .expect("main dispatcher dead");
    wake_main_queue();
    let _ = done_rx.recv_timeout(Duration::from_secs(15));
}

fn call_host<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Result<R, String> {
    let (tx, rx) = mpsc::channel::<R>();
    run_on_main(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(15))
        .map_err(|e| format!("host call timed out: {e}"))
}

fn main() {
    let mtm = MainThreadMarker::new().expect("must run on main thread");

    let (tx, rx) = mpsc::channel::<MainFn>();
    let _ = MAIN_TX.set(tx);
    let _ = MAIN_RX.set(Mutex::new(rx));

    let app = NSApplication::sharedApplication(mtm);
    let rect = NSRect {
        origin: NSPoint { x: 200.0, y: 200.0 },
        size: NSSize {
            width: 900.0,
            height: 700.0,
        },
    };
    let window: *mut NSWindow = unsafe {
        let cls = NSWindow::class();
        let alloc: *mut NSWindow = msg_send![cls, alloc];
        msg_send![alloc, initWithContentRect: rect,
            styleMask: NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable,
            backing: NSBackingStoreType::Buffered,
            defer: false]
    };
    assert!(!window.is_null(), "NSWindow create failed");
    let window = unsafe { &*window };
    window.setTitle(&NSString::from_str("cef-repro"));
    window.makeKeyAndOrderFront(None);
    app.activateIgnoringOtherApps(true);

    let helper = PathBuf::from(
        "/Applications/anyCode.app/Contents/Frameworks/anyCode Helper.app/Contents/MacOS/anyCode Helper",
    );
    let cef_root = PathBuf::from("/Applications/anyCode.app/Contents/Frameworks");
    ensure_initialized(&helper, &cef_root).expect("CEF init failed");
    eprintln!("[repro] CEF initialized");

    set_schedule_pump_callback(Box::new(move |delay_ms| {
        let delay = delay_ms.max(0) as u64;
        std::thread::spawn(move || {
            if delay > 0 {
                std::thread::sleep(Duration::from_millis(delay));
            }
            run_on_main(do_message_loop_work);
        });
    }));

    // Idle pump fallback (mirrors the desktop's IDLE_PUMP_INTERVAL_MS): CEF in
    // external-pump mode starves without periodic DoMessageLoopWork.
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_millis(80));
        run_on_main(do_message_loop_work);
    });

    let win_ptr = window as *const NSWindow as usize;
    std::thread::spawn(move || {
        let panel = EmbedRect {
            x: 20.0,
            y: 20.0,
            width: 640.0,
            height: 480.0,
        };
        std::thread::sleep(Duration::from_secs(3));

        eprintln!("[repro] step 1: show example.com");
        let r =
            call_host(move || show_in_parent(win_ptr as *mut c_void, panel, "https://example.com"));
        eprintln!("[repro] show -> {r:?}");
        std::thread::sleep(Duration::from_secs(4));
        let tabs = list_tabs();
        eprintln!("[repro] tabs after show: {:?}", ids(&tabs));
        assert_eq!(tabs.len(), 1, "expected 1 tab after show");

        eprintln!("[repro] step 2: new_tab github.com");
        let r = call_host(|| new_tab("https://github.com"));
        eprintln!("[repro] new_tab -> {r:?}");
        std::thread::sleep(Duration::from_secs(3));
        let tabs = list_tabs();
        eprintln!("[repro] tabs after new_tab: {:?}", ids(&tabs));
        assert_eq!(tabs.len(), 2, "expected 2 tabs");

        eprintln!("[repro] step 3: close non-last tab (soft path)");
        let id2 = tabs[1].id;
        let r = call_host(move || close_tab(id2));
        eprintln!("[repro] close non-last -> {r:?}");
        std::thread::sleep(Duration::from_secs(3));
        let tabs = list_tabs();
        eprintln!("[repro] tabs after soft close: {:?}", ids(&tabs));
        assert_eq!(tabs.len(), 1, "expected 1 tab after soft close");

        eprintln!("[repro] step 4: close LAST tab (deterministic teardown)");
        let id1 = tabs[0].id;
        let r = call_host(move || close_tab(id1));
        eprintln!("[repro] close last -> {r:?}");
        std::thread::sleep(Duration::from_secs(3));
        let tabs = list_tabs();
        eprintln!("[repro] tabs after last close: {:?}", ids(&tabs));
        assert!(tabs.is_empty(), "expected 0 tabs after last close");

        eprintln!("[repro] step 5: recreate via show (zero tabs)");
        let r =
            call_host(move || show_in_parent(win_ptr as *mut c_void, panel, "https://example.com"));
        eprintln!("[repro] recreate -> {r:?}");
        std::thread::sleep(Duration::from_secs(2));
        let tabs = list_tabs();
        eprintln!("[repro] tabs after recreate: {:?}", ids(&tabs));
        assert_eq!(tabs.len(), 1, "expected 1 tab after recreate");

        eprintln!("[repro] step 5b: hide/show cycles (dock<->tab moves)");
        for i in 0..3 {
            let r = call_host(|| {
                hide();
                Ok::<(), String>(())
            });
            eprintln!("[repro] hide {i} -> {r:?}");
            std::thread::sleep(Duration::from_millis(400));
            let rect2 = EmbedRect {
                x: 120.0 + 40.0 * i as f64,
                y: 60.0,
                width: 500.0,
                height: 400.0,
            };
            let r = call_host(move || show_in_parent(win_ptr as *mut c_void, rect2, ""));
            eprintln!("[repro] reshow {i} -> {r:?}");
            std::thread::sleep(Duration::from_millis(600));
        }
        let tabs = list_tabs();
        eprintln!("[repro] tabs after hide/show cycles: {:?}", ids(&tabs));
        assert_eq!(tabs.len(), 1, "hide/show must keep the tab");

        eprintln!("[repro] step 5c: close last after cycles, recreate again");
        let id = tabs[0].id;
        let r = call_host(move || close_tab(id));
        eprintln!("[repro] close last -> {r:?}");
        std::thread::sleep(Duration::from_secs(2));
        assert!(list_tabs().is_empty(), "expected 0 tabs");
        let r =
            call_host(move || show_in_parent(win_ptr as *mut c_void, panel, "https://example.com"));
        eprintln!("[repro] recreate2 -> {r:?}");
        std::thread::sleep(Duration::from_secs(2));
        assert_eq!(list_tabs().len(), 1, "expected 1 tab after recreate2");

        eprintln!(
            "[repro] step 6: churn close-last + recreate for 25s (CDP hammer runs concurrently)"
        );
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(25) {
            let tabs = list_tabs();
            if let Some(t) = tabs.first() {
                let id = t.id;
                let _ = call_host(move || close_tab(id));
                std::thread::sleep(Duration::from_millis(700));
            }
            let r = call_host(move || {
                show_in_parent(win_ptr as *mut c_void, panel, "https://example.com")
            });
            if r.is_err() {
                eprintln!("[repro] churn recreate err: {r:?}");
            }
            std::thread::sleep(Duration::from_millis(900));
        }
        eprintln!("[repro] PASS — survived close-last + recreate + 10s pump");
        std::process::exit(0);
    });

    eprintln!("[repro] entering NSApplication run loop");
    app.run();
}

fn ids(tabs: &[anycode_browser_cef::TabInfo]) -> Vec<i32> {
    tabs.iter().map(|t| t.id).collect()
}
