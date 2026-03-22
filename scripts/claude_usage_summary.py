#!/usr/bin/env python3
import argparse
import json
import pathlib
import statistics
import sys


def summarize(name: str, values: list[int]) -> None:
    if not values:
        print(f"{name}: no data collected")
        return
    total = sum(values)
    print(
        f"{name}: count={len(values)}  mean={statistics.mean(values):.1f}  "
        f"median={statistics.median(values):.1f}  total={total:,}"
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Summarize Claude token usage from the guardrail log."
    )
    parser.add_argument(
        "--log",
        "-l",
        default="logs/ai_guardrail_feedback.jsonl",
        help="JSONL log with guardrail feedback entries",
    )
    args = parser.parse_args()

    log_path = pathlib.Path(args.log)
    if not log_path.exists():
        print(f"Log file not found: {log_path}", file=sys.stderr)
        return 1

    total_lines = 0
    prompt_tokens: list[int] = []
    completion_tokens: list[int] = []
    total_tokens: list[int] = []

    with log_path.open() as fh:
        for raw in fh:
            total_lines += 1
            raw = raw.strip()
            if not raw:
                continue
            try:
                entry = json.loads(raw)
            except json.JSONDecodeError:
                continue
            prompt = entry.get("prompt_tokens")
            completion = entry.get("completion_tokens")
            total = entry.get("total_tokens")
            if isinstance(prompt, int):
                prompt_tokens.append(prompt)
            if isinstance(completion, int):
                completion_tokens.append(completion)
            if isinstance(total, int):
                total_tokens.append(total)

    print(f"Read {total_lines} guardrail entries from {log_path}")
    summarize("Prompt tokens", prompt_tokens)
    summarize("Completion tokens", completion_tokens)
    summarize("Total tokens", total_tokens)
    if total_tokens and prompt_tokens:
        print(
            "Per review average (total vs prompt): "
            f"{statistics.mean(total_tokens)/statistics.mean(prompt_tokens):.2f}×"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
