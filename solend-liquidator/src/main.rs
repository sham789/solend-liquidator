pub mod binding;
pub mod client;
pub mod client_model;
pub mod constants;
pub mod helpers;
pub mod log;
pub mod model;
pub mod performance;
pub mod utils;

use clap::Parser;

use crate::client::{run_eternal_liquidator, run_liquidator_iter};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_parser, default_value_t = String::from("eternal"))]
    mode: String,
    #[clap(short, long, value_parser, default_value_t = String::from("./private/liquidator_main.json"))]
    keypair_path: String,
}

// #[tokio::main]
// async fn main() {
//     let args = Args::parse();

//     match args.mode.as_str() {
//         "eternal" => run_eternal_liquidator(args.keypair_path).await,
//         "iter" => run_liquidator_iter(args.keypair_path).await,
//         _ => panic!("specified mode is unavaiable"),
//     };
// }

fn main() {
    let args = Args::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .max_blocking_threads(1024 * 10)
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async move {
        match args.mode.as_str() {
            "eternal" => run_eternal_liquidator(args.keypair_path).await,
            "iter" => run_liquidator_iter(args.keypair_path).await,
            _ => panic!("specified mode is unavaiable"),
        };
    });
}
