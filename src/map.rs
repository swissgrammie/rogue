//! Dungeon map generation, 1980-Rogue style.
//!
//! The level is divided into a 3x3 grid of cells. Each cell holds at most one
//! rectangular room; a few cells are left empty ("gone rooms"), as in the
//! original. Rooms are then wired together with orthogonal corridors that run
//! through the gaps between cells, so every room is reachable from every other.

use crate::rng::Rng;

/// Full map extent. 80 columns is the classic Rogue screen width.
pub const MAP_WIDTH: usize = 80;
pub const MAP_HEIGHT: usize = 24;

/// The room grid: 3 columns by 3 rows of cells, at most one room per cell.
pub const GRID_COLS: usize = 3;
pub const GRID_ROWS: usize = 3;
const GRID_CELLS: usize = GRID_COLS * GRID_ROWS;

/// Smallest room interior (floor area) we will place.
const MIN_ROOM_WIDTH: usize = 4;
const MIN_ROOM_HEIGHT: usize = 2;

/// Every cell keeps a two-tile margin on each side: one for the room's own wall
/// and one for corridors to pass through. Rooms therefore never touch, and
/// there is always somewhere to bend a corridor.
const CELL_MARGIN: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    /// Solid rock / room wall.
    Wall,
    /// Lit room floor.
    Floor,
    /// Corridor floor. Walkable like `Floor`, but tracked separately because
    /// Rogue lights rooms and not passages.
    Corridor,
    /// Doorway in a room wall.
    Door,
}

impl Tile {
    pub fn glyph(self) -> char {
        match self {
            Tile::Wall => '#',
            Tile::Floor | Tile::Corridor => '.',
            Tile::Door => '+',
        }
    }

    pub fn is_walkable(self) -> bool {
        !matches!(self, Tile::Wall)
    }
}

/// A room's *interior* (floor) rectangle, inclusive on both ends. Its walls
/// occupy the surrounding ring, i.e. columns `x0 - 1` and `x1 + 1` and rows
/// `y0 - 1` and `y1 + 1`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Room {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Room {
    pub fn width(&self) -> usize {
        self.x1 - self.x0 + 1
    }

    pub fn height(&self) -> usize {
        self.y1 - self.y0 + 1
    }

    pub fn center(&self) -> (usize, usize) {
        ((self.x0 + self.x1) / 2, (self.y0 + self.y1) / 2)
    }

    /// Interiors overlap, counting each room's wall ring so that two rooms
    /// never share a wall.
    pub fn overlaps(&self, other: &Room) -> bool {
        self.x0 <= other.x1 + 1
            && other.x0 <= self.x1 + 1
            && self.y0 <= other.y1 + 1
            && other.y0 <= self.y1 + 1
    }
}

pub struct Dungeon {
    pub width: usize,
    pub height: usize,
    tiles: Vec<Tile>,
    /// One slot per grid cell, in row-major order; `None` for a gone room.
    rooms: [Option<Room>; GRID_CELLS],
}

impl Dungeon {
    /// Generate a level with the default 80x24 extent.
    pub fn generate(seed: u64) -> Dungeon {
        Dungeon::generate_sized(seed, MAP_WIDTH, MAP_HEIGHT)
    }

    pub fn generate_sized(seed: u64, width: usize, height: usize) -> Dungeon {
        let min_width = GRID_COLS * (MIN_ROOM_WIDTH + 2 * CELL_MARGIN);
        let min_height = GRID_ROWS * (MIN_ROOM_HEIGHT + 2 * CELL_MARGIN);
        assert!(
            width >= min_width && height >= min_height,
            "map must be at least {min_width}x{min_height}"
        );

        let mut rng = Rng::new(seed);
        let mut dungeon = Dungeon {
            width,
            height,
            tiles: vec![Tile::Wall; width * height],
            rooms: [None; GRID_CELLS],
        };

        let occupied = pick_occupied_cells(&mut rng);
        for cell in 0..GRID_CELLS {
            if occupied[cell] {
                let room = dungeon.random_room_in_cell(cell, &mut rng);
                dungeon.rooms[cell] = Some(room);
                dungeon.carve_room(&room);
            }
        }

        for (a, b) in corridor_edges(&occupied, &mut rng) {
            dungeon.carve_corridor(a, b, &mut rng);
        }

        dungeon
    }

    pub fn tile(&self, x: usize, y: usize) -> Tile {
        self.tiles[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, tile: Tile) {
        self.tiles[y * self.width + x] = tile;
    }

    /// Only dig through solid rock, so a corridor crossing another corridor's
    /// doorway or a room floor leaves it alone.
    fn dig(&mut self, x: usize, y: usize) {
        if self.tile(x, y) == Tile::Wall {
            self.set(x, y, Tile::Corridor);
        }
    }

    /// The rooms that were placed, in grid order.
    pub fn rooms(&self) -> Vec<Room> {
        self.rooms.iter().flatten().copied().collect()
    }

    /// The room in grid cell `(col, row)`, if that cell is not a gone room.
    pub fn room_at(&self, col: usize, row: usize) -> Option<Room> {
        self.rooms[row * GRID_COLS + col]
    }

    /// The bounds a room in `cell` must stay inside, as inclusive interior
    /// limits `(x0, y0, x1, y1)`.
    pub fn cell_interior_bounds(&self, cell: usize) -> (usize, usize, usize, usize) {
        let (col, row) = (cell % GRID_COLS, cell / GRID_COLS);
        let x_lo = col * self.width / GRID_COLS;
        let x_hi = (col + 1) * self.width / GRID_COLS - 1;
        let y_lo = row * self.height / GRID_ROWS;
        let y_hi = (row + 1) * self.height / GRID_ROWS - 1;
        (
            x_lo + CELL_MARGIN,
            y_lo + CELL_MARGIN,
            x_hi - CELL_MARGIN,
            y_hi - CELL_MARGIN,
        )
    }

    fn random_room_in_cell(&self, cell: usize, rng: &mut Rng) -> Room {
        let (x_lo, y_lo, x_hi, y_hi) = self.cell_interior_bounds(cell);
        let w = rng.range(MIN_ROOM_WIDTH, x_hi - x_lo + 1);
        let h = rng.range(MIN_ROOM_HEIGHT, y_hi - y_lo + 1);
        let x0 = rng.range(x_lo, x_hi - w + 1);
        let y0 = rng.range(y_lo, y_hi - h + 1);
        Room {
            x0,
            y0,
            x1: x0 + w - 1,
            y1: y0 + h - 1,
        }
    }

    fn carve_room(&mut self, room: &Room) {
        for y in (room.y0 - 1)..=(room.y1 + 1) {
            for x in (room.x0 - 1)..=(room.x1 + 1) {
                let inside = (room.y0..=room.y1).contains(&y) && (room.x0..=room.x1).contains(&x);
                self.set(x, y, if inside { Tile::Floor } else { Tile::Wall });
            }
        }
    }

    /// Join two rooms in adjacent (or empty-cell-separated) cells with an
    /// orthogonal corridor: out of a door, along, one bend, and in the far door.
    fn carve_corridor(&mut self, cell_a: usize, cell_b: usize, rng: &mut Rng) {
        let a = self.rooms[cell_a].expect("corridor endpoint must be a room");
        let b = self.rooms[cell_b].expect("corridor endpoint must be a room");

        if cell_a / GRID_COLS == cell_b / GRID_COLS {
            // Same grid row: run left to right.
            let (left, right) = if a.x0 < b.x0 { (a, b) } else { (b, a) };
            let ly = rng.range(left.y0, left.y1);
            let ry = rng.range(right.y0, right.y1);
            let (left_door_x, right_door_x) = (left.x1 + 1, right.x0 - 1);
            let bend_x = rng.range(left_door_x + 1, right_door_x - 1);

            self.set(left_door_x, ly, Tile::Door);
            self.set(right_door_x, ry, Tile::Door);
            for x in (left_door_x + 1)..=bend_x {
                self.dig(x, ly);
            }
            for y in min_max(ly, ry) {
                self.dig(bend_x, y);
            }
            for x in bend_x..right_door_x {
                self.dig(x, ry);
            }
        } else {
            // Same grid column: run top to bottom.
            let (top, bottom) = if a.y0 < b.y0 { (a, b) } else { (b, a) };
            let tx = rng.range(top.x0, top.x1);
            let bx = rng.range(bottom.x0, bottom.x1);
            let (top_door_y, bottom_door_y) = (top.y1 + 1, bottom.y0 - 1);
            let bend_y = rng.range(top_door_y + 1, bottom_door_y - 1);

            self.set(tx, top_door_y, Tile::Door);
            self.set(bx, bottom_door_y, Tile::Door);
            for y in (top_door_y + 1)..=bend_y {
                self.dig(tx, y);
            }
            for x in min_max(tx, bx) {
                self.dig(x, bend_y);
            }
            for y in bend_y..bottom_door_y {
                self.dig(bx, y);
            }
        }
    }

    /// The map as ASCII, one line per row.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity((self.width + 1) * self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                out.push(self.tile(x, y).glyph());
            }
            out.push('\n');
        }
        out
    }
}

fn min_max(a: usize, b: usize) -> std::ops::RangeInclusive<usize> {
    a.min(b)..=a.max(b)
}

/// Choose which grid cells hold a room. Rogue leaves the odd cell empty; we
/// keep at least six rooms, which also guarantees the cell graph below is
/// connected (isolating a cell would need its whole row *and* column empty).
fn pick_occupied_cells(rng: &mut Rng) -> [bool; GRID_CELLS] {
    let room_count = rng.range(6, GRID_CELLS);
    let mut cells: Vec<usize> = (0..GRID_CELLS).collect();
    rng.shuffle(&mut cells);

    let mut occupied = [false; GRID_CELLS];
    for &cell in cells.iter().take(room_count) {
        occupied[cell] = true;
    }
    debug_assert!(cell_graph_is_connected(&occupied));
    occupied
}

/// Cells that a corridor may join directly: the next occupied cell along a row
/// or column, skipping over gone rooms.
fn cell_neighbors(occupied: &[bool; GRID_CELLS], cell: usize) -> Vec<usize> {
    let (col, row) = (cell % GRID_COLS, cell / GRID_COLS);
    let mut out = Vec::new();

    for step in [-1i32, 1] {
        // Along the row.
        let mut c = col as i32 + step;
        while (0..GRID_COLS as i32).contains(&c) {
            let other = row * GRID_COLS + c as usize;
            if occupied[other] {
                out.push(other);
                break;
            }
            c += step;
        }
        // Along the column.
        let mut r = row as i32 + step;
        while (0..GRID_ROWS as i32).contains(&r) {
            let other = r as usize * GRID_COLS + col;
            if occupied[other] {
                out.push(other);
                break;
            }
            r += step;
        }
    }
    out
}

fn cell_graph_is_connected(occupied: &[bool; GRID_CELLS]) -> bool {
    let Some(start) = (0..GRID_CELLS).find(|&c| occupied[c]) else {
        return true;
    };
    let mut seen = [false; GRID_CELLS];
    let mut stack = vec![start];
    seen[start] = true;
    let mut count = 0;
    while let Some(cell) = stack.pop() {
        count += 1;
        for n in cell_neighbors(occupied, cell) {
            if !seen[n] {
                seen[n] = true;
                stack.push(n);
            }
        }
    }
    count == occupied.iter().filter(|&&o| o).count()
}

/// Pick the corridors to dig: a random spanning tree over the occupied cells
/// (guaranteeing full connectivity), plus a few extra links so the level has
/// loops rather than reading as a strict tree.
fn corridor_edges(occupied: &[bool; GRID_CELLS], rng: &mut Rng) -> Vec<(usize, usize)> {
    let mut edges = Vec::new();
    let Some(start) = (0..GRID_CELLS).find(|&c| occupied[c]) else {
        return edges;
    };

    let mut in_tree = [false; GRID_CELLS];
    in_tree[start] = true;
    let mut frontier = vec![start];

    // Randomized depth-first walk: spanning tree, Rogue-ish winding shape.
    while let Some(&cell) = frontier.last() {
        let mut candidates: Vec<usize> = cell_neighbors(occupied, cell)
            .into_iter()
            .filter(|&n| !in_tree[n])
            .collect();
        if candidates.is_empty() {
            frontier.pop();
            continue;
        }
        rng.shuffle(&mut candidates);
        let next = candidates[0];
        in_tree[next] = true;
        edges.push((cell, next));
        frontier.push(next);
    }

    // Extra connections, one in four of the remaining adjacencies.
    for cell in 0..GRID_CELLS {
        if !occupied[cell] {
            continue;
        }
        for n in cell_neighbors(occupied, cell) {
            if n <= cell {
                continue;
            }
            let known = edges
                .iter()
                .any(|&(a, b)| (a, b) == (cell, n) || (a, b) == (n, cell));
            if !known && rng.chance(1, 4) {
                edges.push((cell, n));
            }
        }
    }

    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Enough seeds to shake out placement and routing edge cases.
    const SEEDS: std::ops::Range<u64> = 0..300;

    #[test]
    fn rooms_stay_in_bounds_with_room_for_walls() {
        for seed in SEEDS {
            let d = Dungeon::generate(seed);
            for room in d.rooms() {
                assert!(room.x0 >= 1 && room.y0 >= 1, "seed {seed}: {room:?}");
                assert!(
                    room.x1 + 1 < d.width && room.y1 + 1 < d.height,
                    "seed {seed}: {room:?}"
                );
                assert!(room.width() >= MIN_ROOM_WIDTH, "seed {seed}: {room:?}");
                assert!(room.height() >= MIN_ROOM_HEIGHT, "seed {seed}: {room:?}");
            }
        }
    }

    #[test]
    fn rooms_never_overlap() {
        for seed in SEEDS {
            let d = Dungeon::generate(seed);
            let rooms = d.rooms();
            for (i, a) in rooms.iter().enumerate() {
                for b in &rooms[i + 1..] {
                    assert!(!a.overlaps(b), "seed {seed}: {a:?} overlaps {b:?}");
                }
            }
        }
    }

    #[test]
    fn at_most_one_room_per_grid_cell_and_inside_it() {
        for seed in SEEDS {
            let d = Dungeon::generate(seed);
            let mut placed = 0;
            for cell in 0..GRID_CELLS {
                let Some(room) = d.room_at(cell % GRID_COLS, cell / GRID_COLS) else {
                    continue;
                };
                placed += 1;
                let (x_lo, y_lo, x_hi, y_hi) = d.cell_interior_bounds(cell);
                assert!(
                    room.x0 >= x_lo && room.x1 <= x_hi && room.y0 >= y_lo && room.y1 <= y_hi,
                    "seed {seed}: {room:?} escapes cell {cell} bounds \
                     ({x_lo},{y_lo})..=({x_hi},{y_hi})"
                );
            }
            assert_eq!(placed, d.rooms().len());
            assert!((6..=GRID_CELLS).contains(&placed), "seed {seed}: {placed} rooms");
        }
    }

    /// Flood-fill every walkable tile from one room and check each other room
    /// was reached, i.e. all rooms are mutually connected by corridors.
    #[test]
    fn every_room_reaches_every_other_room() {
        for seed in SEEDS {
            let d = Dungeon::generate(seed);
            let rooms = d.rooms();
            let start = rooms[0].center();

            let mut seen = vec![false; d.width * d.height];
            let mut queue = VecDeque::from([start]);
            seen[start.1 * d.width + start.0] = true;
            while let Some((x, y)) = queue.pop_front() {
                for (nx, ny) in [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ] {
                    if nx >= d.width || ny >= d.height {
                        continue;
                    }
                    let idx = ny * d.width + nx;
                    if !seen[idx] && d.tile(nx, ny).is_walkable() {
                        seen[idx] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }

            for room in &rooms {
                for y in room.y0..=room.y1 {
                    for x in room.x0..=room.x1 {
                        assert!(
                            seen[y * d.width + x],
                            "seed {seed}: ({x},{y}) in {room:?} unreachable from {start:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn doors_sit_in_room_walls() {
        for seed in SEEDS {
            let d = Dungeon::generate(seed);
            let rooms = d.rooms();
            for y in 0..d.height {
                for x in 0..d.width {
                    if d.tile(x, y) != Tile::Door {
                        continue;
                    }
                    let in_a_wall = rooms.iter().any(|r| {
                        let on_wall_ring = (x == r.x0 - 1 || x == r.x1 + 1)
                            && (r.y0..=r.y1).contains(&y)
                            || (y == r.y0 - 1 || y == r.y1 + 1) && (r.x0..=r.x1).contains(&x);
                        on_wall_ring
                    });
                    assert!(in_a_wall, "seed {seed}: door at ({x},{y}) is not in a room wall");
                }
            }
        }
    }

    #[test]
    fn map_border_is_solid() {
        let d = Dungeon::generate(11);
        for x in 0..d.width {
            assert_eq!(d.tile(x, 0), Tile::Wall);
            assert_eq!(d.tile(x, d.height - 1), Tile::Wall);
        }
        for y in 0..d.height {
            assert_eq!(d.tile(0, y), Tile::Wall);
            assert_eq!(d.tile(d.width - 1, y), Tile::Wall);
        }
    }

    #[test]
    fn same_seed_gives_same_map() {
        assert_eq!(Dungeon::generate(42).render(), Dungeon::generate(42).render());
        assert_ne!(Dungeon::generate(42).render(), Dungeon::generate(43).render());
    }

    #[test]
    fn render_shape_and_glyphs() {
        let d = Dungeon::generate(1);
        let rendered = d.render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), MAP_HEIGHT);
        assert!(lines.iter().all(|l| l.chars().count() == MAP_WIDTH));
        assert!(lines.iter().all(|l| l.chars().all(|c| "#.+".contains(c))));
        assert!(d.render().contains('+'), "expected at least one door");
    }
}
