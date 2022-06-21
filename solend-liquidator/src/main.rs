mod client;
mod model;
mod utils;

#[tokio::main]
async fn main() {
    // println!("Hello, world!");

    client::run_liquidator().await;
}
