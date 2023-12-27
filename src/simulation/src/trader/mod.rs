use crate::{
    token::Token,
    event::{Event, MessageTransmitter},
    simulator::SimulatorHandle,
    portfolio::{
        portfolio::Portfolio as PortfolioBuilder,
        profile::{
            Profile,
        },
        repository::{
            PositionHandler,
            ProfileHandler
        },
    },
    types::{TraderId, IdGenerator, interface, deserialize_trader_id, serialize_trader_id},
    executor::{OrderEventWithResponse},
};
use dashmap::{DashMap, mapref};
use std::{
    marker::{
        Sync
    },
    sync::Arc,
};
use serde::{Deserialize, Serialize};
use parking_lot::Mutex;
use tokio::{sync::{mpsc}};
use tokio;
use ethers::prelude::{Address};

pub mod error;
mod trader;
use trader::{
    Trader,
    Command as TraderCommand,
};

#[derive(Debug)]
pub enum Command {
    TerminateTrader(TraderId),
    CreateTrader(SimulatorHandle, Profile, mpsc::Sender<interface::TraderCreatedResponseInterface>),
    UpdateTraderAntiRug(interface::UpdateAntiRugInterface),
    ForceExitPosition(interface::ForceExitPositionInterface),
    TakeProfit(interface::TakeProfitInterface),
}


#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraderCreated {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
    pub profile: Profile
} 



#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraderTerminated {
    #[serde(deserialize_with = "deserialize_trader_id", serialize_with = "serialize_trader_id")]
    pub trader_id: TraderId,
} 


pub struct TraderEngineLego<EventTx, Repository> 
where 
    EventTx: MessageTransmitter<Event> + Send,    
    Repository: PositionHandler + ProfileHandler + Send,

{    
    pub command_rx: mpsc::Receiver<Command>,
    pub event_tx: EventTx,
    pub repository: Arc<Mutex<Repository>>,
    pub executor_tx: mpsc::Sender<OrderEventWithResponse>,
    pub token_pool: Arc<DashMap<Address, Token>>,
}

pub struct TraderEngine<EventTx, Repository, TraderId>
where
    EventTx: MessageTransmitter<Event> + Clone,
    Repository: PositionHandler + ProfileHandler + Send + 'static,
    TraderId: IdGenerator 
{
    command_rx: mpsc::Receiver<Command>,
    traders: DashMap<TraderId, mpsc::Sender<TraderCommand>>,
    event_tx: EventTx,
    repository: Arc<Mutex<Repository>>,
    executor_tx: mpsc::Sender<OrderEventWithResponse>,
    token_pool: Arc<DashMap<Address, Token>>,
 
}

impl<EventTx, Repository, > TraderEngine<EventTx, Repository, TraderId>
where
    EventTx: MessageTransmitter<Event> + Send + Clone + Sync + 'static,
    Repository: PositionHandler + ProfileHandler + Send + 'static,   
    TraderId: IdGenerator 
{

    pub fn new(lego: TraderEngineLego<EventTx, Repository>) -> Self {
        let traders = DashMap::new();
        Self {
            command_rx: lego.command_rx,
            event_tx: lego.event_tx,
            repository: lego.repository,
            executor_tx: lego.executor_tx,
            token_pool: lego.token_pool,
            traders,
          
        }
    }

    pub fn builder() -> TraderEngineBuilder<EventTx, Repository> {
        TraderEngineBuilder::new()
    }
  
    pub async fn run(mut self) {
        loop {              
            
            tokio::select! {                    
                command = self.command_rx.recv() => {
                    if let Some(command) = command {
                        match command {
                            Command::CreateTrader(handle, profile, response) => {
                                let trader_id = self.create_trader(handle, profile).await;                                
                                match response.send(interface::TraderCreatedResponseInterface { trader_id }).await {
                                    Ok(_) => {},
                                    Err(e) => {
                                        log::error!( "{}", format!("Failed to send response: {:?}", e));
                                    }
                                };
                            },
                            Command::UpdateTraderAntiRug(anti_rug) => {
                                self.update_antirug(anti_rug).await;
                            },
                            Command::TerminateTrader(trader_id) => {
                                self.terminate_trader(trader_id).await;
                            },
                            Command::ForceExitPosition(request) => {
                                self.force_exit_position(request).await;
                            },
                            Command::TakeProfit(request) => {
                                self.take_profit(request).await;
                            }
                        }
                    } else {
                        println!("command channel dropped!");
                        break;
                    }
                }
            }
        }
    }
    async fn update_antirug(&self, anti_rug: interface::UpdateAntiRugInterface) {
        let trader_id = anti_rug.trader_id.clone();
        match self.traders.entry(trader_id.clone()) {
            mapref::entry::Entry::Occupied(entry) => {
                let command_tx = entry.get();

                command_tx.send(TraderCommand::UpdateAntiRug(anti_rug)).await;
            },
            mapref::entry::Entry::Vacant(entry) => {
                log::warn!(
                    "{}", format!("Trader {:?} does not exists", trader_id)
                );
            }
        }
    }

    async fn force_exit_position(&self, request: interface::ForceExitPositionInterface) {
        let trader_id = request.trader_id.clone();
        match self.traders.entry(trader_id.clone()) {
            mapref::entry::Entry::Occupied(entry) => {
                let command_tx = entry.get();

                command_tx.send(TraderCommand::ForceExitPosition(request)).await;
            },
            mapref::entry::Entry::Vacant(entry) => {
                log::warn!(
                    "{}", format!("Trader {:?} does not exists", trader_id)
                );
            }
        }
    }

    async fn take_profit(&self, request: interface::TakeProfitInterface) {
        let trader_id = request.trader_id.clone();
        match self.traders.entry(trader_id.clone()) {
            mapref::entry::Entry::Occupied(entry) => {
                let command_tx = entry.get();

                command_tx.send(TraderCommand::TakeProfit(request)).await;
            },
            mapref::entry::Entry::Vacant(entry) => {
                log::warn!(
                    "{}", format!("Trader {:?} does not exists", trader_id)
                );
            }
        }
    }

    async fn terminate_trader(&mut self, trader_id: TraderId) {
        match self.traders.entry(trader_id.clone()) {
            mapref::entry::Entry::Occupied(entry) => {
                let command_tx = entry.get();

                command_tx.send(TraderCommand::Terminate).await;
            },
            mapref::entry::Entry::Vacant(entry) => {

            }
        }
    }

    async fn update_trader(&mut self, trader_id: TraderId, profile: &Profile) {
        let command_tx = self.traders.get(&trader_id).unwrap();
    }

    async fn create_trader(&mut self, handle: SimulatorHandle, profile: Profile) -> TraderId {        
        let trader_id = TraderId::determine_id(&profile.user_id, &profile.token);
        
        //let token = Token::create(token_address, &self.dexes, client.clone()).await;
        
        let result = match self.traders.entry(trader_id.clone()) {
            mapref::entry::Entry::Occupied(entry) => {
                //let m = entry.get();
                //handle::SimulationHandle::new(m.0.subscribe(), m.1.clone())
                // TODO: do nothing?
            },
            mapref::entry::Entry::Vacant(entry) => {
                // Buffer for simulation requests can be sent from traders
                let (trader_command_tx, trader_command_rx) = mpsc::channel(10);
                // Store broadcast TX and the request RX
                entry.insert(trader_command_tx);

                let portfolio = PortfolioBuilder::builder()
                    .repository(self.repository.clone())
                    .initial_profile(profile.clone())
                    .token_pool(self.token_pool.clone())
                    .token_id(trader_id.token_id)
                    .build()
                    .expect("Portfolio cannot be built");
                
                let trader = Trader::builder()
                    .trader_id(trader_id.clone())
                    .event_tx(self.event_tx.clone())
                    .command_rx(trader_command_rx)
                    .executor_tx(self.executor_tx.clone())
                    .portfolio(portfolio)
                    .simulator_rx(handle)
                    .build()
                    .expect("failed to build & initialise Simulator");

                self.event_tx.send(Event::TraderCreated(
                    TraderCreated { 
                        trader_id: trader_id.clone(),
                        profile
                    }
                ));
                
                let trader_handle = tokio::spawn(trader.run());
                let map = self.traders.clone();

                let trader_id = trader_id.clone();
                tokio::spawn(async move {
                    trader_handle.await;
                    map.remove(&trader_id);
                    let traders_left = map.len();
                    log::info!(
                        "{}", format!("Trader {:?} removed from map, {:?} left", trader_id.to_string(), traders_left)
                    );
                });

                
            }
        };
        trader_id
        //respond_to.try_send(result);
         
    }

}
pub struct TraderEngineBuilder<EventTx, Repository>
where
    EventTx: MessageTransmitter<Event>,
    Repository: PositionHandler + ProfileHandler + Send,
    TraderId: IdGenerator,
{
    command_rx: Option<mpsc::Receiver<Command>>,
    event_tx: Option<EventTx>,
    repository: Option<Arc<Mutex<Repository>>>,
    executor_tx: Option<mpsc::Sender<OrderEventWithResponse>>,
    token_pool: Option<Arc<DashMap<Address, Token>>>,
 
}

impl<EventTx, Repository, > TraderEngineBuilder<EventTx, Repository>
where
    EventTx: MessageTransmitter<Event> + Clone,
    Repository: PositionHandler + ProfileHandler + Send,
    TraderId: IdGenerator,
{
    
    pub fn new() -> Self {
        Self {
            command_rx: None,
            event_tx: None,
            repository: None,
            executor_tx: None,
            token_pool: None,
           
        }
    }

    pub fn command_rx(self, value: mpsc::Receiver<Command>) -> Self {
        Self {
            command_rx: Some(value),
            ..self
        }
    }

    pub fn repository(self, value: Arc<Mutex<Repository>>) -> Self {
        Self {
            repository: Some(value),
            ..self
        }
    }

    pub fn token_pool(self, value: Arc<DashMap<Address, Token>>) -> Self {
        Self {
            token_pool: Some(value),
            ..self
        }
    }

    pub fn executor_tx(self, value: mpsc::Sender<OrderEventWithResponse>) -> Self {
        Self {
            executor_tx: Some(value),
            ..self
        }
    }


    pub fn event_tx(self, value: EventTx) -> Self {
        Self {
            event_tx: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<TraderEngine<EventTx, Repository, TraderId>, error::EngineError> {        
        let traders = DashMap::new();
        Ok(TraderEngine {            
            command_rx: self
                .command_rx
                .ok_or(error::EngineError::BuilderIncomplete("command_rx"))?,            
            event_tx: self
                .event_tx
                .ok_or(error::EngineError::BuilderIncomplete("event_tx"))?,  
            token_pool: self
                .token_pool
                .ok_or(error::EngineError::BuilderIncomplete("token_pool"))?,  
                repository: self
            .repository
                .ok_or(error::EngineError::BuilderIncomplete("repository"))?, 
            executor_tx: self
                .executor_tx
                .ok_or(error::EngineError::BuilderIncomplete("executor_tx"))?,  
            traders,
          
        })
    }
}