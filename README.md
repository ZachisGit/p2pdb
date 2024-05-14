# p2pdb

Object Database written in Rust using libp2p.

## First Steps

- Create rendezvous mechanism to automate node localization
- Create identity for peers (probably libp2p peerId + a rsa (or similar) keyPair for new node authentication)
    * maybe just a shared secret among nodes that they hash with the authenticator nodes peerId => sha512(secretToken + authPeerId)
- TestCoreEngine control layer for a shared FileIndex
    * explore the fill first, reduce later redundancy solution
        -> all nodes replicate all files
        -> if a node runs out of storage it removes old files that it knows at least Rth (redundency threshold) nodes have a copy off
            -> delete old file
            -> write new file
        -> if a node can't remove any files without risking the cluster going below Rth copies of another object
            -> send alert on PubSub channel to all other nodes that you failed to allocate a new file to your list
                -> nodes publish local FileIndex and therefore all nodes know about the missing write from one node
        -> interval triggers internal accounting and FileIndex publishing (at least FileIndex changes should be published)
- TestStorageEngine local storage manager and accountant
    * responds to events
    * sends notifications/commands to command core engine
- TestCli?