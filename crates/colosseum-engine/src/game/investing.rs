//! Phase 1: each player in turn makes their investments.

use super::{Game, Phase, Program};
use crate::action::Action;
use crate::event::GameEvent;
use crate::types::{Asset, PlayerId, Side};

impl Game {
    pub(super) fn investing_actions(&self, p: PlayerId, remaining: u32) -> Vec<Action> {
        let rules = self.rules();
        let me = self.player(p);
        let mut actions = Vec::new();
        if remaining > 0 {
            for &program in &self.programs_for_sale {
                let cost = self.config().event(program).map_or(u32::MAX, |e| e.cost);
                if me.coins >= cost {
                    actions.push(Action::BuyProgram { program });
                }
            }
            if me.arena.len < rules.arena.max_squares && me.coins >= rules.arena.expansion_cost {
                for side in [Side::Left, Side::Right] {
                    let square = me.arena.expansion_square(side, rules.board.size);
                    if self.arena_owner(square).is_none() {
                        actions.push(Action::ExpandArena { side });
                    }
                }
            }
            if self.supply.season_tickets > 0 && me.coins >= rules.investing.season_ticket_cost {
                actions.push(Action::BuySeasonTicket);
            }
            if !me.loge && self.supply.emperor_loges > 0 && me.coins >= rules.investing.emperor_loge_cost {
                actions.push(Action::BuildLoge);
            }
        }
        if me.assets.count(Asset::SpecialEvent) > 0 {
            actions.push(Action::PlaySpecialEvent);
        }
        if me.medals >= rules.investing.extra_investment_medal_cost {
            actions.push(Action::SpendMedalsForInvestment);
        }
        actions.push(Action::EndInvesting);
        actions
    }

    pub(super) fn apply_investing(&mut self, p: PlayerId, action: Action, ev: &mut Vec<GameEvent>) {
        let config = self.config.clone();
        let rules = &config.rules;
        let Phase::Investing { turn, remaining } = &mut self.phase else {
            unreachable!()
        };
        let me = &mut self.players[p.0];
        match action {
            Action::BuyProgram { program } => {
                let def = config.event(program).expect("legal program");
                me.coins -= def.cost;
                me.programs.push(Program {
                    id: program,
                    produced: false,
                });
                self.programs_for_sale.retain(|&id| id != program);
                *remaining -= 1;
                ev.push(GameEvent::ProgramBought {
                    player: p,
                    program,
                    name: def.name.clone(),
                    cost: def.cost,
                });
            }
            Action::ExpandArena { side } => {
                let square = me.arena.expansion_square(side, rules.board.size);
                me.coins -= rules.arena.expansion_cost;
                if side == Side::Left {
                    me.arena.start = square;
                }
                me.arena.len += 1;
                *remaining -= 1;
                ev.push(GameEvent::ArenaExpanded { player: p, square });
            }
            Action::BuySeasonTicket => {
                me.coins -= rules.investing.season_ticket_cost;
                me.season_tickets += 1;
                self.supply.season_tickets -= 1;
                *remaining -= 1;
                ev.push(GameEvent::SeasonTicketBought { player: p });
            }
            Action::BuildLoge => {
                me.coins -= rules.investing.emperor_loge_cost;
                me.loge = true;
                self.supply.emperor_loges -= 1;
                *remaining -= 1;
                ev.push(GameEvent::LogeBuilt { player: p });
            }
            Action::PlaySpecialEvent => {
                me.assets.remove(Asset::SpecialEvent, 1);
                *remaining += rules.investing.special_event_extra_investments;
                ev.push(GameEvent::ExtraInvestment {
                    player: p,
                    source: "SpecialEvent token".into(),
                });
            }
            Action::SpendMedalsForInvestment => {
                let cost = rules.investing.extra_investment_medal_cost;
                me.medals -= cost;
                self.supply.medals += cost;
                *remaining += 1;
                ev.push(GameEvent::ExtraInvestment {
                    player: p,
                    source: format!("{cost} medals"),
                });
            }
            Action::EndInvesting => {
                ev.push(GameEvent::Passed { player: p });
                *turn += 1;
                *remaining = rules.investing.investments_per_phase;
                return;
            }
            other => unreachable!("{other:?} is not an investing action"),
        }
        // End the turn automatically once nothing but "end" is left.
        let can_extend = me.assets.count(Asset::SpecialEvent) > 0
            || me.medals >= rules.investing.extra_investment_medal_cost;
        if *remaining == 0 && !can_extend {
            *turn += 1;
            *remaining = rules.investing.investments_per_phase;
        }
    }
}
