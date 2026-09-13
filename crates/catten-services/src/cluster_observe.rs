//! Bounded wire snapshot for the cluster observability keyhole.
//!
//! The Raft/DNS service produces this binary representation; presentation
//! adapters turn it into JSON or another operator-facing format. Keeping the
//! control plane free of HTML/JSON also gives non-browser tools the same
//! versioned, strictly decoded view.

extern crate alloc;

use alloc::vec::Vec;

pub const MAGIC: &[u8; 8] = b"CLOBSV1\0";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 72;
pub const NODE_RECORD_LEN: usize = 60;
pub const DEPLOYMENT_RECORD_LEN: usize = 36;
pub const INGRESS_RECORD_LEN: usize = 32;
pub const MAX_SNAPSHOT_LEN: usize = 64 * 1024;
pub const MAX_NODES: usize = 32;
pub const MAX_DEPLOYMENTS: usize = 256;
pub const MAX_INGRESS: usize = charlotte_launch::ingress::MAX_SERVICES;

pub const FLAG_FRESH_COMMITTED: u16 = 1 << 0;
pub const FLAG_LOCAL_LEADER: u16 = 1 << 1;
pub const FLAG_TRUNCATED: u16 = 1 << 2;

pub const NODE_MEMBER: u16 = 1 << 0;
pub const NODE_SELF: u16 = 1 << 1;
pub const NODE_LEADER: u16 = 1 << 2;
pub const NODE_DRAINING: u16 = 1 << 3;
pub const NODE_CAPACITY_PRESENT: u16 = 1 << 4;
pub const NODE_CAPACITY_FRESH: u16 = 1 << 5;
pub const NODE_MAC_PRESENT: u16 = 1 << 6;
pub const NODE_FLAGS: u16 = NODE_MEMBER
    | NODE_SELF
    | NODE_LEADER
    | NODE_DRAINING
    | NODE_CAPACITY_PRESENT
    | NODE_CAPACITY_FRESH
    | NODE_MAC_PRESENT;

pub const INGRESS_PROJECTION_PRESENT: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ControllerCounters {
    pub capacity_reports_accepted: u32,
    pub capacity_commands_proposed: u32,
    pub placement_reassignments: u32,
    pub forced_reassignments: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Node {
    pub node_key: u64,
    pub flags: u16,
    pub mac: [u8; 6],
    pub capacity_boot_nonce: u64,
    pub capacity_epoch: u64,
    pub free_frames: u64,
    pub usable_frames: u64,
    pub committed_frames: u64,
    pub cpu_load_permille: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deployment {
    pub name: Vec<u8>,
    pub state: u8,
    pub generation: u64,
    pub service_generation: u64,
    pub object_id: u64,
    pub demand_frames: u64,
    pub desired_nodes: Vec<u64>,
    pub ready_nodes: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ingress {
    pub service: charlotte_launch::ingress::ServiceId,
    pub backend_name: Option<Vec<u8>>,
    pub projection_present: bool,
    pub member_count: u16,
    pub advertiser_node: Option<u64>,
    pub epoch: u64,
    pub eligible_nodes: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub flags: u16,
    pub state: u8,
    pub term: u64,
    pub commit_index: u64,
    pub membership_epoch: u64,
    pub observed_millis: u64,
    pub leader_id: Vec<u8>,
    pub self_id: Vec<u8>,
    pub controller: ControllerCounters,
    pub nodes: Vec<Node>,
    pub deployments: Vec<Deployment>,
    pub ingress: Vec<Ingress>,
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8], offset: &mut usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let value = u16::from_le_bytes(bytes.get(*offset..end)?.try_into().ok()?);
    *offset = end;
    Some(value)
}

fn get_u32(bytes: &[u8], offset: &mut usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let value = u32::from_le_bytes(bytes.get(*offset..end)?.try_into().ok()?);
    *offset = end;
    Some(value)
}

fn get_u64(bytes: &[u8], offset: &mut usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    let value = u64::from_le_bytes(bytes.get(*offset..end)?.try_into().ok()?);
    *offset = end;
    Some(value)
}

fn valid_nodes(nodes: &[u64]) -> bool {
    nodes.len() <= MAX_NODES
        && nodes.iter().all(|node| *node != 0)
        && nodes.windows(2).all(|pair| pair[0] < pair[1])
}

fn valid(snapshot: &Snapshot) -> bool {
    snapshot.flags & !(FLAG_FRESH_COMMITTED | FLAG_LOCAL_LEADER | FLAG_TRUNCATED) == 0
        && matches!(snapshot.state, 1..=3)
        && !snapshot.self_id.is_empty()
        && snapshot.self_id.len() <= u8::MAX as usize
        && snapshot.leader_id.len() <= u8::MAX as usize
        && snapshot.nodes.len() <= MAX_NODES
        && snapshot.deployments.len() <= MAX_DEPLOYMENTS
        && snapshot.ingress.len() <= MAX_INGRESS
        && snapshot.nodes.iter().all(|node| {
            node.node_key != 0
                && node.flags & !NODE_FLAGS == 0
                && (node.flags & NODE_CAPACITY_FRESH == 0
                    || node.flags & NODE_CAPACITY_PRESENT != 0)
                && (node.flags & NODE_CAPACITY_PRESENT != 0
                    || (node.capacity_boot_nonce == 0
                        && node.capacity_epoch == 0
                        && node.free_frames == 0
                        && node.usable_frames == 0
                        && node.cpu_load_permille == 0))
                && (node.flags & NODE_CAPACITY_PRESENT == 0
                    || node.cpu_load_permille <= 1_000
                    || node.cpu_load_permille == u16::MAX)
                && (node.flags & NODE_MAC_PRESENT != 0 || node.mac == [0; 6])
        })
        && snapshot.nodes.windows(2).all(|pair| pair[0].node_key < pair[1].node_key)
        && snapshot.deployments.iter().all(|deployment| {
            charlotte_launch::deployment::valid_artifact_name(&deployment.name)
                && deployment.name.len() <= u8::MAX as usize
                && matches!(
                    deployment.state,
                    crate::clusterctl::ROLLOUT_COMMITTED
                        | crate::clusterctl::ROLLOUT_READY
                        | crate::clusterctl::ROLLOUT_REPLACING
                )
                && valid_nodes(&deployment.desired_nodes)
                && valid_nodes(&deployment.ready_nodes)
        })
        && snapshot.deployments.windows(2).all(|pair| pair[0].name < pair[1].name)
        && snapshot.ingress.iter().all(|ingress| {
            ingress.service.is_valid()
                && ingress
                    .backend_name
                    .as_deref()
                    .is_none_or(charlotte_launch::deployment::valid_artifact_name)
                && ingress.backend_name.as_ref().is_none_or(|name| name.len() <= u8::MAX as usize)
                && valid_nodes(&ingress.eligible_nodes)
                && ingress.member_count as usize <= MAX_NODES
                && (ingress.projection_present
                    || (ingress.member_count == 0
                        && ingress.advertiser_node.is_none()
                        && ingress.epoch == 0
                        && ingress.eligible_nodes.is_empty()))
        })
        && snapshot.ingress.windows(2).all(|pair| pair[0].service < pair[1].service)
}

pub fn encode(snapshot: &Snapshot) -> Option<Vec<u8>> {
    if !valid(snapshot) {
        return None;
    }
    let mut bytes = Vec::with_capacity(HEADER_LEN);
    bytes.extend_from_slice(MAGIC);
    put_u16(&mut bytes, VERSION);
    put_u16(&mut bytes, snapshot.flags);
    bytes.push(snapshot.state);
    bytes.push(snapshot.leader_id.len() as u8);
    bytes.push(snapshot.self_id.len() as u8);
    bytes.push(0);
    put_u64(&mut bytes, snapshot.term);
    put_u64(&mut bytes, snapshot.commit_index);
    put_u64(&mut bytes, snapshot.membership_epoch);
    put_u64(&mut bytes, snapshot.observed_millis);
    put_u16(&mut bytes, snapshot.nodes.len() as u16);
    put_u16(&mut bytes, snapshot.deployments.len() as u16);
    put_u16(&mut bytes, snapshot.ingress.len() as u16);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, snapshot.controller.capacity_reports_accepted);
    put_u32(&mut bytes, snapshot.controller.capacity_commands_proposed);
    put_u32(&mut bytes, snapshot.controller.placement_reassignments);
    put_u32(&mut bytes, snapshot.controller.forced_reassignments);
    debug_assert_eq!(bytes.len(), HEADER_LEN);
    bytes.extend_from_slice(&snapshot.leader_id);
    bytes.extend_from_slice(&snapshot.self_id);

    for node in &snapshot.nodes {
        put_u64(&mut bytes, node.node_key);
        put_u16(&mut bytes, node.flags);
        bytes.extend_from_slice(&node.mac);
        put_u64(&mut bytes, node.capacity_boot_nonce);
        put_u64(&mut bytes, node.capacity_epoch);
        put_u64(&mut bytes, node.free_frames);
        put_u64(&mut bytes, node.usable_frames);
        put_u64(&mut bytes, node.committed_frames);
        put_u16(&mut bytes, node.cpu_load_permille);
        put_u16(&mut bytes, 0);
    }
    for deployment in &snapshot.deployments {
        bytes.push(deployment.name.len() as u8);
        bytes.push(deployment.state);
        bytes.push(deployment.desired_nodes.len() as u8);
        bytes.push(deployment.ready_nodes.len() as u8);
        put_u64(&mut bytes, deployment.generation);
        put_u64(&mut bytes, deployment.service_generation);
        put_u64(&mut bytes, deployment.object_id);
        put_u64(&mut bytes, deployment.demand_frames);
        bytes.extend_from_slice(&deployment.name);
        for node in &deployment.desired_nodes {
            put_u64(&mut bytes, *node);
        }
        for node in &deployment.ready_nodes {
            put_u64(&mut bytes, *node);
        }
    }
    for ingress in &snapshot.ingress {
        let name = ingress.backend_name.as_deref().unwrap_or_default();
        bytes.extend_from_slice(&ingress.service.address);
        bytes.push(ingress.service.protocol);
        bytes.push(name.len() as u8);
        bytes.push(ingress.eligible_nodes.len() as u8);
        bytes.push(u8::from(ingress.projection_present) * INGRESS_PROJECTION_PRESENT);
        put_u16(&mut bytes, ingress.service.port);
        put_u16(&mut bytes, ingress.member_count);
        put_u64(&mut bytes, ingress.advertiser_node.unwrap_or(0));
        put_u64(&mut bytes, ingress.epoch);
        put_u32(&mut bytes, 0);
        bytes.extend_from_slice(name);
        for node in &ingress.eligible_nodes {
            put_u64(&mut bytes, *node);
        }
    }
    (bytes.len() <= MAX_SNAPSHOT_LEN).then_some(bytes)
}

pub fn decode(bytes: &[u8]) -> Option<Snapshot> {
    if bytes.len() < HEADER_LEN || bytes.len() > MAX_SNAPSHOT_LEN || &bytes[..8] != MAGIC {
        return None;
    }
    let mut offset = 8;
    if get_u16(bytes, &mut offset)? != VERSION {
        return None;
    }
    let flags = get_u16(bytes, &mut offset)?;
    let state = *bytes.get(offset)?;
    let leader_len = *bytes.get(offset + 1)? as usize;
    let self_len = *bytes.get(offset + 2)? as usize;
    if bytes.get(offset + 3).copied()? != 0 {
        return None;
    }
    offset += 4;
    let term = get_u64(bytes, &mut offset)?;
    let commit_index = get_u64(bytes, &mut offset)?;
    let membership_epoch = get_u64(bytes, &mut offset)?;
    let observed_millis = get_u64(bytes, &mut offset)?;
    let node_count = get_u16(bytes, &mut offset)? as usize;
    let deployment_count = get_u16(bytes, &mut offset)? as usize;
    let ingress_count = get_u16(bytes, &mut offset)? as usize;
    if get_u16(bytes, &mut offset)? != 0
        || node_count > MAX_NODES
        || deployment_count > MAX_DEPLOYMENTS
        || ingress_count > MAX_INGRESS
    {
        return None;
    }
    let controller = ControllerCounters {
        capacity_reports_accepted: get_u32(bytes, &mut offset)?,
        capacity_commands_proposed: get_u32(bytes, &mut offset)?,
        placement_reassignments: get_u32(bytes, &mut offset)?,
        forced_reassignments: get_u32(bytes, &mut offset)?,
    };
    if offset != HEADER_LEN {
        return None;
    }
    let id_end = offset.checked_add(leader_len)?.checked_add(self_len)?;
    let leader_id = bytes.get(offset..offset + leader_len)?.to_vec();
    offset += leader_len;
    let self_id = bytes.get(offset..id_end)?.to_vec();
    offset = id_end;

    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let start = offset;
        let node_key = get_u64(bytes, &mut offset)?;
        let node_flags = get_u16(bytes, &mut offset)?;
        let mut mac = [0; 6];
        mac.copy_from_slice(bytes.get(offset..offset + 6)?);
        offset += 6;
        let node = Node {
            node_key,
            flags: node_flags,
            mac,
            capacity_boot_nonce: get_u64(bytes, &mut offset)?,
            capacity_epoch: get_u64(bytes, &mut offset)?,
            free_frames: get_u64(bytes, &mut offset)?,
            usable_frames: get_u64(bytes, &mut offset)?,
            committed_frames: get_u64(bytes, &mut offset)?,
            cpu_load_permille: get_u16(bytes, &mut offset)?,
        };
        if get_u16(bytes, &mut offset)? != 0 || offset - start != NODE_RECORD_LEN {
            return None;
        }
        nodes.push(node);
    }
    let mut deployments = Vec::with_capacity(deployment_count);
    for _ in 0..deployment_count {
        let fixed_end = offset.checked_add(DEPLOYMENT_RECORD_LEN)?;
        let name_len = *bytes.get(offset)? as usize;
        let state = *bytes.get(offset + 1)?;
        let desired_count = *bytes.get(offset + 2)? as usize;
        let ready_count = *bytes.get(offset + 3)? as usize;
        offset += 4;
        let generation = get_u64(bytes, &mut offset)?;
        let service_generation = get_u64(bytes, &mut offset)?;
        let object_id = get_u64(bytes, &mut offset)?;
        let demand_frames = get_u64(bytes, &mut offset)?;
        if offset != fixed_end {
            return None;
        }
        let name_end = offset.checked_add(name_len)?;
        let name = bytes.get(offset..name_end)?.to_vec();
        offset = name_end;
        let mut desired_nodes = Vec::with_capacity(desired_count);
        for _ in 0..desired_count {
            desired_nodes.push(get_u64(bytes, &mut offset)?);
        }
        let mut ready_nodes = Vec::with_capacity(ready_count);
        for _ in 0..ready_count {
            ready_nodes.push(get_u64(bytes, &mut offset)?);
        }
        deployments.push(Deployment {
            name,
            state,
            generation,
            service_generation,
            object_id,
            demand_frames,
            desired_nodes,
            ready_nodes,
        });
    }
    let mut ingress = Vec::with_capacity(ingress_count);
    for _ in 0..ingress_count {
        let fixed_end = offset.checked_add(INGRESS_RECORD_LEN)?;
        let address = bytes.get(offset..offset + 4)?.try_into().ok()?;
        let protocol = *bytes.get(offset + 4)?;
        let name_len = *bytes.get(offset + 5)? as usize;
        let eligible_count = *bytes.get(offset + 6)? as usize;
        let ingress_flags = *bytes.get(offset + 7)?;
        if ingress_flags & !INGRESS_PROJECTION_PRESENT != 0 {
            return None;
        }
        offset += 8;
        let port = get_u16(bytes, &mut offset)?;
        let member_count = get_u16(bytes, &mut offset)?;
        let advertiser = get_u64(bytes, &mut offset)?;
        let epoch = get_u64(bytes, &mut offset)?;
        if get_u32(bytes, &mut offset)? != 0 || offset != fixed_end {
            return None;
        }
        let name_end = offset.checked_add(name_len)?;
        let name = bytes.get(offset..name_end)?.to_vec();
        offset = name_end;
        let mut eligible_nodes = Vec::with_capacity(eligible_count);
        for _ in 0..eligible_count {
            eligible_nodes.push(get_u64(bytes, &mut offset)?);
        }
        ingress.push(Ingress {
            service: charlotte_launch::ingress::ServiceId {
                address,
                protocol,
                port,
            },
            backend_name: (!name.is_empty()).then_some(name),
            projection_present: ingress_flags & INGRESS_PROJECTION_PRESENT != 0,
            member_count,
            advertiser_node: (advertiser != 0).then_some(advertiser),
            epoch,
            eligible_nodes,
        });
    }
    if offset != bytes.len() {
        return None;
    }
    let snapshot = Snapshot {
        flags,
        state,
        term,
        commit_index,
        membership_epoch,
        observed_millis,
        leader_id,
        self_id,
        controller,
        nodes,
        deployments,
        ingress,
    };
    valid(&snapshot).then_some(snapshot)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    fn sample() -> Snapshot {
        Snapshot {
            flags: FLAG_FRESH_COMMITTED | FLAG_LOCAL_LEADER,
            state: 3,
            term: 8,
            commit_index: 42,
            membership_epoch: 7,
            observed_millis: 12_345,
            leader_id: b"charlotte:a".to_vec(),
            self_id: b"charlotte:a".to_vec(),
            controller: ControllerCounters {
                capacity_reports_accepted: 9,
                capacity_commands_proposed: 3,
                placement_reassignments: 2,
                forced_reassignments: 1,
            },
            nodes: vec![Node {
                node_key: 1,
                flags: NODE_MEMBER
                    | NODE_SELF
                    | NODE_LEADER
                    | NODE_MAC_PRESENT
                    | NODE_CAPACITY_PRESENT
                    | NODE_CAPACITY_FRESH,
                mac: [2, 0, 0, 0, 0, 1],
                capacity_boot_nonce: 11,
                capacity_epoch: 12,
                free_frames: 900,
                usable_frames: 1_000,
                committed_frames: 40,
                cpu_load_permille: 125,
            }],
            deployments: vec![Deployment {
                name: b"orders".to_vec(),
                state: crate::clusterctl::ROLLOUT_READY,
                generation: 4,
                service_generation: 5,
                object_id: 6,
                demand_frames: 24,
                desired_nodes: vec![1],
                ready_nodes: vec![1],
            }],
            ingress: vec![Ingress {
                service: charlotte_launch::ingress::ServiceId::tcp_v4([10, 0, 0, 42], 443),
                backend_name: Some(b"orders".to_vec()),
                projection_present: true,
                member_count: 1,
                advertiser_node: Some(1),
                epoch: 13,
                eligible_nodes: vec![1],
            }],
        }
    }

    #[test]
    fn snapshot_round_trips_and_rejects_trailing_data() {
        let snapshot = sample();
        let encoded = encode(&snapshot).unwrap();
        assert_eq!(decode(&encoded), Some(snapshot));
        let mut trailing = encoded;
        trailing.push(0);
        assert_eq!(decode(&trailing), None);
    }

    #[test]
    fn snapshot_rejects_noncanonical_nodes_and_unknown_flags() {
        let mut snapshot = sample();
        snapshot.nodes.push(snapshot.nodes[0]);
        assert_eq!(encode(&snapshot), None);
        let mut snapshot = sample();
        snapshot.flags |= 0x8000;
        assert_eq!(encode(&snapshot), None);
    }
}
