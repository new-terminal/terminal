//! The menu bar and the app-wide keys. GPUI adds neither on its own.

use gpui_kit::{App, KeyBinding, Menu, MenuItem, SystemMenuType};

use crate::Core;

pub use menu_actions::*;

#[allow(
    clippy::derive_partial_eq_without_eq,
    reason = "GPUI's actions! macro writes the derives, not us"
)]
mod menu_actions {
    use gpui_kit::actions;

    actions!(
        new_terminal,
        [
            /// Quit the app.
            Quit,
            /// Hide the app's window.
            Hide,
            /// Hide every other app.
            HideOthers,
            /// Show every app again.
            ShowAll,
            /// Close the window, which quits: there is only one.
            CloseWindow,
            /// End every agent process.
            StopAllAgents,
        ]
    );
}

/// Installs the menus, the keys, and the rule that closing the window quits.
pub fn install(cx: &mut App) {
    // Every quit path must reach `cx.quit()`, because the quit routine runs
    // only from `on_app_quit`, and with no window left the app has nothing
    // to show.
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(window) = cx.active_window() {
            let _ = window.update(cx, |_, window, _| window.remove_window());
        }
    });
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &StopAllAgents, cx| cx.global::<Core>().0.stop_all());
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-.", StopAllAgents, None),
    ]);
    cx.set_menus([
        Menu::new("New Terminal").items([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide New Terminal", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit New Terminal", Quit),
        ]),
        Menu::new("Agents").items([MenuItem::action("Stop All Agents", StopAllAgents)]),
    ]);
}
