use crate::backend::{
    Backend, Command, LinkAction, MouseButton, RenderableContent,
};
use crate::bindings::{BindingAction, BindingsLayout, InputKind};
use crate::block_glyph::BlockGlyph;
use crate::terminal::{Event, Terminal};
use crate::theme::TerminalStyle;
use alacritty_terminal::index::Point as TerminalGridPoint;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::{cell, TermMode};
use alacritty_terminal::vte::ansi::{self as ansi, CursorShape, NamedColor};
use iced::alignment::Vertical;
use iced::border::Radius;
use iced::font::{Style as FontStyle, Weight as FontWeight};
use iced::mouse::{Cursor, ScrollDelta};
use iced::widget::canvas::{Frame, Path, Text};
use iced::widget::container;
use iced::{Color, Element, Length, Point, Rectangle, Size, Theme};
use iced_core::clipboard::Kind as ClipboardKind;
use iced_core::keyboard::{Key, Modifiers};
use iced_core::mouse::{self, Click};
use iced_core::text::{Alignment, LineHeight, Shaping};
use iced_core::widget::operation::{self, Focusable};
use iced_graphics::core::widget::{tree, Tree};
use iced_graphics::core::Widget;
use iced_graphics::geometry::Stroke;

pub struct TerminalView<'a> {
    term: &'a Terminal,
    dynamic_padding_scale: Option<f32>,
    backing_scale: f32,
    corner_radius: Radius,
}

impl<'a> TerminalView<'a> {
    pub fn show(term: &'a Terminal) -> Element<'a, Event> {
        Self::show_at_scale(term, 1.0)
    }

    /// Show an unpadded terminal at the window's physical-to-logical scale.
    pub fn show_at_scale(term: &'a Terminal, scale: f32) -> Element<'a, Event> {
        Self::show_rounded(term, scale, Radius::default())
    }

    /// Patch: like `show_at_scale`, with rounded corners. Upstream always
    /// painted a square background, which covered the rounded corners of
    /// the pane it is embedded in.
    pub fn show_rounded(
        term: &'a Terminal,
        scale: f32,
        corner_radius: impl Into<Radius>,
    ) -> Element<'a, Event> {
        Self::show_with_options(term, None, scale, corner_radius.into())
    }

    /// Optionally distribute leftover backing pixels evenly around the grid.
    pub fn show_with_dynamic_padding(
        term: &'a Terminal,
        scale: Option<f32>,
    ) -> Element<'a, Event> {
        Self::show_with_options(
            term,
            scale,
            scale.unwrap_or(1.0),
            Radius::default(),
        )
    }

    fn show_with_options(
        term: &'a Terminal,
        padding: Option<f32>,
        scale: f32,
        corner_radius: Radius,
    ) -> Element<'a, Event> {
        container(Self {
            term,
            dynamic_padding_scale: padding,
            backing_scale: scale.max(1.0),
            corner_radius,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| {
            let mut style = term.theme.container_style();
            style.border.radius = corner_radius;
            style
        })
        .into()
    }

    pub fn focus<Message: 'static>(
        id: iced::widget::Id,
    ) -> iced::Task<Message> {
        iced::widget::operation::focus(id)
    }

    fn is_cursor_in_layout(
        &self,
        cursor: Cursor,
        layout: iced_graphics::core::Layout<'_>,
    ) -> bool {
        if let Some(cursor_position) = cursor.position() {
            let layout_position = layout.position();
            let layout_size = layout.bounds();
            let is_triggered = cursor_position.x >= layout_position.x
                && cursor_position.y >= layout_position.y
                && cursor_position.x < (layout_position.x + layout_size.width)
                && cursor_position.y < (layout_position.y + layout_size.height);

            return is_triggered;
        }

        false
    }

    fn is_cursor_hovered_hyperlink(&self, state: &TerminalViewState) -> bool {
        let content = self.term.backend.renderable_content();
        if let Some(hyperlink_range) = &content.hovered_hyperlink {
            return hyperlink_range.contains(&state.mouse_position_on_grid);
        }

        false
    }

    fn handle_resize(
        &mut self,
        state: &mut TerminalViewState,
        layout: iced_graphics::core::Layout<'_>,
        shell: &mut iced_graphics::core::Shell<'_, Event>,
    ) {
        let layout_size = layout.bounds().size();
        if state.size != layout_size {
            state.size = layout_size;
            let cmd = Command::Resize(
                Some(layout_size),
                Some(self.term.font.measure),
            );
            shell.publish(Event::BackendCall(self.term.id, cmd));
        }
    }

    fn handle_focus(
        &self,
        event: &iced_core::Event,
        state: &mut TerminalViewState,
        is_cursor_in_layout: bool,
    ) {
        use iced::Event::Mouse;
        use iced_core::mouse::{Button::Left, Event::ButtonPressed};

        if let Mouse(ButtonPressed(Left)) = event {
            state.focus = is_cursor_in_layout;
        }
    }

    fn handle_mouse_event(
        &self,
        state: &mut TerminalViewState,
        layout_position: Point,
        cursor_position: Point,
        event: &iced::mouse::Event,
    ) -> Vec<Command> {
        let mut commands = Vec::new();
        let terminal_content = self.term.backend.renderable_content();
        let terminal_mode = terminal_content.terminal_mode;

        match event {
            iced_core::mouse::Event::ButtonPressed(
                iced_core::mouse::Button::Left,
            ) => {
                if !state.is_focused() {
                    return Vec::default();
                }

                Self::handle_left_button_pressed(
                    state,
                    &terminal_mode,
                    cursor_position,
                    layout_position,
                    &mut commands,
                );
            },
            iced_core::mouse::Event::CursorMoved { position } => {
                if !state.is_focused() {
                    return Vec::default();
                }

                Self::handle_cursor_moved(
                    state,
                    self.term.backend.renderable_content(),
                    position,
                    layout_position,
                    &mut commands,
                );
            },
            iced_core::mouse::Event::ButtonReleased(
                iced_core::mouse::Button::Left,
            ) => {
                if !state.is_focused() {
                    return Vec::default();
                }

                Self::handle_button_released(
                    state,
                    &terminal_mode,
                    &self.term.bindings,
                    &mut commands,
                );
            },
            iced::mouse::Event::WheelScrolled { delta } => {
                let lines = state.wheel_lines(
                    *delta,
                    terminal_content.terminal_size.cell_height,
                    self.backing_scale,
                );
                if lines != 0 {
                    // Mouse protocols use visible cell coordinates, not history line numbers.
                    let point = Backend::selection_point(
                        cursor_position.x - layout_position.x,
                        cursor_position.y - layout_position.y,
                        &terminal_content.terminal_size,
                        0,
                    );
                    commands.push(Command::WheelScroll(
                        lines,
                        state.keyboard_modifiers,
                        point,
                    ));
                }
            },
            _ => {},
        }

        commands
    }

    fn handle_left_button_pressed(
        state: &mut TerminalViewState,
        terminal_mode: &TermMode,
        cursor_position: Point,
        layout_position: Point,
        commands: &mut Vec<Command>,
    ) {
        let cmd = if terminal_mode.intersects(TermMode::MOUSE_MODE) {
            Command::MouseReport(
                MouseButton::LeftButton,
                state.keyboard_modifiers,
                state.mouse_position_on_grid,
                true,
            )
        } else {
            let current_click = Click::new(
                cursor_position,
                mouse::Button::Left,
                state.last_click,
            );
            let selection_type = match current_click.kind() {
                mouse::click::Kind::Single => SelectionType::Simple,
                mouse::click::Kind::Double => SelectionType::Semantic,
                mouse::click::Kind::Triple => SelectionType::Lines,
            };
            state.last_click = Some(current_click);
            Command::SelectStart(
                selection_type,
                (
                    cursor_position.x - layout_position.x,
                    cursor_position.y - layout_position.y,
                ),
            )
        };
        commands.push(cmd);
        state.is_dragged = true;
    }

    fn handle_cursor_moved(
        state: &mut TerminalViewState,
        terminal_content: &RenderableContent,
        position: &Point,
        layout_position: Point,
        commands: &mut Vec<Command>,
    ) {
        let cursor_x = position.x - layout_position.x;
        let cursor_y = position.y - layout_position.y;
        state.mouse_position_on_grid = Backend::selection_point(
            cursor_x,
            cursor_y,
            &terminal_content.terminal_size,
            terminal_content.grid.display_offset(),
        );

        // Handle command or selection update based on terminal mode and modifiers
        if state.is_dragged {
            let terminal_mode = terminal_content.terminal_mode;
            let cmd = if terminal_mode.intersects(TermMode::MOUSE_MOTION) {
                Command::MouseReport(
                    MouseButton::LeftMove,
                    state.keyboard_modifiers,
                    state.mouse_position_on_grid,
                    true,
                )
            } else {
                Command::SelectUpdate((cursor_x, cursor_y))
            };
            commands.push(cmd);
        }

        // Handle link hover if applicable
        if state.keyboard_modifiers == Modifiers::COMMAND {
            commands.push(Command::ProcessLink(
                LinkAction::Hover,
                state.mouse_position_on_grid,
            ));
        }
    }

    fn handle_button_released(
        state: &mut TerminalViewState,
        terminal_mode: &TermMode,
        bindings: &BindingsLayout, // Use the actual type of your bindings here
        commands: &mut Vec<Command>,
    ) {
        state.is_dragged = false;

        if terminal_mode.intersects(TermMode::MOUSE_MODE) {
            commands.push(Command::MouseReport(
                MouseButton::LeftButton,
                state.keyboard_modifiers,
                state.mouse_position_on_grid,
                false,
            ));
        }

        if bindings.get_action(
            InputKind::Mouse(iced_core::mouse::Button::Left),
            state.keyboard_modifiers,
            *terminal_mode,
        ) == BindingAction::LinkOpen
        {
            commands.push(Command::ProcessLink(
                LinkAction::Open,
                state.mouse_position_on_grid,
            ));
        }
    }

    fn handle_keyboard_event(
        &self,
        state: &mut TerminalViewState,
        clipboard: &mut dyn iced_graphics::core::Clipboard,
        event: &iced::keyboard::Event,
    ) -> Option<Command> {
        let mut binding_action = BindingAction::Ignore;
        let last_content = self.term.backend.renderable_content();
        match event {
            iced::keyboard::Event::ModifiersChanged(m) => {
                state.keyboard_modifiers = *m;
                let action = if state.keyboard_modifiers == Modifiers::COMMAND {
                    LinkAction::Hover
                } else {
                    LinkAction::Clear
                };
                return Some(Command::ProcessLink(
                    action,
                    state.mouse_position_on_grid,
                ));
            },
            iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            } => match &key {
                // Use the physical character key for bindings even when text is None (e.g., Ctrl/Cmd combos)
                Key::Character(k) => {
                    let lower = k.to_ascii_lowercase();
                    binding_action = self.term.bindings.get_action(
                        InputKind::Char(lower),
                        state.keyboard_modifiers,
                        last_content.terminal_mode,
                    );

                    // If no binding matched, only write printable text (when provided)
                    if binding_action == BindingAction::Ignore {
                        if let Some(c) = text {
                            return Some(Command::Write(c.as_bytes().to_vec()));
                        }
                    }
                },
                Key::Named(code) => {
                    binding_action = self.term.bindings.get_action(
                        InputKind::KeyCode(*code),
                        *modifiers,
                        last_content.terminal_mode,
                    );
                },
                _ => {},
            },
            _ => {},
        }

        match binding_action {
            BindingAction::Char(c) => {
                let mut buf = [0, 0, 0, 0];
                let str = c.encode_utf8(&mut buf);
                return Some(Command::Write(str.as_bytes().to_vec()));
            },
            BindingAction::Esc(seq) => {
                return Some(Command::Write(seq.as_bytes().to_vec()));
            },
            BindingAction::Paste => {
                if let Some(data) = clipboard.read(ClipboardKind::Standard) {
                    let input: Vec<u8> = data.bytes().collect();
                    return Some(Command::Write(input));
                }
            },
            BindingAction::Copy => {
                clipboard.write(
                    ClipboardKind::Standard,
                    self.term.backend.selectable_content(),
                );
            },
            _ => {},
        };

        None
    }
}

fn draw_cursor_stroke(
    frame: &mut Frame,
    shape: CursorShape,
    bounds: Rectangle,
    color: Color,
) {
    let Rectangle {
        x,
        y,
        width,
        height,
    } = bounds;
    let thickness = 1.5_f32.min(width).min(height);
    let mut fill = |x, y, width, height| {
        frame.fill_rectangle(Point::new(x, y), Size::new(width, height), color)
    };
    match shape {
        CursorShape::Beam => fill(x, y, thickness, height),
        CursorShape::Underline => {
            fill(x, y + height - thickness, width, thickness)
        },
        CursorShape::HollowBlock => {
            fill(x, y, width, thickness);
            fill(x, y + height - thickness, width, thickness);
            fill(x, y, thickness, height);
            fill(x + width - thickness, y, thickness, height);
        },
        CursorShape::Block | CursorShape::Hidden => {},
    }
}

// Patch: each cell is shaped as its own text run. Advanced shaping adds font
// fallback and complex-script support that printable ASCII never needs, and a
// lone character has no ligatures. Basic shaping keeps cache misses cheap.
fn cell_shaping(c: char) -> Shaping {
    if c.is_ascii_graphic() {
        Shaping::Basic
    } else {
        Shaping::Advanced
    }
}

fn grid_layout(layout: iced_core::Layout<'_>) -> iced_core::Layout<'_> {
    layout.children().next().unwrap_or(layout)
}

fn center_grid(span: f32, cell: f32, scale: f32) -> (f32, f32) {
    if cell <= 0.0 {
        return (span, 0.0);
    }
    let occupied = (span / cell).floor() * cell;
    let before = ((span - occupied) * scale / 2.0).floor() / scale;
    (occupied, before)
}

impl Widget<Event, Theme, iced::Renderer> for TerminalView<'_> {
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<TerminalViewState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(TerminalViewState::new())
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &iced_core::layout::Limits,
    ) -> iced_core::layout::Node {
        let size = limits.resolve(Length::Fill, Length::Fill, Size::ZERO);
        use iced::advanced::layout::Node;
        let Some(scale) =
            self.dynamic_padding_scale.filter(|scale| *scale > 0.0)
        else {
            return Node::new(size);
        };
        let cell = self.term.font.measure;
        let (width, x) = center_grid(size.width, cell.width, scale);
        let (height, y) = center_grid(size.height, cell.height, scale);
        let grid =
            Node::new(Size::new(width, height)).move_to(Point::new(x, y));
        Node::with_children(size, vec![grid])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: iced_core::Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn operation::Operation,
    ) {
        let state = tree.state.downcast_mut::<TerminalViewState>();
        let wid = self.term.widget_id();
        operation.focusable(Some(wid), layout.bounds(), state);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout,
        _cursor: Cursor,
        viewport: &Rectangle,
    ) {
        // Colored cell runs are trimmed to the rounded corners of the whole
        // widget, so read its bounds before narrowing to the grid.
        let clip = RoundedClip {
            bounds: layout.bounds(),
            radius: self.corner_radius,
        };
        let layout = grid_layout(layout);
        let state = tree.state.downcast_ref::<TerminalViewState>();
        let content = self.term.backend.renderable_content();
        let term_size = content.terminal_size;
        let cell_width = term_size.cell_width;
        let cell_height = term_size.cell_height;
        let font_size = self.term.font.size;
        let font_scale_factor = self.term.font.scale_factor;
        let layout_offset_x = layout.position().x;
        let layout_offset_y = layout.position().y;

        let geom = self.term.cache.draw(renderer, viewport.size(), |frame| {
            // Precompute constants used in the inner loop
            let display_offset = content.grid.display_offset() as f32;
            let cell_size = Size::new(cell_width, cell_height);
            let half_w = cell_width * 0.5;
            let half_h = cell_height * 0.5;
            // We use the background pallete color as a default
            // because the widget global background color must be the same
            let default_bg = self
                .term
                .theme
                .get_color(ansi::Color::Named(NamedColor::Background));

            let mut last_line: Option<i32> = None;
            let mut bg_batch_rect = BackgroundRect::default();
            let mut cursor_stroke = None;
            let mut wide_cursor_spacer = None;
            let mut blocks = Vec::new();

            for indexed in content.grid.display_iter() {
                // Compute per-cell geometry cheaply
                let line = indexed.point.line.0;
                let col = indexed.point.column.0 as f32;

                // Resolve position point for this cell
                let x = layout_offset_x + (col * cell_width);
                let y = layout_offset_y
                    + (((line as f32) + display_offset) * cell_height);
                let cell_center_y = y + half_h;
                let cell_center_x = x + half_w;

                // Resolve colors for this cell
                let mut fg = self.term.theme.get_color(indexed.fg);
                let mut bg = self.term.theme.get_color(indexed.bg);

                // If the new line was detected,
                // need to flush pending background rect and init the new one
                if last_line != Some(line) {
                    if bg_batch_rect.can_flush() {
                        let line = last_line.unwrap_or(line);
                        frame.fill(
                            &clip.path(bg_batch_rect.build(line)),
                            bg_batch_rect.color,
                        );
                    }

                    last_line = Some(line);
                    bg_batch_rect = BackgroundRect::default()
                        .with_cell_height(cell_height)
                        .with_display_offset(display_offset)
                        .with_layout_offset_y(layout_offset_y);
                }

                // Handle dim, inverse, and selected text
                if indexed.cell.flags.contains(cell::Flags::DIM) {
                    fg.a *= 0.7;
                }
                if indexed.cell.flags.contains(cell::Flags::INVERSE)
                    || content
                        .selectable_range
                        .is_some_and(|r| r.contains(indexed.point))
                {
                    std::mem::swap(&mut fg, &mut bg);
                }

                // A block cursor is part of the cell's colors. Painting it separately
                // would let a later background batch cover it on highlighted lines.
                if let Some(color) = wide_cursor_spacer.take() {
                    if indexed
                        .cell
                        .flags
                        .contains(cell::Flags::WIDE_CHAR_SPACER)
                    {
                        bg = color;
                    }
                }
                let shape = content.cursor.shape;
                if indexed.point == content.cursor.point
                    && shape != CursorShape::Hidden
                {
                    let (cursor, text) = self.term.theme.cursor_colors(
                        fg,
                        bg,
                        content.cursor_override,
                        content.terminal_mode.contains(TermMode::VI),
                    );
                    let wide =
                        indexed.cell.flags.contains(cell::Flags::WIDE_CHAR);
                    if shape == CursorShape::Block {
                        bg = cursor;
                        fg = text;
                        wide_cursor_spacer = wide.then_some(cursor);
                    } else {
                        cursor_stroke = Some((
                            shape,
                            Rectangle::new(
                                Point::new(x, y),
                                Size::new(
                                    cell_width * if wide { 2.0 } else { 1.0 },
                                    cell_height,
                                ),
                            ),
                            cursor,
                        ));
                    }
                }

                // Batch draw backgrounds: skip default background (container already paints it)
                if bg != default_bg {
                    if bg_batch_rect.can_extend(bg, x) {
                        // Same color and contiguous: extend current run
                        bg_batch_rect.extend(cell_width);
                    } else {
                        // New colored run (or non-contiguous): flush previous run if any
                        if bg_batch_rect.can_flush() {
                            frame.fill(
                                &clip.path(bg_batch_rect.build(line)),
                                bg_batch_rect.color,
                            );
                        }

                        // Start a new run but do not draw yet; wait for potential extensions
                        bg_batch_rect = BackgroundRect::default()
                            .with_cell_height(cell_height)
                            .with_display_offset(display_offset)
                            .with_layout_offset_y(layout_offset_y)
                            .activate()
                            .with_color(bg)
                            .with_start_x(x)
                            .with_width(cell_width);
                    }
                } else if bg_batch_rect.can_flush() {
                    // Background returns to default, flush current background rect and init the new one
                    frame.fill(
                        &clip.path(bg_batch_rect.build(line)),
                        bg_batch_rect.color,
                    );

                    bg_batch_rect = BackgroundRect::default()
                        .with_cell_height(cell_height)
                        .with_display_offset(display_offset)
                        .with_layout_offset_y(layout_offset_y);
                }

                // Draw hovered hyperlink underline (rare; keep per-cell for correctness)
                if content.hovered_hyperlink.as_ref().is_some_and(|range| {
                    range.contains(&indexed.point)
                        && range.contains(&state.mouse_position_on_grid)
                }) || indexed.cell.flags.contains(cell::Flags::UNDERLINE)
                {
                    let underline_height = y + cell_size.height;
                    let underline = Path::line(
                        Point::new(x, underline_height),
                        Point::new(x + cell_size.width, underline_height),
                    );
                    frame.stroke(
                        &underline,
                        Stroke::default()
                            .with_width(font_size * 0.15)
                            .with_color(fg),
                    );
                }

                // Draw text. APP_CURSOR describes arrow-key encoding, not the cursor shape.
                if indexed.c != ' '
                    && indexed.c != '\t'
                    && !indexed.cell.flags.contains(cell::Flags::HIDDEN)
                {
                    if let Some(block) = BlockGlyph::from_char(indexed.c) {
                        blocks.push((
                            block,
                            Rectangle::new(Point::new(x, y), cell_size),
                            fg,
                        ));
                        continue;
                    }
                    // Resolve font style (bold/italic) from cell flags
                    let mut font = self.term.font.font_type;
                    if indexed.cell.flags.contains(cell::Flags::BOLD) {
                        font.weight = FontWeight::Bold;
                    }
                    if indexed.cell.flags.contains(cell::Flags::ITALIC) {
                        font.style = FontStyle::Italic;
                    }
                    let text = Text {
                        content: indexed.cell.c.to_string(),
                        position: Point::new(cell_center_x, cell_center_y),
                        font,
                        size: iced_core::Pixels(font_size),
                        color: fg,
                        align_x: Alignment::Center,
                        align_y: Vertical::Center,
                        shaping: cell_shaping(indexed.c),
                        line_height: LineHeight::Relative(font_scale_factor),
                        ..Default::default()
                    };
                    frame.fill_text(text);
                }
            }

            // Flush any remaining background run at the end
            if bg_batch_rect.can_flush() {
                frame.fill(
                    &clip.path(bg_batch_rect.build(last_line.unwrap_or(0))),
                    bg_batch_rect.color,
                );
            }
            // Like font glyphs, these foreground shapes must sit above all backgrounds.
            for (block, cell, color) in blocks {
                block.draw(cell, |point, size| {
                    frame.fill(&clip.path(Rectangle::new(point, size)), color)
                });
            }
            if let Some((shape, bounds, color)) = cursor_stroke {
                draw_cursor_stroke(frame, shape, bounds, color);
            }
        });

        use iced::advanced::graphics::geometry::Renderer as _;
        renderer.draw_geometry(geom);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced_core::Event,
        layout: iced_graphics::core::Layout<'_>,
        cursor: Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn iced_graphics::core::Clipboard,
        shell: &mut iced_graphics::core::Shell<'_, Event>,
        _viewport: &Rectangle,
    ) {
        let layout = grid_layout(layout);
        let state = tree.state.downcast_mut::<TerminalViewState>();
        self.handle_resize(state, layout, shell);

        let is_cursor_in_layout = self.is_cursor_in_layout(cursor, layout);
        self.handle_focus(event, state, is_cursor_in_layout);

        let commands = match event {
            iced::Event::Mouse(mouse_event) if is_cursor_in_layout => self
                .handle_mouse_event(
                    state,
                    layout.position(),
                    cursor.position().unwrap(),
                    mouse_event,
                ),
            iced::Event::Keyboard(keyboard_event) => {
                if !state.is_focused() {
                    return;
                }

                self.handle_keyboard_event(state, clipboard, keyboard_event)
                    .into_iter()
                    .collect()
            },
            _ => Vec::new(),
        };

        if !commands.is_empty() {
            shell.capture_event();
        }

        for cmd in commands {
            shell.publish(Event::BackendCall(self.term.id, cmd));
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: iced_core::Layout<'_>,
        cursor: iced_core::mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> iced_core::mouse::Interaction {
        let state = tree.state.downcast_ref::<TerminalViewState>();
        let mut cursor_mode = iced_core::mouse::Interaction::Idle;
        let terminal_mode =
            self.term.backend.renderable_content().terminal_mode;
        if self.is_cursor_in_layout(cursor, grid_layout(layout))
            && !terminal_mode.contains(TermMode::SGR_MOUSE)
        {
            cursor_mode = iced_core::mouse::Interaction::Text;
        }

        if self.is_cursor_hovered_hyperlink(state) {
            cursor_mode = iced_core::mouse::Interaction::Pointer;
        }

        cursor_mode
    }
}

impl<'a> From<TerminalView<'a>> for Element<'a, Event, Theme, iced::Renderer> {
    fn from(widget: TerminalView<'a>) -> Self {
        Self::new(widget)
    }
}

#[derive(Debug, Clone)]
struct TerminalViewState {
    focus: bool,
    is_dragged: bool,
    last_click: Option<mouse::Click>,
    scroll_remainder: f32,
    keyboard_modifiers: Modifiers,
    size: Size<f32>,
    mouse_position_on_grid: TerminalGridPoint,
}

impl TerminalViewState {
    fn new() -> Self {
        Self {
            focus: false,
            is_dragged: false,
            last_click: None,
            scroll_remainder: 0.0,
            keyboard_modifiers: Modifiers::empty(),
            size: Size::from([0.0, 0.0]),
            mouse_position_on_grid: TerminalGridPoint::default(),
        }
    }
}

impl TerminalViewState {
    fn wheel_lines(
        &mut self,
        delta: ScrollDelta,
        cell_height: f32,
        scale: f32,
    ) -> i32 {
        let lines = match delta {
            ScrollDelta::Lines { y, .. } => y,
            // iced receives physical pixel deltas from winit; grid geometry is logical.
            ScrollDelta::Pixels { y, .. } => {
                y / (cell_height.max(1.0) * scale.max(1.0))
            },
        };
        if !lines.is_finite() || lines == 0.0 {
            return 0;
        }
        if lines.signum() != self.scroll_remainder.signum() {
            self.scroll_remainder = 0.0;
        }
        let total = self.scroll_remainder + lines;
        self.scroll_remainder = total.fract();
        // Bound bursts of mouse reports if the OS coalesces a large gesture.
        total.clamp(-100.0, 100.0) as i32
    }
}

impl Default for TerminalViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl operation::Focusable for TerminalViewState {
    fn is_focused(&self) -> bool {
        self.focus
    }

    fn focus(&mut self) {
        self.focus = true;
    }

    fn unfocus(&mut self) {
        self.focus = false;
    }
}

#[derive(Default)]
struct BackgroundRect {
    display_offset: f32,
    cell_height: f32,
    layout_offset_y: f32,
    is_active: bool,
    color: Color,
    start_x: f32,
    width: f32,
}

impl BackgroundRect {
    fn with_display_offset(mut self, value: f32) -> Self {
        self.display_offset = value;
        self
    }

    fn with_cell_height(mut self, value: f32) -> Self {
        self.cell_height = value;
        self
    }

    fn with_layout_offset_y(mut self, value: f32) -> Self {
        self.layout_offset_y = value;
        self
    }

    fn with_width(mut self, value: f32) -> Self {
        self.width = value;
        self
    }

    fn with_start_x(mut self, value: f32) -> Self {
        self.start_x = value;
        self
    }

    fn with_color(mut self, value: Color) -> Self {
        self.color = value;
        self
    }

    fn activate(mut self) -> Self {
        self.is_active = true;
        self
    }

    fn build(&self, line: i32) -> Rectangle {
        let flush_y = self.layout_offset_y
            + ((line as f32 + self.display_offset) * self.cell_height);
        Rectangle::new(
            Point::new(self.start_x, flush_y),
            Size::new(self.width, self.cell_height),
        )
    }

    fn can_flush(&self) -> bool {
        self.is_active && self.width > 0.0
    }

    fn can_extend(&self, bg: Color, x: f32) -> bool {
        self.is_active
            && bg == self.color
            && (self.start_x + self.width - x).abs() < f32::EPSILON
    }

    fn extend(&mut self, value: f32) {
        self.width += value;
    }
}

/// Patch: trims colored cell runs (a tmux status line, a selection, block
/// glyphs) to the widget's rounded corners, so they don't paint a square
/// corner over the rounded background. Glyphs are not trimmed; the area
/// outside a corner's curve is too small to hold much of one.
#[derive(Clone, Copy, Debug)]
struct RoundedClip {
    bounds: Rectangle,
    radius: Radius,
}

impl RoundedClip {
    /// Vertical distance between the points sampled along a corner's curve.
    /// Half a point is one pixel on a Retina display.
    const STEP: f32 = 0.5;

    fn path(&self, rect: Rectangle) -> Path {
        match self.outline(rect) {
            None => Path::rectangle(rect.position(), rect.size()),
            Some(points) => Path::new(|builder| {
                let mut points = points.into_iter();
                if let Some(first) = points.next() {
                    builder.move_to(first);
                }
                for point in points {
                    builder.line_to(point);
                }
                builder.close();
            }),
        }
    }

    /// The outline of `rect` trimmed to the rounded corners, as a polygon
    /// (down the right side, then up the left), or `None` when `rect` stays
    /// clear of every rounded corner and can be filled as is.
    fn outline(&self, rect: Rectangle) -> Option<Vec<Point>> {
        if !self.touches_a_corner(rect) {
            return None;
        }
        let steps = ((rect.height / Self::STEP).ceil() as usize).max(1);
        let ys: Vec<f32> = (0..=steps)
            .map(|i| rect.y + rect.height * i as f32 / steps as f32)
            .collect();
        let spans: Vec<(f32, f32)> =
            ys.iter().map(|&y| self.span(rect, y)).collect();
        let right = ys.iter().zip(&spans).map(|(&y, &(_, r))| Point::new(r, y));
        let left = ys
            .iter()
            .zip(&spans)
            .rev()
            .map(|(&y, &(l, _))| Point::new(l, y));
        Some(right.chain(left).collect())
    }

    fn touches_a_corner(&self, rect: Rectangle) -> bool {
        let b = self.bounds;
        let Radius {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        } = self.radius;
        let (rect_right, rect_bottom) =
            (rect.x + rect.width, rect.y + rect.height);
        let (right, bottom) = (b.x + b.width, b.y + b.height);
        let near_top = |r: f32| r > 0.0 && rect.y < b.y + r;
        let near_bottom = |r: f32| r > 0.0 && rect_bottom > bottom - r;
        let near_left = |r: f32| rect.x < b.x + r;
        let near_right = |r: f32| rect_right > right - r;
        (near_top(top_left) && near_left(top_left))
            || (near_top(top_right) && near_right(top_right))
            || (near_bottom(bottom_left) && near_left(bottom_left))
            || (near_bottom(bottom_right) && near_right(bottom_right))
    }

    /// The left and right ends of `rect`'s row of pixels at height `y`.
    fn span(&self, rect: Rectangle, y: f32) -> (f32, f32) {
        let b = self.bounds;
        let from_top = y - b.y;
        let from_bottom = b.y + b.height - y;
        let r = self.radius;
        let left = b.x
            + corner_inset(r.top_left, from_top)
                .max(corner_inset(r.bottom_left, from_bottom));
        let right = b.x + b.width
            - corner_inset(r.top_right, from_top)
                .max(corner_inset(r.bottom_right, from_bottom));
        let start = rect.x.max(left);
        let end = (rect.x + rect.width).min(right).max(start);
        (start, end)
    }
}

/// How far a corner of `radius` pulls the side of the widget inward, at
/// `depth` from the widget's top or bottom edge.
fn corner_inset(radius: f32, depth: f32) -> f32 {
    if radius <= 0.0 || depth >= radius {
        return 0.0;
    }
    let rise = radius - depth.max(0.0);
    radius - (radius * radius - rise * rise).max(0.0).sqrt()
}

#[cfg(test)]
mod tests {
    #[test]
    fn centers_retina_grid_without_rounding_to_logical_pixels() {
        assert_eq!(super::center_grid(792.0, 7.5, 2.0), (787.5, 2.0));
        assert_eq!(super::center_grid(50.5, 16.5, 2.0), (49.5, 0.5));
    }

    #[test]
    fn shapes_ascii_cells_basically_and_falls_back_for_other_glyphs() {
        assert_eq!(super::cell_shaping('a'), Shaping::Basic);
        assert_eq!(super::cell_shaping('~'), Shaping::Basic);
        assert_eq!(super::cell_shaping('é'), Shaping::Advanced);
        assert_eq!(super::cell_shaping('\u{e0b0}'), Shaping::Advanced);
        assert_eq!(super::cell_shaping('漢'), Shaping::Advanced);
    }

    /// A 100×100 widget whose bottom-right corner has a 12pt radius.
    fn bottom_right_clip() -> super::RoundedClip {
        super::RoundedClip {
            bounds: Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0)),
            radius: super::Radius::default().bottom_right(12.0),
        }
    }

    #[test]
    fn cell_runs_away_from_rounded_corners_stay_rectangles() {
        let clip = bottom_right_clip();
        let middle =
            Rectangle::new(Point::new(0.0, 40.0), Size::new(100.0, 16.0));
        assert_eq!(clip.outline(middle), None);
        // The bottom row's left end sits in a square corner.
        let bottom_left =
            Rectangle::new(Point::new(0.0, 84.0), Size::new(40.0, 16.0));
        assert_eq!(clip.outline(bottom_left), None);
        // Without any radius nothing is ever trimmed.
        let square = super::RoundedClip {
            radius: super::Radius::default(),
            ..clip
        };
        let full = Rectangle::new(Point::ORIGIN, Size::new(100.0, 100.0));
        assert_eq!(square.outline(full), None);
    }

    #[test]
    fn cell_runs_in_a_rounded_corner_follow_its_curve() {
        let clip = bottom_right_clip();
        // A status line filling the bottom row to the widget's right edge.
        let status =
            Rectangle::new(Point::new(60.0, 84.0), Size::new(40.0, 16.0));
        let outline = clip.outline(status).unwrap_or_default();
        let right_end = |y: f32| {
            outline
                .iter()
                .filter(|p| p.x > 60.0 && (p.y - y).abs() < 0.01)
                .map(|p| p.x)
                .fold(f32::NAN, f32::max)
        };
        // Above the curve the run reaches the edge; at the bottom it stops
        // where the curve meets the bottom edge, 12pt in.
        assert_eq!(right_end(84.0), 100.0);
        assert_eq!(right_end(88.0), 100.0);
        assert!((right_end(100.0) - 88.0).abs() < 0.01);
        // Every point stays inside the corner's circle.
        let center = Point::new(88.0, 88.0);
        for p in outline.iter().filter(|p| p.x > 88.0 && p.y > 88.0) {
            assert!(p.distance(center) <= 12.0 + 0.01, "{p:?} is outside");
        }
        // The left end is untouched.
        assert!(outline.iter().filter(|p| p.x < 88.0).all(|p| p.x == 60.0));
    }

    use super::*;

    mod handle_left_button_pressed_tests {
        use super::*;
        use alacritty_terminal::index::{Column, Line};

        #[test]
        fn handles_mouse_mode_with_left_click() {
            let mut state = TerminalViewState::new();
            let terminal_mode = TermMode::MOUSE_MODE;
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_left_button_pressed(
                &mut state,
                &terminal_mode,
                cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftButton,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0),
                    },
                    true,
                )
            ));
            assert!(state.is_dragged);
        }

        #[test]
        fn starts_simple_selection_with_left_click() {
            let terminal_mode = TermMode::SGR_MOUSE;
            let cursor_position = Point { x: 200.0, y: 150.0 };
            let layout_position = Point { x: 50.0, y: 50.0 };

            let cases = vec![
                SelectionType::Simple,
                SelectionType::Semantic,
                SelectionType::Lines,
            ];

            for _selection_type in cases {
                let mut state = TerminalViewState::new();
                state.keyboard_modifiers = Modifiers::SHIFT;
                let mut commands = Vec::new();

                TerminalView::handle_left_button_pressed(
                    &mut state,
                    &terminal_mode,
                    cursor_position,
                    layout_position,
                    &mut commands,
                );

                assert_eq!(commands.len(), 1);
                assert!(matches!(
                    commands[0],
                    Command::SelectStart(_selection_type, (150.0, 100.0))
                ),);
                assert!(state.is_dragged);
            }
        }
    }

    mod handle_cursor_moved_tests {
        use alacritty_terminal::index::{Column, Line};

        use super::*;

        #[test]
        fn updates_mouse_position_on_grid() {
            let mut state = TerminalViewState::new();
            let terminal_content = RenderableContent::default();
            let mut commands = Vec::new();
            let cases = vec![
                (
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 1.0, y: 1.0 },
                    TerminalGridPoint {
                        line: Line(1),
                        column: Column(1),
                    },
                ),
                (
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 2.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(2),
                        column: Column(2),
                    },
                ),
                (
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 30.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(2),
                        column: Column(30),
                    },
                ),
                (
                    Point { x: 10.0, y: 0.0 },
                    Point { x: 30.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(2),
                        column: Column(20),
                    },
                ),
                (
                    Point { x: 10.0, y: 10.0 },
                    Point { x: 30.0, y: 2.0 },
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(20),
                    },
                ),
            ];

            for (layout_position, cursor_position, expected) in cases {
                TerminalView::handle_cursor_moved(
                    &mut state,
                    &terminal_content,
                    &cursor_position,
                    layout_position,
                    &mut commands,
                );

                assert_eq!(state.mouse_position_on_grid, expected);
            }
        }

        #[test]
        fn generates_drag_update_command_when_dragged() {
            let mut state = TerminalViewState::new();
            state.is_dragged = true; // Simulate an ongoing drag operation
            let terminal_content = RenderableContent::default();
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::SelectUpdate((95.0, 145.0))
            ));
        }

        #[test]
        fn generates_drag_update_command_when_dragged_in_mouse_motion_mode() {
            let mut state = TerminalViewState::new();
            state.is_dragged = true; // Simulate an ongoing drag operation
            let terminal_content = RenderableContent {
                terminal_mode: TermMode::MOUSE_MOTION,
                ..Default::default()
            };
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftMove,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(49),
                        column: Column(79),
                    },
                    true,
                )
            ));
        }

        #[test]
        fn generates_drag_update_command_when_dragged_in_srg_mode_with_key_mods(
        ) {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::SHIFT;
            state.is_dragged = true; // Simulate an ongoing drag operation
            let terminal_content = RenderableContent {
                terminal_mode: TermMode::SGR_MOUSE,
                ..Default::default()
            };
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::SelectUpdate((95.0, 145.0))
            ));
        }

        #[test]
        fn generates_drag_update_and_link_open() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            state.is_dragged = true; // Simulate an ongoing drag operation
            let terminal_content = RenderableContent {
                terminal_mode: TermMode::SGR_MOUSE,
                ..Default::default()
            };
            let layout_position = Point { x: 5.0, y: 5.0 };
            let cursor_position = Point { x: 100.0, y: 150.0 };
            let mut commands = Vec::new();

            TerminalView::handle_cursor_moved(
                &mut state,
                &terminal_content,
                &cursor_position,
                layout_position,
                &mut commands,
            );

            assert_eq!(commands.len(), 2);
            assert!(matches!(
                commands[0],
                Command::SelectUpdate((95.0, 145.0))
            ));
            assert!(matches!(
                commands[1],
                Command::ProcessLink(
                    LinkAction::Hover,
                    TerminalGridPoint {
                        line: Line(49),
                        column: Column(79),
                    },
                )
            ));
        }
    }

    mod handle_button_released_tests {
        use super::*;
        use alacritty_terminal::index::{Column, Line};

        #[test]
        fn mouse_mode_activated() {
            let mut state = TerminalViewState::new();
            let terminal_mode = TermMode::MOUSE_MODE;
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_button_released(
                &mut state,
                &terminal_mode,
                &bindings,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftButton,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0)
                    },
                    false
                )
            ));
        }

        #[test]
        fn link_open_on_button_release() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            let terminal_mode = TermMode::MOUSE_MODE;
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();
            let _modifiers = Modifiers::empty();

            TerminalView::handle_button_released(
                &mut state,
                &terminal_mode,
                &bindings,
                &mut commands,
            );

            assert_eq!(commands.len(), 2);
            assert!(matches!(
                commands[0],
                Command::MouseReport(
                    MouseButton::LeftButton,
                    _modifiers,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0)
                    },
                    false
                )
            ));
            assert!(matches!(
                commands[1],
                Command::ProcessLink(
                    LinkAction::Open,
                    TerminalGridPoint {
                        line: Line(0),
                        column: Column(0)
                    }
                ),
            ));
        }

        #[test]
        fn link_open_on_button_release_in_non_mouse_mode() {
            let mut state = TerminalViewState::new();
            state.keyboard_modifiers = Modifiers::COMMAND;
            state.mouse_position_on_grid = TerminalGridPoint {
                line: Line(4),
                column: Column(10),
            };
            let terminal_mode = TermMode::empty(); // Assume SGR_MOUSE mode doesn't affect link opening
            let bindings = BindingsLayout::new();
            let mut commands = Vec::new();

            TerminalView::handle_button_released(
                &mut state,
                &terminal_mode,
                &bindings,
                &mut commands,
            );

            assert_eq!(commands.len(), 1);
            assert!(matches!(
                commands[0],
                Command::ProcessLink(
                    LinkAction::Open,
                    TerminalGridPoint {
                        line: Line(4),
                        column: Column(10)
                    }
                ),
            ));
        }
    }

    #[test]
    fn trackpad_and_mouse_wheel_agree_on_scroll_direction() {
        let mut state = TerminalViewState::new();
        for sign in [-1.0, 1.0] {
            assert_eq!(
                state.wheel_lines(
                    ScrollDelta::Lines {
                        x: 0.0,
                        y: 2.0 * sign
                    },
                    16.5,
                    2.0
                ),
                (2.0 * sign) as i32
            );
            assert_eq!(
                state.wheel_lines(
                    ScrollDelta::Pixels {
                        x: 0.0,
                        y: 66.0 * sign
                    },
                    16.5,
                    2.0
                ),
                (2.0 * sign) as i32
            );
        }
    }

    #[test]
    fn retina_trackpad_deltas_accumulate_and_reverse_without_jumping() {
        let mut state = TerminalViewState::new();
        let pixels = |y| ScrollDelta::Pixels { x: 100.0, y };
        assert_eq!(state.wheel_lines(pixels(15.0), 16.5, 2.0), 0);
        assert_eq!(state.wheel_lines(pixels(18.0), 16.5, 2.0), 1);
        assert_eq!(state.wheel_lines(pixels(16.5), 16.5, 2.0), 0);
        assert_eq!(state.wheel_lines(pixels(0.0), 16.5, 2.0), 0);
        assert_eq!(state.wheel_lines(pixels(-33.0), 16.5, 2.0), -1);
    }
}
