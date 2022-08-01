use futures_retry::RetryPolicy;
use solana_program::pubkey::Pubkey;
use solend_program::math::Decimal;
use uint::construct_uint;

construct_uint! {
    pub struct U256(4);
}

pub fn handle_error<E: std::error::Error>(_e: E) -> RetryPolicy<E> {
    RetryPolicy::WaitRetry(std::time::Duration::from_millis(150))
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
