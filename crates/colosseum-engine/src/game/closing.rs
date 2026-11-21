//! Phase 5: closing ceremonies (every round but the last).
//!
//! 1. The player with the best single event so far gets a podium.
//! 2. Each player who produced this round discards one token they used.
//! 3. The player in last place names an asset type for the podium winner
//!    to donate to them.

use super::{Game, Phase};
use crate::action::Action;
use crate::event::{GameEvent, PhaseKind};
use crate::types::PlayerId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closing {
    pub podium_winner: Option<PlayerId>,
    /// Players who still have to discard, in turn order.
    pub discard_queue: Vec<PlayerId>,
    /// `(receiver, donor)` for the donation, if one is still owed.
    pub donation: Option<(PlayerId, PlayerId)>,
}

impl Game {
    pub(super) fn start_closing(&mut self, ev: &mut Vec<GameEvent>) {
        ev.push(GameEvent::PhaseStarted {
            phase: PhaseKind::Closing,
        });
        let n = self.num_players();

        let podium_winner = if self.supply.podiums > 0 {
            let w = self.pick_tied(|g, id| {
                let p = g.player(id);
                (p.best_score as i64, p.coins as i64)
            });
            self.supply.podiums -= 1;
            self.players[w.0].podiums += 1;
            ev.push(GameEvent::PodiumAwarded { player: w });
            Some(w)
        } else {
            None
        };

        let round = self.round;
        let discard_queue = (0..n)
            .map(|t| self.seat(t))
            .filter(|&id| {
                let p = self.player(id);
                p.last_event
                    .as_ref()
                    .is_some_and(|e| e.round == round && e.used.iter().any(|(a, _)| p.assets.count(a) > 0))
            })
            .collect();

        let donation = podium_winner.map(|donor| {
            let receiver = self.pick_tied(|g, id| {
                let p = g.player(id);
                (-(p.best_score as i64), -(p.coins as i64))
            });
            (receiver, donor)
        });

        self.phase = Phase::Closing(Closing {
            podium_winner,
            discard_queue,
            donation,
        });
    }

    /// The player with the highest key; remaining ties are broken randomly
    /// (standing in for the rulebook's dice roll-off).
    fn pick_tied(&mut self, key: impl Fn(&Game, PlayerId) -> (i64, i64)) -> PlayerId {
        let ids: Vec<PlayerId> = self.players.iter().map(|p| p.id).collect();
        let best = ids.iter().map(|&id| key(self, id)).max().expect("players");
        let tied: Vec<PlayerId> = ids.into_iter().filter(|&id| key(self, id) == best).collect();
        tied[self.rng.below(tied.len())]
    }

    pub(super) fn closing_actions(&self, p: PlayerId, c: &Closing) -> Vec<Action> {
        if c.discard_queue.first() == Some(&p) {
            let me = self.player(p);
            let used = me.last_event.as_ref().map(|e| e.used.clone()).unwrap_or_default();
            return used
                .iter()
                .filter(|(a, _)| me.assets.count(*a) > 0)
                .map(|(asset, _)| Action::DiscardAsset { asset })
                .collect();
        }
        match c.donation {
            Some((receiver, donor)) if receiver == p => self
                .player(donor)
                .assets
                .iter()
                .filter(|(a, _)| a.is_standard())
                .map(|(asset, _)| Action::RequestAsset { asset })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn apply_closing(&mut self, p: PlayerId, action: Action, ev: &mut Vec<GameEvent>) {
        let Phase::Closing(c) = &mut self.phase else {
            unreachable!()
        };
        match action {
            Action::DiscardAsset { asset } => {
                c.discard_queue.remove(0);
                self.players[p.0].assets.remove(asset, 1);
                self.discarded.add(asset, 1);
                ev.push(GameEvent::AssetDiscarded { player: p, asset });
            }
            Action::RequestAsset { asset } => {
                let (receiver, donor) = c.donation.take().expect("checked");
                self.players[donor.0].assets.remove(asset, 1);
                self.players[receiver.0].assets.add(asset, 1);
                ev.push(GameEvent::AssetDonated {
                    from: donor,
                    to: receiver,
                    asset,
                });
            }
            other => unreachable!("{other:?} is not a closing action"),
        }
        self.update_stars(ev);
    }
}
