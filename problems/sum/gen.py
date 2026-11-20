#!/usr/bin/env python3
"""Writes the large tests (03, 04), which are too big to keep in git."""
import random
from pathlib import Path

tests = Path(__file__).parent / "tests"


def write(name, nums):
    (tests / f"{name}.in").write_text(f"{len(nums)}\n{' '.join(map(str, nums))}\n")
    (tests / f"{name}.out").write_text(f"{sum(nums)}\n")


random.seed(7)
write("03", [10**9] * 200_000)
write("04", [random.randint(-10**9, 10**9) for _ in range(2_000_000)])
