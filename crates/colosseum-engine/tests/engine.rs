use colosseum_engine::agent::{play_out, Agent, GreedyAgent, RandomAgent};
use colosseum_engine::*;
use std::sync::Arc;

fn config() -> Arc<GameConfig> {
    Arc::new(GameConfig::embedded())
}

fn new_game(players: usize, seed: u64) -> Game {
    Game::new(config(), players, seed).unwrap()
}

/// Standard asset tokens anywhere in the game, including discarded ones.
fn standard_tokens(game: &Game) -> u32 {
    let mut all = game.discarded().clone();
    for p in game.players() {
        all.add_all(&p.assets);
    }
    for t in game.markets().iter().flatten().chain(game.green_pool()).chain(game.bag()) {
        all.add(*t, 1);
    }
    all.iter().filter(|(a, _)| a.is_standard()).map(|(_, n)| n).sum()
}

fn check_invariants(game: &Game) {
    let rules = game.rules();
    let total_standard: u32 = rules
        .assets
        .total
        .iter()
        .filter(|(a, _)| a.is_standard())
        .map(|(_, n)| n)
        .sum();
    assert_eq!(standard_tokens(game), total_standard, "standard tokens are conserved");
    let medals: u32 = game.players().iter().map(|p| p.medals).sum::<u32>() + game.supply().medals;
    assert!(medals <= rules.supply.medals, "never more medals than the supply");
    let podiums: u32 = game.players().iter().map(|p| p.podiums).sum::<u32>() + game.supply().podiums;
    assert_eq!(podiums, rules.supply.podiums);
}

/// Ends phases with the simplest action until `pred` holds.
fn advance_until(game: &mut Game, pred: impl Fn(&Game) -> bool) {
    while !pred(game) {
        let action = match &game.phase() {
            Phase::Investing { .. } => Action::EndInvesting,
            Phase::Acquiring(a) if a.auction.is_none() => Action::PassInitiative,
            Phase::Acquiring(_) => Action::PassBid,
            Phase::Trading { offer: None, .. } => Action::EndTrading,
            Phase::Trading { .. } => Action::RejectTrade,
            Phase::Producing(_) => game
                .legal_actions()
                .into_iter()
                .find(|a| matches!(a, Action::MoveNoble { .. } | Action::SkipEvent))
                .unwrap(),
            _ => game.legal_actions()[0].clone(),
        };
        game.apply(action).unwrap();
    }
}

#[test]
fn embedded_config_loads() {
    let c = GameConfig::embedded();
    assert_eq!(c.events.len(), 30);
    assert_eq!(c.supported_player_counts(), vec![3, 4, 5]);
}

#[test]
fn setup_deals_everything() {
    for n in 3..=5 {
        let game = new_game(n, 7);
        let per_player = game.rules().assets.starting_per_player[&n];
        for p in game.players() {
            assert_eq!(p.assets.total(), per_player);
            assert_eq!(p.programs.len(), 2);
            assert_eq!(p.coins, game.rules().starting_coins);
        }
        assert!(game.markets().iter().all(|m| m.len() == game.rules().acquiring.batch_size));
        assert_eq!(game.round(), 1);
        assert!(matches!(game.phase(), Phase::Investing { turn: 0, .. }));
        assert_eq!(game.current_player(), Some(game.start_player()));
        check_invariants(&game);
    }
}

#[test]
fn unsupported_player_count_is_an_error() {
    assert!(Game::new(config(), 2, 0).is_err());
}

#[test]
fn illegal_actions_are_rejected() {
    let mut game = new_game(3, 1);
    let before = game.clone();
    let err = game.apply(Action::Bid { amount: 10 }).unwrap_err();
    assert!(!err.reason.is_empty());
    assert_eq!(game, before, "a rejected action changes nothing");
}

#[test]
fn bot_games_finish_with_invariants_intact() {
    for n in 3..=5 {
        for seed in 0..40 {
            let mut game = new_game(n, seed);
            let mut agents: Vec<Box<dyn Agent>> = (0..n)
                .map(|i| -> Box<dyn Agent> {
                    if (i + seed as usize).is_multiple_of(2) {
                        Box::new(GreedyAgent::new(seed * 10 + i as u64))
                    } else {
                        Box::new(RandomAgent::new(seed * 10 + i as u64))
                    }
                })
                .collect();
            while let Some(p) = game.current_player() {
                let legal = game.legal_actions();
                assert!(!legal.is_empty(), "the current player always has a move");
                let action = agents[p.0].choose_action(&game, &legal);
                game.apply(action).unwrap();
                check_invariants(&game);
            }
            assert!(game.is_over());
            assert_eq!(game.round(), game.rules().num_rounds);
            assert_eq!(game.ranking().len(), n);
        }
    }
}

#[test]
fn greedy_bots_score_more_than_random_ones() {
    let (mut greedy, mut random) = (0u64, 0u64);
    for seed in 0..30 {
        let mut game = new_game(4, seed);
        let mut agents: Vec<Box<dyn Agent>> = vec![
            Box::new(GreedyAgent::new(seed)),
            Box::new(RandomAgent::new(seed + 1)),
            Box::new(GreedyAgent::new(seed + 2)),
            Box::new(RandomAgent::new(seed + 3)),
        ];
        play_out(&mut game, &mut agents);
        greedy += (game.players()[0].best_score + game.players()[2].best_score) as u64;
        random += (game.players()[1].best_score + game.players()[3].best_score) as u64;
    }
    assert!(greedy > random, "greedy {greedy} vs random {random}");
}

#[test]
fn replay_and_serialization_reproduce_the_game() {
    let mut game = new_game(4, 99);
    let mut agents: Vec<Box<dyn Agent>> = (0..4)
        .map(|i| Box::new(GreedyAgent::new(i)) as Box<dyn Agent>)
        .collect();
    play_out(&mut game, &mut agents);

    let replayed = Game::replay(config(), 4, 99, game.history()).unwrap();
    assert_eq!(replayed, game);

    let json = serde_json::to_string(&game).unwrap();
    let restored: Game = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, game);
}

#[test]
fn uncontested_auction_goes_to_the_initiator() {
    let mut game = new_game(3, 5);
    advance_until(&mut game, |g| matches!(g.phase(), Phase::Acquiring(_)));
    let p = game.current_player().unwrap();
    let coins = game.player(p).coins;
    let held = game.player(p).assets.total();
    let tokens = game.markets()[0].clone();

    game.apply(Action::StartAuction { market: 0, bid: 8 }).unwrap();
    game.apply(Action::PassBid).unwrap();
    game.apply(Action::PassBid).unwrap();

    assert_eq!(game.player(p).coins, coins - 8);
    let specials = tokens.iter().filter(|a| **a == Asset::Emperor).count() as u32;
    assert_eq!(game.player(p).assets.total(), held + tokens.len() as u32 - specials);
    assert_eq!(game.markets()[0].len(), 3, "initiator won, so the market refills");
    assert_ne!(game.current_player(), Some(p), "winners can't start another auction");
}

#[test]
fn outbid_initiator_opens_again() {
    let mut game = new_game(3, 5);
    advance_until(&mut game, |g| matches!(g.phase(), Phase::Acquiring(_)));
    let initiator = game.current_player().unwrap();
    game.apply(Action::StartAuction { market: 1, bid: 8 }).unwrap();
    let rival = game.current_player().unwrap();
    game.apply(Action::Bid { amount: 9 }).unwrap();
    game.apply(Action::PassBid).unwrap(); // third player
    game.apply(Action::PassBid).unwrap(); // initiator
    assert_eq!(game.player(rival).coins, game.rules().starting_coins - 9);
    assert!(game.markets()[1].is_empty(), "no refill when the initiator loses");
    assert_eq!(game.current_player(), Some(initiator));
}

#[test]
fn accepted_trade_moves_assets_and_coins() {
    let mut game = new_game(3, 11);
    advance_until(&mut game, |g| matches!(g.phase(), Phase::Trading { .. }));
    let a = game.current_player().unwrap();
    let b = PlayerId((a.0 + 1) % 3);
    let give_asset = game.player(a).assets.iter().find(|(x, _)| x.is_tradeable()).unwrap().0;
    let terms = TradeTerms {
        give: [give_asset].into_iter().collect(),
        give_coins: 0,
        receive: Assets::new(),
        receive_coins: 5,
    };
    assert!(!game.legal_actions().iter().any(|x| matches!(x, Action::ProposeTrade { .. })));
    game.apply(Action::ProposeTrade { to: b, terms }).unwrap();
    assert_eq!(game.current_player(), Some(b));
    let (a_before, b_before) = (game.player(a).clone(), game.player(b).clone());
    game.apply(Action::AcceptTrade).unwrap();
    assert_eq!(game.player(a).coins, a_before.coins + 5);
    assert_eq!(game.player(b).coins, b_before.coins - 5);
    assert_eq!(game.player(b).assets.count(give_asset), b_before.assets.count(give_asset) + 1);
    assert_eq!(game.current_player(), Some(a), "the active player continues");
}

#[test]
fn trade_for_assets_you_lack_is_illegal() {
    let mut game = new_game(3, 11);
    advance_until(&mut game, |g| matches!(g.phase(), Phase::Trading { .. }));
    let a = game.current_player().unwrap();
    let b = PlayerId((a.0 + 1) % 3);
    let terms = TradeTerms {
        give: Assets::new(),
        give_coins: 1000,
        receive: Assets::new(),
        receive_coins: 0,
    };
    assert!(!game.is_legal(&Action::ProposeTrade { to: b, terms }));
}

#[test]
fn trade_caps_are_per_other_player() {
    let mut game = new_game(3, 11);
    advance_until(&mut game, |g| matches!(g.phase(), Phase::Trading { .. }));
    let a = game.current_player().unwrap();
    let (b, c) = (PlayerId((a.0 + 1) % 3), PlayerId((a.0 + 2) % 3));
    let cap = game.rules().trading.max_offers_per_partner;
    let counter_cap = game.rules().trading.max_counter_offers_per_partner;
    let coins = |give_coins, receive_coins| TradeTerms {
        give: Assets::new(),
        give_coins,
        receive: Assets::new(),
        receive_coins,
    };

    // Counter-offers with B are capped across the whole negotiation.
    game.apply(Action::ProposeTrade { to: b, terms: coins(1, 0) }).unwrap();
    for _ in 0..counter_cap {
        game.apply(Action::CounterTrade { terms: coins(0, 1) }).unwrap();
    }
    assert!(!game.is_legal(&Action::CounterTrade { terms: coins(0, 1) }));
    game.apply(Action::RejectTrade).unwrap();

    for _ in 1..cap {
        game.apply(Action::ProposeTrade { to: b, terms: coins(1, 0) }).unwrap();
        game.apply(Action::RejectTrade).unwrap();
    }
    assert!(!game.is_legal(&Action::ProposeTrade { to: b, terms: coins(1, 0) }), "B's offers are used up");
    assert!(game.is_legal(&Action::ProposeTrade { to: c, terms: coins(1, 0) }), "C still has offers left");
}
