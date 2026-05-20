//! WebSocket based events.
//!
//! Users can connect to a server room, which streams events directly from that
//! server into websockets! The future is NOW.

pub mod protocol;

pub use protocol::{Error, WebSocket};
pub use ring_channel_model::message::Message;

use derive_more::Deref;

use std::sync::Arc;

use futures_util::SinkExt as _;

use ring_channel_model::{
    Battle, BattleWager,
    battle::Participant,
    chat::Message as ChatMessage,
    message::server::{BattleUpdate, MobiumsChange, NewBattle, NewMessage, WagerUpdate},
};

use tokio::sync::{
    RwLock,
    broadcast::{self, Receiver, Sender, error::RecvError},
};

use tracing::instrument;

use crate::{battle::BattleRow, session::SessionUser};

/// An open room.
///
/// Cheaply cloneable.
///
/// This serves as a master object that can lease handles to new websockets.
#[derive(Clone, Debug)]
pub struct Room {
    state: Arc<RoomState>,
}

#[derive(Debug)]
struct RoomState {
    tx: Sender<RoomEvent>,
    current_battle: RwLock<Option<BattleData>>,
}

/// Internal battle data held by the server.
#[derive(Clone, Debug, Deref)]
pub struct BattleData {
    #[deref]
    pub schema: BattleRow,
    pub participants: Vec<Participant>,
}

impl From<BattleData> for Battle {
    fn from(value: BattleData) -> Self {
        let mut battle = Battle::from(value.schema);
        battle.participants = value.participants;
        battle
    }
}

impl From<&BattleData> for Battle {
    fn from(value: &BattleData) -> Self {
        let mut battle = Battle::from(&value.schema);
        battle.participants = value.participants.clone();
        battle
    }
}

impl Room {
    /// Creates a new `Room`.
    pub fn new() -> Room {
        let (tx, _rx) = broadcast::channel(16);

        Room {
            state: Arc::new(RoomState {
                tx,
                current_battle: RwLock::default(),
            }),
        }
    }

    /// Sends a new message in the room.
    pub async fn send_message(&self, message: ChatMessage) {
        let _ = self.state.tx.send(RoomEvent::NewMessage { message });
    }

    /// Sets a new match for the room, broadcasting it to all clients.
    pub async fn update_battle(&self, new_battle: BattleData) {
        *self.state.current_battle.write().await = Some(new_battle.clone());
        let _ = self
            .state
            .tx
            .send(RoomEvent::UpdateBattle { battle: new_battle });
    }

    /// Updates users with a wager change.
    pub fn send_wager_update(&self, wager: BattleWager) {
        let _ = self.state.tx.send(RoomEvent::WagerUpdate { wager });
    }

    /// Notifies a connected client of mobiums loss (or gain).
    pub fn send_mobiums_change(&self, user_id: i32, change: MobiumsChange) {
        let _ = self.state.tx.send(RoomEvent::MobiumsChange {
            user_id,
            message: change,
        });
    }

    /// Serves a new client, with additional authentication information.
    ///
    /// **This commandeers the calling task!**
    pub async fn serve(self, ws: axum::extract::ws::WebSocket, user: Option<SessionUser>) {
        let battle = self.state.current_battle.read().await.clone();

        tracing::debug!(?battle, "serving new client");

        serve(WebSocketState {
            ws: ws.into(),
            handle: self.get_handle(),
            user,
            battle,
        })
        .await;
    }

    fn get_handle(&self) -> Handle {
        Handle {
            rx: self.state.tx.subscribe(),
        }
    }
}

/// A handle to a room.
#[derive(Debug)]
pub struct Handle {
    rx: Receiver<RoomEvent>,
}

#[derive(Debug, Clone)]
enum RoomEvent {
    NewMessage {
        message: ChatMessage,
    },
    UpdateBattle {
        battle: BattleData,
    },
    WagerUpdate {
        wager: BattleWager,
    },
    MobiumsChange {
        user_id: i32,
        message: MobiumsChange,
    },
}

#[allow(dead_code)]
struct WebSocketState {
    // Connection details
    ws: WebSocket,
    handle: Handle,

    // Authentication
    user: Option<SessionUser>,

    // Room state things
    battle: Option<BattleData>,
}

/// Serves a websocket.
async fn serve(mut state: WebSocketState) {
    // Give client the rundown on what's happening
    if let Some(battle) = state.battle.as_ref() {
        let _ = state.ws.send(&NewBattle(battle.into()).into()).await;
    }

    while !state.ws.is_closed() {
        let WebSocketState { ws, handle, .. } = &mut state;

        tokio::select! {
            ev = ws.recv() => {
                tracing::trace!(?ev, "got client msg");
                match ev {
                    Some(Ok(msg)) => {
                        if let Err(err) = handle_message(&mut state, msg).await {
                            tracing::error!("ws error: {}", err);
                        }
                    }
                    // a fatal transfer error occured
                    Some(Err(err)) => {
                        tracing::error!("error receiving message: {}", err);
                        let _ = ws.close().await;
                        break;
                    }
                    // the websocket is closed!
                    None => break,
                }
            }
            ev = handle.rx.recv() => {
                tracing::trace!(?ev, "got server event");
                match ev {
                    Ok(event) => {
                        if let Err(err) = handle_server_event(&mut state, event).await {
                            tracing::error!("ws error: {}", err);
                        }
                    }
                    // Lagged errors are fine
                    Err(RecvError::Lagged(err)) => {
                        tracing::warn!("ws lagged: {}", err);
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    // the websocket closes when it falls out of scope
}

/// Handles a message from the client.
#[instrument(skip(_state))]
async fn handle_message(_state: &mut WebSocketState, message: Message) -> Result<(), Error> {
    match message {
        // lol
        _ => (),
    }

    Ok(())
}

/// Handles an internal server event.
#[instrument(skip(state))]
async fn handle_server_event(state: &mut WebSocketState, ev: RoomEvent) -> Result<(), Error> {
    match ev {
        RoomEvent::NewMessage { message } => {
            state.ws.send(&NewMessage(message).into()).await?;
        }
        RoomEvent::UpdateBattle { battle } => {
            let old_battle = std::mem::replace(&mut state.battle, Some(battle.clone()));

            // A new match was started, or updated
            // Check if the match we have is the same
            if old_battle.as_ref().map(|b| &b.uuid) != Some(&battle.uuid) {
                // This is a new battle!
                state.ws.send(&NewBattle(battle.into()).into()).await?;
            } else {
                // This is the same battle, it just got updated
                state.ws.send(&BattleUpdate(battle.into()).into()).await?;
            }
        }
        RoomEvent::WagerUpdate { wager } => {
            state.ws.send(&WagerUpdate(wager).into()).await?;
        }
        RoomEvent::MobiumsChange { user_id, message }
            if Some(user_id) == state.user.as_ref().map(|u| u.identity()) =>
        {
            state.ws.send(&message.into()).await?;
        }
        _ => (),
    }

    Ok(())
}
