# rogue — agent rules

A terminal roguelike in Rust (ratatui + crossterm). Faithful to 1980 Rogue:
procedurally generated dungeon, 26 levels, @ is the player, monsters A-Z,
find the Amulet of Yendor and get out.

- Build: `cargo build`. Test: `cargo test`. Both must pass before you finish.
- Keep modules small: map generation, entities, combat, FOV, UI, game loop.
  Game logic lives in the library (`src/lib.rs` + one module per concern);
  `src/main.rs` is a thin CLI shell over it, so modules stay unit-testable.
  The play loop is `src/game.rs`: raw crossterm (raw mode + alternate screen)
  until ratatui lands; input mapping and collision rules are pure functions
  with unit tests there.
- Inspect a generated level with `cargo run -- --dump-map --seed 1`; a seed
  makes any map reproducible, which is how the map tests stay stable.
- This working copy is a Jujutsu (jj) workspace, not a plain git checkout.
  If $JJHOUSE_AGENT_GUIDE is set, read that file before touching version
  control. Never run raw git write commands; describe work with `jj describe`.

## Maintaining this file

Keep this file for knowledge useful to almost every future agent session in this project.
Do not repeat what the codebase already shows; point to the authoritative file or command instead.
Prefer rewriting or pruning existing entries over appending new ones.
When updating this file, preserve this bar for all agents and keep entries concise.
