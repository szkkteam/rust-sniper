use crate::{
    portfolio::{
        error::PortfolioError,
        repository::{
            PositionHandler,
            ProfileHandler
        },
        profile::{
            Profile,
            ProfileUpdater,
            Priority,
            TransactionType,
            BundlePriority
        },
        position::{
            Position,
            PositionEnterer,
        },
        builder::{
            generate_backrun_transactions,
            generate_test_frontrun_transactions,
            generate_frontrun_transactions,
            generate_take_profit_transactions
        },
    },
    executor::{
        TransactionEvent,
    },
    simulator::{
        event::{
            SimulationEvent, 
            SimulationState,
            SellSimulationEvent,
        }
    },
    event::Event,
    types::{
        TraderId,
        PositionId,
        ProfileId,
        OrderId,
    },
    utils,
    token::Token,
};
use num_bigfloat::BigFloat;
use parking_lot::Mutex;
use ethers::prelude::{Address, U256, I256};
use super::{
    OrderGenerator,
    OrderEvent,
    OrderType,
    TransactionEventUpdater,
    statistics::{
        Statistics,
        StatisticsCalculator
    },
    strategy::StrategyGenerator,
};
use async_trait::async_trait;
use dashmap::{DashMap};
use std::sync::Arc;

pub struct PortfolioLego<Repository>
where
    Repository: PositionHandler + ProfileHandler
{    
    pub repository: Arc<Mutex<Repository>>,
    pub token_pool: Arc<DashMap<Address, Token>>,
    pub token_id: Address,
}

pub struct Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler
{
    repository: Arc<Mutex<Repository>>,
    token_pool: Arc<DashMap<Address, Token>>,
    token_id: Address,
}

impl<Repository> Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler
{

    pub fn new(lego: PortfolioLego<Repository>) -> Self {

        Self {
            repository: lego.repository,
            token_pool: lego.token_pool,
            token_id: lego.token_id,
        }
    }

    pub fn builder() -> PortfolioBuilder<Repository> {
        PortfolioBuilder::new()
    }
    
}

#[async_trait]
impl<Repository> StrategyGenerator for Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler + Send
{
    async fn generate_strategy_order(&mut self, trader_id: &TraderId, statistics: &Statistics) -> Result<Option<OrderEvent>, PortfolioError>
    {
        let profile_id = ProfileId::from(trader_id);
        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        match profile.strategy {
            Some(strategy) => {
                let gwei_20 = U256::from(20000000000u64);
                // TODO: rethink thie priority
                let priority = Priority {max_prio_fee_per_gas: gwei_20, priority: 1};


                let unrealized_pnl = BigFloat::from_u128(statistics.unralized_pnl.as_u128());
                let total_investment = BigFloat::from_u128(statistics.total_investment.as_u128());

                let target_pnl = total_investment * BigFloat::from_f64(strategy.take_out_initials_at);

                if 
                    unrealized_pnl > target_pnl &&
                    // Currently only take out initials and do not trigger again
                    statistics.realized_pnl == U256::zero()
                {
                    // TODO: Currently hard coded initials
                    self.generate_take_profit_order(trader_id, priority, 50).await

                } else 
                {
                    Ok(None)
                }
            },
            None => {
                Ok(None)
            }
        }

        //self.generate_take_profit_order(trader_id, Priority { max_prio_fee_per_gas: U256::zero(), priority: 1 }, 100).await
        //Ok(None)
    }
}


#[async_trait]
impl<Repository> StatisticsCalculator for Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler + Send
{
    async fn get_trader_statistics(&mut self, trader_id: &TraderId, event: &SellSimulationEvent) -> Result<Option<Statistics>, PortfolioError> {
        let position_id = PositionId::from(trader_id);

        let position = self.repository
            .lock()
            .get_open_position(&position_id)?;
        
        Ok(match position {
            Some(position) => {
                // Only positive, can unwrap
                let total_investment = position.total_investment_cost();
                let sell_gas_cost = match event.state.get_tx() {
                    Some(tx) => { 
                        let (gas, _) = utils::calcualte_transaction_cost(&tx);
                        gas
                    },
                    None => event.block.base_fee * 2
                };
                let sell_gas_cost = sell_gas_cost + U256::from(1000000000); // 1 gwei extra

                let backrun_gas_used: U256 = event.simulation.backrun.gas_used.into();
                let sell_cost = backrun_gas_used * sell_gas_cost;                
                
                // Only positive, can unwrap
                let backrun_balance_change = event.simulation.backrun.gross_balance_change;

                Some(Statistics::new(
                    trader_id.clone(),
                    total_investment,
                    backrun_balance_change,
                    position.realized_pnl
                ))
            },
            None => {
                None
            }
        })
    }

    
}

#[async_trait]
impl<Repository> OrderGenerator for Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler + Send
{
    async fn generate_order_from_simulation_event(&mut self, trader_id: &TraderId, event: &SimulationEvent) -> Result<Option<OrderEvent>, PortfolioError> {

        let position_id = PositionId::from(trader_id);
        let order_id = OrderId::from(trader_id);
        let profile_id = ProfileId::from(trader_id);

        let position = self.repository
            .lock()
            .get_open_position(&position_id)?;

        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        let mut order = OrderEvent::builder()
            .priority(profile.order.priority.clone())
            .order_id(order_id)
            .token(event.token.address.clone())
            .transaction_type(profile.order.transaction_type.clone());

        Ok(match (&event.state, position) {
            // Scenario 1) No position open, launch signal arrived -> Buy
            (SimulationState::Launch(state), None) => {
                let transactions = match &profile.order.taxes {
                    Some(taxes) => {
                     
                        if 
                        BigFloat::from_f64(taxes.buy_fee) >= state.taxes.buy_fee &&
                        BigFloat::from_f64(taxes.sell_fee) >= state.taxes.sell_fee 
                        {
                            generate_backrun_transactions(
                                event.token.pool.unwrap(),
                                state.limits.clone(),
                                &profile
                            ).await
                            // builder::generate_transactions (1 or 2)
                        } else {
                            // User taxes does not let us to buy
                            if BigFloat::from_f64(taxes.buy_fee) < state.taxes.buy_fee {
                                log::info!(
                                    "{}", format!("Order skipped due to buy taxes. Limit: {:?} - Real: {}", taxes.buy_fee, state.taxes.buy_fee.to_string())
                                );
                                return Err(PortfolioError::EntryOrderGeneration(
                                    "Order skipped due to buy tax limits"
                                ))
                            } else if BigFloat::from_f64(taxes.sell_fee) < state.taxes.sell_fee{
                                log::info!(
                                    "{}", format!("Order skipped due to sell taxes. Limit: {:?} - Real: {}", taxes.sell_fee, state.taxes.sell_fee.to_string())
                                );
                                return Err(PortfolioError::EntryOrderGeneration(
                                    "Order skipped due to sell tax imits"
                                ))
                                
                            }
                            return Err(PortfolioError::EntryOrderGeneration("Unkown reason"))
                            //return Ok(None)
                        }
                    }, 
                    None => {
                        generate_backrun_transactions(
                            event.token.pool.unwrap(),
                            state.limits.clone(),
                            &profile
                        ).await
                    }
                };
                if state.launch_block != event.block {
                    order = order.order_type(OrderType::Normal)
                } else {
                    order = order.order_type(OrderType::Backrun(state.tx.clone()))
                }
                // Setup the target block
                Some(order
                    .target_block(state.launch_block.clone())
                    .transactions(transactions)
                    .build()?)
            },
            // Scenario 2) Already running, no position open -> Buy
            (SimulationState::Changed(state), None) => {
                let transactions = match &profile.order.taxes {
                    Some(taxes) => {
                       
                        if 
                            BigFloat::from_f64(taxes.buy_fee) >= state.taxes.buy_fee &&
                            BigFloat::from_f64(taxes.sell_fee) >= state.taxes.sell_fee 
                        {
                            generate_backrun_transactions(
                                event.token.pool.unwrap(),
                                state.limits.clone(),
                                &profile
                            ).await
                            // builder::generate_transactions (1 or 2)
                        } else {
                            if BigFloat::from_f64(taxes.buy_fee) < state.taxes.buy_fee {
                                log::info!(
                                    "{}", format!("Order skipped due to buy taxes. Limit: {:?} - Real: {}", taxes.buy_fee, state.taxes.buy_fee.to_string())
                                );
                                return Err(PortfolioError::EntryOrderGeneration(
                                    "Order skipped due to buy tax limits"
                                ))
                            } else if BigFloat::from_f64(taxes.sell_fee) < state.taxes.sell_fee{
                                log::info!(
                                    "{}", format!("Order skipped due to sell taxes. Limit: {:?} - Real: {}", taxes.sell_fee, state.taxes.sell_fee.to_string())
                                );
                                return Err(PortfolioError::EntryOrderGeneration(
                                    "Order skipped due to sell tax limits"
                                ))
                            }                            
                            // User limits does not let us to buy
                            return Err(PortfolioError::EntryOrderGeneration("Unkown reason"))
                            //return Ok(None)
                        }                        
                    }, 
                    None => {
                        generate_backrun_transactions(
                            event.token.pool.unwrap(),
                            state.limits.clone(),
                            &profile
                        ).await
                    }
                };                
                // Setup the target block
                Some(order
                    .target_block(event.block.clone())
                    .transactions(transactions)
                    .order_type(OrderType::Normal)
                    .build()?)
            },
            _ => {
                None
            }
        })
          
    }

    
    async fn generate_force_exit_order(&mut self, trader_id: &TraderId, priority: Priority) -> Result<Option<OrderEvent>, PortfolioError> {
        let position_id = PositionId::from(trader_id);
        let order_id = OrderId::from(trader_id);
        let profile_id = ProfileId::from(trader_id);

        let position = self.repository
            .lock()
            .get_open_position(&position_id)?;

        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        Ok(match position {
            Some(_) => {
                let token = *self.token_pool.get(&self.token_id).unwrap();

                let transactions = generate_frontrun_transactions(
                    token.pool.unwrap(),
                    &profile
                ).await;

                Some(OrderEvent::builder()
                    .priority(priority)
                    .order_id(order_id)
                    .token(token.address.clone())
                    .transaction_type(TransactionType::Bundle { priority: BundlePriority::Auto })
                    //.target_block(event.block.clone())
                    .transactions(transactions)
                    .order_type(OrderType::Normal)
                    .build()?)
                    
            },
            _ => { None }
        })
    }

    async fn generate_take_profit_order(&mut self, trader_id: &TraderId, priority: Priority, sell_percentage: u8) -> Result<Option<OrderEvent>, PortfolioError> {
        let position_id = PositionId::from(trader_id);
        let order_id = OrderId::from(trader_id);
        let profile_id = ProfileId::from(trader_id);

        let position = self.repository
            .lock()
            .get_open_position(&position_id)?;

        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        Ok(match position {
            Some(_) => {
                let token = *self.token_pool.get(&self.token_id).unwrap();

                let transactions = generate_take_profit_transactions(
                    token.pool.unwrap(),
                    &profile,
                    sell_percentage,
                ).await;

                Some(OrderEvent::builder()
                    .priority(priority)
                    .order_id(order_id)
                    .token(token.address.clone())
                    .transaction_type(TransactionType::Bundle { priority: BundlePriority::Auto })
                    //.target_block(event.block.clone())
                    .transactions(transactions)
                    .order_type(OrderType::Normal)
                    .build()?)
                    
            },
            _ => { None }
        })
    }

    async fn generate_exit_order(&mut self, trader_id: &TraderId, event: &SellSimulationEvent) -> Result<Option<OrderEvent>, PortfolioError> {
        let position_id = PositionId::from(trader_id);
        let order_id = OrderId::from(trader_id);
        let profile_id = ProfileId::from(trader_id);
        
        let position = self.repository
            .lock()
            .get_open_position(&position_id)?;


        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        Ok(match (&profile.order.anti_rug, position) {
            (Some(anti_rug), Some(position)) => {

                let token = *self.token_pool.get(&self.token_id).unwrap();

                let mut order = OrderEvent::builder()
                    .priority(anti_rug.priority.clone())
                    .order_id(order_id)
                    .token(token.address.clone())
                    .transaction_type(TransactionType::Auto);
                
                let is_honeypot = event.is_honeypot || event.is_over_tax_limits(&profile.order.taxes);
                log::info!(
                    "{}", format!("Token {:?} is honeypot? {:?}", token.address,is_honeypot)
                );
                if is_honeypot {
                    let mut sell_gas_cost = match event.state.get_tx() {
                        Some(tx) => { 
                            let (gas, _) = utils::calcualte_transaction_cost(&tx);
                            gas
                        },
                        None => event.block.base_fee * 2
                    };

                    sell_gas_cost += anti_rug.priority.max_prio_fee_per_gas;

                    let frontrun_gas_used: U256 = event.simulation.frontrun.gas_used.into();
                    let sell_transaction_cost = frontrun_gas_used * sell_gas_cost;
                    let frontrun_balance_change = event.simulation.frontrun.gross_balance_change;
                    // If gas cost is higher than the amount what we would recive, does not worth to escape
                    #[cfg(feature = "dry")] 
                    {
                        let transactions = generate_frontrun_transactions(
                            event.token.pool.unwrap(),
                            &profile
                        ).await;

                        order = match event.state.get_tx() {
                            Some(tx) => {
                                log::warn!(
                                    "{}", format!("Transaction {:?} is honeypot", tx.hash)
                                );
                                order.order_type(OrderType::Frontrun(tx)) 
                            },
                            None => {
                                log::warn!(
                                    "{}", format!("Trasanction is honeypot")
                                );
                                order.order_type(OrderType::Normal)
                            }
                        };
                        return Ok(Some(order
                            .target_block(event.block.clone())
                            .transactions(transactions)
                            .build()?))
                    }                    
                    if sell_transaction_cost < frontrun_balance_change {
                        // Generate frontrun sell tx
                        let transactions = generate_frontrun_transactions(
                            event.token.pool.unwrap(),
                            &profile
                        ).await;

                        order = match event.state.get_tx() {
                            Some(tx) => {
                                log::warn!(
                                    "{}", format!("Transaction {:?} is honeypot", tx.hash)
                                );
                                order.order_type(OrderType::Frontrun(tx)) 
                            },
                            None => {
                                log::warn!(
                                    "{}", format!("Trasanction is honeypot")
                                );
                                order.order_type(OrderType::Normal)
                            }
                        };


                        Some(order
                            .target_block(event.block.clone())
                            .transactions(transactions)
                            .build()?)

                    } else {
                        // Well, it's hp, but does not worth to escape atm
                        log::warn!(
                            "{}", format!("Trasanction cost {:?} > Amount to be received {:?}", sell_transaction_cost, frontrun_balance_change)
                        );
                        return Ok(None);
                    }
                } else {
                    return Ok(None);
                }
            },
            _=> {
                // We don't have anti configured, or an open position
                return Ok(None)
            }
        })
        
    }

    async fn generate_test_exit_order(&mut self, trader_id: &TraderId) -> Result<Option<Vec<ethers::types::Transaction>>, PortfolioError> {
        let position_id = PositionId::from(trader_id);
        let profile_id = ProfileId::from(trader_id);

        let position = self.repository
            .lock()
            .get_open_position(&position_id)?;


        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        Ok(match position {
            Some(_) => {
                let token = *self.token_pool.get(&self.token_id).unwrap();
                Some(generate_test_frontrun_transactions(
                    token.pool.unwrap(),
                    &profile
                ).await)
            },
            None => { None }
        })
        
    }
    
}

#[async_trait]
impl<Repository> TransactionEventUpdater for Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler + Send
{
    async fn update_from_transaction(&mut self, trader_id: &TraderId, transaction: &TransactionEvent) -> Result<Vec<Event>, PortfolioError> {
        let mut generated_events = Vec::new();

        let position_id = PositionId::from(trader_id);
        let profile_id = ProfileId::from(trader_id);

        let profile = self.repository
            .lock()
            .get_profile(&profile_id)?
            .ok_or(PortfolioError::ProfileNotExists)?;

        let removed_position = self.repository
            .lock()
            .remove_position(&position_id)?;

        println!("removed_position: {:?}", removed_position);
        match removed_position {
            // EXIT SCENARIO - Transaction Confirmed Event with open position
            Some(position) => {
                
                let position = position.update_from_transaction(profile.contract_address, transaction).await;
                println!("position update_from_transaction: {:?}", position);

                if position.is_closed() {
                    println!("Position closed?: {:?}", position.is_closed());
                    generated_events.push(Event::PositionExited(position))                    
                } else {
                    self.repository
                        .lock()
                        .set_open_position(position.clone())?;

                    generated_events.push(Event::PositionUpdated(position))
                }
            },
            // OPEN SCENARION - Transaction Confirmed Event without open position
            None => {
                // Cost and investment calculations are done in the position enter
                let position = Position::enter(trader_id, profile.contract_address, transaction).await?;
                
                self.repository
                    .lock()
                    .set_open_position(position.clone())?;

                generated_events.push(Event::PositionNew(position))
            }
        };       
        //println!("generated_events: {:?}", generated_events);
        Ok(generated_events)
    }
}

impl<Repository> ProfileUpdater for Portfolio<Repository>
where
    Repository: PositionHandler + ProfileHandler
{   
    
    fn update_profile(&mut self, profile: Profile) -> Result<(), PortfolioError> {
        Ok(self.repository
            .lock()
            .update_profile(profile)?)
    }
   
    fn delete_profile(&mut self, profile_id: &ProfileId) -> Result<(), PortfolioError> {
        Ok(())
    }

    fn get_profile(&mut self, profile_id: &ProfileId) -> Result<Option<Profile>, PortfolioError> {
        Ok(self.repository
            .lock()
            .get_profile(&profile_id)?)
    }

}

pub struct PortfolioBuilder<Repository>
where
    Repository: PositionHandler + ProfileHandler
{
    repository: Option<Arc<Mutex<Repository>>>,
    initial_profile: Option<Profile>,
    token_pool: Option<Arc<DashMap<Address, Token>>>,
    token_id: Option<Address>,
}

impl<Repository> PortfolioBuilder<Repository>
where
    Repository: PositionHandler + ProfileHandler
{
    
    pub fn new() -> Self {
        Self {
           repository: None,
           initial_profile: None,
           token_pool: None,
           token_id: None,
        }
    }

    pub fn initial_profile(self, value: Profile) -> Self {
        Self {
            initial_profile: Some(value),
            ..self
        }
    }

    pub fn repository(self, value: Arc<Mutex<Repository>>) -> Self {
        Self {
            repository: Some(value),
            ..self
        }
    }

    pub fn token_id(self, value: Address) -> Self {
        Self {
            token_id: Some(value),
            ..self
        }
    }

    pub fn token_pool(self, value: Arc<DashMap<Address, Token>>) -> Self {
        Self {
            token_pool: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<Portfolio<Repository>, PortfolioError> {
        let portfolio = 
            Portfolio {
            
            repository: self
                .repository
                .ok_or(PortfolioError::BuilderIncomplete("repository"))?,
            token_pool: self
                .token_pool
                .ok_or(PortfolioError::BuilderIncomplete("token_pool"))?,
                token_id: self
                .token_id
                .ok_or(PortfolioError::BuilderIncomplete("token_id"))?,
          
        };
        // Set initial profile
        match self.initial_profile {
            Some(profile) => {
                portfolio.repository
                    .lock()
                    .update_profile(profile)?
            },
            None => {}
        };

        Ok(portfolio)
    }
}