import argparse
import json
import socket
import sys
import time

MAX_RESPONSE_BYTES = 16 * 1024 * 1024


def send_command(host, port, command, timeout=10):
    deadline = time.monotonic() + timeout
    with socket.create_connection((host, port), timeout=timeout) as connection:
        connection.sendall(json.dumps(command).encode("utf-8"))
        data = bytearray()
        while len(data) <= MAX_RESPONSE_BYTES:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Blender response deadline expired")
            connection.settimeout(remaining)
            chunk = connection.recv(65536)
            if not chunk:
                raise ConnectionError("Blender closed the socket before sending a complete response")
            data.extend(chunk)
            if len(data) > MAX_RESPONSE_BYTES:
                raise RuntimeError("Blender response exceeded 16 MiB")
            try:
                response = json.loads(data)
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue
            if not isinstance(response, dict):
                raise RuntimeError(f"Blender returned a non-object response: {response}")
            if response.get("status") != "success":
                raise RuntimeError(f"Blender command failed: {response}")
            if "result" not in response:
                raise RuntimeError(f"Blender success response is missing its result: {response}")
            return response["result"]


def check_ready(host, port, timeout):
    deadline = time.monotonic() + timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError(f"Blender readiness deadline expired at {host}:{port}")
        try:
            info = send_command(host, port, {"type": "get_addon_info"}, min(5, remaining))
        except (OSError, ConnectionError) as error:
            if time.monotonic() >= deadline:
                raise TimeoutError(f"Blender not ready at {host}:{port}: {error}") from error
            time.sleep(min(1, max(0, deadline - time.monotonic())))
            continue
        if info.get("protocol_version") != 13 or "execute_code" not in info.get("capabilities", []):
            raise RuntimeError(f"Unexpected Blender add-on protocol/capabilities: {info}")
        return info


def main():
    parser = argparse.ArgumentParser(description="Check the Blender GUI add-on, not just its TCP port")
    parser.add_argument("--host", default="blender")
    parser.add_argument("--port", type=int, default=9876)
    parser.add_argument("--timeout", type=float, default=60)
    args = parser.parse_args()
    if args.timeout <= 0 or not 1 <= args.port <= 65535:
        parser.error("timeout must be positive and port must be between 1 and 65535")
    try:
        print(json.dumps(check_ready(args.host, args.port, args.timeout)))
    except (OSError, ConnectionError, RuntimeError, ValueError, KeyError) as error:
        print(f"Blender readiness failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
