#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use std::{
    collections::{BTreeMap, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    ops::Range,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(feature = "runtime")]
use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use shared::{AppError, AppResult};
use tracing::{debug, instrument, warn};

#[cfg(feature = "runtime")]
use rand::Rng;
#[cfg(feature = "runtime")]
use std::sync::Arc;
#[cfg(feature = "runtime")]
use tokio::{
    net::UdpSocket,
    sync::Mutex,
    time::timeout,
};

const PROTOCOL_VERSION: u8 = 1;
const HANDSHAKE_RETRIES: usize = 5;
const DEFAULT_TARGET_BITRATE: u64 = 6_000_000;
const DEFAULT_MAX_PACKET_PAYLOAD: usize = 1_200;
const DEFAULT_JITTER_CAPACITY: usize = 32;
const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 250;
const DEFAULT_KEY_FRAME_REPETITIONS: u8 = 2;
const MIN_PACKET_PAYLOAD: usize = 256;
const MAX_PAYLOAD_LIMIT: usize = 1_300;
const MAX_UDP_DATAGRAM: usize = 1_500;
const REDUNDANCY_CAP: u8 = 4;

fn default_target_bitrate() -> u64 {
    DEFAULT_TARGET_BITRATE
}

fn default_max_packet_payload() -> usize {
    DEFAULT_MAX_PACKET_PAYLOAD
}

fn default_jitter_buffer_capacity() -> usize {
    DEFAULT_JITTER_CAPACITY
}

fn default_handshake_timeout_ms() -> u64 {
    DEFAULT_HANDSHAKE_TIMEOUT_MS
}

fn default_key_frame_redundancy() -> u8 {
    DEFAULT_KEY_FRAME_REPETITIONS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointConfig {
    pub address: String,
    #[serde(default, alias = "bind", alias = "bind_address")]
    pub bind_address: Option<String>,
    #[serde(default = "default_target_bitrate")]
    pub target_bitrate: u64,
    #[serde(default = "default_max_packet_payload")]
    pub max_packet_payload: usize,
    #[serde(default = "default_jitter_buffer_capacity")]
    pub jitter_buffer_capacity: usize,
    #[serde(default = "default_handshake_timeout_ms")]
    pub handshake_timeout_ms: u64,
    #[serde(default = "default_key_frame_redundancy")]
    pub key_frame_redundancy: u8,
}

impl EndpointConfig {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            bind_address: None,
            target_bitrate: default_target_bitrate(),
            max_packet_payload: default_max_packet_payload(),
            jitter_buffer_capacity: default_jitter_buffer_capacity(),
            handshake_timeout_ms: default_handshake_timeout_ms(),
            key_frame_redundancy: default_key_frame_redundancy(),
        }
    }

    pub fn with_bind_address(mut self, bind: impl Into<String>) -> Self {
        self.bind_address = Some(bind.into());
        self
    }

    pub fn with_target_bitrate(mut self, bitrate: u64) -> Self {
        if bitrate > 0 {
            self.target_bitrate = bitrate;
        }
        self
    }

    pub fn with_max_packet_payload(mut self, payload: usize) -> Self {
        if payload >= MIN_PACKET_PAYLOAD {
            self.max_packet_payload = payload.min(MAX_PAYLOAD_LIMIT);
        }
        self
    }

    pub fn with_jitter_capacity(mut self, capacity: usize) -> Self {
        if capacity > 0 {
            self.jitter_buffer_capacity = capacity;
        }
        self
    }

    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        let millis = timeout.as_millis() as u64;
        if millis > 0 {
            self.handshake_timeout_ms = millis;
        }
        self
    }

    pub fn with_key_frame_redundancy(mut self, redundancy: u8) -> Self {
        self.key_frame_redundancy = redundancy.clamp(1, REDUNDANCY_CAP);
        self
    }

    pub fn remote_addr(&self) -> AppResult<SocketAddr> {
        parse_socket_addr(&self.address)
    }

    pub fn bind_addr(&self) -> AppResult<SocketAddr> {
        if let Some(addr) = &self.bind_address {
            return parse_socket_addr(addr);
        }
        let remote = self.remote_addr()?;
        let port = remote.port();
        let bind = match remote {
            SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
            SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port),
        };
        Ok(bind)
    }

    pub fn handshake_timeout(&self) -> Duration {
        Duration::from_millis(self.handshake_timeout_ms.max(1))
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.address.trim().is_empty() {
            return Err(AppError::Message("network address cannot be empty".into()));
        }
        self.remote_addr()?;
        if let Some(bind) = &self.bind_address {
            parse_socket_addr(bind)?;
        }
        if self.target_bitrate == 0 {
            return Err(AppError::Message("target bitrate must be greater than zero".into()));
        }
        if self.max_packet_payload < MIN_PACKET_PAYLOAD || self.max_packet_payload > MAX_PAYLOAD_LIMIT {
            return Err(AppError::Message(format!(
                "max packet payload must be between {MIN_PACKET_PAYLOAD} and {MAX_PAYLOAD_LIMIT} bytes"
            )));
        }
        if self.jitter_buffer_capacity == 0 {
            return Err(AppError::Message("jitter buffer capacity must be greater than zero".into()));
        }
        if self.key_frame_redundancy == 0 {
            return Err(AppError::Message("key frame redundancy must be at least 1".into()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FramePayload<'a> {
    data: &'a [u8],
    timestamp: Option<SystemTime>,
    is_key: bool,
}

impl<'a> FramePayload<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            timestamp: None,
            is_key: false,
        }
    }

    pub fn with_timestamp(mut self, timestamp: SystemTime) -> Self {
        self.timestamp = Some(timestamp);
        self
    }

    pub fn as_key_frame(mut self) -> Self {
        self.is_key = true;
        self
    }

    pub fn data(&self) -> &[u8] {
        self.data
    }

    pub fn timestamp(&self) -> Option<SystemTime> {
        self.timestamp
    }

    pub fn is_key(&self) -> bool {
        self.is_key
    }
}

#[derive(Debug, Clone)]
pub struct ReceivedFrame {
    pub session_id: u64,
    pub frame_id: u64,
    pub data: Vec<u8>,
    pub timestamp: SystemTime,
    pub arrival: SystemTime,
    pub is_key: bool,
    pub latency: Duration,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
struct PacketFlags(u8);

impl PacketFlags {
    const KEY: u8 = 0b0000_0001;
    const REDUNDANT: u8 = 0b0000_0010;

    fn empty() -> Self {
        Self(0)
    }

    fn mark_key(&mut self) {
        self.0 |= Self::KEY;
    }

    fn mark_redundant(&mut self) {
        self.0 |= Self::REDUNDANT;
    }

    fn is_key(self) -> bool {
        self.0 & Self::KEY != 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PacketHeader {
    protocol_version: u8,
    session_id: u64,
    frame_id: u64,
    sequence: u64,
    timestamp_micros: u64,
    fragment_index: u16,
    fragment_count: u16,
    flags: PacketFlags,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DataPacket {
    header: PacketHeader,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandshakeMessage {
    session_id: u64,
    target_bitrate: u64,
    max_packet_payload: u32,
    key_frame_redundancy: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HandshakeAck {
    session_id: u64,
    accepted_packet_payload: u32,
    jitter_capacity: u16,
    key_frame_redundancy: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum WireMessage {
    Handshake(HandshakeMessage),
    HandshakeAck(HandshakeAck),
    Data(DataPacket),
    Redundant(DataPacket),
}

#[derive(Debug, Clone)]
struct FragmentPlan {
    fragment_size: usize,
    fragment_count: usize,
}

impl FragmentPlan {
    fn fragment_ranges(&self, total_len: usize) -> Vec<Range<usize>> {
        let mut ranges = Vec::with_capacity(self.fragment_count);
        let mut start = 0;
        for index in 0..self.fragment_count {
            let mut end = start + self.fragment_size;
            if end > total_len || index == self.fragment_count - 1 {
                end = total_len;
            }
            if end < start {
                end = start;
            }
            ranges.push(start..end);
            start = end;
        }
        ranges
    }
}

#[derive(Debug, Clone)]
struct AdaptivePacketSizer {
    target_bitrate: u64,
    min_payload: usize,
    max_payload: usize,
    current_payload: usize,
}

impl AdaptivePacketSizer {
    fn new(target_bitrate: u64, max_payload: usize) -> Self {
        let max_payload = max_payload.clamp(MIN_PACKET_PAYLOAD, MAX_PAYLOAD_LIMIT);
        let initial = max_payload.min((target_bitrate as usize / 8).max(MIN_PACKET_PAYLOAD));
        Self {
            target_bitrate,
            min_payload: MIN_PACKET_PAYLOAD,
            max_payload,
            current_payload: initial,
        }
    }

    fn update_max_payload(&mut self, max_payload: usize) {
        self.max_payload = max_payload.clamp(self.min_payload, MAX_PAYLOAD_LIMIT);
        self.current_payload = self.current_payload.min(self.max_payload);
    }

    fn plan(&mut self, payload_len: usize, is_key: bool, cap: usize) -> FragmentPlan {
        let frames_per_second = 60u64;
        let per_frame_bits = (self.target_bitrate.max(1) / frames_per_second)
            .max((self.min_payload * 8) as u64);
        let bandwidth_hint = (per_frame_bits / 8) as usize;
        let effective_cap = cap
            .min(bandwidth_hint.max(self.min_payload))
            .clamp(self.min_payload, self.max_payload);
        if is_key {
            self.current_payload = ((self.current_payload + effective_cap) / 2).max(self.min_payload);
        }
        let fragment_size = self.current_payload.min(effective_cap).max(self.min_payload);
        let mut fragment_count = if payload_len == 0 {
            1
        } else {
            (payload_len + fragment_size - 1) / fragment_size
        };
        fragment_count = fragment_count.max(1);
        if fragment_count > 4 {
            self.current_payload = self
                .current_payload
                .saturating_sub(self.min_payload / 2)
                .max(self.min_payload);
        } else {
            let increase = (self.min_payload / 4).max(1);
            self.current_payload = (self.current_payload + increase).min(effective_cap);
        }
        FragmentPlan {
            fragment_size,
            fragment_count,
        }
    }
}

#[derive(Debug, Clone)]
struct FrameAssembly {
    timestamp_micros: u64,
    flags: PacketFlags,
    fragments: Vec<Option<Vec<u8>>>,
    received: usize,
}

impl FrameAssembly {
    fn new(fragment_count: u16, timestamp: u64, flags: PacketFlags) -> Self {
        let count = fragment_count.max(1) as usize;
        Self {
            timestamp_micros: timestamp,
            flags,
            fragments: vec![None; count],
            received: 0,
        }
    }

    fn insert(&mut self, packet: DataPacket) {
        let idx = packet.header.fragment_index as usize;
        if idx >= self.fragments.len() {
            return;
        }
        if self.fragments[idx].is_none() {
            self.fragments[idx] = Some(packet.payload);
            self.received += 1;
        }
        if packet.header.flags.is_key() {
            self.flags.mark_key();
        }
        self.timestamp_micros = packet.header.timestamp_micros;
    }

    fn is_complete(&self) -> bool {
        self.received == self.fragments.len()
    }

    fn into_frame(self, frame_id: u64) -> AssembledFrame {
        let mut payload = Vec::new();
        for fragment in self.fragments.into_iter() {
            if let Some(bytes) = fragment {
                payload.extend_from_slice(&bytes);
            }
        }
        AssembledFrame {
            frame_id,
            timestamp_micros: self.timestamp_micros,
            flags: self.flags,
            payload,
        }
    }
}

#[derive(Debug, Clone)]
struct AssembledFrame {
    frame_id: u64,
    timestamp_micros: u64,
    flags: PacketFlags,
    payload: Vec<u8>,
}

impl AssembledFrame {
    fn into_received(self, session_id: u64) -> ReceivedFrame {
        let timestamp = micros_to_system_time(self.timestamp_micros);
        let arrival = SystemTime::now();
        let latency = arrival
            .duration_since(timestamp)
            .unwrap_or_else(|_| Duration::from_millis(0));
        ReceivedFrame {
            session_id,
            frame_id: self.frame_id,
            data: self.payload,
            timestamp,
            arrival,
            is_key: self.flags.is_key(),
            latency,
        }
    }
}

#[derive(Debug, Clone)]
struct JitterBuffer {
    capacity: usize,
    next_frame_id: u64,
    assemblies: BTreeMap<u64, FrameAssembly>,
    ready: VecDeque<AssembledFrame>,
}

impl JitterBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_frame_id: 0,
            assemblies: BTreeMap::new(),
            ready: VecDeque::new(),
        }
    }

    fn push(&mut self, packet: DataPacket) {
        if packet.header.protocol_version != PROTOCOL_VERSION {
            return;
        }
        let DataPacket { header, payload } = packet;
        let frame_id = header.frame_id;
        if frame_id < self.next_frame_id {
            return;
        }
        let fragment_count = header.fragment_count.max(1);
        let timestamp = header.timestamp_micros;
        let flags = header.flags;
        let complete = {
            let entry = self
                .assemblies
                .entry(frame_id)
                .or_insert_with(|| FrameAssembly::new(fragment_count, timestamp, flags));
            entry.insert(DataPacket { header, payload });
            entry.is_complete()
        };
        if complete {
            if let Some(assembly) = self.assemblies.remove(&frame_id) {
                let frame = assembly.into_frame(frame_id);
                self.insert_ready(frame);
            }
        }
        self.trim_assemblies();
    }

    fn insert_ready(&mut self, frame: AssembledFrame) {
        let pos = self
            .ready
            .iter()
            .position(|existing| frame.frame_id < existing.frame_id)
            .unwrap_or(self.ready.len());
        self.ready.insert(pos, frame);
        while self.ready.len() > self.capacity {
            self.ready.pop_front();
            if let Some(front) = self.ready.front() {
                self.next_frame_id = front.frame_id;
            }
        }
    }

    fn trim_assemblies(&mut self) {
        while self.assemblies.len() > self.capacity {
            if let Some((&frame_id, _)) = self.assemblies.iter().next() {
                self.assemblies.remove(&frame_id);
                self.next_frame_id = frame_id + 1;
            } else {
                break;
            }
        }
    }

    fn pop_ready(&mut self) -> Option<AssembledFrame> {
        let frame = self.ready.pop_front()?;
        if frame.frame_id >= self.next_frame_id {
            self.next_frame_id = frame.frame_id + 1;
        }
        Some(frame)
    }
}

#[derive(Debug, Clone)]
pub struct NetworkSender {
    config: EndpointConfig,
    #[cfg(feature = "runtime")]
    inner: Arc<SenderInner>,
}

impl NetworkSender {
    pub fn new(config: EndpointConfig) -> Self {
        #[cfg(feature = "runtime")]
        let inner = Arc::new(SenderInner::new(config.clone()));
        Self {
            config,
            #[cfg(feature = "runtime")]
            inner,
        }
    }

    pub fn config(&self) -> &EndpointConfig {
        &self.config
    }

    #[cfg(feature = "runtime")]
    #[instrument(skip(self, payload))]
    pub async fn send(&self, payload: &[u8]) -> AppResult<()> {
        self.send_frame(FramePayload::new(payload)).await
    }

    #[cfg(feature = "runtime")]
    #[instrument(skip(self, payload))]
    pub async fn send_key_frame(&self, payload: &[u8]) -> AppResult<()> {
        self.send_frame(FramePayload::new(payload).as_key_frame())
            .await
    }

    #[cfg(feature = "runtime")]
    #[instrument(skip(self, frame), fields(len = frame.data().len(), key = frame.is_key()))]
    pub async fn send_frame(&self, frame: FramePayload<'_>) -> AppResult<()> {
        self.inner.send_frame(frame).await
    }
}

#[derive(Debug, Clone)]
pub struct NetworkReceiver {
    config: EndpointConfig,
    #[cfg(feature = "runtime")]
    inner: Arc<ReceiverInner>,
}

impl NetworkReceiver {
    pub fn new(config: EndpointConfig) -> Self {
        #[cfg(feature = "runtime")]
        let inner = Arc::new(ReceiverInner::new(config.clone()));
        Self {
            config,
            #[cfg(feature = "runtime")]
            inner,
        }
    }

    pub fn config(&self) -> &EndpointConfig {
        &self.config
    }

    #[cfg(feature = "runtime")]
    #[instrument(skip(self))]
    pub async fn receive(&self) -> AppResult<ReceivedFrame> {
        self.inner.receive().await
    }
}

#[cfg(feature = "runtime")]
#[derive(Debug)]
struct SenderSession {
    session_id: u64,
    remote_addr: SocketAddr,
    next_frame_id: u64,
    next_sequence: u64,
    redundancy: u8,
    max_payload: usize,
}

#[cfg(feature = "runtime")]
#[derive(Debug)]
struct SenderInner {
    config: EndpointConfig,
    socket: Mutex<Option<Arc<UdpSocket>>>,
    session: Mutex<Option<SenderSession>>,
    sizer: Mutex<AdaptivePacketSizer>,
}

#[cfg(feature = "runtime")]
impl SenderInner {
    fn new(config: EndpointConfig) -> Self {
        let sizer = AdaptivePacketSizer::new(config.target_bitrate, config.max_packet_payload);
        Self {
            config,
            socket: Mutex::new(None),
            session: Mutex::new(None),
            sizer: Mutex::new(sizer),
        }
    }

    async fn send_frame(&self, frame: FramePayload<'_>) -> AppResult<()> {
        self.config.validate()?;
        let socket = self.ensure_socket().await?;
        {
            let mut session_guard = self.session.lock().await;
            if session_guard.is_none() {
                drop(session_guard);
                self.perform_handshake(&socket).await?;
            }
        }
        let mut session_guard = self.session.lock().await;
        let session = session_guard
            .as_mut()
            .ok_or_else(|| AppError::Message("handshake did not produce a session".into()))?;
        let redundancy = session.redundancy.max(1);
        let max_payload = session.max_payload;
        let payload = frame.data();
        if payload.is_empty() {
            return Ok(());
        }
        let timestamp = frame.timestamp().unwrap_or_else(SystemTime::now);
        let timestamp_micros = system_time_to_micros(timestamp);
        let payload_len = payload.len();
        drop(session_guard);

        let plan = {
            let mut sizer = self.sizer.lock().await;
            sizer.plan(payload_len, frame.is_key(), max_payload)
        };
        let ranges = plan.fragment_ranges(payload_len);
        let fragment_count = ranges.len();

        let mut session_guard = self.session.lock().await;
        let session = session_guard
            .as_mut()
            .ok_or_else(|| AppError::Message("handshake session unavailable".into()))?;
        let session_id = session.session_id;
        let frame_id = session.next_frame_id;
        session.next_frame_id = session.next_frame_id.wrapping_add(1);
        let mut sequence = session.next_sequence;
        let remote_addr = session.remote_addr;

        let mut packets = Vec::with_capacity(fragment_count * redundancy as usize);
        for (index, range) in ranges.into_iter().enumerate() {
            let mut flags = PacketFlags::empty();
            if frame.is_key() {
                flags.mark_key();
            }
            let header = PacketHeader {
                protocol_version: PROTOCOL_VERSION,
                session_id,
                frame_id,
                sequence,
                timestamp_micros,
                fragment_index: index as u16,
                fragment_count: fragment_count as u16,
                flags,
            };
            let fragment_bytes = payload[range.clone()].to_vec();
            packets.push(WireMessage::Data(DataPacket {
                header: header.clone(),
                payload: fragment_bytes.clone(),
            }));
            sequence = sequence.wrapping_add(1);
            if frame.is_key() && redundancy > 1 {
                for _ in 1..redundancy {
                    let mut redundant_header = header.clone();
                    redundant_header.sequence = sequence;
                    let mut redundant_flags = redundant_header.flags;
                    redundant_flags.mark_redundant();
                    redundant_header.flags = redundant_flags;
                    packets.push(WireMessage::Redundant(DataPacket {
                        header: redundant_header,
                        payload: fragment_bytes.clone(),
                    }));
                    sequence = sequence.wrapping_add(1);
                }
            }
        }
        session.next_sequence = sequence;
        drop(session_guard);

        for packet in packets {
            let encoded = encode_message(&packet)?;
            socket
                .send_to(&encoded, remote_addr)
                .await
                .map_err(map_io_error)?;
        }
        Ok(())
    }

    async fn ensure_socket(&self) -> AppResult<Arc<UdpSocket>> {
        if let Some(existing) = self.socket.lock().await.as_ref().cloned() {
            return Ok(existing);
        }
        let bind_addr = self.config.bind_addr()?;
        let socket = Arc::new(UdpSocket::bind(bind_addr).await.map_err(map_io_error)?);
        let mut guard = self.socket.lock().await;
        if let Some(existing) = guard.as_ref() {
            Ok(existing.clone())
        } else {
            *guard = Some(socket.clone());
            Ok(socket)
        }
    }

    async fn perform_handshake(&self, socket: &Arc<UdpSocket>) -> AppResult<()> {
        let remote = self.config.remote_addr()?;
        let session_id = rand::thread_rng().gen::<u64>();
        let max_payload = self
            .config
            .max_packet_payload
            .clamp(MIN_PACKET_PAYLOAD, MAX_PAYLOAD_LIMIT) as u32;
        let redundancy = self.config.key_frame_redundancy.clamp(1, REDUNDANCY_CAP);
        let handshake = WireMessage::Handshake(HandshakeMessage {
            session_id,
            target_bitrate: self.config.target_bitrate,
            max_packet_payload,
            key_frame_redundancy: redundancy,
        });
        let encoded = encode_message(&handshake)?;
        let timeout_duration = self.config.handshake_timeout();
        let mut buffer = vec![0u8; MAX_UDP_DATAGRAM];

        for attempt in 0..HANDSHAKE_RETRIES {
            socket
                .send_to(&encoded, remote)
                .await
                .map_err(map_io_error)?;
            match timeout(timeout_duration, socket.recv_from(&mut buffer)).await {
                Ok(Ok((len, addr))) => {
                    if addr != remote {
                        continue;
                    }
                    let message = decode_message(&buffer[..len])?;
                    if let WireMessage::HandshakeAck(ack) = message {
                        if ack.session_id != session_id {
                            continue;
                        }
                        let accepted_payload = ack
                            .accepted_packet_payload
                            .max(MIN_PACKET_PAYLOAD as u32) as usize;
                        {
                            let mut sizer = self.sizer.lock().await;
                            sizer.update_max_payload(accepted_payload);
                        }
                        let mut session_guard = self.session.lock().await;
                        *session_guard = Some(SenderSession {
                            session_id,
                            remote_addr: remote,
                            next_frame_id: 0,
                            next_sequence: 0,
                            redundancy: ack.key_frame_redundancy.clamp(1, REDUNDANCY_CAP),
                            max_payload: accepted_payload,
                        });
                        debug!(attempt, session = session_id, addr = %remote, "handshake established");
                        return Ok(());
                    }
                }
                Ok(Err(err)) => return Err(map_io_error(err)),
                Err(_) => {
                    warn!(attempt, session = session_id, "handshake timeout, retrying");
                    continue;
                }
            }
        }
        Err(AppError::Message(format!("handshake with {remote} timed out")))
    }
}

#[cfg(feature = "runtime")]
#[derive(Debug)]
struct ReceiverSession {
    remote_addr: SocketAddr,
    jitter: JitterBuffer,
    max_payload: usize,
    redundancy: u8,
}

#[cfg(feature = "runtime")]
#[derive(Debug)]
struct ReceiverInner {
    config: EndpointConfig,
    socket: Mutex<Option<Arc<UdpSocket>>>,
    sessions: Mutex<HashMap<u64, ReceiverSession>>,
}

#[cfg(feature = "runtime")]
impl ReceiverInner {
    fn new(config: EndpointConfig) -> Self {
        Self {
            config,
            socket: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    #[instrument(skip(self))]
    async fn receive(&self) -> AppResult<ReceivedFrame> {
        self.config.validate()?;
        loop {
            if let Some(frame) = self.try_pop_ready_frame().await {
                return Ok(frame);
            }
            let socket = self.ensure_socket().await?;
            let mut buffer = vec![0u8; MAX_UDP_DATAGRAM];
            let (len, addr) = socket.recv_from(&mut buffer).await.map_err(map_io_error)?;
            let message = decode_message(&buffer[..len])?;
            match message {
                WireMessage::Handshake(msg) => {
                    self.handle_handshake(&socket, msg, addr).await?;
                }
                WireMessage::HandshakeAck(_) => {
                    debug!(addr = %addr, "receiver ignoring unexpected handshake ack");
                }
                WireMessage::Data(packet) | WireMessage::Redundant(packet) => {
                    if let Some((session_id, frame)) = self.handle_data(packet, addr).await? {
                        return Ok(frame.into_received(session_id));
                    }
                }
            }
        }
    }

    async fn try_pop_ready_frame(&self) -> Option<ReceivedFrame> {
        let mut sessions = self.sessions.lock().await;
        for (session_id, session) in sessions.iter_mut() {
            if let Some(frame) = session.jitter.pop_ready() {
                return Some(frame.into_received(*session_id));
            }
        }
        None
    }

    async fn ensure_socket(&self) -> AppResult<Arc<UdpSocket>> {
        if let Some(existing) = self.socket.lock().await.as_ref().cloned() {
            return Ok(existing);
        }
        let bind_addr = self.config.bind_addr()?;
        let socket = Arc::new(UdpSocket::bind(bind_addr).await.map_err(map_io_error)?);
        let mut guard = self.socket.lock().await;
        if let Some(existing) = guard.as_ref() {
            Ok(existing.clone())
        } else {
            *guard = Some(socket.clone());
            Ok(socket)
        }
    }

    async fn handle_handshake(
        &self,
        socket: &Arc<UdpSocket>,
        message: HandshakeMessage,
        addr: SocketAddr,
    ) -> AppResult<()> {
        let accepted_payload = message
            .max_packet_payload
            .min(self.config.max_packet_payload as u32)
            .max(MIN_PACKET_PAYLOAD as u32) as usize;
        let redundancy = message.key_frame_redundancy.clamp(1, REDUNDANCY_CAP);
        let jitter_capacity = self
            .config
            .jitter_buffer_capacity
            .min(u16::MAX as usize) as u16;
        let mut sessions = self.sessions.lock().await;
        let entry = sessions.entry(message.session_id).or_insert_with(|| ReceiverSession {
            remote_addr: addr,
            jitter: JitterBuffer::new(self.config.jitter_buffer_capacity),
            max_payload: accepted_payload,
            redundancy,
        });
        entry.remote_addr = addr;
        entry.max_payload = accepted_payload;
        entry.redundancy = redundancy;
        drop(sessions);

        let ack = WireMessage::HandshakeAck(HandshakeAck {
            session_id: message.session_id,
            accepted_packet_payload: accepted_payload as u32,
            jitter_capacity,
            key_frame_redundancy: redundancy,
        });
        let encoded = encode_message(&ack)?;
        socket
            .send_to(&encoded, addr)
            .await
            .map_err(map_io_error)?;
        debug!(session = message.session_id, addr = %addr, "sent handshake ack");
        Ok(())
    }

    async fn handle_data(
        &self,
        packet: DataPacket,
        addr: SocketAddr,
    ) -> AppResult<Option<(u64, AssembledFrame)>> {
        if packet.header.protocol_version != PROTOCOL_VERSION {
            warn!("dropping packet with incompatible protocol version");
            return Ok(None);
        }
        let session_id = packet.header.session_id;
        let mut sessions = self.sessions.lock().await;
        let session = match sessions.get_mut(&session_id) {
            Some(session) => session,
            None => {
                warn!(session = session_id, "dropping packet for unknown session");
                return Ok(None);
            }
        };
        if session.remote_addr != addr {
            session.remote_addr = addr;
        }
        session.jitter.push(packet);
        if let Some(frame) = session.jitter.pop_ready() {
            return Ok(Some((session_id, frame)));
        }
        Ok(None)
    }
}

#[instrument]
pub fn validate_address(address: &str) -> AppResult<()> {
    parse_socket_addr(address).map(|_| ())
}

#[instrument]
pub fn config_from_json(json: &str) -> AppResult<EndpointConfig> {
    let config: EndpointConfig = serde_json::from_str(json)
        .map_err(|err| AppError::Message(format!("invalid endpoint config: {err}")))?;
    config.validate()?;
    Ok(config)
}

fn parse_socket_addr(addr: &str) -> AppResult<SocketAddr> {
    addr.parse::<SocketAddr>().map_err(|err| {
        AppError::Message(format!("invalid socket address '{addr}': {err}"))
    })
}

fn system_time_to_micros(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_millis(0))
        .as_micros() as u64
}

fn micros_to_system_time(micros: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_micros(micros)
}

fn map_io_error(err: impl std::fmt::Display) -> AppError {
    AppError::Message(format!("network io error: {err}"))
}

fn encode_message(message: &WireMessage) -> AppResult<Vec<u8>> {
    bincode::serialize(message)
        .map_err(|err| AppError::Message(format!("failed to encode packet: {err}")))
}

fn decode_message(bytes: &[u8]) -> AppResult<WireMessage> {
    bincode::deserialize(bytes)
        .map_err(|err| AppError::Message(format!("failed to decode packet: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_packet(
        session_id: u64,
        frame_id: u64,
        fragment_index: u16,
        fragment_count: u16,
        payload: &[u8],
    ) -> DataPacket {
        DataPacket {
            header: PacketHeader {
                protocol_version: PROTOCOL_VERSION,
                session_id,
                frame_id,
                sequence: fragment_index as u64,
                timestamp_micros: 42,
                fragment_index,
                fragment_count,
                flags: PacketFlags::empty(),
            },
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn adaptive_packet_sizer_splits_payload() {
        let mut sizer = AdaptivePacketSizer::new(4_000_000, 800);
        let plan = sizer.plan(4_096, false, 800);
        assert!(plan.fragment_count > 1);
        let ranges = plan.fragment_ranges(4_096);
        let total: usize = ranges.iter().map(|r| r.len()).sum();
        assert_eq!(total, 4_096);
        for range in ranges {
            assert!(range.len() <= 800);
        }
    }

    #[test]
    fn jitter_buffer_reorders_frames() {
        let mut buffer = JitterBuffer::new(8);
        buffer.push(make_packet(1, 100, 1, 2, b"world"));
        assert!(buffer.pop_ready().is_none());
        buffer.push(make_packet(1, 100, 0, 2, b"hello"));
        let frame = buffer.pop_ready().expect("assembled frame");
        assert_eq!(frame.payload, b"helloworld".to_vec());
        buffer.push(make_packet(1, 102, 0, 1, b"gamma"));
        buffer.push(make_packet(1, 101, 0, 1, b"beta"));
        let first = buffer.pop_ready().expect("beta frame");
        assert_eq!(first.frame_id, 101);
        let second = buffer.pop_ready().expect("gamma frame");
        assert_eq!(second.frame_id, 102);
    }
}
