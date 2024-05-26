use std::{collections::HashMap, error::Error, hash::{Hash,SipHasher}, str::FromStr};

use futures::StreamExt;
use libp2p::{self, autonat::NatStatus, gossipsub, identify, kad::store::MemoryStore, relay, rendezvous, swarm::{dial_opts::PeerCondition, ListenOpts, NetworkBehaviour, SwarmEvent, ToSwarm}, upnp, Multiaddr, PeerId, Swarm};
use tokio::time::{self, sleep};

use crate::network::discovery;


fn message_id_fn(message: &gossipsub::Message) -> gossipsub::MessageId {
    let mut s = SipHasher::new();
    message.data.hash(&mut s);
    gossipsub::MessageId::from(std::hash::Hasher::finish(&s).to_string())
}

pub fn setup_swarm(
    node_keypair: libp2p::identity::Keypair,
    rendezvous_nodes: Vec<Multiaddr>
) -> Result<Swarm<RendezvousGossipBehaviour>, Box<dyn Error>> {
    
    let swarm = libp2p::SwarmBuilder::with_existing_identity(node_keypair.clone())
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::new().port_reuse(true).nodelay(true),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_behaviour(|key| RendezvousGossipBehaviour {
            discovery: discovery::DiscoveryConfig::new(key.clone(), node_keypair.public(), "p2pdb-testnet")
            .with_mdns(false)
            .with_kademlia(true)
            .with_rendezvous(true)
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
    async fn spinup(&mut self, namespace: String, keypair: libp2p::identity::Keypair, cluster_keypair: libp2p::identity::Keypair, rendezvous_address: Multiaddr) -> Result<(), Box<dyn Error>>;
}


impl Spinup for Swarm<RendezvousGossipBehaviour> {
    async fn spinup(&mut self,namespace: String, keypair: libp2p::identity::Keypair, cluster_keypair: libp2p::identity::Keypair,rendezvous_address: Multiaddr) -> Result<(), Box<dyn Error>> {
        
        let _ = self.listen_on(Multiaddr::from_str("/ip4/0.0.0.0/tcp/0").unwrap());
        self.behaviour_mut().discovery.bootstrap().unwrap();
        let mut first_connect = false;

        loop {
            tokio::select! {
                event = self.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished {peer_id, ..} => {
                        println!("Connection established with: {:?}", peer_id);
                    },
                    SwarmEvent::NewExternalAddrCandidate { address } => {
                        println!("External address candidate: {:?}", address.clone());
                        if matches!(self.behaviour_mut().discovery.nat_status(), libp2p::autonat::NatStatus::Private |  libp2p::autonat::NatStatus::Unknown) {
                            println!("NAT identifed, manualy starting listener on open NAT address {:?}",address.clone());
                            
                        //}
                        //if !first_connect {
                            println!("First");
                            //first_connect=true;
                            self.add_external_address(address.clone());
                            let _ = self.listen_on(address.clone());
                            match self.behaviour_mut().discovery.start_rendezvous() { 
                                true => { println!("Rendezvous started. {:?}",address.clone()); },
                                false => { println!("Failed to start rendezvous."); }
                            }
                        }
                    },
                    SwarmEvent::ExternalAddrConfirmed  { address } => {
                        println!("New External address confirmed: {:?}",address.clone());
                    },
                    SwarmEvent::NewListenAddr { address,.. } => {
                        println!("New listen Address: {:?}",address.clone());
                        
                    },
                    others => {
                        //println!("[E]: {:?};",others);
                    }
                }
            }
        }
        Ok(())
    }
}


#[derive(NetworkBehaviour)]
pub struct RendezvousGossipBehaviour {
    discovery: discovery::DiscoveryBehaviour,
}