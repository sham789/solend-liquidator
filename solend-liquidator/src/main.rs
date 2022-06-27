mod client;
mod model;
mod utils;

use crate::client::{run_eternal_liquidator, run_liquidator_iter};

#[tokio::main]
async fn main() {
    run_eternal_liquidator().await;
}
