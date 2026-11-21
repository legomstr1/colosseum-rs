//! Terminal front end: play against bots and other humans at one keyboard,
//! or run bot-vs-bot simulations.

use clap::{Parser, Subcommand};
use colosseum_engine::agent::{Agent, GreedyAgent, RandomAgent};
use colosseum_engine::{Action, Asset, Assets, Game, GameConfig, GameEvent, Phase, PlayerId, TradeTerms};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "colosseum", about = "Play the Colosseum board game in the terminal")]
struct Cli {
    /// Directory holding game_config.yml and events.yml (defaults to the
    /// data built into the engine).
    #[arg(long, global = true)]
    data: Option<PathBuf>,

    /// RNG seed (random if omitted).
    #[arg(long, global = true)]
    seed: Option<u64>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Play a game. Seats are listed in order: human, greedy or random.
    Play {
        #[arg(long, value_delimiter = ',', default_value = "human,greedy,greedy")]
        players: Vec<String>,
    },
    /// Run many bot-only games and report results by seat.
    Sim {
        #[arg(long, value_delimiter = ',', default_value = "greedy,random,random")]
        players: Vec<String>,
        #[arg(long, default_value_t = 100)]
        games: u64,
    },
}

enum Seat {
    Human,
    Bot(Box<dyn Agent>),
}

fn make_seat(kind: &str, seed: u64) -> Result<Seat, String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "human" | "h" => Ok(Seat::Human),
        "greedy" | "g" => Ok(Seat::Bot(Box::new(GreedyAgent::new(seed)))),
        "random" | "r" => Ok(Seat::Bot(Box::new(RandomAgent::new(seed)))),
        other => Err(format!("unknown player type '{other}' (use human, greedy or random)")),
    }
}

fn main() {
    let cli = Cli::parse();
    let config = match &cli.data {
        Some(dir) => GameConfig::from_dir(dir),
        None => Ok(GameConfig::embedded()),
    };
    let config = match config {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let seed = cli.seed.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64)
    });
    let result = match cli.command.unwrap_or(Command::Play {
        players: vec!["human".into(), "greedy".into(), "greedy".into()],
    }) {
        Command::Play { players } => play(config, &players, seed),
        Command::Sim { players, games } => simulate(config, &players, games, seed),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

// ---- Simulation ------------------------------------------------------------

fn simulate(config: Arc<GameConfig>, kinds: &[String], games: u64, seed: u64) -> Result<(), String> {
    let n = kinds.len();
    let mut wins = vec![0u64; n];
    let mut total_best = vec![0u64; n];
    for g in 0..games {
        let game_seed = seed.wrapping_add(g);
        let mut game = Game::new(config.clone(), n, game_seed).map_err(|e| e.to_string())?;
        let mut agents = Vec::new();
        for (i, k) in kinds.iter().enumerate() {
            match make_seat(k, game_seed.wrapping_mul(31).wrapping_add(i as u64))? {
                Seat::Bot(a) => agents.push(a),
                Seat::Human => return Err("sim only takes bots".into()),
            }
        }
        colosseum_engine::agent::play_out(&mut game, &mut agents);
        wins[game.ranking()[0].0] += 1;
        for p in game.players() {
            total_best[p.id.0] += p.best_score as u64;
        }
    }
    println!("{games} games, seed {seed}");
    println!("{:<6} {:<8} {:>6} {:>8} {:>10}", "seat", "agent", "wins", "win %", "avg best");
    for i in 0..n {
        println!(
            "{:<6} {:<8} {:>6} {:>7.1}% {:>10.1}",
            PlayerId(i).to_string(),
            kinds[i],
            wins[i],
            100.0 * wins[i] as f64 / games as f64,
            total_best[i] as f64 / games as f64
        );
    }
    Ok(())
}

// ---- Interactive play ------------------------------------------------------

fn play(config: Arc<GameConfig>, kinds: &[String], seed: u64) -> Result<(), String> {
    let n = kinds.len();
    let mut game = Game::new(config, n, seed).map_err(|e| e.to_string())?;
    let mut seats = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| make_seat(k, seed.wrapping_add(i as u64 + 1)))
        .collect::<Result<Vec<_>, _>>()?;

    println!("Colosseum - seed {seed}");
    for (i, k) in kinds.iter().enumerate() {
        println!("  {}: {k}", PlayerId(i));
    }
    println!("Type 'h' at any prompt for help.\n");
    println!("=== Round 1 (start player {}) ===", game.start_player());

    let mut input = Input::new();
    while let Some(p) = game.current_player() {
        let legal = game.legal_actions();
        let action = match &mut seats[p.0] {
            Seat::Bot(agent) => agent.choose_action(&game, &legal),
            Seat::Human => match human_turn(&game, p, &legal, &mut input) {
                Some(a) => a,
                None => {
                    println!("Quitting.");
                    return Ok(());
                }
            },
        };
        match game.apply(action) {
            Ok(events) => print_events(&events),
            Err(e) => println!("  ! {}", e.reason),
        }
    }
    println!("\nReplay this game with --seed {seed}.");
    Ok(())
}

fn print_events(events: &[GameEvent]) {
    for e in events {
        match e {
            GameEvent::RoundStarted { .. } | GameEvent::GameOver { .. } => println!("\n{e}"),
            GameEvent::PhaseStarted { .. } => println!("{e}"),
            _ => println!("  {e}"),
        }
    }
}

struct Input {
    lines: io::Lines<io::StdinLock<'static>>,
}

impl Input {
    fn new() -> Self {
        Self {
            lines: io::stdin().lock().lines(),
        }
    }

    /// Prompts and reads one trimmed line; `None` on end of input.
    fn ask(&mut self, prompt: &str) -> Option<String> {
        print!("{prompt}");
        io::stdout().flush().ok();
        self.lines.next()?.ok().map(|l| l.trim().to_string())
    }

    fn ask_number(&mut self, prompt: &str, min: u32, max: u32) -> Option<u32> {
        loop {
            let line = self.ask(&format!("{prompt} [{min}-{max}]: "))?;
            match line.parse::<u32>() {
                Ok(v) if (min..=max).contains(&v) => return Some(v),
                _ => println!("  Enter a number from {min} to {max}."),
            }
        }
    }
}

/// One line of the human's menu: a single action, or a family of actions
/// that differ by one number (a bid, a medal count).
enum MenuItem {
    Single(Action),
    Range {
        label: String,
        prompt: &'static str,
        values: Vec<u32>,
        make: Box<dyn Fn(u32) -> Action>,
    },
}

fn build_menu(game: &Game, legal: &[Action]) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = Vec::new();
    let mut add_range = |key: String, prompt: &'static str, value: u32, make: Box<dyn Fn(u32) -> Action>| {
        for item in items.iter_mut() {
            if let MenuItem::Range { label, values, .. } = item
                && *label == key
            {
                values.push(value);
                return;
            }
        }
        items.push(MenuItem::Range {
            label: key,
            prompt,
            values: vec![value],
            make,
        });
    };
    let mut singles = Vec::new();
    for action in legal {
        match action {
            &Action::StartAuction { market, bid } => {
                let tokens: Assets = game.markets()[market].iter().copied().collect();
                add_range(
                    format!("Open market {} ({tokens})", market + 1),
                    "Opening bid",
                    bid,
                    Box::new(move |bid| Action::StartAuction { market, bid }),
                );
            }
            &Action::Bid { amount } => {
                add_range("Raise the bid".into(), "Bid", amount, Box::new(|amount| Action::Bid { amount }));
            }
            &Action::ProduceEvent { program, medals } => {
                let label = game.describe_action(&Action::ProduceEvent { program, medals: 0 });
                add_range(
                    label,
                    "Medals to spend for extra spectators",
                    medals,
                    Box::new(move |medals| Action::ProduceEvent { program, medals }),
                );
            }
            &Action::MoveNoble { noble, die, distance } => {
                let n = game.nobles()[noble];
                add_range(
                    format!("Move {} (at {}) with die {}", n.kind, n.position, die + 1),
                    "Distance",
                    distance as u32,
                    Box::new(move |d| Action::MoveNoble { noble, die, distance: d as u8 }),
                );
            }
            &Action::MoveNobleCombined { noble, distance } => {
                let n = game.nobles()[noble];
                add_range(
                    format!("Move {} (at {}) with both dice", n.kind, n.position),
                    "Distance",
                    distance as u32,
                    Box::new(move |d| Action::MoveNobleCombined { noble, distance: d as u8 }),
                );
            }
            other => singles.push(other.clone()),
        }
    }
    items.extend(singles.into_iter().map(MenuItem::Single));
    items
}

fn human_turn(game: &Game, me: PlayerId, legal: &[Action], input: &mut Input) -> Option<Action> {
    println!();
    print_state(game, me);
    let menu = build_menu(game, legal);
    let trading = matches!(game.phase(), Phase::Trading { .. });
    let responding = matches!(&game.phase(), Phase::Trading { offer: Some(o), .. } if o.to == me);

    loop {
        if let Phase::Trading { offer: Some(o), .. } = &game.phase() {
            println!(
                "{} offers you {} + {} coins for {} + {} coins",
                o.from, o.terms.give, o.terms.give_coins, o.terms.receive, o.terms.receive_coins
            );
        }
        println!("{me}, choose:");
        for (i, item) in menu.iter().enumerate() {
            match item {
                MenuItem::Single(a) => println!("  {:>2}) {}", i + 1, game.describe_action(a)),
                MenuItem::Range { label, values, .. } => {
                    let (lo, hi) = (values.iter().min().unwrap(), values.iter().max().unwrap());
                    if lo == hi {
                        println!("  {:>2}) {label}", i + 1);
                    } else {
                        println!("  {:>2}) {label} ({lo}-{hi})", i + 1);
                    }
                }
            }
        }
        if trading && !responding {
            println!("   t) Make a trade offer");
        }
        if responding {
            println!("   c) Make a counter-offer");
        }
        let line = input.ask("> ")?;
        match line.as_str() {
            "q" | "quit" => return None,
            "h" | "help" | "?" => print_help(),
            "s" | "state" => print_state(game, me),
            "t" if trading && !responding => {
                if let Some(a) = build_offer(game, me, input, false)? {
                    return Some(a);
                }
            }
            "c" if responding => {
                if let Some(a) = build_offer(game, me, input, true)? {
                    return Some(a);
                }
            }
            _ => match line.parse::<usize>().ok().and_then(|i| menu.get(i.wrapping_sub(1))) {
                Some(MenuItem::Single(a)) => return Some(a.clone()),
                Some(MenuItem::Range { prompt, values, make, .. }) => {
                    let (lo, hi) = (*values.iter().min().unwrap(), *values.iter().max().unwrap());
                    let v = if lo == hi { lo } else { input.ask_number(prompt, lo, hi)? };
                    if values.contains(&v) {
                        return Some(make(v));
                    }
                    println!("  {v} isn't an option here.");
                }
                None => println!("  Pick a number from the list (or 'h' for help)."),
            },
        }
    }
}

/// Asks for trade terms. Returns `Some(None)` if the offer turned out
/// illegal (the reason is printed), `None` on end of input.
fn build_offer(game: &Game, me: PlayerId, input: &mut Input, counter: bool) -> Option<Option<Action>> {
    let to = if counter {
        match &game.phase() {
            Phase::Trading { offer: Some(o), .. } => o.from,
            _ => return Some(None),
        }
    } else {
        let others: Vec<String> = (0..game.num_players())
            .filter(|&i| i != me.0)
            .map(|i| PlayerId(i).to_string())
            .collect();
        let who = input.ask(&format!("Trade with ({}): ", others.join(", ")))?;
        let who = who.trim_start_matches(['P', 'p']);
        match who.parse::<usize>() {
            Ok(i) if i >= 1 && i <= game.num_players() => PlayerId(i - 1),
            _ => {
                println!("  No such player.");
                return Some(None);
            }
        }
    };
    println!("  List assets like '2 gladiator, horse'; leave blank for none.");
    let give = parse_assets(&input.ask("  You give: ")?);
    let give_coins = input.ask("  Coins you give: ")?.parse().unwrap_or(0);
    let receive = parse_assets(&input.ask("  You get: ")?);
    let receive_coins = input.ask("  Coins you get: ")?.parse().unwrap_or(0);
    let (Some(give), Some(receive)) = (give, receive) else {
        println!("  Couldn't read those assets.");
        return Some(None);
    };
    let terms = TradeTerms {
        give,
        give_coins,
        receive,
        receive_coins,
    };
    let action = if counter {
        Action::CounterTrade { terms }
    } else {
        Action::ProposeTrade { to, terms }
    };
    match game.check(&action) {
        Ok(()) => Some(Some(action)),
        Err(reason) => {
            println!("  ! {reason}");
            Some(None)
        }
    }
}

fn parse_assets(s: &str) -> Option<Assets> {
    let mut out = Assets::new();
    for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let mut words = part.split_whitespace();
        let first = words.next()?;
        let (n, name) = match first.parse::<u32>() {
            Ok(n) => (n, words.collect::<Vec<_>>().join(" ")),
            Err(_) => (1, part.to_string()),
        };
        out.add(Asset::parse(&name)?, n);
    }
    Some(out)
}

fn print_help() {
    println!(
        "\
  Enter the number of an option. Options with a range ask for a value next.
  s  show the full game state
  t  make a trade offer (trading phase, on your turn)
  c  counter an offer made to you
  q  quit"
    );
}

/// Prints `items` after `label`, joined by `sep`, wrapping lines at about
/// 110 columns with continuation lines indented under the first item.
fn print_wrapped(label: &str, items: &[String], sep: &str) {
    const WIDTH: usize = 110;
    let indent = " ".repeat(label.len());
    let mut line = label.to_string();
    for (i, item) in items.iter().enumerate() {
        let piece = if i + 1 < items.len() { format!("{item}{sep}") } else { item.clone() };
        if line.len() > label.len() && line.len() + piece.trim_end().len() > WIDTH {
            println!("{}", line.trim_end());
            line = indent.clone();
        }
        line.push_str(&piece);
    }
    println!("{}", line.trim_end());
}

fn print_state(game: &Game, me: PlayerId) {
    let rules = game.rules();
    let phase = game.phase().kind().map_or("Game over".into(), |k| k.to_string());
    println!(
        "Round {}/{} - {phase} - start player {}",
        game.round(), rules.num_rounds, game.start_player()
    );

    let markets: Vec<String> = game
        .markets()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let names: Vec<&str> = m.iter().map(|a| a.name()).collect();
            format!("{}) {}", i + 1, if m.is_empty() { "-".into() } else { names.join(" ") })
        })
        .collect();
    print_wrapped("Markets:  ", &markets, "  |  ");

    let nobles: Vec<String> = game
        .nobles()
        .iter()
        .map(|n| {
            let owner = game.arena_owner(n.position).map_or(String::new(), |p| format!(" ({p})"));
            format!("{}@{}{owner}", n.kind, n.position)
        })
        .collect();
    println!("Nobles:   {}", nobles.join(", "));

    let for_sale: Vec<String> = game
        .programs_for_sale()
        .iter()
        .filter_map(|id| game.config().event(*id))
        .map(|e| format!("#{} {} ({}c, {} spect.)", e.id, e.name, e.cost, e.base_score))
        .collect();
    print_wrapped("For sale: ", &for_sale, "  |  ");

    let stars: Vec<String> = game
        .stars()
        .iter()
        .filter_map(|(a, h)| h.map(|h| format!("{a}:{h}")))
        .collect();
    println!(
        "Stars:    {}   Supply: {} medals, {} podiums, {} tickets, {} loges",
        if stars.is_empty() { "none".into() } else { stars.join(" ") },
        game.supply().medals,
        game.supply().podiums,
        game.supply().season_tickets,
        game.supply().emperor_loges
    );

    if let Phase::Producing(pr) = &game.phase() {
        let dice: Vec<String> = pr
            .dice
            .iter()
            .zip(&pr.used)
            .enumerate()
            .map(|(i, (d, used))| format!("die {}: {d}{}", i + 1, if *used { " (used)" } else { "" }))
            .collect();
        println!("Dice:     {}", dice.join(", "));
    }
    if let Phase::Acquiring(a) = &game.phase()
        && let Some(au) = &a.auction
    {
        println!(
            "Auction:  market {} - high bid {} by {}",
            au.market + 1,
            au.high_bid,
            au.high_bidder
        );
    }

    for p in game.players() {
        let squares = p.arena.squares(rules.board.size);
        let you = if p.id == me { " (you)" } else { "" };
        println!(
            "{}{you}: {} coins, best {}, arena {:?}{}, tickets {}, medals {}, podiums {}",
            p.id,
            p.coins,
            p.best_score,
            squares,
            if p.loge { " + loge" } else { "" },
            p.season_tickets,
            p.medals,
            p.podiums
        );
        println!("    assets:   {}", p.assets);
        let programs: Vec<String> = p
            .programs
            .iter()
            .filter_map(|prog| game.config().event(prog.id).map(|e| (prog, e)))
            .map(|(prog, e)| {
                let req: Assets = e.requirements.iter().copied().collect();
                format!(
                    "#{} {} [{}; size {}]{}",
                    e.id,
                    e.name,
                    req,
                    e.size,
                    if prog.produced { " (produced)" } else { "" }
                )
            })
            .collect();
        println!("    programs: {}", programs.join("; "));
    }
}
