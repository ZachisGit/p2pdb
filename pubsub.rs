use std::{collections::HashMap, error::Error, hash::{Hash,SipHasher}};

use futures::StreamExt;
use libp2p::{self, gossipsub, identify, relay, rendezvous, swarm::{dial_opts::PeerCondition, NetworkBehaviour, SwarmEvent}, upnp, Multiaddr, Swarm};
use tokio::time::sleep;



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
            upnp: upnp::tokio::Behaviour::default(),
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
        self.dial(rendezvous_address.clone()).unwrap();
        let _ = self.listen_on(listener_address);
        let mut is_pub_listener_address_set: bool = false;
        
        let mut cookie_cache: Option<rendezvous::Cookie> = None;
        let mut discovered_peers: std::collections::HashSet<Multiaddr> = std::collections::HashSet::new();

        // Define a dict that maps a peer to its registration failure count
        let mut registration_failures: HashMap<libp2p::PeerId,std::time::Instant> = std::collections::HashMap::new();
        loop {
            tokio::select! {
                event = self.select_next_some() => match event {
                    SwarmEvent::ConnectionEstablished { peer_id,.. } => {
                       
                        println!("Connection established with: {}",peer_id.clone());
                        

                        //self.behaviour_mut().pubsub.publish(topic.clone(), b"First MSG").unwrap();
                        println!("ConnectionEstablished");
                    },
                    SwarmEvent::ConnectionClosed { peer_id, .. } => {
                        if peer_id.to_string() == "12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn" {
                            
                            println!("[!!] Lost connection to rendezvous server!");
                            sleep(std::time::Duration::from_secs(5)).await;
                            let _ = self.dial(rendezvous_address.clone());
                        }
                    },
                    SwarmEvent::OutgoingConnectionError { .. } => {
                        sleep(std::time::Duration::from_secs(5)).await;
                        let _ = self.dial(rendezvous_address.clone());
                    },
                    SwarmEvent::IncomingConnectionError { .. } => {
                        sleep(std::time::Duration::from_secs(5)).await;
                        let _ = self.dial(rendezvous_address.clone());
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
                        rendezvous_node,error,..
                    })) => {
                        println!("RegisterFailed {}; {:?}",rendezvous_node.clone(),error);
                        
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                        self.behaviour_mut().rendezvous.register(
                            rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                            rendezvous_node.clone(),
                            std::default::Default::default(),
                        ).unwrap();

                        /*
                        //self._register_inc_failures(rendezvous_node.clone(), &mut registration_failures, &namespace);

                        if let Some(failure) = registration_failures.get_mut(&rendezvous_node) 
                        { 
                            *failure = std::time::Instant::now(); 
                        } else 
                        { 
                            registration_failures.insert(rendezvous_node.clone(), std::time::Instant::now());
                        }
                        */
                        
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Discovered {
                        cookie, registrations,rendezvous_node,..
                    })) => {

                        
                        if registration_failures.contains_key(&rendezvous_node) { 
                            registration_failures.remove(&rendezvous_node);
                        }

                        println!("[Peers]:");
                        for peer in self.connected_peers() {
                            println!("- {:?}",peer.clone())
                        }
                        println!("");


                        cookie_cache.replace(cookie.clone());
                        
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            None, //[!] cookie_cache.clone(),
                            None,
                            rendezvous_node.clone(),
                        );
                        

                        let mut new_peer: bool = false;
                        for registration in registrations {
                            let address = registration.record.addresses().first().unwrap().clone();
                            
                            let peer = registration.record.peer_id().clone();
                            if peer.clone() == keypair.public().to_peer_id().clone() {
                                continue;
                            }
                            
                            if discovered_peers.insert(address.clone()) {
                            //discovered_peers.insert(address.clone());
                            //let peer_ids: Vec<libp2p::PeerId> = self.connected_peers().cloned().collect();
                            //if !peer_ids.contains(&peer) {
                                //if peer.clone() == <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWDkBFGbXVAGzX6Z3Wa94D9FHHJnotxzrEWJ6r7n8s6S8P").unwrap() {
                                //    new_peer = true;
                                //    println!("Special dial!");
                                //    self.dial(<Multiaddr as std::str::FromStr>::from_str("/ip4/65.109.14.113/tcp/53748/p2p/12D3KooWDkBFGbXVAGzX6Z3Wa94D9FHHJnotxzrEWJ6r7n8s6S8P").unwrap()).unwrap();
                                //} else {
                                    println!("Discovered: {} - {}",peer.clone(), address.clone());
                                    new_peer = true;
                                    
                                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                                    println!("DIAL_RES: {:?}", self.dial(address.clone()).unwrap());
                                    self.add_peer_address(peer.clone(), address.clone());



                                    
                                    //address_split = address.to_string().unwrap().

                                    //self.add_external_address(address.clone());
                                //}
                            }
                        }

                        if !new_peer {
                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        }

                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::DiscoverFailed {
                        error,rendezvous_node,..
                    })) => {

                        println!("Discover failed: {:?}, {:?}",error,rendezvous_node);
                        
                        
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            None, //cookie_cache.clone(),
                            None,
                            keypair.public().to_peer_id(),
                        );                        
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Rendezvous(rendezvous::client::Event::Expired {
                        peer,..
                    })) => {
                        println!("Expired: {:?}",peer);
                        
                        self.behaviour_mut().rendezvous.register(
                            rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                            peer.clone(), 
                            std::default::Default::default(),
                        ).unwrap();

                        /*
                        if peer.clone() == <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(){
                            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                            self.dial(rendezvous_address.clone()).unwrap();
                        }
                        */
                        //self._register_inc_failures(peer.clone(), &mut registration_failures, &namespace);
                        /*
                        self.behaviour_mut().rendezvous.discover(
                            Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                            cookie_cache.clone(),
                            None,
                            keypair.public().to_peer_id(),
                        );*/
                    },
                    SwarmEvent::Behaviour(RendezvousGossipBehaviourEvent::Pubsub(libp2p::gossipsub::Event::Message {
                        message, ..
                    })) => {
                        println!("[PS] MSG: {}", String::from_utf8_lossy(&message.data));
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
                    })) => if peer_id != keypair.public().to_peer_id().clone() {

                        println!("MyPeerId: {:?}; Identified PeerId: {:?}",keypair.public().to_peer_id().clone(),peer_id.clone());
                        if peer_id != <libp2p::PeerId as std::str::FromStr>::from_str("12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap(){
                            println!("[NEW-EXT-ADDR]: {:?}",<Multiaddr as std::str::FromStr>::from_str(&format!("{}/p2p/{}",info.observed_addr,keypair.clone().public().to_peer_id().to_string())).unwrap());
                            //self.add_external_address(<Multiaddr as std::str::FromStr>::from_str(&format!("{}/p2p/{}",info.observed_addr,keypair.clone().public().to_peer_id().to_string())).unwrap());
                            //self.behaviour_mut().pubsub.subscribe(&topic).unwrap();
                            println!("[WELCOME] Identified new peer_id then rendezvous: {:?}",peer_id.clone());
                            

                            match self.behaviour_mut().pubsub.publish(topic.clone(), "") {
                                Ok(result) => {
                                    println!("Send hello Message!");
                                },
                                Err(e) => {
                                    println!("Error: {:?}",e);
                                }
                            }

                            

                            
                        } else {
                            self.add_external_address(<Multiaddr as std::str::FromStr>::from_str(&format!("{}/p2p/{}",info.observed_addr,keypair.clone().public().to_peer_id().to_string())).unwrap());

                            
                            if !is_pub_listener_address_set {
                                let _ = self.listen_on(info.observed_addr.clone());
                                is_pub_listener_address_set = true;
                                self.behaviour_mut().pubsub.subscribe(&topic).unwrap();
                            }

                            //self._register_inc_failures(&);
                            println!("Identified {:?}",info.observed_addr);
                            self.behaviour_mut().rendezvous.register(
                                rendezvous::Namespace::new(namespace.to_string()).unwrap(),
                                peer_id.clone(),
                                std::default::Default::default(),
                            ).unwrap();
                            
                            self.behaviour_mut().rendezvous.discover(
                                Some(rendezvous::Namespace::new(namespace.to_string()).unwrap()),
                                None, //[!]cookie_cache.clone(),
                                None,
                                peer_id.clone(),
                            );
                            
                            self.behaviour_mut().pubsub.subscribe(&topic).unwrap();
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
    //rendezvous_server: rendezvous::server::Behaviour,
    pubsub: gossipsub::Behaviour,
    upnp: upnp::tokio::Behaviour
}
