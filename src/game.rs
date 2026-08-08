//! The game loop: input, rendering, the combat round, and level progression.
//!
//! One terminal frame per keypress: redraw the level with the player marker
//! (`@`) after every move. Input mapping and collision checks live in pure
//! functions so the rules stay unit-testable without a terminal; rendering
//! itself is ratatui's job (`crate::ui`), so the frame, resize handling, and
//! the terminal state (raw mode, alternate screen) are robust by
//! construction while the game rules here stay untouched.
//!
//! Each round follows Rogue's phase order (report §7.1): the player acts
//! first (move, or fight the monster in the way), then the AFTER daemons run
//! in order — `runners` (monsters wake/chase/attack) → `doctor` (healing) →
//! `stomach` (hunger). All combat math lives in `crate::combat`; this module
//! owns input mapping, the status line, and round state (the `quiet` healing
//! counter). Rendering lives in `crate::ui`; the loop here just drives it.
//!
//! Level progression (stairs, the floor counter, the Amulet, the win) is
//! coordinated here too: `Game` is the integration point the combat module
//! reads the floor number from and extends the status line at.

use crossterm::cursor::{Hide, Show};
use crossterm::event::{read, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Terminal;
use std::collections::HashMap;
use std::io::{self, Stdout};

use crate::combat;
use crate::entity::{self, Monster, Player};
use crate::levels::{self, Amulet, Stairs, AMULET_LEVEL, MAX_FLOOR};
use crate::map::{self, Dungeon};
use crate::rng::Rng;

/// Glyph for a down staircase. Classic Rogue draws both stairs as `%`
/// (rogue.h `#define STAIRS '%'`); since our floors carry two staircases the
/// down one gets the familiar `>` so the two ends read clearly.
pub const STAIRS_DOWN_GLYPH: char = '>';
/// Glyph for an up staircase; on floor 1 this is the surface exit.
pub const STAIRS_UP_GLYPH: char = '<';
/// Glyph for the Amulet of Yendor while it lies on the floor (rogue.h
/// `#define AMULET ','`).
pub const AMULET_GLYPH: char = ',';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    North,
    South,
    East,
    West,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Escape,
    Unknown,
}

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
/// door, and corridor tiles are passable. Staircases and the Amulet sit on
/// walkable tiles, so they never block.
pub fn can_move(map: &Dungeon, x: usize, y: usize, dir: Direction) -> bool {
    let Some((nx, ny)) = next_position(x, y, dir) else {
        return false;
    };
    if nx >= map.width || ny >= map.height {
        return false;
    }
    map.tile(nx, ny).is_walkable()
}

/// One play session. `Game` is the integration point between level
/// progression, the spawner (`entity`), and combat: it owns the current
/// floor, the level's features, the victory state, and the round state.
pub struct Game {
    pub map: Dungeon,
    pub player: Player,
    pub monsters: Vec<Monster>,
    /// Current dungeon floor (1..=MAX_FLOOR). Combat reads it for monster
    /// scaling; level progression owns stairs and floors.
    pub floor: u32,
    /// The base seed of the game; each floor's map seed is derived from it
    /// with [`levels::floor_seed`], so re-entering a floor is deterministic.
    pub seed: u64,
    /// RNG for spawning and all combat rolls. Seeded from `seed`, so a seed
    /// reproduces the level, its inhabitants, and every die roll.
    pub rng: Rng,
    /// Quiet rounds since the last combat: resets to 0 on any fight and
    /// gates healing (report §7.3).
    pub quiet: u32,
    /// The most recent event, shown under the status line.
    pub last_message: Option<String>,
    pub stairs: Stairs,
    /// Where the Amulet lies on the floor, if it still does. `None` on every
    /// floor but [`AMULET_LEVEL`], and once the player carries it.
    pub amulet: Option<Amulet>,
    /// Whether the player carries the Amulet of Yendor (a simple carried
    /// flag; a real inventory comes later).
    pub has_amulet: bool,
    /// True once the player escapes: floor 1's exit stair with the Amulet.
    pub won: bool,
    /// Tiles the player has ever seen on the current floor, row-major over
    /// the map. Exploration memory: never-seen tiles render as solid rock,
    /// and re-entering a floor restores its seen grid.
    pub seen: Vec<bool>,
    /// Tiles lit right now — the room the player stands in, or the short
    /// line-of-sight reach down a corridor ([`crate::visibility`]).
    /// Monsters and items render only on visible tiles.
    pub visible: Vec<bool>,
    /// The `seen` grids of the other floors, keyed by floor number, so a
    /// floor the player leaves and re-enters restores its exploration
    /// memory. Cleared when a resize regenerates the map at a new size.
    seen_maps: HashMap<u32, Vec<bool>>,
}

impl Game {
    /// A fresh game on floor 1 of a newly generated dungeon.
    pub fn new(seed: u64, width: usize, height: usize) -> Game {
        Self::at_floor(seed, 1, width, height)
    }

    /// Wrap an already-generated map as a floor-1 game (tests, tools). For
    /// maps the generator would produce from `seed` this matches `new`
    /// exactly; hand-built fixture maps just get their features laid on top.
    pub fn from_map(map: Dungeon, seed: u64) -> Game {
        Self::assemble(map, 1, seed)
    }

    /// Generate (or deterministically regenerate) the floor `floor` and
    /// assemble a full game state around it. Public for tools (`--dump-frame
    /// --floor N`) that want a specific floor's state without playing to it.
    pub fn at_floor(seed: u64, floor: u32, width: usize, height: usize) -> Game {
        assert!((1..=MAX_FLOOR).contains(&floor), "floor {floor} out of range");
        let map = Dungeon::generate_sized(levels::floor_seed(seed, floor), width, height);
        Self::assemble(map, floor, seed)
    }

    /// Lay the player, monsters, stairs, and (on the Amulet floor) the Amulet
    /// onto one map. All placement rolls come from one rng seeded with the
    /// floor's map seed, so the same `(seed, floor)` always rebuilds the same
    /// level — which is what makes re-ascending deterministic.
    fn assemble(map: Dungeon, floor: u32, seed: u64) -> Game {
        let map_seed = levels::floor_seed(seed, floor);
        let mut rng = Rng::new(map_seed);
        let spawn = entity::populate(&map, floor, &mut rng);
        let stairs = levels::place_stairs(&map, &mut rng, (spawn.player.x, spawn.player.y));
        let amulet = (floor == AMULET_LEVEL)
            .then(|| levels::place_amulet(&map, &mut rng, stairs, (spawn.player.x, spawn.player.y)));
        let (map_w, map_h) = (map.width, map.height);
        let mut game = Game {
            map,
            player: Player::new(spawn.player.x, spawn.player.y),
            seed,
            floor,
            stairs,
            amulet,
            has_amulet: false,
            won: false,
            monsters: spawn.monsters,
            rng,
            quiet: 0,
            last_message: None,
            seen: vec![false; map_w * map_h],
            visible: Vec::new(),
            seen_maps: HashMap::new(),
        };
        game.refresh_visibility();
        game
    }

    /// One round (report §7.1): the player acts, then the AFTER daemons run
    /// in order — runners → doctor → stomach. Bumping a wall spends no turn.
    /// Staircases and the Amulet are handled between the player action and
    /// the AFTER phase, so descending moves the player before the new
    /// floor's monsters act.
    ///
    /// Returns whether the game state changed (a move, a fight — hit, miss,
    /// or counterattack — or a floor/win transition). A blocked move (wall
    /// or map edge) changes nothing and returns `false`, which the play loop
    /// uses to skip redrawing.
    pub fn step(&mut self, dir: Direction) -> bool {
        // 1. Player action. Moving into a monster starts a fight; the target
        //    is woken (runto) before the swing, so the player never benefits
        //    from the +4 "defender not running" bonus (report §4.4).
        let Some((nx, ny)) = next_position(self.player.x, self.player.y, dir) else {
            return false;
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
                    "You scored an excellent hit on the {} - you have defeated the {}",
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
            return false; // thud: a blocked step consumes no turn
        }
        self.player.running = true;

        // 2. Level features: pick up the Amulet, take a staircase, win.
        self.after_move();
        if self.won {
            return true;
        }

        // 3. AFTER phase: runners → doctor → stomach.
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
        self.refresh_visibility();
        true
    }

    /// Move the player one tile in `dir`; returns whether the position
    /// changed (a blocked move changes nothing). Pure movement: no fight, no
    /// stairs, no monster phase. Refreshes the view from the new position.
    pub fn move_player(&mut self, dir: Direction) -> bool {
        if can_move(&self.map, self.player.x, self.player.y, dir) {
            let (nx, ny) = next_position(self.player.x, self.player.y, dir)
                .expect("can_move validated the step");
            self.player.x = nx;
            self.player.y = ny;
            self.refresh_visibility();
            true
        } else {
            false
        }
    }

    /// React to the player's position after a successful move: pick up the
    /// Amulet, take a staircase, and detect the escape. The game loop stops
    /// when [`Game::won`] is set.
    pub fn after_move(&mut self) {
        self.try_pick_up_amulet();
        self.take_stairs();
    }

    /// Stepping onto the Amulet tile carries it (the "found the Amulet"
    /// event). It then leaves the floor.
    fn try_pick_up_amulet(&mut self) {
        let Some(amulet) = self.amulet else { return };
        if (self.player.x, self.player.y) == (amulet.x, amulet.y) {
            self.has_amulet = true;
            self.amulet = None;
        }
    }

    /// Stepping onto a staircase moves between floors; stepping onto floor 1's
    /// exit stair with the Amulet wins. On floor 26 the down stair leads
    /// nowhere and on floor 1 the exit is magically blocked without the
    /// Amulet — both are a no-op.
    fn take_stairs(&mut self) {
        let pos = (self.player.x, self.player.y);
        if pos == self.stairs.down {
            if self.floor < MAX_FLOOR {
                self.descend();
            }
        } else if pos == self.stairs.up {
            if levels::has_escaped(self.floor, true, self.has_amulet) {
                self.won = true;
            } else if self.floor > 1 {
                self.ascend();
            }
        }
    }

    /// Descend one floor: regenerate the next floor deterministically and
    /// arrive on its up staircase. Exploration memory is per floor: the
    /// current floor's seen grid is saved and the next floor's is restored
    /// if it was visited before.
    fn descend(&mut self) {
        let carried = self.has_amulet;
        let next = Self::at_floor(self.seed, self.floor + 1, self.map.width, self.map.height);
        let (x, y) = next.stairs.up;
        self.seen_maps.insert(self.floor, std::mem::take(&mut self.seen));
        let mut seen_maps = std::mem::take(&mut self.seen_maps);
        let next_seen = seen_maps.remove(&next.floor).unwrap_or_default();
        *self = next;
        self.seen_maps = seen_maps;
        let size = self.map.width * self.map.height;
        self.seen = if next_seen.len() == size {
            next_seen
        } else {
            vec![false; size]
        };
        self.has_amulet = carried;
        self.player = Player::new(x, y);
        self.refresh_visibility();
    }

    /// Ascend one floor: regenerate the previous floor deterministically and
    /// arrive on its down staircase. The Amulet travels with the player; the
    /// floor's exploration memory is saved and restored like a descent's.
    fn ascend(&mut self) {
        let carried = self.has_amulet;
        let next = Self::at_floor(self.seed, self.floor - 1, self.map.width, self.map.height);
        let (x, y) = next.stairs.down;
        self.seen_maps.insert(self.floor, std::mem::take(&mut self.seen));
        let mut seen_maps = std::mem::take(&mut self.seen_maps);
        let next_seen = seen_maps.remove(&next.floor).unwrap_or_default();
        *self = next;
        self.seen_maps = seen_maps;
        let size = self.map.width * self.map.height;
        self.seen = if next_seen.len() == size {
            next_seen
        } else {
            vec![false; size]
        };
        self.has_amulet = carried;
        self.player = Player::new(x, y);
        self.refresh_visibility();
    }

    /// Regenerate the current floor at a new map size (a terminal resize),
    /// keeping the player's stats, position (when it still lands on walkable
    /// floor), and carried Amulet. The level, monsters, and stairs re-roll
    /// deterministically for the new size from the same floor seed.
    pub fn resize(&mut self, width: usize, height: usize) {
        let carried = self.has_amulet;
        let (x, y) = (self.player.x, self.player.y);
        let mut player = self.player.clone();
        let mut next = Self::at_floor(self.seed, self.floor, width, height);
        if x < width && y < height && next.map.tile(x, y).is_walkable() {
            player.x = x;
            player.y = y;
        } else {
            player.x = next.player.x;
            player.y = next.player.y;
        }
        next.player = player;
        next.monsters.retain(|m| (m.x, m.y) != (next.player.x, next.player.y));
        if carried {
            next.amulet = None;
        }
        next.has_amulet = carried;
        *self = next;
        // The map regenerated at a new size: exploration memory is void.
        self.seen = vec![false; self.map.width * self.map.height];
        self.seen_maps.clear();
        self.refresh_visibility();
    }

    /// Recompute what the player can currently see from their position
    /// ([`crate::visibility`]) and fold it into the floor's exploration
    /// memory: every lit tile becomes seen. Called after every move, floor
    /// change, and resize, so `visible` is always current for rendering.
    pub fn refresh_visibility(&mut self) {
        self.visible = crate::visibility::compute(&self.map, self.player.x, self.player.y);
        debug_assert_eq!(self.visible.len(), self.seen.len());
        for (i, lit) in self.visible.iter().enumerate() {
            if *lit {
                self.seen[i] = true;
            }
        }
    }

    /// The glyph at `(x, y)` as every renderer draws it, through the field
    /// of view: the player is always drawn; a tile currently visible shows
    /// its full overlay (monsters, Amulet, staircases, tile); a tile
    /// explored but not currently visible shows the bare remembered tile
    /// (no features); a tile never explored renders as solid rock.
    pub fn glyph_at(&self, x: usize, y: usize) -> char {
        let pos = (x, y);
        if pos == (self.player.x, self.player.y) {
            return '@';
        }
        let idx = y * self.map.width + x;
        if !self.seen[idx] {
            return map::Tile::Wall.glyph(); // never explored: solid rock
        }
        if !self.visible[idx] {
            return self.map.tile(x, y).glyph(); // explored but dark: the bare tile
        }
        overlay_glyph(
            &self.player,
            &self.monsters,
            self.amulet,
            self.stairs,
            &self.map,
            x,
            y,
        )
    }

    /// The status line, formatted per report §9
    /// (`Level: 1  Gold: 0  Hp: 12(12)  Str: 16(16)  Arm: 4   Exp: 1/0`).
    /// `Level:` is the dungeon floor; `Exp:` is the player's experience level
    /// and points. "Arm" is `10 - effective AC`, so a fresh player in ring
    /// mail shows 4 while combat resolves against 6.
    pub fn status_line(&self) -> String {
        let p = &self.player;
        let mut line = format!(
            "Level: {}  Gold: {}  Hp: {}({})  Str: {}({})  Arm: {}   Exp: {}/{}",
            self.floor,
            p.gold,
            p.hp,
            p.max_hp,
            p.strength,
            p.strength,
            10 - combat::STARTING_ARMOR_AC,
            p.level,
            p.experience,
        );
        if self.has_amulet {
            line.push_str("  Amulet: carried");
        }
        if !self.has_amulet && self.at_exit() {
            line.push_str("  (the way out is magically blocked)");
        }
        line
    }

    /// The frame exactly as the interactive terminal draws it, as plain
    /// text: the map, then the status line and a hint or message line.
    /// Rendered through the shared ratatui buffer path (`crate::ui`), so
    /// `--dump-frame` and the golden tests see output character-identical
    /// to the terminal.
    pub fn render(&self) -> String {
        crate::ui::render_text(self, self.map.width, self.map.height + crate::ui::UI_LINES)
    }

    /// One compact line summarizing the final state, for `--script` and the
    /// golden tests: seed, floor, hp/max, str, gold, exp, level, won/dead,
    /// monsters remaining, and whether the Amulet is carried. Dead and
    /// escaped states are explicit (`dead: true` / `won: true`).
    pub fn snapshot(&self) -> String {
        format!(
            "seed: {}  floor: {}  hp: {}/{}  str: {}  gold: {}  exp: {}  level: {}  won: {}  dead: {}  monsters: {}  amulet: {}",
            self.seed,
            self.floor,
            self.player.hp,
            self.player.max_hp,
            self.player.strength,
            self.player.gold,
            self.player.experience,
            self.player.level,
            self.won,
            self.player.hp <= 0,
            self.monsters.len(),
            self.has_amulet,
        )
    }

    /// Is the player standing on floor 1's exit stair?
    fn at_exit(&self) -> bool {
        self.floor == 1 && (self.player.x, self.player.y) == self.stairs.up
    }

    #[cfg(test)]
    /// The test twin of a successful move in the play loop: put the player
    /// on `pos` and run one `after_move`.
    fn step_onto(&mut self, pos: (usize, usize)) {
        self.player = Player::new(pos.0, pos.1);
        self.after_move();
    }
}

/// The glyph one cell shows, with every feature overlaid in display order:
/// the player, then a monster, then the Amulet, then the staircases, then
/// the bare tile. The single overlay used by `Game::glyph_at`, `render_level`
/// and the ratatui frame, so every renderer agrees on one cell's glyph.
fn overlay_glyph(
    player: &Player,
    monsters: &[Monster],
    amulet: Option<Amulet>,
    stairs: Stairs,
    map: &Dungeon,
    x: usize,
    y: usize,
) -> char {
    let pos = (x, y);
    if pos == (player.x, player.y) {
        '@'
    } else if let Some(monster) = monsters.iter().find(|m| (m.x, m.y) == pos) {
        monster.symbol()
    } else if amulet.is_some_and(|a| (a.x, a.y) == pos) {
        AMULET_GLYPH
    } else if pos == stairs.down {
        STAIRS_DOWN_GLYPH
    } else if pos == stairs.up {
        STAIRS_UP_GLYPH
    } else {
        map.tile(x, y).glyph()
    }
}

/// A level's map with its features overlaid: monsters, staircases, the Amulet
/// (`,`) and the player (`@`). Shared by `--dump-map` and the render tests.
pub fn render_level(
    map: &Dungeon,
    player: Player,
    stairs: Stairs,
    amulet: Option<Amulet>,
    monsters: &[entity::Monster],
) -> String {
    let mut out = String::with_capacity((map.width + 1) * map.height);
    for y in 0..map.height {
        for x in 0..map.width {
            out.push(overlay_glyph(&player, monsters, amulet, stairs, map, x, y));
        }
        out.push('\n');
    }
    out
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

/// Parse a script string into keys, for `--script` and the golden tests: a
/// bare character is that key (`h`/`j`/`k`/`l`, `q`, ...); the arrow and
/// escape keys are spelled `<left>`/`<right>`/`<up>`/`<down>`/`<esc>`.
/// Unknown `<...>` tokens are an error so a typo cannot silently no-op.
pub fn script_keys(script: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = script;
    while !rest.is_empty() {
        if let Some(after_lt) = rest.strip_prefix('<')
            && let Some(end) = after_lt.find('>')
        {
            let (token, tail) = after_lt.split_at(end);
            let key = match token {
                "left" => Key::Left,
                "right" => Key::Right,
                "up" => Key::Up,
                "down" => Key::Down,
                "esc" => Key::Escape,
                other => return Err(format!("unknown key token `<{other}>`")),
            };
            keys.push(key);
            rest = &tail[1..];
            continue;
        }
        let c = rest.chars().next().expect("rest is non-empty");
        keys.push(Key::Char(c.to_ascii_lowercase()));
        rest = &rest[c.len_utf8()..];
    }
    Ok(keys)
}

/// Does the interactive loop treat `key` as quit (`q` or Escape)? The
/// scripted loop uses the same rule so a script and a real session agree.
fn is_quit(key: Key) -> bool {
    matches!(key, Key::Char('q') | Key::Escape)
}

/// Run a scripted session with no terminal: apply each key exactly like the
/// interactive loop would — `q`/Escape quits, unknown keys and blocked
/// moves change nothing, and the game stops early once the player dies or
/// escapes — then return the final state. Deterministic for a given seed,
/// so scripts double as regression tests for whole runs.
pub fn play(seed: u64, width: usize, height: usize, keys: &[Key]) -> Game {
    let mut game = Game::new(seed, width, height);
    for &key in keys {
        if game.won || game.player.hp <= 0 || is_quit(key) {
            break;
        }
        let Some(dir) = key_to_direction(key) else { continue };
        game.step(dir);
    }
    game
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

/// One event the play loop reacts to: a keypress or a terminal resize.
enum LoopEvent {
    Key(Key),
    Resize(u16, u16),
}

/// Block for one event: a keypress, or a resize so the frame can be
/// regenerated at the new size. Mouse events just wait for the next event;
/// the following frame redraws the whole screen anyway.
fn read_event() -> io::Result<LoopEvent> {
    loop {
        match read()? {
            Event::Key(event) => return Ok(LoopEvent::Key(key_from_event(event))),
            Event::Resize(cols, rows) => return Ok(LoopEvent::Resize(cols, rows)),
            _ => continue,
        }
    }
}

/// Block for any keypress; used by the victory screen, which has no layout
/// to resize.
fn any_key() -> io::Result<Key> {
    loop {
        match read()? {
            Event::Key(event) => return Ok(key_from_event(event)),
            _ => continue,
        }
    }
}

/// The victory screen: the player escaped with the Amulet.
fn escape_screen(terminal: &mut Terminal<CrosstermBackend<Stdout>>, game: &Game) -> io::Result<()> {
    terminal.draw(|frame| {
        let text = vec![
            Line::from("You escaped with the Amulet!"),
            Line::from(""),
            Line::from("Climbing the last stair, you step out of the Dungeons of Doom"),
            Line::from("into the light of day, the Amulet of Yendor warm in your pack."),
            Line::from(""),
            Line::from(format!("Final floor: {}   seed: {}", game.floor, game.seed)),
            Line::from(""),
            Line::from("Press any key to return to your terminal."),
        ];
        frame.render_widget(Paragraph::new(text), frame.area());
    })?;
    any_key()?;
    Ok(())
}

/// Play the game: size the map to the terminal (or the explicit
/// `--width`/`--height` overrides), draw the current floor, then loop on
/// keys until the player quits, dies, or escapes with the Amulet. The frame
/// is drawn through ratatui, which owns raw mode, the alternate screen, and
/// resize handling; terminal resizes regenerate the floor at the new size
/// and redraw immediately.
pub fn run(seed: u64, width: Option<usize>, height: Option<usize>) -> io::Result<()> {
    // Fall back to the classic extent when there is no terminal to query.
    let (cols, rows) = size().unwrap_or((map::MAP_WIDTH as u16, map::MAP_HEIGHT as u16));
    let (w, h) = map::resolve_map_size(width, height, cols as usize, rows as usize);
    let mut game = Game::new(seed, w, h);
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    loop {
        terminal.draw(|frame| crate::ui::draw(frame, &game))?;
        if game.player.hp <= 0 {
            return Ok(()); // the frame above shows "you have died"
        }
        // The frame on screen is current; wait for an event that actually
        // changes the game. Unknown keys and blocked moves change nothing
        // and are ignored without a redraw, so an idle terminal stays
        // perfectly still instead of flickering (ratatui's diffing writes
        // only the cells that changed anyway).
        loop {
            match read_event()? {
                LoopEvent::Resize(cols, rows) => {
                    let (w, h) =
                        map::resolve_map_size(width, height, cols as usize, rows as usize);
                    if (w, h) != (game.map.width, game.map.height) {
                        game.resize(w, h);
                        // A new size: ratatui's next draw picks up the new
                        // viewport and the regenerated floor replaces the
                        // old frame wholesale — no stale rows to clear by
                        // hand, and the terminal can never be left torn.
                        break;
                    }
                    // Same size: nothing changed, keep waiting.
                }
                LoopEvent::Key(Key::Char('q')) | LoopEvent::Key(Key::Escape) => return Ok(()),
                LoopEvent::Key(key) => {
                    let Some(dir) = key_to_direction(key) else { continue };
                    let changed = game.step(dir);
                    if game.won {
                        return escape_screen(&mut terminal, &game);
                    }
                    if changed {
                        break; // redraw the new state below
                    }
                    // A blocked move: nothing changed, keep waiting.
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
    fn script_keys_parse_chars_and_arrows() {
        let keys = script_keys("h<left>j<right>k<up>l<down>q<esc>x").unwrap();
        assert_eq!(
            keys,
            vec![
                Key::Char('h'),
                Key::Left,
                Key::Char('j'),
                Key::Right,
                Key::Char('k'),
                Key::Up,
                Key::Char('l'),
                Key::Down,
                Key::Char('q'),
                Key::Escape,
                Key::Char('x'),
            ]
        );
        assert_eq!(
            script_keys("LL").unwrap(),
            vec![Key::Char('l'), Key::Char('l')],
            "lowercased"
        );
        assert!(script_keys("<nope>").is_err(), "unknown token is an error");
        assert_eq!(script_keys("").unwrap(), Vec::new());
    }

    #[test]
    fn play_applies_the_script_like_the_interactive_loop() {
        // An 8x4 map is a single room centered at (3,1). h/west then
        // j/south land cleanly on seed 1 (no monster in the way).
        let game = play(1, 8, 4, &script_keys("hj").unwrap());
        assert_eq!((game.player.x, game.player.y), (2, 2));
        // Unknown keys change nothing; blocked moves change nothing.
        let game = play(1, 8, 4, &script_keys("zh").unwrap());
        assert_eq!((game.player.x, game.player.y), (2, 1));
    }

    #[test]
    fn play_stops_at_quit_death_and_escape() {
        // `q` stops the script: later keys are ignored.
        let quit = play(7, crate::map::MAP_WIDTH, 22, &script_keys("qll").unwrap());
        assert_eq!(
            (quit.player.x, quit.player.y),
            (quit.map.rooms()[0].center().0, quit.map.rooms()[0].center().1)
        );
        assert_eq!(quit.player.hp, 12);
        // Escape quits too.
        let esc = play(7, crate::map::MAP_WIDTH, 22, &script_keys("<esc>l").unwrap());
        assert_eq!(esc.snapshot(), quit.snapshot());
    }

    #[test]
    fn snapshot_reports_the_final_state_explicitly() {
        let game = play(7, crate::map::MAP_WIDTH, 22, &[]);
        let snap = game.snapshot();
        assert!(
            snap.starts_with("seed: 7  floor: 1  hp: 12/12  str: 16  gold: 0  exp: 0  level: 1 "),
            "{snap}"
        );
        assert!(snap.contains("won: false"), "{snap}");
        assert!(snap.contains("dead: false"), "{snap}");
        assert!(snap.contains("monsters: "), "{snap}");
        assert!(snap.contains("amulet: false"), "{snap}");
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
        let game = Game::from_map(map, 7);
        assert_eq!((game.player.x, game.player.y), (x, y));
        assert!(game.map.tile(x, y).is_walkable());
        assert_eq!(game.floor, 1);
        assert!(!game.has_amulet);
        assert!(!game.won);
    }

    #[test]
    fn move_player_walks_the_level_and_stops_at_walls() {
        // map_from maps have no rooms: the player lands in the top-left.
        let mut game = Game::from_map(map_from(&TEST_MAP), 0);
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
            game.move_player(dir);
        }
        assert_eq!((game.player.x, game.player.y), (6, 0));
        assert_eq!(game.map.tile(6, 0), Tile::Corridor);

        // (7,0) is a wall: the step is refused and the player stays put.
        game.move_player(Direction::East);
        assert_eq!((game.player.x, game.player.y), (6, 0));
    }

    #[test]
    fn render_marks_the_player_once_and_shows_the_status_line() {
        let mut game = Game::from_map(map_from(&TEST_MAP), 0);
        game.move_player(Direction::East);
        let frame = game.render();
        assert_eq!(frame.matches('@').count(), 1);
        // Map lines, then the §9 status line, then a hint/message line.
        let lines: Vec<&str> = frame.lines().collect();
        assert_eq!(lines.len(), TEST_MAP.len() + 2);
        // The status line is truncated to the 8-column fixture map so it
        // never wraps; the full §9 format is `status_line()`'s own contract.
        assert_eq!(lines[TEST_MAP.len()], "Level: 1");
        assert!(game.status_line().starts_with("Level: 1  Gold: 0 "));
    }

    /// Round-order smoke: the player acts before monsters. A guaranteed-kill
    /// player moving into a running kestrel kills it during their action, so
    /// it never gets a counterattack that round.
    #[test]
    fn round_order_player_kills_before_monsters_act() {
        let mut game = Game::from_map(map_from(&TEST_MAP), 1);
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
        let mut game = Game::from_map(map_from(&TEST_MAP), 2);
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
        let game = Game::from_map(map_from(&TEST_MAP), 0);
        assert_eq!(
            game.status_line(),
            "Level: 1  Gold: 0  Hp: 12(12)  Str: 16(16)  Arm: 4   Exp: 1/0"
        );
    }

    #[test]
    fn game_starts_on_floor_one_with_both_stairs() {
        let game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        assert_eq!(game.floor, 1);
        assert_ne!(game.stairs.up, game.stairs.down);
        assert!(game.amulet.is_none(), "no Amulet on floor 1");
        assert!(game.map.tile(game.stairs.up.0, game.stairs.up.1).is_walkable());
        assert!(game.map.tile(game.stairs.down.0, game.stairs.down.1).is_walkable());
    }

    /// Descending regenerates the next floor deterministically; ascending
    /// rebuilds the floor above identically, so a round trip returns to the
    /// exact same level (map, monsters, and stairs).
    #[test]
    fn descend_and_ascend_round_trip_is_deterministic() {
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        let floor1_map = game.map.render();
        let floor1_monsters = game.monsters.clone();
        let floor1_stairs = game.stairs;

        game.step_onto(game.stairs.down);
        assert_eq!(game.floor, 2);
        // The player arrives on the new floor's up staircase.
        assert_eq!((game.player.x, game.player.y), game.stairs.up);

        let floor2_map = game.map.render();
        let floor2_stairs = game.stairs;
        assert_ne!(floor2_map, floor1_map, "floor 2 must differ from floor 1");

        game.step_onto(game.stairs.up);
        assert_eq!(game.floor, 1);
        // Back on the down staircase of the regenerated floor 1.
        assert_eq!((game.player.x, game.player.y), game.stairs.down);
        assert_eq!(game.map.render(), floor1_map, "floor 1 must regenerate identically");
        assert_eq!(game.monsters, floor1_monsters);
        assert_eq!(game.stairs, floor1_stairs);
        assert_ne!(game.stairs, floor2_stairs);
    }

    #[test]
    fn floor_counter_stays_within_bounds() {
        // Cannot ascend above floor 1.
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        game.step_onto(game.stairs.up);
        assert_eq!(game.floor, 1, "exit stair is magically blocked without the Amulet");
        assert!(!game.won);
        assert!(game.status_line().contains("magically blocked"));

        // Cannot descend below floor 26.
        for _ in 0..25 {
            game.step_onto(game.stairs.down);
        }
        assert_eq!(game.floor, 26);
        game.step_onto(game.stairs.down);
        assert_eq!(game.floor, 26, "the dungeon ends at floor 26");
    }

    #[test]
    fn amulet_lies_only_on_floor_26_and_is_picked_up_by_stepping_on_it() {
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        for _ in 0..24 {
            game.step_onto(game.stairs.down);
        }
        assert_eq!(game.floor, 25);
        assert!(game.amulet.is_none(), "no Amulet on floor 25");

        game.step_onto(game.stairs.down);
        assert_eq!(game.floor, 26);
        let amulet = game.amulet.expect("the Amulet lies on floor 26");
        assert_eq!(
            game.map.tile(amulet.x, amulet.y),
            Tile::Floor,
            "Amulet not on a floor tile"
        );

        // Stepping onto it is the "found the Amulet" event.
        game.step_onto((amulet.x, amulet.y));
        assert!(game.has_amulet);
        assert!(game.amulet.is_none(), "the Amulet leaves the floor once carried");
        assert!(!game.won, "carrying the Amulet alone does not win");
        assert!(game.status_line().contains("Amulet: carried"));
    }

    #[test]
    fn escape_requires_floor_one_and_the_amulet() {
        // On floor 1's exit stair without the Amulet: magically blocked.
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        game.step_onto(game.stairs.up);
        assert_eq!(game.floor, 1);
        assert!(!game.won);

        // With the Amulet (simulated), stepping on the exit wins.
        game.has_amulet = true;
        game.after_move();
        assert!(game.won);
    }

    /// The whole journey: descend all the way to floor 26, pick up the
    /// Amulet, climb back out, and escape through floor 1's exit.
    #[test]
    fn full_victory_journey() {
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        for _ in 0..25 {
            game.step_onto(game.stairs.down);
        }
        assert_eq!(game.floor, 26);

        let amulet = game.amulet.expect("Amulet on floor 26");
        game.step_onto((amulet.x, amulet.y));
        assert!(game.has_amulet);

        // Climb back up (floor 26 -> 2, then the last stair onto floor 1).
        for _ in 0..24 {
            game.step_onto(game.stairs.up);
        }
        assert_eq!(game.floor, 2);
        game.step_onto(game.stairs.up);
        assert_eq!(game.floor, 1);
        assert!(!game.won, "arriving on floor 1 does not win by itself");

        // Step onto the exit stair with the Amulet: escape.
        game.step_onto(game.stairs.up);
        assert!(game.won);
        assert_eq!(game.floor, 1);
    }

    #[test]
    fn status_line_leads_with_the_level_field() {
        let game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        assert!(game.status_line().starts_with("Level: 1 "), "{}", game.status_line());
        let mut deep = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        for _ in 0..25 {
            deep.step_onto(deep.stairs.down);
        }
        assert!(deep.status_line().starts_with("Level: 26 "), "{}", deep.status_line());
    }

    #[test]
    fn render_level_draws_stairs_and_amulet_glyphs() {
        let map = Dungeon::generate(3);
        let stairs = Stairs {
            up: (1, 1),
            down: (2, 2),
        };
        let amulet = Amulet { x: 4, y: 4 };
        let frame = render_level(&map, Player::new(1, 2), stairs, Some(amulet), &[]);
        assert!(frame.contains(STAIRS_DOWN_GLYPH), "missing down stair glyph");
        assert!(frame.contains(STAIRS_UP_GLYPH), "missing up stair glyph");
        assert!(frame.contains(AMULET_GLYPH), "missing Amulet glyph");
        assert_eq!(frame.matches('@').count(), 1);
    }

    /// A game fitted to a small terminal renders exactly the terminal's
    /// height in rows — map, status line, hint line — with no line wider
    /// than the terminal (nothing to wrap).
    #[test]
    fn fitted_game_renders_within_the_terminal_bounds() {
        let (w, h) = map::fit_bounds(40, 15);
        assert_eq!((w, h), (40, 13));
        let game = Game::new(7, w, h);
        let frame = game.render();
        let lines: Vec<&str> = frame.lines().collect();
        assert_eq!(lines.len(), 15, "map + status + hint");
        assert!(lines.iter().all(|l| l.chars().count() <= 40), "no wrapped lines");
        assert_eq!(lines.len(), game.map.height + 2);
        assert!(lines[13].starts_with("Level: 1 "), "{} ", lines[13]);
        assert!(lines[14].contains("seed"), "{}", lines[14]);
    }

    /// The invariant end to end: a game fitted to a short window renders
    /// exactly the window's height in rows — map, status line, hint line —
    /// with every line exactly the map width (nothing to wrap, no stale
    /// tail), including windows the old `MIN_FIT_HEIGHT` floor overflowed.
    #[test]
    fn fitted_game_renders_within_short_terminals() {
        for (cols, rows) in [(80, 10), (40, 12), (30, 14), (80, 12), (80, 6), (24, 6)] {
            let (w, h) = map::fit_bounds(cols, rows);
            assert_eq!(h + 2, rows, "{cols}x{rows}: map + UI must fit the window");
            let game = Game::new(7, w, h);
            let frame = game.render();
            let lines: Vec<&str> = frame.lines().collect();
            assert_eq!(lines.len(), rows, "{cols}x{rows}: map + status + hint");
            assert_eq!(lines.len(), game.map.height + 2, "{cols}x{rows}");
            assert!(
                lines.iter().all(|l| l.chars().count() == w),
                "{cols}x{rows}: every line exactly the map width"
            );
            assert!(lines[h].starts_with("Level: 1 "), "{cols}x{rows}: {}", lines[h]);
            assert!(lines[h + 1].contains("seed"), "{cols}x{rows}: {}", lines[h + 1]);
        }
    }

    /// A terminal resize regenerates the floor at the new size, keeps the
    /// player's stats (and position when it still lands on floor), and
    /// renders within the new bounds.
    #[test]
    fn resize_regenerates_at_the_new_size_keeping_stats_and_position() {
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        // Make stat preservation observable.
        game.player.hp = 5;
        game.player.experience = 100;
        let (px, py) = (game.player.x, game.player.y);

        game.resize(40, 13);

        assert_eq!((game.map.width, game.map.height), (40, 13));
        assert_eq!(game.player.hp, 5, "resize must not reset the player");
        assert_eq!(game.player.experience, 100);
        assert!(game.player.x < 40 && game.player.y < 13);
        assert!(
            game.map.tile(game.player.x, game.player.y).is_walkable(),
            "player lands on walkable floor"
        );
        if (game.player.x, game.player.y) == (px, py) {
            assert!(px < 40 && py < 13, "kept position must be in-bounds");
        }
        let frame = game.render();
        let lines: Vec<&str> = frame.lines().collect();
        assert_eq!(lines.len(), 15);
        assert!(lines.iter().all(|l| l.chars().count() <= 40));
        assert!(lines[13].starts_with("Level: 1 "));
    }

    /// Resizing mid-descent stays on the same floor.
    #[test]
    fn resize_keeps_the_current_floor() {
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        game.step_onto(game.stairs.down);
        assert_eq!(game.floor, 2);
        let map_seed = levels::floor_seed(game.seed, game.floor);
        game.resize(60, 20);
        assert_eq!(game.floor, 2);
        assert_eq!((game.map.width, game.map.height), (60, 20));
        assert_eq!(
            game.map.render(),
            Dungeon::generate_sized(map_seed, 60, 20).render(),
            "the resized floor regenerates deterministically"
        );
    }

    // -------------------------------------------------------------------
    // Field of view
    // -------------------------------------------------------------------

    /// Two rooms joined by a corridor through doors (the same fixture as
    /// `visibility::tests`): room A interior (1..6, 1..5), room B interior
    /// (16..21, 1..5), doors at (7, 5) and (15, 5), corridor row 5 from
    /// (8, 5) to (14, 5).
    const TWO_ROOMS: [&str; 7] = [
        "#######################",
        "#......#########......#",
        "#......#########......#",
        "#......#########......#",
        "#......#########......#",
        "#......+~~~~~~~+......#",
        "#######################",
    ];

    /// Move the player along a path of directions (no combat, no stairs).
    fn walk(game: &mut Game, dirs: &[Direction]) {
        for &dir in dirs {
            assert!(game.move_player(dir), "the step {:?} must be walkable", dir);
        }
    }

    /// A fresh game renders only the starting room: the room's floor and
    /// door are lit, and everything else is solid rock.
    #[test]
    fn fresh_game_sees_only_the_starting_room() {
        let game = Game::from_map(map_from(&TWO_ROOMS), 1);
        // Roomless fixture maps spawn the player at (1, 1).
        assert_eq!((game.player.x, game.player.y), (1, 1));
        assert_eq!(game.glyph_at(1, 1), '@');
        assert_eq!(game.glyph_at(3, 3), '.', "room A floor is lit");
        assert_eq!(game.glyph_at(7, 5), '+', "the door is lit from inside the room");
        assert_eq!(game.glyph_at(11, 5), '#', "the corridor is unexplored rock");
        assert_eq!(game.glyph_at(18, 3), '#', "room B is unexplored rock");
        assert_eq!(game.glyph_at(20, 5), '#', "room B's far edge is rock too");
    }

    /// Leaving a room keeps its tiles seen (they render as the bare tile,
    /// not rock), while never-seen tiles stay rock.
    #[test]
    fn leaving_a_room_keeps_it_seen_but_unlit() {
        let mut game = Game::from_map(map_from(&TWO_ROOMS), 1);
        // Walk out of room A into the corridor.
        walk(
            &mut game,
            &[
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::South,
                Direction::South,
                Direction::South,
                Direction::South,
                Direction::East,
                Direction::East,
            ],
        );
        assert_eq!((game.player.x, game.player.y), (8, 5), "in the corridor");

        // Room A is remembered: seen but not visible, rendered bare.
        let idx = 3 * game.map.width + 3;
        assert!(game.seen[idx], "room A was explored");
        assert!(!game.visible[idx], "room A is dark from the corridor");
        assert_eq!(game.glyph_at(3, 3), '.', "the remembered tile renders bare");
        // Room B was never seen and renders as rock.
        assert_eq!(game.glyph_at(18, 3), '#', "never-seen room B is rock");
        // The door is within the corridor's short reach and stays visible.
        assert_eq!(game.glyph_at(7, 5), '+', "the door down the corridor is lit");
    }

    /// Monsters render only on currently visible tiles: a monster in a
    /// distant room is not drawn while the player is elsewhere, is drawn
    /// when the player walks into its room, and disappears again once its
    /// tile goes dark (the tile stays remembered).
    #[test]
    fn monsters_render_only_when_on_a_visible_tile() {
        let mut game = Game::from_map(map_from(&TWO_ROOMS), 1);
        let snake = monster(MonsterKind::Snake, 18, 3);
        let letter = snake.symbol();
        game.monsters = vec![snake];

        // In room A the monster in room B is not drawn (its tile is rock).
        assert_eq!(game.glyph_at(18, 3), '#');

        // Walk into room B: the monster is on a visible tile and renders.
        walk(
            &mut game,
            &[
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::South,
                Direction::South,
                Direction::South,
                Direction::South,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
                Direction::East,
            ],
        );
        assert_eq!((game.player.x, game.player.y), (16, 5), "inside room B");
        assert_eq!(game.glyph_at(18, 3), letter, "the monster renders in a lit room");

        // Step back out into the corridor: the tile is remembered but dark,
        // so the monster is no longer drawn.
        walk(&mut game, &[Direction::West, Direction::West]);
        assert_eq!((game.player.x, game.player.y), (14, 5), "out in the corridor");
        assert_eq!(game.glyph_at(18, 3), '.', "the tile is remembered, the monster is not");
    }

    /// Exploration memory persists across floor changes: leaving a floor
    /// saves its seen grid, and re-entering it restores it.
    #[test]
    fn explored_tiles_stay_seen_across_floor_changes() {
        let mut game = Game::new(7, crate::map::MAP_WIDTH, crate::map::MAP_HEIGHT);
        let floor1_seen = game.seen.clone();
        assert!(floor1_seen.iter().any(|&s| s), "the starting room is seen on floor 1");

        game.step_onto(game.stairs.down);
        assert_eq!(game.floor, 2);
        let floor2_seen = game.seen.clone();
        assert!(floor2_seen.iter().any(|&s| s), "the arrival room is seen on floor 2");

        game.step_onto(game.stairs.up);
        assert_eq!(game.floor, 1);
        // Re-entering floor 1 restores its exploration memory (plus any
        // tiles newly lit from the arrival position).
        assert!(
            game.seen.iter().zip(&floor1_seen).all(|(a, b)| !*b || *a),
            "every previously seen floor-1 tile is still seen"
        );
    }
}
