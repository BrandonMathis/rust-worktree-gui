use crate::{pull_request::Status, theme::Palette};
use iced::Color;
use iced::widget::{Svg, svg};
use std::sync::OnceLock;

macro_rules! icon {
    ($name:ident, $drawing:literal) => {
        pub fn $name<'a>(size: u32, color: Color) -> Svg<'a> {
            static HANDLE: OnceLock<svg::Handle> = OnceLock::new();
            let handle = HANDLE.get_or_init(|| svg::Handle::from_memory(concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><g fill="none" stroke="white" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">"#,
                $drawing,
                "</g></svg>"
            ).as_bytes()));
            svg(handle.clone()).width(size).height(size)
                .style(move |_, _| svg::Style { color: Some(color) })
        }
    };
}

icon!(
    branch,
    r#"<path d="M6 7v6m0-1c5 0 8-2 8-5"/><circle cx="6" cy="5" r="2"/><circle cx="6" cy="15" r="2"/><circle cx="14" cy="5" r="2"/>"#
);
icon!(
    pull_request,
    r#"<path d="M5 7v6M14 13V8q0-3-3-3H9m2-2L9 5l2 2"/><circle cx="5" cy="5" r="2"/><circle cx="5" cy="15" r="2"/><circle cx="14" cy="15" r="2"/>"#
);
icon!(
    pr_draft,
    r#"<path d="M5 7v6M14 13v-1m0-3v-1m-3-3h-1"/><circle cx="5" cy="5" r="2"/><circle cx="5" cy="15" r="2"/><circle cx="14" cy="15" r="2"/>"#
);
icon!(
    pr_closed,
    r#"<path d="M5 7v6m6-8 5 5m0-5-5 5"/><circle cx="5" cy="5" r="2"/><circle cx="5" cy="15" r="2"/>"#
);
// Drawn like a merge (the line bends into a middle-right circle) so it can't be
// mistaken for `branch`, whose third circle sits top-right.
icon!(
    pr_merged,
    r#"<path d="M5 6v8m0-8q0 4 8 4"/><circle cx="5" cy="4" r="2"/><circle cx="5" cy="16" r="2"/><circle cx="15" cy="10" r="2"/>"#
);

pub fn pr_status<'a>(size: u32, status: Status, p: Palette) -> Svg<'a> {
    match status {
        Status::Open => pull_request(size, p.green),
        Status::Draft => pr_draft(size, p.muted()),
        Status::Closed => pr_closed(size, p.red),
        Status::Merged => pr_merged(size, p.purple),
    }
}

// The main checkout (the repository directory itself, not a linked worktree).
icon!(
    home,
    r#"<path d="M3 9.5 10 3.5l7 6M5 8v8.5h10V8M8.5 16.5V12h3v4.5"/>"#
);
icon!(chevron_right, r#"<path d="m7.5 4.5 5.5 5.5-5.5 5.5"/>"#);
icon!(chevron_down, r#"<path d="m4.5 7.5 5.5 5.5 5.5-5.5"/>"#);
icon!(chevron_up, r#"<path d="m4.5 12.5 5.5-5.5 5.5 5.5"/>"#);
icon!(
    grip,
    r#"<path d="M7 5h.1M13 5h.1M7 10h.1M13 10h.1M7 15h.1M13 15h.1" stroke-width="2.5"/>"#
);
icon!(folder, r#"<path d="M3 5h5l1.6 2H17v9H3z"/>"#);
icon!(add, r#"<path d="M10 4v12M4 10h12"/>"#);
icon!(
    sidebar,
    r#"<rect x="2.5" y="4" width="15" height="12" rx="2"/><path d="M8 4v12"/>"#
);
icon!(warning, r#"<path d="m10 3 8 14H2zM10 8v3.5m0 2.5v.1"/>"#);
icon!(
    connecting,
    r#"<path d="M8 3a7.3 7.3 0 1 1-4.7 4.8M2 4.5v4h4"/>"#
);
icon!(empty, r#"<circle cx="10" cy="10" r="5.2"/>"#);
icon!(
    connected,
    r#"<circle cx="10" cy="10" r="5.2" fill="white"/>"#
);
// Eight-tooth gear around a hub, for Settings.
icon!(
    settings,
    r#"<path d="M15.6 7.8L17.9 8.6L17.9 11.4L15.6 12.2L15.5 12.4L16.6 14.6L14.6 16.6L12.4 15.5L12.2 15.6L11.4 17.9L8.6 17.9L7.8 15.6L7.6 15.5L5.4 16.6L3.4 14.6L4.5 12.4L4.4 12.2L2.1 11.4L2.1 8.6L4.4 7.8L4.5 7.6L3.4 5.4L5.4 3.4L7.6 4.5L7.8 4.4L8.6 2.1L11.4 2.1L12.2 4.4L12.4 4.5L14.6 3.4L16.6 5.4L15.5 7.6z"/><circle cx="10" cy="10" r="2.4"/>"#
);
// A painter's palette, for the Theme settings page.
icon!(
    palette,
    r#"<path d="M10 3a7 7 0 1 0 0 14c1.2 0 1.6-.9 1.2-1.8-.5-1 .1-2.2 1.3-2.2H15a2.5 2.5 0 0 0 2.5-2.5C17.5 6.3 14.2 3 10 3z"/><circle cx="6.5" cy="9" r=".8"/><circle cx="9" cy="6.3" r=".8"/><circle cx="12.6" cy="6.8" r=".8"/>"#
);

// tmux session badges on the right of a sidebar worktree row. The SVG `fill`
// is tinted by the style color like the strokes, so a filled dot stays a dot.
// A live session with nothing new: a small ring.
icon!(session_live, r#"<circle cx="10" cy="10" r="3"/>"#);
// New output since the user last looked: a filled dot.
icon!(
    session_activity,
    r#"<circle cx="10" cy="10" r="3.5" fill="white"/>"#
);
// A bell rang (for example, an agent finished or is waiting for input).
icon!(
    bell,
    r#"<path d="M6 13.5V9a4 4 0 0 1 8 0v4.5l1.5 1.5h-11zM8.5 17h3"/>"#
);
