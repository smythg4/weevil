#!/usr/bin/env bash
set -euo pipefail

BINARY="./target/release/client"

if [[ ! -x "$BINARY" ]]; then
    echo "Client binary not found. Run: cargo build --release"
    exit 1
fi

# clean slate
rm -f ./data_files/wal.log ./data_files/checkpoint ./data_files/checkpoint.tmp

echo "threads,transfers_per_thread,total_transfers,elapsed_ms,tps"

CONFIGS=(
    "10 1000"
    "20 500"
    "40 250"
    "100 100"
    "200 50"
    "250 40"
)

for config in "${CONFIGS[@]}"; do
    threads=$(echo "$config" | awk '{print $1}')
    txns=$(echo "$config" | awk '{print $2}')
    total=$((threads * txns))

    output=$("$BINARY" "$threads" "$txns" 2>/dev/null)

    elapsed=$(echo "$output" | grep "Total time:" | grep -oE '[0-9]+(\.[0-9]+)?')
    tps=$(echo "$output" | grep "^TPS:" | awk '{print $2}')
    processed=$(echo "$output" | grep "Transactions Processed:" | awk '{print $3}' | cut -d'/' -f1 | tr -d ' ')

    echo "$threads,$txns,$processed,${elapsed},${tps}"

    # let the server drain between runs
    sleep 0.5
done
