#!/bin/bash

# TODO: document, automate and add to CI?

TOKEN_LENDING_PROGRAM_ID="So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo"
RESERVE_ACCOUNT_PUBKEY="BgxfHJDzm44T7XG68MYKx7YisTjZu73tVovyZSjJMpmw" # Solend Main Pool - (USDC) Reserve State

git_root="$(git rev-parse --show-toplevel)"
test_reserve_path="${git_root}/token-lending/tests/liquidity-mining/fixtures/${RESERVE_ACCOUNT_PUBKEY}.json"
token_lending_program_path="${git_root}/target/deploy/solend_program.so"

echo "Building the program..."
cargo-build-sbf -- -p solend-program

echo "Starting the test validator..."
solana-test-validator --reset \
    --upgradeable-program "${TOKEN_LENDING_PROGRAM_ID}" "${token_lending_program_path}" "$(solana address)" \
    --account "${RESERVE_ACCOUNT_PUBKEY}" "${test_reserve_path}"

# cargo run --bin solend-cli -- --url http://127.0.0.1:8899 upgrade-reserve --reserve BgxfHJDzm44T7XG68MYKx7YisTjZu73tVovyZSjJMpmw
