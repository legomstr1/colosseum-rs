#[derive(Clone, Debug)]
pub struct PlayerState {
    pub name: String,
    pub coins: u8,
    pub assets: Vec<Asset>,
    pub arena_level: u8,
    pub podiums: u8,
    pub loge: bool,
    pub season_tickets: u8,
    pub medals: u8,
    pub score: u8,
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
        id: u8, // Restriction: One of the events still available
    },
    ExpandArena {
        space: u8, // Restriction: Space adjacent to existing arena spaces
    },
    BuySeasonTicket,
    BuildLoge,
    ExtraInvestSpecialEvent,
    ExtraInvestTwoMedals,
    PassInvesting,

    // Acquiring
    StartBid {
        batch: u8, // Restriction: One of the available batches in the market
        bid: u8, // Restriction: Higher than eight and within player's available coins + medals * 6
    },
    Bid {
        bid: u8, // Restriction: Higher than current bid and within player's available coins + medals * 6
    },
    PassBid,
    PassAcquiring,

    // Trading
    InitiateTrade {
        player_id: u8, // Restriction: One of the other players
    },
    ProposeTrade {
        assets_given: Vec<Asset>,    // Restriction: Assets owned by the proposer
        money_given: u8,             // Restriction: Money owned by the proposer
        assets_received: Vec<Asset>, // Restriction: Assets owned by the target player
        money_received: u8,          // Restriction: Money owned by the target player
    },
    AcceptTrade,
    RejectTrade,
    CounterTrade {
        assets_given: Vec<Asset>,    // Restriction: Assets owned by the proposer
        money_given: u8,             // Restriction: Money owned by the proposer
        assets_received: Vec<Asset>, // Restriction: Assets owned by the target player
        money_received: u8,          // Restriction: Money owned by the target player
    },
    PassTrading,

    // Producing
    MoveNoble {
        noble_idx: u8, // Restriction: One of the nobles on the board
        spaces: u8,    // Restriction: one of or combined rolled dice values
    },
    ExchangeMedalForMove {
        noble_idx: u8, // Restriction: One of the nobles on the board
        spaces: i8,    // Restriction: -3 - -1 or 1 - 3
    },
    ExchangeMedalForPoints,
    ProduceEvent {
        id: u8, // Restriction: One of the events that the player owns that they have enough assets to produce
    },
    PassProducing,

    // Closing
    DiscardAsset {
        asset: Asset, // Restriction: Assets used in event production this round
    },
    StealAsset {
        asset: Asset, // Restriction: Legal assets from player who won the round
    },

    // Medals
    ExchangeMedalForCoins,
}
