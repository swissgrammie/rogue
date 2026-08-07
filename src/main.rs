use rogue::map::{self, Dungeon};
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE: &str = "\
rogue — a terminal roguelike

Usage:
  rogue [options]

Options:
      --dump-map        Print a generated dungeon level as ASCII and exit
      --seed <n>        Seed the generator (default: from the clock) for
                        reproducible maps
      --width <n>       Map width in columns (default: 80)
      --height <n>      Map height in rows (default: 24)
  -h, --help            Show this help
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

    if opts.dump_map {
        let seed = opts.seed.unwrap_or_else(seed_from_clock);
        let dungeon = Dungeon::generate_sized(seed, opts.width, opts.height);
        println!("seed: {seed}");
        print!("{}", dungeon.render());
        return;
    }

    println!("rogue: nothing to play yet — try --dump-map (see --help)");
}

struct Options {
    dump_map: bool,
    help: bool,
    seed: Option<u64>,
    width: usize,
    height: usize,
}

impl Options {
    fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Options, String> {
        let mut opts = Options {
            dump_map: false,
            help: false,
            seed: None,
            width: map::MAP_WIDTH,
            height: map::MAP_HEIGHT,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--dump-map" => opts.dump_map = true,
                "-h" | "--help" => opts.help = true,
                "--seed" => opts.seed = Some(parse_value(&arg, args.next())?),
                "--width" => opts.width = parse_value(&arg, args.next())?,
                "--height" => opts.height = parse_value(&arg, args.next())?,
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
        assert_eq!(
            (opts.width, opts.height),
            (map::MAP_WIDTH, map::MAP_HEIGHT)
        );
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
