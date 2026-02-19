use super::*;

#[cfg(target_os = "macos")]
use objc2::ffi::class_addMethod;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
#[cfg(target_os = "macos")]
use objc2::MainThreadOnly;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
#[cfg(target_os = "macos")]
use std::sync::{Once, OnceLock};

#[cfg(target_os = "macos")]
static DOCK_MENU_INSTALL_ONCE: Once = Once::new();
#[cfg(target_os = "macos")]
static DOCK_MENU_APP: OnceLock<tauri::AppHandle> = OnceLock::new();

#[cfg(target_os = "macos")]
extern "C" fn application_dock_menu(
    this: &AnyObject,
    _cmd: Sel,
    _sender: &AnyObject,
) -> *mut AnyObject {
    let Some(app) = DOCK_MENU_APP.get() else {
        return std::ptr::null_mut();
    };
    let Some(menu) = build_dock_menu(app, this) else {
        return std::ptr::null_mut();
    };
    Retained::autorelease_return(menu).cast::<AnyObject>()
}

#[cfg(target_os = "macos")]
extern "C" fn dock_open_new_window(_this: &AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
    let Some(app) = DOCK_MENU_APP.get() else {
        return;
    };
    if let Err(err) = open_launcher_window(app) {
        eprintln!("dock menu new window failed: {err:#}");
    }
}

#[cfg(target_os = "macos")]
extern "C" fn dock_open_recent_workspace(_this: &AnyObject, _cmd: Sel, sender: *mut AnyObject) {
    let Some(app) = DOCK_MENU_APP.get() else {
        return;
    };
    let Some(workspace_id) = workspace_id_from_sender(sender) else {
        return;
    };
    let registry = app.state::<WorkspaceWindowRegistry>();
    if let Err(err) = focus_or_open_workspace_window(app, &registry, &workspace_id) {
        eprintln!(
            "dock menu open workspace '{}' failed: {err:#}",
            workspace_id
        );
    }
}

#[cfg(target_os = "macos")]
fn workspace_id_from_sender(sender: *mut AnyObject) -> Option<String> {
    let item = unsafe { (sender as *mut NSMenuItem).as_ref() }?;
    let represented = item.representedObject()?;
    let workspace_id = represented
        .downcast::<NSString>()
        .ok()?
        .to_string()
        .trim()
        .to_string();
    if workspace_id.is_empty() {
        return None;
    }
    Some(workspace_id)
}

#[cfg(target_os = "macos")]
fn build_action_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    target: &AnyObject,
    represented_workspace_id: Option<&str>,
) -> Option<Retained<NSMenuItem>> {
    let title = NSString::from_str(title);
    let key_equivalent = NSString::from_str("");
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &title,
            Some(action),
            &key_equivalent,
        )
    };
    unsafe {
        item.setTarget(Some(target));
    }
    if let Some(workspace_id) = represented_workspace_id {
        let workspace_id = NSString::from_str(workspace_id);
        unsafe {
            item.setRepresentedObject(Some(&workspace_id));
        }
    }
    Some(item)
}

#[cfg(target_os = "macos")]
fn build_dock_menu(app: &tauri::AppHandle, target: &AnyObject) -> Option<Retained<NSMenu>> {
    let mtm = MainThreadMarker::new()?;
    let title = NSString::from_str("ctx");
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
    menu.setAutoenablesItems(false);

    let new_window =
        build_action_item(mtm, "New Window", sel!(ctxDockOpenNewWindow:), target, None)?;
    menu.addItem(&new_window);

    let registry = app.state::<WorkspaceWindowRegistry>();
    let recents = registry.recent_workspaces();
    let separator = NSMenuItem::separatorItem(mtm);
    menu.addItem(&separator);

    if recents.is_empty() {
        let empty = build_action_item(
            mtm,
            "No Recent Workspaces",
            sel!(ctxDockOpenRecentWorkspace:),
            target,
            None,
        )?;
        empty.setEnabled(false);
        menu.addItem(&empty);
        return Some(menu);
    }

    for recent in recents {
        let item = build_action_item(
            mtm,
            &recent.label,
            sel!(ctxDockOpenRecentWorkspace:),
            target,
            Some(&recent.workspace_id),
        )?;
        menu.addItem(&item);
    }
    Some(menu)
}

#[cfg(target_os = "macos")]
fn install_delegate_dock_menu_methods(delegate_class: *mut AnyClass) {
    unsafe {
        let _ = class_addMethod(
            delegate_class,
            sel!(applicationDockMenu:),
            std::mem::transmute::<extern "C" fn(&AnyObject, Sel, &AnyObject) -> *mut AnyObject, Imp>(
                application_dock_menu,
            ),
            b"@@:@\0".as_ptr().cast(),
        );
        let _ = class_addMethod(
            delegate_class,
            sel!(ctxDockOpenNewWindow:),
            std::mem::transmute::<extern "C" fn(&AnyObject, Sel, *mut AnyObject), Imp>(
                dock_open_new_window,
            ),
            b"v@:@\0".as_ptr().cast(),
        );
        let _ = class_addMethod(
            delegate_class,
            sel!(ctxDockOpenRecentWorkspace:),
            std::mem::transmute::<extern "C" fn(&AnyObject, Sel, *mut AnyObject), Imp>(
                dock_open_recent_workspace,
            ),
            b"v@:@\0".as_ptr().cast(),
        );
    }
}

#[cfg(target_os = "macos")]
pub(super) fn install_macos_dock_menu_bridge(app: tauri::AppHandle) {
    let _ = DOCK_MENU_APP.set(app);
    DOCK_MENU_INSTALL_ONCE.call_once(|| unsafe {
        let Some(mtm) = MainThreadMarker::new() else {
            eprintln!("dock menu bridge skipped: no main thread marker");
            return;
        };
        let ns_app = NSApplication::sharedApplication(mtm);
        let delegate: *mut AnyObject = msg_send![&*ns_app, delegate];
        if delegate.is_null() {
            eprintln!("dock menu bridge skipped: NSApplication delegate is null");
            return;
        }
        let delegate_class: *mut AnyClass = msg_send![delegate, class];
        if delegate_class.is_null() {
            eprintln!("dock menu bridge skipped: delegate class is null");
            return;
        }
        install_delegate_dock_menu_methods(delegate_class);
    });
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_macos_dock_menu_bridge(_app: tauri::AppHandle) {}
