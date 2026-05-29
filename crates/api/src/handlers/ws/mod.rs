pub mod dto;
pub mod handlers;

pub use handlers::ws_handler;

use dashmap::DashMap;
use domain::call::CallService;
use infrastructure::repositories::chat::PostgresChatRepository;
use infrastructure::repositories::user::PostgresUserRepository;
use redis::aio::ConnectionManager;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

pub type WsConnection = Arc<Mutex<tokio::sync::mpsc::Sender<String>>>;
pub type WsConnections = DashMap<Uuid, Vec<WsConnection>>;

pub struct WsState {
    pub connections: Arc<WsConnections>,
    pub redis: ConnectionManager,
    pub redis_url: String,
    pub user_repo: Arc<PostgresUserRepository>,
    pub chat_repo: Arc<PostgresChatRepository>,
    /// Orchestrates call business logic from within WebSocket handlers.
    pub call_service: Arc<CallService>,
}
