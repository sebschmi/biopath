#!/usr/bin/env bash

if [ ! -f "$1.gfa" ]; then
    echo "File $1.gfa not found."
    echo "Usage: $0 <file name without ending>"
    exit 1
fi

if [ ! -f "$1.spqr" ]; then
    echo "File $1.spqr not found."
    echo "Usage: $0 <file name without ending>"
    exit 1
fi

cargo run --release -- index --graph-gfa-in "$1.gfa" --spqr-in "$1.spqr" --index-out "$1.index"
cargo run --release -- query --graph-gfa-in "$1.gfa" --spqr-in "$1.spqr" --index-in "$1.index" --query-in "$1.query" --query-out "$1.query_out"
cargo run --release -- query --graph-gfa-in "$1.gfa" --query-in "$1.query" --query-out "$1.query_out_no_index"

diff "$1.query_out" "$1.query_out_no_index"