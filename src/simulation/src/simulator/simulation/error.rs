use std::fmt;
use ethers::abi::{self};
use revm::primitives::Bytes;


use super::fork_db::DatabaseError;


fn convert_revert_error(error: &Bytes) -> String {   
    let binding = hex::encode(error);
    let mut hex_str = binding.as_str().clone();
    // TODO: We need to handle differently the decode, if the starting sig is different:
    // https://ethereum.stackexchange.com/questions/128806/is-there-a-way-to-add-all-revert-error-strings-to-an-ethers-js-interface
    if hex_str.len() > 8 {
        hex_str = &hex_str[8..hex_str.len()];
        let decoded = hex::decode(hex_str).ok().unwrap_or_default();
        let result = abi::decode(&[abi::ParamType::String], &decoded).ok().unwrap_or_default();
        if result.len() > 0 {
            result[0].clone().to_string()
        } else {
            binding
        }

    } else {
        binding
    }
    
}


#[derive(Debug, Clone)]
pub enum SimulationError {
    TokenHasNoPool,
    FrontrunEvmError(revm::primitives::EVMError<DatabaseError>),
    FrontrunHalted(revm::primitives::Halt),
    FrontrunReverted(revm::primitives::Bytes),
    BackrunEvmError(revm::primitives::EVMError<DatabaseError>),
    BackrunHalted(revm::primitives::Halt),
    BackrunReverted(revm::primitives::Bytes),
    FailedToDecodeOutput(),
    EvmError(revm::primitives::EVMError<DatabaseError>),
    EvmHalted(revm::primitives::Halt),
    EvmReverted(revm::primitives::Bytes),
    AbiError(),
    ZeroOptimal(),
}

impl fmt::Display for SimulationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SimulationError::TokenHasNoPool => {
                write!(f, "Token has no valid pool")
            }
            SimulationError::FrontrunEvmError(db_err) => {
                write!(f, "Fromrun ran into an EVM error : {:?}", db_err)
            }
            SimulationError::FrontrunHalted(halt_reason) => {
                write!(f, "Frontrun halted due to : {:?}", halt_reason)
            }
            SimulationError::FrontrunReverted(bytes) => {
                
                write!(f, "Frontrun reverted and returned : {}", convert_revert_error(bytes))
            }           
            SimulationError::BackrunEvmError(db_err) => {
                write!(f, "Backrun ran into an EVM error : {:?}", db_err)
            }
            SimulationError::BackrunHalted(halt_reason) => {
                write!(f, "Backrun halted due to : {:?}", halt_reason)
            }
            SimulationError::BackrunReverted(bytes) => {
                write!(f, "Backrun reverted and returned : {}", convert_revert_error(bytes))
            }     
            SimulationError::FailedToDecodeOutput() => {
                write!(f, "Failed to decode output")
            }
            SimulationError::EvmError(db_err) => {
                write!(f, "Ran into an EVM error : {:?}", db_err)
            }
            SimulationError::EvmHalted(halt_reason) => {
                write!(f, "EVM halted due to : {:?}", halt_reason)
            }
            SimulationError::EvmReverted(bytes) => {
                write!(f, "EVM reverted and returned : {}", convert_revert_error(bytes))
            }
            SimulationError::AbiError() => {
                write!(f, "Failed to decode ABI due to")
            }
            SimulationError::ZeroOptimal() => {
                write!(f, "No optimal sandwich found")
            }
        }
    }
}
