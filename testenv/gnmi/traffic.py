"""Steady UDP traffic for the gNMI lab.

Sends datagrams to --peer at about --kbit kbit/s (IP payload size, so the routers count a
little more with headers), and receives and discards whatever arrives on the same port, so
the peer's stream gets no ICMP port-unreachable replies.
"""

import argparse
import socket
import threading
import time

PORT = 5001
PAYLOAD = 1000  # bytes of UDP payload per datagram


def receive() -> None:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("0.0.0.0", PORT))
    while True:
        sock.recv(65535)


def send(peer: str, kbit: int) -> None:
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    data = b"\0" * PAYLOAD
    interval = PAYLOAD * 8 / (kbit * 1000)
    next_at = time.monotonic()
    while True:
        try:
            sock.sendto(data, (peer, PORT))
        except OSError:
            # The route or the peer may not be up yet; keep the pace and try again.
            pass
        next_at += interval
        delay = next_at - time.monotonic()
        if delay > 0:
            time.sleep(delay)
        else:
            next_at = time.monotonic()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--peer", required=True)
    parser.add_argument("--kbit", type=int, required=True)
    args = parser.parse_args()
    threading.Thread(target=receive, daemon=True).start()
    print(f"sending {args.kbit} kbit/s to {args.peer}:{PORT}, receiving on {PORT}")
    send(args.peer, args.kbit)


if __name__ == "__main__":
    main()
