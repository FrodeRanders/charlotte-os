#!/usr/bin/env python3
"""Native intermediate-date checks and a signed upload spanning issuer expiry."""
from datetime import datetime, timedelta, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import socket
import ssl
import subprocess
import time

from management_tls import ProbeFailure, context, exchange

spec = importlib.util.spec_from_file_location("chain_validity", Path(__file__).with_name("test-management-validity.py"))
validity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validity)
ROOT = Path(__file__).resolve().parents[2]


def issuer(directory, seconds):
    now = datetime.now(timezone.utc).replace(microsecond=0)
    before, after = now-timedelta(minutes=1), now+timedelta(seconds=seconds)
    subprocess.run(["openssl", "ca", "-batch", "-notext", "-config", "chain-ca.conf", "-extfile", "chain-issuer.ext",
                    "-startdate", before.strftime("%Y%m%d%H%M%SZ"), "-enddate", after.strftime("%Y%m%d%H%M%SZ"),
                    "-in", "session-issuer.csr", "-out", "session-issuer-short.pem"],
                   cwd=directory, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    chain = directory / "session-administrator.pem"
    chain.write_bytes((directory / "session-administrator-leaf.pem").read_bytes()+(directory / "session-issuer-short.pem").read_bytes())
    policy = ssl.create_default_context(cafile=directory / "ca.pem")
    policy.minimum_version = policy.maximum_version = ssl.TLSVersion.TLSv1_3
    policy.set_alpn_protocols(["http/1.1"])
    policy.load_cert_chain(chain, directory / "session-administrator.key")
    return policy, {"issuer_not_before": before.isoformat(), "issuer_not_after": after.isoformat(),
                    "not_after_unix_seconds": int(after.timestamp()),
                    "issuer_pem_sha256": hashlib.sha256((directory / "session-issuer-short.pem").read_bytes()).hexdigest()}


def admissions(text, digest):
    return {(int(m[1]), int(m[2])) for m in re.finditer(
        r"\[EL0 LOG\] lp=\d+ asid=(\d+) \[management\] connection=(\d+) certificate="+re.escape(digest)
        +r" role=Administrator permission=Mutate allowed=true credential_utc=\d+\r?$", text, re.MULTILINE)}


def authority_times(text, digest, role, permission):
    return {(int(m[1]), int(m[2])): int(m[3]) for m in re.finditer(
        r"\[EL0 LOG\] lp=\d+ asid=(\d+) \[management\] connection=(\d+) certificate="+re.escape(digest)
        +r" role="+role+r" permission="+permission+r" allowed=true credential_utc=(\d+)\r?$", text, re.MULTILINE)}


def wait_event(log, read, previous, end):
    while True:
        new = read(log.read_text()) - previous
        if new:
            if len(new) != 1:
                raise ProbeFailure("ambiguous session-validity episode")
            return next(iter(new))
        if time.monotonic() >= end:
            raise ProbeFailure("session-validity lacked its exact native event")
        # Serial-file delivery has no host readiness registration surface.
        time.sleep(min(0.05, max(0, end-time.monotonic())))


def held_upload(policy, directory, port, body, log, end, window, observe_clock):
    def remaining():
        value = end-time.monotonic()
        if value <= 0:
            raise TimeoutError("session-validity exhausted its original budget")
        return value
    digest = hashlib.sha256((directory / "session-administrator.der").read_bytes()).hexdigest()
    previous = admissions(log.read_text(), digest)
    expired_before = validity.events(log.read_text(), "expired")
    request = f"POST /v1/ingress-policy HTTP/1.1\r\nHost: localhost\r\nContent-Length: {len(body)}\r\n\r\n".encode()
    sent_final = False
    with socket.create_connection(("127.0.0.1", port), timeout=remaining()) as raw:
        raw.settimeout(remaining())
        with policy.wrap_socket(raw, server_hostname="127.0.0.1") as peer:
            if peer.version() != "TLSv1.3" or peer.cipher()[0] != "TLS_AES_128_GCM_SHA256" or peer.session_reused:
                raise ProbeFailure("unexpected established-session TLS negotiation")
            peer.settimeout(remaining()); peer.sendall(request+body[:-1])
            episode = wait_event(log, lambda text: admissions(text, digest), previous, end)
            admitted_utc = authority_times(log.read_text(), digest, "Administrator", "Mutate")[episode]
            target = time.monotonic()+max(0, window["not_after_unix_seconds"]+1.1-admitted_utc)
            if target >= end:
                raise ProbeFailure("issuer window cannot fit the original probe budget")
            terminal = None
            while time.monotonic() < target:
                if select.select([peer], [], [], min(remaining(), max(0,target-time.monotonic())))[0]:
                    try:
                        data = peer.recv(4096)
                    except ssl.SSLCertVerificationError:
                        raise ProbeFailure("server verification cannot prove issuer expiry") from None
                    except (ssl.SSLError, ConnectionError) as error:
                        terminal = type(error).__name__; break
                    if data:
                        raise ProbeFailure("incomplete signed upload received application bytes")
                    terminal = "EOF"; break
            if terminal is None:
                checked_clock = observe_clock(end)
                if checked_clock["source_asid"] != episode[0] or checked_clock["credential_utc"] <= window["not_after_unix_seconds"]:
                    raise ProbeFailure("native credential clock has not crossed issuer expiry")
                window["native_clock_observation"] = checked_clock
                peer.settimeout(remaining())
                try:
                    peer.sendall(body[-1:]); sent_final = True
                    data = peer.recv(4096)
                    if data:
                        raise ProbeFailure("expired upload received application bytes")
                    terminal = "EOF"
                except ssl.SSLCertVerificationError:
                    raise ProbeFailure("server verification cannot prove issuer expiry") from None
                except (ssl.SSLError, ConnectionError) as error:
                    terminal = type(error).__name__
            expired = wait_event(log, lambda text: validity.events(text, "expired"), expired_before, end)
            if expired != episode:
                raise ProbeFailure("expiry event did not match the admitted partial upload")
    return {**window, "admitted_credential_utc": admitted_utc, "source_asid": episode[0], "connection": episode[1], "client_leaf_sha256": digest,
            "native_admission": "Administrator Mutate allowed=true", "native_rejection": "client certificate expired",
            "transport_result": terminal, "final_body_byte_submitted": sent_final, "application_bytes": 0}


def probe(log, status_port, deployment_port, budget=12):
    validity.probe(log, status_port, deployment_port, budget, scope="issuer")
    directory = Path(os.environ["CATTEN_MANAGEMENT_VALIDITY_DIR"])
    cluster_id = (directory / "policy.bin").read_bytes()[24:56].hex()
    bodies = []
    for sequence in (1, 2):
        path = directory / ("session-ingress-"+str(sequence))
        now = int(time.time())
        subprocess.run([ROOT / "target/debug/cluster-sign", "ingress-policy-sign", path, str(sequence), str(now-60), str(now+1800),
                        cluster_id, ROOT / "tools/cluster-sign/dev-operations-key.hex", "10.0.2.42:443", "10.0.2.42:7444"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        bodies.append(path.read_bytes())
    positive, window = issuer(directory, 12)
    request = f"POST /v1/ingress-policy HTTP/1.1\r\nHost: localhost\r\nContent-Length: {len(bodies[0])}\r\n\r\n".encode()+bodies[0]
    receipt, _ = exchange(positive, deployment_port, request, 202, budget, include_body=True)
    if json.loads(receipt["body"]) != {"generation": 1}:
        raise ProbeFailure("session administrator's positive mutation receipt was not generation 1")
    healthy = context("reader")
    read = b"GET /v1/ingress-policy HTTP/1.1\r\nHost: localhost\r\n\r\n"
    baseline, _ = exchange(healthy, deployment_port, read, 200, budget, include_body=True)
    if json.loads(baseline["body"]).get("sequence") != 1:
        raise ProbeFailure("positive session mutation did not commit")
    end = time.monotonic()+budget
    short, window = issuer(directory, 6)
    print("OBSERVED: management session issuer " + json.dumps(window, sort_keys=True), flush=True)
    reader_digest = hashlib.sha256((directory / "reader.der").read_bytes()).hexdigest()
    def observe_clock(end):
        previous = set(authority_times(log.read_text(), reader_digest, "Reader", "Observe"))
        result, _ = exchange(healthy, deployment_port, read, 200, max(0,end-time.monotonic()), include_body=True)
        if json.loads(result["body"]) != json.loads(baseline["body"]):
            raise ProbeFailure("partial upload changed policy before expiry observation")
        episode = wait_event(log, lambda text: set(authority_times(text, reader_digest, "Reader", "Observe")), previous, end)
        return {"source_asid": episode[0], "connection": episode[1],
                "credential_utc": authority_times(log.read_text(), reader_digest, "Reader", "Observe")[episode]}
    observation = held_upload(short, directory, deployment_port, bodies[1], log, end, window, observe_clock)
    recovered, _ = exchange(healthy, deployment_port, read, 200, budget, include_body=True)
    if json.loads(recovered["body"]) != json.loads(baseline["body"]):
        raise ProbeFailure("expired incomplete upload changed policy state")
    observation.update(policy_generation=1, policy_sequence=1, healthy_reader_status=200,
                       rejected_body_sha256=hashlib.sha256(bodies[1]).hexdigest(),
                       scope="native established administrator upload spanning intermediate expiry")
    print("PASS: management established expiry " + json.dumps(observation, sort_keys=True), flush=True)
    return observation
