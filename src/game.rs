//! The game loop: raw-mode input, rendering, and one combat round per move.
//!
//! Each round follows Rogue's phase order (report §7.1): the player acts
//! first (move, or fight the monster in the way), then the AFTER daemons run
//! in order — `runners` (monsters wake/chase/attack) → `doctor` (healing) →
//! `stomach` (hunger). All combat math lives in `crate::combat`; this module
//! owns input mapping, rendering, the status line, and round state (the
//! `quiet` healing counter). The UI is deliberately raw crossterm for now;
//! ratatui lands later without changing the game rules here.

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{read, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io::{self, Write};

use crate::combat;
use crate::entity::{self, Monster, Player};
use crate::map::Dungeon;
use crate::rng::Rng;

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

/// The tile one step in `dir` from `(x, y)`, or `None` at the map edge.
pub fn next_position(x: usize, y: usize, dir: Direction) -> Option<(usize, usize)> {
    let next = match dir {
        Direction::North => (Some(x), y.checked_sub(1)),
        Direction::South => (Some(x), y.checked_add(1)),
        Direction::East => (x.checked_add(1), Some(y)),
        Direction::West => (x.checked_sub(1), Some(y)),
    };
    let (Some(nx), Some(ny)) = next else {
        return None;
    };
    Some((nx, ny))
}

/// May a step from `(x, y)` in `dir` leave the map or hit a wall? Floor,
/// door, and corridor tiles are passable.
pub fn can_move(map: &Dungeon, x: usize, y: usize, dir: Direction) -> bool {
    let Some((nx, ny)) = next_position(x, y, dir) else {
        return false;
    };
    if nx >= map.width || ny >= map.height {
        return false;
    }
    map.tile(nx, ny).is_walkable()
}

/// One play session over a single level.
pub struct Game {
    pub map: Dungeon,
    pub player: Player,
    pub monsters: Vec<Monster>,
    /// Current dungeon floor. Combat reads it for monster scaling; the level
    /// progression task owns stairs/floors and will evolve this field.
    pub floor: u32,
    /// RNG for spawning and all combat rolls. Seeded from `seed`, so a seed
    /// reproduces the level, its inhabitants, and every die roll.
    pub rng: Rng,
    /// Quiet rounds since the last combat: resets to 0 on any fight and
    /// gates healing (report §7.3).
    pub quiet: u32,
    pub seed: u64,
    /// The most recent event, shown under the status line.
    pub last_message: Option<String>,
}

impl Game {
    /// The player starts in the center of the first room the generator
    /// placed, on floor 1, with the level's monsters already spawned.
    pub fn new(map: Dungeon, seed: u64) -> Game {
        let mut rng = Rng::new(seed);
        let spawn = entity::populate(&map, 1, &mut rng);
        Game {
            map,
            player: spawn.player,
            monsters: spawn.monsters,
            floor: 1,
            rng,
            quiet: 0,
            seed,
            last_message: None,
        }
    }

    /// One round (report §7.1): the player acts, then the AFTER daemons run
    /// in order — runners → doctor → stomach. Bumping a wall spends no turn.
    pub fn step(&mut self, dir: Direction) {
        // 1. Player action. Moving into a monster starts a fight; the target
        //    is woken (runto) before the swing, so the player never benefits
        //    from the +4 "defender not running" bonus (report §4.4).
        let Some((nx, ny)) = next_position(self.player.x, self.player.y, dir) else {
            return;
        };
        let mut fought = false;
        if let Some(monster) = self.monsters.iter_mut().find(|m| m.x == nx && m.y == ny) {
            let result = combat::player_attack(
                &mut self.rng,
                &mut self.player,
                monster,
                &combat::STARTING_WEAPON,
            );
            self.last_message = Some(if monster.hp <= 0 {
                format!(
                    "You scored an excellent hit on the {} — you have defeated the {}",
                    monster.name(),
                    monster.name()
                )
            } else if result.hits > 0 {
                format!("You scored an excellent hit on the {}", monster.name())
            } else {
                format!("You swing and miss the {}", monster.name())
            });
            fought = true;
        } else if can_move(&self.map, self.player.x, self.player.y, dir) {
            self.player.x = nx;
            self.player.y = ny;
        } else {
            return; // thud: a blocked step consumes no turn
        }
        self.player.running = true;

        // 2. AFTER phase: runners → doctor → stomach.
        let report = combat::monster_phase(
            &mut self.rng,
            &self.map,
            &mut self.player,
            &mut self.monsters,
            combat::STARTING_ARMOR_AC,
        );
        if let Some(attack) = report.attacks.iter().find(|a| a.result.hits > 0) {
            self.last_message = Some(format!("The {} has injured you", attack.name));
        } else if let Some(attack) = report.attacks.first() {
            self.last_message = Some(format!("The {} doesn't hit you", attack.name));
        }

        if fought || report.any_attack {
            self.quiet = 0;
        } else {
            self.quiet += 1;
        }
        combat::doctor(&mut self.rng, &mut self.player, self.quiet);
        combat::hunger(&mut self.player);

        self.monsters.retain(|m| m.hp > 0);
        if self.player.hp <= 0 {
            self.last_message = Some("you have died".to_string());
        }
    }

    /// The glyph at `(x, y)`: the player, else a monster, else the tile.
    pub fn glyph_at(&self, x: usize, y: usize) -> char {
        if x == self.player.x && y == self.player.y {
            return '@';
        }
        self.monsters
            .iter()
            .find(|m| m.x == x && m.y == y)
            .map(|m| m.symbol())
            .unwrap_or_else(|| self.map.tile(x, y).glyph())
    }

    /// The status line, formatted per report §9
    /// (`Level: 1  Gold: 0  Hp: 12(12)  Str: 16(16)  Arm: 4   Exp: 1/0`).
    /// "Arm" is `10 - effective AC`, so a fresh player in ring mail shows 4
    /// while combat resolves against 6.
    pub fn status_line(&self) -> String {
        let p = &self.player;
        format!(
            "Level: {}  Gold: {}  Hp: {}({})  Str: {}({})  Arm: {}   Exp: {}/{}",
            p.level,
            p.gold,
            p.hp,
            p.max_hp,
            p.strength,
            p.strength,
            10 - combat::STARTING_ARMOR_AC,
            p.level,
            p.experience,
        )
    }

    /// The level with `@` and the monsters on top of it, then the status
    /// line and a one-line hint or last message.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity((self.map.width + 1) * (self.map.height + 2));
        for y in 0..self.map.height {
            for x in 0..self.map.width {
                out.push(self.glyph_at(x, y));
            }
            out.push('\n');
        }
        out.push_str(&self.status_line());
        out.push('\n');
        match &self.last_message {
            Some(message) => out.push_str(message),
            None => out.push_str(&format!(
                "seed {} — h/j/k/l or arrows move, q quits",
                self.seed
            )),
        }
        out.push('\n');
        out
    }

    /// Repaint the whole frame.
    fn draw(&self, out: &mut impl Write) -> io::Result<()> {
        execute!(out, MoveTo(0, 0), Clear(ClearType::All))?;
        write!(out, "{}", self.render())?;
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

/// Play one level: draw it, then loop on keys until the player quits or dies.
pub fn run(map: Dungeon, seed: u64) -> io::Result<()> {
    let mut game = Game::new(map, seed);
    let _guard = TerminalGuard::enter()?;
    let mut out = io::stdout();

    loop {
        game.draw(&mut out)?;
        if game.player.hp <= 0 {
            return Ok(()); // the frame above shows "you have died"
        }
        match read_key()? {
            Key::Char('q') | Key::Escape => return Ok(()),
            key => {
                if let Some(dir) = key_to_direction(key) {
                    game.step(dir);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::MonsterKind;
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

    /// A monster standing at full health, table stats, asleep.
    fn monster(kind: MonsterKind, x: usize, y: usize) -> Monster {
        let base = kind.stats();
        Monster {
            kind,
            x,
            y,
            hp: 100,
            max_hp: 100,
            level: base.level,
            armor_class: base.armor_class,
            exp: base.exp,
            running: false,
        }
    }

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
        assert!(!can_move(&map, 1, 0, Direction::West));
        assert!(!can_move(&map, 1, 0, Direction::East));
    }

    #[test]
    fn floor_door_and_corridor_are_passable() {
        let map = map_from(&TEST_MAP);
        // Floor.
        assert!(can_move(&map, 1, 0, Direction::South));
        // Door, approached from either side.
        assert!(can_move(&map, 3, 0, Direction::East));
        assert!(can_move(&map, 5, 0, Direction::West));
        // Corridor.
        assert!(can_move(&map, 5, 0, Direction::East));
    }

    #[test]
    fn map_bounds_block_movement() {
        let map = map_from(&TEST_MAP);
        // (0,1) and (7,3) are floor tiles: only the bounds check can stop these.
        assert!(!can_move(&map, 0, 1, Direction::West));
        assert!(!can_move(&map, 7, 3, Direction::East));
        assert!(!can_move(&map, 7, 3, Direction::South));
    }

    #[test]
    fn player_starts_in_first_room_center() {
        let map = Dungeon::generate(7);
        let (x, y) = map.rooms()[0].center();
        let game = Game::new(map, 7);
        assert_eq!((game.player.x, game.player.y), (x, y));
        assert!(game.map.tile(x, y).is_walkable());
    }

    #[test]
    fn move_player_walks_the_level_and_stops_at_walls() {
        // map_from maps have no rooms: the player lands in the top-left.
        let mut game = Game::new(map_from(&TEST_MAP), 0);
        assert_eq!((game.player.x, game.player.y), (1, 1));

        // E E N E E E: floor -> floor -> floor -> door -> floor -> corridor.
        for dir in [
            Direction::East,
            Direction::East,
            Direction::North,
            Direction::East,
            Direction::East,
            Direction::East,
        ] {
            game.step(dir);
        }
        assert_eq!((game.player.x, game.player.y), (6, 0));
        assert_eq!(game.map.tile(6, 0), Tile::Corridor);

        // (7,0) is a wall: the step is refused and the player stays put.
        game.step(Direction::East);
        assert_eq!((game.player.x, game.player.y), (6, 0));
    }

    #[test]
    fn render_marks_the_player_once_and_shows_the_status_line() {
        let mut game = Game::new(map_from(&TEST_MAP), 0);
        game.step(Direction::East);
        let frame = game.render();
        assert_eq!(frame.matches('@').count(), 1);
        // Map lines, then the §9 status line, then a hint/message line.
        let lines: Vec<&str> = frame.lines().collect();
        assert_eq!(lines.len(), TEST_MAP.len() + 2);
        // '~' renders as '.', and the player '@' now sits on row 1, column 2.
        assert_eq!(lines[0], "#.#.+..#");
        assert_eq!(lines[1], "..@.#..#");
        assert_eq!(lines[2], "#....+.#");
        assert_eq!(lines[3], "#.####..");
        assert_eq!(
            lines[4],
            "Level: 1  Gold: 0  Hp: 12(12)  Str: 16(16)  Arm: 4   Exp: 1/0"
        );
    }

    /// Round-order smoke: the player acts before monsters. A guaranteed-kill
    /// player moving into a running kestrel kills it during their action, so
    /// it never gets a counterattack that round.
    #[test]
    fn round_order_player_kills_before_monsters_act() {
        let mut game = Game::new(map_from(&TEST_MAP), 1);
        game.player = Player::new(1, 1);
        game.player.experience = 8_000_000; // level 21: automatic hit
        game.player.level = 21;
        let mut kestrel = monster(MonsterKind::Kestrel, 2, 1);
        kestrel.hp = 1;
        kestrel.max_hp = 1;
        kestrel.running = true;
        game.monsters = vec![kestrel];

        game.step(Direction::East);

        assert_eq!(game.player.x, 1, "fighting happens in place");
        assert!(game.monsters.is_empty(), "the kestrel died during the player's action");
        assert_eq!(game.player.hp, 12, "no monster counterattack in the same round");
        assert!(game.player.experience > 8_000_000, "exp granted on the kill");
    }

    /// Round-order smoke, other side: a player that can never hit and a
    /// monster that always hits. The monster's counterattack lands in the
    /// AFTER phase, still within the same round.
    #[test]
    fn round_order_monsters_strike_back_after_the_player() {
        let mut game = Game::new(map_from(&TEST_MAP), 2);
        game.player = Player::new(1, 1); // level 1, str 16
        let mut jabberwock = monster(MonsterKind::Jabberwock, 2, 1);
        jabberwock.level = 15;
        jabberwock.armor_class = -2; // the player can never roll this
        jabberwock.running = true;
        game.monsters = vec![jabberwock];

        game.step(Direction::East);

        assert_eq!(game.player.x, 1, "fighting in place, not moving");
        assert_eq!(game.monsters.len(), 1, "the jabberwock survived");
        assert_eq!(game.monsters[0].hp, 100, "the player never hit it");
        assert!(game.player.hp < 12, "the monster struck back in the AFTER phase");
        assert!(game.player.hp >= 0);
    }

    #[test]
    fn status_line_matches_report_section_9() {
        let game = Game::new(map_from(&TEST_MAP), 0);
        assert_eq!(
            game.status_line(),
            "Level: 1  Gold: 0  Hp: 12(12)  Str: 16(16)  Arm: 4   Exp: 1/0"
        );
    }
}
