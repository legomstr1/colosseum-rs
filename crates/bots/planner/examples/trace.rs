use colosseum_bot_planner::PlannerAgent;
use colosseum_engine::agent::{Agent, GreedyAgent};
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
fn main() {
    let seed: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let config = Arc::new(GameConfig::embedded());
    let n = 3;
    let mut game = Game::new(config, n, seed).unwrap();
    let mut bot = PlannerAgent::new(seed);
    let mut others: Vec<GreedyAgent> = (0..n).map(|i| GreedyAgent::new(seed + i as u64)).collect();
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let a = if p.0 == 0 { bot.choose_action(&game, &legal) } else { others[p.0].choose_action(&game, &legal) };
        let ev = game.apply(a).unwrap();
        for e in ev {
            println!("{e}");
        }
    }
    for p in game.players() {
        println!("{:?} coins {} tickets {} loge {} arena {} progs {:?} best {}", p.id, p.coins, p.season_tickets, p.loge, p.arena.len, p.programs.iter().map(|q| q.id).collect::<Vec<_>>(), p.best_score);
    }
}
