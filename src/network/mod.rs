mod pubsub;

pub use pubsub::setup_swarm;
pub use pubsub::Spinup;


pub fn get_root_keypair(
    root_dir: &str,
) -> Result<libp2p::identity::Keypair, Box<dyn std::error::Error>> {
    let key_dir: String = format!("{}keys/", root_dir);
    if !std::path::Path::new(&format!("{}", key_dir)).exists() {
        std::fs::create_dir_all(&key_dir).unwrap();
    }
    if !std::path::Path::new(&format!("{}{}", key_dir, "root_keypair")).exists() {
        let tmp_keypair: libp2p::identity::Keypair = libp2p::identity::Keypair::generate_ed25519();
        let keypair_bytes: Vec<u8> = tmp_keypair.to_protobuf_encoding().unwrap();

        // write keypair to file
        let keypair_file: String = format!("{}root_keypair", key_dir);
        std::fs::write(keypair_file, keypair_bytes).unwrap();
    }
    let keypair_bytes: Vec<u8> = std::fs::read(&format!("{}{}", key_dir, "root_keypair")).unwrap();
    let keypair = libp2p::identity::Keypair::from_protobuf_encoding(&keypair_bytes).unwrap();

    Ok(keypair)
}



pub fn get_cluster_keypair(
    root_dir: &str,
) -> Result<libp2p::identity::Keypair, Box<dyn std::error::Error>> {
    let key_dir: String = format!("{}keys/", root_dir);
    if !std::path::Path::new(&format!("{}", key_dir)).exists() {
        std::fs::create_dir_all(&key_dir).unwrap();
    }
    if !std::path::Path::new(&format!("{}{}", key_dir, "cluster_keypair")).exists() {
        let tmp_keypair: libp2p::identity::Keypair = libp2p::identity::Keypair::generate_ed25519();
        let keypair_bytes: Vec<u8> = tmp_keypair.to_protobuf_encoding().unwrap();

        // write keypair to file
        let keypair_file: String = format!("{}cluster_keypair", key_dir);
        std::fs::write(keypair_file, keypair_bytes).unwrap();
    }
    let keypair_bytes: Vec<u8> = std::fs::read(&format!("{}{}", key_dir, "cluster_keypair")).unwrap();
    let keypair = libp2p::identity::Keypair::from_protobuf_encoding(&keypair_bytes).unwrap();

    Ok(keypair)
}
