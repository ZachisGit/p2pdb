use std::{collections::HashMap, error::Error, hash::{Hash,SipHasher}};

use futures::StreamExt;
use libp2p::{self, gossipsub, identify, kad::store::MemoryStore, relay, rendezvous, swarm::{dial_opts::PeerCondition, NetworkBehaviour, SwarmEvent}, upnp, Multiaddr, PeerId, Swarm};
use tokio::time::{self, sleep};



fn message_id_fn(message: &gossipsub::Message) -> gossipsub::MessageId {
    let mut s = SipHasher::new();
    message.data.hash(&mut s);
    gossipsub::MessageId::from(std::hash::Hasher::finish(&s).to_string())
}

pub fn setup_swarm(
    node_keypair: libp2p::identity::Keypair,
) -> Result<Swarm<RendezvousGossipBehaviour>, Box<dyn Error>> {

  // Set a custom gossipsub configuration
  let gossipsub_config = gossipsub::ConfigBuilder::default()
    .heartbeat_interval(std::time::Duration::from_secs(10))
    .validation_mode(gossipsub::ValidationMode::Strict)
    .message_id_fn(message_id_fn)
    .build()
    .expect("Valid config");

    let swarm = libp2p::SwarmBuilder::with_existing_identity(node_keypair.clone())
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::new().port_reuse(true).nodelay(true),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_behaviour(|key| RendezvousGossipBehaviour {
            identify: libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
                "rendezvous-example/1.0.0".to_string(),
                key.clone().public(),
            )),
            rendezvous: libp2p::rendezvous::client::Behaviour::new(key.clone()),
            //rendezvous_server: libp2p::rendezvous::server::Behaviour::new(rendezvous::server::Config::default()),
            pubsub: libp2p::gossipsub::Behaviour::new(gossipsub::MessageAuthenticity::Signed(key.clone()), gossipsub_config).unwrap(),
            kademlia: libp2p::kad::Behaviour::new(node_keypair.public().to_peer_id(), MemoryStore::new(node_keypair.public().to_peer_id()))
        })?
        .with_swarm_config(|cfg| {
            cfg.with_idle_connection_timeout(std::time::Duration::from_secs(365 * 24 * 60 * 60))
        })
        .build();
    Ok(swarm)
}


// Define and implement a trait for Swarm<RendezvousGossipBehaviour>
pub trait Spinup {
    fn _register_inc_failures(&mut self, peer: libp2p::PeerId, registration_failures: &mut std::collections::HashMap<libp2p::PeerId, std::time::Instant>,namespace: &str);
    fn _extract_base_multiaddr(addr: &Multiaddr) -> Multiaddr;
    fn _extract_peer_id(addr: &Multiaddr) -> Option<libp2p::PeerId>;

    async fn spinup(&mut self, namespace: String, keypair: libp2p::identity::Keypair, cluster_keypair: libp2p::identity::Keypair, rendezvous_address: Multiaddr) -> Result<(), Box<dyn Error>>;
}


impl Spinup for Swarm<RendezvousGossipBehaviour> {
    async fn spinup(&mut self,namespace: String, keypair: libp2p::identity::Keypair, cluster_keypair: libp2p::identity::Keypair,rendezvous_address: Multiaddr) -> Result<(), Box<dyn Error>> {

        let topic = gossipsub::IdentTopic::new(namespace.clone());
        let listener_address: Multiaddr = format!("/ip4/0.0.0.0/tcp/0").parse::<Multiaddr>().unwrap();
        let rendezsvous_peer_id: libp2p::PeerId = <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap();
        
        self.dial(rendezvous_address.clone()).unwrap();
        let _ = self.listen_on(listener_address);
        let mut is_pub_listener_address_set: bool = false;
        let mut is_first_node_connection: bool = false;
        
        let mut cookie_cache: Option<rendezvous::Cookie> = None;
        let mut discovered_peers: std::collections::HashSet<PeerId> = std::collections::HashSet::new();
        let local_peer_id = keypair.public().to_peer_id();

        // Define a dict that maps a peer to its registration failure count
        let mut registration_failures: HashMap<libp2p::PeerId,std::time::Instant> = std::collections::HashMap::new();
        loop {
            tokio::select! {
                event = self.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished { peer_id,.. } => {
                       
                        println!("Connection established with: {}",peer_id.clone());
                        if !is_first_node_connection  { 
                            self.behaviour_mut().kademlia.add_address(&rendezsvous_peer_id,rendezvous_address.clone());
                            self.behaviour_mut().kademlia.bootstrap().unwrap();
                            is_first_node_connection = true;
                        }
                    },
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        println!("[ConnectionClosed] {:?}",peer_id);
                        discovered_peers.remove(&peer_id);

                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                            println!("{:?}",self.network_info());
                            
                            match self.dial(peer_id.clone()) {
                                Ok {..} => {println!("Dial good!"); break; },
                                Err{..} => {println!("Dial failed!")}
                            }

                            if self.connected_peers().collect::<Vec<_>>().contains(&&peer_id) {
                                println!("Braking out of hell!");
                                break;
                            }
                        }

                    },
                    SwarmEvent::OutgoingConnectionError { peer_id,error,connection_id } => {
                        
                        println!("[OutgoingConnectionError] {:?}, {:?}",peer_id.clone(),connection_id);
                        discovered_peers.remove(&peer_id.unwrap().clone());
                        match peer_id {
                            Some(_) => {
                                let discon_peer_id = peer_id.unwrap();

                                loop {
                                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                    println!("{:?}",self.network_info());
                                    
                                    match self.dial(discon_peer_id.clone()) {
                                        Ok {..} => {println!("Dial good!"); break; },
                                        Err{..} => {println!("Dial failed!")}
                                    }

                                    if self.connected_peers().collect::<Vec<_>>().contains(&&discon_peer_id) {
                                        println!("Braking out of hell!");
                                        break;
                                    }
                                }
                            },
                            None => {},
                        }
                    },
                    SwarmEvent::IncomingConnectionError { send_back_addr,.. } => {
                        println!("[IncomingConnectionError] {:?}",send_back_addr);
                    },
                    SwarmEvent::NewExternalAddrCandidate { address} => {
                        println!("Adding external address!: {:?}", address.clone());
                        self.add_external_address(address.clone());
                        
                        let _ = self.listen_on(address.clone());
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Registered {
                        rendezvous_node,namespace,..
                    })) => {
                        println!("[Registered] {:?} {:?}",rendezvous_node,namespace);
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::RegisterFailed {
                        rendezvous_node,error,..
                    })) => {
                        println!("RegisterFailed {}; {:?}",rendezvous_node.clone(),error);
                        
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        self.dial(rendezvous_address.clone()).unwrap();
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        self.behaviour_mut().rendezvous.register(
                            rendezvous::Namespace::new(namespace.to_string()).unwrap(),                            
                            <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(),
                            std::default::Default::default(),
                        ).unwrap();                       
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Discovered {
                        cookie, registrations,rendezvous_node,..
                    })) => {

                        println!("[Peers]:");
                        for peer in self.connected_peers() {
                            println!("- {:?}",peer.clone())
                        }
                        println!("");

                        
                        let mut new_peer: bool = false;
                        for registration in registrations {
                            let address = registration.record.addresses().first().unwrap().clone();
                            
                            let peer = registration.record.peer_id().clone();
                            if peer.clone() == keypair.public().to_peer_id().clone() {
                                continue;
                            }
                            
                            if !discovered_peers.contains(&peer.clone()) && peer != keypair.public().to_peer_id() {
                                discovered_peers.insert(peer.clone());
                                println!("Discovered: {} - {}",peer.clone(), address.clone());
                                new_peer = true;

                                if peer != keypair.clone().public().to_peer_id() {
                                    println!("[DIAL] {:?}",address.clone());
                                    self.dial(address.clone()).unwrap();
                                }
                            }
                        }

                        if !new_peer {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        } else {
                            cookie_cache.replace(cookie.clone());
                        }
                        
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            Some(cookie.clone()),
                            None,
                            <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(),
                        );

                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::DiscoverFailed {
                        error,rendezvous_node,..
                    })) => {

                        println!("Discover failed: {:?}, {:?}",error,rendezvous_node);
                        
                        //tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        //self.dial(rendezvous_address.clone()).unwrap();
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        
                        println!("{:?}",self.network_info());
                        /* 
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            None, //cookie_cache.clone(),
                            None,
                            <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(),
                        );*/                        
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Expired {
                        peer,..
                    })) => {
                        println!("Expired: {:?}",peer);
                        
                        self.behaviour_mut().rendezvous.register(
                            rendezvous::Namespace::new(namespace.to_string()).unwrap(),                            
                            <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(),
                            std::default::Default::default(),
                        ).unwrap();
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Pubsub(libp2p::gossipsub::Event::Message {
                        message, ..
                    })) => {
                        println!("[PS] MSG: {:?}", String::from_utf8_lossy(&message.data));
                        //self.behaviour_mut().pubsub.publish(topic.clone(), b"I am Alive!").unwrap();
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Pubsub(libp2p::gossipsub::Event::Subscribed {
                        ..
                    })) => {
                        println!("[PS] Sending first message {:?}", topic);
                        //self.behaviour_mut().pubsub.publish(topic.clone(), b"Hello World").unwrap();
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Identify(identify::Event::Received {
                        peer_id,info,..
                    })) => {

                        println!("Identified {:?}",info.observed_addr.clone());
                        println!("[EXT] {:?}",<Multiaddr as std::str::FromStr>::from_str(&format!("{}/p2p/{}",info.observed_addr,keypair.clone().public().to_peer_id().to_string())).unwrap());
                        
                        if peer_id != keypair.clone().public().to_peer_id() {
                            println!("[ADD] {:?}",peer_id.clone());
                            //self.add_peer_address(peer_id.clone(),info.listen_addrs.first().unwrap().clone());
                        }                        

                        if !is_pub_listener_address_set && peer_id == <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap() {

                            //self.add_external_address(<Multiaddr as std::str::FromStr>::from_str(&format!("{}/p2p/{}",info.observed_addr,keypair.clone().public().to_peer_id().to_string())).unwrap());
                            self.add_external_address(info.observed_addr.clone());
                            println!("[LISTEN] {:?}",peer_id.clone());
                            let _ = self.listen_on(info.observed_addr.clone());
                            is_pub_listener_address_set = true;
                            self.behaviour_mut().pubsub.subscribe(&topic).unwrap();

                            
                            self.behaviour_mut().rendezvous.register(
                                rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                                <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(),
                                std::default::Default::default(),
                            ).unwrap();
                            
                            self.behaviour_mut().rendezvous.discover(
                                Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                                None, //[!]cookie_cache.clone(),
                                None,
                                <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(),
                            );
                        }
                        else if peer_id == rendezsvous_peer_id.clone() {
                            println!("IMPORTANT: {:?}",peer_id.clone());
                            self.behaviour_mut().rendezvous.register(
                                rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                                rendezsvous_peer_id.clone(),
                                std::default::Default::default(),
                            ).unwrap();
                            
                            self.behaviour_mut().rendezvous.discover(
                                Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                                None, //[!]cookie_cache.clone(),
                                None,
                                rendezsvous_peer_id.clone(),
                            );
                        }
                    }
                    others => {
                        println!("OTHERS: {:?};",others);
                    }
                }
            }
        }
        Ok(())
    }

    fn _register_inc_failures(&mut self, peer: libp2p::PeerId, registration_failures: &mut std::collections::HashMap<libp2p::PeerId, std::time::Instant>, namespace: &str) {
        /*
        if !registration_failures.contains_key(&peer) || registration_failures[&peer] < std::time::Instant::now() - std::time::Duration::from_secs(10){ 

            self.behaviour_mut().rendezvous.register(
                rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                peer.clone(), 
                std::default::Default::default(),
            ).unwrap();
        }*/
    }

    fn _extract_base_multiaddr(addr: &Multiaddr) -> Multiaddr {
        let mut base_addr = Multiaddr::empty();
        for protocol in addr.iter() {
            if protocol == libp2p::multiaddr::Protocol::P2p(libp2p::PeerId::random().into()) {
                break;
            }
            base_addr.push(protocol);
        }
        base_addr
    }

    fn _extract_peer_id(addr: &Multiaddr) -> Option<libp2p::PeerId> {
        for protocol in addr.iter() {
            if let libp2p::multiaddr::Protocol::P2p(peer_id) = protocol {
                return Some(libp2p::PeerId::from(peer_id));
            }
        }
        None
    }
}


#[derive(NetworkBehaviour)]
pub struct RendezvousGossipBehaviour {
    identify: identify::Behaviour,
    rendezvous: rendezvous::client::Behaviour,
    pubsub: gossipsub::Behaviour,
    kademlia: libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>
}