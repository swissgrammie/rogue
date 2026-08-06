# rogue — agent rules

A terminal roguelike in Rust (ratatui + crossterm). Faithful to 1980 Rogue:
procedurally generated dungeon, 26 levels, @ is the player, monsters A-Z,
find the Amulet of Yendor and get out.

- Build: `cargo build`. Test: `cargo test`. Both must pass before you finish.
- Keep modules small: map generation, entities, combat, FOV, UI, game loop.
- This working copy is a Jujutsu (jj) workspace, not a plain git checkout.
  If $JJHOUSE_AGENT_GUIDE is set, read that file before touching version
  control. Never run raw git write commands; describe work with `jj describe`.
