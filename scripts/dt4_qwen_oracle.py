#!/usr/bin/env python3
"""Independent Qwen3 reranker oracle for DT4 admission."""

import argparse
import hashlib
import json
from pathlib import Path

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

PREFIX = '<|im_start|>system\nJudge whether the Document meets the requirements based on the Query and the Instruct provided. Note that the answer can only be "yes" or "no".<|im_end|>\n<|im_start|>user\n'
SUFFIX = "<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
MAX_LENGTH = 8192
YES_TOKEN = 9693
NO_TOKEN = 2152


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-root", type=Path, required=True)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    fixture = json.loads(args.cases.read_text(encoding="utf-8"))
    tokenizer = AutoTokenizer.from_pretrained(args.model_root, local_files_only=True, padding_side="left")
    model = AutoModelForCausalLM.from_pretrained(
        args.model_root, local_files_only=True, torch_dtype=torch.float32
    ).eval()
    yes = tokenizer.encode("yes", add_special_tokens=False)
    no = tokenizer.encode("no", add_special_tokens=False)
    if yes != [YES_TOKEN] or no != [NO_TOKEN]:
        raise RuntimeError(f"unexpected yes/no tokens: yes={yes}, no={no}")
    prefix = tokenizer.encode(PREFIX, add_special_tokens=False)
    suffix = tokenizer.encode(SUFFIX, add_special_tokens=False)

    scores = []
    with torch.no_grad():
        for pair in fixture["pairs"]:
            body = (
                f'<Instruct>: {fixture["instruction"]}\n'
                f'<Query>: {pair["query"]}\n<Document>: {pair["document"]}'
            )
            body_tokens = tokenizer.encode(body, add_special_tokens=False)[
                : MAX_LENGTH - len(prefix) - len(suffix)
            ]
            tokens = torch.tensor([prefix + body_tokens + suffix], dtype=torch.long)
            logits = model(input_ids=tokens).logits[0, -1, [YES_TOKEN, NO_TOKEN]].float()
            probability = torch.softmax(logits, dim=0)[0].item()
            if not torch.isfinite(logits).all():
                raise RuntimeError(f'non-finite logits for {pair["id"]}')
            scores.append({"id": pair["id"], "probability_yes": probability})

    output = {
        "schema": 1,
        "engine": "CPython 3.13.14 / torch 2.7.1 / transformers 4.53.3",
        "model_revision": "e61197ed45024b0ed8a2d74b80b4d909f1255473",
        "yes_token": YES_TOKEN,
        "no_token": NO_TOKEN,
        "files": {
            name: sha256(args.model_root / name)
            for name in ["config.json", "tokenizer.json", "model.safetensors"]
        },
        "cases_sha256": sha256(args.cases),
        "scores": scores,
        "order": [item["id"] for item in sorted(scores, key=lambda item: (-item["probability_yes"], item["id"]))],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8", newline="\n") as destination:
        destination.write(json.dumps(output, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
