mod client;
mod model;
mod utils;
mod liquidation;

use crate::client::{run_eternal_liquidator, run_liquidator_iter};

#[tokio::main]
async fn main() {
    // println!("Hello, world!");

    run_eternal_liquidator().await;
}
