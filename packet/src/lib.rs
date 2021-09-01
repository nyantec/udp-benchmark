use pnet_macros::Packet;
use pnet_macros_support::types::u64be;

#[derive(Packet)]
pub struct UdpEcho {
    // Generic identifier, should be different for every client
    pub identifier: u64be,

    // Sequnce nuber, only meaningfull for each client
    pub sequence: u64be,

    // Currently unused, open for later use, e.g. different payloads
    pub next_level: u8,

    #[payload]
    pub payload: Vec<u8>,
}

impl UdpEcho {
    pub fn new(identifier: u64, sequence: u64) -> Self {
        Self {
            identifier,
            sequence,
            next_level: 0,
            payload: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{MutableUdpEchoPacket, UdpEcho};

    #[test]
    fn accessors() {
        let id = 1234;
        let sequence = 5678;
        let echo = UdpEcho::new(id, sequence);

        let mut buf = [0u8; 17];
        let mut mutable = MutableUdpEchoPacket::new(&mut buf).unwrap();
        mutable.populate(&echo);

        assert_eq!(mutable.get_identifier(), id);
        assert_eq!(mutable.get_sequence(), sequence);
        assert_eq!(mutable.get_next_level(), 0);
    }
}
