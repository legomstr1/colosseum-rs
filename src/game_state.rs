use crate::types::*;

#[derive(Clone, Debug)]
pub struct GameState {
    pub players: Vec<PlayerState>,
    pub current_round: u8,
    pub phase: Phase,
    pub remaining_assets: Vec<Asset>, // All assets in game
    pub available_medals: u8,
    pub available_tickets: u8,
    pub available_events: Vec<Event>,
    pub market: Market,
    pub board: Board,
}

impl GameState {
    pub fn new(num_players: usize) -> Self {
        let colosseum_layout = match num_players {
            3 => vec![vec![8, 9], vec![21, 22], vec![32, 33]],
            4 => vec![vec![3, 4], vec![14, 15], vec![23, 24], vec![32, 33]],
            5 => vec![
                vec![2, 3],
                vec![8, 9],
                vec![15, 16],
                vec![23, 24],
                vec![32, 33],
            ],
            _ => panic!("Invalid player count"),
        };

        let players = colosseum_layout
            .into_iter()
            .enumerate()
            .map(|(i, positions)| PlayerState::new(PlayerId(i), positions))
            .collect();

        Self {
            players,
            current_round: 1,
            phase,
            remaining_assets,
            available_medals: 18,
            avaialble_tickets: 10,
            available_events,
            market,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Phase {
    Investing {
        active_player: PlayerId,
    },
    Acquiring {
        active_player: PlayerId,
        current_market: u8,
        current_bidder: Option<PlayerId>,
        current_bid: u32,
    },
    Trading {
        active_player: PlayerId,
    },
    Producing {
        player_order: Vec<PlayerId>,
        selections_made: usize,
    },
    Closing,
    GameOver,
}
