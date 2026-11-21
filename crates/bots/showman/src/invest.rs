//! Investing: score each candidate investment by how much it raises the
//! estimated final event, net of what it costs.

use colosseum_engine::{Action, Assets, Game, PlayerId};

/// Tunable weights (kept in one place; not game data).
pub struct Tune {
    pub coin_weight: f64,
    pub reserve: u32,
    pub loge_bonus: f64,
}

impl Default for Tune {
    fn default() -> Self {
        Tune {
            coin_weight: env_f("SH_COIN", 0.3),
            reserve: env_f("SH_RESERVE", 16.0) as u32,
            loge_bonus: env_f("SH_LOGE", 3.0),
        }
    }
}

fn env_f(name: &str, default: f64) -> f64 {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

pub struct Hyp {
    pub programs: Vec<u8>,
    pub arena_len: u8,
    pub tickets: u32,
    pub loge: bool,
}

impl Hyp {
    pub fn of(game: &Game, me: PlayerId) -> Hyp {
        let p = game.player(me);
        Hyp {
            programs: p.programs.iter().map(|x| x.id).collect(),
            arena_len: p.arena.len,
            tickets: p.season_tickets,
            loge: p.loge,
        }
    }
}

/// Expected acquisitions still ahead (this round's included until it ends).
fn acq_left(game: &Game) -> f64 {
    let rules = game.rules();
    (rules.num_rounds + 1).saturating_sub(game.round()) as f64
}

/// Estimated spectators of the best event the player can aim for.
pub fn estimate(game: &Game, me: PlayerId, h: &Hyp, assets: &Assets) -> (f64, Option<u8>) {
    let rules = game.rules();
    let p = game.player(me);
    let left = acq_left(game);
    let produced = p.programs.iter().filter(|x| x.produced).count() as f64;
    let mut best = (0.0, None);
    for &id in &h.programs {
        let Some(def) = game.config().event(id) else { continue };
        let need_sq = rules.arena.squares_required_by_event_size.get(def.size as usize).copied().unwrap_or(0);
        let max_len = rules.arena.max_squares;
        if need_sq > max_len {
            continue;
        }
        let deficit_sq = need_sq.saturating_sub(h.arena_len) as f64;
        let mut missing = 0u32;
        for (a, n) in def.requirement_counts().iter() {
            missing += n.saturating_sub(assets.count(a));
        }
        missing = missing.saturating_sub(assets.count(colosseum_engine::Asset::Joker));
        // Assume a token or so of useful progress per acquisition left.
        let eff = (missing as f64 - 1.1 * left).max(0.0);
        let lo = eff.floor() as usize;
        let hi = eff.ceil() as usize;
        let s_lo = def.score_with_missing(lo);
        let s_hi = def.score_with_missing(hi);
        let (Some(a), Some(b)) = (s_lo, s_hi) else { continue };
        let frac = eff - lo as f64;
        let base = a as f64 * (1.0 - frac) + b as f64 * frac;
        let sc = &rules.scoring;
        let prev = (produced + (left - 1.0).max(0.0) * 0.5).min(h.programs.len() as f64 - 1.0).max(0.0);
        let total = base + h.tickets as f64 * sc.season_ticket_bonus as f64 + prev * sc.previous_program_bonus as f64 * 0.6
            - deficit_sq * rules.arena.expansion_cost as f64 * 0.4;
        if total > best.0 {
            best = (total, Some(id));
        }
    }
    best
}

pub fn choose(game: &Game, me: PlayerId, legal: &[Action], tune: &Tune) -> Action {
    let rules = game.rules();
    let p = game.player(me);
    let final_round = game.round() >= rules.num_rounds;
    let cw = if final_round { tune.coin_weight * 0.5 } else { tune.coin_weight };
    let before_h = Hyp::of(game, me);
    let (before, _) = estimate(game, me, &before_h, &p.assets);

    let mut best: (f64, Action) = (0.0, Action::EndInvesting);
    for a in legal {
        let mut h = Hyp::of(game, me);
        let cost;
        let mut extra = 0.0;
        match a {
            Action::BuyProgram { program } => {
                let Some(def) = game.config().event(*program) else { continue };
                cost = def.cost;
                h.programs.push(*program);
            }
            Action::ExpandArena { .. } => {
                cost = rules.arena.expansion_cost;
                h.arena_len += 1;
            }
            Action::BuySeasonTicket => {
                cost = rules.investing.season_ticket_cost;
                h.tickets += 1;
            }
            Action::BuildLoge => {
                cost = rules.investing.emperor_loge_cost;
                h.loge = true;
                extra = tune.loge_bonus;
            }
            _ => continue,
        }
        if p.coins < cost + tune.reserve {
            continue;
        }
        let (after, _) = estimate(game, me, &h, &p.assets);
        let delta = after - before + extra - cw * cost as f64;
        if let Action::ExpandArena { side } = a {
            // Prefer squares that hold a resting place, then the nearer one.
            let sq = p.arena.expansion_square(*side, rules.board.size);
            if rules.is_resting_place(sq) {
                let bonus = 0.5;
                if delta + bonus > best.0 {
                    best = (delta + bonus, a.clone());
                }
                continue;
            }
        }
        if delta > best.0 {
            best = (delta, a.clone());
        }
    }
    best.1
}

/// Whether playing a SpecialEvent token for an extra investment pays off.
pub fn special_pays(game: &Game, me: PlayerId, tune: &Tune) -> bool {
    let mut next = game.clone();
    if next.apply(Action::PlaySpecialEvent).is_err() {
        return false;
    }
    if next.current_player() != Some(me) {
        return false;
    }
    let legal = next.legal_actions();
    let a = choose(&next, me, &legal, tune);
    if a == Action::EndInvesting {
        return false;
    }
    true
}
