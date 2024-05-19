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
            rendezvous_server: libp2p::rendezvous::server::Behaviour::new(rendezvous::server::Config::default()),
            pubsub: libp2p::gossipsub::Behaviour::new(gossipsub::MessageAuthenticity::Signed(key.clone()), gossipsub_config).unwrap(),
            upnp: upnp::tokio::Behaviour::default(),
        })?
        .with_swarm_config(|cfg| {
            cfg
        })
        .build();
    Ok(swarm)
}


// Define and implement a trait for Swarm<RendezvousGossipBehaviour>
pub trait Spinup {
    fn _register_inc_failures(&mut self, peer: libp2p::PeerId, registration_failures: &mut std::collections::HashMap<libp2p::PeerId, std::time::Instant>,namespace: &str);
    async fn spinup(&mut self, namespace: String, keypair: libp2p::identity::Keypair, cluster_keypair: libp2p::identity::Keypair, rendezvous_address: Multiaddr) -> Result<(), Box<dyn Error>>;
}


impl Spinup for Swarm<RendezvousGossipBehaviour> {
    async fn spinup(&mut self,namespace: String, keypair: libp2p::identity::Keypair, cluster_keypair: libp2p::identity::Keypair,rendezvous_address: Multiaddr) -> Result<(), Box<dyn Error>> {

        let topic = gossipsub::IdentTopic::new(namespace.clone());


        let listener_address: Multiaddr = format!("/ip4/0.0.0.0/tcp/53748").parse::<Multiaddr>().unwrap();
        self.add_external_address(rendezvous_address.clone());
        self.dial(rendezvous_address.clone()).unwrap();
        let _ = self.listen_on(listener_address);

        let mut cookie_cache: Option<rendezvous::Cookie> = None;

        // Define a dict that maps a peer to its registration failure count
        let mut registration_failures: HashMap<libp2p::PeerId,std::time::Instant> = std::collections::HashMap::new();
        loop {
            tokio::select! {
                event = self.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint,.. } => {
                       
                        println!("{}",peer_id.clone());
                        

                        //self.behaviour_mut().pubsub.publish(topic.clone(), b"First MSG").unwrap();
                        println!("ConnectionEstablished");
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Registered {
                        rendezvous_node,..
                    })) => {
                        if registration_failures.contains_key(&rendezvous_node) { 
                            registration_failures.remove(&rendezvous_node);
                        }

                        println!("Registered to {}",rendezvous_node.clone());
                        println!("Discovering {}",namespace.clone());
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::RegisterFailed {
                        rendezvous_node,..
                    })) => {
                        println!("RegisterFailed {}",rendezvous_node.clone());
                        self._register_inc_failures(rendezvous_node.clone(), &mut registration_failures, &namespace);

                        if let Some(failure) = registration_failures.get_mut(&rendezvous_node) 
                        { 
                            *failure = std::time::Instant::now(); 
                        } else 
                        { 
                            registration_failures.insert(rendezvous_node.clone(), std::time::Instant::now());
                        }
                        
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Discovered {
                        cookie, registrations,..
                    })) => {

                        println!("Discovered");
                        cookie_cache.replace(cookie.clone());

                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            cookie_cache.clone(),
                            None,
                            keypair.public().to_peer_id(),
                        );


                        for registration in registrations {
                            for address in registration.record.addresses() {
                                let peer = registration.record.peer_id().clone();
                                
                                println!("Discovered: {} - {}",peer.clone(), address.clone());
                                
                                self.add_external_address(address.clone());
                                self.dial(address.clone()).unwrap();
                                
                            }
                        }

                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::DiscoverFailed {
                        ..
                    })) => {
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            None,
                            None,
                            keypair.public().to_peer_id(),
                        );                        
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Expired {
                        peer,..
                    })) => {
                        self._register_inc_failures(peer.clone(), &mut registration_failures, &namespace);
                        
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            cookie_cache.clone(),
                            None,
                            keypair.public().to_peer_id(),
                        );
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Pubsub(libp2p::gossipsub::Event::Message {
                        message, ..
                    })) => {
                        println!("{}", String::from_utf8_lossy(&message.data));
                        self.behaviour_mut().pubsub.publish(topic.clone(), b"I am Alive!").unwrap();
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Identify(identify::Event::Received {
                        peer_id,..
                    })) => {
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            None,
                            None,
                            peer_id.clone(),
                        );


                        //self._register_inc_failures(&);
                        self.behaviour_mut().rendezvous.register(
                            rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                            peer_id.clone(),
                            std::default::Default::default(),
                        ).unwrap();
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn _register_inc_failures(&mut self, peer: libp2p::PeerId, registration_failures: &mut std::collections::HashMap<libp2p::PeerId, std::time::Instant>, namespace: &str) {
        if !registration_failures.contains_key(&peer) || registration_failures[&peer] < std::time::Instant::now() - std::time::Duration::from_secs(10){ 

            self.behaviour_mut().rendezvous.register(
                rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                peer.clone(), 
                std::default::Default::default(),
            ).unwrap();
        }
    }
}


#[derive(NetworkBehaviour)]
pub struct RendezvousGossipBehaviour {
    identify: identify::Behaviour,
    rendezvous: rendezvous::client::Behaviour,
    rendezvous_server: rendezvous::server::Behaviour,
    pubsub: gossipsub::Behaviour,
    upnp: upnp::tokio::Behaviour
}
