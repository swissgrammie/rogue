//! Entities: the player, monsters, and the classic Rogue 5.4.4 monster table.
//!
//! Data model only — combat math lives in `crate::combat`, which consumes
//! the table's `exp`/`lvl`/`arm`/`dmg` columns (report §12). Positions are
//! `(x, y)` map coordinates (column, row), matching the map module.
//!
//! Monsters follow the canonical Aquator…Zombie table of Rogue 5.4.4 (the
//! final classic release): all 26 monsters, one per letter A–Z, with the
//! canonical experience, level, armor class and damage strings (see
//! [`MonsterKind::stats`]). The table's `hpt` column is an unused placeholder
//! in the original — real hit points are `roll(lvl, 8)` at spawn ([`Monster::spawn`]).

use crate::map::{Dungeon, Tile};
use crate::rng::Rng;
use std::collections::HashSet;
use std::fmt;

/// The player's starting hit points (Rogue's `INIT_STATS`: HP 12).
pub const PLAYER_START_HP: i32 = 12;
/// The player's starting strength (Rogue's `INIT_STATS`: STR 16).
pub const PLAYER_START_STRENGTH: i32 = 16;
/// The player starts penniless.
pub const PLAYER_START_GOLD: i32 = 0;
/// A fresh adventurer has no experience.
pub const PLAYER_START_EXPERIENCE: i32 = 0;
/// ...and begins at experience level one.
pub const PLAYER_START_LEVEL: u32 = 1;

/// The player character, `@` on the level.
///
/// Classic Rogue attributes — hit points, strength, gold and experience —
/// are tracked here; combat and the game loop read and mutate the fields.
/// The player's weapon and armor live in `combat` (the starting mace and
/// ring mail) until the items task lands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    pub x: usize,
    pub y: usize,
    pub max_hp: i32,
    pub hp: i32,
    pub strength: i32,
    pub gold: i32,
    pub experience: i32,
    pub level: u32,
    /// ISRUN: awake and active. Set when the player takes an action; combat
    /// clears it for disabled (frozen/sleeping) players, who are then easier
    /// to hit (the +4 "defender not running" bonus, report §4.4).
    pub running: bool,
}

impl Player {
    /// A fresh adventurer at `(x, y)` with the classic starting stats.
    pub fn new(x: usize, y: usize) -> Player {
        Player {
            x,
            y,
            max_hp: PLAYER_START_HP,
            hp: PLAYER_START_HP,
            strength: PLAYER_START_STRENGTH,
            gold: PLAYER_START_GOLD,
            experience: PLAYER_START_EXPERIENCE,
            level: PLAYER_START_LEVEL,
            running: true,
        }
    }

    /// The player's map glyph.
    pub fn symbol(&self) -> char {
        '@'
    }
}

/// The canonical, static stats of one monster kind, straight from the classic
/// Rogue 5.4.4 monster table (`extern.c:188`). Combat consumes the `exp`,
/// `level`, `armor_class` and `damage` columns (report §12); hit points are
/// NOT a table stat — they are `roll(level, 8)` at spawn ([`Monster::spawn`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterStats {
    /// Base experience awarded for killing it (`exp` column).
    pub exp: i32,
    /// Base level (`lvl` column): the monster's attacker level in `swing()`
    /// and the dice count for spawn HP.
    pub level: u32,
    /// Armor class (`arm` column). Lower is better; can be negative
    /// (dragon −1, black unicorn −2), which makes those monsters *harder*
    /// to hit.
    pub armor_class: i32,
    /// Damage string (`dmg` column) in `"NdS[/NdS…]"` form: `'x'` separates
    /// dice from sides (`"2x4"` = 2d4), `'/'` separates independent attacks
    /// each with its own to-hit roll. `"0x0"` deals no damage; the flytrap's
    /// `"%%%x0"` is a runtime-rewritten placeholder that parses as `0x0`.
    pub damage: &'static str,
    /// ISMEAN: only mean monsters wake on sight and chase the player
    /// (report §7.2). Greedy monsters chase gold instead and stay put.
    pub is_mean: bool,
}

/// One of the 26 classic Rogue monsters, one per letter A–Z.
///
/// Variants are declared in letter order, so `self as u8` is the A–Z offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterKind {
    Aquator,
    Bat,
    Centaur,
    Dragon,
    Emu,
    Flytrap,
    Griffin,
    Hobgoblin,
    IceMonster,
    Jabberwock,
    Kestrel,
    Leprechaun,
    Medusa,
    Nymph,
    Orc,
    Phantom,
    Quagga,
    Rattlesnake,
    Snake,
    Troll,
    BlackUnicorn,
    Vampire,
    Wraith,
    Xeroc,
    Yeti,
    Zombie,
}

impl MonsterKind {
    /// All 26 kinds in letter order: index 0 = 'A' (Aquator) … 25 = 'Z' (Zombie).
    pub const ALL: [MonsterKind; 26] = [
        MonsterKind::Aquator,
        MonsterKind::Bat,
        MonsterKind::Centaur,
        MonsterKind::Dragon,
        MonsterKind::Emu,
        MonsterKind::Flytrap,
        MonsterKind::Griffin,
        MonsterKind::Hobgoblin,
        MonsterKind::IceMonster,
        MonsterKind::Jabberwock,
        MonsterKind::Kestrel,
        MonsterKind::Leprechaun,
        MonsterKind::Medusa,
        MonsterKind::Nymph,
        MonsterKind::Orc,
        MonsterKind::Phantom,
        MonsterKind::Quagga,
        MonsterKind::Rattlesnake,
        MonsterKind::Snake,
        MonsterKind::Troll,
        MonsterKind::BlackUnicorn,
        MonsterKind::Vampire,
        MonsterKind::Wraith,
        MonsterKind::Xeroc,
        MonsterKind::Yeti,
        MonsterKind::Zombie,
    ];

    /// The kind whose letter is `c`, e.g. `from_letter('K') == Some(Kestrel)`.
    pub fn from_letter(c: char) -> Option<MonsterKind> {
        if c.is_ascii_uppercase() {
            MonsterKind::ALL.get((c as u8 - b'A') as usize).copied()
        } else {
            None
        }
    }

    /// This kind's letter, 'A'..='Z'.
    pub fn letter(self) -> char {
        (self as u8 + b'A') as char
    }

    /// The classic lowercase name, e.g. "black unicorn".
    pub fn name(self) -> &'static str {
        match self {
            MonsterKind::Aquator => "aquator",
            MonsterKind::Bat => "bat",
            MonsterKind::Centaur => "centaur",
            MonsterKind::Dragon => "dragon",
            MonsterKind::Emu => "emu",
            MonsterKind::Flytrap => "venus flytrap",
            MonsterKind::Griffin => "griffin",
            MonsterKind::Hobgoblin => "hobgoblin",
            MonsterKind::IceMonster => "ice monster",
            MonsterKind::Jabberwock => "jabberwock",
            MonsterKind::Kestrel => "kestrel",
            MonsterKind::Leprechaun => "leprechaun",
            MonsterKind::Medusa => "medusa",
            MonsterKind::Nymph => "nymph",
            MonsterKind::Orc => "orc",
            MonsterKind::Phantom => "phantom",
            MonsterKind::Quagga => "quagga",
            MonsterKind::Rattlesnake => "rattlesnake",
            MonsterKind::Snake => "snake",
            MonsterKind::Troll => "troll",
            MonsterKind::BlackUnicorn => "black unicorn",
            MonsterKind::Vampire => "vampire",
            MonsterKind::Wraith => "wraith",
            MonsterKind::Xeroc => "xeroc",
            MonsterKind::Yeti => "yeti",
            MonsterKind::Zombie => "zombie",
        }
    }

    /// The canonical stats from the classic Rogue 5.4.4 monster table
    /// (`extern.c:188`): exp, lvl, arm, dmg per report §6.1.
    pub fn stats(self) -> MonsterStats {
        match self {
            MonsterKind::Aquator => MonsterStats {
                exp: 20,
                level: 5,
                armor_class: 2,
                damage: "0x0/0x0",
                is_mean: true,
            },
            MonsterKind::Bat => MonsterStats {
                exp: 1,
                level: 1,
                armor_class: 3,
                damage: "1x2",
                is_mean: false,
            },
            MonsterKind::Centaur => MonsterStats {
                exp: 17,
                level: 4,
                armor_class: 4,
                damage: "1x2/1x5/1x5",
                is_mean: false,
            },
            MonsterKind::Dragon => MonsterStats {
                exp: 5000,
                level: 10,
                armor_class: -1,
                damage: "1x8/1x8/3x10",
                is_mean: true,
            },
            MonsterKind::Emu => MonsterStats {
                exp: 2,
                level: 1,
                armor_class: 7,
                damage: "1x2",
                is_mean: true,
            },
            MonsterKind::Flytrap => MonsterStats {
                exp: 80,
                level: 8,
                armor_class: 3,
                damage: "%%%x0",
                is_mean: true,
            },
            MonsterKind::Griffin => MonsterStats {
                exp: 2000,
                level: 13,
                armor_class: 2,
                damage: "4x3/3x5",
                is_mean: true,
            },
            MonsterKind::Hobgoblin => MonsterStats {
                exp: 3,
                level: 1,
                armor_class: 5,
                damage: "1x8",
                is_mean: true,
            },
            MonsterKind::IceMonster => MonsterStats {
                exp: 5,
                level: 1,
                armor_class: 9,
                damage: "0x0",
                is_mean: false,
            },
            MonsterKind::Jabberwock => MonsterStats {
                exp: 3000,
                level: 15,
                armor_class: 6,
                damage: "2x12/2x4",
                is_mean: false,
            },
            MonsterKind::Kestrel => MonsterStats {
                exp: 1,
                level: 1,
                armor_class: 7,
                damage: "1x4",
                is_mean: true,
            },
            MonsterKind::Leprechaun => MonsterStats {
                exp: 10,
                level: 3,
                armor_class: 8,
                damage: "1x1",
                is_mean: false,
            },
            MonsterKind::Medusa => MonsterStats {
                exp: 200,
                level: 8,
                armor_class: 2,
                damage: "3x4/3x4/2x5",
                is_mean: true,
            },
            MonsterKind::Nymph => MonsterStats {
                exp: 37,
                level: 3,
                armor_class: 9,
                damage: "0x0",
                is_mean: false,
            },
            MonsterKind::Orc => MonsterStats {
                exp: 5,
                level: 1,
                armor_class: 6,
                damage: "1x8",
                is_mean: false,
            },
            MonsterKind::Phantom => MonsterStats {
                exp: 120,
                level: 8,
                armor_class: 3,
                damage: "4x4",
                is_mean: false,
            },
            MonsterKind::Quagga => MonsterStats {
                exp: 15,
                level: 3,
                armor_class: 3,
                damage: "1x5/1x5",
                is_mean: true,
            },
            MonsterKind::Rattlesnake => MonsterStats {
                exp: 9,
                level: 2,
                armor_class: 3,
                damage: "1x6",
                is_mean: true,
            },
            MonsterKind::Snake => MonsterStats {
                exp: 2,
                level: 1,
                armor_class: 5,
                damage: "1x3",
                is_mean: true,
            },
            MonsterKind::Troll => MonsterStats {
                exp: 120,
                level: 6,
                armor_class: 4,
                damage: "1x8/1x8/2x6",
                is_mean: true,
            },
            MonsterKind::BlackUnicorn => MonsterStats {
                exp: 190,
                level: 7,
                armor_class: -2,
                damage: "1x9/1x9/2x9",
                is_mean: true,
            },
            MonsterKind::Vampire => MonsterStats {
                exp: 350,
                level: 8,
                armor_class: 1,
                damage: "1x10",
                is_mean: true,
            },
            MonsterKind::Wraith => MonsterStats {
                exp: 55,
                level: 5,
                armor_class: 4,
                damage: "1x6",
                is_mean: false,
            },
            MonsterKind::Xeroc => MonsterStats {
                exp: 100,
                level: 7,
                armor_class: 7,
                damage: "4x4",
                is_mean: false,
            },
            MonsterKind::Yeti => MonsterStats {
                exp: 50,
                level: 4,
                armor_class: 6,
                damage: "1x6/1x6",
                is_mean: false,
            },
            MonsterKind::Zombie => MonsterStats {
                exp: 6,
                level: 2,
                armor_class: 8,
                damage: "1x8",
                is_mean: true,
            },
        }
    }

    /// A uniformly random kind. Depth-based selection can layer on top later.
    pub fn random(rng: &mut Rng) -> MonsterKind {
        MonsterKind::ALL[rng.below(MonsterKind::ALL.len())]
    }
}

/// A monster on the current level: which kind, where, its rolled hit points
/// and its spawn-scaled combat stats.
///
/// Everything static about the kind (name, letter, damage string, meanness)
/// comes from the canonical table via [`MonsterKind`]; the struct itself
/// carries the per-spawn rolls: hit points `roll(level, 8)` (report §6.2 —
/// the table's `hpt` column is an unused placeholder) and, on floors deeper
/// than 26, the level/AC/exp depth scaling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monster {
    pub kind: MonsterKind,
    pub x: usize,
    pub y: usize,
    pub hp: i32,
    /// Rolled at spawn: `roll(level, 8)`, not a table stat.
    pub max_hp: i32,
    /// The monster's attacker level: table level + depth scaling.
    pub level: u32,
    /// Scaled armor class (table armor − depth scaling).
    pub armor_class: i32,
    /// Current experience value, granted on kill: base + depth scaling.
    pub exp: i32,
    /// ISRUN: waking and chasing. Sleeping monsters stay put until the player
    /// sees them (ISMEAN, 2/3 chance) or attacks them (runto).
    pub running: bool,
}

impl Monster {
    /// Spawn `kind` at `(x, y)` on `floor` (`monsters.c:60 new_monster`):
    /// HP = `roll(level, 8)`, plus one notch of depth scaling per floor below
    /// the Amulet's level. All rolls come from `rng`, so the same seed
    /// reproduces the same monster.
    pub fn spawn(kind: MonsterKind, x: usize, y: usize, floor: u32, rng: &mut Rng) -> Monster {
        let base = kind.stats();
        let add = crate::combat::lev_add(floor);
        let level = base.level + add;
        let max_hp = crate::combat::roll(rng, level, 8);
        let exp = base.exp + (add * 10) as i32 + crate::combat::exp_add(level, max_hp);
        Monster {
            kind,
            x,
            y,
            hp: max_hp,
            max_hp,
            level,
            armor_class: base.armor_class - add as i32,
            exp,
            running: false,
        }
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    pub fn symbol(&self) -> char {
        self.kind.letter()
    }

    /// The scaled armor class used by combat.
    pub fn armor_class(&self) -> i32 {
        self.armor_class
    }

    /// The monster's damage string (`s_dmg`), parsed by combat's roll_em.
    pub fn damage_string(&self) -> &'static str {
        self.kind.stats().damage
    }

    /// The experience granted for killing this monster (already depth-scaled).
    pub fn exp_value(&self) -> i32 {
        self.exp
    }

    /// ISMEAN: wakes on sight and chases the player (report §7.2).
    pub fn is_mean(&self) -> bool {
        self.kind.stats().is_mean
    }
}

impl fmt::Display for Monster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({},{}), hp {}/{} lvl {} ac {} dmg {} xp {}",
            self.name(),
            self.x,
            self.y,
            self.hp,
            self.max_hp,
            self.level,
            self.armor_class,
            self.damage_string(),
            self.exp
        )
    }
}

/// Everything placed on one generated level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawn {
    pub player: Player,
    pub monsters: Vec<Monster>,
}

impl Spawn {
    /// One compact line per entity, used by the `--dump-map` demo and the
    /// determinism tests.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "{} at ({},{}), hp {}/{} str {} gold {} xp {}",
            self.player.symbol(),
            self.player.x,
            self.player.y,
            self.player.hp,
            self.player.max_hp,
            self.player.strength,
            self.player.gold,
            self.player.experience,
        );
        for m in &self.monsters {
            out.push_str(&format!("\n{m}"));
        }
        out
    }
}

/// Place the player and monsters on a generated level.
///
/// The player starts at the center of the first room. Each room has a
/// three-in-four chance of holding one monster (roughly mirroring Rogue's
/// per-room scatter); monsters land on room floor tiles only — never on the
/// player's tile and never on each other. All rolls come from `rng`, so the
/// same seed yields the same level *and* the same spawn (including rolled
/// hit points). Hand-built maps without recorded rooms get the player in the
/// top-left corner and no monsters.
pub fn populate(dungeon: &Dungeon, floor: u32, rng: &mut Rng) -> Spawn {
    let rooms = dungeon.rooms();
    let Some(room) = rooms.first() else {
        return Spawn {
            player: Player::new(1, 1),
            monsters: Vec::new(),
        };
    };
    let player = Player::new(room.center().0, room.center().1);

    let mut blocked: HashSet<(usize, usize)> = HashSet::from([(player.x, player.y)]);
    let mut monsters = Vec::new();
    for room in &rooms {
        if !rng.chance(3, 4) {
            continue;
        }
        // This room's unblocked floor tiles; pick one uniformly.
        let free: Vec<(usize, usize)> = (room.y0..=room.y1)
            .flat_map(|y| (room.x0..=room.x1).map(move |x| (x, y)))
            .filter(|&(x, y)| dungeon.tile(x, y) == Tile::Floor && !blocked.contains(&(x, y)))
            .collect();
        if free.is_empty() {
            continue;
        }
        let (x, y) = free[rng.below(free.len())];
        blocked.insert((x, y));
        monsters.push(Monster::spawn(MonsterKind::random(rng), x, y, floor, rng));
    }
    Spawn { player, monsters }
}

/// `populate` with a fresh RNG from `seed`, so a seed reproduces the whole
/// level including its inhabitants.
pub fn populate_seeded(dungeon: &Dungeon, floor: u32, seed: u64) -> Spawn {
    populate(dungeon, floor, &mut Rng::new(seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Dungeon;

    /// Enough seeds to shake out placement edge cases.
    const SEEDS: std::ops::Range<u64> = 0..300;

    #[test]
    fn player_starts_with_classic_stats() {
        let player = Player::new(3, 4);
        assert_eq!((player.x, player.y), (3, 4));
        assert_eq!(player.hp, PLAYER_START_HP);
        assert_eq!(player.max_hp, PLAYER_START_HP);
        assert_eq!(player.strength, PLAYER_START_STRENGTH);
        assert_eq!(player.gold, PLAYER_START_GOLD);
        assert_eq!(player.experience, PLAYER_START_EXPERIENCE);
        assert_eq!(player.level, PLAYER_START_LEVEL);
        assert!(player.running);
        assert_eq!(player.symbol(), '@');
    }

    #[test]
    fn monster_table_is_complete_a_to_z() {
        assert_eq!(MonsterKind::ALL.len(), 26);
        let letters: String = MonsterKind::ALL.iter().map(|k| k.letter()).collect();
        assert_eq!(letters, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");

        let names: HashSet<&str> = MonsterKind::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(names.len(), 26, "monster names must be unique");

        for kind in MonsterKind::ALL {
            assert_eq!(MonsterKind::from_letter(kind.letter()), Some(kind));
        }
        assert!(MonsterKind::from_letter('a').is_none());
        assert!(MonsterKind::from_letter('1').is_none());
    }

    #[test]
    fn monster_stats_are_sane() {
        for kind in MonsterKind::ALL {
            let s = kind.stats();
            assert!(s.exp >= 1, "{kind:?} gives no experience");
            assert!((1..=15).contains(&s.level), "{kind:?} level {}", s.level);
            assert!(
                (-2..=9).contains(&s.armor_class),
                "{kind:?} armor class {} out of range",
                s.armor_class
            );
            assert!(
                s.damage.contains('x'),
                "{kind:?} damage string {s:?} has no dice separator"
            );
        }
        // The iconic values from the canonical 5.4.4 table (report §6.1).
        assert_eq!(
            MonsterKind::Zombie.stats(),
            MonsterStats {
                exp: 6,
                level: 2,
                armor_class: 8,
                damage: "1x8",
                is_mean: true,
            }
        );
        assert_eq!(MonsterKind::Bat.stats().level, 1);
        assert_eq!(MonsterKind::Orc.stats().exp, 5);
        assert_eq!(MonsterKind::Orc.stats().armor_class, 6);
        assert_eq!(MonsterKind::Dragon.stats().armor_class, -1);
        assert_eq!(MonsterKind::Dragon.stats().damage, "1x8/1x8/3x10");
        assert_eq!(MonsterKind::Centaur.stats().damage, "1x2/1x5/1x5");
        assert_eq!(MonsterKind::Flytrap.stats().damage, "%%%x0");
        assert!(!MonsterKind::Orc.stats().is_mean, "orc is greedy, not mean");
        assert!(MonsterKind::Kestrel.stats().is_mean);
        // The dragon is the biggest bounty; the jabberwock the highest level;
        // the black unicorn the hardest to hit.
        let most_xp = MonsterKind::ALL
            .iter()
            .max_by_key(|k| k.stats().exp)
            .copied()
            .unwrap();
        assert_eq!(most_xp, MonsterKind::Dragon);
        let highest = MonsterKind::ALL
            .iter()
            .max_by_key(|k| k.stats().level)
            .copied()
            .unwrap();
        assert_eq!(highest, MonsterKind::Jabberwock);
        let lowest_arm = MonsterKind::ALL
            .iter()
            .min_by_key(|k| k.stats().armor_class)
            .copied()
            .unwrap();
        assert_eq!(lowest_arm, MonsterKind::BlackUnicorn);
    }

    #[test]
    fn monster_construction_spawns_with_rolled_hp() {
        let mut rng = Rng::new(13);
        let monster = Monster::spawn(MonsterKind::Kestrel, 2, 3, 1, &mut rng);
        assert_eq!((monster.x, monster.y), (2, 3));
        assert_eq!(monster.max_hp, monster.hp);
        assert!((1..=8).contains(&monster.hp), "hp = roll(1,8)");
        assert_eq!(monster.name(), "kestrel");
        assert_eq!(monster.symbol(), 'K');
        assert_eq!(monster.armor_class(), 7);
        assert_eq!(monster.damage_string(), "1x4");
        assert_eq!(monster.level, 1);
        assert!(monster.exp_value() >= 1);
        assert!(!monster.running, "freshly spawned monsters are asleep");
        assert!(monster.is_mean(), "kestrel is ISMEAN");
    }

    /// In-bounds, on room floor tiles, never the player's tile, never each
    /// other's, for every seed.
    #[test]
    fn spawn_is_valid_across_seeds() {
        for seed in SEEDS {
            let dungeon = Dungeon::generate(seed);
            let spawn = populate_seeded(&dungeon, 1, seed);

            let rooms = dungeon.rooms();
            let player_room = rooms[0];
            assert!(
                (player_room.x0..=player_room.x1).contains(&spawn.player.x)
                    && (player_room.y0..=player_room.y1).contains(&spawn.player.y),
                "seed {seed}: player not inside the first room"
            );
            assert_eq!(
                dungeon.tile(spawn.player.x, spawn.player.y),
                Tile::Floor,
                "seed {seed}: player not on a floor tile"
            );

            let mut seen: HashSet<(usize, usize)> =
                HashSet::from([(spawn.player.x, spawn.player.y)]);
            for monster in &spawn.monsters {
                assert!(
                    monster.x < dungeon.width && monster.y < dungeon.height,
                    "seed {seed}: {monster:?} out of bounds"
                );
                assert_eq!(
                    dungeon.tile(monster.x, monster.y),
                    Tile::Floor,
                    "seed {seed}: {monster:?} not on a floor tile"
                );
                assert!(
                    rooms.iter().any(|r| (r.x0..=r.x1).contains(&monster.x)
                        && (r.y0..=r.y1).contains(&monster.y)),
                    "seed {seed}: {monster:?} not inside a room"
                );
                assert!(
                    seen.insert((monster.x, monster.y)),
                    "seed {seed}: duplicate position {monster:?}"
                );
                assert_eq!(monster.max_hp, monster.hp, "spawned monsters start full");
            }
        }
    }

    #[test]
    fn spawn_is_deterministic_per_seed() {
        for seed in [1, 7, 42, 1234] {
            let dungeon = Dungeon::generate(seed);
            let first = populate_seeded(&dungeon, 1, seed);
            let second = populate_seeded(&dungeon, 1, seed);
            assert_eq!(first, second, "seed {seed} must reproduce its spawn");
        }
    }

    #[test]
    fn different_seeds_place_differently() {
        let summaries: HashSet<String> = (0..20)
            .map(|seed| populate_seeded(&Dungeon::generate(seed), 1, seed).summary())
            .collect();
        assert!(summaries.len() > 1, "expected different seeds to differ");
    }

    #[test]
    fn populate_without_rooms_does_not_panic() {
        let dungeon = Dungeon::from_tiles(
            4,
            3,
            &[Tile::Floor; 12],
        );
        let spawn = populate_seeded(&dungeon, 1, 1);
        assert_eq!(spawn.player.x, 1);
        assert_eq!(spawn.player.y, 1);
        assert!(spawn.monsters.is_empty());
    }
}
