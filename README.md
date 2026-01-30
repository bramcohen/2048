# 2048 Training App

A 2048 trainer with smaller boards that shows optimal move probabilities, helping you learn perfect play.

## Games

### 2x3 (Standalone)
- **File:** `2048-2x3.html`
- **Goal:** Reach 128
- **Setup:** Just open the file in a browser - no server needed
- Computes win probabilities locally in JavaScript on page load

### 3x3 (Client + Server)
- **File:** `2048-3x3-client.html`
- **Goal:** Reach 1024
- **Setup:** Requires the Rust server running (see below)
- Fetches precomputed probabilities from the server

## Features

- **Win Probability:** Shows your current chance of winning with optimal play
- **Hints:** Displays the cost of each move (how much win% you lose vs optimal)
- **Accuracy:** Starts at 100%, multiplied by each move's quality (best=100%, others=ratio to best)
- **Undo:** Take back moves to try different strategies
- **Space Bar:** Auto-play the optimal move

## Running the 3x3 Server

First time setup:
```bash
./run-server.sh compute   # Precompute probabilities (takes a while)
./run-server.sh serve     # Start the server
```

Then open `2048-3x3-client.html` in your browser.

### Server Commands

| Command | Description |
|---------|-------------|
| `./run-server.sh compute` | Compute probabilities (saves to `probability_data/`) |
| `./run-server.sh serve` | Start web server on port 3000 |
| `./run-server.sh clean` | Delete computed probability data |

## Controls

| Key | Action |
|-----|--------|
| Arrow keys | Move tiles |
| Space | Play optimal move |
| Enter | New game (when game over) |

## Project Structure

```
├── 2048-2x3.html          # Standalone 2x3 game
├── 2048-3x3-client.html   # 3x3 game (needs server)
├── src/main.rs            # Rust server
├── run-server.sh          # Server launcher
├── probability_data/      # Precomputed probabilities (generated)
└── Cargo.toml             # Rust dependencies
```

## License

Public domain.
