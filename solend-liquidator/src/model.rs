use serde_derive::Deserialize;
use serde_derive::Serialize;
use serde_json::Value;

// #[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
// #[serde(rename_all = "camelCase")]
// pub struct SolendConfig {
//     #[serde(rename = "programID")]
//     pub program_id: String,
//     pub assets: Vec<Asset>,
//     pub markets: Vec<Market>,
//     pub oracles: Oracles,
// }

pub type SolendConfig = Vec<Market>;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    pub name: String,
    pub is_primary: bool,
    pub description: Option<String>,
    pub creator: String,
    pub address: String,
    pub authority_address: String,
    pub owner: String,
    pub reserves: Vec<Resef>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resef {
    pub liquidity_token: LiquidityToken,
    pub pyth_oracle: String,
    pub switchboard_oracle: String,
    pub address: String,
    pub collateral_mint_address: String,
    pub collateral_supply_address: String,
    pub liquidity_address: String,
    pub liquidity_fee_receiver_address: String,
    pub user_supply_cap: Option<Value>,
    pub user_borrow_cap: Option<Value>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiquidityToken {
    #[serde(rename = "coingeckoID")]
    pub coingecko_id: String,
    pub decimals: i64,
    pub logo: String,
    pub mint: String,
    pub name: String,
    pub symbol: String,
    pub volume24h: String,
}
