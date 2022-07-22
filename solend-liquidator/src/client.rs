use std::collections::{HashMap, HashSet};
use std::ops::{Add, Div, Mul};
use std::sync::Arc;

use hyper::Body;

use hyper::{Client as HyperClient, Method, Request};
use hyper_tls::HttpsConnector;

use borsh::BorshDeserialize;

use {
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_client::rpc_config::RpcAccountInfoConfig,
    solana_program::{program_pack::Pack, pubkey::Pubkey},
    solana_sdk::{
        commitment_config::CommitmentConfig,
        signature::{Keypair, Signer},
        transaction::Transaction,
    },
    std::str::FromStr,
};

use futures_retry::{FutureFactory, FutureRetry, RetryPolicy};

use log::Log;
use pyth_sdk_solana;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcProgramAccountsConfig, RpcSendTransactionConfig};
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};

use solana_client::rpc_request::RpcError;
use solana_program::instruction::Instruction;
use solana_sdk::account::create_is_signer_account_infos;
use solana_sdk::commitment_config::CommitmentLevel;

use solend_program::math::{Decimal, Rate};
use solend_program::state::{Obligation, Reserve};

use spl_associated_token_account::get_associated_token_address;

use switchboard_program::AggregatorState;
use uint::construct_uint;

use crate::log::Logger;
use crate::model::{self, Market, SolendConfig};
use crate::utils::body_to_string;

construct_uint! {
    pub struct U256(4);
}

fn handle_error<E: std::error::Error>(e: E) -> RetryPolicy<E> {
    RetryPolicy::WaitRetry(std::time::Duration::from_millis(150))
    // RetryPolicy::ForwardError(e)
}

pub struct Client {
    client: HyperClient<HttpsConnector<hyper::client::HttpConnector>>,
    config: Config,
    solend_cfg: Option<&'static SolendConfig>,
    logger: Arc<Box<Logger>>,
}

pub struct Config {
    rpc_client: Arc<Box<RpcClient>>,
    signer: Box<Keypair>,
}

pub fn get_config(keypair_path: String) -> Config {
    let cli_config = solana_cli_config::Config {
        keypair_path,
        ..solana_cli_config::Config::default()
    };

    let json_rpc_url = String::from("https://polished-patient-brook.solana-mainnet.quiknode.pro/57a057b48182876ac38c7fb2131d8418e0a92f43/");

    let signer = solana_sdk::signer::keypair::read_keypair_file(cli_config.keypair_path).unwrap();

    Config {
        rpc_client: Arc::new(Box::new(RpcClient::new_with_commitment(
            json_rpc_url,
            CommitmentConfig::confirmed(),
        ))),
        signer: Box::new(signer),
        // lending_program_id,
        // verbose,
        // dry_run,
    }
}

#[derive(Debug, Clone)]
pub struct OracleData {
    pub symbol: String,
    pub reserve_address: Pubkey,
    pub mint_address: Pubkey,
    pub decimals: i64,
    pub price: U256, // pub price: pyth_sdk_solana::state::PriceFeed,
}

#[derive(Debug, Clone)]
pub struct Enhanced<T: Clone> {
    pub inner: T,
    pub pubkey: Pubkey,
}

lazy_static::lazy_static! {
    static ref SWITCHBOARD_V1_ADDRESS: Pubkey =
        Pubkey::from_str("DtmE9D2CSB4L5D6A15mraeEjrGMm6auWVzgaD8hK2tZM").unwrap();
    static ref SWITCHBOARD_V2_ADDRESS: Pubkey =
        Pubkey::from_str("SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f").unwrap();
    static ref NULL_ORACLE: Pubkey =
        Pubkey::from_str("nu11111111111111111111111111111111111111111").unwrap();
    static ref SOLEND_PROGRAM_ID: Pubkey =
        Pubkey::from_str("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo").unwrap();
}

impl Client {
    const CFG_PRESET: &'static str = "production";

    pub fn new(keypair_path: String) -> Self {
        let client = HyperClient::builder().build::<_, Body>(HttpsConnector::new());
        let config = get_config(keypair_path);

        Self {
            client,
            config,
            solend_cfg: None,
            logger: Arc::new(Box::new(Logger::new())),
        }
    }

    pub async fn get_solend_config(&self) -> SolendConfig {
        let request = Request::builder()
            .method(Method::GET)
            .uri(format!(
                "https://api.solend.fi/v1/markets/configs?scope=all&deployment={:}",
                Self::CFG_PRESET
            ))
            .header("Content-Type", "application/json")
            .body(Body::from(""))
            .unwrap();

        let res = self.client.request(request).await.unwrap();

        let body = res.into_body();
        let body_str = body_to_string(body).await;
        // println!("body_str: {:?}", body_str);

        let solend_cfg: SolendConfig = serde_json::from_str(&body_str).unwrap();

        solend_cfg
    }

    pub async fn get_token_oracle_data(
        &self,
        market_reserves: &Vec<model::Resef>,
    ) -> Vec<OracleData> {
        let rpc = &self.config.rpc_client;
        let mut oracle_data_list = vec![];
        let mut handles = vec![];

        for market_reserve in market_reserves {
            let c_rpc = Arc::clone(&rpc);
            let market_reserve = market_reserve.clone();

            let h = tokio::spawn(async move {
                if let Some(oracle_data) = Self::get_oracle_data(&c_rpc, &market_reserve).await {
                    Some(oracle_data)
                } else {
                    None
                }
            });

            handles.push(h);
        }

        for h in handles {
            match h.await.unwrap() {
                Some(v) => {
                    oracle_data_list.push(v);
                }
                None => {
                    continue;
                }
            }
        }

        oracle_data_list
    }

    // const NULL_ORACLE: &'static str = "nu11111111111111111111111111111111111111111";
    // const SWITCHBOARD_V1_ADDRESS: &'static str = "DtmE9D2CSB4L5D6A15mraeEjrGMm6auWVzgaD8hK2tZM";
    // const SWITCHBOARD_V2_ADDRESS: &'static str = "SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f";

    async fn get_oracle_data(
        rpc: &Arc<Box<RpcClient>>,
        reserve: &model::Resef,
    ) -> Option<OracleData> {
        // let oracle = {
        //     let mut v = Default::default();
        //     for oracle_asset in &reserve {
        //         if oracle_asset.address == reserve.address {
        //             v = oracle_asset.clone();
        //             break;
        //         }
        //     }
        //     v
        // };

        // let rpc: &RpcClient = &self.config.rpc_client;
        // let price =

        let price = if !reserve.pyth_oracle.is_empty()
            && Pubkey::from_str(reserve.pyth_oracle.as_str()).unwrap() != *NULL_ORACLE
        {
            let price_public_key = Pubkey::from_str(reserve.pyth_oracle.as_str()).unwrap();
            let mut result = rpc.get_account(&price_public_key).await.unwrap();

            let result =
                pyth_sdk_solana::load_price_feed_from_account(&price_public_key, &mut result)
                    .unwrap();

            // println!(
            //     "🎱 oracle: 1st case: {:?}",
            //     result.get_current_price_unchecked().price
            // );
            // logger
            Some(U256::from(result.get_current_price_unchecked().price))
        } else {
            let price_public_key = Pubkey::from_str(reserve.switchboard_oracle.as_str()).unwrap();
            let info = rpc.get_account(&price_public_key).await.unwrap();
            let owner = info.owner.clone();

            if owner == *SWITCHBOARD_V1_ADDRESS {
                // println!("byte len: {:?}", info.data.len());
                // println!("bytes[0]: {:?}", info.data[0]);
                // println!("bytes[1]: {:?}", info.data[1]);
                // println!("AggregatorState::LEN : {:?}", AggregatorState);

                let result = AggregatorState::try_from_slice(&info.data);

                if result.is_err() {
                    return None;
                }

                let result = result.unwrap();
                // println!(
                //     "🎱 oracle: 2nd case: {:?}",
                //     result.last_round_result.as_ref().unwrap().result.unwrap() as i64
                // );

                Some(U256::from(
                    result.last_round_result.unwrap().result.unwrap() as i64,
                ))
            } else if owner == *SWITCHBOARD_V2_ADDRESS {
                let mut info = info.clone();
                let inner_accs = vec![(&price_public_key, false, &mut info)];
                let inner_accs = Box::leak(Box::new(inner_accs));

                let info = create_is_signer_account_infos(&mut (*inner_accs));
                let result = switchboard_v2::AggregatorAccountData::new(&info[0]).unwrap();
                let retrieved = result.get_result().unwrap();

                let retrieved: f64 = retrieved.try_into().unwrap();
                // println!("🎱 oracle: 3rd case: {:?}", retrieved);

                Some(U256::from(retrieved as i64))
                // Some(retrieved.to_f64())
                // match retrieved {
                //     Ok(v) => {
                //         println!("🎱 oracle: 3rd case: {:?}", v.result.as_ref().unwrap());
                //         Some(U256::from(v.result.unwrap() as i64))
                //     }

                //     Err(_) => None,
                // }
            } else {
                // println!(
                //     "🎱 oracle: unrecognized switchboard owner address: {:}",
                //     owner
                // );
                None
            }
        };

        // let solend_cfg = self.solend_cfg.unwrap();

        match price {
            Some(price) => {
                Some(OracleData {
                    // pub symbol: String,
                    // pub reserve_address: Pubkey,
                    // pub mint_address: Pubkey,
                    // pub decimals: u8,
                    // pub price: pyth_sdk_solana::state::Price
                    symbol: reserve.liquidity_token.symbol.clone(),
                    reserve_address: Pubkey::from_str(reserve.address.as_str()).unwrap(),
                    mint_address: Pubkey::from_str(reserve.liquidity_token.mint.as_str()).unwrap(),
                    decimals: 10i64.pow(reserve.liquidity_token.decimals as u32),
                    price,
                })
            }
            None => None,
        }
    }

    pub async fn get_obligations(&self, market_address: &str) -> Vec<Enhanced<Obligation>> {
        let rpc: &RpcClient = &self.config.rpc_client;
        let _solend_cfg = self.solend_cfg.unwrap();

        let program_id = SOLEND_PROGRAM_ID.clone();
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
        let _solend_cfg = self.solend_cfg.unwrap();

        let program_id = SOLEND_PROGRAM_ID.clone();
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

    // pub fn get_token_info(assets: &Vec<Asset>, symbol: &str) -> Asset {
    //     let idx = assets.iter().position(|a| a.symbol == symbol).unwrap();
    //     assets[idx].clone()
    // }

    async fn get_or_create_account_data(
        funding_address: &Pubkey,
        wallet_address: &Pubkey,
        spl_token_mint_address: &Pubkey,
        rpc_client: &RpcClient,
        signer: &Keypair,
    ) -> Pubkey {
        let associated_token_address = spl_associated_token_account::get_associated_token_address(
            wallet_address,
            spl_token_mint_address,
        );

        match rpc_client.get_account_data(&associated_token_address).await {
            Err(_) => {
                println!("account {:?} is empty. creating...", associated_token_address.to_string());
                
                let ix = spl_associated_token_account::instruction::create_associated_token_account(
                    funding_address,
                    wallet_address,
                    spl_token_mint_address,
                );

                let recent_blockhash = rpc_client.get_latest_blockhash().await.unwrap();
                let mut transaction = Transaction::new_with_payer(&[ix], Some(&signer.pubkey()));

                transaction.sign(&vec![signer], recent_blockhash);

                let transaction: &'static _ = Box::leak(Box::new(transaction.clone()));

                let r = rpc_client
                    .send_and_confirm_transaction(transaction)
                    .await
                    .unwrap();

                println!("created account {:}! tx: {:?}", associated_token_address.to_string(), r);

                associated_token_address
            },
            _ => {
                println!("account {:?} is not empty", associated_token_address.to_string());

                associated_token_address
            }
        }
    }

    async fn get_or_substitute_account_data(
        funding_address: &Pubkey,
        wallet_address: &Pubkey,
        spl_token_mint_address: &Pubkey,
        rpc_client: &RpcClient,
        _signer: &Keypair,
    ) -> (Pubkey, Option<Instruction>) {
        let associated_token_address = spl_associated_token_account::get_associated_token_address(
            wallet_address,
            spl_token_mint_address,
        );

        match rpc_client.get_account_data(&associated_token_address).await {
            Err(_) => {
                let ix = spl_associated_token_account::instruction::create_associated_token_account(
                    funding_address,
                    wallet_address,
                    spl_token_mint_address,
                );

                (associated_token_address, Some(ix))
            },
            _ => (associated_token_address, None)
        }

    }

    pub async fn liquidate_and_redeem(
        &self,
        retrieved_wallet_data: &WalletBalanceData,
        // selected_borrow_symbol: String,
        // selected_deposit_symbol: String,
        selected_borrow: &Borrow,
        selected_deposit: &Deposit,
        lending_market: Market,
        obligation: &Enhanced<Obligation>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _solend_cfg = self.solend_cfg.unwrap();
        let payer_pubkey = self.config.signer.pubkey();

        let repay_token_symbol = &selected_borrow.symbol;
        let withdraw_token_symbol = &selected_deposit.symbol;

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

        let uniq_reserve_addresses: Vec<Pubkey> = uniq_reserve_addresses.into_iter().collect();

        let mut ixs = vec![];
        for reserve in &uniq_reserve_addresses {
            let reserve_info = Self::find_where(&lending_market.reserves, |x| {
                Pubkey::from_str(x.address.as_str()).unwrap() == *reserve
            });

            // const refreshReserveIx = refreshReserveInstruction(
            //     new PublicKey(reserveAddress),
            //     new PublicKey(reserveInfo.pythOracle),
            //     new PublicKey(reserveInfo.switchboardOracle),
            // );
            // ixs.push(refreshReserveIx);

            let refresh_reserve_ix = solend_program::instruction::refresh_reserve(
                // program_id,
                // reserve_pubkey,
                // reserve_liquidity_pyth_oracle_pubkey,
                // reserve_liquidity_switchboard_oracle_pubkey
                *SOLEND_PROGRAM_ID,
                Pubkey::from_str(reserve_info.address.as_str()).unwrap(),
                Pubkey::from_str(&reserve_info.pyth_oracle.as_str()).unwrap(),
                Pubkey::from_str(reserve_info.switchboard_oracle.as_str()).unwrap(),
            );
            ixs.push(refresh_reserve_ix);
        }

        let _refresh_obligation_ix = solend_program::instruction::refresh_obligation(
            // program_id,
            // obligation_pubkey,
            // reserve_pubkeys
            SOLEND_PROGRAM_ID.clone(),
            obligation.pubkey,
            uniq_reserve_addresses.clone(),
        );

        let repay_token_info = Self::find_where(&lending_market.reserves, |x| {
            x.liquidity_token.symbol == *repay_token_symbol
        });

        // get account that will be repaying the reserve liquidity
        let repay_account = spl_associated_token_account::get_associated_token_address(
            // wallet_address,
            // spl_token_mint_address
            &payer_pubkey,
            &Pubkey::from_str(repay_token_info.liquidity_token.mint.as_str()).unwrap(),
        );

        //   const reserveSymbolToReserveMap = new Map<string, MarketConfigReserve>(
        //     lendingMarket.reserves.map((reserve) => [reserve.liquidityToken.symbol, reserve]),
        //   );
        let mut reserve_symbol_to_reserve_map: HashMap<String, model::Resef> = HashMap::new();
        for r in &lending_market.reserves {
            reserve_symbol_to_reserve_map.insert(r.liquidity_token.symbol.clone(), r.clone());
        }

        let repay_reserve = reserve_symbol_to_reserve_map.get(repay_token_symbol);
        let withdraw_reserve = reserve_symbol_to_reserve_map.get(withdraw_token_symbol);

        let withdraw_token_info = Self::find_where(&lending_market.reserves, |x| {
            x.liquidity_token.symbol == *withdraw_token_symbol
        });

        // if (!withdrawReserve || !repayReserve) {
        //     throw new Error('reserves are not identified');
        // }
        if withdraw_reserve.is_none() || repay_reserve.is_none() {
            return Err(Box::new(
                LiquidationAndRedeemError::ReservesAreNotIdentified,
            ));
        }

        let withdraw_reserve = withdraw_reserve.unwrap();
        let repay_reserve = repay_reserve.unwrap();

        // let rewarded_withdrawal_collateral_account =
        //     spl_associated_token_account::get_associated_token_address(
        //         // wallet_address,
        //         // spl_token_mint_address
        //         &payer_pubkey,
        //         &Pubkey::from_str(withdraw_reserve.collateral_mint_address.as_str()).unwrap(),
        //     );
        // let rewarded_withdrawal_collateral_account_info = self
        //     .config
        //     .rpc_client
        //     .get_account(&rewarded_withdrawal_collateral_account)
        //     .await
        //     .unwrap();
        let (rewarded_withdrawal_collateral_account, rewarded_withdrawal_collateral_account_ix) =
            Self::get_or_substitute_account_data(
                &payer_pubkey,
                &payer_pubkey,
                &Pubkey::from_str(withdraw_reserve.collateral_mint_address.as_str()).unwrap(),
                &self.config.rpc_client,
                &self.config.signer,
            )
            .await;
        if let Some(ix) = rewarded_withdrawal_collateral_account_ix {
            ixs.push(ix);
        }

        let (rewarded_withdrawal_liquidity_account, rewarded_withdrawal_liquidity_account_ix) =
            Self::get_or_substitute_account_data(
                &payer_pubkey,
                &payer_pubkey,
                &Pubkey::from_str(&withdraw_token_info.liquidity_token.mint.as_str()).unwrap(),
                &self.config.rpc_client,
                &self.config.signer,
            )
            .await;
        if let Some(ix) = rewarded_withdrawal_liquidity_account_ix {
            ixs.push(ix);
        }

        ixs.push(
            solend_program::instruction::liquidate_obligation_and_redeem_reserve_collateral(
                // program_id,
                SOLEND_PROGRAM_ID.clone(),
                // liquidity_amount,
                retrieved_wallet_data.balance.as_u64(),
                // source_liquidity_pubkey,
                repay_account,
                // destination_collateral_pubkey,
                rewarded_withdrawal_collateral_account,
                // destination_liquidity_pubkey,
                rewarded_withdrawal_liquidity_account,
                // repay_reserve_pubkey,
                Pubkey::from_str(repay_reserve.address.as_str()).unwrap(),
                // repay_reserve_liquidity_supply_pubkey,
                Pubkey::from_str(repay_reserve.liquidity_address.as_str()).unwrap(),
                Pubkey::from_str(withdraw_reserve.address.as_str()).unwrap(),
                Pubkey::from_str(withdraw_reserve.collateral_mint_address.as_str()).unwrap(),
                // withdraw_reserve_pubkey,
                Pubkey::from_str(withdraw_reserve.collateral_supply_address.as_str()).unwrap(),
                // withdraw_reserve_collateral_mint_pubkey,
                Pubkey::from_str(withdraw_reserve.liquidity_address.as_str()).unwrap(),
                // withdraw_reserve_collateral_supply_pubkey,
                Pubkey::from_str(withdraw_reserve.liquidity_fee_receiver_address.as_str()).unwrap(),
                // withdraw_reserve_liquidity_supply_pubkey,
                obligation.pubkey,
                // withdraw_reserve_liquidity_fee_receiver_pubkey,
                Pubkey::from_str(lending_market.address.as_str()).unwrap(),
                // obligation_pubkey,
                payer_pubkey,
                // lending_market_pubkey,
            ),
        );
        let recent_blockhash = self.config.rpc_client.get_latest_blockhash().await.unwrap();
        let mut transaction = Transaction::new_with_payer(&ixs, Some(&self.config.signer.pubkey()));
        transaction.sign(&[self.config.signer.as_ref()], recent_blockhash);
        let transaction: &'static _ = Box::leak(Box::new(transaction.clone()));

        let r = FutureRetry::new(
            move || {
                self.config
                    .rpc_client
                    .send_transaction_with_config(
                        transaction,
                        RpcSendTransactionConfig {
                            skip_preflight: true,
                            preflight_commitment: Some(CommitmentLevel::Confirmed),
                            ..RpcSendTransactionConfig::default()
                        },
                    )
            },
            handle_error,
        )
        .await;

        match r {
            Ok((r, _)) => {
                println!(" 🚀 ✅ 🚀  brodcast: signature: {:?}", r);
            }
            Err(e) => {
                println!(" 🚀 ❌ 🚀 broadcast: err: {:?}", e);
            }
        }

        Ok(())
    }

    fn find_where<T: Clone, F>(list: &Vec<T>, predicate: F) -> T
    where
        F: FnMut(&T) -> bool,
    {
        let idx = list.iter().position(predicate).unwrap();
        list[idx].clone()
    }
}

#[derive(thiserror::Error, Debug)]
pub enum LiquidationAndRedeemError {
    #[error("reserves are not identified")]
    ReservesAreNotIdentified,
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
    logger: &Arc<Box<Logger>>,
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
                " 📥  Missing token info for reserve {:}, skipping this obligation. \n
            Please restart liquidator to fetch latest configs from /v1/config",
                deposit.deposit_reserve
            );
            continue;
        }

        let token_oracle = token_oracle.unwrap();
        let token_oracle = &tokens_oracle[token_oracle];

        let (price, decimals, symbol) = (
            token_oracle.price,
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
        // println!(
        //     " 📥  collateral exchange rate: {:?}",
        //     collateral_exchange_rate
        // );

        let market_value = U256::from(deposit.deposited_amount)
            .mul(wad())
            .mul(price)
            .div(Rate::from(collateral_exchange_rate).0.as_u128())
            .div(decimals as u32);
        // println!(" 📥  market_value: {:?}", market_value);

        let loan_to_value_rate = U256::from(reserve.inner.config.loan_to_value_ratio);
        // println!(" 📥 : loan_to_value_rate: {:?}", loan_to_value_rate);

        let liquidation_threshold_rate = U256::from(reserve.inner.config.liquidation_threshold);
        // println!(
        //     " 📥  liquidation_threshold_rate: {:?}",
        //     liquidation_threshold_rate
        // );

        deposited_value = deposited_value.add(market_value);
        // println!(" 📥  deposited_value: {:?}", deposited_value);
        allowed_borrow_value = allowed_borrow_value.add(market_value * loan_to_value_rate);
        // println!(" 📥  allowed_borrow_value: {:?}", allowed_borrow_value);
        unhealthy_borrow_value =
            unhealthy_borrow_value.add(market_value * liquidation_threshold_rate);
        // println!(" 📥  unhealthy_borrow_value: {:?}", unhealthy_borrow_value);

        let casted_depo = Deposit {
            deposit_reserve: deposit.deposit_reserve,
            deposit_amount: U256::from(deposit.deposited_amount),
            market_value,
            symbol,
        };
        // println!(" 📥  casted_depo: {:?}", casted_depo);
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
            token_oracle.price,
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

        // println!(
        //     " 📤  reserve_cumulative_borrow_rate_wads: {:?}",
        //     reserve_cumulative_borrow_rate_wads
        // );
        // println!(
        //     " 📤  obligation_cumulative_borrow_rate_wads: {:?}",
        //     obligation_cumulative_borrow_rate_wads
        // );

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
        let borrow_amount_wads_with_interest_u192 = {
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
        // println!(
        //     " 📤  borrow_amount_wads_with_interest: {:?}",
        //     borrow_amount_wads_with_interest_u192
        // );

        let mut borrow_amount_wads_with_interest = U256::from(0u32);

        decimal_to_u256(
            &Decimal(borrow_amount_wads_with_interest_u192),
            &mut borrow_amount_wads_with_interest,
        );

        let market_value = borrow_amount_wads_with_interest * price / decimals;
        // println!(" 📤  market_value: {:?}", market_value);

        borrowed_value = borrowed_value + market_value;
        // println!(" 📤  borrowed_value: {:?}", borrowed_value);

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
        // println!("casted_borrow: {:?}", casted_borrow);

        borrows.push(casted_borrow);
    }

    // println!(" ✉️ : borrowed_value: {:?}", borrowed_value);
    // println!(" ✉️ : deposited_value: {:?}", deposited_value);

    // println!(" ✉️ : deposits: {:?}", deposits);
    // println!(" ✉️ : borrows: {:?}", borrows);

    let empty = U256::from(0u8);
    if deposited_value == empty || borrowed_value == empty {
        return None;
    }

    let utilization_ratio = borrowed_value * U256::from(100) / deposited_value;
    // println!("utilization_ratio: {:?}", utilization_ratio);

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
        857345777777777777777777,
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
    let markets_list = client.solend_cfg.unwrap();
    let mut handles = vec![];

    // let solend_cfg = client.solend_cfg.unwrap();

    for current_market in markets_list {
        let c_client = Arc::clone(&client);
        let logger = Arc::clone(&c_client.logger);

        let h = tokio::spawn(async move {
            let lending_market = current_market.address.clone();

            let (oracle_data, all_obligations, reserves) = tokio::join!(
                c_client.get_token_oracle_data(&current_market.reserves),
                c_client.get_obligations(lending_market.as_str()),
                c_client.get_reserves(lending_market.as_str()),
            );
            // println!("oracle_data: {:?}", oracle_data);
            // println!("all_obligations: {:?}", all_obligations);
            // println!("reserves: {:?}", reserves);

            let mut inner_handles = vec![];

            for obligation in &all_obligations {
                let oracle_data = oracle_data.clone();
                let c_client = Arc::clone(&c_client);
                let obligation = obligation.clone();
                let current_market = current_market.clone();
                let lending_market = lending_market.clone();
                let reserves = reserves.clone();

                let logger = Arc::clone(&logger);

                let h = tokio::spawn(async move {
                    let refreshed_obligation = calculate_refreshed_obligation(
                        // obligation: &Obligation,
                        // all_reserves: &Vec<Enhanced<Reserve>>,
                        // tokens_oracle: &Vec<OracleData>,
                        &obligation.inner,
                        &reserves,
                        &oracle_data,
                        &logger,
                    );
                    // println!("refreshed_obligation: {:?}", refreshed_obligation);

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

                    if borrowed_value <= unhealthy_borrow_value {
                        // println!("do nothing if obligation is healthy");
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
                    format!("market addr: {:} ", lending_market);

                    let wallet_address = c_client.config.signer.pubkey();
                    println!(
                        "
                        wallet_address: {:?}
                        selected_borrow.mint_address: {:?}
                        selected_borrow.symbol: {:?}
                    ",
                        wallet_address,
                        selected_borrow.mint_address,
                        selected_borrow.symbol.clone(),
                    );

                    Client::get_or_create_account_data(
                        &c_client.config.signer.pubkey(),
                        &c_client.config.signer.pubkey(),
                        &selected_borrow.mint_address,
                        &c_client.config.rpc_client,
                        &c_client.config.signer,
                    )
                    .await;
                    // panic!("");
                    let retrieved_wallet_data = get_wallet_token_data(
                        &c_client,
                        wallet_address,
                        selected_borrow.mint_address,
                        selected_borrow.symbol.clone(),
                    )
                    .await
                    .unwrap();

                    println!(
                        " 🛢 🛢 🛢  retrieved_wallet_data: {:?}",
                        retrieved_wallet_data
                    );

                    // println!("selected_borrow: {:?}", &selected_borrow);
                    // println!("selected_deposit: {:?}", &selected_deposit);

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
                        println!("potentially network error or token account does not exist in wallet");
                        return;
                    }

                    c_client
                        .liquidate_and_redeem(
                            &retrieved_wallet_data,
                            &selected_borrow,
                            &selected_deposit,
                            current_market.clone(),
                            &obligation,
                        )
                        .await
                        .unwrap();
                });

                inner_handles.push(h);
            }

            for inner_h in inner_handles {
                inner_h.await.unwrap();
            }
        });

        handles.push(h);
    }

    let total_n = handles.len();
    println!("total n: {:?}", total_n);

    let mut cur = 0usize;
    for h in handles {
        h.await.unwrap();
        cur += 1;

        println!("current iters: {:?}", cur);
    }
}

pub async fn run_eternal_liquidator(keypair_path: String) {
    let mut solend_client = Client::new(keypair_path);

    let solend_cfg = solend_client.get_solend_config().await;
    let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

    solend_client.solend_cfg = Some(solend_cfg);

    let c_solend_client = Arc::new(solend_client);

    loop {
        let c_solend_client = Arc::clone(&c_solend_client);
        process_markets(c_solend_client).await;
    }
}

pub async fn run_liquidator_iter(keypair_path: String) {
    let mut solend_client = Client::new(keypair_path);

    let solend_cfg = solend_client.get_solend_config().await;
    let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

    solend_client.solend_cfg = Some(solend_cfg);

    let c_solend_client = Arc::new(solend_client);

    process_markets(c_solend_client).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_market_iter() {
        run_liquidator_iter(String::from("./private/liquidator_main.json")).await;
    }
}
