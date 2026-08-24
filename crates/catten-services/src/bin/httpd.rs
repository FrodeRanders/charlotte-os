//! Minimal hardcoded HTTP server exposing a full report of a node's state.
//!
//! Listens on TCP port 80 through the tcpip service, and for each connection
//! reads the request, then replies with a JSON document aggregating
//! observable state across the node:
//!
//! - `node`    — NIC MAC + link state (`net::OP_STATUS`)
//! - `ns`      — name-service registry catalog + pending lookups (`ns::OP_STATUS`, via the
//!   bootstrap connection)
//! - `tcpip`   — tcpip service counters (`socket::OP_STATUS`)
//! - `frouter` — frame demultiplexer counters (`frouter::OP_STATUS`)
//! - `dns`     — Raft leader/term/catalog (`dns::OP_STATUS`)
//! - `disco`   — discovered peers (`disco::OP_STATUS`)
//! - `relmsg`  — reliable-message transport (`relmsg::OP_STATUS`)
//! - `threads` — system-wide thread statistics via the observe service's `OP_THREAD_SNAPSHOT`
//!   (backed by the kernel SystemObserver capability)
//! - `http`    — this server's own counters
//!
//! Services that are not running are rendered as `null`; the aggregator uses
//! non-blocking `ns::OP_TRY_LOOKUP` so an absent service never stalls a
//! request. This is a deliberate keyhole, not a web server: no routing, no
//! keep-alive, one connection at a time.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;

use catten_rt::{
    Context,
    config,
};
use catten_services::{
    disco,
    dns,
    frouter,
    net,
    ns,
    observability,
    relmsg,
    sleep_ms,
    socket,
    wait_for_local_ready,
    wait_reply,
};
use catten_syscall::{
    THREAD_STATISTICS_HEADER_U64S,
    THREAD_STATISTICS_MAGIC,
    THREAD_STATISTICS_RECORD_U64S,
    THREAD_STATISTICS_VERSION,
    ipc_close,
    ipc_reply_wait_with_memory,
    ipc_scalar_call,
    ipc_scalar_call_move,
    memory_alloc,
    memory_close,
    memory_map_any,
    memory_unmap,
    thread_exit,
};
use catten_syscall::{
    thread_statistics_header as thread_header,
    thread_statistics_record as thread_record,
};
use charlotte_protocol_msg::unpack_address_and_len;
use charlotte_protocol_net::decode_status;
use charlotte_launch::httpd_status as status;

const HTTP_PORT: u16 = 80;
const ACCEPT_POLL_MS: u64 = 50;
const SENTINEL: u32 = 0x4854_5450; // "HTTP"
/// Cap on rendered thread rows so the report fits comfortably in a single
/// TCP segment; the full detail is available from observe directly.
const THREAD_SAMPLE_ROWS: usize = 8;

struct ServiceSet {
    ns_conn: u64,
    tcp_conn: u64,
    frouter_conn: u64,
    dns_conn: u64,
    disco_conn: u64,
    relmsg_conn: u64,
    observe_conn: u64,
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    catten_syscall::el0_log(0x4854_5444, 0xfa00_0000 | code as u64);
    unsafe { thread_exit() };
}

/// Send a payload, retrying while the tcpip service reports `ERR_WOULD_BLOCK`
/// or accepts only part of a chunk (buffer fills as smoltcp drains it).
fn send_all(tcp_conn: u64, sock_id: u64, data: &[u8]) -> bool {
    let mut offset = 0usize;
    for _ in 0..1200 {
        if offset >= data.len() {
            return true;
        }
        let chunk_len = (data.len() - offset).min(4096);
        let cap = memory_alloc(1);
        let (scratch_7_map_status, scratch_7_vaddr) = memory_map_any(cap, true);
        if cap == 0 || scratch_7_map_status != 0 {
            if cap != 0 {
                memory_close(cap);
            }
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                data.as_ptr().add(offset),
                scratch_7_vaddr as *mut u8,
                chunk_len,
            );
        }
        memory_unmap(cap);
        let send = ipc_scalar_call_move(
            tcp_conn,
            socket::OP_SEND,
            ((chunk_len as u64) << 32) | (sock_id & 0xffff_ffff),
            cap,
        );
        if send == 0 {
            memory_close(cap);
            return false;
        }
        let (sent, _) = unsafe { wait_reply(send, 0) };
        if sent > 0 {
            offset += sent as usize;
        } else if sent == socket::ERR_WOULD_BLOCK || sent == 0 {
            // Connection not established yet or transmit buffer full; retry.
            sleep_ms(ACCEPT_POLL_MS);
        } else {
            return false;
        }
    }
    false
}

/// Non-blocking name-service lookup; `None` if the service is not registered.
fn try_lookup(ns_conn: u64, name: u64) -> Option<u64> {
    let call = ipc_scalar_call(ns_conn, ns::OP_TRY_LOOKUP, name);
    if call == 0 {
        return None;
    }
    let (generation, connection) = unsafe { wait_reply(call, 0) };
    if generation < 1 || connection == 0 {
        None
    } else {
        Some(connection)
    }
}

/// Scalar status call; `None` on failure.
fn call_scalar(conn: u64, opcode: u32, arg0: u64) -> Option<i64> {
    let call = ipc_scalar_call(conn, opcode, arg0);
    if call == 0 {
        return None;
    }
    let (result, _) = unsafe { wait_reply(call, 0) };
    Some(result)
}

/// Call `opcode` and copy `words` little-endian u32 words out of the moved
/// reply page.
fn read_words(conn: u64, opcode: u32, arg0: u64, words: usize) -> Option<alloc::vec::Vec<u32>> {
    let call = ipc_scalar_call(conn, opcode, arg0);
    if call == 0 {
        return None;
    }
    let (status, len, _connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    let (scratch_6_map_status, scratch_6_vaddr) = memory_map_any(memory, false);
    if (len as usize) < words * 4 || scratch_6_map_status != 0 {
        memory_close(memory);
        return None;
    }
    let mut out = alloc::vec![0u32; words];
    unsafe {
        core::ptr::copy_nonoverlapping(scratch_6_vaddr as *const u32, out.as_mut_ptr(), words);
    }
    memory_unmap(memory);
    memory_close(memory);
    Some(out)
}

struct ThreadRow {
    tid: u64,
    asid: u64,
    state: u64,
    dispatch: u64,
    runtime_ticks: u128,
}

struct ThreadReport {
    freq_hz: u64,
    mono_ticks: u64,
    rows: alloc::vec::Vec<ThreadRow>,
}

/// Fetch and parse the observe service's system-wide thread snapshot
/// (`CCOSTAT1` wire format).
fn thread_report(observe_conn: u64) -> Option<ThreadReport> {
    let call = ipc_scalar_call(observe_conn, observability::OP_THREAD_SNAPSHOT, 0);
    if call == 0 {
        return None;
    }
    let (status, len, _connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        return None;
    }
    let len = len as usize;
    let (scratch_5_map_status, scratch_5_vaddr) = memory_map_any(memory, false);
    if scratch_5_map_status != 0 {
        memory_close(memory);
        return None;
    }
    let header_words = THREAD_STATISTICS_HEADER_U64S;
    let word_bytes = core::mem::size_of::<u64>();
    if len < header_words * word_bytes {
        memory_unmap(memory);
        memory_close(memory);
        return None;
    }
    let mut header = [0u64; THREAD_STATISTICS_HEADER_U64S];
    unsafe {
        core::ptr::copy_nonoverlapping(
            scratch_5_vaddr as *const u64,
            header.as_mut_ptr(),
            header_words,
        );
    }
    if header[thread_header::MAGIC] != THREAD_STATISTICS_MAGIC
        || header[thread_header::VERSION] != THREAD_STATISTICS_VERSION
        || header[thread_header::RECORD_BYTES]
            != (THREAD_STATISTICS_RECORD_U64S * word_bytes) as u64
    {
        memory_unmap(memory);
        memory_close(memory);
        return None;
    }
    let max_by_len = (len.saturating_sub(header_words * word_bytes))
        / (THREAD_STATISTICS_RECORD_U64S * word_bytes);
    let count = (header[thread_header::RECORD_COUNT] as usize).min(max_by_len);
    let mut rows = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        let base = scratch_5_vaddr
            + header_words * word_bytes
            + i * THREAD_STATISTICS_RECORD_U64S * word_bytes;
        let mut rec: [u64; THREAD_STATISTICS_RECORD_U64S] = [0; THREAD_STATISTICS_RECORD_U64S];
        unsafe {
            core::ptr::copy_nonoverlapping(
                base as *const u64,
                rec.as_mut_ptr(),
                THREAD_STATISTICS_RECORD_U64S,
            );
        }
        rows.push(ThreadRow {
            tid: rec[thread_record::TID],
            asid: rec[thread_record::ASID],
            state: rec[thread_record::STATE],
            dispatch: rec[thread_record::DISPATCH_COUNT],
            runtime_ticks: ((rec[thread_record::TOTAL_TICKS_HIGH] as u128) << 64)
                | rec[thread_record::TOTAL_TICKS_LOW] as u128,
        });
    }
    memory_unmap(memory);
    memory_close(memory);
    Some(ThreadReport {
        freq_hz: header[thread_header::COUNTER_FREQUENCY_HZ],
        mono_ticks: header[thread_header::MONOTONIC_TICKS],
        rows,
    })
}

fn state_name(state: u64) -> &'static str {
    match state {
        1 => "follower",
        2 => "candidate",
        3 => "leader",
        _ => "unknown",
    }
}

fn thread_state_name(state: u64) -> &'static str {
    match state {
        1 => "running",
        2 => "ready",
        3 => "needs-lp",
        4 => "blocked",
        _ => "unknown",
    }
}

fn render_dns(s: &mut String, dns_conn: u64) {
    if let Some(result) = call_scalar(dns_conn, dns::OP_STATUS, 0) {
        let v = result as u64;
        let _ = write!(
            s,
            "\"dns\":{{\"state\":\"{}\",\"term\":{},\"catalog\":",
            state_name(v & 0xff),
            (v >> 8) & 0xff
        );
        // Dump the replicated name -> node catalog.
        let call = ipc_scalar_call(dns_conn, dns::OP_CATALOG, 0);
        let mut rendered = false;
        if call != 0 {
            let (status, len, _connection, memory) = ipc_reply_wait_with_memory(call);
            ipc_close(call);
            if status == 0 && memory != 0 {
                let len = len as usize;
                let (scratch_4_map_status, scratch_4_vaddr) = memory_map_any(memory, false);
                if scratch_4_map_status == 0 {
                    let _ = write!(s, "{{\"count\":{},\"entries\":{{", unsafe {
                        core::ptr::read_volatile(scratch_4_vaddr as *const u32)
                    });
                    let mut offset = dns::CATALOG_HEADER_BYTES;
                    let mut emitted = 0u32;
                    let count = unsafe { core::ptr::read_volatile(scratch_4_vaddr as *const u32) };
                    while emitted < count && offset + 2 < len.min(4096) {
                        let name_len = unsafe {
                            core::ptr::read_volatile((scratch_4_vaddr + offset) as *const u8)
                        } as usize;
                        let node_offset = offset + 1 + name_len;
                        if node_offset + 1 > len.min(4096) {
                            break;
                        }
                        let node_len = unsafe {
                            core::ptr::read_volatile((scratch_4_vaddr + node_offset) as *const u8)
                        } as usize;
                        let generation_offset = node_offset + 1 + node_len;
                        if generation_offset + 8 > len.min(4096) {
                            break;
                        }
                        if emitted > 0 {
                            s.push(',');
                        }
                        s.push('"');
                        for i in 0..name_len {
                            let byte = unsafe {
                                core::ptr::read_volatile(
                                    (scratch_4_vaddr + offset + 1 + i) as *const u8,
                                )
                            };
                            if byte == b'"' {
                                s.push('\\');
                            }
                            if byte >= 0x20 {
                                s.push(byte as char);
                            }
                        }
                        s.push_str("\":{\"node\":\"");
                        for i in 0..node_len {
                            let byte = unsafe {
                                core::ptr::read_volatile(
                                    (scratch_4_vaddr + node_offset + 1 + i) as *const u8,
                                )
                            };
                            if byte == b'"' {
                                s.push('\\');
                            }
                            if byte >= 0x20 {
                                s.push(byte as char);
                            }
                        }
                        let generation = unsafe {
                            u64::from_le(core::ptr::read_unaligned(
                                (scratch_4_vaddr + generation_offset) as *const u64,
                            ))
                        };
                        let _ = write!(s, "\",\"generation\":{generation}}}");
                        offset = generation_offset + 8;
                        emitted += 1;
                    }
                    s.push_str("}}");
                    rendered = true;
                }
                memory_unmap(memory);
                memory_close(memory);
            }
        }
        if !rendered {
            let _ = write!(s, "{{\"count\":{},\"entries\":{{}}}}", (v >> 32) & 0xffff_ffff);
        }
        s.push('}');
    } else {
        s.push_str("\"dns\":null");
    }
}

fn render_threads(s: &mut String, observe_conn: u64) {
    let Some(report) = thread_report(observe_conn) else {
        s.push_str("\"threads\":null");
        return;
    };
    let running = report.rows.iter().filter(|r| r.state == 1).count();
    let ready = report.rows.iter().filter(|r| r.state == 2).count();
    let needs_lp = report.rows.iter().filter(|r| r.state == 3).count();
    let blocked = report.rows.iter().filter(|r| r.state == 4).count();
    let _ = write!(
        s,
        "\"threads\":{{\"count\":{},\"freq_hz\":{},\"mono_ticks\":{},\"by_state\":{{\"running\":{}",
        report.rows.len(),
        report.freq_hz,
        report.mono_ticks,
        running
    );
    let _ = write!(
        s,
        ",\"ready\":{},\"needs_lp\":{},\"blocked\":{}}},\"sample\":[",
        ready, needs_lp, blocked
    );
    let samples = report.rows.len().min(THREAD_SAMPLE_ROWS);
    for i in 0..samples {
        let row = &report.rows[i];
        if i > 0 {
            s.push(',');
        }
        let _ = write!(
            s,
            "{{\"tid\":{},\"asid\":{},\"state\":\"{}\",\"dispatch\":{},\"runtime_ticks\":{}}}",
            row.tid,
            row.asid,
            thread_state_name(row.state),
            row.dispatch,
            row.runtime_ticks
        );
    }
    s.push_str("]}");
}

fn render_ns(s: &mut String, ns_conn: u64) {
    let call = ipc_scalar_call(ns_conn, ns::OP_STATUS, 0);
    if call == 0 {
        s.push_str("\"ns\":null");
        return;
    }
    let (status, len, _connection, memory) = ipc_reply_wait_with_memory(call);
    ipc_close(call);
    if status != 0 || memory == 0 {
        if memory != 0 {
            memory_close(memory);
        }
        s.push_str("\"ns\":null");
        return;
    }
    let len = len as usize;
    let (scratch_3_map_status, scratch_3_vaddr) = memory_map_any(memory, false);
    if scratch_3_map_status != 0 {
        memory_close(memory);
        s.push_str("\"ns\":null");
        return;
    }
    unsafe {
        let magic = core::ptr::read_volatile(
            (scratch_3_vaddr + ns::STATUS_OFFSET_MAGIC as usize * 4) as *const u32,
        );
        let registered = core::ptr::read_volatile(
            (scratch_3_vaddr + ns::STATUS_OFFSET_REGISTERED as usize * 4) as *const u32,
        );
        let pending = core::ptr::read_volatile(
            (scratch_3_vaddr + ns::STATUS_OFFSET_PENDING as usize * 4) as *const u32,
        );
        if magic != ns::STATUS_MAGIC {
            memory_unmap(memory);
            memory_close(memory);
            s.push_str("\"ns\":null");
            return;
        }
        let _ = write!(
            s,
            "\"ns\":{{\"registered\":{},\"pending\":{},\"services\":[",
            registered, pending
        );
        let mut offset = ns::STATUS_HEADER_BYTES;
        let mut emitted = 0u32;
        while emitted < registered && offset + 1 < len.min(4096) {
            let name_len =
                core::ptr::read_volatile((scratch_3_vaddr + offset) as *const u8) as usize;
            if offset + 1 + name_len > len.min(4096) {
                break;
            }
            if emitted > 0 {
                s.push(',');
            }
            s.push('"');
            let printable = (0..name_len).all(|i| {
                let byte =
                    core::ptr::read_volatile((scratch_3_vaddr + offset + 1 + i) as *const u8);
                byte.is_ascii_graphic()
            });
            if printable {
                for i in 0..name_len {
                    let byte =
                        core::ptr::read_volatile((scratch_3_vaddr + offset + 1 + i) as *const u8);
                    if byte == b'"' || byte == b'\\' {
                        s.push('\\');
                    }
                    s.push(byte as char);
                }
            } else {
                s.push_str("hex:");
                for i in 0..name_len {
                    let byte =
                        core::ptr::read_volatile((scratch_3_vaddr + offset + 1 + i) as *const u8);
                    let _ = write!(s, "{byte:02x}");
                }
            }
            s.push('"');
            offset += 1 + name_len;
            emitted += 1;
        }
        s.push_str("]}");
    }
    memory_unmap(memory);
    memory_close(memory);
}

fn build_json(
    mac: &[u8; 6],
    link: u8,
    services: &ServiceSet,
    requests: u32,
    uptime: u32,
) -> String {
    let mut s = String::new();

    // node + tcpip (both mandatory at startup).
    let status = read_words(services.tcp_conn, socket::OP_STATUS, 0, 5);
    let ip = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_IP as usize]);
    let rx = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_RX_FRAMES as usize]);
    let tx = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_TX_SENDS as usize]);
    let socks = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_SOCKETS as usize]);
    let _ = write!(
        &mut s,
        "{{\"node\":{{\"mac\":\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\",\"link\":{}}},",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], link
    );
    let _ = write!(
        &mut s,
        "\"tcpip\":{{\"ip\":\"{}.{}.{}.{}\",\"rx_frames\":{},\"tx_sends\":{},\"sockets\":{}",
        (ip >> 24) & 0xff,
        (ip >> 16) & 0xff,
        (ip >> 8) & 0xff,
        ip & 0xff,
        rx,
        tx,
        socks
    );
    let _ = write!(&mut s, ",\"listen_port\":{}}},", HTTP_PORT);

    // name service: registered-services catalog.
    render_ns(&mut s, services.ns_conn);
    s.push(',');

    // frouter counters.
    if services.frouter_conn != 0 {
        if let Some(w) = read_words(services.frouter_conn, frouter::OP_STATUS, 0, 7) {
            let _ = write!(
                &mut s,
                "\"frouter\":{{\"stage\":{},\"rx\":{},\"forwarded\":{},\"dropped\":{},\"unknown\":\
                 {}",
                w[frouter::STATUS_OFFSET_STAGE as usize],
                w[frouter::STATUS_OFFSET_RX as usize],
                w[frouter::STATUS_OFFSET_FORWARDED as usize],
                w[frouter::STATUS_OFFSET_DROPPED as usize],
                w[frouter::STATUS_OFFSET_UNKNOWN as usize]
            );
            let _ = write!(&mut s, ",\"routes\":{}}},", w[frouter::STATUS_OFFSET_ROUTES as usize]);
        } else {
            s.push_str("\"frouter\":null,");
        }
    } else {
        s.push_str("\"frouter\":null,");
    }

    // dns: Raft state/term + replicated name -> node catalog.
    if services.dns_conn != 0 {
        render_dns(&mut s, services.dns_conn);
        s.push(',');
    } else {
        s.push_str("\"dns\":null,");
    }

    // disco: running | peers<<8.
    if services.disco_conn != 0 {
        if let Some(result) = call_scalar(services.disco_conn, disco::OP_STATUS, 0) {
            let v = result as u64;
            let _ = write!(
                &mut s,
                "\"disco\":{{\"running\":{},\"peers\":{}}},",
                v & 0xff,
                (v >> 8) & 0xff
            );
        } else {
            s.push_str("\"disco\":null,");
        }
    } else {
        s.push_str("\"disco\":null,");
    }

    // relmsg: packed local MAC.
    if services.relmsg_conn != 0 {
        if let Some(result) = call_scalar(services.relmsg_conn, relmsg::OP_STATUS, 0) {
            let (peer_mac, _len) = unpack_address_and_len(result as u64);
            let _ = write!(
                &mut s,
                "\"relmsg\":{{\"local_mac\":\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\"}},",
                peer_mac[0], peer_mac[1], peer_mac[2], peer_mac[3], peer_mac[4], peer_mac[5]
            );
        } else {
            s.push_str("\"relmsg\":null,");
        }
    } else {
        s.push_str("\"relmsg\":null,");
    }

    // observe: system-wide thread statistics.
    if services.observe_conn != 0 {
        render_threads(&mut s, services.observe_conn);
        s.push(',');
    } else {
        s.push_str("\"threads\":null,");
    }

    let _ = write!(&mut s, "\"http\":{{\"requests\":{},\"uptime\":{}}}}}", requests, uptime);
    s
}

fn main(ctx: Context) -> ! {
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = match ctx.bootstrap_cap() {
        Some(cap) => cap,
        None => unsafe { thread_exit() },
    };
    let lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, net::NAME);
    if lookup == 0 {
        unsafe { thread_exit() };
    }
    let (generation, net_conn) = unsafe { wait_reply(lookup, 0) };
    if generation < 1 || net_conn == 0 {
        unsafe { thread_exit() };
    }
    config::write::<u32>(status::STAGE, 2);

    let status = ipc_scalar_call(net_conn, net::OP_STATUS, 0);
    if status == 0 {
        fail(0xe001);
    }
    let (status, _) = unsafe { wait_reply(status, 0) };
    let (link, mac) = decode_status(status);
    config::write::<u32>(status::STAGE, 3);

    let tcp_lookup = ipc_scalar_call(ns_conn, ns::OP_LOOKUP, socket::NAME);
    if tcp_lookup == 0 {
        fail(0xe002);
    }
    let (generation, tcp_conn) = unsafe { wait_reply(tcp_lookup, 0) };
    if generation < 1 || tcp_conn == 0 {
        fail(0xe003);
    }
    config::write::<u32>(status::STAGE, 4);

    // Optional report sources; absent services render as null.
    let services = ServiceSet {
        ns_conn,
        tcp_conn,
        frouter_conn: try_lookup(ns_conn, frouter::NAME).unwrap_or(0),
        dns_conn: try_lookup(ns_conn, dns::NAME).unwrap_or(0),
        disco_conn: try_lookup(ns_conn, disco::NAME).unwrap_or(0),
        relmsg_conn: try_lookup(ns_conn, relmsg::NAME).unwrap_or(0),
        observe_conn: try_lookup(ns_conn, observability::NAME).unwrap_or(0),
    };
    config::write::<u32>(status::STAGE, 5);

    if !wait_for_local_ready(ns_conn) {
        fail(0xe004);
    }
    config::write::<u32>(status::STAGE, 6);

    let mut requests: u32 = 0;
    let mut uptime: u32 = 0;
    loop {
        // Fresh socket + listener per connection (smoltcp's listening socket
        // becomes the established connection, then returns to Closed).
        let sock_call = ipc_scalar_call(tcp_conn, socket::OP_SOCKET, socket::DOMAIN_TCP);
        if sock_call == 0 {
            fail(0xe005);
        }
        let (sock_id, _) = unsafe { wait_reply(sock_call, 0) };
        if sock_id < 1 {
            fail(0xe006);
        }
        let port_cap = memory_alloc(1);
        let (scratch_2_map_status, scratch_2_vaddr) = memory_map_any(port_cap, true);
        if port_cap == 0 || scratch_2_map_status != 0 {
            if port_cap != 0 {
                memory_close(port_cap);
            }
            fail(0xe007);
        }
        unsafe { core::ptr::write_unaligned(scratch_2_vaddr as *mut u16, HTTP_PORT.to_le()) }
        memory_unmap(port_cap);
        let listen = ipc_scalar_call_move(tcp_conn, socket::OP_LISTEN, sock_id as u64, port_cap);
        if listen == 0 {
            memory_close(port_cap);
            fail(0xe008);
        }
        let (result, _) = unsafe { wait_reply(listen, 0) };
        if result != 0 {
            fail(0xe009);
        }

        // Poll for a connection indefinitely: this is a long-lived keyhole
        // server, so an idle listener must stay alive rather than abort.
        loop {
            let accept = ipc_scalar_call(tcp_conn, socket::OP_ACCEPT, sock_id as u64);
            if accept == 0 {
                fail(0xe00a);
            }
            let (result, _) = unsafe { wait_reply(accept, 0) };
            if result == 0 {
                break;
            }
            if result != socket::ERR_WOULD_BLOCK {
                fail(0xe00b);
            }
            sleep_ms(ACCEPT_POLL_MS);
        }

        // Read whatever request arrived (the response is hardcoded state, so
        // even a partial request is fine).
        let recv = ipc_scalar_call(tcp_conn, socket::OP_RECV, sock_id as u64);
        if recv == 0 {
            fail(0xe00d);
        }
        let (status, _len, _connection, memory) = ipc_reply_wait_with_memory(recv);
        ipc_close(recv);
        if status != 0 || memory == 0 {
            if memory != 0 {
                memory_close(memory);
            }
            fail(0xe00e);
        }
        let (scratch_map_status, _scratch_vaddr) = memory_map_any(memory, false);
        if scratch_map_status != 0 {
            memory_close(memory);
            fail(0xe00f);
        }
        memory_unmap(memory);
        memory_close(memory);

        let body = build_json(&mac, link, &services, requests, uptime);
        catten_syscall::el0_log(0x4854_5444, 1);
        let mut response = String::new();
        response.push_str("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: ");
        let _ = write!(response, "{}", body.len());
        response.push_str("\r\nConnection: close\r\n\r\n");
        response.push_str(&body);
        if !send_all(tcp_conn, sock_id as u64, response.as_bytes()) {
            fail(0xe010);
        }

        let close = ipc_scalar_call(tcp_conn, socket::OP_CLOSE, sock_id as u64);
        if close != 0 {
            let _ = unsafe { wait_reply(close, 0) };
        }

        requests = requests.wrapping_add(1);
        uptime = uptime.wrapping_add(1);
        config::write::<u32>(status::REQUESTS, requests);
        config::write::<u32>(status::STAGE, SENTINEL);
    }
}

catten_rt::entry!(main);
