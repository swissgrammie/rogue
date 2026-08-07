//! The game loop: raw-mode input, rendering, and player movement.
//!
//! One terminal frame per keypress: redraw the level with the player marker
//! (`@`) after every move. Input mapping and collision checks live in pure
//! functions so the rules stay unit-testable without a terminal. The UI is
//! deliberately raw crossterm for now; ratatui lands later without changing
//! the game rules here.

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{read, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::{self, Write};

use crate::map::Dungeon;

/// The four movement directions, as in classic Rogue.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    North,
    South,
    East,
    West,
}

/// The keys the game reacts to. Crossterm events are normalized into this
/// small set first, so `key_to_direction` stays pure and terminal-free.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Escape,
    Unknown,
}

/// The player: a position on the map for now. When the full entity `Player`
/// (stats, inventory, ...) lands in its own module, the loop only needs to
/// adopt `x`/`y` and `step` here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Player {
    pub x: usize,
    pub y: usize,
}

impl Player {
    pub fn new(x: usize, y: usize) -> Self {
        Player { x, y }
    }

    /// One tile in `dir`. Callers must have passed `can_move` first —
    /// stepping out of bounds would wrap around.
    pub fn step(self, dir: Direction) -> Player {
        match dir {
            Direction::North => Player::new(self.x, self.y - 1),
            Direction::South => Player::new(self.x, self.y + 1),
            Direction::East => Player::new(self.x + 1, self.y),
            Direction::West => Player::new(self.x - 1, self.y),
        }
    }
}

/// Map `key` to the direction it moves the player, if any.
pub fn key_to_direction(key: Key) -> Option<Direction> {
    match key {
        Key::Char('h') | Key::Left => Some(Direction::West),
        Key::Char('j') | Key::Down => Some(Direction::South),
        Key::Char('k') | Key::Up => Some(Direction::North),
        Key::Char('l') | Key::Right => Some(Direction::East),
        _ => None,
    }
}

/// May the player at `pos` take one step in `dir`? False when the step leaves
/// the map or lands on a wall; floor, door, and corridor tiles are passable.
pub fn can_move(map: &Dungeon, pos: Player, dir: Direction) -> bool {
    let next = match dir {
        Direction::North => (Some(pos.x), pos.y.checked_sub(1)),
        Direction::South => (Some(pos.x), pos.y.checked_add(1)),
        Direction::East => (pos.x.checked_add(1), Some(pos.y)),
        Direction::West => (pos.x.checked_sub(1), Some(pos.y)),
    };
    let (Some(x), Some(y)) = next else {
        return false;
    };
    if x >= map.width || y >= map.height {
        return false;
    }
    map.tile(x, y).is_walkable()
}

/// One play session over a single level.
pub struct Game {
    pub map: Dungeon,
    pub player: Player,
    pub seed: u64,
}

impl Game {
    /// The player starts in the center of the first room the generator placed.
    pub fn new(map: Dungeon, seed: u64) -> Game {
        let player = match map.rooms().first() {
            Some(room) => {
                let (x, y) = room.center();
                Player::new(x, y)
            }
            None => Player::new(1, 1),
        };
        Game { map, player, seed }
    }

    /// Step the player in `dir`; a blocked move changes nothing.
    pub fn move_player(&mut self, dir: Direction) {
        if can_move(&self.map, self.player, dir) {
            self.player = self.player.step(dir);
        }
    }

    /// The level with `@` over the player, one line per row.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity((self.map.width + 1) * self.map.height);
        for y in 0..self.map.height {
            for x in 0..self.map.width {
                if x == self.player.x && y == self.player.y {
                    out.push('@');
                } else {
                    out.push(self.map.tile(x, y).glyph());
                }
            }
            out.push('\n');
        }
        out
    }

    /// Repaint the frame and a one-line status bar.
    fn draw(&self, out: &mut impl Write) -> io::Result<()> {
        execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;
        write!(out, "{}", self.render())?;
        writeln!(out, "seed {} — h/j/k/l or arrows move, q quits", self.seed)?;
        out.flush()
    }
}

/// Enter raw mode and the alternate screen; restores everything on drop no
/// matter how the loop exits, so the terminal is never left broken.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<TerminalGuard> {
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(&mut out, EnterAlternateScreen, Hide)?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = execute!(&mut out, Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Normalize one crossterm key event into our `Key` set.
fn key_from_event(event: KeyEvent) -> Key {
    if matches!(
        event,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    ) {
        // Raw mode swallows SIGINT; treat Ctrl-C as quit too.
        return Key::Escape;
    }
    match event.code {
        KeyCode::Char(c) => Key::Char(c.to_ascii_lowercase()),
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Esc => Key::Escape,
        _ => Key::Unknown,
    }
}

/// Block for one keypress. Resize and mouse events just wait for the next
/// key; the following frame redraws the whole screen anyway.
fn read_key() -> io::Result<Key> {
    loop {
        match read()? {
            Event::Key(event) => return Ok(key_from_event(event)),
            _ => continue,
        }
    }
}

/// Play one level: draw it, then loop on keys until the player quits.
pub fn run(map: Dungeon, seed: u64) -> io::Result<()> {
    let mut game = Game::new(map, seed);
    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    loop {
        game.draw(&mut out)?;
        match read_key()? {
            Key::Char('q') | Key::Escape => return Ok(()),
            key => {
                if let Some(dir) = key_to_direction(key) {
                    game.move_player(dir);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Tile;

    /// Build a dungeon from ASCII rows: `#` wall, `.` floor, `+` door,
    /// `~` corridor.
    fn map_from(rows: &[&str]) -> Dungeon {
        let width = rows[0].len();
        let mut tiles = Vec::new();
        for row in rows {
            assert_eq!(row.len(), width, "test rows must be equal width");
            for c in row.chars() {
                tiles.push(match c {
                    '#' => Tile::Wall,
                    '.' => Tile::Floor,
                    '+' => Tile::Door,
                    '~' => Tile::Corridor,
                    other => panic!("unknown test tile {other:?}"),
                });
            }
        }
        Dungeon::from_tiles(width, rows.len(), &tiles)
    }

    /// A tiny hand-built level exercising every tile kind:
    ///   row0: # . # . + . ~ #
    ///   row1: . . . . # . . #
    ///   row2: # . . . . + . #
    ///   row3: # . # # # # . .
    const TEST_MAP: [&str; 4] = ["#.#.+.~#", "....#..#", "#....+.#", "#.####.."];

    #[test]
    fn hjkl_map_to_directions() {
        assert_eq!(key_to_direction(Key::Char('h')), Some(Direction::West));
        assert_eq!(key_to_direction(Key::Char('j')), Some(Direction::South));
        assert_eq!(key_to_direction(Key::Char('k')), Some(Direction::North));
        assert_eq!(key_to_direction(Key::Char('l')), Some(Direction::East));
    }

    #[test]
    fn arrow_keys_map_to_directions() {
        assert_eq!(key_to_direction(Key::Left), Some(Direction::West));
        assert_eq!(key_to_direction(Key::Down), Some(Direction::South));
        assert_eq!(key_to_direction(Key::Up), Some(Direction::North));
        assert_eq!(key_to_direction(Key::Right), Some(Direction::East));
    }

    #[test]
    fn non_movement_keys_map_to_nothing() {
        assert_eq!(key_to_direction(Key::Char('q')), None);
        assert_eq!(key_to_direction(Key::Char('x')), None);
        assert_eq!(key_to_direction(Key::Escape), None);
        assert_eq!(key_to_direction(Key::Unknown), None);
    }

    #[test]
    fn walls_block_movement() {
        let map = map_from(&TEST_MAP);
        // (0,0) and (2,0) are walls.
        assert!(!can_move(&map, Player::new(1, 0), Direction::West));
        assert!(!can_move(&map, Player::new(1, 0), Direction::East));
    }

    #[test]
    fn floor_door_and_corridor_are_passable() {
        let map = map_from(&TEST_MAP);
        // Floor.
        assert!(can_move(&map, Player::new(1, 0), Direction::South));
        // Door, approached from either side.
        assert!(can_move(&map, Player::new(3, 0), Direction::East));
        assert!(can_move(&map, Player::new(5, 0), Direction::West));
        // Corridor.
        assert!(can_move(&map, Player::new(5, 0), Direction::East));
    }

    #[test]
    fn map_bounds_block_movement() {
        let map = map_from(&TEST_MAP);
        // (0,1) and (7,3) are floor tiles: only the bounds check can stop these.
        assert!(!can_move(&map, Player::new(0, 1), Direction::West));
        assert!(!can_move(&map, Player::new(7, 3), Direction::East));
        assert!(!can_move(&map, Player::new(7, 3), Direction::South));
    }

    #[test]
    fn player_starts_in_first_room_center() {
        let map = Dungeon::generate(7);
        let (x, y) = map.rooms()[0].center();
        let game = Game::new(map, 7);
        assert_eq!(game.player, Player::new(x, y));
        assert!(game.map.tile(x, y).is_walkable());
    }

    #[test]
    fn move_player_walks_the_level_and_stops_at_walls() {
        let mut game = Game::new(map_from(&TEST_MAP), 0);
        assert_eq!(game.player, Player::new(1, 1)); // no rooms -> fallback spawn

        // E E N E E E: floor -> floor -> floor -> door -> floor -> corridor.
        for dir in [
            Direction::East,
            Direction::East,
            Direction::North,
            Direction::East,
            Direction::East,
            Direction::East,
        ] {
            game.move_player(dir);
        }
        assert_eq!(game.player, Player::new(6, 0));
        assert_eq!(game.map.tile(6, 0), Tile::Corridor);

        // (7,0) is a wall: the step is refused and the player stays put.
        game.move_player(Direction::East);
        assert_eq!(game.player, Player::new(6, 0));
    }

    #[test]
    fn render_marks_the_player_once() {
        let mut game = Game::new(map_from(&TEST_MAP), 0);
        game.move_player(Direction::East);
        let frame = game.render();
        assert_eq!(frame.matches('@').count(), 1);
        assert_eq!(frame.lines().count(), TEST_MAP.len());
        let lines: Vec<&str> = frame.lines().collect();
        assert_eq!(lines[1].chars().nth(2), Some('@'));
    }
}
