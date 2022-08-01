use std::collections::HashMap;
use std::{
    cmp::{max, min},
    time::SystemTime,
};

use solana_program::pubkey::Pubkey;

use futures_retry::FutureFactory;

use solend_program::math::{Decimal, Rate};
use solend_program::state::{Obligation, Reserve};

use solana_sdk::clock::Clock;
use solend_program::{
    error::LendingError,
    math::{TryAdd, TryDiv, TryMul, TrySub},
};

use crate::client_model::*;

pub fn refresh_obligation(
    enhanced_obligation: &Enhanced<Obligation>,
    all_reserves: &HashMap<Pubkey, Enhanced<Reserve>>,
    tokens_oracle: &HashMap<Pubkey, OracleData>,
    clock: &Clock,
) -> Result<(Obligation, Vec<Deposit>, Vec<Borrow>), Box<dyn std::error::Error + Send + Sync>> {
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
        unhealthy_borrow_value =
            unhealthy_borrow_value.try_add(market_value.try_mul(liquidation_threshold_rate)?)?;

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
    let current_timestamp_in_range = min(max(start_timestamp, current_timestamp), end_timestamp);
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
    obligation.unhealthy_borrow_value = min(unhealthy_borrow_value, global_unhealthy_borrow_value);

    obligation.last_update.update_slot(clock.slot);

    Ok((obligation, deposits, borrows))
}
