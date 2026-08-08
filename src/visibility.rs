//! Field of view / exploration visibility, classic-Rogue style.
//!
//! The player sees the room they stand in — its floor, its wall ring, and
//! its doors — and, out in a corridor, a short line-of-sight reach with
//! walls and closed doors blocking. Everything else stays dark rock until it
//! is explored. `Game` owns the per-floor `seen` grid (exploration memory,
//! restored when a floor is re-entered) and the per-turn `visible` set; this
//! module computes the latter from the map and the player's position.

use crate::combat::SIGHT_RANGE;
use crate::map::{Dungeon, Tile};

/// The tiles currently visible from `(px, py)`, row-major over the map.
///
/// In a room the whole room is lit: the interior floor, the surrounding wall
/// ring, and the doors in it. In a corridor the player sees a short
/// line-of-sight reach ([`SIGHT_RANGE`]) — walls and closed doors block, so
/// a bend hides what is around the corner. The player's own tile is always
/// lit.
pub fn compute(map: &Dungeon, px: usize, py: usize) -> Vec<bool> {
    let mut visible = vec![false; map.width * map.height];
    match room_bounds(map, px, py) {
        Some((x0, y0, x1, y1)) => {
            // The room interior plus its one-tile wall ring (walls and
            // doors). Corridors never run inside the ring, so nothing
            // beyond the room leaks in.
            let (x0, y0) = (x0.saturating_sub(1), y0.saturating_sub(1));
            let (x1, y1) = ((x1 + 1).min(map.width - 1), (y1 + 1).min(map.height - 1));
            for y in y0..=y1 {
                for x in x0..=x1 {
                    visible[y * map.width + x] = true;
                }
            }
        }
        None => corridor_view(map, px, py, &mut visible),
    }
    visible
}

/// The room interior the player stands in, as an inclusive bounding box, or
/// `None` out in a corridor.
///
/// A room is a maximal region of connected floor tiles — corridors are a
/// distinct tile kind, so they never merge into a room. Standing on a door
/// counts as standing in the room it opens into, so the room lights up as
/// the player steps through a doorway.
fn room_bounds(map: &Dungeon, px: usize, py: usize) -> Option<(usize, usize, usize, usize)> {
    let w = map.width;
    let h = map.height;
    let start = match map.tile(px, py) {
        Tile::Floor => (px, py),
        Tile::Door => {
            let mut found = None;
            for (nx, ny) in neighbors(px, py) {
                if nx < w && ny < h && map.tile(nx, ny) == Tile::Floor {
                    found = Some((nx, ny));
                    break;
                }
            }
            found?
        }
        _ => return None,
    };

    let mut region = vec![false; w * h];
    let mut stack = vec![start];
    region[start.1 * w + start.0] = true;
    while let Some((x, y)) = stack.pop() {
        for (nx, ny) in neighbors(x, y) {
            if nx < w && ny < h && !region[ny * w + nx] && map.tile(nx, ny) == Tile::Floor {
                region[ny * w + nx] = true;
                stack.push((nx, ny));
            }
        }
    }

    let mut x0 = usize::MAX;
    let mut y0 = usize::MAX;
    let mut x1 = 0;
    let mut y1 = 0;
    for y in 0..h {
        for x in 0..w {
            if region[y * w + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    Some((x0, y0, x1, y1))
}

/// A short line-of-sight reach down the corridor: every tile within
/// [`SIGHT_RANGE`] whose line from the player crosses only open floor or
/// corridor. Walls and closed doors stop the line, so a bend hides what is
/// behind it; a wall or door *tile* itself is a valid target, which is how
/// the player sees the walls beside them and the door at a corridor's end.
fn corridor_view(map: &Dungeon, px: usize, py: usize, visible: &mut [bool]) {
    let w = map.width as i32;
    let h = map.height as i32;
    let r = SIGHT_RANGE as i32;
    visible[py * map.width + px] = true;
    for dy in -r..=r {
        for dx in -r..=r {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (tx, ty) = (px as i32 + dx, py as i32 + dy);
            if tx < 0 || ty < 0 || tx >= w || ty >= h {
                continue;
            }
            if line_of_sight(map, px as i32, py as i32, tx, ty) {
                visible[ty as usize * map.width + tx as usize] = true;
            }
        }
    }
}

/// Bresenham line from `(x0, y0)` to `(x1, y1)`: the target is visible when
/// every tile between it and the player is open floor or corridor.
fn line_of_sight(map: &Dungeon, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        if (x, y) == (x1, y1) {
            return true;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
        if (x, y) == (x1, y1) {
            return true;
        }
        if !is_open(map, x, y) {
            return false;
        }
    }
}

/// Open floor or corridor: light passes through; walls and closed doors
/// (which sit in walls) stop it.
fn is_open(map: &Dungeon, x: i32, y: i32) -> bool {
    matches!(map.tile(x as usize, y as usize), Tile::Floor | Tile::Corridor)
}

/// The four orthogonal neighbors of `(x, y)`, with west/north wrapping so
/// callers can check bounds themselves.
fn neighbors(x: usize, y: usize) -> [(usize, usize); 4] {
    [
        (x.wrapping_sub(1), y),
        (x + 1, y),
        (x, y.wrapping_sub(1)),
        (x, y + 1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Two rooms joined by a corridor through doors:
    /// room A interior (1..6, 1..5), room B interior (16..21, 1..5), doors
    /// at (7, 5) and (15, 5), corridor row 5 from (8, 5) to (14, 5).
    const TWO_ROOMS: [&str; 7] = [
        "#######################",
        "#......#########......#",
        "#......#########......#",
        "#......#########......#",
        "#......#########......#",
        "#......+~~~~~~~+......#",
        "#######################",
    ];

    fn seen(visible: &[bool], map: &Dungeon, x: usize, y: usize) -> bool {
        visible[y * map.width + x]
    }

    #[test]
    fn standing_in_a_room_reveals_the_whole_room() {
        let map = map_from(&TWO_ROOMS);
        let visible = compute(&map, 3, 3);
        // The interior is lit, every tile of it.
        for y in 1..=5 {
            for x in 1..=6 {
                assert!(seen(&visible, &map, x, y), "({x},{y}) is room A floor");
            }
        }
        // The wall ring and the door are lit too.
        assert!(seen(&visible, &map, 7, 5), "the door is lit from inside");
        assert!(seen(&visible, &map, 7, 1), "the wall ring is lit");
        assert!(seen(&visible, &map, 0, 3), "the far wall is lit");
        // Room B, the corridor, and the rock beyond stay dark.
        assert!(!seen(&visible, &map, 16, 3), "room B is dark");
        assert!(!seen(&visible, &map, 11, 5), "the corridor is dark from a room");
        assert!(!seen(&visible, &map, 20, 1), "rock beyond is dark");
    }

    #[test]
    fn corridor_reveals_los_reach_and_walls_block() {
        let map = map_from(&TWO_ROOMS);
        let visible = compute(&map, 11, 5);
        // The corridor is lit along its straight reach.
        assert!(seen(&visible, &map, 11, 5), "the player's tile");
        assert!(seen(&visible, &map, 8, 5));
        assert!(seen(&visible, &map, 14, 5));
        // Both doors are visible down the corridor.
        assert!(seen(&visible, &map, 7, 5), "room A's door is visible down the reach");
        assert!(seen(&visible, &map, 15, 5), "room B's door is visible");
        // Rooms beyond the doors stay dark: walls and closed doors block.
        assert!(!seen(&visible, &map, 5, 5), "room A's floor is blocked by its door");
        assert!(!seen(&visible, &map, 16, 5), "room B's floor is blocked by its door");
        // The wall beside the player is lit; tiles behind it are not.
        assert!(seen(&visible, &map, 11, 4), "the wall beside the player is lit");
        assert!(!seen(&visible, &map, 11, 3), "tiles behind the wall are not");
        assert!(!seen(&visible, &map, 18, 3), "a tile deep in room B is not visible");
    }

    #[test]
    fn walls_block_line_of_sight() {
        let map = map_from(&TWO_ROOMS);
        // From room A nothing in room B is visible: the wall column between
        // the rooms blocks every line.
        let visible = compute(&map, 1, 1);
        for y in 1..=5 {
            for x in 16..=21 {
                assert!(!seen(&visible, &map, x, y), "({x},{y}) is behind the wall");
            }
        }
    }

    #[test]
    fn stepping_through_a_door_lights_the_room_ahead() {
        let map = map_from(&TWO_ROOMS);
        // On room A's door, the room it opens into lights up.
        let visible = compute(&map, 7, 5);
        assert!(seen(&visible, &map, 1, 1), "room A is lit from its door");
        assert!(!seen(&visible, &map, 15, 5), "room B's door is not lit from A's door");
        // On room B's door, room B lights up and room A goes dark.
        let visible = compute(&map, 15, 5);
        assert!(seen(&visible, &map, 18, 3), "room B is lit from its door");
        assert!(!seen(&visible, &map, 3, 3), "room A is dark from B's door");
    }

    #[test]
    fn leaving_a_room_hides_it_again() {
        let map = map_from(&TWO_ROOMS);
        // Out in the corridor, neither room is visible.
        let visible = compute(&map, 11, 5);
        assert!(!seen(&visible, &map, 3, 3), "room A is dark from the corridor");
        assert!(!seen(&visible, &map, 18, 3), "room B is dark too");
    }
}
