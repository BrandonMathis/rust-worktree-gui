use iced::{Task, window};
use rfd::{AsyncMessageDialog, MessageButtons, MessageDialogResult};

const QUIT: &str = "Quit";

pub fn confirm(id: window::Id) -> Task<bool> {
    // Construct the alert on the UI thread and attach it to the app window.
    window::run(id, |parent| {
        AsyncMessageDialog::new()
            .set_parent(parent)
            .set_title("Quit Worktree Browser?")
            .set_description("Your tmux sessions will keep running.")
            .set_buttons(MessageButtons::OkCancelCustom(QUIT.into(), "Cancel".into()))
            .show()
    })
    .then(|answer| Task::perform(answer, is_confirmed))
}

fn is_confirmed(answer: MessageDialogResult) -> bool {
    matches!(answer, MessageDialogResult::Custom(button) if button == QUIT)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_explicit_quit_button_confirms() {
        assert!(is_confirmed(MessageDialogResult::Custom("Quit".into())));
        assert!(!is_confirmed(MessageDialogResult::Custom("Cancel".into())));
        assert!(!is_confirmed(MessageDialogResult::Cancel));
    }
}
