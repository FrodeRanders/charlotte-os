use alloc::{
    format,
    string::String,
    vec::Vec,
};

use prost::Message;

use crate::{
    proto::{
        self,
        internal_raft_command,
    },
    types::{
        Peer,
        Role,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigurationCommand {
    Joint(Vec<Peer>),
    Finalize,
    Join(Peer),
}

pub fn encode(command: &ConfigurationCommand) -> Option<Vec<u8>> {
    let command = match command {
        ConfigurationCommand::Joint(members) => {
            internal_raft_command::Command::Joint(proto::JointConfigurationCommand {
                members: members.iter().map(peer_spec).collect(),
            })
        }
        ConfigurationCommand::Finalize => {
            internal_raft_command::Command::Finalize(proto::FinalizeConfigurationCommand {})
        }
        ConfigurationCommand::Join(peer) => {
            internal_raft_command::Command::Join(proto::JoinPeerCommand {
                member: Some(peer_spec(peer)),
            })
        }
    };
    let message = proto::InternalRaftCommand {
        command: Some(command),
    };
    let mut bytes = Vec::with_capacity(message.encoded_len());
    message.encode(&mut bytes).ok()?;
    Some(bytes)
}

pub fn decode(bytes: &[u8]) -> Option<ConfigurationCommand> {
    let message = proto::InternalRaftCommand::decode(bytes).ok()?;
    match message.command? {
        internal_raft_command::Command::Joint(command) => Some(ConfigurationCommand::Joint(
            command.members.into_iter().map(peer).collect::<Option<Vec<_>>>()?,
        )),
        internal_raft_command::Command::Finalize(_) => Some(ConfigurationCommand::Finalize),
        internal_raft_command::Command::Join(command) => {
            Some(ConfigurationCommand::Join(peer(command.member?)?))
        }
    }
}

fn peer_spec(peer: &Peer) -> proto::PeerSpec {
    proto::PeerSpec {
        id: peer.id.clone(),
        // Charlotte's service name is its transport address. Keeping it in
        // the shared host field lets snapshots/logs round-trip locally while
        // remaining a valid cross-language PeerSpec for a gateway to map.
        host: format!("charlotte:{:016x}", peer.service_name),
        port: 0,
        role: if peer.is_voter() {
            String::from("VOTER")
        } else {
            String::from("LEARNER")
        },
    }
}

fn peer(spec: proto::PeerSpec) -> Option<Peer> {
    if spec.id.is_empty() {
        return None;
    }
    let service_name = spec
        .host
        .strip_prefix("charlotte:")
        .and_then(|value| u64::from_str_radix(value, 16).ok())
        .unwrap_or(0);
    let role = if spec.role.eq_ignore_ascii_case("LEARNER") {
        Role::Learner
    } else {
        Role::Voter
    };
    Some(Peer {
        id: spec.id,
        service_name,
        role,
    })
}
