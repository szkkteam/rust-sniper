use crate::{
    types::ProfileId,
};
use super::{
    error::PortfolioError
};
use ethers::{prelude::{Address, U256}};
use serde::{Deserialize, Serialize};

pub trait ProfileUpdater
{
    fn update_profile(&mut self, profile: Profile) -> Result<(), PortfolioError>;

    fn get_profile(&mut self, profile_id: &ProfileId) -> Result<Option<Profile>, PortfolioError>;

    fn delete_profile(&mut self, profile_id: &ProfileId) -> Result<(), PortfolioError>;
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BundlePriority {
    FirstOnly,
    Auto,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum TransactionType {
    Auto,
    Bundle {
        priority: BundlePriority
    },
    InuEth,
    Normal,
}


#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum WalletType {
    #[serde(rename_all = "camelCase")]
    BotWallets { 
        //#[serde(with = "string")]
        num_wallets: u8
    },
    UserWallets {
        wallets: Vec<Address>,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderStrict {
    pub out_amount: U256,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderLimit {
    pub out_amount: U256,
    pub max_amount_in: U256,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderExact {
    pub max_amount_in: U256,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum OrderSize {
    Strict(OrderStrict),
    Limit(OrderLimit),
    Exact(OrderExact)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Taxes {
    //#[serde(with = "string")]
    pub buy_fee: f64,
    //#[serde(with = "string")]
    pub sell_fee: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AntiRug {
    pub priority: Priority,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Priority {
    pub max_prio_fee_per_gas: U256,
    //#[serde(with = "string")]
    pub priority: u8,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub order_size: OrderSize,
    pub wallet_type: WalletType,
    pub transaction_type: TransactionType,
    pub taxes: Option<Taxes>,
    pub anti_rug: Option<AntiRug>,    
    pub priority: Priority,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExitStrategy {
    pub take_out_initials_at: f64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub user_id: String,
    pub contract_address: Address,
    pub token: Address,
    pub private_key: String,
    pub secondary_private_key: String,
    pub order: Order,    
    pub strategy: Option<ExitStrategy>,
}
