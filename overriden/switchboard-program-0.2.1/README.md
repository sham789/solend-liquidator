# switchboard-program

A Rust library to interact with Switchboard's hosted data feeds.

## Description

This package can be used to manage Switchboard data feed account parsing.

Specifically, this package will return the most recent confirmed round result
from a provided data feed AccountInfo.

## Usage

```rust
use switchboard_program;
use switchboard_program::{
    AggregatorState,
    RoundResult,
    FastRoundResultAccountData,
    fast_parse_switchboard_result
};
...
let aggregator: AggregatorState = switchboard_program::get_aggregator(
    switchboard_feed_account // &AccountInfo
)?;
let round_result: RoundResult = switchboard_program::get_aggregator_result(
    &aggregator)?;

// pub struct RoundResult {
    // pub num_success: Option<i32>,
    // pub num_error: Option<i32>,
    // pub result: Option<f64>,
    // pub round_open_slot: Option<u64>,
    // pub round_open_timestamp: Option<i64>,
    // pub min_response: Option<f64>,
    // pub max_response: Option<f64>,
    // pub medians: Vec<f64>,
// }

...
// Compute conservative? Use the parse optimized result account instead:

let fast_parse_feed_round = FastRoundResultAccountData::deserialize(
    &switchboard_parse_optimized_account.try_borrow_data()?).unwrap();

// pub struct FastRoundResultAccountData {
    // pub parent: [u8;32],
    // pub result: FastRoundResult,
// }
// // A precisioned decimal representation of the current aggregator result
// // where `result is represented as ${mantissa} / (10^${scale})`
// pub struct SwitchboardDecimal {
    // pub mantissa: i128,
    // pub scale: u64
// }
// pub struct FastRoundResult {
    // pub num_success: i32,
    // pub num_error: i32,
    // pub result: f64,
    // pub round_open_slot: u64,
    // pub round_open_timestamp: i64,
    // pub min_response: f64,
    // pub max_response: f64,
    // pub decimal: SwitchboardDecimal,
// }

```
