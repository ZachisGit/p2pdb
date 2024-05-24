use std::{collections::HashMap, error::Error, hash::{Hash,SipHasher}};

use futures::StreamExt;
use libp2p::{self, gossipsub, identify, kad::store::MemoryStore, relay, rendezvous, swarm::{dial_opts::PeerCondition, NetworkBehaviour, SwarmEvent}, upnp, Multiaddr, PeerId, Swarm};
use tokio::time::{self, sleep};

use crate::network::discovery::DiscoveryBehaviour;


fn message_id_fn(message: &gossipsub::Message) -> gossipsub::MessageId {
    let mut s = SipHasher::new();
    message.data.hash(&mut s);
    gossipsub::MessageId::from(std::hash::Hasher::finish(&s).to_string())
}

pub fn setup_swarm(
    node_keypair: libp2p::identity::Keypair,
    rendezvous_nodes: Vec<Multiaddr>
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
            discovery: super::discovery::DiscoveryConfig::new(node_keypair.public(), "calibnet")
            .with_mdns(false)
            .with_kademlia(true)
            .with_user_defined(rendezvous_nodes.clone())
            .unwrap()
            .target_peer_count(128)
            .finish()
            .unwrap()
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
        
        self.behaviour_mut().discovery.bootstrap().unwrap();

        loop {
            tokio::select! {
                event = self.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished {peer_id, ..} => {
                        println!("Connection established with: {:?}", peer_id);
                        println!("NetStatus: {:?}",self.behaviour().discovery.nat_status());
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
    discovery: DiscoveryBehaviour
}