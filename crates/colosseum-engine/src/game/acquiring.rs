//! Phase 2: auctions for the market batches.
//!
//! The initiator opens an auction on a market. Bidding goes clockwise until
//! all but one player has passed. A player who wins a batch is out for the
//! rest of the phase. If the initiator did not win, they open another
//! auction; once they win or pass, the initiative moves clockwise.

use super::{Game, Phase};
use crate::action::Action;
use crate::event::{GameEvent, PhaseKind};
use crate::types::PlayerId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acquiring {
    /// The player whose turn it is to open an auction.
    pub cursor: PlayerId,
    /// Players who can no longer open auctions (they won or passed).
    pub done: Vec<bool>,
    /// Players who have won a batch this phase.
    pub won: Vec<bool>,
    pub auction: Option<Auction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Auction {
    pub market: usize,
    pub initiator: PlayerId,
    pub high_bid: u32,
    pub high_bidder: PlayerId,
    /// Players still in this auction.
    pub in_bidding: Vec<bool>,
    pub to_act: PlayerId,
}

impl Auction {
    fn next_bidder(&self, after: PlayerId) -> PlayerId {
        let n = self.in_bidding.len();
        (1..=n)
            .map(|k| PlayerId((after.0 + k) % n))
            .find(|p| self.in_bidding[p.0])
            .expect("at least one bidder remains")
    }

    fn bidders_left(&self) -> usize {
        self.in_bidding.iter().filter(|b| **b).count()
    }
}

impl Game {
    pub(super) fn start_acquiring(&mut self, ev: &mut Vec<GameEvent>) {
        let n = self.num_players();
        ev.push(GameEvent::PhaseStarted {
            phase: PhaseKind::Acquiring,
        });
        self.phase = Phase::Acquiring(Acquiring {
            cursor: self.start_player,
            done: vec![false; n],
            won: vec![false; n],
            auction: None,
        });
    }

    pub(super) fn acquiring_actions(&self, p: PlayerId, a: &Acquiring) -> Vec<Action> {
        let coins = self.player(p).coins;
        let mut actions = Vec::new();
        match &a.auction {
            None => {
                let min = self.rules().acquiring.minimum_opening_bid;
                for (market, tokens) in self.markets.iter().enumerate() {
                    if !tokens.is_empty() {
                        actions.extend((min..=coins).map(|bid| Action::StartAuction { market, bid }));
                    }
                }
                actions.push(Action::PassInitiative);
            }
            Some(auction) => {
                actions.extend((auction.high_bid + 1..=coins).map(|amount| Action::Bid { amount }));
                actions.push(Action::PassBid);
            }
        }
        actions
    }

    pub(super) fn apply_acquiring(&mut self, p: PlayerId, action: Action, ev: &mut Vec<GameEvent>) {
        let Phase::Acquiring(acq) = &mut self.phase else {
            unreachable!()
        };
        match action {
            Action::StartAuction { market, bid } => {
                let in_bidding: Vec<bool> = acq.won.iter().map(|w| !w).collect();
                let mut auction = Auction {
                    market,
                    initiator: p,
                    high_bid: bid,
                    high_bidder: p,
                    in_bidding,
                    to_act: p,
                };
                ev.push(GameEvent::AuctionStarted { player: p, market, bid });
                let alone = auction.bidders_left() == 1;
                auction.to_act = auction.next_bidder(p);
                acq.auction = Some(auction);
                if alone {
                    self.resolve_auction(ev);
                }
            }
            Action::Bid { amount } => {
                let auction = acq.auction.as_mut().expect("bidding needs an auction");
                auction.high_bid = amount;
                auction.high_bidder = p;
                auction.to_act = auction.next_bidder(p);
                ev.push(GameEvent::BidPlaced { player: p, amount });
            }
            Action::PassBid => {
                let auction = acq.auction.as_mut().expect("passing needs an auction");
                auction.in_bidding[p.0] = false;
                ev.push(GameEvent::Passed { player: p });
                if auction.bidders_left() == 1 {
                    self.resolve_auction(ev);
                } else {
                    auction.to_act = auction.next_bidder(p);
                }
            }
            Action::PassInitiative => {
                acq.done[p.0] = true;
                ev.push(GameEvent::Passed { player: p });
                self.refill_markets(ev);
            }
            other => unreachable!("{other:?} is not an acquiring action"),
        }
    }

    fn resolve_auction(&mut self, ev: &mut Vec<GameEvent>) {
        let Phase::Acquiring(acq) = &mut self.phase else {
            unreachable!()
        };
        let auction = acq.auction.take().expect("an auction to resolve");
        let winner = auction.high_bidder;
        acq.won[winner.0] = true;
        acq.done[winner.0] = true;
        let tokens = std::mem::take(&mut self.markets[auction.market]);
        self.players[winner.0].coins -= auction.high_bid;
        ev.push(GameEvent::AuctionWon {
            player: winner,
            market: auction.market,
            price: auction.high_bid,
            assets: tokens.clone(),
        });
        self.gain_assets(winner, tokens, ev);
        if winner == auction.initiator {
            self.refill_markets(ev);
        }
    }
}
