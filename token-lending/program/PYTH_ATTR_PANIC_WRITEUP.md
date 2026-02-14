# Token-Lending: Pyth product attribute parsing hardening (panic → error)

This document accompanies the fix in PR for hardening Pyth product attribute parsing.

## Summary
The Token-Lending program parses the Pyth product account’s `attr` blob to extract the `quote_currency` field.

The previous implementation could panic on malformed attribute data due to unchecked indexing into a fixed-size array (`pyth_product.attr[...]`). On Solana, panics abort program execution and fail the instruction.

This change replaces the ad-hoc parser with a bounds-checked decoder that returns `InvalidOracleConfig` instead of panicking.

## Impact
A panic is effectively a denial-of-service for the affected instruction path when the program is fed malformed account data.

In `InitReserve`, the program checks the oracle account owner against `lending_market.oracle_program_id` and validates basic Pyth header fields (`magic`, `ver`, `atype`). Defensive parsing is still important:
- it makes the program robust against corrupted or malformed oracle accounts
- it avoids foot-guns if an oracle program id is configured that can produce Pyth-lookalike accounts
- it prevents unexpected panics if encoding invariants ever change

## Reproduction (pre-fix)
The attribute blob is encoded as repeating:

```
[key_len: u8][key_bytes...][val_len: u8][val_bytes...]
```

The previous code could index `attr[start]` where `start == PROD_ATTR_SIZE` after a sequence of length-driven skips.

A unit test (`quote_currency_parser_does_not_panic_on_malformed_attr`) constructs a malformed `pyth::Product` where:
- `attr[0] = 255`
- `attr[256] = 0`
- `attr[257] = 206`

This layout previously caused an out-of-bounds panic. After this patch, it returns an error without panicking.

## Fix
Parse `(key_len, key, val_len, val)` pairs in a single pass with explicit bounds checks for every read.

## Verification
- Added unit test that uses `std::panic::catch_unwind` to ensure malformed input does not panic.
