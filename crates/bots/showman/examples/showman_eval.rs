use colosseum_bot_showman::ShowmanAgent;
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Action, Game, GameConfig};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

struct Timed {
    inner: ShowmanAgent,
    nanos: Rc<Cell<u128>>,
    calls: Rc<Cell<u64>>,
}

impl Agent for Timed {
    fn name(&self) -> String {
        "showman".into()
    }
    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        let t = Instant::now();
        let a = self.inner.choose_action(game, legal);
        self.nanos.set(self.nanos.get() + t.elapsed().as_nanos());
        self.calls.set(self.calls.get() + 1);
        a
    }
}

fn main() {
    let games: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(600);
    let config = Arc::new(GameConfig::embedded());
    let nanos = Rc::new(Cell::new(0u128));
    let calls = Rc::new(Cell::new(0u64));
    for opp in ["greedy", "random"] {
        for n in [3usize, 4, 5] {
            let mut wins = 0.0;
            for g in 0..games {
                let seat = (g as usize) % n;
                let seed = 1000 + g * 7919 + n as u64;
                let mut game = Game::new(config.clone(), n, seed).unwrap();
                let mut agents: Vec<Box<dyn Agent>> = (0..n)
                    .map(|i| -> Box<dyn Agent> {
                        if i == seat {
                            Box::new(Timed { inner: ShowmanAgent::new(seed), nanos: nanos.clone(), calls: calls.clone() })
                        } else if opp == "greedy" {
                            Box::new(GreedyAgent::new(seed + i as u64))
                        } else {
                            Box::new(RandomAgent::new(seed + i as u64))
                        }
                    })
                    .collect();
                colosseum_engine::agent::play_out(&mut game, &mut agents);
                if game.ranking()[0].0 == seat {
                    wins += 1.0;
                }
            }
            println!(
                "vs {opp:6} {n}p: win {:5.1}% (fair {:4.1}%)  [{games} games]",
                100.0 * wins / games as f64,
                100.0 / n as f64
            );
        }
    }
    println!(
        "avg {:.1} us/decision over {} decisions",
        nanos.get() as f64 / calls.get() as f64 / 1000.0,
        calls.get()
    );
}
