use std::collections::{HashMap, HashSet};
use std::ops::{Add, Div, Mul};
use std::sync::Arc;

use hyper::Body;

use hyper::{Client as HyperClient, Method, Request};
use hyper_tls::HttpsConnector;

use async_trait::async_trait;
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

use either::Either;
use futures_retry::{FutureFactory, FutureRetry, RetryPolicy};
use parking_lot::{Mutex, RwLock};

use log::Log;
use pyth_sdk_solana;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcProgramAccountsConfig, RpcSendTransactionConfig};
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};

use solana_program::instruction::Instruction;
use solana_sdk::account::create_is_signer_account_infos;
use solana_sdk::commitment_config::CommitmentLevel;

use solend_program::math::{Decimal, Rate};
use solend_program::state::{Obligation, ObligationCollateral, ObligationLiquidity, Reserve};

use spl_associated_token_account::get_associated_token_address;

use switchboard_program::AggregatorState;
use uint::construct_uint;

use crate::log::Logger;
use crate::model::{self, Market, SolendConfig};
use crate::performance::PerformanceMeter;
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

struct FormedOracle {
    pub price_address: Pubkey,
    pub switchboard_feed_address: Pubkey,
}

pub fn get_config(keypair_path: String) -> Config {
    let cli_config = solana_cli_config::Config {
        keypair_path,
        ..solana_cli_config::Config::default()
    };

    let json_rpc_url = String::from("https://broken-dawn-field.solana-mainnet.quiknode.pro/52908360084c7e0666532c96647b9b239ec5cadf/");

    let signer = solana_sdk::signer::keypair::read_keypair_file(cli_config.keypair_path).unwrap();

    Config {
        rpc_client: Arc::new(Box::new(RpcClient::new_with_commitment(
            json_rpc_url,
            CommitmentConfig::confirmed(),
        ))),
        signer: Box::new(signer),
    }
}

#[derive(Debug, Clone, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct OracleData {
    pub symbol: String,
    pub reserve_address: Pubkey,
    pub mint_address: Pubkey,
    pub decimals: i64,
    pub price: u64, // pub price: pyth_sdk_solana::state::PriceFeed,
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
    ) -> HashMap<Pubkey, OracleData> {
        let rpc = &self.config.rpc_client;

        let result: Arc<Mutex<HashMap<Pubkey, OracleData>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut handles = vec![];

        for market_reserve in market_reserves {
            let c_rpc = Arc::clone(&rpc);
            let market_reserve = market_reserve.clone();

            let c_result = Arc::clone(&result);

            let h = tokio::spawn(async move {
                if let Some(oracle_data) = Self::get_oracle_data(&c_rpc, &market_reserve).await {
                    let mut w = c_result.lock();
                    w.insert(oracle_data.reserve_address, oracle_data);
                }
            });

            handles.push(h);
        }

        for h in handles {
            h.await.unwrap();
        }

        Arc::try_unwrap(result).unwrap().into_inner()
    }

    async fn get_oracle_data(
        rpc: &Arc<Box<RpcClient>>,
        reserve: &model::Resef,
    ) -> Option<OracleData> {
        let oracle = FormedOracle {
            price_address: Pubkey::from_str(reserve.pyth_oracle.as_str()).unwrap(),
            switchboard_feed_address: Pubkey::from_str(reserve.switchboard_oracle.as_str())
                .unwrap(),
        };

        let decimals = 10i64.pow(reserve.liquidity_token.decimals as u32);
        let precision = 1e9 as f64;

        let price = if oracle.price_address != *NULL_ORACLE {
            let price_public_key = &oracle.price_address;
            let mut result = rpc.get_account(&price_public_key).await.unwrap();

            let result =
                pyth_sdk_solana::load_price_feed_from_account(&price_public_key, &mut result)
                    .unwrap();

            // println!(
            //     "🎱 oracle: 1st case: {:?}",
            //     result.get_current_price_unchecked().price
            // );

            Some(U256::from(result.get_current_price_unchecked().price))
        } else {
            let price_public_key = Pubkey::from_str(reserve.switchboard_oracle.as_str()).unwrap();
            let info = rpc.get_account(&price_public_key).await.unwrap();
            let owner = info.owner.clone();

            if owner == *SWITCHBOARD_V1_ADDRESS {
                let result = AggregatorState::try_from_slice(&info.data);

                if result.is_err() {
                    return None;
                }

                let result = result.unwrap();
                let result = *result
                    .last_round_result
                    .as_ref()
                    .unwrap()
                    .result
                    .as_ref()
                    .unwrap();
                let result = result * precision;
                let result = U256::from(result as u64)
                    .mul(decimals)
                    .div(precision as u64);

                // println!("🎱 oracle: 2nd case (sb_v1): {:?}", result);

                Some(result)
            } else if owner == *SWITCHBOARD_V2_ADDRESS {
                let mut info = info.clone();
                let inner_accs = vec![(&price_public_key, false, &mut info)];
                let inner_accs = Box::leak(Box::new(inner_accs));

                let info = create_is_signer_account_infos(&mut (*inner_accs));
                let result = switchboard_v2::AggregatorAccountData::new(&info[0]).unwrap();
                let retrieved = result.get_result().unwrap();

                let retrieved: f64 = retrieved.try_into().unwrap();
                let result = retrieved * precision;
                let result = U256::from(result as u64)
                    .mul(decimals)
                    .div(precision as u64);

                // println!("🎱 oracle: 3rd case (sb_v2): {:?}", result);

                Some(result)
            } else {
                None
            }
        };

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
                    decimals,
                    price: price.as_u64(),
                })
            }
            None => None,
        }
    }

    pub async fn get_obligation(
        &self,
        data_account: Either<&str, Pubkey>,
    ) -> Option<Enhanced<Obligation>> {
        let data_pubkey = match data_account {
            Either::Left(v) => Pubkey::from_str(v).unwrap(),
            Either::Right(v) => v,
        };

        match self.config.rpc_client.get_account_data(&data_pubkey).await {
            Ok(data_account) => {
                let obligation = Obligation::unpack(&data_account).unwrap();
                Some(Enhanced {
                    inner: obligation,
                    pubkey: data_pubkey,
                })
            }
            _ => None,
        }
    }

    pub async fn update_obligations(
        &'static self,
        obligations: Vec<Pubkey>,
    ) -> HashMap<Pubkey, Arc<RwLock<Enhanced<Obligation>>>> {
        let result: Arc<Mutex<HashMap<Pubkey, Arc<RwLock<Enhanced<Obligation>>>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let mut handles = vec![];

        for obligation_pubkey in obligations {
            let c_rpc = Arc::clone(&self.config.rpc_client);
            let obligation_pubkey = obligation_pubkey.clone();
            let result = Arc::clone(&result);

            let h = tokio::spawn(async move {
                let obl_account = c_rpc.get_account(&obligation_pubkey).await.unwrap();
                let obligation = Obligation::unpack(&obl_account.data).unwrap();

                let obl = Arc::new(RwLock::new(Enhanced {
                    inner: obligation,
                    pubkey: obligation_pubkey.clone(),
                }));

                let mut w = result.lock();
                w.insert(obligation_pubkey.clone(), obl);
            });

            handles.push(h)
        }

        for h in handles {
            h.await.unwrap();
        }

        Arc::try_unwrap(result).unwrap().into_inner()
    }

    pub async fn get_obligations(
        &self,
        market_address: &str,
    ) -> HashMap<Pubkey, Arc<RwLock<Enhanced<Obligation>>>> {
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

        let mut obligations = HashMap::new();

        if obligations_encoded.is_err() {
            return obligations;
        }

        let obligations_encoded = obligations_encoded.unwrap();
        // let mut obligations_list = vec![];

        for obligation_encoded in &obligations_encoded {
            let &(obl_pubkey, ref obl_account) = obligation_encoded;
            let obligation = Obligation::unpack(&obl_account.data).unwrap();

            // obligations_list.push(Enhanced {
            //     inner: obligation,
            //     pubkey: obl_pubkey,
            // });

            let obl = Arc::new(RwLock::new(Enhanced {
                inner: obligation,
                pubkey: obl_pubkey,
            }));
            obligations.insert(obl_pubkey.clone(), obl);
        }

        obligations
    }

    pub async fn get_reserves(&self, market_address: &str) -> HashMap<Pubkey, Enhanced<Reserve>> {
        let rpc: &RpcClient = &self.config.rpc_client;

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
            return HashMap::new();
        }

        let reserves_encoded = reserves_encoded.unwrap();
        let result: Arc<Mutex<HashMap<Pubkey, Enhanced<Reserve>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let mut handles = vec![];
        for reserve_item in &reserves_encoded {
            let reserve_item = reserve_item.clone();
            let c_result = Arc::clone(&result);

            let h = tokio::spawn(async move {
                let mut w = c_result.lock();

                let (reserve_pubkey, reserve_account) = reserve_item;
                let reserve_unpacked = Reserve::unpack(&reserve_account.data).unwrap();

                w.insert(
                    reserve_pubkey,
                    Enhanced {
                        inner: reserve_unpacked,
                        pubkey: reserve_pubkey,
                    },
                );
            });

            handles.push(h);
        }

        for h in handles {
            h.await.unwrap();
        }

        Arc::try_unwrap(result).unwrap().into_inner()
    }

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
                // println!("account {:?} is empty. creating...", associated_token_address.to_string());

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
                    .send_transaction_with_config(
                        transaction,
                        RpcSendTransactionConfig {
                            skip_preflight: true,
                            preflight_commitment: Some(CommitmentLevel::Confirmed),
                            ..RpcSendTransactionConfig::default()
                        },
                    )
                    .await
                    .unwrap();

                // println!("created account {:}! tx: {:?}", associated_token_address.to_string(), r);

                associated_token_address
            }
            _ => {
                // println!("account {:?} is not empty", associated_token_address.to_string());

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
            }
            _ => (associated_token_address, None),
        }
    }

    pub async fn liquidate_and_redeem(
        &self,
        liqudity_amount: u64,
        selected_borrow: &Borrow,
        selected_deposit: &Deposit,
        lending_market: &Market,
        obligation: &Enhanced<Obligation>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

        let total_reserves = vec![deposit_reserves, borrow_reserves].concat();

        for reserve in &total_reserves {
            uniq_reserve_addresses.insert(reserve.clone());
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
                *reserve,
                Pubkey::from_str(&reserve_info.pyth_oracle.as_str()).unwrap(),
                Pubkey::from_str(reserve_info.switchboard_oracle.as_str()).unwrap(),
            );
            ixs.push(refresh_reserve_ix);
        }

        let refresh_obligation_ix = solend_program::instruction::refresh_obligation(
            // program_id,
            // obligation_pubkey,
            // reserve_pubkeys
            *SOLEND_PROGRAM_ID,
            obligation.pubkey,
            total_reserves,
        );
        ixs.push(refresh_obligation_ix);

        let repay_token_info = Self::find_where(&lending_market.reserves, |x| {
            x.liquidity_token.symbol == *repay_token_symbol
        });

        let repay_account = spl_associated_token_account::get_associated_token_address(
            &payer_pubkey,
            &Pubkey::from_str(repay_token_info.liquidity_token.mint.as_str()).unwrap(),
        );

        let mut reserve_symbol_to_reserve_map: HashMap<String, model::Resef> = HashMap::new();
        for r in &lending_market.reserves {
            reserve_symbol_to_reserve_map.insert(r.liquidity_token.symbol.clone(), r.clone());
        }

        let repay_reserve = reserve_symbol_to_reserve_map.get(repay_token_symbol);
        let withdraw_reserve = reserve_symbol_to_reserve_map.get(withdraw_token_symbol);

        let withdraw_token_info = Self::find_where(&lending_market.reserves, |x| {
            x.liquidity_token.symbol == *withdraw_token_symbol
        });

        if withdraw_reserve.is_none() || repay_reserve.is_none() {
            return Err(Box::new(
                LiquidationAndRedeemError::ReservesAreNotIdentified,
            ));
        }

        let withdraw_reserve = withdraw_reserve.unwrap();
        let repay_reserve = repay_reserve.unwrap();

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
                liqudity_amount,
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
                self.config.rpc_client.send_transaction_with_config(
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
                println!(" 🚀 ✅ 🚀  broadcast: signature: {:?}", r);
            }
            Err(e) => {
                println!(" 🚀 ❌ 🚀  broadcast: err: {:?}", e);
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
    pub borrow_amount_wads: Decimal,
    pub market_value: Decimal,
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
    // pub deposit_amount: Decimal,
    pub deposit_amount: u64,
    pub market_value: Decimal,
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

pub mod fixtures {
    use std::path::Path;

    use super::*;

    #[derive(
        Default, PartialEq, Clone, Debug, serde_derive::Serialize, serde_derive::Deserialize,
    )]
    pub struct CalculateRefreshedObligationFixture {
        pub packed_obligation: Vec<u8>,
        pub packed_all_reserves: Vec<(Pubkey, Vec<u8>)>,
        pub packed_oracles: Vec<Vec<u8>>,
        // pub obligation: Obligation,
        // pub all_reserves: Vec<Enhanced<Reserve>>,
        // pub tokens_oracle: Vec<OracleData>,
    }

    impl CalculateRefreshedObligationFixture {
        pub async fn persist(&self, path: String) {
            let content = serde_json::to_string_pretty(&self).unwrap();
            tokio::fs::write(Path::new(&path), content).await.unwrap();
        }

        pub fn encode(
            obligation: Obligation,
            all_reserves: &Vec<Enhanced<Reserve>>,
            oracles: &Vec<OracleData>,
        ) -> Self {
            let mut packed_obligation = [0u8; 1300];
            Obligation::pack(obligation.clone(), &mut packed_obligation).unwrap();

            let mut packed_all_reserves = vec![];
            for reserve in all_reserves {
                let mut packed_reserve = [0u8; 619];

                Reserve::pack(reserve.inner.clone(), &mut packed_reserve).unwrap();

                // let concated = vec![reserve_pubkey, packed_reserve.to_vec()].concat();
                packed_all_reserves.push((reserve.pubkey, packed_reserve.to_vec()));
            }

            let mut packed_oracles = vec![];
            for oracle in oracles {
                let packed_oracle = serde_json::to_vec(oracle).unwrap();
                packed_oracles.push(packed_oracle)
            }

            Self {
                packed_obligation: packed_obligation.to_vec(),
                packed_all_reserves,
                packed_oracles,
                ..CalculateRefreshedObligationFixture::default()
            }
        }

        pub fn decode(&self) -> (Obligation, Vec<Enhanced<Reserve>>, Vec<OracleData>) {
            let obligation = Obligation::unpack(&self.packed_obligation).unwrap();

            let mut all_reserves = vec![];
            for reserve in self.packed_all_reserves.clone() {
                let inner = Reserve::unpack(&reserve.1).unwrap();
                all_reserves.push(Enhanced {
                    inner,
                    pubkey: reserve.0,
                });
            }

            let mut oracles = vec![];
            for oracle in self.packed_oracles.clone() {
                let value: OracleData = serde_json::from_slice(oracle.as_slice()).unwrap();
                oracles.push(value);
            }

            (obligation, all_reserves, oracles)
        }
    }
}

pub mod binding {
    use super::*;

    use std::{
        cmp::{max, min},
        time::SystemTime,
    };

    use solana_sdk::clock::Clock;
    use solend_program::{
        error::LendingError,
        math::{TryAdd, TryDiv, TryMul, TrySub},
    };

    // process_refresh_obligation

    pub fn refresh_obligation(
        // program_id: &Pubkey,
        enhanced_obligation: &Enhanced<Obligation>,
        all_reserves: &HashMap<Pubkey, Enhanced<Reserve>>,
        tokens_oracle: &HashMap<Pubkey, OracleData>,
        clock: &Clock,
    ) -> Result<(Obligation, Vec<Deposit>, Vec<Borrow>), Box<dyn std::error::Error + Send + Sync>>
    {
        let mut obligation = enhanced_obligation.inner.clone();

        let mut deposited_value = Decimal::zero();
        let mut borrowed_value = Decimal::zero();
        let mut allowed_borrow_value = Decimal::zero();
        let mut unhealthy_borrow_value = Decimal::zero();

        let mut deposits = vec![];
        let mut borrows = vec![];

        for (_index, collateral) in obligation.deposits.iter_mut().enumerate() {
            let deposit_reserve = all_reserves.get(&collateral.deposit_reserve);

            if deposit_reserve.is_none() {
                // println!("DEPOSIT: oracle price not discovered for: {:?}", collateral);
                return Err(LendingError::InvalidAccountInput.into());
            }

            let deposit_reserve = deposit_reserve.unwrap();
            // let deposit_reserve = &all_reserves[deposit_reserve];

            // let token_oracle_idx = tokens_oracle
            //     .iter()
            //     .position(|x| x.reserve_address == collateral.deposit_reserve);
            let token_oracle = tokens_oracle.get(&collateral.deposit_reserve);

            if token_oracle.is_none() {
                // println!("collateral.deposit_reserve: {:?}", collateral.deposit_reserve);
                // println!("oracles.reserves: {:?}", tokens_oracle.iter().map(|x| x.reserve_address).collect::<Vec<Pubkey>>());
                return Err(LendingError::InvalidAccountInput.into());
            }
            // let token_oracle_idx = token_oracle_idx.unwrap();
            // let token_oracle = &tokens_oracle[token_oracle_idx];
            let token_oracle = token_oracle.unwrap();

            // @TODO: add lookup table https://git.io/JOCYq
            let decimals = 10u64
                .checked_pow(deposit_reserve.inner.liquidity.mint_decimals as u32)
                .ok_or(LendingError::MathOverflow)?;

            let market_value = deposit_reserve
                .inner
                .collateral_exchange_rate()?
                .decimal_collateral_to_liquidity(collateral.deposited_amount.into())?
                .try_mul(deposit_reserve.inner.liquidity.market_price)?
                .try_div(decimals)?;
            collateral.market_value = market_value;

            let loan_to_value_rate =
                Rate::from_percent(deposit_reserve.inner.config.loan_to_value_ratio);
            let liquidation_threshold_rate =
                Rate::from_percent(deposit_reserve.inner.config.liquidation_threshold);

            deposited_value = deposited_value.try_add(market_value)?;
            allowed_borrow_value =
                allowed_borrow_value.try_add(market_value.try_mul(loan_to_value_rate)?)?;
            unhealthy_borrow_value = unhealthy_borrow_value
                .try_add(market_value.try_mul(liquidation_threshold_rate)?)?;

            let casted_depo = Deposit {
                deposit_reserve: collateral.deposit_reserve,
                deposit_amount: collateral.deposited_amount,
                market_value,
                symbol: token_oracle.symbol.clone(),
            };
            // println!(" 📥  casted_depo: {:?}", casted_depo);
            deposits.push(casted_depo);
        }

        for (_index, liquidity) in obligation.borrows.iter_mut().enumerate() {
            let reserve = all_reserves.get(&liquidity.borrow_reserve);

            // let token_oracle = tokens_oracle
            //     .iter()
            //     .position(|x| x.reserve_address == liquidity.borrow_reserve);
            let token_oracle = tokens_oracle.get(&liquidity.borrow_reserve);

            if token_oracle.is_none() {
                // println!("BORROW: oracle price not discovered for: {:?}", liquidity);
                return Err(LendingError::InvalidAccountInput.into());
            }

            let token_oracle = token_oracle.unwrap();
            // let token_oracle = &tokens_oracle[token_oracle];

            if reserve.is_none() {
                return Err(LendingError::InvalidAccountInput.into());
            }
            let reserve = reserve.unwrap();
            // let reserve = all_reserves
            //     .iter()
            //     .position(|r| r.pubkey == liquidity.borrow_reserve)
            //     .unwrap();
            // let reserve = &all_reserves[reserve];
            let borrow_reserve = &reserve.inner;

            liquidity.accrue_interest(borrow_reserve.liquidity.cumulative_borrow_rate_wads)?;

            // @TODO: add lookup table https://git.io/JOCYq
            let decimals = 10u64
                .checked_pow(borrow_reserve.liquidity.mint_decimals as u32)
                .ok_or(LendingError::MathOverflow)?;

            let market_value = liquidity
                .borrowed_amount_wads
                .try_mul(borrow_reserve.liquidity.market_price)?
                .try_div(decimals)?;
            liquidity.market_value = market_value;

            borrowed_value = borrowed_value.try_add(market_value)?;

            let casted_borrow = Borrow {
                borrow_reserve: liquidity.borrow_reserve,
                borrow_amount_wads: liquidity.borrowed_amount_wads,
                mint_address: token_oracle.mint_address,
                market_value,
                symbol: token_oracle.symbol.clone(),
            };

            borrows.push(casted_borrow);
        }

        obligation.deposited_value = deposited_value;
        obligation.borrowed_value = borrowed_value;

        // Wednesday, June 22, 2022 12:00:00 PM GMT
        let start_timestamp = 1655899200u64;
        // Wednesday, June 28, 2022 8:00:00 AM GMT
        let end_timestamp = 1656403200u64;
        let current_timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // let current_timestamp = clock.unix_timestamp as u64;
        let current_timestamp_in_range =
            min(max(start_timestamp, current_timestamp), end_timestamp);
        let numerator = end_timestamp
            .checked_sub(current_timestamp_in_range)
            .ok_or(LendingError::MathOverflow)?;
        let denominator = end_timestamp
            .checked_sub(start_timestamp)
            .ok_or(LendingError::MathOverflow)?;

        let start_global_unhealthy_borrow_value = Decimal::from(120000000u64);
        let end_global_unhealthy_borrow_value = Decimal::from(50000000u64);

        let global_unhealthy_borrow_value = end_global_unhealthy_borrow_value.try_add(
            start_global_unhealthy_borrow_value
                .try_sub(end_global_unhealthy_borrow_value)?
                .try_mul(numerator)?
                .try_div(denominator)?,
        )?;
        let global_allowed_borrow_value =
            global_unhealthy_borrow_value.try_sub(Decimal::from(5000000u64))?;

        obligation.allowed_borrow_value = min(allowed_borrow_value, global_allowed_borrow_value);
        obligation.unhealthy_borrow_value =
            min(unhealthy_borrow_value, global_unhealthy_borrow_value);

        obligation.last_update.update_slot(clock.slot);

        Ok((obligation, deposits, borrows))
    }
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
    client: &Client,
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
        Err(e) => {
            println!("err: {:?}", e);
            None
        }
    }
}

pub async fn empty_future() -> usize {
    0usize
}

#[async_trait]
trait UpdateableMarket {
    async fn update_market_data(&mut self, client: &'static Client);
    async fn update_distinct_obligation(&mut self, client: &'static Client, obligation: Pubkey);
}

#[derive(Default, Clone, Debug)]
struct MarketReservesRetriever {
    pub oracle_data: Arc<RwLock<HashMap<Pubkey, OracleData>>>,
    pub obligations: HashMap<Pubkey, Arc<RwLock<Enhanced<Obligation>>>>,
    pub reserves: Arc<RwLock<HashMap<Pubkey, Enhanced<Reserve>>>>,
    lending_market: String,
    input_reserves: Option<&'static Vec<model::Resef>>,
    initialized: bool,
}

#[async_trait]
impl UpdateableMarket for MarketReservesRetriever {
    async fn update_market_data(&mut self, client: &'static Client) {
        let all_obligations = if !self.initialized {
            client.get_obligations(self.lending_market.as_str()).await
        } else {
            let obligation_pubkeys: Vec<Pubkey> =
                self.obligations.keys().into_iter().map(|x| *x).collect();
            client.update_obligations(obligation_pubkeys).await
        };
        self.initialized = true;

        let reserves = client.get_reserves(self.lending_market.as_str()).await;

        let oracle_data = client
            .get_token_oracle_data(self.input_reserves.unwrap())
            .await;

        self.obligations = all_obligations;
        self.reserves = Arc::new(RwLock::new(reserves));
        self.oracle_data = Arc::new(RwLock::new(oracle_data));
    }

    async fn update_distinct_obligation(&mut self, client: &'static Client, obligation: Pubkey) {
        let updated_obligation = client
            .get_obligation(Either::Right(obligation.clone()))
            .await
            .unwrap();

        self.obligations
            .insert(obligation, Arc::new(RwLock::new(updated_obligation)));
    }
}

impl MarketReservesRetriever {
    pub fn new(lending_market: String, input_reserves: &'static Vec<model::Resef>) -> Self {
        Self {
            lending_market,
            input_reserves: Some(input_reserves),
            ..Self::default()
        }
    }
}

#[derive(Default, Clone)]
struct MarketsCapsule {
    // pub markets: HashMap<String, Box<dyn UpdateableMarket + Send + Sync>>
    markets: HashMap<String, Box<MarketReservesRetriever>>,
    client: Option<&'static Client>,
}

impl MarketsCapsule {
    pub fn new(client: &'static Client) -> Self {
        Self {
            client: Some(client),
            ..Default::default()
        }
    }

    pub async fn update_distinct_obligation(&mut self, market: String, obligation: Pubkey) {
        match self.markets.get_mut(&market) {
            Some(target) => {
                target
                    .update_distinct_obligation(self.client.unwrap(), obligation)
                    .await;
            }
            _ => {}
        }
    }

    pub async fn update_initially(
        &mut self,
        market: String,
        input_reserves: &'static Vec<model::Resef>,
    ) {
        self.markets.insert(
            market.clone(),
            Box::new(MarketReservesRetriever::new(market, input_reserves)),
        );
    }

    pub async fn update_distinct_market(&mut self, market: String) {
        match self.markets.get_mut(&market) {
            Some(target) => {
                target.update_market_data(self.client.unwrap()).await;
            }
            _ => {}
        }
    }

    pub fn read_for(self, market: String) -> Box<MarketReservesRetriever> {
        self.markets.get(&market).unwrap().clone()
    }
}

async fn process_markets(client: &'static Client, markets_capsule: Arc<RwLock<MarketsCapsule>>) {
    let markets_list = client.solend_cfg.unwrap();
    let mut handles = vec![];

    let mut meter = PerformanceMeter::new();
    meter.add_point("before capsule initialization");

    for current_market in markets_list {
        markets_capsule
            .write()
            .update_initially(
                current_market.address.clone(),
                Box::leak(Box::new(current_market.reserves.clone())),
            )
            .await;
    }

    meter.add_point("after capsule initialization / before markets update");

    for current_market in markets_list {
        let lending_market = current_market.address.clone();

        // let c_markets_capsule = Arc::clone(&markets_capsule);

        markets_capsule
            .write()
            .update_distinct_market(current_market.address.clone())
            .await;

        let r_markets_capsule = markets_capsule.read().clone();
        let markets = r_markets_capsule.read_for(lending_market.clone());

        let h = tokio::spawn(async move {
            let (oracle_data, all_obligations, reserves) =
                (markets.oracle_data, markets.obligations, markets.reserves);

            let mut inner_handles = vec![];

            for (i, raw_obligation) in all_obligations {
                let obligation = Arc::clone(&raw_obligation);
                let oracle_data = Arc::clone(&oracle_data);
                let reserves = Arc::clone(&reserves);
                let lending_market = lending_market.clone();

                let h = tokio::spawn(async move {

                    let mut meter = PerformanceMeter::new();
                    meter.add_point("before obl read");

                    let r_obligation = obligation.read().clone();

                    meter.add_point("before get slot");
                    let slot = client.config.rpc_client.get_slot().await.unwrap();
                    meter.add_point("before get block time");
                    let block_time = client.config.rpc_client.get_block_time(slot).await.unwrap();
                    meter.add_point("before get epoch info");
                    let epoch_info = client.config.rpc_client.get_epoch_info().await.unwrap();

                    let clock = solana_sdk::clock::Clock {
                        // pub slot: u64,
                        // pub epoch_start_timestamp: i64,
                        // pub epoch: u64,
                        // pub leader_schedule_epoch: u64,
                        // pub unix_timestamp: i64,
                        slot,
                        unix_timestamp: block_time,
                        epoch: epoch_info.epoch,
                        ..solana_sdk::clock::Clock::default()
                    };

                    let r_reserves = reserves.read();
                    let r_oracle_data = oracle_data.read();

                    meter.add_point("before refresh obligation");

                    let r = binding::refresh_obligation(
                        &r_obligation,
                        &*r_reserves,
                        &*r_oracle_data,
                        &clock,
                    );

                    if r.is_err() {
                        return None;
                    }

                    let (refreshed_obligation, deposits, borrows) = r.unwrap();

                    let (borrowed_value, unhealthy_borrow_value) = (
                        refreshed_obligation.borrowed_value.clone(),
                        refreshed_obligation.unhealthy_borrow_value.clone(),
                    );

                    if borrowed_value == Decimal::from(0u64)
                        || unhealthy_borrow_value == Decimal::from(0u64)
                        || borrowed_value <= unhealthy_borrow_value
                    {
                        // println!("do nothing if obligation is healthy");
                        return None;
                    }

                    println!(
                        "
                        obligation: {:?}
                        refreshed_obligation_pubkey: {:?}
                        refreshed_obligation: {:?}
                        unhealthy_borrow_value: {:?}
                        borrowed_value: {:?}
                    ",
                        r_obligation,
                        r_obligation.pubkey.to_string(),
                        refreshed_obligation,
                        unhealthy_borrow_value,
                        borrowed_value,
                    );

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
                        // println!("skip toxic obligations caused by toxic oracle data");
                        return None;
                    }

                    let selected_deposit = selected_deposit.unwrap();
                    let selected_borrow = selected_borrow.unwrap();

                    println!(
                        "obligation: {:} is underwater",
                        r_obligation.pubkey.to_string()
                    );
                    println!("obligation: {:?}", r_obligation.inner);
                    meter.measure();

                    Some((
                        selected_deposit.clone(),
                        selected_borrow.clone(),
                        r_obligation,
                        i,
                        lending_market,
                    ))
                });

                inner_handles.push(h);
            }

            let mut update_required_obligations = vec![];
            for inner_h in inner_handles {
                let r = inner_h.await.unwrap();
                if r.is_none() {
                    continue;
                }

                let values = r.unwrap();
                let (selected_deposit, selected_borrow, r_obligation, i, lending_market) = values;

                Client::get_or_create_account_data(
                    &client.config.signer.pubkey(),
                    &client.config.signer.pubkey(),
                    &selected_borrow.mint_address,
                    &client.config.rpc_client,
                    &client.config.signer,
                )
                .await;

                let retrieved_wallet_data = get_wallet_token_data(
                    &client,
                    client.config.signer.pubkey(),
                    selected_borrow.mint_address,
                    selected_borrow.symbol.clone(),
                )
                .await
                .unwrap();

                let u_zero = U256::from(0);
                if retrieved_wallet_data.balance == u_zero {
                    continue;
                } else if retrieved_wallet_data.balance < u_zero {
                    continue;
                }

                let liquidation_result = client
                    .liquidate_and_redeem(
                        retrieved_wallet_data.balance.as_u64(),
                        &selected_borrow,
                        &selected_deposit,
                        current_market,
                        &r_obligation,
                    )
                    .await;

                if liquidation_result.is_ok() {
                    update_required_obligations.push((lending_market, r_obligation.pubkey));
                }
            }

            return update_required_obligations;
        });

        handles.push(h);
    }

    meter.add_point("before main calc");

    for h in handles {
        let updated_to_obligations = h.await.unwrap();

        for (lending_market, obligation_pubkey) in updated_to_obligations {
            let c_markets_capsule = Arc::clone(&markets_capsule);
            c_markets_capsule
                .write()
                .update_distinct_obligation(lending_market, obligation_pubkey)
                .await;
        }
    }

    meter.measure();
}

pub async fn run_eternal_liquidator(keypair_path: String) {
    let mut solend_client = Client::new(keypair_path);

    let solend_cfg = solend_client.get_solend_config().await;
    let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

    solend_client.solend_cfg = Some(solend_cfg);

    let solend_client: &'static _ = Box::leak(Box::new(solend_client));
    let markets_capsule = Arc::new(RwLock::new(MarketsCapsule::new(solend_client)));

    loop {
        let markets_capsule = Arc::clone(&markets_capsule);
        process_markets(solend_client, markets_capsule).await;
    }
}

pub async fn run_liquidator_iter(keypair_path: String) {
    let mut solend_client = Client::new(keypair_path);

    let solend_cfg = solend_client.get_solend_config().await;
    let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

    solend_client.solend_cfg = Some(solend_cfg);

    let solend_client: &'static _ = Box::leak(Box::new(solend_client));
    let markets_capsule = Arc::new(RwLock::new(MarketsCapsule::new(solend_client)));

    process_markets(solend_client, markets_capsule).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_process_market_iter() {
        run_liquidator_iter(String::from("./private/liquidator_main.json")).await;
    }
}
