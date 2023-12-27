use super::repository::error::RepositoryError;
use thiserror::Error;

/// All errors generated in the barter::portfolio module.
#[derive(Error, Debug)]
pub enum PortfolioError {
    #[error("Failed to build struct due to missing attributes: {0}")]
    BuilderIncomplete(&'static str),

    #[error("Profile for trader does not exists")]
    ProfileNotExists,

    #[error("Failed to parse Position entry Side due to ambiguous fill quantity & Decision.")]
    ParseEntrySide,

    #[error("Cannot exit Position with an entry decision FillEvent.")]
    CannotEnterPositionWithExitFill,

    #[error("Cannot exit Position with an entry decision FillEvent.")]
    CannotExitPositionWithEntryFill,

    #[error("Cannot generate PositionExit from Position that has not been exited")]
    PositionExit,

    #[error("Failed to generate order due to: {0}")]
    EntryOrderGeneration(&'static str),

    #[error("Failed to interact with repository")]
    RepositoryInteraction(#[from] RepositoryError),
}
