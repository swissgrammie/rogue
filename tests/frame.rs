//! The display pinned through `ratatui::TestBackend`, with no terminal: the
//! rendered buffer is exactly the window at every size, the map occupies
//! exactly the available rows, the report-9 status line and the hint line
//! sit at the bottom, and nothing ever overflows or wraps. A TestBackend
//! panics on any out-of-bounds write, so a successful render is itself the
//! no-overflow proof; the assertions below pin the layout.

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Terminal;

use rogue::game::Game;
use rogue::map;
use rogue::ui;

/// Render `game` into an in-memory `cols` x `rows` buffer, exactly as the
/// interactive terminal would draw it (the same `ui::draw` path).
fn render(game: &Game, cols: u16, rows: u16) -> Buffer {
    let backend = TestBackend::new(cols, rows);
    let mut terminal = Terminal::new(backend).expect("TestBackend never fails");
    terminal
        .draw(|frame| ui::draw(frame, game))
        .expect("TestBackend draw never fails");
    terminal.backend().buffer().clone()
}

/// One buffer row as text: exactly `width` characters, unwritten cells as
/// spaces.
fn row(buffer: &Buffer, y: u16) -> String {
    (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect()
}

/// The whole buffer as text: `height` lines of `width` characters.
fn frame_text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| row(buffer, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The map rows of a game as the renderers overlay them (player, monsters,
/// stairs, Amulet, tiles) — what the frame's top rows must contain.
fn expected_map_rows(game: &Game) -> Vec<String> {
    (0..game.map.height)
        .map(|y| (0..game.map.width).map(|x| game.glyph_at(x, y)).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Size sweep
// ---------------------------------------------------------------------------

/// At every window size in a sweep the buffer is exactly the window: the map
/// fills the available rows, the report-9 status line and the hint line sit
/// at the bottom, and every drawn cell is inside the buffer.
#[test]
fn frame_fills_the_window_at_every_size() {
    for (cols, rows) in [(80, 24), (40, 15), (100, 28), (30, 10), (120, 14), (50, 30)] {
        let (w, h) = map::fit_bounds(cols, rows);
        assert_eq!(h + 2, rows, "{cols}x{rows}: the map leaves exactly two rows for the UI");
        let game = Game::new(7, w, h);
        let buffer = render(&game, cols as u16, rows as u16);

        // The frame is exactly the window: nothing overflows.
        assert_eq!(
            buffer.area,
            Rect::new(0, 0, cols as u16, rows as u16),
            "{cols}x{rows}"
        );

        // The map occupies exactly the available rows, glyph for glyph.
        let expected = expected_map_rows(&game);
        for (y, want) in expected.iter().enumerate() {
            assert_eq!(row(&buffer, y as u16), *want, "{cols}x{rows} map row {y}");
        }

        // The status line (report-9 format) is last-but-one, fitted to the
        // map width so it never wraps.
        let status = row(&buffer, h as u16);
        assert_eq!(status.chars().count(), cols, "{cols}x{rows}: status line never wraps");
        let fitted: String = game.status_line().chars().take(w).collect();
        assert_eq!(status.trim_end(), fitted.trim_end(), "{cols}x{rows} status line");
        assert!(status.starts_with("Level: 1 "), "{cols}x{rows}");

        // The hint line is last.
        let hint = row(&buffer, (h + 1) as u16);
        assert!(hint.contains("seed"), "{cols}x{rows} hint: {hint:?}");

        // The frame carries the level's features: exactly one player — in
        // the starting room — and unexplored tiles render as solid rock.
        let text = frame_text(&buffer);
        assert_eq!(text.matches('@').count(), 1, "{cols}x{rows}");
        assert!(
            text.chars().any(|c| c == '.'),
            "{cols}x{rows}: the starting room floor is lit"
        );
    }
}

// ---------------------------------------------------------------------------
// No-overflow invariants at tiny windows
// ---------------------------------------------------------------------------

/// At awkward window sizes the buffer never exceeds the window, the status
/// line is always visible, and every row is exactly the window width.
#[test]
fn tiny_windows_never_overflow_and_keep_the_status_line() {
    for (cols, rows) in [(80, 10), (40, 12), (30, 14), (100, 8)] {
        let (w, h) = map::fit_bounds(cols, rows);
        assert_eq!(h + 2, rows, "{cols}x{rows}: map + status + hint must fit");
        let game = Game::new(7, w, h);
        let buffer = render(&game, cols as u16, rows as u16);

        // The buffer is never larger than the window.
        assert_eq!(buffer.area.width as usize, cols, "{cols}x{rows}");
        assert_eq!(buffer.area.height as usize, rows, "{cols}x{rows}");

        // The map took exactly the available rows, leaving two for the UI.
        assert_eq!(game.map.height, h, "{cols}x{rows}");

        // Every row is exactly the window width: nothing wraps.
        for y in 0..rows {
            assert_eq!(row(&buffer, y as u16).chars().count(), cols, "{cols}x{rows} row {y}");
        }

        // The status line is always visible at the bottom.
        let status = row(&buffer, h as u16);
        assert!(status.starts_with("Level: 1 "), "{cols}x{rows}: {status:?}");
        assert_eq!(row(&buffer, (h + 1) as u16).chars().count(), cols, "{cols}x{rows} hint");
    }
}

/// A window even smaller than the game's map (a terminal that shrank between
/// a resize event and the next draw) renders inside the buffer instead of
/// overflowing: the map and UI lines clamp to what fits.
#[test]
fn a_map_larger_than_the_window_renders_without_overflow() {
    let game = Game::new(7, 80, 22); // wants an 80x24 window
    let buffer = render(&game, 40, 15); // but the window is 40x15

    assert_eq!(buffer.area, Rect::new(0, 0, 40, 15));
    // Rows 0..12 show the map's top 13 rows; the UI lines are clamped on top.
    assert_eq!(row(&buffer, 12).chars().count(), 40);
    assert_eq!(
        row(&buffer, 13).trim_end(),
        game.status_line().chars().take(40).collect::<String>().trim_end(),
        "status line survives the clamp"
    );
    assert!(row(&buffer, 13).starts_with("Level: 1 "));
    assert!(row(&buffer, 14).contains("seed"));
}

// ---------------------------------------------------------------------------
// Resize
// ---------------------------------------------------------------------------

/// A terminal resize regenerates the frame at the new size: the buffer is
/// the new window, the map refills the available rows, the status line sits
/// at the new bottom, and nothing from the old, larger frame lingers (the
/// old right-hand cells simply do not exist in the new buffer).
#[test]
fn resize_regenerates_the_frame_at_the_new_size() {
    let mut game = Game::new(7, 80, 22); // an 80x24 window
    let old = render(&game, 80, 24);
    assert_eq!(old.area, Rect::new(0, 0, 80, 24));
    assert_eq!(
        row(&old, 22).trim_end(),
        game.status_line(),
        "status line at the old bottom"
    );

    // The terminal shrinks to 40x15: the floor regenerates at the new size.
    game.resize(40, 13);
    let new = render(&game, 40, 15);

    assert_eq!(new.area, Rect::new(0, 0, 40, 15), "the buffer is the new window");
    assert_eq!(game.map.height, 13, "the map regenerated at the new height");
    assert_eq!(row(&new, 14).chars().count(), 40, "no stale tail row");

    // The map refilled rows 0..13 at the new width, glyph for glyph.
    let expected = expected_map_rows(&game);
    for (y, want) in expected.iter().enumerate() {
        assert_eq!(row(&new, y as u16), *want, "resized map row {y}");
    }

    // The status line is at the new bottom, report-9 and fully in-bounds.
    let status = row(&new, 13);
    assert_eq!(status.chars().count(), 40, "status line fits the new window");
    let fitted: String = game.status_line().chars().take(40).collect();
    assert_eq!(status.trim_end(), fitted.trim_end(), "status at the new bottom");
    assert!(status.starts_with("Level: 1 "));

    // The player survived the regeneration and is drawn exactly once.
    assert!(game.player.hp > 0, "resize must not reset the player");
    let text = frame_text(&new);
    assert_eq!(text.matches('@').count(), 1);
    assert!(
        text.chars().any(|c| c == '.'),
        "the starting room floor is lit after the resize"
    );
}

/// Resizing keeps the player's position when it still lands on walkable
/// floor, and the frame still draws the player exactly where it is.
#[test]
fn resize_keeps_the_player_and_render_matches_the_play_field() {
    let mut game = Game::new(7, 80, 22);
    game.player.hp = 5; // make state preservation observable
    let (px, py) = (game.player.x, game.player.y);
    game.resize(40, 13);

    assert_eq!(game.player.hp, 5, "resize must not reset the player");
    assert!(game.player.x < 40 && game.player.y < 13);

    let buffer = render(&game, 40, 15);
    let text = frame_text(&buffer);
    assert_eq!(text.matches('@').count(), 1);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[game.player.y].chars().nth(game.player.x), Some('@'));

    if (game.player.x, game.player.y) == (px, py) {
        assert!(px < 40 && py < 13, "kept position must be in-bounds");
    }
}

// ---------------------------------------------------------------------------
// Frame content: status, hint, message, and the plain-text contract
// ---------------------------------------------------------------------------

/// The status line and hint line are drawn at the bottom, and a round's
/// message replaces the default hint.
#[test]
fn the_message_line_replaces_the_hint_after_a_round() {
    let (w, h) = map::fit_bounds(40, 15);
    let mut game = Game::new(7, w, h);
    let message = "The kestrel has injured you";
    game.last_message = Some(message.to_string());
    let buffer = render(&game, 40, 15);

    assert_eq!(
        row(&buffer, h as u16).trim_end(),
        game.status_line().chars().take(w).collect::<String>().trim_end(),
        "status line at the bottom, fitted to the window"
    );
    assert_eq!(row(&buffer, (h + 1) as u16).trim_end(), message);
}

/// A long message is truncated to the window, never wrapped.
#[test]
fn a_long_message_is_truncated_never_wrapped() {
    let (w, h) = map::fit_bounds(30, 10);
    let mut game = Game::new(7, w, h);
    let message = "a very long message that cannot possibly fit in thirty columns";
    game.last_message = Some(message.to_string());
    let buffer = render(&game, 30, 10);

    let hint = row(&buffer, (h + 1) as u16);
    assert_eq!(hint.chars().count(), 30, "the hint line never wraps");
    assert_eq!(
        hint.trim_end(),
        message.chars().take(30).collect::<String>().trim_end()
    );
}

/// `Game::render` (the `--dump-frame` path) is character-identical to the
/// ratatui buffer at the same window: the plain-text frame is exactly what
/// the terminal draws.
#[test]
fn text_frame_matches_the_buffer_frame() {
    for (cols, rows) in [(80, 24), (60, 14), (30, 10)] {
        let (w, h) = map::fit_bounds(cols, rows);
        let game = Game::new(7, w, h);
        let buffer = render(&game, cols as u16, rows as u16);

        let rendered: Vec<String> = game.render().lines().map(str::to_string).collect();
        let buffered: Vec<String> = frame_text(&buffer).lines().map(str::to_string).collect();
        assert_eq!(rendered, buffered, "{cols}x{rows}");
        assert_eq!(rendered.len(), rows, "{cols}x{rows}");
        assert!(rendered.iter().all(|l| l.chars().count() == cols), "{cols}x{rows}");
    }
}

/// `ui::buffer_text` round-trips a buffer with no loss: one line per row,
/// exactly the buffer's width.
#[test]
fn buffer_text_is_one_line_per_row_at_the_buffer_width() {
    let (w, h) = map::fit_bounds(80, 24);
    let game = Game::new(7, w, h);
    let buffer = render(&game, 80, 24);
    let text = ui::buffer_text(&buffer);

    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 24);
    assert!(lines.iter().all(|l| l.chars().count() == 80));
}

// ---------------------------------------------------------------------------
// Field of view
// ---------------------------------------------------------------------------

/// Two rooms joined by a corridor through doors: room A interior
/// (1..6, 1..5), room B interior (16..21, 1..5), doors at (7, 5) and
/// (15, 5), corridor row 5 from (8, 5) to (14, 5).
fn two_rooms() -> rogue::map::Dungeon {
    let rows = [
        "#######################",
        "#......#########......#",
        "#......#########......#",
        "#......#########......#",
        "#......#########......#",
        "#......+~~~~~~~+......#",
        "#######################",
    ];
    let width = rows[0].len();
    let tiles: Vec<rogue::map::Tile> = rows
        .iter()
        .flat_map(|row| row.chars())
        .map(|c| match c {
            '#' => rogue::map::Tile::Wall,
            '.' => rogue::map::Tile::Floor,
            '+' => rogue::map::Tile::Door,
            '~' => rogue::map::Tile::Corridor,
            other => panic!("unknown tile {other:?}"),
        })
        .collect();
    rogue::map::Dungeon::from_tiles(width, rows.len(), &tiles)
}

/// A fresh game's frame shows only the starting room: every lit tile is
/// drawn exactly as `glyph_at` renders it, and every tile outside the view
/// is solid rock (the classic unexplored look).
#[test]
fn fresh_game_shows_only_the_starting_room_and_rock_elsewhere() {
    let game = Game::new(7, 80, 22);
    let buffer = render(&game, 80, 24);
    // On a fresh floor seen == visible: the starting room, nothing more.
    assert_eq!(
        game.visible.iter().filter(|&&v| v).count(),
        game.seen.iter().filter(|&&s| s).count(),
        "fresh floor: seen equals visible"
    );
    for y in 0..game.map.height {
        for x in 0..game.map.width {
            let idx = y * game.map.width + x;
            let glyph = buffer[(x as u16, y as u16)].symbol();
            if game.visible[idx] {
                assert_eq!(glyph, game.glyph_at(x, y).to_string(), "lit ({x},{y})");
            } else {
                assert_eq!(glyph, "#", "unexplored ({x},{y}) must render as rock");
            }
        }
    }
    let (px, py) = (game.player.x, game.player.y);
    assert_eq!(buffer[(px as u16, py as u16)].symbol(), "@", "the player is lit");
    assert!(game.visible[py * game.map.width + px], "the player's tile is visible");
}

/// Walking into a new room reveals it (its monsters render), and leaving it
/// dims it again: the tile stays remembered as bare floor, the monster is
/// no longer drawn.
#[test]
fn moving_to_a_new_room_reveals_it_and_leaving_dims_it() {
    let mut game = Game::from_map(two_rooms(), 1);
    // A snake in room B, far from the starting room.
    let base = rogue::entity::MonsterKind::Snake.stats();
    let letter = rogue::entity::MonsterKind::Snake.letter();
    game.monsters = vec![rogue::entity::Monster {
        kind: rogue::entity::MonsterKind::Snake,
        x: 18,
        y: 3,
        hp: 100,
        max_hp: 100,
        level: base.level,
        armor_class: base.armor_class,
        exp: base.exp,
        running: false,
    }];
    // Roomless fixture maps spawn the player at (1, 1), in room A.
    assert_eq!((game.player.x, game.player.y), (1, 1));
    assert_eq!(game.glyph_at(18, 3), '#', "room B starts unexplored rock");

    // Walk east to the door, south down room A, then east through the
    // corridor and the far door into room B.
    for _ in 0..5 {
        game.move_player(rogue::game::Direction::East);
    }
    for _ in 0..4 {
        game.move_player(rogue::game::Direction::South);
    }
    for _ in 0..10 {
        game.move_player(rogue::game::Direction::East);
    }
    assert_eq!((game.player.x, game.player.y), (16, 5), "inside room B");

    // Room B is revealed: the monster is drawn, room A is remembered.
    let buffer = render(&game, 80, 24);
    assert_eq!(buffer[(18, 3)].symbol(), letter.to_string(), "the monster renders in a lit room");
    assert_eq!(buffer[(3, 3)].symbol(), ".", "room A stays remembered after leaving");
    assert!(!game.visible[3 * game.map.width + 3], "room A is not lit from room B");

    // Step back out into the corridor: room B dims, the monster disappears.
    for _ in 0..2 {
        game.move_player(rogue::game::Direction::West);
    }
    assert_eq!((game.player.x, game.player.y), (14, 5), "out in the corridor");
    let buffer = render(&game, 80, 24);
    assert_eq!(buffer[(18, 3)].symbol(), ".", "the tile is remembered, the monster is not");
    assert_eq!(buffer[(3, 3)].symbol(), ".", "room A stays remembered");
    assert_eq!(buffer[(11, 2)].symbol(), "#", "unexplored rock stays rock");
}
