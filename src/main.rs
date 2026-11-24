mod actions;
mod app;
mod backdrop;
#[cfg(feature = "capture")]
mod capture;
mod color_wheel;
mod config;
mod confirmation;
mod create;
mod git;
mod icons;
#[cfg(feature = "perf")]
mod perf;
mod pull_request;
mod quit;
mod row_menu;
mod runner;
mod settings;
mod style;
mod terminal;
mod terminal_config;
mod theme;
mod tmux;
mod version;
mod watch;

#[cfg(test)]
#[path = "../tests/git_fixtures.rs"]
mod git_fixtures;

fn main() -> iced::Result {
    // SAFETY: this is the first statement in `main`, so no other thread exists yet.
    // It keeps a hook's `GIT_DIR` (and friends) out of the embedded terminal.
    unsafe { runner::remove_git_overrides_from_process() };
    // Latency builds only: `--perf-latency` measures instead of opening a window.
    #[cfg(feature = "perf")]
    if let Some(code) = perf::run_if_requested() {
        std::process::exit(code);
    }
    let size = iced::Size::new(1120.0, 720.0);
    // Screenshot builds can start at another size, such as the minimum window.
    #[cfg(feature = "capture")]
    let size = capture::window_size().unwrap_or(size);
    let preferences = terminal_config::Preferences::load();
    let font = preferences
        .as_ref()
        .map_or(iced::Font::DEFAULT, |p| p.font.font_type);
    let window_radius = style::window_radius();
    let title = version::window_title();
    iced::application(
        move || app::App::new(preferences.clone(), window_radius),
        app::App::update,
        app::App::view,
    )
    .default_font(font)
    // The version, plus the branch and commit for a non-release build. Iced asks
    // for the title on every update, so the closure hands out a copy.
    .title(move |_: &app::App| title.clone())
    .subscription(app::App::subscription)
    // Both read the app's palette, so theme edits apply to the whole window live.
    .theme(app::App::theme)
    .style(|app, _| style::application(app.palette()))
    .window(iced::window::Settings {
        size,
        transparent: true,
        // On macOS the blur is an AppKit view (`backdrop`), which follows the
        // window's rounded corners; winit's blur doesn't.
        blur: false,
        exit_on_close_request: false,
        min_size: Some(iced::Size::new(820.0, 440.0)),
        ..iced::window::Settings::default()
    })
    .run()
}
