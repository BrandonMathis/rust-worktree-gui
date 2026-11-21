//! Cell-aligned glyphs used in terminal QR codes. These must not depend on
//! font coverage, nor leave the side bearings of a normal font glyph.

use iced::{Point, Rectangle, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BlockGlyph {
    columns: u8,
    rows: u8,
    mask: u8,
}

impl BlockGlyph {
    pub(crate) fn from_char(character: char) -> Option<Self> {
        let (columns, rows, mask) = match character {
            '▀' => (1, 2, 0b01),
            '▄' => (1, 2, 0b10),
            '█' => (1, 1, 0b1),
            '▌' => (2, 1, 0b01),
            '▐' => (2, 1, 0b10),
            '\u{1fb00}'..='\u{1fb3b}' => {
                // Unicode numbers sextants from top left to bottom right.
                // The blank, left half, right half, and full blocks already
                // exist elsewhere, so the contiguous range skips those masks.
                let mut mask = (character as u32 - 0x1fb00 + 1) as u8;
                if mask >= 0b010101 {
                    mask += 1;
                }
                if mask >= 0b101010 {
                    mask += 1;
                }
                (2, 3, mask)
            },
            _ => return None,
        };
        Some(Self {
            columns,
            rows,
            mask,
        })
    }

    pub(crate) fn draw(
        self,
        cell: Rectangle,
        mut fill: impl FnMut(Point, Size),
    ) {
        let x = |column| {
            cell.x + cell.width * f32::from(column) / f32::from(self.columns)
        };
        let y =
            |row| cell.y + cell.height * f32::from(row) / f32::from(self.rows);
        for row in 0..self.rows {
            let filled = |column| {
                self.mask & (1u8 << (row * self.columns + column)) != 0
            };
            let mut column = 0;
            while column < self.columns {
                let start = column;
                while column < self.columns && filled(column) {
                    column += 1;
                }
                if start < column {
                    fill(
                        Point::new(x(start), y(row)),
                        Size::new(x(column) - x(start), y(row + 1) - y(row)),
                    );
                } else {
                    column += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_full_sextant_range_in_order() {
        let masks: Vec<_> = ('\u{1fb00}'..='\u{1fb3b}')
            .map(|c| BlockGlyph::from_char(c).unwrap().mask)
            .collect();
        assert_eq!(
            masks,
            (1..63)
                .filter(|n| ![21, 42].contains(n))
                .collect::<Vec<_>>()
        );
        assert!(BlockGlyph::from_char('\u{1fb3c}').is_none());
        assert!(BlockGlyph::from_char('a').is_none());
    }

    #[test]
    fn fills_cell_halves_and_sextants_without_side_bearings() {
        let cell = Rectangle::new(Point::new(5.0, 2.0), Size::new(7.5, 16.5));
        let rectangles = |c| {
            let mut result = Vec::new();
            BlockGlyph::from_char(c).unwrap().draw(cell, |point, size| {
                result.push(Rectangle::new(point, size))
            });
            result
        };
        assert_eq!(rectangles('█'), [cell]);
        assert_eq!(
            rectangles('▄'),
            [Rectangle::new(Point::new(5.0, 10.25), Size::new(7.5, 8.25))]
        );
        assert_eq!(
            rectangles('▐'),
            [Rectangle::new(Point::new(8.75, 2.0), Size::new(3.75, 16.5))]
        );
        // U+1FB02 fills the two segments in the top row as a single rectangle.
        assert_eq!(
            rectangles('\u{1fb02}'),
            [Rectangle::new(Point::new(5.0, 2.0), Size::new(7.5, 5.5))]
        );
    }
}
