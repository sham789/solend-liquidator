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

use crate::binding;
use crate::log::Logger;
use crate::model::{self, Market, SolendConfig};
use crate::performance::PerformanceMeter;
use crate::utils::body_to_string;

construct_uint! {
    pub struct U256(4);
}

pub fn handle_error<E: std::error::Error>(e: E) -> RetryPolicy<E> {
    RetryPolicy::WaitRetry(std::time::Duration::from_millis(150))
    // RetryPolicy::ForwardError(e)
}

#[derive(thiserror::Error, Debug)]
pub enum LiquidationAndRedeemError {
    #[error("reserves are not identified")]
    ReservesAreNotIdentified,
}

#[derive(Debug, Clone)]
pub struct Borrow {
    pub borrow_reserve: Pubkey,
    pub borrow_amount_wads: Decimal,
    pub market_value: Decimal,
    pub mint_address: Pubkey,
    pub symbol: String,
}

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

// basically it is 1e18
pub fn wad() -> U256 {
    U256::from(1000000000000000000u128)
}

pub struct FormedOracle {
    pub price_address: Pubkey,
    pub switchboard_feed_address: Pubkey,
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
