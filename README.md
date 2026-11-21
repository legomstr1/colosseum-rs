# colosseum-rs

A Rust implementation of the classic board game [Colosseum](https://boardgamegeek.com/boardgame/27746/colosseum) by Wolfgang Kramer and Markus Lübke, designed for building and testing AI agents.

## About

In Colosseum, players take on the role of Roman impresarios, producing spectacular events in their arenas to attract the most spectators. This implementation faithfully recreates the 2007 Days of Wonder base game, providing a complete game engine for running matches programmatically.

The primary goal of this project is to enable AI development and experimentation with the complex resource management, auction mechanics, and strategic planning that make Colosseum a compelling game.

## Project Status

🚧 **Work in Progress** 🚧

Current implementation status:
- ✅ Data-driven game engine: all five phases, with every rule constant in YAML
- ✅ AI agent API, plus random and greedy bots
- ✅ CLI for human play and bot simulations
- ✅ Deterministic replay from a seed and action history, plus serializable game state
- 📋 MCP / API layer (planned)
- ✅ Browser GUI (`colosseum-web`), hotseat play against bots
- 📋 Online play with others (planned)

## Features

- **Complete rule implementation** - All mechanics from the original 2007 base game
- **Flexible player count** - Supports 3-5 players (any combination of human/AI)
- **AI-ready architecture** - Clean API for implementing computer opponents

## Quick Start

### Playing the Game

```bash
git clone https://github.com/legomstr1/colosseum-rs.git
cd colosseum-rs

# You against two greedy bots (seats: human, greedy, random; 3-5 players)
cargo run --release -p colosseum-cli -- play --players human,greedy,greedy

# Bot-only simulation
cargo run --release -p colosseum-cli -- sim --players greedy,random,random --games 500

# Browser GUI, then open http://127.0.0.1:8080
cargo run --release -p colosseum-web
```

Pass `--seed N` to replay a game exactly. Pass `--data <dir>` to load a modified `game_config.yml`/`events.yml`.

### Building an AI Agent

The engine is a reducer over plain data. At every decision point it gives you the legal actions, so an agent only has to choose one:

```rust
use colosseum_engine::agent::{play_out, Agent, GreedyAgent};
use colosseum_engine::{Action, Game, GameConfig};
use std::sync::Arc;

struct MyAI;

impl Agent for MyAI {
    fn name(&self) -> String { "mine".into() }
    fn choose_action(&mut self, game: &Game, legal: &[Action]) -> Action {
        // `game` is the full public state; `legal` is every legal move.
        legal[0].clone()
    }
}

let mut game = Game::new(Arc::new(GameConfig::embedded()), 3, 42).unwrap();
let mut agents: Vec<Box<dyn Agent>> =
    vec![Box::new(MyAI), Box::new(GreedyAgent::new(1)), Box::new(GreedyAgent::new(2))];
play_out(&mut game, &mut agents);
println!("{:?}", game.ranking());
```

Trade offers are open-ended, so `legal_actions()` doesn't list them. Build a `ProposeTrade` or `CounterTrade` yourself and validate it with `game.check(&action)`.

## Game Overview

Colosseum is played over 5 rounds, with each round consisting of multiple phases:

1. **Investing** - Buy a new event program, expand the arena, purchase a season ticket, or construct an Emperor's loge
2. **Acquiring Event Asset Tokens** - Bid on batches of assets
3. **Tading Event Asset Tokens** - Buy, sell, or exchange assets with other players
4. **Producing an Event** - Move nobles, produce an event, and count the spectators
5. **Closing Ceremonies** - Award a podium, clean up the events, and donate an asset

Players must carefully manage their resources, plan ahead for future events, and adapt to the changing game state. The winner is the player who attracts the most spectators to a single event over the course of the game.

For complete rules, see the [official BoardGameGeek page](https://boardgamegeek.com/boardgame/27746/colosseum).

## Documentation

- [Architecture Overview](docs/ARCHITECTURE.md) - Design decisions and system structure
- [Game Rules Reference](docs/RULES.md) - Implementation details of game mechanics
- [AI Development Guide](docs/AI_GUIDE.md) - Building and testing AI agents
- [Contributing Guide](docs/CONTRIBUTING.md) - How to contribute to the project

## Development

### Prerequisites

- Rust (latest stable version)
- Cargo

### Building from Source

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Run with logging
RUST_LOG=debug cargo run
```

### Project Structure

```
colosseum-rs/
├── data/
│   ├── game_config.yml   # Every rule constant: costs, supplies, bonuses, board, dice
│   └── events.yml        # The 30 event program cards
└── crates/
    ├── colosseum-engine/ # Library: state, actions, rules reducer, agents
    │   ├── src/game/     # One module per phase (legal actions + effects)
    │   └── tests/        # Full bot games, invariants, replay, scoring
    ├── colosseum-cli/    # `colosseum` binary: play and sim
    └── colosseum-web/    # `colosseum-web` binary: JSON API + browser page in static/
```

## Contributing

Contributions are welcome! This project is open source and maintained for the love of the game. Whether you're interested in:

- Implementing game mechanics
- Building AI agents
- Improving documentation
- Adding tests
- Creating the CLI/TUI interface

Please read [CONTRIBUTING.md](docs/CONTRIBUTING.md) for guidelines on how to get started.

## Roadmap

### Near Term
- [ ] Complete phase state machine implementation
- [ ] Implement all game mechanics and rule enforcement
- [ ] Design and document AI agent API
- [ ] Create CLI/TUI for human players

### Future
- [ ] Example AI implementations (random, greedy, MCTS)
- [ ] Game replay and analysis tools
- [ ] Save/load game states
- [ ] Performance optimizations for AI training
- [ ] Statistics and game analytics

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- Token and noble icons in the browser GUI are by Lorc, Delapouite and Cathelineau from [game-icons.net](https://game-icons.net), licensed under [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/)

- **Wolfgang Kramer** and **Markus Lübke** - Original game designers
- **Days of Wonder** - Original game publisher
- The board gaming community for keeping this classic alive

## Disclaimer

This is a fan-made implementation created for educational and AI research purposes. Colosseum is a trademark of Days of Wonder. This project is not affiliated with or endorsed by Days of Wonder or the original game designers.

---

**Have questions?** Open an issue or start a discussion!

**Want to contribute?** Check out our [contribution guidelines](docs/CONTRIBUTING.md)!
