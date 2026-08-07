//! Dungeon level progression: floor constants, staircases, and the Amulet
//! of Yendor.
//!
//! Pure bookkeeping and placement — no rendering, no input. The game loop in
//! `game.rs` drives descents and ascents through these helpers; the combat
//! module reads the floor number (via the spawner and `Game::floor`) for
//! monster depth scaling, and extends the `Level: N ...` status line.

use crate::map::{Dungeon, Room, Tile};
use crate::rng::Rng;

/// How many floors the dungeon has: 1 (the surface entrance) through 26
/// (the Amulet floor). The floor counter must stay in `1..=MAX_FLOOR`.
pub const MAX_FLOOR: u32 = 26;

/// The floor the Amulet of Yendor lies on (Rogue 5.4.4 `rogue.h`:
/// `#define AMULETLEVEL 26`; the combat research report cites it in §6.2).
pub const AMULET_LEVEL: u32 = 26;

/// Both staircases of one floor, as map coordinates `(x, y)`.
///
/// Every floor carries a down stair (deeper dungeon) and an up stair. Floor
/// 1's up stair is the surface exit: stepping on it with the Amulet wins the
/// game, without it the way out is magically blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stairs {
    pub up: (usize, usize),
    pub down: (usize, usize),
}

/// The Amulet of Yendor while it lies on the floor of [`AMULET_LEVEL`].
///
/// `Game` tracks whether the player carries it; a simple carried flag until a
/// real inventory lands. Glyph: `,` (Rogue 5.4.4 `#define AMULET ','`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Amulet {
    pub x: usize,
    pub y: usize,
}

/// Derive the map seed for one floor from the game's base seed.
///
/// Floor 1 keeps the base seed untouched, so `--seed N` still reproduces the
/// opening level exactly. Deeper floors mix the floor number into the seed;
/// the same `(seed, floor)` pair always yields the same level. That is what
/// makes re-ascending deterministic: going back up regenerates the floor
/// above identically, monsters and stairs included.
pub fn floor_seed(base: u64, floor: u32) -> u64 {
    assert!(
        (1..=MAX_FLOOR).contains(&floor),
        "floor {floor} out of range (1..={MAX_FLOOR})"
    );
    if floor == 1 {
        return base;
    }
    Rng::new(base ^ floor as u64).next_u64()
}

/// The win predicate: the player escapes by stepping onto floor 1's exit
/// stair *while carrying* the Amulet of Yendor.
pub fn has_escaped(floor: u32, at_exit: bool, has_amulet: bool) -> bool {
    floor == 1 && at_exit && has_amulet
}

/// Place both staircases of a floor: each on a floor tile inside a room,
/// never on the tile `avoid` (the player's spawn) and never on each other.
///
/// Rooms are always mutually reachable (see `map.rs`), so stair placement
/// anywhere in a room is automatically reachable by the player. Roomless
/// fixture maps (unit tests) fall back to any walkable tile.
pub fn place_stairs(dungeon: &Dungeon, rng: &mut Rng, avoid: (usize, usize)) -> Stairs {
    let rooms = dungeon.rooms();
    if rooms.is_empty() {
        let down = pick_walkable(dungeon, rng, &[avoid]);
        let up = pick_walkable(dungeon, rng, &[avoid, down]);
        return Stairs { up, down };
    }

    let down_room = rooms[rng.below(rooms.len())];
    let down = floor_in_room(dungeon, down_room, rng, &[avoid]);
    // Put the up stair in a different room when there is one, so the two
    // ends of a floor read clearly.
    let up_room = other_room(&rooms, rng, down_room);
    let up = floor_in_room(dungeon, up_room, rng, &[avoid, down]);
    Stairs { up, down }
}

/// Place the Amulet on the floor: a floor tile in a room, distinct from both
/// staircases and the tile `avoid` (the player's spawn).
///
/// Only [`AMULET_LEVEL`] gets one — the caller decides when to call this —
/// and generated maps always have rooms, so no roomless fallback is needed.
pub fn place_amulet(dungeon: &Dungeon, rng: &mut Rng, stairs: Stairs, avoid: (usize, usize)) -> Amulet {
    let rooms = dungeon.rooms();
    let room = rooms[rng.below(rooms.len())];
    let free: Vec<(usize, usize)> = (room.y0..=room.y1)
        .flat_map(|y| (room.x0..=room.x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            dungeon.tile(x, y) == Tile::Floor
                && (x, y) != avoid
                && (x, y) != stairs.up
                && (x, y) != stairs.down
        })
        .collect();
    let (x, y) = free[rng.below(free.len())];
    Amulet { x, y }
}

/// A uniformly random tile in `room` that is a floor tile and not one of the
/// tiles in `avoid`.
///
/// Rooms are at least 4x2 (`map.rs`), so after removing a couple of tiles
/// there is always something left to pick.
fn floor_in_room(
    dungeon: &Dungeon,
    room: Room,
    rng: &mut Rng,
    avoid: &[(usize, usize)],
) -> (usize, usize) {
    let free: Vec<(usize, usize)> = (room.y0..=room.y1)
        .flat_map(|y| (room.x0..=room.x1).map(move |x| (x, y)))
        .filter(|&(x, y)| dungeon.tile(x, y) == Tile::Floor && !avoid.contains(&(x, y)))
        .collect();
    free[rng.below(free.len())]
}

/// A room other than `except`, for maps with two or more rooms.
fn other_room(rooms: &[Room], rng: &mut Rng, except: Room) -> Room {
    if rooms.len() == 1 {
        return rooms[0];
    }
    loop {
        let room = rooms[rng.below(rooms.len())];
        if room != except {
            return room;
        }
    }
}

/// A uniformly random walkable tile, for roomless fixture maps.
fn pick_walkable(dungeon: &Dungeon, rng: &mut Rng, avoid: &[(usize, usize)]) -> (usize, usize) {
    let mut tiles = Vec::new();
    for y in 0..dungeon.height {
        for x in 0..dungeon.width {
            if dungeon.tile(x, y).is_walkable() && !avoid.contains(&(x, y)) {
                tiles.push((x, y));
            }
        }
    }
    tiles[rng.below(tiles.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity;
    use std::collections::VecDeque;

    /// Enough seeds to shake out placement edge cases.
    const SEEDS: std::ops::Range<u64> = 0..300;

    /// Is `target` reachable from `start` over walkable tiles? (The stairs
    /// themselves sit on walkable tiles, so a plain BFS over the map is the
    /// honest reachability check.)
    fn reachable(map: &Dungeon, start: (usize, usize), target: (usize, usize)) -> bool {
        let mut seen = vec![false; map.width * map.height];
        let mut queue = VecDeque::from([start]);
        seen[start.1 * map.width + start.0] = true;
        while let Some((x, y)) = queue.pop_front() {
            if (x, y) == target {
                return true;
            }
            for (nx, ny) in [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ] {
                if nx >= map.width || ny >= map.height {
                    continue;
                }
                let idx = ny * map.width + nx;
                if !seen[idx] && map.tile(nx, ny).is_walkable() {
                    seen[idx] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
        false
    }

    /// A freshly assembled level for `floor`: map, spawn, and stairs exactly
    /// as the game would build them.
    fn sample_floor(seed: u64, floor: u32) -> (Dungeon, entity::Spawn, Stairs) {
        let map = Dungeon::generate(floor_seed(seed, floor));
        let mut rng = Rng::new(floor_seed(seed, floor));
        let spawn = entity::populate(&map, floor, &mut rng);
        let stairs = place_stairs(&map, &mut rng, (spawn.player.x, spawn.player.y));
        (map, spawn, stairs)
    }

    #[test]
    fn floor_seed_keeps_floor_one_and_diverges_deeper() {
        assert_eq!(floor_seed(7, 1), 7);
        assert_eq!(floor_seed(42, 1), 42);
        assert_ne!(floor_seed(7, 2), floor_seed(7, 3));
        assert_eq!(floor_seed(7, 2), floor_seed(7, 2), "must be deterministic");
        assert_ne!(floor_seed(7, 2), floor_seed(8, 2), "base seed must matter");
    }

    #[test]
    #[should_panic]
    fn floor_seed_rejects_out_of_range_floors() {
        let _ = floor_seed(1, 0);
    }

    #[test]
    fn escape_requires_floor_one_exit_and_amulet() {
        assert!(has_escaped(1, true, true));
        assert!(!has_escaped(1, true, false), "no amulet, no escape");
        assert!(!has_escaped(1, false, true), "not on the exit stair");
        assert!(!has_escaped(2, true, true), "only floor 1 is the exit");
    }

    #[test]
    fn stairs_sit_on_room_floor_tiles_distinct_and_reachable() {
        for seed in SEEDS {
            let (map, spawn, stairs) = sample_floor(seed, 1);
            let rooms = map.rooms();
            let start = (spawn.player.x, spawn.player.y);

            assert_ne!(stairs.up, stairs.down, "seed {seed}: stairs overlap");
            assert_ne!(stairs.up, start, "seed {seed}: up stair on the player");
            assert_ne!(stairs.down, start, "seed {seed}: down stair on the player");

            for pos in [stairs.up, stairs.down] {
                assert!(
                    pos.0 < map.width && pos.1 < map.height,
                    "seed {seed}: stairs at {pos:?} out of bounds"
                );
                assert_eq!(
                    map.tile(pos.0, pos.1),
                    Tile::Floor,
                    "seed {seed}: stairs at {pos:?} not on a floor tile"
                );
                assert!(
                    rooms
                        .iter()
                        .any(|r| (r.x0..=r.x1).contains(&pos.0) && (r.y0..=r.y1).contains(&pos.1)),
                    "seed {seed}: stairs at {pos:?} not inside a room"
                );
                assert!(
                    reachable(&map, start, pos),
                    "seed {seed}: stairs at {pos:?} unreachable from {start:?}"
                );
            }
        }
    }

    #[test]
    fn amulet_lands_on_a_room_floor_tile_clear_of_stairs_and_player() {
        for seed in SEEDS {
            let (map, spawn, stairs) = sample_floor(seed, AMULET_LEVEL);
            let mut rng = Rng::new(floor_seed(seed, AMULET_LEVEL));
            let amulet = place_amulet(&map, &mut rng, stairs, (spawn.player.x, spawn.player.y));
            let rooms = map.rooms();

            assert_eq!(
                map.tile(amulet.x, amulet.y),
                Tile::Floor,
                "seed {seed}: amulet at ({},{}) not on a floor tile",
                amulet.x,
                amulet.y
            );
            assert!(
                rooms
                    .iter()
                    .any(|r| (r.x0..=r.x1).contains(&amulet.x) && (r.y0..=r.y1).contains(&amulet.y)),
                "seed {seed}: amulet at ({},{}) not inside a room",
                amulet.x,
                amulet.y
            );
            assert_ne!(
                (amulet.x, amulet.y),
                (spawn.player.x, spawn.player.y),
                "seed {seed}: amulet on the player"
            );
            assert_ne!((amulet.x, amulet.y), stairs.up, "seed {seed}: amulet on the up stair");
            assert_ne!((amulet.x, amulet.y), stairs.down, "seed {seed}: amulet on the down stair");
        }
    }

    #[test]
    fn stairs_are_deterministic_per_seed_and_floor() {
        for seed in [1, 7, 42, 1234] {
            for floor in [1, 13, AMULET_LEVEL] {
                let (map_a, spawn_a, stairs_a) = sample_floor(seed, floor);
                let (map_b, spawn_b, stairs_b) = sample_floor(seed, floor);
                assert_eq!(stairs_a, stairs_b, "seed {seed} floor {floor}");
                assert_eq!(map_a.render(), map_b.render());
                assert_eq!(spawn_a, spawn_b);
            }
        }
    }
}
