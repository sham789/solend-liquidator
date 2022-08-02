use std::collections::{HashMap, HashSet};
use std::ops::{Deref, DerefMut};
use std::ops::{Div, Mul};
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
use futures_retry::FutureRetry;
use parking_lot::{FairMutex, Mutex, RwLock};

use pyth_sdk_solana;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::{RpcProgramAccountsConfig, RpcSendTransactionConfig};
use solana_client::rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};

use solana_program::instruction::Instruction;
use solana_sdk::account::create_is_signer_account_infos;
use solana_sdk::commitment_config::CommitmentLevel;

use solend_program::math::Decimal;
use solend_program::state::{Obligation, Reserve};

use spl_associated_token_account::get_associated_token_address;

use switchboard_program::AggregatorState;

use crate::client_model::*;
use crate::constants::*;
use crate::helpers::*;
use crate::model::{self, Market, SolendConfig};
// use crate::performance::{PerformanceMeter, Setting};
use crate::utils::body_to_string;

pub struct Client {
    client: HyperClient<HttpsConnector<hyper::client::HttpConnector>>,
    config: Config,
    solend_cfg: Option<&'static SolendConfig>,
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

impl Client {
    const CFG_PRESET: &'static str = "production";

    pub fn signer(&self) -> &Box<Keypair> {
        &self.config.signer
    }

    pub fn rpc_client(&self) -> &Arc<Box<RpcClient>> {
        &self.config.rpc_client
    }

    pub fn solend_config(&self) -> Option<&'static SolendConfig> {
        self.solend_cfg
    }

    pub fn new(keypair_path: String) -> Self {
        let client = HyperClient::builder().build::<_, Body>(HttpsConnector::new());
        let config = get_config(keypair_path);

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
                "https://api.solend.fi/v1/markets/configs?scope=all&deployment={:}",
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

                Some(result)
            } else {
                None
            }
        };

        match price {
            Some(price) => Some(OracleData {
                symbol: reserve.liquidity_token.symbol.clone(),
                reserve_address: Pubkey::from_str(reserve.address.as_str()).unwrap(),
                mint_address: Pubkey::from_str(reserve.liquidity_token.mint.as_str()).unwrap(),
                decimals,
                price: price.as_u64(),
            }),
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

        println!("obligations: {:?}", obligations.len());
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
                    filters: Some(vec![memcmp, RpcFilterType::DataSize(1300)]),
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

        for obligation_encoded in &obligations_encoded {
            let &(obl_pubkey, ref obl_account) = obligation_encoded;
            let obligation = Obligation::unpack(&obl_account.data).unwrap();

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
                    filters: Some(vec![memcmp, RpcFilterType::DataSize(619)]),
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

    pub async fn get_or_create_account_data(
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

                let _r = rpc_client
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

pub async fn get_wallet_token_data(
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
    fn update_market_data(&mut self, market_data: Box<MarketData>);
    async fn fetch_market_data(&self, client: &'static Client) -> Box<MarketData>;
}

#[derive(Default, Clone, Debug)]
pub struct MarketData {
    pub oracle_data: Arc<RwLock<HashMap<Pubkey, OracleData>>>,
    pub obligations: HashMap<Pubkey, Arc<RwLock<Enhanced<Obligation>>>>,
    pub reserves: Arc<RwLock<HashMap<Pubkey, Enhanced<Reserve>>>>,
}

#[derive(Default, Clone, Debug)]
pub struct MarketReservesRetriever {
    pub inner: Box<MarketData>,
    lending_market: String,
    input_reserves: Option<&'static Vec<model::Resef>>,
    initialized: bool,
}

#[async_trait]
impl UpdateableMarket for MarketReservesRetriever {
    fn update_market_data(&mut self, market_data: Box<MarketData>) {
        self.inner = market_data;
        self.initialized = true;
    }

    async fn fetch_market_data(&self, client: &'static Client) -> Box<MarketData> {
        let initialized = self.initialized;
        let input_reserves = self.input_reserves.unwrap();
        let lending_market = self.lending_market.as_str();

        let obligation_pubkeys: Vec<Pubkey> = self
            .inner
            .obligations
            .keys()
            .into_iter()
            .map(|x| *x)
            .collect();

        let all_obligations = async move {
            if !initialized {
                client.get_obligations(lending_market).await
            } else {
                client.update_obligations(obligation_pubkeys).await
            }
        };

        let (reserves, oracle_data, all_obligations) = tokio::join!(
            client.get_reserves(lending_market),
            client.get_token_oracle_data(input_reserves),
            all_obligations
        );

        Box::new(MarketData {
            oracle_data: Arc::new(RwLock::new(oracle_data)),
            obligations: all_obligations,
            reserves: Arc::new(RwLock::new(reserves)),
        })
    }
}

impl MarketReservesRetriever {
    fn update_distinct_obligation(&mut self, obligation: Enhanced<Obligation>) {
        self.inner
            .obligations
            .insert(obligation.pubkey.clone(), Arc::new(RwLock::new(obligation)));
    }
}

impl MarketReservesRetriever {
    pub fn new(lending_market: String, input_reserves: &'static Vec<model::Resef>) -> Self {
        Self {
            lending_market,
            initialized: false,
            input_reserves: Some(input_reserves),
            ..Self::default()
        }
    }
}

#[derive(Default, Clone)]
pub struct MarketsCapsule {
    // pub markets: HashMap<String, Box<dyn UpdateableMarket + Send + Sync>>
    markets: HashMap<String, Box<MarketReservesRetriever>>,
    client: Option<&'static Client>,
}

unsafe impl Send for MarketsCapsule {}
unsafe impl Sync for MarketsCapsule {}

impl MarketsCapsule {
    pub fn new(client: &'static Client) -> Self {
        Self {
            client: Some(client),
            ..Default::default()
        }
    }

    pub fn update_market_data_for(&mut self, market: String, market_data: Box<MarketData>) {
        match self.markets.get_mut(&market) {
            Some(target) => {
                target.update_market_data(market_data);
            }
            _ => {}
        }
    }

    pub async fn fetch_market_data_for(self, market: String) -> Box<MarketData> {
        let market = self.markets.get(&market).unwrap();
        market.fetch_market_data(self.client.unwrap()).await
    }

    pub fn update_initially(&mut self, market: String, input_reserves: &'static Vec<model::Resef>) {
        self.markets.insert(
            market.clone(),
            Box::new(MarketReservesRetriever::new(market, input_reserves)),
        );
    }

    pub async fn fetch_distinct_obligation_for(&self, obligation: Pubkey) -> Enhanced<Obligation> {
        let updated_obligation = self
            .client
            .unwrap()
            .get_obligation(Either::Right(obligation))
            .await
            .unwrap();

        updated_obligation
    }

    pub fn update_distinct_obligation_for(
        &mut self,
        market: String,
        obligation: Enhanced<Obligation>,
    ) {
        match self.markets.get_mut(&market) {
            Some(target) => target.update_distinct_obligation(obligation),
            _ => {}
        }
    }

    pub fn read_for(self, market: String) -> Box<MarketReservesRetriever> {
        self.markets.get(&market).unwrap().clone()
    }
}

pub async fn run_eternal_liquidator(keypair_path: String) {
    let mut solend_client = Client::new(keypair_path);

    let solend_cfg = solend_client.get_solend_config().await;
    let solend_cfg: &'static _ = Box::leak(Box::new(solend_cfg));

    solend_client.solend_cfg = Some(solend_cfg);

    let solend_client: &'static _ = Box::leak(Box::new(solend_client));
    let markets_capsule = Arc::new(RwLock::new(MarketsCapsule::new(solend_client)));

    let mut handles = vec![];
    for current_market in solend_client.solend_config().unwrap() {
        let markets_capsule = Arc::clone(&markets_capsule);

        let market = current_market.address.clone();
        let input_reserves: &'static _ = Box::leak(Box::new(current_market.reserves.clone()));

        let h = tokio::spawn(async move {
            let mut w = markets_capsule.write();
            w.update_initially(market, input_reserves);
        });
        handles.push(h);
    }

    for h in handles {
        h.await.unwrap();
    }

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

    let mut handles = vec![];
    for current_market in solend_client.solend_config().unwrap() {
        let markets_capsule = Arc::clone(&markets_capsule);

        let market = current_market.address.clone();
        let input_reserves: &'static _ = Box::leak(Box::new(current_market.reserves.clone()));

        let h = tokio::spawn(async move {
            let mut w = markets_capsule.write();
            w.update_initially(market, input_reserves);
        });
        handles.push(h);
    }

    for h in handles {
        h.await.unwrap();
    }

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
