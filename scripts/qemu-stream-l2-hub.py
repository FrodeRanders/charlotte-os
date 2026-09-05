#!/usr/bin/env python3
"""Small bounded Ethernet hub for QEMU ``-netdev stream`` fixtures.

QEMU's stream backend carries each Ethernet frame as a four-byte big-endian
length followed by the frame. This process accepts multiple localhost QEMU
connections and relays each complete record to every other participant.
"""

from __future__ import annotations

import argparse
import selectors
import socket
from dataclasses import dataclass, field

MAX_FRAME = 65_536
MAX_QUEUED = 1_048_576
MAX_TCP_TRACE = 512


@dataclass
class Peer:
    sock: socket.socket
    incoming: bytearray = field(default_factory=bytearray)
    outgoing: bytearray = field(default_factory=bytearray)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="127.0.0.1:12042")
    parser.add_argument("--trace-tcp", action="store_true")
    args = parser.parse_args()
    host, port_text = args.listen.rsplit(":", 1)
    port = int(port_text)

    selector = selectors.DefaultSelector()
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind((host, port))
    listener.listen()
    listener.setblocking(False)
    selector.register(listener, selectors.EVENT_READ, None)
    peers: dict[socket.socket, Peer] = {}
    tcp_trace_count = 0

    def trace_tcp(frame: bytes) -> None:
        nonlocal tcp_trace_count
        if not args.trace_tcp or tcp_trace_count >= MAX_TCP_TRACE or len(frame) < 54:
            return
        ethertype = frame[12:14]
        ip_offset = 14
        path = "external"
        if ethertype == b"\x88\xb8" and len(frame) >= 62 and frame[20:22] == b"\x08\x00":
            ip_offset = 22
            path = "forwarded"
        elif ethertype != b"\x08\x00":
            return
        if frame[ip_offset + 9] != 6:
            return
        ihl = (frame[ip_offset] & 0x0F) * 4
        tcp = ip_offset + ihl
        if ihl < 20 or len(frame) < tcp + 20:
            return
        flags = frame[tcp + 13]
        source_port = int.from_bytes(frame[tcp : tcp + 2], "big")
        destination_port = int.from_bytes(frame[tcp + 2 : tcp + 4], "big")
        selected_probe_flow = source_port in (40_000, 40_001, 40_002) or destination_port in (
            40_000,
            40_001,
            40_002,
        )
        # Handshake/teardown packets carry the most useful bounded trace. ACK
        # data traffic is normally omitted; retain it for the three attributed
        # flows so failover diagnostics expose both envelope hops.
        if flags & 0x07 == 0 and flags & 0x02 == 0 and not selected_probe_flow:
            return
        tcp_trace_count += 1
        source = ".".join(str(octet) for octet in frame[ip_offset + 12 : ip_offset + 16])
        destination = ".".join(str(octet) for octet in frame[ip_offset + 16 : ip_offset + 20])
        destination_mac = ":".join(f"{octet:02x}" for octet in frame[:6])
        sequence = int.from_bytes(frame[tcp + 4 : tcp + 8], "big")
        acknowledgement = int.from_bytes(frame[tcp + 8 : tcp + 12], "big")
        header_length = (frame[tcp + 12] >> 4) * 4
        total_length = int.from_bytes(frame[ip_offset + 2 : ip_offset + 4], "big")
        payload_length = max(0, total_length - ihl - header_length)
        print(
            f"tcp path={path} dst-mac={destination_mac} {source}:{source_port} -> "
            f"{destination}:{destination_port} flags=0x{flags:02x} seq={sequence} "
            f"ack={acknowledgement} payload={payload_length}",
            flush=True,
        )

    def close(peer: Peer) -> None:
        peers.pop(peer.sock, None)
        try:
            selector.unregister(peer.sock)
        except (KeyError, ValueError):
            pass
        peer.sock.close()

    def interest(peer: Peer) -> None:
        events = selectors.EVENT_READ
        if peer.outgoing:
            events |= selectors.EVENT_WRITE
        selector.modify(peer.sock, events, peer)

    print(f"QEMU L2 hub listening on {host}:{port}", flush=True)
    while True:
        for key, mask in selector.select():
            if key.data is None:
                connection, address = listener.accept()
                connection.setblocking(False)
                peer = Peer(connection)
                peers[connection] = peer
                selector.register(connection, selectors.EVENT_READ, peer)
                print(f"participant connected from {address[0]}:{address[1]}", flush=True)
                continue

            peer: Peer = key.data
            if mask & selectors.EVENT_READ:
                try:
                    chunk = peer.sock.recv(65_536)
                except BlockingIOError:
                    chunk = None
                except ConnectionResetError:
                    chunk = b""
                if chunk is None:
                    pass
                elif not chunk:
                    close(peer)
                    continue
                else:
                    peer.incoming.extend(chunk)
                    while len(peer.incoming) >= 4:
                        length = int.from_bytes(peer.incoming[:4], "big")
                        if length < 14 or length > MAX_FRAME:
                            close(peer)
                            break
                        record_length = 4 + length
                        if len(peer.incoming) < record_length:
                            break
                        record = bytes(peer.incoming[:record_length])
                        del peer.incoming[:record_length]
                        trace_tcp(record[4:])
                        for target in list(peers.values()):
                            if target is peer:
                                continue
                            if len(target.outgoing) + record_length > MAX_QUEUED:
                                close(target)
                            elif target.sock in peers:
                                target.outgoing.extend(record)
                                interest(target)

            if peer.sock in peers and mask & selectors.EVENT_WRITE and peer.outgoing:
                try:
                    sent = peer.sock.send(peer.outgoing)
                except BlockingIOError:
                    sent = 0
                except (BrokenPipeError, ConnectionResetError):
                    close(peer)
                    continue
                if sent > 0:
                    del peer.outgoing[:sent]
                    interest(peer)


if __name__ == "__main__":
    main()
