mod actions;
mod app;
mod color_wheel;
mod config;
mod create;
mod git;
mod icons;
mod pull_request;
mod quit;
mod row_menu;
mod settings;
mod style;
mod terminal;
mod terminal_config;
mod theme;
mod tmux;

#[cfg(test)]
#[path = "../tests/git_fixtures.rs"]
mod git_fixtures;

fn main() -> iced::Result {
    let preferences = terminal_config::Preferences::load();
    let font = preferences
        .as_ref()
        .map_or(iced::Font::DEFAULT, |p| p.font.font_type);
    let window_radius = style::window_radius();
    iced::application(
        move || app::App::new(preferences.clone(), window_radius),
        app::App::update,
        app::App::view,
    )
    .default_font(font)
    .title("Worktree Browser")
    .subscription(app::App::subscription)
    // Both read the app's palette, so theme edits apply to the whole window live.
    .theme(app::App::theme)
    .style(|app, _| style::application(app.palette()))
    .window(iced::window::Settings {
        size: iced::Size::new(1120.0, 720.0),
        transparent: true,
        blur: cfg!(target_os = "macos"),
        exit_on_close_request: false,
        min_size: Some(iced::Size::new(820.0, 440.0)),
        ..iced::window::Settings::default()
    })
    .run()
}
