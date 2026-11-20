#!/usr/bin/env python3
# Overflows like 32-bit code would: wrong on the larger tests.
import sys
data = sys.stdin.buffer.read().split()
print((sum(map(int, data[1:])) + 2**31) % 2**32 - 2**31)
