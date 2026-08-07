//! Combat, faithful to classic Rogue 5.4.4 (the formulas verified against
//! the C source in the combat research report).
//!
//! Dependency-free like `rng.rs`: this module owns the to-hit roll
//! ([`swing`]), the damage loop ([`roll_em`]), experience and level-ups
//! ([`killed`], [`check_level`]), monster spawn scaling ([`lev_add`],
//! [`exp_add`]), and the AFTER-phase daemons ([`monster_phase`], [`doctor`],
//! [`hunger`]). It consumes the entity types (`Player`, `Monster`) and the
//! `Rng`, and nothing else.
//!
//! The round order (report §7.1) is owned here: the player acts first, then
//! `runners` (monsters chase/attack) → `doctor` (healing) → `stomach`
//! (hunger). The game loop drives the round via the functions in this module.

use crate::entity::{Monster, Player};
use crate::map::Dungeon;
use crate::rng::Rng;

/// Level-up thresholds (`e_levels[]`, report §6.4): level N requires
/// `exp >= e_levels[N-1]`; the last threshold caps the player at level 21.
pub const E_LEVELS: [i32; 20] = [
    10, 20, 40, 80, 160, 320, 640, 1300, 2600, 5200, 13000, 26000, 50000, 100000, 200000, 400000,
    800000, 2000000, 4000000, 8000000,
];

/// The highest experience level (report §6.4: `e_levels` ends at level 21).
pub const MAX_LEVEL: u32 = 21;

/// The floor the Amulet sits on (`AMULETLEVEL`, report §6.2). Monsters on
/// floors deeper than this get one extra notch of toughness per floor.
pub const AMULET_LEVEL: u32 = 26;

/// The fresh player's effective armor class: ring mail's `o_arm` (report
/// §2). The status line prints `10 - o_arm`, which is why a new character
/// shows "Arm: 4" while combat resolves against 6.
pub const STARTING_ARMOR_AC: i32 = 6;

/// How far a monster can "see" the player and wake up. Placeholder until the
/// FOV module lands; Rogue wakes ISMEAN monsters that come into view.
pub const SIGHT_RANGE: u32 = 6;

/// A wielded weapon: to-hit bonus (`o_hplus`), damage bonus (`o_dplus`) and
/// the damage string (`o_damage`). Only the starting gear exists so far; the
/// items task will add the rest of `weapons.c init_dam[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weapon {
    pub name: &'static str,
    pub hplus: i32,
    pub dplus: i32,
    pub damage: &'static str,
}

/// The starting mace +1,+1 (report §2): 2x4 damage, `o_hplus` 1, `o_dplus` 1.
pub const STARTING_WEAPON: Weapon = Weapon {
    name: "mace",
    hplus: 1,
    dplus: 1,
    damage: "2x4",
};

/// One attack segment of a damage string: `dice` rolls of a `sides`-sided die.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Damage {
    pub dice: u32,
    pub sides: u32,
}

impl Damage {
    pub const fn new(dice: u32, sides: u32) -> Damage {
        Damage { dice, sides }
    }
}

/// What one attacker's full sequence did to its defender (pure data; the UI
/// formats it). A multi-attack monster contributes one entry per `/` segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitResult {
    /// How many segments landed. A 0-damage hit still counts (report §5.1).
    pub hits: u32,
    /// Total damage dealt, after the per-hit floor of 0.
    pub damage: i32,
}

/// One attacker's roll_em parameters (report §5.1): level, strength, damage
/// string, and weapon bonuses. Monsters pass `hplus = dplus = 0` and
/// strength 10, so they get no bonuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackSpec {
    pub level: u32,
    pub strength: i32,
    pub damage: &'static str,
    pub hplus: i32,
    pub dplus: i32,
}

/// `rnd(n)` (report §3): uniform in `0..n`; `rnd(0)` is 0. Unlike `Rng::below`,
/// zero ranges do not panic.
pub fn rnd(rng: &mut Rng, n: u32) -> u32 {
    if n == 0 {
        0
    } else {
        rng.below(n as usize) as u32
    }
}

/// `roll(N, S)` (report §3): sum of N dice of S sides. Zero dice or zero
/// sides roll nothing (`roll(0, 0)` is safe and yields 0).
pub fn roll(rng: &mut Rng, number: u32, sides: u32) -> i32 {
    if number == 0 || sides == 0 {
        return 0;
    }
    (0..number).map(|_| rng.below(sides as usize) as i32 + 1).sum()
}

/// The to-hit roll (report §4.1, `fight.c:378 swing`): a d20 in `0..19`;
/// hit iff `res + wplus >= 20 - at_lvl - op_arm`.
///
/// `at_lvl` is the attacker's level, `op_arm` the defender's armor class
/// (lower is better, can be negative), and `wplus` the to-hit bonus.
pub fn swing(rng: &mut Rng, at_lvl: u32, op_arm: i32, wplus: i32) -> bool {
    let need = 20 - at_lvl as i32 - op_arm;
    let res = rnd(rng, 20) as i32;
    res + wplus >= need
}

/// P(hit) = `(at_lvl + op_arm + wplus) / 20`, clamped to `[0, 1]`
/// (report §4.1). Useful for tests and any UI that wants to display odds.
pub fn hit_chance(at_lvl: u32, op_arm: i32, wplus: i32) -> f64 {
    let hits = (at_lvl as i32 + op_arm + wplus).clamp(0, 20);
    hits as f64 / 20.0
}

/// Strength to-hit bonus `str_plus[]` (report §4.2, `fight.c:46`). Only
/// strengths 3..=31 are reachable; monsters use 10 and get 0.
pub fn str_plus(strength: i32) -> i32 {
    match strength {
        0 => -7,
        1 => -6,
        2 => -5,
        3 => -4,
        4 => -3,
        5 => -2,
        6 => -1,
        7..=16 => 0,
        17..=20 => 1,
        21..=30 => 2,
        31 => 3,
        _ => 0,
    }
}

/// Strength damage bonus `add_dam[]` (report §5.1, `fight.c:54`).
pub fn add_dam(strength: i32) -> i32 {
    match strength {
        0 => -7,
        1 => -6,
        2 => -5,
        3 => -4,
        4 => -3,
        5 => -2,
        6 => -1,
        7..=15 => 0,
        16..=17 => 1,
        18 => 2,
        19..=20 => 3,
        21 => 4,
        22..=30 => 5,
        31 => 6,
        _ => 0,
    }
}

/// Parse a damage string into its attack segments (report §5.1): `'x'`
/// separates dice from sides (`"2x4"` = 2d4), `'/'` separates independent
/// attacks each with its own to-hit roll. A segment without leading digits
/// reads as 0 (atoi semantics), so the flytrap's `"%%%x0"` parses as `0x0`.
pub fn parse_damage(s: &str) -> Vec<Damage> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let dice = atoi(rest);
        let Some(x_at) = rest.find('x') else { break };
        let sides = atoi(&rest[x_at + 1..]);
        out.push(Damage { dice, sides });
        let after = &rest[x_at + 1..];
        let Some(slash) = after.find('/') else { break };
        rest = &after[slash + 1..];
    }
    out
}

/// `atoi` semantics: the leading run of digits, or 0 if none.
fn atoi(s: &str) -> u32 {
    s.chars()
        .take_while(|c| c.is_ascii_digit())
        .fold(0, |n, c| n * 10 + (c as u32 - '0' as u32))
}

/// The full attack sequence (`fight.c:391 roll_em`): each `/`-separated
/// damage segment swings independently; a hit deals
/// `dplus + roll(N,S) + add_dam[str]`, floored at zero so combat never heals.
///
/// The +4 "defender not running" bonus (report §4.4) applies here whenever
/// `def_running` is false. The player's own attack wakes its target
/// ([`player_attack`] calls runto first), so the bonus effectively only ever
/// helps monsters — and only against disabled (frozen/sleeping) players.
pub fn roll_em(rng: &mut Rng, spec: &AttackSpec, def_arm: i32, def_running: bool) -> HitResult {
    let wplus = spec.hplus + str_plus(spec.strength) + if def_running { 0 } else { 4 };
    let mut result = HitResult { hits: 0, damage: 0 };
    for seg in parse_damage(spec.damage) {
        if swing(rng, spec.level, def_arm, wplus) {
            let damage = spec.dplus + roll(rng, seg.dice, seg.sides) + add_dam(spec.strength);
            result.damage += damage.max(0);
            result.hits += 1;
        }
    }
    result
}

/// The player attacking a monster it stepped into (`fight.c:64 fight`):
/// wake the target (runto) before any swing, roll the attack, and on a kill
/// grant experience and check for level-ups.
pub fn player_attack(rng: &mut Rng, player: &mut Player, monster: &mut Monster, weapon: &Weapon) -> HitResult {
    // runto(mp): the target is running by the time the swing is rolled, so
    // the player never benefits from the +4 "not running" bonus (§4.4).
    monster.running = true;
    let spec = AttackSpec {
        level: player.level,
        strength: player.strength,
        damage: weapon.damage,
        hplus: weapon.hplus,
        dplus: weapon.dplus,
    };
    let result = roll_em(rng, &spec, monster.armor_class, monster.running);
    monster.hp -= result.damage;
    if monster.hp <= 0 {
        killed(player, monster, rng);
    }
    result
}

/// A monster attacking the player (`fight.c:140 attack`): its table damage
/// string, no weapon or strength bonuses. Returns the hit result so the UI
/// can describe the blow.
pub fn monster_attack(rng: &mut Rng, monster: &Monster, player: &mut Player, player_arm: i32) -> HitResult {
    let spec = AttackSpec {
        level: monster.level,
        strength: 10, // monsters use strength 10: no bonuses (report §5.4)
        damage: monster.damage_string(),
        hplus: 0,
        dplus: 0,
    };
    let result = roll_em(rng, &spec, player_arm, player.running);
    player.hp = (player.hp - result.damage).max(0);
    result
}

/// A kill (`fight.c:629 killed`): grant the monster's *current* (scaled)
/// experience, then zero the monster out. No drops yet — the items task
/// follows. Level-ups are checked right after the exp grant.
pub fn killed(player: &mut Player, monster: &mut Monster, rng: &mut Rng) {
    player.experience += monster.exp;
    monster.hp = 0; // zeroed; the game loop removes it from the level
    check_level(player, rng);
}

/// The level for `exp` (report §6.4): level N requires `exp >= e_levels[N-1]`,
/// capped at [`MAX_LEVEL`] (level 21, at 8,000,000 exp).
pub fn level_for_exp(exp: i32) -> u32 {
    let mut level: u32 = 1;
    for &threshold in &E_LEVELS {
        if threshold > exp {
            break;
        }
        level += 1;
    }
    level.min(MAX_LEVEL)
}

/// Level-up bookkeeping (`misc.c:323 check_level`): recompute the level from
/// experience; each level gained adds `roll(gained, 10)` to BOTH max and
/// current HP. Strength never changes on level-up in 5.4.4. Returns the
/// number of levels gained.
pub fn check_level(player: &mut Player, rng: &mut Rng) -> u32 {
    let target = level_for_exp(player.experience);
    let gained = target.saturating_sub(player.level);
    if gained > 0 {
        let add = roll(rng, gained, 10);
        player.max_hp += add;
        player.hp += add;
        player.level = target;
    }
    gained
}

/// Monster depth scaling (`monsters.c:60 new_monster`): `lev_add` extra
/// levels of toughness per floor below the Amulet level (deeper than 26).
/// Floors 1–26 spawn monsters at their table stats.
pub fn lev_add(floor: u32) -> u32 {
    floor.saturating_sub(AMULET_LEVEL)
}

/// The `exp_add` term (report §6.2, `monsters.c:98`): extra experience from
/// the rolled max HP, scaled up for high-level monsters.
pub fn exp_add(level: u32, max_hp: i32) -> i32 {
    let mut mod_ = if level == 1 { max_hp / 8 } else { max_hp / 6 };
    if level > 9 {
        mod_ *= 20;
    } else if level > 6 {
        mod_ *= 4;
    }
    mod_
}

/// One monster's attack this phase, in the order the monsters were listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterAttack {
    pub name: &'static str,
    pub result: HitResult,
}

/// What the AFTER-phase runners daemon did this round.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonsterPhaseReport {
    /// True if any monster swung this phase (drives the quiet counter).
    pub any_attack: bool,
    /// Every attack, in monster order (only monsters that reached the player).
    pub attacks: Vec<MonsterAttack>,
}

/// Is `monster` orthogonally adjacent to `player`?
pub fn adjacent(monster: &Monster, player: &Player) -> bool {
    monster.x.abs_diff(player.x) + monster.y.abs_diff(player.y) == 1
}

/// Manhattan distance between `monster` and `player`.
pub fn distance(monster: &Monster, player: &Player) -> u32 {
    monster.x.abs_diff(player.x) as u32 + monster.y.abs_diff(player.y) as u32
}

/// Wake sleeping ISMEAN monsters the player can see (`monsters.c:145
/// wake_monster`): each wakes with 2/3 probability. Only ISMEAN monsters
/// wake and chase; the rest stay put (greedy monsters chase gold, which
/// isn't implemented yet). Monsters the player attacked are woken by runto
/// in [`player_attack`] instead.
pub fn wake_monsters(rng: &mut Rng, player: &Player, monsters: &mut [Monster]) {
    for monster in monsters.iter_mut().filter(|m| m.hp > 0 && !m.running && m.is_mean()) {
        if distance(monster, player) <= SIGHT_RANGE && rng.chance(2, 3) {
            monster.running = true;
        }
    }
}

/// One chase step (`chase.c:115 do_chase`): move along the axis of greatest
/// separation toward the player; if that tile is blocked, try the other
/// axis. The caller handles attacks, so this never steps onto the player.
pub fn do_chase(map: &Dungeon, monster: &mut Monster, player: &Player) {
    let dx = player.x as i64 - monster.x as i64;
    let dy = player.y as i64 - monster.y as i64;
    let horizontal = dx.abs() >= dy.abs();
    let primary = if horizontal { (dx.signum(), 0) } else { (0, dy.signum()) };
    let secondary = if horizontal { (0, dy.signum()) } else { (dx.signum(), 0) };
    for (mx, my) in [primary, secondary] {
        let nx = monster.x as i64 + mx;
        let ny = monster.y as i64 + my;
        if nx < 0 || ny < 0 || nx >= map.width as i64 || ny >= map.height as i64 {
            continue;
        }
        if !map.tile(nx as usize, ny as usize).is_walkable() {
            continue;
        }
        monster.x = nx as usize;
        monster.y = ny as usize;
        return;
    }
}

/// The AFTER-phase `runners` daemon (`chase.c:26 runners`): wake ISMEAN
/// monsters the player sees, then every running monster chases the player
/// and attacks when adjacent. Returns a report of what happened.
pub fn monster_phase(
    rng: &mut Rng,
    map: &Dungeon,
    player: &mut Player,
    monsters: &mut [Monster],
    player_arm: i32,
) -> MonsterPhaseReport {
    wake_monsters(rng, player, monsters);

    let mut report = MonsterPhaseReport::default();
    for monster in monsters.iter_mut().filter(|m| m.hp > 0 && m.running) {
        if adjacent(monster, player) {
            let result = monster_attack(rng, monster, player, player_arm);
            report.any_attack = true;
            report.attacks.push(MonsterAttack {
                name: monster.name(),
                result,
            });
        } else {
            do_chase(map, monster, player);
        }
    }
    report
}

/// AFTER-phase healing (`daemons.c:21 doctor`): only after enough quiet
/// rounds (combat resets the counter) does the player recover. Levels 1–7
/// heal +1 once `quiet + 2*level > 20`; level 8+ heals `rnd(level-7)+1`
/// once `quiet >= 3`. Never exceeds max HP.
pub fn doctor(rng: &mut Rng, player: &mut Player, quiet: u32) {
    let heal = if player.level < 8 {
        if quiet + 2 * player.level > 20 {
            1
        } else {
            0
        }
    } else if quiet >= 3 {
        rnd(rng, player.level - 7) as i32 + 1
    } else {
        0
    };
    player.hp = (player.hp + heal).min(player.max_hp);
}

/// AFTER-phase hunger (`daemons.c stomach`): no food system yet, so this is
/// a documented no-op that keeps the round's phase order explicit.
pub fn hunger(_player: &mut Player) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::MonsterKind;
    use crate::map::Tile;

    /// A monster standing at full health, table stats, asleep.
    fn monster(kind: MonsterKind, x: usize, y: usize) -> Monster {
        let base = kind.stats();
        Monster {
            kind,
            x,
            y,
            hp: 100,
            max_hp: 100,
            level: base.level,
            armor_class: base.armor_class,
            exp: base.exp,
            running: false,
        }
    }

    #[test]
    fn strength_bonus_tables_match_the_report() {
        // §4.2 str_plus[] and §5.1 add_dam[]: monsters (10) and the starting
        // player (16) get no bonus; 16 is +1 damage; 31 is +3 hit / +6 damage.
        assert_eq!(str_plus(10), 0);
        assert_eq!(str_plus(16), 0);
        assert_eq!(str_plus(3), -4);
        assert_eq!(str_plus(17), 1);
        assert_eq!(str_plus(21), 2);
        assert_eq!(str_plus(31), 3);
        assert_eq!(add_dam(10), 0);
        assert_eq!(add_dam(7), 0);
        assert_eq!(add_dam(16), 1);
        assert_eq!(add_dam(18), 2);
        assert_eq!(add_dam(21), 4);
        assert_eq!(add_dam(31), 6);
    }

    #[test]
    fn swing_probability_boundaries() {
        // §10: player vs orc 8/20, orc vs player 7/20, §10.C sleeping player 11/20.
        assert_eq!(hit_chance(1, 6, 1), 8.0 / 20.0);
        assert_eq!(hit_chance(1, 6, 0), 7.0 / 20.0);
        assert_eq!(hit_chance(1, 6, 4), 11.0 / 20.0);
        // need <= 0 → automatic hit; need > 19 → automatic miss.
        assert_eq!(hit_chance(20, 10, 0), 1.0);
        assert_eq!(hit_chance(1, -2, 0), 0.0); // black unicorn: impossible at level 1
    }

    #[test]
    fn swing_rolls_obey_the_probability() {
        let mut rng = Rng::new(1234);
        let n = 200_000;
        let hits = (0..n).filter(|_| swing(&mut rng, 1, 6, 1)).count();
        let rate = hits as f64 / n as f64;
        assert!((rate - 0.4).abs() < 0.01, "empirical hit rate {rate}");

        // Boundary behavior is exact for any dice: automatic hit and miss.
        let mut rng = Rng::new(7);
        assert!((0..1000).all(|_| swing(&mut rng, 20, 10, 0)));
        assert!((0..1000).all(|_| !swing(&mut rng, 1, -2, 0)));
    }

    #[test]
    fn damage_strings_parse_into_attack_segments() {
        assert_eq!(parse_damage("2x4"), vec![Damage::new(2, 4)]);
        // A multi-attack monster: three independent attacks.
        assert_eq!(
            parse_damage("1x8/1x8/2x6"),
            vec![Damage::new(1, 8), Damage::new(1, 8), Damage::new(2, 6)]
        );
        assert_eq!(parse_damage("0x0"), vec![Damage::new(0, 0)]);
        assert_eq!(
            parse_damage("0x0/0x0"),
            vec![Damage::new(0, 0), Damage::new(0, 0)]
        );
        // The flytrap's placeholder parses as 0x0 (atoi reads no digits).
        assert_eq!(parse_damage("%%%x0"), vec![Damage::new(0, 0)]);
        assert_eq!(parse_damage(""), Vec::<Damage>::new());
    }

    #[test]
    fn roll_follows_rnd_semantics() {
        let mut rng = Rng::new(3);
        assert_eq!(roll(&mut rng, 0, 6), 0);
        assert_eq!(roll(&mut rng, 2, 0), 0);
        for _ in 0..1000 {
            let v = roll(&mut rng, 1, 6);
            assert!((1..=6).contains(&v), "1d6 rolled {v}");
        }
        let mut rng = Rng::new(9);
        assert_eq!(roll(&mut rng, 5, 1), 5);
        assert_eq!(rnd(&mut rng, 0), 0);
    }

    #[test]
    fn levels_follow_the_e_levels_walk() {
        assert_eq!(level_for_exp(0), 1);
        assert_eq!(level_for_exp(9), 1);
        assert_eq!(level_for_exp(10), 2); // e_levels[0]
        assert_eq!(level_for_exp(19), 2);
        assert_eq!(level_for_exp(20), 3); // e_levels[1]
        assert_eq!(level_for_exp(39), 3);
        assert_eq!(level_for_exp(40), 4); // e_levels[2]
        assert_eq!(level_for_exp(79), 4);
        assert_eq!(level_for_exp(80), 5); // e_levels[3]
        assert_eq!(level_for_exp(7_999_999), 20);
        assert_eq!(level_for_exp(8_000_000), MAX_LEVEL); // the cap
        assert_eq!(E_LEVELS.len(), 20);
    }

    #[test]
    fn level_up_adds_hp_to_both_pools_and_not_strength() {
        let mut player = Player::new(1, 1);
        player.experience = 19; // e_levels[0] = 10 → exactly one level gained
        let mut rng = Rng::new(42);
        let before = (player.max_hp, player.hp, player.strength);
        let gained = check_level(&mut player, &mut rng);
        assert_eq!(gained, 1);
        assert_eq!(player.level, 2);
        let add = player.max_hp - before.0;
        assert!((1..=10).contains(&add), "roll(1,10) gave {add}");
        assert_eq!(player.hp - before.1, add, "current HP grows with max HP");
        assert_eq!(player.strength, before.2, "no strength gain in 5.4.4");
    }

    #[test]
    fn a_big_kill_can_gain_many_levels() {
        let mut player = Player::new(1, 1);
        player.experience = 300; // 10,20,40,80,160 ≤ 300 < 320 → level 6
        let mut rng = Rng::new(5);
        let gained = check_level(&mut player, &mut rng);
        assert_eq!(gained, 5);
        assert_eq!(player.level, 6);
        let add = player.max_hp - 12;
        assert!((5..=50).contains(&add), "roll(5,10) gave {add}");
        assert_eq!(player.hp - 12, add);
    }

    #[test]
    fn monsters_scale_below_level_26() {
        // §6.2: floors 1–26 use table stats; each floor deeper adds a level.
        assert_eq!(lev_add(1), 0);
        assert_eq!(lev_add(26), 0); // the Amulet level itself
        assert_eq!(lev_add(27), 1);
        assert_eq!(lev_add(30), 4);
        // exp_add: lvl 1 → maxhp/8; lvl 7 → maxhp/6 ×4; lvl 10 → maxhp/6 ×20.
        assert_eq!(exp_add(1, 6), 0);
        assert_eq!(exp_add(1, 8), 1);
        assert_eq!(exp_add(7, 60), 40);
        assert_eq!(exp_add(10, 60), 200);
    }

    #[test]
    fn spawn_rolls_hp_and_scales_stats() {
        let mut rng = Rng::new(11);
        let m = Monster::spawn(MonsterKind::Kestrel, 2, 3, 1, &mut rng);
        assert_eq!((m.x, m.y), (2, 3));
        assert_eq!(m.level, 1);
        assert_eq!(m.armor_class, 7);
        assert!(m.max_hp == m.hp && (1..=8).contains(&m.hp), "hp = roll(1,8)");
        assert!((1..=2).contains(&m.exp), "1 + maxhp/8 = {}", m.exp);
        assert!(!m.running, "freshly spawned monsters are asleep");

        // Deeper than 26: +1 level, +1d8 HP, −1 AC, +10 exp (+ exp_add).
        let mut rng = Rng::new(11);
        let deep = Monster::spawn(MonsterKind::Kestrel, 2, 3, 27, &mut rng);
        assert_eq!(deep.level, 2);
        assert_eq!(deep.armor_class, 6);
        assert_eq!(deep.max_hp, deep.hp);
        assert!(deep.max_hp >= 2 && deep.max_hp <= 16, "roll(2,8)");
        assert!(deep.exp >= 11, "1 + 10 + exp_add = {}", deep.exp);
    }

    /// Report §10, both sides of the fight, every die predicted from the same
    /// seed so the assertion is exact rather than statistical.
    #[test]
    fn golden_work_example_orc_fight() {
        let seed = 99u64;
        let mut rng = Rng::new(seed);
        let mut probe = Rng::new(seed); // replay the same stream

        // §10 setup: level-1 player (str 16, HP 12/12, exp 0) with the mace
        // +1,+1; an orc spawned on level 1 with HP 6 ("say roll(1,8) = 6").
        let mut player = Player::new(1, 1);
        let mut orc = Monster {
            kind: MonsterKind::Orc,
            x: 2,
            y: 1,
            hp: 6,
            max_hp: 6,
            level: 1,
            armor_class: 6,
            exp: 5,
            running: false,
        };

        // A. The player attacks: need 13, wplus 1 (mace + str_plus[16] = 0).
        let res = probe.below(20) as i32;
        let hit = res + 1 >= 13; // P = 8/20
        let predicted_damage = if hit {
            1 + (probe.below(4) as i32 + 1) + (probe.below(4) as i32 + 1) + add_dam(16)
        } else {
            0
        };

        let result = player_attack(&mut rng, &mut player, &mut orc, &STARTING_WEAPON);
        assert_eq!(result.hits, u32::from(hit));
        assert_eq!(result.damage, predicted_damage);
        assert!(orc.running, "runto wakes the target before the swing");
        assert_eq!(orc.hp, (6 - predicted_damage).max(0));

        if hit && predicted_damage >= 6 {
            // The orc died during the player's action: exp granted, no counter.
            assert_eq!(orc.hp, 0, "killed() zeroes the monster");
            assert_eq!(player.experience, 5);
            assert_eq!(player.level, 1, "5 < e_levels[0] = 10, no level-up");
            assert_eq!(player.hp, 12);
        } else {
            // B. The orc survived and counterattacks: no bonuses, the player
            // is awake (no +4). need 13, wplus 0 → P = 7/20.
            assert_eq!(orc.hp, 6 - predicted_damage);
            assert_eq!(player.experience, 0);
            let res2 = probe.below(20) as i32;
            let hit2 = res2 >= 13;
            let predicted_damage2 = if hit2 { probe.below(8) as i32 + 1 } else { 0 };
            let result2 = monster_attack(&mut rng, &orc, &mut player, STARTING_ARMOR_AC);
            assert_eq!(result2.damage, predicted_damage2);
            assert_eq!(player.hp, 12 - predicted_damage2);
        }
    }

    #[test]
    fn defender_not_running_bonus_is_monster_side() {
        // The +4 applies in roll_em whenever the defender is disabled...
        let spec = AttackSpec {
            level: 1,
            strength: 16,
            damage: "1x4",
            hplus: 0,
            dplus: 0,
        };
        let mut rng = Rng::new(21);
        let n = 100_000;
        let running = (0..n)
            .filter(|_| roll_em(&mut rng, &spec, 6, true).hits > 0)
            .count() as f64
            / n as f64;
        let frozen = (0..n)
            .filter(|_| roll_em(&mut rng, &spec, 6, false).hits > 0)
            .count() as f64
            / n as f64;
        assert!((running - 7.0 / 20.0).abs() < 0.01);
        assert!((frozen - 11.0 / 20.0).abs() < 0.01);
        // ...but player_attack wakes its target first, so the player's swing
        // never carries the bonus (the golden test asserts this ordering).
        let mut rng = Rng::new(5);
        let mut player = Player::new(1, 1);
        let mut target = monster(MonsterKind::Orc, 2, 1);
        player_attack(&mut rng, &mut player, &mut target, &STARTING_WEAPON);
        assert!(target.running, "runto before roll_em (§4.4)");
    }

    #[test]
    fn only_mean_monsters_wake_on_sight() {
        let player = Player::new(5, 5);
        let trials = 600;
        let mut woke_mean = 0;
        let mut woke_passive = 0;
        for seed in 0..trials {
            let mut rng = Rng::new(seed);
            let mut monsters = [
                monster(MonsterKind::Kestrel, 5, 6), // ISMEAN, adjacent
                monster(MonsterKind::Orc, 4, 5),     // not ISMEAN (greedy)
            ];
            wake_monsters(&mut rng, &player, &mut monsters);
            if monsters[0].running {
                woke_mean += 1;
            }
            if monsters[1].running {
                woke_passive += 1;
            }
        }
        let rate = woke_mean as f64 / trials as f64;
        assert!((rate - 2.0 / 3.0).abs() < 0.1, "ISMEAN wake rate {rate}");
        assert_eq!(woke_passive, 0, "non-ISMEAN monsters never wake");
    }

    /// An all-floor level so chase steps are never blocked.
    fn open_map(width: usize, height: usize) -> Dungeon {
        Dungeon::from_tiles(width, height, &vec![Tile::Floor; width * height])
    }

    #[test]
    fn running_monsters_attack_when_adjacent_and_chase_otherwise() {
        let map = open_map(6, 3);
        let mut rng = Rng::new(8);
        let mut player = Player::new(1, 1);
        let mut monsters = vec![
            monster(MonsterKind::Kestrel, 2, 1),  // adjacent
            monster(MonsterKind::Hobgoblin, 4, 1), // two tiles away
        ];
        for m in &mut monsters {
            m.running = true;
        }
        let report = monster_phase(&mut rng, &map, &mut player, &mut monsters, STARTING_ARMOR_AC);
        assert!(report.any_attack);
        assert_eq!(report.attacks.len(), 1, "only the adjacent kestrel attacks");
        assert_eq!(report.attacks[0].name, "kestrel");
        assert_eq!(monsters[0].x, 2, "an attacker never moves");
        assert_eq!(monsters[1].x, 3, "the hobgoblin stepped one tile closer");
        assert!(player.hp <= 12);
    }

    #[test]
    fn chase_is_blocked_by_walls() {
        // Player west of a monster with a wall in between: it cannot step.
        let map = Dungeon::from_tiles(
            4,
            3,
            &[
                Tile::Wall, Tile::Wall, Tile::Wall, Tile::Wall, //
                Tile::Floor, Tile::Floor, Tile::Wall, Tile::Floor, //
                Tile::Wall, Tile::Wall, Tile::Wall, Tile::Wall, //
            ],
        );
        let mut rng = Rng::new(4);
        let mut player = Player::new(0, 1);
        let mut monsters = vec![monster(MonsterKind::Hobgoblin, 3, 1)];
        monsters[0].running = true;
        monster_phase(&mut rng, &map, &mut player, &mut monsters, STARTING_ARMOR_AC);
        assert_eq!(monsters[0].x, 3, "a walled-off monster cannot approach");
        assert_eq!(monsters[0].y, 1);
    }

    #[test]
    fn doctor_heals_only_after_quiet_rounds() {
        let mut rng = Rng::new(2);
        let mut player = Player::new(1, 1);
        player.hp = 6;
        // Level 1 heals +1 once quiet + 2 > 20, i.e. quiet >= 19.
        doctor(&mut rng, &mut player, 18);
        assert_eq!(player.hp, 6);
        doctor(&mut rng, &mut player, 19);
        assert_eq!(player.hp, 7);
        // Never above max HP.
        player.hp = player.max_hp;
        doctor(&mut rng, &mut player, 1000);
        assert_eq!(player.hp, player.max_hp);
        // Level 10 heals 1..=4 once quiet >= 3.
        player.level = 10;
        player.hp = 10;
        doctor(&mut rng, &mut player, 2);
        assert_eq!(player.hp, 10);
        doctor(&mut rng, &mut player, 3);
        assert!(player.hp > 10 && player.hp <= 14);
    }
}
