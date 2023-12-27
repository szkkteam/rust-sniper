use crate::{
    portfolio::{
        position::{
            Position,
        },
        profile::{
            Profile,
        }
    },
    types::{
        PositionId,
        ProfileId
    }
};
use eyre::Result;

pub mod error;
pub mod redis;

pub trait PositionHandler {

    fn set_open_position(&mut self, position: Position) -> Result<(), error::RepositoryError>;

    fn get_open_position(&mut self, position_id: &PositionId) -> Result<Option<Position>, error::RepositoryError>;

    fn remove_position(&mut self, position_id: &PositionId) -> Result<Option<Position>, error::RepositoryError>;

    fn set_exit_position(&mut self, position_id: &PositionId) -> Result<(), error::RepositoryError>;
}

pub trait ProfileHandler {

    fn update_profile(&mut self, profile: Profile) -> Result<(), error::RepositoryError>;

    fn get_profile(&mut self, profile_id: &ProfileId) -> Result<Option<Profile>, error::RepositoryError>;
    /*
    fn set_order(&mut self);

    fn update_order(&mut self);

    fn delete_order(&mut self);
     */
}

