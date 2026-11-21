use crate::{pull_request::Status, style};
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
icon!(
    pr_merged,
    r#"<path d="M5 7v6m0-2c4 0 9-1 9-4"/><circle cx="5" cy="5" r="2"/><circle cx="5" cy="15" r="2"/><circle cx="14" cy="5" r="2"/>"#
);

pub fn pr_status<'a>(size: u32, status: Status) -> Svg<'a> {
    match status {
        Status::Open => pull_request(size, style::GREEN),
        Status::Draft => pr_draft(size, style::MUTED),
        Status::Closed => pr_closed(size, style::RED),
        Status::Merged => pr_merged(size, style::PURPLE),
    }
}

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
