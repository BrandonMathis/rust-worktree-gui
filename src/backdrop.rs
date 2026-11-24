//! The blurred desktop behind the translucent window (macOS only).
//!
//! winit's `blur` window setting asks the window server for a background
//! blur, but on macOS 26 the server blurs a shape with tighter corners than
//! the 16pt curve AppKit clips the window's content to. The blur then shows
//! as a light crescent outside each rounded corner. An `NSVisualEffectView`
//! behind the content is clipped with the content, so its blur ends exactly
//! on the window's curve.

#[cfg(target_os = "macos")]
use iced::window;

/// Puts a behind-window blur under the content of `window`.
#[cfg(target_os = "macos")]
pub fn install(window: &dyn window::Window) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    use raw_window_handle::RawWindowHandle;

    let Some(main) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // The AppKit handle borrows the window's live content view (winit's view)
    // for this callback.
    let content = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    // The blur is a sibling placed below the content view, not a child of it:
    // a child would sit on top and take the content's mouse events.
    let Some(frame) = (unsafe { content.superview() }) else {
        return;
    };
    let blur = NSVisualEffectView::initWithFrame(main.alloc(), content.frame());
    blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    blur.setMaterial(NSVisualEffectMaterial::UnderWindowBackground);
    // Keep blurring while the window is inactive, as the window server did.
    blur.setState(NSVisualEffectState::Active);
    blur.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    frame.addSubview_positioned_relativeTo(&blur, NSWindowOrderingMode::Below, Some(content));
}
