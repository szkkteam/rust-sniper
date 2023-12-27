use crate::{
    simulator::{
    event::{
        SimulationEvent,
        SellSimulationEvent
    }
    },
    executor::{
    TransactionEvent
    },
    stream::BlockInfo,
    event::Event,
    types::{
        TraderId,
        OrderId,
        deserialize_order_id,
        serialize_order_id
    },
    utils::sign_eip1559,
};
use serde::{Serialize};
use async_trait::async_trait;
use std::sync::Arc;
use ethers::prelude::{
    Transaction,
    LocalWallet,
    Address,
    Provider,
    Ws,
    ProviderError,
    U256,
    Bytes,
    U64,
    Middleware,
    Signer,
    WalletError,
    NameOrAddress,
    Eip1559TransactionRequest
};
pub mod error;
use error::PortfolioError;
pub mod repository;
pub mod position;
pub mod profile;
pub mod portfolio;
pub mod builder;
pub mod statistics;
pub mod strategy;

#[async_trait]
pub trait OrderGenerator: Sync + Send {

    async fn generate_order_from_simulation_event(&mut self, trader_id: &TraderId, event: &SimulationEvent) -> Result<Option<OrderEvent>, error::PortfolioError>;
    async fn generate_exit_order(&mut self, trader_id: &TraderId, event: &SellSimulationEvent) -> Result<Option<OrderEvent>, PortfolioError>;
    async fn generate_force_exit_order(&mut self, trader_id: &TraderId, priority: profile::Priority) -> Result<Option<OrderEvent>, PortfolioError>;
    async fn generate_take_profit_order(&mut self, trader_id: &TraderId, priority: profile::Priority, sell_percentage: u8) -> Result<Option<OrderEvent>, PortfolioError>;
    async fn generate_test_exit_order(&mut self, trader_id: &TraderId) -> Result<Option<Vec<Transaction>>, PortfolioError>;
}

#[async_trait]
pub trait TransactionEventUpdater: Sync + Send {

    async fn update_from_transaction(&mut self, trader_id: &TraderId, transaction: &TransactionEvent) -> Result<Vec<Event>, error::PortfolioError>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BlockTargetType {
    Exact(BlockInfo),
    None,
}


#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderEvent
{
    #[serde(deserialize_with = "deserialize_order_id", serialize_with = "serialize_order_id")]
    pub order_id: OrderId,
    pub token: Address,
    pub block_target_type: BlockTargetType,
    pub order_type: OrderType,
    pub transactions: Vec<TransactionSigner>,
    pub priority: profile::Priority,
    pub transaction_type: profile::TransactionType,
}

impl OrderEvent
{
    /// Returns a OrderEventBuilder instance.
    pub fn builder() -> OrderEventBuilder {
        OrderEventBuilder::new()
    }

    pub fn get_target_block(&self) -> Option<BlockInfo> {
        match &self.block_target_type {
            BlockTargetType::Exact(block) => Some(block.clone()),
            _ => None
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionSigner
{
    pub transaction: Transaction,
    #[serde(skip_serializing)]
    pub signer: LocalWallet
}

impl TransactionSigner {
    pub fn new(
    transaction: Transaction,
    signer: LocalWallet
    ) -> Self {
        Self {
            transaction,
            signer
        }
    }
    
    pub async fn get_nonce(
    &self,
    client: &Arc<Provider<Ws>>
    ) -> Result<U256, ProviderError> {
    client.get_transaction_count(self.signer.address(), None).await        
    }	

    pub async fn generate_signed_eip1559_transactions(
        self,
        base_fee: U256,
        max_fee_per_gas: U256,
    ) -> Result<Bytes, WalletError> {
        let request = Eip1559TransactionRequest {
            to: Some(NameOrAddress::Address(self.transaction.to.unwrap())),
            from: Some(self.transaction.from),
            data: Some(self.transaction.input),
            chain_id: Some(U64::from(1)),
            max_priority_fee_per_gas: Some(max_fee_per_gas),
            max_fee_per_gas: Some(base_fee),
            gas: Some(self.transaction.gas), 
            nonce: Some(self.transaction.nonce),
            value: Some(self.transaction.value),
            access_list: self.transaction.access_list.unwrap_or_default(),
        };
    
        Ok(sign_eip1559(request, &self.signer).await?)
    }

}

#[derive(Debug, Clone, Serialize)]
pub enum OrderType
{
    Backrun(Transaction),
    Frontrun(Transaction),
    Normal,
}

impl Default for OrderType
{
    fn default() -> Self {
    Self::Normal
    }
}

#[derive(Debug, Default)]
pub struct OrderEventBuilder
{
    pub order_id: Option<OrderId>,
    pub token: Option<Address>,
    pub target_block: Option<BlockInfo>,
    pub order_type: Option<OrderType>,
    pub transactions: Option<Vec<TransactionSigner>>,
    pub priority: Option<profile::Priority>,
    pub transaction_type: Option<profile::TransactionType>,
}

impl OrderEventBuilder
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn order_id(self, value: OrderId) -> Self {
    Self {
        order_id: Some(value),
        ..self
    }
    }

    pub fn token(self, value: Address) -> Self {
    Self {
        token: Some(value),
        ..self
    }
    }

    pub fn order_type(self, value: OrderType) -> Self {
        Self {
            order_type: Some(value),
            ..self
        }
    }

    pub fn target_block(self, value: BlockInfo) -> Self {
        Self {
            target_block: Some(value),
            ..self
        }
    }

    pub fn transactions(self, value: Vec<TransactionSigner>) -> Self {
    Self {
        transactions: Some(value),
        ..self
    }
    }

    pub fn transaction_type(self, value: profile::TransactionType) -> Self {
    Self {
        transaction_type: Some(value),
        ..self
    }
    }

    pub fn priority(self, value: profile::Priority) -> Self {
    Self {
        priority: Some(value),
        ..self
    }
    }

    pub fn build(self) -> Result<OrderEvent, PortfolioError> {
        let block_target_type= match self.target_block {
            Some(target) => BlockTargetType::Exact(target),
            None => BlockTargetType::None
        };
        
        Ok(OrderEvent {           
        order_id: self
                .order_id
                .ok_or(PortfolioError::BuilderIncomplete("order_id"))?,  
	token: self
                .token
                .ok_or(PortfolioError::BuilderIncomplete("token"))?,            
            order_type: self
                .order_type
                .ok_or(PortfolioError::BuilderIncomplete("order_type"))?,
        block_target_type: block_target_type,
        transactions: self
                .transactions
                .ok_or(PortfolioError::BuilderIncomplete("transactions"))?,
        priority: self
                .priority
                .ok_or(PortfolioError::BuilderIncomplete("priority"))?,
        transaction_type: self
                .transaction_type
                .ok_or(PortfolioError::BuilderIncomplete("transaction_type"))?,
        })
    }
}