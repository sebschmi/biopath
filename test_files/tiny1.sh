#!/usr/bin/env bash

cargo run -- index --graph-gfa-in test_files/tiny1.gfa --spqr-in test_files/tiny1.spqr --index-out test_files/tiny1.index
cargo run -- query --graph-gfa-in test_files/tiny1.gfa --spqr-in test_files/tiny1.spqr --index-in test_files/tiny1.index --query-in test_files/tiny1.query --query-out test_files/tiny1.query_out
cargo run -- query --graph-gfa-in test_files/tiny1.gfa --query-in test_files/tiny1.query --query-out test_files/tiny1.query_out_no_index

diff test_files/tiny1.query_out test_files/tiny1.query_out_no_index