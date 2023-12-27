use crate::{
    event::{Event, MessageTransmitter},
    simulator::{
        SimulatorHandle,
        SimulatorRequest
    },
    portfolio::{
        OrderGenerator,
        TransactionEventUpdater,
        profile::{
            Priority,
            ProfileUpdater,
            TransactionType,
            BundlePriority,
        },
        statistics::{
            StatisticsCalculator,
        },
        strategy::StrategyGenerator,
        OrderEvent,
        TransactionSigner
    },
    executor::{
        OrderEventWithResponse,
    },
    types::{TraderId, ProfileId, interface::{UpdateAntiRugInterface, ForceExitPositionInterface, TakeProfitInterface}},
};
use std::{collections::VecDeque};

use tokio::sync::{mpsc};
use tokio;
use ethers::prelude::{Transaction};

use super::{
    error::EngineError,
    TraderTerminated
};

#[derive(Debug)]
pub enum Command {
    Terminate,
    UpdateAntiRug(UpdateAntiRugInterface),
    ForceExitPosition(ForceExitPositionInterface),
    TakeProfit(TakeProfitInterface),

}

pub struct TraderLego<EventTx, Portfolio> 
where 
    EventTx: MessageTransmitter<Event> + Send,  
    Portfolio: OrderGenerator + StatisticsCalculator + StrategyGenerator + ProfileUpdater + TransactionEventUpdater + Send
{    
    pub trader_id: TraderId,
    pub command_rx: mpsc::Receiver<Command>,
    pub event_tx: EventTx,
    pub simulator: SimulatorHandle,
    pub event_q: VecDeque<Event>,
    pub portfolio: Portfolio,
    pub executor_tx: mpsc::Sender<OrderEventWithResponse>,
}

pub struct Trader<EventTx, Portfolio>
where
    EventTx: MessageTransmitter<Event> + Clone,
    Portfolio: OrderGenerator + StatisticsCalculator + StrategyGenerator + ProfileUpdater + TransactionEventUpdater + Send + 'static,
{
    trader_id: TraderId,
    command_rx: mpsc::Receiver<Command>,
    event_tx: EventTx,
    simulator: SimulatorHandle,
    event_q: VecDeque<Event>,
    portfolio: Portfolio,
    executor_tx: mpsc::Sender<OrderEventWithResponse>,

}

impl<EventTx, Portfolio> Trader<EventTx, Portfolio>
where
    EventTx: MessageTransmitter<Event> + Send + Clone + 'static,
    Portfolio: OrderGenerator + StatisticsCalculator + StrategyGenerator + ProfileUpdater + TransactionEventUpdater + Send + 'static,
{

    pub fn new(lego: TraderLego<EventTx, Portfolio>) -> Self {

        Self {
            trader_id: lego.trader_id,
            command_rx: lego.command_rx,
            event_tx: lego.event_tx,
            simulator: lego.simulator,
            event_q: VecDeque::with_capacity(10),
            portfolio: lego.portfolio,
            executor_tx: lego.executor_tx,
        }
    }

    pub fn builder() -> TraderBuilder<EventTx, Portfolio> {
        TraderBuilder::new()
    }
   
    async fn estimate_gas(&self, order: &OrderEvent) -> Option<Vec<Transaction>> {
        let (request_tx, mut response) = mpsc::channel(1);            
        // Send estimate gas request
        match self.simulator.simulation_request.send(SimulatorRequest::EstimateGas(
            order.get_target_block(),
            order.transactions
                .iter()
                .map(|f| f.transaction.clone())
                .collect()
            ,
            request_tx)
        ).await {
            Ok(_) => {
                match response.recv().await {
                    Some(v) => {
                        match v {
                            Ok(r) => Some(r),
                            Err(e) => { 
                                log::error!(
                                    "{}", format!("Failed to estimate gas: {:?}", e)
                                );
                                None
                             }
                        }
                    },
                    None => {
                        log::warn!(
                            "{}", format!("Estimate gas received None")
                        );
                        None
                    }
                }
            },
            Err(e) => {
                log::error!(
                    "{}", format!("Failed to send estimate gas request: {:?}", e)
                );
                None
            }
        }
    }

    async fn entry_trade_check(&self) -> Option<Event> {
        // oneshot request-response channel
        let (request_tx, mut response) = mpsc::channel(1);            
        // Send simulation request
        match self.simulator.simulation_request.send(SimulatorRequest::TradeSimulation(request_tx)).await {
            Ok(_) => {
                match response.recv().await {
                    Some(v) => {
                        match v {
                            Ok(event) => Some(Event::SimulationEvent(event)),
                            Err(_) => { None }
                        }
                    },
                    None => {
                        log::warn!(
                            "{}", format!("Simulation request received None")
                        );
                        None
                    }
                    
                }
                
            },
            Err(e) => {
                log::error!(
                    "{}", format!("Simulation request for `TradeSimulation` failed.")
                );
                None
            }
        }
    }

    async fn update_antirug(&mut self) {
        if let Some(transactions) = self.portfolio
            .generate_test_exit_order(&self.trader_id)
            .await.unwrap_or_else(|op| {
                log::warn!(
                    "{}", format!("Failed to generate Anti-Rug transaction: {:?}", op)
                );
                None
            })
        {       
            println!("Test frontrun order: {:?}", transactions);
            match self.simulator.simulation_request.send(
                SimulatorRequest::RegisterAntiRug(
                    self.trader_id.clone(),
                    transactions
                )
            ).await {
                Ok(_) => {
                    log::info!(
                        "{}", format!("Honeypot checker TX set for: {:?}", self.trader_id.to_string())
                    );
                },
                Err(e) => {
                    log::error!(
                        "{}", format!("Failed to set Honeypot checker TX for: {:?}", self.trader_id.to_string())
                    );
                }
            }
        } else {
            match self.simulator.simulation_request.send(
                SimulatorRequest::DeRegisterAntiRug(
                    self.trader_id.clone(),
                )
            ).await {
                Ok(_) => {
                    log::info!(
                        "{}", format!("Anti-Rug disabled for: {:?}", self.trader_id.to_string())
                    );
                },
                Err(e) => {
                    log::error!(
                        "{}", format!("Failed to disable Anti-Rug enabled for: {:?}", self.trader_id.to_string())
                    );
                }
            }
        }
    }

    async fn take_profit(&mut self, priority: Priority, sell_percentage: u8) {
        if let Some(mut order) = self.portfolio
            .generate_take_profit_order(&self.trader_id, priority, sell_percentage)
            .await.unwrap_or_else(|op| {
                log::warn!(
                    "{}", format!("Failed to generate order: {:?}", op)
                );
                None
            })
        {                
            print!("Take profit estimate gas");
            match self.estimate_gas(&order).await {
                Some(r) => {
                    let signers: Vec<_> = order.transactions.iter().
                        map(|t| {
                            t.signer.clone()
                        })
                        .collect();

                    let txs: Vec<TransactionSigner> = r.iter()
                        .flat_map(|tx| 
                            signers
                                .iter()
                                .map(move |s| TransactionSigner::new(tx.clone(), s.clone()))
                        )
                        .collect();
                    
                    order.transactions = txs;
                },
                None => {}
            }
            let target_block = order.get_target_block();
            match target_block {
                Some(target_block) => {
                    log::info!(
                        "{}", format!("Order sent targeting {:?}", target_block.number)
                    );
                },
                None => {
                    log::info!(
                        "{}", format!("Order sent targeting the next block")
                    );
                }
            }
            
            self.event_tx.send(Event::OrderNew(order.clone()));
            self.event_q.push_back(Event::OrderNew(order));
        }
    }

    async fn force_exit_position(&mut self, priority: Priority) {
        if let Some(mut order) = self.portfolio
            .generate_force_exit_order(&self.trader_id, priority)
            .await.unwrap_or_else(|op| {
                log::warn!(
                    "{}", format!("Failed to generate order: {:?}", op)
                );
                None
            })
        {                
            match self.estimate_gas(&order).await {
                Some(r) => {
                    let signers: Vec<_> = order.transactions.iter().
                        map(|t| {
                            t.signer.clone()
                        })
                        .collect();

                    let txs: Vec<TransactionSigner> = r.iter()
                        .flat_map(|tx| 
                            signers
                                .iter()
                                .map(move |s| TransactionSigner::new(tx.clone(), s.clone()))
                        )
                        .collect();
                    
                    order.transactions = txs;
                },
                None => {}
            }
            let target_block = order.get_target_block();
            match target_block {
                Some(target_block) => {
                    log::info!(
                        "{}", format!("Order sent targeting {:?}", target_block.number)
                    );
                },
                None => {
                    log::info!(
                        "{}", format!("Order sent targeting the next block")
                    );
                }
            }
            
            self.event_tx.send(Event::OrderNew(order.clone()));
            self.event_q.push_back(Event::OrderNew(order));
        }
    }

    pub async fn run(mut self) {

        match self.entry_trade_check().await {
            Some(event) => {
                self.event_tx.send(event.clone());
                self.event_q.push_back(event); 
            }
            None => {}
        };

        // TODO: Why do I need this?
        //self.update_antirug().await;

        'trader: loop {                
            // Drain the queue first
            while let Some(event) = self.event_q.pop_front() {
                match event {
                    Event::TraderStatisticsUpdated(statistics) => {
                        if let Some(mut order) = self.portfolio
                            .generate_strategy_order(&self.trader_id, &statistics)
                            .await.unwrap_or_else(|op| {
                                log::warn!(
                                    "{}", format!("Failed to generate order: {:?}", op)
                                );
                                None
                            })
                        {                
                            match self.estimate_gas(&order).await {
                                Some(r) => {
                                    let signers: Vec<_> = order.transactions.iter().
                                        map(|t| {
                                            t.signer.clone()
                                        })
                                        .collect();

                                    let txs: Vec<TransactionSigner> = r.iter()
                                        .flat_map(|tx| 
                                            signers
                                                .iter()
                                                .map(move |s| TransactionSigner::new(tx.clone(), s.clone()))
                                        )
                                        .collect();
                                    
                                    order.transactions = txs;
                                },
                                None => {}
                            }
                            let target_block = order.get_target_block();
                            match target_block {
                                Some(target_block) => {
                                    log::info!(
                                        "{}", format!("Order sent targeting {:?}", target_block.number)
                                    );
                                },
                                None => {
                                    log::info!(
                                        "{}", format!("Order sent targeting the next block")
                                    );
                                }
                            }
                            
                            self.event_tx.send(Event::OrderNew(order.clone()));
                            self.event_q.push_back(Event::OrderNew(order));
                        }
                    },
                    // This could trigger the buy, if the launch TX was a private TX
                    Event::BlockSimulationEvent(event) => {
                        if let Some(mut order) = match self.portfolio
                            .generate_order_from_simulation_event(&self.trader_id, &event)
                            .await {
                                Ok(v) => { v },
                                Err(e) => {
                                    log::warn!(
                                        "{}", format!("Failed to generate order: {:?}", e)
                                    );
                                    break 'trader;
                                }
                            }
                        {                
                            match self.estimate_gas(&order).await {
                                Some(r) => {
                                    let signers: Vec<_> = order.transactions.iter().
                                        map(|t| {
                                            t.signer.clone()
                                        })
                                        .collect();

                                    let txs: Vec<TransactionSigner> = r.iter()
                                        .flat_map(|tx| 
                                            signers
                                                .iter()
                                                .map(move |s| TransactionSigner::new(tx.clone(), s.clone()))
                                        )
                                        .collect();
                                    
                                    order.transactions = txs;
                                },
                                None => {
                                    continue;
                                }
                            }
                            let target_block = order.get_target_block();
                            match target_block {
                                Some(target_block) => {
                                    log::info!(
                                        "{}", format!("Order sent targeting {:?}", target_block.number)
                                    );
                                },
                                None => {
                                    log::info!(
                                        "{}", format!("Order sent targeting the next block")
                                    );
                                }
                            }
                            
                            self.event_tx.send(Event::OrderNew(order.clone()));
                            self.event_q.push_back(Event::OrderNew(order));
                        }
                    },
                    Event::SimulationEvent(event) => {
                        // TODO: We need an event to inform the user, the order was not generated to whatever reasons!!

                        if let Some(mut order) = match self.portfolio
                            .generate_order_from_simulation_event(&self.trader_id, &event)
                            .await {
                                Ok(v) => { v },
                                Err(e) => {
                                    log::warn!(
                                        "{}", format!("Failed to generate order: {:?}", e)
                                    );
                                    break 'trader;
                                }
                            }
                        {                
                            match self.estimate_gas(&order).await {
                                Some(r) => {
                                    let signers: Vec<_> = order.transactions.iter().
                                        map(|t| {
                                            t.signer.clone()
                                        })
                                        .collect();

                                    let txs: Vec<TransactionSigner> = r.iter()
                                        .flat_map(|tx| 
                                            signers
                                                .iter()
                                                .map(move |s| TransactionSigner::new(tx.clone(), s.clone()))
                                        )
                                        .collect();
                                    
                                    order.transactions = txs;
                                },
                                None => {
                                    continue;
                                }
                            }
                            let target_block = order.get_target_block();
                            match target_block {
                                Some(target_block) => {
                                    log::info!(
                                        "{}", format!("Order sent targeting {:?}", target_block.number)
                                    );
                                },
                                None => {
                                    log::info!(
                                        "{}", format!("Order sent targeting the next block")
                                    );
                                }
                            }
                            
                            self.event_tx.send(Event::OrderNew(order.clone()));
                            self.event_q.push_back(Event::OrderNew(order));
                        }
                    },
                    // Used for profit check calculation
                    Event::BlockSellSimulationEvent(event) => {
                        if event.trader_id == self.trader_id {           
                            
                            if let Some(statistics) = self.portfolio
                                .get_trader_statistics(&self.trader_id, &event)
                                .await.unwrap_or_else(|op| {
                                    log::warn!(
                                        "{}", format!("Failed to calculate PNL for transaction: {:?}", op)
                                    );
                                    None
                                })
                            {
                                let event = Event::TraderStatisticsUpdated(statistics);
                                self.event_q.push_back(event.clone());
                                self.event_tx.send(event);
                            } else {
                                log::warn!(
                                    "{}", format!("Failed to generate trader statistics for {:?}", self.trader_id.to_string())
                                );
                            }                        
                        }
                        // Not my signal
                    },
                    Event::SellSimulationEvent(event) => {
                        if event.trader_id == self.trader_id {                            
                            if let Some(mut order) = self.portfolio
                                .generate_exit_order(&self.trader_id, &event)
                                .await.unwrap_or_else(|op| {
                                    log::warn!(
                                        "{}", format!("Failed to generate order: {:?}", op)
                                    );
                                    None
                                })
                            {
                                match self.estimate_gas(&order).await {
                                    Some(r) => {
                                        let signers: Vec<_> = order.transactions.iter().
                                            map(|t| {
                                                t.signer.clone()
                                            })
                                            .collect();
    
                                        let txs: Vec<TransactionSigner> = r.iter()
                                            .flat_map(|tx| 
                                                signers
                                                    .iter()
                                                    .map(move |s| TransactionSigner::new(tx.clone(), s.clone()))
                                            )
                                            .collect();
                                        
                                        order.transactions = txs;
                                    },
                                    None => {
                                        continue;
                                    }
                                }
                                let target_block = order.get_target_block();
                                match target_block {
                                    Some(target_block) => {
                                        log::info!(
                                            "{}", format!("Order sent targeting {:?}", target_block.number)
                                        );
                                    },
                                    None => {
                                        log::info!(
                                            "{}", format!("Order sent targeting the next block")
                                        );
                                    }
                                }
                                
                                self.event_tx.send(Event::OrderNew(order.clone()));
                                self.event_q.push_back(Event::OrderNew(order));
                            }
                            /*
                            else {
                                log::warn!(
                                    "{}", format!("Failed to generate sell simulation event for {:?}", self.trader_id.to_string())
                                );
                            } 
                            */
                        }
                        // Not my signal
                    },
                    Event::OrderNew(order) => {     
                        let target_block = order.get_target_block();
                        let tx_type = &order.transaction_type.clone();         
                        // Create the response channel                
                        let (response_tx, mut response_rx) = mpsc::channel(1);
                        // Send TX to executor
                        match self.executor_tx.send((order.clone(), response_tx)).await {
                            Ok(_) => {},
                            Err(e) => {
                                log::error!(
                                    "{}", format!("Failed to send order to executor: {:?}", e)
                                );
                                continue;
                            }
                        };
                        //println!("Waiting for order response..");
                        // Wait for response (12s - ... lot of seconds)
                        // TODO: Maybe do a select here, listen to incoming sim events, and if HP, somehow try to cancel??
                        if let Some(transaction) = response_rx.recv().await {
                            //println!("Order response recived: {:?}", transaction);
                            self.event_tx.send(Event::TransactionEvent(transaction.clone()));

                            self.event_q.push_back(Event::TransactionEvent(transaction));
                        } else {
                            // Transaction was not included in the block (sender dropped without response)
                            match tx_type {
                                TransactionType::Bundle { priority } => {
                                    match priority {
                                        BundlePriority::FirstOnly => {
                                            log::info!(
                                                "{}", format!("Trader {:?} exiting because bundle was not created by trader", self.trader_id.to_string())
                                            );
                                            self.event_q.push_back(Event::TraderTerminated(TraderTerminated {trader_id: self.trader_id.clone() }));
                                        },
                                        _ => {
                                            
                                        }                                        
                                    };
                                },
                                _ => {}
                            };
                            match target_block {
                                Some(target_block) => {
                                    log::error!(
                                        "{}", format!("Trader {:?} order was not included in target block {:?}", self.trader_id.to_string(), target_block.number)
                                    );
                                },
                                None => {
                                    log::error!(
                                        "{}", format!("Trader {:?} order was not included in the next block", self.trader_id.to_string())
                                    );
                                }
                            }
                            
                        }
                        //println!("Continue processing");

                    },
                    Event::TransactionEvent(transaction) => {
                        match self.portfolio.update_from_transaction(&self.trader_id, &transaction).await {
                            Ok(generated_events) => {
                                self.event_tx.send_many(generated_events.clone());
                                for event in generated_events {
                                    self.event_q.push_back(event)
                                }
                            },
                            Err(e) => {
                                log::error!(
                                    "{}", format!("Failed to update position from transaction {:?}", e)
                                );
                            }
                        };
                    },
                    
                    Event::PositionNew(_) => {
                        log::info!(
                            "{}", format!("Trader {:?} entered a position", self.trader_id.to_string())
                        );
                        self.update_antirug().await;
                    },
                    Event::PositionExited(position) => {
                        log::info!(
                            "{}", format!("Trader {:?} fully exited its position", self.trader_id.to_string())
                        );
                        break 'trader;
                    },
                    Event::PositionUpdated(position) => {
                        log::info!(
                            "{}", format!("Trader {:?} position updated", self.trader_id.to_string())
                        );
                        self.update_antirug().await;
                    },
                    Event::TraderTerminated(trader_id) => {
                        self.event_tx.send(Event::TraderTerminated(trader_id));
                        log::info!(
                            "{}", format!("Trader {:?} terminated", self.trader_id.to_string())
                        );
                        break 'trader;
                    },
                    _ => {}
                    //Event::Or

                };
            }
            
            tokio::select! {
                // First check the cmd
                Some(command) = self.command_rx.recv() => {
                    match command {

                        Command::Terminate => {
                            self.event_q.push_back(Event::TraderTerminated(TraderTerminated { trader_id: self.trader_id.clone() }));
                        },                      
                        Command::UpdateAntiRug(anti_rug) => {
                            let profile_id = ProfileId::from(self.trader_id.clone());
                            if let Some(mut profile) = self.portfolio.get_profile(&profile_id).unwrap_or_else(|op| {
                                log::warn!(
                                    "{}", format!("Failed to generate order: {:?}", op)
                                );
                                None
                                })
                            {
                                profile.order.anti_rug = anti_rug.anti_rug;
                                self.portfolio.update_profile(profile);
                                self.update_antirug().await;
                            }
                        },
                        Command::ForceExitPosition(request) => {
                            self.force_exit_position(request.priority).await;
                        },
                        Command::TakeProfit(request) => {
                            self.take_profit(request.priority, request.sell_percentage).await;
                        },
                    }
                    //self.event_q.push_back(simulation_event);
                }
                Ok(simulation_event) = self.simulator.simulation_rx.recv() => {
                    self.event_q.push_back(simulation_event);
                    
                    // Strategy will generate signal, store it in event_Q
                    // event_q.push_back(SimulationFullfilledEvent)

                    // strategy.generate_signal(simulation) -> 
                }
                
            }
            

            // Process event_q
        }
    }

}

pub struct TraderBuilder<EventTx, Portfolio>
where
    EventTx: MessageTransmitter<Event>,
    Portfolio: OrderGenerator + StatisticsCalculator + StrategyGenerator + ProfileUpdater + TransactionEventUpdater + Send,
{
    trader_id: Option<TraderId>,
    command_rx: Option<mpsc::Receiver<Command>>,
    event_tx: Option<EventTx>,
    simulator: Option<SimulatorHandle>,
    portfolio: Option<Portfolio>,
    executor_tx: Option<mpsc::Sender<OrderEventWithResponse>>,
}

impl<EventTx, Portfolio> TraderBuilder<EventTx, Portfolio>
where
    EventTx: MessageTransmitter<Event> + Clone,
    Portfolio: OrderGenerator + StatisticsCalculator + StrategyGenerator + ProfileUpdater + TransactionEventUpdater + Send,
{
    
    pub fn new() -> Self {
        Self {
            trader_id: None,
            command_rx: None,
            event_tx: None,
            simulator: None,
            portfolio: None,
            executor_tx: None,
        }
    }
    pub fn trader_id(self, value: TraderId) -> Self {
        Self {
            trader_id: Some(value),
            ..self
        }
    }

    pub fn command_rx(self, value: mpsc::Receiver<Command>) -> Self {
        Self {
            command_rx: Some(value),
            ..self
        }
    }

    pub fn event_tx(self, value: EventTx) -> Self {
        Self {
            event_tx: Some(value),
            ..self
        }
    }

    pub fn simulator_rx(self, value: SimulatorHandle) -> Self {
        Self {
            simulator: Some(value),
            ..self
        }
    }

    pub fn portfolio(self, value: Portfolio) -> Self {
        Self {
            portfolio: Some(value),
            ..self
        }
    }

    pub fn executor_tx(self, value: mpsc::Sender<OrderEventWithResponse>) -> Self {
        Self {
            executor_tx: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<Trader<EventTx, Portfolio>, EngineError> {
        Ok(Trader {
            trader_id: self
                .trader_id
                .ok_or(EngineError::BuilderIncomplete("trader_id"))?,
            command_rx: self
                .command_rx
                .ok_or(EngineError::BuilderIncomplete("command_rx"))?,
            event_tx: self
                .event_tx
                .ok_or(EngineError::BuilderIncomplete("event_tx"))?,           
            simulator: self
                .simulator
                .ok_or(EngineError::BuilderIncomplete("simulator"))?, 
            portfolio: self
                .portfolio
                .ok_or(EngineError::BuilderIncomplete("portfolio"))?,  
            executor_tx: self
                .executor_tx
                .ok_or(EngineError::BuilderIncomplete("executor_tx"))?,   
            event_q: VecDeque::with_capacity(10),
        })
    }
}