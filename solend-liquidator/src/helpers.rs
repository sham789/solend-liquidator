use std::sync::Arc;

use either::Either;
use solana_sdk::signature::Signer;

use parking_lot::{FairMutex, RwLock};

use solend_program::math::Decimal;
// use rayon::prelude::*;

use crate::performance::{PerformanceMeter, Setting};

use crate::binding::*;
use crate::client::{self, Client, MarketsCapsule};
use crate::client_model::*;
use rayon::prelude::*;

pub async fn process_markets(
    client: &'static Client,
    markets_capsule: Arc<RwLock<MarketsCapsule>>,
) {
    let markets_list = client.solend_config().unwrap();

    let mut meter = PerformanceMeter::new(Setting::Ms);

    meter.add_point("after capsule initialization / before markets update");
    println!("markets len: {:?}", markets_list.len());

    // fetch market data for update
    let mut handles = vec![];
    for current_market in markets_list {
        let markets_capsule = Arc::clone(&markets_capsule);
        let market = current_market.address.clone();

        let h = tokio::spawn(async move {
            let data = markets_capsule.read().clone();

            (data.fetch_market_data_for(market.clone()).await, market)
        });
        handles.push(h);
    }

    // persist market data
    let mut w_handles = vec![];
    for h in handles {
        let (market_data, market) = h.await.unwrap();

        let markets_capsule = Arc::clone(&markets_capsule);

        let w_h = tokio::spawn(async move {
            let mut w = markets_capsule.write();
            w.update_market_data_for(market, market_data);
        });
        w_handles.push(w_h);
    }

    for w_h in w_handles {
        w_h.await.unwrap();
    }

    let mut handles = vec![];
    for current_market in markets_list {
        let lending_market = current_market.address.clone();
        let r_markets_capsule = markets_capsule.read().clone();
        let markets = r_markets_capsule.read_for(lending_market.clone());
        let markets = markets.inner;

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
                    let r_obligation = obligation.read().clone();
                    let r_reserves = reserves.read().clone();
                    let r_oracle_data = oracle_data.read().clone();

                    let r =
                        refresh_obligation(&r_obligation, &r_reserves, &r_oracle_data);
                    if r.is_err() {
                        tokio::task::yield_now().await;
                        return None;
                    }

                    let (refreshed_obligation, mut deposits, mut borrows) = r.unwrap();

                    let (borrowed_value, unhealthy_borrow_value) = (
                        refreshed_obligation.borrowed_value.clone(),
                        refreshed_obligation.unhealthy_borrow_value.clone(),
                    );

                    if borrowed_value == Decimal::from(0u64)
                        || unhealthy_borrow_value == Decimal::from(0u64)
                        || borrowed_value <= unhealthy_borrow_value
                    {
                        tokio::task::yield_now().await;
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

                    deposits
                        .sort_by(|a, b| a.market_value.partial_cmp(&b.market_value).unwrap());
                    borrows
                        .sort_by(|a, b| a.market_value.partial_cmp(&b.market_value).unwrap());

                    let selected_deposit = deposits.last();
                    let selected_borrow = borrows.last();

                    if selected_deposit.is_none() || selected_borrow.is_none() {
                        tokio::task::yield_now().await;
                        return None;
                    }

                    let selected_deposit = selected_deposit.unwrap();
                    let selected_borrow = selected_borrow.unwrap();

                    println!(
                        "obligation: {:} is underwater",
                        r_obligation.pubkey.to_string()
                    );
                    println!("obligation: {:?}", r_obligation.inner);

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

            let mut upd_handles = vec![];
            for inner_h in inner_handles {
                let r = inner_h.await.unwrap();

                let h = tokio::spawn(async move {
                    if r.is_none() {
                        tokio::task::yield_now().await;
                        return None;
                    }

                    let values = r.unwrap();
                    let (selected_deposit, selected_borrow, r_obligation, _i, lending_market) = values;

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
                        tokio::task::yield_now().await;
                        return None;
                    } else if retrieved_wallet_data.balance < u_zero {
                        tokio::task::yield_now().await;
                        return None;
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
                        let updated_obligation = client
                            .get_obligation(Either::Right(r_obligation.pubkey))
                            .await
                            .unwrap();

                        Some((lending_market, updated_obligation))
                    } else {
                        tokio::task::yield_now().await;
                        None
                    }
                });

                upd_handles.push(h);
            }

            // return update_required_obligations;
            return upd_handles;
        });

        handles.push(h);
    }

    meter.add_point("before main calc");

    let mut u_handles = vec![];
    for h in handles {
        let update_to_obligations = h.await.unwrap();

        let markets_capsule = Arc::clone(&markets_capsule);

        'i: for h in update_to_obligations {
            let r = h.await.unwrap();
            let markets_capsule = Arc::clone(&markets_capsule);

            let h = tokio::spawn(async move {

                if r.is_none() {
                    tokio::task::yield_now().await;
                    return;
                }

                let (lending_market, updated_obligation) = r.unwrap();

                let mut w = markets_capsule.write();
                w.update_distinct_obligation_for(lending_market, updated_obligation);
            });
            u_handles.push(h);
        }
    }

    for h in u_handles {
        h.await.unwrap();
    }

    meter.measure();
}
