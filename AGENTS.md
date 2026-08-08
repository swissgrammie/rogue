# rogue — agent rules

A terminal roguelike in Rust (ratatui + crossterm). Faithful to 1980 Rogue:
procedurally generated dungeon, 26 levels, @ is the player, monsters A-Z,
find the Amulet of Yendor and get out.

- Build: `cargo build`. Test: `cargo test`. Both must pass before you finish.
- Keep modules small: map generation, entities, combat, FOV, UI, game loop.
  Game logic lives in the library (`src/lib.rs` + one module per concern);
  `src/main.rs` is a thin CLI shell over it, so modules stay unit-testable.
  The play loop is `src/game.rs`: ratatui (`Terminal<CrosstermBackend>`)
  owns raw mode, the alternate screen, and resize handling; crossterm still
  supplies input events. `src/ui.rs` is the single draw path — the map as a
  fixed grid of buffer cells, the status/hint lines as Paragraphs — shared
  by the interactive loop, `--dump-frame`, and the TestBackend tests, so
  the plain-text frame is character-identical to the terminal frame.
  Input mapping and collision rules are pure functions with unit tests
  there. The loop redraws only when the state changed (`Game::step` returns
  whether it did; blocked moves and unknown keys draw nothing); ratatui's
  buffer diffing writes only the changed cells, so an idle terminal stays
  perfectly still instead of flickering.
- Field of view lives in `src/visibility.rs` and `Game`'s per-floor state:
  `Game.seen` (tiles ever explored, persisted per floor via `seen_maps` so
  re-entering a floor restores it) and `Game.visible` (recomputed on every
  move/floor change/resize by `Game::refresh_visibility`). Rules: standing in
  a room lights the whole room (floor + wall ring + doors); a corridor gives
  a short `SIGHT_RANGE` line-of-sight reach with walls and closed doors
  blocking. `Game::glyph_at` is the FOV-aware draw path: visible tiles show
  full features, seen-but-dark tiles show the bare tile, never-seen tiles
  are solid rock. `--dump-map` keeps the full map (via `render_level`);
  `--dump-frame` shows the player's actual view. Frame tests pin the FOV in
  `tests/frame.rs` (fresh game = starting room + rock elsewhere; moving to a
  new room reveals it; leaving dims it).
- Combat is `src/combat.rs`: swing/roll_em/killed/check_level and the fight
  round (player acts, then runners → doctor → hunger). The monster table in
  `src/entity.rs` is the Rogue 5.4.4 Aquator…Zombie set (exp/lvl/arm/dmg
  columns); monster HP is `roll(lvl, 8)` at spawn, not a table stat. Formulas
  were verified against Rogue 5.4.4 in the combat research report.
- Inspect a generated level with `cargo run -- --dump-map --seed 1`; a seed
  makes any map reproducible, which is how the map tests stay stable.
  `--floor N` dumps a specific floor; `--seed 1 --floor 26` shows the Amulet.
- Level progression (stairs, floor counter, Amulet, win) lives in
  `src/levels.rs`: `MAX_FLOOR`/`AMULET_LEVEL = 26`, `floor_seed(base, floor)`
  (floor 1 = base seed; re-ascending regenerates the floor above
  deterministically), and stair/Amulet placement on room floor tiles. The
  game loop drives descents/ascents from `Game` state in `src/game.rs`;
  `Game.floor` (1..=26) is the depth seam — `entity::populate(dungeon,
  floor, rng)` records it for combat's monster scaling, and the status line
  leads with `Level: N` per the combat report §9 format.
- The map fits the terminal: interactive play sizes the map via
  `map::fit_bounds`/`resolve_map_size` (width = columns, height = rows minus
  the status/hint lines, so map rows + the two UI lines never exceed the
  terminal; only an absolute 1x1 floor remains), resizes regenerate the
  current floor at the new size in the `run` loop, and maps smaller than the
  3x3 room grid's native minimum (`map::GRID_MIN_*`) get a single-room
  fallback in `Dungeon::generate_sized` instead of panicking. Explicit
  `--width`/`--height` override auto-fit; `--dump-map` keeps its 80x24
  defaults. `ui::draw` also clamps to the frame area, so a terminal that
  shrinks between a resize event and the next draw renders what fits
  instead of overflowing the buffer.
- Terminal-free testability: `game::play(seed, width, height, &[Key])` runs
  a scripted session with no terminal (deterministic for a seed; stops on
  quit/death/escape), `game::script_keys` parses a key string (`h/j/k/l`,
  `<left>`/`<right>`/`<up>`/`<down>`, `<esc>`, `q`), and
  `game::Game::snapshot()` prints one compact final-state line. The CLI
  exposes these as `rogue --script <keys> --seed N [--width/--height]`
  (snapshot to stdout, exit 0) and `rogue --dump-frame [--floor N]
  [--width/--height]` (the exact interactive frame — map + status + hint —
  rendered through the same ratatui buffer path as the terminal via
  `Game::render`, default 80x24). The display itself is deterministically
  testable at any window size with `ratatui::TestBackend` — `tests/frame.rs`
  sweeps window sizes (80x24 … 50x30), pins the no-overflow invariant at
  tiny windows (map rows + the two UI lines always fit, status always
  visible), and covers resizes (regeneration at the new size, no stale
  content). The golden harness lives in `tests/harness.rs`: no-overflow line
  counts at tiny windows, deterministic-run snapshots, frame content at two
  sizes, and a reactive walker (stealth paths around unwinnable monsters,
  hunting winnable ones, sprinting the last steps) that records a key
  script and replays it through the CLI. NOTE: a full 26-floor `won: true`
  journey is currently unreachable — the monster table spawns at full table
  stats on every floor and chasers are unshakeable, so every seed hits an
  unwinnable guard (probe bottoms out around floor 8); see the walker test
  docs.
- This working copy is a Jujutsu (jj) workspace, not a plain git checkout.
  If $JJHOUSE_AGENT_GUIDE is set, read that file before touching version
  control. Never run raw git write commands; describe work with `jj describe`.

## Maintaining this file

Keep this file for knowledge useful to almost every future agent session in this project.
Do not repeat what the codebase already shows; point to the authoritative file or command instead.
Prefer rewriting or pruning existing entries over appending new ones.
When updating this file, preserve this bar for all agents and keep entries concise.
