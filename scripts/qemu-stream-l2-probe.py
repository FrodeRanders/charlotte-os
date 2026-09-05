#!/usr/bin/env python3
"""External Ethernet/TCP probe for the distributed-ingress QEMU fixture.

The process is a fourth peer on ``qemu-stream-l2-hub.py``. SIGUSR1 starts an
ARP lookup and a bounded set of TCP handshakes. After the original VIP MAC
disappears, a gratuitous ARP identifies the replacement advertiser; HTTP is
then sent on the already-established flows to prove deterministic reselection
onto surviving backends.
"""

from __future__ import annotations

import argparse
import signal
import socket
import struct
import time
from dataclasses import dataclass

ETHERTYPE_IPV4 = 0x0800
ETHERTYPE_ARP = 0x0806
CLIENT_MAC = bytes.fromhex("020000000042")
CLIENT_IP = bytes((10, 0, 0, 250))
HTTP_REQUEST = b"GET /metrics HTTP/1.0\r\n\r\n"
MAX_FRAME = 65_536
RECONNECT_PORT_BASE = 41_000


@dataclass
class Flow:
    port: int
    sequence: int
    server_sequence: int | None = None
    backend_mac: bytes | None = None
    established: bool = False
    survived: bool = False


def checksum(data: bytes) -> int:
    if len(data) & 1:
        data += b"\0"
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return (~total) & 0xFFFF


def tcp_frame(
    destination_mac: bytes,
    vip: bytes,
    port: int,
    sequence: int,
    acknowledgement: int,
    flags: int,
    payload: bytes = b"",
) -> bytes:
    tcp = struct.pack(
        "!HHIIBBHHH",
        port,
        80,
        sequence,
        acknowledgement,
        5 << 4,
        flags,
        32_768,
        0,
        0,
    )
    pseudo = CLIENT_IP + vip + b"\0\x06" + struct.pack("!H", len(tcp) + len(payload))
    tcp_checksum = checksum(pseudo + tcp + payload)
    tcp = tcp[:16] + struct.pack("!H", tcp_checksum) + tcp[18:]
    total_length = 20 + len(tcp) + len(payload)
    ip = struct.pack(
        "!BBHHHBBH4s4s",
        0x45,
        0,
        total_length,
        port & 0xFFFF,
        0x4000,
        64,
        6,
        0,
        CLIENT_IP,
        vip,
    )
    ip = ip[:10] + struct.pack("!H", checksum(ip)) + ip[12:]
    return destination_mac + CLIENT_MAC + struct.pack("!H", ETHERTYPE_IPV4) + ip + tcp + payload


def arp_request(vip: bytes) -> bytes:
    return (
        b"\xff" * 6
        + CLIENT_MAC
        + struct.pack("!H", ETHERTYPE_ARP)
        + struct.pack("!HHBBH", 1, ETHERTYPE_IPV4, 6, 4, 1)
        + CLIENT_MAC
        + CLIENT_IP
        + b"\0" * 6
        + vip
    )


def arp_reply(destination_mac: bytes, destination_ip: bytes) -> bytes:
    return (
        destination_mac
        + CLIENT_MAC
        + struct.pack("!H", ETHERTYPE_ARP)
        + struct.pack("!HHBBH", 1, ETHERTYPE_IPV4, 6, 4, 2)
        + CLIENT_MAC
        + CLIENT_IP
        + destination_mac
        + destination_ip
    )


def send_record(connection: socket.socket, frame: bytes) -> None:
    connection.sendall(struct.pack("!I", len(frame)) + frame)


def parse_address(value: str) -> tuple[str, int]:
    host, port = value.rsplit(":", 1)
    return host, int(port)


def parse_ipv4(value: str) -> bytes:
    octets = bytes(int(part) for part in value.split("."))
    if len(octets) != 4:
        raise ValueError("expected an IPv4 address")
    return octets


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--connect", default="127.0.0.1:12042")
    parser.add_argument("--vip", default="10.0.0.42")
    parser.add_argument("--flows", type=int, default=24)
    parser.add_argument("--timeout", type=float, default=45.0)
    args = parser.parse_args()
    vip = parse_ipv4(args.vip)

    start_requested = False

    def start(_signum: int, _frame: object) -> None:
        nonlocal start_requested
        start_requested = True

    signal.signal(signal.SIGUSR1, start)
    connection = socket.create_connection(parse_address(args.connect))
    connection.settimeout(0.1)
    incoming = bytearray()
    flows = {
        40_000 + index: Flow(40_000 + index, 0x1000_0000 + index * 4096)
        for index in range(args.flows)
    }
    reconnect_flows = {
        RECONNECT_PORT_BASE + index: Flow(
            RECONNECT_PORT_BASE + index, 0x2000_0000 + index * 4096
        )
        for index in range(args.flows)
    }
    owner: bytes | None = None
    original_owner: bytes | None = None
    phase = "armed"
    phase_deadline = float("inf")
    last_probe = 0.0
    verify_trace_count = 0
    print("external L2 probe armed; waiting for SIGUSR1", flush=True)

    while True:
        now = time.monotonic()
        if start_requested and phase == "armed":
            phase = "discover"
            phase_deadline = now + args.timeout
            send_record(connection, arp_request(vip))
            last_probe = now
            print("external L2 probe started", flush=True)

        if phase == "discover" and now - last_probe >= 1.0:
            send_record(connection, arp_request(vip))
            last_probe = now
        elif phase == "handshake" and now - last_probe >= 1.0:
            enough_established = sum(flow.established for flow in flows.values()) >= 3
            for flow in flows.values():
                if not flow.established and not enough_established:
                    send_record(
                        connection,
                        tcp_frame(owner or b"\xff" * 6, vip, flow.port, flow.sequence, 0, 0x02),
                    )
                elif flow.server_sequence is not None:
                    # Seeing SYN/ACK establishes only the probe's view. The
                    # server still needs the final ACK, which may be lost in
                    # this deliberately busy emulated L2 fixture. Repeating
                    # the identical ACK is valid TCP and keeps the later
                    # failover assertion from depending on one frame.
                    send_record(
                        connection,
                        tcp_frame(
                            owner or b"\xff" * 6,
                            vip,
                            flow.port,
                            flow.sequence + 1,
                            flow.server_sequence + 1,
                            0x10,
                        ),
                    )
            last_probe = now
        elif phase == "verify" and now - last_probe >= 1.0:
            # Retransmit the same TCP segment while it remains unacknowledged.
            # The first send deliberately sits on the ARP/advertiser handoff
            # boundary and may be dropped while switching state converges.
            for flow in flows.values():
                if flow.established and not flow.survived and flow.server_sequence is not None:
                    send_record(
                        connection,
                        tcp_frame(
                            owner or b"\xff" * 6,
                            vip,
                            flow.port,
                            flow.sequence + 1,
                            flow.server_sequence + 1,
                            0x18,
                            HTTP_REQUEST,
                        ),
                    )
            last_probe = now
        elif phase == "reconnect" and now - last_probe >= 1.0:
            # The failed advertiser was also a backend for one of the original
            # connections. Until Raft removes that voter, some new hashes may
            # still select it; issue a bounded family of fresh five-tuples and
            # require one live backend to complete a new HTTP exchange.
            for flow in reconnect_flows.values():
                if flow.server_sequence is None:
                    send_record(
                        connection,
                        tcp_frame(owner or b"\xff" * 6, vip, flow.port, flow.sequence, 0, 0x02),
                    )
                elif not flow.survived:
                    send_record(
                        connection,
                        tcp_frame(
                            owner or b"\xff" * 6,
                            vip,
                            flow.port,
                            flow.sequence + 1,
                            flow.server_sequence + 1,
                            0x18,
                            HTTP_REQUEST,
                        ),
                    )
            last_probe = now

        if now >= phase_deadline:
            if phase == "handshake":
                established = [flow for flow in flows.values() if flow.established]
                backend_macs = {
                    flow.backend_mac for flow in established if flow.backend_mac is not None
                }
                print(
                    f"external FAILOVER WINDOW OPEN with {len(established)} established flow(s) "
                    f"owner={':'.join(f'{octet:02x}' for octet in original_owner or b'')}",
                    flush=True,
                )
                if len(backend_macs) < 3:
                    raise SystemExit("external TCP flows did not reach three distinct backends")
                if original_owner not in backend_macs:
                    raise SystemExit("VIP advertiser did not own an established backend flow")
                phase = "failover"
                phase_deadline = now + args.timeout
            elif phase in ("discover", "failover", "verify", "reconnect"):
                raise SystemExit(f"external L2 probe timed out in {phase}")

        try:
            chunk = connection.recv(65_536)
        except socket.timeout:
            continue
        if not chunk:
            raise SystemExit("L2 hub closed the probe connection")
        incoming.extend(chunk)
        while len(incoming) >= 4:
            length = int.from_bytes(incoming[:4], "big")
            if length < 14 or length > MAX_FRAME:
                raise SystemExit(f"invalid hub frame length {length}")
            if len(incoming) < 4 + length:
                break
            frame = bytes(incoming[4 : 4 + length])
            del incoming[: 4 + length]

            if frame[12:14] == struct.pack("!H", ETHERTYPE_ARP) and len(frame) >= 42:
                operation = int.from_bytes(frame[20:22], "big")
                sender_ip = frame[28:32]
                sender_mac = frame[22:28]
                if operation == 1 and frame[38:42] == CLIENT_IP:
                    send_record(connection, arp_reply(sender_mac, sender_ip))
                    continue
                if sender_ip == vip and sender_mac != CLIENT_MAC:
                    # Once failover chooses a replacement, pin it for this
                    # verification window. The hub may still contain a GARP
                    # queued by the killed owner; accepting that stale frame
                    # would direct every retransmission back to the dead MAC
                    # and make the fixture timing-dependent.
                    if phase == "verify":
                        continue
                    previous = owner
                    owner = sender_mac
                    if phase == "discover":
                        original_owner = owner
                        for flow in flows.values():
                            send_record(
                                connection,
                                tcp_frame(owner, vip, flow.port, flow.sequence, 0, 0x02),
                            )
                        phase = "handshake"
                        phase_deadline = time.monotonic() + 8.0
                        last_probe = time.monotonic()
                    elif phase == "failover" and previous != owner and owner != original_owner:
                        print(
                            "external observed replacement VIP advertiser "
                            + ":".join(f"{octet:02x}" for octet in owner),
                            flush=True,
                        )
                        for flow in flows.values():
                            if flow.established and flow.server_sequence is not None:
                                send_record(
                                    connection,
                                    tcp_frame(
                                        owner,
                                        vip,
                                        flow.port,
                                        flow.sequence + 1,
                                        flow.server_sequence + 1,
                                        0x18,
                                        HTTP_REQUEST,
                                    ),
                                )
                        phase = "verify"
                        phase_deadline = time.monotonic() + 20.0
                        last_probe = time.monotonic()
                continue

            if (
                frame[0:6] != CLIENT_MAC
                or frame[12:14] != struct.pack("!H", ETHERTYPE_IPV4)
                or len(frame) < 54
                or frame[23] != 6
                or frame[30:34] != CLIENT_IP
            ):
                continue
            ihl = (frame[14] & 0x0F) * 4
            tcp_offset = 14 + ihl
            if ihl < 20 or len(frame) < tcp_offset + 20:
                continue
            source_port, destination_port = struct.unpack("!HH", frame[tcp_offset : tcp_offset + 4])
            if source_port != 80:
                continue
            flow = flows.get(destination_port)
            if flow is None:
                flow = reconnect_flows.get(destination_port)
            if flow is None:
                continue
            sequence, acknowledgement = struct.unpack("!II", frame[tcp_offset + 4 : tcp_offset + 12])
            flags = frame[tcp_offset + 13]
            tcp_header_length = (frame[tcp_offset + 12] >> 4) * 4
            ip_total_length = int.from_bytes(frame[16:18], "big")
            payload_end = min(len(frame), 14 + ip_total_length)
            payload = frame[tcp_offset + tcp_header_length : payload_end]
            if phase == "verify" and verify_trace_count < 32:
                verify_trace_count += 1
                print(
                    f"external verify rx port={destination_port} flags=0x{flags:02x} "
                    f"seq={sequence} ack={acknowledgement} payload={len(payload)}",
                    flush=True,
                )
            if flags & 0x12 == 0x12 and acknowledgement == flow.sequence + 1:
                flow.server_sequence = sequence
                flow.backend_mac = frame[6:12]
                flow.established = True
                send_record(
                    connection,
                    tcp_frame(owner or frame[6:12], vip, flow.port, flow.sequence + 1, sequence + 1, 0x10),
                )
                if phase == "handshake" and sum(
                    candidate.established for candidate in flows.values()
                ) >= 3:
                    # Three simultaneous listeners imply three different
                    # backends in this fixture. Stop the SYN fan-out and leave
                    # quiet retransmission intervals for the final ACKs. Three
                    # TCG guests can take several wall-clock seconds to drain
                    # the asynchronous frouter and smoltcp reactors.
                    phase_deadline = min(phase_deadline, time.monotonic() + 8.0)
                if phase == "reconnect" and destination_port in reconnect_flows:
                    send_record(
                        connection,
                        tcp_frame(
                            owner or frame[6:12],
                            vip,
                            flow.port,
                            flow.sequence + 1,
                            sequence + 1,
                            0x18,
                            HTTP_REQUEST,
                        ),
                    )
            elif phase == "verify" and payload.startswith(b"HTTP/1.1"):
                flow.survived = True
                survivors = sum(candidate.survived for candidate in flows.values())
                print(f"external {survivors} flow(s) survived the failover window", flush=True)
                phase = "reconnect"
                phase_deadline = time.monotonic() + 20.0
                last_probe = 0.0
            elif (
                phase == "reconnect"
                and destination_port in reconnect_flows
                and payload.startswith(b"HTTP/1.1")
            ):
                flow.survived = True
                print(
                    f"external reconnect succeeded on source port {destination_port} "
                    f"backend={':'.join(f'{octet:02x}' for octet in flow.backend_mac or b'')}",
                    flush=True,
                )
                raise SystemExit(0)


if __name__ == "__main__":
    main()
