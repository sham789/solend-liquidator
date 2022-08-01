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
use crate::binding;

use crate::client_model::*;
use crate::helpers::*;
use crate::constants::*;


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