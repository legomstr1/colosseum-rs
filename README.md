# colosseum-rs

A Rust implementation of the classic board game [Colosseum](https://boardgamegeek.com/boardgame/27746/colosseum) by Wolfgang Kramer and Markus Lübke, designed for building and testing AI agents.

## About

In Colosseum, players take on the role of Roman impresarios, producing spectacular events in their arenas to attract the most spectators. This implementation faithfully recreates the 2007 Days of Wonder base game, providing a complete game engine for running matches programmatically.

The primary goal of this project is to enable AI development and experimentation with the complex resource management, auction mechanics, and strategic planning that make Colosseum a compelling game.

## Project Status

🚧 **Work in Progress** 🚧

Current implementation status:
- 🚧 Core data structures and game state representation
- 🚧 Game rules and phase management (in progress)
- 📋 AI agent API (planned)
- 📋 CLI/TUI for human play (planned)
- 📋 Game replay system (planned)

## Features

- **Complete rule implementation** - All mechanics from the original 2007 base game
- **Flexible player count** - Supports 3-5 players (any combination of human/AI)
- **AI-ready architecture** - Clean API for implementing computer opponents

## Quick Start

### Playing the Game

> **Note**: CLI/TUI interface is currently under development. Check back soon!

```bash
# Clone the repository
git clone https://github.com/legomstr1/colosseum-rs.git
cd colosseum-rs

# Build the project
cargo build --release

# Run a game (coming soon)
cargo run --release
```

### Building an AI Agent

The game provides all legal actions at each decision point through a well-defined API. Here's the basic structure:

```rust
// Example structure (API subject to change during development)
use colosseum_rs::{Game, Action, Player};

struct MyAI {
    // Your AI state
}

impl Player for MyAI {
    fn choose_action(&mut self, game: &Game, legal_actions: &[Action]) -> Action {
        // Your AI logic here
        // All legal moves are provided - no rule checking needed!
        legal_actions[0].clone()
    }
}
```

For detailed information on implementing AI agents, see [docs/AI_GUIDE.md](docs/AI_GUIDE.md) (coming soon).

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
├── src/
│   ├── data/           # Relevant game data
│   ├── game_state.rs   # Managing the game state
│   ├── types.rs        # All the structs
│   ├── main.rs         #
│   └── lib.rs          # Library entry point
├── docs/               # Documentation
├── examples/           # Example AI implementations
└── tests/              # Integration tests
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

- **Wolfgang Kramer** and **Markus Lübke** - Original game designers
- **Days of Wonder** - Original game publisher
- The board gaming community for keeping this classic alive

## Disclaimer

This is a fan-made implementation created for educational and AI research purposes. Colosseum is a trademark of Days of Wonder. This project is not affiliated with or endorsed by Days of Wonder or the original game designers.

---

**Have questions?** Open an issue or start a discussion!

**Want to contribute?** Check out our [contribution guidelines](docs/CONTRIBUTING.md)!
