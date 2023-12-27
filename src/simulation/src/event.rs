use crate::{
    simulator::{
        event::TransactionNew,
        event::{
            SimulationEvent,
            SellSimulationEvent,
        },
        //signal::Signal,
    },
    trader::{
        TraderCreated,
        TraderTerminated
    },
    token::Token,
    stream::{
        BlockInfo
    },
    portfolio::{
        OrderEvent,
        position::Position,
        statistics::Statistics,
    },
    executor::TransactionEvent,
    types::TraderId,
};
//use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Display, Formatter, Result};
use tokio::sync::{mpsc, broadcast};
use ethers::prelude::{Address};

#[derive(Clone, Debug)]
pub enum Event {
    
    TransactionEvent(TransactionEvent),
    SimulationEvent(SimulationEvent),
    OrderNew(OrderEvent),
    TransactionNew(TransactionNew),
    BlockConfirmed(BlockInfo),
    PositionNew(Position),
    PositionUpdated(Position),
    PositionExited(Position),

    SellSimulationEvent(SellSimulationEvent),
    BlackListEvent,

    BlockSimulationEvent(SimulationEvent),
    BlockSellSimulationEvent(SellSimulationEvent),

    TraderStatisticsUpdated(Statistics),
    PairUpdatedEvent(Token),
    TraderTerminated(TraderTerminated),
    TraderCreated(TraderCreated),

    SimulationClosed(Address),

}

impl Display for Event {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match &self {
            Self::SimulationEvent(_) => write!(f, "SimulationEvent"),
            Self::BlockConfirmed(_) => write!(f, "BlockConfirmed"),
            Self::TransactionNew(_) => write!(f, "TransactionNew"),
            Self::TransactionEvent(_) => write!(f, "TransactionEvent"),
            Self::OrderNew(_) => write!(f, "OrderNew"),
            Self::TraderStatisticsUpdated(_) => write!(f, "TraderStatisticsUpdated"),      
            Self::PairUpdatedEvent(..) => write!(f, "PairUpdatedEvent"),      
            Self::SellSimulationEvent(..) => write!(f, "SellSimulationEvent"),    
            Self::PositionNew(_) => write!(f, "PositionNew"),            
            Self::PositionUpdated(_) => write!(f, "PositionUpdated"),            
            Self::PositionExited(_) => write!(f, "PositionExited"),      
            Self::TraderTerminated(_) => write!(f, "TraderTerminated"),    
            Self::TraderCreated(_) => write!(f, "TraderCreated"),    
            Self::BlockSellSimulationEvent(_) => write!(f, "BlockSellSimulationEvent"),    
            Self::BlockSimulationEvent(_) => write!(f, "BlockSimulationEvent"),    
             
             
            _ => write!(f, "NotImplemented")
        }
    }
}

/// Message transmitter for sending Barter messages to downstream consumers.
pub trait MessageTransmitter<Message> {
    /// Attempts to send a message to an external message subscriber.
    fn send(&mut self, message: Message);

    /// Attempts to send many messages to an external message subscriber.
    fn send_many(&mut self, messages: Vec<Message>);
}

pub trait MessageSubscriber<Message> {

    fn subsribe(&self) -> broadcast::Receiver<Message>;
}

/// Transmitter for sending Barter [`Event`]s to an external sink. Useful for event-sourcing,
/// real-time dashboards & general monitoring.
#[derive(Debug, Clone)]
pub struct EventTx {
    /// Flag to communicate if the external [`Event`] receiver has been dropped.
    receiver_dropped: bool,
    /// [`Event`] channel transmitter to send [`Event`]s to an external sink.
    event_tx: mpsc::UnboundedSender<Event>,
}

impl MessageTransmitter<Event> for EventTx {
    fn send(&mut self, message: Event) {
        if self.receiver_dropped {
            return;
        }

        if self.event_tx.send(message).is_err() {           
            self.receiver_dropped = true;
        }
    }

    fn send_many(&mut self, messages: Vec<Event>) {
        if self.receiver_dropped {
            return;
        }

        messages.into_iter().for_each(|message| {
            let _ = self.event_tx.send(message);
        })
    }
}

impl EventTx {
    /// Constructs a new [`EventTx`] instance using the provided channel transmitter.
    pub fn new(event_tx: mpsc::UnboundedSender<Event>) -> Self {
        Self {
            receiver_dropped: false,
            event_tx,
        }
    }
}

