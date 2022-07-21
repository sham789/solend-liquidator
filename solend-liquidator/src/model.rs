use serde_derive::Deserialize;
use serde_derive::Serialize;
use serde_json::Value;

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
    pub owner: String,
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
    pub user_supply_cap: Option<Value>,
    pub user_borrow_cap: Option<Value>,
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

pub mod full {
    use serde_derive::Deserialize;
    use serde_derive::Serialize;
    use serde_json::Value;

    pub type ResponseList = Vec<Response>;

    #[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Response {
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
        pub user_supply_cap: Value,
        pub user_borrow_cap: Value,
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
}
