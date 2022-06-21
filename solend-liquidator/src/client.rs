use std::sync::Arc;
use std::time::SystemTime;

use bigdecimal::num_traits::Pow;
use hyper::Body;

use hyper::{Client as HyperClient, Method, Request};
use hyper_tls::HttpsConnector;

use serde_derive::{Deserialize, Serialize};
use serde_json::Value;

// use hmac::{Hmac, Mac};
// use sha2::Sha256;
use {
  // clap::{
  //     crate_description, crate_name, crate_version, value_t, App, AppSettings, Arg, ArgMatches,
  //     SubCommand,
  // },
  solana_clap_utils::{
      fee_payer::fee_payer_arg,
      input_parsers::{keypair_of, pubkey_of, value_of},
      input_validators::{is_amount, is_keypair, is_parsable, is_pubkey, is_url},
      keypair::signer_from_path,
  },
  solana_client::nonblocking::rpc_client::RpcClient,
  solana_program::{native_token::lamports_to_sol, program_pack::Pack, pubkey::Pubkey},
  solana_sdk::{
      commitment_config::CommitmentConfig,
      signature::{Keypair, Signer},
      system_instruction,
      transaction::Transaction,
  },
  // solend_program::{
  //     // self,
  //     instruction::{init_lending_market, init_reserve, update_reserve_config},
  //     math::WAD,
  //     state::{LendingMarket, Reserve, ReserveConfig, ReserveFees},
  // },
  spl_token::{
      amount_to_ui_amount,
      instruction::{approve, revoke},
      state::{Account as Token, Mint},
      ui_amount_to_amount,
  },
  std::{borrow::Borrow, process::exit, str::FromStr},
  system_instruction::create_account,
};

use borsh::{BorshSerialize, BorshDeserialize};
use pyth_sdk_solana;
use bs58;

use crate::model::{self, SolendConfig, Oracles};
use crate::utils::body_to_string;

pub struct Client {
  client: HyperClient<HttpsConnector<hyper::client::HttpConnector>>,
  config: Config,
  solend_cfg: Option<&'static SolendConfig>,
}



pub struct Config {
  rpc_client: RpcClient,
  signer: Box<dyn Signer + Send + Sync>,
  // lending_program_id: Pubkey,
  // verbose: bool,
  // dry_run: bool,
}

pub fn get_config() -> Config {
  let cli_config = solana_cli_config::Config  {
    keypair_path: String::from("./private/liquidator_main.json"),
    ..solana_cli_config::Config::default()
  };
  // let json_rpc_url = value_t!(matches, "json_rpc_url", String)
  //     .unwrap_or_else(|_| cli_config.json_rpc_url.clone());
  let json_rpc_url = String::from("https://broken-dawn-field.solana-mainnet.quiknode.pro/52908360084c7e0666532c96647b9b239ec5cadf/");

  let signer = solana_sdk::signer::keypair::read_keypair_file(cli_config.keypair_path).unwrap();

  // let lending_program_id = pubkey_of(&matches, "lending_program_id").unwrap();
  // let verbose = matches.is_present("verbose");
  // let dry_run = matches.is_present("dry_run");

  Config {
      rpc_client: RpcClient::new_with_commitment(json_rpc_url, CommitmentConfig::confirmed()),
      signer: Box::new(signer),
      // lending_program_id,
      // verbose,
      // dry_run,
  }
}


// #[derive(
//   Copy,
//   Clone,
//   Debug,
//   Default,
//   PartialEq,
//   Eq,
//   BorshSerialize,
//   BorshDeserialize,
// )]
// #[repr(C)]
// // pub struct WrappedPriceAccount(pyth_sdk_solana::state::PriceAccount);
// pub struct WrappedPriceAccount {
//   /// pyth magic number
//   pub magic:          u32,
//   /// program version
//   pub ver:            u32,
//   /// account type
//   pub atype:          u32,
//   /// price account size
//   pub size:           u32,
//   /// price or calculation type
//   pub ptype:          pyth_sdk_solana::state::PriceType,
//   /// price exponent
//   pub expo:           i32,
//   /// number of component prices
//   pub num:            u32,
//   /// number of quoters that make up aggregate
//   pub num_qt:         u32,
//   /// slot of last valid (not unknown) aggregate price
//   pub last_slot:      u64,
//   /// valid slot-time of agg. price
//   pub valid_slot:     u64,
//   /// exponentially moving average price
//   pub ema_price:      pyth_sdk_solana::state::Rational,
//   /// exponentially moving average confidence interval
//   pub ema_conf:       pyth_sdk_solana::state::Rational,
//   /// unix timestamp of aggregate price
//   pub timestamp:      i64,
//   /// min publishers for valid price
//   pub min_pub:        u8,
//   /// space for future derived values
//   pub drv2:           u8,
//   /// space for future derived values
//   pub drv3:           u16,
//   /// space for future derived values
//   pub drv4:           u32,
//   /// product account key
//   pub prod:           pyth_sdk_solana::state::AccKey,
//   /// next Price account in linked list
//   pub next:           pyth_sdk_solana::state::AccKey,
//   /// valid slot of previous update
//   pub prev_slot:      u64,
//   /// aggregate price of previous update with TRADING status
//   pub prev_price:     i64,
//   /// confidence interval of previous update with TRADING status
//   pub prev_conf:      u64,
//   /// unix timestamp of previous aggregate with TRADING status
//   pub prev_timestamp: i64,
//   /// aggregate price info
//   pub agg:            pyth_sdk_solana::state::PriceInfo,
//   /// price components one per quoter
//   pub comp:           [pyth_sdk_solana::state::PriceComp; 32],
// }


pub struct OracleData {
  pub symbol: String,
  pub reserve_address: Pubkey,
  pub mint_address: Pubkey,
  pub decimals: u8,
  pub price: pyth_sdk_solana::state::Price
}


impl Client {
    const CFG_PRESET: &'static str = "production";

    pub fn new() -> Self {
        let client = HyperClient::builder().build::<_, Body>(HttpsConnector::new());
        let config = get_config();

        Self { client, config, solend_cfg: None }
    }

    pub async fn get_solend_config(&self) -> SolendConfig {
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!(
                "https://api.solend.fi/v1/config?deployment={:}",
                Self::CFG_PRESET
            ))
            .header("Content-Type", "application/json")
            .body(Body::from(""))
            .unwrap();

        let res = self.client.request(request).await.unwrap();

        let body = res.into_body();
        let body_str = body_to_string(body).await;

        let solend_cfg: SolendConfig = serde_json::from_str(&body_str).unwrap();

        solend_cfg
    }


    pub async fn get_token_oracle_data(&self, market_reserves: &Vec<model::Resef>) -> Vec<OracleData> {
      let solend_cfg = self.solend_cfg.unwrap();

      let mut oracle_data_list = vec![];
      for market_reserve in market_reserves {
        if let Some(oracle_data) = self.get_oracle_data(market_reserve, &solend_cfg.oracles).await {
          oracle_data_list.push(oracle_data);
        }
      }
      oracle_data_list
    }


    const NULL_ORACLE: &'static str = "nu11111111111111111111111111111111111111111";
    const SWITCHBOARD_V1_ADDRESS: &'static str = "DtmE9D2CSB4L5D6A15mraeEjrGMm6auWVzgaD8hK2tZM";
    const SWITCHBOARD_V2_ADDRESS: &'static str = "SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f";

    async fn get_oracle_data(&self, reserve: &model::Resef, oracles: &Oracles) -> Option<OracleData> {
      let oracle = {
        let mut v = model::Asset2::default();
        for oracle_asset in &oracles.assets {
          if oracle_asset.asset == reserve.asset {
            v = oracle_asset.clone();
            break
          }
        }
        v
      };

      let rpc: &RpcClient = &self.config.rpc_client;

      // let price = 
      let price = if !oracle.price_address.is_empty() && oracle.price_address != Self::NULL_ORACLE {
        let price_public_key = Pubkey::from_str(oracle.price_address.as_str()).unwrap();
        let mut result = rpc.get_account(&price_public_key).await.unwrap();

        let result = pyth_sdk_solana::load_price_feed_from_account(
          &price_public_key,
          &mut result
        ).unwrap();
        // println!("res: {:?}", result);
        Some(result)
      } else {
        None
        // let price_public_key = Pubkey::from_str(oracle.switchboard_feed_address.as_str()).unwrap();
        // let info = rpc.get_account(&price_public_key).await.unwrap();
        // // const owner = info?.owner.toString();
        // let owner = info.owner;

        // if owner == Pubkey::from_str(Self::SWITCHBOARD_V1_ADDRESS).unwrap() {
        //   // let result = 
        // } else if owner == Pubkey::from_str(Self::SWITCHBOARD_V2_ADDRESS).unwrap() {

        // }
      };

      let solend_cfg = self.solend_cfg.unwrap();


      match price {
        Some(price) => {
          let asset_config = {
            let mut v = model::Asset::default();
            for oracle_asset in &solend_cfg.assets {
              if oracle_asset.symbol == oracle.asset {
                v = oracle_asset.clone();
                break
              }
            }
            v
          };

          Some(OracleData {
            // pub symbol: String,
            // pub reserve_address: Pubkey,
            // pub mint_address: Pubkey,
            // pub decimals: u8,
            // pub price: pyth_sdk_solana::state::Price
            symbol: oracle.asset,
            reserve_address: reserve.address,
            mint_address: asset_config.mint_address,
            decimals: 10usize.pow(asset_config.decimals),
            price
          })
        },
        None => None
      }

    }

    //   pub async fn orderbook(&self, market_name: &str) -> models::Response<models::Orderbook> {
    //     let (signer, client) = (&self.signer, &self.client);
    //     let lsize = 35;
    //     let signature_res = signer.sign_req(
    //         "GET",
    //         format!("/markets/{:}/orderbook?depth={:}", market_name, lsize).as_str(),
    //         None,
    //     );

    //     let request = Request::builder()
    //         .method(Method::GET)
    //         .uri(format!(
    //             "https://ftx.com/api/markets/{:}/orderbook?depth={:}",
    //             market_name, lsize
    //         ))
    //         .header("Content-Type", "application/json")
    //         .header("FTX-KEY", signature_res.api_key)
    //         .header("FTX-SIGN", signature_res.signature)
    //         .header("FTX-TS", signature_res.timestamp)
    //         .body(signature_res.body)
    //         .unwrap();

    //     let res = client.request(request).await.unwrap();

    //     let body = res.into_body();
    //     let body_str = body_to_string(body).await;

    //     let d_markets_response: models::Response<models::Orderbook> =
    //         serde_json::from_str(&body_str).unwrap();
    //     d_markets_response
    // }
}

async fn process_markets(
  client: &Arc<Client>,
  solend_cfg: &'static SolendConfig,
  // runtime_cfg: &'static Config
) {


  let markets_n = solend_cfg.markets.len();
  let mut handles = vec![];

  for i in 0..markets_n {
    // let current_market: &'static _ = Box::leak(Box::new());
    let current_market = solend_cfg.markets[i].clone();
    
    let c_client = Arc::clone(&client);
    let h = tokio::spawn(async move {

      // current_market.
      let market_reserves = &current_market.reserves;

      let response = c_client.get_token_oracle_data(market_reserves).await;
      // process_one_market(

      // );
      println!("resp: {:?}", response)

      // drop(current_market);
    });
    handles.push(h);
  }

  for h in handles {
    h.await.unwrap();
  }
}

// async fn process_one_market(

// ) {

// }

pub async fn run_liquidator() {
  let mut solend_client = Client::new();
  // let solend_client: &mut _ = Box::leak(Box::new(solend_client));

  let solend_cfg = solend_client.get_solend_config().await;
  let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

  solend_client.solend_cfg = Some(solend_cfg);

  let c_solend_client = Arc::new(solend_client);

  process_markets(
    // client: &Arc<Client>,
    // solend_cfg: &'static SolendConfig,
    // runtime_cfg: &'static Config
    &c_solend_client,
    solend_cfg,
  ).await;
  // let runtime_cfg: &'static _ = Box::leak(Box::new(solend_client.get_config()));

  // loop {
    
  // }

  drop(solend_cfg);
}
