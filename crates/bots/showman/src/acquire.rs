//! Acquiring: bid for batches that fill the gaps of the program we are
//! actually aiming at (the best feasible estimate, not just the biggest).

use crate::invest::{Hyp, estimate};
use colosseum_engine::{Action, Asset, Assets, Game, Phase, PlayerId};

fn env_f(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

pub struct AcqTune {
    pub pay: f64,
    pub keep: u32,
    pub open_min: f64,
}

impl Default for AcqTune {
    fn default() -> Self {
        AcqTune {
            pay: env_f("SH_PAY", 10.0),
            keep: env_f("SH_KEEP", 5.0) as u32,
            open_min: env_f("SH_OPEN", 0.5),
        }
    }
}

/// Tokens still missing for the aimed-at program.
fn deficit(game: &Game, me: PlayerId) -> Assets {
    let p = game.player(me);
    let h = Hyp::of(game, me);
    let (_, target) = estimate(game, me, &h, &p.assets);
    let mut out = Assets::new();
    if let Some(def) = target.and_then(|id| game.config().event(id)) {
        for (a, n) in def.requirement_counts().iter() {
            out.add(a, n.saturating_sub(p.assets.count(a)));
        }
    }
    out
}

fn token_value(tokens: &[Asset], need: &Assets, extra: &Assets) -> f64 {
    let mut need = need.clone();
    let mut extra = extra.clone();
    tokens
        .iter()
        .map(|&a| {
            if need.remove(a, 1) {
                1.0
            } else if a == Asset::Joker {
                0.8
            } else if matches!(a, Asset::Emperor | Asset::SpecialEvent) {
                0.5
            } else if extra.remove(a, 1) {
                0.35
            } else {
                0.1
            }
        })
        .sum()
}

pub fn choose(game: &Game, me: PlayerId, legal: &[Action], t: &AcqTune) -> Action {
    let Phase::Acquiring(acq) = game.phase() else {
        return legal[0].clone();
    };
    let need = deficit(game, me);
    // Needs of the other programs, as a weaker pull.
    let mut extra = Assets::new();
    for pr in &game.player(me).programs {
        if let Some(def) = game.config().event(pr.id) {
            for (a, n) in def.requirement_counts().iter() {
                extra.add(a, n.saturating_sub(game.player(me).assets.count(a)));
            }
        }
    }
    let coins = game.player(me).coins;
    match &acq.auction {
        None => {
            let min = game.rules().acquiring.minimum_opening_bid;
            let best = (0..game.markets().len())
                .filter(|m| legal.contains(&Action::StartAuction { market: *m, bid: min }))
                .map(|m| (token_value(&game.markets()[m], &need, &extra), m))
                .max_by(|a, b| a.0.total_cmp(&b.0));
            match best {
                Some((value, market)) if value >= t.open_min => Action::StartAuction { market, bid: min },
                _ => Action::PassInitiative,
            }
        }
        Some(auction) => {
            let value = token_value(&game.markets()[auction.market], &need, &extra);
            let max_pay = ((value * t.pay) as u32).min(coins.saturating_sub(t.keep));
            let bid = Action::Bid { amount: auction.high_bid + 1 };
            if auction.high_bid < max_pay && legal.contains(&bid) {
                bid
            } else {
                Action::PassBid
            }
        }
    }
}
