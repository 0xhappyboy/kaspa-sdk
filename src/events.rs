use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

use crate::types::{
    Block, KaspaError, Result, Transaction, UtxosChangedNotification,
    VirtualChainChangedNotification,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum KaspaEvent {
    BlockAdded(BlockAddedEvent),
    VirtualChainChanged(VirtualChainChangedNotification),
    UtxosChanged(UtxosChangedNotification),
    FinalityConflict(FinalityConflictEvent),
    NewTransaction(NewTransactionEvent),
    SyncStateChanged(SyncStateChangedEvent),
    PeerStateChanged(PeerStateChangedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockAddedEvent {
    pub block: Block,
    pub block_hash: String,
    pub daa_score: u64,
    pub blue_score: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalityConflictEvent {
    pub violating_block_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTransactionEvent {
    pub transaction: Transaction,
    pub transaction_id: String,
    pub is_orphan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStateChangedEvent {
    pub is_synced: bool,
    pub current_block_count: u64,
    pub header_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStateChangedEvent {
    pub peer_count: usize,
    pub connected_peers: Vec<String>,
    pub disconnected_peers: Vec<String>,
}

/// event message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMessage {
    pub jsonrpc: String,
    pub method: String,
    pub params: EventParams,
}

/// event params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventParams {
    pub result: serde_json::Value,
}

/// event listener
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: &KaspaEvent);
}

#[derive(Debug, Clone)]
pub struct EventListenerConfig {
    pub ws_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub reconnect_interval: std::time::Duration,
    pub max_reconnect_attempts: usize,
}

impl Default for EventListenerConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://localhost:16110".to_string(),
            username: None,
            password: None,
            reconnect_interval: std::time::Duration::from_secs(5),
            max_reconnect_attempts: 10,
        }
    }
}

pub struct KaspaEventListener {
    config: EventListenerConfig,
    event_sender: broadcast::Sender<KaspaEvent>,
    event_handlers: Arc<Mutex<Vec<Box<dyn Fn(KaspaEvent) + Send + Sync>>>>,
    subscribed_addresses: Arc<Mutex<HashSet<String>>>,
    is_running: Arc<Mutex<bool>>,
}

impl KaspaEventListener {
    /// create a new event
    ///
    /// # Example
    /// ```
    /// let listener = KaspaEventListener::new();
    /// ```
    pub fn new() -> Self {
        let config = EventListenerConfig::default();
        let (event_sender, _) = broadcast::channel(100);
        Self {
            config,
            event_sender,
            event_handlers: Arc::new(Mutex::new(Vec::new())),
            subscribed_addresses: Arc::new(Mutex::new(HashSet::new())),
            is_running: Arc::new(Mutex::new(false)),
        }
    }

    /// creates a new event listener from config
    ///
    /// # Example
    /// ```
    /// let config = EventListenerConfig {
    ///     ws_url: "ws://.....".to_string(),
    ///     ..Default::default()
    /// };
    /// let listener = KaspaEventListener::from_config(config);
    /// ```
    pub fn from_config(config: EventListenerConfig) -> Self {
        let (event_sender, _) = broadcast::channel(100);
        Self {
            config,
            event_sender,
            event_handlers: Arc::new(Mutex::new(Vec::new())),
            subscribed_addresses: Arc::new(Mutex::new(HashSet::new())),
            is_running: Arc::new(Mutex::new(false)),
        }
    }

    /// subscribe to receive events through broadcast channel
    ///
    /// # Example
    /// ```
    /// let mut receiver = listener.subscribe();
    /// tokio::spawn(async move {
    ///     while let Ok(event) = receiver.recv().await {
    ///     }
    /// });
    /// ```
    pub fn subscribe(&self) -> broadcast::Receiver<KaspaEvent> {
        self.event_sender.subscribe()
    }

    /// add an event handling closure.
    ///
    /// # Example
    /// ```
    /// listener.add_event_handler(Box::new(|event| {
    ///     match event {
    ///         KaspaEvent::BlockAdded(block_event) => {
    ///            // ...
    ///         }
    ///         _ => {}
    ///     }
    /// })).await;
    /// ```
    ///
    pub async fn add_event_handler(&self, handler: Box<dyn Fn(KaspaEvent) + Send + Sync>) {
        let mut handlers = self.event_handlers.lock().await;
        handlers.push(handler);
    }

    /// start listening for events
    ///
    /// # Example
    /// ```
    /// listener.start().await?;
    /// ```
    pub async fn start(&self) -> Result<()> {
        let mut running = self.is_running.lock().await;
        if *running {
            return Err(KaspaError::Custom(
                "EventListener is already running".to_string(),
            ));
        }
        *running = true;
        drop(running);
        let config = self.config.clone();
        let event_sender = self.event_sender.clone();
        let event_handlers = self.event_handlers.clone();
        let subscribed_addresses = self.subscribed_addresses.clone();
        let is_running = self.is_running.clone();
        tokio::spawn(async move {
            Self::run_event_loop(
                config,
                event_sender,
                event_handlers,
                subscribed_addresses,
                is_running,
            )
            .await;
        });
        Ok(())
    }

    /// stop listening for events
    ///
    /// # Example
    /// ```
    /// listener.stop().await;
    /// ```
    pub async fn stop(&self) {
        let mut running = self.is_running.lock().await;
        *running = false;
    }

    /// run event loop
    async fn run_event_loop(
        config: EventListenerConfig,
        event_sender: broadcast::Sender<KaspaEvent>,
        event_handlers: Arc<Mutex<Vec<Box<dyn Fn(KaspaEvent) + Send + Sync>>>>,
        subscribed_addresses: Arc<Mutex<HashSet<String>>>,
        is_running: Arc<Mutex<bool>>,
    ) {
        let mut attempt = 0;
        while *is_running.lock().await && attempt < config.max_reconnect_attempts {
            match connect_async(&config.ws_url).await {
                Ok((ws_stream, _)) => {
                    println!("Connected to Kaspa WebSocket");
                    attempt = 0; // Reset attempt counter on successful connection
                    let (mut write, mut read) = ws_stream.split();
                    // Subscribe to events
                    if let Err(e) =
                        Self::subscribe_to_events(&mut write, &subscribed_addresses).await
                    {
                        println!("Failed to subscribe to events: {}", e);
                        continue;
                    }
                    // Event processing loop
                    while *is_running.lock().await {
                        match read.next().await {
                            Some(Ok(message)) => {
                                if let Err(e) =
                                    Self::handle_message(message, &event_sender, &event_handlers)
                                        .await
                                {
                                    println!("Error processing message: {}", e);
                                }
                            }
                            Some(Err(e)) => {
                                println!("WebSocket error: {}", e);
                                break;
                            }
                            None => break,
                        }
                    }
                }
                Err(e) => {
                    println!(
                        "Failed to connect to WebSocket (attempt {}): {}",
                        attempt + 1,
                        e
                    );
                    attempt += 1;
                    tokio::time::sleep(config.reconnect_interval).await;
                }
            }
        }
    }

    /// subscribe to events
    async fn subscribe_to_events(
        write: &mut futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            Message,
        >,
        subscribed_addresses: &Arc<Mutex<HashSet<String>>>,
    ) -> Result<()> {
        // subscribe to the latest trading events
        let tx_subscription = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "notifyNewTransaction",
            "params": [{
                "includeTransactionDetails": true
            }]
        });
        write
            .send(Message::Text(tx_subscription.to_string()))
            .await
            .map_err(|e| {
                KaspaError::Custom(format!("Failed to subscribe to new transactions: {}", e))
            })?;
        //sSubscribe to block added events
        let block_subscription = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "notifyBlockAdded",
            "params": [""]
        });
        write
            .send(Message::Text(block_subscription.to_string()))
            .await
            .map_err(|e| KaspaError::Custom(format!("Failed to subscribe to blocks: {}", e)))?;
        // Subscribe to virtual chain changed events
        let vc_subscription = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "notifyVirtualChainChanged",
            "params": [{
                "includeAcceptedTransactionIds": true
            }]
        });
        write
            .send(Message::Text(vc_subscription.to_string()))
            .await
            .map_err(|e| {
                KaspaError::Custom(format!("Failed to subscribe to virtual chain: {}", e))
            })?;
        // Subscribe to UTXO changes for addresses
        let addresses = subscribed_addresses.lock().await;
        if !addresses.is_empty() {
            let utxo_subscription = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "notifyUtxosChanged",
                "params": [{
                    "addresses": addresses.iter().collect::<Vec<&String>>()
                }]
            });
            write
                .send(Message::Text(utxo_subscription.to_string()))
                .await
                .map_err(|e| KaspaError::Custom(format!("Failed to subscribe to UTXOs: {}", e)))?;
        }
        Ok(())
    }

    /// handle message
    async fn handle_message(
        message: Message,
        event_sender: &broadcast::Sender<KaspaEvent>,
        event_handlers: &Arc<Mutex<Vec<Box<dyn Fn(KaspaEvent) + Send + Sync>>>>,
    ) -> Result<()> {
        if let Message::Text(text) = message {
            if let Ok(event_message) = serde_json::from_str::<EventMessage>(&text) {
                let event = Self::parse_event(&event_message.method, event_message.params.result)?;
                // Send to broadcast channel
                let _ = event_sender.send(event.clone());
                // Call registered handlers
                let handlers = event_handlers.lock().await;
                for handler in handlers.iter() {
                    handler(event.clone());
                }
            }
        }
        Ok(())
    }

    /// parse event
    fn parse_event(method: &str, params: Value) -> Result<KaspaEvent> {
        match method {
            "blockAdded" => {
                let block: Block =
                    serde_json::from_value(params).map_err(|e| KaspaError::JsonError(e))?;
                Ok(KaspaEvent::BlockAdded(BlockAddedEvent {
                    block: block.clone(),
                    block_hash: "".to_string(), // Would need to extract from block
                    daa_score: 0,
                    blue_score: 0,
                }))
            }
            // virtual chain changed
            "virtualChainChanged" => {
                let notification: VirtualChainChangedNotification =
                    serde_json::from_value(params).map_err(|e| KaspaError::JsonError(e))?;
                Ok(KaspaEvent::VirtualChainChanged(notification))
            }
            // Balance changes
            "utxosChanged" => {
                let notification: UtxosChangedNotification =
                    serde_json::from_value(params).map_err(|e| KaspaError::JsonError(e))?;
                Ok(KaspaEvent::UtxosChanged(notification))
            }
            // new transaction
            "newTransaction" => {
                let tx_event: NewTransactionEvent =
                    serde_json::from_value(params).map_err(|e| KaspaError::JsonError(e))?;
                Ok(KaspaEvent::NewTransaction(tx_event))
            }
            "finalityConflict" => Ok(KaspaEvent::FinalityConflict(FinalityConflictEvent {
                violating_block_hash: "".to_string(),
            })),
            _ => Err(KaspaError::Custom(format!(
                "Unknown event method: {}",
                method
            ))),
        }
    }

    /// subscribe to address
    ///
    /// # Example
    /// ```
    /// listener.subscribe_to_address("kaspa:qpm2...").await?;
    /// ```
    pub async fn subscribe_to_address(&self, address: &str) -> Result<()> {
        let mut addresses = self.subscribed_addresses.lock().await;
        addresses.insert(address.to_string());
        Ok(())
    }

    /// unsubscribe from address
    ///
    /// # Example
    /// ```
    /// listener.unsubscribe_from_address("kaspa:qpm2...").await?;
    /// ```
    pub async fn unsubscribe_from_address(&self, address: &str) -> Result<()> {
        let mut addresses = self.subscribed_addresses.lock().await;
        addresses.remove(address);
        Ok(())
    }
}
