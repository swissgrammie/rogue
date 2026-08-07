use rogue::entity;
use rogue::game;
use rogue::levels::{self, AMULET_LEVEL};
use rogue::map::{self, Dungeon};
use rogue::rng::Rng;
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
rogue - a terminal roguelike

Usage:
  rogue [options]

Options:
      --dump-map        Print a generated dungeon floor as ASCII and exit
      --dump-frame      Print the interactive frame (map, status line, hint
                        line) as plain text and exit; the map is fit inside
                        the window (default: 80x24)
      --script <keys>   Run a scripted session with no terminal and print a
                        final-state snapshot; keys are h/j/k/l or
                        <left>/<right>/<up>/<down>, q quits
      --floor <n>       Dungeon floor to dump or start on (1..=26, default: 1);
                        the Amulet of Yendor waits on floor 26
      --seed <n>        Seed the generator (default: from the clock) for
                        reproducible maps
      --width <n>       Window width in columns for --dump-frame/--script
                        (default: 80); with --dump-map, the map width
      --height <n>      Window height in rows for --dump-frame/--script
                        (default: 24); with --dump-map, the map height
  -h, --help            Show this help

Playing:
  Move with h/j/k/l or the arrow keys; walls block movement. Find the down
  stair (>) and descend; the Amulet (,) waits on floor 26. Reach the surface
  exit (<) on floor 1 with the Amulet to win. Press q to quit.
";

fn main() {
    match Options::parse(std::env::args().skip(1)) {
        Ok(opts) => run(opts),
        Err(message) => {
            eprintln!("rogue: {message}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
}

fn run(opts: Options) {
    if opts.help {
        print!("{USAGE}");
        return;
    }

    let seed = opts.seed.unwrap_or_else(seed_from_clock);
    let floor = opts.floor;

    if opts.dump_map {
        // The same (seed, floor) derivation the game uses, so a dumped floor
        // matches what a playthrough descends into.
        let map_seed = levels::floor_seed(seed, floor);
        let dungeon = Dungeon::generate_sized(map_seed, opts.width, opts.height);
        let mut rng = Rng::new(map_seed);
        let spawn = entity::populate(&dungeon, floor, &mut rng);
        let stairs = levels::place_stairs(&dungeon, &mut rng, (spawn.player.x, spawn.player.y));
        let amulet = (floor == AMULET_LEVEL)
            .then(|| levels::place_amulet(&dungeon, &mut rng, stairs, (spawn.player.x, spawn.player.y)));
        let player = entity::Player::new(spawn.player.x, spawn.player.y);
        println!("seed: {seed}  floor: {floor}");
        print!("{}", game::render_level(&dungeon, player, stairs, amulet, &spawn.monsters));
        println!("{}", spawn.summary());
        return;
    }

    if opts.dump_frame {
        // The exact frame the interactive loop renders, as plain text: map
        // rows plus the status and hint lines. The map is fit inside the
        // window (explicit --width/--height or the 80x24 default), so the
        // frame is always exactly `rows` lines.
        let (cols, rows) = opts.window_size();
        let (w, h) = map::fit_bounds(cols, rows);
        let game = game::Game::at_floor(seed, floor, w, h);
        print!("{}", game.render());
        return;
    }

    if let Some(script) = &opts.script {
        let keys = game::script_keys(script).unwrap_or_else(|message| {
            eprintln!("rogue: {message}\n\n{USAGE}");
            std::process::exit(2);
        });
        // Same window contract as --dump-frame: the map is fit inside the
        // window so a scripted run sees exactly the interactive layout.
        let (cols, rows) = opts.window_size();
        let (w, h) = map::fit_bounds(cols, rows);
        let game = game::play(seed, w, h, &keys);
        println!("{}", game.snapshot());
        return;
    }

    let (width, height) = opts.map_size_overrides();
    if let Err(err) = rogue::game::run(seed, width, height) {
        eprintln!("rogue: {err}");
        std::process::exit(1);
    }
}

struct Options {
    dump_map: bool,
    dump_frame: bool,
    script: Option<String>,
    help: bool,
    seed: Option<u64>,
    floor: u32,
    width: usize,
    height: usize,
    width_explicit: bool,
    height_explicit: bool,
}

impl Options {
    /// The window the scripted and frame-dump modes frame: explicit
    /// `--width`/`--height` overrides, else the classic 80x24 extent. The
    /// map is fit inside, so the frame is exactly `rows` lines — the map
    /// plus the status and hint lines never exceed the window.
    fn window_size(&self) -> (usize, usize) {
        (
            if self.width_explicit { self.width } else { map::MAP_WIDTH },
            if self.height_explicit { self.height } else { map::MAP_HEIGHT },
        )
    }

    /// The interactive size: `None` per axis means "fit the terminal"; a
    /// `Some` carries an explicit `--width`/`--height` override. The dump
    /// path uses `width`/`height` directly and keeps its fixed defaults.
    fn map_size_overrides(&self) -> (Option<usize>, Option<usize>) {
        (
            self.width_explicit.then_some(self.width),
            self.height_explicit.then_some(self.height),
        )
    }

    fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
        let mut opts = Options {
            dump_map: false,
            dump_frame: false,
            script: None,
            help: false,
            seed: None,
            floor: 1,
            width: map::MAP_WIDTH,
            height: map::MAP_HEIGHT,
            width_explicit: false,
            height_explicit: false,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dump-map" => opts.dump_map = true,
                "--dump-frame" => opts.dump_frame = true,
                "--script" => {
                    opts.script = Some(parse_value::<String>(&arg, args.next())?);
                }
                "-h" | "--help" => opts.help = true,
                "--seed" => opts.seed = Some(parse_value(&arg, args.next())?),
                "--floor" => {
                    opts.floor = parse_value(&arg, args.next())?;
                    if !(1..=levels::MAX_FLOOR).contains(&opts.floor) {
                        return Err(format!(
                            "`--floor` must be 1..={}, got {}",
                            levels::MAX_FLOOR, opts.floor
                        ));
                    }
                }
                "--width" => {
                    opts.width = parse_value(&arg, args.next())?;
                    opts.width_explicit = true;
                }
                "--height" => {
                    opts.height = parse_value(&arg, args.next())?;
                    opts.height_explicit = true;
                }
                other => return Err(format!("unrecognized argument `{other}`")),
            }
        }
        let modes = [opts.dump_map, opts.dump_frame, opts.script.is_some()]
            .into_iter()
            .filter(|&m| m)
            .count();
        if modes > 1 {
            return Err("`--dump-map`, `--dump-frame` and `--script` are mutually exclusive".into());
        }
        Ok(opts)
    }
}

fn parse_value<T: std::str::FromStr>(flag: &str, value: Option<String>) -> Result<T, String> {
    let value = value.ok_or_else(|| format!("`{flag}` needs a value"))?;
    value
        .parse()
        .map_err(|_| format!("`{flag}` needs a number, got `{value}`"))
}

fn seed_from_clock() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Options, map};

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_dump_map_and_seed() {
        let opts = parse(&["--dump-map", "--seed", "7"]).expect("should parse");
        assert!(opts.dump_map);
        assert_eq!(opts.seed, Some(7));
        assert_eq!(opts.floor, 1, "floor defaults to 1");
        assert_eq!(
            (opts.width, opts.height),
            (map::MAP_WIDTH, map::MAP_HEIGHT)
        );
    }

    #[test]
    fn parses_and_validates_floor() {
        let opts = parse(&["--dump-map", "--seed", "7", "--floor", "26"]).expect("should parse");
        assert_eq!(opts.floor, 26);
        assert!(parse(&["--floor", "0"]).is_err());
        assert!(parse(&["--floor", "27"]).is_err());
        assert!(parse(&["--floor", "abc"]).is_err());
    }

    #[test]
    fn defaults_to_no_seed() {
        assert_eq!(parse(&[]).expect("should parse").seed, None);
    }

    #[test]
    fn parses_script_and_dump_frame() {
        let opts = parse(&["--script", "lll", "--seed", "7", "--width", "60", "--height", "14"])
            .expect("should parse");
        assert_eq!(opts.script.as_deref(), Some("lll"));
        assert_eq!(opts.window_size(), (60, 14));
        let opts = parse(&["--dump-frame", "--floor", "26"]).expect("should parse");
        assert!(opts.dump_frame);
        assert_eq!(opts.floor, 26);
        assert_eq!(opts.window_size(), (map::MAP_WIDTH, map::MAP_HEIGHT), "80x24 default");
    }

    #[test]
    fn new_modes_are_mutually_exclusive() {
        assert!(parse(&["--dump-map", "--dump-frame"]).is_err());
        assert!(parse(&["--dump-frame", "--script", "l"]).is_err());
        assert!(parse(&["--dump-map", "--script", "l"]).is_err());
        assert!(parse(&["--script", "l", "--dump-map", "--dump-frame"]).is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(&["--seed"]).is_err());
        assert!(parse(&["--seed", "abc"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
