
mod network;
use std::{str::FromStr, time::Duration};

use libp2p::Multiaddr;
use network::{setup_swarm, Spinup};
use tokio::time::sleep;

const  ROOT_DIR: &str = ".openrendezvous/";

#[tokio::main]
async fn main(){
    let node_keypair = network::get_root_keypair(ROOT_DIR).unwrap();
    let cluster_keypair = network::get_cluster_keypair(ROOT_DIR).unwrap();
    let rendezvous_node = Multiaddr::from_str("/ip4/157.90.114.32/tcp/53748/p2p/12D3KooWQNTeKVURvL5ZEtUaWCp7JhDaWkC6X9Js3CF2urNLHfBn").unwrap();
        
    let mut swarm = setup_swarm(
        node_keypair.clone(),
        vec![rendezvous_node.clone()]
    ).unwrap();
    
    let _ = swarm.spinup(
        format!("p2pdb-cluster-"), // {}",cluster_keypair.public().to_peer_id()),
        node_keypair.clone(), 
        cluster_keypair.clone(), 
        rendezvous_node.clone()).await;
    
    loop {
        let _ = sleep(Duration::from_secs(2)).await;
        
        println!("Network:");
        for peer in swarm.connected_peers() {
            println!("- {:?}",peer.clone());
        }
        println!("")
    }


}

