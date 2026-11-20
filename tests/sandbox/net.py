#!/usr/bin/env python3
import socket
socket.create_connection(("1.1.1.1", 80), timeout=2)
print("NETWORK OPEN")
