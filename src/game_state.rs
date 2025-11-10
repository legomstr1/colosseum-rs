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
            phase: Phase::Investing {
                active_player: PlayerId(0),
            },
            remaining_assets,
            available_medals: 18,
            avaialble_tickets: 10,
            available_events,
            market,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Trade {
    pub proposer_id: u8,
    pub target_id: u8,
    pub assets_given: Vec<Asset>,
    pub money_given: u8,
    pub assets_received: Vec<Asset>,
    pub money_received: u8,
}

#[derive(Clone, Debug)]
pub enum Phase {
    Investing {
        active_player: u8,
    },
    Acquiring {
        active_player: u8,
        current_batch: u8,
        current_bidder: u8,
        current_bid: u8,
    },
    Trading {
        active_player: u8,
        pending_trade: Option<Trade>,
    },
    Producing {
        active_player: u8,
    },
    Closing,
    GameOver,
}
