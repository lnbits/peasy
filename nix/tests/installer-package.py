"""Approve one fixed test package as root over the real installed daemon IPC."""
import json
import socket
import sys


def request(message):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(600)
        connection.connect("/run/peasy/peasy.sock")
        connection.sendall(json.dumps(message).encode() + b"\n")
        response = json.loads(connection.makefile("rb").readline())
    if response["response"] == "error":
        raise RuntimeError(response["message"])
    return response


operation = sys.argv[1]
assert operation in ["install", "remove"]
proposal = request({"request": "propose_" + operation, "package": "hello"})["proposal"]
print(proposal["title"], flush=True)
result = request({"request": "apply", "proposal": proposal["id"]})["result"]
assert result["activated"], result
packages = request({"request": "get_packages"})["packages"]
assert ("hello" in packages) == (operation == "install"), packages
print(result["message"], flush=True)
