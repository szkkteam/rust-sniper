use super::{
    PositionHandler,
    ProfileHandler,
    error::RepositoryError,
};
use eyre::Result;
use crate::{
    portfolio::{
        error::PortfolioError,
        position::{
            Position,
        },
        profile::{
            Profile,
        },
    },
    types::{
        PositionId,
        ProfileId,
        IdGenerator,
    }
};
use redis::{Commands, Client};
use r2d2::{Pool, PooledConnection};


#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Default)]
pub struct Config {
    pub uri: String,
}

pub struct RedisRepository 
{
    pool: Pool<Client>,
}

impl PositionHandler for RedisRepository
{
    fn get_open_position(
        &mut self,
        position_id: &PositionId
    ) -> Result<Option<Position>, RepositoryError> {
        let value: Option<String> = self
            .get_connection()
            .get(position_id.to_string())
            .map_err(|_| RepositoryError::ReadError )?;
        Ok(match value {
            Some(value) => Some(serde_json::from_str::<Position>(&value)?),
            None => None
        })
    }

    fn set_open_position(
        &mut self,
        position: Position
    ) -> Result<(), RepositoryError> {
        let value_string = serde_json::to_string(&position)?;

        self
            .get_connection()
            .set(position.position_id.to_string(), value_string)
            .map_err(|_| RepositoryError::WriteError)
    }

    fn remove_position(
        &mut self,
        position_id: &PositionId
    ) -> Result<Option<Position>, RepositoryError> {
        let position = self.get_open_position(position_id)?;

        self
            .get_connection()
            .del(position_id.to_string())
            .map_err(|_| RepositoryError::DeleteError)?;
        
        Ok(position)
    }

    fn set_exit_position(
        &mut self,
        position_id: &PositionId
    ) -> Result<(), RepositoryError> {
        Err(RepositoryError::WriteError)
    }
}

impl ProfileHandler for RedisRepository
{
    
   fn update_profile(&mut self, profile: Profile) -> Result<(), RepositoryError> {    
        let profile_id = ProfileId::determine_id(
            &profile.user_id,
            &profile.token
        ); 
        let value_string = serde_json::to_string(&profile)?;
        //println!("save --profile_id: {:?} value: {:?}", profile_id, profile);
        self
            .get_connection()
            .set(profile_id.to_string(), value_string)
            .map_err(|_| RepositoryError::WriteError)
    }

    fn get_profile(&mut self, profile_id: &ProfileId) -> Result<Option<Profile>, RepositoryError> {
        let value: Option<String> = self
            .get_connection()
            .get(profile_id.to_string())
            .map_err(|_| RepositoryError::ReadError)?;
        //println!("load --profile_id: {:?} value: {:?}", profile_id, value);
        Ok(match value {
            Some(value) => Some(serde_json::from_str::<Profile>(&value)?),
            None => None
        })
    }
}

impl RedisRepository 
{
    /// Constructs a new [`RedisRepository`] component using the provided Redis connection struct.
    pub fn new(pool: Pool<Client>) -> Self {
        Self {
            pool: pool,
        }
    }

    /// Returns a [`RedisRepositoryBuilder`] instance.
    pub fn builder() -> RedisRepositoryBuilder {
        RedisRepositoryBuilder::new()
    }

    fn get_connection(&self) -> PooledConnection<Client> {
        self.pool.get().unwrap()
    }

    /// Establish & return a Redis connection.
    pub fn setup_redis_connection(uri: &str) -> Pool<Client> {
        Pool::builder().build(
            redis::Client::open(uri)
            .expect("Failed to create Redis client")
        )
        .expect("Failed to connect to Redis")
        
    }
}

#[derive(Default)]
pub struct RedisRepositoryBuilder
{
    pool: Option<Pool<Client>>,
}

impl RedisRepositoryBuilder
{
    pub fn new() -> Self {
        Self {
            pool: None,
        }
    }

    pub fn pool(self, value: Pool<Client>) -> Self {
        Self {
            pool: Some(value),
            ..self
        }
    }

    pub fn build(self) -> Result<RedisRepository, PortfolioError> {
        Ok(RedisRepository {
            pool: self.pool.ok_or(PortfolioError::BuilderIncomplete("pool"))?,
        })
    }
}
