//! Phase 3: in turn order, the active player may make offers to any other
//! player. Only coins and tradeable assets change hands, immediately.

use super::{Game, Phase};
use crate::action::{Action, TradeTerms};
use crate::event::GameEvent;
use crate::types::PlayerId;
use serde::{Deserialize, Serialize};

/// An offer waiting for `to` to answer. `terms` are from `from`'s view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub from: PlayerId,
    pub to: PlayerId,
    pub terms: TradeTerms,
}

impl Game {
    pub(super) fn trading_actions(&self, offer: Option<&Offer>) -> Vec<Action> {
        match offer {
            None => vec![Action::EndTrading],
            Some(o) if self.check_terms(o.from, o.to, &o.terms).is_ok() => {
                vec![Action::AcceptTrade, Action::RejectTrade]
            }
            Some(_) => vec![Action::RejectTrade],
        }
    }

    pub(super) fn check_proposal(&self, p: PlayerId, to: PlayerId, terms: &TradeTerms) -> Result<(), String> {
        let Phase::Trading { offers, offer, .. } = &self.phase else {
            return Err("offers can only be made in the trading phase".into());
        };
        if offer.is_some() {
            return Err("an offer is already waiting for an answer".into());
        }
        let made = offers.get(to.0).copied().unwrap_or(0);
        if made >= self.rules().trading.max_offers_per_partner {
            return Err(format!("no offers to {to} left this turn"));
        }
        self.check_terms(p, to, terms)
    }

    pub(super) fn check_counter(&self, p: PlayerId, terms: &TradeTerms) -> Result<(), String> {
        let Phase::Trading { turn, counters, offer: Some(offer), .. } = &self.phase else {
            return Err("there is no offer to counter".into());
        };
        let partner = self.trade_partner(*turn, offer);
        if offer.to != p {
            return Err("only the player receiving an offer can counter it".into());
        }
        if counters[partner.0] >= self.rules().trading.max_counter_offers_per_partner {
            return Err(format!("no counter-offers left with {partner} this turn"));
        }
        self.check_terms(p, offer.from, terms)
    }

    /// The player the active trader is negotiating with in `offer`.
    fn trade_partner(&self, turn: usize, offer: &Offer) -> PlayerId {
        if offer.from == self.seat(turn) { offer.to } else { offer.from }
    }

    fn check_terms(&self, from: PlayerId, to: PlayerId, terms: &TradeTerms) -> Result<(), String> {
        if to.0 >= self.num_players() || to == from {
            return Err("pick another player to trade with".into());
        }
        if terms.is_empty() {
            return Err("the offer is empty".into());
        }
        if terms
            .give
            .iter()
            .chain(terms.receive.iter())
            .any(|(a, _)| !a.is_tradeable())
        {
            return Err("only event assets and Jokers can be traded".into());
        }
        let (a, b) = (self.player(from), self.player(to));
        if !a.assets.contains_all(&terms.give) || a.coins < terms.give_coins {
            return Err(format!("{from} doesn't have what they are offering"));
        }
        if !b.assets.contains_all(&terms.receive) || b.coins < terms.receive_coins {
            return Err(format!("{to} doesn't have what is asked for"));
        }
        Ok(())
    }

    pub(super) fn apply_trading(&mut self, p: PlayerId, action: Action, ev: &mut Vec<GameEvent>) {
        let partner = match &self.phase {
            Phase::Trading { turn, offer: Some(o), .. } => Some(self.trade_partner(*turn, o)),
            _ => None,
        };
        let n = self.players.len();
        let Phase::Trading { turn, offers, counters, offer } = &mut self.phase else {
            unreachable!()
        };
        match action {
            Action::ProposeTrade { to, terms } => {
                offers[to.0] += 1;
                ev.push(GameEvent::TradeProposed {
                    from: p,
                    to,
                    terms: terms.clone(),
                });
                *offer = Some(Offer {
                    from: p,
                    to,
                    terms,
                });
            }
            Action::CounterTrade { terms } => {
                let old = offer.take().expect("checked");
                counters[partner.expect("an offer is open").0] += 1;
                ev.push(GameEvent::TradeProposed {
                    from: p,
                    to: old.from,
                    terms: terms.clone(),
                });
                *offer = Some(Offer {
                    from: p,
                    to: old.from,
                    terms,
                });
            }
            Action::AcceptTrade => {
                let o = offer.take().expect("checked");
                let t = &o.terms;
                let (from, to) = (o.from.0, o.to.0);
                self.players[from].assets.remove_all(&t.give);
                self.players[to].assets.add_all(&t.give);
                self.players[to].assets.remove_all(&t.receive);
                self.players[from].assets.add_all(&t.receive);
                self.players[from].coins = self.players[from].coins - t.give_coins + t.receive_coins;
                self.players[to].coins = self.players[to].coins - t.receive_coins + t.give_coins;
                ev.push(GameEvent::TradeAccepted {
                    from: o.from,
                    to: o.to,
                    terms: o.terms.clone(),
                });
                self.update_stars(ev);
            }
            Action::RejectTrade => {
                *offer = None;
                ev.push(GameEvent::TradeRejected { by: p });
            }
            Action::EndTrading => {
                ev.push(GameEvent::Passed { player: p });
                *turn += 1;
                *offers = vec![0; n];
                *counters = vec![0; n];
            }
            other => unreachable!("{other:?} is not a trading action"),
        }
    }
}
