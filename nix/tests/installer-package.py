"""Approve one fixed test package as root over the real installed daemon IPC."""
import json
import errno
import socket
import sys
import time


RESTARTING_MESSAGE = "Peasy is updating. Retry the request shortly."
RESTART_ERRORS = {errno.ENOENT, errno.ECONNREFUSED, errno.ECONNRESET,
                  errno.ECONNABORTED, errno.EPIPE}


def request_once(message, socket_path, timeout):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(timeout)
        connection.connect(socket_path)
        connection.sendall(json.dumps(message).encode() + b"\n")
        with connection.makefile("rb") as reader:
            line = reader.readline()
        if not line:
            raise ConnectionResetError(errno.ECONNRESET, "Peasy closed the connection")
        response = json.loads(line)
    if response["response"] == "error":
        if response["message"] == RESTARTING_MESSAGE:
            raise ConnectionAbortedError(errno.ECONNABORTED, response["message"])
        raise RuntimeError(response["message"])
    return response


def request(message, socket_path="/run/peasy/peasy.sock", restart_timeout=15):
    # The successful apply can be followed immediately by a controlled daemon
    # restart. Wait for the read-back; never replay a proposal or an apply.
    read_only = message["request"] == "get_packages"
    if not read_only:
        return request_once(message, socket_path, 600)
    deadline = time.monotonic() + restart_timeout
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("Peasy did not reconnect after restarting")
        try:
            return request_once(message, socket_path, remaining)
        except OSError as error:
            if error.errno not in RESTART_ERRORS:
                raise
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Peasy did not reconnect after restarting") from error
            time.sleep(min(0.1, remaining))


def main():
    operation = sys.argv[1]
    assert operation in ["install", "remove"]
    package = sys.argv[2] if len(sys.argv) > 2 else "hello"
    assert package in ["hello", "patchelf"]
    proposal = request({"request": "propose_" + operation, "package": package})["proposal"]
    print(proposal["title"], flush=True)
    result = request({"request": "apply", "proposal": proposal["id"]})["result"]
    assert result["activated"], result
    packages = request({"request": "get_packages"})["packages"]
    assert (package in packages) == (operation == "install"), packages
    print(result["message"], flush=True)


if __name__ == "__main__":
    main()
