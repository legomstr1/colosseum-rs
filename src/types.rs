#[derive(Clone, Debug)]
pub struct PlayerState {
    pub name: String,
    pub coins: u32,
    pub assets: Vec<Asset>,
    pub arena_level: u8,
    pub podiums: u8,
    pub loge: bool,
    pub season_tickets: u8,
    pub medals: u8,
    pub score: u32,
    pub purchased: bool,
}

impl PlayerState {
    pub fn new(name: String) -> Self {
        Self {
            name,
            coins: 30, // Starting coins in Colosseum
            assets: Vec::new(),
            arena_level: 0,
            podiums: 0,
            loge: false,
            season_tickets: 0,
            medals: 0,
            score: 0,
        }
    }
}

pub struct Event {
    pub id: u8,
    pub name: String,
    pub cost: u8,
    pub base_score: u8,
    pub size: u8,
    pub requirements: Vec<Asset>,
    pub penalty_scores: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Asset {
    Gladiator,
    Comedian,
    Musician,
    Horse,
    Torch,
    Priest,
    Ship,
    Lion,
    Scenery,
    Decoration,
    Chariot,
    Cage,
    Joker,
    Emperor,
    SpecialEvent,
}

pub type Batch = Vec<Asset>;
pub type Market = Vec<Batch>;

pub enum NobleType {
    Senator,
    Consul,
    Emperor,
}

pub struct Noble {
    pub noble_type: NobleType,
    pub position: u8,
}

pub struct Board {
    pub resting_places: Vec<u8>,
    pub nobles: Vec<Noble>,
}

impl Board {
    pub const BOARD_SIZE: u8 = 36;

    pub fn new() -> Self {
        Self {
            resting_places: vec![0, 6, 12, 19, 26, 30],
            nobles: vec![
                Noble {
                    noble_type: NobleType::Emperor,
                    position: 0,
                },
                Noble {
                    noble_type: NobleType::Consul,
                    position: 12,
                },
                Noble {
                    noble_type: NobleType::Consul,
                    position: 26,
                },
                Noble {
                    noble_type: NobleType::Senator,
                    position: 6,
                },
                Noble {
                    noble_type: NobleType::Senator,
                    position: 19,
                },
                Noble {
                    noble_type: NobleType::Senator,
                    position: 30,
                },
            ],
        }
    }

    pub fn move_noble(&mut self, noble_idx: usize, dice_roll: u8) {
        if noble_idx < self.nobles.len() {
            self.nobles[noble_idx].position =
                (self.nobles[noble_idx].position + dice_roll) % Self::BOARD_SIZE;
        }
    }

    pub fn is_resting_place(&self, position: u8) -> bool {
        self.resting_places.contains(&position)
    }
}

pub enum Action {
    // Investing
    BuyEvent {
        id: u8,
    },
    ExpandArena {
        space: u8,
    },
    BuySeasonTicket,
    BuildLoge,
    ExtraInvestSpecialEvent,
    ExtraInvestTwoMedals,
    PassInvesting,

    // Acquiring
    StartBid {
        batch: u8,
        bid: u8,
    },
    Bid {
        bid: u8,
    },
    PassBid,
    PassAcquiring,

    // Trading
    ProposeTrade {
        player_id: u8,
        assets_given: Vec<Asset>,
        money_given: u8,
        assets_received: Vec<Asset>,
        money_received: u8,
    },
    AcceptTrade,
    RejectTrade,
    PassTrading,

    // Producing
    MoveNoble {
        noble_idx: u8,
        dice_roll: u8,
    },
    ExchangeMedalForMove {
        noble_idx: u8,
        dice_roll: u8,
    },
    ExchangeMedalForPoints,
    ProduceEvent {
        id: u8,
    },
    PassProducing,

    // Closing
    DiscardAsset {
        asset: Asset,
    },
    StealAsset {
        asset: Asset,
    },

    // Medals
    ExchangeMedalForCoins,
}
