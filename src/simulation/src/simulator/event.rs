use crate::{
    token::Token,
    utils::state_diff::{StateDiff},
    stream::{BlockOracle, BlockInfo},
    portfolio::profile::{Profile, Taxes},
    types::{
        TraderId,
        deserialize_trader_id,
        serialize_trader_id
    }
};
use serde::{Deserialize, Serialize};
use ethers::prelude::{Transaction, U256};
use num_bigfloat::BigFloat;
use super::{
    simulation::{
        SimulationResult,
        SellSimulationResult
    },
};


mod string {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{de, Serializer, Deserialize, Deserializer};

    pub fn serialize<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
        where T: Display,
              S: Serializer
    {
        serializer.collect_str(value)
    }
    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
        where T: FromStr,
              T::Err: Display,
              D: Deserializer<'de>
    {
        String::deserialize(deserializer)?.parse().map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionNew {
    pub token: Token,
    #[serde(skip_serializing)]
    pub oracle: BlockOracle,
    pub tx: Transaction,
    #[serde(skip_serializing)]
    pub state_diff: StateDiff
}


#[derive(Clone, Debug, Serialize, Deserialize, )]
#[serde(rename_all = "camelCase")]
pub struct SellSimulationEvent
{
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
    pub token: Token,
    pub block: BlockInfo,
    pub simulation: SellSimulationResult,
    pub state: SimulationState,
    pub is_honeypot: bool
}

impl SellSimulationEvent {
    pub fn new(
        trader_id: TraderId,
        token: Token,
        block: BlockInfo,
        simulation: SellSimulationResult,
        state: SimulationState,
    ) -> Self {
        let mut event = Self {
            trader_id,
            token,
            block,
            simulation,
            state,
            is_honeypot: false
        };
        event.is_honeypot = event.is_honeypot();
        event
    }
  
    pub fn is_sell_price_impact_too_big(&self) -> bool {
        //let frontron_abs = self.simulation.frontrun.gross_balance_change.abs().
        let frontrun_v = BigFloat::from_u128(self.simulation.frontrun.gross_balance_change.as_u128());
        let backrun_v = BigFloat::from_u128(self.simulation.backrun.gross_balance_change.as_u128());

        let price_impact = backrun_v / frontrun_v * BigFloat::from(100);
        // If the frontrun-backrun price impact is more than 60% in 1 TX
        // TODO: This could come from config, to frontrun any big CA sells or other sniper dumps
        // TODO: refine this treshold
        price_impact < BigFloat::from(40)
    }
    
    pub fn is_over_tax_limits(&self, taxes: &Option<Taxes>) -> bool {
         match (taxes, self.state.get_taxes()) {
            (Some(limits), Some(taxes)) => {
                BigFloat::from_f64(limits.buy_fee) < taxes.buy_fee ||
                BigFloat::from_f64(limits.sell_fee) < taxes.sell_fee
            },
            _ => { false }
        }
    }

    pub fn is_max_tx_zero(&self) -> bool {
        self.state.get_max_tx().unwrap_or(U256::MAX) == U256::zero()
    }

    pub fn is_liquidity_manipulated(&self) -> bool {
        let liquidity = self.state.get_liquidity();

        liquidity > BigFloat::from(100) ||
        liquidity < BigFloat::parse("1e-6").unwrap()
    }

    pub fn is_sell_failed(&self) -> bool {
        let sell_valid = match (&self.simulation.frontrun.is_sell_failed(), &self.simulation.backrun.is_sell_failed()) {
            // All fine its Not honeypot
            (false, false) => {
                false
            },
            // Frontrun fine, backrun HP -> Honeypot                    
            (false, true) => {
                true
            },
            // Frontrun HP backrun fine ... some weird thing
            (true, false) => {
                // Some weird thing...
                false
            },
            // Honeypot, but either we missed the TX somehow, or it was private tx. sic!
            (true, true) => {
                log::warn!(
                    "{}", format!("Token {:?} become honeypot by private TX sic!", self.token.address)
                );
                false
            }
        };
        sell_valid || self.state.has_error()
    }

    pub fn is_honeypot(&self) -> bool {
        let is_max_tx_zero = self.is_max_tx_zero();
        let is_liquidity_manipulated = self.is_liquidity_manipulated();
        let is_sell_failed = self.is_sell_failed();
        
        let is_sell_price_impact_too_big = self.is_sell_price_impact_too_big();
        log::info!(
            "{}", format!("Token {:?} honeypot status: max tx zero: {:?} liquidity manipulated: {:?} sell failed: {:?} sell price impact too big: {:?}", 
                self.token.address,
                is_max_tx_zero,
                is_liquidity_manipulated,
                is_sell_failed,
                is_sell_price_impact_too_big
            )
        );
        // Take into consideration every possible HP scenario
        is_max_tx_zero ||
        is_liquidity_manipulated ||
        is_sell_failed ||
        is_sell_price_impact_too_big
        
    }
}

pub trait FromResult
{
    fn from_result(simulation: &SimulationResult) -> Self;
}

#[derive(Clone, PartialEq, Debug, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionTaxes
{
    #[serde(with = "string")]
    pub buy_fee: BigFloat,
    #[serde(with = "string")]
    pub sell_fee: BigFloat,
}

impl FromResult for TransactionTaxes
{
    fn from_result(simulation: &SimulationResult) -> Self {
        Self {
            buy_fee: simulation.buy_fee,
            sell_fee: simulation.sell_fee,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionLimits
{
    pub max_buy_amount: Option<U256>,
    pub max_sell_amount: Option<U256>,
}

impl FromResult for TransactionLimits
{
    fn from_result(simulation: &SimulationResult) -> Self {
        Self {
            max_buy_amount: simulation.max_tx,
            max_sell_amount: simulation.max_tx,
        }
    }
}

#[derive(Clone, PartialEq, Debug, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GasLimits
{
    pub buy_gas: U256,
    pub sell_gas: U256,
}

impl FromResult for GasLimits
{
    fn from_result(simulation: &SimulationResult) -> Self {
        Self {
            buy_gas: simulation.buy_gas,
            sell_gas: simulation.sell_gas,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SimulationEvent
{
    pub token: Token,
    pub block: BlockInfo,   
    pub state: SimulationState,

}

impl SimulationEvent
{
    pub fn new(
        token: Token,
        block: BlockInfo,
        state: SimulationState,
    ) -> Self {
        Self {
            token,
            block,
            state
        }
    }

}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SimulationStateClosed {}

#[derive(Clone, Debug, Serialize, Deserialize, )]
#[serde(rename_all = "camelCase")]
pub struct SimulationStateLaunch {
    pub launch_block: BlockInfo,
    pub tx: Transaction,
    pub limits: TransactionLimits,
    pub taxes: TransactionTaxes,
    pub gas: GasLimits,
    #[serde(with = "string")]
    pub liquidity_ratio: BigFloat,
    pub error: Option<String>
}

impl From<SimulationResult> for SimulationStateLaunch 
{
    fn from(value: SimulationResult) -> Self {
        Self {
            launch_block: value.first_valid_block.unwrap_or(value.block),
            tx: value.tx.unwrap_or_default(),
            limits: TransactionLimits { 
                max_buy_amount: value.max_tx, max_sell_amount: value.max_tx
            },
            taxes: TransactionTaxes { 
                buy_fee: value.buy_fee, sell_fee: value.sell_fee
            },
            gas: GasLimits { 
                buy_gas: value.buy_gas, sell_gas: value.sell_gas
            },
            liquidity_ratio: value.liquidity_ratio,
            error: value.reason
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, )]
#[serde(rename_all = "camelCase")]
pub struct SimulationStateChanged {
    pub tx: Option<Transaction>,
    pub limits: TransactionLimits,
    pub taxes: TransactionTaxes,
    pub gas: GasLimits,
    #[serde(with = "string")]
    pub liquidity_ratio: BigFloat,
    pub error: Option<String>
}


impl From<SimulationResult> for SimulationStateChanged 
{
    fn from(value: SimulationResult) -> Self {
        Self {
            tx: value.tx,
            limits: TransactionLimits { 
                max_buy_amount: value.max_tx, max_sell_amount: value.max_tx
            },
            taxes: TransactionTaxes { 
                buy_fee: value.buy_fee, sell_fee: value.sell_fee
            },
            gas: GasLimits { 
                buy_gas: value.buy_gas, sell_gas: value.sell_gas
            },
            liquidity_ratio: value.liquidity_ratio,
            error: value.reason
        }
    }
}

impl From<SimulationStateLaunch> for SimulationStateChanged 
{
    fn from(value: SimulationStateLaunch) -> Self {
        Self {
            tx: Some(value.tx),
            limits: value.limits,
            taxes: value.taxes,
            gas: value.gas,
            liquidity_ratio: value.liquidity_ratio,
            error: value.error,
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize, )]
#[serde(rename_all = "camelCase")]
pub enum SimulationState 
{
    Closed(SimulationStateClosed),
    Launch(SimulationStateLaunch),
    Changed(SimulationStateChanged),
}

impl Default for SimulationState {
    fn default() -> Self {
        SimulationState::Closed(SimulationStateClosed{})
    }
}

impl SimulationState 
{
    pub fn get_liquidity(&self) -> BigFloat {
        match self {
            Self::Closed(_) => { BigFloat::from(0) },
            Self::Launch(state) => { state.liquidity_ratio },
            Self::Changed(state) => { state.liquidity_ratio },
        }
    }

    pub fn get_error(&self) -> Option<String> {
        match self {
            Self::Closed(_) => { None },
            Self::Launch(state) => { state.error.clone() },
            Self::Changed(state) => { state.error.clone() },
        }
    }

    pub fn has_error(&self) -> bool {
        match self {
            Self::Closed(_) => { false },
            Self::Launch(state) => { state.error.is_some() },
            Self::Changed(state) => { state.error.is_some() },
        }
    }

    pub fn get_taxes(&self) -> Option<TransactionTaxes> {
        match self {
            Self::Closed(_) => { None },
            Self::Launch(state) => { Some(state.taxes) },
            Self::Changed(state) => { Some(state.taxes) }
        }
    }

    pub fn get_tx(&self) -> Option<Transaction> {
        match self {
            Self::Closed(_) => { None },
            Self::Launch(state) => { Some(state.tx.clone()) },
            Self::Changed(state) => { state.tx.clone() }
        }
    }

    pub fn get_max_tx(&self) -> Option<U256> {
        match self {
            Self::Closed(_) => { None },
            Self::Launch(state) => { state.limits.max_sell_amount },
            Self::Changed(state) => { state.limits.max_sell_amount },
        }
    }

    pub fn buy_valid(&self) -> bool {
        match self {
            Self::Closed(_) => false,
            Self::Launch(state) => {
                // At least 20% of liquidity added
                state.limits.max_buy_amount.unwrap_or(U256::MAX) > U256::zero() &&
                state.liquidity_ratio > BigFloat::from(20) &&
                state.taxes.buy_fee <= BigFloat::from(95) &&
                state.error.is_none()
            },
            Self::Changed(state) => {
                state.limits.max_buy_amount.unwrap_or(U256::MAX) > U256::zero() &&
                state.taxes.buy_fee <= BigFloat::from(95) &&
                state.error.is_none()
            },
        }
    }

    pub fn sell_valid(&self) -> bool {
        match self {
            Self::Closed(_) => false,
            Self::Launch(state) => {
                state.taxes.sell_fee < BigFloat::from(100) && 
                state.error.is_none()
            },
            Self::Changed(state) => {
                state.limits.max_sell_amount.unwrap_or(U256::MAX) > U256::zero() &&
                state.taxes.sell_fee < BigFloat::from(100) && 
                state.error.is_none()
            },
        }
    }
}
