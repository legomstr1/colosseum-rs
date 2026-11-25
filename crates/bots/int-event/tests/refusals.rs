//! After a partner refuses an offer, int-event never offers them a worse one that round.

use colosseum_bot_int_event::IntEventAgent;
use colosseum_engine::agent::Agent;
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;

/// After a partner refuses an offer, the bot never offers them one that is
/// no better for them (same or fewer tokens given, same or more asked, same
/// or more coins asked) in that round.
#[test]
fn never_follows_a_refusal_with_a_worse_offer() {
    use colosseum_engine::Assets;
    use colosseum_engine::event::GameEvent;
    let within = |a: &Assets, b: &Assets| a.iter().all(|(x, n)| n <= b.count(x));
    let net = |t: &colosseum_engine::TradeTerms| t.receive_coins as i64 - t.give_coins as i64;
    let config = Arc::new(GameConfig::embedded());
    let mut refusals = 0;
    for n in 3..=5 {
        for seed in 0..8u64 {
            let mut game = Game::new(config.clone(), n, seed * 31 + n as u64).unwrap();
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if i % 2 == 0 {
                        Box::new(IntEventAgent::new(seed + i as u64))
                    } else {
                        Box::new(colosseum_bot_house::HouseAgent::new(seed + i as u64))
                    }
                })
                .collect();
            // (round, from, to, terms) of refused offers.
            let mut refused: Vec<(u32, usize, usize, colosseum_engine::TradeTerms)> = Vec::new();
            let mut last: Option<(usize, usize, colosseum_engine::TradeTerms)> = None;
            while let Some(p) = game.current_player() {
                let legal = game.legal_actions();
                let a = agents[p.0].choose_action(&game, &legal);
                if p.0 % 2 == 0
                    && let Action::ProposeTrade { to, terms } = &a
                {
                    for (r, from, t, old) in &refused {
                        let worse = *r == game.round()
                            && *from == p.0
                            && *t == to.0
                            && within(&terms.give, &old.give)
                            && within(&old.receive, &terms.receive)
                            && net(terms) >= net(old);
                        assert!(!worse, "offered {terms:?} after {old:?} was refused");
                    }
                }
                let round = game.round();
                for e in game.apply(a).unwrap() {
                    match e {
                        GameEvent::TradeProposed { from, to, terms } => last = Some((from.0, to.0, terms)),
                        GameEvent::TradeRejected { .. } => {
                            if let Some((from, to, terms)) = last.take() {
                                refusals += 1;
                                refused.push((round, from, to, terms));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    assert!(refusals > 50, "too few refusals to test: {refusals}");
}
