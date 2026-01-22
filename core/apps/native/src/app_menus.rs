use gpui::{App, KeyBinding, Menu, MenuItem, actions};
#[cfg(target_os = "macos")]
use gpui::SystemMenuType;
use gpui_component::Root;

use crate::app::{ShellRoute, ShellView, open_shell_window};

actions!(
    ctx_native_menu,
    [
        OpenSettings,
        NewSettingsWindow,
        NewLauncherWindow,
        NewTask,
        ToggleSidebar,
        Quit,
        Hide,
        HideOthers,
        ShowAll
    ]
);

pub(crate) fn init(cx: &mut App) {
    cx.on_action(open_settings);
    cx.on_action(new_settings_window);
    cx.on_action(new_launcher_window);
    cx.on_action(new_task);
    cx.on_action(toggle_sidebar);
    cx.on_action(quit);
    cx.on_action(hide);
    cx.on_action(hide_others);
    cx.on_action(show_all);

    cx.bind_keys([
        KeyBinding::new("cmd-n", NewTask, None),
        KeyBinding::new("ctrl-n", NewTask, None),
        KeyBinding::new("shift-cmd-n", NewLauncherWindow, None),
        KeyBinding::new("shift-ctrl-n", NewLauncherWindow, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("shift-cmd-,", NewSettingsWindow, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("ctrl-b", ToggleSidebar, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
    ]);

    set_app_menus(cx);
}

fn set_app_menus(cx: &mut App) {
    let mut app_items = vec![
        MenuItem::action("Settings...", OpenSettings),
        MenuItem::action("New Settings Window", NewSettingsWindow),
        MenuItem::separator(),
    ];

    #[cfg(target_os = "macos")]
    {
        app_items.push(MenuItem::os_submenu("Services", SystemMenuType::Services));
        app_items.push(MenuItem::separator());
        app_items.push(MenuItem::action("Hide ctx", Hide));
        app_items.push(MenuItem::action("Hide Others", HideOthers));
        app_items.push(MenuItem::action("Show All", ShowAll));
        app_items.push(MenuItem::separator());
    }

    app_items.push(MenuItem::action("Quit ctx", Quit));

    let file_items = vec![
        MenuItem::action("New Task", NewTask),
        MenuItem::action("New Launcher Window", NewLauncherWindow),
    ];
    let view_items = vec![MenuItem::action("Toggle Sidebar", ToggleSidebar)];

    cx.set_menus(vec![
        Menu {
            name: "ctx".into(),
            items: app_items,
        },
        Menu {
            name: "File".into(),
            items: file_items,
        },
        Menu {
            name: "View".into(),
            items: view_items,
        },
    ]);
}

fn open_settings(_: &OpenSettings, cx: &mut App) {
    if open_route_in_active_window(ShellRoute::Settings, cx) {
        return;
    }
    if let Err(err) = open_shell_window(cx, ShellRoute::Settings) {
        eprintln!("ctx-native: open settings window failed: {err}");
    }
}

fn new_settings_window(_: &NewSettingsWindow, cx: &mut App) {
    if let Err(err) = open_shell_window(cx, ShellRoute::Settings) {
        eprintln!("ctx-native: open settings window failed: {err}");
    }
}

fn new_launcher_window(_: &NewLauncherWindow, cx: &mut App) {
    if let Err(err) = open_shell_window(cx, ShellRoute::AppSettings) {
        eprintln!("ctx-native: open launcher window failed: {err}");
    }
}

fn new_task(_: &NewTask, cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };

    let _ = active_window.update(cx, |root_view, _window, cx| {
        let Ok(root) = root_view.downcast::<Root>() else {
            return;
        };

        root.update(cx, |root, cx| {
            let Ok(shell_view) = root.view().clone().downcast::<ShellView>() else {
                return;
            };
            let _ = shell_view.update(cx, |view, cx| {
                if view.route != ShellRoute::Workbench {
                    view.set_route(ShellRoute::Workbench, cx);
                }
                view.focus_new_task_shortcut(cx);
            });
        });
    });
}

fn toggle_sidebar(_: &ToggleSidebar, cx: &mut App) {
    let Some(active_window) = cx.active_window() else {
        return;
    };

    let _ = active_window.update(cx, |root_view, _window, cx| {
        let Ok(root) = root_view.downcast::<Root>() else {
            return;
        };

        root.update(cx, |root, cx| {
            let Ok(shell_view) = root.view().clone().downcast::<ShellView>() else {
                return;
            };
            let _ = shell_view.update(cx, |view, cx| {
                if view.route != ShellRoute::Workbench {
                    return;
                }
                let collapsed = view.sidebar_collapsed;
                view.set_sidebar_collapsed(!collapsed, cx);
            });
        });
    });
}

fn quit(_: &Quit, cx: &mut App) {
    cx.quit();
}

fn hide(_: &Hide, cx: &mut App) {
    cx.hide();
}

fn hide_others(_: &HideOthers, cx: &mut App) {
    cx.hide_other_apps();
}

fn show_all(_: &ShowAll, cx: &mut App) {
    cx.unhide_other_apps();
}

fn open_route_in_active_window(route: ShellRoute, cx: &mut App) -> bool {
    let Some(active_window) = cx.active_window() else {
        return false;
    };

    active_window
        .update(cx, |root_view, _window, cx| {
            let Ok(root) = root_view.downcast::<Root>() else {
                return false;
            };

            root.update(cx, |root, cx| {
                let Ok(shell_view) = root.view().clone().downcast::<ShellView>() else {
                    return false;
                };
                let _ = shell_view.update(cx, |view, cx| view.set_route(route, cx));
                true
            })
        })
        .unwrap_or(false)
}
