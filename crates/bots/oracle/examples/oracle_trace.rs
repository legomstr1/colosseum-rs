//! Diagnostics: per-bot averages of end-of-game holdings in mixed games, and
//! optionally a verbose log of one game.
//!
//! `oracle_trace [games] [log-seed]`

use colosseum_bot_auctioneer::AuctioneerAgent;
use colosseum_bot_oracle::OracleAgent;
use colosseum_bot_planner::PlannerAgent;
use colosseum_bot_trader::TraderAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Action, Game, GameConfig, GameEvent};
use std::collections::BTreeMap;
use std::sync::Arc;

const NAMES: [&str; 4] = ["oracle", "planner", "auctioneer", "trader"];

fn make(name: &str, seed: u64) -> Box<dyn Agent> {
    match name {
        "oracle" => Box::new(OracleAgent::new(seed)),
        "planner" => Box::new(PlannerAgent::new(seed)),
        "auctioneer" => Box::new(AuctioneerAgent::new(seed)),
        _ => Box::new(TraderAgent::new(seed)),
    }
}

#[derive(Default)]
struct S {
    seats: f64,
    wins: f64,
    score: f64,
    coins: f64,
    tickets: f64,
    arena: f64,
    progs: f64,
    produced: f64,
    medals: f64,
    loge: f64,
    podiums: f64,
    bought_tokens: f64,
    sold_tokens: f64,
    coins_paid_trade: f64,
    auction_wins: f64,
    auction_paid: f64,
    finalscore: f64,
    best_prog_cost: f64,
    bd: [f64; 8],
    /// medals: from dice, from Emperor tokens, spent on investments (x2),
    /// spent on noble moves, SE played for investments, SE exchanged.
    md: [f64; 6],
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let games: u64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(500);
    let log_seed: Option<u64> = args.get(1).and_then(|s| s.parse().ok()).filter(|s| *s != 0);
    let pool: Vec<&str> = match args.get(2) {
        Some(p) => p.split(',').map(|x| NAMES.iter().copied().find(|n| *n == x).expect("bot name")).collect(),
        None => NAMES.to_vec(),
    };
    let config = Arc::new(GameConfig::embedded());
    let mut st: BTreeMap<&str, S> = BTreeMap::new();
    // Oracle's margin to the best other player (positive = won), bucketed.
    let mut margins: BTreeMap<i32, u32> = BTreeMap::new();
    for g in 0..games {
        let seed = log_seed.unwrap_or(g * 7 + 3);
        let mut rng = Rng::new(seed ^ 0xFEED);
        let n = 3 + rng.below(3);
        let mut seats: Vec<&str> = (0..n).map(|_| pool[rng.below(pool.len())]).collect();
        seats[rng.below(n)] = "oracle";
        let mut game = Game::new(config.clone(), n, seed).unwrap();
        let mut agents: Vec<Box<dyn Agent>> = seats.iter().enumerate().map(|(i, s)| make(s, seed + i as u64)).collect();
        let mut finals = vec![0u32; n];
        if log_seed.is_some() {
            println!("seats {seats:?}");
        }
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let a = agents[p.0].choose_action(&game, &legal);
            let desc = game.describe_action(&a);
            let round = game.round();
            let ev = game.apply(a.clone()).unwrap();
            if log_seed.is_some() && !matches!(a, Action::Bid { .. } | Action::PassBid) {
                println!("r{round} P{} {:<10} {desc}", p.0 + 1, seats[p.0]);
            }
            {
                let s = st.entry(seats[p.0]).or_default();
                match a {
                    Action::SpendMedalsForInvestment => s.md[2] += 1.0,
                    Action::MedalMoveNoble { .. } => s.md[3] += 1.0,
                    Action::PlaySpecialEvent => s.md[4] += 1.0,
                    Action::ExchangeSpecialEventForMedal => s.md[5] += 1.0,
                    _ => {}
                }
            }
            for e in ev {
                match e {
                    GameEvent::NobleMoved { player, medal_earned: true, .. } => {
                        st.entry(seats[player.0]).or_default().md[0] += 1.0;
                    }
                    GameEvent::EmperorTokenConverted { player, medal_gained: true } => {
                        st.entry(seats[player.0]).or_default().md[1] += 1.0;
                    }
                    GameEvent::TradeAccepted { from, to, terms } => {
                        let f = st.entry(seats[from.0]).or_default();
                        f.bought_tokens += terms.receive.total() as f64;
                        f.sold_tokens += terms.give.total() as f64;
                        f.coins_paid_trade += terms.give_coins as f64 - terms.receive_coins as f64;
                        let t = st.entry(seats[to.0]).or_default();
                        t.bought_tokens += terms.give.total() as f64;
                        t.sold_tokens += terms.receive.total() as f64;
                        t.coins_paid_trade += terms.receive_coins as f64 - terms.give_coins as f64;
                        if log_seed.is_some() {
                            println!("    TRADE {} -> {}: {:?}", seats[from.0], seats[to.0], terms);
                        }
                    }
                    GameEvent::AuctionWon { player, price, assets, .. } => {
                        let s = st.entry(seats[player.0]).or_default();
                        s.auction_wins += 1.0;
                        s.auction_paid += price as f64;
                        if log_seed.is_some() {
                            println!("    AUCTION {} wins {:?} for {price}", seats[player.0], assets);
                        }
                    }
                    GameEvent::EventProduced { player, spectators, ref breakdown, .. } => {
                        if game.round() >= game.rules().num_rounds {
                            finals[player.0] = spectators;
                            let b = breakdown;
                            let s = st.entry(seats[player.0]).or_default();
                            for (i, v) in [b.base, b.previous_programs, b.season_tickets, b.stars, b.nobles, b.podiums, b.medals, b.missing_assets].into_iter().enumerate() {
                                s.bd[i] += v as f64;
                            }
                        }
                        if log_seed.is_some() {
                            println!("    EVENT {} {spectators} {breakdown}", seats[player.0]);
                        }
                    }
                    _ => {}
                }
            }
        }
        let w = game.ranking()[0].0;
        if let Some(o) = seats.iter().position(|s| *s == "oracle") {
            let mine = game.players()[o].best_score as i32;
            let other = (0..n).filter(|&i| i != o).map(|i| game.players()[i].best_score as i32).max().unwrap_or(0);
            let d = mine - other;
            *margins.entry((d.clamp(-30, 30) / 5) * 5).or_insert(0) += 1;
        }
        for i in 0..n {
            let p = &game.players()[i];
            let s = st.entry(seats[i]).or_default();
            s.seats += 1.0;
            s.wins += (i == w) as u32 as f64;
            s.score += p.best_score as f64;
            s.finalscore += finals[i] as f64;
            s.coins += p.coins as f64;
            s.tickets += p.season_tickets as f64;
            s.arena += p.arena.len as f64;
            s.progs += p.programs.len() as f64;
            s.produced += p.programs.iter().filter(|q| q.produced).count() as f64;
            s.medals += p.medals as f64;
            s.loge += p.loge as u32 as f64;
            s.podiums += p.podiums as f64;
            s.best_prog_cost += p.programs.iter().filter_map(|q| config.event(q.id)).map(|e| e.base_score).max().unwrap_or(0) as f64;
        }
        if log_seed.is_some() {
            break;
        }
    }
    if log_seed.is_none() {
        let line: Vec<String> = margins.iter().map(|(k, v)| format!("{k}:{v}")).collect();
        println!("first-oracle margin to best other (5-pt buckets): {}", line.join(" "));
    }
    println!(
        "{:<11} {:>6} {:>6} {:>6} {:>6} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "bot", "seats", "win%", "best", "final", "coins", "tix", "arena", "progs", "prod", "medal", "loge", "podium", "bought", "sold", "paidT", "aucW", "aucPay"
    );
    for (k, s) in &st {
        let d = s.seats.max(1.0);
        println!(
            "{:<11} {:>6.0} {:>6.1} {:>6.1} {:>6.1} {:>5.1} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6.1}  topbase {:.1}",
            k,
            s.seats,
            100.0 * s.wins / d,
            s.score / d,
            s.finalscore / d,
            s.coins / d,
            s.tickets / d,
            s.arena / d,
            s.progs / d,
            s.produced / d,
            s.medals / d,
            s.loge / d,
            s.podiums / d,
            s.bought_tokens / d,
            s.sold_tokens / d,
            s.coins_paid_trade / d,
            s.auction_wins / d,
            s.auction_paid / s.auction_wins.max(1.0),
            s.best_prog_cost / d
        );
        let b: Vec<String> = s.bd.iter().map(|x| format!("{:.1}", x / d)).collect();
        let m: Vec<String> = s.md.iter().map(|x| format!("{:.2}", x / d)).collect();
        println!("            medals dice/emperor/invest(x2)/moves/SE-invest/SE-exch: {}", m.join(" / "));
        println!("            final breakdown base/prev/tix/stars/nobles/podium/medals/missing: {}", b.join(" / "));
    }
}
