//! Phase 4: in turn order, each player rolls, moves nobles, then may
//! produce one event.

use super::{Game, Phase, ProducedEvent};
use crate::action::Action;
use crate::config::DieFace;
use crate::event::{GameEvent, ScoreBreakdown};
use crate::types::{Asset, Assets, PlayerId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Producing {
    pub turn: usize,
    pub rolled: bool,
    pub dice: Vec<DieFace>,
    pub used: Vec<bool>,
    /// Nobles already moved by a die this turn (each die moves a different one).
    pub moved: Vec<usize>,
}

impl Producing {
    fn dice_left(&self) -> bool {
        self.used.iter().any(|u| !u)
    }

    /// Every distance the dice allow when all of them move one noble together.
    pub fn combined_distances(&self) -> Vec<u8> {
        let mut sums = vec![0u8];
        for face in &self.dice {
            sums = sums
                .iter()
                .flat_map(|s| face.values().iter().map(move |v| s + v))
                .collect();
        }
        sums.sort_unstable();
        sums.dedup();
        sums
    }
}

/// The result of producing a program right now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPreview {
    pub breakdown: ScoreBreakdown,
    pub used: Assets,
}

impl Game {
    pub(super) fn roll_dice(&mut self, turn: usize, ev: &mut Vec<GameEvent>) {
        let p = self.seat(turn);
        let dice_rules = self.rules().dice.clone();
        let count = if self.player(p).loge {
            dice_rules.loge_count
        } else {
            dice_rules.base_count
        };
        let dice: Vec<DieFace> = (0..count)
            .map(|_| dice_rules.faces[self.rng.below(dice_rules.faces.len())].clone())
            .collect();
        ev.push(GameEvent::DiceRolled {
            player: p,
            dice: dice.clone(),
        });
        self.phase = Phase::Producing(Producing {
            turn,
            rolled: true,
            used: vec![false; dice.len()],
            dice,
            moved: Vec::new(),
        });
    }

    pub(super) fn producing_actions(&self, p: PlayerId, pr: &Producing) -> Vec<Action> {
        let mut actions = Vec::new();
        if pr.dice_left() {
            let mut seen = Vec::new();
            for (die, face) in pr.dice.iter().enumerate() {
                if pr.used[die] || seen.contains(&face) {
                    continue;
                }
                seen.push(face);
                for noble in (0..self.nobles.len()).filter(|n| !pr.moved.contains(n)) {
                    for &distance in face.values() {
                        actions.push(Action::MoveNoble { noble, die, distance });
                    }
                }
            }
            if pr.dice.len() == 2 && pr.moved.is_empty() {
                for noble in 0..self.nobles.len() {
                    for distance in pr.combined_distances() {
                        actions.push(Action::MoveNobleCombined { noble, distance });
                    }
                }
            }
            return actions;
        }

        let me = self.player(p);
        if me.medals > 0 {
            let max = self.rules().medals.max_noble_move as i8;
            for noble in 0..self.nobles.len() {
                for delta in (-max..=max).filter(|d| *d != 0) {
                    actions.push(Action::MedalMoveNoble { noble, delta });
                }
            }
        }
        for program in &me.programs {
            if self.event_preview(p, program.id, 0).is_some() {
                actions.extend((0..=me.medals).map(|medals| Action::ProduceEvent {
                    program: program.id,
                    medals,
                }));
            }
        }
        actions.push(Action::SkipEvent);
        actions
    }

    /// What producing `program` would score for player `p` right now, or
    /// `None` if they can't produce it.
    pub fn event_preview(&self, p: PlayerId, program: u8, medals: u32) -> Option<EventPreview> {
        let rules = self.rules();
        let me = self.player(p);
        let def = self.config().event(program)?;
        if !me.owns_program(program) || medals > me.medals {
            return None;
        }
        let required_squares = *rules
            .arena
            .squares_required_by_event_size
            .get(def.size as usize)?;
        if me.arena.len < required_squares {
            return None;
        }

        let mut used = Assets::new();
        let mut missing = 0;
        for (asset, needed) in def.requirement_counts().iter() {
            let have = me.assets.count(asset).min(needed);
            used.add(asset, have);
            missing += needed - have;
        }
        let jokers = me.assets.count(Asset::Joker).min(missing);
        used.add(Asset::Joker, jokers);
        missing -= jokers;
        let base = def.score_with_missing(missing as usize)?;

        let s = &rules.scoring;
        let previous = me.programs.iter().filter(|q| q.produced && q.id != program).count() as u32;
        let stars = self
            .stars
            .iter()
            .filter(|(asset, holder)| **holder == Some(p) && used.count(**asset) > 0)
            .count() as u32;
        let nobles = self
            .nobles_in_arena(p)
            .into_iter()
            .map(|i| rules.noble_value(self.nobles[i].kind))
            .sum();

        Some(EventPreview {
            breakdown: ScoreBreakdown {
                missing_assets: missing,
                base,
                previous_programs: previous * s.previous_program_bonus,
                season_tickets: me.season_tickets * s.season_ticket_bonus,
                stars: stars * s.star_award_bonus,
                nobles,
                podiums: me.podiums * s.podium_bonus,
                medals: medals * s.medal_bonus,
            },
            used,
        })
    }

    pub(super) fn apply_producing(&mut self, p: PlayerId, action: Action, ev: &mut Vec<GameEvent>) {
        let board = self.rules().board.size;
        let Phase::Producing(pr) = &mut self.phase else {
            unreachable!()
        };
        let mut end_turn = false;
        let mut moves: Vec<(usize, i32, bool)> = Vec::new(); // (noble, distance, earns medal)
        match action {
            Action::MoveNoble { noble, die, distance } => {
                pr.used[die] = true;
                pr.moved.push(noble);
                moves.push((noble, distance as i32, true));
            }
            Action::MoveNobleCombined { noble, distance } => {
                pr.used.iter_mut().for_each(|u| *u = true);
                pr.moved.push(noble);
                moves.push((noble, distance as i32, true));
            }
            Action::MedalMoveNoble { noble, delta } => {
                self.players[p.0].medals -= 1;
                self.supply.medals += 1;
                moves.push((noble, delta as i32, false));
            }
            Action::ProduceEvent { program, medals } => {
                let preview = self.event_preview(p, program, medals).expect("legal event");
                let spectators = preview.breakdown.total();
                let round = self.round;
                let me = &mut self.players[p.0];
                me.medals -= medals;
                self.supply.medals += medals;
                me.coins += spectators;
                if let Some(prog) = me.programs.iter_mut().find(|q| q.id == program) {
                    prog.produced = true;
                }
                let new_best = spectators > me.best_score;
                me.best_score = me.best_score.max(spectators);
                me.last_event = Some(ProducedEvent {
                    round,
                    program,
                    spectators,
                    used: preview.used,
                });
                ev.push(GameEvent::EventProduced {
                    player: p,
                    program,
                    name: self.config.event(program).map_or_else(String::new, |e| e.name.clone()),
                    spectators,
                    breakdown: preview.breakdown,
                    new_best,
                });
                end_turn = true;
            }
            Action::SkipEvent => {
                ev.push(GameEvent::EventSkipped { player: p });
                end_turn = true;
            }
            other => unreachable!("{other:?} is not a producing action"),
        }

        for (noble, distance, can_earn) in moves {
            let from = self.nobles[noble].position;
            let to = (from as i32 + distance).rem_euclid(board as i32) as u8;
            self.nobles[noble].position = to;
            let medal_earned = can_earn && self.rules().is_resting_place(to) && self.supply.medals > 0;
            if medal_earned {
                self.supply.medals -= 1;
                self.players[p.0].medals += 1;
            }
            ev.push(GameEvent::NobleMoved {
                player: p,
                kind: self.nobles[noble].kind,
                from,
                to,
                medal_earned,
            });
        }

        if end_turn && let Phase::Producing(pr) = &mut self.phase {
            *pr = Producing {
                turn: pr.turn + 1,
                ..Producing::default()
            };
        }
    }
}
