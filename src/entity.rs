//! Entities: the player, monsters, and the classic Rogue monster table.
//!
//! Data model only. Combat math is deliberately absent — a separate combat
//! report will define the formulas — so everything here is plain data with a
//! small, documented API for the game loop and combat modules to build on.
//!
//! Monsters follow the classic Rogue monster table from the original 1980
//! game (*Rogue: Exploring the Dungeons of Doom*, Toy/Wichman/Arnold): all 26
//! monsters, one per letter A–Z from Aquator to Zombie, with the canonical
//! hit points, armor class, damage dice and experience values (see
//! [`MonsterKind::stats`]). Positions are `(x, y)` map coordinates (column,
//! row), matching the map module.

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
        }
    }

    /// The player's map glyph.
    pub fn symbol(&self) -> char {
        '@'
    }
}

/// Damage dice in classic Rogue "AxB" notation: `dice` rolls of a `sides`-sided
/// die (`1x4` is 1d4; `0x0` deals nothing). Pure data — the roll formula
/// belongs to the combat module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Damage {
    pub dice: u32,
    pub sides: u32,
}

impl Damage {
    pub const fn new(dice: u32, sides: u32) -> Damage {
        Damage { dice, sides }
    }

    /// No damage at all ("0x0"), e.g. the fungus, which paralyses instead.
    pub const fn none() -> Damage {
        Damage::new(0, 0)
    }
}

impl fmt::Display for Damage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.dice, self.sides)
    }
}

/// The canonical, static stats of one monster kind, straight from the classic
/// Rogue monster table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterStats {
    /// Base hit points, also the max a freshly spawned monster starts with.
    pub hit_points: i32,
    /// Armor class. In the original Rogue this is the number an attacker must
    /// beat to land a blow; the combat module owns the exact formula.
    pub armor_class: i32,
    /// Damage dice, e.g. `1x4` for 1d4.
    pub damage: Damage,
    /// Experience awarded for killing it.
    pub exp: i32,
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
    Fungus,
    Giant,
    Hobgoblin,
    IceMonster,
    Jabberwock,
    Kobold,
    Leprechaun,
    Medusa,
    Naga,
    Orc,
    Phantom,
    Quagga,
    Rat,
    Snake,
    Troll,
    UrVile,
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
        MonsterKind::Fungus,
        MonsterKind::Giant,
        MonsterKind::Hobgoblin,
        MonsterKind::IceMonster,
        MonsterKind::Jabberwock,
        MonsterKind::Kobold,
        MonsterKind::Leprechaun,
        MonsterKind::Medusa,
        MonsterKind::Naga,
        MonsterKind::Orc,
        MonsterKind::Phantom,
        MonsterKind::Quagga,
        MonsterKind::Rat,
        MonsterKind::Snake,
        MonsterKind::Troll,
        MonsterKind::UrVile,
        MonsterKind::Vampire,
        MonsterKind::Wraith,
        MonsterKind::Xeroc,
        MonsterKind::Yeti,
        MonsterKind::Zombie,
    ];

    /// The kind whose letter is `c`, e.g. `from_letter('K') == Some(Kobold)`.
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

    /// The classic lowercase name, e.g. "ur-vile".
    pub fn name(self) -> &'static str {
        match self {
            MonsterKind::Aquator => "aquator",
            MonsterKind::Bat => "bat",
            MonsterKind::Centaur => "centaur",
            MonsterKind::Dragon => "dragon",
            MonsterKind::Emu => "emu",
            MonsterKind::Fungus => "fungus",
            MonsterKind::Giant => "giant",
            MonsterKind::Hobgoblin => "hobgoblin",
            MonsterKind::IceMonster => "ice monster",
            MonsterKind::Jabberwock => "jabberwock",
            MonsterKind::Kobold => "kobold",
            MonsterKind::Leprechaun => "leprechaun",
            MonsterKind::Medusa => "medusa",
            MonsterKind::Naga => "naga",
            MonsterKind::Orc => "orc",
            MonsterKind::Phantom => "phantom",
            MonsterKind::Quagga => "quagga",
            MonsterKind::Rat => "rat",
            MonsterKind::Snake => "snake",
            MonsterKind::Troll => "troll",
            MonsterKind::UrVile => "ur-vile",
            MonsterKind::Vampire => "vampire",
            MonsterKind::Wraith => "wraith",
            MonsterKind::Xeroc => "xeroc",
            MonsterKind::Yeti => "yeti",
            MonsterKind::Zombie => "zombie",
        }
    }

    /// The canonical stats from the classic 1980 Rogue monster table.
    pub fn stats(self) -> MonsterStats {
        match self {
            MonsterKind::Aquator => MonsterStats {
                hit_points: 8,
                armor_class: 4,
                damage: Damage::new(1, 2),
                exp: 4,
            },
            MonsterKind::Bat => MonsterStats {
                hit_points: 1,
                armor_class: 1,
                damage: Damage::new(1, 2),
                exp: 1,
            },
            MonsterKind::Centaur => MonsterStats {
                hit_points: 9,
                armor_class: 6,
                damage: Damage::new(1, 4),
                exp: 2,
            },
            MonsterKind::Dragon => MonsterStats {
                hit_points: 16,
                armor_class: 10,
                damage: Damage::new(3, 4),
                exp: 8,
            },
            MonsterKind::Emu => MonsterStats {
                hit_points: 5,
                armor_class: 4,
                damage: Damage::new(1, 2),
                exp: 1,
            },
            MonsterKind::Fungus => MonsterStats {
                hit_points: 7,
                armor_class: 2,
                damage: Damage::none(),
                exp: 1,
            },
            MonsterKind::Giant => MonsterStats {
                hit_points: 12,
                armor_class: 8,
                damage: Damage::new(2, 4),
                exp: 4,
            },
            MonsterKind::Hobgoblin => MonsterStats {
                hit_points: 3,
                armor_class: 4,
                damage: Damage::new(1, 8),
                exp: 1,
            },
            MonsterKind::IceMonster => MonsterStats {
                hit_points: 6,
                armor_class: 6,
                damage: Damage::new(0, 1),
                exp: 2,
            },
            MonsterKind::Jabberwock => MonsterStats {
                hit_points: 13,
                armor_class: 10,
                damage: Damage::new(2, 10),
                exp: 7,
            },
            MonsterKind::Kobold => MonsterStats {
                hit_points: 4,
                armor_class: 4,
                damage: Damage::new(1, 4),
                exp: 1,
            },
            MonsterKind::Leprechaun => MonsterStats {
                hit_points: 8,
                armor_class: 8,
                damage: Damage::new(1, 4),
                exp: 3,
            },
            MonsterKind::Medusa => MonsterStats {
                hit_points: 9,
                armor_class: 9,
                damage: Damage::new(2, 4),
                exp: 3,
            },
            MonsterKind::Naga => MonsterStats {
                hit_points: 9,
                armor_class: 6,
                damage: Damage::new(1, 4),
                exp: 2,
            },
            MonsterKind::Orc => MonsterStats {
                hit_points: 5,
                armor_class: 5,
                damage: Damage::new(1, 8),
                exp: 2,
            },
            MonsterKind::Phantom => MonsterStats {
                hit_points: 7,
                armor_class: 6,
                damage: Damage::new(1, 4),
                exp: 2,
            },
            MonsterKind::Quagga => MonsterStats {
                hit_points: 6,
                armor_class: 6,
                damage: Damage::new(2, 4),
                exp: 2,
            },
            MonsterKind::Rat => MonsterStats {
                hit_points: 1,
                armor_class: 4,
                damage: Damage::new(1, 4),
                exp: 1,
            },
            MonsterKind::Snake => MonsterStats {
                hit_points: 4,
                armor_class: 4,
                damage: Damage::new(1, 4),
                exp: 2,
            },
            MonsterKind::Troll => MonsterStats {
                hit_points: 11,
                armor_class: 8,
                damage: Damage::new(2, 4),
                exp: 3,
            },
            MonsterKind::UrVile => MonsterStats {
                hit_points: 13,
                armor_class: 8,
                damage: Damage::new(2, 4),
                exp: 6,
            },
            MonsterKind::Vampire => MonsterStats {
                hit_points: 12,
                armor_class: 10,
                damage: Damage::new(1, 8),
                exp: 6,
            },
            MonsterKind::Wraith => MonsterStats {
                hit_points: 13,
                armor_class: 10,
                damage: Damage::new(2, 4),
                exp: 6,
            },
            MonsterKind::Xeroc => MonsterStats {
                hit_points: 9,
                armor_class: 7,
                damage: Damage::new(1, 4),
                exp: 3,
            },
            MonsterKind::Yeti => MonsterStats {
                hit_points: 10,
                armor_class: 8,
                damage: Damage::new(2, 4),
                exp: 3,
            },
            MonsterKind::Zombie => MonsterStats {
                hit_points: 6,
                armor_class: 6,
                damage: Damage::new(1, 8),
                exp: 2,
            },
        }
    }

    /// A uniformly random kind. Depth-based selection can layer on top later.
    pub fn random(rng: &mut Rng) -> MonsterKind {
        MonsterKind::ALL[rng.below(MonsterKind::ALL.len())]
    }
}

/// A monster on the current level: which kind, where, and how hurt it is.
///
/// Everything static about the kind (name, letter, armor class, damage,
/// experience) comes from the canonical table via [`MonsterKind`]; the struct
/// itself tracks identity, position and current hit points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monster {
    pub kind: MonsterKind,
    pub x: usize,
    pub y: usize,
    pub hp: i32,
}

impl Monster {
    /// Spawn `kind` at `(x, y)` at full hit points.
    pub fn new(kind: MonsterKind, x: usize, y: usize) -> Monster {
        Monster {
            kind,
            x,
            y,
            hp: kind.stats().hit_points,
        }
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    pub fn symbol(&self) -> char {
        self.kind.letter()
    }

    pub fn armor_class(&self) -> i32 {
        self.kind.stats().armor_class
    }

    pub fn damage(&self) -> Damage {
        self.kind.stats().damage
    }

    pub fn exp_value(&self) -> i32 {
        self.kind.stats().exp
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
            out.push_str(&format!(
                "\n{} {} at ({},{}), hp {}/{} ac {} dmg {} xp {}",
                m.symbol(),
                m.name(),
                m.x,
                m.y,
                m.hp,
                m.kind.stats().hit_points,
                m.armor_class(),
                m.damage(),
                m.exp_value(),
            ));
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
/// same seed yields the same level *and* the same spawn.
pub fn populate(dungeon: &Dungeon, rng: &mut Rng) -> Spawn {
    let rooms = dungeon.rooms();
    let room = *rooms.first().expect("a generated dungeon always has rooms");
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
        monsters.push(Monster::new(MonsterKind::random(rng), x, y));
    }
    Spawn { player, monsters }
}

/// `populate` with a fresh RNG from `seed`, so a seed reproduces the whole
/// level including its inhabitants.
pub fn populate_seeded(dungeon: &Dungeon, seed: u64) -> Spawn {
    populate(dungeon, &mut Rng::new(seed))
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
            assert!(s.hit_points >= 1, "{kind:?} has no hit points");
            assert!(s.exp >= 1, "{kind:?} gives no experience");
            assert!(
                (0..=12).contains(&s.armor_class),
                "{kind:?} armor class {} out of range",
                s.armor_class
            );
            assert!(
                s.damage.dice <= 4 && s.damage.sides <= 10,
                "{kind:?} damage {s:?} out of range"
            );
        }
        // The iconic values everyone quotes from the classic table.
        assert_eq!(
            MonsterKind::Zombie.stats(),
            MonsterStats {
                hit_points: 6,
                armor_class: 6,
                damage: Damage::new(1, 8),
                exp: 2,
            }
        );
        assert_eq!(MonsterKind::Bat.stats().hit_points, 1);
        assert_eq!(MonsterKind::Rat.stats().hit_points, 1);
        assert_eq!(MonsterKind::Dragon.stats().damage, Damage::new(3, 4));
        assert_eq!(MonsterKind::Fungus.stats().damage, Damage::none());
        // Dragon is the meanest; bat and rat are the feeblest.
        let toughest = MonsterKind::ALL
            .iter()
            .max_by_key(|k| k.stats().hit_points)
            .copied()
            .unwrap();
        assert_eq!(toughest, MonsterKind::Dragon);
        let most_xp = MonsterKind::ALL
            .iter()
            .max_by_key(|k| k.stats().exp)
            .copied()
            .unwrap();
        assert_eq!(most_xp, MonsterKind::Dragon);
        assert_eq!(MonsterKind::Bat.stats().exp, 1);
    }

    #[test]
    fn monster_construction_uses_kind_stats() {
        let monster = Monster::new(MonsterKind::Kobold, 2, 3);
        assert_eq!((monster.x, monster.y), (2, 3));
        assert_eq!(monster.hp, 4);
        assert_eq!(monster.name(), "kobold");
        assert_eq!(monster.symbol(), 'K');
        assert_eq!(monster.armor_class(), 4);
        assert_eq!(monster.damage(), Damage::new(1, 4));
        assert_eq!(monster.exp_value(), 1);
    }

    /// In-bounds, on room floor tiles, never the player's tile, never each
    /// other's, for every seed.
    #[test]
    fn spawn_is_valid_across_seeds() {
        for seed in SEEDS {
            let dungeon = Dungeon::generate(seed);
            let spawn = populate_seeded(&dungeon, seed);

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
            }
        }
    }

    #[test]
    fn spawn_is_deterministic_per_seed() {
        for seed in [1, 7, 42, 1234] {
            let dungeon = Dungeon::generate(seed);
            let first = populate_seeded(&dungeon, seed);
            let second = populate_seeded(&dungeon, seed);
            assert_eq!(first, second, "seed {seed} must reproduce its spawn");
        }
    }

    #[test]
    fn different_seeds_place_differently() {
        let summaries: HashSet<String> = (0..20)
            .map(|seed| populate_seeded(&Dungeon::generate(seed), seed).summary())
            .collect();
        assert!(summaries.len() > 1, "expected different seeds to differ");
    }
}
