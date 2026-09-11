"""Reproducible metrics; raw WER and a separately labelled format-normalized WER."""
import json
import re
from decimal import Decimal
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "target/evaluation"
SMALL = dict(zip("zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen".split(), range(20)))
SMALL.update(dict(zip("twenty thirty forty fifty sixty seventy eighty ninety".split(), range(20, 100, 10))))
SCALE = {"thousand": 1000, "million": 1000000, "billion": 1000000000}


def normalized_words(text):
    # Currency/unit notation and US/UK spelling are formatting differences.
    text = text.lower().replace("litre", "liter").replace("%", " percent")
    text = re.sub(r"\$(\d[\d,.]*(?:\s+(?:million|thousand|billion))?)", r"\1 dollars", text)
    text = re.sub(r"(?<=\d),(?=\d)", "", text)
    tokens = re.findall(r"\d+(?:\.\d+)?|\.\d+|[a-z]+(?:'[a-z]+)?", text)
    out, i = [], 0
    while i < len(tokens):
        t = tokens[i]
        if t not in SMALL and t != "point" and not re.fullmatch(r"\d+(?:\.\d+)?|\.\d+", t):
            out.append(t)
            i += 1
            continue
        total = Decimal(0)
        current = Decimal(0)
        while i < len(tokens):
            t = tokens[i]
            if t in SMALL:
                current += SMALL[t]
            elif re.fullmatch(r"\d+(?:\.\d+)?|\.\d+", t):
                current += Decimal(t)
            elif t == "hundred":
                current = max(current, 1) * 100
            elif t in SCALE:
                total += max(current, 1) * SCALE[t]
                current = Decimal(0)
            elif t == "point":
                i += 1
                digits = ""
                while i < len(tokens) and tokens[i] in SMALL and SMALL[tokens[i]] < 10:
                    digits += str(SMALL[tokens[i]])
                    i += 1
                current += Decimal("0." + (digits or "0"))
                break
            else:
                break
            i += 1
        out.append(format((total + current).normalize(), "f"))
    return out


def distance(a, b):
    row = list(range(len(b) + 1))
    for i, x in enumerate(a):
        corner, row[0] = row[0], i + 1
        for j, y in enumerate(b):
            previous = row[j + 1]
            row[j + 1] = min(corner + (x != y), row[j] + 1, previous + 1)
            corner = previous
    return row[-1]


def main():
    audio = json.loads((OUT / "audio-stress.json").read_text())
    metrics = {}
    for condition in ("clean", "10dB_white_noise"):
        rows = [r for r in audio["measurements"] if r["condition"] == condition]
        edits = sum(r["edits"] for r in rows)
        words = sum(r["words"] for r in rows)
        pairs = [(normalized_words(r["reference"]), normalized_words(r["transcript"])) for r in rows]
        normalized_edits = sum(distance(a, b) for a, b in pairs)
        normalized_count = sum(len(a) for a, _ in pairs)
        metrics[condition] = dict(clips=len(rows), raw_edits=edits, raw_words=words, raw_wer=edits / words,
                                  normalized_edits=normalized_edits, normalized_words=normalized_count,
                                  normalized_wer=normalized_edits / normalized_count)
    workers = audio["parallel_workers"]
    metrics["stress"] = dict(decodes=sum(w["decodes"] for w in workers),
                             audio_minutes=sum(w["audio_seconds"] for w in workers) / 60,
                             wall_seconds=audio["stress_wall_seconds"],
                             empty=sum(w["empty"] for w in workers),
                             raw_wer=sum(w["edits"] for w in workers) / sum(w["reference_words"] for w in workers),
                             lane_p95_ms=[w["p95_ms"] for w in workers])
    samples = json.loads((OUT / "process-memory.json").read_text(encoding="utf-8-sig"))
    steady = [s for s in samples if s["seconds"] >= 30]
    metrics["memory_sampling"] = dict(samples=len(samples),
                                      peak_working_set_mib=max(s["working_set_bytes"] for s in samples) / 2**20,
                                      peak_private_mib=max(s["private_bytes"] for s in samples) / 2**20,
                                      after_30s_private_growth_mib=(steady[-1]["private_bytes"] - steady[0]["private_bytes"]) / 2**20)
    metrics["recall"] = json.loads((OUT / "memory-stress.json").read_text())
    (OUT / "metrics.json").write_text(json.dumps(metrics, indent=2) + "\n")
    print(json.dumps(metrics, indent=2))


if __name__ == "__main__":
    main()
