use tokio::sync::mpsc::{channel as oneshot_channel, Receiver, Sender};
use ethers::prelude::{Address};
use eyre::Result;

//pub mod feed;
pub mod utils;
pub mod abi;
pub mod dex;
pub mod event;
pub mod stream;
pub mod simulator;
pub mod trader;
pub mod token;
pub mod portfolio;
pub mod executor;
pub mod types;

pub mod prelude {
    pub use super::{
        dex::{
            Dex,
            PoolVariant,
            Pool
        },
        stream::{
            stream_block_notification,
            stream_pending_transaction,
        },
        portfolio::{
            portfolio::{
                Portfolio
            },
            profile::{
                Profile,
                AntiRug
            },
            repository,
        },
        executor::{
            Executor,
        },
        simulator::{
            SimulatorEngine,
            Command as SimulatorCommand
        },
        trader::{
            TraderEngine,
            Command as TraderCommand,
        },
        event::{
            Event,
            EventTx
        },
        types::{TraderId, interface},
    };
}

pub async fn add_token(cmd: &Sender<simulator::Command>, token: Address) -> Result<Receiver<simulator::SimulatorHandle>> {
    let (request, response) = oneshot_channel(1);
    cmd.send(simulator::Command::AddToken(token, request)).await?;
    Ok(response)
}
