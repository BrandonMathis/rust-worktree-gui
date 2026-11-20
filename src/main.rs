mod app;
mod config;
mod git;
mod terminal;

#[cfg(test)]
#[path = "../tests/git_fixtures.rs"]
mod git_fixtures;

fn main() -> iced::Result {
    iced::application(app::App::new, app::App::update, app::App::view)
        .title("Worktree Browser")
        .window(iced::window::Settings {
            size: iced::Size::new(980.0, 640.0),
            min_size: Some(iced::Size::new(820.0, 440.0)),
            ..iced::window::Settings::default()
        })
        .run()
}
