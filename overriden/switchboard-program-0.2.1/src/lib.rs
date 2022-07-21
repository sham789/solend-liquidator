use solana_program::pubkey::Pubkey;
use quick_protobuf::deserialize_from_slice;
use solana_program::account_info::AccountInfo;
use solana_program::program_error::ProgramError;

pub use switchboard_utils::{FastRoundResultAccountData, FastRoundResult, fast_parse_switchboard_result};
pub use switchboard_protos::protos::aggregator_state::AggregatorState;
pub use switchboard_protos::protos::aggregator_state::mod_AggregatorState;
pub use switchboard_protos::protos::aggregator_state::RoundResult;
pub use switchboard_protos::protos::switchboard_account_types::SwitchboardAccountType;
use switchboard_protos::protos::vrf::VrfAccountData;
use bytemuck::{bytes_of_mut};

/// Returns whether the current open round is considered valid for usage.
pub fn is_current_round_valid(aggregator: &AggregatorState) -> Result<bool, ProgramError> {
    let maybe_round = aggregator.current_round_result.clone();
    if maybe_round.is_none() {
        return Ok(false);
    }
    let round = maybe_round.unwrap();
    let configs = aggregator.configs.as_ref().ok_or(ProgramError::InvalidAccountData)?;
    if round.num_success < configs.min_confirmations {
        return Ok(false);
    }
    Ok(true)
}

/// Given a Switchboard data feed account, this method will parse the account state.
///
/// Returns a ProgramError if the AccountInfo is unable to be borrowed or the
/// account is not initialized as an aggregator.
pub fn get_aggregator(switchboard_feed: &AccountInfo) -> Result<AggregatorState, ProgramError> {
    let state_buffer = switchboard_feed.try_borrow_data()?;
    if state_buffer.len() == 0 || state_buffer[0] != SwitchboardAccountType::TYPE_AGGREGATOR as u8 {
        return Err(ProgramError::InvalidAccountData);
    }
    let aggregator_state: AggregatorState =
        deserialize_from_slice(&state_buffer[1..]).map_err(|_| ProgramError::InvalidAccountData)?;
    Ok(aggregator_state)
}

/// Returns the most recent resolution round that is considered valid for the aggregator.
pub fn get_aggregator_result(aggregator: &AggregatorState) -> Result<RoundResult, ProgramError> {
    let mut maybe_round = aggregator.current_round_result.clone();
    if !is_current_round_valid(&aggregator)? {
        maybe_round = aggregator.last_round_result.clone();
    }
    maybe_round.ok_or(ProgramError::InvalidAccountData)
}

pub struct BundleAccount<'a> {
    account_info: &'a AccountInfo<'a>
}

impl<'a> BundleAccount<'a> {
    const BUFFER_SIZE: usize = 500;

    pub fn new(account: &'a AccountInfo<'a>) -> Result<Self, ProgramError> {
        let buf = account.try_borrow_data()?;
        if buf.len() == 0 || buf[0] != SwitchboardAccountType::TYPE_BUNDLE as u8 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self{
            account_info: account
        })
    }

    pub fn get_idx(&self, idx: usize) -> Result<FastRoundResultAccountData, ProgramError> {
        let offset: usize = 1 + (idx * Self::BUFFER_SIZE);
        let term = offset + std::mem::size_of::<FastRoundResultAccountData>();
        let buf = self.account_info.try_borrow_data()?;
        if buf.len() < term {
            return Err(ProgramError::InvalidArgument);
        }
        let mut res = FastRoundResultAccountData {
            ..Default::default()
        };
        let recv = bytes_of_mut(&mut res);
        recv.copy_from_slice(&buf[offset..term]);
        if res.result.round_open_slot == 0 {
            return Err(ProgramError::InvalidArgument);
        }
        Ok(res)
    }
}



pub struct VrfAccount<'a> {
    account_info: &'a AccountInfo<'a>
}

impl<'a> VrfAccount<'a> {

    pub fn new(account: &'a AccountInfo<'a>) -> Result<Self, ProgramError> {
        let buf = account.try_borrow_data()?;
        if buf.len() == 0 || buf[0] != SwitchboardAccountType::TYPE_VRF as u8 {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(Self{
            account_info: account
        })
    }

    /// returns the current verified randomness value held in the account.
    /// returns ProgramError if not randomness currently exists or
    /// if the number of proof verificaitons is less than the reuired
    /// minimum numner of verifications.
    pub fn get_verified_randomness(&self) -> Result<Vec<u8>, ProgramError> {
        let vrf_state: VrfAccountData =
            deserialize_from_slice(&self.account_info.try_borrow_data()?[1..])
                .map_err(|_| ProgramError::InvalidAccountData)?;
        let value = vrf_state.value.ok_or(ProgramError::InvalidAccountData)?;
        let min_confirmations = vrf_state.min_proof_confirmations
            .ok_or(ProgramError::InvalidAccountData)?;
        let num_confirmations = vrf_state.num_proof_confirmations
            .ok_or(ProgramError::InvalidAccountData)?;
        if num_confirmations < min_confirmations {
            return Err(ProgramError::InvalidAccountData);
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn new_account_info<'a>(
        owner: &'a Pubkey,
        key: &'a Pubkey,
        lamports: &'a mut u64,
        data: &'a mut [u8],
    ) -> AccountInfo<'a> {
        AccountInfo::new(
            key,      // key: &'a Pubkey,
            false,    // is_signer: bool,
            true,     // is_writable: bool,
            lamports, // lamports: &'a mut u64,
            data,     // data: &'a mut [u8],
            owner,    // owner: &'a Pubkey,
            false,    // executable: bool,
            100,      // rent_epoch: Epoch
        )
    }

    pub fn create_aggregator(current_round: RoundResult, last_round: RoundResult) -> AggregatorState {
        AggregatorState {
            version: Some(1),
            configs: Some(mod_AggregatorState::Configs {
                min_confirmations: Some(10),
                min_update_delay_seconds: Some(10),
                locked: Some(false),
                schedule: None,
            }),
            fulfillment_manager_pubkey: Some(Vec::new()),
            job_definition_pubkeys: Vec::new(),
            agreement: None,
            current_round_result: Some(current_round),
            last_round_result: Some(last_round),
            parse_optimized_result_address: None,
            bundle_auth_addresses: Vec::new(),
        }
    }

    #[test]
    fn test_reject_current_on_sucess_count() {
        let current_round = RoundResult {
            num_success: Some(2),
            num_error: Some(5),
            result: Some(97.0),
            round_open_slot: Some(1),
            round_open_timestamp: Some(1),
            min_response: Some(96.0),
            max_response: Some(100.0),
            medians: Vec::new(),
        };
        let last_round = RoundResult {
            num_success: Some(30),
            num_error: Some(0),
            result: Some(100.0),
            round_open_slot: Some(1),
            round_open_timestamp: Some(1),
            min_response: Some(100.0),
            max_response: Some(100.0),
            medians: Vec::new(),
        };
        let aggregator = create_aggregator(current_round.clone(), last_round.clone());
        assert_eq!(get_aggregator_result(&aggregator).unwrap(), last_round.clone());
    }

    #[test]
    fn test_accept_current_on_sucess_count() {
        let current_round = RoundResult {
            num_success: Some(20),
            num_error: Some(5),
            result: Some(97.0),
            round_open_slot: Some(1),
            round_open_timestamp: Some(1),
            min_response: Some(96.0),
            max_response: Some(100.0),
            medians: Vec::new(),
        };
        let last_round = RoundResult {
            num_success: Some(30),
            num_error: Some(0),
            result: Some(100.0),
            round_open_slot: Some(1),
            round_open_timestamp: Some(1),
            min_response: Some(100.0),
            max_response: Some(100.0),
            medians: Vec::new(),
        };
        let aggregator = create_aggregator(current_round.clone(), last_round.clone());
        assert_eq!(get_aggregator_result(&aggregator).unwrap(), current_round.clone());
    }
}
