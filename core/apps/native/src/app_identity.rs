#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CString;
    use std::os::raw::c_void;

    use objc::{class, msg_send, sel, sel_impl};
    use objc::runtime::Object;

    const ICON_BYTES: &[u8] = include_bytes!("../../desktop/src-tauri/icons/icon.png");

    pub(super) fn apply() {
        unsafe {
            let process_info: *mut Object = msg_send![class!(NSProcessInfo), processInfo];
            let name = ns_string("ctx");
            let _: () = msg_send![process_info, setProcessName: name];

            let data: *mut Object = msg_send![
                class!(NSData),
                dataWithBytes: ICON_BYTES.as_ptr() as *const c_void
                length: ICON_BYTES.len()
            ];
            let image: *mut Object = msg_send![class!(NSImage), alloc];
            let image: *mut Object = msg_send![image, initWithData: data];
            let _: *mut Object = msg_send![image, autorelease];

            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, setApplicationIconImage: image];
        }
    }

    unsafe fn ns_string(value: &str) -> *mut Object {
        let c_string =
            CString::new(value).unwrap_or_else(|_| CString::new("").expect("CString fallback"));
        msg_send![class!(NSString), stringWithUTF8String: c_string.as_ptr()]
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn apply_app_identity() {
    macos::apply();
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn apply_app_identity() {}
