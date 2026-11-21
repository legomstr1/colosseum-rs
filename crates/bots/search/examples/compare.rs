//! How does the search bot play differently from its base bot?
//!
//! Plays one search bot against three copies of its base bot and reports
//! (1) at each kind of searched decision, how often the search overrode
//! the base bot and with what, and (2) end-of-game stats per side.
//!
//! cargo run --release -p colosseum-bot-search --example compare -- [games] [rollout bot]

use colosseum_bot_search::{Config, Decision, SearchAgent, make_bot};
use colosseum_engine::agent::Agent;
use colosseum_engine::rng::Rng;
use colosseum_engine::{Game, GameConfig, GameEvent};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default, Clone)]
struct Side {
    seats: f64,
    wins: f64,
    score: f64,
    coins: f64,
    arena: f64,
    loge: f64,
    tickets: f64,
    medals: f64,
    programs: f64,
    auctions_won: f64,
    paid: f64,
    trades_done: f64,
    events: f64,
    final_round_buy: f64,
}

impl Side {
    fn row(&self, name: &str) {
        let n = self.seats.max(1.0);
        println!(
            "{name:7} {:>5.1}% {:>6.1} {:>6.1} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>6.2} {:>6.1} {:>6.2} {:>6.2} {:>6.2}",
            100.0 * self.wins / n,
            self.score / n,
            self.coins / n,
            self.arena / n,
            self.loge / n,
            self.tickets / n,
            self.medals / n,
            self.programs / n,
            self.auctions_won / n,
            self.paid / self.auctions_won.max(1.0),
            self.trades_done / n,
            self.events / n,
            self.final_round_buy / n,
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(64);
    let rollout = args.get(2).cloned().unwrap_or_else(|| "house".into());
    let config = Arc::new(GameConfig::embedded());
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let next = AtomicUsize::new(0);
    let results = Mutex::new((Side::default(), Side::default(), Vec::<Decision>::new()));

    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= games {
                        break;
                    }
                    let seed = 9000 + i as u64;
                    let seat = Rng::new(seed).below(4);
                    let mut game = Game::new(config.clone(), 4, seed).unwrap();
                    let cfg = Config { rollout: rollout.clone(), ..Config::default() };
                    let mut search = SearchAgent::with_config(seed, cfg);
                    let mut others: Vec<Box<dyn Agent>> = (0..4).map(|k| make_bot("house", seed * 10 + k)).collect();
                    let (mut won, mut paid, mut trades, mut events, mut final_buy) = ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
                    while let Some(p) = game.current_player() {
                        let legal = game.legal_actions();
                        let a = if p.0 == seat { search.choose_action(&game, &legal) } else { others[p.0].choose_action(&game, &legal) };
                        let round = game.round();
                        for ev in game.apply(a).unwrap() {
                            match ev {
                                GameEvent::AuctionWon { player, price, .. } => {
                                    won[player.0] += 1.0;
                                    paid[player.0] += price as f64;
                                }
                                GameEvent::TradeAccepted { from, to, .. } => {
                                    trades[from.0] += 1.0;
                                    trades[to.0] += 1.0;
                                }
                                GameEvent::EventProduced { player, .. } => events[player.0] += 1.0,
                                GameEvent::ProgramBought { player, .. } if round == game.rules().num_rounds => {
                                    final_buy[player.0] += 1.0
                                }
                                _ => {}
                            }
                        }
                    }
                    let winner = game.ranking()[0].0;
                    let mut r = results.lock().unwrap();
                    for (k, p) in game.players().iter().enumerate() {
                        let side = if k == seat { &mut r.0 } else { &mut r.1 };
                        side.seats += 1.0;
                        side.wins += if k == winner { 1.0 } else { 0.0 };
                        side.score += p.best_score as f64;
                        side.coins += p.coins as f64;
                        side.arena += p.arena.len as f64;
                        side.loge += if p.loge { 1.0 } else { 0.0 };
                        side.tickets += p.season_tickets as f64;
                        side.medals += p.medals as f64;
                        side.programs += p.programs.len() as f64;
                        side.auctions_won += won[k];
                        side.paid += paid[k];
                        side.trades_done += trades[k];
                        side.events += events[k];
                        side.final_round_buy += final_buy[k];
                    }
                    r.2.extend(search.decisions().iter().cloned());
                }
            });
        }
    });

    let (search, house, decisions) = results.into_inner().unwrap();
    println!("{games} games, 1 search (rollouts: house me, {rollout} opponents) vs 3 house, 4 players\n");
    println!("End of game, per seat:");
    println!("side      win%  score  coins arena  loge tickt medal progs auctns  price trades events finbuy");
    search.row("search");
    house.row("house");

    println!("\nSearched decisions, and where the search overrode house:");
    let mut by_sit: BTreeMap<&str, (usize, BTreeMap<(String, String), usize>)> = BTreeMap::new();
    for d in &decisions {
        let e = by_sit.entry(d.situation).or_default();
        e.0 += 1;
        if d.base != d.chosen {
            *e.1.entry((d.base.clone(), d.chosen.clone())).or_default() += 1;
        }
    }
    for (sit, (n, changes)) in &by_sit {
        let overridden: usize = changes.values().sum();
        println!("  {sit}: {n} decisions, {overridden} overridden ({:.0}%)", 100.0 * overridden as f64 / *n as f64);
        let mut top: Vec<_> = changes.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        for ((b, c), k) in top.into_iter().take(6) {
            println!("      {k:>5}x  house: {b:<26} search: {c}");
        }
    }

    println!("\nOverrides by round:");
    let mut by_round: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    for d in &decisions {
        let e = by_round.entry(d.round).or_default();
        e.0 += 1;
        e.1 += (d.base != d.chosen) as usize;
    }
    for (r, (n, o)) in by_round {
        println!("  round {r}: {o} of {n} ({:.0}%)", 100.0 * o as f64 / n as f64);
    }
}
