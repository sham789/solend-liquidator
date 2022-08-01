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
use crate::model::{Market, SolendConfig};
use crate::performance::PerformanceMeter;
use crate::utils::body_to_string;
use crate::binding::*;
use crate::client_model::*;
use crate::client::{self, Client, MarketsCapsule};

pub async fn process_markets(
    client: &'static Client,
    markets_capsule: Arc<RwLock<MarketsCapsule>>,
) {
    let markets_list = client.solend_config().unwrap();
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
                    let slot = client.rpc_client().get_slot().await.unwrap();
                    meter.add_point("before get block time");
                    let block_time = client.rpc_client().get_block_time(slot).await.unwrap();
                    meter.add_point("before get epoch info");
                    let epoch_info = client.rpc_client().get_epoch_info().await.unwrap();

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

                    let r = refresh_obligation(
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
                    &client.signer().pubkey(),
                    &client.signer().pubkey(),
                    &selected_borrow.mint_address,
                    &client.rpc_client(),
                    &client.signer(),
                )
                .await;

                let retrieved_wallet_data = client::get_wallet_token_data(
                    &client,
                    client.signer().pubkey(),
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
