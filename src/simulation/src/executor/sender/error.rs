use thiserror::Error;
use ethers::prelude::{
    ProviderError,
    WalletError
};
/// All errors generated in barter-engine.
#[derive(Error, Debug)]
pub enum SenderError {
    #[error("Constructed bundle is incomplete: {0}")]
    BundleIncomplete(&'static str),

    #[error("Failed to interact with provider")]
    ProviderIteractionError(#[from] ProviderError),

    #[error("Failed to interact with wallet")]
    WalletIteractionError(#[from] WalletError),
}
