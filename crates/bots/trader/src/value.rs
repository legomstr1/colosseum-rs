//! A smooth valuation of a player's position, used to price trades, bids and
//! investments in "spectators" (the unit the game is won in).

use colosseum_engine::{Asset, Assets, Game, PlayerId};

/// Which star awards a player would hold.
pub type StarSet = [bool; Asset::COUNT];

/// Who would hold `asset`'s star if the listed players had the given counts
/// of it (everyone else unchanged). Mirrors the engine's award rule.
pub fn star_holder(game: &Game, asset: Asset, over: &[(PlayerId, u32)]) -> Option<PlayerId> {
    let threshold = game.rules().stars.threshold;
    let count = |p: PlayerId| {
        over.iter()
            .find(|(q, _)| *q == p)
            .map_or_else(|| game.player(p).assets.count(asset), |(_, n)| *n)
    };
    let mut max = 0;
    let mut leaders = 0;
    let mut leader = PlayerId(0);
    for pl in game.players() {
        let c = count(pl.id);
        if c > max {
            max = c;
            leaders = 1;
            leader = pl.id;
        } else if c == max {
            leaders += 1;
        }
    }
    let leader = (max >= threshold && leaders == 1).then_some(leader);
    let holder = game.stars().get(&asset).copied().flatten();
    match holder {
        Some(h) if count(h) >= threshold => match leader {
            Some(l) if count(l) > count(h) => Some(l),
            _ => Some(h),
        },
        _ => leader,
    }
}

pub fn stars_now(game: &Game, p: PlayerId) -> StarSet {
    let mut out = [false; Asset::COUNT];
    for (a, h) in game.stars() {
        out[a.index()] = *h == Some(p);
    }
    out
}

/// Tuning hook: `TRADER_TUNE="coin=0.25,rho=0.4"` overrides a default.
pub fn tune(name: &str, default: f64) -> f64 {
    static TABLE: std::sync::OnceLock<Vec<(String, f64)>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::env::var("TRADER_TUNE")
            .unwrap_or_default()
            .split(',')
            .filter_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                Some((k.trim().to_string(), v.trim().parse().ok()?))
            })
            .collect()
    });
    table.iter().find(|(k, _)| k == name).map_or(default, |(_, v)| *v)
}

/// Tunable weights. They are heuristics, not rules.
pub struct Weights {
    pub second: f64,
    pub rest: f64,
    pub coin: f64,
    pub token: f64,
    pub joker: f64,
    pub short_square: f64,
}

pub fn weights(game: &Game) -> Weights {
    let last = game.round() >= game.rules().num_rounds;
    Weights {
        second: if last { 0.0 } else { tune("second", 0.35) },
        rest: if last { 0.0 } else { tune("rest", 0.1) },
        coin: if last { tune("coinlast", 0.03) } else { tune("coin", 0.22) },
        token: tune("token", 0.1),
        joker: tune("joker", 0.35),
        short_square: tune("short", 0.15),
    }
}

/// Worth of holding `assets` and `coins` for player `p`, given the star
/// awards they would hold and an arena of `arena_len` squares. `extra` adds
/// a program the player does not own yet.
#[allow(clippy::too_many_arguments)]
pub fn worth(
    game: &Game,
    w: &Weights,
    p: PlayerId,
    assets: &Assets,
    coins: u32,
    stars: &StarSet,
    arena_len: u8,
    extra: Option<u8>,
) -> f64 {
    let rules = game.rules();
    let me = game.player(p);
    let mut best = 0.0f64;
    let mut second = 0.0f64;
    let mut rest = 0.0f64;
    let ids = me.programs.iter().map(|q| q.id).chain(extra);
    for id in ids {
        let Some(def) = game.config().event(id) else { continue };
        let mut missing = 0u32;
        let mut star_hits = 0u32;
        let mut len = 0u32;
        for (a, n) in def.requirement_counts().iter() {
            let have = assets.count(a).min(n);
            missing += n - have;
            len += n;
            if have > 0 && stars[a.index()] {
                star_hits += 1;
            }
        }
        let jokers = assets.count(Asset::Joker).min(missing);
        missing -= jokers;
        let mut s = match def.score_with_missing(missing as usize) {
            Some(b) => (b + star_hits * rules.scoring.star_award_bonus) as f64,
            None => def.base_score as f64 * 0.25 * (len - missing.min(len)) as f64 / len.max(1) as f64,
        };
        let need = rules
            .arena
            .squares_required_by_event_size
            .get(def.size as usize)
            .copied()
            .unwrap_or(0);
        let short = need.saturating_sub(arena_len) as f64;
        s *= (1.0 - w.short_square * short).max(0.2);
        if s > best {
            rest += second;
            second = best;
            best = s;
        } else if s > second {
            rest += second;
            second = s;
        } else {
            rest += s;
        }
    }
    let mut tokens = 0.0;
    for (a, n) in assets.iter() {
        tokens += n as f64 * if a == Asset::Joker { w.joker } else { w.token };
    }
    best + w.second * second + w.rest * rest + w.coin * coins as f64 + tokens
}

/// `worth` for the player's real holdings.
pub fn worth_now(game: &Game, w: &Weights, p: PlayerId) -> f64 {
    let me = game.player(p);
    worth(game, w, p, &me.assets, me.coins, &stars_now(game, p), me.arena.len, None)
}
