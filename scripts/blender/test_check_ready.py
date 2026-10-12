import json
from pathlib import Path
import socket
import sys
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / ".devcontainer/blender"))
from check_ready import check_ready, send_command


class ReadinessTests(unittest.TestCase):
    def connection(self, chunks):
        connection = MagicMock()
        connection.recv.side_effect = chunks
        connection.__enter__.return_value = connection
        return connection

    def test_fragmented_json_response(self):
        connection = self.connection([
            b'{"status":"success","result":{"name":"',
            b"\xc3",
            b'\xa9"}}',
        ])
        with patch("check_ready.socket.create_connection", return_value=connection):
            self.assertEqual(send_command("blender", 9876, {"type": "ping"}), {"name": "\u00e9"})
        self.assertEqual(json.loads(connection.sendall.call_args.args[0]), {"type": "ping"})

    def test_socket_closed_without_response(self):
        with patch("check_ready.socket.create_connection", return_value=self.connection([b""])):
            with self.assertRaisesRegex(ConnectionError, "complete response"):
                send_command("blender", 9876, {"type": "ping"})

    def test_addon_error_is_not_success(self):
        response = b'{"status":"error","message":"GPU not available"}'
        with patch("check_ready.socket.create_connection", return_value=self.connection([response])):
            with self.assertRaisesRegex(RuntimeError, "GPU not available"):
                send_command("blender", 9876, {"type": "ping"})

    def test_non_object_response(self):
        with patch("check_ready.socket.create_connection", return_value=self.connection([b"[]"])):
            with self.assertRaisesRegex(RuntimeError, "non-object"):
                send_command("blender", 9876, {"type": "ping"})

    def test_missing_result(self):
        response = b'{"status":"success"}'
        with patch("check_ready.socket.create_connection", return_value=self.connection([response])):
            with self.assertRaisesRegex(RuntimeError, "missing its result"):
                send_command("blender", 9876, {"type": "ping"})

    def test_response_deadline(self):
        connection = self.connection([b'{"status":'])
        with patch("check_ready.socket.create_connection", return_value=connection), \
                patch("check_ready.time.monotonic", side_effect=[0, 1, 11]):
            with self.assertRaisesRegex(TimeoutError, "response deadline"):
                send_command("blender", 9876, {"type": "ping"}, timeout=10)

    def test_response_size_limit(self):
        with patch("check_ready.socket.create_connection", return_value=self.connection([b"123456789"])), \
                patch("check_ready.MAX_RESPONSE_BYTES", 8):
            with self.assertRaisesRegex(RuntimeError, "exceeded 16 MiB"):
                send_command("blender", 9876, {"type": "ping"})

    def test_response_at_size_limit(self):
        response = b'{"status":"success","result":{}}'
        with patch("check_ready.socket.create_connection", return_value=self.connection([response])), \
                patch("check_ready.MAX_RESPONSE_BYTES", len(response)):
            self.assertEqual(send_command("blender", 9876, {"type": "ping"}), {})

    def test_protocol_mismatch(self):
        with patch("check_ready.send_command", return_value={"protocol_version": 1}):
            with self.assertRaisesRegex(RuntimeError, "protocol"):
                check_ready("blender", 9876, 10)

    def test_expected_protocol(self):
        info = {"protocol_version": 13, "capabilities": ["execute_code"]}
        with patch("check_ready.send_command", return_value=info):
            self.assertEqual(check_ready("blender", 9876, 10), info)

    def test_connection_retry(self):
        info = {"protocol_version": 13, "capabilities": ["execute_code"]}
        with patch("check_ready.send_command", side_effect=[ConnectionRefusedError(), info]), \
                patch("check_ready.time.sleep"):
            self.assertEqual(check_ready("blender", 9876, 10), info)

    def test_timeout_reports_connection_failure(self):
        with patch("check_ready.send_command", side_effect=socket.timeout("no GUI response")), \
                patch("check_ready.time.monotonic", side_effect=[0, 0, 11]):
            with self.assertRaisesRegex(TimeoutError, "no GUI response"):
                check_ready("blender", 9876, 10)


if __name__ == "__main__":
    unittest.main()
