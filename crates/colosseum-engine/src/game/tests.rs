//! Scenario tests that set up exact game situations. They live inside the
//! crate because only the engine itself can write `Game`'s private fields.

use super::*;
use crate::config::DieFace;

fn new_game(players: usize, seed: u64) -> Game {
    Game::new(Arc::new(GameConfig::embedded()), players, seed).unwrap()
}

/// Ends phases with the simplest action until `pred` holds.
fn advance_until(game: &mut Game, pred: impl Fn(&Game) -> bool) {
    while !pred(game) {
        let action = match &game.phase {
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
fn event_score_adds_up() {
    let mut game = new_game(3, 3);
    let me = PlayerId(0);
    let size = game.rules().board.size;
    let arena = game.player(me).arena;
    // Program 1 needs 2 Gladiators and a Decoration; base 7, [5, 3] if missing.
    {
        let p = &mut game.players[0];
        p.programs = vec![Program { id: 1, produced: false }];
        p.assets = [Asset::Gladiator, Asset::Gladiator, Asset::Decoration].into_iter().collect();
    }
    game.stars.values_mut().for_each(|h| *h = None);
    // Only the Emperor in the arena.
    for (i, noble) in game.nobles.iter_mut().enumerate() {
        noble.position = if i == 0 {
            arena.start
        } else {
            (arena.start + arena.len + 1) % size
        };
    }
    assert_eq!(game.nobles[0].kind, NobleKind::Emperor);

    let full = game.event_preview(me, 1, 0).unwrap();
    assert_eq!(full.breakdown.total(), 7 + 7);

    game.players[0].assets.remove(Asset::Decoration, 1);
    assert_eq!(game.event_preview(me, 1, 0).unwrap().breakdown.total(), 5 + 7);

    game.players[0].assets.add(Asset::Joker, 1);
    let joker = game.event_preview(me, 1, 0).unwrap();
    assert_eq!(joker.breakdown.total(), 7 + 7, "a Joker fills the gap");
    assert_eq!(joker.used.count(Asset::Joker), 1);

    game.players[0].season_tickets = 2;
    game.players[0].podiums = 1;
    game.players[0].medals = 2;
    game.players[0].programs.push(Program { id: 2, produced: true });
    let s = &game.rules().scoring;
    let expected = 7 + 7 + 2 * s.season_ticket_bonus + s.podium_bonus + 2 * s.medal_bonus + s.previous_program_bonus;
    assert_eq!(game.event_preview(me, 1, 2).unwrap().breakdown.total(), expected);

    game.players[0].assets = Assets::new();
    assert!(game.event_preview(me, 1, 0).is_none(), "too many assets missing");
}

#[test]
fn big_events_need_a_big_arena() {
    let mut game = new_game(3, 3);
    let needed = game.config().event(30).unwrap().requirement_counts();
    let p = &mut game.players[0];
    p.programs = vec![Program { id: 30, produced: false }];
    p.assets = needed;
    assert!(game.event_preview(PlayerId(0), 30, 0).is_none());
    game.players[0].arena.len = 4;
    assert!(game.event_preview(PlayerId(0), 30, 0).is_some());
}

#[test]
fn choice_die_face_offers_each_distance() {
    let mut game = new_game(3, 2);
    advance_until(&mut game, |g| matches!(&g.phase, Phase::Producing(pr) if pr.rolled));
    if let Phase::Producing(pr) = &mut game.phase {
        pr.dice = vec![DieFace::Choice(vec![1, 2, 3]), DieFace::Fixed(5)];
        pr.used = vec![false, false];
    }
    let legal = game.legal_actions();
    let distances = |want_die: usize| -> Vec<u8> {
        let mut d: Vec<u8> = legal
            .iter()
            .filter_map(|a| match a {
                Action::MoveNoble { noble: 0, die, distance } if *die == want_die => Some(*distance),
                _ => None,
            })
            .collect();
        d.sort();
        d
    };
    assert_eq!(distances(0), vec![1, 2, 3]);
    assert_eq!(distances(1), vec![5]);
    let mut combined: Vec<u8> = legal
        .iter()
        .filter_map(|a| match a {
            Action::MoveNobleCombined { noble: 0, distance } => Some(*distance),
            _ => None,
        })
        .collect();
    combined.sort();
    assert_eq!(combined, vec![6, 7, 8]);
    assert!(!game.is_legal(&Action::MoveNoble { noble: 0, die: 0, distance: 4 }));

    let from = game.nobles[0].position;
    game.apply(Action::MoveNoble { noble: 0, die: 0, distance: 2 }).unwrap();
    assert_eq!(game.nobles[0].position, (from + 2) % game.rules().board.size);
}

#[test]
fn any_program_for_sale_can_be_bought() {
    let mut game = new_game(3, 4);
    let p = game.current_player().unwrap();
    let last = *game.programs_for_sale.last().unwrap();
    game.players[p.0].coins = 100;
    assert!(game.is_legal(&Action::BuyProgram { program: last }));
    game.apply(Action::BuyProgram { program: last }).unwrap();
    assert!(game.player(p).owns_program(last));
    assert!(!game.programs_for_sale.contains(&last));
}

#[test]
fn jokers_never_earn_star_bonuses() {
    let mut game = new_game(3, 3);
    let me = PlayerId(0);
    // Program 1 needs 2 Gladiators and a Decoration.
    {
        let p = &mut game.players[0];
        p.programs = vec![Program { id: 1, produced: false }];
        p.assets = [Asset::Joker, Asset::Joker, Asset::Decoration].into_iter().collect();
    }
    game.stars.values_mut().for_each(|h| *h = None);
    game.stars.insert(Asset::Gladiator, Some(me));
    let preview = game.event_preview(me, 1, 0).unwrap();
    assert_eq!(preview.used.count(Asset::Joker), 2);
    assert_eq!(preview.breakdown.stars, 0, "Jokers standing in for Gladiators earn no star bonus");
}

#[test]
fn starting_programs_are_never_for_sale() {
    let mut game = new_game(3, 4);
    let p = game.current_player().unwrap();
    game.players[p.0].coins = 1000;
    let starting: Vec<u8> = game.rules().programs.starting_hand_pools.iter().flatten().copied().collect();
    for id in starting {
        assert!(!game.is_legal(&Action::BuyProgram { program: id }), "program {id} can't be bought");
    }
}

#[test]
fn tied_players_get_no_star_from_the_deal() {
    let mut game = new_game(3, 1);
    let threshold = game.rules().stars.threshold;
    for p in &mut game.players {
        p.assets = Assets::new();
    }
    game.stars.values_mut().for_each(|h| *h = None);
    let mut ev = Vec::new();
    for i in 0..2 {
        game.add_assets(PlayerId(i), vec![Asset::Gladiator; threshold as usize], &mut ev);
    }
    game.update_stars(&mut ev);
    assert_eq!(game.stars[&Asset::Gladiator], None);
}

#[test]
fn starting_stars_go_only_to_a_sole_leader() {
    for players in 3..=5 {
        for seed in 0..500 {
            let game = new_game(players, seed);
            for (&asset, &holder) in &game.stars {
                let counts: Vec<u32> = game.players.iter().map(|p| p.assets.count(asset)).collect();
                let max = *counts.iter().max().unwrap();
                let sole = counts.iter().filter(|&&c| c == max).count() == 1;
                let expected = (sole && max >= game.rules().stars.threshold)
                    .then(|| PlayerId(counts.iter().position(|&c| c == max).unwrap()));
                assert_eq!(holder, expected, "{players} players, seed {seed}, {asset:?}");
            }
        }
    }
}
