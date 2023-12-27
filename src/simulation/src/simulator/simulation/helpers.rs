use super::fork_db::fork_db::ForkDB;
use super::fork_db::fork_factory::ForkFactory;
use hex;
use crate::{
    stream::BlockInfo,
    utils::constants,
};
use super::SimulationError;
//use crate::utils::dotenv::{get_sandwich_contract_address, get_searcher_wallet};

use ethers::abi::{self, parse_abi, ParamType};
use ethers::prelude::BaseContract;
use ethers::types::transaction::eip2930::{AccessList, AccessListItem};
use ethers::types::{Address, BigEndianHash, Bytes, H256, U256};
use ethers::utils::parse_ether;
use revm::primitives::{ExecutionResult, Output, TransactTo};
use revm::{
    primitives::{Address as rAddress, Bytecode, U256 as rU256},
    EVM,
};
use std::str::FromStr;


// Setup braindance for current fork factory by injecting braindance
// contract code and setting up balances
//
// Arguments:
// * `&mut fork_factory`: mutable reference to fork db factory
//
// Returns: This function returns nothing
pub fn attach_braindance_module(fork_factory: &mut ForkFactory) {
    inject_braindance_code(fork_factory);

    // Get balance mapping of braindance contract inside of weth contract
    let slot: U256 = ethers::utils::keccak256(abi::encode(&[
        abi::Token::Address(braindance_address().0.into()),
        abi::Token::Uint(U256::from(3)),
    ]))
    .into();

    let value = braindance_starting_balance();

    fork_factory
        .insert_account_storage(
            constants::get_weth_address().0.into(),
            slot.into(),
            value.into(),
        )
        .unwrap();
}
pub fn inject_test_sniper(
    owner: Address,
    contract: Address,
    fork_factory: &mut ForkFactory,
) {
    // give searcher some balance to pay for gas fees
    let account = revm::primitives::AccountInfo::new(
        parse_ether(100).unwrap().into(),
        0,
        Bytecode::default()
    );
    fork_factory.insert_account_info(owner.0.into(), account);
    
    // setup sniper contract
    let account = revm::primitives::AccountInfo::new(
        rU256::from(0),
        0,
        Bytecode::new_raw(constants::get_sniper_code().0),
    );
    fork_factory.insert_account_info(contract.0.into(), account);

    // add starting weth balance to sniper contract
    let slot: U256 = ethers::utils::keccak256(abi::encode(&[
        abi::Token::Address(contract.0.into()),
        abi::Token::Uint(U256::from(3)),
    ]))
    .into();
    
    let value = braindance_starting_balance();

    // update changes
    fork_factory
        .insert_account_storage(
            constants::get_weth_address().0.into(),
            slot.into(),
            value.into(),
        )
        .unwrap();

    inject_test_wallet(fork_factory);
    
}

pub fn inject_test_wallet(
    fork_factory: &mut ForkFactory,
) {
    // give searcher some balance to pay for gas fees
    let account = revm::primitives::AccountInfo::new(
        parse_ether(100).unwrap().into(),
        0,
        Bytecode::default()
    );
    fork_factory.insert_account_info(sniper_wallet_1_address().0.into(), account);
    
    // setup sniper contract
    let account = revm::primitives::AccountInfo::new(
        rU256::from(0),
        0,
        Bytecode::new_raw(constants::get_wallet_code().0),
    );
    fork_factory.insert_account_info(sniper_wallet_1_address().0.into(), account);

    // add starting weth balance to sniper contract
    let slot: U256 = ethers::utils::keccak256(abi::encode(&[
        abi::Token::Address(sniper_wallet_1_address().0.into()),
        abi::Token::Uint(U256::from(3)),
    ]))
    .into();
    
    let value = braindance_starting_balance();

    // update changes
    fork_factory
        .insert_account_storage(
            constants::get_weth_address().0.into(),
            slot.into(),
            value.into(),
        )
        .unwrap();
    
}




// Setup evm blockstate
//
// Arguments:
// * `&mut evm`: mutable refernece to `EVM<ForkDB>` instance which we want to modify
// * `&next_block`: reference to `BlockInfo` of next block to set values against
//
// Returns: This function returns nothing
pub fn setup_block_state(evm: &mut EVM<ForkDB>, next_block: &BlockInfo) {
    evm.env.block.number = rU256::from(next_block.number.as_u64());
    evm.env.block.timestamp = next_block.timestamp.into();
    evm.env.block.basefee = next_block.base_fee.into();
    // use something other than default
    evm.env.block.coinbase =
        rAddress::from_str("0xDecafC0FFEe15BAD000000000000000000000002").unwrap();
}

// Find amount out from an amount in using the k=xy formula
// note: assuming fee is set to 3% for all pools (not case irl)
//
// Arguments:
// * `amount_in`: amount of token in
// * `target_pool`: address of pool
// * `token_in`: address of token in
// * `token_out`: address of token out
// * `evm`: mutable reference to evm used for query
//
// Returns:
// Ok(U256): amount out
// Err(SimulationError): if error during caluclation
pub fn get_amount_out_evm(
    amount_in: U256,
    target_pool: Address,
    token_in: Address,
    token_out: Address,
    evm: &mut EVM<ForkDB>,
) -> Result<U256, SimulationError> {
    // get reserves
    evm.env.tx.transact_to = TransactTo::Call(target_pool.0.into());
    evm.env.tx.caller = constants::get_eth_dev().0.into();
    evm.env.tx.value = rU256::ZERO;
    //evm.env.tx.nonce = Some(1);
    evm.env.tx.gas_priority_fee = None;
    evm.env.tx.nonce = None;
    evm.env.tx.data = Bytes::from_str("0x0902f1ac").unwrap().0; // getReserves()
    let result = match evm.transact_ref() {
        Ok(result) => result.result,
        Err(e) => return Err(SimulationError::EvmError(e)),
    };
    let output: Bytes = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(o) => o.into(),
            Output::Create(o, _) => o.into(),
        },
        ExecutionResult::Revert { output, .. } => return Err(SimulationError::EvmReverted(output)),
        ExecutionResult::Halt { reason, .. } => return Err(SimulationError::EvmHalted(reason)),
    };

    let tokens = abi::decode(
        &vec![
            ParamType::Uint(128),
            ParamType::Uint(128),
            ParamType::Uint(32),
        ],
        &output,
    )
    .unwrap();
    let reserves_0 = tokens[0].clone().into_uint().unwrap();
    let reserves_1 = tokens[1].clone().into_uint().unwrap();

    let (reserve_in, reserve_out) = match token_in < token_out {
        true => (reserves_0, reserves_1),
        false => (reserves_1, reserves_0),
    };

    let a_in_with_fee: U256 = amount_in * 997;
    let numerator: U256 = a_in_with_fee * reserve_out;
    let denominator: U256 = reserve_in * 1000 + a_in_with_fee;
    let amount_out: U256 = numerator.checked_div(denominator).unwrap_or(U256::zero());
    Ok(amount_out)
}

// Get token balance
//
// Arguments:
// * `token`: erc20 token to query
// * `owner`: address to find balance of
// * `next_block`: block to query balance at
// * `evm`: evm instance to run query on
//
// Returns:
// `Ok(balance: U256)` if successful, Err(SimulationError) otherwise
pub fn get_balance_of_evm(
    token: Address,
    owner: Address,
    next_block: &BlockInfo,
    evm: &mut EVM<ForkDB>,
) -> Result<U256, SimulationError> {
    let erc20 = BaseContract::from(
        parse_abi(&["function balanceOf(address) external returns (uint)"]).unwrap(),
    );

    evm.env.tx.transact_to = TransactTo::Call(token.0.into());
    evm.env.tx.data = erc20.encode("balanceOf", owner).unwrap().0;
    evm.env.tx.caller = constants::get_eth_dev().0.into();
    evm.env.tx.gas_price = next_block.base_fee.into();
    evm.env.tx.gas_limit = 700000;
    evm.env.tx.gas_priority_fee = None;
    evm.env.tx.nonce = None;
    evm.env.tx.value = rU256::ZERO;

    let result = match evm.transact_ref() {
        Ok(result) => result.result,
        Err(e) => {
            return Err(SimulationError::EvmError(e));
        }
    };

    let output: Bytes = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(o) => o.into(),
            Output::Create(o, _) => o.into(),
        },
        ExecutionResult::Revert { output, .. } => return Err(SimulationError::EvmReverted(output)),
        ExecutionResult::Halt { reason, .. } => return Err(SimulationError::EvmHalted(reason)),
    };

    match erc20.decode_output("balanceOf", &output) {
        Ok(tokens) => return Ok(tokens),
        //Err(e) => return Err(SimulationError::AbiError(e)),
        Err(_) => return Err(SimulationError::ZeroOptimal()),
    }
}

pub fn get_total_supply_of_evm(
    token: Address,
    next_block: &BlockInfo,
    evm: &mut EVM<ForkDB>,
) -> Result<U256, SimulationError> {
    let erc20 = BaseContract::from(
        parse_abi(&["function totalSupply() external returns (uint)"]).unwrap(),
    );

    evm.env.tx.transact_to = TransactTo::Call(token.0.into());
    evm.env.tx.data = erc20.encode("totalSupply", ()).unwrap().0;
    evm.env.tx.caller = constants::get_eth_dev().0.into();
    evm.env.tx.gas_price = next_block.base_fee.into();
    evm.env.tx.gas_limit = 700000;
    evm.env.tx.gas_priority_fee = None;
    evm.env.tx.nonce = None;
    evm.env.tx.value = rU256::ZERO;

    let result = match evm.transact_ref() {
        Ok(result) => result.result,
        Err(e) => {
            return Err(SimulationError::EvmError(e));
        }
    };

    let output: Bytes = match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(o) => o.into(),
            Output::Create(o, _) => o.into(),
        },
        ExecutionResult::Revert { output, .. } => return Err(SimulationError::EvmReverted(output)),
        ExecutionResult::Halt { reason, .. } => return Err(SimulationError::EvmHalted(reason)),
    };

    match erc20.decode_output("totalSupply", &output) {
        Ok(tokens) => return Ok(tokens),
        //Err(e) => return Err(SimulationError::AbiError(e)),
        Err(_) => return Err(SimulationError::ZeroOptimal()),
    }
}
// Add bytecode to braindance address
//
// Arguments:
// `&mut fork_factory`: mutable reference to `ForkFactory` instance to inject
//
// Returns: This function returns nothing
fn inject_braindance_code(fork_factory: &mut ForkFactory) {
    // setup braindance contract
    let account = revm::primitives::AccountInfo::new(
        rU256::from(0),
        0,
        Bytecode::new_raw(constants::get_braindance_code().0),
    );
    fork_factory.insert_account_info(braindance_address().0.into(), account);

    // setup braindance contract controller
    let account =
        revm::primitives::AccountInfo::new(parse_ether(420).unwrap().into(), 0, Bytecode::default());
    fork_factory.insert_account_info(braindance_controller_address().0.into(), account);
}

// Converts access list from revm to ethers type
//
// Arguments:
// * `access_list`: access list in revm format
//
// Returns:
// `AccessList` in ethers format
pub fn convert_access_list(access_list: Vec<(rAddress, Vec<rU256>)>) -> AccessList {
    let mut converted_access_list = Vec::new();
    for access in access_list {
        let address = access.0;
        let keys = access.1;
        let access_item = AccessListItem {
            address: address.0.into(),
            storage_keys: keys
                .iter()
                .map(|k| {
                    let slot_u256: U256 = k.clone().into();
                    let slot_h256: H256 = H256::from_uint(&slot_u256);
                    slot_h256
                })
                .collect::<Vec<H256>>(),
        };
        converted_access_list.push(access_item);
    }

    AccessList(converted_access_list)
}


pub fn convert_simulation_error(error: SimulationError) -> Option<String> {
    let binding = error.to_string();
    println!("Error message: {:?}", binding);
    let mut hex_str = binding.as_str().clone();
    // TODO: We need to handle differently the decode, if the starting sig is different:
    // https://ethereum.stackexchange.com/questions/128806/is-there-a-way-to-add-all-revert-error-strings-to-an-ethers-js-interface
    hex_str = &hex_str[8..hex_str.len()];
    let decoded = hex::decode(hex_str).ok()?;
    let result = abi::decode(&[abi::ParamType::String], &decoded).ok()?;
    Some(result[0].clone().to_string())
}

// Holds constant value representing braindance contract address
pub fn braindance_address() -> rAddress {
    rAddress::from_str("00000000000000000000000000000000F3370000").unwrap()
}



// Holds constant value representing braindance caller
pub fn braindance_controller_address() -> rAddress {
    rAddress::from_str("000000000000000000000000000000000420BABE").unwrap()
}

pub fn sniper_wallet_1_address() -> rAddress {
    rAddress::from_str("0x000000000000000000000000000000000440BAbe").unwrap()
}

// Holds constant value representing braindance weth starting balance
pub fn braindance_starting_balance() -> U256 {
    parse_ether(420).unwrap()
}