use colosseum_bot_showman::ShowmanAgent;
use colosseum_engine::agent::{Agent, GreedyAgent};
use colosseum_engine::{Game, GameConfig};
use std::sync::Arc;
fn main() {
    let config = Arc::new(GameConfig::embedded());
    let n = 4;
    let (mut sh, mut gr, mut sa, mut ga) = (0.0, 0.0, 0.0, 0.0);
    let (mut sarena, mut garena, mut sprog, mut gprog, mut smed, mut gmed) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let games = 300;
    for g in 0..games {
        let seat = g % n;
        let seed = 5 + g as u64 * 31;
        let mut game = Game::new(config.clone(), n, seed).unwrap();
        let mut agents: Vec<Box<dyn Agent>> = (0..n).map(|i| -> Box<dyn Agent> {
            if i == seat { Box::new(ShowmanAgent::new(seed)) } else { Box::new(GreedyAgent::new(seed + i as u64)) }
        }).collect();
        colosseum_engine::agent::play_out(&mut game, &mut agents);
        for p in game.players() {
            let isme = p.id.0 == seat;
            let (s, a, pr, m) = if isme { (&mut sh, &mut sa, &mut sprog, &mut smed) } else { (&mut gr, &mut ga, &mut gprog, &mut gmed) };
            *s += p.best_score as f64; *a += p.arena.len as f64; *pr += p.programs.len() as f64; *m += p.medals as f64;
        }
        let _ = (&mut sarena, &mut garena);
    }
    let gf = games as f64;
    println!("showman avg best {:.1} arena {:.2} progs {:.2} medals left {:.2}", sh / gf, sa / gf, sprog / gf, smed / gf);
    println!("greedy  avg best {:.1} arena {:.2} progs {:.2} medals left {:.2}", gr / gf / 3.0, ga / gf / 3.0, gprog / gf / 3.0, gmed / gf / 3.0);
}
