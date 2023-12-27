use std::str::FromStr;

use ethers::prelude::*;


// Return the ethdev address (used if we need funds)
pub fn get_eth_dev() -> Address {
    Address::from_str("0x5AbFEc25f74Cd88437631a7731906932776356f9").unwrap()
}

// Return weth address
pub fn get_weth_address() -> Address {
    Address::from_str("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2").unwrap()
}

pub fn get_wallet_code() -> Bytes {
    "0x0".parse().unwrap()
}

pub fn get_sniper_code() -> Bytes {
    "0x0".parse().unwrap()
}

// Returns the bytecode for our custom modded router contract
pub fn get_braindance_code() -> Bytes {
    "0x0".parse().unwrap()
}
