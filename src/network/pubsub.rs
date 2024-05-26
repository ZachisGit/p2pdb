use std::{collections::HashMap, error::Error, hash::{Hash,SipHasher}};

use futures::StreamExt;
use libp2p::{self, upnp, gossipsub, identify, relay,rendezvous, swarm::{NetworkBehaviour, SwarmEvent}, Multiaddr, Swarm};



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


    let store = libp2p::kad::store::MemoryStore::new(node_keypair.public().to_peer_id().clone());
        let kad_config = {
            let mut cfg = libp2p::kad::Config::default();
            cfg.set_protocol_names(vec![libp2p::StreamProtocol::try_from_owned(format!(
                "/openp2p/openrendezvous/1.0.0"
            ))?]);
            cfg
        };
    let kademlia_opt = {
        let mut kademlia = libp2p::kad::Behaviour::with_config(node_keypair.public().to_peer_id().clone(), store, kad_config);
        // `set_mode(Server)` fixes https://github.com/ChainSafe/forest/issues/3620
        // but it should not be required as the behaviour should automatically switch to server mode
        // according to the doc. It might be a bug in `libp2p`.
        // We should fix the bug or report with a minimal reproduction.
        kademlia.set_mode(Some(libp2p::kad::Mode::Server));
        kademlia
    };

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
            rendezvous_server: libp2p::rendezvous::server::Behaviour::new(rendezvous::server::Config::default()),
            upnp: upnp::tokio::Behaviour::default(),
            kad: kademlia_opt.into(),
        })?
        .with_swarm_config(|cfg| {
            cfg.with_idle_connection_timeout(std::time::Duration::from_secs(365 * 24 * 60 * 60))
        })
        .build();
    Ok(swarm)
}


// Define and implement a trait for Swarm<RendezvousGossipBehaviour>
pub trait Spinup {
    async fn spinup(&mut self, keypair: libp2p::identity::Keypair) -> Result<(), Box<dyn Error>>;
}


impl Spinup for Swarm<RendezvousGossipBehaviour> {
    async fn spinup(&mut self,keypair: libp2p::identity::Keypair) -> Result<(), Box<dyn Error>> {

        let listener_address = format!("/ip4/0.0.0.0/tcp/53748").parse::<Multiaddr>().unwrap();
        let _ = self.listen_on(listener_address);
        let mut first_connection = true;


        // Define a dict that maps a peer to its registration failure count
        let mut registration_failures: HashMap<libp2p::PeerId,std::time::Instant> = std::collections::HashMap::new();
        loop {
            tokio::select! {
                event = self.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint,.. } => {
                        println!("ConnectionEstablished: client={};",peer_id);
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                        println!("Identify: client={};",peer_id);
                        
                        if first_connection {
                            first_connection = false;
                            self.behaviour_mut().kad.add_address(&peer_id, info.listen_addrs.first().unwrap().clone());
                            self.behaviour_mut().kad.bootstrap().unwrap();
                        }
                    },
                    others => {
                        //println!("OTHERS: {:?};",others);
                    }                }
            }
        }
        Ok(())
    }
}


#[derive(NetworkBehaviour)]
pub struct RendezvousGossipBehaviour {
    identify: identify::Behaviour,
    rendezvous_server: rendezvous::server::Behaviour,
    upnp: upnp::tokio::Behaviour,
    kad: libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>,
}