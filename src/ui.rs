//! The ratatui frame: one draw path for the interactive loop, the plain-text
//! frame dump (`--dump-frame`), and the `TestBackend` tests.
//!
//! The map is a fixed grid of single-glyph cells, written straight into the
//! buffer; the status line and the hint/message line are ratatui Paragraphs
//! below it. Every renderer shares this one path, so the interactive display
//! and the golden plain-text frames are character-identical — the "messed up
//! terminal" class of bugs is owned by ratatui's buffer diffing, resize
//! handling, and alternate-screen management instead of hand-rolled raw
//! rendering.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::{Frame, Terminal};

use crate::game::Game;

/// The two UI lines (the status line and the hint/message line) reserved
/// below the map. [`crate::map::resolve_map_size`] fits the map to
/// `rows - UI_LINES`, so map rows plus these two lines never exceed the
/// window.
pub const UI_LINES: usize = 2;

/// The hint shown under the status line before any round produces a message.
fn default_hint(game: &Game) -> String {
    format!("seed {} - h/j/k/l or arrows move, q quits", game.seed)
}

/// Draw the game's frame: the map as a fixed grid of glyphs, then the status
/// line and the hint/message line. The map was sized to fit the window, so
/// map rows + [`UI_LINES`] never exceed it; the draw also clamps to the
/// frame area, so a terminal that shrinks between a resize event and the
/// next draw can never push a cell outside the buffer (a ratatui buffer
/// panics on out-of-bounds writes).
pub fn draw(frame: &mut Frame, game: &Game) {
    let area = frame.area();
    let cols = area.width as usize;
    let rows = area.height as usize;
    // The map, then the two UI lines. A transient shrink (a resize event
    // not yet processed) can leave the game's map larger than the window:
    // draw what fits rather than overflow.
    let map_h = game.map.height.min(rows.saturating_sub(UI_LINES));
    let map_w = game.map.width.min(cols);

    // The map: one row of glyphs per buffer row, truncated to the window.
    let buf = frame.buffer_mut();
    for y in 0..map_h {
        let row: String = (0..map_w).map(|x| game.glyph_at(x, y)).collect();
        buf.set_stringn(0, y as u16, row, map_w, Style::default());
    }

    // The status line, then the hint/message line. Paragraphs truncate long
    // lines to the window width instead of wrapping, exactly like the old
    // hand-fitted lines.
    if map_h + 1 < rows {
        frame.render_widget(
            Paragraph::new(Line::from(game.status_line())),
            Rect::new(0, map_h as u16, area.width, 1),
        );
    }
    if map_h + 2 <= rows {
        let hint = match &game.last_message {
            Some(message) => message.clone(),
            None => default_hint(game),
        };
        frame.render_widget(
            Paragraph::new(Line::from(hint)),
            Rect::new(0, (map_h + 1) as u16, area.width, 1),
        );
    }
}

/// Render the game's frame into an in-memory `cols` x `rows` buffer and
/// flatten it to plain text, one line per buffer row. The result is exactly
/// `rows` lines of `cols` characters — the same content the interactive
/// terminal shows — which is what [`crate::game::Game::render`] and
/// `--dump-frame` print.
pub fn render_text(game: &Game, cols: usize, rows: usize) -> String {
    let backend = TestBackend::new(cols as u16, rows as u16);
    let mut terminal = Terminal::new(backend).expect("TestBackend never fails");
    terminal
        .draw(|frame| draw(frame, game))
        .expect("TestBackend draw never fails");
    buffer_text(terminal.backend().buffer())
}

/// One plain-text line per buffer row, in row-major order. Unwritten cells
/// read as spaces, so every line is exactly the buffer's width and the text
/// frame is character-identical to the terminal frame.
pub fn buffer_text(buffer: &Buffer) -> String {
    let mut out = String::with_capacity(buffer.area.width as usize * (buffer.area.height as usize + 1));
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
