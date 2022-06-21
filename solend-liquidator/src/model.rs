use serde_derive::Deserialize;
use serde_derive::Serialize;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolendConfig {
    #[serde(rename = "programID")]
    pub program_id: String,
    pub assets: Vec<Asset>,
    pub markets: Vec<Market>,
    pub oracles: Oracles,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub name: String,
    pub symbol: String,
    pub decimals: i64,
    pub mint_address: String,
    pub logo: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    pub name: String,
    pub is_primary: bool,
    pub description: Option<String>,
    pub creator: String,
    pub address: String,
    pub authority_address: String,
    pub reserves: Vec<Resef>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resef {
    pub asset: String,
    pub address: String,
    pub collateral_mint_address: String,
    pub collateral_supply_address: String,
    pub liquidity_address: String,
    pub liquidity_fee_receiver_address: String,
    pub user_supply_cap: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Oracles {
    #[serde(rename = "pythProgramID")]
    pub pyth_program_id: String,
    #[serde(rename = "switchboardProgramID")]
    pub switchboard_program_id: String,
    pub assets: Vec<Asset2>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset2 {
    pub asset: String,
    pub price_address: String,
    pub switchboard_feed_address: String,
}
