#[cfg(target_os = "macos")]
use iced::window;

#[cfg(target_os = "macos")]
pub fn install_shortcut(parent: &dyn window::Window) {
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSView};
    use raw_window_handle::RawWindowHandle;

    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = parent.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // The AppKit handle borrows the window's live NSView for this callback.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(window) = view.window() else { return };
    let app = NSApplication::sharedApplication(main);
    let Some(menu) = app.mainMenu() else { return };
    for item in &menu.itemArray() {
        if let Some(submenu) = item.submenu() {
            for item in &submenu.itemArray() {
                if item.action() == Some(sel!(terminate:)) {
                    // Winit's menu calls terminate: directly. Keep its Cmd+Q binding,
                    // but send the standard window-close action so iced can confirm it.
                    unsafe {
                        item.setTarget(Some(&window));
                        item.setAction(Some(sel!(performClose:)));
                    }
                    return;
                }
            }
        }
    }
}
