
mod network;
use std::str::FromStr;

use libp2p::Multiaddr;
use network::{setup_swarm, Spinup};

const ROOT_DIR: &str = ".openrendezvous/";

#[tokio::main]
async fn main(){
    let node_keypair = network::get_root_keypair(ROOT_DIR).unwrap();
    let cluster_keypair = network::get_cluster_keypair(ROOT_DIR).unwrap();
    let mut swarm = setup_swarm(node_keypair.clone()).unwrap();
    
    swarm.spinup(
        format!("p2pdb-cluster-{}",cluster_keypair.public().to_peer_id()),
        node_keypair.clone(), 
        cluster_keypair.clone(), 
        Multiaddr::from_str("/ip4/127.0.0.1/tcp/0").unwrap()).await.unwrap();
}

