"""Demo program for dap-cli REPL / show command tests.

Run with the fake adapter:
  ./scripts/test-debug-repl.sh plain

Run with real debugpy (pip install -r scripts/fixtures/requirements.txt):
  ./scripts/test-debug-repl.sh debugpy
"""

from dataclasses import dataclass


@dataclass
class Config:
    """Runtime knobs for the demo workload."""

    multiplier: int = 3
    start: int = 1
    end: int = 11
    label: str = "demo"


def validate_config(config: Config) -> bool:
    if config.start >= config.end:
        return False
    if config.multiplier < 1:
        return False
    return True


def normalize(value: int, offset: int) -> int:
    """Shift and clamp a single input value."""
    shifted = value + offset
    if shifted < 0:
        return 0
    return shifted


def process_item(value: int, multiplier: int, offset: int = 0) -> int:
    """Transform one item before it contributes to the running total."""
    normalized = normalize(value, offset)
    weighted = normalized * multiplier
    bonus = 1 if normalized % 2 == 0 else 0
    return weighted + bonus


def accumulate(items: range, multiplier: int) -> int:
    """Sum processed values — fake adapter stops on the loop body below."""
    total = 0
    offset = 0

    for idx, value in enumerate(items):
        offset = idx % 4
        step = process_item(value, multiplier, offset)
        total += step
        # breakpoint-friendly line for `show` demos
        running = total

    return running


def format_result(config: Config, total: int) -> str:
    span = config.end - config.start
    return f"{config.label}: sum={total} over {span} items (x{config.multiplier})"


def report(config: Config, total: int) -> None:
    message = format_result(config, total)
    print(message)
    print(f"  start={config.start} end={config.end}")


def main() -> int:
    config = Config()
    if not validate_config(config):
        raise ValueError("invalid demo configuration")

    items = range(config.start, config.end)
    total = accumulate(items, config.multiplier)
    report(config, total)
    return total


if __name__ == "__main__":
    main()
