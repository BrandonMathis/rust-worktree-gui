mod actions;
mod app;
#[cfg(feature = "capture")]
mod capture;
mod config;
mod create;
mod git;
mod icons;
mod pull_request;
mod quit;
mod row_menu;
mod style;
mod terminal;
mod terminal_config;
mod tmux;

#[cfg(test)]
#[path = "../tests/git_fixtures.rs"]
mod git_fixtures;

fn main() -> iced::Result {
    let size = iced::Size::new(1120.0, 720.0);
    // Screenshot builds can start at another size, such as the minimum window.
    #[cfg(feature = "capture")]
    let size = capture::window_size().unwrap_or(size);
    let preferences = terminal_config::Preferences::load();
    let font = preferences
        .as_ref()
        .map_or(iced::Font::DEFAULT, |p| p.font.font_type);
    iced::application(
        move || app::App::new(preferences.clone()),
        app::App::update,
        app::App::view,
    )
    .default_font(font)
    .title("Worktree Browser")
    .subscription(app::App::subscription)
    .theme(iced::Theme::Dracula)
    .style(|_, theme| style::application(theme))
    .window(iced::window::Settings {
        size,
        transparent: true,
        blur: cfg!(target_os = "macos"),
        exit_on_close_request: false,
        min_size: Some(iced::Size::new(820.0, 440.0)),
        ..iced::window::Settings::default()
    })
    .run()
}
