//! `new-terminal`: one window with a scrollback, a prompt, and a status bar.
//! The window draws what `nt-core` sends and passes the author's input back.

#[cfg(debug_assertions)]
mod demo;
mod markup;
mod menu;
mod palette;
mod request;
mod root;
mod scrollback;
mod typeface;

use std::time::{Duration, Instant};

use gpui_kit::{
    AppContext as _, Bounds, Global, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use nt_core::{Closed, CoreHandle};

/// The longest the quit routine holds the exit while the core stops its
/// agents. The core's own stop steps end within about 5.5 s.
const QUIT_LIMIT: Duration = Duration::from_secs(6);
/// Fits the 1000 px prompt row and its margins.
const WINDOW_WIDTH: f32 = 1280.;
const WINDOW_HEIGHT: f32 = 820.;

/// The core handle, reachable from menu actions.
#[derive(Debug)]
struct Core(CoreHandle);

impl Global for Core {}

fn main() {
    let launched = Instant::now();
    let Some(app_home) = nt_core::default_app_home() else {
        eprintln!(
            "New Terminal cannot find your home directory, so it has nowhere to keep its logs."
        );
        std::process::exit(1);
    };
    let (core, events) = nt_core::start(app_home);

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            menu::install(cx);
            cx.set_global(Core(core.clone()));
            install_quit_routine(cx);
            typeface::install(cx);
            if pinned_dark() {
                palette::apply(gpui_kit::WindowAppearance::Dark, cx);
            } else {
                palette::apply(cx.window_appearance(), cx);
            }

            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                            None,
                            size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)),
                            cx,
                        ))),
                        titlebar: Some(TitlebarOptions {
                            title: Some("New Terminal".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    move |window, cx| {
                        if !pinned_dark() {
                            palette::follow(window);
                        }
                        cx.new(|cx| root::Root::new(core, events, launched, window, cx))
                    },
                )
                .expect("open the New Terminal window");
            let _ = window.update(cx, |root, window, cx| root.focus_prompt(window, cx));
            #[cfg(debug_assertions)]
            if demo::wanted() {
                let _ = window.update(cx, |root, _, cx| root.show_demo(cx));
            }
            cx.activate(true);
        });
}

/// Whether a debug launch asked to review the dark palette whatever the
/// system appearance, so the window neither reads nor follows it.
#[cfg(debug_assertions)]
fn pinned_dark() -> bool {
    demo::dark()
}

#[cfg(not(debug_assertions))]
const fn pinned_dark() -> bool {
    false
}

/// The one quit routine. Every quit path, from the app or from the system,
/// reaches GPUI's `terminate:`, which runs this before GPUI's own short wait
/// for quit handlers. Blocking here is what keeps the process alive until
/// the core has stopped every agent.
fn install_quit_routine(cx: &gpui_kit::App) {
    cx.on_app_quit(|cx| {
        let core = cx.global::<Core>().0.clone();
        core.quit();
        if core.wait_closed(QUIT_LIMIT) == Closed::TimedOut {
            eprintln!("New Terminal's core did not stop within {QUIT_LIMIT:?}; quitting anyway.");
        }
        async {}
    })
    .detach();
}
