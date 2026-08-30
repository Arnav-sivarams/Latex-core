//! Small S3 Yjs/Yrs transport. Room identity is `(workspace_id, file_id)`, never a path.

use axum::extract::ws::{Message, WebSocket};
use blob_store::BlobStore;
use bytes::Bytes;
use core_types::{UserId, WorkspaceId};
use futures_util::{SinkExt, StreamExt};
use persistence::{
    CollaborationAccess, CollaborationAccessMode, CollaborationUpdateInput, V2Repository,
};
use std::{collections::HashMap, io::Cursor, sync::Arc, time::Duration};
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use uuid::Uuid;
use workspace_model::WorkspaceService;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update, updates::decoder::Decode};

use crate::{AppState, PrincipalKind, digest};

pub(crate) const SOURCE_UPDATE: u8 = 0x01;
const FLUSH: u8 = 0x02;
pub(crate) const INITIAL_STATE: u8 = 0x10;
pub(crate) const REMOTE_SOURCE_UPDATE: u8 = 0x11;
const MAX_UPDATE_BYTES: usize = 1024 * 1024;
const BATCH_BYTES: usize = 64 * 1024;
const SNAPSHOT_BYTES: usize = 1024 * 1024;
const SNAPSHOT_UPDATES: usize = 500;

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub struct RoomKey {
    workspace_id: WorkspaceId,
    file_id: Uuid,
}

impl RoomKey {
    pub const fn new(workspace_id: WorkspaceId, file_id: Uuid) -> Self {
        Self {
            workspace_id,
            file_id,
        }
    }
}

#[derive(Clone)]
pub struct CollaborationHub {
    rooms: Arc<Mutex<HashMap<RoomKey, RoomHandle>>>,
    v2: V2Repository,
    workspaces: WorkspaceService,
    blobs: Arc<dyn BlobStore>,
}

impl std::fmt::Debug for CollaborationHub {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CollaborationHub")
            .finish_non_exhaustive()
    }
}

impl CollaborationHub {
    pub fn new(v2: V2Repository, workspaces: WorkspaceService, blobs: Arc<dyn BlobStore>) -> Self {
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            v2,
            workspaces,
            blobs,
        }
    }

    async fn room(&self, access: &CollaborationAccess) -> Result<RoomHandle, String> {
        let key = RoomKey::new(access.workspace_id, access.file.file_id);
        let mut rooms = self.rooms.lock().await;
        if let Some(room) = rooms.get(&key) {
            if room.document_epoch == access.document_epoch {
                return Ok(room.clone());
            }
            rooms.remove(&key);
        }
        let recovery = self
            .v2
            .collaboration_recovery(key.workspace_id, key.file_id, access.document_epoch)
            .await
            .map_err(|error| error.to_string())?;
        let doc = Doc::new();
        let text = doc.get_or_insert_text("source");
        let mut durable_sequence = 0;
        if let Some((sequence, compressed)) = recovery.snapshot {
            let bytes = zstd::stream::decode_all(Cursor::new(compressed))
                .map_err(|error| format!("invalid collaboration snapshot: {error}"))?;
            apply_update(&doc, &bytes)?;
            durable_sequence = sequence;
        } else {
            let bytes = self
                .workspaces
                .read_file(key.workspace_id, &access.file.path)
                .await
                .map_err(|error| error.to_string())?;
            let source = String::from_utf8(bytes.to_vec())
                .map_err(|_| "binary files cannot enter source collaboration".to_owned())?;
            if !source.is_empty() {
                text.insert(&mut doc.transact_mut(), 0, &source);
            }
            let state = doc
                .transact()
                .encode_state_as_update_v1(&StateVector::default());
            let compressed = zstd::stream::encode_all(Cursor::new(state), 3)
                .map_err(|error| error.to_string())?;
            self.v2
                .save_collaboration_snapshot(
                    key.workspace_id,
                    key.file_id,
                    access.document_epoch,
                    0,
                    &compressed,
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        for (sequence, bytes) in recovery.updates {
            apply_update(&doc, &bytes)?;
            durable_sequence = sequence;
        }
        let room = RoomHandle::spawn(
            key,
            access.document_epoch,
            durable_sequence,
            doc,
            text,
            self.v2.clone(),
            self.blobs.clone(),
        );
        rooms.insert(key, room.clone());
        Ok(room)
    }

    pub async fn file_deleted(&self, workspace_id: WorkspaceId, file_id: Uuid) {
        let key = RoomKey::new(workspace_id, file_id);
        if let Some(room) = self.rooms.lock().await.get(&key) {
            let _ = room.events.send(RoomEvent::ReloadRequired);
        }
    }

    pub async fn policy_changed(&self, workspace_id: WorkspaceId, file_id: Uuid) {
        let key = RoomKey::new(workspace_id, file_id);
        if let Some(room) = self.rooms.lock().await.get(&key) {
            let _ = room.events.send(RoomEvent::PolicyChanged);
        }
    }

    pub async fn epoch_changed(&self, workspace_id: WorkspaceId, document_epoch: u64) {
        let mut rooms = self.rooms.lock().await;
        let keys = rooms
            .keys()
            .filter(|key| key.workspace_id == workspace_id)
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(room) = rooms.remove(&key) {
                let _ = room.events.send(RoomEvent::EpochChanged { document_epoch });
            }
        }
    }

    /// Waits until every currently loaded room in a workspace has persisted and
    /// canonically materialized all updates observed before its flush command.
    /// Unloaded files are already represented by the canonical workspace state.
    pub async fn flush_workspace(&self, workspace_id: WorkspaceId) -> Result<(), String> {
        let rooms = self
            .rooms
            .lock()
            .await
            .iter()
            .filter(|(key, _)| key.workspace_id == workspace_id)
            .map(|(_, room)| room.clone())
            .collect::<Vec<_>>();
        for room in rooms {
            room.flush().await?;
        }
        tracing::info!(%workspace_id, "collaboration workspace flush barrier completed");
        Ok(())
    }
}

#[derive(Clone, Debug)]
enum RoomEvent {
    Update { source: Uuid, bytes: Vec<u8> },
    Durable { sequence: u64 },
    ReloadRequired,
    PolicyChanged,
    EpochChanged { document_epoch: u64 },
}

#[derive(Debug)]
struct RoomJoin {
    initial_state: Vec<u8>,
    durable_sequence: u64,
}

#[derive(Clone, Debug)]
struct RoomHandle {
    commands: mpsc::Sender<RoomCommand>,
    events: broadcast::Sender<RoomEvent>,
    document_epoch: u64,
}

impl RoomHandle {
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        key: RoomKey,
        document_epoch: u64,
        durable_sequence: u64,
        doc: Doc,
        text: yrs::TextRef,
        v2: V2Repository,
        blobs: Arc<dyn BlobStore>,
    ) -> Self {
        let (commands, receiver) = mpsc::channel(256);
        let (events, _) = broadcast::channel(256);
        tokio::spawn(room_actor(
            key,
            document_epoch,
            durable_sequence,
            doc,
            text,
            v2,
            blobs,
            receiver,
            events.clone(),
        ));
        Self {
            commands,
            events,
            document_epoch,
        }
    }

    async fn join(&self, writer: bool) -> Result<RoomJoin, String> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .send(RoomCommand::Join { writer, reply })
            .await
            .map_err(|_| "collaboration room stopped".to_owned())?;
        receive
            .await
            .map_err(|_| "collaboration room stopped".to_owned())
    }

    async fn apply(
        &self,
        source: Uuid,
        actor: UserId,
        client_sequence: u64,
        bytes: Vec<u8>,
        acknowledgements: mpsc::Sender<String>,
    ) -> Result<(), String> {
        self.commands
            .send(RoomCommand::Apply {
                source,
                actor,
                client_sequence,
                bytes,
                acknowledgements,
            })
            .await
            .map_err(|_| "collaboration room stopped".to_owned())
    }

    async fn flush(&self) -> Result<u64, String> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .send(RoomCommand::Flush { reply })
            .await
            .map_err(|_| "collaboration room stopped".to_owned())?;
        receive
            .await
            .map_err(|_| "collaboration room stopped".to_owned())?
    }

    async fn leave(&self, writer: bool) {
        let _ = self.commands.send(RoomCommand::Leave { writer }).await;
    }
}

#[derive(Debug)]
enum RoomCommand {
    Join {
        writer: bool,
        reply: oneshot::Sender<RoomJoin>,
    },
    Apply {
        source: Uuid,
        actor: UserId,
        client_sequence: u64,
        bytes: Vec<u8>,
        acknowledgements: mpsc::Sender<String>,
    },
    Flush {
        reply: oneshot::Sender<Result<u64, String>>,
    },
    Leave {
        writer: bool,
    },
}

#[derive(Debug)]
struct PendingUpdate {
    actor: UserId,
    client_sequence: u64,
    bytes: Vec<u8>,
    acknowledgements: mpsc::Sender<String>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn room_actor(
    key: RoomKey,
    document_epoch: u64,
    mut durable_sequence: u64,
    doc: Doc,
    text: yrs::TextRef,
    v2: V2Repository,
    blobs: Arc<dyn BlobStore>,
    mut commands: mpsc::Receiver<RoomCommand>,
    events: broadcast::Sender<RoomEvent>,
) {
    let mut pending = Vec::new();
    let mut pending_bytes = 0;
    let mut clients = 0_usize;
    let mut writers = 0_usize;
    let mut updates_since_snapshot = 0_usize;
    let mut bytes_since_snapshot = 0_usize;
    let mut timer = tokio::time::interval(Duration::from_millis(50));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = timer.tick(), if !pending.is_empty() => {
                let flushed_updates = pending.len();
                let flushed_bytes = pending_bytes;
                if let Ok(sequence) = flush_batch(
                    key, document_epoch, &doc, &text, &v2, blobs.as_ref(), &mut pending,
                ).await {
                    durable_sequence = sequence;
                    updates_since_snapshot += flushed_updates;
                    bytes_since_snapshot += flushed_bytes;
                    pending_bytes = 0;
                    let _ = events.send(RoomEvent::Durable { sequence });
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match command {
                    RoomCommand::Join { writer, reply } => {
                        clients += 1;
                        writers += usize::from(writer);
                        let initial_state = doc.transact().encode_state_as_update_v1(&StateVector::default());
                        let _ = reply.send(RoomJoin { initial_state, durable_sequence });
                    }
                    RoomCommand::Apply { source, actor, client_sequence, bytes, acknowledgements } => {
                        if let Err(error) = apply_update(&doc, &bytes) {
                            let _ = acknowledgements.send(error_json("malformed_update", &error)).await;
                            continue;
                        }
                        pending_bytes += bytes.len();
                        pending.push(PendingUpdate { actor, client_sequence, bytes: bytes.clone(), acknowledgements });
                        let _ = events.send(RoomEvent::Update { source, bytes });
                        if pending_bytes >= BATCH_BYTES {
                            let flushed_updates = pending.len();
                            let flushed_bytes = pending_bytes;
                            if let Ok(sequence) = flush_batch(
                                key, document_epoch, &doc, &text, &v2, blobs.as_ref(), &mut pending,
                            ).await {
                                durable_sequence = sequence;
                                updates_since_snapshot += flushed_updates;
                                bytes_since_snapshot += flushed_bytes;
                                pending_bytes = 0;
                                let _ = events.send(RoomEvent::Durable { sequence });
                            }
                        }
                    }
                    RoomCommand::Flush { reply } => {
                        let flushed_updates = pending.len();
                        let flushed_bytes = pending_bytes;
                        let result = if pending.is_empty() {
                            Ok(durable_sequence)
                        } else {
                            match flush_batch(
                                key, document_epoch, &doc, &text, &v2, blobs.as_ref(), &mut pending,
                            ).await {
                                Ok(sequence) => {
                                    durable_sequence = sequence;
                                    updates_since_snapshot += flushed_updates;
                                    bytes_since_snapshot += flushed_bytes;
                                    pending_bytes = 0;
                                    let _ = events.send(RoomEvent::Durable { sequence });
                                    Ok(sequence)
                                }
                                Err(error) => Err(error),
                            }
                        };
                        let _ = reply.send(result);
                    }
                    RoomCommand::Leave { writer } => {
                        clients = clients.saturating_sub(1);
                        writers = writers.saturating_sub(usize::from(writer));
                        if writers == 0 && !pending.is_empty() {
                            let flushed_updates = pending.len();
                            let flushed_bytes = pending_bytes;
                            if let Ok(sequence) = flush_batch(
                                key, document_epoch, &doc, &text, &v2, blobs.as_ref(), &mut pending,
                            ).await {
                                durable_sequence = sequence;
                                updates_since_snapshot += flushed_updates;
                                bytes_since_snapshot += flushed_bytes;
                                pending_bytes = 0;
                                let _ = events.send(RoomEvent::Durable { sequence });
                            }
                        }
                        if clients == 0 && durable_sequence > 0 {
                            let _ = save_snapshot(key, document_epoch, durable_sequence, &doc, &v2).await;
                            updates_since_snapshot = 0;
                            bytes_since_snapshot = 0;
                        }
                    }
                }
                if durable_sequence > 0
                    && (updates_since_snapshot >= SNAPSHOT_UPDATES || bytes_since_snapshot >= SNAPSHOT_BYTES)
                    && save_snapshot(key, document_epoch, durable_sequence, &doc, &v2).await.is_ok()
                {
                    updates_since_snapshot = 0;
                    bytes_since_snapshot = 0;
                }
            }
        }
    }
}

async fn flush_batch(
    key: RoomKey,
    document_epoch: u64,
    doc: &Doc,
    text: &yrs::TextRef,
    v2: &V2Repository,
    blobs: &dyn BlobStore,
    pending: &mut Vec<PendingUpdate>,
) -> Result<u64, String> {
    let source = text.get_string(&doc.transact());
    let stored = blobs
        .put(Bytes::from(source))
        .await
        .map_err(|error| error.to_string())?;
    let updates = pending
        .iter()
        .map(|update| CollaborationUpdateInput {
            actor_user_id: update.actor,
            update_bytes: update.bytes.clone(),
        })
        .collect::<Vec<_>>();
    let sequence = v2
        .persist_collaboration_batch(
            key.workspace_id,
            key.file_id,
            document_epoch,
            &updates,
            stored.hash(),
            stored.size_bytes(),
        )
        .await
        .map_err(|error| {
            tracing::error!(workspace_id=%key.workspace_id, file_id=%key.file_id, %error, "collaboration durability batch failed");
            error.to_string()
        })?;
    tracing::info!(workspace_id=%key.workspace_id, file_id=%key.file_id, document_epoch, durable_sequence=sequence, updates=updates.len(), "collaboration batch persisted and materialized");
    for update in pending.drain(..) {
        let _ = update
            .acknowledgements
            .send(
                serde_json::json!({
                    "type":"DURABLE_ACK",
                    "client_seq":update.client_sequence,
                    "durable_seq":sequence
                })
                .to_string(),
            )
            .await;
    }
    Ok(sequence)
}

async fn save_snapshot(
    key: RoomKey,
    document_epoch: u64,
    durable_sequence: u64,
    doc: &Doc,
    v2: &V2Repository,
) -> Result<(), String> {
    let state = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let compressed =
        zstd::stream::encode_all(Cursor::new(state), 3).map_err(|error| error.to_string())?;
    v2.save_collaboration_snapshot(
        key.workspace_id,
        key.file_id,
        document_epoch,
        durable_sequence,
        &compressed,
    )
    .await
    .map_err(|error| error.to_string())?;
    tracing::info!(workspace_id=%key.workspace_id, file_id=%key.file_id, document_epoch, durable_sequence, "collaboration snapshot persisted");
    Ok(())
}

fn apply_update(doc: &Doc, bytes: &[u8]) -> Result<(), String> {
    let update = Update::decode_v1(bytes).map_err(|error| error.to_string())?;
    doc.transact_mut()
        .apply_update(update)
        .map_err(|error| error.to_string())
}

fn validate_update(bytes: &[u8]) -> Result<(), String> {
    Update::decode_v1(bytes)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn error_json(code: &str, message: &str) -> String {
    serde_json::json!({"type":"ERROR","code":code,"message":message}).to_string()
}

#[allow(
    clippy::too_many_lines,
    reason = "the compact S3 protocol is audited in one connection loop"
)]
pub async fn serve_socket(
    state: AppState,
    socket: WebSocket,
    session_token: String,
    user_id: UserId,
    paper_id: Uuid,
    access: CollaborationAccess,
) {
    let room = match state.collaboration.room(&access).await {
        Ok(room) => room,
        Err(error) => {
            let (mut sender, _) = socket.split();
            let _ = sender
                .send(Message::Text(error_json("room_bootstrap", &error).into()))
                .await;
            return;
        }
    };
    let writer = access.mode == CollaborationAccessMode::ReadWrite;
    let join = match room.join(writer).await {
        Ok(join) => join,
        Err(error) => {
            let (mut sender, _) = socket.split();
            let _ = sender
                .send(Message::Text(error_json("room_unavailable", &error).into()))
                .await;
            return;
        }
    };
    let connection_id = Uuid::new_v4();
    let (mut sender, mut receiver) = socket.split();
    let (control_tx, mut control_rx) = mpsc::channel::<String>(128);
    let mut events = room.events.subscribe();
    let accepted = serde_json::json!({
        "type":"JOIN_ACCEPTED",
        "document_epoch":access.document_epoch,
        "durable_seq":join.durable_sequence,
        "access":match access.mode {
            CollaborationAccessMode::ReadWrite => "read_write",
            CollaborationAccessMode::ReadOnly => "read_only",
        }
    });
    if sender
        .send(Message::Text(accepted.to_string().into()))
        .await
        .is_err()
    {
        room.leave(writer).await;
        return;
    }
    let mut initial = Vec::with_capacity(join.initial_state.len() + 1);
    initial.push(INITIAL_STATE);
    initial.extend(join.initial_state);
    if sender.send(Message::Binary(initial.into())).await.is_err() {
        room.leave(writer).await;
        return;
    }
    loop {
        tokio::select! {
            incoming = receiver.next() => {
                let Some(Ok(message)) = incoming else { break; };
                match message {
                    Message::Binary(bytes) => {
                        if bytes.is_empty() {
                            let _ = sender.send(Message::Text(error_json("protocol", "empty binary frame").into())).await;
                            break;
                        }
                        match bytes[0] {
                            SOURCE_UPDATE => {
                                if bytes.len() < 10 || bytes.len() - 9 > MAX_UPDATE_BYTES {
                                    let _ = sender.send(Message::Text(error_json("protocol", "invalid source update envelope").into())).await;
                                    break;
                                }
                                if access.mode != CollaborationAccessMode::ReadWrite {
                                    let _ = sender.send(Message::Text(error_json("read_only", "source updates are forbidden").into())).await;
                                    break;
                                }
                                let mut sequence_bytes = [0_u8; 8];
                                sequence_bytes.copy_from_slice(&bytes[1..9]);
                                let client_sequence = u64::from_be_bytes(sequence_bytes);
                                if let Err(error) = validate_update(&bytes[9..]) {
                                    let value = error_json("malformed_update", &error);
                                    let _ = sender.send(Message::Text(value.into())).await;
                                    break;
                                }
                                let session = state.repo.session(&digest(&session_token)).await.ok().flatten();
                                let still_authorized = matches!(
                                    session.as_ref().map(|session| (session.user_id, session.global_role)),
                                    Some((current_user, Some(persistence::GlobalRole::Writer))) if current_user == user_id
                                );
                                let current_access = if still_authorized {
                                    state.v2.collaboration_access(user_id, paper_id, access.file.file_id).await.ok()
                                } else {
                                    None
                                };
                                if !matches!(current_access, Some(ref current)
                                    if current.mode == CollaborationAccessMode::ReadWrite
                                        && current.document_epoch == access.document_epoch
                                        && current.workspace_id == access.workspace_id)
                                {
                                    let _ = sender.send(Message::Text(error_json("authority_revoked", "collaboration authority was revoked").into())).await;
                                    break;
                                }
                                if let Err(error) = room.apply(
                                    connection_id,
                                    user_id,
                                    client_sequence,
                                    bytes[9..].to_vec(),
                                    control_tx.clone(),
                                ).await {
                                    let _ = sender.send(Message::Text(error_json("room_unavailable", &error).into())).await;
                                    break;
                                }
                            }
                            FLUSH => {
                                match room.flush().await {
                                    Ok(sequence) => {
                                        let value = serde_json::json!({"type":"FLUSHED","durable_seq":sequence});
                                        if sender.send(Message::Text(value.to_string().into())).await.is_err() { break; }
                                    }
                                    Err(error) => {
                                        let _ = sender.send(Message::Text(error_json("durability", &error).into())).await;
                                    }
                                }
                            }
                            _ => {
                                let _ = sender.send(Message::Text(error_json("protocol", "unknown binary frame type").into())).await;
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(bytes) => {
                        if sender.send(Message::Pong(bytes)).await.is_err() { break; }
                    }
                    Message::Text(_) | Message::Pong(_) => {}
                }
            }
            Some(control) = control_rx.recv() => {
                if sender.send(Message::Text(control.into())).await.is_err() { break; }
            }
            event = events.recv() => {
                match event {
                    Ok(RoomEvent::Update { source, bytes }) if source != connection_id => {
                        let mut frame = Vec::with_capacity(bytes.len() + 1);
                        frame.push(REMOTE_SOURCE_UPDATE);
                        frame.extend(bytes);
                        if sender.send(Message::Binary(frame.into())).await.is_err() { break; }
                    }
                    Ok(RoomEvent::ReloadRequired) => {
                        let value = serde_json::json!({"type":"RELOAD_REQUIRED","reason":"file_deleted"});
                        let _ = sender.send(Message::Text(value.to_string().into())).await;
                        break;
                    }
                    Ok(RoomEvent::PolicyChanged) => {
                        let value = serde_json::json!({"type":"POLICY_CHANGED","message":"File policy changed; reload before continuing."});
                        let _ = sender.send(Message::Text(value.to_string().into())).await;
                        break;
                    }
                    Ok(RoomEvent::EpochChanged { document_epoch }) => {
                        let value = serde_json::json!({"type":"PAPER_EPOCH_CHANGED","document_epoch":document_epoch});
                        let _ = sender.send(Message::Text(value.to_string().into())).await;
                        break;
                    }
                    Ok(RoomEvent::Durable { sequence }) => {
                        let value = serde_json::json!({"type":"REMOTE_DURABLE","durable_seq":sequence});
                        if sender.send(Message::Text(value.to_string().into())).await.is_err() { break; }
                    }
                    Ok(RoomEvent::Update { .. }) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = sender.send(Message::Text(error_json("reload_required", "client fell behind room broadcast").into())).await;
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    room.leave(writer).await;
}

pub fn session_token(headers: &axum::http::HeaderMap) -> Option<String> {
    crate::cookie(headers)
}

pub fn websocket_principal_is_supported(principal: &crate::AuthenticatedPrincipal) -> bool {
    matches!(
        principal.kind,
        PrincipalKind::V2(persistence::GlobalRole::Writer | persistence::GlobalRole::Mentor)
    )
}
