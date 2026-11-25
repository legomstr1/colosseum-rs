//! Int-event mostly follows its plan against house bots: a season ticket,
//! the full arena, a tier-3 event and a tier-4 event by the end. Run with
//! `--nocapture` to see the rates and how its events and podiums went.

use colosseum_bot_int_event::IntEventAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;

#[test]
fn follows_the_plan() {
    let config = Arc::new(GameConfig::embedded());
    let sq = &config.rules.arena.squares_required_by_event_size;
    let max = config.rules.arena.max_squares;
    let games = 60u64;
    let (mut ticket, mut arena, mut mid, mut top, mut podiums, mut extra_tickets, mut wins) = (0, 0, 0, 0, 0, 0, 0);
    for seed in 0..games {
        let n = 4;
        let mut game = Game::new(config.clone(), n, 500 + seed).unwrap();
        let me = (seed % n as u64) as usize;
        let mut agents: Vec<Box<dyn Agent>> = (0..n)
            .map(|i| -> Box<dyn Agent> {
                if i == me {
                    Box::new(IntEventAgent::new(seed))
                } else {
                    Box::new(colosseum_bot_house::HouseAgent::new(seed * 7 + i as u64))
                }
            })
            .collect();
        while let Some(p) = game.current_player() {
            let legal = game.legal_actions();
            let a = agents[p.0].choose_action(&game, &legal);
            game.apply(a).unwrap();
        }
        let pl = &game.players()[me];
        let tier = |s: u8| pl.programs.iter().any(|q| config.event(q.id).is_some_and(|d| sq[d.size as usize] == s && q.produced));
        ticket += (pl.season_tickets > 0) as u32;
        extra_tickets += (pl.season_tickets > 1) as u32;
        arena += (pl.arena.len == max) as u32;
        mid += tier(max - 1) as u32;
        top += tier(max) as u32;
        podiums += pl.podiums;
        wins += (game.ranking()[0].0 == me) as u32;
    }
    eprintln!(
        "of {games}: ticket {ticket}, 2+ tickets {extra_tickets}, full arena {arena}, tier-3 produced {mid}, tier-4 produced {top}, podiums {podiums}, wins {wins}"
    );
    let g = games as u32;
    assert!(ticket * 10 >= g * 9, "season ticket in {ticket}/{g}");
    assert!(arena * 10 >= g * 8, "full arena in {arena}/{g}");
    assert!(mid * 10 >= g * 6, "tier-3 produced in {mid}/{g}");
    assert!(top * 10 >= g * 6, "tier-4 produced in {top}/{g}");
}
