
mod network;
use std::{str::FromStr, time::Duration};

use libp2p::Multiaddr;
use network::{setup_swarm, Spinup};
use tokio::time::sleep;

const  ROOT_DIR: &str = ".openrendezvous/";

#[tokio::main]
async fn main(){
    let node_keypair = network::get_root_keypair(ROOT_DIR).unwrap();
     
    let mut swarm = setup_swarm(
        node_keypair.clone(),
    ).unwrap();
    
    let _ = swarm.spinup(node_keypair.clone()).await;
}

