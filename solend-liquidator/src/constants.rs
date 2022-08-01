use solana_program::pubkey::Pubkey;
use std::str::FromStr;

lazy_static::lazy_static! {
    pub static ref SWITCHBOARD_V1_ADDRESS: Pubkey =
        Pubkey::from_str("DtmE9D2CSB4L5D6A15mraeEjrGMm6auWVzgaD8hK2tZM").unwrap();
    pub static ref SWITCHBOARD_V2_ADDRESS: Pubkey =
        Pubkey::from_str("SW1TCH7qEPTdLsDHRgPuMQjbQxKdH2aBStViMFnt64f").unwrap();
    pub static ref NULL_ORACLE: Pubkey =
        Pubkey::from_str("nu11111111111111111111111111111111111111111").unwrap();
    pub static ref SOLEND_PROGRAM_ID: Pubkey =
        Pubkey::from_str("So1endDq2YkqhipRh3WViPa8hdiSpxWy6z3Z6tMCpAo").unwrap();
}
