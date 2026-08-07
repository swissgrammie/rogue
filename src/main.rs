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
      --floor <n>       Dungeon floor to dump or start on (1..=26, default: 1);
                        the Amulet of Yendor waits on floor 26
      --seed <n>        Seed the generator (default: from the clock) for
                        reproducible maps
      --width <n>       Map width in columns (default: fit the terminal;
                         with --dump-map, 80)
      --height <n>      Map height in rows (default: terminal rows minus the
                         status/hint lines; with --dump-map, 24)
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

    let (width, height) = opts.map_size_overrides();
    if let Err(err) = rogue::game::run(seed, width, height) {
        eprintln!("rogue: {err}");
        std::process::exit(1);
    }
}

struct Options {
    dump_map: bool,
    help: bool,
    seed: Option<u64>,
    floor: u32,
    width: usize,
    height: usize,
    width_explicit: bool,
    height_explicit: bool,
}

impl Options {
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
    fn rejects_bad_input() {
        assert!(parse(&["--seed"]).is_err());
        assert!(parse(&["--seed", "abc"]).is_err());
        assert!(parse(&["--nope"]).is_err());
    }
}
