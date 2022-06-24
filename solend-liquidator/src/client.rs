use std::collections::HashSet;
use std::ops::{Add, Div, Mul};
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
    solana_client::rpc_config::RpcAccountInfoConfig,
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
    std::{process::exit, str::FromStr},
    system_instruction::create_account,
};

use lazy_static;

use borsh::{BorshDeserialize, BorshSerialize};
use bs58;
use pyth_sdk_solana;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::RpcProgramAccountsConfig;
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, MemcmpEncoding, RpcFilterType};
use solana_program::instruction::Instruction;
use solana_sdk::signers::Signers;
use solend_program::instruction::{
    liquidate_obligation_and_redeem_reserve_collateral, refresh_obligation, refresh_reserve,
};
use solend_program::math::{Decimal, Rate};
use solend_program::state::{Obligation, Reserve};
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account;
use uint::construct_uint;

use crate::model::{self, Asset, Market, Oracles, SolendConfig};
use crate::utils::body_to_string;

construct_uint! {
    pub struct U256(4);
}

pub struct Client {
    client: HyperClient<HttpsConnector<hyper::client::HttpConnector>>,
    config: Config,
    solend_cfg: Option<&'static SolendConfig>,
}

// unsafe impl Sync for Signer {}
// unsafe impl Send for Signer {}

pub struct Config {
    rpc_client: RpcClient,
    signer: Box<Keypair>,
    // lending_program_id: Pubkey,
    // verbose: bool,
    // dry_run: bool,
}

pub fn get_config() -> Config {
    let cli_config = solana_cli_config::Config {
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

#[derive(Debug, Clone)]
pub struct OracleData {
    pub symbol: String,
    pub reserve_address: Pubkey,
    pub mint_address: Pubkey,
    pub decimals: i64,
    pub price: pyth_sdk_solana::state::PriceFeed,
}

#[derive(Debug, Clone)]
pub struct Enhanced<T: Clone> {
    pub inner: T,
    pub pubkey: Pubkey,
}

impl Client {
    const CFG_PRESET: &'static str = "production";

    pub fn new() -> Self {
        let client = HyperClient::builder().build::<_, Body>(HttpsConnector::new());
        let config = get_config();

        Self {
            client,
            config,
            solend_cfg: None,
        }
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

    pub async fn get_token_oracle_data(
        &self,
        market_reserves: &Vec<model::Resef>,
    ) -> Vec<OracleData> {
        let solend_cfg = self.solend_cfg.unwrap();

        let mut oracle_data_list = vec![];
        for market_reserve in market_reserves {
            if let Some(oracle_data) = self
                .get_oracle_data(market_reserve, &solend_cfg.oracles)
                .await
            {
                oracle_data_list.push(oracle_data);
            }
        }
        oracle_data_list
    }

    const NULL_ORACLE: &'static str = "nu11111111111111111111111111111111111111111";
    const SWITCHBOARD_V1_ADDRESS: &'static str = "DtmE9D2CSB4L5D6A15mraeEjrGMm6auWVzgaD8hK2tZM";
    const SWITCHBOARD_V2_ADDRESS: &'static str = "SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f";

    async fn get_oracle_data(
        &self,
        reserve: &model::Resef,
        oracles: &Oracles,
    ) -> Option<OracleData> {
        let oracle = {
            let mut v = model::Asset2::default();
            for oracle_asset in &oracles.assets {
                if oracle_asset.asset == reserve.asset {
                    v = oracle_asset.clone();
                    break;
                }
            }
            v
        };

        let rpc: &RpcClient = &self.config.rpc_client;

        // let price =
        let price = if !oracle.price_address.is_empty() && oracle.price_address != Self::NULL_ORACLE
        {
            let price_public_key = Pubkey::from_str(oracle.price_address.as_str()).unwrap();
            let mut result = rpc.get_account(&price_public_key).await.unwrap();

            let result =
                pyth_sdk_solana::load_price_feed_from_account(&price_public_key, &mut result)
                    .unwrap();
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
                            break;
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
                    reserve_address: Pubkey::from_str(reserve.address.as_str()).unwrap(),
                    mint_address: Pubkey::from_str(asset_config.mint_address.as_str()).unwrap(),
                    decimals: 10i64.pow(asset_config.decimals as u32),
                    price,
                })
            }
            None => None,
        }
    }

    pub async fn get_obligations(&self, market_address: &str) -> Vec<Enhanced<Obligation>> {
        let rpc: &RpcClient = &self.config.rpc_client;
        let solend_cfg = self.solend_cfg.unwrap();

        let program_id = Pubkey::from_str(solend_cfg.program_id.as_str()).unwrap();
        let market_address = Pubkey::from_str(market_address).unwrap();

        let memcmp = RpcFilterType::Memcmp(Memcmp {
            offset: 10,
            bytes: MemcmpEncodedBytes::Bytes(market_address.to_bytes().to_vec()),
            encoding: None,
        });

        let obligations_encoded = rpc
            .get_program_accounts_with_config(
                &program_id,
                RpcProgramAccountsConfig {
                    filters: Some(vec![
                        // RpcFilterType::DataSize(128),
                        memcmp,
                        // export const OBLIGATION_LEN = 1300;
                        RpcFilterType::DataSize(1300),
                    ]),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(rpc.commitment()),
                        data_slice: None,
                        min_context_slot: None,
                    },
                    with_context: None,
                },
            )
            .await;

        if obligations_encoded.is_err() {
            // panic!("none got");
            return vec![];
        }

        let obligations_encoded = obligations_encoded.unwrap();

        // let obligation = Obligation::unpack(&);

        let mut obligations_list = vec![];

        for obligation_encoded in &obligations_encoded {
            let &(obl_pubkey, ref obl_account) = obligation_encoded;
            let obligation = Obligation::unpack(&obl_account.data).unwrap();

            obligations_list.push(Enhanced {
                inner: obligation,
                pubkey: obl_pubkey,
            });
        }

        obligations_list
        // println!("obligation: {:?}", resp);
    }

    pub async fn get_reserves(&self, market_address: &str) -> Vec<Enhanced<Reserve>> {
        let rpc: &RpcClient = &self.config.rpc_client;
        let solend_cfg = self.solend_cfg.unwrap();

        let program_id = Pubkey::from_str(solend_cfg.program_id.as_str()).unwrap();
        let market_address = Pubkey::from_str(market_address).unwrap();

        let memcmp = RpcFilterType::Memcmp(Memcmp {
            offset: 10,
            bytes: MemcmpEncodedBytes::Bytes(market_address.to_bytes().to_vec()),
            encoding: None,
        });

        let reserves_encoded = rpc
            .get_program_accounts_with_config(
                &program_id,
                RpcProgramAccountsConfig {
                    filters: Some(vec![
                        // RpcFilterType::DataSize(128),
                        memcmp,
                        // export const RESERVE_LEN = 619;
                        RpcFilterType::DataSize(619),
                    ]),
                    account_config: RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        commitment: Some(rpc.commitment()),
                        data_slice: None,
                        min_context_slot: None,
                    },
                    with_context: None,
                },
            )
            .await;

        if reserves_encoded.is_err() {
            // panic!("none got");
            return vec![];
        }
        let reserves_encoded = reserves_encoded.unwrap();

        let mut reserves_list = vec![];

        for reserve_item in &reserves_encoded {
            let &(reserve_pubkey, ref reserve_account) = reserve_item;
            let reserve_unpacked = Reserve::unpack(&reserve_account.data).unwrap();

            reserves_list.push(Enhanced {
                inner: reserve_unpacked,
                pubkey: reserve_pubkey,
            });
        }

        reserves_list
    }

    pub fn get_token_info(assets: &Vec<Asset>, symbol: &str) -> Asset {
        let idx = assets.iter().position(|a| a.symbol == symbol).unwrap();
        assets[idx].clone()
    }

    pub async fn liquidate_and_redeem(
        &self,
        // retrieved_wallet_data,
        // selected_borrow.symbol,
        // selected_deposit.symbol,
        // lending_market,
        // obligation
        retrieved_wallet_data: &WalletBalanceData,
        selected_borrow_symbol: String,
        selected_deposit_symbol: String,
        lending_market: Market,
        obligation: &Enhanced<Obligation>,
    ) {
        let solend_cfg = self.solend_cfg.unwrap();

        let payer_pubkey = self.config.signer.pubkey();

        let repay_token_symbol = &selected_borrow_symbol;
        let withdraw_token_symbol = &selected_deposit_symbol;

        let deposit_reserves: Vec<Pubkey> = obligation
            .inner
            .deposits
            .iter()
            .map(|x| x.deposit_reserve)
            .collect();
        let borrow_reserves: Vec<Pubkey> = obligation
            .inner
            .borrows
            .iter()
            .map(|x| x.borrow_reserve)
            .collect();

        let mut uniq_reserve_addresses: HashSet<Pubkey> = HashSet::new();

        for reserve in vec![deposit_reserves, borrow_reserves].concat() {
            uniq_reserve_addresses.insert(reserve);
        }

        let program_id = Pubkey::from_str(solend_cfg.program_id.as_str()).unwrap();

        let mut ixs_list: Vec<Instruction> = uniq_reserve_addresses
            .iter()
            .map(|reserve_addr| {
                let reserve_info = lending_market
                    .reserves
                    .iter()
                    .position(|r| r.address == reserve_addr.to_string())
                    .unwrap();
                let reserve_info = &lending_market.reserves[reserve_info];

                let oracle_info = solend_cfg
                    .oracles
                    .assets
                    .iter()
                    .position(|a| a.asset == reserve_info.asset)
                    .unwrap();
                let oracle_info = &solend_cfg.oracles.assets[oracle_info];

                // build ix
                refresh_reserve(
                    program_id,
                    reserve_addr.clone(),
                    Pubkey::from_str(oracle_info.price_address.as_str()).unwrap(),
                    Pubkey::from_str(oracle_info.switchboard_feed_address.as_str()).unwrap(),
                )
            })
            .collect();

        let refresh_obligation_ix = refresh_obligation(
            // program_id,
            // obligation_pubkey,
            // reserve_pubkeys
            program_id,
            obligation.pubkey,
            HashSet::into_iter(uniq_reserve_addresses).collect(),
        );
        ixs_list.push(refresh_obligation_ix);

        let repay_token_info =
            Self::get_token_info(&solend_cfg.assets, repay_token_symbol.as_str());

        // get account that will be repaying the reserve liquidity
        let repay_account = get_associated_token_address(
            &payer_pubkey,
            &Pubkey::from_str(repay_token_info.mint_address.as_str()).unwrap(),
        );

        let repay_reserve =
            Self::find_where(&lending_market.reserves, |x| &x.asset == repay_token_symbol);
        let withdraw_reserve = Self::find_where(&lending_market.reserves, |x| {
            &x.asset == withdraw_token_symbol
        });
        let withdraw_token_info =
            Self::get_token_info(&solend_cfg.assets, withdraw_token_symbol.as_str());

        let rewarded_withdrawal_collateral_account = get_associated_token_address(
            &payer_pubkey,
            &Pubkey::from_str(withdraw_reserve.collateral_mint_address.as_str()).unwrap(),
        );

        let rewarded_withdrawal_collateral_account_info = self
            .config
            .rpc_client
            .get_account(&rewarded_withdrawal_collateral_account)
            .await;

        if rewarded_withdrawal_collateral_account_info.is_err() {
            // let create_user_collateral_account
            let create_user_collateral_account_ix = create_associated_token_account(
                // funding_address,
                // wallet_address,
                // spl_token_mint_address
                &payer_pubkey,
                &payer_pubkey,
                &Pubkey::from_str(withdraw_reserve.collateral_mint_address.as_str()).unwrap(),
            );
            ixs_list.push(create_user_collateral_account_ix);
        }

        let rewarded_withdrawal_liquidity_account = get_associated_token_address(
            &payer_pubkey,
            &Pubkey::from_str(withdraw_token_info.mint_address.as_str()).unwrap(),
        );
        let rewarded_withdrawal_liquidity_account_info = self
            .config
            .rpc_client
            .get_account(&rewarded_withdrawal_liquidity_account)
            .await;

        if rewarded_withdrawal_liquidity_account_info.is_err() {
            let create_user_liquidity_account_ix = create_associated_token_account(
                // funding_address,
                // wallet_address,
                // spl_token_mint_address
                &payer_pubkey,
                &payer_pubkey,
                &Pubkey::from_str(withdraw_token_info.mint_address.as_str()).unwrap(),
            );
            ixs_list.push(create_user_liquidity_account_ix);
        }

        ixs_list.push(liquidate_obligation_and_redeem_reserve_collateral(
            // program_id,
            // liquidity_amount,
            // source_liquidity_pubkey,
            // destination_collateral_pubkey,
            // destination_liquidity_pubkey,
            // repay_reserve_pubkey,
            // repay_reserve_liquidity_supply_pubkey,
            // withdraw_reserve_pubkey,
            // withdraw_reserve_collateral_mint_pubkey,
            // withdraw_reserve_collateral_supply_pubkey,
            // withdraw_reserve_liquidity_supply_pubkey,
            // withdraw_reserve_liquidity_fee_receiver_pubkey,
            // obligation_pubkey,
            // lending_market_pubkey,
            // user_transfer_authority_pubkey
            program_id,
            retrieved_wallet_data.balance.as_u64(),
            rewarded_withdrawal_collateral_account,
            rewarded_withdrawal_liquidity_account,
            Pubkey::from_str(repay_reserve.address.as_str()).unwrap(),
            Pubkey::from_str(repay_reserve.liquidity_address.as_str()).unwrap(),
            Pubkey::from_str(withdraw_reserve.address.as_str()).unwrap(),
            Pubkey::from_str(withdraw_reserve.collateral_mint_address.as_str()).unwrap(),
            Pubkey::from_str(withdraw_reserve.collateral_supply_address.as_str()).unwrap(),
            Pubkey::from_str(withdraw_reserve.liquidity_address.as_str()).unwrap(),
            Pubkey::from_str(withdraw_reserve.liquidity_fee_receiver_address.as_str()).unwrap(),
            obligation.pubkey,
            Pubkey::from_str(lending_market.address.as_str()).unwrap(),
            Pubkey::from_str(lending_market.authority_address.as_str()).unwrap(),
            payer_pubkey,
        ));

        let recent_blockhash = self.config.rpc_client.get_latest_blockhash().await.unwrap();
        let mut transaction = Transaction::new_with_payer(&ixs_list, Some(&payer_pubkey));

        transaction.sign(&vec![self.config.signer.as_ref()], recent_blockhash);

        let r = self
            .config
            .rpc_client
            .send_and_confirm_transaction_with_spinner(&transaction)
            .await;

        match r {
            Ok(r) => {
                println!("brodcast: signature: {:?}", r);
            }
            Err(e) => {
                println!("broadcast: err: {:?}", e)
            }
        }
    }

    fn find_where<T: Clone, F>(list: &Vec<T>, predicate: F) -> T
    where
        F: FnMut(&T) -> bool,
    {
        let idx = list.iter().position(predicate).unwrap();
        list[idx].clone()
    }
}

// type Borrow = {
//   borrowReserve: PublicKey;
//   borrowAmountWads: BN;
//   marketValue: BigNumber;
//   mintAddress: string,
//   symbol: string;
// };
#[derive(Debug, Clone)]
pub struct Borrow {
    pub borrow_reserve: Pubkey,
    pub borrow_amount_wads: U256,
    pub market_value: U256,
    pub mint_address: Pubkey,
    pub symbol: String,
}

// type Deposit = {
//   depositReserve: PublicKey,
//   depositAmount: BN,
//   marketValue: BigNumber,
//   symbol: string;
// };

#[derive(Debug, Clone)]
pub struct Deposit {
    pub deposit_reserve: Pubkey,
    pub deposit_amount: U256,
    pub market_value: U256,
    pub symbol: String,
}

#[derive(Debug, Clone)]
pub struct RefreshedObligation {
    pub deposited_value: U256,
    pub borrowed_value: U256,
    pub allowed_borrow_value: U256,
    pub unhealthy_borrow_value: U256,
    pub deposits: Vec<Deposit>,
    pub borrows: Vec<Borrow>,
    pub utilization_ratio: U256,
}

pub fn wad() -> U256 {
    U256::from(1000000000000000000u128)
}

pub fn calculate_refreshed_obligation(
    obligation: &Obligation,
    all_reserves: &Vec<Enhanced<Reserve>>,
    tokens_oracle: &Vec<OracleData>,
) -> Option<RefreshedObligation> {
    let mut deposited_value = U256::from(0u32);
    let mut borrowed_value = U256::from(0u32);
    let mut allowed_borrow_value = U256::from(0u32);
    let mut unhealthy_borrow_value = U256::from(0u32);

    let mut deposits: Vec<Deposit> = vec![];
    let mut borrows: Vec<Borrow> = vec![];

    // checked
    for deposit in &obligation.deposits {
        let token_oracle = tokens_oracle
            .iter()
            .position(|x| x.reserve_address == deposit.deposit_reserve);

        if token_oracle.is_none() {
            println!(
                "(d) Missing token info for reserve {:}, skipping this obligation. \n
            Please restart liquidator to fetch latest configs from /v1/config",
                deposit.deposit_reserve
            );
            continue;
        }

        let token_oracle = token_oracle.unwrap();
        let token_oracle = &tokens_oracle[token_oracle];

        let (price, decimals, symbol) = (
            token_oracle.price.get_current_price_unchecked().price,
            token_oracle.decimals,
            token_oracle.symbol.clone(),
        );

        let reserve = all_reserves
            .iter()
            .position(|r| r.pubkey == deposit.deposit_reserve)
            .unwrap();

        let reserve = &all_reserves[reserve];

        // export const WAD = new BigNumber(1000000000000000000);
        let collateral_exchange_rate = reserve.inner.collateral_exchange_rate().unwrap();
        println!(
            "(d) collateral exchange rate: {:?}",
            collateral_exchange_rate
        );

        let market_value = U256::from(deposit.deposited_amount)
            .mul(wad())
            .mul(price)
            .div(Rate::from(collateral_exchange_rate).0.as_u128())
            .div(decimals as u32);
        println!("(d) market_value: {:?}", market_value);

        let loan_to_value_rate = U256::from(reserve.inner.config.loan_to_value_ratio);
        println!("(d): loan_to_value_rate: {:?}", loan_to_value_rate);

        let liquidation_threshold_rate = U256::from(reserve.inner.config.liquidation_threshold);
        println!(
            "(d) liquidation_threshold_rate: {:?}",
            liquidation_threshold_rate
        );

        deposited_value = deposited_value.add(market_value);
        println!("(d) deposited_value: {:?}", deposited_value);
        allowed_borrow_value = allowed_borrow_value.add(market_value * loan_to_value_rate);
        println!("(d) allowed_borrow_value: {:?}", allowed_borrow_value);
        unhealthy_borrow_value =
            unhealthy_borrow_value.add(market_value * liquidation_threshold_rate);
        println!("(d) unhealthy_borrow_value: {:?}", unhealthy_borrow_value);

        let casted_depo = Deposit {
            deposit_reserve: deposit.deposit_reserve,
            deposit_amount: U256::from(deposit.deposited_amount),
            market_value,
            symbol,
        };
        println!("(d) casted_depo: {:?}", casted_depo);
        deposits.push(casted_depo);
    }

    for borrow in &obligation.borrows {
        let borrow_amount_wads = borrow.borrowed_amount_wads.0;

        let token_oracle = tokens_oracle
            .iter()
            .position(|rcl| rcl.reserve_address == borrow.borrow_reserve);

        if token_oracle.is_none() {
            continue;
        }
        let token_oracle = token_oracle.unwrap();

        let token_oracle = &tokens_oracle[token_oracle];

        let (price, decimals, symbol, mint_address) = (
            token_oracle.price.get_current_price_unchecked().price,
            token_oracle.decimals,
            token_oracle.symbol.clone(),
            token_oracle.mint_address,
        );

        let reserve = all_reserves
            .iter()
            .position(|r| r.pubkey == borrow.borrow_reserve)
            .unwrap();
        let reserve = &all_reserves[reserve];

        // reserveCumulativeBorrowRateWads: BigNumber,
        // obligationCumulativeBorrowRateWads: BigNumber,
        // obligationBorrowAmountWads: BigNumber,
        let reserve_cumulative_borrow_rate_wads =
            reserve.inner.liquidity.cumulative_borrow_rate_wads.0;
        let obligation_cumulative_borrow_rate_wads = borrow.cumulative_borrow_rate_wads.0;

        println!(
            "(b) reserve_cumulative_borrow_rate_wads: {:?}",
            reserve_cumulative_borrow_rate_wads
        );
        println!(
            "(b) obligation_cumulative_borrow_rate_wads: {:?}",
            obligation_cumulative_borrow_rate_wads
        );
        /*
          =>
            new BigNumber(reserve.liquidity.cumulativeBorrowRateWads.toString()),
            new BigNumber(borrow.cumulativeBorrowRateWads.toString()),
            borrowAmountWads,

          =>
            reserveCumulativeBorrowRateWads: BigNumber,
            obligationCumulativeBorrowRateWads: BigNumber,
            obligationBorrowAmountWads: BigNumber,

        */
        let borrow_amount_wads_with_interest = {
            match reserve_cumulative_borrow_rate_wads
                .0
                .cmp(&obligation_cumulative_borrow_rate_wads.0)
            {
                std::cmp::Ordering::Less => {
                    println!(
                        "Interest rate cannot be negative. reserveCumulativeBorrowRateWadsNum: {:} |
                        obligationCumulativeBorrowRateWadsNum: {:}`",
                        reserve_cumulative_borrow_rate_wads.to_string(),
                        obligation_cumulative_borrow_rate_wads.to_string()
                    );
                    borrow_amount_wads.clone()
                }
                std::cmp::Ordering::Equal => borrow_amount_wads.clone(),
                std::cmp::Ordering::Greater => {
                    let compound_interest_rate = reserve_cumulative_borrow_rate_wads
                        / obligation_cumulative_borrow_rate_wads;

                    borrow_amount_wads * compound_interest_rate
                }
            }
        };
        println!(
            "(b) borrow_amount_wads_with_interest: {:?}",
            borrow_amount_wads_with_interest
        );

        // let sm_market_value = borrow_amount_wads_with_interest * price / decimals;
        // println!("(b) market_value: {:?}", sm_market_value);

        // type cast from U192 to U256
        // let mut bytes_baw: Vec<u8> = Vec::with_capacity(3 * 64);
        // market_value.to_big_endian(&mut bytes_baw);
        // let market_value = U256::from_big_endian(bytes_baw.as_slice());
        let mut market_value = U256::from(0u32);
        decimal_to_u256(
            &Decimal(borrow_amount_wads_with_interest * price / decimals),
            &mut market_value,
        );
        println!("(b) market_value: {:?}", market_value);

        borrowed_value = borrowed_value + market_value;
        println!("(b) borrowed_value: {:?}", borrowed_value);

        let mut obl_borrow_borrowed_amount_wads = U256::from(0);
        decimal_to_u256(
            &borrow.borrowed_amount_wads,
            &mut obl_borrow_borrowed_amount_wads,
        );

        let casted_borrow = Borrow {
            borrow_reserve: borrow.borrow_reserve,
            borrow_amount_wads: obl_borrow_borrowed_amount_wads,
            mint_address,
            market_value,
            symbol,
        };
        println!("casted_borrow: {:?}", casted_borrow);

        borrows.push(casted_borrow);
    }

    println!("final: borrowed_value: {:?}", borrowed_value);
    println!("final: deposited_value: {:?}", deposited_value);

    println!("final: deposits: {:?}", deposits);
    println!("final: borrows: {:?}", borrows);

    let empty = U256::from(0u8);
    if deposited_value == empty || borrowed_value == empty {
        return None;
    }

    let utilization_ratio = borrowed_value * U256::from(100) / deposited_value;
    println!("utilization_ratio: {:?}", utilization_ratio);

    Some(RefreshedObligation {
        // pub deposited_value: u32,
        // pub borrowed_value: u32,
        // pub allowed_borrow_value: u32,
        // pub unhealthy_borrow_value: u32,
        // pub deposits: Vec<Deposit>,
        // pub borrows: Vec<Borrow>,
        // pub utilization_ratio: U256,
        deposited_value,
        borrowed_value,
        allowed_borrow_value,
        unhealthy_borrow_value,
        deposits,
        borrows,
        utilization_ratio,
    })
}

pub fn decimal_to_u256(decimal: &Decimal, dest: &mut U256) {
    let mut bytes_baw = [0u8; 8 * 3];
    decimal.0.to_little_endian(&mut bytes_baw);
    let de = 1e18 as u128;
    let r = U256::from_little_endian(bytes_baw.as_slice()) / U256::from(de);
    *dest = r;
}

#[test]
fn test_decimal_to_u256() {
    let test_cases: Vec<u128> = vec![
        85345,
        0,
        92358347573475734753727457,
        285,
        3,
        93674,
        12958324752374577235712,
        3945873256,
    ];

    for internal_base in test_cases {
        let decimal = Decimal::from(internal_base);
        let mut dest_u256 = U256::from(0u32);

        decimal_to_u256(&decimal, &mut dest_u256);

        assert_eq!(U256::from(internal_base), dest_u256)
    }
}

#[derive(Debug, Clone)]
pub struct WalletBalanceData {
    pub balance: U256,
    pub symbol: String,
}

async fn get_wallet_token_data(
    client: &Arc<Client>,
    wallet_address: Pubkey,
    mint_address: Pubkey,
    symbol: String,
) -> Option<WalletBalanceData> {
    let user_token_account = get_associated_token_address(&wallet_address, &mint_address);

    match client
        .config
        .rpc_client
        .get_account(&user_token_account)
        .await
    {
        Ok(result_account_info) => {
            let token_data = spl_token::state::Account::unpack(&result_account_info.data).unwrap();

            Some(WalletBalanceData {
                balance: U256::from(token_data.amount),
                symbol,
            })
        }
        Err(_) => None,
    }
}

async fn process_markets(client: Arc<Client>) {
    let solend_cfg = client.solend_cfg.unwrap();
    let markets_n = solend_cfg.markets.len();
    let mut handles = vec![];

    let solend_cfg = client.solend_cfg.unwrap();

    for i in 0..markets_n {
        let current_market = solend_cfg.markets[i].clone();

        let c_client = Arc::clone(&client);
        let h = tokio::spawn(async move {
            let lending_market = current_market.address.clone();

            let (oracle_data, all_obligations, reserves) = tokio::join!(
                c_client.get_token_oracle_data(&current_market.reserves),
                c_client.get_obligations(lending_market.as_str()),
                c_client.get_reserves(lending_market.as_str()),
            );

            let mut inner_handles = vec![];

            for obligation in &all_obligations {
                let oracle_data = oracle_data.clone();
                let c_client = Arc::clone(&c_client);
                let obligation = obligation.clone();
                let current_market = current_market.clone();
                let lending_market = lending_market.clone();
                let reserves = reserves.clone();

                let h = tokio::spawn(async move {
                    let refreshed_obligation = calculate_refreshed_obligation(
                        // obligation: &Obligation,
                        // all_reserves: &Vec<Enhanced<Reserve>>,
                        // tokens_oracle: &Vec<OracleData>,
                        &obligation.inner,
                        &reserves,
                        &oracle_data,
                    );

                    if refreshed_obligation.is_none() {
                        return;
                    }

                    let refreshed_obligation = refreshed_obligation.unwrap();
                    let (borrowed_value, unhealthy_borrow_value, deposits, borrows) = (
                        refreshed_obligation.borrowed_value,
                        refreshed_obligation.unhealthy_borrow_value,
                        refreshed_obligation.deposits,
                        refreshed_obligation.borrows,
                    );

                    // let (borrowed)

                    if borrowed_value <= unhealthy_borrow_value {
                        println!("do nothing if obligation is healthy");
                        return;
                    }

                    // select repay token that has the highest market value
                    let selected_borrow = {
                        let mut v: Option<Borrow> = None;
                        for borrow in borrows {
                            // if v.is_none() || deposit.market_value >= v.unwrap().market_value
                            match v {
                                Some(ref real_v) => {
                                    if borrow.market_value >= real_v.market_value {
                                        v = Some(borrow);
                                    }
                                }
                                None => v = Some(borrow),
                            }
                        }
                        v
                    };

                    // select the withdrawal collateral token with the highest market value
                    let selected_deposit = {
                        let mut v: Option<Deposit> = None;
                        for deposit in deposits {
                            // if v.is_none() || deposit.market_value >= v.unwrap().market_value
                            match v {
                                Some(ref real_v) => {
                                    if deposit.market_value >= real_v.market_value {
                                        v = Some(deposit);
                                    }
                                }
                                None => v = Some(deposit),
                            }
                        }
                        v
                    };

                    if selected_deposit.is_none() || selected_borrow.is_none() {
                        println!("skip toxic obligations caused by toxic oracle data");
                        return;
                    }
                    let selected_deposit = selected_deposit.unwrap();
                    let selected_borrow = selected_borrow.unwrap();

                    println!(
                        "obligation: {:} is underwater",
                        obligation.pubkey.to_string()
                    );
                    println!("borrowed_value: {:} ", borrowed_value.to_string());
                    println!(
                        "unhealthy_borrow_value: {:} ",
                        unhealthy_borrow_value.to_string()
                    );
                    println!("market addr: {:} ", lending_market);

                    let wallet_address = c_client.config.signer.pubkey();
                    let retrieved_wallet_data = get_wallet_token_data(
                        &c_client,
                        wallet_address,
                        selected_borrow.mint_address,
                        selected_borrow.symbol.clone(),
                    )
                    .await
                    .unwrap();

                    println!("retrieved_wallet_data: {:?}", retrieved_wallet_data);

                    let u_zero = U256::from(0);
                    if retrieved_wallet_data.balance == u_zero {
                        println!(
                            "insufficient {:} to liquidate obligation {:} in market: {:}",
                            selected_borrow.symbol,
                            obligation.pubkey.to_string(),
                            lending_market
                        );
                        return;
                    } else if retrieved_wallet_data.balance < u_zero {
                        println!("failed to get wallet balance for {:} to liquidate obligation {:} in market {:}", selected_borrow.symbol, obligation.pubkey.to_string(), lending_market);
                        println!(
                            "potentially network error or token account does not exist in wallet"
                        );
                        return;
                    }

                    c_client
                        .liquidate_and_redeem(
                            &retrieved_wallet_data,
                            selected_borrow.symbol,
                            selected_deposit.symbol,
                            current_market.clone(),
                            &obligation,
                        )
                        .await;
                });

                inner_handles.push(h);
            }

            for inner_h in inner_handles {
                inner_h.await;
            }
        });
        handles.push(h);
    }

    for h in handles {
        h.await;
    }
}

pub async fn run_liquidator() {
    let mut solend_client = Client::new();
    // let solend_client: &mut _ = Box::leak(Box::new(solend_client));

    let solend_cfg = solend_client.get_solend_config().await;
    let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

    solend_client.solend_cfg = Some(solend_cfg);

    let c_solend_client = Arc::new(solend_client);

    loop {
      let c_solend_client = Arc::clone(&c_solend_client);
      process_markets(c_solend_client).await;
    }

    drop(solend_cfg);
}
