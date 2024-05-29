// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

// Identified => sent
// Identified => received

/*
For listen and dial options

    match opts.mode {
        Mode::Dial => {
            swarm
                .dial(
                    opts.relay_address
                        .with(Protocol::P2pCircuit)
                        .with(Protocol::P2p(opts.remote_peer_id.unwrap())),
                )
                .unwrap();
        }
        Mode::Listen => {
            swarm
                .listen_on(opts.relay_address.with(Protocol::P2pCircuit))
                .unwrap();
        }
    }

*/

use std::{
    cmp, collections::VecDeque, task::{Context, Poll}, time::Duration
};

use ahash::{HashMap, HashMapExt, HashSet, HashSetExt};
use ::futures::FutureExt;
use libp2p::{
    autonat, core::Multiaddr, identify, identity::{Keypair, PeerId, PublicKey}, kad::{self, store::MemoryStore}, mdns::{tokio::Behaviour as Mdns, Event as MdnsEvent}, multiaddr::Protocol, rendezvous, swarm::{
        behaviour::toggle::Toggle, derive_prelude::*, dial_opts::{DialOpts, PeerCondition}, ListenOpts, NetworkBehaviour, ToSwarm
    }, upnp, StreamProtocol
};
use tokio::time::Interval;


#[derive(NetworkBehaviour)]
pub struct DerivedDiscoveryBehaviour {
    /// Kademlia discovery.
    kademlia: Toggle<kad::Behaviour<MemoryStore>>,
    /// Discovers nodes on the local network.
    mdns: Toggle<Mdns>,
    /// [`identify::Behaviour`] needs to be manually hooked up with [`kad::Behaviour`] to make discovery work. See <https://docs.rs/libp2p/latest/libp2p/kad/index.html#important-discrepancies>
    identify: identify::Behaviour,
    /// For details see <https://github.com/libp2p/specs/blob/master/autonat/README.md>
    autonat: autonat::Behaviour,
    /// `UPnP` port mapping that automatically try to map the ports externally to internal addresses on the gateway.
    upnp: upnp::tokio::Behaviour,
    /// Rendezvous for peer discovery
    rendezvous: Toggle<rendezvous::client::Behaviour>,
}

/// Event generated by the `DiscoveryBehaviour`.
#[derive(Debug)]
pub enum DiscoveryEvent {
    /// Event that notifies that we connected to the node with the given peer
    /// id.
    PeerConnected(PeerId),

    /// Event that notifies that we disconnected with the node with the given
    /// peer id.
    PeerDisconnected(PeerId),

    /// Discovery event
    Discovery(Box<DerivedDiscoveryBehaviourEvent>),
}

/// `DiscoveryBehaviour` configuration.
///
/// Note: In order to discover nodes or load and store values via Kademlia one
/// has to add at least one protocol.
pub struct DiscoveryConfig<'a> {
    local_peer_id: PeerId,
    local_public_key: PublicKey,
    keypair: Keypair,
    user_defined: Vec<(PeerId, Multiaddr)>,
    target_peer_count: u64,
    enable_mdns: bool,
    enable_kademlia: bool,
    enable_rendezvous: bool,
    network_name: &'a str,
}

impl<'a> DiscoveryConfig<'a> {
    /// Create a default configuration with the given public key.
    pub fn new(keypair: Keypair, local_public_key: PublicKey, network_name: &'a str) -> Self {
        DiscoveryConfig {
            local_peer_id: local_public_key.to_peer_id(),
            keypair: keypair.clone(),
            local_public_key,
            user_defined: Vec::new(),
            target_peer_count: std::u64::MAX,
            enable_mdns: false,
            enable_kademlia: true,
            enable_rendezvous: true,
            network_name,
        }
    }

    /// Set the number of connected peers at which we pause discovery.
    pub fn target_peer_count(mut self, limit: u64) -> Self {
        self.target_peer_count = limit;
        self
    }

    /// Set custom nodes which never expire, e.g. bootstrap or reserved nodes.
    pub fn with_user_defined(
        mut self,
        user_defined: impl IntoIterator<Item = Multiaddr>,
    ) -> anyhow::Result<Self> {
        for mut addr in user_defined.into_iter() {
            if let Some(Protocol::P2p(peer_id)) = addr.pop() {
                self.user_defined.push((peer_id, addr))
            } else {
                anyhow::bail!("Failed to parse peer id from {addr}")
            }
        }
        Ok(self)
    }

    /// Configures if MDNS is enabled.
    pub fn with_mdns(mut self, value: bool) -> Self {
        self.enable_mdns = value;
        self
    }

    /// Configures if Kademlia is enabled.
    pub fn with_kademlia(mut self, value: bool) -> Self {
        self.enable_kademlia = value;
        self
    }

    /// Configures if Rendezvous is enabled.
    pub fn with_rendezvous(mut self, value: bool) -> Self {
        self.enable_rendezvous = value;
        self
    }

    /// Create a `DiscoveryBehaviour` from this configuration.
    pub fn finish(self) -> anyhow::Result<DiscoveryBehaviour> {
        let DiscoveryConfig {
            local_peer_id,
            local_public_key,
            keypair,
            user_defined,
            target_peer_count,
            enable_mdns,
            enable_kademlia,
            enable_rendezvous,
            network_name,
        } = self;

        let mut peers = HashSet::new();

        // Kademlia config
        let store = MemoryStore::new(local_peer_id);
        let kad_config = {
            let mut cfg = kad::Config::default();
            cfg.set_protocol_names(vec![StreamProtocol::try_from_owned(format!(
                "/openp2p/{network_name}/1.0.0"
            ))?]);
            cfg
        };

        let kademlia_opt = if enable_kademlia {
            let mut kademlia = kad::Behaviour::with_config(local_peer_id, store, kad_config);
            // `set_mode(Server)` fixes https://github.com/ChainSafe/forest/issues/3620
            // but it should not be required as the behaviour should automatically switch to server mode
            // according to the doc. It might be a bug in `libp2p`.
            // We should fix the bug or report with a minimal reproduction.
            kademlia.set_mode(Some(kad::Mode::Server));
            for (peer_id, addr) in &user_defined {
                kademlia.add_address(peer_id, addr.clone());
                peers.insert(*peer_id);
            }
            if let Err(e) = kademlia.bootstrap() {
                println!("Kademlia bootstrap failed: {:?}", e);
            }
            Some(kademlia)
        } else {
            None
        };

        let mdns_opt = if enable_mdns {
            Some(Mdns::new(Default::default(), local_peer_id).expect("Could not start mDNS"))
        } else {
            None
        };

        let rendezvous_opt = if enable_rendezvous {
            Some(libp2p::rendezvous::client::Behaviour::new(keypair))
        } else {
            None
        };

        Ok(DiscoveryBehaviour {
            discovery: DerivedDiscoveryBehaviour {
                kademlia: kademlia_opt.into(),
                mdns: mdns_opt.into(),
                identify: identify::Behaviour::new(
                    identify::Config::new("p2pdb/0.0.1".into(), local_public_key.clone())
                        .with_agent_version(format!("p2pdb-{}", network_name))
                        .with_push_listen_addr_updates(true),
                ),
                autonat: autonat::Behaviour::new(local_peer_id, Default::default()),
                upnp: Default::default(),
                rendezvous: rendezvous_opt.into(),
            },
            next_kad_random_query: tokio::time::interval(Duration::from_secs(1)),
            duration_to_next_kad: Duration::from_secs(1),
            pending_events: VecDeque::new(),
            n_node_connected: 0,
            peers,
            peer_info: HashMap::new(),
            target_peer_count,
            custom_seed_peers: user_defined,
            pending_dial_opts: VecDeque::new(),
            rv_namespace: rendezvous::Namespace::new(network_name.to_string()).unwrap(),
            local_public_key: local_public_key.clone(),
            is_rendezvous_started: false,
            rv_discover_retries: -1,
            rv_registation_retries: -1,
            duration_to_next_rv_register_check: Duration::from_secs(10),
            duration_to_next_rv_discover_check: Duration::from_secs(10),
            next_rv_register_check: tokio::time::interval(Duration::from_secs(10)),
            next_rv_discover_check: tokio::time::interval(Duration::from_secs(10)),
            duration_to_next_rv_discovery_interval: Duration::from_secs(10),
            next_rv_discovery_interval: tokio::time::interval(Duration::from_secs(10)),
            rv_cookie: None,
        })
    }
}

/// Implementation of `NetworkBehaviour` that discovers the nodes on the
/// network.
// Behaviours that manage connections should come first, to get rid of some panics in debug build.
// See <https://github.com/libp2p/rust-libp2p/issues/4773#issuecomment-2042676966>
pub struct DiscoveryBehaviour {
    /// Derived discovery discovery.
    discovery: DerivedDiscoveryBehaviour,
    /// Stream that fires when we need to perform the next random Kademlia
    /// query.
    next_kad_random_query: Interval,
    /// After `next_kad_random_query` triggers, the next one triggers after this
    /// duration.
    duration_to_next_kad: Duration,
    /// Events to return in priority when polled.
    pending_events: VecDeque<DiscoveryEvent>,
    /// Number of nodes we're currently connected to.
    n_node_connected: u64,
    /// Keeps hash set of peers connected.
    peers: HashSet<PeerId>,
    /// Keeps hash map of peers and their information.
    peer_info: HashMap<PeerId, PeerInfo>,
    /// Number of connected peers to pause discovery on.
    target_peer_count: u64,
    /// Seed peers
    custom_seed_peers: Vec<(PeerId, Multiaddr)>,
    /// Options to configure dials to known peers.
    pending_dial_opts: VecDeque<DialOpts>,
    /// Namespace
    rv_namespace: rendezvous::Namespace,
    /// For peerId inference
    local_public_key: PublicKey,
    /// Rendezvous started
    is_rendezvous_started: bool,

    /// Retry count of rendezvous discovery
    rv_discover_retries: i64,
    rv_registation_retries: i64,
    /// Registration failed check next rv_register_check
    next_rv_register_check: Interval,
    next_rv_discover_check: Interval,
    /// Duration to next rv_register_check
    duration_to_next_rv_register_check: Duration,
    duration_to_next_rv_discover_check: Duration,

    /// Natural Rendezvous discover interval
    duration_to_next_rv_discovery_interval: Duration,
    next_rv_discovery_interval: Interval,
    rv_cookie: Option<rendezvous::Cookie>,
}

#[derive(Default)]
pub struct PeerInfo {
    pub addresses: HashSet<Multiaddr>,
    pub agent_version: Option<String>,
}

impl DiscoveryBehaviour {
    /// Returns reference to peer set.
    pub fn peers(&self) -> &HashSet<PeerId> {
        &self.peers
    }

    /// Returns a map of peer ids and their multi-addresses
    pub fn peer_addresses(&self) -> HashMap<PeerId, HashSet<Multiaddr>> {
        self.peer_info
            .iter()
            .map(|(peer_id, info)| (*peer_id, info.addresses.clone()))
            .collect()
    }

    pub fn peer_info(&self, peer_id: &PeerId) -> Option<&PeerInfo> {
        self.peer_info.get(peer_id)
    }

    /// Bootstrap Kademlia network
    pub fn bootstrap(&mut self) -> Result<kad::QueryId, String> {
        if let Some(active_kad) = self.discovery.kademlia.as_mut() {
            active_kad.bootstrap().map_err(|e| e.to_string())
        } else {
            // Manually dial to seed peers when kademlia is disabled
            for (peer_id, address) in &self.custom_seed_peers {
                self.pending_dial_opts.push_back(
                    DialOpts::peer_id(*peer_id)
                        .condition(PeerCondition::Disconnected)
                        .addresses(vec![address.clone()])
                        .build(),
                );
            }
            Err("Kademlia is not activated".to_string())
        }
    }

    /// Gets the NAT status.
    pub fn nat_status(&self) -> autonat::NatStatus {
        self.discovery.autonat.nat_status()
    }

    pub fn start_rendezvous(&mut self,) -> bool{
        if self.is_rendezvous_started {
            println!("RV Start event prevented");
            return true;
        }

        self.is_rendezvous_started = true;
        self.rv_registation_retries = 1;
        false
    }
    
}

impl NetworkBehaviour for DiscoveryBehaviour {
    type ConnectionHandler = <DerivedDiscoveryBehaviour as NetworkBehaviour>::ConnectionHandler;
    type ToSwarm = DiscoveryEvent;

    fn handle_established_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.peer_info
            .entry(peer)
            .or_default()
            .addresses
            .insert(remote_addr.clone());
        self.discovery.handle_established_inbound_connection(
            connection_id,
            peer,
            local_addr,
            remote_addr,
        )
    }

    fn handle_established_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        peer: PeerId,
        addr: &libp2p::Multiaddr,
        role_override: libp2p::core::Endpoint,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.peer_info
            .entry(peer)
            .or_default()
            .addresses
            .insert(addr.clone());
        self.discovery.handle_established_outbound_connection(
            connection_id,
            peer,
            addr,
            role_override,
        )
    }

    fn handle_pending_inbound_connection(
        &mut self,
        connection_id: ConnectionId,
        local_addr: &libp2p::Multiaddr,
        remote_addr: &libp2p::Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        self.discovery
            .handle_pending_inbound_connection(connection_id, local_addr, remote_addr)
    }

    fn handle_pending_outbound_connection(
        &mut self,
        connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addresses: &[libp2p::Multiaddr],
        effective_role: libp2p::core::Endpoint,
    ) -> Result<Vec<libp2p::Multiaddr>, ConnectionDenied> {
        self.discovery.handle_pending_outbound_connection(
            connection_id,
            maybe_peer,
            addresses,
            effective_role,
        )
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match &event {
            FromSwarm::ConnectionEstablished(e) => {
                if e.other_established == 0 {
                    self.n_node_connected += 1;
                    self.peers.insert(e.peer_id);
                    self.pending_events
                        .push_back(DiscoveryEvent::PeerConnected(e.peer_id));
                }
            }
            FromSwarm::ConnectionClosed(e) => {
                if e.remaining_established == 0 {
                    self.n_node_connected -= 1;
                    self.peers.remove(&e.peer_id);
                    self.peer_info.remove(&e.peer_id);
                    self.pending_events
                        .push_back(DiscoveryEvent::PeerDisconnected(e.peer_id));
                }
            }
            _ => {}
        };
        self.discovery.on_swarm_event(event)
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        connection: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        self.discovery
            .on_connection_handler_event(peer_id, connection, event);
    }

    #[allow(clippy::type_complexity)]
    fn poll(
        &mut self,
        cx: &mut Context,
    ) -> Poll<ToSwarm<Self::ToSwarm, libp2p::swarm::THandlerInEvent<Self>>> {
        // Immediately process the content of `discovered`.
        if let Some(ev) = self.pending_events.pop_front() {
            return Poll::Ready(ToSwarm::GenerateEvent(ev));
        }

        // Dial to peers
        if let Some(opts) = self.pending_dial_opts.pop_front() {
            return Poll::Ready(ToSwarm::Dial { opts });
        }

        while self.next_rv_register_check.poll_tick(cx).is_ready() {
            if self.rv_registation_retries > 0 {
                self.rv_registation_retries = 0;
                if let Some(rv) = self.discovery.rendezvous.as_mut() {                    
                    match rv.register(self.rv_namespace.clone(), self.custom_seed_peers.first().unwrap().0.clone(), None) {
                        Ok(..) => {println!("new register request out...")},
                        Err(..) => {println!("failed to send register")}
                    };                    
                }
            }
            self.next_rv_register_check = tokio::time::interval(self.duration_to_next_rv_register_check);
            self.next_rv_register_check.reset();
        }

        while self.next_rv_discover_check.poll_tick(cx).is_ready() {
            if self.rv_discover_retries > 0 {
                if let Some(rv) = self.discovery.rendezvous.as_mut() {
                    rv.discover(Some(self.rv_namespace.clone()), None, Some(32), self.custom_seed_peers.first().unwrap().0.clone());   
                }
            }
            self.next_rv_discover_check = tokio::time::interval(self.duration_to_next_rv_discover_check);
            self.next_rv_discover_check.reset();
            
            println!("[Status] peer-count={:?};  discover-retries={:?};  register-retries={:?};",self.n_node_connected,self.rv_discover_retries,self.rv_registation_retries);
        }

        while self.next_rv_discovery_interval.poll_tick(cx).is_ready() {
            // Only if everything is fine and dandy
            if self.rv_discover_retries == 0 {                
                if let Some(rv) = self.discovery.rendezvous.as_mut() {
                    rv.discover(Some(self.rv_namespace.clone()), self.rv_cookie.clone(), None, self.custom_seed_peers.first().unwrap().0.clone())
                }
            }

            self.next_rv_discovery_interval = tokio::time::interval(self.duration_to_next_rv_discovery_interval);
            self.next_rv_discovery_interval.reset();
        }

        // Poll the stream that fires when we need to start a random Kademlia query.
        while self.next_kad_random_query.poll_tick(cx).is_ready() {
            if self.n_node_connected < self.target_peer_count {
                // We still have not hit the discovery max, send random request for peers.
                let random_peer_id = PeerId::random();
                /*println!(
                    "Libp2p <= Starting random Kademlia request for {:?}",
                    random_peer_id
                );*/
                if let Some(kademlia) = self.discovery.kademlia.as_mut() {
                    kademlia.get_closest_peers(random_peer_id);
                }
            }

            // Schedule the next random query with exponentially increasing delay,
            // capped at 60 seconds.
            self.next_kad_random_query = tokio::time::interval(self.duration_to_next_kad);
            // we need to reset the interval, otherwise the next tick completes immediately.
            self.next_kad_random_query.reset();

            self.duration_to_next_kad =
                cmp::min(self.duration_to_next_kad * 2, Duration::from_secs(60));
        }

        // Poll discovery events.
        while let Poll::Ready(ev) = self.discovery.poll(cx) {
            match ev {
                ToSwarm::GenerateEvent(ev) => {
                    match &ev {
                        DerivedDiscoveryBehaviourEvent::Identify(ev) => {
                            if let identify::Event::Received { peer_id, info } = ev {
                                self.peer_info.entry(*peer_id).or_default().agent_version =
                                    Some(info.agent_version.clone());
                                if let Some(kademlia) = self.discovery.kademlia.as_mut() {
                                    for address in &info.listen_addrs {
                                        kademlia.add_address(peer_id, address.clone());
                                    }
                                }

                                // Check if identified node has rendezvous server
                                // protocol enabled if so call discover and register
                                if self.custom_seed_peers.iter().find(|peer| {peer.0 == peer_id.clone()}).is_some() {
                                    println!("Identified: rendezvous-server {:?}",peer_id.clone());
                                    self.rv_discover_retries = 0;
                                    return Poll::Ready(ToSwarm::ListenOn { opts: ListenOpts::new(info.observed_addr.clone() ) });
                                }
                            }
                        }
                        DerivedDiscoveryBehaviourEvent::Autonat(_) => {}
                        DerivedDiscoveryBehaviourEvent::Upnp(ev) => match ev {
                            upnp::Event::NewExternalAddr(addr) => {
                                println!("UPnP NewExternalAddr: {addr}");
                            }
                            upnp::Event::ExpiredExternalAddr(addr) => {
                                println!("UPnP ExpiredExternalAddr: {addr}");
                            }
                            upnp::Event::GatewayNotFound => {
                                println!("UPnP GatewayNotFound");
                            }
                            upnp::Event::NonRoutableGateway => {
                                println!("UPnP NonRoutableGateway");
                            }
                        },
                        DerivedDiscoveryBehaviourEvent::Kademlia(ev) => match ev {
                            // Adding to Kademlia buckets is automatic with our config,
                            // no need to do manually.
                            kad::Event::RoutingUpdated { .. } => {}
                            kad::Event::RoutablePeer { .. } => {}
                            kad::Event::PendingRoutablePeer { .. } => {
                                // Intentionally ignore
                            }
                            _ => {
                                //println!("Libp2p => Unhandled Kademlia event: {:?}", _)
                            }
                        },
                        DerivedDiscoveryBehaviourEvent::Mdns(ev) => match ev {
                            MdnsEvent::Discovered(list) => {
                                if self.n_node_connected >= self.target_peer_count {
                                    // Already over discovery max, don't add discovered peers.
                                    // We could potentially buffer these addresses to be added later,
                                    // but mdns is not an important use case and may be removed in future.
                                    continue;
                                }

                                // Add any discovered peers to Kademlia
                                for (peer_id, multiaddr) in list {
                                    if let Some(kad) = self.discovery.kademlia.as_mut() {
                                        kad.add_address(peer_id, multiaddr.clone());
                                    }
                                }
                            }
                            MdnsEvent::Expired(_) => {}
                        },
                        DerivedDiscoveryBehaviourEvent::Rendezvous(ev) => match ev {
                            rendezvous::client::Event::Discovered { registrations, cookie, .. } => {

                                self.rv_discover_retries = 0;
                                
                                self.rv_cookie = Some(cookie.clone());

                                for registration in registrations {
                                    if registration.record.peer_id() == self.local_public_key.to_peer_id() {
                                        continue;
                                    }

                                    if let Some(kad) = self.discovery.kademlia.as_mut() {
                                        println!("Added address to KAD {:?} {:?}",registration.record.peer_id().clone(),registration.record.addresses().first().unwrap().clone());
                                        kad.add_address(&registration.record.peer_id(),registration.record.addresses().first().unwrap().clone());
                                        self.peers.insert(registration.record.peer_id().clone());

                                        self.pending_dial_opts.push_back(
                                            DialOpts::peer_id(registration.record.peer_id().clone())
                                                .condition(PeerCondition::Disconnected)
                                                .addresses(vec![registration.record.addresses().first().unwrap().clone()])
                                                .build(),
                                        );

                                        println!("Rendezvous Discovered - {:?}, {:?}",registration.record.peer_id().clone(),registration.record.addresses().first().clone());
                                    }
                                }
                            },
                            rendezvous::client::Event::DiscoverFailed { rendezvous_node, .. } => {
                                
                                    println!("[!] Rendezvous DiscoverFailed - {:?}",rendezvous_node.clone());

                                    // Max wait time after backoff is 1 min between retries
                                    if self.rv_discover_retries < 6 { self.rv_discover_retries += 1; }

                                    self.next_rv_discover_check = tokio::time::interval(self.duration_to_next_rv_discover_check);
                                    self.next_rv_discover_check.reset();
                                    self.rv_cookie = None;
                            },
                            rendezvous::client::Event::Registered { rendezvous_node, .. } => {
                                println!("Rendezvous Registered - {:?}",rendezvous_node.clone());
                                self.rv_registation_retries = 0;
                            },
                            rendezvous::client::Event::RegisterFailed { rendezvous_node, .. } => {
                                if let Some(rv) = self.discovery.rendezvous.as_mut() {
                                    println!("Rendezvous RegisterFailed - {:?}",rendezvous_node.clone());      
                                    
                                    self.rv_registation_retries += 1;
                                    self.next_rv_register_check = tokio::time::interval(self.duration_to_next_rv_register_check);
                                    self.next_rv_register_check.reset();
                                }
                            },
                            rendezvous::client::Event::Expired { peer } => {
                                println!("Rendezvous Expired - {:?}",peer.clone());

                                if peer.to_base58() == self.local_public_key.to_peer_id().to_base58() { 
                                    println!("Re-registering to rendezvous...");
                                    self.rv_registation_retries += 1;
                                }
                            },
                        },
                    }
                    self.pending_events
                        .push_back(DiscoveryEvent::Discovery(Box::new(ev)));
                }
                ToSwarm::Dial { opts } => {
                    return Poll::Ready(ToSwarm::Dial { opts });
                }
                ToSwarm::NotifyHandler {
                    peer_id,
                    handler,
                    event,
                } => {
                    return Poll::Ready(ToSwarm::NotifyHandler {
                        peer_id,
                        handler,
                        event,
                    })
                }
                ToSwarm::CloseConnection {
                    peer_id,
                    connection,
                } => {
                    return Poll::Ready(ToSwarm::CloseConnection {
                        peer_id,
                        connection,
                    })
                }
                ToSwarm::ListenOn { opts } => {println!("ListenOn: {:?}",opts); return Poll::Ready( ToSwarm::ListenOn { opts })},
                ToSwarm::RemoveListener { id } => {
                    return Poll::Ready(ToSwarm::RemoveListener { id })
                }
                ToSwarm::NewExternalAddrCandidate(addr) => {
                    println!("[NEA-Candidate] {:?}",addr.clone());

                    //return Poll::Ready(ToSwarm::ListenOn { opts: ListenOpts::new(addr.clone() ) })
                    return Poll::Ready(ToSwarm::NewExternalAddrCandidate(addr))
                }
                ToSwarm::ExternalAddrConfirmed(addr) => {
                    println!("[NEA] {:?}",addr.clone());
                    return Poll::Ready(ToSwarm::ExternalAddrConfirmed(addr))
                }
                ToSwarm::ExternalAddrExpired(addr) => {
                    return Poll::Ready(ToSwarm::ExternalAddrExpired(addr))
                }
                _ => {}
            }
        }

        Poll::Pending
    }
}