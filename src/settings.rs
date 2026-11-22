//! The full-screen Settings modal: a list of pages on the left, the selected
//! page on the right, and Cancel / Save & Confirm at the bottom.
//!
//! This is a small Elm component nested in `App`: `Draft` is its state,
//! `Message` its actions and `Draft::update` its reducer. The app forwards
//! `app::Message::Settings(..)` here and only acts on the returned `Outcome`,
//! the way a React parent passes `onCancel` / `onSave` callbacks.

use crate::style::{self, Button, Corners, Panel};
use crate::theme::{self, Hsv, Palette, Preset, Slot};
use crate::{color_wheel, icons};
use iced::widget::{
    Space, Svg, button, column, container, pick_list, row, scrollable, slider, text, text_input,
};
use iced::{Center, Element, Fill};

/// Diameter of the color wheel, in logical pixels.
const WHEEL_SIZE: f32 = 200.0;

/// Pages in the left-hand list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Theme,
}

impl Page {
    const ALL: [Self; 1] = [Self::Theme];

    fn label(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
        }
    }

    fn icon<'a>(self, palette: Palette) -> Svg<'a> {
        match self {
            Self::Theme => icons::palette(14, palette.purple),
        }
    }
}

/// Unsaved settings. The whole app draws with `palette` while the modal is
/// open, so every change previews live. Cancel drops the draft.
pub struct Draft {
    page: Page,
    palette: Palette,
    /// The color the wheel is editing.
    selected: Slot,
    /// The wheel and brightness position of `selected`. It is kept apart from
    /// the palette so the hue survives when saturation or brightness hits 0.
    hsv: Hsv,
    /// Each hex field as typed, indexed by `Slot as usize`, so partial input
    /// such as `#BD9` is never overwritten while the user types.
    hex: [String; Slot::ALL.len()],
}

#[derive(Clone, Debug)]
pub enum Message {
    Page(Page),
    Select(Slot),
    Wheel(Hsv),
    Brightness(f32),
    Hex(Slot, String),
    Preset(Preset),
    Cancel,
    Save,
}

/// What the app should do when the modal closes.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Cancel,
    Save(Palette),
}

impl Draft {
    pub fn new(palette: Palette) -> Self {
        let selected = Slot::Purple;
        Self {
            page: Page::Theme,
            palette,
            selected,
            hsv: Hsv::from_color(palette.get(selected)),
            hex: hex_codes(&palette),
        }
    }

    pub fn palette(&self) -> Palette {
        self.palette
    }

    pub fn update(&mut self, message: Message) -> Option<Outcome> {
        match message {
            Message::Page(page) => self.page = page,
            Message::Select(slot) => self.select(slot),
            Message::Wheel(hsv) => self.set_hsv(hsv),
            Message::Brightness(value) => self.set_hsv(Hsv { value, ..self.hsv }),
            Message::Hex(slot, input) => {
                self.select(slot);
                if let Some(color) = theme::parse_hex(&input) {
                    self.palette.set(slot, color);
                    self.hsv = Hsv::from_color(color);
                }
                self.hex[slot as usize] = input;
            }
            Message::Preset(preset) => {
                self.palette = preset.palette();
                self.hex = hex_codes(&self.palette);
                self.hsv = Hsv::from_color(self.palette.get(self.selected));
            }
            Message::Cancel => return Some(Outcome::Cancel),
            Message::Save => return Some(Outcome::Save(self.palette)),
        }
        None
    }

    fn select(&mut self, slot: Slot) {
        if slot != self.selected {
            self.selected = slot;
            self.hsv = Hsv::from_color(self.palette.get(slot));
            // Moving to another color tidies any half-typed hex code.
            self.hex = hex_codes(&self.palette);
        }
    }

    fn set_hsv(&mut self, hsv: Hsv) {
        self.hsv = hsv;
        let color = hsv.to_color();
        self.palette.set(self.selected, color);
        self.hex[self.selected as usize] = theme::to_hex(color);
    }

    /// `corners` rounds the panels that sit in the window's bottom corners.
    pub fn view(&self, corners: Corners) -> Element<'_, Message> {
        let p = self.palette;
        let pages = Page::ALL.map(|page| {
            button(
                row![page.icon(p), text(page.label()).size(13)]
                    .spacing(7)
                    .align_y(Center),
            )
            .width(Fill)
            .padding([5, 8])
            .style(style::button(
                p,
                if page == self.page {
                    Button::Selected
                } else {
                    Button::Nav
                },
            ))
            .on_press(Message::Page(page))
            .into()
        });
        let sidebar = container(
            column![
                text("Settings").size(13).color(p.muted()),
                column(pages).spacing(2)
            ]
            .spacing(8),
        )
        .padding(8)
        .width(180)
        .height(Fill)
        .style(style::panel_in(p, Panel::Glass, corners.left()));

        let page = match self.page {
            Page::Theme => self.theme_page(),
        };
        let footer = row![
            Space::new().width(Fill),
            button(text("Cancel").size(13))
                .style(style::button(p, Button::Secondary))
                .on_press(Message::Cancel),
            button(text("Save & Confirm").size(13))
                .style(style::button(p, Button::Primary))
                .on_press(Message::Save),
        ]
        .spacing(6);
        let main = container(column![container(page).height(Fill), footer].spacing(10))
            .padding(14)
            .width(Fill)
            .height(Fill)
            .style(style::panel_in(p, Panel::Glass, corners.right()));
        row![sidebar, main].spacing(4).height(Fill).into()
    }

    fn theme_page(&self) -> Element<'_, Message> {
        let p = self.palette;
        let header = column![
            text("Theme").size(18),
            text("App colors. The terminal keeps your Alacritty colors.")
                .size(12)
                .color(p.muted()),
        ]
        .spacing(2);
        let presets = row![
            text("Preset").size(13),
            pick_list(
                Preset::ALL,
                Preset::matching(&self.palette),
                Message::Preset
            )
            .placeholder("Custom")
            .text_size(13)
            .padding([4, 8])
            .width(240)
            .style(style::action_picker(p))
            .menu_style(style::action_menu(p)),
        ]
        .spacing(8)
        .align_y(Center);

        let color = self.palette.get(self.selected);
        let picker = column![
            color_wheel::view(self.hsv, p, WHEEL_SIZE, Message::Wheel),
            row![
                text("Brightness").size(12).color(p.muted()),
                slider(0.0..=1.0, self.hsv.value, Message::Brightness)
                    .step(0.001)
                    .style(style::brightness(p, color)),
            ]
            .spacing(8)
            .align_y(Center),
            row![
                chip(p, color, 16),
                text(self.selected.label()).size(13),
                text(theme::to_hex(color)).size(12).color(p.muted()),
            ]
            .spacing(6)
            .align_y(Center),
        ]
        .spacing(10)
        .width(WHEEL_SIZE);

        let colors = column(Slot::ALL.map(|slot| self.color_row(slot))).spacing(2);
        column![
            header,
            presets,
            row![
                picker,
                // Capped so each hex field stays next to its name on wide windows.
                scrollable(colors.padding(iced::padding::right(10))).width(380)
            ]
            .spacing(18)
            .height(Fill),
        ]
        .spacing(12)
        .into()
    }

    /// A color chip and name (click to edit it on the wheel) and its hex field.
    fn color_row(&self, slot: Slot) -> Element<'_, Message> {
        let p = self.palette;
        let label = button(
            row![chip(p, p.get(slot), 18), text(slot.label()).size(13)]
                .spacing(8)
                .align_y(Center),
        )
        .width(Fill)
        .padding([3, 6])
        .style(style::button(
            p,
            if slot == self.selected {
                Button::Selected
            } else {
                Button::Nav
            },
        ))
        .on_press(Message::Select(slot));
        let hex = text_input("#RRGGBB", &self.hex[slot as usize])
            .on_input(move |input| Message::Hex(slot, input))
            .font(iced::Font::MONOSPACE)
            .size(12)
            .padding([4, 6])
            .width(92)
            .style(style::input(p));
        row![label, hex].spacing(6).align_y(Center).into()
    }
}

fn chip<'a>(p: Palette, color: iced::Color, size: u32) -> Element<'a, Message> {
    container(Space::new())
        .width(size)
        .height(size)
        .style(style::swatch(p, color))
        .into()
}

fn hex_codes(palette: &Palette) -> [String; Slot::ALL.len()] {
    Slot::ALL.map(|slot| theme::to_hex(palette.get(slot)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_a_color_moves_the_wheel_to_it() {
        let mut draft = Draft::new(Palette::default());
        draft.update(Message::Select(Slot::Cyan));
        assert_eq!(draft.selected, Slot::Cyan);
        assert_eq!(draft.hsv, Hsv::from_color(Palette::default().cyan));
    }

    #[test]
    fn dragging_the_wheel_and_brightness_recolor_only_the_selected_slot_live() {
        let original = Palette::default();
        let mut draft = Draft::new(original);
        draft.update(Message::Select(Slot::Green));
        draft.update(Message::Wheel(Hsv {
            hue: 0.0,
            saturation: 1.0,
            value: 1.0,
        }));
        assert_eq!(theme::to_hex(draft.palette().green), "#FF0000");
        assert_eq!(draft.hex[Slot::Green as usize], "#FF0000");
        draft.update(Message::Brightness(0.5));
        assert_eq!(theme::to_hex(draft.palette().green), "#800000");
        // Hue is kept while brightness passes through black.
        draft.update(Message::Brightness(0.0));
        draft.update(Message::Brightness(1.0));
        assert_eq!(theme::to_hex(draft.palette().green), "#FF0000");
        let untouched = Slot::ALL.into_iter().filter(|slot| *slot != Slot::Green);
        for slot in untouched {
            assert_eq!(draft.palette().get(slot), original.get(slot), "{slot:?}");
        }
    }

    #[test]
    fn typing_a_hex_code_applies_once_valid_and_keeps_partial_input() {
        let mut draft = Draft::new(Palette::default());
        draft.update(Message::Hex(Slot::Red, "#12".into()));
        assert_eq!(draft.selected, Slot::Red);
        assert_eq!(draft.hex[Slot::Red as usize], "#12");
        assert_eq!(draft.palette().red, Palette::default().red);
        draft.update(Message::Hex(Slot::Red, "#123456".into()));
        assert_eq!(theme::to_hex(draft.palette().red), "#123456");
        assert_eq!(draft.hsv, Hsv::from_color(draft.palette().red));
        // Moving to another color resets a half-typed code to the real value.
        draft.update(Message::Hex(Slot::Red, "#9".into()));
        draft.update(Message::Select(Slot::Pink));
        assert_eq!(draft.hex[Slot::Red as usize], "#123456");
    }

    #[test]
    fn presets_replace_every_color_and_edits_show_as_custom() {
        let mut draft = Draft::new(Palette::default());
        assert_eq!(Preset::matching(&draft.palette()), Some(Preset::Dracula));
        draft.update(Message::Preset(Preset::VanHelsing));
        assert_eq!(draft.palette(), Preset::VanHelsing.palette());
        assert_eq!(draft.hex[Slot::Background as usize], "#0B0D0F");
        draft.update(Message::Hex(Slot::Yellow, "#000000".into()));
        assert_eq!(Preset::matching(&draft.palette()), None);
    }

    #[test]
    fn cancel_and_save_report_the_outcome_with_the_edited_palette() {
        let mut draft = Draft::new(Palette::default());
        draft.update(Message::Preset(Preset::Buffy));
        assert_eq!(draft.update(Message::Cancel), Some(Outcome::Cancel));
        assert_eq!(
            draft.update(Message::Save),
            Some(Outcome::Save(Preset::Buffy.palette()))
        );
    }
}
