#!/bin/bash

# 2048 3x3 Win Probability Server
# Memory-efficient version that computes probabilities incrementally

set -e

cd "$(dirname "$0")"

echo "=== 2048 3x3 Win Probability Server ==="
echo ""

# Build if needed
if [ ! -f "target/release/game2048-server" ]; then
    echo "Building release binary..."
    cargo build --release
    echo "Build complete."
    echo ""
fi

# Check command line args
case "${1:-}" in
    compute)
        echo "Computing probabilities (memory-efficient mode)..."
        echo "This processes states by sum, only keeping 2 sums in memory at a time."
        echo "Progress is saved after each sum, so you can interrupt and resume."
        echo ""
        exec ./target/release/game2048-server compute
        ;;
    serve)
        echo "Starting server (requires precomputed data)..."
        exec ./target/release/game2048-server serve
        ;;
    clean)
        echo "Removing computed probability data..."
        rm -rf probability_data
        echo "Done."
        ;;
    *)
        echo "Usage: $0 [command]"
        echo ""
        echo "Commands:"
        echo "  compute  - Compute probabilities incrementally (memory-efficient)"
        echo "             Saves progress to probability_data/ directory"
        echo "             Can be interrupted and resumed"
        echo ""
        echo "  serve    - Start the web server (loads precomputed data)"
        echo "             Requires 'compute' to have been run first"
        echo ""
        echo "  clean    - Remove all computed probability data"
        echo ""
        echo "  (no arg) - Show this help"
        echo ""
        echo "Typical workflow:"
        echo "  1. ./run-server.sh compute   # Run once, can take a while"
        echo "  2. ./run-server.sh serve     # Start server (fast)"
        ;;
esac
