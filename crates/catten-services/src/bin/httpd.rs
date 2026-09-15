//! Minimal hardcoded HTTP server exposing a full report of a node's state.
//!
//! Listens on TCP port 80 through the tcpip service, and for each connection
//! reads the request, then replies with a JSON document aggregating
//! observable state across the node:
//!
//! - `node`    — NIC MAC + link state (`net::OP_STATUS`)
//! - `meta`    — wall-clock derived from the observe snapshot's monotonic counter (uptime,
//!   inter-request interval, counter frequency)
//! - `ns`      — name-service registry catalog + pending lookups (`ns::OP_STATUS`, via the
//!   bootstrap connection)
//! - `tcpip`   — tcpip service counters (`socket::OP_STATUS`)
//! - `frouter` — frame demultiplexer counters (`frouter::OP_STATUS`)
//! - `dns`     — Raft leader/term/catalog (`dns::OP_STATUS`) plus the replicated cluster posture
//!   (`raft::OP_CLUSTER_STATUS`)
//! - `disco`   — discovered peers (`disco::OP_STATUS`) plus probe-traffic counters
//!   (`disco::OP_DIAG`) and the live peer list (`disco::OP_LIST_PEERS`)
//! - `relmsg`  — reliable-message transport (`relmsg::OP_STATUS`) plus live delivery/retransmit
//!   counters (`relmsg::OP_DIAG`)
//! - `threads` — system-wide thread statistics via the observe service's `OP_THREAD_SNAPSHOT`
//!   (backed by the kernel SystemObserver capability)
//! - `history` — the observe service's bounded resource-history ring (`OP_HISTORY`)
//! - `http`    — this server's own counters and request rate
//!
//! Cumulative counters are paired with `*_delta`/`*_rate` fields measured
//! between consecutive requests, so the report reflects activity rather than
//! lifetime totals.
//!
//! Services that are not running are rendered as `null`; the aggregator uses
//! non-blocking `ns::OP_TRY_LOOKUP` so an absent service never stalls a
//! request. This is a deliberate keyhole, not a general-purpose web server:
//! it has a fixed set of read-only targets, no keep-alive, and serves one
//! connection at a time.
//!
//! The request target selects either the node or cluster view:
//!
//! - `GET /` (or `/index.html`) returns a self-refreshing HTML dashboard whose embedded script
//!   polls `GET /metrics` every five seconds; and
//! - `GET /metrics` (alias `/metric`) returns the node JSON report described above;
//! - `GET /cluster` returns the cluster dashboard; and
//! - `GET /cluster/metrics` returns a bounded snapshot of committed placement, readiness, capacity
//!   posture, and cluster ingress projection.
//!
//! Anything else is a `404`.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use core::fmt::Write as _;

use catten_rt::{
    Context,
    ShutdownRequest,
    config,
    owned::{
        Connection,
        ConnectionRef,
        OwnedMemory,
    },
};
use catten_services::{
    cluster_observe,
    disco,
    dns,
    frouter,
    net,
    ns,
    observability,
    raft,
    relmsg,
    sleep_ms,
    socket,
    wait_for_local_ready_or_shutdown,
    wait_for_registered_name_owned,
};
use catten_syscall::{
    OBSERVABILITY_NONE,
    THREAD_STATISTICS_DOMAIN_RECORD_U64S,
    THREAD_STATISTICS_HEADER_U64S,
    THREAD_STATISTICS_MAGIC,
    THREAD_STATISTICS_RECORD_U64S,
    THREAD_STATISTICS_VERSION,
    thread_domain_record as thread_domain,
    thread_exit,
    thread_statistics_header as thread_header,
    thread_statistics_record as thread_record,
};
use charlotte_launch::httpd_status as status;
use charlotte_protocol_disco::parse_peer_list;
use charlotte_protocol_msg::unpack_mac;
use charlotte_protocol_net::decode_status;

const HTTP_PORT: u16 = 80;
const ACCEPT_POLL_MS: u64 = 50;
const SENTINEL: u32 = 0x4854_5450; // "HTTP"
/// Cap on rendered thread rows. Set above the steady-state thread count so
/// every scheduler-visible thread (including this service) is represented; the
/// response is already multi-segment, so a single TCP segment is no bound.
const THREAD_SAMPLE_ROWS: usize = 64;
/// Cap on rendered per-domain resource rows in the observe snapshot.
const DOMAIN_SAMPLE_ROWS: usize = 128;
/// Cap on rendered history samples in the JSON report.
const HISTORY_RENDER_ROWS: usize = 120;

/// Extract the request target from a `GET <path> HTTP/1.1` request line.
/// Returns `/` when the line is unparsable, so a malformed request still
/// reaches the dashboard rather than an error page.
fn request_path(request: &[u8]) -> &[u8] {
    let line = request.split(|&b| b == b'\r' || b == b'\n').next().unwrap_or(b"");
    let Some(rest) = line.strip_prefix(b"GET ") else {
        return b"/";
    };
    rest.split(|&b| b == b' ').next().unwrap_or(b"/")
}

/// Self-refreshing dashboard served at `/`. The embedded script polls
/// `/metrics` every five seconds and renders the JSON report as cards.
const DASHBOARD: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CharlotteOS</title>
<style>
:root{--bg:#0e1117;--card:#161b22;--border:#2a2f3a;--fg:#e6edf3;--dim:#8b949e;--accent:#58a6ff;--ok:#3fb950;--bad:#f25d3f}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
header{position:sticky;top:0;display:flex;flex-wrap:wrap;gap:6px 22px;align-items:center;padding:12px 16px;border-bottom:1px solid var(--border);background:var(--bg);z-index:2}
header h1{margin:0;font-size:15px;color:var(--accent);letter-spacing:.03em}
.chip{color:var(--dim)}.chip b{color:var(--fg);font-weight:600}
.dot{display:inline-block;width:8px;height:8px;border-radius:50%;background:var(--bad);margin-right:5px}
.dot.up{background:var(--ok)}
main{display:grid;grid-template-columns:repeat(auto-fill,minmax(320px,1fr));gap:12px;padding:12px}
#wide{display:flex;flex-direction:column;gap:12px;padding:0 12px 12px}
.card{background:var(--card);border:1px solid var(--border);border-radius:8px;padding:12px}
.card h2{margin:0 0 8px;font-size:12px;font-weight:600;text-transform:uppercase;letter-spacing:.05em;color:var(--accent)}
table{width:100%;border-collapse:collapse}
td{padding:2px 0;vertical-align:top}
td.k{color:var(--dim);padding-right:14px;white-space:nowrap}
td.v{word-break:break-all}
table.sub th{text-align:left;color:var(--dim);font-weight:600;border-bottom:1px solid var(--border);padding:2px 8px 2px 0}
table.sub td{padding:2px 8px 2px 0;border-bottom:1px solid rgba(42,47,58,.5)}
pre{margin:0;font:11px/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;color:var(--dim);white-space:pre-wrap;word-break:break-all}
.dim{color:var(--dim)}
#err{display:none;margin:12px 12px 0;padding:8px 12px;border:1px solid var(--bad);border-radius:6px;color:var(--bad)}
</style>
</head>
<body>
<header>
<h1>CharlotteOS</h1>
<span class="chip">mac <b id="mac">-</b></span>
<span class="chip">ip <b id="ip">-</b></span>
<span class="chip"><span class="dot" id="linkdot"></span>link <b id="link">-</b></span>
<span class="chip">uptime <b id="uptime">-</b></span>
<span class="chip">requests <b id="requests">-</b></span>
<span class="chip">updated <b id="age">-</b></span>
</header>
<div id="err"></div>
<main id="root"></main>
<div id="wide"></div>
<script>
const CARDS=[["tcpip","TCP/IP"],["frouter","Frame Router"],["dns","Distributed Names"],["disco","Discovery"],["relmsg","Reliable Messages"],["http","HTTP"]];
const WIDE=[["ns","Name Service"],["threads","Threads"]];
function byId(id){return document.getElementById(id)}
function esc(s){return String(s).replace(/[&<>"]/g,function(c){return c==="&"?"&amp;":c==="<"?"&lt;":c===">"?"&gt;":"&quot;"})}
function isObj(v){return v!=null&&typeof v==="object"&&!Array.isArray(v)}
function fmtDuration(ms){if(ms==null)return"-";var s=Math.floor(ms/1000),d=Math.floor(s/86400);s-=d*86400;var h=Math.floor(s/3600);s-=h*3600;var m=Math.floor(s/60);s-=m*60;if(d)return d+"d "+h+"h";if(h)return h+"h "+m+"m";if(m)return m+"m "+s+"s";return s+"s"}
function tableOfRows(rows){if(!rows.length)return '<span class="dim">empty</span>';var cols=[],seen={};rows.forEach(function(r){if(isObj(r))Object.keys(r).forEach(function(k){if(!seen[k]){seen[k]=1;cols.push(k)}})});var h='<table class="sub"><tr>'+cols.map(function(c){return"<th>"+esc(c)+"</th>"}).join("")+"</tr>";rows.forEach(function(r){h+="<tr>"+cols.map(function(c){var v=r[c];return"<td>"+(v==null?"-":(typeof v==="object"?"<pre>"+esc(JSON.stringify(v))+"</pre>":esc(v)))+"</td>"}).join("")+"</tr>"});return h+"</table>"}
function fmtValue(v){if(v==null)return '<span class="dim">null</span>';if(typeof v==="number")return String(v);if(typeof v==="boolean")return v?"true":"false";if(typeof v==="string")return esc(v);return "<pre>"+esc(JSON.stringify(v,null,1))+"</pre>"}
function renderCard(title,obj){if(obj==null)return '<section class="card"><h2>'+esc(title)+'</h2><span class="dim">not running</span></section>';var body="";Object.keys(obj).forEach(function(k){var v=obj[k];body+="<tr><td class=\"k\">"+esc(k)+"</td><td class=\"v\">"+(Array.isArray(v)&&v.every(isObj)?tableOfRows(v):fmtValue(v))+"</td></tr>"});return '<section class="card"><h2>'+esc(title)+'</h2><table>'+body+"</table></section>"}
function show(d){var node=d.node||{},tcp=d.tcpip||{},meta=d.meta||{},http=d.http||{};byId("mac").textContent=node.mac||"-";byId("ip").textContent=tcp.ip||"-";byId("link").textContent=node.link==null?"-":(node.link===1?"up":"down");byId("linkdot").className="dot"+(node.link===1?" up":"");byId("uptime").textContent=fmtDuration(meta.uptime_ms);byId("requests").textContent=http.requests==null?"-":http.requests;byId("root").innerHTML=CARDS.map(function(c){return renderCard(c[1],d[c[0]])}).join("");byId("wide").innerHTML=WIDE.map(function(c){return renderCard(c[1],d[c[0]])}).join("");last=Date.now()}
function err(e){var el=byId("err");if(e){el.style.display="block";el.textContent="metrics unreachable: "+e}else{el.style.display="none"}}
async function poll(){try{var r=await fetch("/metrics",{cache:"no-store"});if(!r.ok)throw new Error("HTTP "+r.status);show(await r.json());err(null)}catch(e){err(e)}}
var last=Date.now();setInterval(function(){byId("age").textContent=Math.max(0,Math.round((Date.now()-last)/1000))+"s ago"},1000);
poll();setInterval(poll,5000);
</script>
</body>
</html>
"##;

/// Cluster-oriented keyhole. The HTML is intentionally only a presentation
/// adapter: `/cluster/metrics` supplies the same bounded, versioned snapshot
/// that a non-browser operations client can consume.
const CLUSTER_DASHBOARD: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>CharlotteOS cluster</title>
<style>
:root{--bg:#0e1117;--card:#161b22;--border:#30363d;--fg:#e6edf3;--dim:#8b949e;--accent:#58a6ff;--ok:#3fb950;--bad:#f85149}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
header{position:sticky;top:0;padding:12px 16px;border-bottom:1px solid var(--border);background:var(--bg);z-index:2;display:flex;gap:18px;align-items:center;flex-wrap:wrap}
h1{margin:0;color:var(--accent);font-size:16px}.dim{color:var(--dim)}.ok{color:var(--ok)}.bad{color:var(--bad)}
main{padding:12px;display:grid;gap:12px}.card{background:var(--card);border:1px solid var(--border);border-radius:8px;padding:12px;overflow:auto}
h2{margin:0 0 8px;color:var(--accent);font-size:12px;text-transform:uppercase}table{width:100%;border-collapse:collapse}th,td{text-align:left;padding:4px 10px 4px 0;border-bottom:1px solid var(--border);white-space:nowrap}th{color:var(--dim)}a{color:var(--accent)}
</style>
</head>
<body>
<header><h1>CharlotteOS cluster</h1><span id="leader" class="dim">leader -</span><span id="term" class="dim">term -</span><span id="commit" class="dim">commit -</span><span id="fresh" class="bad">waiting</span><a href="/">serving node</a></header>
<main><section class="card"><h2>Nodes</h2><div id="nodes"></div></section><section class="card"><h2>Deployments</h2><div id="deployments"></div></section><section class="card"><h2>Cluster ingress</h2><div id="ingress"></div></section><section class="card"><h2>Placement controller</h2><pre id="controller"></pre></section></main>
<script>
function e(s){return String(s==null?"-":s).replace(/[&<>\"]/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]))}
function table(rows,cols){if(!rows.length)return '<span class="dim">empty</span>';let h='<table><tr>'+cols.map(c=>'<th>'+e(c[0])+'</th>').join('')+'</tr>';for(const r of rows)h+='<tr>'+cols.map(c=>'<td>'+e(c[1](r))+'</td>').join('')+'</tr>';return h+'</table>'}
function list(v){return (v||[]).join(', ')}
function show(d){leader.textContent='leader '+(d.raft.leader||'-');term.textContent='term '+d.raft.term;commit.textContent='commit '+d.raft.commit_index;fresh.textContent=d.fresh_committed?(d.truncated?'fresh · truncated':'fresh committed'):'stale';fresh.className=d.fresh_committed?'ok':'bad';nodes.innerHTML=table(d.nodes,[["node",r=>r.node_key],["role",r=>r.leader?'leader':(r.self?'serving node':'member')],["draining",r=>r.draining],["MAC",r=>r.mac],["capacity fresh",r=>r.capacity&&r.capacity.fresh],["free frames",r=>r.capacity&&r.capacity.free_frames],["committed",r=>r.committed_frames],["CPU permille",r=>r.capacity&&r.capacity.cpu_load_permille]]);deployments.innerHTML=table(d.deployments,[["application",r=>r.name],["state",r=>r.state],["generation",r=>r.generation],["desired",r=>list(r.desired_nodes)],["ready",r=>list(r.ready_nodes)],["frames/replica",r=>r.demand_frames]]);ingress.innerHTML=table(d.ingress,[["VIP",r=>r.vip+':'+r.port],["service",r=>r.backend_name||'platform'],["advertiser",r=>r.advertiser_node],["eligible",r=>list(r.eligible_nodes)],["epoch",r=>r.epoch]]);controller.textContent=JSON.stringify(d.controller,null,2)}
async function poll(){try{const r=await fetch('/cluster/metrics',{cache:'no-store'});if(!r.ok)throw Error('HTTP '+r.status);show(await r.json())}catch(err){fresh.textContent=err;fresh.className='bad'}}poll();setInterval(poll,5000)
</script>
</body>
</html>
"##;

struct ServiceSet<'context> {
    ns_conn: ConnectionRef<'context>,
    tcp_conn: Connection,
    frouter_conn: Option<Connection>,
    dns_conn: Option<Connection>,
    disco_conn: Option<Connection>,
    relmsg_conn: Option<Connection>,
    observe_conn: Option<Connection>,
}

fn fail(code: u32) -> ! {
    config::write::<u32>(status::ERROR, code);
    catten_syscall::el0_log(0x4854_5444, 0xfa00_0000 | code as u64);
    unsafe { thread_exit() };
}

/// Non-blocking name-service lookup; `None` if the service is not registered.
fn try_lookup(ns_conn: ConnectionRef<'_>, name: u64) -> Option<Connection> {
    let result = ns_conn.call(ns::OP_TRY_LOOKUP, name).ok()?.wait().ok()?;
    (result.result >= 1).then_some(result.connection?)
}

/// Scalar status call; `None` on failure.
fn call_scalar(conn: ConnectionRef<'_>, opcode: u32, arg0: u64) -> Option<i64> {
    conn.call(opcode, arg0).ok()?.wait().ok().map(|reply| reply.result)
}

/// Call `opcode` and copy `words` little-endian u32 words out of the moved
/// reply page.
fn read_words(
    conn: ConnectionRef<'_>,
    opcode: u32,
    arg0: u64,
    words: usize,
) -> Option<alloc::vec::Vec<u32>> {
    let bytes = read_moved(conn, opcode, arg0, words.checked_mul(4)?)?;
    if bytes.len() != words * 4 {
        return None;
    }
    Some(bytes.as_chunks::<4>().0.iter().copied().map(u32::from_le_bytes).collect())
}

/// Call `opcode` and copy up to `max_len` raw bytes out of the moved reply
/// page (for variable-length payloads such as peer lists and cluster status).
fn read_moved(
    conn: ConnectionRef<'_>,
    opcode: u32,
    arg0: u64,
    max_len: usize,
) -> Option<alloc::vec::Vec<u8>> {
    let result = conn.call(opcode, arg0).ok()?.wait().ok()?;
    if result.result < 0 {
        return None;
    }
    let memory = result.memory?;
    let len = (result.result as usize).min(max_len).min(memory.len());
    let mapping = memory.map_read_only().ok()?;
    Some(mapping.as_slice()[..len].to_vec())
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?))
}

/// Convert a per-interval counter delta to a per-second integer rate.
fn rate(delta: u32, interval_ms: u64) -> u64 {
    (delta as u64 * 1000).checked_div(interval_ms).unwrap_or(0)
}

/// Append a JSON string, escaping quotes/backslashes and non-ASCII bytes.
fn push_json_string(s: &mut String, bytes: &[u8]) {
    s.push('"');
    for &b in bytes {
        match b {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            0x20..=0x7e => s.push(b as char),
            _ => {
                let _ = write!(s, "\\u{:04x}", b);
            }
        }
    }
    s.push('"');
}

fn push_node_key(s: &mut String, node_key: u64) {
    let _ = write!(s, "\"{node_key:016x}\"");
}

fn build_cluster_json(dns_conn: ConnectionRef<'_>) -> Option<String> {
    let bytes =
        read_moved(dns_conn, dns::OP_CLUSTER_SNAPSHOT, 0, cluster_observe::MAX_SNAPSHOT_LEN)?;
    let snapshot = cluster_observe::decode(&bytes)?;
    let mut s = String::new();
    let _ = write!(
        s,
        concat!(
            "{{\"schema\":\"charlotte.cluster.v1\",",
            "\"fresh_committed\":{},\"served_by_leader\":{},\"truncated\":{},",
            "\"raft\":{{\"state\":\"{}\",\"term\":{},\"commit_index\":{},",
            "\"membership_epoch\":{},\"observed_millis\":{},\"leader\":"
        ),
        snapshot.flags & cluster_observe::FLAG_FRESH_COMMITTED != 0,
        snapshot.flags & cluster_observe::FLAG_LOCAL_LEADER != 0,
        snapshot.flags & cluster_observe::FLAG_TRUNCATED != 0,
        state_name(snapshot.state as u64),
        snapshot.term,
        snapshot.commit_index,
        snapshot.membership_epoch,
        snapshot.observed_millis,
    );
    if snapshot.leader_id.is_empty() {
        s.push_str("null");
    } else {
        push_json_string(&mut s, &snapshot.leader_id);
    }
    s.push_str(",\"served_by\":");
    push_json_string(&mut s, &snapshot.self_id);
    let _ = write!(
        s,
        concat!(
            "}},\"controller\":{{\"local\":{},\"capacity_reports_accepted\":{},",
            "\"capacity_commands_proposed\":{},\"placement_reassignments\":{},",
            "\"forced_reassignments\":{}}},\"nodes\":["
        ),
        snapshot.flags & cluster_observe::FLAG_LOCAL_LEADER != 0,
        snapshot.controller.capacity_reports_accepted,
        snapshot.controller.capacity_commands_proposed,
        snapshot.controller.placement_reassignments,
        snapshot.controller.forced_reassignments,
    );
    for (index, node) in snapshot.nodes.iter().enumerate() {
        if index != 0 {
            s.push(',');
        }
        s.push_str("{\"node_key\":");
        push_node_key(&mut s, node.node_key);
        let _ = write!(
            s,
            concat!(
                ",\"member\":{},\"self\":{},\"leader\":{},\"draining\":{},",
                "\"committed_frames\":{},\"mac\":"
            ),
            node.flags & cluster_observe::NODE_MEMBER != 0,
            node.flags & cluster_observe::NODE_SELF != 0,
            node.flags & cluster_observe::NODE_LEADER != 0,
            node.flags & cluster_observe::NODE_DRAINING != 0,
            node.committed_frames,
        );
        if node.flags & cluster_observe::NODE_MAC_PRESENT != 0 {
            let _ = write!(
                s,
                "\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\"",
                node.mac[0], node.mac[1], node.mac[2], node.mac[3], node.mac[4], node.mac[5]
            );
        } else {
            s.push_str("null");
        }
        s.push_str(",\"management_endpoint\":null,\"capacity\":");
        if node.flags & cluster_observe::NODE_CAPACITY_PRESENT != 0 {
            let _ = write!(
                s,
                concat!(
                    "{{\"fresh\":{},\"boot_nonce\":{},\"epoch\":{},",
                    "\"free_frames\":{},\"usable_frames\":{},\"cpu_load_permille\":"
                ),
                node.flags & cluster_observe::NODE_CAPACITY_FRESH != 0,
                node.capacity_boot_nonce,
                node.capacity_epoch,
                node.free_frames,
                node.usable_frames,
            );
            if node.cpu_load_permille == u16::MAX {
                s.push_str("null");
            } else {
                let _ = write!(s, "{}", node.cpu_load_permille);
            }
            s.push('}');
        } else {
            s.push_str("null");
        }
        s.push('}');
    }
    s.push_str("],\"deployments\":[");
    for (index, deployment) in snapshot.deployments.iter().enumerate() {
        if index != 0 {
            s.push(',');
        }
        s.push_str("{\"name\":");
        push_json_string(&mut s, &deployment.name);
        let rollout = match deployment.state {
            catten_services::clusterctl::ROLLOUT_COMMITTED => "committed",
            catten_services::clusterctl::ROLLOUT_READY => "ready",
            catten_services::clusterctl::ROLLOUT_REPLACING => "replacing",
            _ => "unknown",
        };
        let _ = write!(
            s,
            concat!(
                ",\"state\":\"{}\",\"generation\":{},\"service_generation\":{},",
                "\"object_id\":{},\"demand_frames\":{},\"desired_nodes\":["
            ),
            rollout,
            deployment.generation,
            deployment.service_generation,
            deployment.object_id,
            deployment.demand_frames,
        );
        for (node_index, node) in deployment.desired_nodes.iter().enumerate() {
            if node_index != 0 {
                s.push(',');
            }
            push_node_key(&mut s, *node);
        }
        s.push_str("],\"ready_nodes\":[");
        for (node_index, node) in deployment.ready_nodes.iter().enumerate() {
            if node_index != 0 {
                s.push(',');
            }
            push_node_key(&mut s, *node);
        }
        s.push_str("]}");
    }
    s.push_str("],\"ingress\":[");
    for (index, ingress) in snapshot.ingress.iter().enumerate() {
        if index != 0 {
            s.push(',');
        }
        let _ = write!(
            s,
            concat!("{{\"vip\":\"{}.{}.{}.{}\",\"port\":{},\"protocol\":{},", "\"backend_name\":"),
            ingress.service.address[0],
            ingress.service.address[1],
            ingress.service.address[2],
            ingress.service.address[3],
            ingress.service.port,
            ingress.service.protocol,
        );
        if let Some(name) = &ingress.backend_name {
            push_json_string(&mut s, name);
        } else {
            s.push_str("null");
        }
        let _ = write!(
            s,
            ",\"projection_present\":{},\"member_count\":{},\"epoch\":{},\"advertiser_node\":",
            ingress.projection_present, ingress.member_count, ingress.epoch,
        );
        if let Some(node) = ingress.advertiser_node {
            push_node_key(&mut s, node);
        } else {
            s.push_str("null");
        }
        s.push_str(",\"eligible_nodes\":[");
        for (node_index, node) in ingress.eligible_nodes.iter().enumerate() {
            if node_index != 0 {
                s.push(',');
            }
            push_node_key(&mut s, *node);
        }
        s.push_str("]}");
    }
    s.push_str("]}");
    Some(s)
}

struct Prev {
    initialized: bool,
    mono_ticks: u64,
    rx_frames: u32,
    tx_sends: u32,
    frouter_rx: u32,
    forwarded: u32,
    heap_allocations: u64,
    cpu_busy_ticks: u64,
}

/// This service's own request counters, reported under the `http` key so the
/// dashboard surfaces the keyhole's traffic alongside the other services.
struct HttpCounters {
    requests: u32,
    bytes_sent: u64,
    root: u32,
    metrics: u32,
    other: u32,
}

struct ThreadRow {
    tid: u64,
    generation: u64,
    asid: u64,
    state: u64,
    affinity_lp: u64,
    pinned_lp: u64,
    dispatch: u64,
    sample_count: u64,
    min_ticks: u64,
    max_ticks: u64,
    runtime_ticks: u128,
    saturated: u64,
    stack_pages: u64,
    stack_committed_pages: u64,
    stack_used_pages: u64,
}

struct DomainRow {
    asid: u64,
    owned_frames: u64,
    user_stack_pages: u64,
    user_stack_pages_high_water: u64,
    stack_pages_used_high_water: u64,
    threads: u64,
    threads_high_water: u64,
    heap_valid: u64,
    heap_capacity_bytes: u64,
    heap_allocated_bytes: u64,
    heap_peak_bytes: u64,
    heap_allocations: u64,
    heap_total_allocated_bytes: u64,
    heap_lock_spins: u64,
}

struct ThreadReport {
    freq_hz: u64,
    mono_ticks: u64,
    free_frames: u64,
    usable_frames: u64,
    logical_processors: u64,
    cpu_busy_ticks: u64,
    rows: alloc::vec::Vec<ThreadRow>,
    domains: alloc::vec::Vec<DomainRow>,
}

struct HistoryRow {
    ticks: u64,
    free_frames: u64,
    usable_frames: u64,
    logical_processors: u64,
    cpu_busy_ticks: u64,
    threads: u64,
    domains: u64,
    owned_frames: u64,
    heap_allocated_bytes: u64,
    heap_peak_bytes: u64,
    stack_pages: u64,
    stack_used_high_water: u64,
    threads_high_water: u64,
}

struct HistoryReport {
    interval_ms: u64,
    rows: alloc::vec::Vec<HistoryRow>,
}

/// Fetch and parse the observe service's system-wide thread snapshot
/// (`CCOSTAT1` wire format).
fn thread_report(observe_conn: ConnectionRef<'_>) -> Option<ThreadReport> {
    let header_words = THREAD_STATISTICS_HEADER_U64S;
    let word_bytes = core::mem::size_of::<u64>();
    let max_len = (header_words
        + THREAD_SAMPLE_ROWS * THREAD_STATISTICS_RECORD_U64S
        + DOMAIN_SAMPLE_ROWS * THREAD_STATISTICS_DOMAIN_RECORD_U64S)
        .checked_mul(word_bytes)?;
    let bytes = read_moved(observe_conn, observability::OP_THREAD_SNAPSHOT, 0, max_len)?;
    let len = bytes.len();
    let mut header = [0u64; THREAD_STATISTICS_HEADER_U64S];
    for (slot, word) in header.iter_mut().zip(bytes.chunks_exact(word_bytes)) {
        *slot = u64::from_le_bytes(word.try_into().ok()?);
    }
    if header[thread_header::MAGIC] != THREAD_STATISTICS_MAGIC
        || header[thread_header::VERSION] != THREAD_STATISTICS_VERSION
        || header[thread_header::RECORD_BYTES]
            != (THREAD_STATISTICS_RECORD_U64S * word_bytes) as u64
        || header[thread_header::DOMAIN_RECORD_BYTES]
            != (THREAD_STATISTICS_DOMAIN_RECORD_U64S * word_bytes) as u64
    {
        return None;
    }
    let max_by_len = (len.saturating_sub(header_words * word_bytes))
        / (THREAD_STATISTICS_RECORD_U64S * word_bytes);
    let count = (header[thread_header::RECORD_COUNT] as usize).min(max_by_len);
    let mut rows = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        let base = (header_words + i * THREAD_STATISTICS_RECORD_U64S) * word_bytes;
        let mut rec: [u64; THREAD_STATISTICS_RECORD_U64S] = [0; THREAD_STATISTICS_RECORD_U64S];
        for (slot, word) in rec.iter_mut().zip(bytes[base..].chunks_exact(word_bytes)) {
            *slot = u64::from_le_bytes(word.try_into().ok()?);
        }
        rows.push(ThreadRow {
            tid: rec[thread_record::TID],
            generation: rec[thread_record::GENERATION],
            asid: rec[thread_record::ASID],
            state: rec[thread_record::STATE],
            affinity_lp: rec[thread_record::AFFINITY_LP],
            pinned_lp: rec[thread_record::PINNED_LP],
            dispatch: rec[thread_record::DISPATCH_COUNT],
            sample_count: rec[thread_record::SAMPLE_COUNT],
            min_ticks: rec[thread_record::MIN_TICKS],
            max_ticks: rec[thread_record::MAX_TICKS],
            runtime_ticks: ((rec[thread_record::TOTAL_TICKS_HIGH] as u128) << 64)
                | rec[thread_record::TOTAL_TICKS_LOW] as u128,
            saturated: rec[thread_record::SATURATED],
            stack_pages: rec[thread_record::STACK_RESERVED_PAGES],
            stack_committed_pages: rec[thread_record::STACK_COMMITTED_PAGES],
            stack_used_pages: rec[thread_record::STACK_USED_PAGES],
        });
    }
    let domain_base_words = header_words + count * THREAD_STATISTICS_RECORD_U64S;
    let max_domains_by_len = (len.saturating_sub(domain_base_words * word_bytes))
        / (THREAD_STATISTICS_DOMAIN_RECORD_U64S * word_bytes);
    let domain_count =
        (header[thread_header::DOMAIN_RECORD_COUNT] as usize).min(max_domains_by_len);
    let mut domains = alloc::vec::Vec::with_capacity(domain_count);
    for i in 0..domain_count {
        let base = (domain_base_words + i * THREAD_STATISTICS_DOMAIN_RECORD_U64S) * word_bytes;
        let mut rec: [u64; THREAD_STATISTICS_DOMAIN_RECORD_U64S] =
            [0; THREAD_STATISTICS_DOMAIN_RECORD_U64S];
        for (slot, word) in rec.iter_mut().zip(bytes[base..].chunks_exact(word_bytes)) {
            *slot = u64::from_le_bytes(word.try_into().ok()?);
        }
        domains.push(DomainRow {
            asid: rec[thread_domain::ASID],
            owned_frames: rec[thread_domain::OWNED_FRAMES],
            user_stack_pages: rec[thread_domain::USER_STACK_PAGES],
            user_stack_pages_high_water: rec[thread_domain::USER_STACK_PAGES_HIGH_WATER],
            stack_pages_used_high_water: rec[thread_domain::STACK_PAGES_USED_HIGH_WATER],
            threads: rec[thread_domain::THREADS],
            threads_high_water: rec[thread_domain::THREADS_HIGH_WATER],
            heap_valid: rec[thread_domain::HEAP_STATUS_VALID],
            heap_capacity_bytes: rec[thread_domain::HEAP_CAPACITY_BYTES],
            heap_allocated_bytes: rec[thread_domain::HEAP_ALLOCATED_BYTES],
            heap_peak_bytes: rec[thread_domain::HEAP_PEAK_BYTES],
            heap_allocations: rec[thread_domain::HEAP_ALLOCATIONS],
            heap_total_allocated_bytes: rec[thread_domain::HEAP_TOTAL_ALLOCATED_BYTES],
            heap_lock_spins: rec[thread_domain::HEAP_LOCK_SPINS],
        });
    }
    Some(ThreadReport {
        freq_hz: header[thread_header::COUNTER_FREQUENCY_HZ],
        mono_ticks: header[thread_header::MONOTONIC_TICKS],
        free_frames: header[thread_header::FREE_FRAMES],
        usable_frames: header[thread_header::USABLE_FRAMES],
        logical_processors: header[thread_header::LOGICAL_PROCESSORS],
        cpu_busy_ticks: header[thread_header::CPU_BUSY_TICKS],
        rows,
        domains,
    })
}

/// Fetch and parse the observe service's bounded resource history.
fn history_report(observe_conn: ConnectionRef<'_>) -> Option<HistoryReport> {
    use observability::{
        HISTORY_MAGIC,
        HISTORY_VERSION,
        history_header as header,
        history_record as record,
    };

    let word_bytes = core::mem::size_of::<u64>();
    let max_len = (header::WORDS + observability::HISTORY_CAPACITY * record::WORDS)
        .checked_mul(word_bytes)?;
    let bytes = read_moved(observe_conn, observability::OP_HISTORY, 0, max_len)?;
    let len = bytes.len();
    let mut history_header_words = [0u64; header::WORDS];
    for (slot, word) in history_header_words.iter_mut().zip(bytes.chunks_exact(word_bytes)) {
        *slot = u64::from_le_bytes(word.try_into().ok()?);
    }
    if history_header_words[header::MAGIC] != HISTORY_MAGIC
        || history_header_words[header::VERSION] != HISTORY_VERSION
        || history_header_words[header::RECORD_BYTES] != (record::WORDS * word_bytes) as u64
    {
        return None;
    }
    let max_by_len =
        (len.saturating_sub(header::WORDS * word_bytes)) / (record::WORDS * word_bytes);
    let count = (history_header_words[header::RECORD_COUNT] as usize).min(max_by_len);
    let mut rows = alloc::vec::Vec::with_capacity(count);
    for i in 0..count {
        let base = (header::WORDS + i * record::WORDS) * word_bytes;
        let mut rec: [u64; record::WORDS] = [0; record::WORDS];
        for (slot, word) in rec.iter_mut().zip(bytes[base..].chunks_exact(word_bytes)) {
            *slot = u64::from_le_bytes(word.try_into().ok()?);
        }
        rows.push(HistoryRow {
            ticks: rec[record::MONOTONIC_TICKS],
            free_frames: rec[record::FREE_FRAMES],
            usable_frames: rec[record::USABLE_FRAMES],
            logical_processors: rec[record::LOGICAL_PROCESSORS],
            cpu_busy_ticks: rec[record::CPU_BUSY_TICKS],
            threads: rec[record::THREADS],
            domains: rec[record::DOMAINS],
            owned_frames: rec[record::OWNED_FRAMES],
            heap_allocated_bytes: rec[record::HEAP_ALLOCATED_BYTES],
            heap_peak_bytes: rec[record::HEAP_PEAK_BYTES],
            stack_pages: rec[record::STACK_PAGES],
            stack_used_high_water: rec[record::STACK_USED_HIGH_WATER],
            threads_high_water: rec[record::THREADS_HIGH_WATER],
        });
    }
    Some(HistoryReport {
        interval_ms: history_header_words[header::SAMPLE_INTERVAL_MS],
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

fn render_dns(s: &mut String, dns_conn: ConnectionRef<'_>) {
    if let Some(result) = call_scalar(dns_conn, dns::OP_STATUS, 0) {
        let v = result as u64;
        let _ = write!(
            s,
            "\"dns\":{{\"state\":\"{}\",\"term\":{},\"catalog\":",
            state_name(v & 0xff),
            (v >> 8) & 0xff_ffff
        );
        // Dump the replicated name -> node catalog.
        let mut rendered = false;
        if let Some(bytes) = read_moved(dns_conn, dns::OP_CATALOG, 0, 4096)
            && let Some(count) = u32_at(&bytes, 0)
        {
            let _ = write!(s, "{{\"count\":{},\"entries\":{{", count);
            let mut offset = dns::CATALOG_HEADER_BYTES;
            let mut emitted = 0u32;
            while emitted < count && offset + 2 < bytes.len() {
                let name_len = bytes[offset] as usize;
                let node_offset = offset + 1 + name_len;
                if node_offset + 1 > bytes.len() {
                    break;
                }
                let node_len = bytes[node_offset] as usize;
                let generation_offset = node_offset + 1 + node_len;
                let Some(generation) = u64_at(&bytes, generation_offset) else {
                    break;
                };
                if emitted > 0 {
                    s.push(',');
                }
                push_json_string(s, &bytes[offset + 1..node_offset]);
                s.push_str(":{\"node\":");
                push_json_string(s, &bytes[node_offset + 1..generation_offset]);
                let _ = write!(s, ",\"generation\":{generation}}}");
                offset = generation_offset + 8;
                emitted += 1;
            }
            s.push_str("}}");
            rendered = true;
        }
        if !rendered {
            let _ = write!(s, "{{\"count\":{},\"entries\":{{}}}}", (v >> 32) & 0xffff_ffff);
        }
        // Raft cluster posture: commit index, member count, and ids. Served on
        // the same endpoint as the dns opcodes.
        if let Some(bytes) = read_moved(dns_conn, raft::OP_CLUSTER_STATUS, 0, 256)
            && let Some((_state, _term, commit_index, members, leader, self_id)) =
                raft::parse_cluster_status(&bytes)
        {
            let _ = write!(s, ",\"commit_index\":{},\"members\":{}", commit_index, members);
            s.push_str(",\"leader\":");
            push_json_string(s, leader);
            s.push_str(",\"self_id\":");
            push_json_string(s, self_id);
        }
        s.push('}');
    } else {
        s.push_str("\"dns\":null");
    }
}

/// Write an LP id field as a number, or `null` when the kernel sentinel
/// `OBSERVABILITY_NONE` means "no affinity / not pinned" (runs on any LP).
fn write_lp(s: &mut String, key: &str, lp: u64) {
    s.push('"');
    s.push_str(key);
    s.push_str("\":");
    if lp == OBSERVABILITY_NONE {
        s.push_str("null");
    } else {
        let _ = write!(s, "{}", lp);
    }
}

fn render_threads(s: &mut String, report: &ThreadReport) {
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
        let runtime_ms = if report.freq_hz > 0 {
            (row.runtime_ticks * 1000) / report.freq_hz as u128
        } else {
            0
        };
        let cpu_pct = if report.mono_ticks > 0 {
            ((row.runtime_ticks * 100) / report.mono_ticks as u128) as u64
        } else {
            0
        };
        let _ = write!(
            s,
            "{{\"tid\":{},\"asid\":{},\"state\":\"{}\",",
            row.tid,
            row.asid,
            thread_state_name(row.state),
        );
        write_lp(s, "lp", row.affinity_lp);
        s.push(',');
        write_lp(s, "pinned_lp", row.pinned_lp);
        let _ = write!(
            s,
            ",\"generation\":{},\"dispatch\":{},\"samples\":{},\"runtime_ticks\":{},\"runtime_ms\"\
             :{},\"cpu_pct\":{},\"min_ticks\":{},\"max_ticks\":{},\"saturated\":{},\"stack_pages\"\
             :{},\"stack_committed_pages\":{},\"stack_used_pages\":{}}}",
            row.generation,
            row.dispatch,
            row.sample_count,
            row.runtime_ticks,
            runtime_ms,
            cpu_pct,
            row.min_ticks,
            row.max_ticks,
            row.saturated,
            row.stack_pages,
            row.stack_committed_pages,
            row.stack_used_pages
        );
    }
    s.push_str("],\"domains\":[");
    for (i, domain) in report.domains.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(
            s,
            concat!(
                "{{\"asid\":{},\"owned_frames\":{},\"stack_pages\":{},",
                "\"stack_pages_high_water\":{},\"stack_used_high_water\":{},",
                "\"threads\":{},\"threads_high_water\":{},",
                "\"heap_valid\":{},\"heap_capacity_bytes\":{},",
                "\"heap_allocated_bytes\":{},\"heap_peak_bytes\":{},",
                "\"heap_allocations\":{},\"heap_total_allocated_bytes\":{},",
                "\"heap_lock_spins\":{}}}"
            ),
            domain.asid,
            domain.owned_frames,
            domain.user_stack_pages,
            domain.user_stack_pages_high_water,
            domain.stack_pages_used_high_water,
            domain.threads,
            domain.threads_high_water,
            domain.heap_valid,
            domain.heap_capacity_bytes,
            domain.heap_allocated_bytes,
            domain.heap_peak_bytes,
            domain.heap_allocations,
            domain.heap_total_allocated_bytes,
            domain.heap_lock_spins
        );
    }
    s.push_str("]}");
}

fn render_history(s: &mut String, history: &HistoryReport) {
    let start = history.rows.len().saturating_sub(HISTORY_RENDER_ROWS);
    let _ = write!(s, "\"history\":{{\"interval_ms\":{},\"samples\":[", history.interval_ms);
    for (i, row) in history.rows[start..].iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(
            s,
            concat!(
                "{{\"ticks\":{},\"threads\":{},\"domains\":{},",
                "\"owned_frames\":{},\"stack_pages\":{},",
                "\"stack_used_high_water\":{},\"threads_high_water\":{},",
                "\"free_frames\":{},\"usable_frames\":{},",
                "\"logical_processors\":{},\"cpu_busy_ticks\":{},",
                "\"heap_allocated_bytes\":{},\"heap_peak_bytes\":{}}}"
            ),
            row.ticks,
            row.threads,
            row.domains,
            row.owned_frames,
            row.stack_pages,
            row.stack_used_high_water,
            row.threads_high_water,
            row.free_frames,
            row.usable_frames,
            row.logical_processors,
            row.cpu_busy_ticks,
            row.heap_allocated_bytes,
            row.heap_peak_bytes
        );
    }
    s.push_str("]}");
}

fn render_ns(s: &mut String, ns_conn: ConnectionRef<'_>) {
    let Some(bytes) = read_moved(ns_conn, ns::OP_STATUS, 0, 4096) else {
        s.push_str("\"ns\":null");
        return;
    };
    let Some(magic) = u32_at(&bytes, ns::STATUS_OFFSET_MAGIC as usize * 4) else {
        s.push_str("\"ns\":null");
        return;
    };
    let Some(registered) = u32_at(&bytes, ns::STATUS_OFFSET_REGISTERED as usize * 4) else {
        s.push_str("\"ns\":null");
        return;
    };
    let Some(pending) = u32_at(&bytes, ns::STATUS_OFFSET_PENDING as usize * 4) else {
        s.push_str("\"ns\":null");
        return;
    };
    if magic != ns::STATUS_MAGIC {
        s.push_str("\"ns\":null");
        return;
    }
    let _ =
        write!(s, "\"ns\":{{\"registered\":{},\"pending\":{},\"services\":[", registered, pending);
    let mut offset = ns::STATUS_HEADER_BYTES;
    let mut emitted = 0u32;
    while emitted < registered && offset < bytes.len() {
        let name_len = bytes[offset] as usize;
        let end = offset + 1 + name_len;
        let Some(name) = bytes.get(offset + 1..end) else {
            break;
        };
        if emitted > 0 {
            s.push(',');
        }
        if name.iter().all(u8::is_ascii_graphic) {
            push_json_string(s, name);
        } else {
            s.push_str("\"hex:");
            for byte in name {
                let _ = write!(s, "{byte:02x}");
            }
            s.push('"');
        }
        offset = end;
        emitted += 1;
    }
    s.push_str("]}");
}

fn disco_role_name(role: u32) -> &'static str {
    match role {
        0 => "no_cluster",
        1 => "follower",
        2 => "candidate",
        3 => "leader",
        0xff => "unknown",
        _ => "unknown",
    }
}

fn render_disco(s: &mut String, disco_conn: ConnectionRef<'_>) {
    let scalar = call_scalar(disco_conn, disco::OP_STATUS, 0);
    let (running, peers) = scalar.map_or((0u64, 0u64), |r| {
        let v = r as u64;
        (v & 0xff, (v >> 8) & 0xff)
    });
    let diag = read_words(disco_conn, disco::OP_DIAG, 0, disco::DIAG_WORDS)
        .filter(|w| w[disco::DIAG_OFFSET_MAGIC as usize] == disco::DIAG_MAGIC);

    if let Some(d) = &diag {
        let _ = write!(
            s,
            "\"disco\":{{\"running\":{},\"peers\":{},\"cluster_role\":\"{}\",\"rx_raw\":{},\"\
             sent_ok\":{},\"sent_fail\":{},\"decoded\":{},\"called\":{},\"heartbeat\":{}",
            d[disco::DIAG_OFFSET_RUNNING as usize],
            d[disco::DIAG_OFFSET_PEERS as usize],
            disco_role_name(d[disco::DIAG_OFFSET_CLUSTER_ROLE as usize]),
            d[disco::DIAG_OFFSET_RX_RAW as usize],
            d[disco::DIAG_OFFSET_SENT_OK as usize],
            d[disco::DIAG_OFFSET_SENT_FAIL as usize],
            d[disco::DIAG_OFFSET_DECODED as usize],
            d[disco::DIAG_OFFSET_CALLED as usize],
            d[disco::DIAG_OFFSET_HEARTBEAT as usize],
        );
    } else {
        let _ = write!(s, "\"disco\":{{\"running\":{},\"peers\":{}", running, peers);
    }

    // Live peer table (MAC + node id) from the cached discovery state.
    s.push_str(",\"peers_list\":[");
    if let Some(bytes) = read_moved(disco_conn, disco::OP_LIST_PEERS, 0, 4096) {
        let list = parse_peer_list(&bytes);
        for (i, (pmac, node_id)) in list.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"mac\":\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\",\"node_id\":",
                pmac[0], pmac[1], pmac[2], pmac[3], pmac[4], pmac[5]
            );
            push_json_string(s, node_id);
            s.push('}');
        }
    }
    s.push_str("]},");
}

fn render_relmsg(s: &mut String, relmsg_conn: ConnectionRef<'_>) {
    if let Some(result) = call_scalar(relmsg_conn, relmsg::OP_STATUS, 0) {
        let local_mac = unpack_mac(result as u64);
        let _ = write!(
            s,
            "\"relmsg\":{{\"local_mac\":\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\"",
            local_mac[0], local_mac[1], local_mac[2], local_mac[3], local_mac[4], local_mac[5]
        );
        if let Some(d) = read_words(relmsg_conn, relmsg::OP_DIAG, 0, relmsg::DIAG_WORDS)
            .filter(|w| w[relmsg::DIAG_OFFSET_MAGIC as usize] == relmsg::DIAG_MAGIC)
        {
            let _ = write!(
                s,
                ",\"peers\":{},\"handled\":{},\"retransmits\":{},\"send_failures\":{},\"received\"\
                 :{},\"in_flight\":{}",
                d[relmsg::DIAG_OFFSET_PEERS as usize],
                d[relmsg::DIAG_OFFSET_HANDLED as usize],
                d[relmsg::DIAG_OFFSET_RETRANSMITS as usize],
                d[relmsg::DIAG_OFFSET_SEND_FAILURES as usize],
                d[relmsg::DIAG_OFFSET_RECEIVED as usize],
                d[relmsg::DIAG_OFFSET_IN_FLIGHT as usize],
            );
        }
        s.push_str("},");
    } else {
        s.push_str("\"relmsg\":null,");
    }
}

fn build_json(
    mac: &[u8; 6],
    link: u8,
    services: &ServiceSet<'_>,
    counters: &HttpCounters,
    prev: &mut Prev,
) -> String {
    let mut s = String::new();

    // The observe snapshot doubles as the wall-clock source: uptime, inter-
    // request interval, and per-counter rates all derive from its monotonic
    // counter and frequency.
    let report = services.observe_conn.as_ref().and_then(|conn| thread_report(conn.as_ref()));
    let (freq_hz, mono_ticks) = report.as_ref().map_or((0, 0), |r| (r.freq_hz, r.mono_ticks));
    let uptime_ms = mono_ticks.saturating_mul(1000).checked_div(freq_hz).unwrap_or(0);
    let interval_ms = if prev.initialized && freq_hz > 0 && mono_ticks > prev.mono_ticks {
        (mono_ticks - prev.mono_ticks).saturating_mul(1000) / freq_hz
    } else {
        0
    };

    // node + tcpip (both mandatory at startup).
    let status = read_words(services.tcp_conn.as_ref(), socket::OP_STATUS, 0, socket::STATUS_WORDS);
    let ip = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_IP as usize]);
    let rx = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_RX_FRAMES as usize]);
    let tx = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_TX_SENDS as usize]);
    let socks = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_SOCKETS as usize]);
    let tx_err = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_TX_SEND_ERRORS as usize]);
    let dhcp_mode = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_DHCP_MODE as usize]);
    let gateway = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_GATEWAY as usize]);
    let mtu = status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_MTU as usize]);
    let socket_capacity =
        status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_SOCKET_CAPACITY as usize]);
    let socket_quota =
        status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_SOCKET_QUOTA as usize]);
    let buffer_quota =
        status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_BUFFER_QUOTA as usize]);
    let socket_ceiling =
        status.as_ref().map_or(0, |w| w[socket::STATUS_OFFSET_SOCKET_CEILING as usize]);

    let rx_delta = if prev.initialized {
        rx.saturating_sub(prev.rx_frames)
    } else {
        0
    };
    let tx_delta = if prev.initialized {
        tx.saturating_sub(prev.tx_sends)
    } else {
        0
    };

    // rustfmt mishandles the escaped quote after this line continuation.
    #[rustfmt::skip]
    let (free_frames, usable_frames) =
        report.as_ref().map_or((0, 0), |r| (r.free_frames, r.usable_frames));
    let (logical_processors, cpu_busy_ticks) =
        report.as_ref().map_or((0, 0), |r| (r.logical_processors, r.cpu_busy_ticks));
    let cpu_delta = if prev.initialized {
        cpu_busy_ticks.saturating_sub(prev.cpu_busy_ticks)
    } else {
        0
    };
    let mono_delta = if prev.initialized && mono_ticks > prev.mono_ticks {
        mono_ticks - prev.mono_ticks
    } else {
        0
    };
    let cpu_busy_pct = if logical_processors > 0 && mono_delta > 0 {
        cpu_delta.saturating_mul(100) / mono_delta.saturating_mul(logical_processors)
    } else {
        0
    };
    let _ = write!(
        &mut s,
        "{{\"meta\":{{\"uptime_ms\":{},\"interval_ms\":{},\"counter_hz\":{},\"free_frames\":{},\"\
         usable_frames\":{},\"logical_processors\":{},\"cpu_busy_ticks\":{},\"cpu_busy_pct\":{}}},\
         \"node\":{{\"mac\":\"{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}\",\"link\":{}}},",
        uptime_ms,
        interval_ms,
        freq_hz,
        free_frames,
        usable_frames,
        logical_processors,
        cpu_busy_ticks,
        cpu_busy_pct,
        mac[0],
        mac[1],
        mac[2],
        mac[3],
        mac[4],
        mac[5],
        link
    );
    let _ = write!(
        &mut s,
        r#""tcpip":{{"ip":"{}.{}.{}.{}","rx_frames":{},"tx_sends":{},"sockets":{},"listen_port":{},"tx_send_errors":{},"dhcp":{},"gateway":"{}.{}.{}.{}","mtu":{},"socket_capacity":{},"socket_ceiling":{},"socket_quota":{},"buffer_quota_bytes":{},"rx_frames_delta":{},"tx_sends_delta":{},"rx_frames_rate":{},"tx_sends_rate":{}}},"#,
        (ip >> 24) & 0xff,
        (ip >> 16) & 0xff,
        (ip >> 8) & 0xff,
        ip & 0xff,
        rx,
        tx,
        socks,
        HTTP_PORT,
        tx_err,
        dhcp_mode,
        (gateway >> 24) & 0xff,
        (gateway >> 16) & 0xff,
        (gateway >> 8) & 0xff,
        gateway & 0xff,
        mtu,
        socket_capacity,
        socket_ceiling,
        socket_quota,
        buffer_quota,
        rx_delta,
        tx_delta,
        rate(rx_delta, interval_ms),
        rate(tx_delta, interval_ms)
    );

    // name service: registered-services catalog.
    render_ns(&mut s, services.ns_conn);
    s.push(',');

    // frouter counters.
    if let Some(frouter_conn) = services.frouter_conn.as_ref() {
        if let Some(w) =
            read_words(frouter_conn.as_ref(), frouter::OP_STATUS, 0, frouter::STATUS_WORDS)
        {
            let frouter_rx = w[frouter::STATUS_OFFSET_RX as usize];
            let forwarded = w[frouter::STATUS_OFFSET_FORWARDED as usize];
            let rx_delta = if prev.initialized {
                frouter_rx.saturating_sub(prev.frouter_rx)
            } else {
                0
            };
            let forwarded_delta = if prev.initialized {
                forwarded.saturating_sub(prev.forwarded)
            } else {
                0
            };
            let ingress_epoch = u64::from(w[frouter::STATUS_OFFSET_EPOCH_LO as usize])
                | (u64::from(w[frouter::STATUS_OFFSET_EPOCH_HI as usize]) << 32);
            let _ = write!(
                &mut s,
                "\"frouter\":{{\"stage\":{},\"rx\":{},\"forwarded\":{},\"dropped\":{},\"unknown\":\
                 {},\"routes\":{},\"rx_delta\":{},\"forwarded_delta\":{},\"rx_rate\":{},\"\
                 forwarded_rate\":{},\"ingress_epoch\":{},\"ingress_backends\":{},\"\
                 ingress_members\":{},\"ingress_draining\":{},\"vip_advertiser\":{},\"\
                 is_advertiser\":{},\"ingress_local\":{},\"ingress_forwarded\":{},\"\
                 ingress_dropped\":{},\"flow_bindings\":{},\"snapshot_fresh\":{},\"\
                 snapshot_stale_dropped\":{},\"missing_epoch_dropped\":{},\"snapshot_expirations\"\
                 :{},\"ingress_services\":{}}},",
                w[frouter::STATUS_OFFSET_STAGE as usize],
                frouter_rx,
                forwarded,
                w[frouter::STATUS_OFFSET_DROPPED as usize],
                w[frouter::STATUS_OFFSET_UNKNOWN as usize],
                w[frouter::STATUS_OFFSET_ROUTES as usize],
                rx_delta,
                forwarded_delta,
                rate(rx_delta, interval_ms),
                rate(forwarded_delta, interval_ms),
                ingress_epoch,
                w[frouter::STATUS_OFFSET_BACKENDS as usize],
                w[frouter::STATUS_OFFSET_MEMBERS as usize],
                w[frouter::STATUS_OFFSET_MEMBERS as usize]
                    .saturating_sub(w[frouter::STATUS_OFFSET_BACKENDS as usize]),
                w[frouter::STATUS_OFFSET_VIP_ADVERTISER as usize],
                w[frouter::STATUS_OFFSET_IS_ADVERTISER as usize],
                w[frouter::STATUS_OFFSET_INGRESS_LOCAL as usize],
                w[frouter::STATUS_OFFSET_INGRESS_FORWARDED as usize],
                w[frouter::STATUS_OFFSET_INGRESS_DROPPED as usize],
                w[frouter::STATUS_OFFSET_FLOW_BINDINGS as usize],
                w[frouter::STATUS_OFFSET_SNAPSHOT_FRESH as usize],
                w[frouter::STATUS_OFFSET_SNAPSHOT_STALE_DROPPED as usize],
                w[frouter::STATUS_OFFSET_MISSING_EPOCH_DROPPED as usize],
                w[frouter::STATUS_OFFSET_SNAPSHOT_EXPIRATIONS as usize],
                w[frouter::STATUS_OFFSET_SERVICE_COUNT as usize]
            );
            prev.frouter_rx = frouter_rx;
            prev.forwarded = forwarded;
        } else {
            s.push_str("\"frouter\":null,");
        }
    } else {
        s.push_str("\"frouter\":null,");
    }

    // dns: Raft state/term + replicated catalog + cluster posture.
    if let Some(dns_conn) = services.dns_conn.as_ref() {
        render_dns(&mut s, dns_conn.as_ref());
        s.push(',');
    } else {
        s.push_str("\"dns\":null,");
    }

    // disco: probe-traffic counters + live peer table.
    if let Some(disco_conn) = services.disco_conn.as_ref() {
        render_disco(&mut s, disco_conn.as_ref());
    } else {
        s.push_str("\"disco\":null,");
    }

    // relmsg: transport counters + delivery/retransmit diagnostics.
    if let Some(relmsg_conn) = services.relmsg_conn.as_ref() {
        render_relmsg(&mut s, relmsg_conn.as_ref());
    } else {
        s.push_str("\"relmsg\":null,");
    }

    // observe: system-wide thread statistics.
    if let Some(r) = &report {
        render_threads(&mut s, r);
        s.push(',');
    } else {
        s.push_str("\"threads\":null,");
    }

    // observe: bounded resource history from the same service.
    match services.observe_conn.as_ref().and_then(|conn| history_report(conn.as_ref())) {
        Some(history) => {
            render_history(&mut s, &history);
            s.push(',');
        }
        None => s.push_str("\"history\":null,"),
    }

    // Node heap summary: cumulative totals plus the allocation rate since the
    // previous request. Nonzero lock spins are the contention evidence for
    // deciding whether the heap needs sharding.
    let (heap_allocations, heap_total_bytes, heap_lock_spins) =
        report.as_ref().map_or((0, 0, 0), |r| {
            r.domains.iter().fold((0u64, 0u64, 0u64), |(allocs, bytes, spins), domain| {
                (
                    allocs + domain.heap_allocations,
                    bytes + domain.heap_total_allocated_bytes,
                    spins + domain.heap_lock_spins,
                )
            })
        });
    let allocations_delta = if prev.initialized {
        heap_allocations.saturating_sub(prev.heap_allocations)
    } else {
        0
    };
    let allocations_rate = allocations_delta
        .checked_mul(1000)
        .and_then(|scaled| scaled.checked_div(interval_ms))
        .unwrap_or(0);
    let _ = write!(
        s,
        concat!(
            "\"heap\":{{\"allocations_total\":{},\"allocations_delta\":{},",
            "\"allocations_rate\":{},\"bytes_allocated_total\":{},",
            "\"lock_spins_total\":{}}},"
        ),
        heap_allocations, allocations_delta, allocations_rate, heap_total_bytes, heap_lock_spins
    );
    prev.heap_allocations = heap_allocations;

    let _ = write!(
        s,
        "\"http\":{{\"requests\":{},\"bytes_sent\":{},\"uptime_ms\":{},\"interval_ms\":{},\"\
         requests_rate\":{},\"paths\":{{\"root\":{},\"metrics\":{},\"other\":{}}}}}}}",
        counters.requests,
        counters.bytes_sent,
        uptime_ms,
        interval_ms,
        rate(1, interval_ms),
        counters.root,
        counters.metrics,
        counters.other
    );

    prev.initialized = true;
    prev.mono_ticks = mono_ticks;
    prev.cpu_busy_ticks = cpu_busy_ticks;
    prev.rx_frames = rx;
    prev.tx_sends = tx;
    s
}

fn serve(ctx: &Context) -> ShutdownRequest {
    config::write::<u32>(status::STAGE, 1);
    let ns_conn = ctx.bootstrap_connection().unwrap_or_else(|| fail(0xe001));
    let (_, net_conn) =
        wait_for_registered_name_owned(ns_conn, net::NAME).unwrap_or_else(|| fail(0xe002));
    config::write::<u32>(status::STAGE, 2);

    let status = call_scalar(net_conn.as_ref(), net::OP_STATUS, 0).unwrap_or_else(|| fail(0xe002));
    let (link, mac) = decode_status(status);
    drop(net_conn);
    config::write::<u32>(status::STAGE, 3);

    let (_, tcp_conn) =
        wait_for_registered_name_owned(ns_conn, socket::NAME).unwrap_or_else(|| fail(0xe003));
    config::write::<u32>(status::STAGE, 4);

    // Optional report sources; absent services render as null.
    let services = ServiceSet {
        ns_conn,
        tcp_conn,
        frouter_conn: try_lookup(ns_conn, frouter::NAME),
        dns_conn: try_lookup(ns_conn, dns::NAME),
        disco_conn: try_lookup(ns_conn, disco::NAME),
        relmsg_conn: try_lookup(ns_conn, relmsg::NAME),
        observe_conn: try_lookup(ns_conn, observability::NAME),
    };
    config::write::<u32>(status::STAGE, 5);

    if let Err(request) = wait_for_local_ready_or_shutdown(ctx, ns_conn) {
        return request;
    }
    config::write::<u32>(status::STAGE, 6);

    let mut counters = HttpCounters {
        requests: 0,
        bytes_sent: 0,
        root: 0,
        metrics: 0,
        other: 0,
    };
    let mut prev = Prev {
        initialized: false,
        mono_ticks: 0,
        rx_frames: 0,
        tx_sends: 0,
        frouter_rx: 0,
        forwarded: 0,
        heap_allocations: 0,
        cpu_busy_ticks: 0,
    };
    loop {
        if let Some(request) = ctx.lifecycle().shutdown_requested() {
            return request;
        }
        // Fresh socket + listener per connection (smoltcp's listening socket
        // becomes the established connection, then returns to Closed).
        let socket = socket::OwnedSocket::open(services.tcp_conn.as_ref(), socket::DOMAIN_TCP)
            .unwrap_or_else(|_| fail(0xe005));
        let port = OwnedMemory::allocate(1).unwrap_or_else(|_| fail(0xe007));
        let mut mapping = port.map_writable().unwrap_or_else(|_| fail(0xe007));
        mapping.as_mut_slice()[..2].copy_from_slice(&HTTP_PORT.to_le_bytes());
        let port = mapping.unmap().unwrap_or_else(|_| fail(0xe007));
        let listen = services
            .tcp_conn
            .as_ref()
            .call_move(socket::OP_LISTEN, socket.id(), port)
            .unwrap_or_else(|_| fail(0xe008))
            .wait()
            .unwrap_or_else(|_| fail(0xe008));
        if listen.result != 0 {
            fail(0xe009);
        }

        // Poll for a connection indefinitely: this is a long-lived keyhole
        // server, so an idle listener must stay alive rather than abort.
        loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                return request;
            }
            let result = socket
                .call(socket::OP_ACCEPT, socket.id())
                .unwrap_or_else(|_| fail(0xe00a))
                .wait()
                .unwrap_or_else(|_| fail(0xe00a))
                .result;
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
        let chunk = loop {
            if let Some(request) = ctx.lifecycle().shutdown_requested() {
                return request;
            }
            match socket.receive_timeout(1, ACCEPT_POLL_MS) {
                Ok(Some(chunk)) => break chunk,
                Ok(None) => fail(0xe00e),
                Err(socket::SocketError::RetryExhausted) => continue,
                Err(_) => fail(0xe00d),
            }
        };
        let (memory, len) = chunk.into_parts();
        let mapping = memory.map_read_only().unwrap_or_else(|_| fail(0xe00f));
        let req_len = len.min(512);
        let mut req = [0u8; 512];
        req[..req_len].copy_from_slice(&mapping.as_slice()[..req_len]);

        // Route on the fixed node/cluster targets. Account for the request up
        // front so the node report includes the request being served.
        counters.requests = counters.requests.wrapping_add(1);
        let path = request_path(&req[..req_len]);
        let (status_line, content_type, body) = if path == b"/" || path == b"/index.html" {
            counters.root = counters.root.wrapping_add(1);
            ("HTTP/1.1 200 OK", "text/html; charset=utf-8", String::from(DASHBOARD))
        } else if path == b"/metrics" || path == b"/metric" {
            counters.metrics = counters.metrics.wrapping_add(1);
            (
                "HTTP/1.1 200 OK",
                "application/json",
                build_json(&mac, link, &services, &counters, &mut prev),
            )
        } else if path == b"/cluster" || path == b"/cluster/" || path == b"/cluster/index.html" {
            counters.root = counters.root.wrapping_add(1);
            ("HTTP/1.1 200 OK", "text/html; charset=utf-8", String::from(CLUSTER_DASHBOARD))
        } else if path == b"/cluster/metrics" {
            counters.metrics = counters.metrics.wrapping_add(1);
            match services
                .dns_conn
                .as_ref()
                .and_then(|connection| build_cluster_json(connection.as_ref()))
                .or_else(|| {
                    // DNS may register after httpd's optional startup lookup,
                    // or may have restarted since then. A fresh name-service
                    // connection keeps the cluster keyhole recoverable.
                    let connection = try_lookup(services.ns_conn, dns::NAME)?;
                    build_cluster_json(connection.as_ref())
                }) {
                Some(snapshot) => ("HTTP/1.1 200 OK", "application/json", snapshot),
                None => (
                    "HTTP/1.1 503 Service Unavailable",
                    "application/json",
                    String::from("{\"error\":\"fresh cluster snapshot unavailable\"}"),
                ),
            }
        } else {
            counters.other = counters.other.wrapping_add(1);
            ("HTTP/1.1 404 Not Found", "text/plain; charset=utf-8", String::from("not found"))
        };
        catten_syscall::el0_log(0x4854_5444, 1);
        let mut response = String::new();
        response.push_str(status_line);
        response.push_str("\r\nContent-Type: ");
        response.push_str(content_type);
        response.push_str("\r\nContent-Length: ");
        let _ = write!(response, "{}", body.len());
        response.push_str(concat!(
            "\r\nCache-Control: no-store",
            "\r\nX-Content-Type-Options: nosniff",
            "\r\nX-Frame-Options: DENY",
            "\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; ",
            "style-src 'unsafe-inline'; connect-src 'self'; object-src 'none'; ",
            "frame-ancestors 'none'",
            "\r\nConnection: close\r\n\r\n"
        ));
        response.push_str(&body);
        if socket.send_all(response.as_bytes(), 1200, ACCEPT_POLL_MS).is_err() {
            fail(0xe010);
        }
        counters.bytes_sent = counters.bytes_sent.wrapping_add(response.len() as u64);

        let _ = socket.close();

        config::write::<u32>(status::REQUESTS, counters.requests);
        config::write::<u32>(status::STAGE, SENTINEL);
    }
}

fn main(ctx: Context) -> ! {
    serve(&ctx).complete()
}

catten_rt::entry!(main);
